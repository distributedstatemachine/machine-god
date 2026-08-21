#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::future::Future;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use futures_util::{FutureExt, StreamExt, stream};
use machine_god_core::{
    CancellationToken, ContentBlock, Engine, InferenceOptions, Message, ModelEvent, ModelProvider,
    ModelRequest, ProviderError, ProviderErrorKind, Role, SessionId, SessionIncarnationId,
    StopReason, TurnEvent, TurnId,
};
use machine_god_native::{
    AI_GATEWAY_HTTP_DEFAULT_CONNECT_TIMEOUT, AI_GATEWAY_HTTP_DEFAULT_ENDPOINT,
    AI_GATEWAY_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS, AI_GATEWAY_HTTP_DEFAULT_REQUEST_TIMEOUT,
    AI_GATEWAY_HTTP_DEFAULT_RESPONSE_CHUNK_BYTES, AI_GATEWAY_HTTP_MAX_ACTIVE_REQUESTS,
    AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES, AI_GATEWAY_HTTP_MAX_RESPONSE_CHUNK_BYTES,
    AiGatewayBearerToken, AiGatewayByteStream, AiGatewayHttpConfigErrorKind, AiGatewayHttpEndpoint,
    AiGatewayHttpLimits, AiGatewayHttpTransport, AiGatewayProvider, AiGatewayTransport,
    AiGatewayTransportRequest,
};
use machine_god_testkit::{InMemorySessionStore, ScriptedPermissionHandler};

const TEST_TOKEN: &str = "test-token_NEVER_REAL";
const HOSTILE_STATUS_BODY: &str = "hostile-body-credential-NEVER-REFLECT";
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const WAKE_TIMEOUT: Duration = Duration::from_millis(500);

fn http_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build test Tokio runtime")
}

fn block_on_http<F: Future>(future: F) -> F::Output {
    http_runtime().block_on(future)
}

#[derive(Debug, Eq, PartialEq)]
struct CapturedHttpRequest {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl CapturedHttpRequest {
    fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

struct OneShotServer {
    endpoint: String,
    request: mpsc::Receiver<CapturedHttpRequest>,
    worker: thread::JoinHandle<()>,
}

impl OneShotServer {
    fn start(response_parts: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        let address = listener.local_addr().expect("loopback address");
        let (request_tx, request_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept HTTP request");
            configure_stream(&stream);
            let request = read_request(&mut stream).expect("read HTTP request");
            request_tx.send(request).expect("publish HTTP request");
            for part in response_parts {
                stream.write_all(&part).expect("write HTTP response part");
                stream.flush().expect("flush HTTP response part");
            }
            let _ = stream.shutdown(Shutdown::Write);
        });
        Self {
            endpoint: format!("http://{address}/v3/ai/language-model"),
            request: request_rx,
            worker,
        }
    }

    fn endpoint(&self) -> AiGatewayHttpEndpoint {
        AiGatewayHttpEndpoint::loopback_http(&self.endpoint).expect("valid loopback endpoint")
    }

    fn finish(self) -> CapturedHttpRequest {
        let request = self
            .request
            .recv_timeout(IO_TIMEOUT)
            .expect("server did not receive request");
        self.worker.join().expect("loopback server panicked");
        request
    }
}

struct CountingServer {
    endpoint: String,
    requests: Arc<AtomicUsize>,
    stop: mpsc::Sender<()>,
    worker: thread::JoinHandle<()>,
}

struct StalledBodyServer {
    endpoint: String,
    request: mpsc::Receiver<CapturedHttpRequest>,
    teardown: tokio::sync::oneshot::Receiver<PeerTeardown>,
    worker: thread::JoinHandle<()>,
}

struct StalledHeadServer {
    endpoint: String,
    request: mpsc::Receiver<CapturedHttpRequest>,
    request_received: Arc<AtomicUsize>,
    teardown: tokio::sync::oneshot::Receiver<PeerTeardown>,
    worker: thread::JoinHandle<()>,
}

#[derive(Debug, Eq, PartialEq)]
enum PeerTeardown {
    Eof,
    Reset(io::ErrorKind),
    Data(u8),
    TimedOut,
    Other(io::ErrorKind),
}

impl PeerTeardown {
    fn is_closed(&self) -> bool {
        matches!(self, Self::Eof | Self::Reset(_))
    }
}

impl StalledBodyServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stalled-body server");
        let address = listener.local_addr().expect("stalled-body address");
        let (request_tx, request_rx) = mpsc::channel();
        let (teardown_tx, teardown_rx) = tokio::sync::oneshot::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept stalled-body request");
            configure_stream(&stream);
            let request = read_request(&mut stream).expect("read stalled-body request");
            request_tx
                .send(request)
                .expect("publish stalled-body request");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\nConnection: close\r\n\r\nx")
                .expect("write initial stalled body");
            stream.flush().expect("flush initial stalled body");
            let teardown = peer_teardown_within_bound(&mut stream);
            teardown_tx
                .send(teardown)
                .expect("publish connection teardown");
        });
        Self {
            endpoint: format!("http://{address}/v3/ai/language-model"),
            request: request_rx,
            teardown: teardown_rx,
            worker,
        }
    }

    fn endpoint(&self) -> AiGatewayHttpEndpoint {
        AiGatewayHttpEndpoint::loopback_http(&self.endpoint).expect("valid stalled endpoint")
    }

    async fn wait_for_teardown(&mut self) -> PeerTeardown {
        tokio::time::timeout(IO_TIMEOUT, &mut self.teardown)
            .await
            .expect("stalled server teardown timed out")
            .expect("stalled server dropped teardown result")
    }

    fn finish(self) -> CapturedHttpRequest {
        let request = self
            .request
            .recv_timeout(IO_TIMEOUT)
            .expect("stalled server did not receive request");
        self.worker.join().expect("stalled server panicked");
        request
    }
}

