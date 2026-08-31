use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Engine, EngineEvent, Message, ModelEvent, Role,
    SessionId, SessionIncarnationId, StopReason, ToolCall, ToolCallId, ToolName, ToolOutput,
    TurnEvent,
};
use machine_god_native::{
    MCP_FEATURES_TOOL_NAME, McpFeatureAuthority, McpFeatureError, McpFeaturePayload,
    McpFeatureRequest, McpFeaturesTool,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
};
use serde_json::{Value, json};

struct ReadyAuthority {
    calls: AtomicUsize,
}

impl ReadyAuthority {
    const fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl McpFeatureAuthority for ReadyAuthority {
    fn call(
        &self,
        request: McpFeatureRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpFeaturePayload, McpFeatureError>> {
        Box::pin(async move {
            assert!(!cancellation.is_cancelled());
            assert_eq!(request.server(), "docs");
            assert_eq!(request.identity(), Some("custom://guide/start"));
            self.calls.fetch_add(1, Ordering::SeqCst);
            McpFeaturePayload::new(json!({
                "identity": "custom://guide/start",
                "contents": [{
                    "uri": "custom://guide/start",
                    "mimeType": "text/plain",
                    "type": "text",
                    "text": "ENGINE_UNTRUSTED_RESOURCE_SENTINEL"
                }]
            }))
        })
    }
}

fn provider(arguments: Value, name: &str) -> ScriptedModelProvider {
    ScriptedModelProvider::new(
        name,
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("mcp-features-call").unwrap(),
                        name: ToolName::new(MCP_FEATURES_TOOL_NAME).unwrap(),
                        arguments,
                    },
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    )
}

fn collect(engine: &Engine, name: &str) -> (SessionId, Vec<EngineEvent>) {
    let session_id = SessionId::new(name).unwrap();
    let session = engine
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new(format!("incarnation-{name}")).unwrap(),
        )
        .unwrap();
    let events = futures_executor::block_on(async {
        session
            .prompt("read the exact admitted MCP resource")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    });
    (session_id, events)
}

fn second_request_tool_output(provider: &ScriptedModelProvider) -> (Message, ToolOutput) {
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request.tools.len(), 1);
    assert_eq!(
        requests[0].request.tools[0].name.as_str(),
        MCP_FEATURES_TOOL_NAME
    );
    let message = requests[1].request.messages[2].clone();
    assert_eq!(message.role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = message.content.as_slice() else {
        panic!("expected one durable tool result")
    };
    let output = output.clone();
    (message, output)
}

#[test]
fn engine_persists_untrusted_feature_output_without_permission() {
    let authority = Arc::new(ReadyAuthority::new());
    let provider = provider(
        json!({
            "action": "resource_read",
            "server": "docs",
            "uri": "custom://guide/start"
        }),
        "mcp-features-engine",
    );
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(McpFeaturesTool::shared_authority(authority.clone()))
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "mcp-features-engine");
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));
    assert_eq!(authority.call_count(), 1);
    assert!(policy.requests().is_empty());
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::PermissionRequested { .. } | TurnEvent::PermissionResolved { .. }
    )));

    let expected = ToolOutput::success(json!({
        "trust": "untrusted_external",
        "authority": "none",
        "action": "resource_read",
        "server": "docs",
        "identity": "custom://guide/start",
        "contents": [{
            "uri": "custom://guide/start",
            "mimeType": "text/plain",
            "type": "text",
            "text": "ENGINE_UNTRUSTED_RESOURCE_SENTINEL"
        }]
    }));
    let finished = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
        .unwrap();
    assert!(matches!(
        &events[finished].payload,
        TurnEvent::ToolFinished { output, .. } if output == &expected
    ));
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(output, expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}

#[test]
fn engine_invalid_arguments_never_reach_feature_authority() {
    let authority = Arc::new(ReadyAuthority::new());
    let provider = provider(
        json!({
            "action": "resource_read",
            "server": "docs",
            "uri": "custom://guide/start",
            "unknown": "INVALID_FEATURE_ARGUMENT_SENTINEL"
        }),
        "mcp-features-invalid",
    );
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(McpFeaturesTool::shared_authority(authority.clone()))
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "mcp-features-invalid");
    assert_eq!(authority.call_count(), 0);
    assert!(policy.requests().is_empty());
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::PermissionRequested { .. }
            | TurnEvent::PermissionResolved { .. }
            | TurnEvent::ToolStarted { .. }
            | TurnEvent::ToolFinished { .. }
    )));
    let (message, output) = second_request_tool_output(&provider);
    assert!(output.is_error);
    assert_eq!(output.content["code"], "tool_error");
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("INVALID_FEATURE_ARGUMENT_SENTINEL")
    );
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}
