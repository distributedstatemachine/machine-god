#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{Engine, SessionId, SessionIncarnationId, SessionRecord, SessionStore};
use machine_god_native::{
    FILE_SESSION_SCHEMA_VERSION, FileSessionStore, MAX_FILE_SESSION_BYTES,
    MAX_LIST_SESSION_DIRECTORY_ENTRIES, MAX_LIST_SESSION_TOTAL_RECORD_BYTES, MAX_LIST_SESSIONS,
    NativeSessionLifecycle, NativeSessionLifecycleError, NativeSessionLifecycleErrorKind,
    SessionIncarnationSource, SessionIncarnationSourceError,
};
use machine_god_testkit::{ScriptedModelProvider, ScriptedPermissionHandler};
use serde_json::json;

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-session-listing-{label}-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create a temporary directory: {error}"),
            }
        }
        panic!("failed to allocate a unique temporary directory");
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.path);
        if std::thread::panicking() {
            return;
        }
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove a temporary directory: {error}"),
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(Pin::as_mut(&mut future), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("native session listing unexpectedly remained pending"),
    }
}

#[derive(Clone)]
struct CountingIncarnationSource {
    calls: Arc<AtomicUsize>,
    value: SessionIncarnationId,
}

impl fmt::Debug for CountingIncarnationSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CountingIncarnationSource")
            .field("calls", &self.calls.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl SessionIncarnationSource for CountingIncarnationSource {
    fn next_incarnation_id(&self) -> Result<SessionIncarnationId, SessionIncarnationSourceError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.value.clone())
    }
}

struct TestLifecycle {
    lifecycle: NativeSessionLifecycle,
    engine: Engine,
    provider: ScriptedModelProvider,
    source_calls: Arc<AtomicUsize>,
}

fn test_lifecycle(store: &Arc<FileSessionStore>) -> TestLifecycle {
    let provider = ScriptedModelProvider::new("session-listing-test", []);
    let engine_store: Arc<dyn SessionStore> = store.clone();
    let engine = Engine::builder()
        .provider(provider.clone())
        .shared_session_store(engine_store)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let source_calls = Arc::new(AtomicUsize::new(0));
    let source: Arc<dyn SessionIncarnationSource> = Arc::new(CountingIncarnationSource {
        calls: Arc::clone(&source_calls),
        value: incarnation("listing-next-life"),
    });
    let lifecycle =
        NativeSessionLifecycle::shared_incarnation_source(engine.clone(), store.clone(), source)
            .unwrap();
    TestLifecycle {
        lifecycle,
        engine,
        provider,
        source_calls,
    }
}

fn id(value: &str) -> SessionId {
    SessionId::new(value).expect("test session ID is valid")
}

fn incarnation(value: &str) -> SessionIncarnationId {
    SessionIncarnationId::new(value).expect("test incarnation ID is valid")
}

fn record(value: &str) -> SessionRecord {
    SessionRecord::empty(id(value), incarnation("listing-original-life"))
}

fn save(store: &FileSessionStore, value: &str) {
    ready(store.save(record(value), None)).expect("test record save succeeds");
}

fn save_record(store: &FileSessionStore, record: SessionRecord) {
    ready(store.save(record, None)).expect("test record save succeeds");
}

fn directory_entries(root: &Path) -> Vec<OsString> {
    let mut entries = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn entry_with_suffix(root: &Path, suffix: &str) -> PathBuf {
    let matches = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(suffix))
        })
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one {suffix} entry");
    matches.into_iter().next().unwrap()
}

fn entries_with_suffix(root: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut matches = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(suffix))
        })
        .collect::<Vec<_>>();
    matches.sort();
    matches
}

fn sibling_with_suffix(path: &Path, old_suffix: &str, new_suffix: &str) -> PathBuf {
    let name = path.file_name().unwrap().to_str().unwrap();
    path.with_file_name(format!(
        "{}{}",
        name.strip_suffix(old_suffix).unwrap(),
        new_suffix
    ))
}

