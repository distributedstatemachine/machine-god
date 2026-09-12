use super::*;
use crate::mcp::{
    catalog_refresh::{
        McpCatalogRefresh, McpRefreshGeneration, McpRefreshNotification, McpSubscriptionFilters,
    },
    lifetime::McpPeerLifetime,
};

const ACK: &[u8] = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/subscriptions/acknowledged\",\"params\":{\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":2},\"notifications\":{\"resourcesListChanged\":true}}}\n\n";
const INVALIDATION: &[u8] = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/resources/list_changed\",\"params\":{\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":2}}}\n\n";
const QUEUED_INVALIDATION: &[u8] = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/resources/list_changed\",\"params\":{\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":2},\"marker\":\"ordinary\"}}\n\n";

struct SelectedClock {
    origin: Instant,
    elapsed: std::sync::atomic::AtomicU64,
}
impl SelectedClock {
    fn at(&self, millis: u64) -> Instant {
        self.origin + Duration::from_millis(millis)
    }
}
impl McpHttpClock for SelectedClock {
    fn now(&self) -> Instant {
        self.at(self.elapsed.load(std::sync::atomic::Ordering::Acquire))
    }
    fn sleep_until(&self, _deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

async fn listen_head(listener: &TcpListener) -> tokio::net::TcpStream {
    let mut socket = listener.accept().await.unwrap().0;
    let received = request(&mut socket).await;
    let text = String::from_utf8(received).unwrap();
    assert!(text.starts_with("POST /mcp HTTP/1.1"));
    assert!(text.contains("mcp-method: subscriptions/listen\r\n"));
    let body: serde_json::Value =
        serde_json::from_str(text.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["id"], 2);
    assert_eq!(body["method"], "subscriptions/listen");
    assert_eq!(
        body["params"]["notifications"],
        serde_json::json!({"resourcesListChanged":true})
    );
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    socket.flush().await.unwrap();
    socket
}

async fn catalog_with_notification(listener: &TcpListener, id: i64) {
    let mut body = QUEUED_INVALIDATION.to_vec();
    body.extend_from_slice(b"data: ");
    body.extend_from_slice(&success(id, serde_json::json!({"tools":[]})));
    body.extend_from_slice(b"\n\n");
    accept_reply(listener, 200, SSE, &body).await;
}

#[test]
fn ordinary_stream_notifications_drain_before_listener_reads_and_after_its_final_response() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(listener.local_addr().unwrap(), TransportKind::StreamableHttp);
        let partial = CancellationToken::new();
        let catalog_allowed = CancellationToken::new();
        let listener_allowed = CancellationToken::new();
        let server = async {
            accept_reply(&listener, 200, JSON, &modern(1)).await;
            let mut socket = listen_head(&listener).await;
            socket.write_all(ACK).await.unwrap();
            let split = INVALIDATION.len() / 2;
            socket.write_all(&INVALIDATION[..split]).await.unwrap();
            partial.cancel();
            catalog_allowed.cancelled().await;
            catalog_with_notification(&listener, 3).await;
            listener_allowed.cancelled().await;
            socket.write_all(&INVALIDATION[split..]).await.unwrap();
            socket.write_all(b"data: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"resultType\":\"complete\",\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":2}}}\n\n").await.unwrap();
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
            catalog_with_notification(&listener, 4).await;
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline()).await.unwrap();
            let filters = McpSubscriptionFilters::new(peer.capabilities(), &[]).unwrap();
            peer.start_subscription(&filters, deadline()).await.unwrap();
            let generation = McpRefreshGeneration::new();
            let mut policy = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
            policy.install_subscription(&generation, 2, filters).unwrap();
            let ack = peer.poll_subscription(deadline()).await.unwrap().unwrap();
            assert_eq!(policy.observe(&generation, ack.envelope()).unwrap(), McpRefreshNotification::Acknowledged);
            partial.cancelled().await;
            {
                let mut pending = Box::pin(peer.poll_subscription(deadline()));
                assert!(futures_util::poll!(&mut pending).is_pending());
            }
            catalog_allowed.cancel();
            peer.catalog(McpCatalogKind::Tools, McpCatalogLimits::default(), Instant::now(), deadline()).await.unwrap();
            let queued = {
                let mut poll = Box::pin(peer.poll_subscription(deadline()));
                let std::task::Poll::Ready(Ok(Some(frame))) = futures_util::poll!(&mut poll) else {
                    panic!("ordinary queue must precede the blocked listener read")
                };
                frame
            };
            assert_eq!(queued.envelope().params().unwrap()["marker"], "ordinary");
            assert_eq!(policy.observe(&generation, queued.envelope()).unwrap(), McpRefreshNotification::AllResourceReads);
            assert!(peer.notifications.is_empty());
            assert_eq!(peer.notification_bytes, 0);
            assert_eq!(peer.active_subscription(), Some(RpcId::Integer(2)));
            listener_allowed.cancel();
            let frame = peer.poll_subscription(deadline()).await.unwrap().unwrap();
            assert!(frame.envelope().params().unwrap().get("marker").is_none());
            assert_eq!(policy.observe(&generation, frame.envelope()).unwrap(), McpRefreshNotification::AllResourceReads);
            assert!(peer.poll_subscription(deadline()).await.unwrap().is_none());
            assert!(peer.active_subscription().is_none());
            peer.catalog(McpCatalogKind::Tools, McpCatalogLimits::default(), Instant::now(), deadline()).await.unwrap();
            let retained = peer.notification_bytes;
            assert!(retained > 0);
            assert!(peer.poll_subscription(Instant::now()).await.unwrap().is_none());
            assert_eq!(peer.notification_bytes, retained);
            let queued = peer.poll_subscription(deadline()).await.unwrap().unwrap();
            assert_eq!(policy.observe(&generation, queued.envelope()).unwrap(), McpRefreshNotification::AllResourceReads);
            assert_eq!(peer.notification_bytes, 0);
            assert!(peer.poll_subscription(deadline()).await.unwrap().is_none());
            assert!(peer.readiness().is_ready());
        };
        tokio::time::timeout(Duration::from_secs(3), join(client, server)).await.unwrap();
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
    });
}

