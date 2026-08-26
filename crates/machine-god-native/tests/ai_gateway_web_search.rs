#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::{future, stream};
use machine_god_core::{
    BoxFuture, CancellationToken, NetworkTarget, ProviderError, SessionId, SessionIncarnationId,
    Tool, ToolCallId, ToolContext, ToolErrorKind, TurnId,
};
use machine_god_native::{
    AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION, AI_GATEWAY_PROTOCOL_VERSION,
    AiGatewayByteStream, AiGatewayTransport, AiGatewayTransportRequest,
    AiGatewayWebSearchTransport, MAX_WEB_SEARCH_JSON_NODES, MAX_WEB_SEARCH_RESPONSE_BYTES,
    MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES, MAX_WEB_SEARCH_RESPONSE_RECORDS,
    MAX_WEB_SEARCH_SOURCE_TITLE_BYTES, MAX_WEB_SEARCH_SOURCE_URL_BYTES, WebSearchDeadline,
    WebSearchTool, WebSearchTransportError,
};
use serde_json::{Value, json};

const MODEL: &str = "test/search-worker-model";
const PRIVATE_QUERY: &str = "latest Rust release PRIVATE_QUERY_SENTINEL";

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

fn result_event(id: &str, result: impl Into<Value>) -> Value {
    let result = result.into();
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
fn dedicated_codec_deduplicates_in_order_and_marks_only_unique_overflow_truncated() {
    let mut results = vec![json!({
        "title": "first",
        "url": "https://source0.example.org/path"
    })];
    results.push(json!({
        "title": "duplicate title is ignored",
        "url": "https://source0.example.org/path"
    }));
    for index in 1..=10 {
        results.push(json!({
            "title": format!("source {index}"),
            "url": format!("https://source{index}.example.org/path")
        }));
    }
    let events = vec![
        call_event("provider-search-1", "perplexity_search", true),
        result_event("provider-search-1", json!({ "results": results })),
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
        "\"toolName\":\"perplexity_search\",\"input\":{},",
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
    let tail = sse(&valid_events());
    let mut response = Vec::with_capacity(wire_len);
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
    let mut exact_record_response = exact_record;
    exact_record_response.extend(sse(&valid_events()));
    assert!(execute(exact_record_response).1.is_ok());
    let oversized_record = metadata_record_with_wire_len(MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES + 3);
    let mut oversized_record_response = oversized_record;
    oversized_record_response.extend(sse(&valid_events()));
    assert!(execute(oversized_record_response).1.is_err());

    let mut exact_records = Vec::new();
    for _ in 0..(MAX_WEB_SEARCH_RESPONSE_RECORDS - 4) {
        exact_records.extend_from_slice(b"data: {\"type\":\"response-metadata\"}\n\n");
    }
    exact_records.extend(sse(&valid_events()));
    assert!(execute(exact_records.clone()).1.is_ok());
    exact_records.splice(
        0..0,
        b"data: {\"type\":\"response-metadata\"}\n\n"
            .iter()
            .copied(),
    );
    assert!(execute(exact_records).1.is_err());

    // Required call/result-with-one-source/finish records consume eighteen nodes. Metadata's
    // object, type string, padding array, and elements consume the remainder.
    let exact_padding = vec![json!(0); MAX_WEB_SEARCH_JSON_NODES - 21];
    let mut exact_nodes = vec![json!({
        "type": "response-metadata",
        "padding": exact_padding
    })];
    exact_nodes.extend(valid_events());
    assert!(execute(sse(&exact_nodes)).1.is_ok());

    let oversized_padding = vec![json!(0); MAX_WEB_SEARCH_JSON_NODES - 20];
    let mut oversized_nodes = vec![json!({
        "type": "response-metadata",
        "padding": oversized_padding
    })];
    oversized_nodes.extend(valid_events());
    assert!(execute(sse(&oversized_nodes)).1.is_err());
}

#[test]
fn dedicated_codec_accepts_exact_source_field_limits_and_rejects_one_byte_over() {
    let url_prefix = "https://source.example.org/";
    let exact_url = format!(
        "{url_prefix}{}",
        "u".repeat(MAX_WEB_SEARCH_SOURCE_URL_BYTES - url_prefix.len())
    );
    let exact_result = json!({
        "results": [{
            "title": "t".repeat(MAX_WEB_SEARCH_SOURCE_TITLE_BYTES),
            "url": exact_url
        }]
    });
    let exact_events = vec![
        call_event("provider-search-1", "perplexity_search", true),
        result_event("provider-search-1", exact_result),
        finish_event("stop"),
    ];
    assert!(execute(sse(&exact_events)).1.is_ok());

    for result in [
        json!({
            "results": [{
                "title": "t".repeat(MAX_WEB_SEARCH_SOURCE_TITLE_BYTES + 1),
                "url": "https://source.example.org/"
            }]
        }),
        json!({
            "results": [{
                "title": "title",
                "url": format!("{url_prefix}{}", "u".repeat(MAX_WEB_SEARCH_SOURCE_URL_BYTES + 1 - url_prefix.len()))
            }]
        }),
    ] {
        let events = vec![
            call_event("provider-search-1", "perplexity_search", true),
            result_event("provider-search-1", result),
            finish_event("stop"),
        ];
        assert!(execute(sse(&events)).1.is_err());
    }
}
