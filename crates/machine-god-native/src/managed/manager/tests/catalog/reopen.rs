use super::{Fixture, NativeManagedCatalogFilter, read_page};
use crate::managed::manager::ManagedForegroundSelection;
use crate::managed::store::{JournalMutation, JournalPublication};
use futures_executor::block_on;
use machine_god_core::{
    CancellationToken, ManagedAgentState, ManagedFailureCode, ManagedSubagentCommand,
    ManagedSubagentResult,
};
use std::task::{Context, Poll, Waker};

fn archived() -> Fixture {
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({
                "create": {"name": "saved", "mode": "persistent"}
            }))
            .ok
    );
    assert!(
        fixture
            .command(serde_json::json!({
                "lifecycle": {"id": "child-1", "action": "close"}
            }))
            .ok
    );
    fixture.restart_journal_owner();
    let head = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert!(head.recovery_required());
    assert_eq!(head.head.status, ManagedAgentState::Archived);
    fixture
}

fn foreground(fixture: &mut Fixture) -> ManagedForegroundSelection {
    let prepared = fixture.notified_foreground(fixture.notice_session());
    enroll(fixture, prepared)
}

fn enroll(
    fixture: &mut Fixture,
    prepared: crate::managed::manager::factory::PreparedManagedRuntime,
) -> ManagedForegroundSelection {
    let reservation = fixture.manager.reserve_foreground().unwrap();
    fixture.drive(|f| {
        f.manager
            .poll_foreground_reservation(&reservation, &Context::from_waker(Waker::noop()))
            .is_ready()
    });
    fixture
        .manager
        .enroll_foreground(Box::new(prepared), &reservation)
        .unwrap()
}

#[test]
fn observed_human_reopen_accepts_owned_ack_progress_after_recovery() {
    reopen_with_ack(false);
}

#[test]
fn observed_human_reopen_rejects_external_same_generation_change_before_owned_ack() {
    reopen_with_ack(true);
}

#[allow(clippy::too_many_lines)] // Actual observed human, saved checkpoint and source ACK share one original lifetime.
fn reopen_with_ack(external_ack: bool) {
    use crate::managed::{
        manager::tests::{completed, delivery},
        notices::{NoticeRelationship, NoticeTerminal, PreparedNotice},
        store::JournalRecord,
    };
    use machine_god_core::ManagedNotifications;
    use std::{num::NonZeroU64, sync::atomic::Ordering};

    let mut fixture = Fixture::new(vec![completed()]);
    let session = fixture.notice_session();
    let original = delivery::original(&mut fixture, &session);
    // Two distinct durable originals let an external ACK advance the same
    // generation before ordinary delivery publishes the remaining valid ACK.
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let mut second = original.source.clone();
    second.work_id = "second-work".into();
    let work = fixture
        .manager
        .notices
        .register_work(
            &second,
            ManagedNotifications::default(),
            &NoticeRelationship {
                generation: original.target.relationship_generation,
                parent: Some(original.target.parent.clone()),
                parent_incarnation: Some(original.target.parent_incarnation.clone()),
            },
            snapshot.head.notice_cursor,
        )
        .unwrap();
    let PreparedNotice::Staged(stage) = fixture
        .manager
        .notices
        .prepare_terminal(
            &work,
            NonZeroU64::new(snapshot.head.next_sequence).unwrap(),
            NoticeTerminal::Completed,
            None,
        )
        .unwrap()
    else {
        panic!("second original");
    };
    let JournalPublication::Confirmed(snapshot) = block_on(fixture.journal.mutate(
        snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::Notice(stage.notice().clone())]),
    ))
    .unwrap() else {
        panic!("confirmed second original");
    };
    fixture.manager.children[0].snapshot = *snapshot;
    fixture.manager.notices.confirm_durable(&stage).unwrap();
    fixture.manager.notices.release_work(&work).unwrap();
    let (context, conversation) = delivery::deliver(&fixture, &session);
    let receipt = context.delivery().unwrap();
    assert_eq!(receipt.originals().len(), 2);
    assert!(
        fixture
            .command(serde_json::json!({
                "lifecycle": {"id": "child-1", "action": "close"}
            }))
            .ok
    );
    fixture.restart_journal_owner();
    let prepared = fixture.notified_foreground(session);
    let foreground = enroll(&mut fixture, prepared);
    fixture
        .manager
        .request_catalog(NativeManagedCatalogFilter::Archived, None, 1)
        .unwrap();
    let observed = read_page(&mut fixture)
        .unwrap()
        .entries
        .remove(0)
        .observation;
    fixture.manager.limits.work_per_poll = 1;
    fixture.factory.cleanup.store(false, Ordering::Release);
    let preparations = fixture.factory.prepared.load(Ordering::Acquire);
    let response = fixture
        .manager
        .request_observed_human_command(
            &foreground,
            Some(observed),
            ManagedSubagentCommand::decode(serde_json::json!({
                "command": {"lifecycle": {"id": "child-1", "action": "reopen"}}
            }))
            .unwrap(),
            &[],
            CancellationToken::new(),
        )
        .unwrap();
    fixture.drive(|f| {
        f.factory.prepared.load(Ordering::Acquire) == preparations + 1 && f.manager.active.is_none()
    });
    assert_eq!(fixture.manager.saved_lifetimes.len(), 1);
    let expected = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    if external_ack {
        assert!(matches!(
            block_on(fixture.journal.mutate(
                expected.clone(),
                JournalMutation::AppendHistory(vec![JournalRecord::NoticeAcknowledged {
                    identity: original.identity(),
                    target: original.target,
                    checkpoint: receipt.checkpoint().clone(),
                }])
            ))
            .unwrap(),
            JournalPublication::Confirmed(_)
        ));
    }
    fixture.manager.stage_parent_context(&context).unwrap();
    fixture.drive(|f| {
        f.manager
            .parents
            .iter()
            .any(|parent| parent.completed.is_some())
    });
    let acknowledged = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert_eq!(acknowledged.head.generation, expected.head.generation);
    assert_eq!(
        acknowledged.head.revision,
        expected.head.revision + if external_ack { 2 } else { 1 }
    );
    block_on(conversation.clear_notice_delivery(&receipt)).unwrap();
    fixture.factory.cleanup.store(true, Ordering::Release);
    let result = finish(&mut fixture, response);
    fixture.drive(|f| f.manager.active.is_none());
    let after = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    if external_ack {
        assert_eq!(result.error_code, Some(ManagedFailureCode::StaleGeneration));
        assert_eq!(after.head.generation, 1);
        assert_eq!(
            fixture.factory.prepared.load(Ordering::Acquire),
            preparations + 1
        );
    } else {
        assert!(result.ok, "{result:?}");
        assert_eq!(after.head.generation, 2);
    }
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}

