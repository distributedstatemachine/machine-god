use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(not(target_family = "wasm"))]
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(target_family = "wasm"))]
use machine_god_native::inspect_process_doctor;
use machine_god_native::{
    MAX_CONFIG_BYTES, NATIVE_DOCTOR_CHECK_COUNT, NativeDoctorCheckStatus,
    NativeDoctorCredentialStatus, NativeDoctorReport, NativeEnvironment, inspect_native_doctor,
};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[cfg(not(target_family = "wasm"))]
const PROCESS_CHILD_MODE_ENV: &str = "MACHINE_GOD_DOCTOR_TEST_CHILD_MODE";
#[cfg(not(target_family = "wasm"))]
const PROCESS_OIDC_TOKEN: &str = "doctor-process-oidc_SECRET_NEVER_REAL";
#[cfg(not(target_family = "wasm"))]
const PROCESS_API_KEY: &str = "doctor-process-api-key_SECRET_NEVER_REAL";
#[cfg(not(target_family = "wasm"))]
const PROCESS_INVALID_TOKEN: &str = "doctor-process-invalid_SECRET NEVER REAL";
#[cfg(not(target_family = "wasm"))]
const PROCESS_PROBE_OK: &str = "machine-god doctor process probe ok";

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

#[cfg(unix)]
#[test]
fn overlong_roots_are_inaccessible_redacted_and_never_created() {
    let temporary = TemporaryDirectory::new("overlong-roots");
    let secret_component = "DOCTOR_OVERLONG_PATH_SECRET_".repeat(32);
    let overlong_root = temporary.path().join(&secret_component);
    let report = inspect_native_doctor(
        &environment(Some(&overlong_root), Some(&overlong_root), None),
        NativeDoctorCredentialStatus::Missing,
    );

    assert_report(
        &report,
        expected_report(
            (
                NativeDoctorCheckStatus::Fail,
                "native configuration file is unreadable",
            ),
            (
                NativeDoctorCheckStatus::Fail,
                "no AI Gateway credential is configured",
            ),
            (
                NativeDoctorCheckStatus::Fail,
                "state directory is inaccessible",
            ),
        ),
    );
    assert!(!format!("{report:?}").contains("DOCTOR_OVERLONG_PATH_SECRET"));
    assert!(!overlong_root.exists());
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[cfg(not(target_family = "wasm"))]
struct ProcessDoctorCase {
    mode: &'static str,
    oidc: Option<OsString>,
    api_key: Option<OsString>,
}

#[cfg(not(target_family = "wasm"))]
fn run_process_doctor_child(case: ProcessDoctorCase, temporary: &TemporaryDirectory) -> Output {
    let config_root = temporary.path().join("PROCESS_CONFIG_PATH_SECRET");
    let state_root = temporary.path().join("PROCESS_STATE_PATH_SECRET");
    let home = temporary.path().join("PROCESS_HOME_PATH_SECRET");
    let mut command = Command::new(std::env::current_exe().expect("doctor test executable"));
    command
        .arg("--exact")
        .arg("process_doctor_environment_probe")
        .arg("--nocapture")
        .env(PROCESS_CHILD_MODE_ENV, case.mode)
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_STATE_HOME", &state_root)
        .env("HOME", &home)
        .env_remove("VERCEL_OIDC_TOKEN")
        .env_remove("AI_GATEWAY_API_KEY");
    if let Some(value) = case.oidc {
        command.env("VERCEL_OIDC_TOKEN", value);
    }
    if let Some(value) = case.api_key {
        command.env("AI_GATEWAY_API_KEY", value);
    }

    let output = command.output().expect("run doctor inspection child");
    assert!(!config_root.exists());
    assert!(!state_root.exists());
    assert!(!home.exists());
    output
}

#[cfg(not(target_family = "wasm"))]
fn assert_process_doctor_child_success(mode: &str, output: &Output) {
    assert!(
        output.status.success(),
        "doctor child failed for mode {mode}; stdout bytes={}, stderr bytes={}",
        output.stdout.len(),
        output.stderr.len()
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(PROCESS_PROBE_OK),
        "doctor child probe did not execute for mode {mode}"
    );
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        for forbidden in [
            PROCESS_OIDC_TOKEN,
            PROCESS_API_KEY,
            PROCESS_INVALID_TOKEN,
            "PROCESS_NON_UNICODE_SECRET",
            "PROCESS_CONFIG_PATH_SECRET",
            "PROCESS_STATE_PATH_SECRET",
            "PROCESS_HOME_PATH_SECRET",
        ] {
            assert!(
                !text.contains(forbidden),
                "doctor child output leaked an injected marker in mode {mode}"
            );
        }
    }
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn process_doctor_environment_probe() {
    let Some(mode) = std::env::var_os(PROCESS_CHILD_MODE_ENV) else {
        return;
    };
    let mode = mode.into_string().expect("doctor child mode is Unicode");
    let credential = if cfg!(any(
        feature = "ai-gateway-http",
        feature = "ai-gateway-model-catalog-http"
    )) {
        match mode.as_str() {
            "missing" => (
                NativeDoctorCheckStatus::Fail,
                "no AI Gateway credential is configured",
            ),
            "api-key" => (
                NativeDoctorCheckStatus::Ok,
                "AI_GATEWAY_API_KEY is configured",
            ),
            "oidc-precedence" => (
                NativeDoctorCheckStatus::Ok,
                "VERCEL_OIDC_TOKEN is configured",
            ),
            "invalid-bearer" => (
                NativeDoctorCheckStatus::Fail,
                "AI Gateway bearer token is invalid",
            ),
            #[cfg(unix)]
            "invalid-environment" => (
                NativeDoctorCheckStatus::Fail,
                "AI Gateway credential environment is invalid",
            ),
            other => panic!("unsupported doctor child mode {other}"),
        }
    } else {
        (
            NativeDoctorCheckStatus::Fail,
            "credential inspection is unavailable on this build",
        )
    };
    let report = inspect_process_doctor();

    assert_report(
        &report,
        expected_report(
            (
                NativeDoctorCheckStatus::Warn,
                "configuration file is missing; using built-in defaults",
            ),
            credential,
            (
                NativeDoctorCheckStatus::Warn,
                "state directory is not initialized",
            ),
        ),
    );
    let diagnostics = format!("{report:?}");
    for forbidden in [
        PROCESS_OIDC_TOKEN,
        PROCESS_API_KEY,
        PROCESS_INVALID_TOKEN,
        "PROCESS_NON_UNICODE_SECRET",
        "PROCESS_CONFIG_PATH_SECRET",
        "PROCESS_STATE_PATH_SECRET",
        "PROCESS_HOME_PATH_SECRET",
    ] {
        assert!(!diagnostics.contains(forbidden));
    }
    println!("{PROCESS_PROBE_OK}");
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn process_doctor_inspection_isolated_by_feature_mode_and_redacted() {
    let cases = vec![
        ProcessDoctorCase {
            mode: "missing",
            oidc: None,
            api_key: None,
        },
        ProcessDoctorCase {
            mode: "api-key",
            oidc: None,
            api_key: Some(OsString::from(PROCESS_API_KEY)),
        },
        ProcessDoctorCase {
            mode: "oidc-precedence",
            oidc: Some(OsString::from(PROCESS_OIDC_TOKEN)),
            api_key: Some(OsString::from(PROCESS_API_KEY)),
        },
        ProcessDoctorCase {
            mode: "invalid-bearer",
            oidc: Some(OsString::from(PROCESS_INVALID_TOKEN)),
            api_key: Some(OsString::from(PROCESS_API_KEY)),
        },
    ];
    #[cfg(unix)]
    let cases = {
        use std::os::unix::ffi::OsStringExt;

        let mut cases = cases;
        cases.push(ProcessDoctorCase {
            mode: "invalid-environment",
            oidc: Some(OsString::from_vec(
                b"PROCESS_NON_UNICODE_SECRET\xff".to_vec(),
            )),
            api_key: Some(OsString::from(PROCESS_API_KEY)),
        });
        cases
    };

    for case in cases {
        let mode = case.mode;
        let temporary = TemporaryDirectory::new(mode);
        let output = run_process_doctor_child(case, &temporary);
        assert_process_doctor_child_success(mode, &output);
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
    }
}
