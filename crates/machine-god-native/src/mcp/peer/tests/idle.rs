use super::*;
use std::sync::Mutex;

mod exchange;
mod runtime;
mod subscriptions;

struct ManualTimer {
    origin: Instant,
    state: Mutex<(u64, CancellationToken)>,
}
impl ManualTimer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            origin: Instant::now(),
            state: Mutex::new((0, CancellationToken::new())),
        })
    }
    fn at(&self, millis: u64) -> Instant {
        self.origin + Duration::from_millis(millis)
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
}
impl McpPeerTimer for ManualTimer {
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

fn inert(timer: Arc<dyn McpPeerTimer>) -> McpStdioPeer {
    McpStdioPeer {
        connection: McpStdioConnection::inert_for_test(),
        protocol: NegotiatedProtocol {
            transport: super::super::super::protocol::TransportKind::Stdio,
            version: ProtocolVersion::Modern,
        },
        capabilities: McpPeerCapabilities::default(),
        timer,
        cancellation: CancellationToken::new(),
        lifetime: McpPeerLifetime::OwnerControlled,
        feature_identity: Arc::new(()),
        next_id: Some(1),
        reserved: McpPendingToolReservation::default(),
        notifications: VecDeque::new(),
        notification_bytes: 0,
        pending_replies: routing::Replies::new(),
        subscription: crate::mcp::peer::subscription::State::default(),
        closed: false,
    }
}

fn retain_notice(peer: &mut McpStdioPeer) {
    let bytes = br#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed","params":{"marker":"exact"}}"#;
    peer.notifications
        .push_back(parse_envelope(bytes, WireLimits::default()).unwrap());
    peer.notification_bytes = bytes.len();
}

#[test]
fn dropping_idle_receiver_releases_lane_without_closing_inert_peer() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        for _ in 0..2 {
            let mut observation = Box::pin(peer.next_notification(timer.at(1000)));
            assert!(futures_util::poll!(&mut observation).is_pending());
            drop(observation);
            assert!(peer.readiness().is_ready());
        }
        peer.close();
    });
}

#[test]
fn completed_notification_stays_queued_across_abandoned_reply_drain() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let finish = CancellationToken::new();
        let pending = finish.clone();
        // Explicit receipt-custody double, not evidence of an actual pipe write.
        peer.pending_replies.push(Box::pin(async move {
            pending.cancelled().await;
            Ok(())
        }));
        retain_notice(&mut peer);
        let charged = peer.notification_bytes;
        let mut observation = Box::pin(peer.next_notification(timer.at(1000)));
        assert!(futures_util::poll!(&mut observation).is_pending());
        drop(observation);
        assert_eq!(peer.notifications.len(), 1);
        assert_eq!(peer.notification_bytes, charged);
        assert_eq!(peer.pending_replies.len(), 1);
        finish.cancel();
        let notice = peer.next_notification(timer.at(2000)).await.unwrap();
        assert_eq!(notice.into_value()["params"]["marker"], "exact");
        assert_eq!(peer.notification_bytes, 0);
        assert!(peer.pending_replies.is_empty());
        peer.close();
    });
}

#[test]
fn idle_timeout_retains_pending_reply_but_cannot_renew_its_deadline() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        peer.pending_replies.push(
            routing::unsupported_reply(
                &peer.connection,
                &RpcId::String("exact-id".into()),
                peer.timer.clone(),
                peer.cancellation.clone(),
                timer.at(2000),
            )
            .unwrap(),
        );
        let mut observation = Box::pin(peer.next_notification(timer.at(1000)));
        assert!(futures_util::poll!(&mut observation).is_pending());
        timer.advance(1000);
        assert!(matches!(observation.await, Err(McpPeerError::Deadline)));
        assert!(peer.readiness().is_ready());
        assert_eq!(peer.pending_replies.len(), 1);
        timer.advance(2000);
        assert!(matches!(
            peer.next_notification(timer.at(3000)).await,
            Err(McpPeerError::Deadline)
        ));
        assert!(peer.closed);
        assert!(peer.pending_replies.is_empty());
    });
}

#[test]
fn owner_cutoff_on_idle_drop_closes_and_releases_pending_custody() {
    futures_executor::block_on(async {
        for expiry in [false, true] {
            let timer = ManualTimer::new();
            let mut peer = inert(timer.clone());
            peer.lifetime = McpPeerLifetime::Until(timer.at(2000));
            peer.pending_replies.push(Box::pin(std::future::pending()));
            let cancellation = peer.cancellation.clone();
            let mut observation = Box::pin(peer.next_notification(timer.at(3000)));
            assert!(futures_util::poll!(&mut observation).is_pending());
            if expiry {
                timer.advance(2000);
            } else {
                cancellation.cancel();
            }
            drop(observation);
            assert!(peer.closed);
            assert!(!peer.readiness().is_ready());
            assert!(peer.pending_replies.is_empty());
        }
    });
}

#[test]
fn reply_completion_at_observation_deadline_keeps_queued_notification() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        retain_notice(&mut peer);
        let during_poll = timer.clone();
        // A completed receipt advances the observation clock in this custody double.
        peer.pending_replies.push(Box::pin(async move {
            during_poll.advance(1000);
            Ok(())
        }));
        assert!(matches!(
            peer.next_notification(timer.at(1000)).await,
            Err(McpPeerError::Deadline)
        ));
        assert!(peer.readiness().is_ready());
        assert_eq!(peer.notifications.len(), 1);
        assert!(peer.pending_replies.is_empty());
        let notice = peer.next_notification(timer.at(2000)).await.unwrap();
        assert_eq!(notice.into_value()["params"]["marker"], "exact");
        assert!(peer.take_notification().is_none());
        peer.close();
    });
}

#[test]
fn partial_receipt_double_is_retained_and_failure_is_not_replayed() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let polls = Arc::new(AtomicUsize::new(0));
        let attempted = polls.clone();
        let finish = CancellationToken::new();
        let pending = finish.clone();
        // This double tests receipt custody only; real partial pipe submission
        // remains covered by the lower-level stdio writer's process fixtures.
        peer.pending_replies.push(Box::pin(async move {
            attempted.fetch_add(1, Ordering::SeqCst);
            pending.cancelled().await;
            routing::validate_receipt(Ok(super::super::super::stdio::McpStdioWriteReceipt {
                outcome: Err(McpStdioError::Protocol),
                attempted: true,
                acknowledged_bytes: 3,
            }))
        }));
        let mut observation = Box::pin(peer.next_notification(timer.at(1000)));
        assert!(futures_util::poll!(&mut observation).is_pending());
        drop(observation);
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        assert_eq!(peer.pending_replies.len(), 1);
        finish.cancel();
        assert!(matches!(
            peer.next_notification(timer.at(2000)).await,
            Err(McpPeerError::Transport(McpStdioError::Protocol))
        ));
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        assert!(peer.closed);
    });
}
