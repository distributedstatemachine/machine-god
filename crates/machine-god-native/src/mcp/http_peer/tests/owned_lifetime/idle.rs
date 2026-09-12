use super::*;

const NOTICE: &[u8] = b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}";

#[test]
fn dropped_partial_observation_preserves_both_listener_families() {
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
            let (prefix_sent, prefix_received) = tokio::sync::oneshot::channel();
            let (tail_allowed, tail_ready) = tokio::sync::oneshot::channel();
            let server = async {
                let mut events = listener_server(&listener, deprecated).await;
                events
                    .write_all(b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/")
                    .await
                    .unwrap();
                prefix_sent.send(()).unwrap();
                tail_ready.await.unwrap();
                events.write_all(b"list_changed\"}\n\n").await.unwrap();
                assert_eq!(events.read(&mut [0]).await.unwrap(), 0);
            };
            let client = async {
                let mut peer =
                    McpHttpPeer::connect(selected, CancellationToken::new(), clock.at(1000))
                        .await
                        .unwrap();
                peer.start_listener(clock.at(1000)).await.unwrap();
                prefix_received.await.unwrap();
                let mut observation = Box::pin(peer.next_notification(clock.at(2000)));
                assert!(futures_util::poll!(&mut observation).is_pending());
                tokio::task::yield_now().await;
                assert!(futures_util::poll!(&mut observation).is_pending());
                drop(observation); // The manual UI won; no stream ownership was surrendered.
                assert!(peer.readiness().is_ready());
                assert!(!peer.completion().is_complete());
                tail_allowed.send(()).unwrap();
                let frame = peer.next_notification(clock.at(3000)).await.unwrap();
                assert_eq!(frame.bytes.as_ref(), NOTICE);
                assert!(peer.take_notification().is_none());
                peer.close();
                assert!(peer.completion().is_complete());
            };
            tokio::time::timeout(Duration::from_secs(3), join(client, server))
                .await
                .unwrap();
        }
    });
}

#[test]
fn dropping_observation_after_owner_cutoff_retires_listener() {
    executor().block_on(async {
        for expiry in [false, true] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let clock = TestClock::new();
            let mut selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            selected.clock = clock.clone();
            selected.lifetime = McpPeerLifetime::Until(clock.at(2000));
            let cancellation = CancellationToken::new();
            let server = async {
                let mut events = listener_server(&listener, false).await;
                assert_eq!(events.read(&mut [0]).await.unwrap(), 0);
            };
            let client = async {
                let mut peer = McpHttpPeer::connect(selected, cancellation.clone(), clock.at(1000))
                    .await
                    .unwrap();
                peer.start_listener(clock.at(1000)).await.unwrap();
                let mut observation = Box::pin(peer.next_notification(clock.at(3000)));
                assert!(futures_util::poll!(&mut observation).is_pending());
                if expiry {
                    clock.advance(2000);
                } else {
                    cancellation.cancel();
                }
                drop(observation);
                assert!(peer.closed);
                assert!(peer.completion().is_complete());
            };
            tokio::time::timeout(Duration::from_secs(3), join(client, server))
                .await
                .unwrap();
        }
    });
}

#[test]
fn dropped_reconnect_retains_delay_and_single_get_acquisition() {
    executor().block_on(reconnect_after_drop(false));
}

#[test]
fn dropped_reconnect_cannot_renew_original_acquisition_deadline() {
    executor().block_on(reconnect_after_drop(true));
}

