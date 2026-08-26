#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Instant;

use futures_util::{Stream, future, stream};
use machine_god_core::{
    BoxFuture, CancellationToken, NetworkTarget, ProviderError, SessionId, SessionIncarnationId,
    Tool, ToolCallId, ToolContext, ToolErrorKind, TurnId,
};
use machine_god_native::{
    AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION, AI_GATEWAY_PROTOCOL_VERSION,
    AiGatewayByteStream, AiGatewayTransport, AiGatewayTransportRequest,
    AiGatewayWebSearchTransport, MAX_WEB_SEARCH_JSON_NODES, MAX_WEB_SEARCH_REQUEST_BYTES,
    MAX_WEB_SEARCH_RESPONSE_BYTES, MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES,
    MAX_WEB_SEARCH_RESPONSE_RECORDS, MAX_WEB_SEARCH_SOURCE_TITLE_BYTES,
    MAX_WEB_SEARCH_SOURCE_URL_BYTES, WebSearchDeadline, WebSearchTool, WebSearchTransportError,
};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MODEL: &str = "test/search-worker-model";
const PRIVATE_QUERY: &str = "latest Rust release PRIVATE_QUERY_SENTINEL";
const MAX_PROVIDER_TOOL_INPUT_JSON_NODES: usize = 256;
const MAX_PROVIDER_RESULT_ID_BYTES: usize = 512;

struct NeverDeadline;

impl WebSearchDeadline for NeverDeadline {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        Box::pin(future::pending())
    }
}

