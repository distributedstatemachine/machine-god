use super::*;
use crate::mcp::{catalog_refresh::McpSubscriptionFilters, stdio::testing::Pipe};
use serde_json::json;

fn filters() -> McpSubscriptionFilters {
    let envelope = parse_envelope(br#"{"jsonrpc":"2.0","id":0,"result":{"resultType":"complete","capabilities":{"tools":{"listChanged":true}}}}"#, WireLimits::default()).unwrap();
    let capabilities = McpPeerCapabilities::admit(&envelope, ProtocolVersion::Modern).unwrap();
    McpSubscriptionFilters::new(capabilities, &[]).unwrap()
}

async fn start(peer: &mut McpStdioPeer, pipe: &Pipe, deadline: Instant) -> RpcId {
    let selected = filters();
    let mut pending = Box::pin(peer.start_subscription(&selected, deadline));
    assert!(futures_util::poll!(&mut pending).is_pending());
    let write = pipe.take_write().unwrap();
    let json = write.json();
    assert_eq!(json["method"], "subscriptions/listen");
    assert_eq!(
        json["params"]["notifications"],
        json!({"toolsListChanged":true})
    );
    assert_eq!(
        json["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        ProtocolVersion::Modern.as_str()
    );
    write.settle(Ok(()));
    pending.await.unwrap()
}

fn feed(pipe: &mut Pipe, value: serde_json::Value) {
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    pipe.feed(&bytes);
}

#[test]
fn subscription_ack_partial_idle_drop_and_timeout_preserve_identity() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let mut pipe = Pipe::new(&peer.connection);
        let selected = filters();
        drop(peer.start_subscription(&selected, timer.at(1000)));
        assert!(pipe.take_write().is_none());
        assert!(peer.active_subscription().is_none());
        let id = start(&mut peer, &pipe, timer.at(1000)).await;
        pipe.feed(br#"{"jsonrpc":"2.0","method":"notifications/subscriptions/ack","params":{"x":"#);
        let mut pending = Box::pin(peer.poll_subscription(timer.at(1000)));
        assert!(futures_util::poll!(&mut pending).is_pending());
        drop(pending);
        pipe.feed(b"1}}\n");
        let ack = peer
            .poll_subscription(timer.at(1000))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ack.method(), Some("notifications/subscriptions/ack"));
        let mut pending = Box::pin(peer.poll_subscription(timer.at(500)));
        assert!(futures_util::poll!(&mut pending).is_pending());
        timer.advance(500);
        assert!(pending.await.unwrap().is_none());
        assert_eq!(peer.active_subscription(), Some(id));
        assert!(peer.readiness().is_ready());
        peer.close();
    });
}

#[test]
fn cancellation_is_exact_and_late_final_does_not_steal_catalog_response() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let mut pipe = Pipe::new(&peer.connection);
        let RpcId::Integer(id) = start(&mut peer, &pipe, timer.at(1000)).await else {
            unreachable!()
        };
        drop(peer.close_subscription(timer.at(1000)));
        assert_eq!(peer.active_subscription(), Some(RpcId::Integer(id)));
        let mut close = Box::pin(peer.close_subscription(timer.at(1000)));
        assert!(futures_util::poll!(&mut close).is_pending());
        let write = pipe.take_write().unwrap();
        assert_eq!(
            write.json(),
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"Cancelled"}})
        );
        write.settle(Ok(()));
        close.await.unwrap(); // No server final is required.
        let mut catalog = Box::pin(peer.catalog(
            McpCatalogKind::Tools,
            McpCatalogLimits::default(),
            timer.origin,
            timer.at(1000),
        ));
        assert!(futures_util::poll!(&mut catalog).is_pending());
        let write = pipe.take_write().unwrap();
        let next = write.json()["id"].as_i64().unwrap();
        assert!(next > id);
        write.settle(Ok(()));
        feed(
            &mut pipe,
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-1,"message":"cancelled"}}),
        );
        feed(
            &mut pipe,
            json!({"jsonrpc":"2.0","id":next,"result":{"resultType":"complete","tools":[]}}),
        );
        catalog.await.unwrap();
        assert!(peer.active_subscription().is_none());
        assert!(peer.readiness().is_ready());
        peer.close();
    });
}

