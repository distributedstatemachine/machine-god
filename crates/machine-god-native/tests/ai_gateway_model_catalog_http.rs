#![cfg(all(
    any(feature = "ai-gateway-http", feature = "ai-gateway-model-catalog-http"),
    not(target_family = "wasm")
))]

use std::future::Future;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use futures_util::future::{Either, select};
use machine_god_core::{CancellationToken, ModelCatalogProvider, ProviderErrorKind};
use machine_god_native::{
    AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_CONNECT_TIMEOUT,
    AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_ENDPOINT,
    AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS,
    AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_REQUEST_TIMEOUT,
    AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ACTIVE_REQUESTS,
    AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ENDPOINT_BYTES, AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES,
    AiGatewayBearerToken, AiGatewayModelCatalogAccessMode,
    AiGatewayModelCatalogHttpConfigErrorKind, AiGatewayModelCatalogHttpEndpoint,
    AiGatewayModelCatalogHttpLimits, AiGatewayModelCatalogHttpTransport,
    AiGatewayModelCatalogProvider, AiGatewayModelCatalogRequestAccess,
    AiGatewayModelCatalogTransport, AiGatewayModelCatalogTransportErrorKind,
};

const TOKEN: &str = "catalog-token_NEVER_REAL";
const IO_TIMEOUT: Duration = Duration::from_secs(10);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime")
}

fn block_on<F: Future>(future: F) -> F::Output {
    runtime().block_on(future)
}

#[derive(Debug, Eq, PartialEq)]
struct CapturedRequest {
    line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl CapturedRequest {
    fn values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

struct ScriptedServer {
    endpoint: String,
    requests: mpsc::Receiver<CapturedRequest>,
    worker: thread::JoinHandle<()>,
}

impl ScriptedServer {
    fn start(responses: Vec<Vec<u8>>) -> Self {
        Self::start_inner(responses, false)
    }

    fn start_allowing_early_peer_close(responses: Vec<Vec<u8>>) -> Self {
        Self::start_inner(responses, true)
    }

    fn start_inner(responses: Vec<Vec<u8>>, allow_early_peer_close: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        let address = listener.local_addr().expect("server address");
        let (request_tx, request_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept request");
                configure(&stream);
                request_tx
                    .send(read_request(&mut stream).expect("read request"))
                    .expect("publish request");
                if let Err(error) = stream.write_all(&response) {
                    assert!(
                        allow_early_peer_close && is_peer_close_error(&error),
                        "write response: {error}"
                    );
                    continue;
                }
                if let Err(error) = stream.flush() {
                    assert!(
                        allow_early_peer_close && is_peer_close_error(&error),
                        "flush response: {error}"
                    );
                    continue;
                }
                let _ = stream.shutdown(Shutdown::Write);
            }
        });
        Self {
            endpoint: format!("http://{address}/coding-agent/v1/models"),
            requests: request_rx,
            worker,
        }
    }

    fn endpoint(&self) -> AiGatewayModelCatalogHttpEndpoint {
        AiGatewayModelCatalogHttpEndpoint::loopback_http(&self.endpoint)
            .expect("approved loopback endpoint")
    }

    fn finish(self, expected: usize) -> Vec<CapturedRequest> {
        let requests = (0..expected)
            .map(|_| {
                self.requests
                    .recv_timeout(IO_TIMEOUT)
                    .expect("server did not receive expected request")
            })
            .collect();
        self.worker.join().expect("server worker panicked");
        requests
    }
}

fn configure(stream: &TcpStream) {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .expect("set read timeout");
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .expect("set write timeout");
}

fn is_peer_close_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::NotConnected
    )
}

fn read_request(stream: &mut TcpStream) -> io::Result<CapturedRequest> {
    let mut received = Vec::new();
    let header_end = loop {
        if let Some(offset) = received.windows(4).position(|window| window == b"\r\n\r\n") {
            break offset + 4;
        }
        if received.len() >= 64 * 1024 {
            return Err(io::Error::other("request head exceeded test bound"));
        }
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended before head",
            ));
        }
        received.extend_from_slice(&chunk[..read]);
    };
    let head = std::str::from_utf8(&received[..header_end - 4])
        .map_err(|_| io::Error::other("request head was not UTF-8"))?;
    let mut lines = head.split("\r\n");
    let line = lines
        .next()
        .ok_or_else(|| io::Error::other("missing request line"))?
        .to_owned();
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::other("malformed request header"))?;
        headers.push((name.to_owned(), value.trim().to_owned()));
    }
    let body = received[header_end..].to_vec();
    Ok(CapturedRequest {
        line,
        headers,
        body,
    })
}

