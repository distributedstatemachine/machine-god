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
async fn legacy_start(listener: &TcpListener, version: &str, session: &str) {
    accept_reply(listener, 404, "", b"").await;
    let header = format!("{JSON}Mcp-Session-Id: {session}\r\n");
    let received = accept_reply(listener, 200, &header, &success(2, serde_json::json!({"protocolVersion":version,"capabilities":{"tools":{},"resources":{}}}))).await;
    assert!(!String::from_utf8_lossy(&received).contains("mcp-protocol-version:"));
    let initialized = accept_reply(listener, 202, "", b"").await;
    assert!(String::from_utf8_lossy(&initialized).contains("notifications/initialized"));
    assert!(String::from_utf8_lossy(&initialized).contains(&format!("mcp-session-id: {session}")));
    assert_eq!(
        String::from_utf8_lossy(&initialized).contains("mcp-protocol-version:"),
        version != "2025-03-26"
    );
}

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
fn legacy_session_initialization_versions_and_explicit_delete_receipts() {
    executor().block_on(async {
        for version in ["2025-11-25", "2025-06-18", "2025-03-26"] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            let server = async {
                legacy_start(&listener, version, "owned-session").await;
                let request = accept_reply(&listener, 204, "", b"").await;
                assert!(request.starts_with(b"DELETE /mcp HTTP/1.1\r\n"));
                assert!(
                    String::from_utf8_lossy(&request).contains("mcp-session-id: owned-session")
                );
            };
            let client = async {
                let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                    .await
                    .unwrap();
                assert_eq!(peer.protocol().version.as_str(), version);
                let completion = peer.completion();
                assert_eq!(
                    peer.shutdown(deadline()).await,
                    McpHttpSessionTeardown::Confirmed
                );
                assert!(completion.is_complete());
            };
            join(client, server).await;
        }
    });
}

#[test]
fn malformed_success_redirect_and_authentication_never_downgrade() {
    executor().block_on(async {
        for (status, headers, bytes) in [
            (200, JSON, &b"{}"[..]),
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
fn duplicate_and_invalid_session_headers_fail_before_initialized() {
    executor().block_on(async {
        for value in [
            "Mcp-Session-Id: a\r\nMcp-Session-Id: a\r\n",
            "Mcp-Session-Id: has space\r\n",
            "Mcp-Session-Id: \r\n",
        ] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            let server = async {
                accept_reply(&listener, 404, "", b"").await;
                accept_reply(
                    &listener,
                    200,
                    &format!("{JSON}{value}"),
                    &success(
                        2,
                        serde_json::json!({"protocolVersion":"2025-11-25","capabilities":{}}),
                    ),
                )
                .await;
            };
            let client = async {
                assert!(
                    McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                        .await
                        .is_err()
                );
            };
            join(client, server).await;
            assert!(
                tokio::time::timeout(Duration::from_millis(10), listener.accept())
                    .await
                    .is_err()
            );
        }
    });
}

#[test]
fn legacy_post_sse_resumes_with_get_and_never_reposts_catalog_request() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let server = async {
            legacy_start(&listener, "2025-11-25", "resume").await;
            let initial =
                accept_reply(&listener, 200, SSE, b"id: cursor-one\nretry: 0\ndata:\n\n").await;
            assert!(initial.starts_with(b"POST "));
            let final_body = b"data: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"tools\":[]}}\n\n";
            let resumed = accept_reply(&listener, 200, SSE, final_body).await;
            assert!(resumed.starts_with(b"GET "));
            assert!(String::from_utf8_lossy(&resumed).contains("last-event-id: cursor-one"));
            assert!(String::from_utf8_lossy(&resumed).contains("accept: text/event-stream"));
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            peer.catalog(
                McpCatalogKind::Tools,
                McpCatalogLimits::default(),
                Instant::now(),
                deadline(),
            )
            .await
            .unwrap();
        };
        join(client, server).await;
    });
}

#[test]
fn legacy_listener_reconnects_and_preserves_committed_cursor() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(listener.local_addr().unwrap(), TransportKind::StreamableHttp);
        let server = async {
            legacy_start(&listener, "2025-06-18", "listener").await;
            accept_reply(&listener, 200, SSE, b"id: first\n\n").await;
            let request = accept_reply(&listener, 200, SSE, b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/resources/list_changed\"}\n\n").await;
            assert!(String::from_utf8_lossy(&request).contains("last-event-id: first"));
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline()).await.unwrap();
            peer.start_listener(deadline()).await.unwrap();
            assert_eq!(peer.next_notification(deadline()).await.unwrap().envelope().method(), Some("notifications/resources/list_changed"));
            let completion = peer.completion();
            peer.close();
            assert!(completion.is_complete());
        };
        join(client, server).await;
    });
}

#[test]
fn deprecated_sse_owns_get_and_correlates_response_before_post_ack() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(listener.local_addr().unwrap(), TransportKind::LegacySse);
        let server = async {
            let (mut events, _) = listener.accept().await.unwrap();
            let get = request(&mut events).await;
            assert!(get.starts_with(b"GET /mcp "));
            events.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\nevent: endpoint\ndata: /messages?session=one\n\n").await.unwrap();
            let (mut post, _) = listener.accept().await.unwrap();
            let init = request(&mut post).await;
            assert!(init.starts_with(b"POST /messages?session=one "));
            let body = format!("data: {}\n\n", String::from_utf8(success(1, serde_json::json!({"protocolVersion":"2024-11-05","capabilities":{}}))).unwrap());
            events.write_all(body.as_bytes()).await.unwrap();
            tokio::task::yield_now().await;
            post.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n").await.unwrap();
            accept_reply(&listener, 202, "", b"").await;
            events.write_all(b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n").await.unwrap();
            let mut scratch = [0];
            assert_eq!(events.read(&mut scratch).await.unwrap(), 0);
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline()).await.unwrap();
            assert_eq!(peer.protocol().version, ProtocolVersion::Legacy20241105);
            assert_eq!(peer.next_notification(deadline()).await.unwrap().envelope().method(), Some("notifications/tools/list_changed"));
            let completion = peer.completion();
            drop(peer);
            completion.completed().await;
        };
        join(client, server).await;
    });
}

#[test]
fn deprecated_sse_endpoint_cannot_widen_selected_authority() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(listener.local_addr().unwrap(), TransportKind::LegacySse);
        let client = async {
            assert!(
                McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                    .await
                    .is_err()
            );
        };
        join(
            client,
            accept_reply(
                &listener,
                200,
                SSE,
                b"event: endpoint\ndata: https://foreign.test/messages\n\n",
            ),
        )
        .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn dropped_polled_catalog_closes_peer_and_persistent_listener() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let server = async {
            legacy_start(&listener, "2025-11-25", "drop").await;
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
