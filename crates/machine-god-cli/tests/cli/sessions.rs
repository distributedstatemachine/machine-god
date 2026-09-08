use super::*;
use serde_json::{Value, json};

fn save_rich_session(state: &Path, id: &str, workspace: &Path, time: i64, title: &str) {
    let root = state.join("machine-god");
    private_directory(&root);
    let store = FileSessionStore::open(&root).unwrap();
    let mut record = SessionRecord::empty(
        SessionId::new(id).unwrap(),
        SessionIncarnationId::new(format!("incarnation-{id}")).unwrap(),
    );
    let mut metadata =
        NativeSessionMetadata::new(workspace, time, NativeSessionOrigin::Cli).unwrap();
    metadata.rename(title, time).unwrap();
    metadata.set_language("en", time).unwrap();
    record
        .metadata
        .insert(NATIVE_SESSION_METADATA_KEY.into(), metadata.to_value());
    record.messages = vec![
        Message::text(Role::User, "/help"),
        Message::text(
            Role::User,
            "First prompt line\nSecond prompt line\nOmitted preview line",
        ),
        Message::text(Role::Assistant, "Assistant continuation one"),
        Message::text(Role::Assistant, "Assistant continuation two"),
    ];
    ready(store.save(record, None)).unwrap();
}

fn command(state: &Path, workspace: &Path) -> Command {
    let mut command = sessions_command(OsStr::new("ignored-relative-config"), state.as_os_str());
    command.current_dir(workspace);
    command
}

