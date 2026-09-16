use super::*;
use crate::interactive_session::tests::support::Fixture;
use crate::interactive_session::{
    NativeInteractiveOutcome, NativeInteractiveTransition, transition,
};
use crate::mcp::runtime::NativeMcpRuntimeClock;
use futures_util::future::poll_fn;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[derive(Default)]
struct Clock(AtomicBool);
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if !self.0.load(Ordering::Acquire) {
                std::future::pending::<()>().await;
            }
        })
    }
}

fn fixture() -> (Fixture, Arc<Clock>) {
    let clock = Arc::new(Clock::default());
    let selected = clock.clone();
    let fixture = Fixture::with_workspace_options(move |options| {
        options
            .with_mcp_runtime(crate::NativeReferenceHostMcpOptions::new(
                Arc::new(crate::mcp::context::NativeMcpContexts::new()),
                selected.clone(),
            ))
            .with_managed_agents(crate::NativeReferenceHostManagedOptions::new(selected))
    });
    (fixture, clock)
}
fn options(fixture: &Fixture) -> NativeInteractiveSessionOptions {
    NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        fixture.host.loaded_config().config().model_preferences(),
    )
    .unwrap()
}
fn journal(fixture: &Fixture) -> rustix::fd::OwnedFd {
    use std::os::unix::fs::DirBuilderExt;
    let path = fixture.state_root().join("foreground-custody");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    std::fs::File::open(path).unwrap().into()
}
async fn startup(
    fixture: &mut Fixture,
    options: NativeInteractiveSessionOptions,
) -> NativeManagedInteractiveStartup {
    let directory = journal(fixture);
    let agents = Arc::get_mut(&mut fixture.host)
        .unwrap()
        .open_managed_agents(directory, options.defaults.clone(), options.origin)
        .await
        .unwrap();
    NativeManagedInteractiveStartup::new(fixture.host.clone(), options, agents).unwrap()
}
fn reserve_cleanup(fixture: &Fixture) -> Vec<crate::owned_worker::NativeOwnedWorkerRun> {
    let workers = fixture.host.control_workers().unwrap();
    (0..64)
        .map(|_| {
            workers
                .begin_cleanup_run_with_keepalive(Arc::new(()))
                .unwrap()
        })
        .collect()
}
fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}
async fn outcome(owner: &mut NativeInteractiveSession) -> NativeInteractiveOutcome {
    poll_fn(|cx| {
        let result = owner.poll_progress(cx, 3);
        if let Some(outcome) = owner.take_outcome() {
            return Poll::Ready(outcome);
        }
        if result.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await
}

#[test]
fn ordinary_initial_preparation_timeout_retains_startup_and_reservation() {
    run(async {
        let (mut fixture, clock) = fixture();
        let mut options = options(&fixture);
        // Fail after creating the actual runtime, in foreground configuration.
        options.process_model = Some(String::new());
        let mut startup = startup(&mut fixture, options).await;
        let reserve = reserve_cleanup(&fixture);
        clock.0.store(true, Ordering::Release);
        startup
            .request_open(NativeInteractiveInitialSession::Fresh, 1)
            .unwrap();
        assert!(poll_fn(|cx| startup.poll_open(cx, 2)).await.is_err());
        let State::Fenced { owner, .. } = &mut startup.state else {
            panic!("unsettled candidate must fence startup");
        };
        assert!(
            owner
                .agents
                .poll_shutdown(&mut Context::from_waker(std::task::Waker::noop()), 3)
                .is_pending()
        );
        drop(reserve);
        clock.0.store(false, Ordering::Release);
        assert!(
            startup
                .request_open(NativeInteractiveInitialSession::Fresh, 3)
                .is_err()
        );
        startup.request_shutdown();
        for _ in 0..3 {
            assert!(poll_fn(|cx| startup.poll_open(cx, 4)).await.is_err());
            assert!(!startup.is_finished());
        }
        drop(startup);
        fixture.finish();
    });
}

#[test]
fn initial_enrollment_rejection_retains_candidate_until_cleanup_settles() {
    run(async {
        let (mut fixture, clock) = fixture();
        let options = options(&fixture);
        let mut startup = startup(&mut fixture, options.clone()).await;
        let State::Idle(owner) = std::mem::replace(&mut startup.state, State::Finished) else {
            unreachable!()
        };
        let mut owner = Some(owner);
        let selected = owner.as_mut().unwrap();
        let reservation =
            managed::reserve_initial(&mut selected.agents, 1, &CancellationToken::new())
                .await
                .unwrap();
        let conversation =
            transition::prepare(&fixture.host, &options, NativeInteractiveTransition::New, 1)
                .await
                .unwrap();
        let prepared = managed::prepare(
            &selected.agents,
            fixture.host.clone(),
            &options,
            conversation,
            managed::Selection {
                reservation,
                workspace: None,
                policy: None,
                catalog: None,
                initial: true,
                now_ms: 1,
                cancellation: CancellationToken::new(),
            },
        )
        .await
        .unwrap();
        selected.agents.request_shutdown();
        let (error, prepared, reservation) = managed::enroll(&mut owner, prepared).unwrap_err();
        let runtime = Arc::downgrade(&prepared.runtime);
        let failure = managed::staged::Failure::prepared(error, prepared, reservation);
        let reserve = reserve_cleanup(&fixture);
        clock.0.store(true, Ordering::Release);
        startup.state = State::Opening {
            future: Box::pin(async move { Err(rejected(owner.unwrap(), failure).await) }),
            cancellation: CancellationToken::new(),
        };
        assert!(poll_fn(|cx| startup.poll_open(cx, 2)).await.is_err());
        assert!(matches!(startup.state, State::Fenced { .. }));
        assert!(runtime.upgrade().is_some());
        drop(reserve);
        clock.0.store(false, Ordering::Release);
        startup.request_shutdown();
        assert!(poll_fn(|cx| startup.poll_open(cx, 3)).await.is_err());
        assert!(!startup.is_finished());
        assert!(runtime.upgrade().is_some());
        drop(startup);
        assert!(runtime.upgrade().is_none());
        fixture.finish();
    });
}

#[test]
fn ordinary_initial_error_with_settled_cleanup_is_retryable() {
    run(async {
        let (mut fixture, _) = fixture();
        let mut options = options(&fixture);
        options.process_model = Some(String::new());
        let mut startup = startup(&mut fixture, options).await;
        startup
            .request_open(NativeInteractiveInitialSession::Fresh, 1)
            .unwrap();
        assert!(poll_fn(|cx| startup.poll_open(cx, 2)).await.is_err());
        assert!(matches!(startup.state, State::Idle(_)));
        startup.options.process_model = None;
        startup
            .request_open(NativeInteractiveInitialSession::Fresh, 3)
            .unwrap();
        let mut owner = poll_fn(|cx| startup.poll_open(cx, 3))
            .await
            .unwrap()
            .unwrap();
        drop(startup);
        owner.request_shutdown();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Shutdown
        ));
        drop(owner);
        fixture.finish();
    });
}

