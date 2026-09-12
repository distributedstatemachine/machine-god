use super::*;
use crate::mcp::{
    controller::{
        NativeMcpController, NativeMcpControllerOptions, NativeMcpControllerStartupOptions,
    },
    management::NativeMcpManagementService,
    startup::NativeMcpStartupPhase,
    store::NativeMcpConfigStore,
};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

#[cfg(feature = "mcp-http")]
mod http;

struct LiveClock(AtomicUsize);
impl NativeMcpRuntimeClock for LiveClock {
    fn now(&self) -> Instant {
        self.0.fetch_add(1, Ordering::SeqCst);
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move { tokio::time::sleep_until(deadline.into()).await })
    }
}

struct Fixture {
    base: PathBuf,
    runtime: Arc<NativeMcpRuntime>,
    clock: Arc<LiveClock>,
    options: NativeMcpControllerOptions,
}
impl Fixture {
    fn new(config: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "mg-deferred-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let clock = Arc::new(LiveClock(AtomicUsize::new(0)));
        let runtime = Arc::new(
            NativeMcpRuntime::new(
                Arc::new(NativeMcpContexts::new()),
                clock.clone(),
                Arc::new(Executor::default()),
                policy(),
                NativeMcpRuntimeLimits::default(),
            )
            .unwrap(),
        );
        let options = NativeMcpControllerOptions {
            runtime: runtime.clone(),
            management: Arc::new(NativeMcpManagementService::new(Arc::new(
                NativeMcpConfigStore::new(base.join("profile")).unwrap(),
            ))),
            workers: NativeOwnedWorkerScope::new(),
            reserved_tool_names: Box::new([]),
            startup: NativeMcpControllerStartupOptions {
                captured_environment: vec![],
                stdio: None,
                clock: clock.clone(),
                catalog_epoch: Instant::now(),
                owner_cancellation: CancellationToken::new(),
                #[cfg(feature = "mcp-http")]
                network: None,
                #[cfg(feature = "mcp-http")]
                authentication: vec![],
                peer_lifetime: crate::mcp::lifetime::McpPeerLifetime::OwnerControlled,
                max_retained_bytes: 1024 * 1024,
            },
            max_retained_generations: 4,
        };
        let fixture = Self {
            base,
            runtime,
            clock,
            options,
        };
        fixture.seed(config);
        fixture
    }
    fn seed(&self, config: &str) {
        let profile = self.base.join("profile");
        fs::create_dir_all(&profile).unwrap();
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(profile.join("mcp.json"), config).unwrap();
        fs::set_permissions(profile.join("mcp.json"), fs::Permissions::from_mode(0o600)).unwrap();
    }
    fn controller(&self) -> Arc<NativeMcpController> {
        Arc::new(
            NativeMcpController::new(NativeMcpControllerOptions {
                runtime: self.runtime.clone(),
                management: self.options.management.clone(),
                workers: self.options.workers.clone(),
                reserved_tool_names: Box::new([]),
                startup: self.options.startup.clone(),
                max_retained_generations: 4,
            })
            .unwrap(),
        )
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
fn binding_is_exact_one_time_inert_and_weak() {
    let fixture = Fixture::new(r#"{"mcp":{}}"#);
    let controller = fixture.controller();
    assert_eq!(
        standalone().bind_controller(&controller),
        Err(NativeMcpRuntimeError::Invalid)
    );
    fixture.runtime.bind_controller(&controller).unwrap();
    assert_eq!(
        fixture.runtime.bind_controller(&controller),
        Err(NativeMcpRuntimeError::Invalid)
    );
    assert_eq!(fixture.clock.0.load(Ordering::SeqCst), 0);
    let weak = Arc::downgrade(&controller);
    drop(controller);
    assert!(weak.upgrade().is_none());
    assert!(fixture.runtime.state.lock().unwrap().closed);
    assert!(
        fixture
            .runtime
            .bind_controller(&fixture.controller())
            .is_err()
    );
}

#[test]
fn missing_stale_precancelled_and_unpolled_turns_never_activate() {
    run(async {
        let fixture = Fixture::new(r#"{"mcp":{}}"#);
        let controller = fixture.controller();
        fixture.runtime.bind_controller(&controller).unwrap();
        let (_engine, _conversation, turn, context) =
            addition::conversation(&fixture.runtime, "exact");
        drop(
            fixture
                .runtime
                .snapshot_for_turn(context.clone(), CancellationToken::new()),
        );
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(
            fixture
                .runtime
                .snapshot_for_turn(context.clone(), cancelled)
                .await
                .is_err()
        );
        let mut foreign = context.clone();
        foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
        assert!(
            fixture
                .runtime
                .snapshot_for_turn(foreign.clone(), CancellationToken::new())
                .await
                .is_err()
        );
        let query = crate::mcp::control::tests::request("resource list optional");
        assert!(
            fixture
                .runtime
                .feature_for_turn(foreign, &query, CancellationToken::new())
                .await
                .is_err()
        );
        assert!(turn.handle().cancel());
        assert!(
            fixture
                .runtime
                .snapshot_for_turn(context.clone(), CancellationToken::new())
                .await
                .is_err()
        );
        assert!(
            fixture
                .runtime
                .feature_for_turn(context, &query, CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(fixture.clock.0.load(Ordering::SeqCst), 0);
        assert!(fixture.runtime.state.lock().unwrap().turns.is_empty());
        controller
            .settle(deadline(), CancellationToken::new())
            .await
            .unwrap();
    });
}

#[test]
fn deferred_failure_keeps_required_view_and_existing_pin_skips_activation() {
    run(async {
        let fixture = Fixture::new(r#"{"mcp":{"optional":{"command":"/missing/server"}}}"#);
        let controller = fixture.controller();
        fixture.runtime.bind_controller(&controller).unwrap();
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        fixture
            .runtime
            .publish(candidate(
                &fixture.runtime,
                "required",
                &["lookup"],
                Arc::default(),
            ))
            .unwrap();
        let (_engine, _conversation, _turn, context) =
            addition::conversation(&fixture.runtime, "required");
        let snapshot = fixture
            .runtime
            .snapshot_for_turn(context.clone(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(snapshot.tools().len(), 1);
        assert_eq!(snapshot.tools()[0].name(), "mcp_required_lookup");
        fixture.seed("invalid");
        let before = fixture.clock.0.load(Ordering::SeqCst);
        assert_eq!(
            fixture
                .runtime
                .snapshot_for_turn(context, CancellationToken::new())
                .await
                .unwrap()
                .tools()
                .len(),
            1
        );
        assert_eq!(fixture.clock.0.load(Ordering::SeqCst), before);
        let (_next_engine, _next_conversation, _next_turn, next) =
            addition::conversation(&fixture.runtime, "next");
        assert_eq!(
            fixture
                .runtime
                .snapshot_for_turn(next.clone(), CancellationToken::new())
                .await
                .unwrap()
                .tools()
                .len(),
            1
        );
        controller.close();
        assert!(
            fixture
                .runtime
                .snapshot_for_turn(next, CancellationToken::new())
                .await
                .is_err()
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
fn older_pin_bypasses_deferred_and_global_failure_cannot_pin_a_new_turn() {
    run(async {
        let fixture = Fixture::new(r#"{"mcp":{"optional":{"command":"/missing/server"}}}"#);
        let controller = fixture.controller();
        fixture.runtime.bind_controller(&controller).unwrap();
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        fixture
            .runtime
            .publish(candidate(
                &fixture.runtime,
                "required",
                &["lookup"],
                Arc::default(),
            ))
            .unwrap();
        let (_engine, _conversation, _turn, context) =
            addition::conversation(&fixture.runtime, "pinned");
        let exact = fixture
            .runtime
            .contexts
            .snapshot_for_tool(&context)
            .unwrap();
        let old = fixture
            .runtime
            .for_turn(&exact.registry().unwrap())
            .unwrap()
            .unwrap();
        fixture.seed(r#"{"mcp":{}}"#);
        let before = fixture.clock.0.load(Ordering::SeqCst);
        fixture
            .runtime
            .snapshot_for_turn(context.clone(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(fixture.clock.0.load(Ordering::SeqCst), before);
        let (_next_engine, _next_conversation, _next_turn, next) =
            addition::conversation(&fixture.runtime, "unavailable");
        assert!(
            fixture
                .runtime
                .snapshot_for_turn(next, CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(fixture.runtime.state.lock().unwrap().turns.len(), 1);
        old.check().unwrap();
        fixture
            .runtime
            .snapshot_for_turn(context, CancellationToken::new())
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
fn all_mode_lazy_lookup_never_reloads_configuration() {
    run(async {
        let fixture = Fixture::new(r#"{"mcp":{}}"#);
        let controller = fixture.controller();
        fixture.runtime.bind_controller(&controller).unwrap();
        controller
            .start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        fixture.seed("invalid");
        let (_engine, _conversation, _turn, context) =
            addition::conversation(&fixture.runtime, "all");
        let snapshot = fixture
            .runtime
            .snapshot_for_turn(context, CancellationToken::new())
            .await
            .unwrap();
        assert!(snapshot.tools().is_empty());
        assert_eq!(fixture.runtime.state.lock().unwrap().turns.len(), 1);
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}
