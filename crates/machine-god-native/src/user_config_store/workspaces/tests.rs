use super::*;
use crate::{
    NativeConfiguredPermissionMutation, NativeConfiguredPermissionScope, NativeModelPreferences,
};
use futures_executor::block_on;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "mg-workspace-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        Self(base)
    }
    fn root(&self) -> PathBuf {
        self.0.join("config")
    }
    fn store(&self) -> NativeUserConfigStore {
        NativeUserConfigStore::new(self.root())
    }
    fn legacy(&self) -> Vec<u8> {
        fs::create_dir(self.root()).unwrap();
        fs::set_permissions(self.root(), fs::Permissions::from_mode(0o700)).unwrap();
        let bytes = br#"{"schema_version":1,"permission_mode":"ask"}"#.to_vec();
        fs::write(self.root().join(DATA), &bytes).unwrap();
        bytes
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn record(path: &[u8]) -> NativeSavedWorkspaceDirectory {
    NativeSavedWorkspaceDirectory::new(path, path, true).unwrap()
}
fn add(path: &[u8]) -> NativeWorkspaceDirectoryMutation {
    NativeWorkspaceDirectoryMutation::Add(record(path))
}
fn edit(store: &NativeUserConfigStore, path: &[u8]) -> NativeUserWorkspaceCommit {
    block_on(store.apply_workspace_directory_mutation(b"/work", &add(path), &[])).unwrap()
}

fn provisional_record() -> NativeSavedWorkspaceDirectory {
    NativeSavedWorkspaceDirectory::new(b"/source", b"/pending", false).unwrap()
}

fn fifteen_saved_with_provisional(store: &NativeUserConfigStore) {
    block_on(store.apply_workspace_directory_mutation(
        b"/work",
        &NativeWorkspaceDirectoryMutation::Add(provisional_record()),
        &[],
    ))
    .unwrap();
    for index in 0..14 {
        edit(store, format!("/other-{index}").as_bytes());
    }
}

#[test]
fn observed_capacity_counts_exact_provisional_alias_once_without_persisting_proof() {
    let fixture = Fixture::new();
    let store = fixture.store();
    fifteen_saved_with_provisional(&store);
    let aliases = [WorkspaceDirectoryAlias::new(provisional_record(), b"/actual").unwrap()];
    let launch = [b"/actual".to_vec()];
    assert!(matches!(
        block_on(store.apply_workspace_directory_mutation(b"/work", &add(b"/last"), &launch)),
        Err(NativeUserConfigError::InvalidConfig(_))
    ));
    let result = store
        .publish_workspace_mutation_observed(
            b"/work",
            &add(b"/last"),
            || {},
            |root| rustix::fs::fsync(root),
            &launch,
            &aliases,
        )
        .unwrap();
    assert!(result.changed);
    assert_eq!(result.after.len(), 16);
    assert_eq!(result.after[0], provisional_record());
    assert_eq!(
        store
            .load()
            .unwrap()
            .loaded()
            .config()
            .saved_workspace_directories(b"/work")
            .unwrap(),
        result.after
    );
}

#[test]
fn locked_latest_replacement_cannot_borrow_removed_records_alias() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let other = fixture.store();
    fifteen_saved_with_provisional(&store);
    let aliases = [WorkspaceDirectoryAlias::new(provisional_record(), b"/actual").unwrap()];
    let replacement =
        NativeSavedWorkspaceDirectory::new(b"/new-source", b"/pending", false).unwrap();
    let result = store.publish_workspace_mutation_observed(
        b"/work",
        &add(b"/last"),
        || {
            block_on(other.apply_workspace_directory_mutation(
                b"/work",
                &NativeWorkspaceDirectoryMutation::Remove(b"/pending".to_vec()),
                &[],
            ))
            .unwrap();
            block_on(other.apply_workspace_directory_mutation(
                b"/work",
                &NativeWorkspaceDirectoryMutation::Add(replacement.clone()),
                &[],
            ))
            .unwrap();
        },
        |root| rustix::fs::fsync(root),
        &[b"/actual".to_vec()],
        &aliases,
    );
    assert!(matches!(
        result,
        Err(NativeUserConfigError::InvalidConfig(_))
    ));
    let observed = store.load().unwrap();
    let saved = observed
        .loaded()
        .config()
        .saved_workspace_directories(b"/work")
        .unwrap();
    assert_eq!(saved.len(), 15);
    assert!(saved.contains(&replacement));
    assert!(!saved.contains(&record(b"/last")));
    assert!(!fixture.root().join(TEMP).exists());
}

