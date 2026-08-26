use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use machine_god_native::{
    MAX_CONFIG_BYTES, NATIVE_DOCTOR_CHECK_COUNT, NativeDoctorCheckStatus,
    NativeDoctorCredentialStatus, NativeDoctorReport, NativeEnvironment, inspect_native_doctor,
};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(test_name: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "machine-god-doctor-{}-{test_name}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create temporary directory: {error}"),
            }
        }
        panic!("failed to allocate a unique temporary directory");
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0)
            && error.kind() != io::ErrorKind::NotFound
            && !std::thread::panicking()
        {
            panic!("failed to remove temporary directory: {error}");
        }
    }
}

fn environment(
    config_root: Option<&Path>,
    state_root: Option<&Path>,
    home: Option<&Path>,
) -> NativeEnvironment {
    NativeEnvironment::new(
        config_root.map(Path::as_os_str).map(OsString::from),
        state_root.map(Path::as_os_str).map(OsString::from),
        home.map(Path::as_os_str).map(OsString::from),
    )
}

fn config_path(config_root: &Path) -> PathBuf {
    config_root.join("machine-god/config.json")
}

fn state_path(state_root: &Path) -> PathBuf {
    state_root.join("machine-god")
}

fn write_config(config_root: &Path, contents: &[u8]) -> PathBuf {
    let path = config_path(config_root);
    fs::create_dir_all(path.parent().expect("config has a parent")).unwrap();
    fs::write(&path, contents).unwrap();
    path
}

fn platform_expectation() -> (NativeDoctorCheckStatus, &'static str) {
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        (
            NativeDoctorCheckStatus::Ok,
            "native host platform is supported",
        )
    } else {
        (
            NativeDoctorCheckStatus::Fail,
            "native host platform is unsupported",
        )
    }
}

fn assert_report(
    report: &NativeDoctorReport,
    expected: [(NativeDoctorCheckStatus, &'static str, &'static str); 4],
) {
    assert_eq!(report.checked_count(), NATIVE_DOCTOR_CHECK_COUNT);
    assert_eq!(report.checks().len(), NATIVE_DOCTOR_CHECK_COUNT);

    let mut counts = [0_usize; 3];
    for (check, (status, name, detail)) in report.checks().iter().zip(expected) {
        assert_eq!(check.status(), status);
        assert_eq!(check.name(), name);
        assert_eq!(check.detail(), detail);
        match status {
            NativeDoctorCheckStatus::Ok => counts[0] += 1,
            NativeDoctorCheckStatus::Warn => counts[1] += 1,
            NativeDoctorCheckStatus::Fail => counts[2] += 1,
        }
    }

    assert_eq!(report.ok_count(), counts[0]);
    assert_eq!(report.warn_count(), counts[1]);
    assert_eq!(report.fail_count(), counts[2]);
    assert_eq!(counts.into_iter().sum::<usize>(), NATIVE_DOCTOR_CHECK_COUNT);
}