fn gateway_target() -> NetworkTarget {
    NetworkTarget {
        scheme: "https".to_owned(),
        host: "ai-gateway.vercel.sh".to_owned(),
        port: None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CapturedRequest {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[derive(Clone)]
struct ScriptedTransport {
    chunks: Arc<Vec<Vec<u8>>>,
    calls: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
}

impl ScriptedTransport {
    fn new(response: impl Into<Vec<u8>>) -> Self {
        let response = response.into();
        let split = response.len() / 2;
        Self {
            chunks: Arc::new(vec![response[..split].to_vec(), response[split..].to_vec()]),
            calls: Arc::new(AtomicUsize::new(0)),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl AiGatewayTransport for ScriptedTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (headers, body) = request.into_parts();
        self.requests.lock().unwrap().push(CapturedRequest {
            headers: headers
                .into_iter()
                .map(machine_god_native::AiGatewayHeader::into_parts)
                .collect(),
            body,
        });
        let chunks = self.chunks.as_ref().clone();
        Box::pin(async move {
            Ok(Box::pin(stream::iter(chunks.into_iter().map(Ok))) as AiGatewayByteStream)
        })
    }
}

#[derive(Clone)]
struct PendingAfterDoneTransport {
    response: Arc<Vec<u8>>,
    permits: Arc<Semaphore>,
    calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl PendingAfterDoneTransport {
    fn new(response: Vec<u8>) -> Self {
        Self {
            response: Arc::new(response),
            permits: Arc::new(Semaphore::new(1)),
            calls: Arc::new(AtomicUsize::new(0)),
            drops: Arc::new(AtomicUsize::new(0)),
        }
    }
}

struct PendingAfterDoneStream {
    chunk: Option<Vec<u8>>,
    _permit: OwnedSemaphorePermit,
    drops: Arc<AtomicUsize>,
}

impl Stream for PendingAfterDoneStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.chunk
            .take()
            .map_or(Poll::Pending, |chunk| Poll::Ready(Some(Ok(chunk))))
    }
}

impl Drop for PendingAfterDoneStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayTransport for PendingAfterDoneTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = self.response.as_ref().clone();
        let permit = Arc::clone(&self.permits).acquire_owned();
        let drops = Arc::clone(&self.drops);
        Box::pin(async move {
            let permit = permit.await.expect("test semaphore remains open");
            Ok(Box::pin(PendingAfterDoneStream {
                chunk: Some(response),
                _permit: permit,
                drops,
            }) as AiGatewayByteStream)
        })
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("web-search-codec-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("web-search-codec-incarnation").unwrap(),
        turn_id: TurnId::new("web-search-codec-turn").unwrap(),
        call_id: ToolCallId::new("web-search-codec-call").unwrap(),
    }
}

fn sse(events: &[Value]) -> Vec<u8> {
    let mut bytes = sse_records(events);
    bytes.extend_from_slice(b"data: [DONE]\n\n");
    bytes
}

fn sse_records(events: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend_from_slice(b"data: ");
        bytes.extend_from_slice(serde_json::to_string(event).unwrap().as_bytes());
        bytes.extend_from_slice(b"\n\n");
    }
    bytes
}

fn stream_start_event() -> Value {
    json!({
        "type": "stream-start",
        "warnings": []
    })
}

fn call_event(id: &str, name: &str, provider_executed: bool) -> Value {
    call_event_with_input(
        id,
        name,
        provider_executed,
        serde_json::to_string(&json!({ "query": PRIVATE_QUERY })).unwrap(),
    )
}

fn call_event_with_input(
    id: &str,
    name: &str,
    provider_executed: bool,
    input: impl Into<Value>,
) -> Value {
    json!({
        "type": "tool-call",
        "toolCallId": id,
        "toolName": name,
        "input": input.into(),
        "providerExecuted": provider_executed
    })
}

fn result_event(id: &str, result: impl Into<Value>) -> Value {
    let result = result.into();
    json!({
        "type": "tool-result",
        "toolCallId": id,
        "toolName": "perplexity_search",
        "result": result
    })
}

fn source(title: impl Into<String>, url: impl Into<String>) -> Value {
    json!({
        "title": title.into(),
        "url": url.into(),
        "snippet": "Bounded provider snippet",
        "date": "2026-08-26",
        "lastUpdated": "2026-08-27"
    })
}

fn success_result(id: impl Into<String>, results: impl IntoIterator<Item = Value>) -> Value {
    let results = results.into_iter().collect::<Vec<_>>();
    json!({
        "id": id.into(),
        "results": results
    })
}

fn finish_event(reason: &str) -> Value {
    json!({
        "type": "finish",
        "finishReason": { "unified": reason }
    })
}

fn valid_events_after_stream_start() -> Vec<Value> {
    vec![
        call_event("provider-search-1", "perplexity_search", true),
        result_event(
            "provider-search-1",
            success_result(
                "search-response-1",
                vec![source(
                    "Rust releases",
                    "https://www.rust-lang.org/tools/install",
                )],
            ),
        ),
        finish_event("stop"),
    ]
}

fn valid_events() -> Vec<Value> {
    let mut events = vec![stream_start_event()];
    events.extend(valid_events_after_stream_start());
    events
}

fn json_node_count(value: &Value) -> usize {
    let mut nodes = 0_usize;
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        nodes += 1;
        match value {
            Value::Array(values) => stack.extend(values),
            Value::Object(values) => stack.extend(values.values()),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    nodes
}

fn tool_input_with_len(length: usize) -> String {
    let empty = r#"{"query":""}"#;
    let input = format!(
        "{{\"query\":\"{}\"}}",
        "q".repeat(length.checked_sub(empty.len()).unwrap())
    );
    assert_eq!(input.len(), length);
    input
}

fn tool_input_with_nodes(nodes: usize) -> String {
    assert!(nodes >= 2);
    let input = serde_json::to_string(&json!({
        "values": vec![Value::Null; nodes - 2]
    }))
    .unwrap();
    let parsed: Value = serde_json::from_str(&input).unwrap();
    assert_eq!(json_node_count(&parsed), nodes);
    input
}

fn execute_with_arguments(
    response: Vec<u8>,
    arguments: Value,
) -> (
    ScriptedTransport,
    Result<machine_god_core::ToolOutput, machine_god_core::ToolError>,
) {
    let transport = ScriptedTransport::new(response);
    let adapter = AiGatewayWebSearchTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let tool =
        WebSearchTool::with_transport(gateway_target(), Arc::new(adapter), Arc::new(NeverDeadline))
            .unwrap();
    let output =
        futures_executor::block_on(tool.execute(context(), arguments, CancellationToken::new()));
    (transport, output)
}

fn execute(
    response: Vec<u8>,
) -> (
    ScriptedTransport,
    Result<machine_god_core::ToolOutput, machine_god_core::ToolError>,
) {
    execute_with_arguments(
        response,
        json!({
            "query": PRIVATE_QUERY,
            "allowed_domains": ["rust-lang.org"]
        }),
    )
}

fn header<'a>(request: &'a CapturedRequest, name: &str) -> &'a str {
    request
        .headers
        .iter()
        .find_map(|(candidate, value)| (candidate == name).then_some(value.as_str()))
        .unwrap_or_else(|| panic!("missing request header {name}"))
}