impl StalledHeadServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stalled-head server");
        let address = listener.local_addr().expect("stalled-head address");
        let (request_tx, request_rx) = mpsc::channel();
        let request_received = Arc::new(AtomicUsize::new(0));
        let worker_request_received = Arc::clone(&request_received);
        let (teardown_tx, teardown_rx) = tokio::sync::oneshot::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept stalled-head request");
            configure_stream(&stream);
            let request = read_request(&mut stream).expect("read stalled-head request");
            request_tx
                .send(request)
                .expect("publish stalled-head request");
            worker_request_received.store(1, Ordering::Release);
            let teardown = peer_teardown_within_bound(&mut stream);
            teardown_tx
                .send(teardown)
                .expect("publish connection teardown");
        });
        Self {
            endpoint: format!("http://{address}/v3/ai/language-model"),
            request: request_rx,
            request_received,
            teardown: teardown_rx,
            worker,
        }
    }

    fn endpoint(&self) -> AiGatewayHttpEndpoint {
        AiGatewayHttpEndpoint::loopback_http(&self.endpoint).expect("valid stalled endpoint")
    }

    async fn wait_for_teardown(&mut self) -> PeerTeardown {
        tokio::time::timeout(IO_TIMEOUT, &mut self.teardown)
            .await
            .expect("stalled server teardown timed out")
            .expect("stalled server dropped teardown result")
    }

    fn finish(self) -> CapturedHttpRequest {
        let request = self
            .request
            .recv_timeout(IO_TIMEOUT)
            .expect("stalled server did not receive request");
        self.worker.join().expect("stalled server panicked");
        request
    }
}

fn peer_teardown_within_bound(stream: &mut TcpStream) -> PeerTeardown {
    stream
        .set_read_timeout(Some(CLOSE_TIMEOUT))
        .expect("set teardown observation timeout");
    let mut byte = [0_u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => PeerTeardown::Eof,
        Ok(_) => PeerTeardown::Data(byte[0]),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::NotConnected
            ) =>
        {
            PeerTeardown::Reset(error.kind())
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            PeerTeardown::TimedOut
        }
        Err(error) => PeerTeardown::Other(error.kind()),
    }
}

impl CountingServer {
    fn start(response: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind counting server");
        listener
            .set_nonblocking(true)
            .expect("make counting listener nonblocking");
        let address = listener.local_addr().expect("counting server address");
        let requests = Arc::new(AtomicUsize::new(0));
        let worker_requests = Arc::clone(&requests);
        let (stop_tx, stop_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            loop {
                if stop_rx.try_recv().is_ok() {
                    drain_pending(&listener, &response, &worker_requests);
                    break;
                }
                match listener.accept() {
                    Ok((stream, _)) => serve_counted(stream, &response, &worker_requests),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::yield_now(),
                    Err(error) => panic!("counting listener failed: {error}"),
                }
            }
        });
        Self {
            endpoint: format!("http://{address}/v3/ai/language-model"),
            requests,
            stop: stop_tx,
            worker,
        }
    }

    fn endpoint(&self) -> AiGatewayHttpEndpoint {
        AiGatewayHttpEndpoint::loopback_http(&self.endpoint).expect("valid loopback endpoint")
    }

    fn finish(self) -> usize {
        self.stop.send(()).expect("stop counting server");
        self.worker.join().expect("counting server panicked");
        self.requests.load(Ordering::Acquire)
    }
}

