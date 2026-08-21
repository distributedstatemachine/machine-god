#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Child, Command, ExitStatus};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    ContentBlock, Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionRevision,
    SessionStore, SessionStoreError, SessionStoreErrorKind, ToolCall, ToolCallId, ToolName,
    ToolOutput,
};
use machine_god_native::{
    FILE_SESSION_SCHEMA_VERSION, FileSessionStore, FileSessionStoreOpenErrorKind,
    MAX_FILE_SESSION_BYTES,
};
use serde_json::{Value, json};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = Path::new("/tmp").join(format!("mgss-{}-{identifier}", std::process::id()));
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
        Poll::Pending => panic!("file session-store operation unexpectedly remained pending"),
    }
}

fn store(root: &Path) -> FileSessionStore {
    FileSessionStore::open(root).expect("temporary session root is valid")
}

fn id(value: &str) -> SessionId {
    SessionId::new(value).expect("test session ID is valid")
}

fn record(name: &str) -> SessionRecord {
    let id = id(name);
    SessionRecord::empty(
        id,
        SessionIncarnationId::new("test-incarnation").expect("test incarnation ID is valid"),
    )
}

fn save_new(store: &FileSessionStore, record: SessionRecord) -> SessionRevision {
    ready(store.save(record, None)).expect("initial save succeeds")
}

fn load(store: &FileSessionStore, id: SessionId) -> Option<SessionRecord> {
    ready(store.load(id)).expect("load succeeds")
}

fn directory_entries(root: &Path) -> Vec<String> {
    let mut entries = fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .file_name()
                .into_string()
                .expect("session filenames are UTF-8")
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn entry_with_suffix(root: &Path, suffix: &str) -> PathBuf {
    let matches = directory_entries(root)
        .into_iter()
        .filter(|name| name.ends_with(suffix))
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one {suffix} entry");
    root.join(&matches[0])
}

fn sibling_with_suffix(path: &Path, old_suffix: &str, new_suffix: &str) -> PathBuf {
    let name = path.file_name().unwrap().to_str().unwrap();
    let stem = name.strip_suffix(old_suffix).unwrap();
    path.with_file_name(format!("{stem}{new_suffix}"))
}

fn persisted_bytes(record: &SessionRecord) -> Vec<u8> {
    format!(
        "{{\"schema_version\":{FILE_SESSION_SCHEMA_VERSION},\"record\":{}}}",
        serde_json::to_string(record).unwrap()
    )
    .into_bytes()
}

fn assert_store_error(
    error: &SessionStoreError,
    kind: SessionStoreErrorKind,
    code: &str,
    retryable: bool,
) {
    assert_eq!(error.kind, kind);
    assert_eq!(error.code, code);
    assert_eq!(error.retryable, retryable);
}

fn create_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("failed to invoke POSIX mkfifo");
    assert!(status.success(), "mkfifo failed with {status}");
}

#[test]
fn open_requires_an_absolute_existing_real_directory_and_redacts_paths() {
    let temporary = TemporaryDirectory::new();
    let secret = temporary.path().join("private-session-root");
    let missing = temporary.path().join("private-missing-root");
    let file = temporary.path().join("private-root-file");
    let link = temporary.path().join("private-root-link");
    let fifo = temporary.path().join("private-root-fifo");
    let socket = temporary.path().join("private-root-socket");
    fs::create_dir(&secret).unwrap();
    fs::write(&file, b"not a directory").unwrap();
    symlink(&secret, &link).unwrap();
    create_fifo(&fifo);
    UnixListener::bind(&socket).unwrap();

    let relative = FileSessionStore::open(Path::new("relative-session-root")).unwrap_err();
    assert_eq!(relative.kind(), FileSessionStoreOpenErrorKind::InvalidRoot);
    let missing_error = FileSessionStore::open(&missing).unwrap_err();
    assert_eq!(
        missing_error.kind(),
        FileSessionStoreOpenErrorKind::Unavailable
    );
    for path in [&file, &link, &fifo, &socket] {
        let error = FileSessionStore::open(path).unwrap_err();
        assert_eq!(error.kind(), FileSessionStoreOpenErrorKind::InvalidFileType);
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains(path.to_string_lossy().as_ref()));
        assert!(!rendered.contains("private-session-root"));
    }

    let opened = store(&secret);
    let debug = format!("{opened:?}");
    assert!(!debug.contains(secret.to_string_lossy().as_ref()));
    assert!(!debug.contains("private-session-root"));
}