#[test]
fn subscription_ack_partial_read_abandonment_and_timeout_keep_original_stream() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        selected.lifetime = McpPeerLifetime::OwnerControlled;
        let sent_partial = CancellationToken::new();
        let resume = CancellationToken::new();
        let server = async {
            accept_reply(&listener, 200, JSON, &modern(1)).await;
            let mut socket = listen_head(&listener).await;
            socket.write_all(ACK).await.unwrap();
            let split = INVALIDATION.len() / 2;
            socket.write_all(&INVALIDATION[..split]).await.unwrap();
            sent_partial.cancel();
            resume.cancelled().await;
            // An independent ordinary catalog connection does not replace the
            // subscription stream, even while that stream retains a half-line.
            accept_reply(
                &listener,
                200,
                JSON,
                &success(3, serde_json::json!({"tools":[]})),
            )
            .await;
            socket.write_all(&INVALIDATION[split..]).await.unwrap();
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let filters = McpSubscriptionFilters::new(peer.capabilities(), &[]).unwrap();
            let unpolled = peer.start_subscription(&filters, deadline());
            drop(unpolled);
            assert!(peer.active_subscription().is_none());
            assert_eq!(peer.next_id, Some(2));
            let id = peer.start_subscription(&filters, deadline()).await.unwrap();
            assert_eq!(id, RpcId::Integer(2));
            let generation = McpRefreshGeneration::new();
            let mut policy = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
            policy
                .install_subscription(&generation, 2, filters)
                .unwrap();
            let ack = peer.poll_subscription(deadline()).await.unwrap().unwrap();
            assert_eq!(
                policy.observe(&generation, ack.envelope()).unwrap(),
                McpRefreshNotification::Acknowledged
            );
            sent_partial.cancelled().await;
            {
                let mut pending = Box::pin(peer.poll_subscription(deadline()));
                assert!(futures_util::poll!(&mut pending).is_pending());
            }
            assert_eq!(peer.active_subscription(), Some(id.clone()));
            assert!(
                peer.poll_subscription(Instant::now() + Duration::from_millis(10))
                    .await
                    .unwrap()
                    .is_none()
            );
            assert_eq!(peer.active_subscription(), Some(id));
            resume.cancel();
            peer.catalog(
                McpCatalogKind::Tools,
                McpCatalogLimits::default(),
                Instant::now(),
                deadline(),
            )
            .await
            .unwrap();
            let frame = peer.poll_subscription(deadline()).await.unwrap().unwrap();
            assert_eq!(
                policy.observe(&generation, frame.envelope()).unwrap(),
                McpRefreshNotification::AllResourceReads
            );
            let observation = peer.completion();
            peer.close_subscription();
            assert!(peer.active_subscription().is_none());
            assert!(peer.readiness().is_ready());
            peer.close();
            assert!(observation.is_complete());
        };
        tokio::time::timeout(Duration::from_secs(3), join(client, server))
            .await
            .unwrap();
    });
}

