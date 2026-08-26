#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, NetworkTarget, SessionId, SessionIncarnationId, Tool,
    ToolCall, ToolCallId, ToolContext, ToolErrorKind, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    MAX_WEB_SEARCH_DOMAIN_BYTES, MAX_WEB_SEARCH_DOMAIN_FILTERS, MAX_WEB_SEARCH_JSON_NODES,
    MAX_WEB_SEARCH_QUERY_BYTES, MAX_WEB_SEARCH_REQUEST_BYTES, MAX_WEB_SEARCH_RESPONSE_BYTES,
    MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES, MAX_WEB_SEARCH_RESPONSE_RECORDS,
    MAX_WEB_SEARCH_SERIALIZED_RESULT_BYTES, MAX_WEB_SEARCH_SOURCE_TITLE_BYTES,
    MAX_WEB_SEARCH_SOURCE_URL_BYTES, MAX_WEB_SEARCH_SOURCES, MAX_WEB_SEARCH_TOTAL_DOMAIN_BYTES,
    WEB_SEARCH_DEFAULT_MAX_ACTIVE_REQUESTS, WEB_SEARCH_DEFAULT_REQUEST_TIMEOUT,
    WEB_SEARCH_MAX_ACTIVE_REQUESTS, WEB_SEARCH_TOOL_NAME, WebSearchLimits, WebSearchRequest,
    WebSearchResponse, WebSearchSource, WebSearchTool, WebSearchTransport, WebSearchTransportError,
    WebSearchTransportErrorKind,
};
use serde_json::{Value, json};

const PRIVATE_QUERY: &str = "latest Rust release PRIVATE_QUERY_SENTINEL";

fn gateway_target() -> NetworkTarget {
    NetworkTarget {
        scheme: "https".to_owned(),
        host: "ai-gateway.vercel.sh".to_owned(),
        port: None,
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Success,
    Error(WebSearchTransportErrorKind),
    Pending,
    CancelThenRespond,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RequestRecord {
    query: String,
    allowed_domains: Option<Vec<String>>,
    blocked_domains: Option<Vec<String>>,
    debug: String,
}

#[derive(Default)]
struct State {
    calls: AtomicUsize,
    polls: AtomicUsize,
    drops: AtomicUsize,
    requests: Mutex<Vec<RequestRecord>>,
}

#[derive(Clone)]
struct FakeTransport {
    mode: Mode,
    state: Arc<State>,
}

impl FakeTransport {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            state: Arc::new(State::default()),
        }
    }

    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    fn polls(&self) -> usize {
        self.state.polls.load(Ordering::SeqCst)
    }

    fn drops(&self) -> usize {
        self.state.drops.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<RequestRecord> {
        self.state.requests.lock().unwrap().clone()
    }
}

impl WebSearchTransport for FakeTransport {
    fn search(
        &self,
        request: WebSearchRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<WebSearchResponse, WebSearchTransportError>> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        self.state.requests.lock().unwrap().push(RequestRecord {
            query: request.query().to_owned(),
            allowed_domains: request.allowed_domains().map(<[String]>::to_vec),
            blocked_domains: request.blocked_domains().map(<[String]>::to_vec),
            debug: format!("{request:?}"),
        });
        Box::pin(FakeFuture {
            mode: self.mode,
            cancellation,
            state: Arc::clone(&self.state),
        })
    }
}

struct FakeFuture {
    mode: Mode,
    cancellation: CancellationToken,
    state: Arc<State>,
}

impl Future for FakeFuture {
    type Output = Result<WebSearchResponse, WebSearchTransportError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.state.polls.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            Mode::Success => Poll::Ready(success_response()),
            Mode::Error(kind) => Poll::Ready(Err(WebSearchTransportError::new(kind))),
            Mode::Pending => Poll::Pending,
            Mode::CancelThenRespond => {
                assert!(self.cancellation.cancel());
                Poll::Ready(success_response())
            }
        }
    }
}

