use super::*;

fn open(fixture: &Fixture, entries: usize) -> ManagedJournal {
    block_on(ManagedJournal::open(
        fixture.root(),
        fixture.workers.clone(),
        JournalLimits {
            directory_entries: entries,
            ..JournalLimits::default()
        },
    ))
    .unwrap()
}

#[test]
fn entry_pressure_rejects_before_publication_and_preserves_readable_reopen() {
    let fixture = Fixture::new();
    // Fourteen durable credits, four retained files, and two staging spares.
    let journal = open(&fixture, 20);
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 4);
    assert!(matches!(
        block_on(journal.mutate(snapshot.clone(), detach())),
        Err(JournalError::Limit)
    ));
    assert!(journal.pending_receipt().is_none());
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 4);
    assert_eq!(
        block_on(journal.inspect("child".into()))
            .unwrap()
            .head
            .revision,
        snapshot.head.revision
    );
    assert!(
        !block_on(journal.history(snapshot, None, 100))
            .unwrap()
            .records
            .is_empty()
    );
    drop(journal);
    let reopened = open(&fixture, 20);
    assert!(block_on(reopened.inspect("child".into())).is_ok());
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 4);
}

#[test]
fn creation_reserves_epoch_replacement_headroom() {
    let fixture = Fixture::new();
    let journal = open(&fixture, 4);
    assert!(matches!(
        block_on(journal.create(create("child"))),
        Err(JournalError::Limit)
    ));
    assert!(journal.pending_receipt().is_none());
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 2);
    drop(journal);
    drop(open(&fixture, 4));
}

#[test]
fn failed_publication_accounts_for_orphans_before_next_reservation() {
    let fixture = Fixture::new();
    let journal = open(&fixture, 22);
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    *journal.shared.failure.lock().unwrap() = Some(FailurePoint::AfterPageRename);
    let JournalPublication::Ambiguous(receipt) =
        block_on(journal.mutate(snapshot, detach())).unwrap()
    else {
        panic!("injected ambiguity");
    };
    assert!(matches!(
        block_on(journal.reconcile(receipt)).unwrap(),
        JournalPublication::NotApplied
    ));
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 5);
    let original = block_on(journal.inspect("child".into())).unwrap();
    assert!(matches!(
        block_on(journal.mutate(original, detach())),
        Err(JournalError::Limit)
    ));
    assert!(journal.pending_receipt().is_none());
    drop(journal);
    assert!(block_on(open(&fixture, 22).inspect("child".into())).is_ok());
}

#[test]
fn abandoned_head_staging_retains_room_for_reopening() {
    let fixture = Fixture::new();
    let journal = open(&fixture, 22);
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    *journal.shared.failure.lock().unwrap() = Some(FailurePoint::BeforeHeadRename);
    assert!(matches!(
        block_on(journal.mutate(snapshot, detach())).unwrap(),
        JournalPublication::Ambiguous(_)
    ));
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 6);
    drop(journal);
    let reopened = open(&fixture, 22);
    assert!(block_on(reopened.inspect("child".into())).is_ok());
}

#[test]
#[allow(clippy::too_many_lines)] // Exercise every protected publication and exact ACK at the entry boundary.
fn exact_settlement_entry_credits_leave_exhausted_journal_readable() {
    use crate::managed::notices::{
        ManagedNotice, NoticeEvent, NoticePrincipal, NoticeTarget, WorkNoticeIdentity,
    };
    use crate::managed::prompt_context::NoticeCheckpoint;
    use std::num::NonZeroU64;
    let nz = |value| NonZeroU64::new(value).unwrap();
    let fixture = Fixture::new();
    let mut journal = open(&fixture, 20);
    let mut snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    assert!(!journal.ordinary_publication_available());
    let mut originals = Vec::new();
    // Four already-owned notice obligations consume four publication credits
    // and transfer four independent future ACK credits into exact identities.
    for _ in 0..4 {
        let original = ManagedNotice {
            source: WorkNoticeIdentity {
                source: NoticePrincipal {
                    id: "child".into(),
                    generation: nz(1),
                },
                work_id: "work-1".into(),
                work_generation: nz(1),
            },
            source_sequence: nz(snapshot.head.next_sequence),
            target: NoticeTarget {
                parent: NoticePrincipal {
                    id: "parent".into(),
                    generation: nz(1),
                },
                parent_incarnation: transcript("parent").incarnation,
                relationship_generation: nz(1),
            },
            event: NoticeEvent::Started,
            history: None,
        };
        snapshot = confirmed(
            block_on(journal.mutate(
                snapshot,
                JournalMutation::AppendHistory(vec![JournalRecord::Notice(original.clone())]),
            ))
            .unwrap(),
        );
        originals.push(original);
        accounting::assert_matches_inventory(&journal);
    }
    // Six other bounded owned writes use the rest of the ten-publication pool.
    for _ in 0..4 {
        let sequence = snapshot.head.next_sequence;
        snapshot = confirmed(
            block_on(journal.mutate(snapshot, JournalMutation::SuppressedNotice(sequence)))
                .unwrap(),
        );
    }
    snapshot = confirmed(
        block_on(journal.mutate(snapshot, JournalMutation::InterruptForPressure)).unwrap(),
    );
    let interrupted_revision = snapshot.head.revision;
    drop(journal);
    journal = open(&fixture, 20);
    snapshot = block_on(journal.inspect("child".into())).unwrap();
    assert_eq!(snapshot.head.revision, interrupted_revision);
    snapshot = confirmed(block_on(journal.mutate(snapshot, JournalMutation::Recover)).unwrap());
    assert_eq!(snapshot.head.cleanup_entries, 0);
    accounting::assert_matches_inventory(&journal);
    assert_eq!(snapshot.head.notice_reservations.len(), 4);
    for original in originals {
        snapshot = confirmed(
            block_on(journal.mutate(
                snapshot,
                JournalMutation::AppendHistory(vec![JournalRecord::NoticeAcknowledged {
                    identity: original.identity(),
                    target: original.target,
                    checkpoint: NoticeCheckpoint {
                        session_id: transcript("parent").session_id,
                        incarnation_id: transcript("parent").incarnation,
                        expected_revision: machine_god_core::SessionRevision::default(),
                        turn_sequence: 1,
                        first_user_message: 0,
                    },
                }]),
            ))
            .unwrap(),
        );
        accounting::assert_matches_inventory(&journal);
    }
    assert!(snapshot.head.notice_reservations.is_empty());
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 18);
    assert!(matches!(
        block_on(journal.mutate(
            snapshot.clone(),
            JournalMutation::Intent(JournalIntent::Archive)
        )),
        Err(JournalError::Limit)
    ));
    assert!(journal.pending_receipt().is_none());
    assert!(block_on(journal.history(snapshot, None, 100)).is_ok());
    drop(journal);
    let journal = open(&fixture, 20);
    let snapshot = block_on(journal.inspect("child".into())).unwrap();
    assert!(matches!(
        block_on(journal.mutate(snapshot.clone(), JournalMutation::Recover)),
        Err(JournalError::Limit)
    ));
    assert!(block_on(journal.history(snapshot, None, 100)).is_ok());
    assert!(block_on(journal.catalog(None, 10)).is_ok());
}
