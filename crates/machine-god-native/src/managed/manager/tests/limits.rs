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

#[test]
fn reserved_foreground_capacity_rejects_create_without_consuming_the_ticket() {
    let mut fixture = Fixture::new(vec![]);
    fixture.manager.limits.residents = 1;
    let reservation = fixture.manager.reserve_foreground().unwrap();
    fixture.drive(|f| f.manager.reserved_foregrounds() == 1);
    let result = fixture.command(serde_json::json!({"create": {
        "name": "must-not-borrow", "mode": "persistent"
    }}));
    assert!(!result.ok);
    assert_eq!(result.error_code, Some(ManagedFailureCode::ResourceLimit));
    assert_eq!(reservation.validate_preparation(), Ok(()));
    assert_eq!(fixture.factory.prepared.load(Ordering::Acquire), 0);
    assert!(fixture.manager.children.is_empty());
    drop(reservation);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "after-ticket-drop", "mode": "persistent"
            }}))
            .ok
    );
}

#[test]
fn started_idle_retirement_keeps_original_request_until_actual_cleanup() {
    let mut fixture = Fixture::new(vec![]);
    fixture.manager.limits.residents = 1;
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "idle", "mode": "persistent"
            }}))
            .ok
    );
    fixture.drive(|f| f.manager.active.is_none());
    fixture.factory.cleanup.store(false, Ordering::Release);
    let (_admission, invocation) = fixture.invocation(serde_json::json!({"create": {
        "name": "replacement", "mode": "persistent"
    }}));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(response.as_mut().poll(&mut cx).is_pending());
    fixture.drive(|f| !f.manager.retiring.is_empty() && f.manager.pending_job.is_some());
    for _ in 0..3 {
        assert!(response.as_mut().poll(&mut cx).is_pending());
        assert!(!matches!(
            fixture.manager.poll_progress(&mut cx, 100),
            Poll::Ready(Err(_))
        ));
        assert_eq!(fixture.factory.prepared.load(Ordering::Acquire), 1);
        assert_eq!(fixture.manager.retiring.len(), 1);
    }
    fixture.factory.cleanup.store(true, Ordering::Release);
    fixture.drive(|f| {
        f.manager
            .children
            .iter()
            .any(|child| child.snapshot.head.id == "child-2")
    });
    assert!(block_on(response).unwrap().ok);
    assert!(fixture.manager.retiring.is_empty());
    assert_eq!(fixture.factory.prepared.load(Ordering::Acquire), 2);
    assert!(fixture.factory.provider.requests().is_empty());
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

