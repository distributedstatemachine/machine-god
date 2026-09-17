use super::*;
use crate::managed::notices::{
    ManagedNotice, NoticeEvent, NoticePrincipal, NoticeTarget, NoticeTerminal, WorkNoticeIdentity,
};
use std::num::NonZeroU64;

fn original(snapshot: &JournalSnapshot) -> ManagedNotice {
    let nz = |value| NonZeroU64::new(value).unwrap();
    ManagedNotice {
        source: WorkNoticeIdentity {
            source: NoticePrincipal {
                id: snapshot.head.id.clone(),
                generation: nz(snapshot.head.generation),
            },
            work_id: snapshot.head.queue[0].id.clone(),
            work_generation: nz(1),
        },
        source_sequence: nz(snapshot.head.next_sequence),
        target: NoticeTarget {
            parent: NoticePrincipal {
                id: "parent".into(),
                generation: nz(1),
            },
            parent_incarnation: SessionIncarnationId::new("incarnation").unwrap(),
            relationship_generation: nz(snapshot.head.revision),
        },
        event: NoticeEvent::Terminal {
            outcome: NoticeTerminal::Cancelled,
        },
        history: None,
    }
}

fn originals(journal: &ManagedJournal, snapshot: &JournalSnapshot) -> Vec<ManagedNotice> {
    block_on(journal.history(snapshot.clone(), None, 100))
        .unwrap()
        .records
        .into_iter()
        .filter_map(|record| match record {
            JournalRecord::Notice(notice) => Some(notice),
            _ => None,
        })
        .collect()
}

#[test]
fn cancellation_notice_and_fifo_removal_share_exact_ambiguity_custody() {
    for point in [
        FailurePoint::BeforePageRename,
        FailurePoint::AfterPageRename,
        FailurePoint::BeforeHeadRename,
        FailurePoint::AfterHeadRename,
    ] {
        let fixture = Fixture::new();
        let journal = fixture.open();
        let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
        let snapshot = mutate(
            &journal,
            snapshot,
            JournalMutation::Intent(JournalIntent::Cancel),
        );
        let notice = original(&snapshot);
        let mutation = JournalMutation::CancelHead {
            work_id: "work-1".into(),
            notice: Some(notice.clone()),
        };
        *journal.shared.failure.lock().unwrap() = Some(point);
        let JournalPublication::Ambiguous(receipt) =
            block_on(journal.mutate(snapshot.clone(), mutation.clone())).unwrap()
        else {
            panic!("exact cancellation candidate must remain ambiguous")
        };
        *journal.shared.failure.lock().unwrap() = Some(FailurePoint::ReconcileSync);
        assert_eq!(
            block_on(journal.reconcile(receipt.clone())).unwrap_err(),
            JournalError::Persistence
        );
        assert!(journal.pending_receipt().is_some());
        let settled = match block_on(journal.reconcile(receipt)).unwrap() {
            JournalPublication::Confirmed(snapshot) => {
                assert_eq!(point, FailurePoint::AfterHeadRename);
                *snapshot
            }
            JournalPublication::NotApplied => {
                assert_ne!(point, FailurePoint::AfterHeadRename);
                let unchanged = block_on(journal.inspect("child".into())).unwrap();
                assert_eq!(unchanged.head, snapshot.head);
                assert!(originals(&journal, &unchanged).is_empty());
                confirmed(block_on(journal.mutate(unchanged, mutation)).unwrap())
            }
            JournalPublication::Ambiguous(_) => panic!("reconciliation did not settle"),
        };
        assert!(settled.head.queue.is_empty());
        assert_eq!(settled.head.intent, None);
        assert_eq!(settled.head.notice_cursor, notice.source_sequence.get());
        assert_eq!(originals(&journal, &settled), std::slice::from_ref(&notice));
        // No runtime or in-memory tracker is needed to recover the same original.
        drop(journal);
        let journal = fixture.open();
        let reopened = block_on(journal.inspect("child".into())).unwrap();
        assert!(reopened.head.queue.is_empty());
        assert_eq!(originals(&journal, &reopened), [notice]);
    }
}

#[test]
fn cancellation_rejects_a_retargeted_original_without_removing_work() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let snapshot = mutate(
        &journal,
        snapshot,
        JournalMutation::Intent(JournalIntent::Cancel),
    );
    for changed in 0..4 {
        let mut notice = original(&snapshot);
        match changed {
            0 => notice.target.parent.generation = NonZeroU64::new(2).unwrap(),
            1 => notice.target.parent_incarnation = SessionIncarnationId::new("foreign").unwrap(),
            2 => notice.target.relationship_generation = NonZeroU64::new(99).unwrap(),
            _ => notice.source.work_id = "other-work".into(),
        }
        assert_eq!(
            block_on(journal.mutate(
                snapshot.clone(),
                JournalMutation::CancelHead {
                    work_id: "work-1".into(),
                    notice: Some(notice),
                }
            ))
            .unwrap_err(),
            JournalError::Invalid
        );
        assert_eq!(
            block_on(journal.inspect("child".into())).unwrap().head,
            snapshot.head
        );
    }
}
