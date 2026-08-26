use std::collections::VecDeque;
use std::pin::Pin;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use futures_util::{FutureExt, Stream, StreamExt, future};
use machine_god_core::{
    CancellationToken, ContentBlock, InferenceOptions, Message, ModelEvent, ModelEventStream,
    ModelProvider, ModelRequest, ProviderError, ProviderErrorKind, Role, SessionId,
    SessionIncarnationId, StopReason, TokenUsage, ToolCall, ToolCallId, ToolName, ToolOutput,
    ToolSpec, TurnId,
};
use machine_god_native::{
    AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION, AI_GATEWAY_PROTOCOL_VERSION,
    AI_GATEWAY_PROVIDER_NAME, AiGatewayByteStream, AiGatewayConfigErrorKind, AiGatewayLimits,
    AiGatewayProvider, AiGatewayTransport, AiGatewayTransportRequest,
};
use serde_json::{Value, json};

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordedRequest {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[derive(Clone, Debug)]
enum ByteStep {
    Chunk(Vec<u8>),
    Error(ProviderError),
    CancelThenChunk(Vec<u8>),
    CancelThenError(ProviderError),
    CancelThenEof,
    Pending,
}

#[derive(Clone, Debug)]
enum TransportStep {
    Bytes(Vec<ByteStep>),
    Error(ProviderError),
    CancelThenError(ProviderError),
    Pending,
}

#[derive(Debug)]
struct TransportState {
    steps: VecDeque<TransportStep>,
    requests: Vec<RecordedRequest>,
}

#[derive(Clone, Debug)]
struct ScriptedTransport {
    state: Arc<Mutex<TransportState>>,
}

impl ScriptedTransport {
    fn new(steps: impl IntoIterator<Item = TransportStep>) -> Self {
        Self {
            state: Arc::new(Mutex::new(TransportState {
                steps: steps.into_iter().collect(),
                requests: Vec::new(),
            })),
        }
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl AiGatewayTransport for ScriptedTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> machine_god_core::BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        let snapshot = RecordedRequest {
            headers: request
                .headers()
                .iter()
                .map(|header| (header.name().to_owned(), header.value().to_owned()))
                .collect(),
            body: request.body().to_vec(),
        };
        let step = {
            let mut state = self.state.lock().unwrap();
            state.requests.push(snapshot);
            state.steps.pop_front()
        };
        Box::pin(async move {
            match step {
                Some(TransportStep::Bytes(steps)) => Ok(Box::pin(ScriptedByteStream {
                    steps: steps.into(),
                    cancellation,
                }) as AiGatewayByteStream),
                Some(TransportStep::Error(error)) => Err(error),
                Some(TransportStep::CancelThenError(error)) => {
                    cancellation.cancel();
                    Err(error)
                }
                Some(TransportStep::Pending) => future::pending().await,
                None => Err(ProviderError::new(
                    ProviderErrorKind::Other,
                    "test_transport_script_exhausted",
                    "scripted transport was called after its script was exhausted",
                    false,
                )),
            }
        })
    }
}

#[derive(Debug)]
struct ScriptedByteStream {
    steps: VecDeque<ByteStep>,
    cancellation: CancellationToken,
}

#[derive(Debug, Default)]
struct WakeCounter(AtomicUsize);

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

impl Stream for ScriptedByteStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.steps.front() {
            Some(ByteStep::Pending) => Poll::Pending,
            Some(_) => match self.steps.pop_front().unwrap() {
                ByteStep::Chunk(bytes) => Poll::Ready(Some(Ok(bytes))),
                ByteStep::Error(error) => Poll::Ready(Some(Err(error))),
                ByteStep::CancelThenChunk(bytes) => {
                    self.cancellation.cancel();
                    Poll::Ready(Some(Ok(bytes)))
                }
                ByteStep::CancelThenError(error) => {
                    self.cancellation.cancel();
                    Poll::Ready(Some(Err(error)))
                }
                ByteStep::CancelThenEof => {
                    self.cancellation.cancel();
                    Poll::Ready(None)
                }
                ByteStep::Pending => unreachable!(),
            },
            None => Poll::Ready(None),
        }
    }
}

fn bytes(body: impl Into<Vec<u8>>) -> TransportStep {
    TransportStep::Bytes(vec![ByteStep::Chunk(body.into())])
}

fn fragments(body: &[u8], widths: &[usize]) -> TransportStep {
    let mut chunks = Vec::new();
    let mut offset = 0;
    for width in widths.iter().copied().cycle() {
        if offset == body.len() {
            break;
        }
        let end = (offset + width).min(body.len());
        chunks.push(ByteStep::Chunk(body[offset..end].to_vec()));
        offset = end;
    }
    TransportStep::Bytes(chunks)
}

fn provider(transport: &ScriptedTransport) -> AiGatewayProvider {
    AiGatewayProvider::new("provider/default", Arc::new(transport.clone())).unwrap()
}

fn request(messages: Vec<Message>) -> ModelRequest {
    ModelRequest {
        session_id: SessionId::new("session-1").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("incarnation-1").unwrap(),
        turn_id: TurnId::new("turn-1").unwrap(),
        messages,
        tools: Vec::new(),
        options: InferenceOptions::default(),
    }
}

fn start(
    provider: &AiGatewayProvider,
    request: ModelRequest,
    cancellation: CancellationToken,
) -> Result<ModelEventStream, ProviderError> {
    futures_executor::block_on(provider.stream(request, cancellation))
}

fn expect_start_error(
    result: Result<ModelEventStream, ProviderError>,
    context: &str,
) -> ProviderError {
    match result {
        Ok(_) => panic!("{context}: provider unexpectedly returned a stream"),
        Err(error) => error,
    }
}

