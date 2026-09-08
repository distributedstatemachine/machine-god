#![cfg(any(target_os = "linux", target_os = "macos"))]

use futures_executor::block_on;
use machine_god_native::{
    ConfigOrigin, NativeModelPreferences, NativeReasoningEffort, NativeUserConfigError,
    NativeUserConfigStore,
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
    assert_eq!(saved.config().schema_version(), 4);
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
        let saved =
            block_on(store.set_model_preferences(&snapshot, &preferences("next/model"))).unwrap();
        assert_eq!(saved.config().permission_mode(), before.permission_mode());
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
