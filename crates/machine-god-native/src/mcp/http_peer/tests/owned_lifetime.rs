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
                let readiness = peer.readiness();
                assert!(readiness.is_ready());
                clock.advance(2000);
                assert_eq!(readiness.is_ready(), !expires);
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
                assert!(!readiness.is_ready());
                assert!(completion.is_complete());
            };
            tokio::time::timeout(Duration::from_secs(3), join(client, server))
                .await
                .unwrap();
            assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        }
    });
}
