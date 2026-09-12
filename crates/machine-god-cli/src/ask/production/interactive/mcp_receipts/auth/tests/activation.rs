use super::*;
use machine_god_core::{BoxFuture, CancellationToken, ToolError, ToolExecution};
use machine_god_native::{
    NativeOwnedWorkerScope,
    mcp::{
        clock::TokioMcpClock,
        context::NativeMcpContexts,
        controller::{
            NativeMcpController, NativeMcpControllerOptions, NativeMcpControllerStartupOptions,
        },
        management::NativeMcpManagementService,
        runtime::{
            NativeMcpRuntime, NativeMcpRuntimeLimits, NativeMcpRuntimeToolCall,
            NativeMcpToolExecutionPolicy, NativeMcpToolExecutor,
        },
        startup::NativeMcpStartupPhase,
    },
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::{Duration, Instant},
};

struct NeverExecute;
impl NativeMcpToolExecutor for NeverExecute {
    fn execute(
        &self,
        _: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        panic!("receipt renderer never executes a tool")
    }
}

fn controller(
    management: Arc<NativeMcpManagementService>,
    workers: NativeOwnedWorkerScope,
    deadline: Instant,
) -> NativeMcpController {
    let clock = Arc::new(TokioMcpClock);
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
    NativeMcpController::new(NativeMcpControllerOptions {
        runtime,
        management,
        workers,
        reserved_tool_names: Box::new([]),
        max_retained_generations: 4,
        stored_authentication: None,
        startup: NativeMcpControllerStartupOptions {
            captured_environment: vec![],
            stdio: None,
            clock,
            catalog_epoch: Instant::now(),
            owner_cancellation: CancellationToken::new(),
            network: None,
            authentication: vec![],
            peer_lifetime: deadline.into(),
            max_retained_bytes: 1024 * 1024,
        },
    })
    .unwrap()
}

#[test]
fn actual_reload_and_failure_observations_remain_separate_from_saved_credentials() {
    let fixture = super::super::super::super::support::Fixture::new_with_mcp();
    let workers = NativeOwnedWorkerScope::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let controller = controller(
        fixture.host.mcp_management().unwrap(),
        workers.clone(),
        deadline,
    );
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let activation = controller
                .start(
                    NativeMcpStartupPhase::AskStartup,
                    CancellationToken::new(),
                    deadline,
                )
                .await
                .unwrap();
            let text = rendered(&NativeMcpAuthenticationReceipt::Authenticated {
                server: "demo".into(),
                usable: true,
                activation: Some(Ok(activation)),
            });
            assert!(text.contains("Credential persistence: confirmed"));
            assert!(text.contains("full configured reload observation (not a targeted reconnect)"));
            assert!(text.contains("Publication: published"));
            assert!(text.contains("Startup observation: AskStartup; 0 servers"));
            failed_server_observations(&controller, &fixture.workspace).await;
            controller.close();
            let failure = controller
                .reload(CancellationToken::new(), deadline)
                .await
                .unwrap_err();
            let text = rendered(&NativeMcpAuthenticationReceipt::Authenticated {
                server: "demo".into(),
                usable: true,
                activation: Some(Err(failure.clone())),
            });
            assert!(text.contains("Credential persistence: confirmed"));
            assert!(text.contains("Reload failed (Closed)"));
            assert!(!text.contains("Publication: published"));
            let text = String::from_utf8(
                render(9, Err(&NativeMcpAuthenticationError::Selection(failure))).unwrap(),
            )
            .unwrap();
            assert!(text.contains("Authentication selection failed (Closed)"));
            assert!(!text.contains("Credential persistence: confirmed"));
            assert!(
                controller
                    .settle(deadline, CancellationToken::new())
                    .await
                    .unwrap()
                    .complete
            );
        });
    drop(controller);
    workers.close();
    workers.completion().wait_on_worker().unwrap();
    assert!(fixture.transport.requests().is_empty());
    fixture.finish();
}

async fn failed_server_observations(controller: &NativeMcpController, workspace: &std::path::Path) {
    let profile = workspace.parent().unwrap().join("mcp-profile");
    fs::create_dir(&profile).unwrap();
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o700)).unwrap();
    let path = profile.join("mcp.json");
    fs::write(&path, br#"{"mcp":{"demo":{"command":"/missing/server"}}}"#).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let activation = controller
        .reload_configured(CancellationToken::new())
        .await
        .unwrap();
    assert!(activation.startup().unwrap().required_ready());
    let text = rendered(&NativeMcpAuthenticationReceipt::Authenticated {
        server: "demo".into(),
        usable: true,
        activation: Some(Ok(activation)),
    });
    assert!(text.contains("Credential persistence: confirmed"));
    assert!(text.contains("Publication: published"));
    assert!(text.contains("Startup observation: All; 1 servers; failures: true"));
    assert!(text.contains("demo: startup failed; optional"));
    fs::write(
        &path,
        br#"{"mcp":{"demo":{"command":"/missing/server","required":true}}}"#,
    )
    .unwrap();
    let failure = controller
        .reload_configured(CancellationToken::new())
        .await
        .unwrap_err();
    assert!(!failure.startup().unwrap().required_ready());
    let text = rendered(&NativeMcpAuthenticationReceipt::Authenticated {
        server: "demo".into(),
        usable: true,
        activation: Some(Err(failure)),
    });
    assert!(text.contains("Credential persistence: confirmed"));
    assert!(text.contains("Reload failed (Startup(RequiredUnavailable))"));
    assert!(text.contains("demo: startup failed; required"));
    assert!(!text.contains("Publication: published"));
}
