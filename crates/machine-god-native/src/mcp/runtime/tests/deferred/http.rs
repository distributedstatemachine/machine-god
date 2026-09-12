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
