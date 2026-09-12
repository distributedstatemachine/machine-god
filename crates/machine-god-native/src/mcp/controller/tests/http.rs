use super::*;
use crate::mcp::{
    http::{McpHttpClock, tests::request},
    network::{McpResolverConfig, NativeMcpNetwork},
};
use futures_util::future::join;
use std::net::Ipv4Addr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

#[test]
fn abandoned_http_reload_keeps_cleanup_custody_until_explicit_settlement() {
    run(async {
        use futures_util::future::{Either, select};
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut fixture = Fixture::new();
        configure(&mut fixture, &listener);
        let controller = fixture.controller();
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let original = lock(&controller.inner.state).active.clone().unwrap();
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            socket
        };
        let (mut socket, abandoned) = match select(
            controller.reload(CancellationToken::new(), deadline()),
            Box::pin(server),
        )
        .await
        {
            Either::Right(value) => value,
            Either::Left(_) => panic!("reload cannot finish without discovery response"),
        };
        drop(abandoned);
        assert!(!original.cancellation.is_cancelled());
        let cancelled_cleanup = CancellationToken::new();
        cancelled_cleanup.cancel();
        let failure = controller
            .settle(deadline(), cancelled_cleanup)
            .await
            .unwrap_err();
        assert_eq!(failure.kind(), NativeMcpControllerError::Cancelled);
        assert!(!failure.cleanup_complete());
        assert!(lock(&controller.inner.state).running.is_some());
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

impl McpHttpClock for Clock {
    fn now(&self) -> Instant {
        NativeMcpRuntimeClock::now(self)
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        NativeMcpRuntimeClock::sleep_until(self, deadline)
    }
}
fn configure(fixture: &mut Fixture, listener: &TcpListener) {
    fixture.seed(&format!(r#"{{"mcp":{{"optional":{{"type":"http","url":"http://127.0.0.1:{}/mcp","startup_timeout_ms":5000}}}}}}"#, listener.local_addr().unwrap().port()));
    fixture.options.startup.network = Some(Arc::new(
        NativeMcpNetwork::new(
            McpResolverConfig::literal_only(),
            [9; 32],
            None,
            Arc::new(Clock::default()),
            CancellationToken::new(),
            2,
        )
        .unwrap(),
    ));
}
async fn discovery(socket: &mut TcpStream) {
    let body = br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"resources":{}}}}"#;
    socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    socket.write_all(body).await.unwrap();
    socket.flush().await.unwrap();
}

#[test]
fn deferred_http_waiters_coalesce_and_one_cancel_does_not_cancel_loader() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut fixture = Fixture::new();
        configure(&mut fixture, &listener);
        let controller = fixture.controller();
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let original = lock(&controller.inner.state).active.clone().unwrap();
        let cancel = CancellationToken::new();
        let (done, first_done) = oneshot::channel();
        let first = async {
            let result = controller
                .activate_deferred(cancel.clone(), deadline())
                .await;
            assert_eq!(
                result.unwrap_err().kind(),
                NativeMcpControllerError::Cancelled
            );
            done.send(()).unwrap();
        };
        let server_and_second = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let received = request(&mut socket).await;
            assert!(String::from_utf8_lossy(&received).contains("server/discover"));
            cancel.cancel();
            first_done.await.unwrap();
            let (receipt, ()) = join(
                controller.activate_deferred(CancellationToken::new(), deadline()),
                discovery(&mut socket),
            )
            .await;
            assert_eq!(
                receipt.unwrap().publication(),
                NativeMcpControllerPublication::Published
            );
        };
        join(first, server_and_second).await;
        assert!(Arc::ptr_eq(
            &original,
            lock(&controller.inner.state).active.as_ref().unwrap()
        ));
        assert!(!original.cancellation.is_cancelled());
        // A settled result does not open a second connection or reload the profile.
        fixture.seed("invalid");
        assert_eq!(
            controller
                .activate_deferred(CancellationToken::new(), deadline())
                .await
                .unwrap()
                .publication(),
            NativeMcpControllerPublication::Published
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
fn deferred_http_revalidates_exact_profile_after_discovery_before_append() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut fixture = Fixture::new();
        configure(&mut fixture, &listener);
        let controller = fixture.controller();
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let original = lock(&controller.inner.state).active.clone().unwrap();
        let checkpoint = fixture.options.runtime.publication_checkpoint().unwrap();
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            fixture.seed(r#"{"mcp":{}}"#);
            discovery(&mut socket).await;
        };
        let (result, ()) = join(
            controller.activate_deferred(CancellationToken::new(), deadline()),
            server,
        )
        .await;
        assert_eq!(
            result.unwrap_err().kind(),
            NativeMcpControllerError::Store(NativeMcpConfigStoreError::Conflict)
        );
        assert!(!original.cancellation.is_cancelled());
        let candidate = fixture
            .options
            .runtime
            .prepare_candidate(vec![], &[])
            .unwrap();
        fixture
            .options
            .runtime
            .publish_if(candidate, &checkpoint)
            .unwrap();
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
fn external_publication_during_reload_rejects_stale_candidate_and_preserves_old_token() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut fixture = Fixture::new();
        configure(&mut fixture, &listener);
        let controller = fixture.controller();
        controller
            .start(
                NativeMcpStartupPhase::AskStartup,
                CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let original = lock(&controller.inner.state).active.clone().unwrap();
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            let candidate = fixture
                .options
                .runtime
                .prepare_candidate(vec![], &[])
                .unwrap();
            fixture.options.runtime.publish(candidate).unwrap();
            discovery(&mut socket).await;
        };
        let (result, ()) = join(
            controller.reload(CancellationToken::new(), deadline()),
            server,
        )
        .await;
        assert!(matches!(
            result.unwrap_err().kind(),
            NativeMcpControllerError::Runtime(_)
        ));
        assert!(!original.cancellation.is_cancelled());
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}