#[test]
fn invalid_aliases_reject_before_store_path_resolution() {
    let store = NativeUserConfigStore::new(PathBuf::from("relative-invalid-root"));
    let alias = WorkspaceDirectoryAlias::new(provisional_record(), b"/actual").unwrap();
    for aliases in [vec![alias.clone(); 17], vec![alias; 2]] {
        assert_eq!(
            store
                .publish_workspace_mutation_observed(
                    b"/work",
                    &add(b"/last"),
                    || panic!("no lock"),
                    |_| panic!("no sync"),
                    &[],
                    &aliases
                )
                .unwrap_err(),
            NativeUserConfigError::Conflict
        );
    }
    assert!(WorkspaceDirectoryAlias::new(provisional_record(), b"/../invalid").is_err());
    assert_eq!(
        WorkspaceDirectoryAlias::new(record(b"/fixed"), b"/different").unwrap_err(),
        NativeUserConfigError::Conflict
    );
}

#[test]
fn observed_noop_does_not_upgrade_or_rewrite_provisional_record() {
    let fixture = Fixture::new();
    let store = fixture.store();
    fifteen_saved_with_provisional(&store);
    let original = fs::read(fixture.root().join(DATA)).unwrap();
    let aliases = [WorkspaceDirectoryAlias::new(provisional_record(), b"/actual").unwrap()];
    let result = store
        .publish_workspace_mutation_observed(
            b"/work",
            &NativeWorkspaceDirectoryMutation::Add(provisional_record()),
            || panic!("no lock"),
            |_| panic!("no sync"),
            &[b"/actual".to_vec()],
            &aliases,
        )
        .unwrap();
    assert!(!result.changed);
    assert_eq!(fs::read(fixture.root().join(DATA)).unwrap(), original);
}

#[test]
fn inert_future_and_missing_observational_noops_create_nothing() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mutation = add(b"/extra");
    drop(store.apply_workspace_directory_mutation(b"/work", &mutation, &[]));
    assert!(!fixture.root().exists());
    for mutation in [
        NativeWorkspaceDirectoryMutation::Clear,
        NativeWorkspaceDirectoryMutation::Remove(b"/missing".to_vec()),
    ] {
        let receipt =
            block_on(store.apply_workspace_directory_mutation(b"/work", &mutation, &[])).unwrap();
        assert!(!receipt.changed);
        assert!(receipt.before.is_empty());
        assert!(receipt.after.is_empty());
        assert_eq!(
            receipt.loaded.origin(),
            crate::ConfigOrigin::BuiltInDefaults
        );
        assert!(!fixture.root().exists());
    }
}

#[test]
fn legacy_noop_preserves_bytes_schema_and_absent_lock() {
    let fixture = Fixture::new();
    let original = fixture.legacy();
    let store = fixture.store();
    let receipt = block_on(store.apply_workspace_directory_mutation(
        b"/work",
        &NativeWorkspaceDirectoryMutation::Clear,
        &[],
    ))
    .unwrap();
    assert!(!receipt.changed);
    assert_eq!(receipt.loaded.config().schema_version(), 1);
    assert_eq!(fs::read(fixture.root().join(DATA)).unwrap(), original);
    assert!(!fixture.root().join(LOCK).exists());
    assert!(!fixture.root().join(TEMP).exists());
}

#[test]
fn invalid_request_rejects_before_even_opening_store() {
    let store = NativeUserConfigStore::new(PathBuf::from("relative-invalid-root"));
    for (primary, mutation) in [
        (
            b"relative".as_slice(),
            NativeWorkspaceDirectoryMutation::Clear,
        ),
        (
            b"/work".as_slice(),
            NativeWorkspaceDirectoryMutation::Remove(b"/../escape".to_vec()),
        ),
        (b"/work".as_slice(), add(b"/work")),
        (
            b"/work".as_slice(),
            NativeWorkspaceDirectoryMutation::Remove(vec![b'x'; 4097]),
        ),
    ] {
        assert!(matches!(
            block_on(store.apply_workspace_directory_mutation(primary, &mutation, &[])),
            Err(NativeUserConfigError::InvalidConfig(_))
        ));
    }
}

