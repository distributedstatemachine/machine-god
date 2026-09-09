use super::*;
use crate::{
    NativeSavedWorkspaceDirectory as Saved, NativeWorkspaceDirectoryMutation as Mutation,
    NativeWorkspaceEntrySpec as Spec, NativeWorkspaceSource as Source,
};
use futures_executor::block_on;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::AtomicU64;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    base: PathBuf,
    primary: PathBuf,
    store: Arc<NativeUserConfigStore>,
    workers: NativeOwnedWorkerScope,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "machine-god-workspace-service-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let primary = base.join("primary");
        std::fs::create_dir(&primary).unwrap();
        Self {
            store: Arc::new(NativeUserConfigStore::new(base.join("config"))),
            workers: NativeOwnedWorkerScope::new(),
            base,
            primary,
        }
    }
    fn directory(&self, name: &str) -> PathBuf {
        let path = self.base.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn service(&self, specs: Vec<Spec>, suppressed: bool) -> Arc<NativeWorkspaceService> {
        let fd = rustix::fs::open(
            &self.primary,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        let authority = NativeWorkspaceAuthority::open_blocking(
            fd,
            self.primary.clone(),
            None,
            self.base.join("state"),
            specs,
            suppressed,
        )
        .unwrap();
        Arc::new(NativeWorkspaceService::new(
            authority,
            Arc::clone(&self.store),
            self.workers.clone(),
        ))
    }
    fn saved(&self) -> Vec<Saved> {
        self.store
            .load()
            .unwrap()
            .loaded()
            .config()
            .saved_workspace_directories(self.primary.as_os_str().as_bytes())
            .unwrap()
            .to_vec()
    }
    fn save(&self, path: &Path) {
        block_on(self.store.apply_workspace_directory_mutation(
            self.primary.as_os_str().as_bytes(),
            &Mutation::Add(saved(path)),
            &[],
        ))
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.workers.close();
        self.workers.completion().wait_on_worker().unwrap();
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}
fn saved(path: &Path) -> Saved {
    Saved::new(
        path.as_os_str().as_bytes(),
        path.as_os_str().as_bytes(),
        true,
    )
    .unwrap()
}
fn spec(path: &Path, saved: bool, launch: bool) -> Spec {
    Spec::new(
        Source::new(path.to_path_buf(), path.to_path_buf(), true).unwrap(),
        saved,
        launch,
    )
    .unwrap()
}
fn apply(
    service: &Arc<NativeWorkspaceService>,
    action: NativeWorkspaceAction,
) -> NativeWorkspaceReceipt {
    block_on(service.execute(action)).unwrap()
}

#[test]
fn add_promotion_remove_clear_and_suppression_preserve_provenance() {
    let fixture = Fixture::new();
    let first = fixture.directory("first");
    let second = fixture.directory("second");
    let service = fixture.service(vec![spec(&first, false, true)], true);
    let old = service.authority.snapshot().unwrap();
    let added = apply(
        &service,
        NativeWorkspaceAction::Add(PathBuf::from("../first")),
    );
    assert_eq!(added.saved_changed, Some(true));
    assert!(
        added.snapshot.entries()[0].saved()
            && added.snapshot.entries()[0].launch()
            && added.snapshot.entries()[0].active()
    );
    let duplicate = apply(&service, NativeWorkspaceAction::Add(first.clone()));
    assert_eq!(duplicate.saved_changed, Some(false));
    assert_eq!(duplicate.runtime_changed, Some(false));
    let added = apply(&service, NativeWorkspaceAction::Add(second.clone()));
    assert!(!added.snapshot.entries()[1].active());
    let removed = apply(&service, NativeWorkspaceAction::Remove(first.clone()));
    assert!(removed.launch_flag_can_restore);
    assert_eq!(fixture.saved(), vec![saved(&second)]);
    assert!(old.route(&first.join("retained")).is_ok());
    assert!(removed.snapshot.route(&first.join("gone")).is_err());
    let cleared = apply(&service, NativeWorkspaceAction::Clear);
    assert!(cleared.snapshot.entries().is_empty() && cleared.snapshot.saved_suppressed());
    assert!(fixture.saved().is_empty());
}

#[test]
fn missing_and_retargeted_saved_source_is_removable_without_following_it() {
    let fixture = Fixture::new();
    let original = fixture.directory("original");
    let target = fixture.directory("target");
    let link = fixture.base.join("link");
    std::os::unix::fs::symlink(&original, &link).unwrap();
    let saved = Saved::new(
        link.as_os_str().as_bytes(),
        original.as_os_str().as_bytes(),
        true,
    )
    .unwrap();
    block_on(fixture.store.apply_workspace_directory_mutation(
        fixture.primary.as_os_str().as_bytes(),
        &Mutation::Add(saved),
        &[],
    ))
    .unwrap();
    let source = Source::new(link.clone(), original.clone(), true).unwrap();
    let service = fixture.service(vec![Spec::new(source, true, false).unwrap()], false);
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    std::fs::remove_dir(&original).unwrap();
    let listed = apply(&service, NativeWorkspaceAction::List);
    assert!(!listed.snapshot.entries()[0].available());
    let removed = apply(&service, NativeWorkspaceAction::Remove(link));
    assert_eq!(removed.saved_changed, Some(true));
    assert!(removed.snapshot.entries().is_empty());
    assert!(target.is_dir());
}

#[test]
fn launch_only_clear_and_empty_list_do_not_create_config() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let service = fixture.service(vec![spec(&extra, false, true)], false);
    let listed = apply(&service, NativeWorkspaceAction::List);
    assert_eq!(
        listed.reconciliation,
        NativeWorkspaceReconciliation::Refreshed
    );
    let cleared = apply(&service, NativeWorkspaceAction::Clear);
    assert_eq!(cleared.saved_changed, Some(false));
    assert_eq!(cleared.runtime_changed, Some(true));
    assert!(cleared.launch_flag_can_restore);
    assert!(!fixture.base.join("config").exists());
}

#[test]
fn unpolled_future_has_no_lane_or_disk_effects() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let service = fixture.service(vec![], false);
    let future = service.execute(NativeWorkspaceAction::Add(extra));
    assert!(!service.active.load(Ordering::Acquire));
    assert!(!fixture.base.join("config").exists());
    drop(future);
    assert!(
        apply(&service, NativeWorkspaceAction::List)
            .snapshot
            .entries()
            .is_empty()
    );
}

