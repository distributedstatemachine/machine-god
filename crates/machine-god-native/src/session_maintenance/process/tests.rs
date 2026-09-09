use super::*;
use crate::{NativeEnvironment, NativeSessionMigration};
use futures_executor::block_on;
use machine_god_core::{SessionRecord, SessionStore};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[test]
fn process_future_is_inert_and_precancel_does_not_capture_environment() {
    let called = Arc::new(AtomicBool::new(false));
    let observation = called.clone();
    let request = NativeSessionMaintenanceRequest::Migrate {
        session_id: SessionId::new("test").unwrap(),
    };
    drop(execute_with_environment(
        request,
        CancellationToken::new(),
        move || {
            observation.store(true, Ordering::SeqCst);
            NativeEnvironment::new(None, None, None)
        },
    ));
    assert!(!called.load(Ordering::SeqCst));
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = block_on(execute_with_environment(
        NativeSessionMaintenanceRequest::Migrate {
            session_id: SessionId::new("test").unwrap(),
        },
        cancel,
        || panic!("pre-cancel must not capture environment"),
    ));
    assert!(matches!(
        result,
        Err(NativeSessionMaintenanceError::Cancelled)
    ));
}

#[test]
fn process_factory_uses_existing_hierarchy_and_returns_migrated_record() {
    let base = std::env::temp_dir().join(random_identity("mg-session-process-").unwrap());
    fs::create_dir(&base).unwrap();
    fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
    let state = base.join(crate::STATE_NAMESPACE);
    let make_environment =
        || NativeEnvironment::new(None, Some(base.clone().into_os_string()), None);
    let environment = make_environment();
    let id = SessionId::new("migration-process").unwrap();
    let missing = block_on(execute_with_environment(
        NativeSessionMaintenanceRequest::Migrate {
            session_id: id.clone(),
        },
        CancellationToken::new(),
        move || environment,
    ));
    assert!(matches!(
        missing,
        Err(NativeSessionMaintenanceError::Missing)
    ));
    assert!(!state.exists());
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let store = FileSessionStore::open(&state).unwrap();
    let record = SessionRecord::empty(
        id.clone(),
        SessionIncarnationId::new("inc-process").unwrap(),
    );
    block_on(store.save(record, None)).unwrap();
    let environment = make_environment();
    let receipt = block_on(execute_with_environment(
        NativeSessionMaintenanceRequest::Migrate { session_id: id },
        CancellationToken::new(),
        move || environment,
    ))
    .unwrap();
    assert!(matches!(
        receipt,
        NativeSessionMaintenanceReceipt::Migration(NativeSessionMigration::Migrated(_))
    ));
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn recovery_publication_uncertainty_exposes_the_exact_separate_copy() {
    let root = std::env::temp_dir().join(random_identity("mg-session-uncertain-").unwrap());
    fs::create_dir(&root).unwrap();
    let store = FileSessionStore::open(&root).unwrap();
    let id = SessionId::new("source-uncertain").unwrap();
    block_on(store.save(
        SessionRecord::empty(id.clone(), SessionIncarnationId::new("inc-source").unwrap()),
        None,
    ))
    .unwrap();
    let original = block_on(store.load(id.clone())).unwrap().unwrap();
    let control = FileSessionScanControl {
        cancel: CancellationToken::new(),
        abandoned: CancellationToken::new(),
        after_read: None,
    };
    let receipt = FileSessionStore::maintenance_test_after_commit_failure(|| {
        dispatch(
            &store,
            NativeSessionMaintenanceRequest::Recover {
                session_id: id.clone(),
            },
            &control,
        )
    })
    .unwrap();
    let NativeSessionMaintenanceReceipt::RecoveryIndeterminate { session_id } = receipt else {
        panic!("must expose uncertain copy identity")
    };
    assert_ne!(session_id, id);
    let copy = block_on(store.load(session_id.clone())).unwrap().unwrap();
    assert_eq!(copy.id, session_id);
    assert_eq!(copy.revision, machine_god_core::SessionRevision(1));
    assert_eq!(block_on(store.load(id)).unwrap().unwrap(), original);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn join_uncertainty_retains_generated_copy_identity_without_confirming_revision() {
    let id = SessionId::new("known-copy").unwrap();
    let record = SessionRecord::empty(id.clone(), SessionIncarnationId::new("known-inc").unwrap());
    let success = NativeSessionMaintenanceReceipt::Recovery(crate::NativeSessionRecovery {
        record,
        truncated_source: false,
        unknown_tool_results: 0,
    });
    for receipt in [
        success,
        NativeSessionMaintenanceReceipt::RecoveryIndeterminate {
            session_id: id.clone(),
        },
    ] {
        let result = join_failure(Ok(Ok(receipt))).unwrap();
        assert!(
            matches!(result, NativeSessionMaintenanceReceipt::RecoveryIndeterminate {session_id} if session_id == id)
        );
    }
}
