use super::*;

fn limited(fixture: &Fixture) -> ManagedJournal {
    block_on(ManagedJournal::open(
        fixture.root(),
        fixture.workers.clone(),
        JournalLimits {
            aggregate_bytes: 16 * 1024 * 1024,
            ..JournalLimits::default()
        },
    ))
    .unwrap()
}

fn publish(
    journal: &ManagedJournal,
    snapshot: JournalSnapshot,
    mutation: JournalMutation,
) -> JournalSnapshot {
    let snapshot = confirmed(block_on(journal.mutate(snapshot, mutation)).unwrap());
    accounting::assert_matches_inventory(journal);
    snapshot
}

fn history(bytes: usize) -> JournalRecord {
    JournalRecord::History(machine_god_core::ManagedHistoryItem {
        kind: machine_god_core::ManagedHistoryKind::Interrupted,
        work_id: Some("work-1".into()),
        user: Some("\0".repeat(bytes)),
        assistant: Some("\0".repeat(bytes)),
        user_truncated: false,
        assistant_truncated: false,
    })
}

fn fill(journal: &ManagedJournal, mut snapshot: JournalSnapshot) -> JournalSnapshot {
    let mut publications = 0;
    while journal.ordinary_publication_available() {
        snapshot = publish(
            journal,
            snapshot,
            JournalMutation::AppendHistory(vec![history(16 * 1024); 5]),
        );
        publications += 1;
        assert!(
            publications < 100,
            "fixture did not reach ordinary byte pressure"
        );
    }
    assert!(publications > 0);
    snapshot
}

#[test]
#[allow(clippy::too_many_lines)] // One exact pressure/settlement/reopen/ACK lifecycle.
fn legal_growth_protects_pending_fifo_recovery_archive_and_original_acknowledgement() {
    use crate::managed::notices::{
        ManagedNotice, NoticeEvent, NoticePrincipal, NoticeTarget, WorkNoticeIdentity,
    };
    use crate::managed::prompt_context::NoticeCheckpoint;
    use std::num::NonZeroU64;
    let nz = |value| NonZeroU64::new(value).unwrap();
    let fixture = Fixture::new();
    let journal = limited(&fixture);
    let mut snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    snapshot = publish(&journal, snapshot, JournalMutation::Enqueue(work("work-2")));
    snapshot = publish(
        &journal,
        snapshot,
        JournalMutation::HeadState {
            work_id: "work-1".into(),
            status: ManagedQueueStatus::Running,
            failure: None,
        },
    );
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
            relationship_generation: nz(snapshot.head.revision),
        },
        event: NoticeEvent::Started,
        history: None,
    };
    snapshot = publish(
        &journal,
        snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::Notice(original.clone())]),
    );
    snapshot = publish(
        &journal,
        snapshot,
        JournalMutation::Intent(JournalIntent::Archive),
    );
    snapshot = fill(&journal, snapshot);
    let protected_before_settlement = snapshot.head.cleanup_bytes;
    assert_eq!(
        snapshot.head.notice_reservations,
        vec![original.source_sequence.get()]
    );
    assert!(matches!(
        block_on(journal.mutate(snapshot.clone(), JournalMutation::Enqueue(work("rejected")))),
        Err(JournalError::Conflict)
    ));
    assert!(matches!(
        block_on(journal.mutate(snapshot.clone(), JournalMutation::Configure(config()))),
        Err(JournalError::Limit)
    ));
    assert!(journal.pending_receipt().is_none());
    assert!(matches!(
        block_on(journal.mutate(
            snapshot.clone(),
            JournalMutation::Intent(JournalIntent::Cancel)
        )),
        Err(JournalError::Limit)
    ));

    // Both notice flags may already belong to the manager at the cutoff.
    let mut originals = vec![original];
    for _ in 0..2 {
        let mut staged = originals[0].clone();
        staged.source_sequence = nz(snapshot.head.next_sequence);
        snapshot = publish(
            &journal,
            snapshot,
            JournalMutation::AppendHistory(vec![JournalRecord::Notice(staged.clone())]),
        );
        originals.push(staged);
    }

    // Two already-owned bounded writes and the largest escaped final summary.
    for _ in 0..3 {
        snapshot = publish(
            &journal,
            snapshot,
            JournalMutation::AppendHistory(vec![history(16 * 1024)]),
        );
    }
    snapshot = publish(&journal, snapshot, JournalMutation::InterruptForPressure);
    assert!(snapshot.head.cleanup_bytes < protected_before_settlement);
    assert_eq!(snapshot.head.status, ManagedAgentState::Interrupted);
    assert!(
        snapshot
            .head
            .queue
            .iter()
            .all(|work| work.status == ManagedQueueStatus::Interrupted)
    );
    let before = (snapshot.head.cleanup_bytes, snapshot.head.cleanup_entries);
    drop(journal);
    let journal = limited(&fixture);
    snapshot = block_on(journal.inspect("child".into())).unwrap();
    assert_eq!(
        (snapshot.head.cleanup_bytes, snapshot.head.cleanup_entries),
        before
    );
    assert!(!journal.ordinary_publication_available());
    snapshot = publish(&journal, snapshot, JournalMutation::Recover);
    let recovered_revision = snapshot.head.revision;
    drop(journal);
    let journal = limited(&fixture);
    snapshot = block_on(journal.inspect("child".into())).unwrap();
    assert_eq!(snapshot.head.revision, recovered_revision);
    snapshot = publish(&journal, snapshot, JournalMutation::Recover);
    snapshot = publish(&journal, snapshot, JournalMutation::Archive);
    assert_eq!(snapshot.head.cleanup_bytes, 0);
    assert_eq!(snapshot.head.cleanup_entries, 0);
    assert_eq!(snapshot.head.notice_reservations.len(), 3);

    drop(journal);
    let journal = limited(&fixture);
    snapshot = block_on(journal.inspect("child".into())).unwrap();
    assert!(snapshot.recovery_required());
    assert!(matches!(
        block_on(journal.mutate(
            snapshot.clone(),
            JournalMutation::AppendHistory(vec![history(1)])
        )),
        Err(JournalError::RecoveryRequired)
    ));

    for original in originals {
        snapshot = publish(
            &journal,
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
        );
    }
    assert!(snapshot.head.notice_reservations.is_empty());
    assert!(!snapshot.recovery_required());
    assert!(block_on(journal.history(snapshot, None, 100)).is_ok());
    assert!(block_on(journal.catalog(None, 10)).is_ok());
}

