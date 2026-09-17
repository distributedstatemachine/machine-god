//! Preacceptance limits reject without borrowing accepted-work settlement custody.
use super::*;
use machine_god_core::{CancellationToken, ManagedSubagentResult};
use store::{JournalError, JournalPublication};

#[test]
fn full_residency_rejects_create_before_later_cancel_without_external_progress() {
    let mut fixture = Fixture::new(vec![ModelProviderStep::pending()]);
    fixture.manager.limits.residents = 1;
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "running", "mode": "persistent", "prompt": "keep running"
            }}))
            .ok
    );
    fixture.drive(|f| f.factory.provider.requests().len() == 1);
    fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
    let (_create_admission, create) = fixture.invocation(serde_json::json!({"create": {
        "name": "overflow", "mode": "persistent"
    }}));
    let (_cancel_admission, cancel) = fixture.invocation(serde_json::json!({"lifecycle": {
        "id": "child-1", "action": "cancel"
    }}));
    let requester = fixture.requester.clone();
    let mut create = requester.execute(create, CancellationToken::new());
    let mut cancel = requester.execute(cancel, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(create.as_mut().poll(&mut cx).is_pending());
    assert!(cancel.as_mut().poll(&mut cx).is_pending());
    // No clock, provider response, unrelated wakeup or caller retirement can
    // release this slot. A bounded poll budget makes the original deadlock red.
    let mut rejected = None;
    for _ in 0..10 {
        if let Poll::Ready(result) = create.as_mut().poll(&mut cx) {
            rejected = Some(result.unwrap());
            break;
        }
        assert!(!matches!(
            fixture.manager.poll_progress(&mut cx, 100),
            Poll::Ready(Err(_))
        ));
    }
    let rejected = rejected.expect("unaccepted create parked ahead of the only cancellation");
    assert!(!rejected.ok);
    assert_eq!(rejected.error_code, Some(ManagedFailureCode::ResourceLimit));
    assert!(rejected.retryable);
    assert!(
        block_on(std::future::poll_fn(|cx| {
            if let Poll::Ready(result) = cancel.as_mut().poll(cx) {
                return Poll::Ready(result.unwrap());
            }
            if fixture.manager.poll_progress(cx, 100).is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        }))
        .ok
    );
    assert_eq!(fixture.factory.provider.requests().len(), 1);
    assert_eq!(fixture.factory.prepared.load(Ordering::Acquire), 1);
    assert!(fixture.manager.children[0].snapshot.head.queue.is_empty());
    assert_eq!(
        fixture.manager.children[0].snapshot.head.status,
        ManagedAgentState::Idle
    );
}

fn saturated() -> Fixture {
    let mut fixture = Fixture::new(vec![ModelProviderStep::pending(), completed(), completed()]);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "full", "mode": "persistent", "prompt": "head"
            }}))
            .ok
    );
    fixture.drive(|f| f.factory.provider.requests().len() == 1);
    assert_eq!(store::JournalLimits::default().queue_entries, 64);
    for index in 1..64 {
        assert!(
            fixture
                .command(serde_json::json!({"message": {"send": {
                    "id": "child-1", "content": format!("accepted-{index}")
                }}}))
                .ok
        );
    }
    fixture.drive(|f| f.manager.active.is_none());
    assert_eq!(fixture.manager.children[0].snapshot.head.queue.len(), 64);
    assert_eq!(fixture.factory.provider.requests().len(), 1);
    fixture
}

fn overflow(fixture: &mut Fixture) -> ManagedSubagentResult {
    let (admission, invocation) = fixture.invocation(serde_json::json!({"message": {"send": {
        "id": "child-1", "content": "must-not-be-accepted"
    }}}));
    let mut admission = Some(admission);
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let mut parked = false;
    let result = block_on(std::future::poll_fn(|cx| {
        if let Poll::Ready(result) = response.as_mut().poll(cx) {
            return Poll::Ready(result.unwrap());
        }
        let progress = fixture.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        if fixture.manager.retry.issue() == Some(ManagerBlock::Capacity) {
            // Make the regression fail without hanging Fixture::drop on the
            // broken original implementation's retained command. No durable
            // acceptance exists, so retire its actual caller and wake its retry.
            parked = true;
            admission.take();
            fixture.manager.retry_reconciliation();
            cx.waker().wake_by_ref();
        } else if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }));
    assert!(!parked, "unaccepted FIFO overflow held the journal lane");
    result
}

