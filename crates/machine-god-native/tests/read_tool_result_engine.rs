use std::sync::{Arc, Mutex};

use futures_util::{StreamExt, stream};
use machine_god_core::{
    CancellationToken, ContentBlock, Engine, PermissionDecision, PermissionGrantScope, Role,
    SessionId, SessionIncarnationId, SessionStore, StopReason, ToolName, ToolOutput, ToolSpec,
    TurnEvent,
};
use machine_god_native::{
    AiGatewayByteStream, AiGatewayProvider, AiGatewayTransport, AiGatewayTransportRequest,
    READ_TOOL_RESULT_TOOL_NAME, ReadToolResultLimits, ReadToolResultTool,
};
use machine_god_testkit::{
    InMemorySessionStore, PermissionStep, ScriptedPermissionHandler, ScriptedTool, ToolStep,
};
use serde_json::{Value, json};

const LARGE_TOOL_NAME: &str = "large_output";
const LARGE_PAYLOAD_BYTES: usize = 20_000;
const READER_SCAN_BYTES: usize = 8 * 1024 * 1024;
const SIBLING_TOOL_NAME: &str = "sibling_output";

#[derive(Debug, Default)]
struct ContinuationState {
    bodies: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct ContinuationTransport {
    state: Arc<Mutex<ContinuationState>>,
    page_bytes: usize,
}

impl ContinuationTransport {
    fn new(page_bytes: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(ContinuationState::default())),
            page_bytes,
        }
    }

    fn bodies(&self) -> Vec<Vec<u8>> {
        self.state.lock().unwrap().bodies.clone()
    }
}

impl AiGatewayTransport for ContinuationTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> machine_god_core::BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>>
    {
        let response = {
            let mut state = self.state.lock().unwrap();
            state.bodies.push(request.body().to_vec());
            match state.bodies.len() {
                1 => tool_call_response(
                    "large-call",
                    LARGE_TOOL_NAME,
                    &json!({"value": "make it large"}),
                ),
                2 => {
                    let request: Value = serde_json::from_slice(&state.bodies[1]).unwrap();
                    let preview = projected_output(&request);
                    assert_eq!(preview["type"], "tool_result_preview");
                    assert_eq!(preview["total_bytes"], serialized_large_output_len());
                    assert_eq!(preview["is_error"], false);
                    assert_eq!(preview["read_more_with"], READ_TOOL_RESULT_TOOL_NAME);
                    assert!(preview["preview"].as_str().unwrap().len() <= 4_096);
                    let handle = preview["handle"].as_str().unwrap();
                    assert!(handle.starts_with("tool-result-sha256-"));
                    assert_eq!(handle.len(), 83);
                    tool_call_response(
                        "read-call",
                        READ_TOOL_RESULT_TOOL_NAME,
                        &json!({
                            "handle": handle,
                            "start_byte": 1,
                            "byte_count": self.page_bytes,
                        }),
                    )
                }
                3 => {
                    let reader_wire = reader_wire(&state.bodies[2]);
                    assert_eq!(reader_wire["is_error"], false);
                    assert_eq!(reader_wire["content"]["start_byte"], 1);
                    assert_eq!(reader_wire["content"]["has_more"], true);
                    assert_eq!(
                        reader_wire["content"]["serialized_tool_output"]
                            .as_str()
                            .unwrap()
                            .len(),
                        self.page_bytes
                    );
                    assert!(reader_wire.get("type").is_none());
                    text_response("continued from the exact durable result")
                }
                call => panic!("unexpected gateway request {call}"),
            }
        };
        Box::pin(async move {
            Ok(Box::pin(stream::iter(response.into_iter().map(Ok))) as AiGatewayByteStream)
        })
    }
}

#[derive(Clone, Debug)]
struct MultiCallContinuationTransport {
    state: Arc<Mutex<ContinuationState>>,
    page_bytes: usize,
}

impl MultiCallContinuationTransport {
    fn new(page_bytes: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(ContinuationState::default())),
            page_bytes,
        }
    }

    fn bodies(&self) -> Vec<Vec<u8>> {
        self.state.lock().unwrap().bodies.clone()
    }
}