fn collect(step: TransportStep) -> Result<Vec<ModelEvent>, ProviderError> {
    let transport = ScriptedTransport::new([step]);
    let provider = provider(&transport);
    collect_with_provider(&provider)
}

fn collect_with_limits(
    step: TransportStep,
    limits: AiGatewayLimits,
) -> Result<Vec<ModelEvent>, ProviderError> {
    let transport = ScriptedTransport::new([step]);
    let provider =
        AiGatewayProvider::with_limits("provider/default", Arc::new(transport), limits).unwrap();
    collect_with_provider(&provider)
}

fn collect_with_provider(provider: &AiGatewayProvider) -> Result<Vec<ModelEvent>, ProviderError> {
    let stream = start(
        provider,
        request(vec![Message::text(Role::User, "hello")]),
        CancellationToken::new(),
    )?;
    futures_executor::block_on(stream.collect::<Vec<_>>())
        .into_iter()
        .collect()
}

fn protocol_error(result: Result<Vec<ModelEvent>, ProviderError>) -> ProviderError {
    let error = result.expect_err("fixture must be rejected");
    assert_eq!(error.kind, ProviderErrorKind::Protocol);
    assert!(!error.retryable);
    error
}

fn finish(reason: &str) -> String {
    format!("data: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"{reason}\"}}}}\n\n")
}

#[test]
fn provider_identity_configuration_and_debug_are_fixed_and_redacted() {
    let transport = ScriptedTransport::new([]);
    let provider = AiGatewayProvider::new("provider/secret-model", Arc::new(transport)).unwrap();
    assert_eq!(provider.name(), AI_GATEWAY_PROVIDER_NAME);
    assert_eq!(AI_GATEWAY_PROTOCOL_VERSION, "0.0.1");
    assert_eq!(AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION, "4");
    let debug = format!("{provider:?}");
    assert!(debug.contains("AiGatewayProvider"));
    assert!(!debug.contains("secret-model"));

    let invalid = AiGatewayProvider::new("", Arc::new(ScriptedTransport::new([]))).unwrap_err();
    assert_eq!(invalid.kind(), AiGatewayConfigErrorKind::InvalidModel);
    assert!(!format!("{invalid:?}").contains("secret"));
    AiGatewayProvider::new("m".repeat(128), Arc::new(ScriptedTransport::new([]))).unwrap();
    for invalid_model in [
        "m".repeat(129),
        "provider bad".to_owned(),
        "modèle".to_owned(),
    ] {
        let error = AiGatewayProvider::new(invalid_model, Arc::new(ScriptedTransport::new([])))
            .unwrap_err();
        assert_eq!(error.kind(), AiGatewayConfigErrorKind::InvalidModel);
    }

    let limits = AiGatewayLimits {
        max_request_bytes: 0,
        ..AiGatewayLimits::default()
    };
    let invalid = AiGatewayProvider::with_limits(
        "provider/model",
        Arc::new(ScriptedTransport::new([])),
        limits,
    )
    .unwrap_err();
    assert_eq!(invalid.kind(), AiGatewayConfigErrorKind::InvalidLimits);
}

#[test]
fn request_model_override_uses_the_same_visible_ascii_boundary() {
    let transport = ScriptedTransport::new([bytes(finish("stop"))]);
    let gateway = provider(&transport);
    let mut valid = request(vec![Message::text(Role::User, "hello")]);
    valid.options.model = Some("m".repeat(128));
    let stream = start(&gateway, valid, CancellationToken::new()).unwrap();
    futures_executor::block_on(stream.collect::<Vec<_>>());
    assert_eq!(transport.requests().len(), 1);

    for invalid_model in [
        "m".repeat(129),
        "provider bad".to_owned(),
        "bad\nmodel".to_owned(),
    ] {
        let transport = ScriptedTransport::new([]);
        let provider = provider(&transport);
        let mut invalid = request(vec![Message::text(Role::User, "hello")]);
        invalid.options.model = Some(invalid_model);
        let error = expect_start_error(
            start(&provider, invalid, CancellationToken::new()),
            "invalid model override",
        );
        assert_eq!(error.code, "gateway_invalid_model");
        assert!(transport.requests().is_empty());
    }
}