fn persisted_bytes(record: &SessionRecord) -> Vec<u8> {
    format!(
        "{{\"schema_version\":{FILE_SESSION_SCHEMA_VERSION},\"record\":{}}}",
        serde_json::to_string(record).unwrap()
    )
    .into_bytes()
}

fn list_error(lifecycle: &NativeSessionLifecycle) -> NativeSessionLifecycleError {
    ready(lifecycle.list_sessions()).expect_err("listing unexpectedly succeeded")
}

fn assert_corrupt(error: NativeSessionLifecycleError) {
    assert_eq!(error.kind(), NativeSessionLifecycleErrorKind::Corrupt);
    assert_eq!(error.kind().as_str(), "corrupt");
    assert_eq!(error.to_string(), "native session record is corrupt");
}

fn listed_strings(lifecycle: &NativeSessionLifecycle) -> (Vec<String>, bool) {
    let listed = ready(lifecycle.list_sessions()).unwrap();
    let truncated = listed.truncated();
    let values = listed
        .into_session_ids()
        .into_iter()
        .map(|session_id| session_id.as_str().to_owned())
        .collect();
    (values, truncated)
}

#[test]
fn empty_and_unpolled_listing_are_inert() {
    let empty = TemporaryDirectory::new("empty");
    let empty_store = Arc::new(FileSessionStore::open(empty.path()).unwrap());
    let empty_test = test_lifecycle(&empty_store);
    let listed = ready(empty_test.lifecycle.list_sessions()).unwrap();
    assert!(listed.session_ids().is_empty());
    assert!(!listed.truncated());
    assert!(directory_entries(empty.path()).is_empty());

    let seeded = TemporaryDirectory::new("unpolled");
    let seeded_store = Arc::new(FileSessionStore::open(seeded.path()).unwrap());
    save(&seeded_store, "unpolled-listing");
    let lock_path = entry_with_suffix(seeded.path(), ".lock");
    fs::remove_file(&lock_path).unwrap();
    let seeded_test = test_lifecycle(&seeded_store);
    let future = seeded_test.lifecycle.list_sessions();
    drop(future);
    assert!(!lock_path.exists(), "an unpolled listing recreated a lock");
    assert_eq!(seeded_test.source_calls.load(Ordering::Relaxed), 0);
    assert!(seeded_test.provider.requests().is_empty());
}

#[test]
fn listing_is_sorted_unique_and_reset_does_not_duplicate_an_id() {
    let temporary = TemporaryDirectory::new("sorted-reset");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    for value in ["zulu-list", "alpha-list", "middle-list"] {
        save(&store, value);
    }
    let test = test_lifecycle(&store);

    let reset = ready(test.lifecycle.reset(id("middle-list"))).unwrap();
    assert_eq!(
        reset.record().incarnation_id,
        incarnation("listing-next-life")
    );
    drop(reset);
    let listed = ready(test.lifecycle.list_sessions()).unwrap();
    let expected = vec![id("alpha-list"), id("middle-list"), id("zulu-list")];
    assert_eq!(listed.session_ids(), expected.as_slice());
    assert!(!listed.truncated());
    assert_eq!(listed.clone().into_session_ids(), expected);
    assert_eq!(test.source_calls.load(Ordering::Relaxed), 1);
    assert!(test.provider.requests().is_empty());
}

#[test]
fn listing_does_not_consult_source_provider_or_live_engine_registry() {
    let temporary = TemporaryDirectory::new("inert-collaborators");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    save(&store, "durable-only");
    let test = test_lifecycle(&store);
    let live = test
        .engine
        .create_session(id("engine-only"), incarnation("engine-only-life"))
        .unwrap();

    let (values, truncated) = listed_strings(&test.lifecycle);
    assert_eq!(values, ["durable-only"]);
    assert!(!truncated);
    assert_eq!(live.record().id, id("engine-only"));
    assert_eq!(test.source_calls.load(Ordering::Relaxed), 0);
    assert!(test.provider.requests().is_empty());
}

