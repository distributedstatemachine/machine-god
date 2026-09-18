use super::*;

pub(super) fn assert_matches_inventory(journal: &ManagedJournal) {
    let inventory = filesystem::scan_usage(&journal.shared.root, journal.shared.limits).unwrap();
    let state = journal.shared.state.lock().unwrap();
    assert_eq!(state.used, inventory.bytes);
    assert_eq!(state.entries, inventory.entries);
    assert_eq!(state.protected_bytes, inventory.protected_bytes);
    assert_eq!(state.protected_entries, inventory.protected_entries);
    assert_eq!(state.headroom_low_heads, inventory.headroom_low_heads);
    assert_eq!(
        state.namespace_revision,
        filesystem::directory_revision(&journal.shared.root).unwrap()
    );
}

fn append() -> JournalMutation {
    JournalMutation::AppendHistory(vec![JournalRecord::History(
        machine_god_core::ManagedHistoryItem {
            kind: machine_god_core::ManagedHistoryKind::Conversation,
            work_id: None,
            user: Some("retained history".into()),
            assistant: None,
            user_truncated: false,
            assistant_truncated: false,
        },
    )])
}

fn measured_append(journal: &ManagedJournal, snapshot: JournalSnapshot) -> JournalSnapshot {
    confirmed(
        block_on(journal.run(move |shared| {
            ACCOUNTING_WORK.set((0, 0));
            let publication = transaction::mutate(shared, snapshot, append())?;
            assert_eq!(
                ACCOUNTING_WORK.get(),
                (0, 8),
                "four names before/after; no scan"
            );
            Ok(publication)
        }))
        .unwrap(),
    )
}

#[test]
fn ordinary_accounting_touches_eight_names_independent_of_retained_history_and_heads() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    snapshot = measured_append(&journal, snapshot);
    assert_matches_inventory(&journal);
    for index in 0..16 {
        let mut idle = create(&format!("sibling-{index}"));
        idle.initial_work = None;
        confirmed(block_on(journal.create(idle)).unwrap());
    }
    for index in 0..256 {
        snapshot = measured_append(&journal, snapshot);
        if matches!(index, 0 | 15 | 127 | 255) {
            assert_matches_inventory(&journal);
        }
    }
    assert!(journal.shared.state.lock().unwrap().entries > 290);
    drop(journal);
    let reopened = fixture.open();
    assert_matches_inventory(&reopened);
    let restored = block_on(reopened.inspect("child".into())).unwrap();
    assert_eq!(restored.head, snapshot.head);
}

#[test]
fn reused_page_and_staging_orphans_have_exact_zero_or_negative_deltas() {
    for phase in [
        FailurePoint::BeforePageRename,
        FailurePoint::AfterPageRename,
        FailurePoint::BeforeHeadRename,
    ] {
        let fixture = Fixture::new();
        let journal = fixture.open();
        let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
        let entries = journal.shared.state.lock().unwrap().entries;
        *journal.shared.failure.lock().unwrap() = Some(phase);
        let JournalPublication::Ambiguous(receipt) =
            block_on(journal.mutate(snapshot.clone(), append())).unwrap()
        else {
            panic!("expected publication ambiguity");
        };
        assert!(journal.shared.state.lock().unwrap().reserved > 0);
        assert!(matches!(
            block_on(journal.mutate(snapshot.clone(), append())),
            Err(JournalError::Ambiguous)
        ));
        assert!(matches!(
            block_on(journal.reconcile(receipt)).unwrap(),
            JournalPublication::NotApplied
        ));
        assert_matches_inventory(&journal);
        // AppendHistory has no timestamped event: the retry has exactly the same
        // page/head bytes and therefore reuses the precise orphan/staging names.
        let next = measured_append(&journal, snapshot);
        assert_matches_inventory(&journal);
        assert_eq!(journal.shared.state.lock().unwrap().entries, entries + 1);
        assert_eq!(next.head.revision, 2);
    }
}

