//! Bounded, read-only diagnostics for native hosts.

use std::fmt;

use super::config::{ConfigOrigin, NativeConfigErrorKind, load_native_config};
use super::{
    NativeEnvironment, ResolvedPath, StateDirectoryState, inspect_state_directory,
    resolve_state_directory,
};

/// Number of checks in every native doctor report.
pub const NATIVE_DOCTOR_CHECK_COUNT: usize = 4;

/// Stable outcome of one native doctor check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeDoctorCheckStatus {
    /// The inspected capability is ready.
    Ok,
    /// The inspected capability can continue with a safe fallback.
    Warn,
    /// The inspected capability is unavailable or invalid.
    Fail,
}

impl NativeDoctorCheckStatus {
    /// Returns the stable, machine-readable status name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

/// Closed, non-secret credential status supplied to native doctor inspection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NativeDoctorCredentialStatus {
    /// A validated `VERCEL_OIDC_TOKEN` is available.
    OidcAvailable,
    /// A validated `AI_GATEWAY_API_KEY` is available.
    ApiKeyAvailable,
    /// Neither supported credential is present.
    Missing,
    /// The selected credential environment value is invalid.
    InvalidEnvironment,
    /// The selected credential is not a valid bearer token.
    InvalidBearerToken,
    /// This build cannot inspect credentials.
    #[default]
    Unavailable,
}

/// One fixed, redacted native doctor check result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeDoctorCheck {
    status: NativeDoctorCheckStatus,
    name: &'static str,
    detail: &'static str,
}

impl NativeDoctorCheck {
    const fn new(
        status: NativeDoctorCheckStatus,
        name: &'static str,
        detail: &'static str,
    ) -> Self {
        Self {
            status,
            name,
            detail,
        }
    }

    /// Returns the check outcome.
    #[must_use]
    pub const fn status(&self) -> NativeDoctorCheckStatus {
        self.status
    }

    /// Returns the fixed check name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the fixed, redacted check detail.
    #[must_use]
    pub const fn detail(&self) -> &'static str {
        self.detail
    }
}

/// Fixed-size result of read-only native doctor inspection.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeDoctorReport {
    checks: [NativeDoctorCheck; NATIVE_DOCTOR_CHECK_COUNT],
    ok_count: usize,
    warn_count: usize,
    fail_count: usize,
}

impl NativeDoctorReport {
    fn new(checks: [NativeDoctorCheck; NATIVE_DOCTOR_CHECK_COUNT]) -> Self {
        let mut ok_count = 0;
        let mut warn_count = 0;
        let mut fail_count = 0;
        for check in checks {
            match check.status {
                NativeDoctorCheckStatus::Ok => ok_count += 1,
                NativeDoctorCheckStatus::Warn => warn_count += 1,
                NativeDoctorCheckStatus::Fail => fail_count += 1,
            }
        }
        Self {
            checks,
            ok_count,
            warn_count,
            fail_count,
        }
    }

    /// Returns the four checks in stable `config`, `credential`, `state`,
    /// `platform` order.
    #[must_use]
    pub const fn checks(&self) -> &[NativeDoctorCheck; NATIVE_DOCTOR_CHECK_COUNT] {
        &self.checks
    }

    /// Returns the total number of checks performed.
    #[must_use]
    pub const fn checked_count(&self) -> usize {
        NATIVE_DOCTOR_CHECK_COUNT
    }

    /// Returns the number of successful checks.
    #[must_use]
    pub const fn ok_count(&self) -> usize {
        self.ok_count
    }

    /// Returns the number of warning checks.
    #[must_use]
    pub const fn warn_count(&self) -> usize {
        self.warn_count
    }

    /// Returns the number of failed checks.
    #[must_use]
    pub const fn fail_count(&self) -> usize {
        self.fail_count
    }
}

impl fmt::Debug for NativeDoctorReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeDoctorReport")
            .field("checks", &self.checks)
            .field("ok_count", &self.ok_count)
            .field("warn_count", &self.warn_count)
            .field("fail_count", &self.fail_count)
            .finish()
    }
}

