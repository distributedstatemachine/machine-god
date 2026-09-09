use super::*;
use serde_json::{Value, json};

fn saved(state: &Path, metadata: Value) -> (FileSessionStore, SessionRecord) {
    let root = state.join("machine-god");
    private_directory(&root);
    let store = FileSessionStore::open(&root).unwrap();
    let mut record = SessionRecord::empty(
        SessionId::new("source").unwrap(),
        SessionIncarnationId::new("source-incarnation").unwrap(),
    );
    record
        .messages
        .push(Message::text(Role::User, "retained message"));
    record
        .metadata
        .insert(NATIVE_SESSION_METADATA_KEY.into(), metadata);
    let id = record.id.clone();
    ready(store.save(record, None)).unwrap();
    let record = ready(store.load(id)).unwrap().unwrap();
    (store, record)
}

fn invoke(state: &Path, args: &[&str]) -> Output {
    session_command(
        OsStr::new("intentionally-unused-relative-config"),
        state.as_os_str(),
    )
    .args(args)
    .output()
    .unwrap()
}

#[test]
fn actual_cli_migrates_native_metadata_once_and_preserves_unknown_origin() {
    let temporary = TestDirectory::new("cli-maintenance-migrate");
    let state = temporary.path().canonicalize().unwrap().join("state");
    let (store, before) = saved(&state, json!({"schema_version":1,"title":"Before"}));
    let output = invoke(&state, &["session", "migrate", "source", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "migrated");
    let after = ready(store.load(before.id.clone())).unwrap().unwrap();
    assert_eq!(after.id, before.id);
    assert_eq!(after.incarnation_id, before.incarnation_id);
    assert_eq!(after.messages, before.messages);
    assert_eq!(after.revision.0, before.revision.0 + 1);
    assert_eq!(
        after.metadata[NATIVE_SESSION_METADATA_KEY]["schema_version"],
        2
    );
    assert!(after.metadata[NATIVE_SESSION_METADATA_KEY]["origin_workspace_hex"].is_null());
    let output = invoke(&state, &["session", "migrate", "source", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "already_current");
    assert_eq!(ready(store.load(before.id)).unwrap().unwrap(), after);
}

#[test]
fn actual_cli_recovery_creates_a_valid_copy_without_modifying_corrupt_source() {
    let temporary = TestDirectory::new("cli-maintenance-recover");
    let state = temporary.path().canonicalize().unwrap().join("state");
    let (store, source) = saved(&state, json!("corrupt native metadata"));
    let (path, _) = session_artifacts(&state.join("machine-god"));
    let original = fs::read(&path).unwrap();
    let output = invoke(&state, &["session", "recover", "source", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["source_unchanged"], true);
    let id = SessionId::new(value["id"].as_str().unwrap()).unwrap();
    assert_ne!(id, source.id);
    let recovered = ready(store.load(id.clone())).unwrap().unwrap();
    assert_eq!(recovered.messages, source.messages);
    NativeSessionMetadata::from_metadata(&recovered.metadata).unwrap();
    assert_eq!(fs::read(path).unwrap(), original);
    let inspected = invoke(&state, &["session", id.as_str(), "--json"]);
    assert_eq!(inspected.status.code(), Some(0), "{inspected:?}");
}

#[test]
fn actual_cli_refuses_future_metadata_without_overwriting_it() {
    let temporary = TestDirectory::new("cli-maintenance-future");
    let state = temporary.path().canonicalize().unwrap().join("state");
    let (_store, _) = saved(&state, json!({"schema_version":99}));
    let (path, _) = session_artifacts(&state.join("machine-god"));
    let before = fs::read(&path).unwrap();
    for action in ["migrate", "recover"] {
        let output = invoke(&state, &["session", action, "source", "--json"]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["code"], "UnsupportedVersion");
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn actual_cli_cleanup_preserves_unrelated_files_in_both_modes() {
    let temporary = TestDirectory::new("cli-maintenance-cleanup");
    let state = temporary.path().canonicalize().unwrap().join("state");
    let (_store, _) = saved(&state, json!({"schema_version":2}));
    let unrelated = state.join("machine-god").join("user-notes.tmp");
    fs::write(&unrelated, b"not a native staging artifact").unwrap();
    for args in [
        vec!["doctor", "cleanup", "--json"],
        vec!["doctor", "cleanup", "--apply", "--json"],
    ] {
        let output = invoke(&state, &args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["completed"], 0);
        assert_eq!(value["scan_complete"], true);
        assert_eq!(
            fs::read(&unrelated).unwrap(),
            b"not a native staging artifact"
        );
    }
}

#[test]
fn actual_cli_cleanup_requires_apply_to_delete_a_proven_redundant_stage() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = TestDirectory::new("cli-maintenance-cleanup-apply");
    let state = temporary.path().canonicalize().unwrap().join("state");
    let (_store, _) = saved(&state, json!({"schema_version":2}));
    let (path, lock) = session_artifacts(&state.join("machine-god"));
    let original = fs::read(&path).unwrap();
    let staging_file = path.with_extension("tmp");
    fs::write(&staging_file, &original).unwrap();
    fs::set_permissions(&staging_file, fs::Permissions::from_mode(0o600)).unwrap();
    let report = invoke(&state, &["doctor", "cleanup", "--json"]);
    assert_eq!(report.status.code(), Some(0), "{report:?}");
    let value: Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(value["report_only"], 1);
    assert_eq!(value["completed"], 0);
    assert_eq!(fs::read(&staging_file).unwrap(), original);

    let applied = invoke(&state, &["doctor", "cleanup", "--apply", "--json"]);
    assert_eq!(applied.status.code(), Some(0), "{applied:?}");
    let value: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(value["completed"], 1);
    assert_eq!(value["scan_complete"], true);
    assert!(!staging_file.exists());
    assert!(lock.is_file());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn actual_cli_maintenance_missing_state_never_creates_a_root() {
    let temporary = TestDirectory::new("cli-maintenance-missing");
    let state = temporary.path().canonicalize().unwrap().join("missing");
    for args in [
        vec!["session", "migrate", "source", "--json"],
        vec!["session", "recover", "source", "--json"],
        vec!["doctor", "cleanup", "--apply", "--json"],
    ] {
        let output = invoke(&state, &args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["code"], "Missing");
        assert!(!state.exists());
    }
}