#[test]
fn request_encoding_matches_the_pinned_gateway_shape() {
    let transport = ScriptedTransport::new([bytes(finish("stop"))]);
    let provider = provider(&transport);
    let call = ToolCall {
        id: ToolCallId::new("call-1").unwrap(),
        name: ToolName::new("read_file").unwrap(),
        arguments: json!({"path":"README.md"}),
    };
    let mut model_request = request(vec![
        Message::text(Role::System, "system guidance"),
        Message::text(Role::User, "first question"),
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "checking".to_owned(),
                },
                ContentBlock::ToolCall { call: call.clone() },
            ],
        },
        Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id: call.id,
                output: ToolOutput::success(json!({"contents":"hello"})),
            }],
        },
        Message::text(Role::User, "continue"),
    ]);
    model_request.tools.push(ToolSpec {
        name: ToolName::new("read_file").unwrap(),
        description: "Read one file".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        }),
    });
    model_request.options.model = Some("provider/override".to_owned());
    model_request.options.max_output_tokens = Some(321);

    let stream = start(&provider, model_request, CancellationToken::new()).unwrap();
    let events = futures_executor::block_on(stream.collect::<Vec<_>>());
    assert_eq!(
        events,
        [Ok(ModelEvent::Stop {
            reason: StopReason::Completed
        })]
    );

    let requests = transport.requests();
    let [recorded] = requests.as_slice() else {
        panic!("expected one request")
    };
    assert_eq!(
        recorded.headers,
        [
            ("content-type".to_owned(), "application/json".to_owned()),
            ("ai-gateway-protocol-version".to_owned(), "0.0.1".to_owned()),
            (
                "ai-language-model-specification-version".to_owned(),
                "4".to_owned()
            ),
            (
                "ai-language-model-id".to_owned(),
                "provider/override".to_owned()
            ),
            ("ai-language-model-streaming".to_owned(), "true".to_owned()),
            ("x-session-id".to_owned(), "session-1".to_owned()),
            ("x-session-affinity".to_owned(), "session-1".to_owned()),
        ]
    );
    let body: Value = serde_json::from_slice(&recorded.body).unwrap();
    assert_eq!(
        body,
        json!({
            "prompt": [
                {"role":"system", "content":"system guidance"},
                {"role":"user", "content":[{"type":"text", "text":"first question"}]},
                {"role":"assistant", "content":[
                    {"type":"text", "text":"checking"},
                    {"type":"tool-call", "toolCallId":"call-1", "toolName":"read_file", "input":{"path":"README.md"}}
                ]},
                {"role":"tool", "content":[{
                    "type":"tool-result", "toolCallId":"call-1", "toolName":"read_file",
                    "output":{"type":"text", "value":"{\"content\":{\"contents\":\"hello\"},\"is_error\":false}"}
                }]},
                {"role":"user", "content":[{"type":"text", "text":"continue"}]}
            ],
            "tools": [{
                "type":"function", "name":"read_file", "description":"Read one file",
                "inputSchema":{"type":"object", "properties":{"path":{"type":"string"}}, "required":["path"]}
            }],
            "toolChoice":{"type":"auto"},
            "maxOutputTokens":321
        })
    );
    for absent in ["model", "temperature", "metadata", "stream"] {
        assert!(body.get(absent).is_none(), "unexpected body field {absent}");
    }
}

#[test]
fn unsupported_optional_inference_fields_are_ignored_and_omitted() {
    let transport = ScriptedTransport::new([bytes(finish("stop"))]);
    let provider = provider(&transport);
    let mut value = request(vec![Message::text(Role::User, "hello")]);
    value.options.temperature = Some(0.5);
    value
        .options
        .metadata
        .insert("secret".to_owned(), json!({"nested":["value"]}));
    let stream = start(&provider, value, CancellationToken::new()).unwrap();
    let events = futures_executor::block_on(stream.collect::<Vec<_>>())
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        events,
        [ModelEvent::Stop {
            reason: StopReason::Completed
        }]
    );
    let requests = transport.requests();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(body.get("temperature").is_none());
    assert!(body.get("metadata").is_none());
    assert!(!String::from_utf8_lossy(&requests[0].body).contains("secret"));
}

#[test]
fn request_rejects_unsupported_content_without_transport() {
    let cases = [
        request(vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Json {
                value: json!({"x":1}),
            }],
        }]),
        request(Vec::new()),
        request(vec![Message {
            role: Role::Tool,
            content: vec![ContentBlock::Text {
                text: "orphan".to_owned(),
            }],
        }]),
    ];

    for case in cases {
        let transport = ScriptedTransport::new([]);
        let provider = provider(&transport);
        let error = expect_start_error(
            start(&provider, case, CancellationToken::new()),
            "invalid request",
        );
        assert_eq!(error.kind, ProviderErrorKind::InvalidRequest);
        assert!(!error.retryable);
        assert!(transport.requests().is_empty());
    }
}

#[test]
fn fragmented_crlf_text_reasoning_usage_and_finish_are_ordered() {
    let body = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"text-1\",\"delta\":\"hel\"}\r\n\r\n",
        "data: {\"type\":\"reasoning-delta\",\"id\":\"reason-1\",\"delta\":\"think\"}\r\n\r\n",
        "data: {\"type\":\"text-delta\",\"id\":\"text-1\",\"delta\":\"lo\"}\r\n\r\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"},",
        "\"usage\":{\"inputTokens\":{\"total\":13,\"cacheRead\":5},\"outputTokens\":{\"total\":8}}}\r\n\r\n"
    );
    let events = collect(fragments(body.as_bytes(), &[1, 2, 7, 3, 11])).unwrap();
    assert_eq!(
        events,
        [
            ModelEvent::TextDelta {
                text: "hel".to_owned()
            },
            ModelEvent::ReasoningDelta {
                text: "think".to_owned()
            },
            ModelEvent::TextDelta {
                text: "lo".to_owned()
            },
            ModelEvent::Usage {
                usage: TokenUsage {
                    input_tokens: 13,
                    output_tokens: 8,
                    cached_input_tokens: 5
                }
            },
            ModelEvent::Stop {
                reason: StopReason::Completed
            },
        ]
    );
}

#[test]
fn complete_object_and_serialized_tool_calls_are_decoded() {
    let body = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-object\",\"toolName\":\"read_file\",\"input\":{\"path\":\"a\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-string\",\"toolName\":\"list_files\",\"input\":\"{\\\"path\\\":\\\"b\\\"}\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    let events = collect(bytes(body)).unwrap();
    assert_eq!(
        events,
        [
            ModelEvent::ToolCall {
                call: ToolCall {
                    id: ToolCallId::new("call-object").unwrap(),
                    name: ToolName::new("read_file").unwrap(),
                    arguments: json!({"path":"a"})
                }
            },
            ModelEvent::ToolCall {
                call: ToolCall {
                    id: ToolCallId::new("call-string").unwrap(),
                    name: ToolName::new("list_files").unwrap(),
                    arguments: json!({"path":"b"})
                }
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls
            },
        ]
    );
}

