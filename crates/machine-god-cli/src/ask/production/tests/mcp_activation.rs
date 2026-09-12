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

mod auth_controls;

fn host(directory: &ScopedTestDirectory) -> (NativeReferenceHost, Arc<OneShotTransport>) {
    host_with_capture(directory, false)
}

fn host_with_capture(
    directory: &ScopedTestDirectory,
    capture: bool,
) -> (NativeReferenceHost, Arc<OneShotTransport>) {
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
    let helper = if capture {
        let helper = PathBuf::from(
            std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY")
                .expect("actual auth-session tests require the freshly built release helper"),
        );
        assert!(helper.is_absolute() && helper.is_file());
        helper
    } else {
        "/explicit-unexecuted-mcp-helper".into()
    };
    let terminal = NativeReferenceHostTerminalOptions::new(helper, None, vec![]).unwrap();
    let mcp = mcp_options(&roots, &terminal, capture);
    let options = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
        .with_model_routes(Arc::new(
            machine_god_native::NativeConversationModelRoutes::new(),
        ))
        .with_observations(Arc::new(
            machine_god_native::NativeConversationObservations::new(),
        ))
        .with_terminal(terminal)
        .with_permissions(NativeReferenceHostPermissionOptions::new(
            Arc::new(NativePermissionContexts::new()),
            Arc::new(TokioPermissionReviewClock),
        ))
        .with_mcp_management(Arc::new(NativeMcpManagementService::new(Arc::new(
            NativeMcpConfigStore::new(directory.path().join("profile")).unwrap(),
        ))))
        .with_mcp_runtime(mcp);
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

fn mcp_options(
    roots: &PreparedNativeRoots,
    terminal: &NativeReferenceHostTerminalOptions,
    capture: bool,
) -> NativeReferenceHostMcpOptions {
    let contexts = Arc::new(NativeMcpContexts::new());
    if capture {
        // Actual production auth/store/worker composition, with no process or
        // socket started by capture. These fixtures never activate stdio.
        return NativeReferenceHostMcpOptions::capture_startup(roots, terminal, contexts).unwrap();
    }
    let clock = Arc::new(TokioMcpClock);
    NativeReferenceHostMcpOptions::new(contexts, clock.clone()).with_controller_startup(
        NativeMcpControllerStartupOptions {
            captured_environment: vec![],
            stdio: None,
            clock,
            catalog_epoch: Instant::now(),
            owner_cancellation: CancellationToken::new(),
            network: None,
            authentication: vec![],
            peer_lifetime: McpPeerLifetime::OwnerControlled,
            max_retained_bytes: 1024 * 1024,
        },
    )
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
            runtime
                .block_on(mcp_startup::activate_interactive(host, &mut signals))
                .map(|_| ())
        })
        .is_err()
    );
    assert!(completion.is_complete());
    assert!(transport.request_bodies().is_empty());
    assert_eq!(signals.first_observed, Some(AskSignal::Terminate));
}

#[test]
fn mcp_cli_required_startup_failure_prevents_conversation_work_and_still_joins() {
    let directory = ScopedTestDirectory::new("mcp-required-ask-failure");
    seed_required_profile(&directory);
    let (host, transport) = host(&directory);
    let completion = host.terminal_shutdown_completion().unwrap();
    let (runtime, _) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
    let (_sender, receiver) = tokio::sync::mpsc::channel(1);
    let mut signals = AskSignals::new(receiver);
    let mut conversation_started = false;
    assert!(
        with_settled_terminal_host(host, &runtime, |host| {
            runtime.block_on(mcp_startup::activate(
                host,
                NativeMcpStartupPhase::AskStartup,
                &mut signals,
            ))?;
            conversation_started = true;
            Ok(())
        })
        .is_err()
    );
    assert!(!conversation_started);
    assert!(transport.request_bodies().is_empty());
    assert!(signals.first_observed.is_none());
    assert!(completion.is_complete());
}

