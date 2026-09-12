use super::*;
use std::{sync::Mutex, task::Poll};

struct AdvancingClock(Mutex<Instant>);
impl NativeMcpRuntimeClock for AdvancingClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::poll_fn(move |_| {
            if NativeMcpRuntimeClock::now(self) >= deadline {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
    }
}
impl McpHttpClock for AdvancingClock {
    fn now(&self) -> Instant {
        NativeMcpRuntimeClock::now(self)
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        NativeMcpRuntimeClock::sleep_until(self, deadline)
    }
}

#[test]
fn configured_serial_servers_retain_full_maximum_timeout_without_an_aggregate_cap() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let url = format!(
            "http://127.0.0.1:{}/mcp",
            listener.local_addr().unwrap().port()
        );
        let config = json!({"mcp": {
            "first": {"type":"http", "url":url, "required":true, "startup_timeout_ms":u32::MAX},
            "second": {"type":"http", "url":url, "required":true, "startup_timeout_ms":u32::MAX}
        }});
        let mut selected = options(&config.to_string());
        let now = Instant::now();
        let clock = Arc::new(AdvancingClock(Mutex::new(now)));
        selected.clock = clock.clone();
        selected.catalog_epoch = now;
        selected.peer_lifetime = McpPeerLifetime::OwnerControlled;
        selected.network = Some(Arc::new(
            NativeMcpNetwork::new(
                McpResolverConfig::literal_only(),
                [7; 32],
                None,
                clock.clone(),
                CancellationToken::new(),
                2,
            )
            .unwrap(),
        ));
        let startup = NativeMcpStartup::new(selected).unwrap();
        let server = async {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                request(&mut socket).await;
                *clock.0.lock().unwrap() += Duration::from_millis(u64::from(u32::MAX) - 1);
                let body = discover(false);
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
                socket.flush().await.unwrap();
            }
        };
        let (batch, ()) = join(
            startup.build_configured(NativeMcpStartupPhase::All, CancellationToken::new()),
            server,
        )
        .await;
        assert!(batch.receipt().required_ready());
        assert_eq!(batch.servers().len(), 2);
        assert!(
            NativeMcpRuntimeClock::now(clock.as_ref())
                > now + Duration::from_millis(u64::from(u32::MAX))
        );
        drop(batch);
    });
}

#[test]
fn configured_pending_startup_remains_cancellable_without_an_outer_timer() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut selected = http_options(listener.local_addr().unwrap());
        selected.peer_lifetime = McpPeerLifetime::OwnerControlled;
        let startup = NativeMcpStartup::new(selected).unwrap();
        let cancellation = CancellationToken::new();
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            cancellation.cancel();
        };
        let (batch, ()) = join(
            startup.build_configured(NativeMcpStartupPhase::All, cancellation.clone()),
            server,
        )
        .await;
        assert!(!batch.receipt().required_ready());
        assert_eq!(
            batch.receipt().failure,
            Some(NativeMcpStartupError::Cancelled)
        );
        drop(batch);
    });
}