#[test]
fn active_error_ends_only_listener_and_wrong_id_retires_peer() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let mut pipe = Pipe::new(&peer.connection);
        let RpcId::Integer(id) = start(&mut peer, &pipe, timer.at(1000)).await else {
            unreachable!()
        };
        feed(
            &mut pipe,
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-1,"message":"unsupported"}}),
        );
        assert!(matches!(
            peer.poll_subscription(timer.at(1000)).await,
            Err(McpPeerError::InvalidResult)
        ));
        assert!(peer.active_subscription().is_none());
        assert!(peer.readiness().is_ready());
        start(&mut peer, &pipe, timer.at(1000)).await;
        feed(&mut pipe, json!({"jsonrpc":"2.0","id":999,"result":{}}));
        assert!(matches!(
            peer.poll_subscription(timer.at(1000)).await,
            Err(McpPeerError::Correlation)
        ));
        assert!(!peer.readiness().is_ready());
        peer.close();
    });
}

#[test]
fn sixty_four_retired_ids_allow_one_more_active_listener() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let mut pipe = Pipe::new(&peer.connection);
        let mut first = None;
        for _ in 0..64 {
            let id = start(&mut peer, &pipe, timer.at(1000)).await;
            first.get_or_insert(id);
            let mut close = Box::pin(peer.close_subscription(timer.at(1000)));
            assert!(futures_util::poll!(&mut close).is_pending());
            pipe.take_write().unwrap().settle(Ok(()));
            close.await.unwrap();
        }
        let active = start(&mut peer, &pipe, timer.at(1000)).await;
        assert!(matches!(
            peer.close_subscription(timer.at(1000)).await,
            Err(McpPeerError::Capacity)
        ));
        assert_eq!(peer.active_subscription(), Some(active));
        assert!(pipe.take_write().is_none());
        let RpcId::Integer(first) = first.unwrap() else {
            unreachable!()
        };
        feed(&mut pipe, json!({"jsonrpc":"2.0","id":first,"result":{}}));
        let mut poll = Box::pin(peer.poll_subscription(timer.at(500)));
        assert!(futures_util::poll!(&mut poll).is_pending());
        drop(poll);
        let mut close = Box::pin(peer.close_subscription(timer.at(1000)));
        assert!(futures_util::poll!(&mut close).is_pending());
        pipe.take_write().unwrap().settle(Ok(()));
        close.await.unwrap();
        assert!(peer.readiness().is_ready());
        peer.close();
    });
}

#[test]
fn listener_final_interleaves_with_inherited_reply_and_catalog() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let mut pipe = Pipe::new(&peer.connection);
        let RpcId::Integer(id) = start(&mut peer, &pipe, timer.at(1000)).await else {
            unreachable!()
        };
        feed(
            &mut pipe,
            json!({"jsonrpc":"2.0","id":"server-id","method":"unsupported"}),
        );
        feed(
            &mut pipe,
            json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}),
        );
        let mut observe = Box::pin(peer.poll_subscription(timer.at(800)));
        assert!(futures_util::poll!(&mut observe).is_pending());
        let reply = pipe.take_write().unwrap();
        assert_eq!(reply.json()["id"], "server-id");
        assert_eq!(reply.json()["error"]["code"], -32601);
        drop(observe);
        assert_eq!(peer.notifications.len(), 1);
        let mut catalog = Box::pin(peer.catalog(
            McpCatalogKind::Tools,
            McpCatalogLimits::default(),
            timer.origin,
            timer.at(1000),
        ));
        assert!(futures_util::poll!(&mut catalog).is_pending());
        assert!(pipe.take_write().is_none());
        feed(
            &mut pipe,
            json!({"jsonrpc":"2.0","id":id,"result":{"resultType":"complete","_meta":{"io.modelcontextprotocol/subscriptionId":id}}}),
        );
        assert!(futures_util::poll!(&mut catalog).is_pending());
        reply.settle(Ok(()));
        assert!(futures_util::poll!(&mut catalog).is_pending());
        let write = pipe.take_write().unwrap();
        let next = write.json()["id"].as_i64().unwrap();
        write.settle(Ok(()));
        feed(
            &mut pipe,
            json!({"jsonrpc":"2.0","id":next,"result":{"resultType":"complete","tools":[]}}),
        );
        catalog.await.unwrap();
        assert!(pipe.take_write().is_none());
        assert!(peer.active_subscription().is_none());
        assert_eq!(
            peer.poll_subscription(timer.at(1000))
                .await
                .unwrap()
                .unwrap()
                .method(),
            Some("notifications/tools/list_changed")
        );
        assert!(peer.readiness().is_ready());
        peer.close();
    });
}

