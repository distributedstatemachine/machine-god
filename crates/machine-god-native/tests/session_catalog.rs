#![cfg(any(target_os = "linux", target_os = "macos"))]

#[path = "session_catalog/paging.rs"]
mod paging;

use futures_executor::block_on;
use machine_god_core::{
    Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionStore,
};
use machine_god_native::{
    FileSessionStore, MAX_FILE_SESSION_BYTES, MAX_LIST_SESSION_DIRECTORY_ENTRIES,
    MAX_LIST_SESSION_TOTAL_RECORD_BYTES, NATIVE_SESSION_METADATA_KEY, NativeEnvironment,
    NativeSessionCatalog, NativeSessionCatalogErrorKind, NativeSessionCatalogQuery,
    NativeSessionMetadata, NativeSessionOrigin, NativeSessionSelectionIncomplete,
    inspect_native_session_catalog_entry, list_native_session_catalog, list_native_sessions,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

struct Fixture {
    base: PathBuf,
    root: PathBuf,
    store: Arc<FileSessionStore>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "machine-god-catalog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let root = base.join("machine-god");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let store = Arc::new(FileSessionStore::open(&root).unwrap());
        Self { base, root, store }
    }
    fn environment(&self) -> NativeEnvironment {
        NativeEnvironment::new(None, Some(self.base.as_os_str().to_owned()), None)
    }
    fn catalog(&self) -> NativeSessionCatalog {
        NativeSessionCatalog::new(self.store.clone())
    }
    fn save(&self, record: SessionRecord) {
        block_on(self.store.save(record, None)).unwrap();
    }
    fn list(
        &self,
        query: NativeSessionCatalogQuery,
    ) -> machine_god_native::NativeSessionCatalogPage {
        block_on(self.catalog().list(query)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}
fn id(value: &str) -> SessionId {
    SessionId::new(value).unwrap()
}
fn record(value: &str, time: Option<i64>, workspace: &str) -> SessionRecord {
    let mut record = SessionRecord::empty(
        id(value),
        SessionIncarnationId::new(format!("life-{value}")).unwrap(),
    );
    if let Some(time) = time {
        record.metadata.insert(
            NATIVE_SESSION_METADATA_KEY.into(),
            NativeSessionMetadata::new(Path::new(workspace), time, NativeSessionOrigin::Cli)
                .unwrap()
                .to_value(),
        );
    }
    record
}
fn data_name(value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"machine-god:file-session:v1:");
    digest.update(value.as_bytes());
    format!("session-{:x}.json", digest.finalize())
}

#[test]
fn ranks_the_full_scan_before_display_limit_instead_of_decorating_first_hundred_ids() {
    let fixture = Fixture::new();
    let mut ids: Vec<_> = (0..120).map(|n| format!("session-{n:03}")).collect();
    ids.sort_by_key(|value| data_name(value));
    for (index, value) in ids.iter().enumerate() {
        fixture.save(record(
            value,
            Some(i64::try_from(index).unwrap()),
            "/workspace",
        ));
    }
    let legacy = block_on(list_native_sessions(fixture.environment())).unwrap();
    assert_eq!(legacy.session_ids().len(), 100);
    assert!(legacy.truncated());
    assert!(!legacy.session_ids().contains(&id(ids.last().unwrap())));
    let page = fixture.list(NativeSessionCatalogQuery::new(3).unwrap());
    assert_eq!(page.entries().len(), 3);
    assert!(page.scan_complete());
    assert!(page.results_truncated());
    assert_eq!(page.matched_count(), 120);
    assert_eq!(page.scanned_records(), 120);
    assert_eq!(
        page.latest().unwrap().unwrap().id(),
        &id(ids.last().unwrap())
    );
}

#[test]
fn unknown_fields_stay_visible_but_never_falsely_match_workspace_or_activity() {
    let fixture = Fixture::new();
    fixture.save(record("known", Some(50), "/workspace"));
    let mut old = record("old", None, "/ignored");
    old.messages
        .push(Message::text(Role::User, "Not an authoritative title"));
    fixture.save(old);
    let page = fixture.list(NativeSessionCatalogQuery::default());
    assert_eq!(page.entries().len(), 2);
    assert_eq!(page.unknown_activity_count(), 1);
    assert_eq!(
        page.latest(),
        Err(NativeSessionSelectionIncomplete::UnknownActivity)
    );
    let old = page
        .entries()
        .iter()
        .find(|entry| entry.id() == &id("old"))
        .unwrap();
    assert_eq!(old.native_metadata(), &NativeSessionMetadata::default());
    assert_eq!(old.preview(), Some("Not an authoritative title"));
    for query in [
        NativeSessionCatalogQuery::default()
            .with_workspace(Path::new("/workspace"))
            .unwrap(),
        NativeSessionCatalogQuery::default().with_updated_since(0),
    ] {
        let page = fixture.list(query);
        assert_eq!(page.entries().len(), 1);
        assert_eq!(page.latest().unwrap().unwrap().id(), &id("known"));
    }
    let page = fixture.list(
        NativeSessionCatalogQuery::default()
            .with_search("unknown workspace")
            .unwrap(),
    );
    assert!(page.entries().is_empty());
}

#[test]
fn ties_use_descending_ids_and_search_matches_only_bounded_explicit_fields() {
    let fixture = Fixture::new();
    for value in ["alpha", "zeta"] {
        let mut current = record(value, Some(7), "/Projects/Widget");
        let mut metadata = NativeSessionMetadata::from_metadata(&current.metadata).unwrap();
        metadata.rename("Fix Renderer", 7).unwrap();
        current
            .metadata
            .insert(NATIVE_SESSION_METADATA_KEY.into(), metadata.to_value());
        current.messages.push(Message::text(Role::User, "/status"));
        current
            .messages
            .push(Message::text(Role::Assistant, "assistant is not preview"));
        current.messages.push(Message::text(
            Role::User,
            "  Inspect footer\n\n second line \nthird omitted\n",
        ));
        fixture.save(current);
    }
    let page = fixture.list(NativeSessionCatalogQuery::default());
    assert_eq!(page.entries()[0].id(), &id("zeta"));
    assert_eq!(
        page.entries()[0].preview(),
        Some("Inspect footer\nsecond line")
    );
    assert!(page.entries()[0].preview_truncated());
    for search in ["RENDER", " widget ", "FOOTER"] {
        assert_eq!(
            fixture
                .list(
                    NativeSessionCatalogQuery::default()
                        .with_search(search)
                        .unwrap()
                )
                .matched_count(),
            2
        );
    }
    for search in ["third omitted", "assistant is not preview", "zeta"] {
        assert!(
            fixture
                .list(
                    NativeSessionCatalogQuery::default()
                        .with_search(search)
                        .unwrap()
                )
                .entries()
                .is_empty()
        );
    }
}

#[test]
fn corrupt_metadata_reached_beyond_display_limit_or_filter_fails_whole_scan() {
    let fixture = Fixture::new();
    let mut ids = ["first", "second"];
    ids.sort_by_key(|value| data_name(value));
    fixture.save(record(ids[0], Some(999), "/workspace"));
    let mut invalid = record(ids[1], Some(1), "/other");
    invalid.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.into(),
        json!({"schema_version":99}),
    );
    fixture.save(invalid);
    let query = NativeSessionCatalogQuery::new(1)
        .unwrap()
        .with_workspace(Path::new("/workspace"))
        .unwrap();
    let error = block_on(fixture.catalog().list(query)).unwrap_err();
    assert_eq!(error.kind(), NativeSessionCatalogErrorKind::Corrupt);
    assert_eq!(
        block_on(list_native_sessions(fixture.environment()))
            .unwrap()
            .session_ids()
            .len(),
        2
    );
    assert_eq!(
        block_on(fixture.catalog().exact(id(ids[1])))
            .unwrap_err()
            .kind(),
        NativeSessionCatalogErrorKind::Corrupt
    );
}