#[test]
fn latest_concurrent_config_is_merged_and_confirmed_reload_preserves_external_roots() {
    let fixture = Fixture::new();
    let first = fixture.directory("first");
    let other = fixture.directory("other");
    let service = fixture.service(vec![], false);
    let store = Arc::clone(&fixture.store);
    let primary = fixture.primary.clone();
    let other_saved = saved(&other);
    let hook: Hook = Arc::new(move |stage| {
        if stage == Stage::BeforeCommit {
            block_on(store.apply_workspace_directory_mutation(
                primary.as_os_str().as_bytes(),
                &Mutation::Add(other_saved.clone()),
                &[],
            ))
            .unwrap();
        }
    });
    let receipt = block_on(service.execute_inner(
        None,
        NativeWorkspaceAction::Add(first.clone()),
        Some(hook),
    ))
    .unwrap();
    assert_eq!(fixture.saved(), vec![saved(&other), saved(&first)]);
    assert_eq!(receipt.snapshot.entries().len(), 2);
    assert_eq!(
        receipt.reconciliation,
        NativeWorkspaceReconciliation::Confirmed
    );
}

#[test]
fn confirmed_saved_receipt_survives_reload_failure_without_runtime_rollback_claim() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let service = fixture.service(vec![], false);
    let config = fixture.base.join("config/config.json");
    let hook: Hook = Arc::new(move |stage| {
        if stage == Stage::AfterCommit {
            std::fs::write(&config, b"invalid").unwrap();
        }
    });
    let receipt =
        block_on(service.execute_inner(None, NativeWorkspaceAction::Add(extra), Some(hook)))
            .unwrap();
    assert_eq!(receipt.saved_changed, Some(true));
    assert_eq!(receipt.runtime_changed, Some(false));
    assert!(matches!(
        receipt.reconciliation,
        NativeWorkspaceReconciliation::ReloadFailed(_)
    ));
    assert!(receipt.snapshot.entries().is_empty());
}