#[test]
fn streamed_tool_input_falls_back_only_after_start_delta_and_end() {
    let body = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"call-stream\",\"toolName\":\"read_file\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"call-stream\",\"delta\":\"{\\\"path\\\":\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"call-stream\",\"delta\":\"\\\"README.md\\\"}\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"call-stream\"}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-stream\",\"toolName\":\"read_file\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    let events = collect(fragments(body.as_bytes(), &[9, 1, 4])).unwrap();
    assert_eq!(
        events,
        [
            ModelEvent::ToolCall {
                call: ToolCall {
                    id: ToolCallId::new("call-stream").unwrap(),
                    name: ToolName::new("read_file").unwrap(),
                    arguments: json!({"path":"README.md"})
                }
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls
            },
        ]
    );
}

#[test]
fn final_tool_calls_reconcile_unique_provisional_ids_by_name_and_input() {
    let body = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"provisional-a\",\"toolName\":\"operate\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"provisional-a\",\"delta\":\"{\\\"alpha\\\":1,\\\"beta\\\":2}\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"provisional-a\"}\n\n",
        "data: {\"type\":\"tool-input-start\",\"id\":\"provisional-b\",\"toolName\":\"operate\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"provisional-b\",\"delta\":\"{\\\"alpha\\\":3,\\\"beta\\\":4}\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"provisional-b\"}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"final-b\",\"toolName\":\"operate\",\"input\":{\"beta\":4,\"alpha\":3}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"final-a\",\"toolName\":\"operate\",\"input\":{\"beta\":2,\"alpha\":1}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    let events = collect(fragments(body.as_bytes(), &[3, 11, 1, 19])).unwrap();
    assert_eq!(events.len(), 3);
    assert!(matches!(
        &events[0],
        ModelEvent::ToolCall { call }
            if call.id.as_str() == "final-b"
                && call.arguments == json!({"alpha":3,"beta":4})
    ));
    assert!(matches!(
        &events[1],
        ModelEvent::ToolCall { call }
            if call.id.as_str() == "final-a"
                && call.arguments == json!({"alpha":1,"beta":2})
    ));
    assert_eq!(
        events[2],
        ModelEvent::Stop {
            reason: StopReason::ToolCalls
        }
    );
}

#[test]
fn explicit_final_input_corrects_same_id_streamed_input() {
    let body = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"call-1\",\"toolName\":\"read_file\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"call-1\",\"delta\":\"{\\\"path\\\":\\\"old\\\"}\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"call-1\"}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-1\",\"toolName\":\"read_file\",\"input\":{\"path\":\"new\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    let events = collect(bytes(body)).unwrap();
    assert!(matches!(
        &events[0],
        ModelEvent::ToolCall { call } if call.arguments == json!({"path":"new"})
    ));
}

#[test]
fn authoritative_exact_id_final_tolerates_malformed_or_late_provisional_completion() {
    let malformed_ended = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"call-1\",\"toolName\":\"echo\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"call-1\",\"delta\":\"{\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"call-1\"}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-1\",\"toolName\":\"echo\",\"input\":{\"value\":1}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    let events = collect(bytes(malformed_ended)).unwrap();
    assert!(matches!(
        &events[0],
        ModelEvent::ToolCall { call } if call.arguments == json!({"value":1})
    ));

    let final_before_end = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"call-2\",\"toolName\":\"echo\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"call-2\",\"delta\":\"{\"}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-2\",\"toolName\":\"echo\",\"input\":{\"value\":2}}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"call-2\",\"delta\":\"}\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"call-2\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    let events = collect(bytes(final_before_end)).unwrap();
    assert!(matches!(
        &events[0],
        ModelEvent::ToolCall { call } if call.arguments == json!({"value":2})
    ));

    let required_fallback = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"call-3\",\"toolName\":\"echo\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"call-3\",\"delta\":\"{\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"call-3\"}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-3\",\"toolName\":\"echo\"}\n\n"
    );
    protocol_error(collect(bytes(required_fallback)));
}

#[test]
fn changed_id_reconciliation_normalizes_signed_floating_zero() {
    let body = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"provisional\",\"toolName\":\"measure\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"provisional\",\"delta\":\"{\\\"value\\\":-0.0}\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"provisional\"}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"final\",\"toolName\":\"measure\",\"input\":{\"value\":0.0}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    let events = collect(bytes(body)).unwrap();
    assert!(matches!(
        &events[0],
        ModelEvent::ToolCall { call } if call.id.as_str() == "final"
    ));
}

#[test]
fn ambiguous_provisional_tool_call_reconciliation_is_rejected() {
    let body = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"p-1\",\"toolName\":\"echo\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"p-1\",\"delta\":\"{}\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"p-1\"}\n\n",
        "data: {\"type\":\"tool-input-start\",\"id\":\"p-2\",\"toolName\":\"echo\"}\n\n",
        "data: {\"type\":\"tool-input-delta\",\"id\":\"p-2\",\"delta\":\"{}\"}\n\n",
        "data: {\"type\":\"tool-input-end\",\"id\":\"p-2\"}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"final\",\"toolName\":\"echo\",\"input\":{}}\n\n"
    );
    protocol_error(collect(bytes(body)));
}

