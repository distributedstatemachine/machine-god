//! Actual startup ACK and issued-lease ownership, without process fixtures.
use super::*;
use crate::mcp::{protocol::RpcId, runtime::NativeMcpOwnedPeer};
use refresh::{AuthClock, authenticated};
use tokio::net::TcpStream;

fn filters(tools: bool, features: bool) -> serde_json::Value {
    let mut filters = json!({});
    if tools {
        filters["toolsListChanged"] = json!(true);
    }
    if features {
        filters["resourcesListChanged"] = json!(true);
        filters["promptsListChanged"] = json!(true);
    }
    filters
}

async fn discovery(listener: &TcpListener, tools: bool, features: bool) {
    let mut capabilities = json!({});
    if tools {
        capabilities["tools"] = json!({"listChanged":true});
    }
    if features {
        capabilities["resources"] = json!({"listChanged":true});
        capabilities["prompts"] = json!({"listChanged":true});
    }
    let bytes = serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"result":{
        "resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":capabilities,
    }}))
    .unwrap();
    reply(listener, &bytes).await;
}

async fn listen(listener: &TcpListener, tools: bool, features: bool) -> (TcpStream, i64, Vec<u8>) {
    if tools {
        let received = reply(
            listener,
            br#"{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","tools":[]}}"#,
        )
        .await;
        assert!(String::from_utf8_lossy(&received).contains("mcp-method: tools/list\r\n"));
    }
    let id = if tools { 3 } else { 2 };
    let (mut socket, _) = listener.accept().await.unwrap();
    let received = request(&mut socket).await;
    let text = String::from_utf8_lossy(&received);
    assert!(text.starts_with("POST /mcp HTTP/1.1\r\n"));
    assert!(text.contains("mcp-method: subscriptions/listen\r\n"));
    let value: serde_json::Value =
        serde_json::from_str(text.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(value["id"], id);
    assert_eq!(value["method"], "subscriptions/listen");
    assert_eq!(value["params"]["notifications"], filters(tools, features));
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    socket.flush().await.unwrap();
    (socket, id, received)
}

async fn acknowledge(socket: &mut TcpStream, id: i64, filters: serde_json::Value) {
    let value = json!({"jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged","params":{
        "_meta":{"io.modelcontextprotocol/subscriptionId":id},"notifications":filters,
    }});
    socket
        .write_all(format!("data: {value}\n\n").as_bytes())
        .await
        .unwrap();
    socket.flush().await.unwrap();
}

async fn closed(socket: &mut TcpStream) {
    let mut byte = [0];
    match socket.read(&mut byte).await {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) => {}
        other => panic!("startup must release the exact subscription socket: {other:?}"),
    }
}

#[test]
fn startup_requires_the_original_subscription_id_and_exact_filter_acknowledgement() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let startup = NativeMcpStartup::new(http_options(listener.local_addr().unwrap())).unwrap();
        let wrong_ack_sent = CancellationToken::new();
        let allow_ack = CancellationToken::new();
        let server = async {
            discovery(&listener, true, false).await;
            let (mut socket, id, _) = listen(&listener, true, false).await;
            acknowledge(&mut socket, id + 1, filters(true, false)).await;
            wrong_ack_sent.cancel();
            allow_ack.cancelled().await;
            acknowledge(&mut socket, id, filters(true, false)).await;
            closed(&mut socket).await;
        };
        let client = async {
            let build = Box::pin(
                startup.build_configured(NativeMcpStartupPhase::All, CancellationToken::new()),
            );
            let mut build = match select(build, Box::pin(wrong_ack_sent.cancelled())).await {
                Either::Right(((), build)) => build,
                Either::Left(_) => {
                    panic!("response head or foreign ACK cannot establish readiness")
                }
            };
            assert!(futures_util::poll!(&mut build).is_pending());
            allow_ack.cancel();
            let mut batch = build.await;
            assert!(batch.receipt().required_ready());
            assert_eq!(batch.servers[0].catalogs.len(), 1);
            let NativeMcpOwnedPeer::Http(peer) = &mut batch.servers[0].peer else {
                panic!("HTTP peer")
            };
            assert_eq!(peer.active_subscription(), Some(RpcId::Integer(3)));
            drop(batch);
        };
        join(client, server).await;
        assert!(startup.cleanup_observations().is_empty());
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
    });
}

#[test]
fn startup_rejects_a_correlated_ack_with_changed_filters_without_replay() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let startup = NativeMcpStartup::new(http_options(listener.local_addr().unwrap())).unwrap();
        let server = async {
            discovery(&listener, true, false).await;
            let (mut socket, id, _) = listen(&listener, true, false).await;
            acknowledge(&mut socket, id, filters(true, true)).await;
            closed(&mut socket).await;
        };
        let (batch, ()) = join(
            startup.build_configured(NativeMcpStartupPhase::All, CancellationToken::new()),
            server,
        )
        .await;
        assert!(!batch.receipt().required_ready());
        assert!(matches!(
            batch.receipt().servers[0].state,
            NativeMcpStartupState::Failed(_)
        ));
        assert!(batch.receipt().cleanup_complete());
        assert!(batch.servers().is_empty());
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
    });
}