fn expected_report(
    config: (NativeDoctorCheckStatus, &'static str),
    credential: (NativeDoctorCheckStatus, &'static str),
    state: (NativeDoctorCheckStatus, &'static str),
) -> [(NativeDoctorCheckStatus, &'static str, &'static str); 4] {
    let platform = platform_expectation();
    [
        (config.0, "config", config.1),
        (credential.0, "credential", credential.1),
        (state.0, "state", state.1),
        (platform.0, "platform", platform.1),
    ]
}

#[test]
fn missing_roots_report_all_four_checks_without_creating_anything() {
    let temporary = TemporaryDirectory::new("missing");
    let config_root = temporary.path().join("absent-config");
    let state_root = temporary.path().join("absent-state");

    let report = inspect_native_doctor(
        &environment(Some(&config_root), Some(&state_root), None),
        NativeDoctorCredentialStatus::Missing,
    );

    assert_report(
        &report,
        expected_report(
            (
                NativeDoctorCheckStatus::Warn,
                "configuration file is missing; using built-in defaults",
            ),
            (
                NativeDoctorCheckStatus::Fail,
                "no AI Gateway credential is configured",
            ),
            (
                NativeDoctorCheckStatus::Warn,
                "state directory is not initialized",
            ),
        ),
    );
    assert!(!config_root.exists());
    assert!(!state_root.exists());
}

#[test]
fn strict_config_versions_and_ready_state_are_read_without_rewrite() {
    let schemas: [(&str, &[u8]); 3] = [
        ("v1", br#"{"schema_version":1,"permission_mode":"ask"}"#),
        (
            "v2",
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"doctor-v2"}"#,
        ),
        (
            "v3",
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"doctor-v3","credential_source":"environment"}"#,
        ),
    ];

    for (schema, contents) in schemas {
        let temporary = TemporaryDirectory::new(schema);
        let config_root = temporary.path().join("config");
        let state_root = temporary.path().join("state");
        let path = write_config(&config_root, contents);
        fs::create_dir_all(state_path(&state_root)).unwrap();

        let report = inspect_native_doctor(
            &environment(Some(&config_root), Some(&state_root), None),
            NativeDoctorCredentialStatus::OidcAvailable,
        );

        assert_report(
            &report,
            expected_report(
                (NativeDoctorCheckStatus::Ok, "configuration file is valid"),
                (
                    NativeDoctorCheckStatus::Ok,
                    "VERCEL_OIDC_TOKEN is configured",
                ),
                (NativeDoctorCheckStatus::Ok, "state directory is ready"),
            ),
        );
        assert_eq!(fs::read(path).unwrap(), contents);
    }
}

#[test]
fn credential_statuses_have_closed_redacted_classifications() {
    let temporary = TemporaryDirectory::new("credentials");
    let config_root = temporary.path().join("missing-config");
    let state_root = temporary.path().join("missing-state");
    let cases = [
        (
            NativeDoctorCredentialStatus::OidcAvailable,
            NativeDoctorCheckStatus::Ok,
            "VERCEL_OIDC_TOKEN is configured",
        ),
        (
            NativeDoctorCredentialStatus::ApiKeyAvailable,
            NativeDoctorCheckStatus::Ok,
            "AI_GATEWAY_API_KEY is configured",
        ),
        (
            NativeDoctorCredentialStatus::Missing,
            NativeDoctorCheckStatus::Fail,
            "no AI Gateway credential is configured",
        ),
        (
            NativeDoctorCredentialStatus::InvalidEnvironment,
            NativeDoctorCheckStatus::Fail,
            "AI Gateway credential environment is invalid",
        ),
        (
            NativeDoctorCredentialStatus::InvalidBearerToken,
            NativeDoctorCheckStatus::Fail,
            "AI Gateway bearer token is invalid",
        ),
        (
            NativeDoctorCredentialStatus::Unavailable,
            NativeDoctorCheckStatus::Fail,
            "credential inspection is unavailable on this build",
        ),
    ];

    for (credential, status, detail) in cases {
        let report = inspect_native_doctor(
            &environment(Some(&config_root), Some(&state_root), None),
            credential,
        );
        assert_report(
            &report,
            expected_report(
                (
                    NativeDoctorCheckStatus::Warn,
                    "configuration file is missing; using built-in defaults",
                ),
                (status, detail),
                (
                    NativeDoctorCheckStatus::Warn,
                    "state directory is not initialized",
                ),
            ),
        );
    }

    assert!(!config_root.exists());
    assert!(!state_root.exists());
}

#[test]
fn invalid_config_kinds_are_fixed_and_do_not_disclose_inputs() {
    let mut oversized = vec![b'x'; MAX_CONFIG_BYTES + 1];
    oversized[..22].copy_from_slice(b"DOCTOR_OVERSIZE_SECRET");
    let cases = [
        (
            "malformed",
            b"DOCTOR_MALFORMED_SECRET:not-json".to_vec(),
            "native configuration format is invalid",
        ),
        (
            "unsupported",
            br#"{"schema_version":999,"future":"DOCTOR_VERSION_SECRET"}"#.to_vec(),
            "native configuration schema version is unsupported",
        ),
        (
            "oversized",
            oversized,
            "native configuration file is too large",
        ),
    ];

    for (case, contents, expected_detail) in cases {
        let temporary = TemporaryDirectory::new(case);
        let config_root = temporary.path().join(format!("config-{case}-PATH_SECRET"));
        let state_root = temporary.path().join("missing-state");
        let path = write_config(&config_root, &contents);
        let report = inspect_native_doctor(
            &environment(Some(&config_root), Some(&state_root), None),
            NativeDoctorCredentialStatus::Unavailable,
        );

        assert_report(
            &report,
            expected_report(
                (NativeDoctorCheckStatus::Fail, expected_detail),
                (
                    NativeDoctorCheckStatus::Fail,
                    "credential inspection is unavailable on this build",
                ),
                (
                    NativeDoctorCheckStatus::Warn,
                    "state directory is not initialized",
                ),
            ),
        );
        let diagnostics = format!("{report:?}");
        for forbidden in [
            "PATH_SECRET",
            "DOCTOR_MALFORMED_SECRET",
            "DOCTOR_VERSION_SECRET",
            "DOCTOR_OVERSIZE_SECRET",
        ] {
            assert!(!diagnostics.contains(forbidden));
        }
        assert_eq!(fs::read(path).unwrap(), contents);
        assert!(!state_root.exists());
    }
}

#[test]
fn wrong_config_and_state_file_types_are_classified_without_mutation() {
    let temporary = TemporaryDirectory::new("wrong-types");
    let config_root = temporary.path().join("config");
    let state_root = temporary.path().join("state");
    fs::create_dir_all(config_path(&config_root)).unwrap();
    fs::create_dir_all(&state_root).unwrap();
    fs::write(state_path(&state_root), b"state-file-secret").unwrap();

    let report = inspect_native_doctor(
        &environment(Some(&config_root), Some(&state_root), None),
        NativeDoctorCredentialStatus::ApiKeyAvailable,
    );

    assert_report(
        &report,
        expected_report(
            (
                NativeDoctorCheckStatus::Fail,
                "native configuration path is not a regular file",
            ),
            (
                NativeDoctorCheckStatus::Ok,
                "AI_GATEWAY_API_KEY is configured",
            ),
            (
                NativeDoctorCheckStatus::Fail,
                "state path is not a directory",
            ),
        ),
    );
    assert!(config_path(&config_root).is_dir());
    assert_eq!(
        fs::read(state_path(&state_root)).unwrap(),
        b"state-file-secret"
    );
}

#[test]
fn unavailable_and_invalid_locations_fail_closed_without_home_fallback() {
    let unavailable = inspect_native_doctor(
        &NativeEnvironment::new(None, None, None),
        NativeDoctorCredentialStatus::Missing,
    );
    assert_report(
        &unavailable,
        expected_report(
            (
                NativeDoctorCheckStatus::Warn,
                "configuration file is missing; using built-in defaults",
            ),
            (
                NativeDoctorCheckStatus::Fail,
                "no AI Gateway credential is configured",
            ),
            (
                NativeDoctorCheckStatus::Fail,
                "state directory location is unavailable",
            ),
        ),
    );

    let temporary = TemporaryDirectory::new("invalid-locations");
    let fallback_home = temporary.path().join("fallback-home");
    let invalid = NativeEnvironment::new(
        Some(OsString::from("relative-config-secret")),
        Some(OsString::from("relative-state-secret")),
        Some(fallback_home.as_os_str().to_owned()),
    );
    let report = inspect_native_doctor(&invalid, NativeDoctorCredentialStatus::Missing);
    assert_report(
        &report,
        expected_report(
            (
                NativeDoctorCheckStatus::Fail,
                "native configuration environment is invalid",
            ),
            (
                NativeDoctorCheckStatus::Fail,
                "no AI Gateway credential is configured",
            ),
            (
                NativeDoctorCheckStatus::Fail,
                "state directory environment is invalid",
            ),
        ),
    );
    let diagnostics = format!("{report:?}");
    assert!(!diagnostics.contains("relative-config-secret"));
    assert!(!diagnostics.contains("relative-state-secret"));
    assert!(!fallback_home.exists());
}

#[cfg(unix)]
#[test]
fn non_unicode_selected_locations_are_redacted_and_invalid() {
    use std::os::unix::ffi::OsStringExt;

    let temporary = TemporaryDirectory::new("non-unicode");
    let mut bytes = temporary.path().as_os_str().as_encoded_bytes().to_vec();
    bytes.extend_from_slice(b"/doctor-secret-");
    bytes.push(0xff);
    let invalid = OsString::from_vec(bytes);
    let report = inspect_native_doctor(
        &NativeEnvironment::new(Some(invalid.clone()), Some(invalid), None),
        NativeDoctorCredentialStatus::InvalidEnvironment,
    );

    assert_report(
        &report,
        expected_report(
            (
                NativeDoctorCheckStatus::Fail,
                "native configuration environment is invalid",
            ),
            (
                NativeDoctorCheckStatus::Fail,
                "AI Gateway credential environment is invalid",
            ),
            (
                NativeDoctorCheckStatus::Fail,
                "state directory environment is invalid",
            ),
        ),
    );
    assert!(!format!("{report:?}").contains("doctor-secret"));
}