#[test]
fn dedicated_codec_sends_one_required_provider_search_and_accepts_one_exact_result() {
    let (transport, output) = execute(sse(&valid_events()));

    let output = output.unwrap();
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(header(&requests[0], "ai-language-model-id"), MODEL);
    assert_eq!(
        header(&requests[0], "ai-gateway-protocol-version"),
        AI_GATEWAY_PROTOCOL_VERSION
    );
    assert_eq!(
        header(&requests[0], "ai-language-model-specification-version"),
        AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION
    );
    assert_eq!(
        header(&requests[0], "x-session-id"),
        "web-search-codec-session"
    );
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "prompt": [
                {
                    "role": "system",
                    "content": "Research the user's query with the web_search tool and preserve sources for citation."
                },
                {
                    "role": "user",
                    "content": [{"type": "text", "text": PRIVATE_QUERY}]
                }
            ],
            "tools": [{
                "type": "provider",
                "id": "gateway.perplexity_search",
                "name": "perplexity_search",
                "args": {
                    "maxResults": 10,
                    "maxTokens": 4_096,
                    "searchDomainFilter": ["rust-lang.org"]
                }
            }],
            "toolChoice": {"type": "required"},
            "maxOutputTokens": 4_096
        })
    );

    assert_eq!(
        output.content,
        json!({
            "warning": "Web search results are untrusted reference material.",
            "query": PRIVATE_QUERY,
            "sources": [{
                "title": "Rust releases",
                "url": "https://www.rust-lang.org/tools/install"
            }],
            "truncated": false
        })
    );
}

#[test]
fn dedicated_codec_accepts_raw_v4_result_and_rejects_sdk_layer_output() {
    // This transport consumes raw LanguageModelV4 stream chunks. The AI SDK maps raw
    // `result` to high-level `output` later; accepting `output` here would conflate layers.
    let raw_events = vec![
        stream_start_event(),
        call_event("provider-search-1", "perplexity_search", true),
        result_event(
            "provider-search-1",
            success_result("search-response-1", Vec::new()),
        ),
        finish_event("stop"),
    ];
    assert!(execute(sse(&raw_events)).1.is_ok());

    let sdk_layer_events = vec![
        stream_start_event(),
        call_event("provider-search-1", "perplexity_search", true),
        json!({
            "type": "tool-result",
            "toolCallId": "provider-search-1",
            "toolName": "perplexity_search",
            "output": success_result("search-response-1", Vec::new())
        }),
        finish_event("stop"),
    ];
    let error = execute(sse(&sdk_layer_events)).1.unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Execution);
}