async fn reconnect_after_drop(expire: bool) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let clock = TestClock::new();
    let mut selected = options(
        listener.local_addr().unwrap(),
        TransportKind::StreamableHttp,
    );
    selected.clock = clock.clone();
    selected.lifetime = McpPeerLifetime::OwnerControlled;
    let (get_seen, get_ready) = tokio::sync::oneshot::channel();
    let (finish_allowed, finish_ready) = tokio::sync::oneshot::channel();
    let server = reconnect_server(&listener, get_seen, finish_ready, expire);
    let client = async {
        let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), clock.at(1000))
            .await
            .unwrap();
        peer.start_listener(clock.at(1000)).await.unwrap();
        assert_eq!(
            peer.next_notification(clock.at(1000))
                .await
                .unwrap()
                .bytes
                .as_ref(),
            NOTICE
        );
        let (reader, event) = peer.listener.take().unwrap().await;
        assert!(event.unwrap().is_none());
        peer.listener = Some(Box::pin(async move { (reader, Ok(None)) }));
        let mut observation = Box::pin(peer.next_notification(clock.at(1000)));
        assert!(futures_util::poll!(&mut observation).is_pending());
        drop(observation); // Retain the exact 100 ms retry delay, before any GET write.
        assert!(peer.readiness().is_ready());
        assert_eq!(peer.listener_reconnects, 1);
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        clock.advance(100);
        let mut observation = Box::pin(peer.next_notification(clock.at(2000)));
        let mut get_ready = Box::pin(get_ready);
        std::future::poll_fn(|cx| {
            assert!(observation.as_mut().poll(cx).is_pending());
            get_ready.as_mut().poll(cx)
        })
        .await
        .unwrap();
        drop(observation); // Retain partial response head and original 1,000 ms bound.
        assert!(peer.readiness().is_ready());
        if expire {
            clock.advance(1000);
        }
        finish_allowed.send(()).unwrap();
        let result = peer.next_notification(clock.at(3000)).await;
        if expire {
            assert!(matches!(
                result,
                Err(McpHttpPeerError::Deadline
                    | McpHttpPeerError::Transport(McpHttpError::Deadline))
            ));
            assert!(peer.closed);
        } else {
            assert_eq!(result.unwrap().bytes.as_ref(), NOTICE);
            assert!(peer.readiness().is_ready());
            peer.close();
        }
        assert!(peer.completion().is_complete());
    };
    tokio::time::timeout(Duration::from_secs(3), join(client, server))
        .await
        .unwrap();
    assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
}

async fn reconnect_server(
    listener: &TcpListener,
    get_seen: tokio::sync::oneshot::Sender<()>,
    finish_ready: tokio::sync::oneshot::Receiver<()>,
    expire: bool,
) {
    let mut events = listener_server(listener, false).await;
    events
        .write_all(b"id: kept\nretry: 100\ndata: ")
        .await
        .unwrap();
    events.write_all(NOTICE).await.unwrap();
    events.write_all(b"\n\n").await.unwrap();
    events.shutdown().await.unwrap();
    let (mut resumed, _) = listener.accept().await.unwrap();
    let bytes = request(&mut resumed).await;
    assert!(bytes.starts_with(b"GET "));
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .to_lowercase()
            .contains("last-event-id: kept\r\n")
    );
    // Leave the GET midway through response-head parsing across observer drop.
    resumed
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/")
        .await
        .unwrap();
    get_seen.send(()).unwrap();
    finish_ready.await.unwrap();
    if !expire {
        resumed
            .write_all(b"event-stream\r\n\r\ndata: ")
            .await
            .unwrap();
        resumed.write_all(NOTICE).await.unwrap();
        resumed.write_all(b"\n\n").await.unwrap();
    }
    assert_eq!(resumed.read(&mut [0]).await.unwrap(), 0);
}

#[test]
fn malformed_event_after_dropped_observer_still_retires_peer() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let clock = TestClock::new();
        let mut selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        selected.clock = clock.clone();
        selected.lifetime = McpPeerLifetime::OwnerControlled;
        let (send, receive) = tokio::sync::oneshot::channel();
        let server = async {
            let mut events = listener_server(&listener, false).await;
            receive.await.unwrap();
            events.write_all(b"data: invalid-json\n\n").await.unwrap();
            assert_eq!(events.read(&mut [0]).await.unwrap(), 0);
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), clock.at(1000))
                .await
                .unwrap();
            peer.start_listener(clock.at(1000)).await.unwrap();
            let mut observation = Box::pin(peer.next_notification(clock.at(1000)));
            assert!(futures_util::poll!(&mut observation).is_pending());
            drop(observation);
            send.send(()).unwrap();
            assert!(matches!(
                peer.next_notification(clock.at(2000)).await,
                Err(McpHttpPeerError::Protocol)
            ));
            assert!(peer.closed);
            assert!(peer.completion().is_complete());
        };
        tokio::time::timeout(Duration::from_secs(3), join(client, server))
            .await
            .unwrap();
    });
}