#[test]
fn result_limit_is_exact_at_one_hundred_and_truncates_at_one_hundred_one() {
    for (count, expected_truncated) in [(MAX_LIST_SESSIONS, false), (MAX_LIST_SESSIONS + 1, true)] {
        let temporary = TemporaryDirectory::new(&format!("result-cap-{count}"));
        let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
        for index in 0..count {
            save(&store, &format!("bounded-session-{index:03}"));
        }
        let test = test_lifecycle(&store);
        let (values, truncated) = listed_strings(&test.lifecycle);
        assert_eq!(values.len(), count.min(MAX_LIST_SESSIONS));
        assert_eq!(truncated, expected_truncated);
        assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(values.iter().collect::<BTreeSet<_>>().len(), values.len());
    }
}

#[test]
fn directory_scan_limit_is_exact_and_counts_ignored_artifacts() {
    for (count, expected_truncated) in [
        (MAX_LIST_SESSION_DIRECTORY_ENTRIES, false),
        (MAX_LIST_SESSION_DIRECTORY_ENTRIES + 1, true),
    ] {
        let temporary = TemporaryDirectory::new(&format!("directory-cap-{count}"));
        let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
        let test = test_lifecycle(&store);
        for index in 0..count {
            fs::write(
                temporary
                    .path()
                    .join(format!("ignored-artifact-{index:04}")),
                b"ignored",
            )
            .unwrap();
        }

        let listed = ready(test.lifecycle.list_sessions()).unwrap();
        assert!(listed.session_ids().is_empty());
        assert_eq!(listed.truncated(), expected_truncated);
        assert_eq!(directory_entries(temporary.path()).len(), count);
    }
}

#[test]
fn unrelated_non_utf8_lock_and_temp_artifacts_are_ignored_and_preserved() {
    let temporary = TemporaryDirectory::new("ignored-artifacts");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    save(&store, "visible-session");
    let unrelated = temporary.path().join("PRIVATE_UNRELATED_ARTIFACT");
    let fake_lock = temporary
        .path()
        .join(format!("session-{}.lock", "a".repeat(64)));
    let fake_temp = temporary
        .path()
        .join(format!("session-{}.tmp", "b".repeat(64)));
    let non_utf8_name = OsString::from_vec(vec![b'n', b'o', b'n', b'-', 0xff]);
    let non_utf8 = temporary.path().join(non_utf8_name);
    for path in [&unrelated, &fake_lock, &fake_temp] {
        fs::write(path, b"PRIVATE_IGNORED_BYTES").unwrap();
    }
    let non_utf8_created = match fs::write(&non_utf8, b"PRIVATE_IGNORED_BYTES") {
        Ok(()) => true,
        Err(error) => {
            assert!(
                [libc::EILSEQ, libc::EINVAL].contains(
                    &error
                        .raw_os_error()
                        .expect("non-UTF8 name refusal has an OS error")
                ),
                "unexpected non-UTF8 name refusal: {error}"
            );
            false
        }
    };
    let before = directory_entries(temporary.path());
    let test = test_lifecycle(&store);

    let (values, truncated) = listed_strings(&test.lifecycle);
    assert_eq!(values, ["visible-session"]);
    assert!(!truncated);
    assert_eq!(directory_entries(temporary.path()), before);
    for path in [&unrelated, &fake_lock, &fake_temp] {
        assert_eq!(fs::read(path).unwrap(), b"PRIVATE_IGNORED_BYTES");
    }
    if non_utf8_created {
        assert_eq!(fs::read(&non_utf8).unwrap(), b"PRIVATE_IGNORED_BYTES");
    }
}