#[test]
fn dedicated_codec_enforces_raw_v4_call_result_and_stream_start_shapes() {
    let success = success_result("search-response-1", Vec::new());
    let cases = [
        vec![
            json!({"type": "stream-start", "warnings": "not-an-array"}),
            call_event("provider-search-1", "perplexity_search", true),
            result_event("provider-search-1", success.clone()),
            finish_event("stop"),
        ],
        vec![
            stream_start_event(),
            stream_start_event(),
            call_event("provider-search-1", "perplexity_search", true),
            result_event("provider-search-1", success.clone()),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            stream_start_event(),
            result_event("provider-search-1", success.clone()),
            finish_event("stop"),
        ],
        vec![
            call_event_with_input("provider-search-1", "perplexity_search", true, json!({})),
            result_event("provider-search-1", success.clone()),
            finish_event("stop"),
        ],
        vec![
            call_event_with_input("provider-search-1", "perplexity_search", true, "not JSON"),
            result_event("provider-search-1", success.clone()),
            finish_event("stop"),
        ],
        vec![
            call_event_with_input("provider-search-1", "perplexity_search", true, "[]"),
            result_event("provider-search-1", success.clone()),
            finish_event("stop"),
        ],
        vec![
            call_event_with_input(
                "provider-search-1",
                "perplexity_search",
                true,
                r#"{"query":"first","query":"second"}"#,
            ),
            result_event("provider-search-1", success.clone()),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            json!({
                "type": "tool-result",
                "toolCallId": "provider-search-1",
                "result": success.clone()
            }),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            json!({
                "type": "tool-result",
                "toolCallId": "provider-search-1",
                "toolName": "parallel_search",
                "result": success.clone()
            }),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            json!({
                "type": "tool-result",
                "toolCallId": "provider-search-1",
                "toolName": "perplexity_search",
                "result": success.clone(),
                "isError": true
            }),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            json!({
                "type": "tool-result",
                "toolCallId": "provider-search-1",
                "toolName": "perplexity_search",
                "result": success.clone(),
                "isError": "false"
            }),
            finish_event("stop"),
        ],
    ];

    for events in cases {
        let error = execute(sse(&events)).1.unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::Execution);
    }

    assert!(
        execute(sse(&explicitly_non_error_events(&success)))
            .1
            .is_ok()
    );
}

fn explicitly_non_error_events(success: &Value) -> Vec<Value> {
    vec![
        stream_start_event(),
        call_event("provider-search-1", "perplexity_search", true),
        json!({
            "type": "tool-result",
            "toolCallId": "provider-search-1",
            "toolName": "perplexity_search",
            "result": success,
            "preliminary": false,
            "isError": false
        }),
        finish_event("stop"),
    ]
}

#[test]
fn dedicated_codec_accepts_exact_call_input_and_result_id_bounds() {
    for (input_length, id_length, accepted) in [
        (
            MAX_WEB_SEARCH_REQUEST_BYTES,
            MAX_PROVIDER_RESULT_ID_BYTES,
            true,
        ),
        (
            MAX_WEB_SEARCH_REQUEST_BYTES + 1,
            MAX_PROVIDER_RESULT_ID_BYTES,
            false,
        ),
        (
            MAX_WEB_SEARCH_REQUEST_BYTES,
            MAX_PROVIDER_RESULT_ID_BYTES + 1,
            false,
        ),
    ] {
        let events = vec![
            stream_start_event(),
            call_event_with_input(
                "provider-search-1",
                "perplexity_search",
                true,
                tool_input_with_len(input_length),
            ),
            result_event(
                "provider-search-1",
                success_result("i".repeat(id_length), Vec::new()),
            ),
            finish_event("stop"),
        ];
        assert_eq!(
            execute(sse(&events)).1.is_ok(),
            accepted,
            "input length {input_length}, id length {id_length}"
        );
    }

    for (nodes, accepted) in [
        (MAX_PROVIDER_TOOL_INPUT_JSON_NODES, true),
        (MAX_PROVIDER_TOOL_INPUT_JSON_NODES + 1, false),
    ] {
        let events = vec![
            stream_start_event(),
            call_event_with_input(
                "provider-search-1",
                "perplexity_search",
                true,
                tool_input_with_nodes(nodes),
            ),
            result_event(
                "provider-search-1",
                success_result("search-response-1", Vec::new()),
            ),
            finish_event("stop"),
        ];
        assert_eq!(
            execute(sse(&events)).1.is_ok(),
            accepted,
            "parsed input nodes {nodes}"
        );
    }
}

