use super::*;
use crate::mcp::lifetime::McpPeerLifetime;

struct TestClock {
    origin: Instant,
    state: Mutex<(u64, CancellationToken)>,
}
impl TestClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            origin: Instant::now(),
            state: Mutex::new((0, CancellationToken::new())),
        })
    }
    fn advance(&self, millis: u64) {
        let changed = {
            let mut state = self.state.lock().unwrap();
            assert!(millis >= state.0);
            state.0 = millis;
            std::mem::replace(&mut state.1, CancellationToken::new())
        };
        changed.cancel();
    }
    fn at(&self, millis: u64) -> Instant {
        self.origin + Duration::from_millis(millis)
    }
}
impl McpHttpClock for TestClock {
    fn now(&self) -> Instant {
        self.at(self.state.lock().unwrap().0)
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            loop {
                let changed = {
                    let state = self.state.lock().unwrap();
                    if self.at(state.0) >= deadline {
                        return;
                    }
                    state.1.clone()
                };
                changed.cancelled().await;
            }
        })
    }
}

#[test]
fn peer_owner_outlives_startup_but_explicit_expiry_remains_exact() {
    executor().block_on(async {
        for expires in [false, true] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let clock = TestClock::new();
            let mut selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            selected.clock = clock.clone();
            selected.lifetime = if expires {
                McpPeerLifetime::Until(clock.at(1500))
            } else {
                McpPeerLifetime::OwnerControlled
            };
            let server = async {
                accept_reply(&listener, 200, JSON, &modern(1)).await;
                if !expires {
                    accept_reply(
                        &listener,
                        200,
                        JSON,
                        &success(2, serde_json::json!({"tools":[]})),
                    )
                    .await;
                }
            };
            let client = async {
                let mut peer =
                    McpHttpPeer::connect(selected, CancellationToken::new(), clock.at(1000))
                        .await
                        .unwrap();
                clock.advance(2000);
                let result = peer
                    .catalog(
                        McpCatalogKind::Tools,
                        McpCatalogLimits::default(),
                        clock.origin,
                        clock.at(3000),
                    )
                    .await;
                if expires {
                    assert!(matches!(result, Err(McpHttpPeerError::Deadline)));
                } else {
                    assert!(result.is_ok());
                }
                let completion = peer.completion();
                peer.close();
                assert!(completion.is_complete());
            };
            tokio::time::timeout(Duration::from_secs(3), join(client, server))
                .await
                .unwrap();
            assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        }
    });
}

async fn listener_server(listener: &TcpListener, deprecated: bool) -> tokio::net::TcpStream {
    if !deprecated {
        legacy_start(listener, "2025-11-25", "owned-listener").await;
    }
    let (mut events, _) = listener.accept().await.unwrap();
    assert!(request(&mut events).await.starts_with(b"GET "));
    events
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
        .await
        .unwrap();
    if deprecated {
        events
            .write_all(b"event: endpoint\ndata: /messages?session=owned\n\n")
            .await
            .unwrap();
        let (mut post, _) = listener.accept().await.unwrap();
        assert!(
            request(&mut post)
                .await
                .starts_with(b"POST /messages?session=owned ")
        );
        let init = success(
            1,
            serde_json::json!({"protocolVersion":"2024-11-05","capabilities":{}}),
        );
        events
            .write_all(format!("data: {}\n\n", String::from_utf8(init).unwrap()).as_bytes())
            .await
            .unwrap();
        post.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        accept_reply(listener, 202, "", b"").await;
    }
    events
}