#[test]
fn aggregate_record_byte_limit_is_exact_and_plus_one_record_truncates() {
    const RECORD_BYTES: usize = 8 * 1_024 * 1_024;
    const EXACT_RECORDS: usize = MAX_LIST_SESSION_TOTAL_RECORD_BYTES / RECORD_BYTES;
    assert_eq!(
        EXACT_RECORDS * RECORD_BYTES,
        MAX_LIST_SESSION_TOTAL_RECORD_BYTES
    );
    const { assert!(RECORD_BYTES <= MAX_FILE_SESSION_BYTES) };

    let temporary = TemporaryDirectory::new("aggregate-bytes");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    for index in 0..EXACT_RECORDS {
        let mut exact = record(&format!("aggregate-exact-{index}"));
        exact.metadata.insert("padding".to_owned(), json!(""));
        let base_size = persisted_bytes(&exact).len();
        assert!(base_size < RECORD_BYTES);
        exact.metadata.insert(
            "padding".to_owned(),
            json!("x".repeat(RECORD_BYTES - base_size)),
        );
        assert_eq!(persisted_bytes(&exact).len(), RECORD_BYTES);
        save_record(&store, exact);
    }
    let data_paths = entries_with_suffix(temporary.path(), ".json");
    assert_eq!(data_paths.len(), EXACT_RECORDS);
    assert!(data_paths.iter().all(|path| {
        usize::try_from(fs::metadata(path).unwrap().len()).unwrap() == RECORD_BYTES
    }));
    let total_bytes = data_paths.iter().fold(0_usize, |total, path| {
        total + usize::try_from(fs::metadata(path).unwrap().len()).unwrap()
    });
    assert_eq!(total_bytes, MAX_LIST_SESSION_TOTAL_RECORD_BYTES);
    let test = test_lifecycle(&store);

    let exact = ready(test.lifecycle.list_sessions()).unwrap();
    assert_eq!(exact.session_ids().len(), EXACT_RECORDS);
    assert!(!exact.truncated());

    save(&store, "aggregate-overflow-record");
    let overflow = ready(test.lifecycle.list_sessions()).unwrap();
    assert_eq!(overflow.session_ids().len(), EXACT_RECORDS);
    assert!(overflow.truncated());
}

#[test]
fn malformed_future_schema_wrong_digest_and_oversize_records_are_corrupt_and_preserved() {
    enum Corruption {
        Bytes(Vec<u8>),
        WrongDigest,
    }

    let future_record = record("corrupt-record");
    let future_schema = format!(
        "{{\"schema_version\":{},\"record\":{}}}",
        FILE_SESSION_SCHEMA_VERSION + 1,
        serde_json::to_string(&future_record).unwrap()
    )
    .into_bytes();
    let cases = [
        (
            "malformed",
            Corruption::Bytes(b"PRIVATE_MALFORMED_RECORD_SECRET".to_vec()),
        ),
        ("future-schema", Corruption::Bytes(future_schema)),
        ("wrong-digest", Corruption::WrongDigest),
        (
            "oversize",
            Corruption::Bytes(vec![b'x'; MAX_FILE_SESSION_BYTES + 1]),
        ),
    ];

    for (label, corruption) in cases {
        let temporary = TemporaryDirectory::new(label);
        let private_root = temporary.path().join("PRIVATE_SESSION_ROOT_SECRET");
        fs::create_dir(&private_root).unwrap();
        let store = Arc::new(FileSessionStore::open(&private_root).unwrap());
        save(&store, "corrupt-record");
        let data_path = entry_with_suffix(&private_root, ".json");
        let bytes = match corruption {
            Corruption::Bytes(bytes) => bytes,
            Corruption::WrongDigest => persisted_bytes(&record("different-record-id")),
        };
        fs::write(&data_path, &bytes).unwrap();
        let test = test_lifecycle(&store);

        let error = list_error(&test.lifecycle);
        assert_corrupt(error);
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("PRIVATE_SESSION_ROOT_SECRET"));
        assert!(!rendered.contains("PRIVATE_MALFORMED_RECORD_SECRET"));
        assert_eq!(fs::read(&data_path).unwrap(), bytes, "case {label}");
    }
}

