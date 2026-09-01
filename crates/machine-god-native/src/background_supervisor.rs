//! Host-owned composition of background persistence and process supervision.

use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use machine_god_core::{
    BackgroundClock, BackgroundCompletionRecord, BackgroundProcessOutcome as CoreProcessOutcome,
    BackgroundProcessRetainer, BackgroundProcessSpawner, BackgroundRecordLease,
    BackgroundRetentionPermit, BackgroundRunningRecord, BackgroundStartError,
    BackgroundStartErrorKind, BackgroundStartRequest, BackgroundStore as CoreBackgroundStore,
    BackgroundSupervisor, BoxFuture, CancellationToken, OwnedBackgroundProcess as CoreOwnedProcess,
    PreparedBackgroundProcess as CorePreparedProcess,
};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{FileType, Mode, OFlags};

use crate::background_inspection::{NativeBackgroundState, StoredBackgroundRecord};
#[cfg(target_os = "macos")]
use crate::background_process::BackgroundProcessHelper;
use crate::background_process::{
    BackgroundProcessExit, BackgroundProcessOutcome, BackgroundProcessRequest,
    OwnedBackgroundProcess, PreparedBackgroundProcess, SystemBackgroundProcessAdapter,
};
use crate::background_store::{
    BackgroundReconciliation, BackgroundRecordLease as NativeRecordAuthority, BackgroundStore,
};
use crate::session_store::FileSessionStore;

/// Default number of concurrently retained background processes.
pub const NATIVE_BACKGROUND_DEFAULT_MAX_ACTIVE: usize = 4;
/// Hard upper bound for concurrently retained background processes.
pub const NATIVE_BACKGROUND_HARD_MAX_ACTIVE: usize = 16;
/// Exact private CLI argument used by the macOS inherited-directory helper.
pub const BACKGROUND_PROCESS_HELPER_ARGUMENT: &str = "--__machine-god-background-exec-helper";

/// Fixed host-owned background concurrency limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeBackgroundLimits {
    max_active: usize,
}

impl NativeBackgroundLimits {
    /// Validates one fail-fast active-process limit.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-configuration error outside `1..=16`.
    pub const fn new(max_active: usize) -> Result<Self, NativeBackgroundSupervisorError> {
        if max_active == 0 || max_active > NATIVE_BACKGROUND_HARD_MAX_ACTIVE {
            return Err(NativeBackgroundSupervisorError::new(
                NativeBackgroundSupervisorErrorKind::InvalidConfiguration,
            ));
        }
        Ok(Self { max_active })
    }

    /// Returns the configured maximum active count.
    #[must_use]
    pub const fn max_active(self) -> usize {
        self.max_active
    }
}

impl Default for NativeBackgroundLimits {
    fn default() -> Self {
        Self {
            max_active: NATIVE_BACKGROUND_DEFAULT_MAX_ACTIVE,
        }
    }
}

/// Stable category for native supervisor construction or reconciliation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeBackgroundSupervisorErrorKind {
    InvalidConfiguration,
    Workspace,
    State,
    Environment,
    Process,
    Worker,
    Reconciliation,
}

/// Fixed, data-free native supervisor failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeBackgroundSupervisorError {
    kind: NativeBackgroundSupervisorErrorKind,
}

impl NativeBackgroundSupervisorError {
    const fn new(kind: NativeBackgroundSupervisorErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(self) -> NativeBackgroundSupervisorErrorKind {
        self.kind
    }
}

impl fmt::Debug for NativeBackgroundSupervisorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeBackgroundSupervisorError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeBackgroundSupervisorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("native background supervisor is unavailable")
    }
}

impl Error for NativeBackgroundSupervisorError {}

/// Bounded startup-reconciliation result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeBackgroundReconciliation {
    inspected: usize,
    active: usize,
    marked_stale: usize,
}

impl NativeBackgroundReconciliation {
    #[must_use]
    pub const fn inspected(self) -> usize {
        self.inspected
    }

    #[must_use]
    pub const fn active(self) -> usize {
        self.active
    }

    #[must_use]
    pub const fn marked_stale(self) -> usize {
        self.marked_stale
    }
}

impl From<BackgroundReconciliation> for NativeBackgroundReconciliation {
    fn from(value: BackgroundReconciliation) -> Self {
        Self {
            inspected: value.inspected,
            active: value.active,
            marked_stale: value.marked_stale,
        }
    }
}