#[test]
fn overlapping_lifecycle_keeps_the_first_accepted_control_until_actual_settlement() {
    for action in ["cancel", "close"] {
        let mut fixture = Fixture::new(vec![ModelProviderStep::pending()]);
        assert!(
            fixture
                .command(serde_json::json!({"create": {
                    "name": "worker", "mode": "persistent", "prompt": "work"
                }}))
                .ok
        );
        fixture.drive(|f| f.factory.provider.requests().len() == 1);
        fixture.factory.cleanup.store(false, Ordering::Release);
        let (_first_admission, first) = fixture.invocation(serde_json::json!({"lifecycle": {
            "id": "child-1", "action": "cancel"
        }}));
        let requester = fixture.requester.clone();
        let mut first = requester.execute(first, CancellationToken::new());
        assert!(
            first
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        fixture.drive(|f| {
            let child = &f.manager.children[0];
            child.snapshot.head.intent.is_none()
                && child.snapshot.head.queue.is_empty()
                && child.control.is_some()
                && child.settlement.is_some()
        });
        let (_second_admission, second) = fixture.invocation(serde_json::json!({"lifecycle": {
            "id": "child-1", "action": action
        }}));
        let mut second = requester.execute(second, CancellationToken::new());
        let (early_first, early_second) = block_on(std::future::poll_fn(|cx| {
            if let Poll::Ready(result) = first.as_mut().poll(cx) {
                return Poll::Ready((Some(result), None));
            }
            if let Poll::Ready(result) = second.as_mut().poll(cx) {
                return Poll::Ready((None, Some(result)));
            }
            let progress = fixture.manager.poll_progress(cx, 100);
            assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        }));
        // Release the exact delayed cleanup even on the broken implementation,
        // so failure reports never strand this test's manager-owned resources.
        fixture.factory.cleanup.store(true, Ordering::Release);
        fixture.drive(|f| {
            f.manager
                .children
                .iter()
                .all(|child| child.control.is_none())
        });
        let displaced = early_first.is_some();
        let first = early_first.unwrap_or_else(|| block_on(first));
        let second = early_second.unwrap_or_else(|| block_on(second));
        assert!(
            !displaced,
            "{action} displaced the accepted cancellation receipt"
        );
        assert!(first.unwrap().ok);
        let rejected = second.unwrap();
        assert_eq!(rejected.error_code, Some(ManagedFailureCode::ResourceLimit));
        assert!(rejected.retryable);
        assert_eq!(
            fixture.manager.children[0].snapshot.head.status,
            ManagedAgentState::Idle
        );
        assert!(
            fixture
                .command(serde_json::json!({"lifecycle": {
                    "id": "child-1", "action": action
                }}))
                .ok
        );
    }
}

#[test]
fn replenished_sibling_writes_cannot_starve_original_notice_replay() {
    let mut fixture = Fixture::new(vec![]);
    let parent = fixture.notice_session();
    let original = super::delivery::original(&mut fixture, &parent);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "busy-sibling", "mode": "persistent"
            }}))
            .ok
    );
    fixture.restart_manager();
    let context = Arc::new(crate::managed::prompt_context::ParentNoticeContext::new(
        &parent,
        original.target.parent.clone(),
        &fixture.manager.notices,
    ));
    fixture.manager.register_parent_context(&context).unwrap();
    fixture.manager.limits.work_per_poll = 1;
    let requester = fixture.requester.clone();
    let configure = || {
        serde_json::json!({"configure": {
            "id": "child-2", "name": "busy"
        }})
    };
    let mut cx = Context::from_waker(Waker::noop());
    let mut traffic = Vec::new();
    for _ in 0..4 {
        let (admission, invocation) = fixture.invocation(configure());
        let mut response = requester.execute(invocation, CancellationToken::new());
        assert!(response.as_mut().poll(&mut cx).is_pending());
        traffic.push((admission, response));
    }
    for _ in 0..64 {
        let index = block_on(std::future::poll_fn(|cx| {
            for (index, slot) in traffic.iter_mut().enumerate() {
                if let Poll::Ready(result) = slot.1.as_mut().poll(cx) {
                    assert!(result.unwrap().ok);
                    return Poll::Ready(index);
                }
            }
            let progress = fixture.manager.poll_progress(cx, 100);
            assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        }));
        // Keep three other actual admitted mutations continuously queued.
        let (admission, invocation) = fixture.invocation(configure());
        let mut response = requester.execute(invocation, CancellationToken::new());
        assert!(response.as_mut().poll(&mut cx).is_pending());
        traffic[index] = (admission, response);
    }
    let replayed = fixture
        .manager
        .notices
        .snapshot(&original.target.parent, 64, 64 * 1024)
        .unwrap()
        .entries()
        .iter()
        .any(|entry| entry.notice() == &original);
    drop(traffic);
    fixture.drive(|f| f.manager.replay.done && f.manager.active.is_none());
    assert!(
        fixture
            .manager
            .notices
            .snapshot(&original.target.parent, 64, 64 * 1024)
            .unwrap()
            .entries()
            .iter()
            .any(|entry| entry.notice() == &original)
    );
    assert!(fixture.factory.provider.requests().is_empty());
    assert!(
        replayed,
        "unrelated admitted writes repeatedly reset durable replay"
    );
}

