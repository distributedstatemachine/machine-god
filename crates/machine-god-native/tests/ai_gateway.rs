use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

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
    Pending,
}

#[derive(Clone, Debug)]
enum TransportStep {
    Bytes(Vec<ByteStep>),
    Error(ProviderError),
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
        _cancellation: CancellationToken,
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
                }) as AiGatewayByteStream),
                Some(TransportStep::Error(error)) => Err(error),
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
}

impl Stream for ScriptedByteStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.steps.front() {
            Some(ByteStep::Pending) => Poll::Pending,
            Some(_) => match self.steps.pop_front().unwrap() {
                ByteStep::Chunk(bytes) => Poll::Ready(Some(Ok(bytes))),
                ByteStep::Error(error) => Poll::Ready(Some(Err(error))),
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
fn request_rejects_unsupported_options_and_content_without_transport() {
    let cases = [
        {
            let mut value = request(vec![Message::text(Role::User, "hello")]);
            value.options.temperature = Some(0.5);
            value
        },
        {
            let mut value = request(vec![Message::text(Role::User, "hello")]);
            value
                .options
                .metadata
                .insert("secret".to_owned(), json!("value"));
            value
        },
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
fn json_data_event_after_finish_is_rejected() {
    let body = concat!(
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n",
        "data: {\"type\":\"text-delta\",\"delta\":\"late\"}\n\n"
    );
    protocol_error(collect(bytes(body)));
}
