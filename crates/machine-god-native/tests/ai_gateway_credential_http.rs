#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::future::Future;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use futures_util::StreamExt;
use machine_god_core::{
    CancellationToken, InferenceOptions, Message, ModelEvent, ModelProvider, ModelRequest, Role,
    SessionId, SessionIncarnationId, StopReason, TurnId,
};
use machine_god_native::{
    AiGatewayCredentialEnvironment, AiGatewayCredentialSource, AiGatewayHttpEndpoint,
    AiGatewayHttpLimits, AiGatewayHttpTransport, AiGatewayProvider, discover_ai_gateway_credential,
};

const SELECTED_TOKEN: &str = "credential-bridge-oidc_NEVER_REAL";
const LOWER_TOKEN: &str = "credential-bridge-api-key_NEVER_REAL";
const IO_TIMEOUT: Duration = Duration::from_secs(10);

fn block_on_http<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build test Tokio runtime")
        .block_on(future)
}

fn request() -> ModelRequest {
    ModelRequest {
        session_id: SessionId::new("credential-bridge-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("credential-bridge-incarnation").unwrap(),
        turn_id: TurnId::new("credential-bridge-turn").unwrap(),
        messages: vec![Message::text(Role::User, "hello")],
        tools: Vec::new(),
        options: InferenceOptions::default(),
    }
}

struct OneShotServer {
    endpoint: String,
    request_head: mpsc::Receiver<String>,
    worker: thread::JoinHandle<()>,
}

impl OneShotServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        let address = listener.local_addr().expect("read loopback address");
        let (request_tx, request_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept HTTP request");
            configure_stream(&stream);
            let request_head = read_complete_request(&mut stream).expect("read HTTP request");
            request_tx
                .send(request_head)
                .expect("publish captured request");
            let body = b"data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
            stream.flush().unwrap();
            let _ = stream.shutdown(Shutdown::Write);
        });
        Self {
            endpoint: format!("http://{address}/v3/ai/language-model"),
            request_head: request_rx,
            worker,
        }
    }

    fn endpoint(&self) -> AiGatewayHttpEndpoint {
        AiGatewayHttpEndpoint::loopback_http(&self.endpoint).expect("valid loopback endpoint")
    }

    fn finish(self) -> String {
        let request = self
            .request_head
            .recv_timeout(IO_TIMEOUT)
            .expect("server did not receive request");
        self.worker.join().expect("loopback server panicked");
        request
    }
}

fn configure_stream(stream: &TcpStream) {
    stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
    stream.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
}

fn read_complete_request(stream: &mut TcpStream) -> io::Result<String> {
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
    let content_length = head
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .ok_or_else(|| io::Error::other("missing content-length"))?
        .1
        .trim()
        .parse::<usize>()
        .map_err(|_| io::Error::other("invalid content-length"))?;
    let mut body_bytes = received.len() - header_end;
    while body_bytes < content_length {
        let mut buffer = [0_u8; 4096];
        let bytes = stream.read(&mut buffer)?;
        if bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended before body",
            ));
        }
        body_bytes += bytes;
    }
    Ok(head.to_owned())
}

fn assert_wire_handoff(
    oidc: Option<&str>,
    api_key: Option<&str>,
    expected_source: AiGatewayCredentialSource,
    expected_token: &str,
    forbidden_token: Option<&str>,
) {
    let credential = discover_ai_gateway_credential(AiGatewayCredentialEnvironment::new(
        oidc.map(Into::into),
        api_key.map(Into::into),
    ))
    .unwrap();
    assert_eq!(credential.source(), expected_source);

    let server = OneShotServer::start();
    let transport = AiGatewayHttpTransport::with_endpoint_and_limits(
        credential.into_bearer_token(),
        server.endpoint(),
        AiGatewayHttpLimits::default(),
    )
    .unwrap();
    let provider = AiGatewayProvider::new("provider/model", Arc::new(transport)).unwrap();
    let events = block_on_http(async {
        provider
            .stream(request(), CancellationToken::new())
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

    let request_head = server.finish();
    let authorization = request_head
        .split("\r\n")
        .filter(|line| line.to_ascii_lowercase().starts_with("authorization:"))
        .collect::<Vec<_>>();
    assert_eq!(
        authorization,
        [format!("authorization: Bearer {expected_token}")]
    );
    if let Some(forbidden_token) = forbidden_token {
        assert!(!request_head.contains(forbidden_token));
    }
}

#[test]
fn selected_oidc_credential_is_the_only_authorization_header_on_the_wire() {
    assert_wire_handoff(
        Some(SELECTED_TOKEN),
        Some(LOWER_TOKEN),
        AiGatewayCredentialSource::VercelOidcToken,
        SELECTED_TOKEN,
        Some(LOWER_TOKEN),
    );
}

#[test]
fn fallback_api_key_is_the_only_authorization_header_on_the_wire() {
    assert_wire_handoff(
        Some(""),
        Some(LOWER_TOKEN),
        AiGatewayCredentialSource::AiGatewayApiKey,
        LOWER_TOKEN,
        Some(SELECTED_TOKEN),
    );
}
