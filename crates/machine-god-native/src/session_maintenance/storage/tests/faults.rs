use super::*;

#[test]
fn interrupted_precommit_keeps_original_and_retry_succeeds() {
    let f = Fixture::new();
    let original = record();
    let bytes = f.write(&original);
    let hook = HookGuard::install(|stage| {
        if stage == PublicationStage::BeforeCommit {
            Err(Error::Unavailable)
        } else {
            Ok(())
        }
    });
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &control()),
        Err(Error::Unavailable)
    ));
    assert_eq!(f.bytes(&original.id), bytes);
    drop(hook);
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &control()),
        Ok(NativeSessionMigration::Migrated(_))
    ));
}

#[test]
fn postcommit_failure_is_indeterminate_and_cancellation_does_not_hide_receipt() {
    let f = Fixture::new();
    let original = record();
    f.write(&original);
    let hook = HookGuard::install(|stage| {
        if stage == PublicationStage::AfterCommit {
            Err(Error::Unavailable)
        } else {
            Ok(())
        }
    });
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &control()),
        Err(Error::Indeterminate)
    ));
    assert_eq!(
        decode(&f.bytes(&original.id), &original.id)
            .unwrap()
            .revision,
        SessionRevision(4)
    );
    drop(hook);
    f.write(&original);
    let control = control();
    let cancel = control.cancel.clone();
    let _hook = HookGuard::install(move |stage| {
        if stage == PublicationStage::AfterCommit {
            cancel.cancel();
        }
        Ok(())
    });
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &control),
        Ok(NativeSessionMigration::Migrated(_))
    ));
}

#[test]
fn midread_cancellation_is_not_corruption_or_partial_publication() {
    let f = Fixture::new();
    let original = record();
    let bytes = f.write(&original);
    let mut control = control();
    let cancel = control.cancel.clone();
    control.after_read = Some(Arc::new(move |_| {
        cancel.cancel();
    }));
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &control),
        Err(Error::Cancelled)
    ));
    assert_eq!(f.bytes(&original.id), bytes);
}

#[test]
fn stale_partial_stage_does_not_block_retry_or_get_deleted() {
    let f = Fixture::new();
    let original = record();
    f.write(&original);
    let names = SessionNames::for_id(&original.id);
    let name = format!("{}.maintenance-{}.tmp", names.data, "0".repeat(32));
    fs::write(f.root.join(&name), b"interrupted").unwrap();
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &control()),
        Ok(NativeSessionMigration::Migrated(_))
    ));
    assert_eq!(fs::read(f.root.join(name)).unwrap(), b"interrupted");
}

#[test]
fn cancellation_or_changed_source_at_precommit_cannot_publish() {
    let f = Fixture::new();
    let original = record();
    let bytes = f.write(&original);
    let control = control();
    let cancel = control.cancel.clone();
    let hook = HookGuard::install(move |stage| {
        if stage == PublicationStage::BeforeCommit {
            cancel.cancel();
        }
        Ok(())
    });
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &control),
        Err(Error::Cancelled)
    ));
    assert_eq!(f.bytes(&original.id), bytes);
    drop(hook);
    let path = f.root.join(SessionNames::for_id(&original.id).data);
    let _hook = HookGuard::install(move |stage| {
        if stage == PublicationStage::BeforeCommit {
            fs::write(&path, b"new authoritative observation").unwrap();
        }
        Ok(())
    });
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &super::control()),
        Err(Error::Busy)
    ));
    assert_eq!(f.bytes(&original.id), b"new authoritative observation");
}

#[test]
fn recovery_rejects_unproven_metadata_tail_future_metadata_and_orphan_outputs() {
    let original = record();
    let bytes = serialize_record(&original).unwrap();
    assert!(recovery::decode_source(&bytes[..bytes.len() - 1], &original.id).is_err());
    let metadata_key = bytes
        .windows(10)
        .position(|window| window == b"\"metadata\"")
        .unwrap();
    assert!(recovery::decode_source(&bytes[..metadata_key + 4], &original.id).is_err());
    let mut future = original.clone();
    future.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.to_owned(),
        serde_json::json!({"schema_version":99}),
    );
    assert!(matches!(
        recovery::decode_source(&serialize_record(&future).unwrap(), &future.id),
        Err(Error::UnsupportedVersion)
    ));
    let mut orphan = original;
    orphan.messages.push(Message {
        role: Role::Tool,
        content: vec![ContentBlock::ToolResult {
            call_id: ToolCallId::new("orphan").unwrap(),
            output: machine_god_core::ToolOutput {
                content: serde_json::json!({}),
                is_error: false,
            },
        }],
    });
    assert!(matches!(
        recovery::close_tools(&mut orphan),
        Err(Error::Corrupt)
    ));
}

