#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures_executor::block_on;
use machine_god_native::{
    NativeBackgroundInspection, NativeBackgroundInspectionErrorKind, NativeBackgroundQuery,
    NativeBackgroundRecordSummary, NativeBackgroundState, NativeEnvironment,
    inspect_native_background,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const WORKSPACE_DIGEST_DOMAIN: &[u8] = b"machine-god:background-workspace:v1:";
const RECORD_DIGEST_DOMAIN: &[u8] = b"machine-god:background-record:v1:";

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        loop {
            let sequence = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "machine-god-background-inspection-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    private_directory(&path);
                    return Self(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create temporary directory: {error}"),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    _temporary: TempDirectory,
    state_base: PathBuf,
    workspace: PathBuf,
    record_root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDirectory::new();
        let state_base = temporary.path().join("state");
        let workspace = temporary.path().join("workspace");
        fs::create_dir(&state_base).unwrap();
        fs::create_dir(&workspace).unwrap();
        private_directory(&state_base);
        private_directory(&workspace);
        let workspace = fs::canonicalize(workspace).unwrap();
        let record_root = state_base
            .join("machine-god")
            .join("background-v1")
            .join(workspace_name(workspace.to_str().unwrap()));
        fs::create_dir_all(&record_root).unwrap();
        for directory in [
            state_base.join("machine-god"),
            state_base.join("machine-god/background-v1"),
            record_root.clone(),
        ] {
            private_directory(&directory);
        }
        Self {
            _temporary: temporary,
            state_base,
            workspace,
            record_root,
        }
    }

    fn environment(&self) -> NativeEnvironment {
        NativeEnvironment::new(None, Some(self.state_base.clone().into_os_string()), None)
    }

    fn write_record(&self, id: u64, updated_at_ms: u64, command: &str) -> PathBuf {
        let value = self.record_value(id, updated_at_ms, command);
        self.write_value(id, &value)
    }

    fn record_value(&self, id: u64, updated_at_ms: u64, command: &str) -> Value {
        json!({
            "version": 1,
            "workspace": self.workspace.to_str().unwrap(),
            "id": id,
            "started_at_ms": 10,
            "updated_at_ms": updated_at_ms,
            "command": command,
            "cwd": self.workspace.to_str().unwrap(),
            "state": "exited",
            "pid": null,
            "exit_code": 0,
            "server_url": null,
            "diagnostic": null
        })
    }

    fn write_value(&self, id: u64, value: &Value) -> PathBuf {
        self.write_bytes(id, &serde_json::to_vec(value).unwrap())
    }

    fn write_bytes(&self, id: u64, value: &[u8]) -> PathBuf {
        let path = self.record_root.join(record_name(id));
        fs::write(&path, value).unwrap();
        private_file(&path);
        path
    }
}

fn private_directory(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn private_file(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn workspace_name(workspace: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(WORKSPACE_DIGEST_DOMAIN);
    hasher.update(workspace.as_bytes());
    format!("workspace-{:x}", hasher.finalize())
}

fn record_name(id: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(RECORD_DIGEST_DOMAIN);
    hasher.update(id.to_be_bytes());
    format!("record-{:x}.json", hasher.finalize())
}

#[test]
fn list_is_ordered_bounded_and_exposes_utf8_safe_previews() {
    let fixture = Fixture::new();
    fixture.write_record(3, 30, &format!("{}é", "a".repeat(255)));
    fixture.write_record(7, 30, "second");
    fixture.write_record(9, 20, "oldest");

    let result = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::List,
    ))
    .unwrap();
    let NativeBackgroundInspection::List(list) = result else {
        panic!("expected list");
    };
    assert!(!list.truncated());
    assert_eq!(
        list.records()
            .iter()
            .map(NativeBackgroundRecordSummary::id)
            .collect::<Vec<_>>(),
        [7, 3, 9]
    );
    let truncated = &list.records()[1];
    assert_eq!(truncated.command_preview().len(), 255);
    assert!(truncated.preview_truncated());
    assert_eq!(truncated.state(), NativeBackgroundState::Exited);

    let NativeBackgroundInspection::Detail(latest) = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::Last,
    ))
    .unwrap() else {
        panic!("expected latest detail");
    };
    assert_eq!(latest.id(), 7);
}

