use super::*;

#[test]
fn resumed_post_stream_requires_fresh_evidence_for_a_second_get() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let server = async {
            legacy_start(&listener, "2025-11-25", "fresh-evidence").await;
            accept_reply(&listener, 200, SSE, b"id: once\n\n").await;
            let request = accept_reply(&listener, 200, SSE, b"").await;
            assert!(request.starts_with(b"GET "));
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            assert!(
                peer.catalog(
                    McpCatalogKind::Tools,
                    McpCatalogLimits::default(),
                    Instant::now(),
                    deadline()
                )
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
    });
}

#[test]
fn incomplete_initialization_cannot_use_unadmitted_session_to_resume() {
    executor().block_on(async {
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
                "Content-Type: text/event-stream\r\nMcp-Session-Id: not-yet-owned\r\n",
                b"id: unadmitted\n\n",
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
    });
}

struct FixedClock(Instant);
impl McpHttpClock for FixedClock {
    fn now(&self) -> Instant {
        self.0
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

#[test]
fn injected_clock_controls_peer_connector_write_and_response_deadlines() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let now = Instant::now();
        selected.clock = Arc::new(FixedClock(
            now.checked_sub(Duration::from_secs(60)).unwrap(),
        ));
        selected.lifetime = now.checked_sub(Duration::from_secs(20)).unwrap().into();
        let body = modern(1);
        let server = accept_reply(&listener, 200, JSON, &body);
        let client = async {
            let peer = McpHttpPeer::connect(
                selected,
                CancellationToken::new(),
                now.checked_sub(Duration::from_secs(30)).unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(peer.protocol().version, ProtocolVersion::Modern);
        };
        tokio::time::timeout(Duration::from_secs(1), join(client, server))
            .await
            .unwrap();
    });
}

#[test]
fn completed_operations_do_not_exhaust_event_or_owner_lifetime_budgets() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        selected.lifetime = (Instant::now() + Duration::from_secs(7 * 24 * 60 * 60)).into();
        let server = async {
            accept_reply(&listener, 200, JSON, &modern(1)).await;
            accept_reply(
                &listener,
                200,
                JSON,
                &success(2, serde_json::json!({"tools":[]})),
            )
            .await;
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            for _ in 0..4096 {
                routing::charge_event(&mut peer).unwrap();
            }
            assert!(matches!(
                routing::charge_event(&mut peer),
                Err(McpHttpPeerError::Limit)
            ));
            peer.listener_reconnects = 32;
            let notification = br#"{"jsonrpc":"2.0","method":"notifications/progress"}"#;
            peer.retain(McpHttpPeerFrame::parse(notification.as_slice().into()).unwrap())
                .unwrap();
            peer.catalog(
                McpCatalogKind::Tools,
                McpCatalogLimits::default(),
                Instant::now(),
                deadline(),
            )
            .await
            .unwrap();
            assert_eq!(peer.operation_events, 0);
            assert_eq!(peer.listener_reconnects, 0);
            assert_eq!(peer.take_notification().unwrap().bytes(), notification);
            routing::charge_event(&mut peer).unwrap();
            assert!(!peer.completion().is_complete());
        };
        join(client, server).await;
    });
}

#[test]
fn unsupported_listener_preserves_peer_but_expired_session_retires_it() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let server = async {
            legacy_start(&listener, "2025-11-25", "expires").await;
            accept_reply(&listener, 405, "", b"").await;
            accept_reply(&listener, 404, "", b"").await;
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            assert!(matches!(
                peer.start_listener(deadline()).await,
                Err(McpHttpPeerError::ListenerUnsupported)
            ));
            assert_eq!(peer.reserve_tool_id().unwrap(), RpcId::Integer(3));
            peer.discard_tool_id();
            assert!(matches!(
                peer.catalog(
                    McpCatalogKind::Tools,
                    McpCatalogLimits::default(),
                    Instant::now(),
                    deadline()
                )
                .await,
                Err(McpHttpPeerError::SessionExpired)
            ));
            assert!(peer.completion().is_complete());
            assert_eq!(
                peer.shutdown(deadline()).await,
                McpHttpSessionTeardown::NotAttempted
            );
            assert!(peer.completion().is_complete());
        };
        join(client, server).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn modern_sse_resume_hints_and_wrong_response_ids_cannot_trigger_reconnect() {
    executor().block_on(async {
        for bytes in [
            b"id: cursor\nretry: 0\ndata:\n\n".as_slice(),
            b"data: {\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{}}\n\n",
        ] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            let server = async {
                accept_reply(&listener, 200, JSON, &modern(1)).await;
                accept_reply(&listener, 200, SSE, bytes).await;
            };
            let client = async {
                let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                    .await
                    .unwrap();
                assert!(
                    peer.catalog(
                        McpCatalogKind::Tools,
                        McpCatalogLimits::default(),
                        Instant::now(),
                        deadline()
                    )
                    .await
                    .is_err()
                );
                assert!(peer.completion().is_complete());
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
fn notification_queue_is_bounded_and_does_not_reset_on_partial_drain() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let body = modern(1);
        let server = accept_reply(&listener, 200, JSON, &body);
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let notification = br#"{"jsonrpc":"2.0","method":"notifications/progress"}"#;
            for _ in 0..64 {
                peer.retain(McpHttpPeerFrame::parse(notification.as_slice().into()).unwrap())
                    .unwrap();
            }
            assert!(matches!(
                peer.retain(McpHttpPeerFrame::parse(notification.as_slice().into()).unwrap()),
                Err(McpHttpPeerError::Limit)
            ));
            peer.take_notification().unwrap();
            assert_eq!(peer.notification_bytes, notification.len() * 63);
            peer.retain(McpHttpPeerFrame::parse(notification.as_slice().into()).unwrap())
                .unwrap();
            peer.next_id = Some(i64::MAX);
            assert_eq!(peer.reserve_tool_id().unwrap(), RpcId::Integer(i64::MAX));
            peer.discard_tool_id();
            assert!(peer.reserve_tool_id().is_err());
        };
        join(client, server).await;
    });
}
