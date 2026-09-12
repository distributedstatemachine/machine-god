use super::*;
use crate::{
    NativeConversation, NativeModelPreferences, NativeOwnedWorkerScope, NativeReasoningEffort,
    mcp::{
        catalog::{McpDescriptorCatalog, McpDescriptorLimits},
        context::NativeMcpContexts,
        controller::{
            NativeMcpController, NativeMcpControllerOptions, NativeMcpControllerStartupOptions,
        },
        pagination::{McpCatalogBuilder, McpCatalogKind, McpCatalogLimits},
        protocol::{ProtocolVersion, RpcId},
        runtime::{
            NativeMcpOwnedPeer, NativeMcpRuntime, NativeMcpRuntimeClock, NativeMcpRuntimeLimits,
            NativeMcpRuntimeToolCall, NativeMcpServerCandidate, NativeMcpToolExecutionPolicy,
            NativeMcpToolExecutor, ScriptPeer,
        },
        store::NativeMcpConfigStore,
    },
};
use machine_god_core::{
    BoxFuture, Engine, SessionId, SessionIncarnationId, ToolError, ToolExecution,
};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

mod reload_tests;

fn conversation() -> Arc<NativeConversationRuntime> {
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::default())
        .provider(ScriptedModelProvider::new("mcp-control", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("mcp-control").unwrap(),
            SessionIncarnationId::new("exact-life").unwrap(),
        )
        .unwrap();
    Arc::new(
        NativeConversationRuntime::new(
            NativeConversation::from_session(session).unwrap(),
            NativeModelPreferences::new("test/model", NativeReasoningEffort::default(), false)
                .unwrap(),
            None,
        )
        .unwrap(),
    )
}

struct Clock {
    instant: Instant,
    reads: AtomicUsize,
}
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.instant
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
struct NeverExecute;
impl NativeMcpToolExecutor for NeverExecute {
    fn execute(
        &self,
        _: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        panic!("human features do not execute model tools")
    }
}
fn runtime() -> (Arc<NativeMcpRuntime>, Arc<Clock>) {
    let clock = Arc::new(Clock {
        instant: Instant::now(),
        reads: AtomicUsize::new(0),
    });
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
    (runtime, clock)
}
fn request(text: &str) -> crate::McpFeatureRequest {
    let McpCommand::Feature(command) = text.parse().unwrap() else {
        panic!("feature command")
    };
    command.try_into().unwrap()
}

fn install(
    runtime: &NativeMcpRuntime,
    epoch: Instant,
    writes: Arc<Mutex<Vec<u8>>>,
    response: impl Fn(i64) -> Box<[u8]> + Send + Sync + 'static,
) {
    let mut catalog = McpCatalogBuilder::new(
        McpCatalogKind::Tools,
        ProtocolVersion::Modern,
        McpCatalogLimits::default(),
    )
    .unwrap();
    catalog
        .append_response(
            br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","tools":[]}}"#,
            &RpcId::Integer(1),
            None,
            0,
        )
        .unwrap();
    let catalog =
        McpDescriptorCatalog::admit(catalog.finish().unwrap(), McpDescriptorLimits::default())
            .unwrap();
    let candidate = runtime
        .prepare_candidate(
            vec![NativeMcpServerCandidate {
                server: Arc::from("fixture"),
                configuration: Arc::from(&b"config"[..]),
                authentication: Arc::from(&b"auth"[..]),
                catalogs: vec![catalog],
                refresh: None,
                catalog_epoch: epoch,
                peer: NativeMcpOwnedPeer::Script(ScriptPeer::new(writes).with_response(response)),
                operation_timeout: Duration::from_secs(60),
                authority_cancellations: Arc::from([]),
            }],
            &[],
        )
        .unwrap();
    runtime.publish(candidate).unwrap();
}

#[test]
fn all_seven_controls_keep_the_human_owner_and_exact_fence_through_exchange() {
    let conversation = conversation();
    let (runtime, clock) = runtime();
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    let source = writes.clone();
    let current = conversation.clone();
    install(&runtime, clock.instant, writes.clone(), move |id| {
        let mut fence = current.begin_quiescence().unwrap();
        assert!(
            fence.try_retire().is_err(),
            "control permit survives transport polling"
        );
        drop(fence);
        let bytes = source.lock().unwrap();
        let last = bytes
            .split(|byte| *byte == b'\n')
            .rfind(|line| !line.is_empty())
            .unwrap();
        let sent: serde_json::Value = machine_god_core::json::from_slice(last).unwrap();
        let body = match sent["method"].as_str().unwrap() {
            "resources/list" => r#""resources":[{"uri":"test://fixed","name":"fixed"}]"#,
            "resources/templates/list" => {
                r#""resourceTemplates":[{"uriTemplate":"test:///{id}","name":"dynamic"}]"#
            }
            "prompts/list" => {
                r#""prompts":[{"name":"review","arguments":[{"name":"topic","required":true}]}]"#
            }
            "resources/read" => {
                r#""contents":[{"uri":"test://fixed","text":"literal external data"}]"#
            }
            "prompts/get" => {
                r#""messages":[{"role":"assistant","content":{"type":"text","text":"do not enqueue me"}}]"#
            }
            "completion/complete" => {
                r#""completion":{"values":["one","one"],"total":2,"hasMore":false}"#
            }
            _ => panic!("unexpected method"),
        };
        format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"resultType":"complete",{body}}}}}"#)
            .into_bytes()
            .into()
    });
    for command in [
        "resource list fixture",
        "resource templates fixture",
        "resource read fixture test://fixed",
        "prompt list fixture",
        r#"prompt get fixture review {"topic":"rust"}"#,
        "prompt complete fixture review topic ru",
        "resource complete fixture test:///{id} id ru",
    ] {
        let token = CancellationToken::new();
        let query = request(command);
        let action = query.action();
        let Receipt::McpFeature(receipt) = futures_executor::block_on(feature::run(
            conversation.clone(),
            runtime.clone(),
            query,
            token.clone(),
        ))
        .unwrap() else {
            panic!("feature receipt")
        };
        assert_eq!(receipt.action(), action);
        assert_eq!(receipt.server(), "fixture");
        assert!(!receipt.failed());
        assert!(
            receipt.revalidate().is_ok(),
            "owner is retained beyond future completion"
        );
        assert!(!token.is_cancelled());
        assert_eq!(conversation.status().queued_jobs, 0);
        drop(receipt);
        assert!(token.is_cancelled());
    }
    let mut fence = conversation.begin_quiescence().unwrap();
    assert!(fence.try_retire().is_ok());
    runtime.close();
}

