use super::*;
use serde_json::{Value, json};

fn config(version: u32, mode: &str) -> Value {
    let mut value = json!({"schema_version":version,"permission_mode":mode,"sandbox_mode":"none",
        "permission_rules":[
            {"permission":"terminal","pattern":"echo \"safe\"\n\u{1b}[2J","action":"allow"},
            {"permission":"read_file","pattern":"**","action":"ask"},
            {"permission":"web_fetch","pattern":"INVALID_PATTERN_SECRET","action":"deny"},
            {"permission":"web_fetch","pattern":"domain:example.com","action":"deny"}],
        "provider":"vercel_ai_gateway","transport":"ai_gateway_http",
        "model":"MODEL_SECRET","credential_source":"environment","effort":"high","fast_mode":true});
    if version >= 6 {
        value["workspace_permission_rules"] = json!([]);
    }
    if version >= 7 {
        value["workspace_directories"] = json!([]);
    }
    value
}

fn inspect(config: &Path, state: &Path, cwd: &Path, json: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_machine-god"));
    command
        .current_dir(cwd)
        .env_clear()
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_STATE_HOME", state);
    command.arg("permissions");
    if json {
        command.arg("--json");
    }
    command.output().unwrap()
}

fn report(output: &Output) -> Value {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty());
    assert!(output.stdout.ends_with(b"\n"));
    for secret in ["INVALID_PATTERN_SECRET", "MODEL_SECRET"] {
        assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
    }
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn permissions_schemas_five_through_seven_report_ordered_all_decisions_without_writes() {
    let temporary = TestDirectory::new("permission-pattern-schemas");
    let config_root = temporary.path().join("config");
    let state = temporary.path().join("state");
    for version in 5..=7 {
        for mode in ["ask", "auto", "yolo"] {
            let contents = config(version, mode).to_string();
            let path = write_config(&config_root, contents.as_bytes());
            let value = report(&inspect(&config_root, &state, temporary.path(), true));
            assert_eq!(value["permission_mode"], mode);
            assert_eq!(value["configuration_origin"], "file");
            assert_eq!(value["configured_rules"]["effective_source"], "user");
            assert_eq!(value["configured_rules"]["local"], Value::Null);
            let rows = value["configured_rules"]["user"].as_array().unwrap();
            assert_eq!(rows.len(), 4);
            assert_eq!(rows[0]["action"], "allow");
            assert_eq!(rows[1]["action"], "ask");
            assert_eq!(rows[2]["action"], "deny");
            assert_eq!(rows[2]["inert"], true);
            assert_eq!(rows[2]["pattern"], Value::Null);
            assert_eq!(rows[3]["inert"], false);
            assert_eq!(value["saved_exact_rules_available"], false);
            assert_eq!(value["runtime_grants_available"], false);
            assert!(value.get("grants").is_none());
            let human = inspect(&config_root, &state, temporary.path(), false);
            assert_eq!(human.status.code(), Some(0));
            let human = String::from_utf8(human.stdout).unwrap();
            assert!(human.contains("[inert pattern omitted]"));
            assert!(!human.contains("INVALID_PATTERN_SECRET"));
            assert!(!human.contains('\u{1b}'));
            assert!(human.contains("\\u001b[2J"));
            assert_eq!(fs::read(path).unwrap(), contents.as_bytes());
            assert!(!state.exists());
            assert_eq!(
                fs::read_dir(config_root.join("machine-god"))
                    .unwrap()
                    .count(),
                1
            );
        }
    }
}

#[cfg(unix)]
fn scoped(value: &mut Value, workspace: &Path, rules: &Value) {
    use std::os::unix::ffi::OsStrExt;
    let mut hex = String::new();
    for byte in workspace.as_os_str().as_bytes() {
        write!(&mut hex, "{byte:02x}").unwrap();
    }
    value["workspace_permission_rules"] = json!([{"workspace_hex":hex,"permission_rules":rules}]);
}