#[test]
fn replenished_mailbox_traffic_cannot_starve_accepted_work_or_ready_waits() {
    let mut fixture = Fixture::new(vec![completed()]);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "inspection-target", "mode": "persistent"
            }}))
            .ok
    );
    fixture.manager.limits.work_per_poll = 1;
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "accepted-worker", "mode": "one_off", "prompt": "must-progress"
            }}))
            .ok
    );
    assert!(fixture.factory.provider.requests().is_empty());
    let requester = fixture.requester.clone();
    let (_wait_admission, wait) = fixture.invocation(serde_json::json!({"inspect": {
        "id": "child-1", "sections": ["status"], "wait": {"until": "settled", "timeout_ms": 100}
    }}));
    let mut wait = requester.execute(wait, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(wait.as_mut().poll(&mut cx).is_pending());
    let Poll::Ready(Some(job)) = fixture.manager.mailbox.poll_next(&mut cx) else {
        panic!("actual admitted inspection job");
    };
    fixture
        .manager
        .ready_jobs
        .push_back((job, false, "settled-wait".into()));

    let inspection = || {
        serde_json::json!({"inspect": {
            "id": "child-1", "sections": ["status"]
        }})
    };
    let mut traffic = Vec::new();
    for _ in 0..4 {
        let (admission, invocation) = fixture.invocation(inspection());
        let mut response = requester.execute(invocation, CancellationToken::new());
        assert!(response.as_mut().poll(&mut cx).is_pending());
        traffic.push((admission, response));
    }
    let mut responses = 0;
    let mut waited = None;
    while responses < 64 {
        let index = block_on(std::future::poll_fn(|cx| {
            if waited.is_none()
                && let Poll::Ready(result) = wait.as_mut().poll(cx)
            {
                waited = Some(result);
            }
            for (index, slot) in traffic.iter_mut().enumerate() {
                if let Poll::Ready(result) = slot.1.as_mut().poll(cx) {
                    assert!(result.unwrap().ok);
                    return Poll::Ready(index);
                }
            }
            let progress = fixture.manager.poll_progress(cx, 100);
            assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        }));
        responses += 1;
        // Fixture invocation drives a real core caller, outside this manager's
        // executor. Three other admitted inspections remain continuously queued.
        let (admission, invocation) = fixture.invocation(inspection());
        let mut response = requester.execute(invocation, CancellationToken::new());
        assert!(response.as_mut().poll(&mut cx).is_pending());
        traffic[index] = (admission, response);
    }
    let progressed = fixture.manager.children[1].snapshot.head.status
        == ManagedAgentState::Completed
        && !fixture.manager.children[1].busy();
    let replayed = fixture.manager.replay.done;
    drop(traffic);
    drop(wait);
    fixture.drive(|f| {
        f.manager.children[1].snapshot.head.status == ManagedAgentState::Completed
            && !f.manager.children[1].busy()
    });
    assert!(
        progressed,
        "bounded, continually replenished inspection traffic starved accepted work"
    );
    assert!(
        waited.is_some(),
        "completed wait response starved behind new inspections"
    );
    assert!(waited.unwrap().unwrap().ok);
    assert!(
        replayed,
        "unchanged inspections repeatedly reset durable replay"
    );
}

fn exhaust_ordinary_history(fixture: &mut Fixture, index: usize) {
    let filler = JournalRecord::History(machine_god_core::ManagedHistoryItem {
        kind: machine_god_core::ManagedHistoryKind::Conversation,
        work_id: None,
        user: Some("\0".repeat(16 * 1024)),
        assistant: Some("\0".repeat(16 * 1024)),
        user_truncated: false,
        assistant_truncated: false,
    });
    for _ in 0..64 {
        if !fixture.journal.ordinary_publication_available() {
            return;
        }
        let JournalPublication::Confirmed(snapshot) = block_on(fixture.journal.mutate(
            fixture.manager.children[index].snapshot.clone(),
            JournalMutation::AppendHistory(vec![filler.clone(), filler.clone()]),
        ))
        .unwrap() else {
            panic!("confirmed bounded ordinary history publication");
        };
        fixture.manager.children[index].snapshot = *snapshot;
    }
    panic!("finite journal did not reach its protected settlement reserve");
}