impl Drop for FakeFuture {
    fn drop(&mut self) {
        self.state.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn success_response() -> Result<WebSearchResponse, WebSearchTransportError> {
    let source = WebSearchSource::new(
        "Rust releases".to_owned(),
        "https://www.rust-lang.org/tools/install".to_owned(),
    )?;
    WebSearchResponse::new(vec![source], false)
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    match poll_once(future.as_mut()) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("web_search execution unexpectedly remained pending"),
    }
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("web-search-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("web-search-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("web-search-incarnation").unwrap(),
        turn_id: TurnId::new("web-search-turn").unwrap(),
        call_id: ToolCallId::new("web-search-call").unwrap(),
    }
}

fn tool(transport: FakeTransport) -> WebSearchTool {
    WebSearchTool::with_transport(gateway_target(), Arc::new(transport)).unwrap()
}

fn execute(
    tool: &WebSearchTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, machine_god_core::ToolError> {
    poll_ready(tool.execute(context(), arguments, cancellation))
}

#[test]
fn exported_contract_limits_and_schema_are_exact() {
    assert_eq!(WEB_SEARCH_TOOL_NAME, "web_search");
    assert_eq!(MAX_WEB_SEARCH_QUERY_BYTES, 4_096);
    assert_eq!(MAX_WEB_SEARCH_DOMAIN_FILTERS, 16);
    assert_eq!(MAX_WEB_SEARCH_DOMAIN_BYTES, 253);
    assert_eq!(MAX_WEB_SEARCH_TOTAL_DOMAIN_BYTES, 4_096);
    assert_eq!(MAX_WEB_SEARCH_SOURCES, 10);
    assert_eq!(MAX_WEB_SEARCH_SOURCE_TITLE_BYTES, 512);
    assert_eq!(MAX_WEB_SEARCH_SOURCE_URL_BYTES, 2_048);
    assert_eq!(MAX_WEB_SEARCH_REQUEST_BYTES, 16 * 1_024);
    assert_eq!(MAX_WEB_SEARCH_RESPONSE_BYTES, 256 * 1_024);
    assert_eq!(MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES, 64 * 1_024);
    assert_eq!(MAX_WEB_SEARCH_RESPONSE_RECORDS, 256);
    assert_eq!(MAX_WEB_SEARCH_JSON_NODES, 16_384);
    assert_eq!(MAX_WEB_SEARCH_SERIALIZED_RESULT_BYTES, 48 * 1_024);
    assert_eq!(WEB_SEARCH_DEFAULT_REQUEST_TIMEOUT, Duration::from_secs(30));
    assert_eq!(WEB_SEARCH_DEFAULT_MAX_ACTIVE_REQUESTS, 4);
    assert_eq!(WEB_SEARCH_MAX_ACTIVE_REQUESTS, 16);

    let limits = WebSearchLimits::default();
    assert_eq!(limits.request_timeout(), WEB_SEARCH_DEFAULT_REQUEST_TIMEOUT);
    assert_eq!(
        limits.max_active_requests(),
        WEB_SEARCH_DEFAULT_MAX_ACTIVE_REQUESTS
    );
    let custom = WebSearchLimits::new(Duration::from_secs(1), 16).unwrap();
    assert_eq!(custom.request_timeout(), Duration::from_secs(1));
    assert_eq!(custom.max_active_requests(), 16);
    assert!(WebSearchLimits::new(Duration::ZERO, 1).is_err());
    assert!(WebSearchLimits::new(Duration::from_secs(31), 1).is_err());
    assert!(WebSearchLimits::new(Duration::from_secs(1), 0).is_err());
    assert!(WebSearchLimits::new(Duration::from_secs(1), 17).is_err());

    let spec = tool(FakeTransport::new(Mode::Pending)).spec();
    assert_eq!(spec.name.as_str(), WEB_SEARCH_TOOL_NAME);
    assert_eq!(spec.input_schema["type"], "object");
    assert_eq!(spec.input_schema["required"], json!(["query"]));
    assert_eq!(spec.input_schema["additionalProperties"], false);
    assert_eq!(spec.input_schema["properties"]["query"]["type"], "string");
    for name in ["allowed_domains", "blocked_domains"] {
        assert_eq!(spec.input_schema["properties"][name]["type"], "array");
        assert_eq!(
            spec.input_schema["properties"][name]["items"]["type"],
            "string"
        );
        assert_eq!(
            spec.input_schema["properties"][name]["maxItems"],
            MAX_WEB_SEARCH_DOMAIN_FILTERS
        );
    }
}

#[test]
fn prepare_requires_exact_shape_and_canonicalizes_for_one_gateway_capability() {
    let transport = FakeTransport::new(Mode::Pending);
    let tool = tool(transport.clone());
    for invalid in [
        call("another_tool", json!({ "query": "rust release" })),
        call(WEB_SEARCH_TOOL_NAME, json!(null)),
        call(WEB_SEARCH_TOOL_NAME, json!({})),
        call(WEB_SEARCH_TOOL_NAME, json!({ "query": 42 })),
        call(WEB_SEARCH_TOOL_NAME, json!({ "query": "x" })),
        call(
            WEB_SEARCH_TOOL_NAME,
            json!({ "query": "rust release", "extra": true }),
        ),
        call(
            WEB_SEARCH_TOOL_NAME,
            json!({ "query": "rust release", "allowed_domains": "rust-lang.org" }),
        ),
        call(
            WEB_SEARCH_TOOL_NAME,
            json!({
                "query": "rust release",
                "allowed_domains": ["rust-lang.org"],
                "blocked_domains": ["example.com"]
            }),
        ),
    ] {
        let error = tool.prepare(invalid).unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::InvalidInput);
    }

    let prepared = tool
        .prepare(call(
            WEB_SEARCH_TOOL_NAME,
            json!({
                "query": "  latest Rust release  ",
                "allowed_domains": [" RUST-LANG.ORG. ", "docs.rs", "rust-lang.org"]
            }),
        ))
        .unwrap();
    assert_eq!(
        prepared.capability(),
        &Capability::Network {
            target: gateway_target()
        }
    );
    assert_eq!(
        prepared.arguments(),
        &json!({
            "query": "latest Rust release",
            "allowed_domains": ["rust-lang.org", "docs.rs"]
        })
    );
    assert_eq!(transport.calls(), 0);

    let empty = tool
        .prepare(call(
            WEB_SEARCH_TOOL_NAME,
            json!({
                "query": "rust release",
                "allowed_domains": [],
                "blocked_domains": []
            }),
        ))
        .unwrap();
    assert_eq!(empty.arguments(), &json!({ "query": "rust release" }));
}

#[test]
fn query_and_domain_bounds_are_rejected_before_transport() {
    let transport = FakeTransport::new(Mode::Pending);
    let tool = tool(transport.clone());

    let exact_query = "q".repeat(MAX_WEB_SEARCH_QUERY_BYTES);
    assert!(
        tool.prepare(call(WEB_SEARCH_TOOL_NAME, json!({ "query": exact_query })))
            .is_ok()
    );
    let long_query = "q".repeat(MAX_WEB_SEARCH_QUERY_BYTES + 1);
    assert!(
        tool.prepare(call(WEB_SEARCH_TOOL_NAME, json!({ "query": long_query })))
            .is_err()
    );

    let exact_domains = (0..MAX_WEB_SEARCH_DOMAIN_FILTERS)
        .map(|index| format!("d{index}.example.com"))
        .collect::<Vec<_>>();
    assert!(
        tool.prepare(call(
            WEB_SEARCH_TOOL_NAME,
            json!({ "query": "bounded query", "allowed_domains": exact_domains })
        ))
        .is_ok()
    );
    let too_many = (0..=MAX_WEB_SEARCH_DOMAIN_FILTERS)
        .map(|index| format!("d{index}.example.com"))
        .collect::<Vec<_>>();
    assert!(
        tool.prepare(call(
            WEB_SEARCH_TOOL_NAME,
            json!({ "query": "bounded query", "allowed_domains": too_many })
        ))
        .is_err()
    );

    for domain in [
        "localhost",
        "127.0.0.1",
        "[::1]",
        "*.example.com",
        "https://example.com",
        "example.com/path",
        "user@example.com",
        "example.com:443",
        "éxample.com",
        "host.local",
        "home.arpa",
    ] {
        assert!(
            tool.prepare(call(
                WEB_SEARCH_TOOL_NAME,
                json!({ "query": "bounded query", "allowed_domains": [domain] })
            ))
            .is_err(),
            "accepted invalid domain {domain:?}"
        );
    }
    assert_eq!(transport.calls(), 0);
}

#[test]
fn execute_requires_the_exact_prepared_form_and_passes_only_canonical_values() {
    let transport = FakeTransport::new(Mode::Success);
    let tool = tool(transport.clone());
    for arguments in [
        json!({ "query": "  latest Rust release  " }),
        json!({ "query": "latest Rust release", "allowed_domains": ["RUST-LANG.ORG"] }),
        json!({ "query": "latest Rust release", "allowed_domains": [] }),
        json!({ "query": "latest Rust release", "extra": true }),
    ] {
        let error = execute(&tool, arguments, CancellationToken::new()).unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::InvalidInput);
    }
    assert_eq!(transport.calls(), 0);