#[test]
fn opened_store_retains_the_original_root_after_path_replacement() {
    let temporary = TemporaryDirectory::new();
    let original = temporary.path().join("root");
    let retained = temporary.path().join("retained");
    fs::create_dir(&original).unwrap();
    let store = store(&original);
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("attacker-marker"), b"unchanged").unwrap();

    let expected = record("retained-root");
    assert_eq!(save_new(&store, expected.clone()), SessionRevision(1));
    let mut persisted = expected;
    persisted.revision = SessionRevision(1);
    assert_eq!(load(&store, id("retained-root")), Some(persisted));
    assert_eq!(directory_entries(&original), ["attacker-marker"]);
    assert!(directory_entries(&retained).iter().any(|name| {
        Path::new(name)
            .extension()
            .is_some_and(|extension| extension == "json")
    }));
}

#[test]
fn missing_and_unpolled_operations_create_no_artifacts() {
    let temporary = TemporaryDirectory::new();
    let store = store(temporary.path());
    assert_eq!(load(&store, id("missing")), None);
    assert!(directory_entries(temporary.path()).is_empty());

    let load_future = store.load(id("never-polled-load"));
    drop(load_future);
    let save_future = store.save(record("never-polled-save"), None);
    drop(save_future);
    assert!(directory_entries(temporary.path()).is_empty());
}

#[test]
fn direct_zero_turn_sequence_is_rejected_before_filesystem_effects() {
    let temporary = TemporaryDirectory::new();
    let store = store(temporary.path());
    let mut invalid = record("zero-turn-sequence");
    invalid.next_turn_sequence = 0;

    let error = ready(store.save(invalid, None)).unwrap_err();
    assert_store_error(
        &error,
        SessionStoreErrorKind::Other,
        "session_serialization_failed",
        false,
    );
    assert!(directory_entries(temporary.path()).is_empty());
}

#[test]
fn newly_created_files_are_exact_0600_under_a_restrictive_umask() {
    const CHILD_ROOT: &str = "MACHINE_GOD_FILE_SESSION_UMASK_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let store = store(&root);
        assert_eq!(
            save_new(&store, record("restrictive-umask")),
            SessionRevision(1)
        );
        let data_path = entry_with_suffix(&root, ".json");
        let lock_path = sibling_with_suffix(&data_path, ".json", ".lock");
        for path in [data_path, lock_path] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        return;
    }

    let temporary = TemporaryDirectory::new();
    let status = Command::new("sh")
        .arg("-c")
        .arg("umask 0777; exec \"$1\" --exact newly_created_files_are_exact_0600_under_a_restrictive_umask")
        .arg("machine-god-session-store-test")
        .arg(std::env::current_exe().unwrap())
        .env(CHILD_ROOT, temporary.path())
        .status()
        .unwrap();
    assert!(status.success(), "restrictive-umask child failed: {status}");
}

