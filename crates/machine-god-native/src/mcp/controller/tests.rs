use super::state::lock;
use super::*;
use crate::mcp::store::NativeMcpConfigStore;
use crate::mcp::{
    context::NativeMcpContexts,
    runtime::{
        NativeMcpRuntimeLimits, NativeMcpRuntimeToolCall, NativeMcpToolExecutionPolicy,
        NativeMcpToolExecutor,
    },
};
use machine_god_core::{ToolError, ToolExecution};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

#[cfg(feature = "mcp-http")]
mod http;

#[derive(Default)]
struct Clock(AtomicUsize);
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        self.0.fetch_add(1, Ordering::Relaxed);
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}
struct NeverExecute;
impl NativeMcpToolExecutor for NeverExecute {
    fn execute(
        &self,
        _: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, std::result::Result<ToolExecution, ToolError>> {
        panic!("controller startup cannot execute tools")
    }
}
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    base: PathBuf,
    options: NativeMcpControllerOptions,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "mg-controller-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let clock = Arc::new(Clock::default());
        let runtime = Arc::new(
            NativeMcpRuntime::new(
                Arc::new(NativeMcpContexts::new()),
                clock.clone(),
                Arc::new(NeverExecute),
                NativeMcpToolExecutionPolicy::default(),
                NativeMcpRuntimeLimits::default(),
            )
            .unwrap(),
        );
        let options = NativeMcpControllerOptions {
            runtime,
            management: Arc::new(NativeMcpManagementService::new(Arc::new(
                NativeMcpConfigStore::new(base.join("profile")).unwrap(),
            ))),
            workers: NativeOwnedWorkerScope::new(),
            reserved_tool_names: Box::new([]),
            startup: NativeMcpControllerStartupOptions {
                captured_environment: vec![],
                stdio: None,
                clock,
                catalog_epoch: Instant::now(),
                owner_cancellation: CancellationToken::new(),
                #[cfg(feature = "mcp-http")]
                network: None,
                #[cfg(feature = "mcp-http")]
                authentication: vec![],
                peer_lifetime_deadline: deadline(),
                max_retained_bytes: 1024 * 1024,
            },
            max_retained_generations: 4,
        };
        Self { base, options }
    }
    fn controller(&self) -> NativeMcpController {
        NativeMcpController::new(NativeMcpControllerOptions {
            runtime: self.options.runtime.clone(),
            management: self.options.management.clone(),
            workers: self.options.workers.clone(),
            reserved_tool_names: self.options.reserved_tool_names.clone(),
            startup: self.options.startup.clone(),
            max_retained_generations: self.options.max_retained_generations,
        })
        .unwrap()
    }
    fn seed(&self, config: &str) {
        let profile = self.base.join("profile");
        fs::create_dir_all(&profile).unwrap();
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(profile.join("mcp.json"), config).unwrap();
        fs::set_permissions(profile.join("mcp.json"), fs::Permissions::from_mode(0o600)).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.options.workers.close();
        fs::remove_dir_all(&self.base).unwrap();
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

#[test]
fn selected_clock_deadlines_reject_zero_and_overflow_and_work_after_close() {
    struct FixedClock {
        instant: Instant,
        reads: AtomicUsize,
    }
    impl NativeMcpRuntimeClock for FixedClock {
        fn now(&self) -> Instant {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.instant
        }
        fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }
    let mut fixture = Fixture::new();
    let clock = Arc::new(FixedClock {
        instant: Instant::now(),
        reads: AtomicUsize::new(0),
    });
    fixture.options.startup.clock = clock.clone();
    let controller = fixture.controller();
    assert_eq!(clock.reads.load(Ordering::Relaxed), 0);
    assert_eq!(
        controller
            .deadline_after(Duration::ZERO)
            .unwrap_err()
            .kind(),
        NativeMcpControllerError::Invalid
    );
    assert_eq!(clock.reads.load(Ordering::Relaxed), 0);
    assert_eq!(
        controller.deadline_after(Duration::from_secs(3)).unwrap(),
        clock.instant + Duration::from_secs(3)
    );
    assert_eq!(
        controller.deadline_after(Duration::MAX).unwrap_err().kind(),
        NativeMcpControllerError::Limit
    );
    controller.close();
    assert_eq!(
        controller.deadline_after(Duration::from_secs(7)).unwrap(),
        clock.instant + Duration::from_secs(7)
    );
    assert_eq!(clock.reads.load(Ordering::Relaxed), 3);
}
fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(15), future)
                .await
                .unwrap();
        });
}

#[test]
fn inert_unpolled_and_precancelled_start_never_load_profile() {
    let fixture = Fixture::new();
    let controller = fixture.controller();
    drop(controller.start(
        NativeMcpStartupPhase::All,
        CancellationToken::new(),
        deadline(),
    ));
    assert!(lock(&controller.inner.state).generations.is_empty());
    assert!(!fixture.base.join("profile").exists());
    run(async {
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            controller
                .start(NativeMcpStartupPhase::All, cancel, deadline())
                .await
                .unwrap_err()
                .kind(),
            NativeMcpControllerError::Cancelled
        );
        assert!(lock(&controller.inner.state).generations.is_empty());
    });
}