fn runtime() -> Arc<NativeConversationRuntime> {
    use machine_god_core::{
        Engine, SessionId, SessionIncarnationId, SessionRecord, SessionRevision,
    };
    use machine_god_testkit::{
        InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler, SessionStoreScript,
    };
    let mut record = SessionRecord::empty(
        SessionId::new("workspace-session").unwrap(),
        SessionIncarnationId::new("workspace-life").unwrap(),
    );
    record.revision = SessionRevision(1);
    record.metadata.insert(
        crate::NATIVE_SESSION_METADATA_KEY.to_owned(),
        crate::NativeSessionMetadata::default().to_value(),
    );
    let id = record.id.clone();
    let store = InMemorySessionStore::configured(
        std::collections::BTreeMap::from([(id.clone(), record)]),
        SessionStoreScript::default(),
        256,
    );
    let engine = Engine::builder()
        .session_store(store)
        .provider(ScriptedModelProvider::new("test", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(id)).unwrap().unwrap();
    Arc::new(
        NativeConversationRuntime::new(
            crate::NativeConversation::from_session(session).unwrap(),
            crate::NativeModelPreferences::new(
                "test/model",
                crate::NativeReasoningEffort::parse("high").unwrap(),
                false,
            )
            .unwrap(),
            None,
        )
        .unwrap(),
    )
}

#[test]
fn active_and_queued_runtime_reject_mutations_and_return_cached_list_without_io() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let service = fixture.service(vec![], false);
    let runtime = runtime();
    let lease = runtime.acquire_workspace_control().unwrap();
    assert!(matches!(
        block_on(service.execute_for_runtime(
            Arc::clone(&runtime),
            NativeWorkspaceAction::Add(extra.clone())
        )),
        Err(NativeWorkspaceServiceError::Busy)
    ));
    let cached =
        block_on(service.execute_for_runtime(Arc::clone(&runtime), NativeWorkspaceAction::List))
            .unwrap();
    assert_eq!(
        cached.reconciliation,
        NativeWorkspaceReconciliation::CachedBusy
    );
    assert_eq!(cached.snapshot.generation(), 0);
    drop(lease);
    runtime.enqueue("queued".into()).unwrap();
    assert!(matches!(
        block_on(
            service.execute_for_runtime(Arc::clone(&runtime), NativeWorkspaceAction::Add(extra))
        ),
        Err(NativeWorkspaceServiceError::Busy)
    ));
    assert_eq!(
        block_on(service.execute_for_runtime(runtime, NativeWorkspaceAction::List))
            .unwrap()
            .reconciliation,
        NativeWorkspaceReconciliation::CachedBusy
    );
    assert!(!fixture.base.join("config").exists());
}

