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
    let journal = open(&fixture, 5);
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
    let reopened = open(&fixture, 5);
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
    let journal = open(&fixture, 7);
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
    assert!(block_on(open(&fixture, 7).inspect("child".into())).is_ok());
}

#[test]
fn abandoned_head_staging_retains_room_for_reopening() {
    let fixture = Fixture::new();
    let journal = open(&fixture, 7);
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    *journal.shared.failure.lock().unwrap() = Some(FailurePoint::BeforeHeadRename);
    assert!(matches!(
        block_on(journal.mutate(snapshot, detach())).unwrap(),
        JournalPublication::Ambiguous(_)
    ));
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 6);
    drop(journal);
    let reopened = open(&fixture, 7);
    assert!(block_on(reopened.inspect("child".into())).is_ok());
}