#[test]
fn exact_id_returns_full_detail_and_ignores_other_corrupt_candidates() {
    let fixture = Fixture::new();
    fixture.write_record(42, 20, "cargo test --workspace");
    fs::write(fixture.record_root.join(record_name(99)), b"not-json").unwrap();
    private_file(&fixture.record_root.join(record_name(99)));

    let result = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::Id(42),
    ))
    .unwrap();
    let NativeBackgroundInspection::Detail(detail) = result else {
        panic!("expected detail");
    };
    assert_eq!(detail.id(), 42);
    assert_eq!(detail.state(), NativeBackgroundState::Exited);
    assert_eq!(detail.started_at_ms(), 10);
    assert_eq!(detail.updated_at_ms(), 20);
    assert_eq!(detail.pid(), None);
    assert_eq!(detail.command(), "cargo test --workspace");
    assert_eq!(detail.cwd(), fixture.workspace.to_str().unwrap());
    assert_eq!(detail.exit_code(), Some(0));
    assert_eq!(detail.server_url(), None);
    assert_eq!(detail.diagnostic(), None);

    let error = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::List,
    ))
    .unwrap_err();
    assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);
}

#[test]
fn last_refuses_to_guess_after_record_limit_truncation() {
    let fixture = Fixture::new();
    for id in 0..=100 {
        fixture.write_record(id, id + 10, "bounded");
    }

    let error = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::Last,
    ))
    .unwrap_err();
    assert_eq!(
        error.kind(),
        NativeBackgroundInspectionErrorKind::ResourceLimit
    );

    let NativeBackgroundInspection::List(list) = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::List,
    ))
    .unwrap() else {
        panic!("expected list");
    };
    assert_eq!(list.records().len(), 100);
    assert!(list.truncated());
}

#[test]
fn missing_hierarchy_is_empty_for_list_and_not_found_for_detail() {
    let temporary = TempDirectory::new();
    let workspace = temporary.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    private_directory(&workspace);
    let state = temporary.path().join("missing-state");
    let environment = NativeEnvironment::new(None, Some(state.into_os_string()), None);

    let NativeBackgroundInspection::List(list) = block_on(inspect_native_background(
        environment.clone(),
        workspace.clone(),
        NativeBackgroundQuery::List,
    ))
    .unwrap() else {
        panic!("expected list");
    };
    assert!(list.records().is_empty());
    assert!(!list.truncated());

    let error = block_on(inspect_native_background(
        environment,
        workspace,
        NativeBackgroundQuery::Id(1),
    ))
    .unwrap_err();
    assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::NotFound);
}

#[test]
fn construction_is_inert_until_the_future_is_polled() {
    let temporary = TempDirectory::new();
    let state_base = temporary.path().join("state");
    let workspace = temporary.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    private_directory(&workspace);
    let future = inspect_native_background(
        NativeEnvironment::new(None, Some(OsString::from(state_base.as_os_str())), None),
        workspace,
        NativeBackgroundQuery::List,
    );
    assert!(!state_base.exists());
    drop(future);
    assert!(!state_base.exists());
}

#[test]
fn schema_filename_and_selected_file_type_are_strict() {
    let fixture = Fixture::new();
    let mut wrong = json!({
        "version": 1,
        "workspace": fixture.workspace.to_str().unwrap(),
        "id": 8,
        "started_at_ms": 10,
        "updated_at_ms": 10,
        "command": "command",
        "cwd": fixture.workspace.to_str().unwrap(),
        "state": "running",
        "pid": null,
        "exit_code": null,
        "server_url": null,
        "diagnostic": null
    });
    wrong["unexpected"] = json!(true);
    fixture.write_value(8, &wrong);
    let error = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::Id(8),
    ))
    .unwrap_err();
    assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);

    fs::remove_file(fixture.record_root.join(record_name(8))).unwrap();
    let target = fixture.record_root.join("target");
    fs::write(&target, b"{}").unwrap();
    private_file(&target);
    symlink(&target, fixture.record_root.join(record_name(8))).unwrap();
    let error = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::Id(8),
    ))
    .unwrap_err();
    assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);
}

#[test]
fn every_nullable_schema_field_is_required_even_when_null() {
    for field in ["pid", "exit_code", "server_url", "diagnostic"] {
        let fixture = Fixture::new();
        let mut value = fixture.record_value(12, 20, "command");
        value.as_object_mut().unwrap().remove(field);
        fixture.write_value(12, &value);

        let error = block_on(inspect_native_background(
            fixture.environment(),
            fixture.workspace.clone(),
            NativeBackgroundQuery::Id(12),
        ))
        .unwrap_err();
        assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);
    }
}

#[test]
fn duplicate_schema_fields_are_rejected_before_projection() {
    let fixture = Fixture::new();
    let encoded = serde_json::to_string(&fixture.record_value(13, 20, "command")).unwrap();
    let duplicated = encoded.replacen("\"pid\":null", "\"pid\":null,\"pid\":null", 1);
    fixture.write_bytes(13, duplicated.as_bytes());

    let error = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::Id(13),
    ))
    .unwrap_err();
    assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);
}