#[test]
fn gateway_worker_body_is_exact_for_no_filter_allow_filter_and_block_filter() {
    let cases = [
        (json!({"query": PRIVATE_QUERY}), None),
        (
            json!({
                "query": PRIVATE_QUERY,
                "allowed_domains": ["rust-lang.org", "docs.rs"]
            }),
            Some(json!(["rust-lang.org", "docs.rs"])),
        ),
        (
            json!({
                "query": PRIVATE_QUERY,
                "blocked_domains": ["example.com", "example.org"]
            }),
            Some(json!(["-example.com", "-example.org"])),
        ),
    ];

    for (arguments, expected_filter) in cases {
        let (transport, output) = execute_with_arguments(sse(&valid_events()), arguments);
        output.unwrap();
        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        let mut provider_args = json!({
            "maxResults": 10,
            "maxTokens": 4_096
        });
        if let Some(filter) = expected_filter {
            provider_args["searchDomainFilter"] = filter;
        }
        assert_eq!(
            body,
            json!({
                "prompt": [
                    {
                        "role": "system",
                        "content": "Research the user's query with the web_search tool and preserve sources for citation."
                    },
                    {
                        "role": "user",
                        "content": [{"type": "text", "text": PRIVATE_QUERY}]
                    }
                ],
                "tools": [{
                    "type": "provider",
                    "id": "gateway.perplexity_search",
                    "name": "perplexity_search",
                    "args": provider_args
                }],
                "toolChoice": {"type": "required"},
                "maxOutputTokens": 4_096
            })
        );
    }
}

#[test]
fn dedicated_codec_rejects_ambiguous_or_non_authoritative_provider_records() {
    let cases = [
        vec![
            call_event("provider-search-1", "perplexity_search", false),
            result_event(
                "provider-search-1",
                success_result("search-response-1", Vec::new()),
            ),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "parallel_search", true),
            result_event(
                "provider-search-1",
                success_result("search-response-1", Vec::new()),
            ),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            result_event(
                "provider-search-2",
                success_result("search-response-1", Vec::new()),
            ),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            call_event("provider-search-2", "perplexity_search", true),
            result_event(
                "provider-search-2",
                success_result("search-response-1", Vec::new()),
            ),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            result_event(
                "provider-search-1",
                success_result("search-response-1", Vec::new()),
            ),
            result_event(
                "provider-search-1",
                success_result("search-response-1", Vec::new()),
            ),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            json!({
                "type": "tool-result",
                "toolCallId": "provider-search-1",
                "toolName": "perplexity_search",
                "result": success_result("search-response-1", Vec::new()),
                "preliminary": true
            }),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            result_event(
                "provider-search-1",
                success_result("search-response-1", Vec::new()),
            ),
            finish_event("tool-calls"),
        ],
    ];

    for events in cases {
        let (transport, error) = execute(sse(&events));
        let error = error.unwrap_err();
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(error.kind, ToolErrorKind::Execution);
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains(PRIVATE_QUERY));
        assert!(!rendered.contains("PRIVATE_QUERY_SENTINEL"));
    }
}

