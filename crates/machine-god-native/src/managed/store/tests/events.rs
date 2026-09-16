use super::*;
use machine_god_core::{ManagedEvent, ManagedEventKind};

fn event(journal: &ManagedJournal, snapshot: &JournalSnapshot) -> ManagedEvent {
    let page = block_on(journal.history(snapshot.clone(), None, 1)).unwrap();
    let JournalRecord::Event(event) = &page.records[0] else {
        panic!("mutation must publish its typed event first");
    };
    assert_eq!(event.sequence, snapshot.head.last_event_sequence);
    assert_eq!(event.revision, snapshot.head.revision);
    event.clone()
}

#[test]
fn recovery_event_and_original_receipt_survive_exact_publication_reconciliation() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    *journal.shared.failure.lock().unwrap() = Some(FailurePoint::AfterHeadRename);
    let JournalPublication::Ambiguous(receipt) = block_on(journal.create(create("child"))).unwrap()
    else {
        panic!("ambiguous creation");
    };
    let snapshot = confirmed(block_on(journal.reconcile(receipt)).unwrap());
    let original = event(&journal, &snapshot);
    assert_eq!(original.kind, ManagedEventKind::Created);
    drop(journal);
    let journal = fixture.open();
    let snapshot = block_on(journal.inspect("child".into())).unwrap();
    let snapshot = mutate(&journal, snapshot, JournalMutation::Recover);
    assert_eq!(
        event(&journal, &snapshot).kind,
        ManagedEventKind::LifecycleChanged {
            previous: ManagedAgentState::Queued,
            current: ManagedAgentState::Interrupted,
        }
    );
    let recovered_event = snapshot.head.last_event_sequence;
    let snapshot = mutate(
        &journal,
        snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::History(
            machine_god_core::ManagedHistoryItem {
                kind: machine_god_core::ManagedHistoryKind::Conversation,
                work_id: None,
                user: None,
                assistant: Some("retained text".into()),
                user_truncated: false,
                assistant_truncated: false,
            },
        )]),
    );
    assert_eq!(snapshot.head.last_event_sequence, recovered_event);
    assert!(snapshot.head.next_sequence - 1 > recovered_event);
    let history = block_on(journal.history(snapshot, None, 100)).unwrap();
    assert!(history.records.contains(&JournalRecord::Event(original)));
}

#[test]
fn event_identity_is_exact_and_duplicate_events_cannot_share_one_page_sequence() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let original = JournalRecord::Event(event(&journal, &snapshot));
    assert_eq!(
        block_on(journal.mutate(
            snapshot.clone(),
            JournalMutation::AppendHistory(vec![original.clone()])
        ))
        .unwrap_err(),
        JournalError::Invalid
    );
    let JournalRecord::Event(mut next) = original else {
        unreachable!()
    };
    next.sequence = snapshot.head.next_sequence;
    next.revision = snapshot.head.revision + 1;
    let record = JournalRecord::Event(next);
    assert_eq!(
        block_on(journal.mutate(
            snapshot.clone(),
            JournalMutation::AppendHistory(vec![record.clone(), record])
        ))
        .unwrap_err(),
        JournalError::Invalid
    );
    assert_eq!(
        block_on(journal.inspect("child".into())).unwrap().head,
        snapshot.head
    );
}

#[test]
fn detached_and_duplicate_milestones_record_acceptance_without_inventing_notices() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let snapshot = mutate(
        &journal,
        snapshot,
        JournalMutation::HeadState {
            work_id: "work-1".into(),
            status: ManagedQueueStatus::Running,
            failure: None,
        },
    );
    let mut snapshot = mutate(&journal, snapshot, detach());
    let mut consumed = 0;
    for consume_sequence in [true, false] {
        let sequence = snapshot.head.next_sequence;
        snapshot = mutate(
            &journal,
            snapshot,
            JournalMutation::Milestone {
                operation_id: format!("milestone-{sequence}"),
                work_id: "work-1".into(),
                name: "ready".into(),
                notice: None,
                consume_sequence,
            },
        );
        assert_eq!(
            event(&journal, &snapshot).kind,
            ManagedEventKind::MilestoneRecorded {
                operation_id: format!("milestone-{sequence}"),
                source_child_id: "child".into(),
                target_parent_id: None,
                notice_emitted: false,
                work_item_id: "work-1".into(),
                name: "ready".into(),
            }
        );
        if consume_sequence {
            consumed = sequence;
        }
        assert_eq!(snapshot.head.notice_cursor, consumed);
    }
    assert!(
        block_on(journal.history(snapshot, None, 100))
            .unwrap()
            .records
            .iter()
            .all(|record| !matches!(record, JournalRecord::Notice(_)))
    );
}

#[test]
fn emitted_milestone_event_and_notice_share_one_atomic_sequence() {
    use crate::managed::notices::{
        ManagedNotice, NoticeEvent, NoticePrincipal, NoticeTarget, WorkNoticeIdentity,
    };
    use std::num::NonZeroU64;
    let nz = |value| NonZeroU64::new(value).unwrap();
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let snapshot = mutate(
        &journal,
        snapshot,
        JournalMutation::HeadState {
            work_id: "work-1".into(),
            status: ManagedQueueStatus::Running,
            failure: None,
        },
    );
    let notice = ManagedNotice {
        source: WorkNoticeIdentity {
            source: NoticePrincipal {
                id: "child".into(),
                generation: nz(1),
            },
            work_id: "work-1".into(),
            work_generation: nz(snapshot.head.revision),
        },
        source_sequence: nz(snapshot.head.next_sequence),
        target: NoticeTarget {
            parent: NoticePrincipal {
                id: "parent".into(),
                generation: nz(1),
            },
            relationship_generation: nz(snapshot.head.revision),
        },
        event: NoticeEvent::Milestone {
            name: "ready".into(),
        },
        history: None,
    };
    let snapshot = mutate(
        &journal,
        snapshot,
        JournalMutation::Milestone {
            operation_id: "milestone-call".into(),
            work_id: "work-1".into(),
            name: "ready".into(),
            notice: Some(notice.clone()),
            consume_sequence: true,
        },
    );
    let event = event(&journal, &snapshot);
    assert_eq!(event.sequence, notice.source_sequence.get());
    assert!(
        matches!(event.kind, ManagedEventKind::MilestoneRecorded { target_parent_id: Some(parent), notice_emitted: true, .. } if parent == "parent")
    );
    let page = block_on(journal.history(snapshot, None, 3)).unwrap();
    assert!(
        matches!(page.records.as_slice(), [JournalRecord::Event(_), JournalRecord::Notice(original), JournalRecord::Control(_)] if original == &notice)
    );
}