#[test]
fn public_managed_open_failure_returns_unsettled_startup_custody() {
    run(async {
        let (fixture, clock) = fixture();
        let mut options = options(&fixture);
        options.process_model = Some(String::new());
        let directory = journal(&fixture);
        let reserve = reserve_cleanup(&fixture);
        let completion = fixture.host.terminal_shutdown_completion().unwrap();
        clock.0.store(true, Ordering::Release);
        let host = Arc::try_unwrap(fixture.host).unwrap();
        let failure = NativeInteractiveSession::open_managed(
            host,
            directory,
            options,
            NativeInteractiveInitialSession::Fresh,
            1,
        )
        .await
        .unwrap_err();
        let mut startup = failure
            .startup
            .expect("public error must retain its manager and failed candidate");
        assert!(matches!(startup.state, State::Fenced { .. }));
        drop(reserve);
        clock.0.store(false, Ordering::Release);
        startup.request_shutdown();
        assert!(poll_fn(|cx| startup.poll_open(cx, 3)).await.is_err());
        assert!(!startup.is_finished());
        drop(startup);
        completion.wait().await;
    });
}

async fn opened(fixture: &mut Fixture) -> NativeInteractiveSession {
    let selected = options(fixture);
    let mut startup = startup(fixture, selected).await;
    startup
        .request_open(NativeInteractiveInitialSession::Fresh, 1)
        .unwrap();
    poll_fn(|cx| startup.poll_open(cx, 2))
        .await
        .unwrap()
        .unwrap()
}

async fn preparation(
    owner: &mut NativeInteractiveSession,
    conversation: crate::NativeConversation,
) -> BoxFuture<'static, Result<managed::Prepared, managed::staged::Failure>> {
    let agents = &mut owner.managed.as_mut().unwrap().agents;
    let reservation = managed::reserve_initial(agents, 2, &CancellationToken::new())
        .await
        .unwrap();
    managed::prepare(
        agents,
        owner.host.clone(),
        &owner.options,
        conversation,
        managed::Selection {
            reservation,
            workspace: None,
            policy: None,
            catalog: None,
            initial: false,
            now_ms: 2,
            cancellation: CancellationToken::new(),
        },
    )
}
fn install(owner: &mut NativeInteractiveSession, phase: transition::Phase) {
    owner
        .request_transition(NativeInteractiveTransition::New, 2)
        .unwrap();
    let mut transition = transition::Transition::new(owner.pending.take().unwrap());
    transition.guard = Some(owner.quiesce_current().unwrap());
    transition.phase = phase;
    owner.transition = Some(transition);
}