fn response(status: u16, body: &[u8], headers: &[(&str, String)]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .into_bytes();
    for (name, value) in headers {
        bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    bytes.extend_from_slice(b"\r\n");
    bytes.extend_from_slice(body);
    bytes
}

fn chunked_response(body: &[u8]) -> Vec<u8> {
    let mut response =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
    response.extend_from_slice(body);
    response.extend_from_slice(b"\r\n0\r\n\r\n");
    response
}

fn token() -> AiGatewayBearerToken {
    AiGatewayBearerToken::new(TOKEN).expect("valid test token")
}

fn catalog_transport(
    endpoint: AiGatewayModelCatalogHttpEndpoint,
    limits: AiGatewayModelCatalogHttpLimits,
    authenticated: bool,
) -> AiGatewayModelCatalogHttpTransport {
    AiGatewayModelCatalogHttpTransport::with_endpoint_and_limits(
        authenticated.then(token),
        endpoint,
        limits,
    )
    .expect("construct test transport")
}

fn read_observed_peer_close(stream: &mut TcpStream) -> bool {
    let mut byte = [0_u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(error) => is_peer_close_error(&error),
    }
}

fn exact_limit_body() -> Vec<u8> {
    let empty = br#"{"data":[],"pad":""}"#;
    let mut body = empty.to_vec();
    let insert_at = body.len() - 2;
    body.splice(
        insert_at..insert_at,
        std::iter::repeat_n(b'x', AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES - empty.len()),
    );
    assert_eq!(body.len(), AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES);
    body
}

#[test]
fn production_endpoint_limits_and_debug_are_pinned_and_redacted() {
    assert_eq!(
        AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_ENDPOINT,
        "https://ai-gateway.vercel.sh/coding-agent/v1/models"
    );
    let endpoint = AiGatewayModelCatalogHttpEndpoint::default();
    let endpoint_debug = format!("{endpoint:?}");
    assert!(endpoint_debug.contains("Production"));
    assert!(!endpoint_debug.contains("ai-gateway.vercel.sh"));

    let limits = AiGatewayModelCatalogHttpLimits::default();
    assert_eq!(
        limits.connect_timeout(),
        AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_CONNECT_TIMEOUT
    );
    assert_eq!(
        limits.request_timeout(),
        AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_REQUEST_TIMEOUT
    );
    assert_eq!(
        limits.max_active_requests(),
        AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS
    );
    assert_eq!(AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS, 8);
    assert_eq!(AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ACTIVE_REQUESTS, 32);

    let transport = AiGatewayModelCatalogHttpTransport::new(Some(token())).unwrap();
    let debug = format!("{transport:?}");
    assert!(debug.contains("AiGatewayModelCatalogHttpTransport"));
    assert!(!debug.contains(TOKEN));
    assert!(!debug.contains("ai-gateway.vercel.sh"));
}

#[test]
fn loopback_endpoint_accepts_only_canonical_numeric_loopback_urls() {
    for accepted in [
        "http://127.0.0.1:1/coding-agent/v1/models",
        "http://127.99.42.7:65535/absolute/path",
        "http://[::1]:8080/coding-agent/v1/models",
    ] {
        AiGatewayModelCatalogHttpEndpoint::loopback_http(accepted)
            .unwrap_or_else(|error| panic!("rejected {accepted}: {error}"));
    }

    let oversized = format!(
        "http://127.0.0.1:1/{}",
        "x".repeat(AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ENDPOINT_BYTES)
    );
    let rejected = [
        "",
        "http://localhost:8080/coding-agent/v1/models",
        "http://user@127.0.0.1:8080/coding-agent/v1/models",
        "http://user:pass@127.0.0.1:8080/coding-agent/v1/models",
        "http://127.0.0.1:8080/coding-agent/v1/models?team=x",
        "http://127.0.0.1:8080/coding-agent/v1/models#fragment",
        "https://127.0.0.1:8080/coding-agent/v1/models",
        "ftp://127.0.0.1:8080/coding-agent/v1/models",
        "http://127.0.0.1/coding-agent/v1/models",
        "http://127.0.0.1:0/coding-agent/v1/models",
        "http://10.0.0.1:8080/coding-agent/v1/models",
        "http://[::2]:8080/coding-agent/v1/models",
        "http://[::ffff:127.0.0.1]:8080/coding-agent/v1/models",
        "http://127.1:8080/coding-agent/v1/models",
        "http://2130706433:8080/coding-agent/v1/models",
        "http://0177.0.0.1:8080/coding-agent/v1/models",
        "http://0x7f.0.0.1:8080/coding-agent/v1/models",
        "http://127.0.0.1.:8080/coding-agent/v1/models",
    ];
    for rejected in rejected.into_iter().chain([oversized.as_str()]) {
        let error = AiGatewayModelCatalogHttpEndpoint::loopback_http(rejected).unwrap_err();
        assert_eq!(
            error.kind(),
            AiGatewayModelCatalogHttpConfigErrorKind::InvalidEndpoint
        );
        if !rejected.is_empty() {
            assert!(!format!("{error:?} {error}").contains(rejected));
        }
    }
}

#[test]
fn limits_accept_inclusive_range_and_reject_zero_or_excess() {
    AiGatewayModelCatalogHttpLimits::new(
        Duration::from_nanos(1),
        AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_REQUEST_TIMEOUT,
        1,
    )
    .unwrap();
    AiGatewayModelCatalogHttpLimits::new(
        AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_CONNECT_TIMEOUT,
        AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_REQUEST_TIMEOUT,
        AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ACTIVE_REQUESTS,
    )
    .unwrap();
    for result in [
        AiGatewayModelCatalogHttpLimits::new(Duration::ZERO, Duration::from_secs(1), 1),
        AiGatewayModelCatalogHttpLimits::new(Duration::from_secs(1), Duration::ZERO, 1),
        AiGatewayModelCatalogHttpLimits::new(Duration::from_secs(2), Duration::from_secs(1), 1),
        AiGatewayModelCatalogHttpLimits::new(Duration::from_secs(1), Duration::from_secs(1), 0),
        AiGatewayModelCatalogHttpLimits::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ACTIVE_REQUESTS + 1,
        ),
    ] {
        assert_eq!(
            result.unwrap_err().kind(),
            AiGatewayModelCatalogHttpConfigErrorKind::InvalidLimits
        );
    }
}