#[test]
fn all_canonical_finish_reasons_map_and_emit_one_stop() {
    let cases = [
        ("stop", StopReason::Completed),
        ("length", StopReason::MaxOutputTokens),
        ("content-filter", StopReason::ContentFilter),
        ("tool-calls", StopReason::ToolCalls),
        ("other", StopReason::Other("other".to_owned())),
    ];
    for (wire, expected) in cases {
        let body = if wire == "tool-calls" {
            format!(
                "data: {{\"type\":\"tool-call\",\"toolCallId\":\"call-1\",\"toolName\":\"echo\",\"input\":{{}}}}\n\n{}",
                finish(wire)
            )
        } else {
            finish(wire)
        };
        let events = collect(bytes(body)).unwrap();
        assert_eq!(events.last(), Some(&ModelEvent::Stop { reason: expected }));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ModelEvent::Stop { .. }))
                .count(),
            1
        );
    }
}

#[test]
fn malformed_framing_json_schema_and_finish_are_protocol_errors() {
    let cases: Vec<Vec<u8>> = vec![
        b"data: {not json}\n\n".to_vec(),
        b"data: 1\n\n".to_vec(),
        b"data: {\"type\":\"text-delta\",\"delta\":1}\n\n".to_vec(),
        b"data: {\"type\":\"finish\",\"finishReason\":\"stop\"}\n\n".to_vec(),
        b"data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"future\"}}\n\n".to_vec(),
        vec![b'd', b'a', b't', b'a', b':', b' ', 0xff, b'\n', b'\n'],
    ];
    for case in cases {
        protocol_error(collect(bytes(case)));
    }
}

#[test]
fn duplicate_keys_and_inconsistent_usage_are_protocol_errors() {
    let cases = [
        concat!(
            "data: {\"type\":\"finish\",\"type\":\"finish\",",
            "\"finishReason\":{\"unified\":\"stop\"}}\n\n"
        ),
        concat!(
            "data: {\"type\":\"finish\",",
            "\"finishReason\":{\"unified\":\"stop\",\"unified\":\"stop\"}}\n\n"
        ),
        concat!(
            "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-1\",",
            "\"toolName\":\"echo\",\"input\":\"{\\\"x\\\":1,\\\"x\\\":2}\"}\n\n"
        ),
        concat!(
            "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"},",
            "\"usage\":{\"inputTokens\":{\"total\":1}}}\n\n"
        ),
        concat!(
            "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"},",
            "\"usage\":{\"inputTokens\":{},\"outputTokens\":{\"total\":1}}}\n\n"
        ),
        concat!(
            "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"},",
            "\"usage\":{\"inputTokens\":{\"total\":1,\"cacheRead\":2},",
            "\"outputTokens\":{\"total\":1}}}\n\n"
        ),
    ];
    for case in cases {
        protocol_error(collect(bytes(case)));
    }
}

#[test]
fn eof_done_and_incomplete_records_without_finish_are_protocol_errors() {
    for case in [
        "",
        "data: [DONE]\n\n",
        "data: {\"type\":\"text-delta\",\"delta\":\"partial\"}\n\n",
        "data: {\"type\":\"text-delta\",\"delta\":\"unterminated",
    ] {
        protocol_error(collect(bytes(case)));
    }
}

#[test]
fn duplicate_tool_ids_and_conflicting_stream_identity_are_rejected() {
    let duplicate = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"same\",\"toolName\":\"a\",\"input\":{}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"same\",\"toolName\":\"b\",\"input\":{}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    protocol_error(collect(bytes(duplicate)));

    let conflict = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"call-1\",\"toolName\":\"a\"}\n\n",
        "data: {\"type\":\"tool-input-start\",\"id\":\"call-1\",\"toolName\":\"b\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    protocol_error(collect(bytes(conflict)));
}

#[test]
fn outer_scalar_tool_input_provider_execution_and_tool_results_are_rejected() {
    let scalar = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-1\",\"toolName\":\"read_file\",\"input\":1}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    );
    protocol_error(collect(bytes(scalar)));

    let executed = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-1\",\"toolName\":\"web_search\",\"input\":{},\"providerExecuted\":true}\n\n",
        "data: {\"type\":\"tool-result\",\"toolCallId\":\"call-1\",\"result\":{\"secret\":true}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    );
    protocol_error(collect(bytes(executed)));
}

#[test]
fn transport_startup_and_stream_errors_preserve_structured_errors() {
    let expected = ProviderError::new(
        ProviderErrorKind::Transport,
        "fixture_transport",
        "transport failed",
        true,
    );
    let transport = ScriptedTransport::new([TransportStep::Error(expected.clone())]);
    let provider = provider(&transport);
    let error = expect_start_error(
        start(
            &provider,
            request(vec![Message::text(Role::User, "hello")]),
            CancellationToken::new(),
        ),
        "transport startup error",
    );
    assert_eq!(error, expected);

    let actual = collect(TransportStep::Bytes(vec![ByteStep::Error(
        expected.clone(),
    )]))
    .unwrap_err();
    assert_eq!(actual, expected);
}

#[test]
fn cancellation_wins_same_poll_startup_stream_error_chunk_and_eof_races() {
    let transport_error = ProviderError::new(
        ProviderErrorKind::Transport,
        "fixture_race",
        "fixture transport race",
        true,
    );
    let transport =
        ScriptedTransport::new([TransportStep::CancelThenError(transport_error.clone())]);
    let provider = provider(&transport);
    let startup_error = expect_start_error(
        start(
            &provider,
            request(vec![Message::text(Role::User, "hello")]),
            CancellationToken::new(),
        ),
        "same-poll startup cancellation",
    );
    assert_eq!(startup_error.kind, ProviderErrorKind::Cancelled);

    for step in [
        ByteStep::CancelThenError(transport_error),
        ByteStep::CancelThenChunk(finish("stop").into_bytes()),
        ByteStep::CancelThenEof,
    ] {
        let error = collect(TransportStep::Bytes(vec![step])).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    }
}

