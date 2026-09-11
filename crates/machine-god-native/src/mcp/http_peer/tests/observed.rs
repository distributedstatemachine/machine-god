use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct SelectedClock {
    origin: Instant,
    millis: AtomicU64,
}
impl McpHttpClock for SelectedClock {
    fn now(&self) -> Instant {
        self.origin + Duration::from_millis(self.millis.load(Ordering::SeqCst))
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}

#[test]
fn observation_precedes_network_and_survives_rejection_cancellation_and_drop() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let make = || {
            options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            )
        };
        let observed = Arc::new(Mutex::new(Vec::new()));
        let capture = observed.clone();
        let admit: McpHttpCompletionObserver = Arc::new(move |completion| {
            assert!(!completion.is_complete());
            capture.lock().unwrap().push(completion);
            false
        });
        drop(McpHttpPeer::connect_observed(
            make(),
            CancellationToken::new(),
            deadline(),
            Duration::from_secs(1),
            None,
            admit.clone(),
        ));
        assert!(observed.lock().unwrap().is_empty());
        assert!(matches!(
            McpHttpPeer::connect_observed(
                make(),
                CancellationToken::new(),
                deadline(),
                Duration::from_secs(1),
                None,
                admit
            )
            .await,
            Err(McpHttpPeerError::Limit)
        ));
        assert!(observed.lock().unwrap()[0].is_complete());
        // Rejection must not have opened even a loopback socket.
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        let cancel = CancellationToken::new();
        let selected_cancel = cancel.clone();
        let capture = observed.clone();
        let admit = Arc::new(move |completion| {
            capture.lock().unwrap().push(completion);
            selected_cancel.cancel();
            true
        });
        assert!(matches!(
            McpHttpPeer::connect_observed(
                make(),
                cancel,
                deadline(),
                Duration::from_secs(1),
                None,
                admit
            )
            .await,
            Err(McpHttpPeerError::Cancelled)
        ));
        assert!(observed.lock().unwrap()[1].is_complete());
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());

        let capture = observed.clone();
        let mut future = Box::pin(McpHttpPeer::connect_observed(
            make(),
            CancellationToken::new(),
            deadline(),
            Duration::from_secs(1),
            None,
            Arc::new(move |completion| {
                capture.lock().unwrap().push(completion);
                true
            }),
        ));
        assert!(futures_util::poll!(&mut future).is_pending());
        drop(future);
        let completion = observed.lock().unwrap()[2].clone();
        completion.completed().await;
        assert!(completion.is_complete());
    });
}

#[test]
fn configured_fallback_refreshes_deadline_and_returns_remaining_catalog_budget() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let clock = Arc::new(SelectedClock {
            origin: Instant::now(),
            millis: AtomicU64::new(0),
        });
        let mut selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        selected.clock = clock.clone();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let capture = observed.clone();
        let server = async {
            let mut socket = listener.accept().await.unwrap().0;
            // Time changes before the modern mismatch is consumed. No sleeps.
            clock.millis.store(100, Ordering::SeqCst);
            reply(&mut socket, 404, "", b"").await;
            accept_reply(
                &listener,
                200,
                JSON,
                &success(
                    2,
                    serde_json::json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}),
                ),
            )
            .await;
            accept_reply(&listener, 202, "", b"").await;
            accept_reply(
                &listener,
                200,
                JSON,
                &success(3, serde_json::json!({"tools":[]})),
            )
            .await;
        };
        let client = async {
            let (mut peer, attempt) = McpHttpPeer::connect_observed(
                selected,
                CancellationToken::new(),
                clock.origin + Duration::from_secs(5),
                Duration::from_secs(1),
                Some(clock.origin + Duration::from_millis(500)),
                Arc::new(move |completion| {
                    capture.lock().unwrap().push(completion);
                    true
                }),
            )
            .await
            .unwrap();
            assert_eq!(attempt, clock.origin + Duration::from_millis(1100));
            assert_eq!(observed.lock().unwrap().len(), 1);
            peer.catalog(
                McpCatalogKind::Tools,
                McpCatalogLimits::default(),
                clock.origin,
                attempt,
            )
            .await
            .unwrap();
            peer.close();
            assert!(observed.lock().unwrap()[0].is_complete());
        };
        join(client, server).await;
    });
}

#[test]
fn configured_upper_timeout_is_inert_bounded_and_old_connector_cap_is_preserved() {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 1));
    let clock = Arc::new(SelectedClock {
        origin: Instant::now(),
        millis: AtomicU64::new(0),
    });
    let mut selected = options(address, TransportKind::StreamableHttp);
    selected.clock = clock.clone();
    let head = Arc::new(McpSubmissionHttpHead::new(selected.destination.endpoint(), &[]).unwrap());
    let maximum = Duration::from_millis(u64::from(u32::MAX));
    let create = |configured, duration| {
        let constructor = if configured {
            McpHttpConnection::from_configured_head
        } else {
            McpHttpConnection::from_prepared_head
        };
        constructor(
            selected.destination.clone(),
            head.clone(),
            None,
            McpHttpLimits::default(),
            CancellationToken::new(),
            clock.origin + duration,
            clock.clone(),
        )
    };
    assert!(create(false, Duration::from_secs(24 * 60 * 60)).is_ok());
    assert!(create(false, maximum).is_err());
    let connection = create(true, maximum).unwrap();
    let observation = connection.observation();
    drop(connection);
    assert!(observation.is_complete());
    assert!(create(true, maximum + Duration::from_millis(1)).is_err());
    let observed = Arc::new(Mutex::new(Vec::new()));
    for duration in [Duration::ZERO, maximum + Duration::from_millis(1), maximum] {
        let mut selected = options(address, TransportKind::StreamableHttp);
        selected.clock = clock.clone();
        selected.lifetime_deadline = clock.origin + maximum;
        let capture = observed.clone();
        assert!(matches!(
            futures_executor::block_on(McpHttpPeer::connect_observed(
                selected,
                CancellationToken::new(),
                clock.origin + maximum,
                duration,
                None,
                Arc::new(move |completion| {
                    capture.lock().unwrap().push(completion);
                    false
                })
            )),
            Err(McpHttpPeerError::Limit)
        ));
    }
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert!(observed.lock().unwrap()[0].is_complete());
}

#[test]
fn first_attempt_preserves_spent_auth_dns_budget_and_cannot_extend_configuration() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let clock = Arc::new(SelectedClock {
            origin: Instant::now(),
            millis: AtomicU64::new(100),
        });
        let make = || {
            let mut selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            selected.clock = clock.clone();
            selected
        };
        assert!(matches!(
            McpHttpPeer::connect_observed(
                make(),
                CancellationToken::new(),
                clock.origin + Duration::from_secs(5),
                Duration::from_secs(1),
                Some(clock.now()),
                Arc::new(|_| panic!("expired first budget must not observe"))
            )
            .await,
            Err(McpHttpPeerError::Deadline)
        ));
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        for (first_ms, expected_ms) in [(400, 400), (2000, 1100)] {
            let body = modern(1);
            let server = accept_reply(&listener, 200, JSON, &body);
            let client = async {
                let (mut peer, attempt) = McpHttpPeer::connect_observed(
                    make(),
                    CancellationToken::new(),
                    clock.origin + Duration::from_secs(5),
                    Duration::from_secs(1),
                    Some(clock.origin + Duration::from_millis(first_ms)),
                    Arc::new(|_| true),
                )
                .await
                .unwrap();
                assert_eq!(attempt, clock.origin + Duration::from_millis(expected_ms));
                peer.close();
            };
            join(client, server).await;
        }
    });
}