fn drain_pending(listener: &TcpListener, response: &[u8], requests: &AtomicUsize) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => serve_counted(stream, response, requests),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(error) => panic!("draining counting listener failed: {error}"),
        }
    }
}

fn serve_counted(mut stream: TcpStream, response: &[u8], requests: &AtomicUsize) {
    configure_stream(&stream);
    read_request(&mut stream).expect("read counted HTTP request");
    requests.fetch_add(1, Ordering::AcqRel);
    stream
        .write_all(response)
        .expect("write counted HTTP response");
    let _ = stream.shutdown(Shutdown::Write);
}

fn configure_stream(stream: &TcpStream) {
    stream
        .set_nonblocking(false)
        .expect("make accepted stream blocking");
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .expect("set server read timeout");
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .expect("set server write timeout");
}

fn read_request(stream: &mut TcpStream) -> io::Result<CapturedHttpRequest> {
    let mut received = Vec::new();
    let header_end = loop {
        if let Some(offset) = received.windows(4).position(|window| window == b"\r\n\r\n") {
            break offset + 4;
        }
        if received.len() >= 128 * 1024 {
            return Err(io::Error::other("request headers exceeded test bound"));
        }
        let mut buffer = [0_u8; 4096];
        let bytes = stream.read(&mut buffer)?;
        if bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended before headers",
            ));
        }
        received.extend_from_slice(&buffer[..bytes]);
    };

    let head = std::str::from_utf8(&received[..header_end - 4])
        .map_err(|_| io::Error::other("non-UTF-8 HTTP request head"))?;
    let mut lines = head.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| io::Error::other("missing HTTP request line"))?
        .to_owned();
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::other("malformed HTTP request header"))?;
        headers.push((name.to_owned(), value.trim().to_owned()));
    }
    let content_length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .ok_or_else(|| io::Error::other("missing content-length"))?
        .1
        .parse::<usize>()
        .map_err(|_| io::Error::other("invalid content-length"))?;
    let mut body = received[header_end..].to_vec();
    while body.len() < content_length {
        let mut buffer = [0_u8; 4096];
        let bytes = stream.read(&mut buffer)?;
        if bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended before body",
            ));
        }
        body.extend_from_slice(&buffer[..bytes]);
    }
    if body.len() != content_length {
        return Err(io::Error::other("request body exceeded content-length"));
    }
    Ok(CapturedHttpRequest {
        request_line,
        headers,
        body,
    })
}

fn response(status: u16, body: &[u8], extra_headers: &[(&str, &str)]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .into_bytes();
    for (name, value) in extra_headers {
        response.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    response.extend_from_slice(b"\r\n");
    response.extend_from_slice(body);
    response
}

fn model_request() -> ModelRequest {
    ModelRequest {
        session_id: SessionId::new("http-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("http-incarnation").unwrap(),
        turn_id: TurnId::new("http-turn").unwrap(),
        messages: vec![Message::text(Role::User, "hello")],
        tools: Vec::new(),
        options: InferenceOptions::default(),
    }
}

#[derive(Debug, Default)]
struct CaptureTransport {
    request: Mutex<Option<AiGatewayTransportRequest>>,
}

impl AiGatewayTransport for CaptureTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> machine_god_core::BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        *self.request.lock().unwrap() = Some(request);
        Box::pin(async {
            Ok(Box::pin(stream::iter([Ok(
                b"data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n".to_vec(),
            )])) as AiGatewayByteStream)
        })
    }
}

fn codec_request() -> AiGatewayTransportRequest {
    let capture = Arc::new(CaptureTransport::default());
    let provider = AiGatewayProvider::new("provider/model", capture.clone()).unwrap();
    let events = futures_executor::block_on(async {
        provider
            .stream(model_request(), CancellationToken::new())
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    });
    assert_eq!(
        events,
        [Ok(ModelEvent::Stop {
            reason: StopReason::Completed
        })]
    );
    capture
        .request
        .lock()
        .unwrap()
        .take()
        .expect("codec produced transport request")
}

fn http_transport(
    endpoint: AiGatewayHttpEndpoint,
    limits: AiGatewayHttpLimits,
) -> AiGatewayHttpTransport {
    AiGatewayHttpTransport::with_endpoint_and_limits(
        AiGatewayBearerToken::new(TEST_TOKEN).unwrap(),
        endpoint,
        limits,
    )
    .unwrap()
}