#[test]
fn full_default_fifo_rejects_overflow_and_preserves_command_and_sibling_progress() {
    let mut fixture = saturated();
    let accepted = fixture.manager.children[0].snapshot.head.queue.clone();
    let result = overflow(&mut fixture);
    assert!(!result.ok);
    assert_eq!(result.status, ManagedResultStatus::Rejected);
    assert_eq!(result.error_code, Some(ManagedFailureCode::ResourceLimit));
    assert!(result.retryable);
    assert!(result.requested.is_none());
    assert!(fixture.manager.retry.issue().is_none());
    assert_eq!(fixture.manager.children[0].snapshot.head.queue, accepted);

    assert!(
        fixture
            .command(serde_json::json!({"configure": {"id": "child-1", "name": "still-live"}}))
            .ok
    );
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "sibling", "mode": "one_off", "prompt": "sibling-work"
            }}))
            .ok
    );
    fixture.drive(|f| {
        f.manager.children[1].snapshot.head.status == ManagedAgentState::Completed
            && !f.manager.children[1].busy()
    });
    assert_eq!(fixture.factory.provider.requests().len(), 2);
    assert_eq!(fixture.manager.children[0].snapshot.head.queue, accepted);

    // Release the stalled head through the public durable cancellation path.
    // Accepted successors remain intact and require explicit resume.
    assert!(
        fixture
            .command(serde_json::json!({"lifecycle": {"id": "child-1", "action": "cancel"}}))
            .ok
    );
    assert_eq!(fixture.manager.children[0].snapshot.head.queue.len(), 63);
    assert_eq!(
        fixture.manager.children[0].snapshot.head.queue[0].id,
        accepted[1].id
    );
    assert_eq!(
        fixture.manager.children[0].snapshot.head.queue[0].status,
        ManagedQueueStatus::Interrupted
    );
    assert!(
        fixture
            .command(serde_json::json!({"lifecycle": {"id": "child-1", "action": "resume"}}))
            .ok
    );
    fixture.drive(|f| {
        f.manager.children[0].snapshot.head.queue.len() == 62 && !f.manager.children[0].busy()
    });
    assert_eq!(fixture.factory.provider.requests().len(), 3);
    assert_eq!(
        fixture.manager.children[0].snapshot.head.queue[0].id,
        accepted[2].id
    );
    assert!(
        fixture
            .command(serde_json::json!({"message": {"send": {
                "id": "child-1", "content": "accepted-after-capacity-freed"
            }}}))
            .ok
    );
    assert_eq!(fixture.manager.children[0].snapshot.head.queue.len(), 63);
}

#[test]
fn internal_journal_limit_keeps_original_settlement_until_explicit_retry() {
    let mut fixture = Fixture::new(vec![]);
    let gate = Arc::new(durability::RetryGate::default());
    let calls = std::cell::Cell::new(0);
    let mut settlement = Box::pin(durability::confirm(&fixture.journal, &gate, None, || {
        calls.set(calls.get() + 1);
        Box::pin(std::future::ready(Err(JournalError::Limit)))
    }));
    for _ in 0..2 {
        assert!(
            settlement
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert_eq!(gate.issue(), Some(ManagerBlock::Capacity));
        let before = calls.get();
        assert!(
            settlement
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert_eq!(calls.get(), before, "no unrequested settlement retry");
        gate.retry_capacity();
    }
    assert_eq!(calls.get(), 2);
    // The synthetic operation published nothing and holds no original receipt.
    drop(settlement);
    assert!(
        fixture
            .command(serde_json::json!({"create": {"name": "after-probe", "mode": "persistent"}}))
            .ok
    );
}

#[test]
fn admitted_journal_limit_rejects_but_busy_retains_the_original_operation() {
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({"create": {"name": "original", "mode": "persistent"}}))
            .ok
    );
    fixture.drive(|f| f.manager.active.is_none());
    let snapshot = fixture.manager.children[0].snapshot.clone();
    let (_admission, invocation) = fixture.invocation(serde_json::json!({"configure": {
        "id": "child-1", "name": "unpublished"
    }}));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(response.as_mut().poll(&mut cx).is_pending());
    let Poll::Ready(Some(job)) = fixture.manager.mailbox.poll_next(&mut cx) else {
        panic!("the actual invocation owns a mailbox job");
    };
    let gate = durability::RetryGate::default();
    let rejected = {
        let mut rejection = Box::pin(durability::confirm(
            &fixture.journal,
            &gate,
            Some(job.lease()),
            || Box::pin(std::future::ready(Err(JournalError::Limit))),
        ));
        let Poll::Ready(result) = rejection.as_mut().poll(&mut cx) else {
            panic!("an unaccepted limit must reject, not wait for a retry");
        };
        result
    };
    assert!(matches!(
        rejected,
        Err(durability::Failure::Rejected(JournalError::Limit))
    ));
    assert!(gate.issue().is_none());

    let calls = std::cell::Cell::new(0);
    let mut confirmation = Box::pin(durability::confirm(
        &fixture.journal,
        &gate,
        Some(job.lease()),
        || {
            calls.set(calls.get() + 1);
            Box::pin(std::future::ready(if calls.get() == 1 {
                Err(JournalError::Busy)
            } else {
                Ok(JournalPublication::Confirmed(Box::new(snapshot.clone())))
            }))
        },
    ));
    assert!(confirmation.as_mut().poll(&mut cx).is_pending());
    assert_eq!(gate.issue(), Some(ManagerBlock::Capacity));
    assert_eq!(calls.get(), 1);
    assert!(confirmation.as_mut().poll(&mut cx).is_pending());
    assert_eq!(calls.get(), 1);
    gate.retry_capacity();
    assert_eq!(block_on(confirmation).unwrap().head, snapshot.head);
    assert_eq!(calls.get(), 2);
    assert!(gate.issue().is_none());
    job.complete(Err(machine_god_core::ManagedSubagentError::Cancelled));
    assert!(block_on(response).is_err());
}
