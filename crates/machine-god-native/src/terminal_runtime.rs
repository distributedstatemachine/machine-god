//! Lazy assembly of one terminal owner on an explicitly owned blocking worker.
//! No native registry, profile, backend, or thread is created by construction or
//! an unpolled request. Futures retain request state, never a host lifetime vote.

use crate::terminal_owner::{
    TerminalOwnerError, TerminalOwnerFuture, TerminalOwnerHandle, TerminalOwnerLoop,
};
use crate::terminal_profile::TerminalProfileBudget;
use crate::terminal_profile_store::TerminalProfileStore;
use crate::terminal_registry::{TerminalRegistry, TerminalRegistryStep};
use crate::terminal_session::TerminalSessionBackend;
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
type Initializer<B> = Box<dyn FnOnce() -> Result<TerminalRuntimeWorker<B>> + Send + 'static>;

fn contain<T>(operation: impl FnOnce() -> T) -> std::result::Result<T, ()> {
    catch_unwind(AssertUnwindSafe(operation)).map_err(|payload| {
        // Opaque panic payloads may themselves have hostile destructors.
        std::mem::forget(payload);
    })
}

struct Starter<B: TerminalSessionBackend> {
    owner: TerminalOwnerLoop<B>,
    initialize: Initializer<B>,
}
enum Phase<B: TerminalSessionBackend> {
    Dormant(Starter<B>),
    Starting(Arc<Mutex<Option<Starter<B>>>>),
    Running,
    Failed(TerminalRuntimeError),
    Stopped,
}
struct Shared<B: TerminalSessionBackend> {
    phase: Mutex<Phase<B>>,
    spawner: Arc<dyn TerminalRuntimeSpawner>,
    hosts: AtomicUsize,
    closing: AtomicBool,
}
impl<B: TerminalSessionBackend + Send + 'static> Shared<B> {
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

    fn set_phase(&self, phase: Phase<B>) {
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
                    worker.run(owner);
                    shared.set_phase(Phase::Stopped);
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
pub(crate) struct TerminalRuntime<B: TerminalSessionBackend + Send + 'static> {
    shared: Arc<Shared<B>>,
    owner: TerminalOwnerHandle<B>,
}
impl<B: TerminalSessionBackend + Send + 'static> Clone for TerminalRuntime<B> {
    fn clone(&self) -> Self {
        self.shared.hosts.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
            owner: self.owner.clone(),
        }
    }
}
impl<B: TerminalSessionBackend + Send + 'static> Drop for TerminalRuntime<B> {
    fn drop(&mut self) {
        if self.shared.hosts.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.owner.shutdown();
            self.shared.close();
        }
    }
}
impl<B: TerminalSessionBackend + Send + 'static> fmt::Debug for TerminalRuntime<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalRuntime")
            .finish_non_exhaustive()
    }
}
impl<B: TerminalSessionBackend + Send + 'static> TerminalRuntime<B> {
    pub(crate) fn new(
        initialize: impl FnOnce() -> Result<TerminalRuntimeWorker<B>> + Send + 'static,
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

    pub(crate) fn request<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(&mut TerminalRegistry<B>, i64, &CancellationToken) -> T + Send + 'static,
    ) -> TerminalRuntimeFuture<B, T> {
        self.wrap(self.owner.request(caller, operation))
    }

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
    ) -> TerminalRuntimeFuture<B, T> {
        self.wrap(self.owner.request_with_profile(caller, operation))
    }

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
    ) -> TerminalRuntimeFuture<B, T> {
        self.wrap(self.owner.request_with_waits(caller, operation))
    }

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
    ) -> TerminalRuntimeFuture<B, T> {
        self.wrap(self.owner.request_with_writes(caller, operation))
    }

    fn wrap<T>(&self, request: TerminalOwnerFuture<B, T>) -> TerminalRuntimeFuture<B, T> {
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

pub(crate) struct TerminalRuntimeFuture<B: TerminalSessionBackend, T> {
    shared: Arc<Shared<B>>,
    request: TerminalOwnerFuture<B, T>,
    finished: bool,
}
impl<B: TerminalSessionBackend + Send + 'static, T: Send + 'static> Future
    for TerminalRuntimeFuture<B, T>
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
pub(crate) struct TerminalRuntimeWorker<B: TerminalSessionBackend> {
    registry: TerminalRegistry<B>,
    store: TerminalProfileStore,
    budget: TerminalProfileBudget,
    clock: Box<dyn FnMut() -> i64 + Send>,
    observer: Box<dyn FnMut(Vec<TerminalRegistryStep>) + Send>,
}
impl<B: TerminalSessionBackend> TerminalRuntimeWorker<B> {
    pub(crate) fn new(
        registry: TerminalRegistry<B>,
        store: TerminalProfileStore,
        budget: TerminalProfileBudget,
        clock: impl FnMut() -> i64 + Send + 'static,
        observer: impl FnMut(Vec<TerminalRegistryStep>) + Send + 'static,
    ) -> Self {
        Self {
            registry,
            store,
            budget,
            clock: Box::new(clock),
            observer: Box::new(observer),
        }
    }

    fn run(mut self, owner: TerminalOwnerLoop<B>) {
        let exit = contain(|| {
            owner.run_with_profile(
                &mut self.registry,
                &self.store,
                &self.budget,
                &mut self.clock,
                &mut self.observer,
            )
        });
        if exit.is_ok_and(|exit| exit.shutdown.is_ok_and(|failures| failures.is_empty())) {
            return;
        }
        // An old callback/clock error does not keep a clean registry alive.
        // Re-evaluate only current shutdown obligations, without polling more
        // user callbacks or promoting numeric persisted process identities.
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
            if shutdown.is_ok_and(|result| result.is_ok_and(|failures| failures.is_empty())) {
                break;
            }
            backoff = backoff.saturating_mul(2).min(CLEANUP_MAX_BACKOFF);
        }
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
        drop(runtime);
    }
}