/// Production Linux/macOS background supervisor.
pub struct NativeBackgroundSupervisor {
    supervisor: BackgroundSupervisor,
    store: Arc<NativeStore>,
    retainer: Arc<WorkerRetainer>,
}

impl NativeBackgroundSupervisor {
    /// Opens retained workspace and state-root authorities and snapshots the environment.
    ///
    /// # Errors
    ///
    /// Returns a fixed category if either root cannot be retained, the
    /// environment exceeds its bounds, reconciliation fails, or workers
    /// cannot be created.
    pub fn open(
        workspace_root: &Path,
        state_root: &Path,
    ) -> Result<Self, NativeBackgroundSupervisorError> {
        Self::open_with_limits(
            workspace_root,
            state_root,
            NativeBackgroundLimits::default(),
        )
    }

    /// Opens the production supervisor with explicit bounded concurrency.
    ///
    /// # Errors
    ///
    /// Returns a fixed category for invalid roots, environment, state,
    /// process-helper configuration, reconciliation, or worker creation.
    pub fn open_with_limits(
        workspace_root: &Path,
        state_root: &Path,
        limits: NativeBackgroundLimits,
    ) -> Result<Self, NativeBackgroundSupervisorError> {
        let (workspace, workspace_root) =
            retain_canonical_directory(workspace_root).map_err(|()| {
                NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Workspace)
            })?;
        let session_store = FileSessionStore::open(state_root).map_err(|_| {
            NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::State)
        })?;
        let state_root = session_store.try_clone_root_descriptor().map_err(|_| {
            NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::State)
        })?;
        Self::from_root_descriptors(
            workspace,
            workspace_root,
            state_root,
            env::vars_os().collect(),
            limits,
        )
    }

    /// Composes already-retained exact root authorities.
    ///
    /// `workspace` must name the canonical identity represented by
    /// `workspace_root`; callers retaining descriptors are responsible for
    /// establishing that association before path replacement can occur.
    ///
    /// # Errors
    ///
    /// Returns a fixed category when a supplied authority or bounded setting
    /// is invalid, or when state, reconciliation, helper, or workers fail.
    pub fn from_root_descriptors(
        workspace: String,
        workspace_root: OwnedFd,
        state_root: OwnedFd,
        environment: Vec<(OsString, OsString)>,
        limits: NativeBackgroundLimits,
    ) -> Result<Self, NativeBackgroundSupervisorError> {
        #[cfg(target_os = "linux")]
        let adapter = system_process_adapter();
        #[cfg(target_os = "macos")]
        let adapter = system_process_adapter()?;
        Self::from_parts(
            workspace,
            workspace_root,
            state_root,
            environment,
            limits,
            adapter,
        )
    }

    fn from_parts(
        workspace: String,
        workspace_root: OwnedFd,
        state_root: OwnedFd,
        environment: Vec<(OsString, OsString)>,
        limits: NativeBackgroundLimits,
        adapter: SystemBackgroundProcessAdapter,
    ) -> Result<Self, NativeBackgroundSupervisorError> {
        if !canonical_absolute_path(&workspace) || limits.max_active() == 0 {
            return Err(NativeBackgroundSupervisorError::new(
                NativeBackgroundSupervisorErrorKind::InvalidConfiguration,
            ));
        }
        validate_directory(workspace_root.as_fd()).map_err(|()| {
            NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Workspace)
        })?;
        validate_environment(&workspace, &workspace_root, &environment)?;

        let store = Arc::new(NativeStore {
            inner: Arc::new(
                BackgroundStore::prepare(state_root, workspace.clone()).map_err(|_| {
                    NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::State)
                })?,
            ),
        });
        store.inner.reconcile().map_err(|_| {
            NativeBackgroundSupervisorError::new(
                NativeBackgroundSupervisorErrorKind::Reconciliation,
            )
        })?;

        let spawner = Arc::new(NativeSpawner {
            workspace,
            workspace_root: Arc::new(workspace_root),
            environment: Arc::new(environment),
            adapter,
        });
        let retainer = Arc::new(WorkerRetainer::new(limits.max_active())?);
        let supervisor = BackgroundSupervisor::new(
            Arc::new(SystemClock),
            Arc::clone(&store) as Arc<dyn CoreBackgroundStore>,
            spawner as Arc<dyn BackgroundProcessSpawner>,
            Arc::clone(&retainer) as Arc<dyn BackgroundProcessRetainer>,
        );
        Ok(Self {
            supervisor,
            store,
            retainer,
        })
    }

    /// Returns an inert start future. Effects begin only when it is polled.
    #[must_use]
    pub fn start(
        &self,
        request: BackgroundStartRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<machine_god_core::BackgroundHandle, BackgroundStartError>> {
        self.supervisor.start(request, cancellation)
    }

    /// Reconciles persisted unlocked running records when this future is polled.
    #[must_use]
    pub fn reconcile(
        &self,
    ) -> BoxFuture<'_, Result<NativeBackgroundReconciliation, NativeBackgroundSupervisorError>>
    {
        Box::pin(async move {
            self.store.inner.reconcile().map(Into::into).map_err(|_| {
                NativeBackgroundSupervisorError::new(
                    NativeBackgroundSupervisorErrorKind::Reconciliation,
                )
            })
        })
    }
}