#[cfg(unix)]
#[test]
fn permissions_scoped_sources_keep_empty_shadow_and_follow_canonical_alias_without_mutation() {
    let temporary = TestDirectory::new("permission-pattern-scoped");
    let workspace = temporary.path().canonicalize().unwrap();
    let config_root = workspace.join("config");
    let state = workspace.join("state");
    let alias = workspace.join("alias");
    std::os::unix::fs::symlink(&workspace, &alias).unwrap();
    for rules in [
        json!([]),
        json!([{"permission":"terminal","pattern":"local","action":"deny"}]),
    ] {
        let mut value = config(7, "ask");
        scoped(&mut value, &workspace, &rules);
        let bytes = value.to_string();
        let path = write_config(&config_root, bytes.as_bytes());
        for selected in [&workspace, &alias] {
            let value = report(&inspect(&config_root, &state, selected, true));
            assert_eq!(value["configured_rules"]["effective_source"], "local");
            assert_eq!(
                value["configured_rules"]["local"].as_array().unwrap().len(),
                rules.as_array().unwrap().len()
            );
            assert_eq!(
                value["configured_rules"]["user"].as_array().unwrap().len(),
                4
            );
        }
        let value = report(&inspect(&config_root, &state, &config_root, true));
        assert_eq!(value["configured_rules"]["effective_source"], "user");
        assert_eq!(value["configured_rules"]["local"], Value::Null);
        assert_eq!(fs::read(path).unwrap(), bytes.as_bytes());
        assert!(!state.exists());
    }
}

// macOS filesystems reject these filename bytes; native byte selection is also
// exercised with an injected workspace on every Unix target.
#[cfg(target_os = "linux")]
#[test]
fn permissions_non_unicode_cwd_selects_local_without_displaying_raw_path() {
    use std::os::unix::ffi::OsStringExt;
    let temporary = TestDirectory::new("permission-pattern-nonunicode");
    let workspace = temporary
        .path()
        .join(OsString::from_vec(b"workspace-\xff".to_vec()));
    fs::create_dir(&workspace).unwrap();
    let workspace = workspace.canonicalize().unwrap();
    let config_root = temporary.path().join("config");
    let state = temporary.path().join("state");
    let mut value = config(6, "auto");
    scoped(&mut value, &workspace, &json!([]));
    let contents = value.to_string();
    let path = write_config(&config_root, contents.as_bytes());
    let value = report(&inspect(&config_root, &state, &workspace, true));
    assert_eq!(value["configured_rules"]["effective_source"], "local");
    assert!(value.get("workspace").is_none());
    assert_eq!(fs::read(path).unwrap(), contents.as_bytes());
    assert!(!state.exists());
}

#[test]
fn permissions_large_escaped_projection_is_complete_bounded_and_bad_shape_is_redacted() {
    let temporary = TestDirectory::new("permission-pattern-bounds");
    let config_root = temporary.path().join("config");
    let state = temporary.path().join("state");
    let mut value = config(7, "ask");
    let pattern = "\u{202e}".repeat(20_000);
    value["permission_rules"] =
        json!([{"permission":"terminal","pattern":pattern,"action":"deny"}]);
    let contents = value.to_string();
    assert!(contents.len() < machine_god_native::MAX_CONFIG_BYTES);
    let path = write_config(&config_root, contents.as_bytes());
    for json in [false, true] {
        let output = inspect(&config_root, &state, temporary.path(), json);
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stdout.len() < 512 * 1024);
        assert!(!String::from_utf8_lossy(&output.stdout).contains('\u{202e}'));
        if json {
            let value = report(&output);
            assert_eq!(value["configured_rules"]["user"][0]["pattern"], pattern);
        }
    }
    assert_eq!(fs::read(&path).unwrap(), contents.as_bytes());
    value["permission_rules"][0]["action"] = "SECRET_BAD_ACTION".into();
    let invalid = value.to_string();
    write_config(&config_root, invalid.as_bytes());
    assert_config_failure(&inspect(&config_root, &state, temporary.path(), true));
    assert_eq!(fs::read(path).unwrap(), invalid.as_bytes());
    assert!(!state.exists());
}
