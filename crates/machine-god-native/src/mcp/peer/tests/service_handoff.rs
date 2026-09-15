use super::*;
use crate::mcp::{
    catalog::{McpDescriptorCatalog, McpDescriptorLimits},
    context::NativeMcpContexts,
    runtime::{
        NativeMcpOwnedPeer, NativeMcpRuntime, NativeMcpRuntimeClock, NativeMcpRuntimeLimits,
        NativeMcpRuntimeToolCall, NativeMcpServerCandidate, NativeMcpToolExecutionPolicy,
        NativeMcpToolExecutor,
    },
};
use std::future::{Future, poll_fn};

struct ServiceClock;
impl NativeMcpRuntimeClock for ServiceClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move { tokio::time::sleep_until(deadline.into()).await })
    }
}
struct NoToolCalls;
impl NativeMcpToolExecutor for NoToolCalls {
    fn execute(
        &self,
        _: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<
        '_,
        std::result::Result<machine_god_core::ToolExecution, machine_god_core::ToolError>,
    > {
        Box::pin(async { panic!("publication fixture must not execute tools") })
    }
}

async fn attributed<T>(
    run: &crate::owned_worker::NativeOwnedWorkerRun,
    future: impl Future<Output = T>,
) -> T {
    let mut future = std::pin::pin!(future);
    poll_fn(|cx| run.with_poll(|| future.as_mut().poll(cx))).await
}

// Requires the canonical freshly built explicit release helper. The in-memory
// focused lane compiles this; the supported-platform runtime gate executes it.
#[test]
fn actual_stdio_publication_settles_run_while_retained_server_stays_healthy() {
    let fixture = Fixture::new();
    let run = fixture.host.begin_run().unwrap();
    let launch = fixture.launch(r#"
IFS= read -r line
case "$line" in *server/discover*) : ;; *) exit 4 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}}}}'
IFS= read -r line
case "$line" in *tools/list*) : ;; *) exit 5 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}'
while IFS= read -r line; do :; done
"#);
    let mut factory = || Ok(launch.clone());
    let mut peer = fixture
        .runtime
        .block_on(attributed(
            &run,
            McpStdioPeer::connect(
                &mut factory,
                fixture.host.clone(),
                Arc::new(Timer),
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(20),
                Duration::from_secs(5),
            ),
        ))
        .unwrap();
    let readiness = peer.readiness();
    let completion = peer.completion();
    let catalog = fixture
        .runtime
        .block_on(attributed(
            &run,
            peer.catalog(
                McpCatalogKind::Tools,
                McpCatalogLimits::default(),
                Instant::now(),
                Instant::now() + Duration::from_secs(5),
            ),
        ))
        .unwrap();
    let runtime = NativeMcpRuntime::new(
        Arc::new(NativeMcpContexts::new()),
        Arc::new(ServiceClock),
        Arc::new(NoToolCalls),
        NativeMcpToolExecutionPolicy::default(),
        NativeMcpRuntimeLimits::default(),
    )
    .unwrap();
    let candidate = runtime
        .prepare_candidate(
            vec![NativeMcpServerCandidate {
                server: Arc::from("retained"),
                configuration: Arc::from(&b"configuration"[..]),
                authentication: Arc::from(&b"authentication"[..]),
                catalogs: vec![
                    McpDescriptorCatalog::admit(catalog, McpDescriptorLimits::default()).unwrap(),
                ],
                refresh: None,
                catalog_epoch: Instant::now(),
                peer: NativeMcpOwnedPeer::Stdio(peer),
                operation_timeout: Duration::from_secs(5),
                authority_cancellations: Arc::from([]),
            }],
            &[],
        )
        .unwrap();
    run.close();
    assert!(
        !run.completion().is_complete(),
        "negotiation/catalog preparation must not promote"
    );
    runtime.publish(candidate).unwrap();
    fixture.runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), run.completion().wait())
            .await
            .unwrap();
    });
    assert!(readiness.is_ready());
    assert!(!completion.is_complete());
    runtime.close();
    fixture
        .runtime
        .block_on(runtime.drain_retired(
            Instant::now() + Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .unwrap();
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
}