#[test]
fn empty_chunks_fail_and_no_event_chunks_yield_after_one_source_poll() {
    let error = protocol_error(collect(TransportStep::Bytes(vec![ByteStep::Chunk(
        Vec::new(),
    )])));
    assert_eq!(error.code, "gateway_empty_chunk");

    let transport = ScriptedTransport::new([TransportStep::Bytes(vec![
        ByteStep::Chunk(b": keepalive\n".to_vec()),
        ByteStep::Chunk(finish("stop").into_bytes()),
    ])]);
    let provider = provider(&transport);
    let mut events = start(
        &provider,
        request(vec![Message::text(Role::User, "hello")]),
        CancellationToken::new(),
    )
    .unwrap();
    assert!(events.next().now_or_never().is_none());
    let events = futures_executor::block_on(events.collect::<Vec<_>>())
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        events,
        [ModelEvent::Stop {
            reason: StopReason::Completed
        }]
    );
}

#[test]
fn ready_stream_outcomes_do_not_retain_or_spuriously_wake_pollers() {
    let cases = [
        (
            b"data: {\"type\":\"text-delta\",\"delta\":\"ready\"}\n\n".to_vec(),
            false,
        ),
        (finish("stop").into_bytes(), true),
        (Vec::new(), true),
    ];
    for (chunk, terminal) in cases {
        let cancellation = CancellationToken::new();
        let transport =
            ScriptedTransport::new([TransportStep::Bytes(vec![ByteStep::Chunk(chunk)])]);
        let provider = provider(&transport);
        let mut events = start(
            &provider,
            request(vec![Message::text(Role::User, "hello")]),
            cancellation.clone(),
        )
        .unwrap();
        let wake_counter = Arc::new(WakeCounter::default());
        let waker = Waker::from(Arc::clone(&wake_counter));
        let mut context = Context::from_waker(&waker);
        let result = events.as_mut().poll_next(&mut context);
        assert!(matches!(result, Poll::Ready(Some(_))));
        if terminal {
            assert!(matches!(
                events.as_mut().poll_next(&mut context),
                Poll::Ready(None)
            ));
        }
        assert_eq!(Arc::strong_count(&wake_counter), 2);
        cancellation.cancel();
        assert_eq!(wake_counter.0.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn cancellation_before_poll_has_no_transport_side_effect_and_drop_detaches_no_work() {
    let transport = ScriptedTransport::new([TransportStep::Pending]);
    let provider = provider(&transport);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let future = provider.stream(
        request(vec![Message::text(Role::User, "hello")]),
        cancellation,
    );
    drop(future);
    assert!(transport.requests().is_empty());
}

#[test]
fn cancelled_pending_transport_startup_resolves_as_cancelled() {
    let transport = ScriptedTransport::new([TransportStep::Pending]);
    let provider = provider(&transport);
    let cancellation = CancellationToken::new();
    let mut future = Box::pin(provider.stream(
        request(vec![Message::text(Role::User, "hello")]),
        cancellation.clone(),
    ));
    assert!(future.as_mut().now_or_never().is_none());
    assert_eq!(transport.requests().len(), 1);
    cancellation.cancel();
    let error = expect_start_error(
        futures_executor::block_on(future),
        "cancelled transport startup",
    );
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    assert!(!error.retryable);
}

#[test]
fn cancelled_pending_byte_stream_wakes_and_resolves_as_cancelled() {
    let transport = ScriptedTransport::new([TransportStep::Bytes(vec![ByteStep::Pending])]);
    let provider = provider(&transport);
    let cancellation = CancellationToken::new();
    let mut events = start(
        &provider,
        request(vec![Message::text(Role::User, "hello")]),
        cancellation.clone(),
    )
    .unwrap();
    let mut next = Box::pin(events.next());
    assert!(next.as_mut().now_or_never().is_none());
    cancellation.cancel();
    let error = futures_executor::block_on(next).unwrap().unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
}

fn deeply_nested_array(depth: usize) -> Value {
    let mut value = Value::Null;
    for _ in 0..depth {
        value = Value::Array(vec![value]);
    }
    value
}

#[test]
fn request_guard_iteratively_drops_deep_json_before_and_during_polling() {
    const CHILD_MODE: &str = "MACHINE_GOD_GATEWAY_DEEP_DROP_MODE";
    if let Ok(mode) = std::env::var(CHILD_MODE) {
        let transport = ScriptedTransport::new([]);
        let mut model_request = request(vec![Message::text(Role::User, "hello")]);
        model_request
            .options
            .metadata
            .insert("deep".to_owned(), deeply_nested_array(20_000));
        model_request.tools.push(ToolSpec {
            name: ToolName::new("deep_tool").unwrap(),
            description: "deep schema fixture".to_owned(),
            input_schema: deeply_nested_array(20_000),
        });
        let call_id = ToolCallId::new("deep-call").unwrap();
        model_request.messages.extend([
            Message {
                role: Role::User,
                content: vec![ContentBlock::Json {
                    value: deeply_nested_array(20_000),
                }],
            },
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolCall {
                    call: ToolCall {
                        id: call_id.clone(),
                        name: ToolName::new("deep_tool").unwrap(),
                        arguments: deeply_nested_array(20_000),
                    },
                }],
            },
            Message {
                role: Role::Tool,
                content: vec![ContentBlock::ToolResult {
                    call_id,
                    output: ToolOutput::success(deeply_nested_array(20_000)),
                }],
            },
        ]);
        if mode == "content_count" {
            model_request.messages[2].content.extend([
                ContentBlock::Text {
                    text: "excess-1".to_owned(),
                },
                ContentBlock::Text {
                    text: "excess-2".to_owned(),
                },
            ]);
        }
        let provider = if mode == "message_count" {
            AiGatewayProvider::with_limits(
                "provider/default",
                Arc::new(transport.clone()),
                AiGatewayLimits {
                    max_messages: 1,
                    ..AiGatewayLimits::default()
                },
            )
            .unwrap()
        } else if mode == "content_count" {
            AiGatewayProvider::with_limits(
                "provider/default",
                Arc::new(transport.clone()),
                AiGatewayLimits {
                    max_tool_calls: 1,
                    ..AiGatewayLimits::default()
                },
            )
            .unwrap()
        } else {
            provider(&transport)
        };
        let future = provider.stream(model_request, CancellationToken::new());
        if mode == "unpolled" {
            drop(future);
        } else {
            let error = expect_start_error(futures_executor::block_on(future), "deep request JSON");
            assert_eq!(error.kind, ProviderErrorKind::InvalidRequest);
            let expected = match mode.as_str() {
                "message_count" => "gateway_request_count_limit",
                "content_count" => "gateway_invalid_history",
                _ => "gateway_json_depth_limit",
            };
            assert_eq!(error.code, expected);
        }
        return;
    }

    for mode in ["unpolled", "polled", "message_count", "content_count"] {
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("request_guard_iteratively_drops_deep_json_before_and_during_polling")
            .env(CHILD_MODE, mode)
            .status()
            .unwrap();
        assert!(status.success(), "deep-drop child failed in {mode} mode");
    }
}

fn tight_limits() -> AiGatewayLimits {
    AiGatewayLimits {
        max_request_bytes: 512,
        max_chunk_bytes: 128,
        max_record_bytes: 96,
        max_undecoded_bytes: 96,
        max_total_response_bytes: 192,
        max_records: 2,
        max_messages: 1,
        max_tools: 1,
        max_streamed_tool_calls: 1,
        max_tool_calls: 1,
        max_tool_arguments_bytes: 16,
        max_json_nodes: 32,
    }
}

#[test]
fn default_limits_are_fixed_and_request_and_chunk_caps_are_enforced() {
    let default = AiGatewayLimits::default();
    assert_eq!(default.max_request_bytes, 12 * 1024 * 1024);
    assert_eq!(default.max_chunk_bytes, 1024 * 1024);
    assert_eq!(default.max_record_bytes, 1024 * 1024);
    assert_eq!(default.max_undecoded_bytes, 1024 * 1024);
    assert_eq!(default.max_total_response_bytes, 16 * 1024 * 1024);
    assert_eq!(default.max_records, 8192);
    assert_eq!(default.max_messages, 4096);
    assert_eq!(default.max_tools, 1024);
    assert_eq!(default.max_streamed_tool_calls, 64);
    assert_eq!(default.max_tool_calls, 64);
    assert_eq!(default.max_tool_arguments_bytes, 64 * 1024);
    assert_eq!(default.max_json_nodes, 262_144);

    let base = tight_limits();
    let oversized_chunk = "x".repeat(129);
    let transport = ScriptedTransport::new([bytes(oversized_chunk)]);
    let provider =
        AiGatewayProvider::with_limits("provider/model", Arc::new(transport), base).unwrap();
    let stream = start(
        &provider,
        request(vec![Message::text(Role::User, "hello")]),
        CancellationToken::new(),
    )
    .unwrap();
    let error = futures_executor::block_on(stream.collect::<Vec<_>>())
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Protocol);

    let transport = ScriptedTransport::new([]);
    let provider =
        AiGatewayProvider::with_limits("provider/model", Arc::new(transport.clone()), base)
            .unwrap();
    let error = expect_start_error(
        start(
            &provider,
            request(vec![Message::text(Role::User, "x".repeat(600))]),
            CancellationToken::new(),
        ),
        "oversized request",
    );
    assert_eq!(error.kind, ProviderErrorKind::InvalidRequest);
    assert!(transport.requests().is_empty());
}