fn seed_required_profile(directory: &ScopedTestDirectory) {
    let profile = directory.path().join("profile");
    fs::create_dir(&profile).unwrap();
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o700)).unwrap();
    let config = profile.join("mcp.json");
    fs::write(
        &config,
        r#"{"mcp":{"required":{"command":"/unexecuted-server","required":true}}}"#,
    )
    .unwrap();
    fs::set_permissions(config, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn mcp_cli_interactive_required_failure_preserves_management_and_rejects_each_prompt() {
    use machine_god_native::{
        NativeConversationError, NativeConversationRuntimeError, NativeInteractiveControlReceipt,
        NativeInteractiveInitialSession, NativeInteractiveSession, NativeInteractiveSessionOptions,
        mcp::{commands::McpCommand, management::NativeMcpManagementReceipt},
    };
    let directory = ScopedTestDirectory::new("mcp-interactive-required");
    seed_required_profile(&directory);
    let (host, transport) = host(&directory);
    let host = Arc::new(host);
    let completion = host.terminal_shutdown_completion().unwrap();
    let (runtime, _) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
    let (_sender, receiver) = tokio::sync::mpsc::channel(1);
    let mut signals = AskSignals::new(receiver);
    runtime.block_on(async {
        assert!(
            mcp_startup::activate_interactive(&host, &mut signals)
                .await
                .unwrap()
                .is_some()
        );
        let options = NativeInteractiveSessionOptions::new(
            host.workspace_root().to_owned(),
            host.loaded_config().config().model_preferences(),
        )
        .unwrap();
        let mut session = NativeInteractiveSession::open(
            host.clone(),
            options,
            NativeInteractiveInitialSession::Fresh,
            1,
        )
        .await
        .unwrap();
        let control = management(&mut session, McpCommand::List).await;
        assert!(matches!(
            control.result,
            Ok(NativeInteractiveControlReceipt::Mcp(
                NativeMcpManagementReceipt::Configured(_)
            ))
        ));
        for _ in 0..2 {
            session
                .runtime()
                .enqueue("blocked required prompt".into())
                .unwrap();
            assert!(matches!(
                session.runtime().start_next(3).await,
                Err(NativeConversationRuntimeError::Conversation(
                    NativeConversationError::McpRequiredUnavailable
                ))
            ));
            assert_eq!(session.runtime().status().queued_jobs, 0);
        }
        assert!(transport.request_bodies().is_empty());
        assert!(!session.is_closed());
        // Management repairs the source through its actual native worker lane.
        let removed = management(
            &mut session,
            McpCommand::Remove {
                server: "required".into(),
            },
        )
        .await;
        assert!(!removed.failed());
        drop(
            host.mcp_controller()
                .unwrap()
                .reload_configured(CancellationToken::new())
                .await
                .unwrap(),
        );
        assert!(session.runtime().start_next(5).await.unwrap().is_none());
        session
            .runtime()
            .enqueue("new explicitly submitted prompt".into())
            .unwrap();
        let turn = session.runtime().start_next(5).await.unwrap().unwrap();
        assert!(transport.request_bodies().is_empty());
        drop(turn);
        session.request_shutdown();
        poll_fn(|cx| {
            let _ = session.poll_progress(cx, 6);
            if session.is_closed() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
    });
    mcp_startup::settle(&host, &runtime).unwrap();
    drop(host);
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
    assert!(signals.first_observed.is_none());
}

async fn management(
    session: &mut machine_god_native::NativeInteractiveSession,
    command: machine_god_native::mcp::commands::McpCommand,
) -> machine_god_native::NativeInteractiveControlOutcome {
    session
        .request_control(
            machine_god_native::NativeInteractiveControl::Mcp { command },
            2,
        )
        .unwrap();
    poll_fn(|cx| {
        let _ = session.poll_progress(cx, 2);
        session
            .take_control_outcome()
            .map_or(Poll::Pending, Poll::Ready)
    })
    .await
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