impl AiGatewayTransport for MultiCallContinuationTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> machine_god_core::BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>>
    {
        let response = {
            let mut state = self.state.lock().unwrap();
            state.bodies.push(request.body().to_vec());
            match state.bodies.len() {
                1 => tool_call_response(
                    "large-call",
                    LARGE_TOOL_NAME,
                    &json!({"value": "make it large"}),
                ),
                2 => {
                    let request: Value = serde_json::from_slice(&state.bodies[1]).unwrap();
                    let preview = projected_output(&request);
                    assert_eq!(preview["type"], "tool_result_preview");
                    assert_eq!(preview["total_bytes"], serialized_large_output_len());
                    assert_eq!(preview["is_error"], false);
                    assert_eq!(preview["read_more_with"], READ_TOOL_RESULT_TOOL_NAME);
                    let handle = preview["handle"].as_str().unwrap();
                    tool_calls_response(&[
                        (
                            "sibling-before-call",
                            SIBLING_TOOL_NAME,
                            json!({"position": "before"}),
                        ),
                        (
                            "read-call",
                            READ_TOOL_RESULT_TOOL_NAME,
                            json!({
                                "handle": handle,
                                "start_byte": 1,
                                "byte_count": self.page_bytes,
                            }),
                        ),
                        (
                            "sibling-after-call",
                            SIBLING_TOOL_NAME,
                            json!({"position": "after"}),
                        ),
                    ])
                }
                3 => {
                    let before = tool_result_wire(&state.bodies[2], "sibling-before-call");
                    assert_eq!(before, ToolOutput::success(json!({"position": "before"})));
                    let reader = tool_result_wire(&state.bodies[2], "read-call");
                    assert!(!reader.is_error);
                    assert_eq!(reader.content["start_byte"], 1);
                    assert_eq!(reader.content["has_more"], true);
                    assert_eq!(
                        reader.content["serialized_tool_output"]
                            .as_str()
                            .unwrap()
                            .len(),
                        self.page_bytes
                    );
                    let after = tool_result_wire(&state.bodies[2], "sibling-after-call");
                    assert_eq!(after, ToolOutput::success(json!({"position": "after"})));
                    text_response("continued past current-round placeholders")
                }
                call => panic!("unexpected gateway request {call}"),
            }
        };
        Box::pin(async move {
            Ok(Box::pin(stream::iter(response.into_iter().map(Ok))) as AiGatewayByteStream)
        })
    }
}

fn sse(value: &Value) -> Vec<u8> {
    format!("data: {value}\n\n").into_bytes()
}

fn tool_call_response(call_id: &str, tool_name: &str, arguments: &Value) -> Vec<Vec<u8>> {
    tool_calls_response(&[(call_id, tool_name, arguments.clone())])
}

fn tool_calls_response(calls: &[(&str, &str, Value)]) -> Vec<Vec<u8>> {
    let mut response = Vec::with_capacity(calls.len() * 4 + 1);
    for (call_id, tool_name, arguments) in calls {
        response.push(sse(&json!({
            "type": "tool-input-start",
            "id": call_id,
            "toolName": tool_name,
        })));
        response.push(sse(&json!({
            "type": "tool-input-delta",
            "id": call_id,
            "delta": arguments.to_string(),
        })));
        response.push(sse(&json!({
            "type": "tool-input-end",
            "id": call_id,
        })));
        response.push(sse(&json!({
            "type": "tool-call",
            "toolCallId": call_id,
            "toolName": tool_name,
        })));
    }
    response.push(sse(&json!({
        "type": "finish",
        "finishReason": {"unified": "tool-calls"},
    })));
    response
}

fn text_response(text: &str) -> Vec<Vec<u8>> {
    vec![
        sse(&json!({
            "type": "text-delta",
            "id": "text-final",
            "delta": text,
        })),
        sse(&json!({
            "type": "finish",
            "finishReason": {"unified": "stop"},
        })),
    ]
}

fn projected_output(request: &Value) -> Value {
    let value = request["prompt"][2]["content"][0]["output"]["value"]
        .as_str()
        .expect("projected tool output string");
    serde_json::from_str(value).expect("structured preview")
}

fn reader_wire(body: &[u8]) -> Value {
    serde_json::to_value(tool_result_wire(body, "read-call")).unwrap()
}

