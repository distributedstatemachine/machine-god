use super::*;
use crate::mcp::{
    config::{McpServerConfig, McpTransportConfig},
    endpoint::McpEndpoint,
    http::tests::{executor, request},
};
use futures_util::future::join;
use std::net::{Ipv4Addr, SocketAddr};

mod feature;
mod lifecycle;
mod observed;
mod owned_lifetime;
mod submission;
mod subscription;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
};

struct Clock;
impl McpHttpClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn require_send<T: Send>(value: T) -> T {
    value
}
fn options(address: SocketAddr, transport: TransportKind) -> McpHttpPeerOptions {
    let url = format!("http://127.0.0.1:{}/mcp", address.port());
    let endpoint = McpEndpoint::parse(&url).unwrap();
    let config = McpServerConfig::decode(
        "test",
        &serde_json::to_vec(
            &serde_json::json!({"type":"http","url":url,"headers":{"X-Selected":"secret"}}),
        )
        .unwrap(),
    )
    .unwrap();
    let McpTransportConfig::Http(remote) = config.transport() else {
        panic!();
    };
    McpHttpPeerOptions {
        destination: McpHttpDestination::new(endpoint, &[address]).unwrap(),
        trust: None,
        headers: McpResolvedHeaders::resolve(remote, |_| None, None, &[]).unwrap(),
        clock: Arc::new(Clock),
        transport,
        lifetime: deadline().into(),
    }
}
fn success(id: i64, result: serde_json::Value) -> Vec<u8> {
    let mut body = serde_json::json!({"jsonrpc":"2.0","id":id});
    body["result"] = result;
    serde_json::to_vec(&body).unwrap()
}
fn modern(id: i64) -> Vec<u8> {
    success(
        id,
        serde_json::json!({"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{},"resources":{"listChanged":true},"prompts":{}}}),
    )
}
async fn reply(
    socket: &mut (impl AsyncRead + AsyncWrite + Unpin),
    status: u16,
    headers: &str,
    body: &[u8],
) -> Vec<u8> {
    let received = request(socket).await;
    let head = if status == 204 {
        assert!(body.is_empty());
        format!("HTTP/1.1 204 No Content\r\n{headers}\r\n")
    } else {
        format!(
            "HTTP/1.1 {status} Status\r\nContent-Length: {}\r\n{headers}\r\n",
            body.len()
        )
    };
    socket.write_all(head.as_bytes()).await.unwrap();
    socket.write_all(body).await.unwrap();
    socket.flush().await.unwrap();
    received
}
async fn accept_reply(listener: &TcpListener, status: u16, headers: &str, body: &[u8]) -> Vec<u8> {
    reply(
        &mut listener.accept().await.unwrap().0,
        status,
        headers,
        body,
    )
    .await
}
const JSON: &str = "Content-Type: application/json\r\n";
const SSE: &str = "Content-Type: text/event-stream\r\n";

#[test]
fn modern_discovery_catalog_and_notifications_preserve_exact_raw_numbers() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(listener.local_addr().unwrap(), TransportKind::StreamableHttp);
        let server = async {
            let discover = accept_reply(&listener, 200, JSON, &modern(1)).await;
            assert!(String::from_utf8_lossy(&discover).contains("mcp-method: server/discover"));
            let body = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"resultType\":\"complete\",\"tools\":[{\"name\":\"exact\",\"inputSchema\":{\"type\":\"object\",\"minimum\":9007199254740993}}]}}\n\n";
            accept_reply(&listener, 200, SSE, body).await;
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline()).await.unwrap();
            assert_eq!(peer.protocol().version, ProtocolVersion::Modern);
            assert!(peer.capabilities().tools());
            let catalog = peer.catalog(McpCatalogKind::Tools, McpCatalogLimits::default(), Instant::now(), deadline()).await.unwrap();
            assert!(catalog.items().next().unwrap().1.get().contains("9007199254740993"));
            assert_eq!(peer.take_notification().unwrap().envelope().method(), Some("notifications/tools/list_changed"));
            let completion = peer.completion();
            drop(peer);
            assert!(completion.is_complete());
        };
        join(client, server).await;
    });
}

#[test]
fn malformed_success_redirect_authentication_and_old_endpoints_never_retry() {
    executor().block_on(async {
        for (status, headers, bytes) in [
            (200, JSON, &b"{}"[..]),
            (404, "", b""),
            (405, "", b""),
            (200, SSE, b"event: endpoint\ndata: /messages\n\n"),
            (
                200,
                "Content-Type: application/json\r\nMcp-Session-Id: old\r\n",
                b"{}",
            ),
            (302, "Location: https://foreign.test/\r\n", b""),
            (401, "WWW-Authenticate: Bearer secret-challenge\r\n", b""),
        ] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            let client = async {
                let error = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                    .await
                    .unwrap_err();
                assert!(!format!("{error:?} {error}").contains("secret"));
                if status == 401 {
                    assert!(matches!(error, McpHttpPeerError::Authentication(_)));
                }
            };
            join(client, accept_reply(&listener, status, headers, bytes)).await;
            assert!(
                tokio::time::timeout(Duration::from_millis(10), listener.accept())
                    .await
                    .is_err()
            );
        }
    });
}

#[test]
fn dropped_polled_catalog_closes_peer_and_owned_response() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let server = async {
            accept_reply(&listener, 200, JSON, &modern(1)).await;
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let completion = peer.completion();
            let future = peer.catalog(
                McpCatalogKind::Tools,
                McpCatalogLimits::default(),
                Instant::now(),
                deadline(),
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(30), future)
                    .await
                    .is_err()
            );
            assert!(peer.reserve_tool_id().is_err());
            completion.completed().await;
        };
        join(client, server).await;
    });
}

#[test]
fn unpolled_startup_is_inert_and_precancelled_startup_never_connects() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let cancel = CancellationToken::new();
        drop(McpHttpPeer::connect(
            options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            ),
            cancel.clone(),
            deadline(),
        ));
        cancel.cancel();
        assert!(matches!(
            McpHttpPeer::connect(
                options(
                    listener.local_addr().unwrap(),
                    TransportKind::StreamableHttp
                ),
                cancel,
                deadline()
            )
            .await,
            Err(McpHttpPeerError::Cancelled)
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn selected_tls_trust_and_sni_drive_actual_modern_negotiation() {
    use crate::mcp::http::tests::tls_fixture::{
        TEST_TLS_CERTIFICATE_DER, TEST_TLS_PRIVATE_KEY_DER,
    };
    executor().block_on(async {
        let certificate =
            rustls::pki_types::CertificateDer::from(TEST_TLS_CERTIFICATE_DER.to_vec());
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(TEST_TLS_PRIVATE_KEY_DER.to_vec()),
        );
        let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.clone()], key)
        .unwrap();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut selected = options(address, TransportKind::StreamableHttp);
        let endpoint =
            McpEndpoint::parse(&format!("https://example.com:{}/mcp", address.port())).unwrap();
        selected.destination = McpHttpDestination::new(endpoint, &[address]).unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certificate).unwrap();
        selected.trust = Some(McpHttpTrust::new(roots).unwrap());
        let server = async {
            let mut tls = acceptor
                .accept(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            assert_eq!(tls.get_ref().1.server_name(), Some("example.com"));
            reply(&mut tls, 200, JSON, &modern(1)).await;
        };
        let client = async {
            let peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            assert_eq!(peer.protocol().version, ProtocolVersion::Modern);
        };
        join(client, server).await;
    });
}