#[test]
fn strict_schema_round_trips_all_content_and_persists_assigned_revisions() {
    let temporary = TemporaryDirectory::new();
    let store = store(temporary.path());
    let call_id = ToolCallId::new("call-1").unwrap();
    let mut expected = record("round-trip");
    expected.messages = vec![
        Message::text(Role::System, "system"),
        Message {
            role: Role::User,
            content: vec![
                ContentBlock::Text {
                    text: "hello".to_owned(),
                },
                ContentBlock::Json {
                    value: json!({"z": 2, "a": [true, null]}),
                },
            ],
        },
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolCall {
                call: ToolCall {
                    id: call_id.clone(),
                    name: ToolName::new("lookup").unwrap(),
                    arguments: json!({"key": "value"}),
                },
            }],
        },
        Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id,
                output: ToolOutput::success(json!({"answer": 42})),
            }],
        },
    ];
    expected.metadata.insert("z-key".to_owned(), json!(2));
    expected.metadata.insert("a-key".to_owned(), json!(1));

    assert_eq!(save_new(&store, expected.clone()), SessionRevision(1));
    expected.revision = SessionRevision(1);
    assert_eq!(load(&store, expected.id.clone()), Some(expected.clone()));
    let data_path = entry_with_suffix(temporary.path(), ".json");
    assert_eq!(fs::read(&data_path).unwrap(), persisted_bytes(&expected));
    assert_eq!(
        fs::metadata(&data_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let lock_path = sibling_with_suffix(&data_path, ".json", ".lock");
    assert_eq!(
        fs::metadata(lock_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    expected
        .messages
        .push(Message::text(Role::Assistant, "updated"));
    assert_eq!(
        ready(store.save(expected.clone(), Some(SessionRevision(1)))).unwrap(),
        SessionRevision(2)
    );
    expected.revision = SessionRevision(2);
    assert_eq!(load(&store, expected.id.clone()), Some(expected));
}

#[test]
fn filenames_are_fixed_hashes_and_hostile_valid_ids_cannot_traverse() {
    let cases = [
        (
            ".",
            "9e3c34d75139138b1ebcf49b648f3f60159d453cc6c87ec25a7aceeb0d8e1fd3",
        ),
        (
            "..",
            "29b8a0c5aa8e22b78e428bda0bd0039f12d055cfec100789029fbbaa4b95601c",
        ),
        (
            "colon:name",
            "dfbd256ff1a0eff67b21738117b0e9b22498a5c55253ab6f6d6f5059a6484c53",
        ),
        (
            "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
            "4c4b0ae90a5add695b04790293fe9cf0f36f5353782831f881f49ceb3dc546ba",
        ),
    ];
    for (name, digest) in cases {
        let temporary = TemporaryDirectory::new();
        let root = temporary.path().join("sessions");
        fs::create_dir(&root).unwrap();
        fs::write(temporary.path().join("outside"), b"unchanged").unwrap();
        let store = store(&root);

        assert_eq!(save_new(&store, record(name)), SessionRevision(1));
        assert_eq!(
            directory_entries(&root),
            [
                format!("session-{digest}.json"),
                format!("session-{digest}.lock"),
            ]
        );
        assert_eq!(
            fs::read(temporary.path().join("outside")).unwrap(),
            b"unchanged"
        );
    }
}

#[test]
fn exact_cas_and_incarnation_are_enforced_without_changing_prior_bytes() {
    {
        let absent_root = TemporaryDirectory::new();
        let absent_store = store(absent_root.path());
        let error =
            ready(absent_store.save(record("absent-cas"), Some(SessionRevision(1)))).unwrap_err();
        assert_store_error(
            &error,
            SessionStoreErrorKind::Conflict,
            "revision_conflict",
            true,
        );
        assert_eq!(load(&absent_store, id("absent-cas")), None);
    }

    let temporary = TemporaryDirectory::new();
    let store = store(temporary.path());
    let mut current = record("cas");
    assert_eq!(save_new(&store, current.clone()), SessionRevision(1));
    current.revision = SessionRevision(1);
    let data_path = entry_with_suffix(temporary.path(), ".json");
    let prior = fs::read(&data_path).unwrap();

    for expected in [None, Some(SessionRevision(0)), Some(SessionRevision(2))] {
        let error = ready(store.save(current.clone(), expected)).unwrap_err();
        assert_store_error(
            &error,
            SessionStoreErrorKind::Conflict,
            "revision_conflict",
            true,
        );
        assert_eq!(fs::read(&data_path).unwrap(), prior);
    }
    let mut other_incarnation = current.clone();
    other_incarnation.incarnation_id = SessionIncarnationId::new("other-incarnation").unwrap();
    let error = ready(store.save(other_incarnation, Some(SessionRevision(1)))).unwrap_err();
    assert_store_error(
        &error,
        SessionStoreErrorKind::Conflict,
        "incarnation_conflict",
        false,
    );
    assert_eq!(fs::read(&data_path).unwrap(), prior);
}

#[test]
fn assigned_revision_is_checked_max_of_stored_and_candidate_plus_one() {
    let temporary = TemporaryDirectory::new();
    let store = store(temporary.path());
    let mut candidate = record("revision-base");
    candidate.revision = SessionRevision(7);

    assert_eq!(save_new(&store, candidate.clone()), SessionRevision(8));
    candidate.revision = SessionRevision(20);
    assert_eq!(
        ready(store.save(candidate.clone(), Some(SessionRevision(8)))).unwrap(),
        SessionRevision(21)
    );
    let persisted = load(&store, candidate.id.clone()).unwrap();
    assert_eq!(persisted.revision, SessionRevision(21));

    let data_path = entry_with_suffix(temporary.path(), ".json");
    let prior = fs::read(&data_path).unwrap();
    candidate.revision = SessionRevision(u64::MAX);
    let error = ready(store.save(candidate, Some(SessionRevision(21)))).unwrap_err();
    assert_store_error(
        &error,
        SessionStoreErrorKind::Other,
        "revision_exhausted",
        false,
    );
    assert_eq!(fs::read(data_path).unwrap(), prior);
}

#[test]
fn corrupt_destination_and_revision_overflow_preserve_prior_bytes() {
    let temporary = TemporaryDirectory::new();
    let store = store(temporary.path());
    let mut current = record("preserve-corrupt");
    assert_eq!(save_new(&store, current.clone()), SessionRevision(1));
    current.revision = SessionRevision(1);
    let data_path = entry_with_suffix(temporary.path(), ".json");
    let corrupt = b"{private-corrupt-destination";
    fs::write(&data_path, corrupt).unwrap();

    let error = ready(store.save(current.clone(), Some(SessionRevision(1)))).unwrap_err();
    assert_store_error(
        &error,
        SessionStoreErrorKind::Corrupt,
        "file_session_corrupt",
        false,
    );
    assert_eq!(fs::read(&data_path).unwrap(), corrupt);

    current.revision = SessionRevision(u64::MAX);
    fs::write(&data_path, persisted_bytes(&current)).unwrap();
    let overflow_bytes = fs::read(&data_path).unwrap();
    let error = ready(store.save(current.clone(), Some(current.revision))).unwrap_err();
    assert_store_error(
        &error,
        SessionStoreErrorKind::Other,
        "revision_exhausted",
        false,
    );
    assert_eq!(fs::read(&data_path).unwrap(), overflow_bytes);
}

#[test]
fn malformed_unknown_duplicate_and_invalid_records_fail_closed() {
    let documents = [
        b"not-json".to_vec(),
        b"{\"schema_version\":1,\"record\":\xff}".to_vec(),
        br#"{"schema_version":2,"record":{}}"#.to_vec(),
        br#"{"schema_version":1,"unknown":true,"record":{}}"#.to_vec(),
        br#"{"schema_version":1,"schema_version":1,"record":{}}"#.to_vec(),
        br#"{"schema_version":1,"record":{"id":"disk-corrupt","incarnation_id":"test-incarnation","revision":1,"next_turn_sequence":1,"messages":[],"metadata":{},"unknown":true}}"#.to_vec(),
        br#"{"schema_version":1,"record":{"id":"disk-corrupt","id":"other","incarnation_id":"test-incarnation","revision":1,"next_turn_sequence":1,"messages":[],"metadata":{}}}"#.to_vec(),
        br#"{"schema_version":1,"record":{"id":"other","incarnation_id":"test-incarnation","revision":1,"next_turn_sequence":1,"messages":[],"metadata":{}}}"#.to_vec(),
        br#"{"schema_version":1,"record":{"id":"disk-corrupt","incarnation_id":"test-incarnation","revision":0,"next_turn_sequence":1,"messages":[],"metadata":{}}}"#.to_vec(),
        br#"{"schema_version":1,"record":{"id":"disk-corrupt","incarnation_id":"test-incarnation","revision":1,"next_turn_sequence":0,"messages":[],"metadata":{}}}"#.to_vec(),
        br#"{"schema_version":1,"record":{"id":"disk-corrupt","incarnation_id":"test-incarnation","revision":1,"next_turn_sequence":1,"messages":[{"role":"user","content":[],"extra":true}],"metadata":{}}}"#.to_vec(),
        br#"{"schema_version":1,"record":{"id":"disk-corrupt","incarnation_id":"test-incarnation","revision":1,"next_turn_sequence":1,"messages":[{"role":"user","content":[{"type":"text","text":"x","extra":true}]}],"metadata":{}}}"#.to_vec(),
        br#"{"schema_version":1,"record":{"id":"disk-corrupt","incarnation_id":"test-incarnation","revision":1,"next_turn_sequence":1,"messages":[{"role":"assistant","content":[{"type":"tool_call","call":{"id":"call","name":"tool","arguments":{},"extra":true}}]}],"metadata":{}}}"#.to_vec(),
        br#"{"schema_version":1,"record":{"id":"disk-corrupt","incarnation_id":"test-incarnation","revision":1,"next_turn_sequence":1,"messages":[{"role":"tool","content":[{"type":"tool_result","call_id":"call","output":{"content":{},"is_error":false,"extra":true}}]}],"metadata":{}}}"#.to_vec(),
        br#"{"schema_version":1,"record":{"id":"disk-corrupt","incarnation_id":"test-incarnation","revision":1,"next_turn_sequence":1,"messages":[],"metadata":{}}} trailing"#.to_vec(),
    ];

    for (index, document) in documents.into_iter().enumerate() {
        let temporary = TemporaryDirectory::new();
        let store = store(temporary.path());
        assert_eq!(save_new(&store, record("disk-corrupt")), SessionRevision(1));
        let data_path = entry_with_suffix(temporary.path(), ".json");
        fs::write(&data_path, &document).unwrap();
        let error = ready(store.load(id("disk-corrupt"))).unwrap_err();
        assert_store_error(
            &error,
            SessionStoreErrorKind::Corrupt,
            "file_session_corrupt",
            false,
        );
        assert_eq!(fs::read(&data_path).unwrap(), document, "case {index}");
    }
}

#[test]
fn exact_serialized_limit_succeeds_and_plus_one_is_bounded_and_atomic() {
    let temporary = TemporaryDirectory::new();
    let store = store(temporary.path());
    let mut exact = record("exact-size");
    exact.metadata.insert("padding".to_owned(), json!(""));
    exact.revision = SessionRevision(1);
    let base_size = persisted_bytes(&exact).len();
    assert!(base_size < MAX_FILE_SESSION_BYTES);
    exact.metadata.insert(
        "padding".to_owned(),
        json!("x".repeat(MAX_FILE_SESSION_BYTES - base_size)),
    );
    let mut input = exact.clone();
    input.revision = SessionRevision(0);

    assert_eq!(save_new(&store, input), SessionRevision(1));
    let data_path = entry_with_suffix(temporary.path(), ".json");
    assert_eq!(
        fs::metadata(&data_path).unwrap().len(),
        u64::try_from(MAX_FILE_SESSION_BYTES).unwrap()
    );
    let prior = fs::read(&data_path).unwrap();
    let Value::String(padding) = exact.metadata.get_mut("padding").unwrap() else {
        unreachable!()
    };
    padding.push('x');
    let error = ready(store.save(exact, Some(SessionRevision(1)))).unwrap_err();
    assert_store_error(
        &error,
        SessionStoreErrorKind::Other,
        "session_too_large",
        false,
    );
    assert_eq!(fs::read(&data_path).unwrap(), prior);
    assert!(!directory_entries(temporary.path()).iter().any(|name| {
        Path::new(name)
            .extension()
            .is_some_and(|extension| extension == "tmp")
    }));
}

#[test]
fn oversized_disk_input_is_corrupt_and_left_unchanged() {
    let temporary = TemporaryDirectory::new();
    let store = store(temporary.path());
    assert_eq!(
        save_new(&store, record("oversized-load")),
        SessionRevision(1)
    );
    let data_path = entry_with_suffix(temporary.path(), ".json");
    let oversized = vec![b'x'; MAX_FILE_SESSION_BYTES + 1];
    fs::write(&data_path, &oversized).unwrap();
    let error = ready(store.load(id("oversized-load"))).unwrap_err();
    assert_store_error(
        &error,
        SessionStoreErrorKind::Corrupt,
        "file_session_corrupt",
        false,
    );
    assert_eq!(
        fs::metadata(data_path).unwrap().len(),
        u64::try_from(oversized.len()).unwrap()
    );
}

#[test]
fn stale_regular_temp_is_recovered_but_nonregular_temps_fail_closed() {
    let temporary = TemporaryDirectory::new();
    let store = store(temporary.path());
    assert_eq!(save_new(&store, record("stale-temp")), SessionRevision(1));
    let data_path = entry_with_suffix(temporary.path(), ".json");
    let temp_path = sibling_with_suffix(&data_path, ".json", ".tmp");
    fs::write(&temp_path, b"stale incomplete bytes").unwrap();
    let mut update = load(&store, id("stale-temp")).unwrap();
    update.messages.push(Message::text(Role::User, "update"));
    assert_eq!(
        ready(store.save(update, Some(SessionRevision(1)))).unwrap(),
        SessionRevision(2)
    );
    assert!(!temp_path.exists());

    for kind in ["directory", "symlink", "fifo", "socket"] {
        let temporary = TemporaryDirectory::new();
        let store = FileSessionStore::open(temporary.path()).unwrap();
        assert_eq!(save_new(&store, record("hostile-temp")), SessionRevision(1));
        let data_path = entry_with_suffix(temporary.path(), ".json");
        let temp_path = sibling_with_suffix(&data_path, ".json", ".tmp");
        let target = temporary.path().join("target");
        fs::write(&target, b"unchanged").unwrap();
        match kind {
            "directory" => fs::create_dir(&temp_path).unwrap(),
            "symlink" => symlink(&target, &temp_path).unwrap(),
            "fifo" => create_fifo(&temp_path),
            "socket" => {
                UnixListener::bind(&temp_path).unwrap();
            }
            _ => unreachable!(),
        }
        let mut update = load(&store, id("hostile-temp")).unwrap();
        update
            .messages
            .push(Message::text(Role::User, "must not persist"));
        let error = ready(store.save(update, Some(SessionRevision(1)))).unwrap_err();
        assert_store_error(
            &error,
            SessionStoreErrorKind::Corrupt,
            "file_session_corrupt",
            false,
        );
        assert!(fs::symlink_metadata(&temp_path).is_ok());
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
    }
}

#[test]
fn data_and_lock_entries_must_be_nofollow_regular_files() {
    for entry in ["data", "lock"] {
        for kind in ["directory", "symlink", "fifo", "socket"] {
            let temporary = TemporaryDirectory::new();
            let store = store(temporary.path());
            assert_eq!(
                save_new(&store, record("hostile-entry")),
                SessionRevision(1)
            );
            let data_path = entry_with_suffix(temporary.path(), ".json");
            let lock_path = sibling_with_suffix(&data_path, ".json", ".lock");
            let path = if entry == "data" {
                &data_path
            } else {
                &lock_path
            };
            fs::remove_file(path).unwrap();
            let target = temporary.path().join("private-target");
            fs::write(&target, b"unchanged").unwrap();
            match kind {
                "directory" => fs::create_dir(path).unwrap(),
                "symlink" => symlink(&target, path).unwrap(),
                "fifo" => create_fifo(path),
                "socket" => {
                    UnixListener::bind(path).unwrap();
                }
                _ => unreachable!(),
            }

            let result = if entry == "data" {
                ready(store.load(id("hostile-entry"))).map(|_| ())
            } else {
                let mut update = record("hostile-entry");
                update.revision = SessionRevision(1);
                ready(store.save(update, Some(SessionRevision(1)))).map(|_| ())
            };
            assert_store_error(
                &result.unwrap_err(),
                SessionStoreErrorKind::Corrupt,
                "file_session_corrupt",
                false,
            );
            assert_eq!(fs::read(&target).unwrap(), b"unchanged");
            assert!(fs::symlink_metadata(path).is_ok());
        }
    }
}

fn deeply_nested_array(depth: usize) -> Value {
    (0..depth).fold(Value::Null, |value, _| Value::Array(vec![value]))
}

#[test]
fn deep_direct_save_is_stack_safe_when_unpolled_and_polled() {
    const CHILD_MODE: &str = "MACHINE_GOD_FILE_SESSION_DEEP_MODE";
    if let Ok(mode) = std::env::var(CHILD_MODE) {
        let temporary = TemporaryDirectory::new();
        let store = store(temporary.path());
        let mut deep = record("deep-direct-save");
        deep.metadata
            .insert("deep".to_owned(), deeply_nested_array(20_000));
        let future = store.save(deep, None);
        if mode == "unpolled" {
            drop(future);
        } else {
            let error = ready(future).unwrap_err();
            assert_store_error(
                &error,
                SessionStoreErrorKind::Other,
                "session_serialization_failed",
                false,
            );
        }
        assert!(directory_entries(temporary.path()).is_empty());
        return;
    }

    for mode in ["unpolled", "polled"] {
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("deep_direct_save_is_stack_safe_when_unpolled_and_polled")
            .env(CHILD_MODE, mode)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "deep direct-save child failed in {mode} mode"
        );
    }
}

#[test]
fn concurrent_exact_cas_has_one_winner_for_shared_and_independent_stores() {
    for independent in [false, true] {
        let temporary = TemporaryDirectory::new();
        let first = Arc::new(store(temporary.path()));
        assert_eq!(save_new(&first, record("thread-cas")), SessionRevision(1));
        let second = if independent {
            Arc::new(store(temporary.path()))
        } else {
            Arc::clone(&first)
        };
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut workers =
            Vec::<std::thread::JoinHandle<Result<SessionRevision, SessionStoreError>>>::new();
        for (store, text) in [(Arc::clone(&first), "first"), (second, "second")] {
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                let mut candidate = record("thread-cas");
                candidate.revision = SessionRevision(1);
                candidate.messages.push(Message::text(Role::User, text));
                barrier.wait();
                ready(store.save(candidate, Some(SessionRevision(1))))
            }));
        }
        barrier.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let loser = results.into_iter().find_map(Result::err).unwrap();
        assert_store_error(
            &loser,
            SessionStoreErrorKind::Conflict,
            "revision_conflict",
            true,
        );
        let persisted = load(&first, id("thread-cas")).unwrap();
        assert_eq!(persisted.revision, SessionRevision(2));
        assert_eq!(persisted.messages.len(), 1);
    }
}