#[test]
fn persistent_get_and_deprecated_sse_survive_startup_and_idle_read_deadlines() {
    executor().block_on(async {
        for deprecated in [false, true] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let clock = TestClock::new();
            let mut selected = options(
                listener.local_addr().unwrap(),
                if deprecated {
                    TransportKind::LegacySse
                } else {
                    TransportKind::StreamableHttp
                },
            );
            selected.clock = clock.clone();
            selected.lifetime = McpPeerLifetime::OwnerControlled;
            let cancellation = CancellationToken::new();
            let (send_prefix, receive_prefix) = tokio::sync::oneshot::channel();
            let (prefix_sent, prefix_received) = tokio::sync::oneshot::channel();
            let (send_tail, receive_tail) = tokio::sync::oneshot::channel();
            let server = async {
                let mut events = listener_server(&listener, deprecated).await;
                receive_prefix.await.unwrap();
                events
                    .write_all(b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/")
                    .await
                    .unwrap();
                prefix_sent.send(()).unwrap();
                receive_tail.await.unwrap();
                events.write_all(b"list_changed\"}\n\n").await.unwrap();
                let mut byte = [0];
                assert_eq!(events.read(&mut byte).await.unwrap(), 0);
            };
            let client = async {
                let mut peer = McpHttpPeer::connect(selected, cancellation.clone(), clock.at(1000))
                    .await
                    .unwrap();
                if !deprecated {
                    peer.start_listener(clock.at(1000)).await.unwrap();
                }
                let completion = peer.completion();
                clock.advance(2000); // No observation is tied to the spent startup budget.
                send_prefix.send(()).unwrap();
                prefix_received.await.unwrap();
                let mut read = Box::pin(peer.next_notification(clock.at(2500)));
                assert!(futures_util::poll!(&mut read).is_pending());
                tokio::task::yield_now().await;
                assert!(futures_util::poll!(&mut read).is_pending());
                clock.advance(2500);
                assert!(matches!(read.await, Err(McpHttpPeerError::Deadline)));
                assert!(!completion.is_complete());
                assert!(peer.request_head().is_ok());
                send_tail.send(()).unwrap();
                let notification = peer.next_notification(clock.at(4000)).await.unwrap();
                assert_eq!(
                    notification.envelope().method(),
                    Some("notifications/tools/list_changed")
                );
                let mut read = Box::pin(peer.next_notification(clock.at(4000)));
                assert!(futures_util::poll!(&mut read).is_pending());
                cancellation.cancel();
                assert!(matches!(read.await, Err(McpHttpPeerError::Cancelled)));
                completion.completed().await;
                assert!(completion.is_complete());
            };
            tokio::time::timeout(Duration::from_secs(3), join(client, server))
                .await
                .unwrap();
            assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        }
    });
}

#[test]
fn promoted_listener_still_observes_explicit_owner_expiry() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let clock = TestClock::new();
        let mut selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        selected.clock = clock.clone();
        selected.lifetime = McpPeerLifetime::Until(clock.at(2000));
        let server = async {
            let mut events = listener_server(&listener, false).await;
            let mut byte = [0];
            assert_eq!(events.read(&mut byte).await.unwrap(), 0);
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), clock.at(1000))
                .await
                .unwrap();
            peer.start_listener(clock.at(1000)).await.unwrap();
            let completion = peer.completion();
            let mut read = Box::pin(peer.next_notification(clock.at(5000)));
            assert!(futures_util::poll!(&mut read).is_pending());
            clock.advance(2000);
            assert!(matches!(read.await, Err(McpHttpPeerError::Deadline)));
            assert!(completion.is_complete());
        };
        tokio::time::timeout(Duration::from_secs(3), join(client, server))
            .await
            .unwrap();
    });
}

#[test]
fn configured_startup_has_no_whole_operation_cap_and_fallback_gets_fresh_budget() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let clock = TestClock::new();
        let mut selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        selected.clock = clock.clone();
        selected.lifetime = McpPeerLifetime::OwnerControlled;
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            clock.advance(900);
            reply(&mut socket, 404, "", b"").await;
            let (mut socket, _) = listener.accept().await.unwrap();
            clock.advance(1500);
            reply(
                &mut socket,
                200,
                JSON,
                &success(
                    2,
                    serde_json::json!({"protocolVersion":"2025-11-25","capabilities":{}}),
                ),
            )
            .await;
            accept_reply(&listener, 202, "", b"").await;
        };
        let client = async {
            let (mut peer, attempt) = McpHttpPeer::connect_configured_observed(
                selected,
                CancellationToken::new(),
                Duration::from_millis(1000),
                Some(clock.at(1000)),
                Arc::new(|_| true),
            )
            .await
            .unwrap();
            assert_eq!(attempt, clock.at(1900));
            assert_eq!(clock.now(), clock.at(1500));
            peer.close();
            assert!(peer.completion().is_complete());
        };
        tokio::time::timeout(Duration::from_secs(3), join(client, server))
            .await
            .unwrap();
    });
}