#[test]
fn json_node_budget_is_independent_for_requests_records_and_string_arguments() {
    let mut limits = AiGatewayLimits {
        max_json_nodes: 4,
        ..AiGatewayLimits::default()
    };
    let transport = ScriptedTransport::new([]);
    let provider =
        AiGatewayProvider::with_limits("provider/model", Arc::new(transport.clone()), limits)
            .unwrap();
    let mut model_request = request(vec![Message::text(Role::User, "hello")]);
    model_request
        .options
        .metadata
        .insert("wide".to_owned(), json!([0, 1, 2, 3]));
    let error = expect_start_error(
        start(&provider, model_request, CancellationToken::new()),
        "wide request JSON",
    );
    assert_eq!(error.code, "gateway_json_node_limit");
    assert!(transport.requests().is_empty());

    let wide_record = concat!(
        "data: {\"type\":\"future\",\"wide\":[0,1,2]}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    );
    protocol_error(collect_with_limits(bytes(wide_record), limits));

    limits.max_json_nodes = 8;
    let nested_argument = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-1\",",
        "\"toolName\":\"echo\",\"input\":\"{\\\"wide\\\":[0,1,2,3,4,5,6,7]}\"}\n\n"
    );
    protocol_error(collect_with_limits(bytes(nested_argument), limits));
}

