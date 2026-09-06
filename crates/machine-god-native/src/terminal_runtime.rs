//! Lazy assembly of one terminal owner on an explicitly owned blocking worker.
//! No native registry, profile, backend, or thread is created by construction or
//! an unpolled request. Futures retain request state, never a host lifetime vote.

use crate::terminal_owner::{
    TerminalOwnerContext, TerminalOwnerError, TerminalOwnerFuture, TerminalOwnerHandle,
    TerminalOwnerLoop,
};
use crate::terminal_profile::TerminalProfileBudget;
use crate::terminal_profile_store::TerminalProfileStore;
use crate::terminal_registry::{
    TerminalRegistry, TerminalRegistryError, TerminalRegistryFailure, TerminalRegistryStep,
};
use crate::terminal_session::TerminalSessionBackend;
#[cfg(test)]
use crate::terminal_wait::TerminalWaitCoordinator;
use machine_god_core::{CancellationToken, TerminalClosePolicy};
use std::fmt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

const CLEANUP_INITIAL_BACKOFF: Duration = Duration::from_millis(10);
const CLEANUP_MAX_BACKOFF: Duration = Duration::from_secs(1);

pub(crate) type TerminalRuntimeJob = Box<dyn FnOnce() + Send + 'static>;

/// The production binding registers the thread in the native owned-worker
/// collector before releasing this job. An error means the job did not execute;
/// the spawner must release it and collect any failed-registration thread.
/// This callback must not execute the blocking job inline on the polling thread.
pub(crate) trait TerminalRuntimeSpawner: Send + Sync {
    fn spawn(&self, job: TerminalRuntimeJob) -> std::result::Result<(), ()>;
}

impl TerminalRuntimeSpawner for crate::NativeOwnedWorkerSpawner {
    fn spawn(&self, job: TerminalRuntimeJob) -> std::result::Result<(), ()> {
        crate::NativeOwnedWorkerSpawner::spawn(self, job).map_err(|_| ())
    }
}

impl TerminalRuntimeSpawner for crate::NativeOwnedWorkerScope {
    fn spawn(&self, job: TerminalRuntimeJob) -> std::result::Result<(), ()> {
        crate::NativeOwnedWorkerScope::spawn(self, job).map_err(|_| ())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalRuntimeError {
    Spawn,
    Initialization,
    Panicked,
    Owner(TerminalOwnerError),
}
impl fmt::Display for TerminalRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal runtime unavailable")
    }
}
impl std::error::Error for TerminalRuntimeError {}
type Result<T> = std::result::Result<T, TerminalRuntimeError>;
type Initializer<B, S> = Box<dyn FnOnce() -> Result<TerminalRuntimeWorker<B, S>> + Send + 'static>;

fn contain<T>(operation: impl FnOnce() -> T) -> std::result::Result<T, ()> {
    catch_unwind(AssertUnwindSafe(operation)).map_err(|payload| {
        // Opaque panic payloads may themselves have hostile destructors.
        std::mem::forget(payload);
    })
}

struct Starter<B: TerminalSessionBackend, S> {
    owner: TerminalOwnerLoop<B, S>,
    initialize: Initializer<B, S>,
}
enum Phase<B: TerminalSessionBackend, S> {
    Dormant(Starter<B, S>),
    Starting(Arc<Mutex<Option<Starter<B, S>>>>),
    Running,
    Failed(TerminalRuntimeError),
    Stopped,
}
struct Shared<B: TerminalSessionBackend, S = ()> {
    phase: Mutex<Phase<B, S>>,
    spawner: Arc<dyn TerminalRuntimeSpawner>,
    hosts: AtomicUsize,
    closing: AtomicBool,
}
impl<B: TerminalSessionBackend + Send + 'static, S: 'static> Shared<B, S> {
    fn failure(&self) -> Option<TerminalRuntimeError> {
        match &*self
            .phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            Phase::Failed(error) => Some(*error),
            _ => None,
        }
    }

    fn set_phase(&self, phase: Phase<B, S>) {
        let previous = {
            let mut current = self
                .phase
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::replace(&mut *current, phase)
        };
        // Owner-loop destruction resolves queued replies and invokes their
        // wakers. It must never happen while holding the initialization mutex.
        let _ = contain(|| drop(previous));
    }

    fn close(&self) {
        self.closing.store(true, Ordering::Release);
        let dormant = {
            let mut phase = self
                .phase
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &*phase {
                Phase::Dormant(_) => {
                    let Phase::Dormant(starter) = std::mem::replace(&mut *phase, Phase::Stopped)
                    else {
                        unreachable!("checked dormant phase")
                    };
                    Some(starter)
                }
                Phase::Starting(slot) => slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take(),
                _ => None,
            }
        };
        let _ = contain(|| drop(dormant));
    }

    fn start(self: &Arc<Self>) {
        let slot = {
            let mut phase = self
                .phase
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !matches!(*phase, Phase::Dormant(_)) || self.closing.load(Ordering::Acquire) {
                return;
            }
            let Phase::Dormant(starter) = std::mem::replace(&mut *phase, Phase::Stopped) else {
                unreachable!("checked dormant phase")
            };
            let slot = Arc::new(Mutex::new(Some(starter)));
            *phase = Phase::Starting(Arc::clone(&slot));
            slot
        };
        let shared = Arc::clone(self);
        let job = Box::new(move || {
            let starter = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(starter) = starter else { return };
            if shared.closing.load(Ordering::Acquire) {
                shared.set_phase(Phase::Stopped);
                drop(starter);
                return;
            }
            let Starter { owner, initialize } = starter;
            match contain(initialize) {
                Ok(Ok(worker)) => {
                    shared.set_phase(Phase::Running);
                    let completed = contain(|| worker.run(owner));
                    shared.set_phase(match completed {
                        Ok(Ok(())) => Phase::Stopped,
                        Ok(Err(error)) => Phase::Failed(error),
                        Err(()) => Phase::Failed(TerminalRuntimeError::Panicked),
                    });
                }
                outcome => {
                    let error = match outcome {
                        Ok(Err(error)) => error,
                        Err(()) => TerminalRuntimeError::Panicked,
                        Ok(Ok(_)) => unreachable!("successful initializer handled"),
                    };
                    shared.set_phase(Phase::Failed(error));
                    // No successfully initialized native owner leaves this
                    // worker. Closing the queue wakes every admitted request.
                    drop(owner);
                }
            }
        });
        let error = match contain(|| self.spawner.spawn(job)) {
            Ok(Ok(())) => None,
            Ok(Err(())) => Some(TerminalRuntimeError::Spawn),
            Err(()) => Some(TerminalRuntimeError::Panicked),
        };
        if let Some(error) = error {
            self.set_phase(Phase::Failed(error));
        }
    }
}