impl fmt::Debug for NativeBackgroundSupervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeBackgroundSupervisor")
            .finish_non_exhaustive()
    }
}

impl Drop for NativeBackgroundSupervisor {
    fn drop(&mut self) {
        self.retainer.shutdown();
    }
}

#[derive(Debug)]
struct SystemClock;

impl BackgroundClock for SystemClock {
    fn now_millis(&self) -> Result<u64, BackgroundStartError> {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| start_error(BackgroundStartErrorKind::Clock))?
            .as_millis();
        u64::try_from(millis).map_err(|_| start_error(BackgroundStartErrorKind::Clock))
    }
}

struct NativeStore {
    inner: Arc<BackgroundStore>,
}

impl fmt::Debug for NativeStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStore")
            .finish_non_exhaustive()
    }
}

impl CoreBackgroundStore for NativeStore {
    fn reserve(
        &self,
    ) -> BoxFuture<'_, Result<Box<dyn BackgroundRecordLease>, BackgroundStartError>> {
        Box::pin(async move {
            let lease = self
                .inner
                .reserve_id()
                .map_err(|_| start_error(BackgroundStartErrorKind::Persistence))?;
            Ok(Box::new(NativeLease {
                store: Arc::clone(&self.inner),
                lease,
            }) as Box<dyn BackgroundRecordLease>)
        })
    }
}

struct NativeLease {
    store: Arc<BackgroundStore>,
    lease: NativeRecordAuthority,
}

impl BackgroundRecordLease for NativeLease {
    fn id(&self) -> u64 {
        self.lease.id()
    }

    fn publish_initial<'a>(
        &'a self,
        record: &'a BackgroundRunningRecord,
    ) -> BoxFuture<'a, Result<(), BackgroundStartError>> {
        Box::pin(async move {
            self.store
                .publish_initial(&self.lease, &initial_record(self.store.workspace(), record))
                .map_err(|_| start_error(BackgroundStartErrorKind::Persistence))
        })
    }

    fn publish_completion<'a>(
        &'a self,
        initial: &'a BackgroundRunningRecord,
        completion: &'a BackgroundCompletionRecord,
    ) -> BoxFuture<'a, Result<(), BackgroundStartError>> {
        Box::pin(async move {
            self.store
                .replace(
                    &self.lease,
                    &completion_record(self.store.workspace(), initial, completion),
                )
                .map_err(|_| start_error(BackgroundStartErrorKind::Persistence))
        })
    }
}

fn initial_record(workspace: &str, record: &BackgroundRunningRecord) -> StoredBackgroundRecord {
    StoredBackgroundRecord {
        version: 1,
        workspace: workspace.to_owned(),
        id: record.id(),
        started_at_ms: record.started_at_ms(),
        updated_at_ms: record.started_at_ms(),
        command: record.command().to_owned(),
        cwd: record.cwd().to_owned(),
        state: NativeBackgroundState::Running,
        pid: record.pid().map(std::num::NonZeroU32::get),
        exit_code: None,
        server_url: None,
        diagnostic: None,
    }
}

fn completion_record(
    workspace: &str,
    initial: &BackgroundRunningRecord,
    completion: &BackgroundCompletionRecord,
) -> StoredBackgroundRecord {
    let (state, exit_code) = match completion.outcome() {
        CoreProcessOutcome::Exited(0) => (NativeBackgroundState::Exited, Some(0)),
        CoreProcessOutcome::Exited(code) => (NativeBackgroundState::Failed, Some(code)),
        CoreProcessOutcome::Stopped(code) => (NativeBackgroundState::Stopped, code),
        _ => (NativeBackgroundState::Dead, None),
    };
    StoredBackgroundRecord {
        version: 1,
        workspace: workspace.to_owned(),
        id: initial.id(),
        started_at_ms: initial.started_at_ms(),
        updated_at_ms: completion.updated_at_ms(),
        command: initial.command().to_owned(),
        cwd: initial.cwd().to_owned(),
        state,
        pid: initial.pid().map(std::num::NonZeroU32::get),
        exit_code,
        server_url: None,
        diagnostic: None,
    }
}

