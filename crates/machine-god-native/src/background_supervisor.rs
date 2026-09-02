//! Host-owned composition of background persistence and process supervision.

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::os::unix::ffi::OsStrExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use machine_god_core::{
    BackgroundClock, BackgroundCompletionRecord, BackgroundProcessOutcome as CoreProcessOutcome,
    BackgroundProcessRetainer, BackgroundProcessSpawner, BackgroundRecordLease,
    BackgroundRetentionPermit, BackgroundRunningRecord, BackgroundStartError,
    BackgroundStartErrorKind, BackgroundStartRequest, BackgroundStore as CoreBackgroundStore,
    BackgroundSupervisor, BoxFuture, CancellationToken, Cancelled,
    OwnedBackgroundProcess as CoreOwnedProcess, PreparedBackgroundProcess as CorePreparedProcess,
};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{FileType, Mode, OFlags};

use crate::background_inspection::{NativeBackgroundState, StoredBackgroundRecord};
use crate::background_process::BackgroundProcessHelper;
use crate::background_process::{
    BackgroundProcessErrorKind, BackgroundProcessExit, BackgroundProcessOutcome,
    BackgroundProcessRequest, MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES,
    MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES, MAX_BACKGROUND_PROCESS_ENVIRONMENT_KEY_BYTES,
    MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES, OwnedBackgroundProcess,
    PreparedBackgroundProcess, SystemBackgroundProcessAdapter,
};
use crate::background_store::{
    BackgroundReconciliation, BackgroundRecordLease as NativeRecordAuthority, BackgroundStore,
};
use crate::session_store::FileSessionStore;

/// Default number of concurrently retained background processes.
pub const NATIVE_BACKGROUND_DEFAULT_MAX_ACTIVE: usize = 4;
/// Hard upper bound for concurrently retained background processes.
pub const NATIVE_BACKGROUND_HARD_MAX_ACTIVE: usize = 16;
/// Exact private CLI argument used by the Linux/macOS safe launch helper.
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
    blocking: BlockingExecutor,
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
        let environment = collect_bounded_environment(env::vars_os()).map_err(|()| {
            NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Environment)
        })?;
        Self::from_root_descriptors(workspace, workspace_root, state_root, environment, limits)
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
        let environment = accept_bounded_environment(environment).map_err(|()| {
            NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Environment)
        })?;

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
        let blocking = BlockingExecutor::new(limits.max_active())?;
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
            blocking,
        })
    }

    /// Returns an inert start future. Effects begin only when it is polled.
    #[must_use]
    pub fn start(
        &self,
        request: BackgroundStartRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<machine_god_core::BackgroundHandle, BackgroundStartError>> {
        let operation_cancellation = CancellationToken::new();
        let caller_cancellation = cancellation.cancelled();
        drop(cancellation);
        let start = self
            .supervisor
            .start(request, operation_cancellation.clone());
        let blocking = self.blocking.run_cancellable(
            move || {
                catch_unwind(AssertUnwindSafe(|| futures_executor::block_on(start)))
                    .unwrap_or_else(|_| Err(start_error(BackgroundStartErrorKind::Process)))
            },
            caller_cancellation,
            operation_cancellation,
        );
        Box::pin(async move {
            match blocking.await {
                Ok(result) => result,
                Err(BlockingTaskFailure::Admission) => {
                    Err(start_error(BackgroundStartErrorKind::Capacity))
                }
                Err(BlockingTaskFailure::CancelledBeforeSubmission) => {
                    Err(start_error(BackgroundStartErrorKind::Cancelled))
                }
            }
        })
    }

    /// Reconciles persisted unlocked running records when this future is polled.
    #[must_use]
    pub fn reconcile(
        &self,
    ) -> BoxFuture<'_, Result<NativeBackgroundReconciliation, NativeBackgroundSupervisorError>>
    {
        let store = Arc::clone(&self.store.inner);
        Box::pin(self.blocking.run(
            move || {
                catch_unwind(AssertUnwindSafe(|| {
                    store.reconcile().map(Into::into).map_err(|_| {
                        NativeBackgroundSupervisorError::new(
                            NativeBackgroundSupervisorErrorKind::Reconciliation,
                        )
                    })
                }))
                .unwrap_or_else(|_| {
                    Err(NativeBackgroundSupervisorError::new(
                        NativeBackgroundSupervisorErrorKind::Worker,
                    ))
                })
            },
            Err(NativeBackgroundSupervisorError::new(
                NativeBackgroundSupervisorErrorKind::Worker,
            )),
        ))
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
        self.blocking.shutdown();
    }
}

type BlockingJob = Box<dyn FnOnce() + Send + 'static>;

const WORKER_OWNERSHIP_CAPACITY: usize = 256;

struct WorkerOwnershipRegistry {
    state: Arc<WorkerOwnershipState>,
    retained: Arc<AtomicUsize>,
    collector: Option<JoinHandle<()>>,
}

struct WorkerOwnershipState {
    handles: Mutex<Vec<OwnedWorkerHandle>>,
    wake: Condvar,
    shutdown: AtomicBool,
    #[cfg(test)]
    probes: AtomicUsize,
    #[cfg(test)]
    armed_handle_count: AtomicUsize,
}

struct WorkerOwnershipReservation {
    permits: Vec<WorkerOwnershipPermit>,
    cohort: Arc<AtomicUsize>,
}

struct WorkerOwnershipPermit {
    retained: Arc<AtomicUsize>,
    cohort: Arc<AtomicUsize>,
}

struct OwnedWorkerHandle {
    handle: JoinHandle<()>,
    completed: Arc<AtomicBool>,
    _permit: WorkerOwnershipPermit,
}

struct WorkerCompletionGuard {
    state: Arc<WorkerOwnershipState>,
    completed: Arc<AtomicBool>,
}

impl WorkerOwnershipRegistry {
    fn new() -> Result<Self, ()> {
        let state = Arc::new(WorkerOwnershipState {
            handles: Mutex::new(Vec::with_capacity(WORKER_OWNERSHIP_CAPACITY)),
            wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
            #[cfg(test)]
            probes: AtomicUsize::new(0),
            #[cfg(test)]
            armed_handle_count: AtomicUsize::new(0),
        });
        let retained = Arc::new(AtomicUsize::new(0));
        let collector_state = Arc::clone(&state);
        let collector = thread::Builder::new()
            .name("machine-god-bg-worker-collector".to_owned())
            .spawn(move || worker_collector_loop(&collector_state))
            .map_err(|_| ())?;
        Ok(Self {
            state,
            retained,
            collector: Some(collector),
        })
    }

    fn reserve(&self, count: usize) -> Result<WorkerOwnershipReservation, ()> {
        let mut retained = self.retained.load(Ordering::Acquire);
        loop {
            let next = retained.checked_add(count).ok_or(())?;
            if next > WORKER_OWNERSHIP_CAPACITY {
                return Err(());
            }
            match self.retained.compare_exchange_weak(
                retained,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(observed) => retained = observed,
            }
        }
        let cohort = Arc::new(AtomicUsize::new(count));
        let permits = (0..count)
            .map(|_| WorkerOwnershipPermit {
                retained: Arc::clone(&self.retained),
                cohort: Arc::clone(&cohort),
            })
            .collect();
        Ok(WorkerOwnershipReservation { permits, cohort })
    }

    fn register(
        &self,
        handle: JoinHandle<()>,
        completed: Arc<AtomicBool>,
        permit: WorkerOwnershipPermit,
    ) -> Result<(), OwnedWorkerHandle> {
        let owned = OwnedWorkerHandle {
            handle,
            completed,
            _permit: permit,
        };
        let mut handles = self
            .state
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.state.shutdown.load(Ordering::Acquire) || handles.len() >= WORKER_OWNERSHIP_CAPACITY
        {
            return Err(owned);
        }
        handles.push(owned);
        drop(handles);
        self.state.wake.notify_one();
        Ok(())
    }

    fn completion_guard(&self) -> (WorkerCompletionGuard, Arc<AtomicBool>) {
        let completed = Arc::new(AtomicBool::new(false));
        (
            WorkerCompletionGuard {
                state: Arc::clone(&self.state),
                completed: Arc::clone(&completed),
            },
            completed,
        )
    }
}