    let output = execute(
        &tool,
        json!({
            "query": PRIVATE_QUERY,
            "blocked_domains": ["example.com", "invalid.example.org"]
        }),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(transport.calls(), 1);
    let requests = transport.requests();
    assert_eq!(requests[0].query, PRIVATE_QUERY);
    assert_eq!(requests[0].allowed_domains, None);
    assert_eq!(
        requests[0].blocked_domains,
        Some(vec![
            "example.com".to_owned(),
            "invalid.example.org".to_owned()
        ])
    );
    assert_eq!(requests[0].debug, "WebSearchRequest { .. }");
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
    assert!(serde_json::to_vec(&output).unwrap().len() <= MAX_WEB_SEARCH_SERIALIZED_RESULT_BYTES);
}

#[test]
fn pre_cancel_same_poll_cancel_and_drop_are_owned_without_leaks() {
    let pending = FakeTransport::new(Mode::Pending);
    let tool = tool(pending.clone());
    let unpolled = tool.execute(
        context(),
        json!({ "query": "latest Rust release" }),
        CancellationToken::new(),
    );
    assert_eq!(pending.calls(), 0);
    drop(unpolled);
    assert_eq!(pending.calls(), 0);

    let cancelled = CancellationToken::new();
    assert!(cancelled.cancel());
    let error = execute(&tool, json!({ "query": "latest Rust release" }), cancelled).unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(pending.calls(), 0);

    let racing = FakeTransport::new(Mode::CancelThenRespond);
    let error = execute(
        &tool(racing.clone()),
        json!({ "query": "latest Rust release" }),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(racing.calls(), 1);
    assert_eq!(racing.polls(), 1);
    assert_eq!(racing.drops(), 1);

    let mut execution = Box::pin(tool.execute(
        context(),
        json!({ "query": "latest Rust release" }),
        CancellationToken::new(),
    ));
    assert!(poll_once(execution.as_mut()).is_pending());
    assert_eq!(pending.calls(), 1);
    assert_eq!(pending.drops(), 0);
    drop(execution);
    assert_eq!(pending.drops(), 1);
}

#[test]
fn transport_errors_are_fixed_and_never_reflect_query_or_remote_diagnostics() {
    for kind in [
        WebSearchTransportErrorKind::Cancelled,
        WebSearchTransportErrorKind::Timeout,
        WebSearchTransportErrorKind::Authentication,
        WebSearchTransportErrorKind::RateLimited,
        WebSearchTransportErrorKind::Unavailable,
        WebSearchTransportErrorKind::Protocol,
        WebSearchTransportErrorKind::ResponseTooLarge,
        WebSearchTransportErrorKind::ResultTooLarge,
    ] {
        let transport = FakeTransport::new(Mode::Error(kind));
        let error = execute(
            &tool(transport),
            json!({ "query": PRIVATE_QUERY }),
            CancellationToken::new(),
        )
        .unwrap_err();
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains(PRIVATE_QUERY));
        assert!(!rendered.contains("PRIVATE_QUERY_SENTINEL"));
    }
}