/// Loads configuration and inspects native metadata using injected snapshots.
///
/// Inspection performs no network access and does not create state, start a
/// runtime or process, or open a session. Credential values are never accepted
/// by this API; callers inject only their already-classified non-secret status.
#[must_use]
pub fn inspect_native_doctor(
    environment: &NativeEnvironment,
    credential_status: NativeDoctorCredentialStatus,
) -> NativeDoctorReport {
    inspect_native_doctor_with_credential(environment, || credential_status)
}

fn inspect_native_doctor_with_credential(
    environment: &NativeEnvironment,
    inspect_credential_status: impl FnOnce() -> NativeDoctorCredentialStatus,
) -> NativeDoctorReport {
    let config = inspect_config(environment);
    let credential = inspect_credential(inspect_credential_status());
    let state = inspect_state(environment);
    let platform = inspect_platform();

    NativeDoctorReport::new([config, credential, state, platform])
}

/// Captures process environment inputs and returns a bounded native doctor report.
///
/// Builds with native AI Gateway credential support consume the existing
/// bounded discovery result and retain only its non-secret source or error
/// category. Other builds fail the credential check closed as unavailable.
#[must_use]
pub fn inspect_process_doctor() -> NativeDoctorReport {
    let environment = NativeEnvironment::from_process();
    inspect_native_doctor_with_credential(&environment, inspect_process_credential)
}

fn inspect_config(environment: &NativeEnvironment) -> NativeDoctorCheck {
    match load_native_config(environment) {
        Ok(config) => match config.origin() {
            ConfigOrigin::File => NativeDoctorCheck::new(
                NativeDoctorCheckStatus::Ok,
                "config",
                "configuration file is valid",
            ),
            ConfigOrigin::BuiltInDefaults => NativeDoctorCheck::new(
                NativeDoctorCheckStatus::Warn,
                "config",
                "configuration file is missing; using built-in defaults",
            ),
        },
        Err(error) => NativeDoctorCheck::new(
            NativeDoctorCheckStatus::Fail,
            "config",
            config_error_detail(error.kind()),
        ),
    }
}

const fn config_error_detail(kind: NativeConfigErrorKind) -> &'static str {
    match kind {
        NativeConfigErrorKind::InvalidEnvironment => "native configuration environment is invalid",
        NativeConfigErrorKind::InvalidFileType => "native configuration path is not a regular file",
        NativeConfigErrorKind::Unreadable => "native configuration file is unreadable",
        NativeConfigErrorKind::TooLarge => "native configuration file is too large",
        NativeConfigErrorKind::InvalidFormat => "native configuration format is invalid",
        NativeConfigErrorKind::UnsupportedSchemaVersion => {
            "native configuration schema version is unsupported"
        }
    }
}

const fn inspect_credential(status: NativeDoctorCredentialStatus) -> NativeDoctorCheck {
    let (check_status, detail) = match status {
        NativeDoctorCredentialStatus::OidcAvailable => (
            NativeDoctorCheckStatus::Ok,
            "VERCEL_OIDC_TOKEN is configured",
        ),
        NativeDoctorCredentialStatus::ApiKeyAvailable => (
            NativeDoctorCheckStatus::Ok,
            "AI_GATEWAY_API_KEY is configured",
        ),
        NativeDoctorCredentialStatus::Missing => (
            NativeDoctorCheckStatus::Fail,
            "no AI Gateway credential is configured",
        ),
        NativeDoctorCredentialStatus::InvalidEnvironment => (
            NativeDoctorCheckStatus::Fail,
            "AI Gateway credential environment is invalid",
        ),
        NativeDoctorCredentialStatus::InvalidBearerToken => (
            NativeDoctorCheckStatus::Fail,
            "AI Gateway bearer token is invalid",
        ),
        NativeDoctorCredentialStatus::Unavailable => (
            NativeDoctorCheckStatus::Fail,
            "credential inspection is unavailable on this build",
        ),
    };
    NativeDoctorCheck::new(check_status, "credential", detail)
}