#[test]
fn ready_listener_at_deadline_retains_exact_event_without_repolling_completed_future() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let clock = TestClock::new();
        let mut selected = options(listener.local_addr().unwrap(), TransportKind::StreamableHttp);
        selected.clock = clock.clone();
        selected.lifetime = McpPeerLifetime::OwnerControlled;
        let server = async {
            let mut events = listener_server(&listener, false).await;
            events.write_all(b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n").await.unwrap();
            assert_eq!(events.read(&mut [0]).await.unwrap(), 0);
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), clock.at(1000)).await.unwrap();
            peer.start_listener(clock.at(1000)).await.unwrap();
            clock.advance(2000);
            let (reader, event) = peer.listener.take().unwrap().await;
            let during_read = clock.clone();
            peer.listener = Some(Box::pin(async move {
                during_read.advance(2500);
                (reader, event)
            }));
            assert!(matches!(peer.next_notification(clock.at(2500)).await, Err(McpHttpPeerError::Deadline)));
            assert!(peer.request_head().is_ok());
            let frame = peer.next_notification(clock.at(3000)).await.unwrap();
            assert_eq!(frame.envelope().method(), Some("notifications/tools/list_changed"));
            assert!(peer.take_notification().is_none());
            peer.close();
            assert!(peer.completion().is_complete());
        };
        tokio::time::timeout(Duration::from_secs(3), join(client, server)).await.unwrap();
    });
}

#[test]
fn feature_reconnect_transfers_only_live_get_acquisition_to_peer_ownership() {
    executor().block_on(async {
        for revoke_during_acquisition in [false, true] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let mut selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            selected.lifetime = McpPeerLifetime::OwnerControlled;
            let feature_cancel = CancellationToken::new();
            let server = async {
                legacy_start(&listener, "2025-11-25", "guarded-listener").await;
                // A valid empty GET is clean EOF, not a malformed listener.
                accept_reply(&listener, 200, SSE, b"").await;
                let first = listener.accept().await.unwrap().0;
                let second = listener.accept().await.unwrap().0;
                let get_ready = CancellationToken::new();
                // The feature POST may have connected without writing when
                // an already-ready listener EOF initiates its guarded GET.
                // Drive both accepted owners; never assume wire arrival order.
                let (first_get, second_get) = join(
                    feature_socket(
                        first,
                        &feature_cancel,
                        &get_ready,
                        revoke_during_acquisition,
                    ),
                    feature_socket(
                        second,
                        &feature_cancel,
                        &get_ready,
                        revoke_during_acquisition,
                    ),
                )
                .await;
                assert_ne!(first_get, second_get);
            };
            let client = async {
                let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                    .await
                    .unwrap();
                peer.start_listener(deadline()).await.unwrap();
                let completion = peer.completion();
                let result = peer
                    .feature(
                        &crate::mcp::control::tests::request("resource list srv"),
                        "srv",
                        &[],
                        crate::mcp::control::tests::human(feature_cancel.clone()),
                        crate::mcp::control::McpFeatureOperationOptions::new(Instant::now()),
                        deadline(),
                    )
                    .await;
                if revoke_during_acquisition {
                    assert!(matches!(result, Err(McpHttpPeerError::Cancelled)));
                } else {
                    assert!(result.is_ok(), "{result:?}");
                    assert!(peer.feature_authority.is_none());
                    feature_cancel.cancel();
                    let frame = peer.next_notification(deadline()).await.unwrap();
                    assert_eq!(
                        frame.envelope().method(),
                        Some("notifications/resources/list_changed")
                    );
                    peer.close();
                }
                completion.completed().await;
                assert!(completion.is_complete());
            };
            tokio::time::timeout(Duration::from_secs(3), join(client, server))
                .await
                .unwrap();
            assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        }
    });
}

async fn feature_socket(
    mut socket: tokio::net::TcpStream,
    feature_cancel: &CancellationToken,
    get_ready: &CancellationToken,
    revoked: bool,
) -> bool {
    use futures_util::future::{Either, select};
    let incoming = match select(
        Box::pin(feature_cancel.cancelled()),
        Box::pin(request(&mut socket)),
    )
    .await
    {
        Either::Left(_) => None,
        Either::Right((bytes, _)) => Some(bytes),
    };
    let get = incoming
        .as_ref()
        .is_some_and(|bytes| bytes.starts_with(b"GET "));
    if get {
        if revoked {
            feature_cancel.cancel();
        }
        let head = socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
            .await;
        get_ready.cancel();
        if !revoked {
            head.unwrap();
            feature_cancel.cancelled().await;
            socket.write_all(b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/resources/list_changed\"}\n\n").await.unwrap();
        }
    } else if let Some(bytes) = incoming {
        assert!(String::from_utf8_lossy(&bytes).contains("resources/list"));
        get_ready.cancelled().await;
        if !revoked {
            let body = success(3, serde_json::json!({"resources":[]}));
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\n{JSON}Content-Length: {}\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(&body).await.unwrap();
        }
    }
    let mut remaining = 0;
    loop {
        match socket.read(&mut [0; 512]).await {
            Ok(0) => break,
            Ok(count) => {
                remaining += count;
                assert!(remaining <= 2048);
            }
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => panic!("{error}"),
        }
    }
    get
}