/// Only actual host handles count toward lifetime. The initializer is trusted
/// host code and must create all registry/profile/backend authority in its body,
/// not capture an already-created native session. Failed spawn then has no PTY
/// to destroy on the polling thread.
pub(crate) struct TerminalRuntime<B: TerminalSessionBackend + Send + 'static, S: 'static = ()> {
    shared: Arc<Shared<B, S>>,
    owner: TerminalOwnerHandle<B, S>,
}

/// Multi-request access for explicitly owned effect workers. Unlike a host
/// handle, this cannot prolong native/session lifetime. It carries no `S` value.
pub(crate) struct TerminalRuntimeRequester<B: TerminalSessionBackend + Send + 'static, S: 'static> {
    shared: Arc<Shared<B, S>>,
    owner: TerminalOwnerHandle<B, S>,
}
impl<B: TerminalSessionBackend + Send + 'static, S: 'static> Clone
    for TerminalRuntimeRequester<B, S>
{
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            owner: self.owner.clone(),
        }
    }
}
impl<B: TerminalSessionBackend + Send + 'static, S: 'static> TerminalRuntimeRequester<B, S> {
    pub(crate) fn request_with_context<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(TerminalOwnerContext<'_, B, S>) -> T + Send + 'static,
    ) -> TerminalRuntimeFuture<B, T, S> {
        TerminalRuntimeFuture {
            shared: Arc::clone(&self.shared),
            request: self.owner.request_with_context(caller, operation),
            finished: false,
        }
    }
}
impl<B: TerminalSessionBackend + Send + 'static, S: 'static> Clone for TerminalRuntime<B, S> {
    fn clone(&self) -> Self {
        self.shared.hosts.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
            owner: self.owner.clone(),
        }
    }
}
impl<B: TerminalSessionBackend + Send + 'static, S: 'static> Drop for TerminalRuntime<B, S> {
    fn drop(&mut self) {
        if self.shared.hosts.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.owner.shutdown();
            self.shared.close();
        }
    }
}
impl<B: TerminalSessionBackend + Send + 'static, S: 'static> fmt::Debug for TerminalRuntime<B, S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalRuntime")
            .finish_non_exhaustive()
    }
}
impl<B: TerminalSessionBackend + Send + 'static, S: 'static> TerminalRuntime<B, S> {
    pub(crate) fn requester(&self) -> TerminalRuntimeRequester<B, S> {
        TerminalRuntimeRequester {
            shared: Arc::clone(&self.shared),
            owner: self.owner.requester(),
        }
    }
    pub(crate) fn new(
        initialize: impl FnOnce() -> Result<TerminalRuntimeWorker<B, S>> + Send + 'static,
        spawner: Arc<dyn TerminalRuntimeSpawner>,
    ) -> Self {
        let (owner_loop, owner) = TerminalOwnerLoop::new();
        Self {
            shared: Arc::new(Shared {
                phase: Mutex::new(Phase::Dormant(Starter {
                    owner: owner_loop,
                    initialize: Box::new(initialize),
                })),
                spawner,
                hosts: AtomicUsize::new(1),
                closing: AtomicBool::new(false),
            }),
            owner,
        }
    }

    #[cfg(test)]
    pub(crate) fn request<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(&mut TerminalRegistry<B>, i64, &CancellationToken) -> T + Send + 'static,
    ) -> TerminalRuntimeFuture<B, T, S> {
        self.wrap(self.owner.request(caller, operation))
    }

    #[cfg(test)]
    pub(crate) fn request_with_profile<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(
            &mut TerminalRegistry<B>,
            &TerminalProfileStore,
            &TerminalProfileBudget,
            i64,
            &CancellationToken,
        ) -> T
        + Send
        + 'static,
    ) -> TerminalRuntimeFuture<B, T, S> {
        self.wrap(self.owner.request_with_profile(caller, operation))
    }

    #[cfg(test)]
    pub(crate) fn request_with_waits<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(
            &mut TerminalRegistry<B>,
            &TerminalProfileStore,
            &TerminalProfileBudget,
            &mut TerminalWaitCoordinator,
            i64,
            &CancellationToken,
        ) -> T
        + Send
        + 'static,
    ) -> TerminalRuntimeFuture<B, T, S> {
        self.wrap(self.owner.request_with_waits(caller, operation))
    }

    #[cfg(test)]
    pub(crate) fn request_with_writes<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(
            &mut TerminalRegistry<B>,
            &TerminalProfileStore,
            &TerminalProfileBudget,
            &mut crate::terminal_write_completion::TerminalWriteCoordinator,
            i64,
            &CancellationToken,
        ) -> T
        + Send
        + 'static,
    ) -> TerminalRuntimeFuture<B, T, S> {
        self.wrap(self.owner.request_with_writes(caller, operation))
    }

    /// Borrow typed host state and the complete short-dispatch context on the
    /// owning worker. No `S` value is created or stored by this future.
    #[cfg(test)]
    pub(crate) fn request_with_context<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(TerminalOwnerContext<'_, B, S>) -> T + Send + 'static,
    ) -> TerminalRuntimeFuture<B, T, S> {
        self.wrap(self.owner.request_with_context(caller, operation))
    }

    #[cfg(test)]
    fn wrap<T>(&self, request: TerminalOwnerFuture<B, T, S>) -> TerminalRuntimeFuture<B, T, S> {
        TerminalRuntimeFuture {
            shared: Arc::clone(&self.shared),
            request,
            finished: false,
        }
    }

    /// Explicit shutdown is idempotent and nonblocking like last-host drop.
    pub(crate) fn shutdown(&self) {
        self.owner.shutdown();
        self.shared.close();
    }
}