#[test]
fn directory_incompleteness_never_claims_latest_and_exact_lookup_still_works() {
    let fixture = Fixture::new();
    fixture.save(record("exact", Some(999), "/workspace"));
    for index in 0..=MAX_LIST_SESSION_DIRECTORY_ENTRIES {
        fs::write(fixture.root.join(format!("ignored-{index}")), b"").unwrap();
    }
    let page = fixture.list(NativeSessionCatalogQuery::new(1).unwrap());
    assert!(!page.scan_complete());
    assert_eq!(
        page.latest(),
        Err(NativeSessionSelectionIncomplete::ScanIncomplete)
    );
    let exact = block_on(fixture.catalog().exact(id("exact")))
        .unwrap()
        .unwrap();
    assert_eq!(exact.id(), &id("exact"));
    assert_eq!(exact.native_metadata().updated_at_ms(), Some(999));
}

#[test]
fn aggregate_byte_incompleteness_preserves_the_existing_store_bound() {
    let fixture = Fixture::new();
    let body = "x".repeat(8 * 1024 * 1024);
    for index in 0..9 {
        let mut current = record(&format!("large-{index}"), Some(index), "/workspace");
        current
            .messages
            .push(Message::text(Role::Assistant, body.clone()));
        fixture.save(current);
    }
    let page = fixture.list(NativeSessionCatalogQuery::new(1).unwrap());
    assert!(!page.scan_complete());
    assert!(page.scanned_record_bytes() <= MAX_LIST_SESSION_TOTAL_RECORD_BYTES);
    assert_eq!(
        page.latest(),
        Err(NativeSessionSelectionIncomplete::ScanIncomplete)
    );
    assert!(page.entries()[0].preview().is_none());
}