fn send_direct(
    endpoint: AiGatewayHttpEndpoint,
    limits: AiGatewayHttpLimits,
) -> Result<Vec<Vec<u8>>, ProviderError> {
    let transport = http_transport(endpoint, limits);
    block_on_http(async {
        let stream = transport
            .stream(codec_request(), CancellationToken::new())
            .await?;
        stream.collect::<Vec<_>>().await.into_iter().collect()
    })
}

#[test]
fn default_endpoint_and_transport_construction_are_pinned_and_effect_free() {
    assert_eq!(
        AI_GATEWAY_HTTP_DEFAULT_ENDPOINT,
        "https://ai-gateway.vercel.sh/v3/ai/language-model"
    );
    let endpoint = AiGatewayHttpEndpoint::default();
    let endpoint_debug = format!("{endpoint:?}");
    assert!(endpoint_debug.contains("Production"));
    assert!(!endpoint_debug.contains("ai-gateway.vercel.sh"));

    let transport = AiGatewayHttpTransport::new(AiGatewayBearerToken::new(TEST_TOKEN).unwrap())
        .expect("constructing a transport performs no network request");
    let transport_debug = format!("{transport:?}");
    assert!(transport_debug.contains("AiGatewayHttpTransport"));
    assert!(!transport_debug.contains(TEST_TOKEN));
    assert!(!transport_debug.contains("ai-gateway.vercel.sh"));
}

#[test]
fn bearer_tokens_and_limits_are_strict_and_redacted() {
    let token = AiGatewayBearerToken::new(TEST_TOKEN).unwrap();
    assert_eq!(format!("{token:?}"), "AiGatewayBearerToken(<redacted>)");

    for invalid in [
        String::new(),
        "x".repeat(AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES + 1),
        "line\r\ninjection".to_owned(),
        "contains space".to_owned(),
        "not-ascii-é".to_owned(),
        "=padding-without-token".to_owned(),
        "padding=then-data".to_owned(),
    ] {
        let error = AiGatewayBearerToken::new(invalid.clone()).unwrap_err();
        assert_eq!(
            error.kind(),
            AiGatewayHttpConfigErrorKind::InvalidBearerToken
        );
        let diagnostic = format!("{error:?} {error}");
        if !invalid.is_empty() {
            assert!(!diagnostic.contains(&invalid));
        }
        assert!(!diagnostic.contains("line\r\ninjection"));
    }
    AiGatewayBearerToken::new("a".repeat(AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES)).unwrap();
    AiGatewayBearerToken::new("abc+/._~-==").unwrap();

    let defaults = AiGatewayHttpLimits::default();
    assert_eq!(
        defaults.connect_timeout(),
        AI_GATEWAY_HTTP_DEFAULT_CONNECT_TIMEOUT
    );
    assert_eq!(
        defaults.request_timeout(),
        AI_GATEWAY_HTTP_DEFAULT_REQUEST_TIMEOUT
    );
    assert_eq!(
        defaults.max_active_requests(),
        AI_GATEWAY_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS
    );
    assert_eq!(
        defaults.max_response_chunk_bytes(),
        AI_GATEWAY_HTTP_DEFAULT_RESPONSE_CHUNK_BYTES
    );
    let invalid = AiGatewayHttpLimits::new(Duration::ZERO, IO_TIMEOUT, 1, 1).unwrap_err();
    assert_eq!(invalid.kind(), AiGatewayHttpConfigErrorKind::InvalidLimits);
    let invalid = AiGatewayHttpLimits::new(
        Duration::from_secs(1),
        IO_TIMEOUT,
        AI_GATEWAY_HTTP_MAX_ACTIVE_REQUESTS + 1,
        1,
    )
    .unwrap_err();
    assert_eq!(invalid.kind(), AiGatewayHttpConfigErrorKind::InvalidLimits);
    let invalid = AiGatewayHttpLimits::new(
        Duration::from_secs(1),
        IO_TIMEOUT,
        1,
        AI_GATEWAY_HTTP_MAX_RESPONSE_CHUNK_BYTES + 1,
    )
    .unwrap_err();
    assert_eq!(invalid.kind(), AiGatewayHttpConfigErrorKind::InvalidLimits);
}