impl Drop for WorkerOwnershipRegistry {
    fn drop(&mut self) {
        self.state.shutdown.store(true, Ordering::Release);
        self.state.wake.notify_one();
        if let Some(collector) = self.collector.take() {
            let _ = collector.join();
        }
    }
}

impl WorkerOwnershipReservation {
    fn take(&mut self) -> WorkerOwnershipPermit {
        self.permits
            .pop()
            .expect("worker ownership was reserved before spawning")
    }
}

impl Drop for WorkerOwnershipPermit {
    fn drop(&mut self) {
        self.cohort.fetch_sub(1, Ordering::AcqRel);
        self.retained.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for WorkerCompletionGuard {
    fn drop(&mut self) {
        self.completed.store(true, Ordering::Release);
        self.state.wake.notify_one();
    }
}

fn worker_ownership_registry() -> Result<&'static WorkerOwnershipRegistry, ()> {
    static REGISTRY: OnceLock<Option<WorkerOwnershipRegistry>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| WorkerOwnershipRegistry::new().ok())
        .as_ref()
        .ok_or(())
}

fn worker_collector_loop(state: &WorkerOwnershipState) {
    loop {
        let mut handles = state
            .handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        #[cfg(test)]
        state.probes.fetch_add(1, Ordering::Relaxed);
        while !handles
            .iter()
            .any(|worker| worker.completed.load(Ordering::Acquire))
        {
            if state.shutdown.load(Ordering::Acquire) && handles.is_empty() {
                return;
            }
            #[cfg(test)]
            state
                .armed_handle_count
                .store(handles.len().saturating_add(1), Ordering::Release);
            handles = state
                .wake
                .wait(handles)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            #[cfg(test)]
            state.armed_handle_count.store(0, Ordering::Release);
            #[cfg(test)]
            state.probes.fetch_add(1, Ordering::Relaxed);
        }
        let completed = handles
            .iter()
            .position(|worker| worker.completed.load(Ordering::Acquire))
            .expect("completion notification identifies one retained worker");
        let worker = handles.swap_remove(completed);
        drop(handles);
        let _ = worker.handle.join();
    }
}

enum BlockingMessage {
    Run(BlockingJob),
    Shutdown,
}

struct BlockingExecutor {
    pool: Arc<BlockingPool>,
}

struct BlockingPool {
    senders: Vec<SyncSender<BlockingMessage>>,
    admissions: Vec<Mutex<BlockingWorkerAdmission>>,
    available: Arc<Mutex<Vec<usize>>>,
    cancellations: Arc<Vec<Mutex<Option<CancellationToken>>>>,
    closing: Arc<AtomicBool>,
    worker_cohort: Arc<AtomicUsize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BlockingWorkerAdmission {
    Open,
    Closed,
}

struct BlockingResult<T> {
    value: Option<T>,
    waker: Option<Waker>,
}

struct BlockingTaskFuture<T> {
    pool: Arc<BlockingPool>,
    task: Option<Box<dyn FnOnce() -> T + Send + 'static>>,
    admission_failure: Option<T>,
    pre_submission_cancellation: Option<T>,
    result: Arc<Mutex<BlockingResult<T>>>,
    submitted: bool,
    caller_cancellation: Option<Cancelled>,
    operation_cancellation: Option<CancellationToken>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BlockingTaskFailure {
    Admission,
    CancelledBeforeSubmission,
}

impl BlockingExecutor {
    fn new(size: usize) -> Result<Self, NativeBackgroundSupervisorError> {
        let registry = worker_ownership_registry().map_err(|()| {
            NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Worker)
        })?;
        let mut ownership = registry.reserve(size).map_err(|()| {
            NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Worker)
        })?;
        let available = Arc::new(Mutex::new((0..size).rev().collect()));
        let cancellations = Arc::new(
            (0..size)
                .map(|_| Mutex::new(None))
                .collect::<Vec<Mutex<Option<CancellationToken>>>>(),
        );
        let closing = Arc::new(AtomicBool::new(false));
        let mut senders: Vec<SyncSender<BlockingMessage>> = Vec::with_capacity(size);
        let mut admissions = Vec::with_capacity(size);
        for index in 0..size {
            let (sender, receiver) = sync_channel(1);
            let worker_available = Arc::clone(&available);
            let worker_cancellations = Arc::clone(&cancellations);
            let worker_closing = Arc::clone(&closing);
            let (completion_guard, completed) = registry.completion_guard();
            let Ok(handle) = thread::Builder::new()
                .name(format!("machine-god-bg-blocking-{index}"))
                .spawn(move || {
                    let _completion_guard = completion_guard;
                    blocking_worker_loop(
                        index,
                        &receiver,
                        &worker_available,
                        &worker_cancellations,
                        &worker_closing,
                    );
                })
            else {
                closing.store(true, Ordering::Release);
                for sender in &senders {
                    let _ = sender.try_send(BlockingMessage::Shutdown);
                }
                return Err(NativeBackgroundSupervisorError::new(
                    NativeBackgroundSupervisorErrorKind::Worker,
                ));
            };
            senders.push(sender);
            admissions.push(Mutex::new(BlockingWorkerAdmission::Open));
            if let Err(owned) = registry.register(handle, completed, ownership.take()) {
                closing.store(true, Ordering::Release);
                for sender in &senders {
                    let _ = sender.try_send(BlockingMessage::Shutdown);
                }
                let _ = owned.handle.join();
                return Err(NativeBackgroundSupervisorError::new(
                    NativeBackgroundSupervisorErrorKind::Worker,
                ));
            }
        }
        Ok(Self {
            pool: Arc::new(BlockingPool {
                senders,
                admissions,
                available,
                cancellations,
                closing,
                worker_cohort: ownership.cohort,
            }),
        })
    }

    fn run<T>(
        &self,
        task: impl FnOnce() -> T + Send + 'static,
        admission_failure: T,
    ) -> BlockingTaskFuture<T>
    where
        T: Send + Unpin + 'static,
    {
        BlockingTaskFuture {
            pool: Arc::clone(&self.pool),
            task: Some(Box::new(task)),
            admission_failure: Some(admission_failure),
            pre_submission_cancellation: None,
            result: Arc::new(Mutex::new(BlockingResult {
                value: None,
                waker: None,
            })),
            submitted: false,
            caller_cancellation: None,
            operation_cancellation: None,
        }
    }

    fn run_cancellable<T>(
        &self,
        task: impl FnOnce() -> T + Send + 'static,
        caller_cancellation: Cancelled,
        operation_cancellation: CancellationToken,
    ) -> BlockingTaskFuture<Result<T, BlockingTaskFailure>>
    where
        T: Send + Unpin + 'static,
    {
        BlockingTaskFuture {
            pool: Arc::clone(&self.pool),
            task: Some(Box::new(move || Ok(task()))),
            admission_failure: Some(Err(BlockingTaskFailure::Admission)),
            pre_submission_cancellation: Some(Err(BlockingTaskFailure::CancelledBeforeSubmission)),
            result: Arc::new(Mutex::new(BlockingResult {
                value: None,
                waker: None,
            })),
            submitted: false,
            caller_cancellation: Some(caller_cancellation),
            operation_cancellation: Some(operation_cancellation),
        }
    }

    fn shutdown(&self) {
        self.pool.shutdown();
    }
}

impl Drop for BlockingExecutor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl BlockingPool {
    fn try_submit(
        &self,
        job: BlockingJob,
        cancellation: Option<CancellationToken>,
    ) -> Result<(), ()> {
        self.try_submit_after_reservation(job, cancellation, || {})
    }