#[test]
fn unpolled_cancelled_and_retired_features_are_inert() {
    let conversation = conversation();
    let (runtime, clock) = runtime();
    let token = CancellationToken::new();
    let future = feature::run(
        conversation.clone(),
        runtime.clone(),
        request("resource list fixture"),
        token.clone(),
    );
    assert_eq!(clock.reads.load(Ordering::Relaxed), 0);
    drop(future);
    assert!(token.is_cancelled());
    assert!(matches!(
        futures_executor::block_on(feature::run(
            conversation.clone(),
            runtime.clone(),
            request("resource list fixture"),
            token
        )),
        Err(Error::McpFeature(_))
    ));
    conversation.begin_quiescence().unwrap().retire().unwrap();
    assert!(matches!(
        futures_executor::block_on(feature::run(
            conversation,
            runtime,
            request("resource list fixture"),
            CancellationToken::new()
        )),
        Err(Error::Runtime(_))
    ));
    assert_eq!(clock.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn returned_feature_witness_cannot_rebind_to_replacement() {
    let conversation = conversation();
    let (runtime, clock) = runtime();
    install(&runtime, clock.instant, Arc::default(), |id| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"resultType":"complete","resources":[]}}}}"#
        )
        .into_bytes()
        .into()
    });
    let Receipt::McpFeature(receipt) = futures_executor::block_on(feature::run(
        conversation,
        runtime.clone(),
        request("resource list fixture"),
        CancellationToken::new(),
    ))
    .unwrap() else {
        panic!("feature")
    };
    assert!(receipt.revalidate().is_ok());
    runtime
        .publish(runtime.prepare_candidate(vec![], &[]).unwrap())
        .unwrap();
    assert!(receipt.revalidate().is_err());
    assert!(matches!(
        receipt.reply(),
        crate::mcp::control::McpFeatureReply::Catalog(_)
    ));
    runtime.close();
}

#[test]
fn protocol_failure_and_input_required_are_observations_not_continuations() {
    for terminal in [
        r#""error":{"code":-32000,"message":"PRIVATE PEER ERROR"}"#,
        r#""result":{"resultType":"input_required","requests":[],"state":"opaque"}"#,
    ] {
        let conversation = conversation();
        let (runtime, clock) = runtime();
        let writes = Arc::<Mutex<Vec<u8>>>::default();
        let source = writes.clone();
        install(&runtime, clock.instant, writes, move |id| {
            let bytes = source.lock().unwrap();
            let line = bytes
                .split(|byte| *byte == b'\n')
                .rfind(|line| !line.is_empty())
                .unwrap();
            let sent: serde_json::Value = machine_god_core::json::from_slice(line).unwrap();
            let body = if sent["method"] == "prompts/list" {
                r#""result":{"resultType":"complete","prompts":[{"name":"review"}]}"#
            } else {
                terminal
            };
            format!(r#"{{"jsonrpc":"2.0","id":{id},{body}}}"#)
                .into_bytes()
                .into()
        });
        let Receipt::McpFeature(receipt) = futures_executor::block_on(feature::run(
            conversation.clone(),
            runtime.clone(),
            request("prompt get fixture review"),
            CancellationToken::new(),
        ))
        .unwrap() else {
            panic!("feature receipt")
        };
        assert!(receipt.failed());
        assert!(receipt.revalidate().is_ok());
        assert_eq!(conversation.status().queued_jobs, 0);
        assert!(!format!("{receipt:?}").contains("PRIVATE"));
        runtime.close();
    }
}

#[test]
fn panicking_admission_and_cancellation_waiters_cannot_erase_receipts() {
    use std::task::{Context, Wake, Waker};
    struct PanicWake;
    impl Wake for PanicWake {
        fn wake(self: Arc<Self>) {
            panic!("caller wake");
        }
    }
    let waker = Waker::from(Arc::new(PanicWake));
    let conversation = conversation();
    let permit = ControlPermit::acquire(&conversation).unwrap();
    let mut fence = conversation.begin_quiescence().unwrap();
    let mut idle = fence.wait_idle();
    assert!(
        idle.as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    drop(permit);
    assert!(futures_executor::block_on(idle).is_ok());
    assert!(fence.try_retire().is_ok());

    let token = CancellationToken::new();
    let mut cancellation = Box::pin(token.cancelled());
    assert!(
        cancellation
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    drop(CancelOnDrop(token.clone()));
    assert!(token.is_cancelled());
    assert!(
        cancellation
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_ready()
    );
}