#[test]
fn stale_preflight_preserves_concurrent_unrelated_model_and_permission_updates() {
    let fixture = Fixture::new();
    fixture.legacy();
    let store = fixture.store();
    let other = fixture.store();
    let preferences = NativeModelPreferences::new(
        "concurrent/model",
        crate::NativeReasoningEffort::default(),
        true,
    )
    .unwrap();
    let receipt = store
        .publish_workspace_mutation(
            b"/work",
            &add(b"/extra"),
            || {
                block_on(other.set_model_preferences(&other.load().unwrap(), &preferences))
                    .unwrap();
                block_on(other.apply_permission_mutation(
                    &other.load().unwrap(),
                    Path::new("/work"),
                    NativeConfiguredPermissionScope::Local,
                    &NativeConfiguredPermissionMutation::Add {
                        permission: "bash".into(),
                        pattern: "git *".into(),
                    },
                ))
                .unwrap();
            },
            |root| rustix::fs::fsync(root),
            &[],
        )
        .unwrap();
    assert!(receipt.changed);
    assert!(receipt.before.is_empty());
    assert_eq!(receipt.after, vec![record(b"/extra")]);
    assert_eq!(receipt.loaded.config().model_preferences(), preferences);
    assert_eq!(
        receipt
            .loaded
            .config()
            .permission_sources(Path::new("/work"))
            .unwrap()
            .effective()
            .rules()[0]
            .pattern(),
        "git *"
    );
    assert_eq!(store.load().unwrap().loaded(), &receipt.loaded);
}

#[test]
fn stale_preflight_merges_latest_same_list_in_durable_order() {
    let fixture = Fixture::new();
    let store = fixture.store();
    edit(&store, b"/first");
    let other = fixture.store();
    let receipt = store
        .publish_workspace_mutation(
            b"/work",
            &add(b"/last"),
            || {
                edit(&other, b"/middle");
                block_on(other.apply_workspace_directory_mutation(
                    b"/another",
                    &add(b"/elsewhere"),
                    &[],
                ))
                .unwrap();
            },
            |root| rustix::fs::fsync(root),
            &[],
        )
        .unwrap();
    assert_eq!(receipt.before, vec![record(b"/first"), record(b"/middle")]);
    assert_eq!(
        receipt.after,
        vec![record(b"/first"), record(b"/middle"), record(b"/last")]
    );
    assert_eq!(
        receipt
            .loaded
            .config()
            .saved_workspace_directories(b"/another")
            .unwrap(),
        &[record(b"/elsewhere")]
    );
}

#[test]
fn raced_noop_after_changed_preflight_does_not_rewrite_or_create_temp() {
    let fixture = Fixture::new();
    fixture.legacy();
    let store = fixture.store();
    let mutation = add(b"/extra");
    let (candidate, _) = store
        .load()
        .unwrap()
        .loaded()
        .config()
        .with_workspace_directory_mutation(b"/work", &mutation)
        .unwrap();
    let mut published = candidate.serialize_current().unwrap();
    published.push(b'\n');
    let receipt = store
        .publish_workspace_mutation(
            b"/work",
            &mutation,
            || {
                // Deterministically publish between observation and lock acquisition.
                fs::write(fixture.root().join(DATA), &published).unwrap();
            },
            |_| panic!("no-op must not publish"),
            &[],
        )
        .unwrap();
    assert!(!receipt.changed);
    assert_eq!(receipt.before, receipt.after);
    assert_eq!(fs::read(fixture.root().join(DATA)).unwrap(), published);
    assert!(fixture.root().join(LOCK).exists());
    assert!(!fixture.root().join(TEMP).exists());
}

#[test]
fn ambiguous_postrename_receipt_retains_previous_and_intended_sets() {
    let fixture = Fixture::new();
    let store = fixture.store();
    edit(&store, b"/before");
    let receipt = store
        .publish_workspace_mutation(
            b"/work",
            &add(b"/after"),
            || {},
            |_| Err(rustix::io::Errno::IO),
            &[],
        )
        .unwrap();
    assert!(receipt.changed);
    assert_eq!(
        receipt.durability,
        NativeWorkspaceCommitDurability::Ambiguous
    );
    assert_eq!(receipt.before, vec![record(b"/before")]);
    assert_eq!(receipt.after, vec![record(b"/before"), record(b"/after")]);
    assert_eq!(store.load().unwrap().loaded(), &receipt.loaded);
    assert!(!fixture.root().join(TEMP).exists());
    // Scope exit released the lock even on the ambiguous return.
    edit(&store, b"/third");
}