#[test]
fn uncertain_mutation_reconstructs_exactly_even_after_accounting_was_applied() {
    for phase in [
        FailurePoint::AfterHeadRename,
        FailurePoint::AfterAccountingCommit,
    ] {
        let fixture = Fixture::new();
        let journal = fixture.open();
        let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
        *journal.shared.failure.lock().unwrap() = Some(phase);
        let JournalPublication::Ambiguous(receipt) =
            block_on(journal.mutate(snapshot, append())).unwrap()
        else {
            panic!("expected publication ambiguity");
        };
        *journal.shared.failure.lock().unwrap() = Some(FailurePoint::ReconcileSync);
        assert_eq!(
            block_on(journal.reconcile(receipt.clone())).unwrap_err(),
            JournalError::Persistence
        );
        assert!(journal.pending_receipt().is_some());
        let repaired = confirmed(block_on(journal.reconcile(receipt.clone())).unwrap());
        assert_matches_inventory(&journal);
        assert_eq!(
            block_on(journal.reconcile(receipt)).unwrap_err(),
            JournalError::Conflict
        );
        measured_append(&journal, repaired);
        assert_matches_inventory(&journal);
    }
}

#[test]
fn foreign_namespace_change_fences_cached_admission_until_reopen() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let orphan = fixture.path.join("t-foreign.tmp");
    std::fs::write(&orphan, b"unexplained retained bytes").unwrap();
    std::fs::set_permissions(&orphan, std::fs::Permissions::from_mode(0o600)).unwrap();
    let entries = std::fs::read_dir(&fixture.path).unwrap().count();
    assert_eq!(
        block_on(journal.inspect("child".into())).unwrap().head,
        snapshot.head
    );
    assert!(block_on(journal.history(snapshot.clone(), None, 100)).is_ok());
    assert!(block_on(journal.read_work(snapshot.head.queue[0].page.clone())).is_ok());
    assert_eq!(
        block_on(journal.mutate(snapshot, append())).unwrap_err(),
        JournalError::Conflict
    );
    assert!(journal.pending_receipt().is_none());
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), entries);
    drop(journal);
    let reopened = fixture.open();
    assert_matches_inventory(&reopened);
    assert_eq!(reopened.shared.state.lock().unwrap().entries, entries);
    let snapshot = block_on(reopened.inspect("child".into())).unwrap();
    let snapshot = confirmed(block_on(reopened.recover(snapshot)).unwrap());
    measured_append(&reopened, snapshot);
    assert_matches_inventory(&reopened);
}

#[test]
fn changed_referenced_page_never_confirms_a_new_head() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let reference = snapshot.head.history_tail.as_ref().unwrap();
    std::fs::write(fixture.path.join(filesystem::page_name(reference)), b"{}").unwrap();
    // In-place damage leaves the namespace unchanged but exact reference
    // validation still rejects it; no head can confirm or grant execution.
    let JournalPublication::Ambiguous(receipt) =
        block_on(journal.mutate(snapshot, append())).unwrap()
    else {
        panic!("corrupt predecessor must retain publication custody");
    };
    assert!(matches!(
        block_on(journal.reconcile(receipt)).unwrap(),
        JournalPublication::NotApplied
    ));
    assert_matches_inventory(&journal);
    assert_eq!(
        block_on(journal.inspect("child".into())).unwrap_err(),
        JournalError::Invalid
    );
}

#[test]
fn historical_page_namespace_damage_preserves_bounded_reads_and_restored_history() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut idle = create("child");
    idle.initial_work = None;
    let initial = confirmed(block_on(journal.create(idle)).unwrap());
    let middle = measured_append(&journal, initial);
    let reference = middle.head.history_tail.clone().unwrap();
    let snapshot = measured_append(&journal, middle);
    let path = fixture.path.join(filesystem::page_name(&reference));
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        block_on(journal.inspect("child".into())).unwrap().head,
        snapshot.head
    );
    assert!(block_on(journal.history(snapshot.clone(), None, 1)).is_ok());
    assert_eq!(
        block_on(journal.history(snapshot.clone(), None, 100)).unwrap_err(),
        JournalError::Invalid
    );
    std::fs::write(&path, bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(block_on(journal.history(snapshot.clone(), None, 100)).is_ok());
    assert_eq!(
        block_on(journal.mutate(snapshot, append())).unwrap_err(),
        JournalError::Conflict
    );
    assert!(journal.pending_receipt().is_none());
}