#[test]
fn exact_record_snapshot_and_preview_are_bounded_without_modifying_source() {
    let fixture = Fixture::new();
    let mut current = record("unicode", Some(5), "/workspace");
    current
        .messages
        .push(Message::text(Role::User, "界".repeat(1000)));
    fixture.save(current);
    let path = fixture.root.join(data_name("unicode"));
    let before = fs::read(&path).unwrap();
    let entry = block_on(inspect_native_session_catalog_entry(
        fixture.environment(),
        id("unicode"),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(entry.message_count(), 1);
    assert_eq!(entry.preview().unwrap().len(), 240);
    assert!(entry.preview_truncated());
    assert!(entry.native_metadata().title().is_none());
    assert_eq!(before, fs::read(path).unwrap());
    assert!(!format!("{entry:?}").contains('界'));
}

#[test]
fn retained_root_rename_does_not_redirect_and_unlinked_empty_root_is_unavailable() {
    let fixture = Fixture::new();
    fixture.save(record("retained", Some(1), "/workspace"));
    let catalog = fixture.catalog();
    let future = catalog.list(NativeSessionCatalogQuery::default());
    fs::rename(&fixture.root, fixture.base.join("moved")).unwrap();
    fs::create_dir(&fixture.root).unwrap();
    let page = block_on(future).unwrap();
    assert_eq!(page.entries()[0].id(), &id("retained"));
    let empty = Fixture::new();
    let future = empty.catalog().list(NativeSessionCatalogQuery::default());
    fs::remove_dir(&empty.root).unwrap();
    assert_eq!(
        block_on(future).unwrap_err().kind(),
        NativeSessionCatalogErrorKind::Unavailable
    );
}

#[test]
fn native_facades_are_inert_and_never_create_missing_hierarchy_or_exact_record() {
    let fixture = Fixture::new();
    let missing = fixture.base.join("missing");
    let environment = || NativeEnvironment::new(None, Some(missing.as_os_str().to_owned()), None);
    let future = list_native_session_catalog(environment(), NativeSessionCatalogQuery::default());
    assert!(!missing.exists());
    assert!(block_on(future).unwrap().entries().is_empty());
    assert!(
        block_on(inspect_native_session_catalog_entry(
            environment(),
            id("absent")
        ))
        .unwrap()
        .is_none()
    );
    assert!(!missing.exists());
    let future = fixture.catalog().exact(id("absent"));
    drop(future);
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
    assert!(
        block_on(fixture.catalog().exact(id("absent")))
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
}

#[test]
fn malformed_candidates_and_hostile_locks_are_fixed_corruption_not_partial_success() {
    for hostile in ["symlink", "directory", "oversized", "lock"] {
        let fixture = Fixture::new();
        let path = fixture.root.join(data_name("bad"));
        match hostile {
            "symlink" => std::os::unix::fs::symlink("unrelated", &path).unwrap(),
            "directory" => fs::create_dir(&path).unwrap(),
            "oversized" => {
                let file = fs::File::create(&path).unwrap();
                file.set_len(u64::try_from(MAX_FILE_SESSION_BYTES + 1).unwrap())
                    .unwrap();
            }
            _ => {
                fixture.save(record("bad", Some(1), "/workspace"));
                let lock = fixture
                    .root
                    .join(data_name("bad").replace(".json", ".lock"));
                fs::remove_file(&lock).unwrap();
                std::os::unix::fs::symlink("unrelated", lock).unwrap();
            }
        }
        let error =
            block_on(fixture.catalog().list(NativeSessionCatalogQuery::default())).unwrap_err();
        assert_eq!(
            error.kind(),
            NativeSessionCatalogErrorKind::Corrupt,
            "{hostile}"
        );
        assert!(!format!("{error:?} {error}").contains("bad"));
    }
}

#[test]
fn query_bounds_and_unknown_time_filter_are_pure_and_explicit() {
    assert!(NativeSessionCatalogQuery::new(0).is_err());
    assert!(NativeSessionCatalogQuery::new(101).is_err());
    for path in ["relative", "/a/../b", "/a//b", "/a/./b"] {
        assert!(
            NativeSessionCatalogQuery::default()
                .with_workspace(Path::new(path))
                .is_err()
        );
    }
    assert!(
        NativeSessionCatalogQuery::default()
            .with_search(&"x".repeat(1025))
            .is_err()
    );
    assert!(
        NativeSessionCatalogQuery::default()
            .with_search("\0")
            .is_err()
    );
    let fixture = Fixture::new();
    assert!(
        fixture
            .list(NativeSessionCatalogQuery::default())
            .latest()
            .unwrap()
            .is_none()
    );
}