struct NativeSpawner {
    workspace: String,
    workspace_root: Arc<OwnedFd>,
    environment: Arc<Vec<(OsString, OsString)>>,
    adapter: SystemBackgroundProcessAdapter,
}

impl BackgroundProcessSpawner for NativeSpawner {
    fn prepare<'a>(
        &'a self,
        request: &'a BackgroundStartRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn CorePreparedProcess>, BackgroundStartError>> {
        Box::pin(async move {
            check_cancelled(&cancellation)?;
            let directory = self.resolve_cwd(request.cwd(), &cancellation)?;
            let process_request = BackgroundProcessRequest::from_directory(
                request.command().to_owned(),
                request.cwd().to_owned(),
                self.environment.as_ref().clone(),
                directory,
            )
            .map_err(|_| start_error(BackgroundStartErrorKind::Process))?;
            check_cancelled(&cancellation)?;
            let process = self
                .adapter
                .prepare(process_request)
                .map_err(|_| start_error(BackgroundStartErrorKind::Process))?;
            if cancellation.is_cancelled() {
                drop(process);
                return Err(start_error(BackgroundStartErrorKind::Cancelled));
            }
            Ok(Box::new(NativePrepared(Some(process))) as Box<dyn CorePreparedProcess>)
        })
    }
}

impl NativeSpawner {
    fn resolve_cwd(
        &self,
        cwd: &str,
        cancellation: &CancellationToken,
    ) -> Result<OwnedFd, BackgroundStartError> {
        let relative = Path::new(cwd)
            .strip_prefix(Path::new(&self.workspace))
            .map_err(|_| start_error(BackgroundStartErrorKind::InvalidRequest))?;
        let mut directory = self
            .workspace_root
            .try_clone()
            .map_err(|_| start_error(BackgroundStartErrorKind::Process))?;
        for component in relative.components() {
            check_cancelled(cancellation)?;
            let std::path::Component::Normal(component) = component else {
                return Err(start_error(BackgroundStartErrorKind::InvalidRequest));
            };
            directory = rustix::fs::openat(
                directory.as_fd(),
                component,
                OFlags::RDONLY
                    | OFlags::DIRECTORY
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC
                    | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|_| start_error(BackgroundStartErrorKind::Process))?;
        }
        validate_directory(directory.as_fd())
            .map_err(|()| start_error(BackgroundStartErrorKind::Process))?;
        Ok(directory)
    }
}

struct NativePrepared(Option<PreparedBackgroundProcess>);

impl CorePreparedProcess for NativePrepared {
    fn pid(&self) -> Option<std::num::NonZeroU32> {
        self.0.as_ref().map(PreparedBackgroundProcess::pid)
    }

    fn release(mut self: Box<Self>) -> Result<Box<dyn CoreOwnedProcess>, BackgroundStartError> {
        let prepared = self
            .0
            .take()
            .ok_or_else(|| start_error(BackgroundStartErrorKind::Process))?;
        let owned = prepared
            .release()
            .map_err(|_| start_error(BackgroundStartErrorKind::Process))?;
        Ok(Box::new(NativeOwned(Some(owned))))
    }
}

struct NativeOwned(Option<OwnedBackgroundProcess>);

impl CoreOwnedProcess for NativeOwned {
    fn pid(&self) -> Option<std::num::NonZeroU32> {
        self.0.as_ref().map(OwnedBackgroundProcess::pid)
    }

    fn wait(
        mut self: Box<Self>,
        stop: CancellationToken,
    ) -> BoxFuture<'static, Result<CoreProcessOutcome, BackgroundStartError>> {
        let process = self.0.take();
        Box::pin(async move {
            let process = process.ok_or_else(|| start_error(BackgroundStartErrorKind::Process))?;
            match process.wait_with_stop(&stop) {
                Ok(BackgroundProcessOutcome::Stopped) => Ok(CoreProcessOutcome::Stopped(None)),
                Ok(BackgroundProcessOutcome::Completed(BackgroundProcessExit::Exited(code))) => {
                    Ok(CoreProcessOutcome::Exited(code))
                }
                Ok(BackgroundProcessOutcome::Completed(BackgroundProcessExit::Signaled(
                    signal,
                ))) => Ok(CoreProcessOutcome::Exited(128_i32.saturating_add(signal))),
                Err(_) => Ok(CoreProcessOutcome::Dead),
            }
        })
    }
}

