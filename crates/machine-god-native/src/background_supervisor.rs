//! Host-owned composition of background persistence and process supervision.

use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
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

enum BlockingMessage {
    Run(BlockingJob),
    Shutdown,
}

struct BlockingExecutor {
    pool: Arc<BlockingPool>,
}

struct BlockingPool {
    senders: Vec<SyncSender<BlockingMessage>>,
    available: Arc<Mutex<Vec<usize>>>,
    closing: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
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
        let available = Arc::new(Mutex::new((0..size).rev().collect()));
        let closing = Arc::new(AtomicBool::new(false));
        let mut senders: Vec<SyncSender<BlockingMessage>> = Vec::with_capacity(size);
        let mut workers: Vec<JoinHandle<()>> = Vec::with_capacity(size);
        for index in 0..size {
            let (sender, receiver) = sync_channel(1);
            let worker_available = Arc::clone(&available);
            let worker_closing = Arc::clone(&closing);
            let Ok(handle) = thread::Builder::new()
                .name(format!("machine-god-bg-blocking-{index}"))
                .spawn(move || {
                    blocking_worker_loop(index, &receiver, &worker_available, &worker_closing);
                })
            else {
                closing.store(true, Ordering::Release);
                for sender in &senders {
                    let _ = sender.try_send(BlockingMessage::Shutdown);
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
            pool: Arc::new(BlockingPool {
                senders,
                available,
                closing,
                workers: Mutex::new(workers),
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
    fn try_submit(&self, job: BlockingJob) -> Result<(), ()> {
        if self.closing.load(Ordering::Acquire) {
            return Err(());
        }
        let mut available = self.available.try_lock().map_err(|_| ())?;
        let index = available.pop().ok_or(())?;
        if self.closing.load(Ordering::Acquire) {
            available.push(index);
            return Err(());
        }
        match self.senders[index].try_send(BlockingMessage::Run(job)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                available.push(index);
                Err(())
            }
        }
    }

    fn shutdown(&self) {
        if self.closing.swap(true, Ordering::AcqRel) {
            return;
        }
        for sender in &self.senders {
            let _ = sender.try_send(BlockingMessage::Shutdown);
        }
        if let Ok(mut workers) = self.workers.lock() {
            for worker in workers.drain(..) {
                let _ = worker.join();
            }
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
            if this.pool.try_submit(job).is_err() {
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
    closing: &AtomicBool,
) {
    while let Ok(message) = receiver.recv() {
        match message {
            BlockingMessage::Run(job) => {
                let _ = catch_unwind(AssertUnwindSafe(job));
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
                .map_err(|_| start_error(BackgroundStartErrorKind::Process))?;
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
    let stop = CancellationToken::new();
    stop.cancel();
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
        BlockingExecutor, BlockingResult, BlockingTaskFailure, NativeBackgroundLimits,
        NativeBackgroundSupervisor, NativeSpawner, NativeStore, RetainedJob,
        SystemBackgroundProcessAdapter, SystemClock, finish_prepared_after_readiness,
        finish_without_worker, open_directory, retain_canonical_directory_with,
    };
    use crate::background_process::{BackgroundProcessHelper, run_background_process_helper};
    use crate::{
        NativeBackgroundInspection, NativeBackgroundQuery, NativeBackgroundState,
        NativeEnvironment, inspect_native_background,
    };
    use machine_god_core::{
        BackgroundProcessRetainer, BackgroundRetentionPermit, BackgroundStartError,
        BackgroundStartErrorKind, BackgroundStartRequest, BackgroundSupervisor, CancellationToken,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::future::Future;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, Weak, mpsc};
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
            self.worker_can_finish.store(true, Ordering::Release);
            let Some(result) = self.result.upgrade() else {
                return;
            };
            let _result = result
                .try_lock()
                .expect("superseded waker must be dropped after unlocking its result");
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

        drop(supervisor);
        assert!(!process_exists(pid));
        assert_eq!(
            fixture.await_terminal(first.id()).state(),
            NativeBackgroundState::Stopped
        );
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
    fn closing_dispatch_waits_and_persists_a_successful_stop() {
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

        finish_without_worker(job);

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
