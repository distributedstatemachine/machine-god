#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::future::Future;
use std::pin::Pin;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

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
    WEB_SEARCH_MAX_ACTIVE_REQUESTS, WEB_SEARCH_TOOL_NAME, WebSearchConfigErrorKind,
    WebSearchDeadline, WebSearchLimits, WebSearchRequest, WebSearchResponse, WebSearchSource,
    WebSearchTool, WebSearchTransport, WebSearchTransportError, WebSearchTransportErrorKind,
};
use serde_json::{Value, json};

mod web_search_support;

use web_search_support::{never_deadline, production_gateway_target};

const PRIVATE_QUERY: &str = "latest Rust release PRIVATE_QUERY_SENTINEL";
const NONCANONICAL_URL_IPV4_HOSTS: &[(&str, &str)] = &[
    ("127.1", "127.0.0.1"),
    ("127.0.1", "127.0.0.1"),
    ("127.65537", "127.1.0.1"),
    ("2130706433", "127.0.0.1"),
    ("127.0.0.01", "127.0.0.1"),
    ("0177.0.0.1", "127.0.0.1"),
    ("0300.0000.0002.0001", "192.0.2.1"),
    ("017700000001", "127.0.0.1"),
    ("0x7f.0.0.1", "127.0.0.1"),
    ("0X7f.0.0.1", "127.0.0.1"),
    ("0x7f.1", "127.0.0.1"),
    ("0x7f000001", "127.0.0.1"),
    ("127.0.0.0x", "127.0.0.0"),
    ("1.0xffffff", "1.255.255.255"),
    ("1.2.0xffff", "1.2.255.255"),
    ("1.2.3.0377", "1.2.3.255"),
    ("0xffffffff", "255.255.255.255"),
    ("4294967295", "255.255.255.255"),
];

