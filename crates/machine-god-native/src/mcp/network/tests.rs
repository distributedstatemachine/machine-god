use super::*;
use futures_util::{FutureExt, future::join};
use hickory_proto::{
    op::{Message, MessageType, OpCode},
    rr::{RData, Record, RecordType},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
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
fn executor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
fn network(
    udp: Option<SocketAddr>,
    tcp: Option<SocketAddr>,
    owner: CancellationToken,
) -> NativeMcpNetwork {
    NativeMcpNetwork::new(
        McpResolverConfig::new(
            &[McpNameServer {
                udp,
                tcp,
                trust_negative_responses: true,
            }],
            Duration::from_millis(100),
            1,
            1,
        )
        .unwrap(),
        [7; 32],
        None,
        Arc::new(Clock),
        owner,
        1,
    )
    .unwrap()
}
fn answer(wire: &[u8]) -> Message {
    let query = Message::from_vec(wire).unwrap();
    assert_eq!(query.queries.len(), 1);
    assert!(query.queries[0].name().is_fqdn());
    assert_eq!(query.queries[0].name().to_string(), "private.example.");
    let question = query.queries[0].clone();
    let data = match question.query_type() {
        RecordType::A => RData::A(Ipv4Addr::new(10, 1, 2, 3).into()),
        RecordType::AAAA => RData::AAAA("fd00::123".parse::<Ipv6Addr>().unwrap().into()),
        other => panic!("unexpected family: {other:?}"),
    };
    let mut response = Message::new(query.metadata.id, MessageType::Response, OpCode::Query);
    response.queries.push(question.clone());
    response
        .answers
        .push(Record::from_rdata(question.name().clone(), 60, data));
    response
}
async fn udp_answers(socket: &UdpSocket, wrong_id: bool) {
    for _ in 0..2 {
        let mut wire = [0; 4096];
        let (n, from) = socket.recv_from(&mut wire).await.unwrap();
        let mut response = answer(&wire[..n]);
        if wrong_id {
            response.metadata.id = response.metadata.id.wrapping_add(1);
        }
        socket
            .send_to(&response.to_vec().unwrap(), from)
            .await
            .unwrap();
    }
}

#[test]
fn explicit_authority_and_unpolled_futures_are_inert() {
    executor().block_on(async {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let network = network(
            Some(socket.local_addr().unwrap()),
            None,
            CancellationToken::new(),
        );
        let endpoint = McpEndpoint::parse("http://localhost:1234/mcp").unwrap();
        let cancellation = CancellationToken::new();
        drop(network.admit_endpoint(&endpoint, &cancellation, deadline()));
        let mut buffer = [0; 512];
        assert!(socket.try_recv_from(&mut buffer).is_err());
        assert_eq!(network.permits.available_permits(), 1);
        assert!(!format!("{network:?}").contains("127.0.0.1"));
    });
}

#[test]
fn literal_localhost_ipv6_and_oauth_urls_keep_exact_endpoint() {
    executor().block_on(async {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let network = network(
            Some(socket.local_addr().unwrap()),
            None,
            CancellationToken::new(),
        );
        for url in [
            "http://127.0.0.1:1234/mcp?a=secret",
            "http://[::1]:1234/token",
            "http://localhost:1234/other",
        ] {
            let admitted =
                McpAuthNetwork::admit(&network, url, &CancellationToken::new(), deadline())
                    .await
                    .unwrap();
            assert_eq!(admitted.destination.endpoint().as_str(), url);
            assert!(admitted.trust.is_none());
        }
        assert!(
            McpAuthNetwork::admit(
                &network,
                "http://private.example:80/token",
                &CancellationToken::new(),
                deadline()
            )
            .await
            .is_err()
        );
        assert!(
            McpAuthNetwork::admit(
                &network,
                "https://private.example/token",
                &CancellationToken::new(),
                deadline()
            )
            .await
            .is_err()
        );
        assert!(socket.try_recv_from(&mut [0; 512]).is_err());
    });
}

#[test]
fn udp_resolves_both_private_families_as_one_absolute_name() {
    executor().block_on(async {
        for host in ["private.example", "private.example."] {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let network = network(
                Some(socket.local_addr().unwrap()),
                None,
                CancellationToken::new(),
            );
            let cancellation = CancellationToken::new();
            let (result, ()) = join(
                network.lookup(host, &cancellation, deadline()),
                udp_answers(&socket, false),
            )
            .await;
            assert_eq!(
                result.unwrap(),
                vec![
                    "10.1.2.3".parse::<IpAddr>().unwrap(),
                    "fd00::123".parse::<IpAddr>().unwrap()
                ]
            );
        }
    });
}

#[test]
fn wrong_id_is_rejected_without_a_successful_partial_family() {
    executor().block_on(async {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let network = network(
            Some(socket.local_addr().unwrap()),
            None,
            CancellationToken::new(),
        );
        let cancellation = CancellationToken::new();
        let (result, ()) = join(
            network.lookup("private.example", &cancellation, deadline()),
            udp_answers(&socket, true),
        )
        .await;
        assert!(result.is_err());
    });
}

#[test]
fn truncated_udp_replays_only_the_dns_question_over_owned_tcp() {
    executor().block_on(async {
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let network = network(
            Some(udp.local_addr().unwrap()),
            Some(tcp.local_addr().unwrap()),
            CancellationToken::new(),
        );
        let cancellation = CancellationToken::new();
        let serve_udp = async {
            for _ in 0..2 {
                let mut wire = [0; 4096];
                let (n, peer) = udp.recv_from(&mut wire).await.unwrap();
                let mut response = answer(&wire[..n]);
                response.metadata.truncation = true;
                response.answers.clear();
                udp.send_to(&response.to_vec().unwrap(), peer)
                    .await
                    .unwrap();
            }
        };
        let serve_tcp = async {
            for _ in 0..2 {
                let (mut socket, _) = tcp.accept().await.unwrap();
                let n = socket.read_u16().await.unwrap();
                let mut wire = vec![0; usize::from(n)];
                socket.read_exact(&mut wire).await.unwrap();
                let response = answer(&wire).to_vec().unwrap();
                socket
                    .write_u16(u16::try_from(response.len()).unwrap())
                    .await
                    .unwrap();
                socket.write_all(&response).await.unwrap();
            }
        };
        let (result, _) = join(
            network.lookup("private.example", &cancellation, deadline()),
            join(serve_udp, serve_tcp),
        )
        .await;
        assert_eq!(result.unwrap().len(), 2);
    });
}

#[test]
fn cancellation_deadline_and_drop_release_lookup_permits() {
    executor().block_on(async {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let owner = CancellationToken::new();
        let network = network(Some(socket.local_addr().unwrap()), None, owner.clone());
        let cancel = CancellationToken::new();
        let operation = network.run(&cancel, deadline(), async {
            let _permit = network.permits.acquire().await.unwrap();
            network.lookup("private.example", &cancel, deadline()).await
        });
        let mut operation = Box::pin(operation);
        assert!(operation.as_mut().now_or_never().is_none());
        assert_eq!(network.permits.available_permits(), 0);
        owner.cancel();
        assert_eq!(operation.await.unwrap_err(), McpNetworkError::Cancelled);
        assert_eq!(network.permits.available_permits(), 1);
        let endpoint = McpEndpoint::parse("http://localhost:1234/mcp").unwrap();
        assert!(
            network
                .admit_endpoint(&endpoint, &cancel, deadline())
                .await
                .is_err()
        );
        let fresh = self::network(
            Some(socket.local_addr().unwrap()),
            None,
            CancellationToken::new(),
        );
        assert!(matches!(
            fresh
                .admit_endpoint(&endpoint, &cancel, Instant::now())
                .await,
            Err(McpNetworkError::Deadline)
        ));
        let mut pending = Box::pin(fresh.run(&cancel, deadline(), async {
            let _permit = fresh.permits.acquire().await.unwrap();
            std::future::pending::<Result<()>>().await
        }));
        assert!(pending.as_mut().now_or_never().is_none());
        drop(pending);
        assert_eq!(fresh.permits.available_permits(), 1);
    });
}

#[test]
fn bounds_reject_empty_invalid_and_oversized_selection() {
    let server = McpNameServer {
        udp: Some("127.0.0.1:53".parse().unwrap()),
        tcp: None,
        trust_negative_responses: true,
    };
    for servers in [&[][..], &[server; 33][..]] {
        assert!(McpResolverConfig::new(servers, Duration::from_secs(1), 1, 1).is_err());
    }
    assert!(McpResolverConfig::new(&[server], Duration::ZERO, 1, 1).is_err());
    assert!(McpResolverConfig::new(&[server], Duration::from_secs(1), 6, 1).is_err());
    assert!(
        McpResolverConfig::new(
            &[McpNameServer {
                udp: Some("0.0.0.0:53".parse().unwrap()),
                ..server
            }],
            Duration::from_secs(1),
            1,
            1
        )
        .is_err()
    );
}

#[test]
fn tcp_only_supports_ipv6_nameserver_and_owned_cancellation() {
    executor().block_on(async {
        let tcp = TcpListener::bind("[::1]:0").await.unwrap();
        let network = network(
            None,
            Some(tcp.local_addr().unwrap()),
            CancellationToken::new(),
        );
        let cancellation = CancellationToken::new();
        let server = async {
            for _ in 0..2 {
                let (mut socket, _) = tcp.accept().await.unwrap();
                let n = socket.read_u16().await.unwrap();
                let mut wire = vec![0; usize::from(n)];
                socket.read_exact(&mut wire).await.unwrap();
                let response = answer(&wire).to_vec().unwrap();
                socket
                    .write_u16(u16::try_from(response.len()).unwrap())
                    .await
                    .unwrap();
                socket.write_all(&response).await.unwrap();
            }
        };
        let (result, ()) = join(
            network.lookup("private.example", &cancellation, deadline()),
            server,
        )
        .await;
        assert_eq!(result.unwrap().len(), 2);
        let cancel_on_accept = async {
            let (mut stream, _) = tcp.accept().await.unwrap();
            // Read the complete query so EOF below cannot be buffered query data.
            let n = stream.read_u16().await.unwrap();
            let mut wire = vec![0; usize::from(n)];
            stream.read_exact(&mut wire).await.unwrap();
            cancellation.cancel();
            let result = stream.read(&mut [0; 1]).await;
            assert!(matches!(result, Ok(0)) || result.is_err());
        };
        let (result, ()) = join(
            network.lookup("private.example", &cancellation, deadline()),
            cancel_on_accept,
        )
        .await;
        assert_eq!(result.unwrap_err(), McpNetworkError::Cancelled);
    });
}

#[test]
fn tls_admission_uses_exact_dns_results_and_explicit_trust() {
    executor().block_on(async {
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut network = network(
            Some(udp.local_addr().unwrap()),
            None,
            CancellationToken::new(),
        );
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(rustls::pki_types::CertificateDer::from(
                crate::mcp::http::tests::tls_fixture::TEST_TLS_CERTIFICATE_DER.to_vec(),
            ))
            .unwrap();
        network.trust = Some(McpHttpTrust::new(roots).unwrap());
        let endpoint =
            McpEndpoint::parse("https://private.example:8443/token?audience=private").unwrap();
        let cancel = CancellationToken::new();
        let (result, ()) = join(
            network.admit_endpoint(&endpoint, &cancel, deadline()),
            udp_answers(&udp, false),
        )
        .await;
        let admitted = result.unwrap();
        assert!(admitted.trust.is_some());
        assert_eq!(admitted.destination.endpoint(), &endpoint);
        assert_eq!(network.permits.available_permits(), 1);
    });
}

#[test]
fn queued_admission_deadline_and_cancellation_never_acquire_network() {
    executor().block_on(async {
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let network = network(
            Some(udp.local_addr().unwrap()),
            None,
            CancellationToken::new(),
        );
        let permit = network.permits.acquire().await.unwrap();
        let endpoint = McpEndpoint::parse("http://localhost:1234/mcp").unwrap();
        let cancel = CancellationToken::new();
        let mut queued = Box::pin(network.admit_endpoint(&endpoint, &cancel, deadline()));
        assert!(queued.as_mut().now_or_never().is_none());
        cancel.cancel();
        assert!(matches!(queued.await, Err(McpNetworkError::Cancelled)));
        let cancel = CancellationToken::new();
        assert!(matches!(
            network
                .admit_endpoint(
                    &endpoint,
                    &cancel,
                    Instant::now() + Duration::from_millis(5)
                )
                .await,
            Err(McpNetworkError::Deadline)
        ));
        assert!(udp.try_recv_from(&mut [0; 512]).is_err());
        drop(permit);
        assert_eq!(network.permits.available_permits(), 1);
    });
}

#[test]
fn oversized_dns_family_is_not_silently_discarded() {
    executor().block_on(async {
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let network = network(
            Some(udp.local_addr().unwrap()),
            None,
            CancellationToken::new(),
        );
        let cancel = CancellationToken::new();
        let server = async {
            for _ in 0..2 {
                let mut wire = [0; 4096];
                let (n, peer) = udp.recv_from(&mut wire).await.unwrap();
                let mut response = answer(&wire[..n]);
                if response.queries[0].query_type() == RecordType::AAAA {
                    response.answers = vec![response.answers[0].clone(); 33];
                }
                udp.send_to(&response.to_vec().unwrap(), peer)
                    .await
                    .unwrap();
            }
        };
        let (result, ()) = join(
            network.lookup("private.example", &cancel, deadline()),
            server,
        )
        .await;
        assert_eq!(result.unwrap_err(), McpNetworkError::Unavailable);
    });
}

#[test]
fn explicit_literal_only_never_resolves_or_normalizes_foreign_names() {
    executor().block_on(async {
        let network = NativeMcpNetwork::new(
            McpResolverConfig::literal_only(),
            [7; 32],
            None,
            Arc::new(Clock),
            CancellationToken::new(),
            1,
        )
        .unwrap();
        let cancel = CancellationToken::new();
        assert_eq!(
            network
                .lookup("private.example", &cancel, deadline())
                .await
                .unwrap_err(),
            McpNetworkError::Unavailable
        );
        assert_eq!(
            network
                .lookup("private.example..", &cancel, deadline())
                .await
                .unwrap_err(),
            McpNetworkError::Invalid
        );
        let endpoint = McpEndpoint::parse("http://[::1]:1234/token").unwrap();
        assert!(
            network
                .admit_endpoint(&endpoint, &cancel, deadline())
                .await
                .is_ok()
        );
    });
}
