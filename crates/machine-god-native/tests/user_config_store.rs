#![cfg(any(target_os = "linux", target_os = "macos"))]

use futures_executor::block_on;
use machine_god_native::{
    ConfigOrigin, NativeConfiguredPermissionMutation as Mutation,
    NativeConfiguredPermissionMutationOutcome as MutationOutcome,
    NativeConfiguredPermissionReset as Reset, NativeConfiguredPermissionScope as Scope,
    NativeModelPreferences, NativeReasoningEffort, NativeUserConfigError, NativeUserConfigStore,
};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mg-config-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn root(&self) -> PathBuf {
        self.0.join("machine-god")
    }
    fn store(&self) -> NativeUserConfigStore {
        NativeUserConfigStore::new(self.root())
    }
    fn write(&self, bytes: &[u8]) {
        fs::create_dir(self.root()).unwrap();
        fs::set_permissions(self.root(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(self.root().join("config.json"), bytes).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn preferences(model: &str) -> NativeModelPreferences {
    NativeModelPreferences::new(
        model,
        NativeReasoningEffort::parse("future-tier").unwrap(),
        true,
    )
    .unwrap()
}

fn add_rule(pattern: &str) -> Mutation {
    Mutation::Add {
        permission: "bash".into(),
        pattern: pattern.into(),
    }
}

#[test]
fn public_workspace_transaction_roundtrips_raw_source_identity_and_receipts() {
    use machine_god_native::{
        NativeSavedWorkspaceDirectory, NativeWorkspaceCommitDurability,
        NativeWorkspaceDirectoryMutation,
    };
    let fixture = Fixture::new();
    let store = fixture.store();
    let record =
        NativeSavedWorkspaceDirectory::new(b"/source-\xff", b"/identity-\xfe", false).unwrap();
    let mutation = NativeWorkspaceDirectoryMutation::Add(record.clone());
    let receipt =
        block_on(store.apply_workspace_directory_mutation(b"/primary-\x80", &mutation, &[]))
            .unwrap();
    assert!(receipt.changed);
    assert_eq!(
        receipt.durability,
        NativeWorkspaceCommitDurability::Confirmed
    );
    assert!(receipt.before.is_empty());
    assert_eq!(receipt.after, vec![record.clone()]);
    let observed = store.load().unwrap();
    assert_eq!(observed.loaded(), &receipt.loaded);
    assert_eq!(
        observed
            .loaded()
            .config()
            .saved_workspace_directories(b"/primary-\x80")
            .unwrap(),
        &[record]
    );
    let unchanged =
        block_on(store.apply_workspace_directory_mutation(b"/primary-\x80", &mutation, &[]))
            .unwrap();
    assert!(!unchanged.changed);
    assert_eq!(unchanged.before, unchanged.after);
}

#[test]
fn permission_edits_are_inert_and_missing_noops_create_nothing() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let workspace = std::path::Path::new("/work");
    let mutation = add_rule("git *");
    drop(store.apply_permission_mutation(&snapshot, workspace, Scope::Local, &mutation));
    assert!(!fixture.root().exists());
    for scope in [Scope::User, Scope::Local] {
        let receipt = block_on(store.apply_permission_mutation(
            &snapshot,
            workspace,
            scope,
            &Mutation::Reset(Reset::All),
        ))
        .unwrap();
        assert_eq!(receipt.outcome, MutationOutcome::Unchanged);
        assert_eq!(receipt.loaded.origin(), ConfigOrigin::BuiltInDefaults);
        assert!(!fixture.root().exists());
    }
    let receipt =
        block_on(store.apply_permission_mutation(&snapshot, workspace, Scope::Local, &mutation))
            .unwrap();
    assert_eq!(
        receipt.outcome,
        MutationOutcome::Changed { removed_rules: 0 }
    );
    assert_eq!(receipt.loaded.config().schema_version(), 7);
    let sources = receipt
        .loaded
        .config()
        .permission_sources(workspace)
        .unwrap();
    assert_eq!(sources.effective().rules()[0].pattern(), "git *");
    assert!(sources.user().rules().is_empty());
    assert_eq!(store.load().unwrap().loaded(), &receipt.loaded);
}

#[test]
fn permission_noop_preserves_legacy_bytes_schema_and_no_lock() {
    let fixture = Fixture::new();
    let original = br#"{"schema_version":1,"permission_mode":"ask"}"#;
    fixture.write(original);
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let receipt = block_on(store.apply_permission_mutation(
        &snapshot,
        std::path::Path::new("/work"),
        Scope::Local,
        &Mutation::Reset(Reset::All),
    ))
    .unwrap();
    assert_eq!(receipt.outcome, MutationOutcome::Unchanged);
    assert_eq!(receipt.loaded.config().schema_version(), 1);
    assert_eq!(
        fs::read(fixture.root().join("config.json")).unwrap(),
        original
    );
    assert!(!fixture.root().join(".config.lock").exists());
}

#[test]
fn permission_mutations_preserve_hidden_users_other_workspaces_and_model_updates() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let work = std::path::Path::new("/work");
    let other = std::path::Path::new("/other");
    for (path, scope, pattern) in [
        (work, Scope::User, "user *"),
        (work, Scope::Local, "local *"),
        (other, Scope::Local, "other *"),
    ] {
        block_on(store.apply_permission_mutation(
            &store.load().unwrap(),
            path,
            scope,
            &add_rule(pattern),
        ))
        .unwrap();
    }
    let observed = store.load().unwrap();
    let prefs = preferences("new/model");
    let saved = block_on(store.set_model_preferences(&observed, &prefs)).unwrap();
    assert_eq!(saved.config().model_preferences(), prefs);
    let work_sources = saved.config().permission_sources(work).unwrap();
    assert_eq!(work_sources.user().rules()[0].pattern(), "user *");
    assert_eq!(work_sources.effective().rules()[0].pattern(), "local *");
    let other_sources = saved.config().permission_sources(other).unwrap();
    assert_eq!(other_sources.effective().rules()[0].pattern(), "other *");
    let removed = block_on(store.apply_permission_mutation(
        &store.load().unwrap(),
        work,
        Scope::Local,
        &Mutation::Remove {
            permission: "bash".into(),
            pattern: "local *".into(),
        },
    ))
    .unwrap();
    assert_eq!(
        removed.outcome,
        MutationOutcome::Changed { removed_rules: 1 }
    );
    assert!(
        removed
            .loaded
            .config()
            .permission_sources(work)
            .unwrap()
            .effective()
            .rules()
            .is_empty()
    );
    let before = fs::read(fixture.root().join("config.json")).unwrap();
    let unchanged = block_on(store.apply_permission_mutation(
        &store.load().unwrap(),
        work,
        Scope::Local,
        &Mutation::Reset(Reset::All),
    ))
    .unwrap();
    assert_eq!(unchanged.outcome, MutationOutcome::Unchanged);
    assert_eq!(
        fs::read(fixture.root().join("config.json")).unwrap(),
        before
    );
    assert!(
        unchanged
            .loaded
            .config()
            .permission_sources(work)
            .unwrap()
            .user_shadowed_by_local()
    );
    block_on(store.apply_permission_mutation(
        &store.load().unwrap(),
        work,
        Scope::Local,
        &add_rule("reset-me"),
    ))
    .unwrap();
    let revealed = block_on(store.apply_permission_mutation(
        &store.load().unwrap(),
        work,
        Scope::Local,
        &Mutation::Reset(Reset::All),
    ))
    .unwrap();
    let sources = revealed.loaded.config().permission_sources(work).unwrap();
    assert!(!sources.user_shadowed_by_local());
    assert_eq!(sources.effective().rules()[0].pattern(), "user *");
    assert_eq!(
        revealed
            .loaded
            .config()
            .permission_sources(other)
            .unwrap()
            .effective()
            .rules()[0]
            .pattern(),
        "other *"
    );
    assert_eq!(revealed.loaded.config().model_preferences(), prefs);
}

#[test]
fn permission_edits_reject_foreign_stale_and_replaced_snapshots_even_for_noops() {
    let fixture = Fixture::new();
    fixture.write(br#"{"schema_version":1,"permission_mode":"ask"}"#);
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let foreign = fixture.store();
    let work = std::path::Path::new("/work");
    assert!(matches!(
        block_on(foreign.apply_permission_mutation(
            &snapshot,
            work,
            Scope::Local,
            &Mutation::Reset(Reset::All)
        )),
        Err(NativeUserConfigError::Conflict)
    ));
    block_on(store.apply_permission_mutation(&snapshot, work, Scope::Local, &add_rule("saved")))
        .unwrap();
    assert!(matches!(
        block_on(store.apply_permission_mutation(
            &snapshot,
            work,
            Scope::User,
            &Mutation::Reset(Reset::All)
        )),
        Err(NativeUserConfigError::Conflict)
    ));
    let current = store.load().unwrap();
    fs::rename(fixture.root(), fixture.0.join("original")).unwrap();
    fs::create_dir(fixture.root()).unwrap();
    fs::set_permissions(fixture.root(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        block_on(store.apply_permission_mutation(
            &current,
            work,
            Scope::User,
            &Mutation::Reset(Reset::All)
        )),
        Err(NativeUserConfigError::Conflict)
    ));
}

#[test]
fn permission_invalid_candidates_do_not_create_directory_lock_or_temp() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    for (path, mutation) in [
        ("relative", add_rule("x")),
        (
            "/work",
            add_rule(&"x".repeat(machine_god_native::MAX_CONFIG_BYTES)),
        ),
        (
            "/work",
            Mutation::Add {
                permission: " ".into(),
                pattern: "*".into(),
            },
        ),
    ] {
        assert!(matches!(
            block_on(store.apply_permission_mutation(
                &snapshot,
                std::path::Path::new(path),
                Scope::Local,
                &mutation
            )),
            Err(NativeUserConfigError::InvalidConfig(_))
        ));
        assert!(!fixture.root().exists());
    }
}

#[test]
fn permission_edits_report_busy_and_leave_existing_artifacts_untouched() {
    let fixture = Fixture::new();
    fixture.write(br#"{"schema_version":1,"permission_mode":"ask"}"#);
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let lock = fs::File::create(fixture.root().join(".config.lock")).unwrap();
    lock.set_permissions(fs::Permissions::from_mode(0o600))
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
    let work = std::path::Path::new("/work");
    assert!(matches!(
        block_on(store.apply_permission_mutation(&snapshot, work, Scope::Local, &add_rule("busy"))),
        Err(NativeUserConfigError::Busy)
    ));
    drop(lock);
    fs::write(fixture.root().join(".config.tmp"), "untrusted").unwrap();
    assert!(matches!(
        block_on(store.apply_permission_mutation(
            &snapshot,
            work,
            Scope::Local,
            &add_rule("collision")
        )),
        Err(NativeUserConfigError::Persistence)
    ));
    assert_eq!(
        fs::read_to_string(fixture.root().join(".config.tmp")).unwrap(),
        "untrusted"
    );
    assert_eq!(store.load().unwrap().loaded().config().schema_version(), 1);
}

#[test]
fn missing_load_and_unpolled_write_are_inert_then_publish_current_defaults() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    assert_eq!(snapshot.loaded().origin(), ConfigOrigin::BuiltInDefaults);
    assert!(!fixture.root().exists());
    let prefs = preferences("提供者/模型");
    drop(store.set_model_preferences(&snapshot, &prefs));
    assert!(!fixture.root().exists());
    let saved = block_on(store.set_model_preferences(&snapshot, &prefs)).unwrap();
    assert_eq!(saved.config().schema_version(), 7);
    assert_eq!(
        saved.config().sandbox_mode(),
        machine_god_native::NativeSandboxMode::None
    );
    assert_eq!(saved.config().model_preferences(), prefs);
    assert_eq!(store.load().unwrap().loaded(), &saved);
    assert_eq!(
        fs::metadata(fixture.root().join("config.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!fixture.root().join(".config.tmp").exists());
}

#[test]
fn legacy_versions_are_not_migrated_until_explicit_write_and_keep_other_fields() {
    for source in [
        r#"{"schema_version":1,"permission_mode":"ask"}"#,
        r#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"legacy/model"}"#,
        r#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"legacy/model","credential_source":"environment"}"#,
        r#"{"schema_version":4,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"legacy/model","credential_source":"environment","effort":"high","fast_mode":false}"#,
        r#"{"schema_version":5,"permission_mode":"yolo","sandbox_mode":"none","permission_rules":[{"permission":"edit","pattern":"private/*","action":"deny"},{"permission":"edit","pattern":"private/*","action":"allow"}],"provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"legacy/model","credential_source":"environment","effort":"high","fast_mode":false}"#,
        r#"{"schema_version":5,"permission_mode":"ask","sandbox_mode":"os","permission_rules":[],"provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"legacy/model","credential_source":"environment","effort":"high","fast_mode":false}"#,
    ] {
        let fixture = Fixture::new();
        fixture.write(source.as_bytes());
        let store = fixture.store();
        let snapshot = store.load().unwrap();
        assert_eq!(
            fs::read(fixture.root().join("config.json")).unwrap(),
            source.as_bytes()
        );
        let before = snapshot.loaded().config();
        let original: serde_json::Value = serde_json::from_str(source).unwrap();
        let expected_sandbox = original
            .get("sandbox_mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("none");
        assert_eq!(before.sandbox_mode().as_str(), expected_sandbox);
        let saved =
            block_on(store.set_model_preferences(&snapshot, &preferences("next/model"))).unwrap();
        assert_eq!(saved.config().permission_mode(), before.permission_mode());
        assert_eq!(saved.config().sandbox_mode(), before.sandbox_mode());
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.root().join("config.json")).unwrap()).unwrap();
        assert_eq!(written["sandbox_mode"], expected_sandbox);
        assert_eq!(saved.config().permission_rules(), before.permission_rules());
        assert_eq!(saved.config().schema_version(), 7);
        assert_eq!(saved.config().provider(), before.provider());
        assert_eq!(saved.config().transport(), before.transport());
        assert_eq!(
            saved.config().credential_source(),
            before.credential_source()
        );
        assert_eq!(saved.config().effort().label(), "future-tier");
        assert!(saved.config().fast_mode());
    }
}

#[test]
fn whole_config_write_bound_preserves_policy_and_original_bytes() {
    for version in [5, 6, 7] {
        let fixture = Fixture::new();
        let mut value = serde_json::json!({
            "schema_version":5, "permission_mode":"auto", "sandbox_mode":"none",
            "permission_rules":[{"permission":"write","pattern":"","action":"ask"}],
            "provider":"vercel_ai_gateway", "transport":"ai_gateway_http", "model":"x",
            "credential_source":"environment", "effort":"future-tier", "fast_mode":true,
        });
        value["schema_version"] = version.into();
        if version >= 6 {
            value["workspace_permission_rules"] = serde_json::json!([]);
        }
        if version == 7 {
            value["workspace_directories"] = serde_json::json!([]);
        }
        let base = serde_json::to_vec(&value).unwrap().len();
        value["permission_rules"][0]["pattern"] = "x"
            .repeat(machine_god_native::MAX_CONFIG_BYTES - base)
            .into();
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(bytes.len(), machine_god_native::MAX_CONFIG_BYTES);
        fixture.write(&bytes);
        let store = fixture.store();
        let snapshot = store.load().unwrap();
        assert_eq!(
            snapshot.loaded().config().permission_mode(),
            machine_god_native::PermissionMode::Auto
        );
        for oversized in ["xx", "\""] {
            let error = block_on(store.set_model_preferences(&snapshot, &preferences(oversized)))
                .unwrap_err();
            assert!(
                matches!(error, NativeUserConfigError::InvalidConfig(error) if error.kind() == machine_god_native::NativeConfigErrorKind::TooLarge)
            );
            assert_eq!(fs::read(fixture.root().join("config.json")).unwrap(), bytes);
            assert!(!fixture.root().join(".config.tmp").exists());
        }
        if version < 7 {
            // Even an equal-size model cannot hide the required v7 envelope overhead.
            assert!(
                matches!(block_on(store.set_model_preferences(&snapshot, &preferences("y"))),
            Err(NativeUserConfigError::InvalidConfig(error)) if error.kind() == machine_god_native::NativeConfigErrorKind::TooLarge)
            );
            assert_eq!(fs::read(fixture.root().join("config.json")).unwrap(), bytes);
            assert!(!fixture.root().join(".config.lock").exists());
            continue;
        }
        let saved = block_on(store.set_model_preferences(&snapshot, &preferences("y"))).unwrap();
        assert_eq!(
            fs::metadata(fixture.root().join("config.json"))
                .unwrap()
                .len(),
            u64::try_from(machine_god_native::MAX_CONFIG_BYTES).unwrap()
        );
        assert_eq!(
            saved.config().permission_rules(),
            snapshot.loaded().config().permission_rules()
        );
        assert_eq!(
            saved.config().sandbox_mode(),
            machine_god_native::NativeSandboxMode::None
        );
        assert_eq!(store.load().unwrap().loaded(), &saved);
    }
}

#[test]
fn stale_cross_instance_and_foreign_store_snapshots_cannot_clobber() {
    let fixture = Fixture::new();
    fixture.write(br#"{"schema_version":1,"permission_mode":"ask"}"#);
    let one = fixture.store();
    let two = fixture.store();
    let a = one.load().unwrap();
    let b = two.load().unwrap();
    assert_eq!(
        block_on(two.set_model_preferences(&a, &preferences("foreign"))).unwrap_err(),
        NativeUserConfigError::Conflict
    );
    block_on(one.set_model_preferences(&a, &preferences("winner"))).unwrap();
    assert_eq!(
        block_on(two.set_model_preferences(&b, &preferences("stale"))).unwrap_err(),
        NativeUserConfigError::Conflict
    );
    assert_eq!(two.load().unwrap().loaded().config().model(), "winner");
}

#[test]
fn simultaneous_instances_have_exactly_one_winner() {
    let fixture = Fixture::new();
    fixture.write(br#"{"schema_version":1,"permission_mode":"ask"}"#);
    let one = fixture.store();
    let two = fixture.store();
    let a = one.load().unwrap();
    let b = two.load().unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let barrier_two = barrier.clone();
    let first = std::thread::spawn(move || {
        barrier.wait();
        block_on(one.set_model_preferences(&a, &preferences("one")))
    });
    let second = std::thread::spawn(move || {
        barrier_two.wait();
        block_on(two.set_model_preferences(&b, &preferences("two")))
    });
    let results = [first.join().unwrap(), second.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|error| matches!(
                error,
                NativeUserConfigError::Busy | NativeUserConfigError::Conflict
            ))
    );
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(fixture.store().load().unwrap().loaded(), &winner);
}

#[test]
fn invalid_future_oversized_and_duplicate_configs_are_preserved() {
    for source in [
        b"invalid".to_vec(),
        br#"{"schema_version":99}"#.to_vec(),
        br#"{"schema_version":1,"permission_mode":"ask","permission_mode":"ask"}"#.to_vec(),
        vec![b' '; 65_537],
    ] {
        let fixture = Fixture::new();
        fixture.write(&source);
        assert!(matches!(
            fixture.store().load(),
            Err(NativeUserConfigError::InvalidConfig(_))
        ));
        assert_eq!(
            fs::read(fixture.root().join("config.json")).unwrap(),
            source
        );
        assert!(!fixture.root().join(".config.lock").exists());
    }
}

#[test]
fn replacement_root_and_final_symlinks_are_rejected() {
    let fixture = Fixture::new();
    fixture.write(br#"{"schema_version":1,"permission_mode":"ask"}"#);
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    fs::rename(fixture.root(), fixture.0.join("original")).unwrap();
    fixture.write(br#"{"schema_version":1,"permission_mode":"ask"}"#);
    assert_eq!(
        block_on(store.set_model_preferences(&snapshot, &preferences("redirected"))).unwrap_err(),
        NativeUserConfigError::Conflict
    );
    fs::remove_file(fixture.root().join("config.json")).unwrap();
    symlink(
        fixture.0.join("original/config.json"),
        fixture.root().join("config.json"),
    )
    .unwrap();
    assert!(matches!(
        store.load(),
        Err(NativeUserConfigError::UnsafePath)
    ));
}

#[test]
fn preexisting_temp_is_preserved_and_prior_configuration_stays_authoritative() {
    let fixture = Fixture::new();
    fixture.write(br#"{"schema_version":1,"permission_mode":"ask"}"#);
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    fs::write(fixture.root().join(".config.tmp"), b"unowned").unwrap();
    assert_eq!(
        block_on(store.set_model_preferences(&snapshot, &preferences("never"))).unwrap_err(),
        NativeUserConfigError::Persistence
    );
    assert_eq!(store.load().unwrap().loaded(), snapshot.loaded());
    assert_eq!(
        fs::read(fixture.root().join(".config.tmp")).unwrap(),
        b"unowned"
    );
}

#[test]
fn active_lock_is_busy_and_not_waited_on() {
    let fixture = Fixture::new();
    fixture.write(br#"{"schema_version":1,"permission_mode":"ask"}"#);
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    let lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(fixture.root().join(".config.lock"))
        .unwrap();
    lock.set_permissions(fs::Permissions::from_mode(0o600))
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
    assert_eq!(
        block_on(store.set_model_preferences(&snapshot, &preferences("blocked"))).unwrap_err(),
        NativeUserConfigError::Busy
    );
    assert_eq!(store.load().unwrap().loaded(), snapshot.loaded());
}

#[test]
fn strict_current_model_and_effort_bounds_are_shared_with_runtime() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let model = "é".repeat(512);
    let prefs = NativeModelPreferences::new(
        &model,
        NativeReasoningEffort::parse(&"x".repeat(64)).unwrap(),
        true,
    )
    .unwrap();
    block_on(store.set_model_preferences(&store.load().unwrap(), &prefs)).unwrap();
    assert_eq!(
        store.load().unwrap().loaded().config().model_preferences(),
        prefs
    );
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.root().join("config.json")).unwrap()).unwrap();
    for (field, invalid) in [
        ("model", "é".repeat(513)),
        ("effort", "x".repeat(65)),
        ("effort", "非ASCII".to_owned()),
    ] {
        let mut bad = value.clone();
        bad[field] = invalid.into();
        fs::write(
            fixture.root().join("config.json"),
            serde_json::to_vec(&bad).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            store.load(),
            Err(NativeUserConfigError::InvalidConfig(_))
        ));
    }
    value["effort"] = "ADAPTIVE".into();
    fs::write(
        fixture.root().join("config.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().loaded().config().effort().label(),
        "auto"
    );
}

#[test]
fn unsafe_roots_and_lock_symlinks_do_not_grant_authority() {
    assert!(matches!(
        NativeUserConfigStore::new(PathBuf::from("relative")).load(),
        Err(NativeUserConfigError::UnsafePath)
    ));
    let fixture = Fixture::new();
    fixture.write(br#"{"schema_version":1,"permission_mode":"ask"}"#);
    let store = fixture.store();
    let snapshot = store.load().unwrap();
    symlink("config.json", fixture.root().join(".config.lock")).unwrap();
    assert_eq!(
        block_on(store.set_model_preferences(&snapshot, &preferences("never"))).unwrap_err(),
        NativeUserConfigError::UnsafePath
    );
    fs::set_permissions(fixture.root(), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        store.load(),
        Err(NativeUserConfigError::UnsafePath)
    ));
}
