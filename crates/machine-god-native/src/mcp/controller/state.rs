use super::{
    BoxFuture, CancellationToken, Instant, NativeMcpConfigStoreError, NativeMcpControllerError,
    NativeMcpControllerFailure, NativeMcpControllerOptions, NativeMcpControllerPublication,
    NativeMcpRuntimeError, NativeMcpStartupError, NativeMcpStartupPhase, NativeMcpStartupReceipt,
};
use crate::mcp::{
    runtime::{NativeMcpPeerCompletion, NativeMcpPublicationCheckpoint},
    startup::{NativeMcpStartup, NativeMcpStartupCompletion},
    store::NativeMcpConfigSnapshot,
};
use futures_util::future::Shared;
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};

pub(super) type JobResult = std::result::Result<Receipt, Failure>;
pub(super) type Job = Shared<BoxFuture<'static, JobResult>>;
pub(super) const MAX_PEER_OBSERVATIONS: usize = 1024;
pub(super) const MAX_DRAIN_BATCH: usize = 128;

#[derive(Clone)]
pub(super) struct Receipt {
    pub startup: Option<NativeMcpStartupReceipt>,
    pub publication: NativeMcpControllerPublication,
    pub closed: bool,
}
#[derive(Clone)]
pub(super) struct Failure {
    pub kind: NativeMcpControllerError,
    pub startup: Option<NativeMcpStartupReceipt>,
}
impl From<NativeMcpControllerError> for Failure {
    fn from(kind: NativeMcpControllerError) -> Self {
        Self {
            kind,
            startup: None,
        }
    }
}
impl From<NativeMcpRuntimeError> for Failure {
    fn from(error: NativeMcpRuntimeError) -> Self {
        NativeMcpControllerError::Runtime(error).into()
    }
}
impl From<NativeMcpConfigStoreError> for Failure {
    fn from(error: NativeMcpConfigStoreError) -> Self {
        NativeMcpControllerError::Store(error).into()
    }
}
impl From<NativeMcpStartupError> for Failure {
    fn from(error: NativeMcpStartupError) -> Self {
        NativeMcpControllerError::Startup(error).into()
    }
}
impl From<crate::mcp::startup::NativeMcpStartupFailure> for Failure {
    fn from(failure: crate::mcp::startup::NativeMcpStartupFailure) -> Self {
        Self {
            kind: NativeMcpControllerError::Startup(failure.error),
            startup: Some(failure.receipt),
        }
    }
}

pub(super) fn failure(kind: NativeMcpControllerError) -> NativeMcpControllerFailure {
    NativeMcpControllerFailure {
        data: kind.into(),
        generation: None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Start(NativeMcpStartupPhase),
    Reload,
    Deferred,
}

pub(super) struct Loaded {
    pub snapshot: Arc<NativeMcpConfigSnapshot>,
    pub startup: Arc<NativeMcpStartup>,
    pub checkpoint: Option<NativeMcpPublicationCheckpoint>,
}
pub(super) struct Generation {
    pub phase: NativeMcpStartupPhase,
    pub cancellation: CancellationToken,
    pub loaded: Mutex<Option<Loaded>>,
    pub deferred: Mutex<Option<JobResult>>,
    pub workers: std::sync::atomic::AtomicUsize,
}
impl Generation {
    pub fn cleanup_complete(&self) -> bool {
        self.workers.load(Ordering::Acquire) == 0
            && lock(&self.loaded).as_ref().is_none_or(|loaded| {
                loaded
                    .startup
                    .cleanup_observations()
                    .iter()
                    .all(NativeMcpStartupCompletion::is_complete)
            })
    }
}
pub(super) struct WorkerReservation(pub Arc<Generation>);
impl WorkerReservation {
    pub fn new(generation: &Arc<Generation>) -> Self {
        generation.workers.fetch_add(1, Ordering::AcqRel);
        Self(generation.clone())
    }
}
impl Drop for WorkerReservation {
    fn drop(&mut self) {
        self.0.workers.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Clone)]
pub(super) struct Running {
    pub kind: Kind,
    pub generation: Arc<Generation>,
    pub cancellation: CancellationToken,
    pub future: Job,
}
#[derive(Default)]
pub(super) struct State {
    pub closed: bool,
    pub active: Option<Arc<Generation>>,
    pub generations: Vec<Arc<Generation>>,
    pub running: Option<Running>,
    pub peers: Vec<NativeMcpPeerCompletion>,
}
pub(super) struct Inner {
    pub options: Arc<NativeMcpControllerOptions>,
    pub state: Mutex<State>,
    pub settling: AtomicBool,
}
impl Inner {
    pub fn new(options: NativeMcpControllerOptions) -> Self {
        Self {
            options: Arc::new(options),
            state: Mutex::default(),
            settling: AtomicBool::new(false),
        }
    }
    pub fn close(&self) {
        let (generations, job, active) = {
            let mut state = lock(&self.state);
            state.closed = true;
            (
                state.generations.clone(),
                state.running.as_ref().map(|job| job.cancellation.clone()),
                state.active.take(),
            )
        };
        self.options.startup.owner_cancellation.cancel();
        if let Some(job) = job {
            job.cancel();
        }
        for generation in generations {
            generation.cancellation.cancel();
        }
        self.options.runtime.close();
        drop(active);
    }
    pub fn check(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> std::result::Result<(), Failure> {
        if lock(&self.state).closed || self.options.startup.owner_cancellation.is_cancelled() {
            return Err(NativeMcpControllerError::Closed.into());
        }
        cleanup_check(&self.options, cancellation, deadline)
    }
    /// Drop completed shared futures outside the mutex: their last allocation
    /// may own cancellation/cleanup drops. Active jobs remain retained.
    pub fn release_completed(&self) {
        let completed = {
            let mut state = lock(&self.state);
            if state
                .running
                .as_ref()
                .is_some_and(|job| job.future.peek().is_some())
            {
                state.running.take()
            } else {
                None
            }
        };
        drop(completed);
    }
    pub fn prune(&self) {
        self.release_completed();
        let mut state = lock(&self.state);
        state.peers.retain(|value| !value.is_complete());
        state.generations.retain(|generation| {
            Arc::strong_count(generation) != 1 || !generation.cleanup_complete()
        });
    }
}
pub(super) fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub(super) fn cleanup_check(
    options: &NativeMcpControllerOptions,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> std::result::Result<(), Failure> {
    if cancellation.is_cancelled() {
        return Err(NativeMcpControllerError::Cancelled.into());
    }
    if options.startup.clock.now() >= deadline {
        return Err(NativeMcpControllerError::Deadline.into());
    }
    Ok(())
}
pub(super) struct CancelOnDrop(pub CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
pub(super) struct Settlement(pub Arc<Inner>);
impl Drop for Settlement {
    fn drop(&mut self) {
        self.0.settling.store(false, Ordering::Release);
    }
}