fn records(state: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut records: Vec<_> = fs::read_dir(state.join("machine-god"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    records.sort_by(|a, b| a.0.cmp(&b.0));
    records
}

fn edit_fixture_metadata(state: &Path, id: &str, edit: impl FnOnce(&mut Value)) {
    let store = FileSessionStore::open(&state.join("machine-god")).unwrap();
    let mut record = ready(store.load(SessionId::new(id).unwrap()))
        .unwrap()
        .unwrap();
    let revision = record.revision;
    edit(
        record
            .metadata
            .get_mut(NATIVE_SESSION_METADATA_KEY)
            .unwrap(),
    );
    ready(store.save(record, Some(revision))).unwrap();
}

#[test]
fn persisted_origin_and_current_workspace_are_projected_independently() {
    use std::os::unix::ffi::OsStrExt;
    let temporary = TestDirectory::new("rich-sessions-origin");
    let current = temporary.path().canonicalize().unwrap();
    let state = current.join("state");
    save_rich_session(
        &state,
        "moved",
        Path::new("/original-workspace"),
        20,
        "Moved",
    );
    let mut hex = String::new();
    for byte in current.as_os_str().as_bytes() {
        write!(hex, "{byte:02x}").unwrap();
    }
    edit_fixture_metadata(&state, "moved", |value| value["workspace_hex"] = hex.into());
    let before = records(&state);
    let output = command(&state, &current)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    let page = assert_sessions_rows(&output, &["moved"], 0);
    assert_eq!(
        page["sessions"][0]["workspace_root"],
        current.to_str().unwrap()
    );
    assert_eq!(
        page["sessions"][0]["origin_workspace_root"],
        "/original-workspace"
    );
    assert_eq!(records(&state), before);
}

#[test]
fn legacy_known_workspace_does_not_invent_an_origin_or_upgrade_the_record() {
    let temporary = TestDirectory::new("rich-sessions-legacy-origin");
    let current = temporary.path().canonicalize().unwrap();
    let state = current.join("state");
    save_rich_session(&state, "legacy", &current, 10, "Legacy");
    edit_fixture_metadata(&state, "legacy", |value| {
        value["schema_version"] = 1.into();
        value
            .as_object_mut()
            .unwrap()
            .remove("origin_workspace_hex");
    });
    let before = records(&state);
    let output = command(&state, &current)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    let page = assert_sessions_rows(&output, &["legacy"], 0);
    assert_eq!(
        page["sessions"][0]["workspace_root"],
        current.to_str().unwrap()
    );
    assert!(page["sessions"][0]["origin_workspace_root"].is_null());
    assert_eq!(records(&state), before);
}

#[test]
fn default_workspace_rich_metadata_and_known_cursor_round_trip() {
    let temporary = TestDirectory::new("rich-sessions-workspace");
    let workspace = temporary.path().canonicalize().unwrap();
    let state = workspace.join("state");
    for (id, time) in [("a", 100), ("b", 200), ("c", 200)] {
        save_rich_session(&state, id, &workspace, time, &format!("Title {id}"));
    }
    save_rich_session(
        &state,
        "outside",
        Path::new("/different-workspace"),
        500,
        "Outside",
    );
    save_session(&state, "legacy-unknown");
    let before = records(&state);
    let output = command(&state, &workspace)
        .args(["sessions", "--json", "--limit", "2"])
        .output()
        .unwrap();
    let page = assert_sessions_rows(&output, &["c", "b"], 0);
    let entry = &page["sessions"][0];
    assert_eq!(entry["title"], "Title c");
    assert_eq!(entry["workspace_root"], workspace.to_str().unwrap());
    assert_eq!(entry["origin_workspace_root"], workspace.to_str().unwrap());
    assert_eq!(entry["created_at_ms"], 200);
    assert_eq!(entry["updated_at_ms"], 200);
    assert_eq!(entry["history_len"], 2);
    assert_eq!(entry["conversation_language"], "en");
    assert_eq!(entry["preview"], "First prompt line\nSecond prompt line");
    assert_eq!(page["has_more"], true);
    assert_eq!(page["next_cursor"], "v1:200:b");
    let output = command(&state, &workspace)
        .args([
            "sessions",
            "--limit",
            "2",
            "--cursor",
            page["next_cursor"].as_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    let last = assert_sessions_rows(&output, &["a"], 0);
    assert_ne!(last["has_more"], true);
    assert!(last["next_cursor"].is_null());
    let output = command(&state, &workspace)
        .args(["sessions", "--all", "--json"])
        .output()
        .unwrap();
    let all = assert_sessions_rows(&output, &["outside", "c", "b", "a", "legacy-unknown"], 0);
    assert!(all["sessions"][4]["updated_at_ms"].is_null());
    assert_eq!(records(&state), before);
}

#[test]
fn default_scope_resolves_workspace_alias_without_creating_state() {
    let temporary = TestDirectory::new("rich-sessions-alias");
    let workspace = temporary.path().canonicalize().unwrap();
    let state = workspace.join("state");
    let alias = workspace.join("alias");
    std::os::unix::fs::symlink(&workspace, &alias).unwrap();
    save_rich_session(&state, "selected", &workspace, 10, "Known title");
    let output = command(&state, &alias)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    assert_sessions_rows(&output, &["selected"], 0);
    let missing = workspace.join("missing-state");
    let output = command(&missing, &alias)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    assert_success(
        &output,
        "{\"kind\":\"sessions\",\"count\":0,\"sessions\":[]}\n",
    );
    assert!(!missing.exists());
}

#[test]
fn mixed_corrupt_records_are_reported_without_hiding_healthy_rows_or_changing_files() {
    let temporary = TestDirectory::new("rich-sessions-corruption");
    let workspace = temporary.path().canonicalize().unwrap();
    let state = workspace.join("state");
    save_rich_session(&state, "healthy", &workspace, 10, "Known title");
    save_session(&state, "broken");
    let bad_path = records(&state)
        .into_iter()
        .find(|(_, bytes)| {
            let value: Value = serde_json::from_slice(bytes).unwrap();
            value["record"]["id"] == "broken"
        })
        .unwrap()
        .0;
    fs::write(&bad_path, b"PRIVATE_CORRUPT_RECORD_SECRET").unwrap();
    let before = records(&state);
    let output = command(&state, &workspace)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    let page = assert_sessions_rows(&output, &["healthy"], 1);
    assert!(page["next_cursor"].is_null());
    assert_output_omits(&output, &["broken", "PRIVATE_CORRUPT_RECORD_SECRET"]);
    assert_eq!(records(&state), before);
}

#[test]
fn noncanonical_cursors_and_repeated_flags_fail_before_session_access() {
    let temporary = TestDirectory::new("rich-sessions-grammar");
    let state = temporary.path().join("must-not-exist");
    for arguments in [
        vec!["sessions", "--all", "--all"],
        vec!["sessions", "--limit", "0"],
        vec!["sessions", "--limit", "101"],
        vec!["sessions", "--limit", "2", "--limit", "3"],
        vec!["sessions", "--limit"],
        vec!["sessions", "--cursor"],
        vec!["sessions", "--cursor", "v1:020:a"],
        vec!["sessions", "--cursor", "v1:+20:a"],
        vec!["sessions", "--cursor", "v1:-0:a"],
        vec!["sessions", "--cursor", "v2:20:a"],
        vec!["sessions", "--cursor", "v1:20:../unsafe"],
        vec!["sessions", "--cursor", "v1:20:a", "--cursor", "v1:10:a"],
    ] {
        let output = command(&state, temporary.path())
            .args(&arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8_lossy(&output.stderr), INVALID_ARGUMENTS);
        assert!(!state.exists());
    }
}

#[test]
fn human_output_escapes_format_controls_and_json_preserves_explicit_facts() {
    let temporary = TestDirectory::new("rich-sessions-display");
    let workspace = temporary.path().canonicalize().unwrap();
    let state = workspace.join("state");
    let title = "Title\u{85}\u{202e}suffix";
    save_rich_session(&state, "display", &workspace, 10, title);
    let output = command(&state, &workspace)
        .args(["sessions"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains('\u{85}'));
    assert!(!text.contains('\u{202e}'));
    assert!(text.contains("Title"));
    let output = command(&state, &workspace)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    let page = assert_sessions_rows(&output, &["display"], 0);
    assert_eq!(page["sessions"][0]["title"], json!(title));
}

#[test]
fn scan_incompleteness_is_not_a_false_continuation_promise() {
    let temporary = TestDirectory::new("rich-sessions-scan-bound");
    let workspace = temporary.path().canonicalize().unwrap();
    let state = workspace.join("state");
    save_rich_session(&state, "a", &workspace, 100, "First");
    save_rich_session(&state, "b", &workspace, 200, "Second");
    let root = state.join("machine-god");
    for index in 0..1025 {
        fs::write(root.join(format!("unrelated-{index}")), b"").unwrap();
    }
    let before = records(&state);
    let output = command(&state, &workspace)
        .args(["sessions", "--json", "--limit", "1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let page: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(page["kind"], "sessions");
    assert_eq!(page["scan_complete"], false);
    assert_eq!(page["truncated"], true);
    assert_ne!(page["has_more"], true);
    assert!(page["next_cursor"].is_null());
    assert!(page["sessions"].as_array().unwrap().len() <= 1);
    let output = command(&state, &workspace)
        .args(["sessions", "--limit", "1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("listing incomplete"));
    assert!(!text.contains("no saved sessions"));
    assert!(!text.contains("no readable saved sessions"));
    assert_eq!(records(&state), before);
}

#[test]
fn non_unicode_workspace_metadata_is_reported_without_loss_or_path_access() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let temporary = TestDirectory::new("rich-sessions-non-unicode");
    let workspace = temporary
        .path()
        .canonicalize()
        .unwrap()
        .join(OsString::from_vec(b"workspace-\xff".to_vec()));
    let state = temporary.path().join("state");
    save_rich_session(&state, "bytes", &workspace, 10, "Byte-preserving workspace");
    let output = command(&state, temporary.path())
        .args(["sessions", "--all", "--json"])
        .output()
        .unwrap();
    let page = assert_sessions_rows(&output, &["bytes"], 0);
    let entry = &page["sessions"][0];
    assert!(entry["workspace_root"].is_null());
    let mut hex = String::new();
    for byte in workspace.as_os_str().as_bytes() {
        write!(hex, "{byte:02x}").unwrap();
    }
    assert_eq!(entry["workspace_root_hex"], hex);
    assert!(entry["origin_workspace_root"].is_null());
    assert_eq!(entry["origin_workspace_root_hex"], hex);
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains('\u{fffd}')
    );
}

#[cfg(target_os = "linux")]
#[test]
fn an_actual_non_unicode_working_directory_filters_exact_native_bytes() {
    use std::os::unix::ffi::OsStringExt;
    let temporary = TestDirectory::new("rich-sessions-byte-workspace");
    let workspace = temporary
        .path()
        .join(OsString::from_vec(b"workspace-\xff".to_vec()));
    fs::create_dir(&workspace).unwrap();
    let workspace = workspace.canonicalize().unwrap();
    let state = temporary.path().join("state");
    save_rich_session(&state, "bytes", &workspace, 10, "Byte-preserving workspace");
    save_rich_session(&state, "other", Path::new("/workspace-other"), 20, "Other");
    let output = command(&state, &workspace)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    assert_sessions_rows(&output, &["bytes"], 0);
}
