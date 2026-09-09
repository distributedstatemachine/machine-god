use super::*;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "machine-god-permission-inspection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("machine-god")).unwrap();
        Self(path)
    }
    fn load(&self, value: &Value) -> LoadedNativeConfig {
        fs::write(self.0.join("machine-god/config.json"), value.to_string()).unwrap();
        crate::load_native_config(&crate::NativeEnvironment::new(
            Some(self.0.clone().into()),
            None,
            None,
        ))
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn config() -> Value {
    json!({"schema_version":7,"permission_mode":"auto","sandbox_mode":"none",
        "permission_rules":[
            {"permission":"terminal","pattern":"USER_SECRET","action":"allow"},
            {"permission":"web_fetch","pattern":"INVALID_SECRET","action":"deny"},
            {"permission":"web_fetch","pattern":"domain:example.com","action":"ask"}],
        "workspace_permission_rules":[{"workspace_hex":"2f73656c6563746564","permission_rules":[]}],
        "workspace_directories":[],"provider":"vercel_ai_gateway","transport":"ai_gateway_http",
        "model":"MODEL_SECRET","credential_source":"environment","effort":"high","fast_mode":false})
}

#[test]
fn explicit_sources_preserve_empty_shadow_order_decisions_and_redact_inert_rows() {
    let fixture = Fixture::new();
    let value = config();
    let loaded = fixture.load(&value);
    let report = inspect_native_permissions(&loaded, Path::new("/selected")).unwrap();
    assert_eq!(report.origin(), ConfigOrigin::File);
    assert_eq!(report.permission_mode(), PermissionMode::Auto);
    assert_eq!(
        report.effective_scope(),
        NativeConfiguredPermissionScope::Local
    );
    assert_eq!(report.local_rules(), Some([].as_slice()));
    assert_eq!(report.user_rules().len(), 3);
    assert_eq!(report.user_rules()[0].pattern(), Some("USER_SECRET"));
    assert_eq!(report.user_rules()[1].pattern(), None);
    assert!(report.user_rules()[1].inert());
    assert_eq!(
        report.user_rules()[1].decision(),
        NativeConfiguredPermissionDecision::Deny
    );
    assert!(!report.user_rules()[2].inert());
    assert_eq!(
        report.user_rules()[2].decision(),
        NativeConfiguredPermissionDecision::Ask
    );
    for secret in ["USER_SECRET", "INVALID_SECRET", "MODEL_SECRET"] {
        assert!(!format!("{report:?}").contains(secret));
    }
    assert_eq!(
        fs::read(fixture.0.join("machine-god/config.json")).unwrap(),
        value.to_string().as_bytes()
    );
    let other = inspect_native_permissions(&loaded, Path::new("/other")).unwrap();
    assert_eq!(
        other.effective_scope(),
        NativeConfiguredPermissionScope::User
    );
    assert!(other.local_rules().is_none());
}

#[test]
fn process_selection_observes_workspace_once_only_when_local_sources_exist() {
    let defaults = LoadedNativeConfig::built_in_defaults();
    let report =
        inspect_process_loaded(&defaults, || panic!("unnecessary CWD observation")).unwrap();
    assert_eq!(report.origin(), ConfigOrigin::BuiltInDefaults);
    let fixture = Fixture::new();
    let loaded = fixture.load(&config());
    let calls = std::cell::Cell::new(0);
    let report = inspect_process_loaded(&loaded, || {
        calls.set(calls.get() + 1);
        Ok("/selected".into())
    })
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(
        report.effective_scope(),
        NativeConfiguredPermissionScope::Local
    );
    assert_eq!(
        inspect_process_loaded(&loaded, || Err(std::io::Error::other("CWD_SECRET"))),
        Err(NativePermissionInspectionError::Workspace)
    );
}

#[test]
fn invalid_workspace_labels_never_fall_back_to_user_rules() {
    let fixture = Fixture::new();
    let loaded = fixture.load(&config());
    for path in [
        "relative",
        "/selected/../other",
        "/selected/",
        "/selected//child",
    ] {
        assert_eq!(
            inspect_native_permissions(&loaded, Path::new(path)),
            Err(NativePermissionInspectionError::Workspace)
        );
    }
    let oversized = format!("/{}", "a".repeat(4096));
    assert_eq!(
        inspect_native_permissions(&loaded, Path::new(&oversized)),
        Err(NativePermissionInspectionError::Workspace)
    );
}

#[cfg(unix)]
#[test]
fn non_unicode_workspace_bytes_select_exact_local_source() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let mut value = config();
    value["workspace_permission_rules"][0]["workspace_hex"] = "2fff".into();
    let loaded = fixture.load(&value);
    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', 255]));
    let report = inspect_native_permissions(&loaded, &path).unwrap();
    assert_eq!(
        report.effective_scope(),
        NativeConfiguredPermissionScope::Local
    );
    assert_eq!(
        inspect_process_loaded(&loaded, || Ok(path)).unwrap(),
        report
    );
}