#[test]
#[allow(clippy::too_many_lines)] // One finite-budget interruption across publication, cleanup and restart.
fn journal_pressure_settles_owned_writes_and_interrupts_fifo_without_starting_successors() {
    let mut fixture = Fixture::with_journal_limits(
        vec![ModelProviderStep::pending()],
        store::JournalLimits {
            head_bytes: 16 * 1024,
            page_bytes: 512 * 1024,
            aggregate_bytes: 16 * 1024 * 1024,
            ..store::JournalLimits::default()
        },
    );
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "running", "mode": "persistent", "prompt": "original",
                "notifications": {"started": true}
            }}))
            .ok
    );
    fixture.drive(|f| f.factory.provider.requests().len() == 1);
    for content in ["successor-a", "successor-b"] {
        assert!(
            fixture
                .command(serde_json::json!({"message": {"send": {
                    "id": "child-1", "content": content
                }}}))
                .ok
        );
    }
    fixture.drive(|f| {
        f.manager.active.is_none()
            && f.manager.replay.done
            && f.manager.children[0].pending.is_empty()
            && !f.manager.children[0].notice_started
    });
    fixture.manager.limits.work_per_poll = 1;
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "not-started", "mode": "one_off", "prompt": "accepted-unstarted"
            }}))
            .ok
    );
    assert_eq!(fixture.factory.provider.requests().len(), 1);
    let queue: Vec<_> = fixture.manager.children[0]
        .snapshot
        .head
        .queue
        .iter()
        .map(|work| work.id.clone())
        .collect();
    fixture.manager.children[0].pending.push_back(ChildWrite {
        mutation: JournalMutation::AppendHistory(vec![JournalRecord::History(
            machine_god_core::ManagedHistoryItem {
                kind: machine_god_core::ManagedHistoryKind::Conversation,
                work_id: Some(queue[0].clone()),
                user: Some("owned-before-pressure".into()),
                assistant: None,
                user_truncated: false,
                assistant_truncated: false,
            },
        )]),
        after: WriteAfter::Observe,
    });
    exhaust_ordinary_history(&mut fixture, 1);
    fixture.factory.cleanup.store(false, Ordering::Release);
    fixture.drive(|f| {
        f.manager
            .children
            .iter()
            .all(|child| child.snapshot.head.status == ManagedAgentState::Interrupted)
            && f.manager.children[0].settlement.is_some()
    });
    assert!(!fixture.manager.children[0].actual_settled);
    assert_eq!(fixture.manager.retry.issue(), None);
    assert_eq!(fixture.factory.provider.requests().len(), 1);
    assert!(
        fixture
            .command(serde_json::json!({"inspect": {
                "id": "child-2", "sections": ["status"]
            }}))
            .ok
    );
    fixture.factory.cleanup.store(true, Ordering::Release);
    fixture.drive(|f| {
        f.manager
            .children
            .iter()
            .all(|child| !child.busy() && child.actual_settled)
            && f.manager.active.is_none()
            && f.manager.replay.done
    });
    let snapshot = fixture.manager.children[0].snapshot.clone();
    assert_eq!(
        snapshot
            .head
            .queue
            .iter()
            .map(|work| work.id.clone())
            .collect::<Vec<_>>(),
        queue
    );
    assert!(
        snapshot
            .head
            .queue
            .iter()
            .all(|work| work.status == ManagedQueueStatus::Interrupted)
    );
    assert!(snapshot.head.intent.is_none());
    let mut cursor = None;
    let mut owned = 0;
    let mut interrupted = 0;
    loop {
        let page = block_on(fixture.journal.history(snapshot.clone(), cursor, 100)).unwrap();
        for record in page.records {
            match record {
                JournalRecord::History(item) => {
                    owned += usize::from(item.user.as_deref() == Some("owned-before-pressure"));
                    interrupted +=
                        usize::from(item.kind == machine_god_core::ManagedHistoryKind::Interrupted);
                }
                JournalRecord::Notice(notice) => assert!(!matches!(
                    notice.event,
                    crate::managed::notices::NoticeEvent::Terminal { .. }
                )),
                _ => {}
            }
        }
        let Some(next) = page.next else {
            break;
        };
        cursor = Some(next);
    }
    assert_eq!((owned, interrupted), (1, 1));
    fixture.restart_manager();
    assert!(
        fixture
            .command(serde_json::json!({"inspect": {
                "id": "child-1", "sections": ["status"]
            }}))
            .ok
    );
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}

#[test]
fn journal_pressure_preserves_previously_accepted_archive_until_actual_cleanup() {
    let mut fixture = Fixture::with_journal_limits(
        vec![ModelProviderStep::pending()],
        store::JournalLimits {
            head_bytes: 16 * 1024,
            page_bytes: 512 * 1024,
            aggregate_bytes: 16 * 1024 * 1024,
            ..store::JournalLimits::default()
        },
    );
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "running", "mode": "persistent", "prompt": "original"
            }}))
            .ok
    );
    fixture.drive(|f| f.factory.provider.requests().len() == 1);
    assert!(
        fixture
            .command(serde_json::json!({"message": {"send": {
                "id": "child-1", "content": "accepted-successor"
            }}}))
            .ok
    );
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "filler", "mode": "persistent"
            }}))
            .ok
    );
    fixture.manager.limits.work_per_poll = 1;
    fixture.factory.cleanup.store(false, Ordering::Release);
    let (_admission, invocation) = fixture.invocation(serde_json::json!({"lifecycle": {
        "id": "child-1", "action": "close"
    }}));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(response.as_mut().poll(&mut cx).is_pending());
    fixture.drive(|f| {
        f.manager.children[0].control.is_some()
            && f.manager.children[0].snapshot.head.intent == Some(store::JournalIntent::Archive)
    });
    exhaust_ordinary_history(&mut fixture, 1);
    fixture.drive(|f| {
        f.manager.children[0].settlement.is_some()
            && f.manager.children[0].work.is_none()
            && f.manager.children[0].pending.is_empty()
            && f.manager.active.is_none()
    });
    assert!(response.as_mut().poll(&mut cx).is_pending());
    assert_eq!(
        fixture.manager.children[0].snapshot.head.intent,
        Some(store::JournalIntent::Archive)
    );
    assert_eq!(fixture.manager.retry.issue(), None);
    fixture.factory.cleanup.store(true, Ordering::Release);
    let result = block_on(std::future::poll_fn(|cx| {
        if let Poll::Ready(result) = response.as_mut().poll(cx) {
            return Poll::Ready(result.unwrap());
        }
        let progress = fixture.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }));
    assert!(result.ok);
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert_eq!(snapshot.head.status, ManagedAgentState::Archived);
    assert!(snapshot.head.intent.is_none());
    assert!(
        snapshot
            .head
            .queue
            .iter()
            .all(|work| work.status == ManagedQueueStatus::Interrupted)
    );
    assert_eq!(fixture.factory.provider.requests().len(), 1);
    let filler = fixture
        .manager
        .children
        .iter()
        .position(|child| child.snapshot.head.id == "child-2")
        .unwrap();
    exhaust_ordinary_history(&mut fixture, filler);
    fixture.restart_manager();
    let inspected = fixture.command(serde_json::json!({"inspect": {
        "id": "child-1", "sections": ["status"]
    }}));
    assert!(inspected.ok);
    assert_eq!(fixture.manager.retry.issue(), None);
}

