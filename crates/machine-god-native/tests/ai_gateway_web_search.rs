#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::stream;
use machine_god_core::{
    BoxFuture, CancellationToken, NetworkTarget, ProviderError, SessionId, SessionIncarnationId,
    Tool, ToolCallId, ToolContext, ToolErrorKind, TurnId,
};
use machine_god_native::{
    AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION, AI_GATEWAY_PROTOCOL_VERSION,
    AiGatewayByteStream, AiGatewayTransport, AiGatewayTransportRequest,
    AiGatewayWebSearchTransport, WebSearchTool,
};
use serde_json::{Value, json};

const MODEL: &str = "test/search-worker-model";
const PRIVATE_QUERY: &str = "latest Rust release PRIVATE_QUERY_SENTINEL";

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

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("web-search-codec-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("web-search-codec-incarnation").unwrap(),
        turn_id: TurnId::new("web-search-codec-turn").unwrap(),
        call_id: ToolCallId::new("web-search-codec-call").unwrap(),
    }
}

fn sse(events: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend_from_slice(b"data: ");
        bytes.extend_from_slice(serde_json::to_string(event).unwrap().as_bytes());
        bytes.extend_from_slice(b"\n\n");
    }
    bytes.extend_from_slice(b"data: [DONE]\n\n");
    bytes
}

fn call_event(id: &str, name: &str, provider_executed: bool) -> Value {
    json!({
        "type": "tool-call",
        "toolCallId": id,
        "toolName": name,
        "input": {},
        "providerExecuted": provider_executed
    })
}

fn result_event(id: &str, result: Value) -> Value {
    json!({
        "type": "tool-result",
        "toolCallId": id,
        "result": result
    })
}

fn finish_event(reason: &str) -> Value {
    json!({
        "type": "finish",
        "finishReason": { "unified": reason }
    })
}

fn valid_events() -> Vec<Value> {
    vec![
        call_event("provider-search-1", "perplexity_search", true),
        result_event(
            "provider-search-1",
            json!({
                "results": [{
                    "title": "Rust releases",
                    "url": "https://www.rust-lang.org/tools/install"
                }]
            }),
        ),
        finish_event("stop"),
    ]
}

fn execute(
    response: Vec<u8>,
) -> (
    ScriptedTransport,
    Result<machine_god_core::ToolOutput, machine_god_core::ToolError>,
) {
    let transport = ScriptedTransport::new(response);
    let adapter = AiGatewayWebSearchTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let tool = WebSearchTool::with_transport(gateway_target(), Arc::new(adapter)).unwrap();
    let output = futures_executor::block_on(tool.execute(
        context(),
        json!({
            "query": PRIVATE_QUERY,
            "allowed_domains": ["rust-lang.org"]
        }),
        CancellationToken::new(),
    ));
    (transport, output)
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
    let serialized = body.to_string();
    assert!(serialized.contains(PRIVATE_QUERY));
    assert!(serialized.contains("rust-lang.org"));
    assert!(serialized.contains("perplexity_search"));
    assert!(!serialized.contains("parallel_search"));
    assert_eq!(body["maxOutputTokens"], 4_096);

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
fn dedicated_codec_rejects_ambiguous_or_non_authoritative_provider_records() {
    let cases = [
        vec![
            call_event("provider-search-1", "perplexity_search", false),
            result_event("provider-search-1", json!({ "results": [] })),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "parallel_search", true),
            result_event("provider-search-1", json!({ "results": [] })),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            result_event("provider-search-2", json!({ "results": [] })),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            call_event("provider-search-2", "perplexity_search", true),
            result_event("provider-search-2", json!({ "results": [] })),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            result_event("provider-search-1", json!({ "results": [] })),
            result_event("provider-search-1", json!({ "results": [] })),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            json!({
                "type": "tool-result",
                "toolCallId": "provider-search-1",
                "result": { "results": [] },
                "preliminary": true
            }),
            finish_event("stop"),
        ],
        vec![
            call_event("provider-search-1", "perplexity_search", true),
            result_event("provider-search-1", json!({ "results": [] })),
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
        json!({ "results": "not-an-array" }),
        json!({ "results": [{ "url": "https://example.com" }] }),
        json!({ "results": [{ "title": "private", "url": "file:///private" }] }),
        json!({ "results": [{ "title": "private", "url": "https://127.0.0.1/private" }] }),
        json!({ "results": [{ "title": "private\nheading", "url": "https://example.com" }] }),
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
