use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use futures_util::{StreamExt, stream};
use machine_god_core::{
    CancellationToken, ContentBlock, Engine, ModelProvider, PermissionDecision,
    PermissionGrantScope, Role, SessionId, SessionIncarnationId, StopReason, ToolName, ToolOutput,
    ToolSpec, TurnEvent,
};
use machine_god_native::{
    AiGatewayByteStream, AiGatewayProvider, AiGatewayTransport, AiGatewayTransportRequest,
};
use machine_god_testkit::{
    InMemorySessionStore, PermissionStep, ScriptedPermissionHandler, ScriptedTool, ToolStep,
};
use serde_json::{Value, json};

#[derive(Debug)]
struct EngineTransportState {
    scripts: VecDeque<Vec<Vec<u8>>>,
    bodies: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct EngineTransport {
    state: Arc<Mutex<EngineTransportState>>,
}

impl EngineTransport {
    fn new(scripts: impl IntoIterator<Item = Vec<Vec<u8>>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(EngineTransportState {
                scripts: scripts.into_iter().collect(),
                bodies: Vec::new(),
            })),
        }
    }

    fn bodies(&self) -> Vec<Vec<u8>> {
        self.state.lock().unwrap().bodies.clone()
    }
}

impl AiGatewayTransport for EngineTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> machine_god_core::BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>>
    {
        let script = {
            let mut state = self.state.lock().unwrap();
            state.bodies.push(request.body().to_vec());
            state.scripts.pop_front().expect("engine transport script")
        };
        Box::pin(async move {
            Ok(Box::pin(stream::iter(script.into_iter().map(Ok))) as AiGatewayByteStream)
        })
    }
}

fn fragment(bytes: &[u8], widths: &[usize]) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    let mut offset = 0;
    for width in widths.iter().copied().cycle() {
        if offset == bytes.len() {
            break;
        }
        let end = (offset + width).min(bytes.len());
        result.push(bytes[offset..end].to_vec());
        offset = end;
    }
    result
}

fn echo_tool() -> ScriptedTool {
    ScriptedTool::new(
        ToolSpec {
            name: ToolName::new("echo").unwrap(),
            description: "Echo a value".to_owned(),
            input_schema: json!({
                "type":"object",
                "properties":{"value":{"type":"string"}},
                "required":["value"]
            }),
        },
        [ToolStep::Output(ToolOutput::success(json!({
            "echoed":"hello"
        })))],
    )
}

#[test]
fn real_engine_consumes_fragmented_gateway_text_and_persists_it() {
    let response = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"text-1\",\"delta\":\"fast \"}\r\n\r\n",
        "data: {\"type\":\"text-delta\",\"id\":\"text-1\",\"delta\":\"answer\"}\r\n\r\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"},\"usage\":{\"inputTokens\":{\"total\":3},\"outputTokens\":{\"total\":2}}}\r\n\r\n"
    );
    let transport = EngineTransport::new([fragment(response.as_bytes(), &[1, 5, 2, 13])]);
    let provider = AiGatewayProvider::new("provider/model", Arc::new(transport.clone())).unwrap();
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session_id = SessionId::new("gateway-text-session").unwrap();
    let session = engine
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new("gateway-text-incarnation").unwrap(),
        )
        .unwrap();

    let events = futures_executor::block_on(async {
        session
            .prompt("question")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    });
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));
    let record = store.record(&session_id).unwrap();
    assert_eq!(record.messages.len(), 2);
    assert_eq!(record.messages[0].role, Role::User);
    assert_eq!(record.messages[1].role, Role::Assistant);
    assert_eq!(
        record.messages[1].content,
        [ContentBlock::Text {
            text: "fast answer".to_owned()
        }]
    );
    assert_eq!(transport.bodies().len(), 1);
}

#[test]
fn real_engine_completes_gateway_tool_round_and_persists_request_sequence() {
    let first = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"call-1\",\"toolName\":\"echo\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"call-1\",\"delta\":\"{\\\"value\\\":\\\"hello\\\"}\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"call-1\"}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-1\",\"toolName\":\"echo\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    let second = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"text-2\",\"delta\":\"tool complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    );
    let transport = EngineTransport::new([
        fragment(first.as_bytes(), &[2, 1, 17, 3]),
        fragment(second.as_bytes(), &[7, 4, 1]),
    ]);
    let provider = AiGatewayProvider::new("provider/model", Arc::new(transport.clone())).unwrap();
    assert_eq!(provider.name(), "vercel_ai_gateway");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let tool = echo_tool();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(tool.clone())
        .build()
        .unwrap();
    let session_id = SessionId::new("gateway-tool-session").unwrap();
    let session = engine
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new("gateway-tool-incarnation").unwrap(),
        )
        .unwrap();

    let events = futures_executor::block_on(async {
        session
            .prompt("use echo")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    });
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));
    assert_eq!(policy.requests().len(), 1);
    assert_eq!(tool.invocations().len(), 1);
    assert_eq!(tool.invocations()[0].arguments, json!({"value":"hello"}));

    let bodies = transport.bodies();
    assert_eq!(bodies.len(), 2);
    let first_request: Value = serde_json::from_slice(&bodies[0]).unwrap();
    assert_eq!(first_request["prompt"].as_array().unwrap().len(), 1);
    assert_eq!(first_request["tools"][0]["name"], "echo");
    let second_request: Value = serde_json::from_slice(&bodies[1]).unwrap();
    let prompt = second_request["prompt"].as_array().unwrap();
    assert_eq!(prompt.len(), 3);
    assert_eq!(prompt[1]["role"], "assistant");
    assert_eq!(prompt[1]["content"][0]["type"], "tool-call");
    assert_eq!(prompt[2]["role"], "tool");
    assert_eq!(prompt[2]["content"][0]["type"], "tool-result");
    assert_eq!(prompt[2]["content"][0]["toolCallId"], "call-1");
    assert_eq!(
        prompt[2]["content"][0]["output"]["value"],
        "{\"content\":{\"echoed\":\"hello\"},\"is_error\":false}"
    );

    let record = store.record(&session_id).unwrap();
    assert_eq!(record.messages.len(), 4);
    assert_eq!(record.messages[0].role, Role::User);
    assert_eq!(record.messages[1].role, Role::Assistant);
    assert!(matches!(
        record.messages[1].content.as_slice(),
        [ContentBlock::ToolCall { .. }]
    ));
    assert_eq!(record.messages[2].role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = record.messages[2].content.as_slice() else {
        panic!("expected durable tool result")
    };
    assert_eq!(output.content, json!({"echoed":"hello"}));
    assert_eq!(record.messages[3].role, Role::Assistant);
    assert_eq!(
        record.messages[3].content,
        [ContentBlock::Text {
            text: "tool complete".to_owned()
        }]
    );
}