#[test]
fn loopback_http_accepts_only_canonical_numeric_loopback_origins() {
    for accepted in [
        "http://127.0.0.1:1/v3/ai/language-model",
        "http://127.0.0.1:80/v3/ai/language-model",
        "http://127.23.45.67:65535/test",
        "http://[::1]:8080/v3/ai/language-model",
    ] {
        AiGatewayHttpEndpoint::loopback_http(accepted)
            .unwrap_or_else(|error| panic!("rejected {accepted}: {error}"));
    }

    for rejected in [
        "http://localhost:8080/v3/ai/language-model",
        "http://user@127.0.0.1:8080/v3/ai/language-model",
        "http://user:password@127.0.0.1:8080/v3/ai/language-model",
        "http://127.0.0.1:8080/v3/ai/language-model#fragment",
        "http://127.0.0.1:8080/v3/ai/language-model?query=1",
        "http://10.0.0.1:8080/v3/ai/language-model",
        "http://[::ffff:127.0.0.1]:8080/v3/ai/language-model",
        "http://127.1:8080/v3/ai/language-model",
        "http://2130706433:8080/v3/ai/language-model",
        "http://0177.0.0.1:8080/v3/ai/language-model",
        "http://0x7f.0.0.1:8080/v3/ai/language-model",
        "http://127.0.0.1.:8080/v3/ai/language-model",
        "https://127.0.0.1:8080/v3/ai/language-model",
        "http://127.0.0.1/v3/ai/language-model",
        "http://127.0.0.1:0/v3/ai/language-model",
    ] {
        let error = AiGatewayHttpEndpoint::loopback_http(rejected).unwrap_err();
        assert_eq!(error.kind(), AiGatewayHttpConfigErrorKind::InvalidEndpoint);
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains(rejected));
    }
}

#[test]
fn one_post_preserves_codec_bytes_and_headers_without_auth_duplication_or_compression() {
    let body = b"data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n";
    let server = OneShotServer::start(vec![response(
        200,
        body,
        &[("Content-Type", "text/event-stream")],
    )]);
    let provider = AiGatewayProvider::new(
        "provider/model",
        Arc::new(http_transport(
            server.endpoint(),
            AiGatewayHttpLimits::default(),
        )),
    )
    .unwrap();
    let events = block_on_http(async {
        provider
            .stream(model_request(), CancellationToken::new())
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    });
    assert_eq!(
        events,
        [Ok(ModelEvent::Stop {
            reason: StopReason::Completed
        })]
    );

    let request = server.finish();
    assert!(
        request
            .request_line
            .starts_with("POST /v3/ai/language-model HTTP/")
    );
    assert_eq!(
        request.body,
        br#"{"prompt":[{"role":"user","content":[{"type":"text","text":"hello"}]}],"tools":[],"toolChoice":{"type":"none"}}"#
    );
    for (name, value) in [
        ("content-type", "application/json"),
        ("ai-gateway-protocol-version", "0.0.1"),
        ("ai-language-model-specification-version", "4"),
        ("ai-language-model-id", "provider/model"),
        ("ai-language-model-streaming", "true"),
        ("x-session-id", "http-session"),
        ("x-session-affinity", "http-session"),
        ("authorization", "Bearer test-token_NEVER_REAL"),
        ("accept", "text/event-stream"),
        ("accept-encoding", "identity"),
    ] {
        assert_eq!(request.header_values(name), [value], "header {name}");
    }
    for absent in [
        "expect",
        "content-encoding",
        "transfer-encoding",
        "user-agent",
        "referer",
        "cookie",
    ] {
        assert!(
            request.header_values(absent).is_empty(),
            "unexpected {absent}"
        );
    }
}

#[test]
fn fragmented_streaming_response_and_http_chunk_bound_reconstruct_exactly() {
    let body = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"text-1\",\"delta\":\"héllo \"}\r\n\r\n",
        "data: {\"type\":\"text-delta\",\"id\":\"text-1\",\"delta\":\"world\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\r\n\r\n"
    );
    let complete_response = response(
        200,
        body.as_bytes(),
        &[("Content-Type", "text/event-stream")],
    );
    let parts = complete_response
        .chunks(2)
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>();
    let server = OneShotServer::start(parts);
    let limits = AiGatewayHttpLimits::new(Duration::from_secs(2), IO_TIMEOUT, 2, 3).unwrap();
    let provider = AiGatewayProvider::new(
        "provider/model",
        Arc::new(http_transport(server.endpoint(), limits)),
    )
    .unwrap();
    let events = block_on_http(async {
        provider
            .stream(model_request(), CancellationToken::new())
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    });
    assert_eq!(
        events,
        [
            Ok(ModelEvent::TextDelta {
                text: "héllo ".to_owned()
            }),
            Ok(ModelEvent::TextDelta {
                text: "world".to_owned()
            }),
            Ok(ModelEvent::Stop {
                reason: StopReason::Completed
            })
        ]
    );
    server.finish();

    let raw_server = OneShotServer::start(vec![response(200, b"abcdefghi", &[])]);
    let chunks = send_direct(raw_server.endpoint(), limits).unwrap();
    assert_eq!(chunks.concat(), b"abcdefghi");
    assert!(
        chunks
            .iter()
            .all(|chunk| !chunk.is_empty() && chunk.len() <= 3)
    );
    raw_server.finish();
}

