use super::super::{mcp_startup, with_settled_terminal_host};
use super::*;
use machine_god_native::{
    FileUndoTracker, NativePermissionContexts, NativeReferenceHostConversationOptions,
    NativeReferenceHostMcpOptions, NativeReferenceHostPermissionOptions,
    NativeReferenceHostTerminalOptions, NativeRootSelection, PreparedNativeRoots,
    TokioPermissionReviewClock, TokioWebSearchDeadline,
    mcp::{
        clock::TokioMcpClock, context::NativeMcpContexts,
        controller::NativeMcpControllerStartupOptions, lifetime::McpPeerLifetime,
        management::NativeMcpManagementService, startup::NativeMcpStartupPhase,
        store::NativeMcpConfigStore,
    },
};

fn host(directory: &ScopedTestDirectory) -> (NativeReferenceHost, Arc<OneShotTransport>) {
    let workspace = directory.path().join("workspace");
    let state = directory.path().join("state");
    for path in [&workspace, &state] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let environment = NativeEnvironment::new(None, Some(state.into_os_string()), None);
    let roots = PreparedNativeRoots::prepare(
        NativeRootSelection::from_environment(&environment, &workspace).unwrap(),
    )
    .unwrap();
    let clock = Arc::new(TokioMcpClock);
    let options = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
        .with_terminal(
            NativeReferenceHostTerminalOptions::new(
                "/explicit-unexecuted-mcp-helper".into(),
                None,
                vec![],
            )
            .unwrap(),
        )
        .with_permissions(NativeReferenceHostPermissionOptions::new(
            Arc::new(NativePermissionContexts::new()),
            Arc::new(TokioPermissionReviewClock),
        ))
        .with_mcp_management(Arc::new(NativeMcpManagementService::new(Arc::new(
            NativeMcpConfigStore::new(directory.path().join("profile")).unwrap(),
        ))))
        .with_mcp_runtime(
            NativeReferenceHostMcpOptions::new(Arc::new(NativeMcpContexts::new()), clock.clone())
                .with_controller_startup(NativeMcpControllerStartupOptions {
                    captured_environment: vec![],
                    stdio: None,
                    clock,
                    catalog_epoch: Instant::now(),
                    owner_cancellation: CancellationToken::new(),
                    network: None,
                    authentication: vec![],
                    peer_lifetime: McpPeerLifetime::OwnerControlled,
                    max_retained_bytes: 1024 * 1024,
                }),
        );
    let transport = Arc::new(OneShotTransport::new(""));
    let host =
        NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            load_native_config(&NativeEnvironment::new(None, None, None)).unwrap(),
            transport.clone(),
            NetworkTarget {
                scheme: "https".into(),
                host: "ai-gateway.vercel.sh".into(),
                port: None,
            },
            roots,
            Arc::new(DenyPermissionPrompter),
            Arc::new(UnavailableQuestionPrompter),
            Arc::new(NeverWebSearchDeadline),
            options,
        )
        .unwrap();
    (host, transport)
}

#[test]
fn mcp_cli_activation_and_settlement_use_the_actual_host_without_a_provider_turn() {
    for phase in [
        NativeMcpStartupPhase::All,
        NativeMcpStartupPhase::AskStartup,
    ] {
        let directory = ScopedTestDirectory::new(&format!("mcp-activation-{phase:?}"));
        let (host, transport) = host(&directory);
        let completion = host.terminal_shutdown_completion().unwrap();
        let (runtime, _) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        let (_sender, receiver) = tokio::sync::mpsc::channel(1);
        let mut signals = AskSignals::new(receiver);
        with_settled_terminal_host(host, &runtime, |host| {
            runtime.block_on(mcp_startup::activate(host, phase, &mut signals))
        })
        .unwrap();
        assert!(completion.is_complete());
        assert!(transport.request_bodies().is_empty());
        assert!(signals.first_observed.is_none());
    }
}

#[test]
fn mcp_cli_startup_signal_cancels_before_activation_and_still_joins_the_host() {
    let directory = ScopedTestDirectory::new("mcp-activation-signal");
    let (host, transport) = host(&directory);
    let completion = host.terminal_shutdown_completion().unwrap();
    let (runtime, _) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    sender.try_send(AskSignal::Terminate).unwrap();
    let mut signals = AskSignals::new(receiver);
    assert!(
        with_settled_terminal_host(host, &runtime, |host| {
            runtime.block_on(mcp_startup::activate(
                host,
                NativeMcpStartupPhase::All,
                &mut signals,
            ))
        })
        .is_err()
    );
    assert!(completion.is_complete());
    assert!(transport.request_bodies().is_empty());
    assert_eq!(signals.first_observed, Some(AskSignal::Terminate));
}

#[test]
fn mcp_cli_settlement_survives_operation_error_and_unwind() {
    for panic in [false, true] {
        let directory = ScopedTestDirectory::new(&format!("mcp-settlement-{panic}"));
        let (host, _) = host(&directory);
        let completion = host.terminal_shutdown_completion().unwrap();
        let (runtime, _) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        let result: Result<(), ()> = with_settled_terminal_host(host, &runtime, |_| {
            assert!(!panic, "synthetic MCP CLI operation unwind");
            Err(())
        });
        assert!(result.is_err());
        assert!(completion.is_complete());
    }
}