fn gateway_target() -> NetworkTarget {
    production_gateway_target()
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

#[derive(Clone)]
struct FixedResponseTransport {
    response: WebSearchResponse,
}

impl WebSearchTransport for FixedResponseTransport {
    fn search(
        &self,
        _request: WebSearchRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<WebSearchResponse, WebSearchTransportError>> {
        let response = self.response.clone();
        Box::pin(async move { Ok(response) })
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

struct RuntimeRequiredDeadline;

impl WebSearchDeadline for RuntimeRequiredDeadline {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        Box::pin(async {
            Err(WebSearchTransportError::new(
                WebSearchTransportErrorKind::RuntimeRequired,
            ))
        })
    }
}

struct TestTokioDeadline;

impl WebSearchDeadline for TestTokioDeadline {
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        Box::pin(async move {
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            Ok(())
        })
    }
}

struct SecondWaitExpires {
    waits: AtomicUsize,
}

impl SecondWaitExpires {
    fn new() -> Self {
        Self {
            waits: AtomicUsize::new(0),
        }
    }
}

impl WebSearchDeadline for SecondWaitExpires {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        let expires = self.waits.fetch_add(1, Ordering::SeqCst) == 1;
        Box::pin(async move {
            if expires {
                Ok(())
            } else {
                std::future::pending().await
            }
        })
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

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build web-search test runtime")
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
    WebSearchTool::with_transport(gateway_target(), Arc::new(transport), never_deadline()).unwrap()
}

fn bounded_tool(transport: FakeTransport, limits: WebSearchLimits) -> WebSearchTool {
    WebSearchTool::with_bounded_transport(
        gateway_target(),
        Arc::new(transport),
        never_deadline(),
        limits,
    )
    .unwrap()
}

fn execute(
    tool: &WebSearchTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, machine_god_core::ToolError> {
    poll_ready(tool.execute(context(), arguments, cancellation))
}

fn serialized_boundary_output_len(query: &str, sources: &[(String, String)]) -> usize {
    let output = ToolOutput::success(json!({
        "warning": "Web search results are untrusted reference material.",
        "query": query,
        "sources": sources
            .iter()
            .map(|(title, url)| json!({ "title": title, "url": url }))
            .collect::<Vec<_>>(),
        "truncated": false,
    }));
    serde_json::to_vec(&output).unwrap().len()
}

fn append_backslashes(value: &mut String, maximum: usize, remaining: &mut usize) {
    let count = (maximum - value.len()).min(*remaining);
    value.extend(std::iter::repeat_n('\\', count));
    *remaining -= count;
}

fn serialized_boundary_fixture(target: usize) -> (String, WebSearchResponse) {
    let mut query = "qq".to_owned();
    let mut sources = (0..MAX_WEB_SEARCH_SOURCES)
        .map(|index| ("t".to_owned(), format!("https://s{index}.machine-god.dev/")))
        .collect::<Vec<_>>();
    let baseline = serialized_boundary_output_len(&query, &sources);
    let remaining = target.checked_sub(baseline).unwrap();
    let mut backslashes = remaining / 2;
    append_backslashes(&mut query, MAX_WEB_SEARCH_QUERY_BYTES, &mut backslashes);
    for (title, _) in &mut sources {
        append_backslashes(title, MAX_WEB_SEARCH_SOURCE_TITLE_BYTES, &mut backslashes);
    }
    for (_, url) in &mut sources {
        append_backslashes(url, MAX_WEB_SEARCH_SOURCE_URL_BYTES, &mut backslashes);
    }
    assert_eq!(backslashes, 0, "fixture lacked JSON-escape capacity");
    if remaining % 2 == 1 {
        let (_, url) = sources.last_mut().unwrap();
        assert!(url.len() < MAX_WEB_SEARCH_SOURCE_URL_BYTES);
        url.push('a');
    }
    assert_eq!(serialized_boundary_output_len(&query, &sources), target);
    let sources = sources
        .into_iter()
        .map(|(title, url)| WebSearchSource::new(title, url).unwrap())
        .collect();
    let response = WebSearchResponse::new(sources, false).unwrap();
    (query, response)
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
#[allow(clippy::too_many_lines)]
fn both_public_constructors_strictly_validate_the_exact_authorization_target() {
    for target in [
        NetworkTarget {
            scheme: "ftp".to_owned(),
            host: "search.machine-god.dev".to_owned(),
            port: None,
        },
        NetworkTarget {
            scheme: "HTTPS".to_owned(),
            host: "search.machine-god.dev".to_owned(),
            port: None,
        },
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "gateway".to_owned(),
            port: None,
        },
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "Search.machine-god.dev".to_owned(),
            port: None,
        },
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "search.machine-god.dev.".to_owned(),
            port: None,
        },
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "user@search.machine-god.dev".to_owned(),
            port: None,
        },
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "search.machine-god.dev/path".to_owned(),
            port: None,
        },
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "[2001:db8::1]".to_owned(),
            port: None,
        },
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "search.machine-god.dev".to_owned(),
            port: Some(0),
        },
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "search.machine-god.dev".to_owned(),
            port: Some(443),
        },
        NetworkTarget {
            scheme: "http".to_owned(),
            host: "search.machine-god.dev".to_owned(),
            port: Some(80),
        },
    ] {
        let default_error = WebSearchTool::with_transport(
            target.clone(),
            Arc::new(FakeTransport::new(Mode::Pending)),
            never_deadline(),
        )
        .unwrap_err();
        assert_eq!(
            default_error.kind(),
            WebSearchConfigErrorKind::InvalidTarget
        );
        let bounded_error = WebSearchTool::with_bounded_transport(
            target,
            Arc::new(FakeTransport::new(Mode::Pending)),
            never_deadline(),
            WebSearchLimits::default(),
        )
        .unwrap_err();
        assert_eq!(
            bounded_error.kind(),
            WebSearchConfigErrorKind::InvalidTarget
        );
    }

    for target in [
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "search.machine-god.dev".to_owned(),
            port: Some(8443),
        },
        NetworkTarget {
            scheme: "http".to_owned(),
            host: "192.0.2.1".to_owned(),
            port: None,
        },
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "2001:db8::1".to_owned(),
            port: None,
        },
    ] {
        assert!(
            WebSearchTool::with_transport(
                target,
                Arc::new(FakeTransport::new(Mode::Pending)),
                never_deadline(),
            )
            .is_ok()
        );
    }
}