#[test]
fn clearing_one_low_head_does_not_clear_another_heads_pressure() {
    use crate::managed::{notices::*, prompt_context::NoticeCheckpoint};
    use std::num::NonZeroU64;
    let nz = |value| NonZeroU64::new(value).unwrap();
    let fixture = Fixture::new();
    let journal = fixture.open();
    for id in ["first", "second"] {
        let mut idle = create(id);
        idle.initial_work = None;
        confirmed(block_on(journal.create(idle)).unwrap());
    }
    drop(journal);
    // A reopened inventory can contain heads at the exact persisted-credit
    // threshold. Keep the fixture within the validated head/aggregate bounds.
    for id in ["first", "second"] {
        let path = fixture.path.join(filesystem::head_name(id));
        let mut head: JournalHead = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        head.notice_reservations = (1..=4092).collect();
        head.next_sequence = 4093;
        std::fs::write(path, serde_json::to_vec(&head).unwrap()).unwrap();
    }
    let journal = block_on(ManagedJournal::open(
        fixture.root(),
        fixture.workers.clone(),
        JournalLimits {
            aggregate_bytes: 2 * 1024 * 1024 * 1024,
            ..JournalLimits::default()
        },
    ))
    .unwrap();
    assert_eq!(journal.shared.state.lock().unwrap().headroom_low_heads, 2);
    assert!(!journal.ordinary_publication_available());
    for (id, remaining) in [("first", 1), ("second", 0)] {
        let snapshot = block_on(journal.inspect(id.into())).unwrap();
        let acknowledgement = JournalRecord::NoticeAcknowledged {
            identity: NoticeIdentity {
                source: WorkNoticeIdentity {
                    source: NoticePrincipal {
                        id: id.into(),
                        generation: nz(1),
                    },
                    work_id: "work-1".into(),
                    work_generation: nz(1),
                },
                source_sequence: nz(1),
                kind: NoticeKind::Started,
            },
            target: NoticeTarget {
                parent: NoticePrincipal {
                    id: "parent".into(),
                    generation: nz(1),
                },
                parent_incarnation: transcript("parent").incarnation,
                relationship_generation: nz(1),
            },
            checkpoint: NoticeCheckpoint {
                session_id: transcript("parent").session_id,
                incarnation_id: transcript("parent").incarnation,
                expected_revision: machine_god_core::SessionRevision(1),
                turn_sequence: 1,
                first_user_message: 0,
            },
        };
        confirmed(
            block_on(journal.mutate(
                snapshot,
                JournalMutation::AppendHistory(vec![acknowledgement]),
            ))
            .unwrap(),
        );
        assert_matches_inventory(&journal);
        assert_eq!(
            journal.shared.state.lock().unwrap().headroom_low_heads,
            remaining
        );
        assert_eq!(journal.ordinary_publication_available(), remaining == 0);
    }
}

#[test]
fn startup_inventory_still_rejects_unrecognized_and_symlink_entries() {
    for symlink in [false, true] {
        let fixture = Fixture::new();
        if symlink {
            std::os::unix::fs::symlink("missing-target", fixture.path.join("p-foreign.json"))
                .unwrap();
        } else {
            let path = fixture.path.join("unexpected");
            std::fs::write(&path, b"unrecognized").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert_eq!(
            block_on(ManagedJournal::open(
                fixture.root(),
                fixture.workers.clone(),
                JournalLimits::default(),
            ))
            .unwrap_err(),
            JournalError::Invalid
        );
    }
}
