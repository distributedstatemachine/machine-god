use super::*;
use crate::{
    ConfigOrigin, NativeConfiguredPermissionMutation as PermissionMutation,
    NativeConfiguredPermissionReset, NativeConfiguredPermissionScope, NativeModelPreferences,
    NativeSavedWorkspaceDirectory, NativeUserConfigStore,
    NativeWorkspaceDirectoryMutation as Mutation,
};
use futures_executor::block_on;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    base: PathBuf,
    ancestor: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "mg-config-parents-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let ancestor = base.join("existing-parent");
        fs::create_dir(&ancestor).unwrap();
        fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o755)).unwrap();
        Self { base, ancestor }
    }
    fn root(&self) -> PathBuf {
        self.ancestor.join("missing-config/nested/machine-god")
    }
    fn store(&self) -> NativeUserConfigStore {
        NativeUserConfigStore::new(self.root())
    }
    fn missing(&self) -> PathBuf {
        self.ancestor.join("missing-config")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}
fn add() -> Mutation {
    Mutation::Add(NativeSavedWorkspaceDirectory::new(b"/extra", b"/extra", true).unwrap())
}
fn preferences() -> NativeModelPreferences {
    NativeModelPreferences::new("test/first", crate::NativeReasoningEffort::default(), false)
        .unwrap()
}

#[test]
fn missing_ancestors_load_defaults_and_noops_never_create_any_directory() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    assert_eq!(snapshot.loaded().origin(), ConfigOrigin::BuiltInDefaults);
    for mutation in [Mutation::Clear, Mutation::Remove(b"/unknown".to_vec())] {
        assert!(
            !block_on(store.apply_workspace_directory_mutation(b"/work", &mutation, &[]))
                .unwrap()
                .changed
        );
    }
    block_on(store.apply_permission_mutation(
        &snapshot,
        Path::new("/work"),
        NativeConfiguredPermissionScope::User,
        &PermissionMutation::Reset(NativeConfiguredPermissionReset::All),
    ))
    .unwrap();
    drop(store.set_model_preferences(&snapshot, &preferences()));
    assert!(!fixture.missing().exists());
}

#[test]
fn first_workspace_add_creates_only_required_private_namespace_under_existing_0755_parent() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let receipt =
        block_on(store.apply_workspace_directory_mutation(b"/work", &add(), &[])).unwrap();
    assert!(receipt.changed);
    assert_eq!(
        receipt.durability,
        crate::NativeWorkspaceCommitDurability::Confirmed
    );
    for path in [
        fixture.missing(),
        fixture.missing().join("nested"),
        fixture.root(),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert_eq!(
        fs::metadata(&fixture.ancestor)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(store.load().unwrap().loaded(), &receipt.loaded);
}

#[test]
fn first_model_and_permission_publications_use_same_missing_parent_authority() {
    for model in [true, false] {
        let fixture = Fixture::new();
        let store = fixture.store();
        let snapshot = store.load().unwrap();
        if model {
            block_on(store.set_model_preferences(&snapshot, &preferences())).unwrap();
            assert_eq!(
                store.load().unwrap().loaded().config().model_preferences(),
                preferences()
            );
        } else {
            let mutation = PermissionMutation::Add {
                permission: "bash".into(),
                pattern: "git *".into(),
            };
            block_on(store.apply_permission_mutation(
                &snapshot,
                Path::new("/work"),
                NativeConfiguredPermissionScope::User,
                &mutation,
            ))
            .unwrap();
            assert_eq!(
                store
                    .load()
                    .unwrap()
                    .loaded()
                    .config()
                    .permission_rules()
                    .rules()[0]
                    .pattern(),
                "git *"
            );
        }
        assert!(fixture.root().join("config.json").is_file());
    }
}

#[test]
fn invalid_and_oversized_proposals_do_not_create_missing_parent() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let oversized = PermissionMutation::Add {
        permission: "bash".into(),
        pattern: "x".repeat(crate::MAX_CONFIG_BYTES + 1),
    };
    assert!(
        block_on(store.apply_permission_mutation(
            &snapshot,
            Path::new("/work"),
            NativeConfiguredPermissionScope::User,
            &oversized
        ))
        .is_err()
    );
    assert!(block_on(store.apply_workspace_directory_mutation(b"relative", &add(), &[])).is_err());
    let too_deep =
        NativeUserConfigStore::new(fixture.ancestor.join("a/".repeat(65)).join("config"));
    assert!(matches!(too_deep.load(), Err(Error::UnsafePath)));
    assert!(!fixture.missing().exists());
    assert!(!fixture.ancestor.join("a").exists());
}

#[test]
fn replaced_observed_ancestor_rejects_before_new_namespace_creation() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let moved = fixture.base.join("moved");
    fs::rename(&fixture.ancestor, &moved).unwrap();
    fs::create_dir(&fixture.ancestor).unwrap();
    assert!(matches!(
        block_on(store.set_model_preferences(&snapshot, &preferences())),
        Err(Error::Conflict)
    ));
    assert!(!fixture.missing().exists());
    assert!(!moved.join("missing-config").exists());
}

#[test]
fn dangling_and_injected_parent_symlinks_are_never_adopted() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let outside = fixture.base.join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, fixture.missing()).unwrap();
    assert!(matches!(
        block_on(store.set_model_preferences(&snapshot, &preferences())),
        Err(Error::UnsafePath)
    ));
    assert!(!outside.join("nested").exists());
    fs::remove_file(fixture.missing()).unwrap();
    std::os::unix::fs::symlink(fixture.base.join("absent"), fixture.missing()).unwrap();
    assert!(matches!(store.load(), Err(Error::UnsafePath)));
}

#[test]
fn publication_after_intermediate_replacement_is_ambiguous_not_rolled_back() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let moved = fixture.ancestor.join("moved");
    let result = store.publish_preferences(&snapshot, &preferences(), |root| {
        fs::rename(fixture.missing(), &moved).unwrap();
        fs::create_dir(fixture.missing()).unwrap();
        rustix::fs::fsync(root)
    });
    assert!(matches!(result, Err(Error::CommitAmbiguous)));
    assert!(moved.join("nested/machine-god/config.json").is_file());
    assert!(!fixture.missing().join("nested").exists());
}

#[test]
fn failed_ancestor_sync_reports_failure_and_retains_created_directory_truthfully() {
    let fixture = Fixture::new();
    let observed = ParentObservation::observe(fixture.root().parent().unwrap()).unwrap();
    let result = observed.resolve_with_sync(true, |_| Err(rustix::io::Errno::IO));
    assert!(matches!(result, Err(Error::Persistence)));
    assert!(fixture.missing().is_dir());
    assert!(!fixture.missing().join("nested").exists());
    assert!(!fixture.root().exists());
}