#[test]
fn canonical_symlink_and_nonregular_data_entries_are_corrupt_and_preserved() {
    for kind in ["symlink", "directory"] {
        let temporary = TemporaryDirectory::new(kind);
        let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
        save(&store, "hostile-data-entry");
        let data_path = entry_with_suffix(temporary.path(), ".json");
        fs::remove_file(&data_path).unwrap();
        let target = temporary.path().join("PRIVATE_SYMLINK_TARGET");
        fs::write(&target, b"PRIVATE_TARGET_BYTES").unwrap();
        match kind {
            "symlink" => symlink(&target, &data_path).unwrap(),
            "directory" => fs::create_dir(&data_path).unwrap(),
            _ => unreachable!(),
        }
        let test = test_lifecycle(&store);

        let error = list_error(&test.lifecycle);
        assert_corrupt(error);
        assert_eq!(fs::read(&target).unwrap(), b"PRIVATE_TARGET_BYTES");
        let metadata = fs::symlink_metadata(&data_path).unwrap();
        if kind == "symlink" {
            assert!(metadata.file_type().is_symlink());
        } else {
            assert!(metadata.file_type().is_dir());
        }
    }
}

#[test]
fn a_hostile_derived_lock_sidecar_is_corrupt_and_preserved() {
    let temporary = TemporaryDirectory::new("hostile-lock");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    save(&store, "hostile-lock-sidecar");
    let data_path = entry_with_suffix(temporary.path(), ".json");
    let lock_path = sibling_with_suffix(&data_path, ".json", ".lock");
    fs::remove_file(&lock_path).unwrap();
    fs::create_dir(&lock_path).unwrap();
    let test = test_lifecycle(&store);

    let error = list_error(&test.lifecycle);
    assert_corrupt(error);
    assert!(
        fs::symlink_metadata(&lock_path)
            .unwrap()
            .file_type()
            .is_dir()
    );
    assert!(data_path.is_file());
}

#[test]
fn a_missing_lock_sidecar_is_created_with_owner_only_permissions() {
    let temporary = TemporaryDirectory::new("missing-lock");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    save(&store, "missing-lock-sidecar");
    let lock_path = entry_with_suffix(temporary.path(), ".lock");
    fs::remove_file(&lock_path).unwrap();
    let test = test_lifecycle(&store);

    let (values, truncated) = listed_strings(&test.lifecycle);
    assert_eq!(values, ["missing-lock-sidecar"]);
    assert!(!truncated);
    let mode = fs::metadata(&lock_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn listing_retains_the_opened_root_across_rename_and_path_replacement() {
    let temporary = TemporaryDirectory::new("retained-root");
    let original = temporary.path().join("sessions");
    let retained = temporary.path().join("retained-sessions");
    fs::create_dir(&original).unwrap();
    let store = Arc::new(FileSessionStore::open(&original).unwrap());
    save(&store, "retained-session");
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("PRIVATE_ATTACKER_MARKER"), b"unchanged").unwrap();
    let test = test_lifecycle(&store);

    let (values, truncated) = listed_strings(&test.lifecycle);
    assert_eq!(values, ["retained-session"]);
    assert!(!truncated);
    assert_eq!(
        directory_entries(&original),
        [OsString::from("PRIVATE_ATTACKER_MARKER")]
    );
    assert!(directory_entries(&retained).len() >= 2);
}

#[test]
fn listing_a_removed_retained_root_is_fixed_unavailable() {
    let temporary = TemporaryDirectory::new("removed-root-parent");
    let root = temporary.path().join("PRIVATE_REMOVED_ROOT_SECRET");
    fs::create_dir(&root).unwrap();
    let store = Arc::new(FileSessionStore::open(&root).unwrap());
    let test = test_lifecycle(&store);
    fs::remove_dir(&root).unwrap();

    let error = list_error(&test.lifecycle);
    assert_eq!(error.kind(), NativeSessionLifecycleErrorKind::Unavailable);
    assert_eq!(error.kind().as_str(), "unavailable");
    assert_eq!(
        error.to_string(),
        "native session persistence is unavailable"
    );
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_REMOVED_ROOT_SECRET"));
}
