#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::ffi::OsString;
use std::fs;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::fs::FileTimes;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "linux")]
use std::time::SystemTime;

use futures_executor::block_on;
use machine_god_native::{
    MAX_BACKGROUND_RECORD_BYTES, MAX_BACKGROUND_STATE_BASE_BYTES, NativeBackgroundInspection,
    NativeBackgroundInspectionErrorKind, NativeBackgroundQuery, NativeBackgroundRecordSummary,
    NativeBackgroundState, NativeEnvironment, inspect_native_background,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const WORKSPACE_DIGEST_DOMAIN: &[u8] = b"machine-god:background-workspace:v1:";
const RECORD_DIGEST_DOMAIN: &[u8] = b"machine-god:background-record:v1:";

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

#[cfg(target_os = "macos")]
struct MacAclCleanup(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for MacAclCleanup {
    fn drop(&mut self) {
        let _ = Command::new("/bin/chmod").arg("-N").arg(&self.0).status();
    }
}

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
                    return Self(fs::canonicalize(path).unwrap());
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
    for id in 1..=101 {
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
fn zero_id_persisted_record_is_corrupt_before_list_projection() {
    let fixture = Fixture::new();
    fixture.write_record(0, 20, "PRIVATE_ZERO_ID_COMMAND");

    let error = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::List,
    ))
    .unwrap_err();
    assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);
    assert!(!format!("{error:?}").contains("PRIVATE_ZERO_ID_COMMAND"));
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

#[test]
fn record_size_limit_accepts_exact_and_rejects_one_byte_overflow_witness() {
    let exact = Fixture::new();
    let mut exact_bytes = serde_json::to_vec(&exact.record_value(14, 20, "command")).unwrap();
    exact_bytes.resize(MAX_BACKGROUND_RECORD_BYTES, b' ');
    exact.write_bytes(14, &exact_bytes);
    let result = block_on(inspect_native_background(
        exact.environment(),
        exact.workspace.clone(),
        NativeBackgroundQuery::Id(14),
    ));
    assert!(result.is_ok());

    let overflow = Fixture::new();
    let mut overflow_bytes = serde_json::to_vec(&overflow.record_value(15, 20, "command")).unwrap();
    overflow_bytes.resize(MAX_BACKGROUND_RECORD_BYTES + 1, b' ');
    overflow.write_bytes(15, &overflow_bytes);
    let error = block_on(inspect_native_background(
        overflow.environment(),
        overflow.workspace.clone(),
        NativeBackgroundQuery::Id(15),
    ))
    .unwrap_err();
    assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);
}

#[cfg(target_os = "macos")]
#[test]
fn selected_record_with_access_granting_acl_is_corrupt() {
    let fixture = Fixture::new();
    let record = fixture.write_record(16, 20, "command");
    let status = Command::new("/bin/chmod")
        .args(["+a", "everyone allow read"])
        .arg(&record)
        .status()
        .expect("macOS chmod executable is available");
    assert!(
        status.success(),
        "failed to install record ACL fixture: {status}"
    );
    let _acl_cleanup = MacAclCleanup(record);

    let error = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::Id(16),
    ))
    .unwrap_err();
    assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);
}

#[cfg(target_os = "macos")]
#[test]
fn selected_record_with_read_denying_acl_is_corrupt() {
    let fixture = Fixture::new();
    let record = fixture.write_record(19, 20, "command");
    let status = Command::new("/bin/chmod")
        .args(["+a", "everyone deny read"])
        .arg(&record)
        .status()
        .expect("macOS chmod executable is available");
    assert!(
        status.success(),
        "failed to install read-denying record ACL fixture: {status}"
    );
    let _acl_cleanup = MacAclCleanup(record);

    let error = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::Id(19),
    ))
    .unwrap_err();
    assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);
}

#[cfg(target_os = "macos")]
#[test]
fn selected_record_accepts_a_protective_deny_delete_acl() {
    let fixture = Fixture::new();
    let record = fixture.write_record(17, 20, "command");
    let status = Command::new("/bin/chmod")
        .args(["+a", "everyone deny delete"])
        .arg(&record)
        .status()
        .expect("macOS chmod executable is available");
    assert!(
        status.success(),
        "failed to install protective record ACL fixture: {status}"
    );
    let _acl_cleanup = MacAclCleanup(record);

    let result = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::Id(17),
    ));
    assert!(result.is_ok());
}

#[test]
fn record_paths_reject_noncanonical_lexical_spellings() {
    let fixture = Fixture::new();
    let workspace = fixture.workspace.to_str().unwrap();
    for invalid in [
        format!("{workspace}/."),
        format!("{workspace}/child/../child"),
        format!("{workspace}//child"),
        format!("{workspace}/"),
    ] {
        let mut value = fixture.record_value(18, 20, "command");
        value["cwd"] = json!(invalid);
        fixture.write_value(18, &value);

        let error = block_on(inspect_native_background(
            fixture.environment(),
            fixture.workspace.clone(),
            NativeBackgroundQuery::Id(18),
        ))
        .unwrap_err();
        assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);
    }

    let mut value = fixture.record_value(18, 20, "command");
    value["workspace"] = json!(format!("{workspace}/."));
    fixture.write_value(18, &value);
    let error = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::Id(18),
    ))
    .unwrap_err();
    assert_eq!(error.kind(), NativeBackgroundInspectionErrorKind::Corrupt);
}