struct RetainedJob {
    lease: Box<dyn BackgroundRecordLease>,
    record: BackgroundRunningRecord,
    process: Box<dyn CoreOwnedProcess>,
}

enum WorkerMessage {
    Run(RetainedJob),
    Shutdown,
}

struct WorkerRetainer {
    pool: Arc<WorkerPool>,
}

struct WorkerPool {
    senders: Vec<SyncSender<WorkerMessage>>,
    available: Arc<Mutex<Vec<usize>>>,
    stops: Arc<Vec<Mutex<Option<CancellationToken>>>>,
    closing: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl WorkerRetainer {
    fn new(size: usize) -> Result<Self, NativeBackgroundSupervisorError> {
        let available = Arc::new(Mutex::new((0..size).rev().collect()));
        let stops = Arc::new((0..size).map(|_| Mutex::new(None)).collect::<Vec<_>>());
        let closing = Arc::new(AtomicBool::new(false));
        let mut senders: Vec<SyncSender<WorkerMessage>> = Vec::with_capacity(size);
        let mut workers: Vec<JoinHandle<()>> = Vec::with_capacity(size);
        for index in 0..size {
            let (sender, receiver) = sync_channel(1);
            let worker_available = Arc::clone(&available);
            let worker_stops = Arc::clone(&stops);
            let worker_closing = Arc::clone(&closing);
            let Ok(handle) = thread::Builder::new()
                .name(format!("machine-god-bg-{index}"))
                .spawn(move || {
                    worker_loop(
                        index,
                        &receiver,
                        &worker_available,
                        &worker_stops,
                        &worker_closing,
                    );
                })
            else {
                closing.store(true, Ordering::Release);
                for sender in &senders {
                    let _ = sender.try_send(WorkerMessage::Shutdown);
                }
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(NativeBackgroundSupervisorError::new(
                    NativeBackgroundSupervisorErrorKind::Worker,
                ));
            };
            senders.push(sender);
            workers.push(handle);
        }
        Ok(Self {
            pool: Arc::new(WorkerPool {
                senders,
                available,
                stops,
                closing,
                workers: Mutex::new(workers),
            }),
        })
    }

    fn shutdown(&self) {
        self.pool.shutdown();
    }
}

impl WorkerPool {
    fn dispatch(&self, index: usize, job: RetainedJob) {
        if self.closing.load(Ordering::Acquire) {
            finish_without_worker(job);
            return;
        }
        match self.senders[index].try_send(WorkerMessage::Run(job)) {
            Ok(()) => {}
            Err(TrySendError::Full(message) | TrySendError::Disconnected(message)) => {
                if let WorkerMessage::Run(job) = message {
                    finish_without_worker(job);
                }
            }
        }
    }