#[test]
fn idle_creation_reserves_only_maintenance_until_work_is_accepted() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut idle = create("idle");
    idle.initial_work = None;
    let snapshot = confirmed(block_on(journal.create(idle)).unwrap());
    let idle_bytes = snapshot.head.cleanup_bytes;
    assert_eq!(snapshot.head.cleanup_entries, 4);
    let snapshot = publish(&journal, snapshot, JournalMutation::Enqueue(work("later")));
    assert!(snapshot.head.cleanup_bytes > idle_bytes);
    assert_eq!(snapshot.head.cleanup_entries, 14);
}

#[test]
fn pressure_interrupts_fifo_without_inventing_user_intent_and_reopens_readably() {
    let fixture = Fixture::new();
    let journal = limited(&fixture);
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let snapshot = publish(&journal, snapshot, JournalMutation::Enqueue(work("second")));
    let snapshot = fill(&journal, snapshot);
    assert!(matches!(
        block_on(journal.mutate(snapshot.clone(), JournalMutation::Enqueue(work("rejected")))),
        Err(JournalError::Limit)
    ));
    let snapshot = publish(&journal, snapshot, JournalMutation::InterruptForPressure);
    assert!(snapshot.head.intent.is_none());
    assert!(
        snapshot
            .head
            .queue
            .iter()
            .all(|work| work.status == ManagedQueueStatus::Interrupted)
    );
    drop(journal);
    let journal = limited(&fixture);
    let snapshot = block_on(journal.inspect("child".into())).unwrap();
    let snapshot = publish(&journal, snapshot, JournalMutation::Recover);
    assert!(snapshot.head.intent.is_none());
    assert!(block_on(journal.history(snapshot, None, 100)).is_ok());
}

#[test]
fn default_capacity_keeps_many_idle_nonresident_histories_readable() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    for index in 0..70 {
        let mut idle = create(&format!("idle-{index}"));
        idle.initial_work = None;
        confirmed(block_on(journal.create(idle)).unwrap());
    }
    assert!(journal.ordinary_publication_available());
    for index in 0..70 {
        let snapshot = block_on(journal.inspect(format!("idle-{index}"))).unwrap();
        assert!(block_on(journal.history(snapshot, None, 1)).is_ok());
    }
    drop(journal);
    let journal = fixture.open();
    assert!(block_on(journal.catalog(None, 100)).is_ok());
}

#[test]
fn byte_pressure_preserves_close_of_every_accepted_fifo_successor() {
    let fixture = Fixture::new();
    let journal = limited(&fixture);
    let mut snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    for id in ["second", "third"] {
        snapshot = publish(&journal, snapshot, JournalMutation::Enqueue(work(id)));
    }
    snapshot = publish(
        &journal,
        snapshot,
        JournalMutation::Intent(JournalIntent::Archive),
    );
    snapshot = fill(&journal, snapshot);
    while let Some(first) = snapshot.head.queue.first() {
        let work_id = first.id.clone();
        snapshot = publish(
            &journal,
            snapshot,
            JournalMutation::CancelHead {
                work_id,
                notice: None,
            },
        );
    }
    snapshot = publish(&journal, snapshot, JournalMutation::Archive);
    assert_eq!(snapshot.head.status, ManagedAgentState::Archived);
    assert!(block_on(journal.history(snapshot, None, 100)).is_ok());
}

#[test]
fn legal_minimum_capacity_never_confirms_an_unreadable_journal() {
    let fixture = Fixture::new();
    let mut limits = JournalLimits::default();
    limits.aggregate_bytes = 4 * limits.head_bytes + 8 * limits.page_bytes + 1024;
    let journal = block_on(ManagedJournal::open(
        fixture.root(),
        fixture.workers.clone(),
        limits,
    ))
    .unwrap();
    let publication = block_on(journal.create(create("child")));
    match publication {
        Ok(JournalPublication::Confirmed(snapshot)) => {
            let observed = block_on(journal.inspect("child".into()))
                .expect("a confirmed publication must preserve read admission");
            assert_eq!(observed.head, snapshot.head);
            assert!(block_on(journal.history(*snapshot, None, 100)).is_ok());
        }
        Err(JournalError::Limit) => {
            assert!(journal.pending_receipt().is_none());
            assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 2);
        }
        other => panic!("unexpected capacity outcome: {other:?}"),
    }
    assert!(block_on(journal.catalog(None, 1)).is_ok());
    drop(journal);
    let reopened = block_on(ManagedJournal::open(
        fixture.root(),
        fixture.workers.clone(),
        limits,
    ))
    .unwrap();
    assert!(block_on(reopened.catalog(None, 1)).is_ok());
}