#[test]
fn response_decompression_is_disabled_and_encoded_bytes_are_exposed_raw() {
    let encoded = b"not-a-valid-gzip-stream";
    let server = OneShotServer::start(vec![response(
        200,
        encoded,
        &[("Content-Encoding", "gzip")],
    )]);
    let chunks = send_direct(server.endpoint(), AiGatewayHttpLimits::default())
        .expect("encoded response is exposed without automatic decoding");
    assert_eq!(chunks.concat(), encoded);
    server.finish();
}

#[test]
fn truncated_response_body_is_a_retryable_redacted_transport_failure() {
    let server = OneShotServer::start(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\nConnection: close\r\n\r\nx".to_vec(),
    ]);
    let error = send_direct(server.endpoint(), AiGatewayHttpLimits::default()).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Transport);
    assert_eq!(error.code, "ai_gateway_http_transport");
    assert!(error.retryable);
    assert!(!format!("{error:?} {error}").contains("1024"));
    server.finish();
}

#[test]
fn malformed_response_framing_is_a_nonretryable_redacted_protocol_failure() {
    let server = OneShotServer::start(vec![
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nZ\r\n".to_vec(),
    ]);
    let error = send_direct(server.endpoint(), AiGatewayHttpLimits::default()).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Protocol);
    assert_eq!(error.code, "ai_gateway_http_protocol");
    assert!(!error.retryable);
    assert!(!format!("{error:?} {error}").contains("chunked"));
    server.finish();
}

#[test]
fn status_classes_are_closed_retryable_only_where_declared_and_redacted() {
    let cases = [
        (
            401,
            ProviderErrorKind::Authentication,
            "ai_gateway_http_authentication",
            false,
        ),
        (
            403,
            ProviderErrorKind::Authentication,
            "ai_gateway_http_authentication",
            false,
        ),
        (
            429,
            ProviderErrorKind::RateLimited,
            "ai_gateway_http_rate_limited",
            true,
        ),
        (
            408,
            ProviderErrorKind::Unavailable,
            "ai_gateway_http_unavailable",
            true,
        ),
        (
            425,
            ProviderErrorKind::Unavailable,
            "ai_gateway_http_unavailable",
            true,
        ),
        (
            500,
            ProviderErrorKind::Unavailable,
            "ai_gateway_http_unavailable",
            true,
        ),
        (
            599,
            ProviderErrorKind::Unavailable,
            "ai_gateway_http_unavailable",
            true,
        ),
        (
            400,
            ProviderErrorKind::InvalidRequest,
            "ai_gateway_http_invalid_request",
            false,
        ),
        (
            404,
            ProviderErrorKind::InvalidRequest,
            "ai_gateway_http_invalid_request",
            false,
        ),
        (
            451,
            ProviderErrorKind::InvalidRequest,
            "ai_gateway_http_invalid_request",
            false,
        ),
        (
            301,
            ProviderErrorKind::Protocol,
            "ai_gateway_http_redirect",
            false,
        ),
        (
            307,
            ProviderErrorKind::Protocol,
            "ai_gateway_http_redirect",
            false,
        ),
        (
            201,
            ProviderErrorKind::Protocol,
            "ai_gateway_http_unexpected_status",
            false,
        ),
    ];

    for (status, kind, code, retryable) in cases {
        let server =
            OneShotServer::start(vec![response(status, HOSTILE_STATUS_BODY.as_bytes(), &[])]);
        let error = send_direct(server.endpoint(), AiGatewayHttpLimits::default()).unwrap_err();
        assert_eq!(error.kind, kind, "status {status}");
        assert_eq!(error.code, code, "status {status}");
        assert_eq!(error.retryable, retryable, "status {status}");
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains(HOSTILE_STATUS_BODY), "status {status}");
        server.finish();
    }
}

#[test]
fn redirect_is_not_followed_and_bearer_token_is_not_forwarded() {
    let target = CountingServer::start(response(200, b"redirect target", &[]));
    let source = OneShotServer::start(vec![response(
        302,
        HOSTILE_STATUS_BODY.as_bytes(),
        &[("Location", target.endpoint.as_str())],
    )]);

    let error = send_direct(source.endpoint(), AiGatewayHttpLimits::default()).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Protocol);
    assert_eq!(error.code, "ai_gateway_http_redirect");
    let source_request = source.finish();
    assert_eq!(source_request.header_values("authorization").len(), 1);
    assert_eq!(target.finish(), 0, "redirect target received a request");
}

