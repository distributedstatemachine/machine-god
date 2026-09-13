use super::*;
use crate::mcp::{
    http::{McpHttpClock, tests::request},
    network::{McpResolverConfig, NativeMcpNetwork},
};
use futures_util::{FutureExt, future::join};
use std::net::Ipv4Addr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

impl McpHttpClock for LiveClock {
    fn now(&self) -> Instant {
        NativeMcpRuntimeClock::now(self)
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        NativeMcpRuntimeClock::sleep_until(self, deadline)
    }
}

fn configured(listener: &TcpListener) -> Fixture {
    let mut fixture = Fixture::new(&format!(
        r#"{{"mcp":{{"optional":{{"type":"http","url":"http://127.0.0.1:{}/mcp","startup_timeout_ms":5000}}}}}}"#,
        listener.local_addr().unwrap().port(),
    ));
    fixture.options.startup.network = Some(Arc::new(
        NativeMcpNetwork::new(
            McpResolverConfig::literal_only(),
            [17; 32],
            None,
            fixture.clock.clone(),
            CancellationToken::new(),
            2,
        )
        .unwrap(),
    ));
    fixture
}

async fn resource_discovery(listener: &TcpListener) {
    for (id, method, result) in [
        (
            1,
            "server/discover",
            json!({"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"resources":{}}}),
        ),
        (
            2,
            "resources/list",
            json!({"resultType":"complete","resources":[{"uri":"test://fixed","name":"fixed"}]}),
        ),
        (
            3,
            "resources/list",
            json!({"resultType":"complete","resources":[{"uri":"test://fixed","name":"fixed"}]}),
        ),
    ] {
        let (mut socket, _) = listener.accept().await.unwrap();
        let received = request(&mut socket).await;
        assert!(String::from_utf8_lossy(&received).contains(&format!("mcp-method: {method}")));
        let body = serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"result":result})).unwrap();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes()).await.unwrap();
        socket.write_all(&body).await.unwrap();
        socket.flush().await.unwrap();
    }
}

#[test]
fn human_feature_activates_optional_ask_server_without_switching_existing_pin() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let fixture = configured(&listener);
        let controller = fixture.controller();
        fixture.runtime.bind_controller(&controller).unwrap();
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let (_engine, _conversation, _turn, context) =
            addition::conversation(&fixture.runtime, "pinned-empty");
        let exact = fixture
            .runtime
            .contexts
            .snapshot_for_tool(&context)
            .unwrap();
        let pinned = fixture
            .runtime
            .for_turn(&exact.registry().unwrap())
            .unwrap()
            .unwrap();
        assert!(pinned.servers.is_empty());
        let human = fixture.runtime.human_command();
        let query = crate::mcp::control::tests::request("resource list optional");
        drop(human.feature(&query, CancellationToken::new()));
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(human.feature(&query, cancelled).await.is_err());
        assert!(listener.accept().now_or_never().is_none());
        let server = resource_discovery(&listener);
        let client = async {
            assert!(matches!(
                human
                    .feature(&query, CancellationToken::new())
                    .await
                    .unwrap()
                    .reply(),
                crate::mcp::control::McpFeatureReply::Catalog(_)
            ));
            assert!(
                fixture
                    .runtime
                    .snapshot_for_turn(context, CancellationToken::new())
                    .await
                    .unwrap()
                    .tools()
                    .is_empty()
            );
            let selected = fixture
                .runtime
                .for_turn(&exact.registry().unwrap())
                .unwrap()
                .unwrap();
            assert!(Arc::ptr_eq(&selected, &pinned));
            // Completed deferred discovery never loads a later saved edit.
            fixture.seed("invalid");
            assert!(
                human
                    .feature(&query, CancellationToken::new())
                    .await
                    .is_ok()
            );
        };
        join(client, server).await;
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn human_deferred_demand_rejects_saved_edit_before_any_connection() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let fixture = configured(&listener);
        let controller = fixture.controller();
        fixture.runtime.bind_controller(&controller).unwrap();
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        fixture.seed(r#"{"mcp":{}}"#);
        let human = fixture.runtime.human_command();
        let query = crate::mcp::control::tests::request("resource list optional");
        assert!(
            human
                .feature(&query, CancellationToken::new())
                .await
                .is_err()
        );
        assert!(listener.accept().now_or_never().is_none());
        assert!(
            fixture
                .runtime
                .state
                .lock()
                .unwrap()
                .active
                .as_ref()
                .unwrap()
                .servers
                .is_empty()
        );
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn actual_turn_catalog_and_feature_waiters_share_discovery_without_cancelled_pins() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let fixture = configured(&listener);
        let controller = fixture.controller();
        fixture.runtime.bind_controller(&controller).unwrap();
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        assert!(listener.accept().now_or_never().is_none());
        let (_e1, _c1, _t1, first) = addition::conversation(&fixture.runtime, "cancelled");
        let (_e2, _c2, _t2, second) = addition::conversation(&fixture.runtime, "feature");
        let (_e3, _c3, third_turn, third) = addition::conversation(&fixture.runtime, "retired");
        let cancellation = CancellationToken::new();
        let waits = async {
            let query = crate::mcp::control::tests::request("resource list absent");
            let ((first, second), third) = join(
                join(
                    fixture
                        .runtime
                        .snapshot_for_turn(first, cancellation.clone()),
                    fixture
                        .runtime
                        .feature_for_turn(second, &query, CancellationToken::new()),
                ),
                fixture
                    .runtime
                    .snapshot_for_turn(third, CancellationToken::new()),
            )
            .await;
            assert!(first.is_err());
            // Feature selection is after shared discovery and publication pinning.
            assert!(second.is_err());
            assert!(third.is_err());
        };
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let received = request(&mut socket).await;
            assert!(String::from_utf8_lossy(&received).contains("server/discover"));
            cancellation.cancel();
            assert!(third_turn.handle().cancel());
            let body = br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{}}}"#;
            socket.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()
            ).as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
            socket.flush().await.unwrap();
            socket
        };
        let ((), mut socket) = join(waits, server).await;
        assert_eq!(fixture.runtime.state.lock().unwrap().turns.len(), 1);
        assert_eq!(
            fixture
                .runtime
                .state
                .lock()
                .unwrap()
                .active
                .as_ref()
                .unwrap()
                .servers
                .len(),
            1
        );
        fixture.seed("invalid");
        let (_e4, _c4, _t4, fourth) = addition::conversation(&fixture.runtime, "completed");
        fixture
            .runtime
            .snapshot_for_turn(fourth, CancellationToken::new())
            .await
            .unwrap();
        assert!(listener.accept().now_or_never().is_none());
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
        let mut byte = [0];
        assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
    });
}