#[test]
fn exact_remove_clear_and_duplicate_preserve_other_scopes() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let original =
        NativeSavedWorkspaceDirectory::new(b"/source-\xff", b"/identity-\xfe", false).unwrap();
    block_on(store.apply_workspace_directory_mutation(
        b"/work",
        &NativeWorkspaceDirectoryMutation::Add(original.clone()),
        &[],
    ))
    .unwrap();
    block_on(store.apply_workspace_directory_mutation(b"/other", &add(b"/keep"), &[])).unwrap();
    let unknown = block_on(store.apply_workspace_directory_mutation(
        b"/work",
        &NativeWorkspaceDirectoryMutation::Remove(original.source_bytes().to_vec()),
        &[],
    ))
    .unwrap();
    assert!(!unknown.changed);
    let duplicate = block_on(store.apply_workspace_directory_mutation(
        b"/work",
        &NativeWorkspaceDirectoryMutation::Add(original.clone()),
        &[],
    ))
    .unwrap();
    assert!(!duplicate.changed);
    let removed = block_on(store.apply_workspace_directory_mutation(
        b"/work",
        &NativeWorkspaceDirectoryMutation::Remove(original.identity_bytes().to_vec()),
        &[],
    ))
    .unwrap();
    assert_eq!(removed.before, vec![original]);
    assert!(removed.after.is_empty());
    assert_eq!(
        removed
            .loaded
            .config()
            .saved_workspace_directories(b"/other")
            .unwrap(),
        &[record(b"/keep")]
    );
    let clear = block_on(store.apply_workspace_directory_mutation(
        b"/other",
        &NativeWorkspaceDirectoryMutation::Clear,
        &[],
    ))
    .unwrap();
    assert!(clear.changed);
    assert_eq!(clear.before, vec![record(b"/keep")]);
    assert!(clear.after.is_empty());
}

#[test]
fn existing_cas_routes_preserve_directories_and_still_reject_stale_tokens() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let stale = store.load().unwrap();
    edit(&store, b"/extra");
    let preferences =
        NativeModelPreferences::new("next/model", crate::NativeReasoningEffort::default(), false)
            .unwrap();
    assert_eq!(
        block_on(store.set_model_preferences(&stale, &preferences)).unwrap_err(),
        NativeUserConfigError::Conflict
    );
    let permission = NativeConfiguredPermissionMutation::Add {
        permission: "bash".into(),
        pattern: "git *".into(),
    };
    assert_eq!(
        block_on(store.apply_permission_mutation(
            &stale,
            Path::new("/work"),
            NativeConfiguredPermissionScope::User,
            &permission
        ))
        .unwrap_err(),
        NativeUserConfigError::Conflict
    );
    block_on(store.set_model_preferences(&store.load().unwrap(), &preferences)).unwrap();
    block_on(store.apply_permission_mutation(
        &store.load().unwrap(),
        Path::new("/work"),
        NativeConfiguredPermissionScope::User,
        &permission,
    ))
    .unwrap();
    assert_eq!(
        store
            .load()
            .unwrap()
            .loaded()
            .config()
            .saved_workspace_directories(b"/work")
            .unwrap(),
        &[record(b"/extra")]
    );
}

#[test]
fn busy_lock_is_not_retried_and_foreign_temp_is_not_removed() {
    let fixture = Fixture::new();
    let original = fixture.legacy();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let root = snapshot.root.as_ref().unwrap();
    let lock = open_lock(root).unwrap();
    let guard = lock_config(&lock).unwrap();
    assert_eq!(
        block_on(store.apply_workspace_directory_mutation(b"/work", &add(b"/extra"), &[]))
            .unwrap_err(),
        NativeUserConfigError::Busy
    );
    drop(guard);
    fs::write(fixture.root().join(TEMP), b"owned elsewhere").unwrap();
    assert_eq!(
        block_on(store.apply_workspace_directory_mutation(b"/work", &add(b"/extra"), &[]))
            .unwrap_err(),
        NativeUserConfigError::Persistence
    );
    assert_eq!(
        fs::read(fixture.root().join(TEMP)).unwrap(),
        b"owned elsewhere"
    );
    assert_eq!(fs::read(fixture.root().join(DATA)).unwrap(), original);
}