#[test]
fn retryable_status_still_causes_exactly_one_post() {
    let server = CountingServer::start(response(503, HOSTILE_STATUS_BODY.as_bytes(), &[]));
    let error = send_direct(server.endpoint(), AiGatewayHttpLimits::default()).unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Unavailable);
    assert!(error.retryable);
    assert_eq!(server.finish(), 1, "transport retried a delivered POST");
}

#[test]
fn pre_cancelled_transport_dispatches_zero_requests() {
    let server = CountingServer::start(response(200, b"unused", &[]));
    let transport = http_transport(server.endpoint(), AiGatewayHttpLimits::default());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let Err(error) = block_on_http(transport.stream(codec_request(), cancellation)) else {
        panic!("pre-cancelled transport unexpectedly returned a stream");
    };
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    assert!(!error.retryable);
    assert_eq!(server.finish(), 0, "pre-cancelled transport connected");
}

#[test]
fn polling_without_tokio_runtime_fails_redacted_and_dispatches_zero_requests() {
    let server = CountingServer::start(response(200, b"unused", &[]));
    let transport = http_transport(server.endpoint(), AiGatewayHttpLimits::default());
    let Err(error) =
        futures_executor::block_on(transport.stream(codec_request(), CancellationToken::new()))
    else {
        panic!("runtime-free transport unexpectedly returned a stream");
    };
    assert_eq!(error.kind, ProviderErrorKind::Transport);
    assert_eq!(error.code, "ai_gateway_http_runtime_required");
    assert!(!error.retryable);
    assert_eq!(
        server.finish(),
        0,
        "runtime-free transport connected to the endpoint"
    );
}