    fn try_submit_after_reservation(
        &self,
        job: BlockingJob,
        cancellation: Option<CancellationToken>,
        after_reservation: impl FnOnce(),
    ) -> Result<(), ()> {
        if self.closing.load(Ordering::Acquire) {
            return Err(());
        }
        let mut available = self.available.try_lock().map_err(|_| ())?;
        let index = available.pop().ok_or(())?;
        {
            let Ok(mut registered) = self.cancellations[index].try_lock() else {
                available.push(index);
                return Err(());
            };
            debug_assert!(registered.is_none());
            *registered = cancellation;
        }
        after_reservation();
        let Ok(admission) = self.admissions[index].try_lock() else {
            if let Some(cancellation) = self.cancellations[index]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                cancellation.cancel();
            }
            available.push(index);
            return Err(());
        };
        if self.closing.load(Ordering::Acquire) || *admission == BlockingWorkerAdmission::Closed {
            if let Some(cancellation) = self.cancellations[index]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                cancellation.cancel();
            }
            available.push(index);
            return Err(());
        }
        match self.senders[index].try_send(BlockingMessage::Run(job)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.cancellations[index]
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                available.push(index);
                Err(())
            }
        }
    }

    fn shutdown(&self) {
        debug_assert!(self.worker_cohort.load(Ordering::Acquire) <= self.senders.len());
        if self.closing.swap(true, Ordering::AcqRel) {
            return;
        }
        for (index, registered) in self.cancellations.iter().enumerate() {
            let mut admission = self.admissions[index]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *admission = BlockingWorkerAdmission::Closed;
            if let Some(cancellation) = registered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
            {
                cancellation.cancel();
            }
            let _ = self.senders[index].try_send(BlockingMessage::Shutdown);
        }
    }
}

impl<T> Future for BlockingTaskFuture<T>
where
    T: Send + Unpin + 'static,
{
    type Output = T;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let caller_cancelled = this
            .caller_cancellation
            .as_mut()
            .is_some_and(|cancelled| Pin::new(cancelled).poll(context).is_ready());
        if caller_cancelled {
            if !this.submitted {
                this.task = None;
                this.caller_cancellation = None;
                this.operation_cancellation = None;
                return Poll::Ready(
                    this.pre_submission_cancellation
                        .take()
                        .expect("pre-submission cancellation completes once"),
                );
            }
            if let Some(cancellation) = &this.operation_cancellation {
                cancellation.cancel();
            }
            this.caller_cancellation = None;
            this.pre_submission_cancellation = None;
        }
        if !this.submitted {
            let incoming = context.waker().clone();
            let superseded = {
                let mut result = this
                    .result
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                result.waker.replace(incoming)
            };
            drop(superseded);
            let task = this.task.take().expect("blocking task is submitted once");
            let result = Arc::clone(&this.result);
            let job = Box::new(move || {
                let value = task();
                let wake = {
                    let mut result = result
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    result.value = Some(value);
                    result.waker.take()
                };
                if let Some(waker) = wake {
                    waker.wake();
                }
            });
            if this
                .pool
                .try_submit(job, this.operation_cancellation.clone())
                .is_err()
            {
                this.caller_cancellation = None;
                this.operation_cancellation = None;
                return Poll::Ready(
                    this.admission_failure
                        .take()
                        .expect("blocking admission fails once"),
                );
            }
            this.submitted = true;
            return Poll::Pending;
        }

        let incoming = context.waker().clone();
        let (value, superseded, unused) = {
            let mut result = this
                .result
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(value) = result.value.take() {
                (Some(value), None, Some(incoming))
            } else if result
                .waker
                .as_ref()
                .is_some_and(|waker| waker.will_wake(context.waker()))
            {
                (None, None, Some(incoming))
            } else {
                (None, result.waker.replace(incoming), None)
            }
        };
        drop(superseded);
        drop(unused);
        if let Some(value) = value {
            this.caller_cancellation = None;
            this.operation_cancellation = None;
            return Poll::Ready(value);
        }
        Poll::Pending
    }
}

impl<T> Drop for BlockingTaskFuture<T> {
    fn drop(&mut self) {
        if self.submitted
            && let Some(cancellation) = self.operation_cancellation.take()
        {
            cancellation.cancel();
        }
    }
}

fn blocking_worker_loop(
    index: usize,
    receiver: &Receiver<BlockingMessage>,
    available: &Mutex<Vec<usize>>,
    cancellations: &[Mutex<Option<CancellationToken>>],
    closing: &AtomicBool,
) {
    while let Ok(message) = receiver.recv() {
        match message {
            BlockingMessage::Run(job) => {
                let _ = catch_unwind(AssertUnwindSafe(job));
                cancellations[index]
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if closing.load(Ordering::Acquire) {
                    break;
                }
                return_slot(available, index);
            }
            BlockingMessage::Shutdown => break,
        }
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
                .prepare_cancellable(process_request, &cancellation)
                .map_err(|error| start_error(process_error_kind(error.kind())))?;
            let process = finish_prepared_after_readiness(
                process,
                &cancellation,
                PreparedBackgroundProcess::abort_and_reap,
            )?;
            Ok(Box::new(NativePrepared(Some(process))) as Box<dyn CorePreparedProcess>)
        })
    }
}

fn finish_prepared_after_readiness<T, E>(
    prepared: T,
    cancellation: &CancellationToken,
    abort_and_reap: impl FnOnce(T) -> Result<(), E>,
) -> Result<T, BackgroundStartError> {
    if !cancellation.is_cancelled() {
        return Ok(prepared);
    }
    abort_and_reap(prepared).map_err(|_| start_error(BackgroundStartErrorKind::Process))?;
    Err(start_error(BackgroundStartErrorKind::Cancelled))
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

    fn release(
        mut self: Box<Self>,
        cancellation: &CancellationToken,
    ) -> Result<Box<dyn CoreOwnedProcess>, BackgroundStartError> {
        let prepared = self
            .0
            .take()
            .ok_or_else(|| start_error(BackgroundStartErrorKind::Process))?;
        let owned = prepared
            .release_cancellable(cancellation)
            .map_err(|error| start_error(process_error_kind(error.kind())))?;
        Ok(Box::new(NativeOwned(Some(owned))))
    }

    fn abort(mut self: Box<Self>) -> Result<(), BackgroundStartError> {
        let prepared = self
            .0
            .take()
            .ok_or_else(|| start_error(BackgroundStartErrorKind::Process))?;
        prepared
            .abort_and_reap()
            .map_err(|_| start_error(BackgroundStartErrorKind::Process))
    }
}

