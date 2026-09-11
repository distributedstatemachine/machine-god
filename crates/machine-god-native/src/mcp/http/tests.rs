use super::*;
use futures_util::future::join;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
};

mod streaming;
mod submission;
pub(crate) mod tls_fixture;

const DISCOVER: &[u8] = br#"{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{}}"#;

pub(crate) fn executor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn connection(address: SocketAddr, cancellation: CancellationToken) -> McpHttpConnection {
    let endpoint = McpEndpoint::parse(&format!(
        "http://127.0.0.1:{}/mcp?private=query",
        address.port()
    ))
    .unwrap();
    McpHttpConnection::new(
        McpHttpDestination::new(endpoint, &[address]).unwrap(),
        &[("Authorization", b"Bearer private-value")],
        None,
        McpHttpLimits::default(),
        cancellation,
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap()
}
pub(crate) async fn request(stream: &mut (impl AsyncRead + Unpin)) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut scratch = [0u8; 4096];
    loop {
        let count = stream.read(&mut scratch).await.unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&scratch[..count]);
        assert!(bytes.len() < 2 * 1024 * 1024);
        if let Some(head) = memchr::memmem::find(&bytes, b"\r\n\r\n") {
            let mut headers = vec![httparse::EMPTY_HEADER; 270];
            let mut request = httparse::Request::new(&mut headers);
            request.parse(&bytes).unwrap();
            let length: usize = std::str::from_utf8(
                request
                    .headers
                    .iter()
                    .find(|header| header.name.eq_ignore_ascii_case("content-length"))
                    .map_or(b"0".as_slice(), |header| header.value),
            )
            .unwrap()
            .parse()
            .unwrap();
            if bytes.len() >= head + 4 + length {
                assert_eq!(bytes.len(), head + 4 + length);
                return bytes;
            }
        }
    }
}
async fn collect(body: &mut McpHttpBody) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next_chunk().await? {
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn serve(mut stream: impl AsyncRead + AsyncWrite + Unpin, response: &[u8]) -> Vec<u8> {
    let bytes = request(&mut stream).await;
    for part in response.chunks(3) {
        if stream.write_all(part).await.is_err() {
            break;
        }
        if stream.flush().await.is_err() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let _ = stream.shutdown().await;
    bytes
}

#[test]
fn plaintext_streams_content_length_chunked_extensions_trailers_and_eof() {
    executor().block_on(async {
        for wire in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: application/json\r\n\r\nhello"[..],
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2;test=yes\r\nhe\r\n3\r\nllo\r\n0\r\nX-Trailer: observed\r\n\r\n"[..],
            &b"HTTP/1.0 200 OK\r\n\r\nhello"[..],
            &b"HTTP/1.1 103 Early Hints\r\nLink: </hint>\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"[..],
        ] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let client = connection(listener.local_addr().unwrap(), CancellationToken::new());
            let observation = client.observation();
            let server = async { serve(listener.accept().await.unwrap().0, wire).await };
            let client = async {
                let mut response = client.control(McpHttpControl::discovery(DISCOVER).unwrap()).await.unwrap();
                assert_eq!(response.status, 200);
                assert_eq!(collect(&mut response.body).await.unwrap(), b"hello");
                assert!(observation.is_complete());
                if wire.windows(7).any(|value| value == b"chunked") { assert_eq!(response.body.trailers().unwrap().iter().collect::<Vec<_>>(), vec![("x-trailer", &b"observed"[..])]); }
            };
            let ((), written) = join(client, server).await;
            assert!(written.starts_with(b"POST /mcp?private=query HTTP/1.1\r\n"));
            assert!(written.ends_with(DISCOVER));
            assert!(observation.was_attempted());
            assert_eq!(observation.acknowledged_bytes(), written.len());
        }
    });
}

#[test]
fn invalid_or_truncated_framing_poison_the_owned_body() {
    executor().block_on(async {
        for wire in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nshort"[..],
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nab"[..],
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\naXX"[..],
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nContent-Length: 0\r\n\r\n"
                [..],
        ] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let client = connection(listener.local_addr().unwrap(), CancellationToken::new());
            let observation = client.observation();
            let server = async { serve(listener.accept().await.unwrap().0, wire).await };
            let client = async {
                let mut response = client
                    .control(McpHttpControl::discovery(DISCOVER).unwrap())
                    .await
                    .unwrap();
                assert!(collect(&mut response.body).await.is_err());
                assert!(observation.is_complete());
                assert_eq!(response.body.next_chunk().await, Err(McpHttpError::Closed));
            };
            join(client, server).await;
        }
    });
}