#[test]
fn dedicated_codec_rejects_malformed_or_unsafe_results_without_reflection() {
    for result in [
        Value::Null,
        json!({}),
        json!({ "results": [] }),
        json!({ "id": "search-response-1", "results": "not-an-array" }),
        json!({ "id": "", "results": [] }),
        json!({ "id": "private\nidentifier", "results": [] }),
        json!({
            "id": "search-response-1",
            "results": [],
            "unknown": true
        }),
        success_result(
            "search-response-1",
            vec![json!({
                "title": "private",
                "url": "https://example.com"
            })],
        ),
        success_result(
            "search-response-1",
            vec![json!({
                "title": "private",
                "url": "https://example.com",
                "snippet": 7
            })],
        ),
        success_result(
            "search-response-1",
            vec![json!({
                "title": "private",
                "url": "https://example.com",
                "snippet": "private",
                "date": false
            })],
        ),
        success_result(
            "search-response-1",
            vec![json!({
                "title": "private",
                "url": "https://example.com",
                "snippet": "private",
                "lastUpdated": 7
            })],
        ),
        success_result(
            "search-response-1",
            vec![json!({
                "title": "private",
                "url": "https://example.com",
                "snippet": "private",
                "unknown": "private"
            })],
        ),
        success_result(
            "search-response-1",
            vec![source("private", "file:///private")],
        ),
        success_result(
            "search-response-1",
            vec![source("private", "https://127.0.0.1/private")],
        ),
        success_result(
            "search-response-1",
            vec![source("private", "https://example.com:+443/private")],
        ),
        success_result(
            "search-response-1",
            vec![source("private\nheading", "https://example.com")],
        ),
        json!({
            "error": "api_error",
            "message": "private provider failure"
        }),
    ] {
        let events = vec![
            call_event("provider-search-1", "perplexity_search", true),
            result_event("provider-search-1", result),
            finish_event("stop"),
        ];
        let (_, error) = execute(sse(&events));
        let error = error.unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::Execution);
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("private"));
        assert!(!rendered.contains("127.0.0.1"));
    }
}

#[test]
fn dedicated_codec_validates_but_does_not_project_official_auxiliary_fields() {
    let events = vec![
        stream_start_event(),
        call_event("provider-search-1", "perplexity_search", true),
        result_event(
            "provider-search-1",
            success_result(
                "search-response-1",
                vec![
                    json!({
                        "title": "Required fields only",
                        "url": "https://docs.rs/serde/latest/serde/",
                        "snippet": "No optional metadata"
                    }),
                    json!({
                        "title": "Rust releases",
                        "url": "https://www.rust-lang.org/tools/install",
                        "snippet": "s".repeat(20_000),
                        "date": "d".repeat(4_000),
                        "lastUpdated": "u".repeat(4_000)
                    }),
                ],
            ),
        ),
        finish_event("stop"),
    ];
    let output = execute(sse(&events)).1.unwrap();
    assert_eq!(output.content["sources"].as_array().unwrap().len(), 2);
    assert_eq!(
        output.content["sources"][0]["title"],
        "Required fields only"
    );
    assert_eq!(output.content["sources"][1]["title"], "Rust releases");
    for source in output.content["sources"].as_array().unwrap() {
        assert!(source.get("snippet").is_none());
        assert!(source.get("date").is_none());
        assert!(source.get("lastUpdated").is_none());
    }
}

#[test]
fn dedicated_codec_accepts_crlf_and_a_terminal_record_without_a_blank_line() {
    let crlf = String::from_utf8(sse(&valid_events()))
        .unwrap()
        .replace('\n', "\r\n")
        .into_bytes();
    assert!(execute(crlf).1.is_ok());

    let mut no_done = sse(&valid_events());
    let done = b"data: [DONE]\n\n";
    let done_start = no_done.len() - done.len();
    assert_eq!(&no_done[done_start..], done);
    no_done.truncate(done_start);
    assert_eq!(no_done.pop(), Some(b'\n'));
    assert!(execute(no_done).1.is_ok());
}

#[test]
fn dedicated_codec_stops_at_done_drops_capacity_and_allows_the_next_request() {
    let transport = PendingAfterDoneTransport::new(sse(&valid_events()));
    let adapter = AiGatewayWebSearchTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let tool =
        WebSearchTool::with_transport(gateway_target(), Arc::new(adapter), Arc::new(NeverDeadline))
            .unwrap();

    for completed in 1..=2 {
        let mut execution = tool.execute(
            context(),
            json!({"query": PRIVATE_QUERY}),
            CancellationToken::new(),
        );
        let Poll::Ready(output) = poll_once(execution.as_mut()) else {
            panic!("execution {completed} polled the transport again after [DONE]");
        };
        output.unwrap();
        assert_eq!(transport.calls.load(Ordering::SeqCst), completed);
        assert_eq!(transport.drops.load(Ordering::SeqCst), completed);
        assert_eq!(transport.permits.available_permits(), 1);
    }
}