    fn shutdown(&self) {
        if self.closing.swap(true, Ordering::AcqRel) {
            return;
        }
        for stop in self.stops.iter() {
            if let Ok(guard) = stop.lock()
                && let Some(token) = guard.as_ref()
            {
                token.cancel();
            }
        }
        for sender in &self.senders {
            let _ = sender.try_send(WorkerMessage::Shutdown);
        }
        if let Ok(mut workers) = self.workers.lock() {
            for worker in workers.drain(..) {
                let _ = worker.join();
            }
        }
    }
}

impl BackgroundProcessRetainer for WorkerRetainer {
    fn try_admit(&self) -> Result<Box<dyn BackgroundRetentionPermit>, BackgroundStartError> {
        if self.pool.closing.load(Ordering::Acquire) {
            return Err(start_error(BackgroundStartErrorKind::Capacity));
        }
        let index = self
            .pool
            .available
            .try_lock()
            .map_err(|_| start_error(BackgroundStartErrorKind::Capacity))?
            .pop()
            .ok_or_else(|| start_error(BackgroundStartErrorKind::Capacity))?;
        if self.pool.closing.load(Ordering::Acquire) {
            try_return_slot(&self.pool.available, index);
            return Err(start_error(BackgroundStartErrorKind::Capacity));
        }
        Ok(Box::new(WorkerPermit {
            index: Some(index),
            pool: Arc::clone(&self.pool),
        }))
    }
}

struct WorkerPermit {
    index: Option<usize>,
    pool: Arc<WorkerPool>,
}

impl BackgroundRetentionPermit for WorkerPermit {
    fn retain(
        mut self: Box<Self>,
        lease: Box<dyn BackgroundRecordLease>,
        record: BackgroundRunningRecord,
        process: Box<dyn CoreOwnedProcess>,
    ) {
        let Some(index) = self.index.take() else {
            finish_without_worker(RetainedJob {
                lease,
                record,
                process,
            });
            return;
        };
        self.pool.dispatch(
            index,
            RetainedJob {
                lease,
                record,
                process,
            },
        );
    }
}

impl Drop for WorkerPermit {
    fn drop(&mut self) {
        if let Some(index) = self.index.take() {
            return_slot(&self.pool.available, index);
        }
    }
}

fn worker_loop(
    index: usize,
    receiver: &Receiver<WorkerMessage>,
    available: &Mutex<Vec<usize>>,
    stops: &[Mutex<Option<CancellationToken>>],
    closing: &AtomicBool,
) {
    while let Ok(message) = receiver.recv() {
        match message {
            WorkerMessage::Run(job) => {
                let stop = CancellationToken::new();
                if let Ok(mut slot) = stops[index].lock() {
                    *slot = Some(stop.clone());
                }
                if closing.load(Ordering::Acquire) {
                    stop.cancel();
                }
                finish_job(job, stop);
                if let Ok(mut slot) = stops[index].lock() {
                    *slot = None;
                }
                if closing.load(Ordering::Acquire) {
                    break;
                }
                return_slot(available, index);
            }
            WorkerMessage::Shutdown => break,
        }
    }
}

fn finish_job(job: RetainedJob, stop: CancellationToken) {
    let RetainedJob {
        lease,
        record,
        process,
    } = job;
    let outcome =
        futures_executor::block_on(process.wait(stop)).unwrap_or(CoreProcessOutcome::Dead);
    publish_terminal(lease.as_ref(), &record, outcome);
}

fn finish_without_worker(job: RetainedJob) {
    let RetainedJob {
        lease,
        record,
        process,
    } = job;
    drop(process);
    publish_terminal(lease.as_ref(), &record, CoreProcessOutcome::Dead);
}

fn publish_terminal(
    lease: &dyn BackgroundRecordLease,
    record: &BackgroundRunningRecord,
    outcome: CoreProcessOutcome,
) {
    let updated_at_ms = system_millis()
        .unwrap_or(record.started_at_ms())
        .max(record.started_at_ms());
    if let Ok(completion) = record.completion(updated_at_ms, outcome) {
        let _ = futures_executor::block_on(lease.publish_completion(record, &completion));
    }
}

fn return_slot(available: &Mutex<Vec<usize>>, index: usize) {
    if let Ok(mut slots) = available.lock() {
        slots.push(index);
    }
}

fn try_return_slot(available: &Mutex<Vec<usize>>, index: usize) {
    if let Ok(mut slots) = available.try_lock() {
        slots.push(index);
    }
}

fn system_millis() -> Option<u64> {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis(),
    )
    .ok()
}

#[cfg(target_os = "linux")]
fn system_process_adapter() -> SystemBackgroundProcessAdapter {
    SystemBackgroundProcessAdapter::default()
}

#[cfg(target_os = "macos")]
fn system_process_adapter()
-> Result<SystemBackgroundProcessAdapter, NativeBackgroundSupervisorError> {
    let executable = env::current_exe().map_err(|_| {
        NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Process)
    })?;
    let helper = BackgroundProcessHelper::new(
        executable,
        vec![OsString::from(BACKGROUND_PROCESS_HELPER_ARGUMENT)],
    )
    .map_err(|_| {
        NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Process)
    })?;
    Ok(SystemBackgroundProcessAdapter::with_helper(helper))
}

fn validate_environment(
    workspace: &str,
    workspace_root: &OwnedFd,
    environment: &[(OsString, OsString)],
) -> Result<(), NativeBackgroundSupervisorError> {
    let directory = workspace_root.try_clone().map_err(|_| {
        NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Workspace)
    })?;
    BackgroundProcessRequest::from_directory(
        ":".to_owned(),
        workspace.to_owned(),
        environment.to_vec(),
        directory,
    )
    .map(drop)
    .map_err(|_| {
        NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Environment)
    })
}

fn open_directory(path: &Path) -> Result<OwnedFd, ()> {
    let descriptor = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| ())?;
    validate_directory(descriptor.as_fd())?;
    Ok(descriptor)
}