#[test]
fn dropped_response_retains_runtime_and_lane_through_worker_cleanup() {
    use std::task::{Context, Poll};
    use std::time::Duration;
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let service = fixture.service(vec![], false);
    let runtime = runtime();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let hook: Hook = Arc::new(move |stage| {
        if stage == Stage::BeforeCleanup {
            entered_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
    });
    let mut future = service.execute_inner(
        Some(Arc::clone(&runtime)),
        NativeWorkspaceAction::Add(extra.clone()),
        Some(hook),
    );
    let waker = futures_util::task::noop_waker();
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    drop(future);
    fixture.workers.close();
    assert!(!fixture.workers.completion().is_complete());
    assert!(runtime.acquire_workspace_control().is_err());
    assert!(matches!(
        block_on(service.execute(NativeWorkspaceAction::Clear)),
        Err(NativeWorkspaceServiceError::Busy)
    ));
    assert_eq!(fixture.saved(), vec![saved(&extra)]);
    release_tx.send(()).unwrap();
    fixture.workers.completion().wait_on_worker().unwrap();
    assert!(runtime.acquire_workspace_control().is_ok());
    assert!(!service.active.load(Ordering::Acquire));
    assert!(
        service
            .authority
            .snapshot()
            .unwrap()
            .route(&extra.join("file"))
            .is_ok()
    );
}

#[test]
fn ambiguous_reconciliation_accepts_only_intended_or_before_and_never_claims_durability() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let other = fixture.directory("other");
    let service = fixture.service(vec![], false);
    let previous = service.authority.snapshot().unwrap();
    let mut commit = block_on(fixture.store.apply_workspace_directory_mutation(
        fixture.primary.as_os_str().as_bytes(),
        &Mutation::Add(saved(&extra)),
        &[],
    ))
    .unwrap();
    // The store's own fault-injection tests establish post-rename ambiguity.
    // Here a real committed candidate exercises this owner's observed-state
    // classifier with that typed durability input, without faking disk reads.
    commit.durability = crate::NativeWorkspaceCommitDurability::Ambiguous;
    let intended = operation::reconcile(
        &service,
        NativeWorkspaceAction::Add(extra.clone()),
        &previous,
        &[],
        &commit,
        &[],
        None,
    );
    assert_eq!(
        intended.reconciliation,
        NativeWorkspaceReconciliation::AmbiguousIntended
    );
    assert_eq!(intended.saved_changed, Some(true));
    assert!(intended.snapshot.route(&extra.join("file")).is_ok());
    let service = fixture.service(vec![], false);
    block_on(fixture.store.apply_workspace_directory_mutation(
        fixture.primary.as_os_str().as_bytes(),
        &Mutation::Clear,
        &[],
    ))
    .unwrap();
    let before = operation::reconcile(
        &service,
        NativeWorkspaceAction::Add(extra.clone()),
        &previous,
        &[],
        &commit,
        &[],
        None,
    );
    assert_eq!(
        before.reconciliation,
        NativeWorkspaceReconciliation::AmbiguousBefore
    );
    assert_eq!(before.saved_changed, Some(false));
    fixture.save(&other);
    let unconfirmed = operation::reconcile(
        &service,
        NativeWorkspaceAction::Add(extra.clone()),
        &previous,
        &[],
        &commit,
        &[],
        None,
    );
    assert_eq!(
        unconfirmed.reconciliation,
        NativeWorkspaceReconciliation::Indeterminate
    );
    assert_eq!(unconfirmed.saved_changed, None);
    assert!(service.authority.snapshot().unwrap().entries().is_empty());
    std::fs::write(fixture.base.join("config/config.json"), b"broken").unwrap();
    let failed = operation::reconcile(
        &service,
        NativeWorkspaceAction::Add(extra),
        &previous,
        &[],
        &commit,
        &[],
        None,
    );
    assert!(matches!(
        failed.reconciliation,
        NativeWorkspaceReconciliation::ReloadFailed(_)
    ));
    assert_eq!(failed.saved_changed, None);
}

#[test]
fn stale_full_runtime_add_uses_latest_capacity_but_never_publishes_invalid_requested_root() {
    let fixture = Fixture::new();
    let roots: Vec<_> = (0..16)
        .map(|index| fixture.directory(&format!("root-{index}")))
        .collect();
    let service = fixture.service(
        roots.iter().map(|path| spec(path, true, false)).collect(),
        false,
    );
    let extra = fixture.directory("extra");
    let added = apply(&service, NativeWorkspaceAction::Add(extra.clone()));
    assert_eq!(added.snapshot.entries().len(), 1);
    assert_eq!(fixture.saved(), vec![saved(&extra)]);
    let unsafe_root = fixture.directory("state/inside");
    assert!(matches!(
        block_on(service.execute(NativeWorkspaceAction::Add(unsafe_root))),
        Err(NativeWorkspaceServiceError::Authority(_))
    ));
    assert_eq!(fixture.saved(), vec![saved(&extra)]);
}

#[test]
fn full_saved_and_launch_union_rejects_before_publication() {
    let fixture = Fixture::new();
    let roots: Vec<_> = (0..16)
        .map(|index| fixture.directory(&format!("root-{index}")))
        .collect();
    let service = fixture.service(
        roots.iter().map(|path| spec(path, false, true)).collect(),
        true,
    );
    let extra = fixture.directory("extra");
    assert!(matches!(
        block_on(service.execute(NativeWorkspaceAction::Add(extra))),
        Err(NativeWorkspaceServiceError::Config(_))
    ));
    assert!(!fixture.base.join("config").exists());
    assert_eq!(service.authority.snapshot().unwrap().entries().len(), 16);
}

#[test]
fn non_unicode_directory_roundtrips_losslessly() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let extra = fixture
        .base
        .join(std::ffi::OsString::from_vec(vec![b'x', 0xff]));
    #[cfg(target_os = "linux")]
    let service = {
        std::fs::create_dir(&extra).unwrap();
        let service = fixture.service(vec![], false);
        let added = apply(&service, NativeWorkspaceAction::Add(extra.clone()));
        assert_eq!(added.snapshot.entries()[0].source().identity(), extra);
        service
    };
    #[cfg(target_os = "macos")]
    let service = {
        // APFS refuses creation of invalid UTF-8 names. The persisted Unix byte
        // contract still retains such unavailable entries and permits removal.
        fixture.save(&extra);
        fixture.service(vec![spec(&extra, true, false)], false)
    };
    assert_eq!(fixture.saved(), vec![saved(&extra)]);
    assert!(
        apply(&service, NativeWorkspaceAction::Remove(extra))
            .snapshot
            .entries()
            .is_empty()
    );
}