fn inspect_state(environment: &NativeEnvironment) -> NativeDoctorCheck {
    let state = match resolve_state_directory(environment) {
        ResolvedPath::Path(path) => inspect_state_directory(&path),
        ResolvedPath::Unavailable => StateDirectoryState::Unavailable,
        ResolvedPath::InvalidEnvironment => StateDirectoryState::InvalidEnvironment,
    };
    let (status, detail) = match state {
        StateDirectoryState::Directory => (NativeDoctorCheckStatus::Ok, "state directory is ready"),
        StateDirectoryState::Missing => (
            NativeDoctorCheckStatus::Warn,
            "state directory is not initialized",
        ),
        StateDirectoryState::NotDirectory => (
            NativeDoctorCheckStatus::Fail,
            "state path is not a directory",
        ),
        StateDirectoryState::Inaccessible => (
            NativeDoctorCheckStatus::Fail,
            "state directory is inaccessible",
        ),
        StateDirectoryState::Unavailable => (
            NativeDoctorCheckStatus::Fail,
            "state directory location is unavailable",
        ),
        StateDirectoryState::InvalidEnvironment => (
            NativeDoctorCheckStatus::Fail,
            "state directory environment is invalid",
        ),
    };
    NativeDoctorCheck::new(status, "state", detail)
}

const fn inspect_platform() -> NativeDoctorCheck {
    classify_platform(cfg!(any(target_os = "linux", target_os = "macos")))
}

const fn classify_platform(supported: bool) -> NativeDoctorCheck {
    if supported {
        NativeDoctorCheck::new(
            NativeDoctorCheckStatus::Ok,
            "platform",
            "native host platform is supported",
        )
    } else {
        NativeDoctorCheck::new(
            NativeDoctorCheckStatus::Fail,
            "platform",
            "native host platform is unsupported",
        )
    }
}

#[cfg(all(
    any(feature = "ai-gateway-http", feature = "ai-gateway-model-catalog-http"),
    not(target_family = "wasm")
))]
fn inspect_process_credential() -> NativeDoctorCredentialStatus {
    use super::ai_gateway_credential::{
        AiGatewayCredentialErrorKind, AiGatewayCredentialSource,
        discover_process_ai_gateway_credential,
    };

    match discover_process_ai_gateway_credential() {
        Ok(credential) => match credential.source() {
            AiGatewayCredentialSource::VercelOidcToken => {
                NativeDoctorCredentialStatus::OidcAvailable
            }
            AiGatewayCredentialSource::AiGatewayApiKey => {
                NativeDoctorCredentialStatus::ApiKeyAvailable
            }
        },
        Err(error) => match error.kind() {
            AiGatewayCredentialErrorKind::Missing => NativeDoctorCredentialStatus::Missing,
            AiGatewayCredentialErrorKind::InvalidEnvironment => {
                NativeDoctorCredentialStatus::InvalidEnvironment
            }
            AiGatewayCredentialErrorKind::InvalidBearerToken => {
                NativeDoctorCredentialStatus::InvalidBearerToken
            }
        },
    }
}

