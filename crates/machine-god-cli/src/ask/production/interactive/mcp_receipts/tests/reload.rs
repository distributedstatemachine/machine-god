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
        runtime::{
            NativeMcpRuntime, NativeMcpRuntimeLimits, NativeMcpRuntimeToolCall,
            NativeMcpToolExecutionPolicy, NativeMcpToolExecutor,
        },
        startup::NativeMcpStartupPhase,
    },
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

struct NeverExecute;
impl NativeMcpToolExecutor for NeverExecute {
    fn execute(
        &self,
        _: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        panic!("receipt rendering does not execute a tool")
    }
}

#[test]
fn actual_controller_receipts_keep_publication_and_cleanup_observations_separate() {
    let fixture = super::super::super::support::Fixture::new_with_mcp();
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
    let workers = NativeOwnedWorkerScope::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let controller = NativeMcpController::new(NativeMcpControllerOptions {
        runtime,
        management: fixture.host.mcp_management().unwrap(),
        workers: workers.clone(),
        reserved_tool_names: Box::new([]),
        max_retained_generations: 4,
        startup: NativeMcpControllerStartupOptions {
            captured_environment: vec![],
            stdio: None,
            clock,
            catalog_epoch: Instant::now(),
            owner_cancellation: CancellationToken::new(),
            network: None,
            authentication: vec![],
            peer_lifetime_deadline: deadline,
            max_retained_bytes: 1024 * 1024,
        },
    })
    .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let receipt = controller
                .start(
                    NativeMcpStartupPhase::AskStartup,
                    CancellationToken::new(),
                    deadline,
                )
                .await
                .unwrap();
            let text = String::from_utf8(render_reload(1, Ok(&receipt)).unwrap()).unwrap();
            assert!(text.contains("Publication: published"));
            assert!(text.contains("Startup observation: AskStartup; 0 servers"));
            assert!(text.contains("not remote revocation or current connection evidence"));
            let unchanged = controller
                .activate_deferred(CancellationToken::new(), deadline)
                .await
                .unwrap();
            let text = String::from_utf8(render_reload(2, Ok(&unchanged)).unwrap()).unwrap();
            assert!(text.contains("Publication: unchanged"));
            controller.close();
            let failure = controller
                .reload(CancellationToken::new(), deadline)
                .await
                .unwrap_err();
            let text = String::from_utf8(render_reload(3, Err(&failure)).unwrap()).unwrap();
            assert!(text.contains("Reload failed (Closed)"));
            assert!(text.contains("No automatic retry"));
            assert!(text.contains("Local cleanup evidence: not complete or not established"));
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