#[test]
fn queued_notification_precedes_listener_error_and_staged_whitelist_drop_is_inert() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let pipe = Pipe::new(&peer.connection);
        let RpcId::Integer(id) = start(&mut peer, &pipe, timer.at(1000)).await else {
            unreachable!()
        };
        retain_notice(&mut peer);
        let error = parse_envelope(
            &serde_json::to_vec(
                &json!({"jsonrpc":"2.0","id":id,"error":{"code":-1,"message":"unsupported"}}),
            )
            .unwrap(),
            WireLimits::default(),
        )
        .unwrap();
        assert!(peer.subscription.consume(&error));
        assert!(
            peer.poll_subscription(timer.at(1000))
                .await
                .unwrap()
                .is_some()
        );
        assert!(matches!(
            peer.poll_subscription(timer.at(1000)).await,
            Err(McpPeerError::InvalidResult)
        ));
        assert!(peer.readiness().is_ready());

        let fixture = crate::mcp::submission::tests::Fixture::new();
        peer.admit_runtimes(vec![fixture.runtime.clone()]).unwrap();
        drop(peer.prepare_runtime_set(Vec::new()).unwrap());
        let old = peer.prepare_runtime_set(Vec::new()).unwrap().commit();
        assert_eq!(old.len(), 1);
        assert!(Arc::ptr_eq(&old[0], &fixture.runtime));
        drop(old);
        assert!(
            peer.prepare_runtime_set(Vec::new())
                .unwrap()
                .commit()
                .is_empty()
        );
        peer.close();
    });
}

#[test]
fn typed_listen_rejects_empty_filters_bad_ids_and_raw_allowlist_bypass() {
    use crate::mcp::stdio::McpStdioControl;
    let empty = McpSubscriptionFilters::new(McpPeerCapabilities::default(), &[]).unwrap();
    assert!(
        McpStdioControl::subscription(&RpcId::Integer(1), &empty, ProtocolVersion::Modern).is_err()
    );
    for id in [RpcId::Integer(-1), RpcId::String("1".into()), RpcId::Null] {
        assert!(McpStdioControl::subscription(&id, &filters(), ProtocolVersion::Modern).is_err());
    }
    assert!(McpStdioControl::discovery(br#"{"jsonrpc":"2.0","id":1,"method":"subscriptions/listen","params":{"notifications":{"toolsListChanged":true}}}"#).is_err());
}

#[test]
fn abandoned_cancel_write_and_owner_cutoff_retire_connection() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let pipe = Pipe::new(&peer.connection);
        start(&mut peer, &pipe, timer.at(1000)).await;
        let mut close = Box::pin(peer.close_subscription(timer.at(1000)));
        assert!(futures_util::poll!(&mut close).is_pending());
        let write = pipe.take_write().unwrap();
        drop(close);
        assert!(!peer.readiness().is_ready());
        drop(write); // Receipt double: no claim of real partial pipe coverage.
        peer.close();

        let mut peer = inert(timer.clone());
        let pipe = Pipe::new(&peer.connection);
        start(&mut peer, &pipe, timer.at(1000)).await;
        peer.lifetime = McpPeerLifetime::Until(timer.at(500));
        let mut observe = Box::pin(peer.poll_subscription(timer.at(1000)));
        assert!(futures_util::poll!(&mut observe).is_pending());
        timer.advance(500);
        assert!(matches!(observe.await, Err(McpPeerError::Deadline)));
        assert!(!peer.readiness().is_ready());
        peer.close();
    });
}

#[test]
fn malformed_listener_success_and_cancel_receipt_failure_are_not_idle_timeouts() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let mut pipe = Pipe::new(&peer.connection);
        let RpcId::Integer(id) = start(&mut peer, &pipe, timer.at(1000)).await else {
            unreachable!()
        };
        feed(
            &mut pipe,
            json!({"jsonrpc":"2.0","id":id,"result":{"resultType":"complete"}}),
        );
        assert!(matches!(
            peer.poll_subscription(timer.at(1000)).await,
            Err(McpPeerError::InvalidResult)
        ));
        assert!(peer.readiness().is_ready());
        start(&mut peer, &pipe, timer.at(1000)).await;
        let mut close = Box::pin(peer.close_subscription(timer.at(1000)));
        assert!(futures_util::poll!(&mut close).is_pending());
        pipe.take_write()
            .unwrap()
            .settle(Err(McpStdioError::Process));
        assert!(matches!(
            close.await,
            Err(McpPeerError::Transport(McpStdioError::Process))
        ));
        assert!(!peer.readiness().is_ready());
        peer.close();
    });
}