#[test]
fn response_head_cancellation_wakes_and_closes_owned_connection() {
    let mut server = StalledHeadServer::start();
    let transport = http_transport(server.endpoint(), AiGatewayHttpLimits::default());
    let cancellation = CancellationToken::new();
    let runtime = http_runtime();
    let (error, teardown) = runtime.block_on(async {
        let mut startup = Box::pin(transport.stream(codec_request(), cancellation.clone()));
        tokio::time::timeout(IO_TIMEOUT, async {
            loop {
                assert!(
                    startup.as_mut().now_or_never().is_none(),
                    "startup completed before the stalled server received the request"
                );
                if server.request_received.load(Ordering::Acquire) == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("request dispatch did not reach the stalled server");
        let cancel_task = tokio::spawn(async move { cancellation.cancel() });
        let result = tokio::time::timeout(WAKE_TIMEOUT, startup)
            .await
            .unwrap_or_else(|error| {
                panic!("cancellation did not wake stalled response-head acquisition: {error}")
            });
        let Err(error) = result else {
            panic!("cancelled startup unexpectedly returned a stream");
        };
        cancel_task.await.expect("cancellation task panicked");
        let teardown = server.wait_for_teardown().await;
        (error, teardown)
    });
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    assert!(
        teardown.is_closed(),
        "cancelling startup retained the HTTP connection: {teardown:?}"
    );
    server.finish();
}

#[test]
fn dropping_dispatched_startup_closes_owned_connection() {
    let mut server = StalledHeadServer::start();
    let transport = http_transport(server.endpoint(), AiGatewayHttpLimits::default());
    let runtime = http_runtime();
    let teardown = runtime.block_on(async {
        let mut startup = Box::pin(transport.stream(codec_request(), CancellationToken::new()));
        tokio::time::timeout(IO_TIMEOUT, async {
            loop {
                assert!(
                    startup.as_mut().now_or_never().is_none(),
                    "startup completed before the stalled server received the request"
                );
                if server.request_received.load(Ordering::Acquire) == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("request dispatch did not reach the stalled server");
        drop(startup);
        server.wait_for_teardown().await
    });
    assert!(
        teardown.is_closed(),
        "dropping startup retained the HTTP connection: {teardown:?}"
    );
    server.finish();
}

#[test]
fn dropping_unpolled_startup_dispatches_zero_requests() {
    let server = CountingServer::start(response(200, b"unused", &[]));
    let transport = http_transport(server.endpoint(), AiGatewayHttpLimits::default());
    let startup = transport.stream(codec_request(), CancellationToken::new());
    drop(startup);
    assert_eq!(server.finish(), 0, "unpolled startup connected");
}

#[test]
fn body_cancellation_wakes_and_closes_owned_connection() {
    let mut server = StalledBodyServer::start();
    let transport = http_transport(server.endpoint(), AiGatewayHttpLimits::default());
    let cancellation = CancellationToken::new();
    let runtime = http_runtime();
    let mut body = runtime
        .block_on(transport.stream(codec_request(), cancellation.clone()))
        .expect("response head succeeds");
    let first = runtime
        .block_on(body.next())
        .expect("first body chunk")
        .expect("first body chunk succeeds");
    assert_eq!(first, b"x");

    let (error, teardown) = runtime.block_on(async {
        let cancel_task = tokio::spawn(async move { cancellation.cancel() });
        let error = tokio::time::timeout(WAKE_TIMEOUT, body.next())
            .await
            .expect("cancellation did not wake the stalled body")
            .expect("cancellation produces a terminal item")
            .expect_err("cancellation must fail the stream");
        cancel_task.await.expect("cancellation task panicked");
        drop(body);
        let teardown = server.wait_for_teardown().await;
        (error, teardown)
    });
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    assert!(
        teardown.is_closed(),
        "cancelling the stream retained the HTTP connection: {teardown:?}"
    );
    server.finish();
}

#[test]
fn dropping_body_closes_owned_connection_when_host_runtime_advances() {
    let mut server = StalledBodyServer::start();
    let transport = http_transport(server.endpoint(), AiGatewayHttpLimits::default());
    let runtime = http_runtime();
    let mut body = runtime
        .block_on(transport.stream(codec_request(), CancellationToken::new()))
        .expect("response head succeeds");
    let first = runtime
        .block_on(body.next())
        .expect("first body chunk")
        .expect("first body chunk succeeds");
    assert_eq!(first, b"x");
    let teardown = runtime.block_on(async {
        drop(body);
        server.wait_for_teardown().await
    });
    assert!(
        teardown.is_closed(),
        "dropping the stream retained the HTTP connection: {teardown:?}"
    );
    server.finish();
}

#[test]
fn capacity_wait_cancellation_wakes_without_dispatching_another_request() {
    let mut server = StalledBodyServer::start();
    let limits = AiGatewayHttpLimits::new(
        Duration::from_secs(1),
        IO_TIMEOUT,
        1,
        AI_GATEWAY_HTTP_DEFAULT_RESPONSE_CHUNK_BYTES,
    )
    .unwrap();
    let transport = http_transport(server.endpoint(), limits);
    let runtime = http_runtime();
    let mut first_body = runtime
        .block_on(transport.stream(codec_request(), CancellationToken::new()))
        .expect("first response head succeeds");
    let first = runtime
        .block_on(first_body.next())
        .expect("first body chunk")
        .expect("first body chunk succeeds");
    assert_eq!(first, b"x");

    let second_cancellation = CancellationToken::new();
    let error = runtime.block_on(async {
        let second = transport.stream(codec_request(), second_cancellation.clone());
        let cancel_task = tokio::spawn(async move { second_cancellation.cancel() });
        let result = tokio::time::timeout(WAKE_TIMEOUT, second)
            .await
            .unwrap_or_else(|error| panic!("cancellation did not wake capacity waiting: {error}"));
        let Err(error) = result else {
            panic!("capacity-bound second request unexpectedly returned a stream");
        };
        cancel_task.await.expect("cancellation task panicked");
        error
    });
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    let teardown = runtime.block_on(async {
        drop(first_body);
        server.wait_for_teardown().await
    });
    assert!(
        teardown.is_closed(),
        "dropping the capacity holder retained the connection: {teardown:?}"
    );
    server.finish();
}

#[test]
fn real_engine_uses_http_transport_and_persists_assistant_text() {
    let response_body = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"text-1\",\"delta\":\"native \"}\n\n",
        "data: {\"type\":\"text-delta\",\"id\":\"text-1\",\"delta\":\"http\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    );
    let server = OneShotServer::start(vec![response(
        200,
        response_body.as_bytes(),
        &[("Content-Type", "text/event-stream")],
    )]);
    let provider = AiGatewayProvider::new(
        "provider/model",
        Arc::new(http_transport(
            server.endpoint(),
            AiGatewayHttpLimits::default(),
        )),
    )
    .unwrap();
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session_id = SessionId::new("gateway-http-engine-session").unwrap();
    let session = engine
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new("gateway-http-engine-incarnation").unwrap(),
        )
        .unwrap();
    let events = block_on_http(async {
        session
            .prompt("hello")
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
    assert_eq!(record.messages[1].role, Role::Assistant);
    assert_eq!(
        record.messages[1].content,
        [ContentBlock::Text {
            text: "native http".to_owned()
        }]
    );
    server.finish();
}