#[test]
fn concurrent_manager_generation_is_not_overwritten_after_confirmed_save() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let service = fixture.service(vec![], false);
    let authority = service.authority.clone();
    let hook: Hook = Arc::new(move |stage| {
        if stage == Stage::BeforeInstall {
            authority
                .install(authority.refresh_blocking().unwrap())
                .unwrap();
        }
    });
    let receipt = block_on(service.execute_inner(
        None,
        NativeWorkspaceAction::Add(extra.clone()),
        Some(hook),
    ))
    .unwrap();
    assert_eq!(receipt.saved_changed, Some(true));
    assert!(matches!(
        receipt.reconciliation,
        NativeWorkspaceReconciliation::ReloadFailed(NativeWorkspaceServiceError::Authority(
            NativeWorkspaceAuthorityError::StaleGeneration
        ))
    ));
    assert!(service.authority.snapshot().unwrap().entries().is_empty());
    assert_eq!(fixture.saved(), vec![saved(&extra)]);
}

#[test]
fn promoted_provisional_saved_source_remains_duplicate_add_and_removal_addressable() {
    let fixture = Fixture::new();
    let missing = fixture.base.join("missing");
    let target = fixture.directory("target");
    let record = Saved::new(
        missing.as_os_str().as_bytes(),
        missing.as_os_str().as_bytes(),
        false,
    )
    .unwrap();
    block_on(fixture.store.apply_workspace_directory_mutation(
        fixture.primary.as_os_str().as_bytes(),
        &Mutation::Add(record.clone()),
        &[],
    ))
    .unwrap();
    let source = Source::new(missing.clone(), missing.clone(), false).unwrap();
    let service = fixture.service(vec![Spec::new(source, true, false).unwrap()], false);
    std::os::unix::fs::symlink(&target, &missing).unwrap();
    let refreshed = apply(&service, NativeWorkspaceAction::List);
    assert_eq!(refreshed.snapshot.entries()[0].source().identity(), target);
    assert!(refreshed.snapshot.entries()[0].available());
    assert!(refreshed.snapshot.entries()[0].active());
    assert!(
        refreshed.snapshot.entries()[0]
            .source()
            .identity_canonical()
    );
    let merged = Spec::new(refreshed.snapshot.entries()[0].source().clone(), true, true).unwrap();
    service
        .authority
        .install(
            service
                .authority
                .prepare_blocking(vec![merged], false)
                .unwrap(),
        )
        .unwrap();
    let duplicate = apply(&service, NativeWorkspaceAction::Add(target));
    assert_eq!(duplicate.saved_changed, Some(false));
    assert_eq!(duplicate.snapshot.entries().len(), 1);
    assert!(duplicate.snapshot.entries()[0].launch());
    assert_eq!(fixture.saved(), vec![record]);
    let removed = apply(&service, NativeWorkspaceAction::Remove(missing));
    assert_eq!(removed.saved_changed, Some(true));
    assert!(removed.snapshot.entries().is_empty());
    assert!(fixture.saved().is_empty());
}

#[test]
fn shared_merger_counts_unique_roots_after_bounded_launch_alias_deduplication() {
    let fixture = Fixture::new();
    let extra = fixture.directory("extra");
    let source = Source::new(extra.clone(), extra.clone(), true).unwrap();
    let merged =
        merge_workspace_sources_blocking(&[saved(&extra)], &vec![source.clone(); 64]).unwrap();
    assert_eq!(merged.len(), 1);
    assert!(merged[0].saved() && merged[0].launch());
    assert!(matches!(
        merge_workspace_sources_blocking(&[], &vec![source; 65]),
        Err(NativeWorkspaceServiceError::Authority(
            NativeWorkspaceAuthorityError::TooManyDirectories
        ))
    ));
}