#[test]
fn subscription_final_response_and_malformed_stream_close_only_listener() {
    executor().block_on(async {
        for (body, valid) in [
            (b"data: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"resultType\":\"complete\",\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":2}}}\n\n".as_slice(), true),
            (b"data: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"resultType\":\"complete\",\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":3}}}\n\n", false),
            (b"data: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{}}\n\n", false),
            (b"data: {\"jsonrpc\":\"2.0\",\"id\":2,\"error\":{\"code\":-32601,\"message\":\"unsupported\"}}\n\n", false),
            (b"data: {\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"roots/list\"}\n\n", false),
            (b"data: {\"jsonrpc\":", false),
            (b"", false),
        ] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let selected = options(listener.local_addr().unwrap(), TransportKind::StreamableHttp);
            let server = async {
                accept_reply(&listener, 200, JSON, &modern(1)).await;
                let mut socket = listen_head(&listener).await;
                socket.write_all(body).await.unwrap();
            };
            let client = async {
                let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline()).await.unwrap();
                let filters = McpSubscriptionFilters::new(peer.capabilities(), &[]).unwrap();
                peer.start_subscription(&filters, deadline()).await.unwrap();
                let result = peer.poll_subscription(deadline()).await;
                assert_eq!(result.is_ok(), valid);
                assert!(peer.active_subscription().is_none());
                assert!(peer.readiness().is_ready());
                let observation = peer.completion();
                peer.close();
                assert!(observation.is_complete());
            };
            tokio::time::timeout(Duration::from_secs(3), join(client, server)).await.unwrap();
            assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        }
    });
}

#[test]
fn subscription_outlives_start_deadline_but_obeys_selected_owner_and_pinned_cap() {
    executor().block_on(async {
        for owner_expiry in [None, Some(2000)] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let clock = Arc::new(SelectedClock {
                origin: Instant::now(),
                elapsed: std::sync::atomic::AtomicU64::new(0),
            });
            let mut selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            selected.clock = clock.clone();
            selected.lifetime = owner_expiry.map_or(McpPeerLifetime::OwnerControlled, |at| {
                McpPeerLifetime::Until(clock.at(at))
            });
            let server = async {
                accept_reply(&listener, 200, JSON, &modern(1)).await;
                let mut socket = listen_head(&listener).await;
                socket.write_all(ACK).await.unwrap();
                let mut byte = [0];
                assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
            };
            let client = async {
                let mut peer =
                    McpHttpPeer::connect(selected, CancellationToken::new(), clock.at(500))
                        .await
                        .unwrap();
                let filters = McpSubscriptionFilters::new(peer.capabilities(), &[]).unwrap();
                peer.start_subscription(&filters, clock.at(500))
                    .await
                    .unwrap();
                clock
                    .elapsed
                    .store(1000, std::sync::atomic::Ordering::Release);
                assert!(
                    peer.poll_subscription(clock.at(1500))
                        .await
                        .unwrap()
                        .is_some()
                );
                let expiry = owner_expiry.unwrap_or(u64::from(u32::MAX));
                clock
                    .elapsed
                    .store(expiry, std::sync::atomic::Ordering::Release);
                assert!(matches!(
                    peer.poll_subscription(clock.at(expiry + 1)).await,
                    Err(McpHttpPeerError::Deadline)
                ));
                assert!(peer.active_subscription().is_none());
                assert_eq!(peer.readiness().is_ready(), owner_expiry.is_none());
                let completion = peer.completion();
                peer.close();
                assert!(completion.is_complete());
            };
            tokio::time::timeout(Duration::from_secs(3), join(client, server))
                .await
                .unwrap();
        }
    });
}

#[test]
fn subscription_notification_frame_budget_is_independent_of_application_limit() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(listener.local_addr().unwrap(), TransportKind::StreamableHttp);
        let server = async {
            accept_reply(&listener, 200, JSON, &modern(1)).await;
            let mut socket = listen_head(&listener).await;
            let event = format!("data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/unknown\",\"params\":{{\"payload\":\"{}\"}}}}\n\n", "x".repeat(64 * 1024));
            socket.write_all(event.as_bytes()).await.unwrap();
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline()).await.unwrap();
            let filters = McpSubscriptionFilters::new(peer.capabilities(), &[]).unwrap();
            peer.start_subscription(&filters, deadline()).await.unwrap();
            assert!(peer.poll_subscription(deadline()).await.is_err());
            assert!(peer.active_subscription().is_none());
            assert!(peer.readiness().is_ready());
        };
        tokio::time::timeout(Duration::from_secs(3), join(client, server)).await.unwrap();
    });
}

#[test]
fn subscription_event_quota_is_cumulative_across_observations() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let server = async {
            accept_reply(&listener, 200, JSON, &modern(1)).await;
            let mut socket = listen_head(&listener).await;
            let event = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/unknown\"}\n\n";
            socket.write_all(&event.repeat(1025)).await.unwrap();
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let filters = McpSubscriptionFilters::new(peer.capabilities(), &[]).unwrap();
            peer.start_subscription(&filters, deadline()).await.unwrap();
            for _ in 0..1024 {
                assert!(peer.poll_subscription(deadline()).await.unwrap().is_some());
            }
            assert!(peer.poll_subscription(deadline()).await.is_err());
            assert!(peer.active_subscription().is_none());
            assert!(peer.readiness().is_ready());
        };
        tokio::time::timeout(Duration::from_secs(3), join(client, server))
            .await
            .unwrap();
    });
}