#[test]
fn dedicated_codec_rejects_records_and_partial_bytes_trailing_done_in_one_chunk() {
    let complete = b"data: {\"type\":\"response-metadata\"}\n\n";
    let partial = b"data: {\"type\":\"response-metadata\"";
    for trailing in [complete.as_slice(), partial.as_slice()] {
        let mut response = sse(&valid_events());
        response.extend_from_slice(trailing);
        let error = execute(response).1.unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::Execution);
    }
}

#[test]
fn dedicated_codec_deduplicates_in_order_and_marks_only_unique_overflow_truncated() {
    let mut results = vec![source("first", "https://source0.example.org/path")];
    results.push(source(
        "duplicate title is ignored",
        "https://source0.example.org/path",
    ));
    for index in 1..=10 {
        results.push(source(
            format!("source {index}"),
            format!("https://source{index}.example.org/path"),
        ));
    }
    let events = vec![
        call_event("provider-search-1", "perplexity_search", true),
        result_event(
            "provider-search-1",
            success_result("search-response-1", results),
        ),
        finish_event("stop"),
    ];
    let output = execute(sse(&events)).1.unwrap();
    assert_eq!(output.content["sources"].as_array().unwrap().len(), 10);
    assert_eq!(output.content["sources"][0]["title"], "first");
    assert_eq!(output.content["truncated"], true);
}

#[test]
fn dedicated_codec_enforces_stream_record_count_and_strict_json_bounds() {
    let oversized = vec![b'x'; MAX_WEB_SEARCH_RESPONSE_BYTES + 1];
    assert!(execute(oversized).1.is_err());

    let oversized_record = format!(
        "data: {{\"type\":\"response-metadata\",\"padding\":\"{}\"}}\n\n",
        "x".repeat(MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES)
    )
    .into_bytes();
    assert!(execute(oversized_record).1.is_err());

    let mut too_many_records = Vec::new();
    for _ in 0..=MAX_WEB_SEARCH_RESPONSE_RECORDS {
        too_many_records.extend_from_slice(b"data: {\"type\":\"response-metadata\"}\n\n");
    }
    assert!(execute(too_many_records).1.is_err());

    let duplicate_key = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"provider-search-1\",",
        "\"toolName\":\"perplexity_search\",\"input\":\"{}\",",
        "\"providerExecuted\":true,\"providerExecuted\":true}\n\n"
    );
    assert!(execute(duplicate_key.as_bytes().to_vec()).1.is_err());
}

fn metadata_record_with_wire_len(wire_len: usize) -> Vec<u8> {
    let empty = b"data: {\"padding\":\"\",\"type\":\"response-metadata\"}\n\n";
    assert!(wire_len >= empty.len());
    let mut record = b"data: {\"padding\":\"".to_vec();
    record.extend(std::iter::repeat_n(b'x', wire_len - empty.len()));
    record.extend_from_slice(b"\",\"type\":\"response-metadata\"}\n\n");
    assert_eq!(record.len(), wire_len);
    record
}

fn response_with_wire_len(wire_len: usize) -> Vec<u8> {
    let start = sse_records(&[stream_start_event()]);
    let tail = sse(&valid_events_after_stream_start());
    let mut response = Vec::with_capacity(wire_len);
    response.extend(start);
    while wire_len - response.len() - tail.len() > MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES + 2 {
        response.extend(metadata_record_with_wire_len(
            MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES + 2,
        ));
    }
    let remaining = wire_len - response.len() - tail.len();
    if remaining > 0 {
        response.extend(metadata_record_with_wire_len(remaining));
    }
    response.extend(tail);
    assert_eq!(response.len(), wire_len);
    response
}