#[test]
fn tool_result_projection_uses_one_cumulative_request_budget() {
    let calls = ["call-1", "call-2"].map(|id| ToolCall {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new("echo").unwrap(),
        arguments: json!({}),
    });
    let model_request = request(vec![
        Message::text(Role::User, "run both"),
        Message {
            role: Role::Assistant,
            content: calls
                .iter()
                .cloned()
                .map(|call| ContentBlock::ToolCall { call })
                .collect(),
        },
        Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id: calls[0].id.clone(),
                output: ToolOutput::success("x".repeat(60)),
            }],
        },
        Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id: calls[1].id.clone(),
                output: ToolOutput::success("y".repeat(60)),
            }],
        },
    ]);
    let limits = AiGatewayLimits {
        max_request_bytes: 100,
        ..AiGatewayLimits::default()
    };
    let transport = ScriptedTransport::new([]);
    let provider =
        AiGatewayProvider::with_limits("provider/model", Arc::new(transport.clone()), limits)
            .unwrap();
    let error = expect_start_error(
        start(&provider, model_request, CancellationToken::new()),
        "cumulative projected tool results",
    );
    assert_eq!(error.code, "gateway_request_byte_limit");
    assert!(transport.requests().is_empty());
}

#[test]
fn response_record_buffer_total_and_count_limits_are_independent() {
    let mut limits = tight_limits();
    protocol_error(collect_with_limits(
        bytes(format!("data: {}\n\n", "x".repeat(91))),
        limits,
    ));

    limits.max_record_bytes = 128;
    limits.max_undecoded_bytes = 32;
    protocol_error(collect_with_limits(
        bytes(format!("data: {}\n\n", "x".repeat(40))),
        limits,
    ));

    limits.max_undecoded_bytes = 128;
    limits.max_total_response_bytes = 64;
    protocol_error(collect_with_limits(
        bytes(format!("data: {}\n\n", "x".repeat(60))),
        limits,
    ));

    limits.max_total_response_bytes = 512;
    limits.max_records = 1;
    protocol_error(collect_with_limits(
        bytes(concat!(
            "data: {\"type\":\"start\"}\n\n",
            "data: {\"type\":\"start-step\"}\n\n",
            "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
        )),
        limits,
    ));
}

#[test]
fn streamed_tool_call_completed_call_and_argument_limits_are_independent() {
    let mut limits = tight_limits();
    limits.max_total_response_bytes = 1024;
    let streamed = concat!(
        "data: {\"type\":\"tool-input-start\",\"id\":\"call-1\",\"toolName\":\"echo\"}\n\n",
        "data: {\"type\":\"tool-input-start\",\"id\":\"call-2\",\"toolName\":\"echo\"}\n\n"
    );
    protocol_error(collect_with_limits(bytes(streamed), limits));

    let complete = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-1\",\"toolName\":\"echo\",\"input\":{}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-2\",\"toolName\":\"echo\",\"input\":{}}\n\n"
    );
    protocol_error(collect_with_limits(bytes(complete), limits));

    let arguments = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"call-1\",\"toolName\":\"echo\",",
        "\"input\":{\"value\":\"123456789\"}}\n\n"
    );
    protocol_error(collect_with_limits(bytes(arguments), limits));
}

#[test]
fn request_message_tool_and_historical_argument_limits_skip_transport() {
    let limits = tight_limits();
    let cases = [
        (
            request(vec![
                Message::text(Role::User, "one"),
                Message::text(Role::User, "two"),
            ]),
            limits,
        ),
        (
            {
                let mut value = request(vec![Message::text(Role::User, "hello")]);
                for name in ["one", "two"] {
                    value.tools.push(ToolSpec {
                        name: ToolName::new(name).unwrap(),
                        description: "fixture".to_owned(),
                        input_schema: json!({"type":"object"}),
                    });
                }
                value
            },
            limits,
        ),
        (
            {
                let call = ToolCall {
                    id: ToolCallId::new("call-history").unwrap(),
                    name: ToolName::new("echo").unwrap(),
                    arguments: json!({"value":"123456789"}),
                };
                request(vec![
                    Message::text(Role::User, "use echo"),
                    Message {
                        role: Role::Assistant,
                        content: vec![ContentBlock::ToolCall { call: call.clone() }],
                    },
                    Message {
                        role: Role::Tool,
                        content: vec![ContentBlock::ToolResult {
                            call_id: call.id,
                            output: ToolOutput::success("done"),
                        }],
                    },
                ])
            },
            AiGatewayLimits {
                max_messages: 4,
                ..limits
            },
        ),
    ];
    for (case, case_limits) in cases {
        let transport = ScriptedTransport::new([]);
        let provider = AiGatewayProvider::with_limits(
            "provider/model",
            Arc::new(transport.clone()),
            case_limits,
        )
        .unwrap();
        let error = expect_start_error(
            start(&provider, case, CancellationToken::new()),
            "bounded request",
        );
        assert_eq!(error.kind, ProviderErrorKind::InvalidRequest);
        assert!(transport.requests().is_empty());
    }
}

#[test]
fn safe_unknown_events_and_late_framing_do_not_create_extra_output() {
    let body = concat!(
        "data: {\"type\":\"start\"}\n\n",
        "data: {\"type\":\"response-metadata\",\"modelId\":\"provider/resolved\"}\n\n",
        "data: {\"type\":\"source\",\"value\":{}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n",
        ": harmless comment\n",
        "event: ignored\n",
        "data: [DONE]\n\n"
    );
    assert_eq!(
        collect(bytes(body)).unwrap(),
        [ModelEvent::Stop {
            reason: StopReason::Completed
        }]
    );
}

#[test]
fn validated_finish_is_terminal_and_late_source_bytes_are_not_observed() {
    assert_eq!(
        collect(TransportStep::Bytes(vec![
            ByteStep::Chunk(finish("stop").into_bytes()),
            ByteStep::Chunk(b"data: {\"type\":\"text-delta\",\"delta\":\"late\"}\n\n".to_vec(),),
        ]))
        .unwrap(),
        [ModelEvent::Stop {
            reason: StopReason::Completed
        }]
    );
}