#[test]
fn exact_get_request_has_no_body_and_only_frozen_application_headers() {
    let server = ScriptedServer::start(vec![response(200, br#"{"data":[]}"#, &[])]);
    let transport = catalog_transport(
        server.endpoint(),
        AiGatewayModelCatalogHttpLimits::default(),
        true,
    );
    let response = block_on(transport.get(
        AiGatewayModelCatalogRequestAccess::Authenticated,
        Instant::now() + IO_TIMEOUT,
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(response.status(), 200);

    let request = server.finish(1).pop().unwrap();
    assert!(
        request
            .line
            .starts_with("GET /coding-agent/v1/models HTTP/1.1")
    );
    assert!(request.body.is_empty());
    assert_eq!(request.values("accept"), ["application/json"]);
    assert_eq!(request.values("accept-encoding"), ["identity"]);
    assert_eq!(
        request.values("authorization"),
        ["Bearer catalog-token_NEVER_REAL"]
    );
    assert_eq!(request.values("user-agent"), ["machine-god/0.1.0"]);
    for absent in [
        "content-length",
        "transfer-encoding",
        "content-type",
        "cookie",
        "referer",
        "x-vercel-ai-gateway-team",
        "x-team",
    ] {
        assert!(request.values(absent).is_empty(), "unexpected {absent}");
    }
}

#[test]
fn public_request_and_authenticated_fallback_strip_authorization() {
    let server = ScriptedServer::start(vec![
        response(401, b"HOSTILE_SECRET", &[]),
        response(200, br#"{"data":[{"id":"public/model"}]}"#, &[]),
    ]);
    let transport = Arc::new(catalog_transport(
        server.endpoint(),
        AiGatewayModelCatalogHttpLimits::default(),
        true,
    ));
    let provider = AiGatewayModelCatalogProvider::new(
        AiGatewayModelCatalogAccessMode::Authenticated,
        transport,
    );
    let catalog = block_on(provider.list_models(CancellationToken::new())).unwrap();
    assert_eq!(catalog.models()[0].id(), "public/model");
    let requests = server.finish(2);
    assert_eq!(
        requests[0].values("authorization"),
        ["Bearer catalog-token_NEVER_REAL"]
    );
    assert!(requests[1].values("authorization").is_empty());
    assert_eq!(requests[0].line, requests[1].line);
}

#[test]
fn public_direct_request_never_adds_authorization() {
    let server = ScriptedServer::start(vec![response(200, br#"{"data":[]}"#, &[])]);
    let transport = catalog_transport(
        server.endpoint(),
        AiGatewayModelCatalogHttpLimits::default(),
        true,
    );
    block_on(transport.get(
        AiGatewayModelCatalogRequestAccess::Public,
        Instant::now() + IO_TIMEOUT,
        CancellationToken::new(),
    ))
    .unwrap();
    assert!(server.finish(1)[0].values("authorization").is_empty());
}

#[test]
fn redirects_are_terminal_and_the_location_target_is_never_contacted() {
    let target = TcpListener::bind("127.0.0.1:0").expect("bind redirect target");
    target.set_nonblocking(true).unwrap();
    let location = format!("http://{}/stolen", target.local_addr().unwrap());
    let server = ScriptedServer::start(vec![response(
        302,
        b"HOSTILE_SECRET",
        &[("Location", location.clone())],
    )]);
    let transport = Arc::new(catalog_transport(
        server.endpoint(),
        AiGatewayModelCatalogHttpLimits::default(),
        false,
    ));
    let provider =
        AiGatewayModelCatalogProvider::new(AiGatewayModelCatalogAccessMode::PublicOnly, transport);
    let error = block_on(provider.list_models(CancellationToken::new())).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Unavailable);
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains("HOSTILE_SECRET"));
    assert!(!diagnostic.contains(&location));
    server.finish(1);
    assert!(matches!(target.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
}

struct HeadOnlyServer {
    endpoint: String,
    request: mpsc::Receiver<CapturedRequest>,
    peer_closed: mpsc::Receiver<bool>,
    worker: thread::JoinHandle<()>,
}

impl HeadOnlyServer {
    fn start(status: u16, content_length: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind head-only server");
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = mpsc::channel();
        let (closed_tx, closed_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept head-only request");
            configure(&stream);
            request_tx
                .send(read_request(&mut stream).expect("read head-only request"))
                .unwrap();
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            stream.flush().unwrap();
            let closed = read_observed_peer_close(&mut stream);
            closed_tx.send(closed).unwrap();
        });
        Self {
            endpoint: format!("http://{address}/coding-agent/v1/models"),
            request: request_rx,
            peer_closed: closed_rx,
            worker,
        }
    }

    fn endpoint(&self) -> AiGatewayModelCatalogHttpEndpoint {
        AiGatewayModelCatalogHttpEndpoint::loopback_http(&self.endpoint).unwrap()
    }

    fn finish(self) {
        self.request.recv_timeout(IO_TIMEOUT).unwrap();
        assert!(self.peer_closed.recv_timeout(IO_TIMEOUT).unwrap());
        self.worker.join().unwrap();
    }
}

#[test]
fn non_200_body_is_not_consumed_and_oversized_content_length_fails_before_body() {
    let server = HeadOnlyServer::start(503, AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES + 1);
    let transport = catalog_transport(
        server.endpoint(),
        AiGatewayModelCatalogHttpLimits::default(),
        false,
    );
    let response = block_on(transport.get(
        AiGatewayModelCatalogRequestAccess::Public,
        Instant::now() + IO_TIMEOUT,
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(response.status(), 503);
    assert!(response.body().is_empty());
    server.finish();

    let server = HeadOnlyServer::start(200, AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES + 1);
    let transport = catalog_transport(
        server.endpoint(),
        AiGatewayModelCatalogHttpLimits::default(),
        false,
    );
    let error = block_on(transport.get(
        AiGatewayModelCatalogRequestAccess::Public,
        Instant::now() + IO_TIMEOUT,
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(
        error.kind(),
        AiGatewayModelCatalogTransportErrorKind::ResourceLimit
    );
    server.finish();
}

#[test]
fn content_length_and_chunked_body_caps_accept_exact_and_reject_one_excess() {
    for chunked in [false, true] {
        let exact = exact_limit_body();
        let exact_response = if chunked {
            chunked_response(&exact)
        } else {
            response(200, &exact, &[])
        };
        let server = ScriptedServer::start(vec![exact_response]);
        let transport = Arc::new(catalog_transport(
            server.endpoint(),
            AiGatewayModelCatalogHttpLimits::default(),
            false,
        ));
        let provider = AiGatewayModelCatalogProvider::new(
            AiGatewayModelCatalogAccessMode::PublicOnly,
            transport,
        );
        block_on(provider.list_models(CancellationToken::new())).unwrap();
        server.finish(1);

        let mut excess = exact;
        excess.push(b' ');
        let excess_response = if chunked {
            chunked_response(&excess)
        } else {
            response(200, &excess, &[])
        };
        let server = ScriptedServer::start_allowing_early_peer_close(vec![excess_response]);
        let transport = catalog_transport(
            server.endpoint(),
            AiGatewayModelCatalogHttpLimits::default(),
            false,
        );
        let error = block_on(transport.get(
            AiGatewayModelCatalogRequestAccess::Public,
            Instant::now() + IO_TIMEOUT,
            CancellationToken::new(),
        ))
        .unwrap_err();
        assert_eq!(
            error.kind(),
            AiGatewayModelCatalogTransportErrorKind::ResourceLimit
        );
        server.finish(1);
    }
}

#[test]
fn unpolled_expired_cancelled_and_runtime_missing_requests_dispatch_zero_network() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!(
        "http://{}/coding-agent/v1/models",
        listener.local_addr().unwrap()
    );
    let transport = catalog_transport(
        AiGatewayModelCatalogHttpEndpoint::loopback_http(&endpoint).unwrap(),
        AiGatewayModelCatalogHttpLimits::default(),
        false,
    );

    let unpolled = transport.get(
        AiGatewayModelCatalogRequestAccess::Public,
        Instant::now() + IO_TIMEOUT,
        CancellationToken::new(),
    );
    drop(unpolled);

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = block_on(transport.get(
        AiGatewayModelCatalogRequestAccess::Public,
        Instant::now() + IO_TIMEOUT,
        cancellation,
    ))
    .unwrap_err();
    assert_eq!(
        cancelled.kind(),
        AiGatewayModelCatalogTransportErrorKind::Cancelled
    );

    let expired = block_on(transport.get(
        AiGatewayModelCatalogRequestAccess::Public,
        Instant::now(),
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(
        expired.kind(),
        AiGatewayModelCatalogTransportErrorKind::ResourceLimit
    );

    let no_runtime = futures_executor::block_on(transport.get(
        AiGatewayModelCatalogRequestAccess::Public,
        Instant::now() + IO_TIMEOUT,
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(
        no_runtime.kind(),
        AiGatewayModelCatalogTransportErrorKind::RuntimeRequired
    );
    assert!(matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
}

#[test]
fn provider_over_concrete_transport_returns_runtime_required_without_panicking() {
    let transport = Arc::new(AiGatewayModelCatalogHttpTransport::new(None).unwrap());
    let provider =
        AiGatewayModelCatalogProvider::new(AiGatewayModelCatalogAccessMode::PublicOnly, transport);
    let error =
        futures_executor::block_on(provider.list_models(CancellationToken::new())).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Transport);
    assert_eq!(error.code, "RuntimeRequired");
    assert!(!error.retryable);
}

struct StalledBodyServer {
    endpoint: String,
    received: tokio::sync::oneshot::Receiver<()>,
    peer_closed: mpsc::Receiver<bool>,
    worker: thread::JoinHandle<()>,
}

impl StalledBodyServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (received_tx, received_rx) = tokio::sync::oneshot::channel();
        let (closed_tx, closed_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            configure(&stream);
            read_request(&mut stream).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{")
                .unwrap();
            stream.flush().unwrap();
            let _ = received_tx.send(());
            let closed = read_observed_peer_close(&mut stream);
            closed_tx.send(closed).unwrap();
        });
        Self {
            endpoint: format!("http://{address}/coding-agent/v1/models"),
            received: received_rx,
            peer_closed: closed_rx,
            worker,
        }
    }

    fn endpoint(&self) -> AiGatewayModelCatalogHttpEndpoint {
        AiGatewayModelCatalogHttpEndpoint::loopback_http(&self.endpoint).unwrap()
    }

    fn finish(self) {
        assert!(self.peer_closed.recv_timeout(IO_TIMEOUT).unwrap());
        self.worker.join().unwrap();
    }
}

#[test]
fn cancellation_and_drop_tear_down_pending_body_work_and_release_owned_state() {
    let mut server = StalledBodyServer::start();
    let transport = catalog_transport(
        server.endpoint(),
        AiGatewayModelCatalogHttpLimits::default(),
        false,
    );
    let cancellation = CancellationToken::new();
    let cancellation_for_request = cancellation.clone();
    let request = transport.get(
        AiGatewayModelCatalogRequestAccess::Public,
        Instant::now() + IO_TIMEOUT,
        cancellation_for_request,
    );
    let error = runtime().block_on(async {
        let mut request = Box::pin(request);
        match select(request.as_mut(), &mut server.received).await {
            Either::Left((result, _)) => panic!("stalled request completed early: {result:?}"),
            Either::Right((received, _)) => received.expect("server notification"),
        }
        cancellation.cancel();
        request.await.unwrap_err()
    });
    assert_eq!(
        error.kind(),
        AiGatewayModelCatalogTransportErrorKind::Cancelled
    );
    server.finish();

    let mut server = StalledBodyServer::start();
    let transport = catalog_transport(
        server.endpoint(),
        AiGatewayModelCatalogHttpLimits::default(),
        false,
    );
    runtime().block_on(async {
        let mut request = Box::pin(transport.get(
            AiGatewayModelCatalogRequestAccess::Public,
            Instant::now() + IO_TIMEOUT,
            CancellationToken::new(),
        ));
        match select(request.as_mut(), &mut server.received).await {
            Either::Left((result, _)) => panic!("stalled request completed early: {result:?}"),
            Either::Right((received, _)) => received.expect("server notification"),
        }
        drop(request);
    });
    server.finish();
}

#[test]
fn request_timeout_is_resource_limited_and_tears_down_the_connection() {
    let server = StalledBodyServer::start();
    let limits = AiGatewayModelCatalogHttpLimits::new(
        Duration::from_millis(25),
        Duration::from_millis(25),
        1,
    )
    .unwrap();
    let transport = catalog_transport(server.endpoint(), limits, false);
    let error = block_on(transport.get(
        AiGatewayModelCatalogRequestAccess::Public,
        Instant::now() + IO_TIMEOUT,
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(
        error.kind(),
        AiGatewayModelCatalogTransportErrorKind::ResourceLimit
    );
    server.finish();
}

struct CapacityServer {
    endpoint: String,
    first_received: tokio::sync::oneshot::Receiver<()>,
    probe: mpsc::Sender<()>,
    probe_result: mpsc::Receiver<bool>,
    worker: thread::JoinHandle<usize>,
}

impl CapacityServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (first_tx, first_rx) = tokio::sync::oneshot::channel();
        let (probe_tx, probe_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            configure(&first);
            read_request(&mut first).unwrap();
            first
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{")
                .unwrap();
            first.flush().unwrap();
            let _ = first_tx.send(());

            probe_rx.recv_timeout(IO_TIMEOUT).unwrap();
            listener.set_nonblocking(true).unwrap();
            let early = match listener.accept() {
                Ok((mut stream, _)) => {
                    configure(&stream);
                    read_request(&mut stream).unwrap();
                    stream
                        .write_all(&response(200, br#"{"data":[]}"#, &[]))
                        .unwrap();
                    true
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
                Err(error) => panic!("capacity probe failed: {error}"),
            };
            result_tx.send(early).unwrap();
            if early {
                return 2;
            }

            assert!(read_observed_peer_close(&mut first));
            listener.set_nonblocking(false).unwrap();
            let (mut second, _) = listener.accept().unwrap();
            configure(&second);
            read_request(&mut second).unwrap();
            second
                .write_all(&response(200, br#"{"data":[]}"#, &[]))
                .unwrap();
            second.flush().unwrap();
            2
        });
        Self {
            endpoint: format!("http://{address}/coding-agent/v1/models"),
            first_received: first_rx,
            probe: probe_tx,
            probe_result: result_rx,
            worker,
        }
    }

    fn endpoint(&self) -> AiGatewayModelCatalogHttpEndpoint {
        AiGatewayModelCatalogHttpEndpoint::loopback_http(&self.endpoint).unwrap()
    }

    fn finish(self) {
        assert_eq!(self.worker.join().unwrap(), 2);
    }
}

#[test]
fn one_permit_queues_a_second_attempt_and_cancellation_releases_capacity() {
    let mut server = CapacityServer::start();
    let limits =
        AiGatewayModelCatalogHttpLimits::new(Duration::from_secs(1), Duration::from_secs(1), 1)
            .unwrap();
    let transport = Arc::new(catalog_transport(server.endpoint(), limits, false));
    let first_cancellation = CancellationToken::new();
    runtime().block_on(async {
        let first_transport = Arc::clone(&transport);
        let first_token = first_cancellation.clone();
        let first = tokio::spawn(async move {
            first_transport
                .get(
                    AiGatewayModelCatalogRequestAccess::Public,
                    Instant::now() + IO_TIMEOUT,
                    first_token,
                )
                .await
        });
        (&mut server.first_received)
            .await
            .expect("first request reached server");

        let second_transport = Arc::clone(&transport);
        let second = tokio::spawn(async move {
            second_transport
                .get(
                    AiGatewayModelCatalogRequestAccess::Public,
                    Instant::now() + IO_TIMEOUT,
                    CancellationToken::new(),
                )
                .await
        });
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        server.probe.send(()).unwrap();
        assert!(
            !server.probe_result.recv_timeout(IO_TIMEOUT).unwrap(),
            "a second socket escaped the configured one-attempt capacity"
        );

        first_cancellation.cancel();
        let first_error = first.await.unwrap().unwrap_err();
        assert_eq!(
            first_error.kind(),
            AiGatewayModelCatalogTransportErrorKind::Cancelled
        );
        assert_eq!(second.await.unwrap().unwrap().status(), 200);
    });
    server.finish();
}

#[test]
fn missing_authenticated_credential_fails_before_dispatch_and_is_redacted() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!(
        "http://{}/coding-agent/v1/models",
        listener.local_addr().unwrap()
    );
    let transport = catalog_transport(
        AiGatewayModelCatalogHttpEndpoint::loopback_http(&endpoint).unwrap(),
        AiGatewayModelCatalogHttpLimits::default(),
        false,
    );
    let error = block_on(transport.get(
        AiGatewayModelCatalogRequestAccess::Authenticated,
        Instant::now() + IO_TIMEOUT,
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(
        error.kind(),
        AiGatewayModelCatalogTransportErrorKind::MalformedResponse
    );
    assert!(!format!("{error:?} {error}").contains(TOKEN));
    assert!(matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
}

#[test]
fn transport_is_safe_to_share_across_parallel_public_requests() {
    let requests = 8;
    let server = ScriptedServer::start(
        (0..requests)
            .map(|_| response(200, br#"{"data":[]}"#, &[]))
            .collect(),
    );
    let transport = Arc::new(catalog_transport(
        server.endpoint(),
        AiGatewayModelCatalogHttpLimits::default(),
        false,
    ));
    let results = Arc::new(Mutex::new(Vec::new()));
    runtime().block_on(async {
        let mut tasks = Vec::new();
        for _ in 0..requests {
            let transport = Arc::clone(&transport);
            let results = Arc::clone(&results);
            tasks.push(tokio::spawn(async move {
                let result = transport
                    .get(
                        AiGatewayModelCatalogRequestAccess::Public,
                        Instant::now() + IO_TIMEOUT,
                        CancellationToken::new(),
                    )
                    .await;
                results
                    .lock()
                    .unwrap()
                    .push(result.map(|response| response.status()));
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
    });
    assert!(
        results
            .lock()
            .unwrap()
            .iter()
            .all(|result| *result == Ok(200))
    );
    assert_eq!(server.finish(requests).len(), requests);
}
