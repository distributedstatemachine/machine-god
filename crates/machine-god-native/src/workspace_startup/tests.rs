use super::*;
use crate::{NativeEnvironment, NativeSavedWorkspaceDirectory, NativeWorkspaceDirectoryMutation};
use futures_executor::block_on;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: PathBuf,
    primary: PathBuf,
    selection: NativeRootSelection,
    store: Arc<NativeUserConfigStore>,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "machine-god-workspace-startup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let primary = base.join("primary");
        std::fs::create_dir(&primary).unwrap();
        let environment = NativeEnvironment::new(
            Some(base.as_os_str().to_owned()),
            Some(base.join("absent-state-parent").into_os_string()),
            None,
        );
        let selection = NativeRootSelection::from_environment(&environment, &primary).unwrap();
        let store = Arc::new(NativeUserConfigStore::new(base.join("config")));
        Self {
            base,
            primary,
            selection,
            store,
        }
    }

    fn directory(&self, name: &str) -> PathBuf {
        let path = self.base.join(name);
        std::fs::create_dir(&path).unwrap();
        path
    }

    fn save(&self, source: &Path, identity: &Path, canonical: bool) {
        let record = NativeSavedWorkspaceDirectory::new(
            source.as_os_str().as_bytes(),
            identity.as_os_str().as_bytes(),
            canonical,
        )
        .unwrap();
        block_on(self.store.apply_workspace_directory_mutation(
            self.primary.as_os_str().as_bytes(),
            &NativeWorkspaceDirectoryMutation::Add(record),
            &[],
        ))
        .unwrap();
    }

    fn prepare(
        &self,
        launch: &[PathBuf],
        suppressed: bool,
    ) -> Result<NativeWorkspaceAuthority, Error> {
        prepare_workspace_blocking(&self.selection, &self.store, launch, suppressed)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}

#[test]
fn startup_reads_without_creating_configuration_or_state() {
    let fixture = Fixture::new();
    let authority = fixture.prepare(&[], false).unwrap();
    let snapshot = authority.snapshot().unwrap();
    assert_eq!(snapshot.primary_identity(), fixture.primary);
    assert!(snapshot.entries().is_empty());
    assert!(!fixture.base.join("config").exists());
    assert!(!fixture.base.join("absent-state-parent").exists());
}

#[test]
fn launch_relative_and_symlink_paths_merge_with_saved_provenance() {
    let fixture = Fixture::new();
    let additional = fixture.directory("additional");
    let alias = fixture.base.join("alias");
    std::os::unix::fs::symlink(&additional, &alias).unwrap();
    fixture.save(&alias, &additional, true);
    let launch = [PathBuf::from("../alias"), additional.clone()];
    let authority = fixture.prepare(&launch, true).unwrap();
    let snapshot = authority.snapshot().unwrap();
    assert!(snapshot.saved_suppressed());
    assert_eq!(snapshot.entries().len(), 1);
    let entry = &snapshot.entries()[0];
    assert!(entry.saved() && entry.launch() && entry.active() && entry.available());
    assert_eq!(entry.source().source(), alias);
    assert_eq!(entry.source().identity(), additional);
    assert!(snapshot.route(&additional.join("file")).is_ok());
}

#[test]
fn provisional_saved_alias_merges_with_first_canonical_launch_observation() {
    let fixture = Fixture::new();
    let alias = fixture.base.join("future-alias");
    fixture.save(&alias, &alias, false);
    let additional = fixture.directory("additional");
    std::os::unix::fs::symlink(&additional, &alias).unwrap();
    let snapshot = fixture
        .prepare(std::slice::from_ref(&additional), false)
        .unwrap()
        .snapshot()
        .unwrap();
    assert_eq!(snapshot.entries().len(), 1);
    let entry = &snapshot.entries()[0];
    assert!(entry.active() && entry.saved() && entry.launch());
    assert_eq!(entry.source().source(), alias);
    assert_eq!(entry.source().identity(), additional);
}

#[test]
fn missing_saved_roots_remain_visible_but_missing_launch_roots_fail() {
    let fixture = Fixture::new();
    let missing = fixture.base.join("missing");
    fixture.save(&missing, &missing, false);
    let snapshot = fixture.prepare(&[], false).unwrap().snapshot().unwrap();
    assert_eq!(snapshot.entries().len(), 1);
    assert!(!snapshot.entries()[0].available());
    assert!(!snapshot.entries()[0].active());
    assert!(matches!(
        fixture.prepare(&[missing], false),
        Err(Error::InvalidPath)
    ));
}

#[test]
fn rejects_state_overlap_files_and_excessive_arguments_without_publication() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .prepare(std::slice::from_ref(&fixture.base), false)
            .is_err()
    );
    let file = fixture.base.join("file");
    std::fs::write(&file, b"not a directory").unwrap();
    assert!(matches!(
        fixture.prepare(&[file], false),
        Err(Error::InvalidPath)
    ));
    let repeated = vec![PathBuf::from("unused"); MAX_WORKSPACE_LAUNCH_ARGUMENTS + 1];
    assert!(matches!(
        fixture.prepare(&repeated, false),
        Err(Error::InvalidPath)
    ));
    assert!(!fixture.base.join("config").exists());
    assert!(!fixture.selection.state_root().exists());
}

#[test]
fn owned_startup_is_inert_before_poll_and_settles_its_scope() {
    let fixture = Fixture::new();
    let workers = NativeOwnedWorkerScope::default();
    let completion = workers.completion();
    let prepare = || {
        prepare_native_workspace(
            fixture.selection.clone(),
            Arc::clone(&fixture.store),
            vec![],
            false,
            workers.clone(),
        )
    };
    drop(prepare());
    assert!(!fixture.base.join("config").exists());
    let authority = block_on(prepare()).unwrap();
    assert_eq!(
        authority.snapshot().unwrap().primary_identity(),
        fixture.primary
    );
    workers.close();
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
}
