use super::tests::Temp;
use super::*;
use machine_god_core::{SessionId, SessionIncarnationId};
use rustix::fd::AsFd;
use std::{fs, sync::mpsc};

fn owner(name: &str) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new(name).unwrap(),
        SessionIncarnationId::new(name).unwrap(),
    )
}
fn tracker(budget: &Arc<NativeUndoBudget>, name: &str) -> Arc<FileUndoTracker> {
    Arc::new(FileUndoTracker::for_principal(budget.clone(), owner(name), 1).unwrap())
}
fn commit(tracker: &FileUndoTracker, temp: &Temp, name: &str, bytes: &[u8]) {
    let root = fs::File::open(&temp.0).unwrap();
    let mut transaction = tracker
        .begin(
            root.as_fd(),
            Operation::Replace(name),
            &CancellationToken::new(),
        )
        .unwrap();
    fs::write(temp.0.join(name), bytes).unwrap();
    let published = fs::File::open(temp.0.join(name)).unwrap();
    transaction.committed(Some(published.as_fd()));
}

#[test]
fn shared_undo_budget_validates_limits_and_exact_principal_generation() {
    assert_eq!(
        FileUndoError::ResourceLimit.tool().code,
        "file_undo_resource_limit"
    );
    let defaults = NativeUndoLimits::default();
    for limits in [
        NativeUndoLimits {
            max_entries: 0,
            ..defaults
        },
        NativeUndoLimits {
            max_bytes: 0,
            ..defaults
        },
        NativeUndoLimits {
            max_descriptors: 0,
            ..defaults
        },
    ] {
        assert_eq!(
            NativeUndoBudget::new(limits).unwrap_err(),
            FileUndoError::ResourceLimit
        );
    }
    let budget = Arc::new(NativeUndoBudget::default());
    assert!(FileUndoTracker::for_principal(budget.clone(), owner("a"), 0).is_err());
    let a = tracker(&budget, "a");
    assert_eq!(a.check_principal(&owner("a"), 1), Ok(()));
    assert_eq!(
        a.check_principal(&owner("b"), 1),
        Err(FileUndoError::Rejected)
    );
    assert_eq!(
        a.check_principal(&owner("a"), 2),
        Err(FileUndoError::Rejected)
    );
    assert_eq!(budget.usage(), NativeUndoUsage::default());
}

#[test]
fn shared_undo_budget_counts_inflight_before_opens_without_global_effect_lock() {
    let temp = Temp::new();
    let budget = Arc::new(
        NativeUndoBudget::new(NativeUndoLimits {
            max_entries: 1,
            ..NativeUndoLimits::default()
        })
        .unwrap(),
    );
    let a = tracker(&budget, "a");
    let b = tracker(&budget, "b");
    std::thread::scope(|scope| {
        let (started, observed) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let temp = &temp;
        let a = &a;
        let worker = scope.spawn(move || {
            let root = fs::File::open(&temp.0).unwrap();
            let _transaction = a
                .begin(
                    root.as_fd(),
                    Operation::Replace("absent"),
                    &CancellationToken::new(),
                )
                .unwrap();
            started.send(()).unwrap();
            let _ = released.recv();
        });
        observed.recv().unwrap();
        assert_eq!(budget.usage().entries, 1);
        assert_eq!(budget.usage().bytes, budget::OPERATION_BYTES);
        // Invalid root/path must not be examined under aggregate pressure.
        let root = fs::File::open("/dev/null").unwrap();
        assert!(matches!(
            b.begin(
                root.as_fd(),
                Operation::Replace("absent"),
                &CancellationToken::new()
            ),
            Err(FileUndoError::ResourceLimit)
        ));
        assert_eq!(b.clear(), Ok(())); // sibling lock remains independent
        drop(release);
        worker.join().unwrap();
    });
    assert_eq!(budget.usage(), NativeUndoUsage::default());
    assert!(!temp.0.join("absent").exists());
}

#[test]
fn shared_undo_histories_clear_and_inverse_only_the_selected_principal() {
    let temp = Temp::new();
    let budget = Arc::new(NativeUndoBudget::default());
    let a = tracker(&budget, "a");
    let b = tracker(&budget, "b");
    fs::write(temp.0.join("a"), b"old a").unwrap();
    fs::write(temp.0.join("b"), b"old b").unwrap();
    commit(&a, &temp, "a", b"new a");
    let a_charge = budget.usage();
    assert_eq!(a_charge.entries, 1);
    assert!(a_charge.bytes > 0 && a_charge.bytes < budget::OPERATION_BYTES);
    assert_eq!(a_charge.descriptors, 4);
    commit(&b, &temp, "b", b"new b");
    assert_eq!(budget.usage().entries, 2);
    a.reserve_clear().unwrap().commit();
    assert_eq!(budget.usage().entries, 1);
    assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"new a");
    assert_eq!(
        b.undo_last(&CancellationToken::new()),
        Ok(FileUndoOutcome::Restored("b".into()))
    );
    assert_eq!(fs::read(temp.0.join("b")).unwrap(), b"old b");
    assert_eq!(budget.usage(), NativeUndoUsage::default());
}