#[test]
fn empty_all_start_is_once_and_deferred_is_inert() {
    run(async {
        let fixture = Fixture::new();
        let controller = fixture.controller();
        let receipt = controller
            .start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        assert_eq!(
            receipt.publication(),
            NativeMcpControllerPublication::Published
        );
        fixture.seed("not json");
        assert_eq!(
            controller
                .activate_deferred(CancellationToken::new(), deadline())
                .await
                .unwrap()
                .publication(),
            NativeMcpControllerPublication::Unchanged
        );
        assert_eq!(
            controller
                .start(
                    NativeMcpStartupPhase::All,
                    CancellationToken::new(),
                    deadline()
                )
                .await
                .unwrap_err()
                .kind(),
            NativeMcpControllerError::Invalid
        );
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn empty_ask_deferred_set_is_an_inert_noop_even_after_profile_changes() {
    run(async {
        let fixture = Fixture::new();
        let controller = fixture.controller();
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        fixture.seed("invalid");
        assert_eq!(
            controller
                .activate_deferred(CancellationToken::new(), deadline())
                .await
                .unwrap()
                .publication(),
            NativeMcpControllerPublication::Unchanged
        );
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn failed_reload_preserves_exact_active_token_and_runtime_view() {
    run(async {
        let fixture = Fixture::new();
        let controller = fixture.controller();
        controller
            .start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let old = lock(&controller.inner.state).active.clone().unwrap();
        let checkpoint = fixture.options.runtime.publication_checkpoint().unwrap();
        fixture.seed(r#"{"mcp":{"optional":{"command":"/missing/server"}}}"#);
        let error = controller
            .reload(CancellationToken::new(), deadline())
            .await
            .unwrap_err();
        assert!(matches!(error.kind(), NativeMcpControllerError::Startup(_)));
        assert!(error.startup().is_some());
        assert!(!old.cancellation.is_cancelled());
        assert!(Arc::ptr_eq(
            &old,
            lock(&controller.inner.state).active.as_ref().unwrap()
        ));
        let candidate = fixture
            .options
            .runtime
            .prepare_candidate(vec![], &[])
            .unwrap();
        fixture
            .options
            .runtime
            .publish_if(candidate, &checkpoint)
            .unwrap();
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn reload_loads_fresh_snapshot_and_cancels_only_successfully_replaced_generation() {
    run(async {
        let fixture = Fixture::new();
        let controller = fixture.controller();
        controller
            .start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let old = lock(&controller.inner.state).active.clone().unwrap();
        fixture.seed(r#"{"mcp":{"disabled":{"command":"/missing/server","enabled":false}}}"#);
        let receipt = controller
            .reload(CancellationToken::new(), deadline())
            .await
            .unwrap();
        assert_eq!(receipt.startup().unwrap().servers.len(), 1);
        assert!(old.cancellation.is_cancelled());
        assert!(
            !lock(&controller.inner.state)
                .active
                .as_ref()
                .unwrap()
                .cancellation
                .is_cancelled()
        );
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn stale_ask_snapshot_fails_once_without_invalidating_required_publication() {
    run(async {
        let fixture = Fixture::new();
        let controller = fixture.controller();
        fixture.seed(r#"{"mcp":{"optional":{"command":"/missing/server"}}}"#);
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let old = lock(&controller.inner.state).active.clone().unwrap();
        fixture.seed(r#"{"mcp":{}}"#);
        for _ in 0..2 {
            assert_eq!(
                controller
                    .activate_deferred(CancellationToken::new(), deadline())
                    .await
                    .unwrap_err()
                    .kind(),
                NativeMcpControllerError::Store(NativeMcpConfigStoreError::Conflict)
            );
            assert!(!old.cancellation.is_cancelled());
        }
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn retained_outcomes_charge_generation_capacity_until_dropped() {
    run(async {
        let mut fixture = Fixture::new();
        fixture.options.max_retained_generations = 2;
        let controller = fixture.controller();
        let initial = controller
            .start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        controller
            .reload(CancellationToken::new(), deadline())
            .await
            .unwrap();
        assert_eq!(
            controller
                .reload(CancellationToken::new(), deadline())
                .await
                .unwrap_err()
                .kind(),
            NativeMcpControllerError::Limit
        );
        drop(initial);
        controller
            .reload(CancellationToken::new(), deadline())
            .await
            .unwrap();
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn reentrant_close_during_old_generation_retirement_preserves_published_receipt() {
    run(async {
        use machine_god_reentrant_waker_test::{Callback, new};
        use std::task::Context;
        let fixture = Fixture::new();
        let controller = Arc::new(fixture.controller());
        controller
            .start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let original = lock(&controller.inner.state).active.clone().unwrap();
        let weak = Arc::downgrade(&controller);
        let (waker, callbacks) = new(Callback::Wake, move || weak.upgrade().unwrap().close());
        let mut cancelled = Box::pin(original.cancellation.cancelled());
        assert!(
            cancelled
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        let receipt = controller
            .reload(CancellationToken::new(), deadline())
            .await
            .unwrap();
        assert_eq!(
            receipt.publication(),
            NativeMcpControllerPublication::Published
        );
        assert!(receipt.closed_after_publication());
        assert!(callbacks.calls() > 0);
        assert!(lock(&controller.inner.state).active.is_none());
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn abandoned_start_retains_job_and_close_settles_before_worker_scope_shutdown() {
    run(async {
        use std::task::{Context, Poll};
        let fixture = Fixture::new();
        let controller = fixture.controller();
        let mut request = controller.start(
            NativeMcpStartupPhase::All,
            CancellationToken::new(),
            deadline(),
        );
        assert!(matches!(
            request.as_mut().poll(&mut Context::from_waker(
                futures_util::task::noop_waker_ref()
            )),
            Poll::Pending
        ));
        drop(request);
        assert!(lock(&controller.inner.state).running.is_some());
        let receipt = controller
            .settle(deadline(), CancellationToken::new())
            .await
            .unwrap();
        assert!(receipt.complete);
        assert_eq!(
            controller
                .reload(CancellationToken::new(), deadline())
                .await
                .unwrap_err()
                .kind(),
            NativeMcpControllerError::Closed
        );
        assert!(fixture.options.workers.run(|| 7).await.is_ok());
    });
}