fn wait_for_files(paths: &[&Path]) {
    for _ in 0..1_000 {
        if paths.iter().all(|path| path.is_file()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for subprocess barrier files");
}

fn wait_for_child(mut child: Child) -> ExitStatus {
    for _ in 0..1_000 {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child.kill().unwrap();
    let _ = child.wait();
    panic!("session-store contention subprocess did not exit");
}

#[test]
fn independent_processes_serialize_exact_cas_without_lost_updates() {
    const ROOT: &str = "MACHINE_GOD_FILE_SESSION_PROCESS_ROOT";
    const LABEL: &str = "MACHINE_GOD_FILE_SESSION_PROCESS_LABEL";
    const READY: &str = "MACHINE_GOD_FILE_SESSION_PROCESS_READY";
    const RELEASE: &str = "MACHINE_GOD_FILE_SESSION_PROCESS_RELEASE";
    const RESULT: &str = "MACHINE_GOD_FILE_SESSION_PROCESS_RESULT";

    if let (Ok(root), Ok(label), Ok(ready_path), Ok(release_path), Ok(result_path)) = (
        std::env::var(ROOT),
        std::env::var(LABEL),
        std::env::var(READY),
        std::env::var(RELEASE),
        std::env::var(RESULT),
    ) {
        let store = store(Path::new(&root));
        let mut candidate = load(&store, id("process-cas")).unwrap();
        assert_eq!(candidate.revision, SessionRevision(1));
        candidate.messages.push(Message::text(Role::User, label));
        fs::write(&ready_path, b"ready").unwrap();
        wait_for_files(&[Path::new(&release_path)]);
        let outcome = match ready(store.save(candidate, Some(SessionRevision(1)))) {
            Ok(SessionRevision(2)) => "ok",
            Err(error)
                if error.kind == SessionStoreErrorKind::Conflict
                    && error.code == "revision_conflict" =>
            {
                "conflict"
            }
            other => panic!("unexpected process CAS result: {other:?}"),
        };
        fs::write(result_path, outcome).unwrap();
        return;
    }

    let temporary = TemporaryDirectory::new();
    let root = temporary.path().join("sessions");
    fs::create_dir(&root).unwrap();
    let store = store(&root);
    assert_eq!(save_new(&store, record("process-cas")), SessionRevision(1));
    let release_path = temporary.path().join("release");
    let mut children = Vec::new();
    let mut ready_paths = Vec::new();
    let mut result_paths = Vec::new();
    for label in ["process-first", "process-second"] {
        let ready_path = temporary.path().join(format!("{label}.ready"));
        let result_path = temporary.path().join(format!("{label}.result"));
        let child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("independent_processes_serialize_exact_cas_without_lost_updates")
            .env(ROOT, &root)
            .env(LABEL, label)
            .env(READY, &ready_path)
            .env(RELEASE, &release_path)
            .env(RESULT, &result_path)
            .spawn()
            .unwrap();
        children.push(child);
        ready_paths.push(ready_path);
        result_paths.push(result_path);
    }
    wait_for_files(&ready_paths.iter().map(PathBuf::as_path).collect::<Vec<_>>());
    fs::write(&release_path, b"release").unwrap();
    for child in children {
        assert!(wait_for_child(child).success());
    }
    let outcomes = result_paths
        .iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes.iter().filter(|outcome| *outcome == "ok").count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| *outcome == "conflict")
            .count(),
        1
    );
    let persisted = load(&store, id("process-cas")).unwrap();
    assert_eq!(persisted.revision, SessionRevision(2));
    assert_eq!(persisted.messages.len(), 1);
    assert!(matches!(
        &persisted.messages[0].content[..],
        [ContentBlock::Text { text }]
            if text == "process-first" || text == "process-second"
    ));
}

#[test]
fn store_errors_do_not_disclose_root_paths_or_corrupt_contents() {
    let temporary = TemporaryDirectory::new();
    let private_root = temporary.path().join("private-session-root-secret");
    fs::create_dir(&private_root).unwrap();
    let store = store(&private_root);
    assert_eq!(save_new(&store, record("redacted")), SessionRevision(1));
    let data_path = entry_with_suffix(&private_root, ".json");
    fs::write(&data_path, b"PRIVATE_CORRUPT_CONTENT_SECRET").unwrap();
    let error = ready(store.load(id("redacted"))).unwrap_err();
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains(private_root.to_string_lossy().as_ref()));
    assert!(!rendered.contains("PRIVATE_CORRUPT_CONTENT_SECRET"));
    assert_store_error(
        &error,
        SessionStoreErrorKind::Corrupt,
        "file_session_corrupt",
        false,
    );
}