fn tool_result_wire(body: &[u8], call_id: &str) -> ToolOutput {
    let request: Value = serde_json::from_slice(body).unwrap();
    let result = request["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .find(|content| {
            content["type"].as_str() == Some("tool-result")
                && content["toolCallId"].as_str() == Some(call_id)
        })
        .unwrap_or_else(|| panic!("missing tool result for {call_id}"));
    serde_json::from_str(
        result["output"]["value"]
            .as_str()
            .expect("complete tool result string"),
    )
    .unwrap()
}

fn large_tool() -> ScriptedTool {
    ScriptedTool::new(
        ToolSpec {
            name: ToolName::new(LARGE_TOOL_NAME).unwrap(),
            description: "Return a deterministic large value".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {"value": {"type": "string"}},
                "required": ["value"],
                "additionalProperties": false,
            }),
        },
        [ToolStep::Output(ToolOutput::success(json!({
            "payload": "\\".repeat(LARGE_PAYLOAD_BYTES),
        })))],
    )
}

fn serialized_large_output_len() -> usize {
    serde_json::to_vec(&ToolOutput::success(json!({
        "payload": "\\".repeat(LARGE_PAYLOAD_BYTES),
    })))
    .unwrap()
    .len()
}

fn constrained_reader(store: Arc<dyn SessionStore>) -> ReadToolResultTool {
    ReadToolResultTool::with_limits(
        store,
        ReadToolResultLimits::new(1, 1, READER_SCAN_BYTES).unwrap(),
    )
    .unwrap()
}

fn sibling_tool() -> ScriptedTool {
    ScriptedTool::new(
        ToolSpec {
            name: ToolName::new(SIBLING_TOOL_NAME).unwrap(),
            description: "Return the sibling's deterministic position".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {"position": {"enum": ["before", "after"]}},
                "required": ["position"],
                "additionalProperties": false,
            }),
        },
        [
            ToolStep::Output(ToolOutput::success(json!({"position": "before"}))),
            ToolStep::Output(ToolOutput::success(json!({"position": "after"}))),
        ],
    )
}