#[test]
fn xdg_and_home_state_bases_reject_symlinks_in_ancestors() {
    let temporary = TempDirectory::new();
    let workspace = temporary.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    private_directory(&workspace);

    let real = temporary.path().join("real");
    fs::create_dir(&real).unwrap();
    private_directory(&real);
    let linked = temporary.path().join("linked");
    symlink(&real, &linked).unwrap();

    let environments = [
        NativeEnvironment::new(None, Some(linked.join("state").into_os_string()), None),
        NativeEnvironment::new(None, None, Some(linked.join("home").into_os_string())),
    ];
    for environment in environments {
        let error = block_on(inspect_native_background(
            environment,
            workspace.clone(),
            NativeBackgroundQuery::List,
        ))
        .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeBackgroundInspectionErrorKind::Unavailable
        );
    }
}

#[test]
fn xdg_and_home_state_bases_accept_normalized_absolute_spellings() {
    let temporary = TempDirectory::new();
    let workspace = temporary.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    private_directory(&workspace);

    let state = temporary.path().join("state");
    let home = temporary.path().join("home");
    fs::create_dir(&state).unwrap();
    fs::create_dir(&home).unwrap();
    private_directory(&state);
    private_directory(&home);

    let state_spelling = OsString::from(format!("{}//./", state.display()));
    let home_spelling = OsString::from(format!("{}//./", home.display()));
    let environments = [
        NativeEnvironment::new(None, Some(state_spelling), None),
        NativeEnvironment::new(None, None, Some(home_spelling)),
    ];
    for environment in environments {
        let NativeBackgroundInspection::List(list) = block_on(inspect_native_background(
            environment,
            workspace.clone(),
            NativeBackgroundQuery::List,
        ))
        .unwrap() else {
            panic!("expected list");
        };
        assert!(list.records().is_empty());
        assert!(!list.truncated());
    }
}

#[test]
fn xdg_state_home_raw_byte_limit_accepts_exact_and_rejects_one_over() {
    state_base_raw_byte_limit_accepts_exact_and_rejects_one_over(true);
}

#[test]
fn home_raw_byte_limit_accepts_exact_and_rejects_one_over() {
    state_base_raw_byte_limit_accepts_exact_and_rejects_one_over(false);
}

fn state_base_raw_byte_limit_accepts_exact_and_rejects_one_over(use_xdg: bool) {
    let temporary = TempDirectory::new();
    let workspace = temporary.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    private_directory(&workspace);

    let base = temporary.path().join("base");
    fs::create_dir(&base).unwrap();
    private_directory(&base);
    let base_bytes = base.as_os_str().as_bytes();
    assert!(base_bytes.len() + 2 < MAX_BACKGROUND_STATE_BASE_BYTES);

    let exact = normalized_spelling_with_raw_length(base_bytes, MAX_BACKGROUND_STATE_BASE_BYTES);
    let environment = if use_xdg {
        NativeEnvironment::new(None, Some(exact), None)
    } else {
        NativeEnvironment::new(None, None, Some(exact))
    };
    let result = block_on(inspect_native_background(
        environment,
        workspace.clone(),
        NativeBackgroundQuery::List,
    ));
    assert!(result.is_ok());

    let overflow =
        normalized_spelling_with_raw_length(base_bytes, MAX_BACKGROUND_STATE_BASE_BYTES + 1);
    let environment = if use_xdg {
        NativeEnvironment::new(None, Some(overflow), None)
    } else {
        NativeEnvironment::new(None, None, Some(overflow))
    };
    let error = block_on(inspect_native_background(
        environment,
        workspace.clone(),
        NativeBackgroundQuery::List,
    ))
    .unwrap_err();
    assert_eq!(
        error.kind(),
        NativeBackgroundInspectionErrorKind::ResourceLimit
    );
}

fn normalized_spelling_with_raw_length(base: &[u8], length: usize) -> OsString {
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(base);
    bytes.resize(length - 1, b'/');
    bytes.push(b'.');
    assert_eq!(bytes.len(), length);
    OsString::from_vec(bytes)
}

#[cfg(target_os = "linux")]
#[test]
fn successful_listing_does_not_advance_record_or_directory_access_times() {
    let fixture = Fixture::new();
    let record = fixture.write_record(20, 20, "command");
    set_old_access_time(&record);
    set_old_access_time(&fixture.record_root);
    let record_atime = access_time(&record);
    let directory_atime = access_time(&fixture.record_root);

    let result = block_on(inspect_native_background(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundQuery::List,
    ));
    assert!(result.is_ok());
    assert_eq!(access_time(&record), record_atime);
    assert_eq!(access_time(&fixture.record_root), directory_atime);
}

#[cfg(target_os = "linux")]
fn set_old_access_time(path: &Path) {
    let file = File::open(path).unwrap();
    let modified = file.metadata().unwrap().modified().unwrap();
    file.set_times(
        FileTimes::new()
            .set_accessed(SystemTime::UNIX_EPOCH)
            .set_modified(modified),
    )
    .unwrap();
}

#[cfg(target_os = "linux")]
fn access_time(path: &Path) -> (i64, i64) {
    let metadata = fs::metadata(path).unwrap();
    (metadata.atime(), metadata.atime_nsec())
}