fn finish(
    fixture: &mut Fixture,
    mut response: crate::managed::mailbox::ManagedCommandResponse,
) -> ManagedSubagentResult {
    block_on(std::future::poll_fn(|cx| {
        if let Poll::Ready(result) = response.as_mut().poll(cx) {
            return Poll::Ready(result.unwrap());
        }
        let progress = fixture.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }))
}

#[test]
fn observed_human_reopen_accepts_its_own_old_owner_recovery() {
    observed_reopen(false);
}

#[test]
fn observed_human_reopen_rejects_external_recovery_before_admission() {
    observed_reopen(true);
}

fn observed_reopen(external_recovery: bool) {
    let mut fixture = archived();
    let foreground = foreground(&mut fixture);
    fixture
        .manager
        .request_catalog(NativeManagedCatalogFilter::Archived, None, 1)
        .unwrap();
    let observed = read_page(&mut fixture)
        .unwrap()
        .entries
        .remove(0)
        .observation;
    if external_recovery {
        fixture.drive(|f| f.manager.active.is_none());
        let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
        assert!(matches!(
            block_on(fixture.journal.mutate(snapshot, JournalMutation::Recover)).unwrap(),
            JournalPublication::Confirmed(_)
        ));
    }
    let operation = format!("operation-{}", fixture.manager.next_operation);
    let prepared = fixture
        .factory
        .prepared
        .load(std::sync::atomic::Ordering::Acquire);
    let response = fixture
        .manager
        .request_observed_human_command(
            &foreground,
            Some(observed),
            ManagedSubagentCommand::decode(serde_json::json!({
                "command": {"lifecycle": {"id": "child-1", "action": "reopen"}}
            }))
            .unwrap(),
            &[],
            CancellationToken::new(),
        )
        .unwrap();
    let result = finish(&mut fixture, response);
    assert_eq!(result.operation_id, operation);
    fixture.drive(|f| f.manager.active.is_none());
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    if external_recovery {
        assert_eq!(result.error_code, Some(ManagedFailureCode::StaleGeneration));
        assert_eq!(snapshot.head.generation, 1);
        assert_eq!(
            fixture
                .factory
                .prepared
                .load(std::sync::atomic::Ordering::Acquire),
            prepared
        );
    } else {
        assert!(result.ok, "{result:?}");
        assert_eq!(snapshot.head.generation, 2);
        assert_eq!(snapshot.head.status, ManagedAgentState::Idle);
    }
    assert!(fixture.factory.provider.requests().is_empty());
}
