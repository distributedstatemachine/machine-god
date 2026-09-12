use super::*;
use crate::mcp::stdio::McpStdioWriteReceipt;

#[test]
fn unpolled_exchange_does_not_take_pending_reply_custody() {
    let timer = ManualTimer::new();
    let mut peer = inert(timer.clone());
    peer.pending_replies.push(Box::pin(std::future::pending()));
    drop(peer.catalog(
        McpCatalogKind::Tools,
        McpCatalogLimits::default(),
        timer.origin,
        timer.at(1000),
    ));
    assert_eq!(peer.pending_replies.len(), 1);
    assert!(peer.readiness().is_ready());
    peer.close();
}

#[test]
fn primary_writer_waits_for_inherited_reply_receipt() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        let released = CancellationToken::new();
        let waiting = released.clone();
        peer.pending_replies.push(Box::pin(async move {
            waiting.cancelled().await;
            Ok(())
        }));
        let submissions = Arc::new(AtomicUsize::new(0));
        let observed = submissions.clone();
        let writer = Box::pin(async move {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(McpStdioWriteReceipt {
                outcome: Ok(()),
                attempted: true,
                acknowledged_bytes: 12,
            })
        });
        let id = RpcId::Integer(1);
        let mut exchange = Box::pin(routing::exchange(
            context(&mut peer),
            writer,
            &id,
            timer.at(1000),
            false,
        ));
        assert!(futures_util::poll!(&mut exchange).is_pending());
        assert_eq!(submissions.load(Ordering::SeqCst), 0);
        released.cancel();
        assert!(futures_util::poll!(&mut exchange).is_pending());
        assert_eq!(submissions.load(Ordering::SeqCst), 1);
        drop(exchange);
        assert!(!peer.readiness().is_ready()); // Consequential abandonment is unchanged.
        peer.close();
    });
}

#[test]
fn later_primary_cannot_extend_inherited_reply_deadline() {
    futures_executor::block_on(async {
        let timer = ManualTimer::new();
        let mut peer = inert(timer.clone());
        peer.pending_replies.push(
            routing::unsupported_reply(
                &peer.connection,
                &RpcId::Integer(77),
                peer.timer.clone(),
                peer.cancellation.clone(),
                timer.at(1000),
            )
            .unwrap(),
        );
        let mut observation = Box::pin(peer.next_notification(timer.at(500)));
        assert!(futures_util::poll!(&mut observation).is_pending());
        drop(observation);
        timer.advance(1000);
        let submissions = Arc::new(AtomicUsize::new(0));
        let observed = submissions.clone();
        let writer = Box::pin(async move {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(McpStdioWriteReceipt {
                outcome: Ok(()),
                attempted: true,
                acknowledged_bytes: 12,
            })
        });
        assert!(matches!(
            routing::exchange(
                context(&mut peer),
                writer,
                &RpcId::Integer(1),
                timer.at(3000),
                false
            )
            .await,
            Err(McpPeerError::Deadline)
        ));
        assert_eq!(submissions.load(Ordering::SeqCst), 0);
        assert!(!peer.readiness().is_ready());
        peer.close();
    });
}

fn context(peer: &mut McpStdioPeer) -> routing::Exchange<'_> {
    routing::Exchange {
        connection: &peer.connection,
        notifications: &mut peer.notifications,
        notification_bytes: &mut peer.notification_bytes,
        pending_replies: &mut peer.pending_replies,
        timer: &peer.timer,
        cancellation: &peer.cancellation,
    }
}