fn retain_canonical_directory(path: &Path) -> Result<(String, OwnedFd), ()> {
    retain_canonical_directory_with(path, |_| {})
}

fn retain_canonical_directory_with(
    path: &Path,
    before_open: impl FnOnce(&Path),
) -> Result<(String, OwnedFd), ()> {
    let canonical = std::fs::canonicalize(path).map_err(|_| ())?;
    let workspace = canonical.to_str().ok_or(())?.to_owned();
    let expected = rustix::fs::stat(&canonical).map_err(|_| ())?;
    if expected.st_nlink == 0 || !FileType::from_raw_mode(expected.st_mode).is_dir() {
        return Err(());
    }
    before_open(&canonical);
    let descriptor = open_directory(&canonical)?;
    let retained = rustix::fs::fstat(descriptor.as_fd()).map_err(|_| ())?;
    if expected.st_dev != retained.st_dev || expected.st_ino != retained.st_ino {
        return Err(());
    }
    Ok((workspace, descriptor))
}

fn validate_directory(directory: rustix::fd::BorrowedFd<'_>) -> Result<(), ()> {
    let metadata = rustix::fs::fstat(directory).map_err(|_| ())?;
    if metadata.st_nlink == 0 || !FileType::from_raw_mode(metadata.st_mode).is_dir() {
        return Err(());
    }
    Ok(())
}

fn canonical_absolute_path(path: &str) -> bool {
    path == "/"
        || (path.starts_with('/')
            && !path.ends_with('/')
            && !path.contains('\0')
            && path
                .split('/')
                .skip(1)
                .all(|part| !part.is_empty() && part != "." && part != ".."))
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), BackgroundStartError> {
    if cancellation.is_cancelled() {
        Err(start_error(BackgroundStartErrorKind::Cancelled))
    } else {
        Ok(())
    }
}

const fn start_error(kind: BackgroundStartErrorKind) -> BackgroundStartError {
    BackgroundStartError::new(kind)
}

#[cfg(test)]
mod tests {
    use super::{
        NativeBackgroundLimits, NativeBackgroundSupervisor, SystemBackgroundProcessAdapter,
        open_directory, retain_canonical_directory_with,
    };
    #[cfg(target_os = "macos")]
    use crate::background_process::{BackgroundProcessHelper, run_background_process_helper};
    use crate::{
        NativeBackgroundInspection, NativeBackgroundQuery, NativeBackgroundState,
        NativeEnvironment, inspect_native_background,
    };
    use machine_god_core::{BackgroundStartErrorKind, BackgroundStartRequest, CancellationToken};
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    #[cfg(target_os = "macos")]
    const HELPER_TEST_ENVIRONMENT: &str = "MACHINE_GOD_BACKGROUND_HELPER_TEST";

    struct Fixture {
        root: PathBuf,
        state_base: PathBuf,
        state_root: PathBuf,
        workspace_path: PathBuf,
        workspace: String,
    }