#[test]
fn url_standard_ipv4_spellings_have_one_exact_target_identity() {
    for &(host, canonical) in NONCANONICAL_URL_IPV4_HOSTS {
        let parsed = reqwest::Url::parse(&format!("https://{host}/")).unwrap();
        assert_eq!(parsed.host_str(), Some(canonical));
        let target = NetworkTarget {
            scheme: "https".to_owned(),
            host: host.to_owned(),
            port: None,
        };
        let default_error = WebSearchTool::with_transport(
            target.clone(),
            Arc::new(FakeTransport::new(Mode::Pending)),
            never_deadline(),
        )
        .unwrap_err();
        assert_eq!(
            default_error.kind(),
            WebSearchConfigErrorKind::InvalidTarget,
            "default constructor accepted URL IPv4 alias {host:?}"
        );
        let bounded_error = WebSearchTool::with_bounded_transport(
            target,
            Arc::new(FakeTransport::new(Mode::Pending)),
            never_deadline(),
            WebSearchLimits::default(),
        )
        .unwrap_err();
        assert_eq!(
            bounded_error.kind(),
            WebSearchConfigErrorKind::InvalidTarget,
            "bounded constructor accepted URL IPv4 alias {host:?}"
        );
    }

    for host in [
        "0.0.0.0",
        "127.0.0.1",
        "192.0.2.1",
        "255.255.255.255",
        "123.search.machine-god.dev",
        "v2.123-machine.example.org",
    ] {
        let target = NetworkTarget {
            scheme: "https".to_owned(),
            host: host.to_owned(),
            port: None,
        };
        assert!(
            WebSearchTool::with_transport(
                target,
                Arc::new(FakeTransport::new(Mode::Pending)),
                never_deadline(),
            )
            .is_ok(),
            "rejected canonical IP or ordinary DNS host {host:?}"
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

    let maximum_domains = (0..MAX_WEB_SEARCH_DOMAIN_FILTERS)
        .map(|index| {
            format!(
                "d{index:02}{}.{}.{}.{}",
                "a".repeat(60),
                "b".repeat(63),
                "c".repeat(63),
                "d".repeat(61)
            )
        })
        .collect::<Vec<_>>();
    assert!(
        maximum_domains
            .iter()
            .all(|domain| domain.len() == MAX_WEB_SEARCH_DOMAIN_BYTES)
    );
    assert_eq!(
        maximum_domains.iter().map(String::len).sum::<usize>(),
        MAX_WEB_SEARCH_DOMAIN_FILTERS * MAX_WEB_SEARCH_DOMAIN_BYTES
    );
    assert!(
        tool.prepare(call(
            WEB_SEARCH_TOOL_NAME,
            json!({ "query": "bounded query", "allowed_domains": maximum_domains })
        ))
        .is_ok()
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
fn url_standard_ipv4_spellings_are_never_normalized_as_dns_filters() {
    let transport = FakeTransport::new(Mode::Pending);
    let tool = tool(transport.clone());
    for domain in NONCANONICAL_URL_IPV4_HOSTS
        .iter()
        .map(|(host, _)| *host)
        .chain(["127.0.0.1", "127.1."])
    {
        for arguments in [
            json!({ "query": "bounded query", "allowed_domains": [domain] }),
            json!({ "query": "bounded query", "blocked_domains": [format!(" {domain} ")] }),
        ] {
            let error = tool
                .prepare(call(WEB_SEARCH_TOOL_NAME, arguments))
                .unwrap_err();
            assert_eq!(
                error.kind,
                ToolErrorKind::InvalidInput,
                "accepted URL IPv4 spelling as a DNS filter: {domain:?}"
            );
        }
    }

    for domain in ["123.search.machine-god.dev", "v2.123-machine.example.org"] {
        assert!(
            tool.prepare(call(
                WEB_SEARCH_TOOL_NAME,
                json!({ "query": "bounded query", "allowed_domains": [domain] }),
            ))
            .is_ok(),
            "rejected ordinary DNS host {domain:?}"
        );
    }
    assert_eq!(transport.calls(), 0);
}

#[test]
fn source_and_response_constructors_accept_exact_limits_and_reject_one_excess() {
    let prefix = "https://example.com/";
    let exact_url = format!(
        "{prefix}{}",
        "x".repeat(MAX_WEB_SEARCH_SOURCE_URL_BYTES - prefix.len())
    );
    let source = WebSearchSource::new(
        "t".repeat(MAX_WEB_SEARCH_SOURCE_TITLE_BYTES),
        exact_url.clone(),
    )
    .unwrap();
    let debug = format!("{source:?}");
    assert_eq!(debug, "WebSearchSource { .. }");
    assert!(!debug.contains(&exact_url));
    assert!(!debug.contains(&"t".repeat(MAX_WEB_SEARCH_SOURCE_TITLE_BYTES)));
    assert_eq!(source.title().len(), MAX_WEB_SEARCH_SOURCE_TITLE_BYTES);
    assert_eq!(source.url().len(), MAX_WEB_SEARCH_SOURCE_URL_BYTES);
    assert!(
        WebSearchSource::new(
            "t".repeat(MAX_WEB_SEARCH_SOURCE_TITLE_BYTES + 1),
            "https://example.com".to_owned(),
        )
        .is_err()
    );
    assert!(WebSearchSource::new("title".to_owned(), format!("{exact_url}x")).is_err());
    assert!(WebSearchResponse::new(vec![source.clone(); MAX_WEB_SEARCH_SOURCES], false).is_ok());
    assert!(WebSearchResponse::new(vec![source; MAX_WEB_SEARCH_SOURCES + 1], false).is_err());
}

#[test]
fn citation_urls_reject_url_ipv4_alias_matrix_without_rejecting_dns() {
    for host in NONCANONICAL_URL_IPV4_HOSTS
        .iter()
        .map(|(host, _)| *host)
        .chain(["127.0.0.1", "127.1."])
    {
        for authority in [host.to_owned(), format!("{host}:8443")] {
            let error =
                WebSearchSource::new("title".to_owned(), format!("https://{authority}/result"))
                    .unwrap_err();
            assert_eq!(
                error.kind(),
                WebSearchTransportErrorKind::InvalidResponse,
                "accepted URL IPv4 spelling as a citation: {authority:?}"
            );
        }
    }

    for url in [
        "https://123.search.machine-god.dev/result",
        "https://v2.123-machine.example.org/result",
    ] {
        assert!(
            WebSearchSource::new("title".to_owned(), url.to_owned()).is_ok(),
            "rejected ordinary DNS citation {url:?}"
        );
    }
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
fn serialized_tool_output_accepts_exact_limit_and_rejects_one_escaped_byte_more() {
    let (exact_query, exact_response) =
        serialized_boundary_fixture(MAX_WEB_SEARCH_SERIALIZED_RESULT_BYTES);
    let exact_tool = WebSearchTool::with_transport(
        gateway_target(),
        Arc::new(FixedResponseTransport {
            response: exact_response,
        }),
        never_deadline(),
    )
    .unwrap();
    let output = execute(
        &exact_tool,
        json!({ "query": exact_query }),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_vec(&output).unwrap().len(),
        MAX_WEB_SEARCH_SERIALIZED_RESULT_BYTES
    );

    let (over_query, over_response) =
        serialized_boundary_fixture(MAX_WEB_SEARCH_SERIALIZED_RESULT_BYTES + 1);
    let over_tool = WebSearchTool::with_transport(
        gateway_target(),
        Arc::new(FixedResponseTransport {
            response: over_response,
        }),
        never_deadline(),
    )
    .unwrap();
    let error = execute(
        &over_tool,
        json!({ "query": over_query }),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Execution);
    assert_eq!(error.code, "web_search_result_too_large");
}

#[test]
fn pre_cancel_same_poll_cancel_and_drop_are_owned_without_leaks() {
    let pending = FakeTransport::new(Mode::Pending);
    let pending_tool = tool(pending.clone());
    let unpolled = pending_tool.execute(
        context(),
        json!({ "query": "latest Rust release" }),
        CancellationToken::new(),
    );
    assert_eq!(pending.calls(), 0);
    drop(unpolled);
    assert_eq!(pending.calls(), 0);

    let cancelled = CancellationToken::new();
    assert!(cancelled.cancel());
    let error = execute(
        &pending_tool,
        json!({ "query": "latest Rust release" }),
        cancelled,
    )
    .unwrap_err();
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

    let mut execution = Box::pin(pending_tool.execute(
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

#[test]
fn driverless_tokio_runtime_returns_typed_error_without_panicking_or_aborting() {
    const CHILD: &str = "MACHINE_GOD_DRIVERLESS_WEB_SEARCH_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let transport = FakeTransport::new(Mode::Pending);
        let tool = WebSearchTool::with_transport(
            gateway_target(),
            Arc::new(transport.clone()),
            Arc::new(RuntimeRequiredDeadline),
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build deliberately driverless runtime");
        let error = runtime
            .block_on(tool.execute(
                context(),
                json!({ "query": "latest Rust release" }),
                CancellationToken::new(),
            ))
            .unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::Unavailable);
        assert_eq!(transport.calls(), 0);
        return;
    }

    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "driverless_tokio_runtime_returns_typed_error_without_panicking_or_aborting",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .status()
        .expect("run isolated driverless-runtime regression");
    assert!(status.success(), "driverless child exited with {status}");
}

#[test]
fn bounded_timeout_covers_transport_and_releases_the_owned_future() {
    let transport = FakeTransport::new(Mode::Pending);
    let limits = WebSearchLimits::new(Duration::from_millis(10), 1).unwrap();
    let tool = WebSearchTool::with_bounded_transport(
        gateway_target(),
        Arc::new(transport.clone()),
        Arc::new(TestTokioDeadline),
        limits,
    )
    .unwrap();
    let error = runtime()
        .block_on(tool.execute(
            context(),
            json!({ "query": "latest Rust release" }),
            CancellationToken::new(),
        ))
        .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Unavailable);
    assert_eq!(transport.calls(), 1);
    assert_eq!(transport.drops(), 1);
}

#[test]
fn queued_execution_times_out_without_transport_dispatch() {
    let transport = FakeTransport::new(Mode::Pending);
    let tool = WebSearchTool::with_bounded_transport(
        gateway_target(),
        Arc::new(transport.clone()),
        Arc::new(SecondWaitExpires::new()),
        WebSearchLimits::new(Duration::from_secs(5), 1).unwrap(),
    )
    .unwrap();
    let mut first = Box::pin(tool.execute(
        context(),
        json!({ "query": "first queued timeout query" }),
        CancellationToken::new(),
    ));
    assert!(poll_once(first.as_mut()).is_pending());
    assert_eq!(transport.calls(), 1);

    let mut second = Box::pin(tool.execute(
        context(),
        json!({ "query": "second queued timeout query" }),
        CancellationToken::new(),
    ));
    let Poll::Ready(Err(error)) = poll_once(second.as_mut()) else {
        panic!("second queued search must expire before dispatch")
    };
    assert_eq!(error.kind, ToolErrorKind::Unavailable);
    assert_eq!(transport.calls(), 1);
    drop(first);
    assert_eq!(transport.drops(), 1);
}

#[test]
fn bounded_capacity_queues_without_dispatch_and_cancel_releases_every_future() {
    runtime().block_on(async {
        let transport = FakeTransport::new(Mode::Pending);
        let limits = WebSearchLimits::new(Duration::from_secs(5), 1).unwrap();
        let tool = bounded_tool(transport.clone(), limits);
        let mut first = Box::pin(tool.execute(
            context(),
            json!({ "query": "first bounded query" }),
            CancellationToken::new(),
        ));
        assert!(poll_once(first.as_mut()).is_pending());
        assert_eq!(transport.calls(), 1);

        let second_cancellation = CancellationToken::new();
        let mut second = Box::pin(tool.execute(
            context(),
            json!({ "query": "second bounded query" }),
            second_cancellation.clone(),
        ));
        assert!(poll_once(second.as_mut()).is_pending());
        assert_eq!(transport.calls(), 1);
        assert!(second_cancellation.cancel());
        let Poll::Ready(Err(error)) = poll_once(second.as_mut()) else {
            panic!("queued cancellation must settle on the next poll")
        };
        assert_eq!(error.kind, ToolErrorKind::Cancelled);
        assert_eq!(transport.calls(), 1);

        drop(first);
        assert_eq!(transport.drops(), 1);
        drop(second);
        assert_eq!(transport.drops(), 1);
    });
}

fn assert_capacity_width(tool: &WebSearchTool, transport: &FakeTransport, width: usize) {
    let mut executions = (0..=width)
        .map(|index| {
            Box::pin(tool.execute(
                context(),
                json!({ "query": format!("capacity query {index}") }),
                CancellationToken::new(),
            ))
        })
        .collect::<Vec<_>>();
    for execution in executions.iter_mut().take(width) {
        assert!(poll_once(execution.as_mut()).is_pending());
    }
    assert_eq!(transport.calls(), width);
    assert!(poll_once(executions[width].as_mut()).is_pending());
    assert_eq!(transport.calls(), width);
    drop(executions.remove(0));
    assert_eq!(transport.drops(), 1);
    assert!(poll_once(executions.last_mut().unwrap().as_mut()).is_pending());
    assert_eq!(transport.calls(), width + 1);
}

#[test]
fn every_public_constructor_enforces_default_four_or_explicit_hard_sixteen_width() {
    let default_transport = FakeTransport::new(Mode::Pending);
    let default_tool = tool(default_transport.clone());
    assert_capacity_width(
        &default_tool,
        &default_transport,
        WEB_SEARCH_DEFAULT_MAX_ACTIVE_REQUESTS,
    );

    let hard_transport = FakeTransport::new(Mode::Pending);
    let hard_tool = bounded_tool(
        hard_transport.clone(),
        WebSearchLimits::new(Duration::from_secs(1), WEB_SEARCH_MAX_ACTIVE_REQUESTS).unwrap(),
    );
    assert_capacity_width(&hard_tool, &hard_transport, WEB_SEARCH_MAX_ACTIVE_REQUESTS);
}

#[test]
fn capacity_one_permit_is_reused_after_success_error_and_drop() {
    let limits = WebSearchLimits::new(Duration::from_secs(1), 1).unwrap();

    let success = FakeTransport::new(Mode::Success);
    let success_tool = bounded_tool(success.clone(), limits);
    for query in ["first successful query", "second successful query"] {
        execute(
            &success_tool,
            json!({ "query": query }),
            CancellationToken::new(),
        )
        .unwrap();
    }
    assert_eq!(success.calls(), 2);
    assert_eq!(success.drops(), 2);

    let failing = FakeTransport::new(Mode::Error(WebSearchTransportErrorKind::Protocol));
    let failing_tool = bounded_tool(failing.clone(), limits);
    for query in ["first failing query", "second failing query"] {
        assert!(
            execute(
                &failing_tool,
                json!({ "query": query }),
                CancellationToken::new(),
            )
            .is_err()
        );
    }
    assert_eq!(failing.calls(), 2);
    assert_eq!(failing.drops(), 2);

    let pending = FakeTransport::new(Mode::Pending);
    let pending_tool = bounded_tool(pending.clone(), limits);
    let mut first = Box::pin(pending_tool.execute(
        context(),
        json!({ "query": "first dropped query" }),
        CancellationToken::new(),
    ));
    assert!(poll_once(first.as_mut()).is_pending());
    assert_eq!(pending.calls(), 1);
    drop(first);
    assert_eq!(pending.drops(), 1);
    let mut second = Box::pin(pending_tool.execute(
        context(),
        json!({ "query": "second after drop" }),
        CancellationToken::new(),
    ));
    assert!(poll_once(second.as_mut()).is_pending());
    assert_eq!(pending.calls(), 2);
}