fn provisional_launch_fixture(
    fixture: &Fixture,
    other_count: usize,
) -> (Arc<NativeWorkspaceService>, PathBuf, PathBuf, Saved) {
    let target = fixture.directory("alias-target");
    let source = fixture.base.join("saved-alias");
    std::os::unix::fs::symlink(&target, &source).unwrap();
    let record = Saved::new(
        source.as_os_str().as_bytes(),
        source.as_os_str().as_bytes(),
        false,
    )
    .unwrap();
    block_on(fixture.store.apply_workspace_directory_mutation(
        fixture.primary.as_os_str().as_bytes(),
        &Mutation::Add(record.clone()),
        &[],
    ))
    .unwrap();
    for index in 0..other_count {
        fixture.save(&fixture.directory(&format!("other-{index}")));
    }
    let launch = Source::new(target.clone(), target.clone(), true).unwrap();
    let specs = merge_workspace_sources_blocking(&fixture.saved(), &[launch]).unwrap();
    (fixture.service(specs, false), source, target, record)
}

#[test]
fn canonical_alias_of_provisional_saved_root_does_not_consume_seventeenth_slot() {
    let fixture = Fixture::new();
    let (service, _, target, record) = provisional_launch_fixture(&fixture, 14);
    let extra = fixture.directory("last");
    let receipt = apply(&service, NativeWorkspaceAction::Add(extra));
    assert_eq!(receipt.saved_changed, Some(true));
    assert_eq!(
        receipt.reconciliation,
        NativeWorkspaceReconciliation::Confirmed
    );
    assert_eq!(receipt.snapshot.entries().len(), 16);
    let alias = receipt
        .snapshot
        .entries()
        .iter()
        .find(|entry| entry.source().identity() == target)
        .unwrap();
    assert!(alias.saved() && alias.launch());
    assert_eq!(alias.spec().saved_record(), Some(&record));
    assert_eq!(fixture.saved()[0], record);
}

#[test]
fn changed_record_before_admission_cannot_borrow_same_spelling_scope_provenance() {
    let fixture = Fixture::new();
    let (service, source, _, record) = provisional_launch_fixture(&fixture, 14);
    block_on(fixture.store.apply_workspace_directory_mutation(
        fixture.primary.as_os_str().as_bytes(),
        &Mutation::Remove(record.identity_bytes().to_vec()),
        &[],
    ))
    .unwrap();
    let changed = Saved::new(
        source.as_os_str().as_bytes(),
        fixture
            .base
            .join("different-provisional")
            .as_os_str()
            .as_bytes(),
        false,
    )
    .unwrap();
    block_on(fixture.store.apply_workspace_directory_mutation(
        fixture.primary.as_os_str().as_bytes(),
        &Mutation::Add(changed.clone()),
        &[],
    ))
    .unwrap();
    let before = std::fs::read(fixture.base.join("config/config.json")).unwrap();
    let extra = fixture.directory("last");
    assert!(matches!(
        block_on(service.execute(NativeWorkspaceAction::Add(extra))),
        Err(NativeWorkspaceServiceError::Config(_))
    ));
    assert_eq!(
        std::fs::read(fixture.base.join("config/config.json")).unwrap(),
        before
    );
    assert!(fixture.saved().contains(&changed));
    assert_eq!(service.authority.snapshot().unwrap().entries().len(), 15);
}

#[test]
fn unchanged_provisional_source_uses_accepted_identity_after_source_retarget() {
    let fixture = Fixture::new();
    let (service, source, target, record) = provisional_launch_fixture(&fixture, 0);
    let replacement = fixture.directory("replacement");
    let replacement_for_hook = replacement.clone();
    let hook: Hook = Arc::new(move |stage| {
        if stage == Stage::BeforeCommit {
            std::fs::remove_file(&source).unwrap();
            std::os::unix::fs::symlink(&replacement_for_hook, &source).unwrap();
        }
    });
    let receipt = block_on(service.execute_inner(
        None,
        NativeWorkspaceAction::Add(fixture.directory("other")),
        Some(hook),
    ))
    .unwrap();
    assert_eq!(
        receipt.reconciliation,
        NativeWorkspaceReconciliation::Confirmed
    );
    assert_eq!(receipt.snapshot.entries()[0].source().identity(), target);
    assert!(receipt.snapshot.route(&replacement.join("file")).is_err());
    assert_eq!(fixture.saved()[0], record);
    let startup = merge_workspace_sources_blocking(&fixture.saved(), &[]).unwrap();
    assert_eq!(
        startup[0].source().identity(),
        replacement,
        "a new startup has no old authority observation"
    );
}