const fn process_error_kind(kind: BackgroundProcessErrorKind) -> BackgroundStartErrorKind {
    match kind {
        BackgroundProcessErrorKind::Cancelled => BackgroundStartErrorKind::Cancelled,
        _ => BackgroundStartErrorKind::Process,
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
    rescue: SyncSender<RetainedJob>,
    worker_cohort: Arc<AtomicUsize>,
}

impl WorkerRetainer {
    fn new(size: usize) -> Result<Self, NativeBackgroundSupervisorError> {
        let registry = worker_ownership_registry().map_err(|()| {
            NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Worker)
        })?;
        let mut ownership = registry.reserve(size.saturating_add(1)).map_err(|()| {
            NativeBackgroundSupervisorError::new(NativeBackgroundSupervisorErrorKind::Worker)
        })?;
        let available = Arc::new(Mutex::new((0..size).rev().collect()));
        let stops = Arc::new((0..size).map(|_| Mutex::new(None)).collect::<Vec<_>>());
        let closing = Arc::new(AtomicBool::new(false));
        let mut senders: Vec<SyncSender<WorkerMessage>> = Vec::with_capacity(size);
        for index in 0..size {
            let (sender, receiver) = sync_channel(1);
            let worker_available = Arc::clone(&available);
            let worker_stops = Arc::clone(&stops);
            let worker_closing = Arc::clone(&closing);
            let (completion_guard, completed) = registry.completion_guard();
            let Ok(handle) = thread::Builder::new()
                .name(format!("machine-god-bg-{index}"))
                .spawn(move || {
                    let _completion_guard = completion_guard;
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
                return Err(NativeBackgroundSupervisorError::new(
                    NativeBackgroundSupervisorErrorKind::Worker,
                ));
            };
            senders.push(sender);
            if let Err(owned) = registry.register(handle, completed, ownership.take()) {
                closing.store(true, Ordering::Release);
                let _ = owned.handle.join();
                return Err(NativeBackgroundSupervisorError::new(
                    NativeBackgroundSupervisorErrorKind::Worker,
                ));
            }
        }
        let (rescue, rescue_receiver) = sync_channel(size);
        let (completion_guard, completed) = registry.completion_guard();
        let Ok(rescue_worker) = thread::Builder::new()
            .name("machine-god-bg-rescue".to_owned())
            .spawn(move || {
                let _completion_guard = completion_guard;
                rescue_worker_loop(&rescue_receiver);
            })
        else {
            closing.store(true, Ordering::Release);
            return Err(NativeBackgroundSupervisorError::new(
                NativeBackgroundSupervisorErrorKind::Worker,
            ));
        };
        if let Err(owned) = registry.register(rescue_worker, completed, ownership.take()) {
            drop(rescue);
            let _ = owned.handle.join();
            closing.store(true, Ordering::Release);
            return Err(NativeBackgroundSupervisorError::new(
                NativeBackgroundSupervisorErrorKind::Worker,
            ));
        }
        Ok(Self {
            pool: Arc::new(WorkerPool {
                senders,
                available,
                stops,
                closing,
                rescue,
                worker_cohort: ownership.cohort,
            }),
        })
    }

    fn shutdown(&self) {
        self.pool.shutdown();
    }
}

impl WorkerPool {
    fn dispatch(&self, index: usize, job: RetainedJob) {
        match self.senders[index].try_send(WorkerMessage::Run(job)) {
            Ok(()) => {}
            Err(TrySendError::Full(message) | TrySendError::Disconnected(message)) => {
                let WorkerMessage::Run(job) = message else {
                    unreachable!("dispatch submits only retained jobs")
                };
                self.rescue
                    .try_send(job)
                    .unwrap_or_else(|_| unreachable!("bounded rescue queue retains every permit"));
            }
        }
    }

    fn shutdown(&self) {
        debug_assert!(self.worker_cohort.load(Ordering::Acquire) <= self.senders.len() + 1);
        if self.closing.swap(true, Ordering::AcqRel) {
            return;
        }
        for stop in self.stops.iter() {
            if let Ok(guard) = stop.try_lock()
                && let Some(token) = guard.as_ref()
            {
                token.cancel();
            }
        }
        if let Ok(idle) = self.available.try_lock() {
            for index in idle.iter().copied() {
                let _ = self.senders[index].try_send(WorkerMessage::Shutdown);
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
        let index = self
            .index
            .take()
            .expect("a consumed retention permit owns exactly one worker slot");
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
            if self.pool.closing.load(Ordering::Acquire) {
                let _ = self.pool.senders[index].try_send(WorkerMessage::Shutdown);
            }
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

fn rescue_worker_loop(receiver: &Receiver<RetainedJob>) {
    while let Ok(job) = receiver.recv() {
        let stop = CancellationToken::new();
        stop.cancel();
        finish_job(job, stop);
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

fn collect_bounded_environment(
    environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<Vec<(OsString, OsString)>, ()> {
    let mut bounded = Vec::new();
    let mut aggregate = 0_usize;
    for (key, value) in environment {
        if bounded.len() == MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES {
            return Err(());
        }
        validate_environment_entry(&key, &value, &mut aggregate)?;
        bounded.push((key, value));
    }
    validate_environment(&bounded)?;
    Ok(bounded)
}

fn accept_bounded_environment(
    environment: Vec<(OsString, OsString)>,
) -> Result<Vec<(OsString, OsString)>, ()> {
    validate_environment(&environment)?;
    Ok(environment)
}

fn validate_environment(environment: &[(OsString, OsString)]) -> Result<(), ()> {
    if environment.len() > MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES {
        return Err(());
    }
    let mut aggregate = 0_usize;
    let mut keys = BTreeSet::new();
    for (key, value) in environment {
        validate_environment_entry(key, value, &mut aggregate)?;
        if !keys.insert(key.as_os_str().as_bytes()) {
            return Err(());
        }
    }
    Ok(())
}

fn validate_environment_entry(
    key: &OsString,
    value: &OsString,
    aggregate: &mut usize,
) -> Result<(), ()> {
    let key = key.as_os_str().as_bytes();
    let value = value.as_os_str().as_bytes();
    if key.is_empty()
        || key.len() > MAX_BACKGROUND_PROCESS_ENVIRONMENT_KEY_BYTES
        || value.len() > MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES
        || key.contains(&b'=')
        || key.contains(&0)
        || value.contains(&0)
    {
        return Err(());
    }
    *aggregate = aggregate
        .checked_add(key.len())
        .and_then(|total| total.checked_add(value.len()))
        .filter(|total| *total <= MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES)
        .ok_or(())?;
    Ok(())
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
        BlockingExecutor, BlockingResult, BlockingTaskFailure,
        MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES, MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES,
        NativeBackgroundLimits, NativeBackgroundSupervisor, NativeSpawner, NativeStore,
        RetainedJob, SystemBackgroundProcessAdapter, SystemClock, WORKER_OWNERSHIP_CAPACITY,
        WorkerOwnershipRegistry, WorkerRetainer, accept_bounded_environment,
        collect_bounded_environment, finish_prepared_after_readiness, open_directory,
        process_error_kind, retain_canonical_directory_with, validate_environment,
        worker_ownership_registry,
    };
    use crate::background_process::{
        BackgroundProcessErrorKind, BackgroundProcessHelper, run_background_process_helper,
    };
    use crate::{
        NativeBackgroundInspection, NativeBackgroundQuery, NativeBackgroundState,
        NativeEnvironment, inspect_native_background,
    };
    use machine_god_core::{
        BackgroundProcessOutcome, BackgroundProcessRetainer, BackgroundProcessSpawner,
        BackgroundRetentionPermit, BackgroundStartError, BackgroundStartErrorKind,
        BackgroundStartRequest, BackgroundSupervisor, BoxFuture, CancellationToken,
        OwnedBackgroundProcess, PreparedBackgroundProcess,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::future::Future;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex, Weak, mpsc};
    use std::task::{Context, Poll, Wake, Waker};
    use std::thread;
    use std::time::{Duration, Instant};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct WakeFlag(AtomicBool);

    impl Wake for WakeFlag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::Release);
        }
    }

    struct ReentrantDropWake {
        result: Weak<Mutex<BlockingResult<u8>>>,
        worker_can_finish: Arc<AtomicBool>,
        entered: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl Wake for ReentrantDropWake {
        fn wake(self: Arc<Self>) {}
    }

    impl Drop for ReentrantDropWake {
        fn drop(&mut self) {
            let Some(result) = self.result.upgrade() else {
                return;
            };
            let _result = result
                .try_lock()
                .expect("superseded waker must be dropped after unlocking its result");
            self.worker_can_finish.store(true, Ordering::Release);
            self.entered.send(()).expect("report hostile drop");
            self.release
                .lock()
                .expect("lock hostile release")
                .recv_timeout(Duration::from_secs(1))
                .expect("release hostile drop");
        }
    }

    struct CapturingRetainer {
        job: Arc<Mutex<Option<RetainedJob>>>,
    }

    impl BackgroundProcessRetainer for CapturingRetainer {
        fn try_admit(&self) -> Result<Box<dyn BackgroundRetentionPermit>, BackgroundStartError> {
            Ok(Box::new(CapturingPermit {
                job: Arc::clone(&self.job),
            }))
        }
    }

    struct CapturingPermit {
        job: Arc<Mutex<Option<RetainedJob>>>,
    }

    impl BackgroundRetentionPermit for CapturingPermit {
        fn retain(
            self: Box<Self>,
            lease: Box<dyn machine_god_core::BackgroundRecordLease>,
            record: machine_god_core::BackgroundRunningRecord,
            process: Box<dyn machine_god_core::OwnedBackgroundProcess>,
        ) {
            *self.job.lock().expect("capture job") = Some(RetainedJob {
                lease,
                record,
                process,
            });
        }
    }

    struct BlockingOwnedProcess {
        inner: Box<dyn machine_god_core::OwnedBackgroundProcess>,
        resource: Arc<()>,
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    }

    struct PausedSpawner {
        entered: mpsc::SyncSender<()>,
        release: Mutex<Option<mpsc::Receiver<()>>>,
        cleaned: Arc<AtomicBool>,
        command_executed: Arc<AtomicBool>,
    }

    impl BackgroundProcessSpawner for PausedSpawner {
        fn prepare<'a>(
            &'a self,
            _request: &'a BackgroundStartRequest,
            _cancellation: CancellationToken,
        ) -> BoxFuture<'a, Result<Box<dyn PreparedBackgroundProcess>, BackgroundStartError>>
        {
            let prepared = PausedPrepared {
                entered: self.entered.clone(),
                release: self.release.lock().expect("lock release gate").take(),
                cleaned: Arc::clone(&self.cleaned),
                command_executed: Arc::clone(&self.command_executed),
            };
            Box::pin(async move { Ok(Box::new(prepared) as Box<dyn PreparedBackgroundProcess>) })
        }
    }

    struct PausedPrepared {
        entered: mpsc::SyncSender<()>,
        release: Option<mpsc::Receiver<()>>,
        cleaned: Arc<AtomicBool>,
        command_executed: Arc<AtomicBool>,
    }

    impl PreparedBackgroundProcess for PausedPrepared {
        fn pid(&self) -> Option<std::num::NonZeroU32> {
            None
        }

        fn release(
            mut self: Box<Self>,
            cancellation: &CancellationToken,
        ) -> Result<Box<dyn OwnedBackgroundProcess>, BackgroundStartError> {
            self.entered.send(()).expect("report release boundary");
            self.release
                .take()
                .expect("release gate is present")
                .recv_timeout(Duration::from_secs(2))
                .expect("open release boundary");
            if cancellation.is_cancelled() {
                self.cleaned.store(true, Ordering::Release);
                return Err(BackgroundStartError::new(
                    BackgroundStartErrorKind::Cancelled,
                ));
            }
            self.command_executed.store(true, Ordering::Release);
            Ok(Box::new(ImmediateOwned))
        }

        fn abort(self: Box<Self>) -> Result<(), BackgroundStartError> {
            self.cleaned.store(true, Ordering::Release);
            Ok(())
        }
    }

    impl Drop for PausedPrepared {
        fn drop(&mut self) {
            self.cleaned.store(true, Ordering::Release);
        }
    }

    struct ImmediateOwned;

    impl OwnedBackgroundProcess for ImmediateOwned {
        fn pid(&self) -> Option<std::num::NonZeroU32> {
            None
        }

        fn wait(
            self: Box<Self>,
            _stop: CancellationToken,
        ) -> BoxFuture<'static, Result<BackgroundProcessOutcome, BackgroundStartError>> {
            Box::pin(async { Ok(BackgroundProcessOutcome::Exited(0)) })
        }
    }

    impl machine_god_core::OwnedBackgroundProcess for BlockingOwnedProcess {
        fn pid(&self) -> Option<std::num::NonZeroU32> {
            self.inner.pid()
        }

        fn wait(
            self: Box<Self>,
            stop: CancellationToken,
        ) -> machine_god_core::BoxFuture<
            'static,
            Result<
                machine_god_core::BackgroundProcessOutcome,
                machine_god_core::BackgroundStartError,
            >,
        > {
            let Self {
                inner,
                resource,
                entered,
                release,
            } = *self;
            Box::pin(async move {
                entered.send(()).expect("report retained process ownership");
                release
                    .recv_timeout(Duration::from_secs(2))
                    .expect("release retained process ownership");
                let outcome = inner.wait(stop).await;
                drop(resource);
                outcome
            })
        }
    }

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

    fn start_eventually(
        supervisor: &NativeBackgroundSupervisor,
        request: &BackgroundStartRequest,
    ) -> machine_god_core::BackgroundHandle {
        futures_executor::block_on(supervisor.start(request.clone(), CancellationToken::new()))
            .expect("start background test process")
    }

    fn test_adapter() -> SystemBackgroundProcessAdapter {
        let helper = BackgroundProcessHelper::new(
            std::env::current_exe().expect("test executable"),
            vec![
                OsString::from("--exact"),
                OsString::from("background_supervisor::tests::helper_process_entry"),
                OsString::from("--nocapture"),
            ],
        )
        .expect("test helper");
        SystemBackgroundProcessAdapter::with_helper(helper)
    }

    fn test_environment() -> Vec<(OsString, OsString)> {
        Vec::new()
    }

    #[test]
    fn helper_process_entry() {
        if std::env::var_os("MACHINE_GOD_BACKGROUND_HELPER_MODE").is_some() {
            run_background_process_helper().expect("helper exec");
        }
    }

    #[test]
    fn post_readiness_cancellation_reports_cancelled_only_after_proven_cleanup() {
        let cancellation = CancellationToken::new();
        assert!(cancellation.cancel());
        let mut aborted = false;

        let error = finish_prepared_after_readiness(7_u8, &cancellation, |prepared| {
            assert_eq!(prepared, 7);
            aborted = true;
            Ok::<(), ()>(())
        })
        .expect_err("post-readiness cancellation must abort the prepared process");

        assert!(aborted);
        assert_eq!(error.kind(), BackgroundStartErrorKind::Cancelled);
    }

    #[test]
    fn post_readiness_cleanup_failure_outranks_cancellation() {
        let cancellation = CancellationToken::new();
        assert!(cancellation.cancel());

        let error = finish_prepared_after_readiness(7_u8, &cancellation, |_| Err::<(), ()>(()))
            .expect_err("failed cleanup must reject the start as a process failure");

        assert_eq!(error.kind(), BackgroundStartErrorKind::Process);
    }

    #[test]
    fn proven_native_process_cancellation_maps_to_cancelled_start() {
        assert_eq!(
            process_error_kind(BackgroundProcessErrorKind::Cancelled),
            BackgroundStartErrorKind::Cancelled
        );
    }

    #[test]
    fn native_spawn_and_ambiguous_cleanup_map_to_process_failure() {
        assert_eq!(
            process_error_kind(BackgroundProcessErrorKind::Spawn),
            BackgroundStartErrorKind::Process
        );
        assert_eq!(
            process_error_kind(BackgroundProcessErrorKind::Cleanup),
            BackgroundStartErrorKind::Process
        );
        assert_eq!(
            process_error_kind(BackgroundProcessErrorKind::Release),
            BackgroundStartErrorKind::Process
        );
    }

    #[test]
    fn production_composition_publishes_then_completes_a_real_process() {
        let fixture = Fixture::new();
        let supervisor = fixture.supervisor(2);
        let marker = fixture.workspace_path.join("completed");
        let request = BackgroundStartRequest::new("printf ready > completed", &fixture.workspace)
            .expect("request");
        let handle = start_eventually(&supervisor, &request);

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
        let first = start_eventually(&supervisor, &request);
        let pid = first.pid().expect("process PID").get();
        let second =
            futures_executor::block_on(supervisor.start(request, CancellationToken::new()))
                .expect_err("capacity must reject");
        assert_eq!(second.kind(), BackgroundStartErrorKind::Capacity);

        let drop_started = Instant::now();
        drop(supervisor);
        assert!(drop_started.elapsed() < Duration::from_millis(250));
        assert_eq!(
            fixture.await_terminal(first.id()).state(),
            NativeBackgroundState::Stopped
        );
        assert!(!process_exists(pid));
    }

    #[test]
    fn production_start_and_reconcile_complete_by_waking_the_polling_thread() {
        let fixture = Fixture::new();
        let supervisor = fixture.supervisor(1);
        let request = BackgroundStartRequest::new(":", &fixture.workspace).expect("request");
        let wake = Arc::new(WakeFlag(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&wake));
        let mut context = Context::from_waker(&waker);
        let mut start = supervisor.start(request, CancellationToken::new());
        let started = Instant::now();

        assert!(matches!(start.as_mut().poll(&mut context), Poll::Pending));
        assert!(started.elapsed() < Duration::from_millis(250));
        wait_for_wake(&wake);
        let handle = match start.as_mut().poll(&mut context) {
            Poll::Ready(Ok(handle)) => handle,
            other => panic!("woken start must be ready: {other:?}"),
        };
        fixture.await_terminal(handle.id());

        wake.0.store(false, Ordering::Release);
        let mut reconcile = supervisor.reconcile();
        let started = Instant::now();
        assert!(matches!(
            reconcile.as_mut().poll(&mut context),
            Poll::Pending
        ));
        assert!(started.elapsed() < Duration::from_millis(250));
        wait_for_wake(&wake);
        assert!(matches!(
            reconcile.as_mut().poll(&mut context),
            Poll::Ready(Ok(_))
        ));
    }

    #[test]
    fn pre_cancelled_start_completes_on_first_poll_without_process_execution() {
        let fixture = Fixture::new();
        let supervisor = fixture.supervisor(1);
        let marker = fixture.workspace_path.join("must-not-exist");
        let request =
            BackgroundStartRequest::new("printf executed > must-not-exist", &fixture.workspace)
                .expect("request");
        let cancellation = CancellationToken::new();
        let mut start = supervisor.start(request, cancellation.clone());
        cancellation.cancel();

        let result = start.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        assert!(matches!(
            result,
            Poll::Ready(Err(error))
                if error.kind() == BackgroundStartErrorKind::Cancelled
        ));
        assert!(!marker.exists(), "cancelled start must remain inert");
    }

    #[test]
    fn pre_submission_cancellation_wins_saturated_pool_without_execution() {
        let executor = BlockingExecutor::new(1).expect("blocking executor");
        let (occupied, occupied_receiver) = mpsc::sync_channel(1);
        let (release, release_receiver) = mpsc::sync_channel(1);
        let mut blocker = Box::pin(executor.run(
            move || {
                occupied.send(()).expect("report occupied worker");
                release_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .expect("release occupied worker");
                1_u8
            },
            0,
        ));
        assert!(matches!(
            blocker
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        occupied_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("sole worker is occupied");

        let caller_cancellation = CancellationToken::new();
        let operation_cancellation = CancellationToken::new();
        let executed = Arc::new(AtomicBool::new(false));
        let worker_executed = Arc::clone(&executed);
        let mut cancelled = Box::pin(executor.run_cancellable(
            move || {
                worker_executed.store(true, Ordering::Release);
                7_u8
            },
            caller_cancellation.cancelled(),
            operation_cancellation.clone(),
        ));
        caller_cancellation.cancel();

        assert!(matches!(
            cancelled
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(BlockingTaskFailure::CancelledBeforeSubmission))
        ));
        assert!(!executed.load(Ordering::Acquire));
        assert!(!operation_cancellation.is_cancelled());

        release.send(()).expect("release occupied worker");
        assert_eq!(futures_executor::block_on(blocker), 1);
        executor.shutdown();
        assert!(!executed.load(Ordering::Acquire));
    }

    #[test]
    fn worker_ownership_capacity_rejects_an_unreservable_cohort() {
        assert!(
            worker_ownership_registry()
                .expect("worker ownership registry")
                .reserve(WORKER_OWNERSHIP_CAPACITY + 1)
                .is_err()
        );
    }

    #[test]
    fn worker_collection_sleeps_until_an_actual_completion_notification() {
        let registry = WorkerOwnershipRegistry::new().expect("worker registry");
        let mut ownership = registry.reserve(1).expect("worker ownership");
        let cohort = Arc::clone(&ownership.cohort);
        let (completion_guard, completed) = registry.completion_guard();
        let (entered, entered_receiver) = mpsc::sync_channel(1);
        let (release, release_receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let _completion_guard = completion_guard;
            entered.send(()).expect("report idle worker");
            release_receiver.recv().expect("release idle worker");
        });
        if registry
            .register(worker, completed, ownership.take())
            .is_err()
        {
            panic!("register worker");
        }
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker entered");

        let wait_deadline = Instant::now() + Duration::from_secs(1);
        while registry.state.armed_handle_count.load(Ordering::Acquire) != 2 {
            assert!(
                Instant::now() < wait_deadline,
                "collector did not observe the registered worker before sleeping"
            );
            thread::yield_now();
        }
        let idle_probes = registry.state.probes.load(Ordering::Acquire);
        thread::sleep(Duration::from_millis(75));
        let after_idle = registry.state.probes.load(Ordering::Acquire);

        release.send(()).expect("complete worker");
        wait_for_zero(&cohort, "completed worker was not collected");
        assert!(
            registry
                .state
                .handles
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "completion notification must remove the retained handle"
        );
        drop(registry);
        assert_eq!(
            after_idle, idle_probes,
            "idle collector must not wake or rescan retained handles"
        );
    }

    #[test]
    fn shutdown_dequeue_closes_a_racing_worker_admission_before_run_send() {
        for _ in 0..128 {
            let executor = BlockingExecutor::new(1).expect("blocking executor");
            let pool = Arc::clone(&executor.pool);
            let worker_cohort = Arc::clone(&pool.worker_cohort);
            let operation_cancellation = CancellationToken::new();
            let observed_cancellation = operation_cancellation.clone();
            let executed = Arc::new(AtomicBool::new(false));
            let worker_executed = Arc::clone(&executed);
            let (reserved, reserved_receiver) = mpsc::sync_channel(1);
            let (resume, resume_receiver) = mpsc::sync_channel(1);
            let (reported, reported_receiver) = mpsc::sync_channel(1);
            let registrar = thread::spawn(move || {
                let result = pool.try_submit_after_reservation(
                    Box::new(move || worker_executed.store(true, Ordering::Release)),
                    Some(operation_cancellation),
                    || {
                        reserved.send(()).expect("report reserved worker");
                        resume_receiver
                            .recv_timeout(Duration::from_secs(1))
                            .expect("resume registrar");
                    },
                );
                reported.send(result).expect("report admission result");
            });
            reserved_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("registrar reserved idle worker");

            executor.shutdown();
            wait_for_zero(
                &worker_cohort,
                "worker did not dequeue shutdown before raced admission resumed",
            );
            assert!(observed_cancellation.is_cancelled());
            resume.send(()).expect("resume raced registrar");
            assert!(
                reported_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .expect("raced admission completes")
                    .is_err(),
                "a worker committed to exit must reject later admission"
            );
            registrar.join().expect("join raced registrar");
            assert!(!executed.load(Ordering::Acquire));
        }
    }

    #[test]
    fn bounded_environment_acceptance_moves_the_original_buffers() {
        let environment = vec![(OsString::from("KEY"), OsString::from("value"))];
        let vector = environment.as_ptr();
        let key = environment[0].0.as_os_str().as_bytes().as_ptr();
        let value = environment[0].1.as_os_str().as_bytes().as_ptr();

        let accepted = accept_bounded_environment(environment).expect("bounded environment");

        assert_eq!(accepted.as_ptr(), vector);
        assert_eq!(accepted[0].0.as_os_str().as_bytes().as_ptr(), key);
        assert_eq!(accepted[0].1.as_os_str().as_bytes().as_ptr(), value);
    }

    #[test]
    fn huge_environment_iterators_stop_at_the_first_hard_bound() {
        let entry_count = Arc::new(AtomicUsize::new(0));
        let observed_entries = Arc::clone(&entry_count);
        let too_many = std::iter::repeat_with(move || {
            let index = observed_entries.fetch_add(1, Ordering::AcqRel);
            (OsString::from(format!("K{index}")), OsString::new())
        });
        assert!(collect_bounded_environment(too_many).is_err());
        assert_eq!(
            entry_count.load(Ordering::Acquire),
            MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES + 1
        );

        let aggregate_count = Arc::new(AtomicUsize::new(0));
        let observed_aggregate = Arc::clone(&aggregate_count);
        let too_large = std::iter::repeat_with(move || {
            let index = observed_aggregate.fetch_add(1, Ordering::AcqRel);
            (
                OsString::from(format!("V{index}")),
                OsString::from("x".repeat(MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES)),
            )
        });
        assert!(collect_bounded_environment(too_large).is_err());
        assert!(
            aggregate_count.load(Ordering::Acquire) <= 17,
            "aggregate rejection must not consume or retain the unbounded tail"
        );
    }

    #[test]
    fn borrowed_environment_validation_rejects_huge_entry_counts_immediately() {
        let environment = vec![
            (OsString::from("K"), OsString::new());
            MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES + 1
        ];
        let vector = environment.as_ptr();
        let mut result = None;
        allocation_counter::measure(|| {});
        let allocations = allocation_counter::measure(|| {
            result = Some(validate_environment(&environment));
        });
        assert_eq!(result, Some(Err(())));
        assert_eq!(allocations.count_total, 0, "{allocations:?}");
        assert_eq!(allocations.bytes_total, 0, "{allocations:?}");
        assert_eq!(environment.as_ptr(), vector);
    }

    #[test]
    fn aggregate_environment_rejection_never_clones_large_values() {
        let environment = (0..17)
            .map(|index| {
                (
                    OsString::from(format!("V{index}")),
                    OsString::from("x".repeat(MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES)),
                )
            })
            .collect::<Vec<_>>();
        let mut result = None;
        allocation_counter::measure(|| {});
        let allocations = allocation_counter::measure(|| {
            result = Some(validate_environment(&environment));
        });

        assert_eq!(result, Some(Err(())));
        assert!(
            allocations.bytes_total
                < u64::try_from(MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES)
                    .expect("environment value bound fits u64"),
            "aggregate validation cloned an invalid value: {allocations:?}"
        );
    }

    #[test]
    fn dropped_blocking_future_leaves_cancelled_cleanup_owned_by_worker() {
        let executor = BlockingExecutor::new(1).expect("blocking executor");
        let caller_cancellation = CancellationToken::new();
        let operation_cancellation = CancellationToken::new();
        let worker_cancellation = operation_cancellation.clone();
        let (finished, finished_receiver) = mpsc::sync_channel(1);
        let mut task = Box::pin(executor.run_cancellable(
            move || {
                futures_executor::block_on(worker_cancellation.cancelled());
                finished.send(()).expect("report cleanup");
                Ok::<(), ()>(())
            },
            caller_cancellation.cancelled(),
            operation_cancellation,
        ));

        assert!(matches!(
            task.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        drop(task);
        finished_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker retained cleanup ownership");
        assert!(!caller_cancellation.is_cancelled());
    }

    #[test]
    fn blocking_shutdown_transfers_a_stuck_worker_to_bounded_ownership() {
        let executor = BlockingExecutor::new(1).expect("blocking executor");
        let worker_cohort = Arc::clone(&executor.pool.worker_cohort);
        let owned = Arc::new(());
        let weak_owned = Arc::downgrade(&owned);
        let (entered, entered_receiver) = mpsc::sync_channel(1);
        let (release, release_receiver) = mpsc::sync_channel(1);
        let mut task = Box::pin(executor.run(
            move || {
                entered.send(()).expect("report stuck worker");
                release_receiver.recv().expect("release stuck worker");
                owned
            },
            Arc::new(()),
        ));

        assert!(matches!(
            task.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker owns the submitted job");

        let (shutdown_done, shutdown_receiver) = mpsc::sync_channel(1);
        let shutdown_thread = thread::spawn(move || {
            let started = Instant::now();
            executor.shutdown();
            shutdown_done
                .send(started.elapsed())
                .expect("report shutdown latency");
        });
        let Ok(shutdown_elapsed) = shutdown_receiver.recv_timeout(Duration::from_millis(500))
        else {
            release.send(()).expect("release stuck shutdown");
            shutdown_thread.join().expect("join stuck shutdown");
            panic!("shutdown joined a stuck blocking worker");
        };
        assert!(shutdown_elapsed < Duration::from_millis(500));
        assert!(
            weak_owned.upgrade().is_some(),
            "the detached worker must retain its job resources"
        );

        release.send(()).expect("release detached worker");
        let returned = futures_executor::block_on(task);
        assert!(Arc::ptr_eq(
            &returned,
            &weak_owned
                .upgrade()
                .expect("result remains owned by the future")
        ));
        drop(returned);
        assert!(weak_owned.upgrade().is_none());
        shutdown_thread.join().expect("join bounded shutdown");
        wait_for_zero(&worker_cohort, "blocking worker was not collected");
    }

    #[test]
    fn supervisor_shutdown_cancels_an_admitted_start_before_release() {
        let fixture = Fixture::new();
        let state_root = open_directory(&fixture.state_root).expect("state descriptor");
        let store = Arc::new(NativeStore {
            inner: Arc::new(
                crate::background_store::BackgroundStore::prepare(
                    state_root,
                    fixture.workspace.clone(),
                )
                .expect("background store"),
            ),
        });
        let (entered, entered_receiver) = mpsc::sync_channel(1);
        let (release, release_receiver) = mpsc::sync_channel(1);
        let cleaned = Arc::new(AtomicBool::new(false));
        let command_executed = Arc::new(AtomicBool::new(false));
        let spawner = Arc::new(PausedSpawner {
            entered,
            release: Mutex::new(Some(release_receiver)),
            cleaned: Arc::clone(&cleaned),
            command_executed: Arc::clone(&command_executed),
        });
        let retainer = Arc::new(WorkerRetainer::new(1).expect("worker retainer"));
        let supervisor = NativeBackgroundSupervisor {
            supervisor: BackgroundSupervisor::new(
                Arc::new(SystemClock),
                Arc::clone(&store) as Arc<dyn machine_god_core::BackgroundStore>,
                spawner as Arc<dyn BackgroundProcessSpawner>,
                Arc::clone(&retainer) as Arc<dyn BackgroundProcessRetainer>,
            ),
            store,
            retainer,
            blocking: BlockingExecutor::new(1).expect("blocking executor"),
        };
        let request =
            BackgroundStartRequest::new("must-not-execute", &fixture.workspace).expect("request");
        let mut start = supervisor.start(request, CancellationToken::new());

        assert!(matches!(
            start.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("admitted start reached the release boundary");

        let drop_started = Instant::now();
        drop(supervisor);
        assert!(
            drop_started.elapsed() < Duration::from_millis(250),
            "supervisor shutdown waited for the admitted start"
        );
        release.send(()).expect("open paused release boundary");

        let error = futures_executor::block_on(start)
            .expect_err("shutdown cancellation must reject the admitted start");
        assert_eq!(error.kind(), BackgroundStartErrorKind::Cancelled);
        assert!(cleaned.load(Ordering::Acquire));
        assert!(!command_executed.load(Ordering::Acquire));
    }

    #[test]
    fn cancellable_admission_and_shutdown_race_never_misses_worker_cancellation() {
        for _ in 0..64 {
            let executor = BlockingExecutor::new(1).expect("blocking executor");
            let shutdown = BlockingExecutor {
                pool: Arc::clone(&executor.pool),
            };
            let worker_cohort = Arc::clone(&executor.pool.worker_cohort);
            let operation_cancellation = CancellationToken::new();
            let worker_cancellation = operation_cancellation.clone();
            let caller_cancellation = CancellationToken::new();
            let race = Arc::new(Barrier::new(2));
            let poll_race = Arc::clone(&race);
            let (finished, finished_receiver) = mpsc::sync_channel(1);
            let task = executor.run_cancellable(
                move || {
                    let deadline = Instant::now() + Duration::from_secs(1);
                    while !worker_cancellation.is_cancelled() && Instant::now() < deadline {
                        thread::yield_now();
                    }
                    worker_cancellation.is_cancelled()
                },
                caller_cancellation.cancelled(),
                operation_cancellation,
            );
            let poller = thread::spawn(move || {
                poll_race.wait();
                finished
                    .send(futures_executor::block_on(task))
                    .expect("report raced admission");
            });

            race.wait();
            shutdown.shutdown();
            match finished_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("raced start must finish")
            {
                Ok(cancelled) => assert!(cancelled, "submitted worker missed shutdown"),
                Err(BlockingTaskFailure::Admission) => {}
                Err(BlockingTaskFailure::CancelledBeforeSubmission) => {
                    panic!("caller token was not cancelled")
                }
            }
            poller.join().expect("join raced admission");
            drop(shutdown);
            drop(executor);
            wait_for_zero(&worker_cohort, "raced blocking worker was not collected");
        }
    }

    #[test]
    fn caller_cancellation_reaches_a_blocked_start_before_release() {
        let executor = BlockingExecutor::new(1).expect("blocking executor");
        let caller_cancellation = CancellationToken::new();
        let operation_cancellation = CancellationToken::new();
        let worker_cancellation = operation_cancellation.clone();
        let released = Arc::new(AtomicBool::new(false));
        let worker_released = Arc::clone(&released);
        let cleaned = Arc::new(AtomicBool::new(false));
        let worker_cleaned = Arc::clone(&cleaned);
        let (blocked, blocked_receiver) = mpsc::sync_channel(1);
        let (unblock, unblock_receiver) = mpsc::sync_channel(1);
        let task = executor.run_cancellable(
            move || {
                blocked.send(()).expect("report blocked start step");
                unblock_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .expect("unblock start step");
                if worker_cancellation.is_cancelled() {
                    worker_cleaned.store(true, Ordering::Release);
                    Err(())
                } else {
                    worker_released.store(true, Ordering::Release);
                    Ok(())
                }
            },
            caller_cancellation.cancelled(),
            operation_cancellation.clone(),
        );

        let caller = thread::spawn(move || futures_executor::block_on(task));
        blocked_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker reached blocked start step");
        caller_cancellation.cancel();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !operation_cancellation.is_cancelled() {
            assert!(
                Instant::now() < deadline,
                "caller executor did not bridge cancellation"
            );
            thread::yield_now();
        }
        assert!(operation_cancellation.is_cancelled());
        unblock.send(()).expect("unblock start step");
        assert!(matches!(
            caller.join().expect("caller executor"),
            Ok(Err(()))
        ));
        assert!(cleaned.load(Ordering::Acquire));
        assert!(!released.load(Ordering::Acquire));

        let sibling_caller = CancellationToken::new();
        let sibling_operation = CancellationToken::new();
        assert_eq!(
            futures_executor::block_on(executor.run_cancellable(
                || 7_u8,
                sibling_caller.cancelled(),
                sibling_operation.clone(),
            )),
            Ok(7)
        );
        assert!(!sibling_caller.is_cancelled());
        assert!(!sibling_operation.is_cancelled());
    }

    #[test]
    fn superseded_waker_drop_is_reentrant_without_blocking_publication_or_reuse() {
        let executor = BlockingExecutor::new(1).expect("blocking executor");
        let worker_can_finish = Arc::new(AtomicBool::new(false));
        let worker_gate = Arc::clone(&worker_can_finish);
        let task = executor.run(
            move || {
                while !worker_gate.load(Ordering::Acquire) {
                    thread::yield_now();
                }
                7_u8
            },
            0,
        );
        let result = Arc::downgrade(&task.result);
        let mut task = Box::pin(task);
        let (entered, entered_receiver) = mpsc::sync_channel(1);
        let (release, release_receiver) = mpsc::sync_channel(1);
        let hostile = Arc::new(ReentrantDropWake {
            result,
            worker_can_finish,
            entered,
            release: Mutex::new(release_receiver),
        });
        let hostile_waker = Waker::from(hostile);

        assert!(matches!(
            task.as_mut().poll(&mut Context::from_waker(&hostile_waker)),
            Poll::Pending
        ));
        drop(hostile_waker);

        let replacement = Arc::new(WakeFlag(AtomicBool::new(false)));
        let poll_wake = Arc::clone(&replacement);
        let poller = thread::spawn(move || {
            let waker = Waker::from(Arc::clone(&poll_wake));
            let poll = task.as_mut().poll(&mut Context::from_waker(&waker));
            (task, poll_wake, poll)
        });
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("hostile waker drop entered after unlocking");
        release.send(()).expect("release hostile waker drop");
        let (mut task, replacement, poll) = poller.join().expect("replacement poll");
        assert!(matches!(poll, Poll::Pending));
        wait_for_wake(&replacement);
        assert!(matches!(
            task.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(7)
        ));

        let slot_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let slot_available = !executor
                .pool
                .available
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty();
            if slot_available {
                break;
            }
            assert!(
                Instant::now() < slot_deadline,
                "worker did not return its blocking slot"
            );
            thread::yield_now();
        }
        assert_eq!(futures_executor::block_on(executor.run(|| 9_u8, 0)), 9);
        executor.shutdown();
    }

    #[test]
    fn dropping_one_submitted_start_does_not_cancel_a_shared_caller_token() {
        let dropped_fixture = Fixture::new();
        let sibling_fixture = Fixture::new();
        let dropped_supervisor = dropped_fixture.supervisor(1);
        let sibling_supervisor = sibling_fixture.supervisor(1);
        let dropped_request =
            BackgroundStartRequest::new(":", &dropped_fixture.workspace).expect("request");
        let sibling_request =
            BackgroundStartRequest::new(":", &sibling_fixture.workspace).expect("request");
        let shared = CancellationToken::new();
        let mut dropped = dropped_supervisor.start(dropped_request, shared.clone());
        let mut sibling = sibling_supervisor.start(sibling_request, shared.clone());

        assert!(matches!(
            dropped
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        assert!(matches!(
            sibling
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        drop(dropped);

        assert!(!shared.is_cancelled());
        let sibling = futures_executor::block_on(sibling).expect("sibling start completes");
        assert!(!shared.is_cancelled());
        sibling_fixture.await_terminal(sibling.id());
    }

    #[test]
    fn closing_dispatch_returns_promptly_and_persists_a_successful_stop() {
        let fixture = Fixture::new();
        let state_root = open_directory(&fixture.state_root).expect("state descriptor");
        let store = Arc::new(NativeStore {
            inner: Arc::new(
                crate::background_store::BackgroundStore::prepare(
                    state_root,
                    fixture.workspace.clone(),
                )
                .expect("background store"),
            ),
        });
        let captured = Arc::new(Mutex::new(None));
        let retainer = Arc::new(CapturingRetainer {
            job: Arc::clone(&captured),
        });
        let spawner = Arc::new(NativeSpawner {
            workspace: fixture.workspace.clone(),
            workspace_root: Arc::new(
                open_directory(&fixture.workspace_path).expect("workspace descriptor"),
            ),
            environment: Arc::new(test_environment()),
            adapter: test_adapter(),
        });
        let supervisor = BackgroundSupervisor::new(
            Arc::new(SystemClock),
            Arc::clone(&store) as Arc<dyn machine_god_core::BackgroundStore>,
            spawner as Arc<dyn machine_god_core::BackgroundProcessSpawner>,
            retainer as Arc<dyn BackgroundProcessRetainer>,
        );
        let request = BackgroundStartRequest::new(
            "trap '' TERM; while :; do /bin/sleep 1; done",
            &fixture.workspace,
        )
        .expect("request");
        let handle =
            futures_executor::block_on(supervisor.start(request, CancellationToken::new()))
                .expect("capture released job");
        let job = captured
            .lock()
            .expect("captured job")
            .take()
            .expect("released job");

        let resource = Arc::new(());
        let weak_resource = Arc::downgrade(&resource);
        let (entered, entered_receiver) = mpsc::sync_channel(1);
        let (release, release_receiver) = mpsc::sync_channel(1);
        let process = Box::new(BlockingOwnedProcess {
            inner: job.process,
            resource,
            entered,
            release: release_receiver,
        });
        let retainer = WorkerRetainer::new(1).expect("worker retainer");
        let worker_cohort = Arc::clone(&retainer.pool.worker_cohort);
        let permit = retainer.try_admit().expect("retention permit");
        retainer.shutdown();
        let started = Instant::now();
        permit.retain(job.lease, job.record, process);
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "closing dispatch performed process cleanup on the caller"
        );
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker retained the process");
        let drop_started = Instant::now();
        drop(retainer);
        assert!(
            drop_started.elapsed() < Duration::from_millis(250),
            "retainer Drop waited for a process worker"
        );
        assert!(
            weak_resource.upgrade().is_some(),
            "the ownership registry must retain the process job after Drop"
        );
        release.send(()).expect("release retained process");
        wait_for_zero(&worker_cohort, "retainer workers were not collected");
        assert!(weak_resource.upgrade().is_none());
        assert_eq!(
            fixture.detail(handle.id()).state(),
            NativeBackgroundState::Stopped
        );
    }

    fn wait_for_wake(wake: &WakeFlag) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !wake.0.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "worker did not wake task");
            thread::yield_now();
        }
    }

    fn wait_for_zero(value: &AtomicUsize, message: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while value.load(Ordering::Acquire) != 0 {
            assert!(Instant::now() < deadline, "{message}");
            thread::yield_now();
        }
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