struct PressureClock(std::sync::Mutex<Instant>);
impl NativeMcpRuntimeClock for PressureClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
    fn sleep_until(&self, deadline: Instant) -> machine_god_core::BoxFuture<'_, ()> {
        // Tests advance and poll explicitly; no ambient timer or wall-clock wait.
        Box::pin(std::future::poll_fn(move |_| {
            if self.now() >= deadline {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
    }
}

#[test]
fn journal_pressure_confirms_the_owned_interval_but_stops_later_deadlines() {
    let mut fixture = Fixture::with_journal_limits(
        vec![ModelProviderStep::pending()],
        store::JournalLimits {
            head_bytes: 16 * 1024,
            page_bytes: 512 * 1024,
            aggregate_bytes: 16 * 1024 * 1024,
            ..store::JournalLimits::default()
        },
    );
    let clock = Arc::new(PressureClock(std::sync::Mutex::new(Instant::now())));
    fixture.manager.clock = clock.clone();
    fixture.manager.notices = Arc::new(
        ManagedNotices::new(
            crate::managed::notices::NoticeLimits::default(),
            clock.clone(),
        )
        .unwrap(),
    );
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "reporting", "mode": "persistent", "prompt": "pending forever",
                "notifications": {"report_interval_ms": 1}
            }}))
            .ok
    );
    fixture.drive(|f| f.factory.provider.requests().len() == 1);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "filler", "mode": "persistent"
            }}))
            .ok
    );
    fixture.drive(|f| {
        f.manager.active.is_none()
            && f.manager.replay.done
            && f.manager.children[0].pending.is_empty()
            && !f.manager.children[0].notice_started
    });
    *clock.0.lock().unwrap() += std::time::Duration::from_millis(2);
    let mut cx = Context::from_waker(Waker::noop());
    assert!(fixture.manager.pump_notices(&mut cx).unwrap());
    assert!(matches!(
        fixture.manager.children[0].pending.front().unwrap().after,
        WriteAfter::Notice { stage: Some(_), .. }
    ));
    exhaust_ordinary_history(&mut fixture, 1);
    *clock.0.lock().unwrap() += std::time::Duration::from_millis(100);
    fixture.drive(|f| {
        f.manager.children[0].snapshot.head.status == ManagedAgentState::Interrupted
            && !f.manager.children[0].busy()
            && f.manager.children[0].actual_settled
            && f.manager.active.is_none()
            && f.manager.replay.done
    });
    let snapshot = fixture.manager.children[0].snapshot.clone();
    let mut cursor = None;
    let mut intervals = 0;
    loop {
        let page = block_on(fixture.journal.history(snapshot.clone(), cursor, 100)).unwrap();
        for record in page.records {
            if let JournalRecord::Notice(notice) = record {
                intervals += usize::from(matches!(
                    notice.event,
                    crate::managed::notices::NoticeEvent::Interval { .. }
                ));
                assert!(!matches!(
                    notice.event,
                    crate::managed::notices::NoticeEvent::Terminal { .. }
                ));
            }
        }
        let Some(next) = page.next else {
            break;
        };
        cursor = Some(next);
    }
    assert_eq!(intervals, 1);
    assert!(fixture.manager.children[0].notice.is_none());
    assert_eq!(fixture.manager.retry.issue(), None);
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}
