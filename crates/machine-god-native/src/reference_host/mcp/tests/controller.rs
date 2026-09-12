use super::*;
use crate::mcp::{
    controller::{NativeMcpControllerPublication, NativeMcpControllerStartupOptions},
    management::NativeMcpManagementService,
    startup::NativeMcpStartupPhase,
    store::NativeMcpConfigStore,
};

fn startup(clock: Arc<Clock>) -> NativeMcpControllerStartupOptions {
    let now = Instant::now();
    NativeMcpControllerStartupOptions {
        captured_environment: vec![],
        stdio: None,
        clock,
        catalog_epoch: now,
        owner_cancellation: CancellationToken::new(),
        #[cfg(feature = "mcp-http")]
        network: None,
        #[cfg(feature = "mcp-http")]
        authentication: vec![],
        peer_lifetime: (now + Duration::from_secs(60)).into(),
        max_retained_bytes: 1024 * 1024,
    }
}

#[test]
fn controller_selection_requires_management_and_identical_clock_before_acquisition() {
    let directory = Directory::new();
    let service = Arc::new(NativeMcpManagementService::new(Arc::new(
        NativeMcpConfigStore::new(directory.0.join("profile")).unwrap(),
    )));
    let clock = Arc::new(Clock::default());
    let make = |startup_clock| {
        NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
            .with_terminal(terminal())
            .with_permissions(permission(clock.clone()))
            .with_mcp_runtime(
                NativeReferenceHostMcpOptions::new(
                    Arc::new(NativeMcpContexts::new()),
                    clock.clone(),
                )
                .with_controller_startup(startup(startup_clock)),
            )
    };
    let config = LoadedNativeConfig::from_file(NativeConfig::default());
    for options in [
        make(clock.clone()),
        make(Arc::new(Clock::default())).with_mcp_management(service.clone()),
    ] {
        assert_eq!(
            validate_prepared_selections(&config, &options.into())
                .unwrap_err()
                .kind(),
            NativeReferenceHostBuildErrorKind::McpConfig
        );
    }
    validate_prepared_selections(
        &config,
        &make(clock.clone()).with_mcp_management(service).into(),
    )
    .unwrap();
    assert_eq!(clock.0.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
}

fn fixture() -> Fixture {
    Fixture::with_options("ask", true, |mut options, directory, clock| {
        let mcp = options
            .mcp_runtime
            .take()
            .unwrap()
            .with_controller_startup(startup(clock));
        options.with_mcp_runtime(mcp).with_mcp_management(Arc::new(
            NativeMcpManagementService::new(Arc::new(
                NativeMcpConfigStore::new(directory.0.join("profile")).unwrap(),
            )),
        ))
    })
}

#[test]
fn actual_host_controller_is_inert_and_publishes_only_when_polled() {
    let fixture = fixture();
    let host = fixture.host();
    let controller = host.mcp_controller().unwrap();
    let pending = controller.start(
        NativeMcpStartupPhase::AskStartup,
        CancellationToken::new(),
        Instant::now() + Duration::from_secs(5),
    );
    assert_eq!(fixture.clock.0.load(Ordering::Relaxed), 0);
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
    assert!(host.mcp_runtime().unwrap().publication_checkpoint().is_ok());
    run(async {
        let receipt = pending.await.unwrap();
        assert_eq!(
            receipt.publication(),
            NativeMcpControllerPublication::Published
        );
        host.close_mcp();
        assert!(
            controller
                .settle(
                    Instant::now() + Duration::from_secs(5),
                    CancellationToken::new()
                )
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn retained_controller_does_not_extend_engine_resource_lifetime() {
    let mut fixture = fixture();
    let host = fixture.host.take().unwrap();
    let controller = host.mcp_controller().unwrap();
    let completion = host.terminal_shutdown_completion().unwrap();
    let engine = host.into_engine();
    drop(engine);
    run(async {
        let result = controller
            .start(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(5),
            )
            .await;
        assert!(result.is_err());
        assert!(
            controller
                .settle(
                    Instant::now() + Duration::from_secs(5),
                    CancellationToken::new()
                )
                .await
                .unwrap()
                .complete
        );
    });
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
}