#[test]
fn replaced_root_and_symlink_lock_do_not_redirect_publication() {
    let fixture = Fixture::new();
    let original = fixture.legacy();
    let store = fixture.store();
    let moved = fixture.0.join("moved");
    assert_eq!(
        store
            .publish_workspace_mutation(
                b"/work",
                &add(b"/extra"),
                || {
                    fs::rename(fixture.root(), &moved).unwrap();
                    fs::create_dir(fixture.root()).unwrap();
                    fs::set_permissions(fixture.root(), fs::Permissions::from_mode(0o700)).unwrap();
                },
                |root| rustix::fs::fsync(root),
                &[]
            )
            .unwrap_err(),
        NativeUserConfigError::Conflict
    );
    assert_eq!(fs::read(moved.join(DATA)).unwrap(), original);
    symlink(moved.join(DATA), fixture.root().join(LOCK)).unwrap();
    assert_eq!(
        block_on(store.apply_workspace_directory_mutation(b"/work", &add(b"/extra"), &[]))
            .unwrap_err(),
        NativeUserConfigError::UnsafePath
    );
    assert_eq!(fs::read(moved.join(DATA)).unwrap(), original);
}

#[test]
fn malformed_and_oversized_latest_configs_are_not_overwritten() {
    for bytes in [
        b"not-json".to_vec(),
        vec![b' '; crate::MAX_CONFIG_BYTES + 1],
    ] {
        let fixture = Fixture::new();
        fixture.legacy();
        fs::write(fixture.root().join(DATA), &bytes).unwrap();
        let store = fixture.store();
        assert!(matches!(
            block_on(store.apply_workspace_directory_mutation(b"/work", &add(b"/extra"), &[])),
            Err(NativeUserConfigError::InvalidConfig(_))
        ));
        assert_eq!(fs::read(fixture.root().join(DATA)).unwrap(), bytes);
        assert!(!fixture.root().join(LOCK).exists());
    }
}

#[test]
fn launch_inputs_are_validated_before_effects_and_overlap_counts_once() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for launches in [
        vec![b"/work".to_vec()],
        vec![b"relative".to_vec()],
        vec![b"/nul\0x".to_vec()],
        vec![b"/same".to_vec(), b"/same".to_vec()],
        vec![vec![b'x'; 4097]],
        (0..17)
            .map(|index| format!("/launch-{index}").into_bytes())
            .collect(),
    ] {
        assert!(matches!(
            block_on(store.apply_workspace_directory_mutation(
                b"/work",
                &add(b"/extra"),
                &launches
            )),
            Err(NativeUserConfigError::InvalidConfig(_))
        ));
        assert!(!fixture.root().exists());
    }
    let launches = (0..16)
        .map(|index| format!("/launch-{index}").into_bytes())
        .collect::<Vec<_>>();
    let receipt =
        block_on(store.apply_workspace_directory_mutation(b"/work", &add(&launches[0]), &launches))
            .unwrap();
    assert!(receipt.changed);
    assert_eq!(receipt.after, vec![record(&launches[0])]);
}

#[test]
fn latest_saved_add_plus_retained_launch_overflow_is_rejected_before_publication() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for index in 0..14 {
        edit(&store, format!("/saved-{index}").as_bytes());
    }
    let other = fixture.store();
    let mut latest_bytes = Vec::new();
    let result = store.publish_workspace_mutation(
        b"/work",
        &add(b"/requested"),
        || {
            edit(&other, b"/concurrent");
            latest_bytes = fs::read(fixture.root().join(DATA)).unwrap();
        },
        |_| panic!("overflow must not publish"),
        &[b"/launch-only".to_vec()],
    );
    assert!(
        matches!(result, Err(NativeUserConfigError::InvalidConfig(error)) if error.kind() == crate::NativeConfigErrorKind::TooLarge)
    );
    assert_eq!(fs::read(fixture.root().join(DATA)).unwrap(), latest_bytes);
    assert!(!fixture.root().join(TEMP).exists());
    let observed = store.load().unwrap();
    let saved = observed
        .loaded()
        .config()
        .saved_workspace_directories(b"/work")
        .unwrap();
    assert_eq!(saved.len(), 15);
    assert_eq!(saved.last().unwrap().identity_bytes(), b"/concurrent");
}