    impl Fixture {
        fn new() -> Self {
            loop {
                let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
                let root = std::env::temp_dir().join(format!(
                    "machine-god-background-supervisor-{}-{suffix}",
                    std::process::id()
                ));
                match fs::create_dir(&root) {
                    Ok(()) => {
                        private_directory(&root);
                        let root = fs::canonicalize(root).expect("canonical fixture");
                        let state_base = root.join("state");
                        let state_root = state_base.join(crate::STATE_NAMESPACE);
                        let workspace_path = root.join("workspace");
                        for directory in [&state_base, &state_root, &workspace_path] {
                            fs::create_dir(directory).expect("create fixture directory");
                            private_directory(directory);
                        }
                        let workspace_path =
                            fs::canonicalize(workspace_path).expect("canonical workspace");
                        let workspace = workspace_path
                            .to_str()
                            .expect("Unicode workspace")
                            .to_owned();
                        return Self {
                            root,
                            state_base,
                            state_root,
                            workspace_path,
                            workspace,
                        };
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create fixture: {error}"),
                }
            }
        }

        fn supervisor(&self, limit: usize) -> NativeBackgroundSupervisor {
            let workspace_root =
                open_directory(&self.workspace_path).expect("workspace descriptor");
            let state_root = open_directory(&self.state_root).expect("state descriptor");
            NativeBackgroundSupervisor::from_parts(
                self.workspace.clone(),
                workspace_root,
                state_root,
                test_environment(),
                NativeBackgroundLimits::new(limit).expect("valid limit"),
                test_adapter(),
            )
            .expect("supervisor")
        }

        fn detail(&self, id: u64) -> crate::NativeBackgroundDetail {
            match futures_executor::block_on(inspect_native_background(
                NativeEnvironment::new(None, Some(self.state_base.clone().into_os_string()), None),
                self.workspace_path.clone(),
                NativeBackgroundQuery::Id(id),
            ))
            .expect("inspect background")
            {
                NativeBackgroundInspection::Detail(detail) => detail,
                NativeBackgroundInspection::List(_) => panic!("expected detail"),
            }
        }

        fn await_terminal(&self, id: u64) -> crate::NativeBackgroundDetail {
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                let detail = self.detail(id);
                if detail.state() != NativeBackgroundState::Running {
                    return detail;
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for completion"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn private_directory(path: &Path) {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .expect("private directory mode");
    }

    #[cfg(target_os = "linux")]
    fn test_adapter() -> SystemBackgroundProcessAdapter {
        SystemBackgroundProcessAdapter::default()
    }

    #[cfg(target_os = "macos")]
    fn test_adapter() -> SystemBackgroundProcessAdapter {
        let helper = BackgroundProcessHelper::new(
            std::env::current_exe().expect("test executable"),
            vec![
                OsString::from("--exact"),
                OsString::from("background_supervisor::tests::macos_helper_process_entry"),
                OsString::from("--nocapture"),
            ],
        )
        .expect("test helper");
        SystemBackgroundProcessAdapter::with_helper(helper)
    }

    #[cfg(target_os = "linux")]
    fn test_environment() -> Vec<(OsString, OsString)> {
        Vec::new()
    }

    #[cfg(target_os = "macos")]
    fn test_environment() -> Vec<(OsString, OsString)> {
        vec![(OsString::from(HELPER_TEST_ENVIRONMENT), OsString::from("1"))]
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_helper_process_entry() {
        if std::env::var_os(HELPER_TEST_ENVIRONMENT).is_some() {
            run_background_process_helper().expect("helper exec");
        }
    }

    #[test]
    fn production_composition_publishes_then_completes_a_real_process() {
        let fixture = Fixture::new();
        let supervisor = fixture.supervisor(2);
        let marker = fixture.workspace_path.join("completed");
        let request = BackgroundStartRequest::new("printf ready > completed", &fixture.workspace)
            .expect("request");
        let handle =
            futures_executor::block_on(supervisor.start(request, CancellationToken::new()))
                .expect("start");

        let detail = fixture.await_terminal(handle.id());
        assert_eq!(detail.state(), NativeBackgroundState::Exited);
        assert_eq!(detail.exit_code(), Some(0));
        assert_eq!(detail.cwd(), fixture.workspace);
        assert_eq!(fs::read_to_string(marker).expect("marker"), "ready");
    }

    #[test]
    fn production_workspace_open_rejects_identity_replacement() {
        let fixture = Fixture::new();
        let moved = fixture.root.join("workspace-before-replacement");
        let replacement = fixture.workspace_path.clone();
        let retained = retain_canonical_directory_with(&fixture.workspace_path, |_| {
            fs::rename(&replacement, &moved).expect("move original workspace");
            fs::create_dir(&replacement).expect("create replacement workspace");
            private_directory(&replacement);
        });

        assert!(retained.is_err(), "replacement identity must fail closed");
    }

    #[test]
    fn capacity_is_fail_fast_and_drop_stops_the_exact_owned_process() {
        let fixture = Fixture::new();
        let supervisor = fixture.supervisor(1);
        let request = BackgroundStartRequest::new(
            "trap '' TERM; while :; do /bin/sleep 1; done",
            &fixture.workspace,
        )
        .expect("request");
        let first =
            futures_executor::block_on(supervisor.start(request.clone(), CancellationToken::new()))
                .expect("first start");
        let pid = first.pid().expect("process PID").get();
        let second =
            futures_executor::block_on(supervisor.start(request, CancellationToken::new()))
                .expect_err("capacity must reject");
        assert_eq!(second.kind(), BackgroundStartErrorKind::Capacity);

        drop(supervisor);
        assert!(!process_exists(pid));
        assert_eq!(
            fixture.await_terminal(first.id()).state(),
            NativeBackgroundState::Stopped
        );
    }

    fn process_exists(pid: u32) -> bool {
        let Some(pid) = i32::try_from(pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
        else {
            return false;
        };
        rustix::process::test_kill_process(pid).is_ok()
    }
}