#[test]
fn feature_only_startup_subscribes_without_eager_catalog_requests() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let startup = NativeMcpStartup::new(http_options(listener.local_addr().unwrap())).unwrap();
        let server = async {
            discovery(&listener, false, true).await;
            let (mut socket, id, _) = listen(&listener, false, true).await;
            acknowledge(&mut socket, id, filters(false, true)).await;
            closed(&mut socket).await;
        };
        let client = async {
            let mut batch = startup
                .build_configured(NativeMcpStartupPhase::All, CancellationToken::new())
                .await;
            assert!(batch.receipt().required_ready());
            assert!(batch.servers[0].catalogs.is_empty());
            let NativeMcpOwnedPeer::Http(peer) = &mut batch.servers[0].peer else {
                panic!("HTTP peer")
            };
            assert_eq!(peer.active_subscription(), Some(RpcId::Integer(2)));
            drop(batch);
        };
        join(client, server).await;
        assert!(startup.cleanup_observations().is_empty());
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
    });
}

#[test]
fn issued_authentication_expiry_closes_a_pending_partial_subscription_read() {
    for wall_only in [false, true] {
        run(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let clock = Arc::new(AuthClock::new());
            let (startup, _credentials, lease) =
                authenticated(listener.local_addr().unwrap(), clock.clone()).await;
            let partial_sent = CancellationToken::new();
            let server = async {
                discovery(&listener, true, false).await;
                let (mut socket, id, received) = listen(&listener, true, false).await;
                assert!(
                    String::from_utf8_lossy(&received).contains("authorization: Bearer old-secret")
                );
                acknowledge(&mut socket, id, filters(true, false)).await;
                socket
                    .write_all(b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_")
                    .await
                    .unwrap();
                socket.flush().await.unwrap();
                partial_sent.cancel();
                closed(&mut socket).await;
            };
            let client = async {
                let mut batch = startup
                    .build_configured(NativeMcpStartupPhase::All, CancellationToken::new())
                    .await;
                assert!(batch.receipt().required_ready());
                partial_sent.cancelled().await;
                let NativeMcpOwnedPeer::Http(peer) = &mut batch.servers[0].peer else {
                    panic!("HTTP peer")
                };
                let readiness = peer.readiness();
                let mut pending = Box::pin(peer.poll_subscription(deadline()));
                assert!(futures_util::poll!(&mut pending).is_pending());
                if wall_only {
                    clock.advance(Duration::ZERO, 120_000);
                } else {
                    clock.advance(Duration::from_secs(120), -120_000);
                }
                assert!(pending.await.is_err());
                assert!(lease.access_token().is_err());
                assert!(!lease.generation().is_cancelled());
                assert!(!readiness.is_ready());
                assert!(peer.active_subscription().is_none());
                assert!(startup.authentication_refresh_due().unwrap());
                drop(batch);
            };
            join(client, server).await;
            assert!(startup.cleanup_observations().is_empty());
            assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        });
    }
}

struct StartupClock(Arc<AuthClock>);
impl NativeMcpRuntimeClock for StartupClock {
    fn now(&self) -> Instant {
        McpHttpClock::now(&*self.0)
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        McpHttpClock::sleep_until(&*self.0, deadline)
    }
}

#[test]
fn subscription_ack_wait_retains_the_original_startup_attempt_deadline() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let clock = Arc::new(AuthClock::new());
        let mut selected = http_options(listener.local_addr().unwrap());
        selected.clock = Arc::new(StartupClock(clock.clone()));
        selected.catalog_epoch = clock.now();
        selected.peer_lifetime = McpPeerLifetime::OwnerControlled;
        selected.network = Some(Arc::new(
            NativeMcpNetwork::new(
                McpResolverConfig::literal_only(),
                [7; 32],
                None,
                clock.clone(),
                CancellationToken::new(),
                1,
            )
            .unwrap(),
        ));
        let startup = NativeMcpStartup::new(selected).unwrap();
        let server = async {
            discovery(&listener, true, false).await;
            clock.advance(Duration::from_secs(3), 0);
            let (mut socket, _, _) = listen(&listener, true, false).await;
            clock.advance(Duration::from_secs(2), 0);
            closed(&mut socket).await;
        };
        let (batch, ()) = join(
            startup.build_configured(NativeMcpStartupPhase::All, CancellationToken::new()),
            server,
        )
        .await;
        assert!(!batch.receipt().required_ready());
        assert!(batch.receipt().cleanup_complete());
        assert!(batch.servers().is_empty());
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
    });
}