pub(crate) struct TerminalRuntimeFuture<B: TerminalSessionBackend, T, S = ()> {
    shared: Arc<Shared<B, S>>,
    request: TerminalOwnerFuture<B, T, S>,
    finished: bool,
}
impl<B: TerminalSessionBackend + Send + 'static, T: Send + 'static, S: 'static> Future
    for TerminalRuntimeFuture<B, T, S>
{
    type Output = Result<T>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(
            !this.finished,
            "completed terminal runtime request polled again"
        );
        if let Some(error) = this.shared.failure() {
            this.finished = true;
            return Poll::Ready(Err(error));
        }
        // The owner checks cancellation/closure/capacity before queueing. Only
        // an actually admitted Pending request may trigger worker startup.
        match Pin::new(&mut this.request).poll(cx) {
            Poll::Ready(result) => {
                this.finished = true;
                Poll::Ready(result.map_err(TerminalRuntimeError::Owner))
            }
            Poll::Pending => {
                this.shared.start();
                if let Some(error) = this.shared.failure() {
                    this.finished = true;
                    Poll::Ready(Err(error))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

/// Created inside the owned worker initializer. Registry and persistence remain
/// on that worker through every failed native or durable cleanup retry.
pub(crate) struct TerminalRuntimeWorker<B: TerminalSessionBackend, S = ()> {
    registry: TerminalRegistry<B>,
    store: TerminalProfileStore,
    budget: TerminalProfileBudget,
    clock: Box<dyn FnMut() -> i64 + Send>,
    observer: StateObserver<S>,
    // Fields drop in declaration order, also during unwinding. State's catalog
    // locks and cleanup authority outlive registry/backend destruction.
    state: WorkerState<S>,
}
type StateObserver<S> = Box<dyn FnMut(&mut S, Vec<TerminalRegistryStep>) + Send>;

struct WorkerState<S>(Option<S>);
impl<S> Drop for WorkerState<S> {
    fn drop(&mut self) {
        // A state destructor must not turn another field's teardown unwind
        // into a double panic, or run an opaque panic-payload destructor.
        let _ = contain(|| drop(self.0.take()));
    }
}

#[cfg(test)]
impl<B: TerminalSessionBackend> TerminalRuntimeWorker<B> {
    pub(crate) fn new(
        registry: TerminalRegistry<B>,
        store: TerminalProfileStore,
        budget: TerminalProfileBudget,
        clock: impl FnMut() -> i64 + Send + 'static,
        mut observer: impl FnMut(Vec<TerminalRegistryStep>) + Send + 'static,
    ) -> Self {
        Self::new_with_state(registry, store, budget, (), clock, move |(), steps| {
            observer(steps);
        })
    }
}

impl<B: TerminalSessionBackend, S> TerminalRuntimeWorker<B, S> {
    /// Construct inside the initializer, not on the polling thread. `S` need
    /// not be Send or Sync: only the initializer and request closures cross
    /// threads, never its result's worker-owned host state.
    pub(crate) fn new_with_state(
        registry: TerminalRegistry<B>,
        store: TerminalProfileStore,
        budget: TerminalProfileBudget,
        state: S,
        clock: impl FnMut() -> i64 + Send + 'static,
        observer: impl FnMut(&mut S, Vec<TerminalRegistryStep>) + Send + 'static,
    ) -> Self {
        Self {
            registry,
            store,
            budget,
            clock: Box::new(clock),
            observer: Box::new(observer),
            state: WorkerState(Some(state)),
        }
    }

    fn run(mut self, owner: TerminalOwnerLoop<B, S>) -> Result<()> {
        let exit = contain(|| {
            owner.run_with_profile_and_state(
                &mut self.registry,
                &self.store,
                &self.budget,
                self.state
                    .0
                    .as_mut()
                    .expect("worker owns its initialized state"),
                &mut self.clock,
                &mut self.observer,
            )
        });
        let mut observe;
        let (mut failure, cleaned) = if let Ok(exit) = exit {
            // The owner does not distinguish clock/request/observer panics.
            // Never re-enter a possibly failed callback during cleanup.
            observe = exit.error != Some(TerminalOwnerError::Panicked);
            let mut failure = exit.error.map(TerminalRuntimeError::Owner);
            let cleaned = self.observe_shutdown(exit.shutdown, &mut failure, &mut observe);
            (failure, cleaned)
        } else {
            observe = false;
            (Some(TerminalRuntimeError::Panicked), false)
        };
        // An old callback/clock error does not keep a clean registry alive.
        // Only shutdown diagnostics are observed during retries, never new
        // requests or numeric persisted process identities. A panicking
        // observer is disabled without interrupting native cleanup.
        if !cleaned {
            let mut backoff = CLEANUP_INITIAL_BACKOFF;
            loop {
                std::thread::sleep(backoff);
                let shutdown = contain(|| {
                    self.registry.shutdown_with_profile(
                        &self.store,
                        &self.budget,
                        self.registry.minimum_time_ms(),
                        TerminalClosePolicy::Force,
                    )
                });
                if shutdown
                    .is_ok_and(|result| self.observe_shutdown(result, &mut failure, &mut observe))
                {
                    break;
                }
                backoff = backoff.saturating_mul(2).min(CLEANUP_MAX_BACKOFF);
            }
        }
        // Preserve the initiating failure after native cleanup converges. A
        // clean registry is not evidence that its owner exited successfully.
        failure.map_or(Ok(()), Err)
    }

    fn observe_shutdown(
        &mut self,
        shutdown: std::result::Result<Vec<TerminalRegistryFailure>, TerminalRegistryError>,
        failure: &mut Option<TerminalRuntimeError>,
        observe: &mut bool,
    ) -> bool {
        let Ok(failures) = shutdown else {
            return false;
        };
        if failures.is_empty() {
            return true;
        }
        if *observe {
            let steps = failures
                .into_iter()
                .map(|failure| TerminalRegistryStep {
                    session_id: failure.session_id,
                    owner: failure.owner,
                    result: Err(failure.error),
                    cleanup_error: Some(failure.error),
                })
                .collect();
            if contain(|| {
                (self.observer)(
                    self.state
                        .0
                        .as_mut()
                        .expect("worker owns its initialized state"),
                    steps,
                );
            })
            .is_err()
            {
                *observe = false;
                failure.get_or_insert(TerminalRuntimeError::Panicked);
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_input::{BackgroundInputReceipt, BackgroundInputStatus};
    use crate::terminal_history::TerminalHistory;
    use crate::terminal_journal::TerminalJournalLimits;
    use crate::terminal_profile::{TerminalProfileLimits, TerminalProfileMutationContext};
    use crate::terminal_pty::{TerminalPtyClose, TerminalPtyRead, TerminalPtyStatus};
    use crate::terminal_session::TerminalSession;
    use crate::terminal_session_record::test_metadata;
    use machine_god_core::{
        BackgroundOutputOwner, SessionId, SessionIncarnationId, TerminalDimensions,
        TerminalSessionId, TerminalSignal,
    };
    use rustix::fd::OwnedFd;
    use rustix::fs::{Mode, OFlags};
    use std::fs::DirBuilder;
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;
    use std::sync::mpsc::{Receiver, sync_channel};
    use std::task::{Wake, Waker};
    use std::thread::{JoinHandle, ThreadId};

    #[derive(Default)]
    struct TestSpawner {
        calls: AtomicUsize,
        reject: AtomicBool,
        handles: Mutex<Vec<JoinHandle<()>>>,
        gate: Mutex<Option<Receiver<()>>>,
    }
    impl TerminalRuntimeSpawner for TestSpawner {
        fn spawn(&self, job: TerminalRuntimeJob) -> std::result::Result<(), ()> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.reject.load(Ordering::Acquire) {
                drop(job);
                return Err(());
            }
            let gate = self.gate.lock().unwrap().take();
            let (registered, owned) = sync_channel(1);
            let handle = std::thread::spawn(move || {
                owned.recv().unwrap();
                if let Some(gate) = gate {
                    gate.recv().unwrap();
                }
                job();
            });
            self.handles.lock().unwrap().push(handle);
            registered.send(()).unwrap();
            Ok(())
        }
    }
    impl TestSpawner {
        fn collect(&self) {
            let handles = std::mem::take(&mut *self.handles.lock().unwrap());
            for handle in handles {
                handle.join().unwrap();
            }
        }
    }

    #[derive(Default)]
    struct BackendState {
        close_attempts: usize,
        close_failures: usize,
        close_threads: Vec<ThreadId>,
        dropped_on: Option<ThreadId>,
        closed: bool,
    }
    struct Backend(Arc<Mutex<BackendState>>);
    impl Drop for Backend {
        fn drop(&mut self) {
            self.0.lock().unwrap().dropped_on = Some(std::thread::current().id());
        }
    }
    impl TerminalSessionBackend for Backend {
        fn read(&mut self, _: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
            Ok(TerminalPtyRead {
                bytes_read: 0,
                closed: false,
            })
        }
        fn write(&mut self, _: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
            Ok(BackgroundInputReceipt::new(
                0,
                false,
                BackgroundInputStatus::Backpressure,
            ))
        }
        fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
            Ok(if self.0.lock().unwrap().closed {
                TerminalPtyStatus::Exited(0)
            } else {
                TerminalPtyStatus::Running
            })
        }
        fn resize(&mut self, _: &TerminalDimensions) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn signal(&mut self, _: TerminalSignal) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn signal_may_discard_output(&self) -> bool {
            false
        }
        fn close(
            &mut self,
            _: bool,
            _: &mut dyn FnMut(&[u8]),
        ) -> std::result::Result<TerminalPtyClose, ()> {
            let mut state = self.0.lock().unwrap();
            state.close_attempts += 1;
            state.close_threads.push(std::thread::current().id());
            if state.close_failures > 0 {
                state.close_failures -= 1;
                return Err(());
            }
            state.closed = true;
            Ok(TerminalPtyClose {
                status: TerminalPtyStatus::Exited(0),
                output_incomplete: false,
            })
        }
    }

    struct Fixture {
        path: PathBuf,
        spawner: Arc<TestSpawner>,
        initialized: Arc<AtomicUsize>,
        worker_thread: Arc<Mutex<Option<ThreadId>>>,
        backend: Arc<Mutex<BackendState>>,
    }
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "machine-god-runtime-test-{:032x}",
                u128::from_le_bytes(random)
            ));
            DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self {
                path,
                spawner: Arc::new(TestSpawner::default()),
                initialized: Arc::new(AtomicUsize::new(0)),
                worker_thread: Arc::new(Mutex::new(None)),
                backend: Arc::new(Mutex::new(BackendState::default())),
            }
        }
        fn initializer(
            &self,
            session: bool,
        ) -> impl FnOnce() -> Result<TerminalRuntimeWorker<Backend>> + Send + 'static {
            let path = self.path.clone();
            let initialized = Arc::clone(&self.initialized);
            let worker_thread = Arc::clone(&self.worker_thread);
            let backend = Arc::clone(&self.backend);
            move || {
                initialized.fetch_add(1, Ordering::Relaxed);
                *worker_thread.lock().unwrap() = Some(std::thread::current().id());
                let fd: OwnedFd = rustix::fs::open(
                    path,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                    Mode::empty(),
                )
                .unwrap();
                let store = TerminalProfileStore::prepare(fd).unwrap();
                let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
                let mut registry = TerminalRegistry::new("/workspace".into()).unwrap();
                if session {
                    add_session(&mut registry, &store, budget, Backend(backend));
                }
                Ok(TerminalRuntimeWorker::new(
                    registry,
                    store,
                    budget,
                    || 0,
                    |_| {},
                ))
            }
        }
        fn runtime(&self, session: bool) -> TerminalRuntime<Backend> {
            TerminalRuntime::new(self.initializer(session), self.spawner.clone())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.spawner.collect();
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn add_session(
        registry: &mut TerminalRegistry<Backend>,
        store: &TerminalProfileStore,
        budget: TerminalProfileBudget,
        backend: Backend,
    ) {
        let owner = BackgroundOutputOwner::new(
            SessionId::new("owner").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        );
        let id = TerminalSessionId::new("runtime-session").unwrap();
        let mut transaction = store.transaction().unwrap();
        let mut catalog = transaction
            .prepare_catalog("/workspace".into(), owner.clone())
            .unwrap();
        drop(transaction.create_session(&mut catalog, &id).unwrap());
        let namespace = catalog.namespace_key();
        let created = budget
            .create_journal(
                &mut transaction,
                namespace,
                &id,
                TerminalJournalLimits::default(),
            )
            .unwrap();
        created.accounting.unwrap();
        let mut context = TerminalProfileMutationContext::new(&mut transaction, budget, namespace);
        let history = TerminalHistory::create_with(
            &mut context,
            created.operation.unwrap(),
            &TerminalDimensions::new(3, 20).unwrap(),
        )
        .unwrap();
        let mut session = TerminalSession::new_with(
            &mut context,
            backend,
            history,
            owner.clone(),
            id.clone(),
            test_metadata(),
            0,
        )
        .unwrap();
        session.shell_ready_with(&mut context, 0).unwrap();
        registry.start(owner, id, || Ok(session)).unwrap();
    }
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    fn poll<T>(future: &mut (impl Future<Output = T> + Unpin)) -> Poll<T> {
        let waker = Waker::from(Arc::new(Noop));
        Pin::new(future).poll(&mut Context::from_waker(&waker))
    }

    #[test]
    fn construction_unpolled_and_pre_cancelled_requests_never_spawn() {
        let fixture = Fixture::new();
        let runtime = fixture.runtime(true);
        drop(runtime.request(CancellationToken::new(), |_, _, _| ()));
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            futures_executor::block_on(
                runtime.request(cancellation, |_, _, _| panic!("cancelled operation"))
            ),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Cancelled))
        );
        assert_eq!(fixture.spawner.calls.load(Ordering::Acquire), 0);
        assert_eq!(fixture.initialized.load(Ordering::Acquire), 0);
        assert!(!fixture.path.join("terminal-v1").exists());
        drop(runtime);
    }

    #[test]
    fn cloned_hosts_share_one_lazy_worker_and_explicit_profile() {
        let fixture = Fixture::new();
        let runtime = fixture.runtime(false);
        let second_host = runtime.clone();
        let thread = futures_executor::block_on(runtime.request_with_profile(
            CancellationToken::new(),
            |_, store, _, _, _| {
                assert!(store.transaction().is_ok());
                std::thread::current().id()
            },
        ))
        .unwrap();
        drop(runtime);
        assert_eq!(
            futures_executor::block_on(second_host.request_with_waits(
                CancellationToken::new(),
                |_, store, _, _, _, _| {
                    assert!(store.transaction().is_ok());
                    42
                }
            )),
            Ok(42)
        );
        assert_ne!(thread, std::thread::current().id());
        assert_eq!(fixture.spawner.calls.load(Ordering::Acquire), 1);
        assert_eq!(fixture.initialized.load(Ordering::Acquire), 1);
        drop(second_host);
        fixture.spawner.collect();
    }

    #[test]
    fn last_host_drop_closes_native_sessions_while_unpolled_future_survives() {
        let fixture = Fixture::new();
        let runtime = fixture.runtime(true);
        futures_executor::block_on(runtime.request(CancellationToken::new(), |_, _, _| ()))
            .unwrap();
        let future = runtime.request(CancellationToken::new(), |_, _, _| panic!("closed request"));
        drop(runtime);
        fixture.spawner.collect();
        let state = fixture.backend.lock().unwrap();
        assert_eq!(state.close_attempts, 1);
        assert_eq!(state.dropped_on, *fixture.worker_thread.lock().unwrap());
        assert_ne!(state.dropped_on, Some(std::thread::current().id()));
        drop(state);
        assert_eq!(
            futures_executor::block_on(future),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Closed))
        );
    }

    #[test]
    fn host_drop_before_worker_release_rejects_queue_without_initializing() {
        let fixture = Fixture::new();
        let (release, gate) = sync_channel(1);
        *fixture.spawner.gate.lock().unwrap() = Some(gate);
        let runtime = fixture.runtime(true);
        let mut future = runtime.request(CancellationToken::new(), |_, _, _| {
            panic!("closed queued request")
        });
        let wake = Arc::new(ReentrantWake {
            shared: Arc::downgrade(&runtime.shared),
            woke: AtomicBool::new(false),
        });
        let waker = Waker::from(Arc::clone(&wake));
        assert!(
            Pin::new(&mut future)
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        drop(runtime);
        assert!(wake.woke.load(Ordering::Acquire));
        assert_eq!(
            futures_executor::block_on(future),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Closed))
        );
        release.send(()).unwrap();
        fixture.spawner.collect();
        assert_eq!(fixture.initialized.load(Ordering::Acquire), 0);
        assert!(!fixture.path.join("terminal-v1").exists());
    }

    struct ReentrantWake {
        shared: std::sync::Weak<Shared<Backend>>,
        woke: AtomicBool,
    }
    impl Wake for ReentrantWake {
        fn wake(self: Arc<Self>) {
            let shared = self.shared.upgrade().unwrap();
            assert!(
                shared.phase.try_lock().is_ok(),
                "wake held initialization mutex"
            );
            self.woke.store(true, Ordering::Release);
        }
    }

    #[test]
    fn startup_queue_is_bounded_and_does_not_create_extra_workers() {
        let fixture = Fixture::new();
        let (release, gate) = sync_channel(1);
        *fixture.spawner.gate.lock().unwrap() = Some(gate);
        let runtime = fixture.runtime(false);
        let mut pending = Vec::new();
        for _ in 0..32 {
            let mut future = runtime.request(CancellationToken::new(), |_, _, _| ());
            assert!(poll(&mut future).is_pending());
            pending.push(future);
        }
        let mut rejected = runtime.request(CancellationToken::new(), |_, _, _| ());
        assert_eq!(
            poll(&mut rejected),
            Poll::Ready(Err(TerminalRuntimeError::Owner(TerminalOwnerError::Busy)))
        );
        assert_eq!(fixture.spawner.calls.load(Ordering::Acquire), 1);
        drop(pending);
        drop(runtime);
        release.send(()).unwrap();
        fixture.spawner.collect();
        assert_eq!(fixture.initialized.load(Ordering::Acquire), 0);
    }

    #[test]
    fn spawn_failure_is_shared_and_cannot_construct_native_authority() {
        let fixture = Fixture::new();
        fixture.spawner.reject.store(true, Ordering::Release);
        let runtime = fixture.runtime(true);
        for _ in 0..3 {
            assert_eq!(
                futures_executor::block_on(runtime.request(CancellationToken::new(), |_, _, _| ())),
                Err(TerminalRuntimeError::Spawn)
            );
        }
        assert_eq!(fixture.spawner.calls.load(Ordering::Acquire), 1);
        assert_eq!(fixture.initialized.load(Ordering::Acquire), 0);
        assert!(fixture.backend.lock().unwrap().dropped_on.is_none());
        drop(runtime);
    }

    #[test]
    fn initialization_failure_and_panic_resolve_every_queued_request() {
        for panicked in [false, true] {
            let fixture = Fixture::new();
            let (release, gate) = sync_channel(1);
            *fixture.spawner.gate.lock().unwrap() = Some(gate);
            let runtime: TerminalRuntime<Backend> = TerminalRuntime::new(
                move || {
                    assert!(!panicked, "initializer panic");
                    Err(TerminalRuntimeError::Initialization)
                },
                fixture.spawner.clone(),
            );
            let mut first = runtime.request(CancellationToken::new(), |_, _, _| ());
            let mut second = runtime.request(CancellationToken::new(), |_, _, _| ());
            assert!(poll(&mut first).is_pending());
            assert!(poll(&mut second).is_pending());
            release.send(()).unwrap();
            let expected = Err(if panicked {
                TerminalRuntimeError::Panicked
            } else {
                TerminalRuntimeError::Initialization
            });
            assert_eq!(futures_executor::block_on(first), expected);
            assert_eq!(futures_executor::block_on(second), expected);
            drop(runtime);
            fixture.spawner.collect();
        }
    }

    #[test]
    fn requesters_are_inert_and_cannot_reopen_a_dropped_host() {
        let fixture = Fixture::new();
        let runtime = fixture.runtime(false);
        let requester = runtime.requester();
        let cloned = requester.clone();
        let pending = requester.request_with_context(CancellationToken::new(), |_| {
            panic!("closed host initialized")
        });
        assert_eq!(fixture.spawner.calls.load(Ordering::SeqCst), 0);
        drop(runtime);
        assert!(matches!(
            futures_executor::block_on(pending),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Closed))
        ));
        assert!(matches!(
            futures_executor::block_on(
                cloned.request_with_context(CancellationToken::new(), |_| ())
            ),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Closed))
        ));
        assert_eq!(fixture.spawner.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn requesters_allow_followups_but_do_not_delay_native_shutdown() {
        let fixture = Fixture::new();
        let runtime = fixture.runtime(true);
        let requester = runtime.requester();
        let cloned = requester.clone();
        let first = futures_executor::block_on(
            requester.request_with_context(CancellationToken::new(), |context| context.now_ms),
        )
        .unwrap();
        let second = futures_executor::block_on(
            cloned.request_with_context(CancellationToken::new(), |context| context.now_ms),
        )
        .unwrap();
        assert!(second >= first);
        drop(runtime);
        fixture.spawner.collect();
        assert_eq!(fixture.backend.lock().unwrap().close_attempts, 1);
        assert!(matches!(
            futures_executor::block_on(
                requester.request_with_context(CancellationToken::new(), |_| ())
            ),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Closed))
        ));
    }

    #[test]
    fn scoped_runtime_is_lazy_and_closed_scope_prevents_initialization() {
        let fixture = Fixture::new();
        let scope = crate::NativeOwnedWorkerScope::new();
        let runtime = TerminalRuntime::new(fixture.initializer(true), Arc::new(scope.clone()));
        drop(runtime.request(CancellationToken::new(), |_, _, _| ()));
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        assert_eq!(fixture.initialized.load(Ordering::Acquire), 0);
        assert_eq!(
            futures_executor::block_on(runtime.request(CancellationToken::new(), |_, _, _| ())),
            Err(TerminalRuntimeError::Spawn)
        );
        assert_eq!(fixture.initialized.load(Ordering::Acquire), 0);
    }

    #[test]
    fn scoped_runtime_completion_collects_cleanup_without_future_or_requester_votes() {
        let fixture = Fixture::new();
        fixture.backend.lock().unwrap().close_failures = 2;
        let scope = crate::NativeOwnedWorkerScope::new();
        let runtime = TerminalRuntime::new(fixture.initializer(true), Arc::new(scope.clone()));
        futures_executor::block_on(runtime.request(CancellationToken::new(), |_, _, _| ()))
            .unwrap();
        let requester = runtime.requester();
        let response = requester.request_with_context(CancellationToken::new(), |_| ());
        scope.close();
        assert!(!scope.completion().is_complete());
        drop(runtime);
        scope.completion().wait_on_worker().unwrap();
        assert_eq!(fixture.backend.lock().unwrap().close_attempts, 3);
        assert!(fixture.backend.lock().unwrap().dropped_on.is_some());
        assert!(futures_executor::block_on(response).is_err());
        drop(requester);
    }

    #[test]
    fn shutdown_retry_observer_receives_exact_owned_cleanup_failures() {
        let fixture = Fixture::new();
        fixture.backend.lock().unwrap().close_failures = 2;
        let initialize = fixture.initializer(true);
        let observed = Arc::new(Mutex::new(Vec::new()));
        let worker_observed = Arc::clone(&observed);
        let runtime = TerminalRuntime::new(
            move || {
                let mut worker = initialize()?;
                worker.observer = Box::new(move |(), steps| {
                    for step in steps {
                        if let Err(error) = step.result {
                            worker_observed.lock().unwrap().push((
                                step.owner,
                                step.session_id,
                                error,
                                step.cleanup_error,
                                std::thread::current().id(),
                            ));
                        }
                    }
                });
                Ok(worker)
            },
            fixture.spawner.clone(),
        );
        futures_executor::block_on(runtime.request(CancellationToken::new(), |_, _, _| ()))
            .unwrap();
        drop(runtime);
        fixture.spawner.collect();
        let expected_owner = BackgroundOutputOwner::new(
            SessionId::new("owner").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        );
        let observed = observed.lock().unwrap();
        assert_eq!(observed.len(), 2);
        for (owner, id, error, cleanup_error, thread) in observed.iter() {
            assert_eq!(owner, &expected_owner);
            assert_eq!(id, &TerminalSessionId::new("runtime-session").unwrap());
            assert_eq!(
                *error,
                crate::terminal_session::TerminalSessionError::Native
            );
            assert_eq!(*cleanup_error, Some(*error));
            assert_eq!(Some(*thread), *fixture.worker_thread.lock().unwrap());
        }
    }

    #[test]
    fn shutdown_observer_panic_is_not_retried_and_does_not_interrupt_cleanup() {
        let fixture = Fixture::new();
        fixture.backend.lock().unwrap().close_failures = 2;
        let initialize = fixture.initializer(true);
        let diagnostics = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&diagnostics);
        let runtime = TerminalRuntime::new(
            move || {
                let mut worker = initialize()?;
                worker.observer = Box::new(move |(), steps| {
                    if steps.iter().any(|step| step.result.is_err()) {
                        observed.fetch_add(1, Ordering::AcqRel);
                        panic!("shutdown observer failure");
                    }
                });
                Ok(worker)
            },
            fixture.spawner.clone(),
        );
        futures_executor::block_on(runtime.request(CancellationToken::new(), |_, _, _| ()))
            .unwrap();
        runtime.shutdown();
        fixture.spawner.collect();
        assert_eq!(diagnostics.load(Ordering::Acquire), 1);
        assert_eq!(fixture.backend.lock().unwrap().close_attempts, 3);
        assert_eq!(
            runtime.shared.failure(),
            Some(TerminalRuntimeError::Panicked)
        );
    }

    #[test]
    fn failed_owner_observer_is_never_reentered_for_shutdown_diagnostics() {
        let fixture = Fixture::new();
        fixture.backend.lock().unwrap().close_failures = 2;
        let initialize = fixture.initializer(true);
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let runtime = TerminalRuntime::new(
            move || {
                let mut worker = initialize()?;
                worker.observer = Box::new(move |(), _| {
                    observed.fetch_add(1, Ordering::AcqRel);
                    panic!("owner observer failed");
                });
                Ok(worker)
            },
            fixture.spawner.clone(),
        );
        let _ = futures_executor::block_on(runtime.request(CancellationToken::new(), |_, _, _| ()));
        fixture.spawner.collect();
        assert_eq!(calls.load(Ordering::Acquire), 1);
        assert_eq!(fixture.backend.lock().unwrap().close_attempts, 3);
        assert_eq!(
            runtime.shared.failure(),
            Some(TerminalRuntimeError::Owner(TerminalOwnerError::Panicked))
        );
    }

    #[test]
    fn failed_native_shutdown_retries_on_same_worker_until_authority_is_released() {
        let fixture = Fixture::new();
        fixture.backend.lock().unwrap().close_failures = 2;
        let runtime = fixture.runtime(true);
        futures_executor::block_on(runtime.request(CancellationToken::new(), |_, _, _| ()))
            .unwrap();
        drop(runtime);
        fixture.spawner.collect();
        let state = fixture.backend.lock().unwrap();
        assert_eq!(state.close_attempts, 3);
        let worker = fixture.worker_thread.lock().unwrap().unwrap();
        assert!(state.close_threads.iter().all(|thread| *thread == worker));
        assert_eq!(state.dropped_on, Some(worker));
    }

    #[test]
    fn callback_failure_does_not_retain_worker_after_clean_shutdown() {
        let fixture = Fixture::new();
        let runtime = fixture.runtime(true);
        assert_eq!(
            futures_executor::block_on(
                runtime.request(CancellationToken::new(), |_, _, _| panic!(
                    "request failure"
                ))
            ),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Panicked))
        );
        fixture.spawner.collect();
        assert_eq!(fixture.backend.lock().unwrap().close_attempts, 1);
        assert_eq!(
            runtime.shared.failure(),
            Some(TerminalRuntimeError::Owner(TerminalOwnerError::Panicked))
        );
        drop(runtime);
    }

    #[test]
    fn owner_failure_survives_cleanup_retries_and_late_nonowning_requests() {
        let fixture = Fixture::new();
        fixture.backend.lock().unwrap().close_failures = 2;
        let runtime = fixture.runtime(true);
        let requester = runtime.requester();
        assert_eq!(
            futures_executor::block_on(
                requester.request_with_context(CancellationToken::new(), |_| panic!(
                    "request failure before cleanup retries"
                ),)
            ),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Panicked))
        );
        fixture.spawner.collect();
        assert_eq!(fixture.backend.lock().unwrap().close_attempts, 3);
        assert_eq!(
            runtime.shared.failure(),
            Some(TerminalRuntimeError::Owner(TerminalOwnerError::Panicked))
        );
        assert_eq!(
            futures_executor::block_on(
                requester.request_with_context(CancellationToken::new(), |_| panic!(
                    "failed owner must never accept new work"
                ),)
            ),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Panicked))
        );
    }

    #[derive(Default)]
    struct StateLog {
        initialized_on: Option<ThreadId>,
        accesses: Vec<ThreadId>,
        observations: usize,
        dropped_on: Option<ThreadId>,
        backend_dropped_before_state: bool,
        close_attempts_at_drop: usize,
    }

    // Deliberately !Send and !Sync. The queue and runtime handles remain Send
    // because they carry typed callbacks, never a HostState instance.
    struct HostState {
        counter: std::rc::Rc<std::cell::Cell<usize>>,
        catalog: crate::terminal_catalog::TerminalCatalog,
        log: Arc<Mutex<StateLog>>,
        backend: Arc<Mutex<BackendState>>,
        panic_on_drop: bool,
    }
    impl Drop for HostState {
        fn drop(&mut self) {
            let backend = self.backend.lock().unwrap();
            let mut log = self.log.lock().unwrap();
            log.dropped_on = Some(std::thread::current().id());
            log.backend_dropped_before_state = backend.dropped_on.is_some();
            log.close_attempts_at_drop = backend.close_attempts;
            drop(log);
            drop(backend);
            assert!(!self.panic_on_drop, "state destructor panic");
        }
    }

    impl Fixture {
        fn state_initializer(
            &self,
            session: bool,
            log: Arc<Mutex<StateLog>>,
            panic_on_drop: bool,
            panic_in_observer: bool,
        ) -> impl FnOnce() -> Result<TerminalRuntimeWorker<Backend, HostState>> + Send + 'static
        {
            let initialize = self.initializer(session);
            let backend = Arc::clone(&self.backend);
            move || {
                let TerminalRuntimeWorker {
                    registry,
                    store,
                    budget,
                    clock,
                    ..
                } = initialize()?;
                let owner = BackgroundOutputOwner::new(
                    SessionId::new("owner").unwrap(),
                    SessionIncarnationId::new("incarnation").unwrap(),
                );
                let catalog = store
                    .transaction()
                    .unwrap()
                    .prepare_catalog("/workspace".into(), owner)
                    .unwrap();
                log.lock().unwrap().initialized_on = Some(std::thread::current().id());
                let state = HostState {
                    counter: std::rc::Rc::new(std::cell::Cell::new(0)),
                    catalog,
                    log,
                    backend,
                    panic_on_drop,
                };
                Ok(TerminalRuntimeWorker::new_with_state(
                    registry,
                    store,
                    budget,
                    state,
                    clock,
                    move |state, _| {
                        state.log.lock().unwrap().observations += 1;
                        assert!(!panic_in_observer, "state observer panic");
                    },
                ))
            }
        }

        fn state_runtime(
            &self,
            session: bool,
            log: Arc<Mutex<StateLog>>,
        ) -> TerminalRuntime<Backend, HostState> {
            TerminalRuntime::new(
                self.state_initializer(session, log, false, false),
                self.spawner.clone(),
            )
        }
    }

    #[test]
    fn typed_non_send_state_is_initialized_accessed_and_destroyed_on_one_worker() {
        fn send_sync<T: Send + Sync>(_: &T) {}
        fn send<T: Send>(_: &T) {}
        let fixture = Fixture::new();
        let log = Arc::new(Mutex::new(StateLog::default()));
        let runtime = fixture.state_runtime(true, Arc::clone(&log));
        send_sync(&runtime);
        for expected in 1..=3 {
            let request = runtime.request_with_context(CancellationToken::new(), |context| {
                let state = context.state;
                state
                    .log
                    .lock()
                    .unwrap()
                    .accesses
                    .push(std::thread::current().id());
                state.counter.set(state.counter.get() + 1);
                context
                    .store
                    .transaction()
                    .unwrap()
                    .validate_catalog(&state.catalog)
                    .unwrap();
                assert_eq!(context.registry.workspace(), "/workspace");
                assert!(context.budget.output_limit() > 0);
                assert!(context.waits.next_deadline().is_none());
                assert!(context.writes.observe(context.registry));
                assert_eq!(context.now_ms, 0);
                assert!(!context.cancellation.is_cancelled());
                state.counter.get()
            });
            send(&request);
            assert_eq!(futures_executor::block_on(request), Ok(expected));
        }
        assert!(log.lock().unwrap().dropped_on.is_none());
        drop(runtime);
        fixture.spawner.collect();
        let log = log.lock().unwrap();
        assert_ne!(log.initialized_on, Some(std::thread::current().id()));
        assert_eq!(log.dropped_on, log.initialized_on);
        assert!(
            log.accesses
                .iter()
                .all(|thread| Some(*thread) == log.initialized_on)
        );
        assert!(log.observations > 0);
        assert!(log.backend_dropped_before_state);
    }

    #[test]
    fn typed_unpolled_cancelled_and_early_closed_requests_never_construct_state() {
        let fixture = Fixture::new();
        let log = Arc::new(Mutex::new(StateLog::default()));
        let runtime = fixture.state_runtime(true, Arc::clone(&log));
        drop(runtime.request_with_context(CancellationToken::new(), |_| panic!("unpolled")));
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            futures_executor::block_on(runtime.request_with_context(cancellation, |_| ())),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Cancelled))
        );
        runtime.shutdown();
        assert_eq!(
            futures_executor::block_on(
                runtime.request_with_context(CancellationToken::new(), |_| ())
            ),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Closed))
        );
        drop(runtime);
        assert_eq!(fixture.spawner.calls.load(Ordering::Acquire), 0);
        assert!(log.lock().unwrap().initialized_on.is_none());
        assert!(log.lock().unwrap().dropped_on.is_none());
    }

    #[test]
    fn typed_future_does_not_retain_state_after_last_host_drop() {
        let fixture = Fixture::new();
        let log = Arc::new(Mutex::new(StateLog::default()));
        let runtime = fixture.state_runtime(true, Arc::clone(&log));
        futures_executor::block_on(runtime.request_with_context(CancellationToken::new(), |_| ()))
            .unwrap();
        let future =
            runtime.request_with_context(CancellationToken::new(), |_| panic!("closed future"));
        drop(runtime);
        fixture.spawner.collect();
        assert!(log.lock().unwrap().backend_dropped_before_state);
        assert_eq!(
            futures_executor::block_on(future),
            Err(TerminalRuntimeError::Owner(TerminalOwnerError::Closed))
        );
    }

    #[test]
    fn typed_state_survives_every_failed_cleanup_attempt() {
        let fixture = Fixture::new();
        fixture.backend.lock().unwrap().close_failures = 2;
        let log = Arc::new(Mutex::new(StateLog::default()));
        let runtime = fixture.state_runtime(true, Arc::clone(&log));
        futures_executor::block_on(runtime.request_with_context(CancellationToken::new(), |_| ()))
            .unwrap();
        drop(runtime);
        fixture.spawner.collect();
        let log = log.lock().unwrap();
        assert_eq!(log.close_attempts_at_drop, 3);
        assert!(log.backend_dropped_before_state);
        assert_eq!(log.dropped_on, log.initialized_on);
    }

    #[test]
    fn typed_committed_receipt_wins_cancellation_and_state_remains_available() {
        let fixture = Fixture::new();
        let log = Arc::new(Mutex::new(StateLog::default()));
        let runtime = fixture.state_runtime(false, log);
        let caller = CancellationToken::new();
        let cancel = caller.clone();
        assert_eq!(
            futures_executor::block_on(runtime.request_with_context(caller, move |context| {
                context.state.counter.set(42);
                cancel.cancel();
                42
            })),
            Ok(42)
        );
        assert_eq!(
            futures_executor::block_on(
                runtime.request_with_context(CancellationToken::new(), |context| context
                    .state
                    .counter
                    .get())
            ),
            Ok(42)
        );
        drop(runtime);
        fixture.spawner.collect();
    }

    #[test]
    fn typed_spawn_and_gated_early_close_failures_never_construct_state() {
        for rejected in [false, true] {
            let fixture = Fixture::new();
            fixture.spawner.reject.store(rejected, Ordering::Release);
            let (release, gate) = sync_channel(1);
            if !rejected {
                *fixture.spawner.gate.lock().unwrap() = Some(gate);
            }
            let log = Arc::new(Mutex::new(StateLog::default()));
            let runtime = fixture.state_runtime(true, Arc::clone(&log));
            let mut request =
                runtime.request_with_context(CancellationToken::new(), |_| panic!("not admitted"));
            let first = poll(&mut request);
            if rejected {
                assert_eq!(first, Poll::Ready(Err(TerminalRuntimeError::Spawn)));
            } else {
                assert!(first.is_pending());
            }
            drop(runtime);
            if !rejected {
                assert_eq!(
                    futures_executor::block_on(request),
                    Err(TerminalRuntimeError::Owner(TerminalOwnerError::Closed))
                );
                release.send(()).unwrap();
            }
            fixture.spawner.collect();
            assert!(log.lock().unwrap().initialized_on.is_none());
        }
    }

    #[test]
    fn typed_initialization_error_and_unwind_destroy_backend_before_state() {
        for panicked in [false, true] {
            let fixture = Fixture::new();
            let log = Arc::new(Mutex::new(StateLog::default()));
            let initialize = fixture.state_initializer(true, Arc::clone(&log), panicked, false);
            let runtime: TerminalRuntime<Backend, HostState> = TerminalRuntime::new(
                move || {
                    let _worker = initialize()?;
                    assert!(!panicked, "initialization fails after state construction");
                    Err(TerminalRuntimeError::Initialization)
                },
                fixture.spawner.clone(),
            );
            let expected = if panicked {
                TerminalRuntimeError::Panicked
            } else {
                TerminalRuntimeError::Initialization
            };
            assert_eq!(
                futures_executor::block_on(
                    runtime.request_with_context(CancellationToken::new(), |_| ())
                ),
                Err(expected)
            );
            drop(runtime);
            fixture.spawner.collect();
            let log = log.lock().unwrap();
            assert!(log.backend_dropped_before_state);
            assert_eq!(log.dropped_on, log.initialized_on);
            assert_ne!(log.dropped_on, Some(std::thread::current().id()));
        }
    }

    #[test]
    fn typed_callback_and_observer_panics_cleanup_before_state_destruction() {
        for observer_panics in [false, true] {
            let fixture = Fixture::new();
            let log = Arc::new(Mutex::new(StateLog::default()));
            let runtime = TerminalRuntime::new(
                fixture.state_initializer(true, Arc::clone(&log), true, observer_panics),
                fixture.spawner.clone(),
            );
            assert!(
                futures_executor::block_on(
                    runtime.request_with_context(CancellationToken::new(), |_| panic!(
                        "state callback panic"
                    ))
                )
                .is_err()
            );
            fixture.spawner.collect();
            let log = log.lock().unwrap();
            assert!(log.backend_dropped_before_state);
            assert_eq!(log.dropped_on, log.initialized_on);
            drop(runtime);
        }
    }
}