#[test]
fn ordinary_preparation_failure_fences_only_when_original_cleanup_fails() {
    for timeout in [false, true] {
        run(async {
            let (mut fixture, clock) = fixture();
            let mut owner = opened(&mut fixture).await;
            let original = owner.current.id();
            let conversation = transition::prepare(
                &owner.host,
                &owner.options,
                NativeInteractiveTransition::New,
                2,
            )
            .await
            .unwrap();
            let block = fixture.block_publication(&conversation.id());
            let future = preparation(&mut owner, conversation).await;
            let reserve = timeout.then(|| reserve_cleanup(&fixture));
            clock.0.store(timeout, Ordering::Release);
            install(&mut owner, transition::Phase::ComposingStaged(future));
            let result = outcome(&mut owner).await;
            assert_eq!(owner.current.id(), original);
            drop(block);
            if timeout {
                assert!(matches!(
                    result,
                    NativeInteractiveOutcome::Indeterminate { .. }
                ));
                assert!(owner.is_fenced());
                drop(reserve);
                clock.0.store(false, Ordering::Release);
                owner.request_shutdown();
                for _ in 0..3 {
                    let _ =
                        owner.poll_progress(&mut Context::from_waker(std::task::Waker::noop()), 4);
                    assert!(!owner.is_closed());
                    assert!(owner.is_fenced());
                }
                assert!(owner.shutdown_error().is_some());
            } else {
                assert!(matches!(result, NativeInteractiveOutcome::Rejected { .. }));
                assert!(!owner.is_fenced());
                owner
                    .request_transition(NativeInteractiveTransition::New, 4)
                    .unwrap();
                assert!(matches!(
                    outcome(&mut owner).await,
                    NativeInteractiveOutcome::Transition(_)
                ));
                owner.request_shutdown();
                assert!(matches!(
                    outcome(&mut owner).await,
                    NativeInteractiveOutcome::Shutdown
                ));
            }
            drop(owner);
            fixture.finish();
        });
    }
}

#[test]
fn ordinary_enrollment_rejection_retains_runtime_through_worker_tls_timeout() {
    struct Tls {
        entered: Option<tokio::sync::oneshot::Sender<()>>,
        receiver: std::sync::mpsc::Receiver<()>,
    }
    impl Drop for Tls {
        fn drop(&mut self) {
            let _ = self.entered.take().unwrap().send(());
            let _ = self
                .receiver
                .recv_timeout(std::time::Duration::from_secs(10));
        }
    }
    thread_local! { static TLS: std::cell::RefCell<Option<Tls>> = const { std::cell::RefCell::new(None) }; }
    run(async {
        let (mut fixture, clock) = fixture();
        let mut owner = opened(&mut fixture).await;
        let conversation = transition::prepare(
            &owner.host,
            &owner.options,
            NativeInteractiveTransition::New,
            2,
        )
        .await
        .unwrap();
        let managed::Prepared::Managed(prepared, reservation) =
            preparation(&mut owner, conversation).await.await.unwrap()
        else {
            panic!("managed candidate");
        };
        let runtime = Arc::downgrade(&prepared.runtime);
        let admission = prepared.owner.binding().prepare_admission().unwrap();
        let completion = admission.cohort().unwrap().completion();
        let (release, receiver) = std::sync::mpsc::channel();
        let (entered, waiting) = tokio::sync::oneshot::channel();
        admission
            .cohort()
            .unwrap()
            .with_poll(|| {
                fixture.host.control_workers().unwrap().spawn(move || {
                    TLS.with(|tls| {
                        *tls.borrow_mut() = Some(Tls {
                            entered: Some(entered),
                            receiver,
                        });
                    });
                })
            })
            .unwrap();
        drop(admission);
        waiting.await.unwrap();
        assert!(!completion.is_complete());
        install(
            &mut owner,
            transition::Phase::Composing(Box::pin(async move {
                Ok(managed::Prepared::Managed(prepared, reservation))
            })),
        );
        // Admission now rejects this prepared candidate, not just its label.
        owner.managed.as_mut().unwrap().agents.request_shutdown();
        clock.0.store(true, Ordering::Release);
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Indeterminate { .. }
        ));
        assert!(owner.is_fenced());
        assert!(runtime.upgrade().is_some());
        release.send(()).unwrap();
        completion.wait().await;
        clock.0.store(false, Ordering::Release);
        owner.request_shutdown();
        let _ = owner.poll_progress(&mut Context::from_waker(std::task::Waker::noop()), 4);
        assert!(!owner.is_closed());
        assert!(owner.shutdown_error().is_some());
        assert!(runtime.upgrade().is_some());
        drop(owner);
        assert!(runtime.upgrade().is_none());
        fixture.finish();
    });
}