#[cfg(not(all(
    any(feature = "ai-gateway-http", feature = "ai-gateway-model-catalog-http"),
    not(target_family = "wasm")
)))]
const fn inspect_process_credential() -> NativeDoctorCredentialStatus {
    NativeDoctorCredentialStatus::Unavailable
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        NATIVE_DOCTOR_CHECK_COUNT, NativeDoctorCheckStatus, NativeDoctorCredentialStatus,
        classify_platform, config_error_detail, inspect_credential, inspect_native_doctor,
        inspect_native_doctor_with_credential,
    };
    use crate::{NativeConfigErrorKind, NativeEnvironment};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(test_name: &str) -> Self {
            let base = std::env::temp_dir().join("machine-god-native-doctor-tests");
            fs::create_dir_all(&base).expect("failed to create doctor test base directory");
            loop {
                let id = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = base.join(format!("{}-{test_name}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("failed to create test directory {path:?}: {error}"),
                }
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!(
                    "failed to remove doctor test directory {:?}: {error}",
                    self.path
                );
            }
        }
    }

    fn environment(config_root: &Path, state_root: &Path) -> NativeEnvironment {
        NativeEnvironment::new(
            Some(OsString::from(config_root)),
            Some(OsString::from(state_root)),
            None,
        )
    }

    #[test]
    fn status_names_are_stable() {
        assert_eq!(NativeDoctorCheckStatus::Ok.as_str(), "ok");
        assert_eq!(NativeDoctorCheckStatus::Warn.as_str(), "warn");
        assert_eq!(NativeDoctorCheckStatus::Fail.as_str(), "fail");
        assert_eq!(
            NativeDoctorCredentialStatus::default(),
            NativeDoctorCredentialStatus::Unavailable
        );
    }

    #[test]
    fn report_has_exact_order_details_and_counts() {
        let temporary = TestDirectory::new("order-counts");
        let config_root = temporary.path().join("config");
        let state_root = temporary.path().join("state");
        let report = inspect_native_doctor(
            &environment(&config_root, &state_root),
            NativeDoctorCredentialStatus::OidcAvailable,
        );

        assert_eq!(report.checked_count(), NATIVE_DOCTOR_CHECK_COUNT);
        assert_eq!(
            report.checks().map(|check| check.name()),
            ["config", "credential", "state", "platform"]
        );
        assert_eq!(report.checks()[0].status(), NativeDoctorCheckStatus::Warn);
        assert_eq!(
            report.checks()[0].detail(),
            "configuration file is missing; using built-in defaults"
        );
        assert_eq!(report.checks()[1].status(), NativeDoctorCheckStatus::Ok);
        assert_eq!(
            report.checks()[1].detail(),
            "VERCEL_OIDC_TOKEN is configured"
        );
        assert_eq!(report.checks()[2].status(), NativeDoctorCheckStatus::Warn);
        assert_eq!(
            report.checks()[2].detail(),
            "state directory is not initialized"
        );
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            assert_eq!(report.checks()[3].status(), NativeDoctorCheckStatus::Ok);
            assert_eq!(report.ok_count(), 2);
            assert_eq!(report.warn_count(), 2);
            assert_eq!(report.fail_count(), 0);
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            assert_eq!(report.checks()[3].status(), NativeDoctorCheckStatus::Fail);
            assert_eq!(report.ok_count(), 1);
            assert_eq!(report.warn_count(), 2);
            assert_eq!(report.fail_count(), 1);
        }
        assert_eq!(
            report.ok_count() + report.warn_count() + report.fail_count(),
            report.checked_count()
        );
    }

    #[test]
    fn configuration_is_inspected_before_credential_status() {
        let temporary = TestDirectory::new("inspection-order");
        let config_root = temporary.path().join("config");
        let state_root = temporary.path().join("state");
        let config_path = config_root.join("machine-god/config.json");
        let report = inspect_native_doctor_with_credential(
            &environment(&config_root, &state_root),
            || {
                fs::create_dir_all(config_path.parent().unwrap()).unwrap();
                fs::write(
                    &config_path,
                    br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"openai/gpt-5.2-codex","credential_source":"environment"}"#,
                )
                .unwrap();
                NativeDoctorCredentialStatus::ApiKeyAvailable
            },
        );

        assert!(config_path.is_file());
        assert_eq!(report.checks()[0].status(), NativeDoctorCheckStatus::Warn);
        assert_eq!(
            report.checks()[0].detail(),
            "configuration file is missing; using built-in defaults"
        );
        assert_eq!(report.checks()[1].status(), NativeDoctorCheckStatus::Ok);
    }

    #[test]
    fn platform_classifier_has_fixed_supported_and_unsupported_results() {
        let supported = classify_platform(true);
        assert_eq!(supported.name(), "platform");
        assert_eq!(supported.status(), NativeDoctorCheckStatus::Ok);
        assert_eq!(supported.detail(), "native host platform is supported");

        let unsupported = classify_platform(false);
        assert_eq!(unsupported.name(), "platform");
        assert_eq!(unsupported.status(), NativeDoctorCheckStatus::Fail);
        assert_eq!(unsupported.detail(), "native host platform is unsupported");
    }

    #[test]
    fn valid_config_and_existing_state_are_ok() {
        let temporary = TestDirectory::new("ready");
        let config_root = temporary.path().join("config");
        let state_root = temporary.path().join("state");
        let config_path = config_root.join("machine-god/config.json");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::create_dir_all(state_root.join("machine-god")).unwrap();
        fs::write(
            config_path,
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"openai/gpt-5.2-codex","credential_source":"environment"}"#,
        )
        .unwrap();

        let report = inspect_native_doctor(
            &environment(&config_root, &state_root),
            NativeDoctorCredentialStatus::ApiKeyAvailable,
        );

        assert_eq!(report.checks()[0].status(), NativeDoctorCheckStatus::Ok);
        assert_eq!(report.checks()[0].detail(), "configuration file is valid");
        assert_eq!(
            report.checks()[1].detail(),
            "AI_GATEWAY_API_KEY is configured"
        );
        assert_eq!(report.checks()[2].status(), NativeDoctorCheckStatus::Ok);
        assert_eq!(report.checks()[2].detail(), "state directory is ready");
    }

    #[test]
    fn invalid_config_and_state_fail_without_exposing_paths() {
        let temporary = TestDirectory::new("redacted-failures");
        let secret = "secret-path-component";
        let config_root = temporary.path().join(secret).join("config");
        let state_root = temporary.path().join(secret).join("state");
        let config_path = config_root.join("machine-god/config.json");
        let state_path = state_root.join("machine-god");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        fs::write(config_path, b"not json").unwrap();
        fs::write(state_path, b"not a directory").unwrap();

        let report = inspect_native_doctor(
            &environment(&config_root, &state_root),
            NativeDoctorCredentialStatus::Missing,
        );

        assert_eq!(report.checks()[0].status(), NativeDoctorCheckStatus::Fail);
        assert_eq!(
            report.checks()[0].detail(),
            "native configuration format is invalid"
        );
        assert_eq!(report.checks()[1].status(), NativeDoctorCheckStatus::Fail);
        assert_eq!(
            report.checks()[1].detail(),
            "no AI Gateway credential is configured"
        );
        assert_eq!(report.checks()[2].status(), NativeDoctorCheckStatus::Fail);
        assert_eq!(report.checks()[2].detail(), "state path is not a directory");
        assert!(!format!("{report:?}").contains(secret));
    }

    #[test]
    fn credential_mapping_is_closed_and_secret_free() {
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

        for (input, status, detail) in cases {
            let check = inspect_credential(input);
            assert_eq!(check.name(), "credential");
            assert_eq!(check.status(), status);
            assert_eq!(check.detail(), detail);
        }
    }

    #[test]
    fn config_failure_details_match_redacted_loader_messages() {
        let cases = [
            (
                NativeConfigErrorKind::InvalidEnvironment,
                "native configuration environment is invalid",
            ),
            (
                NativeConfigErrorKind::InvalidFileType,
                "native configuration path is not a regular file",
            ),
            (
                NativeConfigErrorKind::Unreadable,
                "native configuration file is unreadable",
            ),
            (
                NativeConfigErrorKind::TooLarge,
                "native configuration file is too large",
            ),
            (
                NativeConfigErrorKind::InvalidFormat,
                "native configuration format is invalid",
            ),
            (
                NativeConfigErrorKind::UnsupportedSchemaVersion,
                "native configuration schema version is unsupported",
            ),
        ];
        for (kind, expected) in cases {
            assert_eq!(config_error_detail(kind), expected);
        }
    }

    #[test]
    fn unavailable_and_invalid_state_locations_fail_closed() {
        let unavailable = inspect_native_doctor(
            &NativeEnvironment::new(None, None, None),
            NativeDoctorCredentialStatus::Unavailable,
        );
        assert_eq!(
            unavailable.checks()[2].status(),
            NativeDoctorCheckStatus::Fail
        );
        assert_eq!(
            unavailable.checks()[2].detail(),
            "state directory location is unavailable"
        );

        let invalid = inspect_native_doctor(
            &NativeEnvironment::new(None, Some(OsString::from("relative-state-root")), None),
            NativeDoctorCredentialStatus::Unavailable,
        );
        assert_eq!(invalid.checks()[2].status(), NativeDoctorCheckStatus::Fail);
        assert_eq!(
            invalid.checks()[2].detail(),
            "state directory environment is invalid"
        );
    }

    #[test]
    fn inspection_does_not_initialize_state_or_config() {
        let temporary = TestDirectory::new("read-only");
        let config_root = temporary.path().join("missing-config");
        let state_root = temporary.path().join("missing-state");

        let _report = inspect_native_doctor(
            &environment(&config_root, &state_root),
            NativeDoctorCredentialStatus::Unavailable,
        );

        assert!(!config_root.exists());
        assert!(!state_root.exists());
    }
}