#[test]
fn constructors_and_unpolled_future_drops_have_no_network_effects() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let client = connection(listener.local_addr().unwrap(), CancellationToken::new());
        let observation = client.observation();
        let future = client.control(McpHttpControl::discovery(DISCOVER).unwrap());
        assert!(!observation.was_attempted());
        drop(future);
        observation.completed().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn cancelled_request_and_dropped_pending_body_close_without_replay() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let cancel = CancellationToken::new();
        let client = connection(listener.local_addr().unwrap(), cancel.clone());
        let observation = client.observation();
        let server = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            request(&mut stream).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\n")
                .await
                .unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
        };
        let client = async {
            let mut response = client.control(McpHttpControl::listen()).await.unwrap();
            let mut read = Box::pin(response.body.next_chunk());
            assert!(futures_util::poll!(read.as_mut()).is_pending());
            drop(read);
            assert!(observation.is_complete());
            assert_eq!(response.body.next_chunk().await, Err(McpHttpError::Closed));
        };
        join(client, server).await;
        let client = connection(listener.local_addr().unwrap(), cancel.clone());
        let observation = client.observation();
        cancel.cancel();
        assert_eq!(
            client.control(McpHttpControl::listen()).await.unwrap_err(),
            McpHttpError::Cancelled
        );
        assert!(!observation.was_attempted());
        assert!(observation.is_complete());
    });
}

#[test]
fn typed_controls_and_address_authority_reject_bypasses() {
    for method in ["tools/call", "resources/read", "prompts/get", "evil/method"] {
        let bytes = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}","params":{{}}}}"#);
        assert!(McpHttpControl::discovery(bytes.as_bytes()).is_err());
        assert!(McpHttpControl::notification(bytes.as_bytes()).is_err());
    }
    let endpoint = McpEndpoint::parse("http://localhost:8000/mcp").unwrap();
    for addresses in [
        vec![],
        vec!["10.0.0.1:8000".parse().unwrap()],
        vec!["127.0.0.1:8001".parse().unwrap()],
        vec!["127.0.0.1:8000".parse().unwrap(); 2],
    ] {
        assert!(McpHttpDestination::new(endpoint.clone(), &addresses).is_err());
    }
    let private = McpEndpoint::parse("https://private.example:8000/mcp").unwrap();
    assert!(McpHttpDestination::new(private, &["10.0.0.1:8000".parse().unwrap()]).is_ok());
    assert!(McpHttpTrust::new(rustls::RootCertStore::empty()).is_err());
}

#[test]
fn real_tls_preserves_hostname_verification_and_exact_plaintext() {
    executor().block_on(async {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
        let certificate = CertificateDer::from(tls_fixture::TEST_TLS_CERTIFICATE_DER.to_vec());
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            tls_fixture::TEST_TLS_PRIVATE_KEY_DER.to_vec(),
        ));
        let mut server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.clone()], key)
        .unwrap();
        server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certificate).unwrap();
        let trust = McpHttpTrust::new(roots).unwrap();
        for host in ["example.com", "wrong.example"] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let endpoint =
                McpEndpoint::parse(&format!("https://{host}:{}/mcp", address.port())).unwrap();
            let client = McpHttpConnection::new(
                McpHttpDestination::new(endpoint, &[address]).unwrap(),
                &[],
                Some(trust.clone()),
                McpHttpLimits::default(),
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
            let observation = client.observation();
            let server = async {
                let socket = listener.accept().await.unwrap().0;
                if let Ok(tls) = acceptor.accept(socket).await {
                    assert_eq!(tls.get_ref().1.server_name(), Some("example.com"));
                    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
                    Some(serve(tls, b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").await)
                } else {
                    None
                }
            };
            let client = async {
                let response = client
                    .control(McpHttpControl::discovery(DISCOVER).unwrap())
                    .await;
                if host == "example.com" {
                    assert_eq!(collect(&mut response.unwrap().body).await.unwrap(), b"{}");
                } else {
                    assert_eq!(response.unwrap_err(), McpHttpError::Tls);
                    assert!(!observation.was_attempted());
                }
            };
            let ((), request) = join(client, server).await;
            assert!(observation.is_complete());
            assert_eq!(request.is_some(), host == "example.com");
        }
    });
}