fn assert_multi_call_record(
    store: &InMemorySessionStore,
    session_id: &SessionId,
    page_bytes: usize,
) {
    let record = store.record(session_id).unwrap();
    assert_eq!(record.messages.len(), 8);
    assert_eq!(record.messages[2].role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = record.messages[2].content.as_slice() else {
        panic!("expected durable large result")
    };
    assert_eq!(
        output.content["payload"].as_str().unwrap().len(),
        LARGE_PAYLOAD_BYTES
    );
    assert_eq!(record.messages[4].role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = record.messages[4].content.as_slice() else {
        panic!("expected durable first sibling result")
    };
    assert_eq!(*output, ToolOutput::success(json!({"position": "before"})));
    assert_eq!(record.messages[5].role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = record.messages[5].content.as_slice() else {
        panic!("expected durable reader result")
    };
    assert_eq!(output.content["start_byte"], 1);
    assert_eq!(
        output.content["serialized_tool_output"]
            .as_str()
            .unwrap()
            .len(),
        page_bytes
    );
    assert_eq!(record.messages[6].role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = record.messages[6].content.as_slice() else {
        panic!("expected durable second sibling result")
    };
    assert_eq!(*output, ToolOutput::success(json!({"position": "after"})));
    assert_eq!(record.messages[7].role, Role::Assistant);
}

#[test]
fn large_result_preview_continues_through_reader_without_new_permission_or_duplicate_storage() {
    for page_bytes in [8_192, 16_384] {
        let transport = ContinuationTransport::new(page_bytes);
        let provider =
            AiGatewayProvider::new("provider/model", Arc::new(transport.clone())).unwrap();
        let store = InMemorySessionStore::new();
        let reader_store: Arc<dyn SessionStore> = Arc::new(store.clone());
        let policy =
            ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })]);
        let large_tool = large_tool();
        let engine = Engine::builder()
            .provider(provider)
            .session_store(store.clone())
            .permission_handler(policy.clone())
            .tool(large_tool.clone())
            .tool(constrained_reader(reader_store))
            .build()
            .unwrap();
        let session_id = SessionId::new(format!("read-tool-result-engine-{page_bytes}")).unwrap();
        let session = engine
            .create_session(
                session_id.clone(),
                SessionIncarnationId::new(format!(
                    "read-tool-result-engine-incarnation-{page_bytes}"
                ))
                .unwrap(),
            )
            .unwrap();

        let events = futures_executor::block_on(async {
            session
                .prompt("read the large result")
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
        assert_eq!(large_tool.invocations().len(), 1);

        let bodies = transport.bodies();
        assert_eq!(bodies.len(), 3);
        let second: Value = serde_json::from_slice(&bodies[1]).unwrap();
        let preview = projected_output(&second);
        assert!(preview["preview"].as_str().unwrap().len() <= 4_096);
        let reader_wire = reader_wire(&bodies[2]);
        assert_eq!(reader_wire["is_error"], false);
        assert_eq!(reader_wire["content"]["start_byte"], 1);
        assert_eq!(reader_wire["content"]["has_more"], true);
        assert_eq!(
            reader_wire["content"]["serialized_tool_output"]
                .as_str()
                .unwrap()
                .len(),
            page_bytes
        );
        assert!(
            reader_wire["content"]["serialized_tool_output"]
                .as_str()
                .unwrap()
                .starts_with("{\"content\":{\"payload\":\"\\\\")
        );

        let record = store.record(&session_id).unwrap();
        assert_eq!(record.messages.len(), 6);
        assert_eq!(record.messages[2].role, Role::Tool);
        let [ContentBlock::ToolResult { output, .. }] = record.messages[2].content.as_slice()
        else {
            panic!("expected durable large result")
        };
        assert_eq!(
            output.content["payload"].as_str().unwrap().len(),
            LARGE_PAYLOAD_BYTES
        );
        assert_eq!(record.messages[4].role, Role::Tool);
        let [ContentBlock::ToolResult { output, .. }] = record.messages[4].content.as_slice()
        else {
            panic!("expected durable reader result")
        };
        assert_eq!(output.content["start_byte"], 1);
        assert_eq!(
            output.content["serialized_tool_output"]
                .as_str()
                .unwrap()
                .len(),
            page_bytes
        );
        assert_eq!(record.messages[5].role, Role::Assistant);
    }
}

#[test]
fn current_round_siblings_do_not_consume_the_prior_result_scan_allowance() {
    let page_bytes = 8_192;
    let transport = MultiCallContinuationTransport::new(page_bytes);
    let provider = AiGatewayProvider::new("provider/model", Arc::new(transport.clone())).unwrap();
    let store = InMemorySessionStore::new();
    let reader_store: Arc<dyn SessionStore> = Arc::new(store.clone());
    let policy = ScriptedPermissionHandler::new([
        PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        }),
        PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        }),
        PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        }),
    ]);
    let large_tool = large_tool();
    let sibling_tool = sibling_tool();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(large_tool.clone())
        .tool(sibling_tool.clone())
        .tool(constrained_reader(reader_store))
        .build()
        .unwrap();
    let session_id = SessionId::new("read-tool-result-engine-sibling-round").unwrap();
    let session = engine
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new("read-tool-result-engine-sibling-round-incarnation").unwrap(),
        )
        .unwrap();

    let events = futures_executor::block_on(async {
        session
            .prompt("read the large result after running sibling calls")
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
    assert_eq!(policy.requests().len(), 3);
    assert_eq!(large_tool.invocations().len(), 1);
    let sibling_invocations = sibling_tool.invocations();
    assert_eq!(sibling_invocations.len(), 2);
    assert_eq!(
        sibling_invocations[0].arguments,
        json!({"position": "before"})
    );
    assert_eq!(
        sibling_invocations[1].arguments,
        json!({"position": "after"})
    );

    let bodies = transport.bodies();
    assert_eq!(bodies.len(), 3);
    let reader = tool_result_wire(&bodies[2], "read-call");
    assert!(!reader.is_error);
    assert_eq!(reader.content["start_byte"], 1);
    assert_eq!(reader.content["has_more"], true);
    assert_eq!(
        reader.content["serialized_tool_output"]
            .as_str()
            .unwrap()
            .len(),
        page_bytes
    );

    assert_multi_call_record(&store, &session_id, page_bytes);
}