#[test]
fn shared_undo_ambiguity_retains_preimage_and_descriptor_charges_until_clear() {
    let temp = Temp::new();
    let budget = Arc::new(NativeUndoBudget::default());
    let a = tracker(&budget, "a");
    fs::write(temp.0.join("a"), b"preimage").unwrap();
    let root = fs::File::open(&temp.0).unwrap();
    let mut transaction = a
        .begin(
            root.as_fd(),
            Operation::Replace("a"),
            &CancellationToken::new(),
        )
        .unwrap();
    transaction.uncertain();
    drop(transaction);
    assert_eq!(budget.usage().entries, 1);
    assert!(budget.usage().bytes >= b"preimage".len());
    assert_eq!(budget.usage().descriptors, 3);
    assert_eq!(
        a.undo_last(&CancellationToken::new()),
        Err(FileUndoError::Ambiguous)
    );
    let clear = a.reserve_clear().unwrap();
    drop(clear);
    assert_eq!(budget.usage().entries, 1);
    a.reserve_clear().unwrap().commit();
    assert_eq!(budget.usage(), NativeUndoUsage::default());
}

#[test]
fn shared_undo_each_aggregate_limit_rejects_without_eviction_or_mutation() {
    let temp = Temp::new();
    for limits in [
        NativeUndoLimits {
            max_entries: 1,
            ..NativeUndoLimits::default()
        },
        NativeUndoLimits {
            max_bytes: budget::OPERATION_BYTES,
            ..NativeUndoLimits::default()
        },
        NativeUndoLimits {
            max_descriptors: budget::OPERATION_DESCRIPTORS,
            ..NativeUndoLimits::default()
        },
    ] {
        let budget = Arc::new(NativeUndoBudget::new(limits).unwrap());
        let a = tracker(&budget, "a");
        let b = tracker(&budget, "b");
        commit(&a, &temp, "a", b"original");
        let retained = budget.usage();
        let root = fs::File::open(&temp.0).unwrap();
        assert!(matches!(
            b.begin(
                root.as_fd(),
                Operation::Replace("untouched"),
                &CancellationToken::new()
            ),
            Err(FileUndoError::ResourceLimit)
        ));
        assert_eq!(budget.usage(), retained);
        assert!(!temp.0.join("untouched").exists());
        a.clear().unwrap();
        assert_eq!(budget.usage(), NativeUndoUsage::default());
    }
}

#[test]
fn shared_undo_owned_transaction_unwind_refunds_uncommitted_resources() {
    let temp = Temp::new();
    let budget = Arc::new(NativeUndoBudget::default());
    let a = tracker(&budget, "a");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let root = fs::File::open(&temp.0).unwrap();
        let _transaction = a
            .begin(
                root.as_fd(),
                Operation::Replace("absent"),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(budget.usage().entries, 1);
        panic!("injected before dispatch");
    }));
    assert!(result.is_err());
    assert_eq!(budget.usage(), NativeUndoUsage::default());
}

#[test]
fn shared_undo_eviction_and_tracker_drop_never_refund_a_siblings_history() {
    let temp = Temp::new();
    let budget = Arc::new(
        NativeUndoBudget::new(NativeUndoLimits {
            max_entries: MAX_FILE_UNDO_ENTRIES + 2,
            ..NativeUndoLimits::default()
        })
        .unwrap(),
    );
    let a = tracker(&budget, "a");
    let b = tracker(&budget, "b");
    commit(&b, &temp, "b", b"sibling");
    let sibling = budget.usage();
    for _ in 0..=MAX_FILE_UNDO_ENTRIES {
        commit(&a, &temp, "a", b"bounded own history");
    }
    assert_eq!(budget.usage().entries, MAX_FILE_UNDO_ENTRIES + 1);
    drop(a);
    assert_eq!(budget.usage(), sibling);
    b.undo_last(&CancellationToken::new()).unwrap();
    assert!(!temp.0.join("b").exists());
    assert_eq!(budget.usage(), NativeUndoUsage::default());
}