#[test]
fn dedicated_codec_accepts_exact_stream_record_count_and_json_node_limits() {
    assert!(
        execute(response_with_wire_len(MAX_WEB_SEARCH_RESPONSE_BYTES))
            .1
            .is_ok()
    );
    assert!(
        execute(response_with_wire_len(MAX_WEB_SEARCH_RESPONSE_BYTES + 1))
            .1
            .is_err()
    );

    let exact_record = metadata_record_with_wire_len(MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES + 2);
    let mut exact_record_response = sse_records(&[stream_start_event()]);
    exact_record_response.extend(exact_record);
    exact_record_response.extend(sse(&valid_events_after_stream_start()));
    assert!(execute(exact_record_response).1.is_ok());
    let oversized_record = metadata_record_with_wire_len(MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES + 3);
    let mut oversized_record_response = sse_records(&[stream_start_event()]);
    oversized_record_response.extend(oversized_record);
    oversized_record_response.extend(sse(&valid_events_after_stream_start()));
    assert!(execute(oversized_record_response).1.is_err());

    let start = sse_records(&[stream_start_event()]);
    let mut exact_records = start.clone();
    let tail_records = valid_events().len() + 1;
    for _ in 0..(MAX_WEB_SEARCH_RESPONSE_RECORDS - tail_records) {
        exact_records.extend_from_slice(b"data: {\"type\":\"response-metadata\"}\n\n");
    }
    exact_records.extend(sse(&valid_events_after_stream_start()));
    assert!(execute(exact_records.clone()).1.is_ok());
    exact_records.splice(
        start.len()..start.len(),
        b"data: {\"type\":\"response-metadata\"}\n\n"
            .iter()
            .copied(),
    );
    assert!(execute(exact_records).1.is_err());

    let required_nodes = valid_events().iter().map(json_node_count).sum::<usize>();
    // The metadata object, type string, and padding array consume three nodes.
    let exact_padding = vec![json!(0); MAX_WEB_SEARCH_JSON_NODES - required_nodes - 3];
    let mut exact_nodes = vec![
        stream_start_event(),
        json!({
            "type": "response-metadata",
            "padding": exact_padding
        }),
    ];
    exact_nodes.extend(valid_events_after_stream_start());
    assert!(execute(sse(&exact_nodes)).1.is_ok());

    let oversized_padding = vec![json!(0); MAX_WEB_SEARCH_JSON_NODES - required_nodes - 2];
    let mut oversized_nodes = vec![
        stream_start_event(),
        json!({
            "type": "response-metadata",
            "padding": oversized_padding
        }),
    ];
    oversized_nodes.extend(valid_events_after_stream_start());
    assert!(execute(sse(&oversized_nodes)).1.is_err());
}

#[test]
fn dedicated_codec_accepts_exact_source_field_limits_and_rejects_one_byte_over() {
    let url_prefix = "https://source.example.org/";
    let exact_url = format!(
        "{url_prefix}{}",
        "u".repeat(MAX_WEB_SEARCH_SOURCE_URL_BYTES - url_prefix.len())
    );
    let exact_result = success_result(
        "search-response-1",
        vec![source(
            "t".repeat(MAX_WEB_SEARCH_SOURCE_TITLE_BYTES),
            exact_url,
        )],
    );
    let exact_events = vec![
        call_event("provider-search-1", "perplexity_search", true),
        result_event("provider-search-1", exact_result),
        finish_event("stop"),
    ];
    assert!(execute(sse(&exact_events)).1.is_ok());

    for result in [
        success_result(
            "search-response-1",
            vec![source(
                "t".repeat(MAX_WEB_SEARCH_SOURCE_TITLE_BYTES + 1),
                "https://source.example.org/",
            )],
        ),
        success_result(
            "search-response-1",
            vec![source(
                "title",
                format!(
                    "{url_prefix}{}",
                    "u".repeat(MAX_WEB_SEARCH_SOURCE_URL_BYTES + 1 - url_prefix.len())
                ),
            )],
        ),
    ] {
        let events = vec![
            call_event("provider-search-1", "perplexity_search", true),
            result_event("provider-search-1", result),
            finish_event("stop"),
        ];
        assert!(execute(sse(&events)).1.is_err());
    }
}