#[test]
fn cleanup_hardlinks_symlinks_and_wrong_mode_are_retained() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for kind in 0..3 {
        let f = Fixture::new();
        let original = record();
        let bytes = f.write(&original);
        let names = SessionNames::for_id(&original.id);
        let _lock = open_lock(f.store.root.as_fd(), &names.lock).unwrap();
        let temporary = f.root.join(&names.temp);
        match kind {
            0 => fs::hard_link(f.root.join(&names.data), &temporary).unwrap(),
            1 => symlink(f.root.join(&names.data), &temporary).unwrap(),
            _ => {
                fs::write(&temporary, &bytes).unwrap();
                fs::set_permissions(&temporary, fs::Permissions::from_mode(0o644)).unwrap();
            }
        }
        let report = f
            .store
            .maintenance_cleanup(CleanupMode::Apply, &control())
            .unwrap();
        assert_eq!(report.outcomes, vec![Status::Untrusted]);
        assert!(fs::symlink_metadata(temporary).is_ok());
        assert_eq!(f.bytes(&original.id), bytes);
    }
}

#[test]
fn cleanup_reports_post_unlink_uncertainty() {
    let f = Fixture::new();
    let original = record();
    let bytes = f.write(&original);
    let names = SessionNames::for_id(&original.id);
    let _lock = open_lock(f.store.root.as_fd(), &names.lock).unwrap();
    let staging = create_new_temp(f.store.root.as_fd(), &names.temp).unwrap();
    write_all(&staging, &bytes).unwrap();
    let _hook = HookGuard::install(|stage| {
        if stage == PublicationStage::AfterCleanup {
            Err(Error::Unavailable)
        } else {
            Ok(())
        }
    });
    let report = f
        .store
        .maintenance_cleanup(CleanupMode::Apply, &control())
        .unwrap();
    assert_eq!(report.outcomes, vec![Status::Indeterminate]);
    assert!(!f.root.join(&names.temp).exists());
    assert_eq!(f.bytes(&original.id), bytes);
}

#[test]
fn cleanup_retains_future_native_metadata_even_when_staging_is_redundant() {
    let f = Fixture::new();
    let mut original = record();
    original.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.to_owned(),
        serde_json::json!({"schema_version":99}),
    );
    let bytes = f.write(&original);
    let names = SessionNames::for_id(&original.id);
    let _lock = open_lock(f.store.root.as_fd(), &names.lock).unwrap();
    let staging = create_new_temp(f.store.root.as_fd(), &names.temp).unwrap();
    write_all(&staging, &bytes).unwrap();
    let report = f
        .store
        .maintenance_cleanup(CleanupMode::Apply, &control())
        .unwrap();
    assert_eq!(report.outcomes, vec![Status::Untrusted]);
    assert_eq!(fs::read(f.root.join(&names.temp)).unwrap(), bytes);
}

#[test]
fn recovered_copy_really_resumes_without_provider_or_tool_execution() {
    use machine_god_testkit::{ScriptedModelProvider, ScriptedPermissionHandler};
    let f = Fixture::new();
    let original = record();
    f.write(&original);
    let recovered = recover(&f, &original.id).unwrap();
    let provider = ScriptedModelProvider::new("test", []);
    let engine = machine_god_core::Engine::builder()
        .provider(provider.clone())
        .shared_session_store(f.store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let lifecycle = crate::NativeSessionLifecycle::new(engine, f.store.clone()).unwrap();
    let session = block_on(lifecycle.resume(recovered.record.id)).unwrap();
    let _conversation = crate::NativeConversation::from_session(session).unwrap();
    assert!(provider.requests().is_empty());
}
