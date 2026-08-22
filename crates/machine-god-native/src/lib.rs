#![doc = "Explicit native capabilities for machine-god hosts."]

use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

mod ai_gateway;
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
mod ai_gateway_credential;
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
mod ai_gateway_http;
mod ask_permission;
mod config;
mod list_files;
mod read_file;
#[cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]
mod reference_host;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod root_selection;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod session_lifecycle;
mod session_store;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod workspace;

pub use ai_gateway::{
    AI_GATEWAY_DEFAULT_MODEL, AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION,
    AI_GATEWAY_MAX_MODEL_BYTES, AI_GATEWAY_PROTOCOL_VERSION, AI_GATEWAY_PROVIDER_NAME,
    AiGatewayByteStream, AiGatewayConfigError, AiGatewayConfigErrorKind, AiGatewayHeader,
    AiGatewayLimits, AiGatewayProvider, AiGatewayTransport, AiGatewayTransportRequest,
};
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
pub use ai_gateway_credential::{
    AI_GATEWAY_API_KEY_ENV, AiGatewayCredentialEnvironment, AiGatewayCredentialError,
    AiGatewayCredentialErrorKind, AiGatewayCredentialSource, DiscoveredAiGatewayCredential,
    VERCEL_OIDC_TOKEN_ENV, discover_ai_gateway_credential, discover_process_ai_gateway_credential,
};
#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
pub use ai_gateway_http::{
    AI_GATEWAY_HTTP_DEFAULT_CONNECT_TIMEOUT, AI_GATEWAY_HTTP_DEFAULT_ENDPOINT,
    AI_GATEWAY_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS, AI_GATEWAY_HTTP_DEFAULT_REQUEST_TIMEOUT,
    AI_GATEWAY_HTTP_DEFAULT_RESPONSE_CHUNK_BYTES, AI_GATEWAY_HTTP_MAX_ACTIVE_REQUESTS,
    AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES, AI_GATEWAY_HTTP_MAX_CONNECT_TIMEOUT,
    AI_GATEWAY_HTTP_MAX_ENDPOINT_BYTES, AI_GATEWAY_HTTP_MAX_REQUEST_TIMEOUT,
    AI_GATEWAY_HTTP_MAX_RESPONSE_CHUNK_BYTES, AiGatewayBearerToken, AiGatewayHttpConfigError,
    AiGatewayHttpConfigErrorKind, AiGatewayHttpEndpoint, AiGatewayHttpLimits,
    AiGatewayHttpTransport,
};
pub use ask_permission::{
    ASK_PERMISSION_DENIED_REASON, ASK_PERMISSION_PROMPT_ERROR_CODE,
    ASK_PERMISSION_PROMPT_ERROR_MESSAGE, AskPermissionHandler, PermissionPromptDecision,
    PermissionPromptError, PermissionPrompter,
};

pub use config::{
    CONFIG_SCHEMA_VERSION, ConfigOrigin, LoadedNativeConfig, MAX_CONFIG_BYTES, NativeConfig,
    NativeConfigError, NativeConfigErrorKind, NativeCredentialSourceKind, NativeProviderKind,
    NativeTransportKind, load_native_config, load_process_config,
};
pub use list_files::{
    LIST_FILES_TOOL_NAME, ListFilesTool, ListFilesToolOpenError, ListFilesToolOpenErrorKind,
    MAX_LIST_FILES_ENTRIES, MAX_LIST_FILES_PATH_BYTES, MAX_LIST_FILES_TOTAL_NAME_BYTES,
};
pub use read_file::{
    MAX_READ_FILE_BYTES, MAX_READ_FILE_PATH_BYTES, READ_FILE_TOOL_NAME, ReadFileTool,
    ReadFileToolOpenError, ReadFileToolOpenErrorKind,
};
#[cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]
pub use reference_host::{
    NativeReferenceHost, NativeReferenceHostBuildError, NativeReferenceHostBuildErrorKind,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use root_selection::{
    NativeRootSelection, NativeRootSelectionError, NativeRootSelectionErrorKind,
    PreparedNativeRoots, PreparedNativeRootsError, PreparedNativeRootsErrorKind,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use session_lifecycle::{
    MAX_SESSION_INCARNATION_ATTEMPTS, NativeSessionLifecycle, NativeSessionLifecycleBuildError,
    NativeSessionLifecycleBuildErrorKind, NativeSessionLifecycleError,
    NativeSessionLifecycleErrorKind, NativeSessionList, SessionIncarnationSource,
    SessionIncarnationSourceError,
};
pub use session_store::{
    FILE_SESSION_SCHEMA_VERSION, FileSessionStore, FileSessionStoreOpenError,
    FileSessionStoreOpenErrorKind, MAX_FILE_SESSION_BYTES, MAX_LIST_SESSION_DIRECTORY_ENTRIES,
    MAX_LIST_SESSION_TOTAL_RECORD_BYTES, MAX_LIST_SESSIONS,
};

/// Core API version intentionally supported by this native host.
pub const SUPPORTED_CORE_API_VERSION: u32 = 1;

/// Namespace used for machine-god's native state and configuration.
pub const STATE_NAMESPACE: &str = "machine-god";

/// File name used for machine-god's native configuration.
pub const CONFIG_FILE_NAME: &str = "config.json";

/// Returns the core API version supported by this native host.
#[must_use]
pub const fn supported_core_api_version() -> u32 {
    SUPPORTED_CORE_API_VERSION
}

/// Permission behavior used by the native host.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PermissionMode {
    /// Ask before exercising a permission-gated native capability.
    #[default]
    Ask,
}

impl PermissionMode {
    /// Returns the stable, machine-readable name of this mode.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
        }
    }
}

/// Environment inputs used to locate native configuration and state.
///
/// Owned values make inspection deterministic when callers inject a snapshot.
/// The debug representation intentionally reports only whether each input was
/// present because environment values can contain sensitive path information.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeEnvironment {
    xdg_config_home: Option<OsString>,
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
}

impl NativeEnvironment {
    /// Creates an environment snapshot from injected values.
    #[must_use]
    pub const fn new(
        xdg_config_home: Option<OsString>,
        xdg_state_home: Option<OsString>,
        home: Option<OsString>,
    ) -> Self {
        Self {
            xdg_config_home,
            xdg_state_home,
            home,
        }
    }

    /// Captures the relevant values from the current process environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self::new(
            env::var_os("XDG_CONFIG_HOME"),
            env::var_os("XDG_STATE_HOME"),
            env::var_os("HOME"),
        )
    }
}

impl fmt::Debug for NativeEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeEnvironment")
            .field("has_xdg_config_home", &self.xdg_config_home.is_some())
            .field("has_xdg_state_home", &self.xdg_state_home.is_some())
            .field("has_home", &self.home.is_some())
            .finish()
    }
}

/// Observed state of the resolved native configuration file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigFileState {
    /// The path is a regular file.
    File,
    /// Nothing exists at the path.
    Missing,
    /// The path exists but is not a regular file.
    NotFile,
    /// Metadata for the path could not be inspected.
    Inaccessible,
    /// No path could be resolved because no applicable environment input exists.
    Unavailable,
    /// The selected environment input is invalid.
    InvalidEnvironment,
}

impl ConfigFileState {
    /// Returns the stable, machine-readable name of this state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Missing => "missing",
            Self::NotFile => "not_file",
            Self::Inaccessible => "inaccessible",
            Self::Unavailable => "unavailable",
            Self::InvalidEnvironment => "invalid_environment",
        }
    }
}

/// Observed state of the resolved native state directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateDirectoryState {
    /// The path is a directory.
    Directory,
    /// Nothing exists at the path.
    Missing,
    /// The path exists but is not a directory.
    NotDirectory,
    /// Metadata for the path could not be inspected.
    Inaccessible,
    /// No path could be resolved because no applicable environment input exists.
    Unavailable,
    /// The selected environment input is invalid.
    InvalidEnvironment,
}

impl StateDirectoryState {
    /// Returns the stable, machine-readable name of this state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::Missing => "missing",
            Self::NotDirectory => "not_directory",
            Self::Inaccessible => "inaccessible",
            Self::Unavailable => "unavailable",
            Self::InvalidEnvironment => "invalid_environment",
        }
    }
}

/// Read-only native host status derived from an environment snapshot.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeStatus {
    permission_mode: PermissionMode,
    config_file_path: Option<PathBuf>,
    config_file_state: ConfigFileState,
    state_directory_path: Option<PathBuf>,
    state_directory_state: StateDirectoryState,
}

impl NativeStatus {
    /// Returns the permission behavior of the native host.
    #[must_use]
    pub const fn permission_mode(&self) -> PermissionMode {
        self.permission_mode
    }

    /// Returns the resolved configuration file path, when available.
    #[must_use]
    pub fn config_file_path(&self) -> Option<&Path> {
        self.config_file_path.as_deref()
    }

    /// Returns the observed configuration file state.
    #[must_use]
    pub const fn config_file_state(&self) -> ConfigFileState {
        self.config_file_state
    }

    /// Returns the resolved state directory path, when available.
    #[must_use]
    pub fn state_directory_path(&self) -> Option<&Path> {
        self.state_directory_path.as_deref()
    }

    /// Returns the observed state directory state.
    #[must_use]
    pub const fn state_directory_state(&self) -> StateDirectoryState {
        self.state_directory_state
    }
}

impl fmt::Debug for NativeStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStatus")
            .field("permission_mode", &self.permission_mode)
            .field("has_config_file_path", &self.config_file_path.is_some())
            .field("config_file_state", &self.config_file_state)
            .field(
                "has_state_directory_path",
                &self.state_directory_path.is_some(),
            )
            .field("state_directory_state", &self.state_directory_state)
            .finish()
    }
}

/// Resolves and inspects native configuration and state without modifying them.
#[must_use]
pub fn inspect_native_status(environment: &NativeEnvironment) -> NativeStatus {
    let config_file_path = resolve_config_file(environment);
    let state_directory_path = resolve_state_directory(environment);

    let (config_file_path, config_file_state) = match config_file_path {
        ResolvedPath::Path(path) => {
            let state = inspect_config_file(&path);
            (Some(path), state)
        }
        ResolvedPath::Unavailable => (None, ConfigFileState::Unavailable),
        ResolvedPath::InvalidEnvironment => (None, ConfigFileState::InvalidEnvironment),
    };
    let (state_directory_path, state_directory_state) = match state_directory_path {
        ResolvedPath::Path(path) => {
            let state = inspect_state_directory(&path);
            (Some(path), state)
        }
        ResolvedPath::Unavailable => (None, StateDirectoryState::Unavailable),
        ResolvedPath::InvalidEnvironment => (None, StateDirectoryState::InvalidEnvironment),
    };

    NativeStatus {
        permission_mode: PermissionMode::Ask,
        config_file_path,
        config_file_state,
        state_directory_path,
        state_directory_state,
    }
}

/// Captures the process environment and inspects native configuration and state.
#[must_use]
pub fn inspect_process_status() -> NativeStatus {
    inspect_native_status(&NativeEnvironment::from_process())
}

enum ResolvedPath {
    Path(PathBuf),
    Unavailable,
    InvalidEnvironment,
}

fn resolve_config_file(environment: &NativeEnvironment) -> ResolvedPath {
    resolve_root(
        environment.xdg_config_home.as_deref(),
        environment.home.as_deref(),
        &[".config"],
    )
    .map(|root| root.join(STATE_NAMESPACE).join(CONFIG_FILE_NAME))
}

fn resolve_state_directory(environment: &NativeEnvironment) -> ResolvedPath {
    resolve_root(
        environment.xdg_state_home.as_deref(),
        environment.home.as_deref(),
        &[".local", "state"],
    )
    .map(|root| root.join(STATE_NAMESPACE))
}

fn resolve_root(
    selected_xdg: Option<&OsStr>,
    home: Option<&OsStr>,
    home_suffix: &[&str],
) -> ResolvedPath {
    if let Some(root) = nonempty(selected_xdg) {
        return validate_root(root);
    }

    let Some(home) = nonempty(home) else {
        return ResolvedPath::Unavailable;
    };
    validate_root(home).map(|root| home_suffix.iter().fold(root, |path, part| path.join(part)))
}

fn nonempty(value: Option<&OsStr>) -> Option<&OsStr> {
    value.filter(|value| !value.is_empty())
}

fn validate_root(root: &OsStr) -> ResolvedPath {
    let path = Path::new(root);
    if root.to_str().is_none() || !path.is_absolute() {
        ResolvedPath::InvalidEnvironment
    } else {
        ResolvedPath::Path(path.to_path_buf())
    }
}

impl ResolvedPath {
    fn map(self, operation: impl FnOnce(PathBuf) -> PathBuf) -> Self {
        match self {
            Self::Path(path) => Self::Path(operation(path)),
            Self::Unavailable => Self::Unavailable,
            Self::InvalidEnvironment => Self::InvalidEnvironment,
        }
    }
}

fn inspect_config_file(path: &Path) -> ConfigFileState {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_file() => ConfigFileState::File,
        Ok(_) => ConfigFileState::NotFile,
        Err(error) if error.kind() == io::ErrorKind::NotFound => ConfigFileState::Missing,
        Err(_) => ConfigFileState::Inaccessible,
    }
}

fn inspect_state_directory(path: &Path) -> StateDirectoryState {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_dir() => StateDirectoryState::Directory,
        Ok(_) => StateDirectoryState::NotDirectory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => StateDirectoryState::Missing,
        Err(_) => StateDirectoryState::Inaccessible,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CONFIG_FILE_NAME, ConfigFileState, NativeEnvironment, PermissionMode, STATE_NAMESPACE,
        SUPPORTED_CORE_API_VERSION, StateDirectoryState, inspect_native_status,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[derive(Debug)]
    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(test_name: &str) -> Self {
            let base = std::env::temp_dir().join("machine-god-native-tests");
            fs::create_dir_all(&base).expect("failed to create native test base directory");
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
                eprintln!("failed to remove test directory {:?}: {error}", self.path);
            }
        }
    }

    fn environment(
        xdg_config_home: Option<&Path>,
        xdg_state_home: Option<&Path>,
        home: Option<&Path>,
    ) -> NativeEnvironment {
        NativeEnvironment::new(
            xdg_config_home.map(Path::as_os_str).map(OsString::from),
            xdg_state_home.map(Path::as_os_str).map(OsString::from),
            home.map(Path::as_os_str).map(OsString::from),
        )
    }

    #[test]
    fn compatibility_version_is_deliberately_current() {
        assert_eq!(SUPPORTED_CORE_API_VERSION, machine_god_core::API_VERSION);
    }

    #[test]
    fn public_names_and_stable_strings_are_deliberate() {
        assert_eq!(STATE_NAMESPACE, "machine-god");
        assert_eq!(CONFIG_FILE_NAME, "config.json");
        assert_eq!(PermissionMode::default(), PermissionMode::Ask);
        assert_eq!(PermissionMode::Ask.as_str(), "ask");

        let config_states = [
            (ConfigFileState::File, "file"),
            (ConfigFileState::Missing, "missing"),
            (ConfigFileState::NotFile, "not_file"),
            (ConfigFileState::Inaccessible, "inaccessible"),
            (ConfigFileState::Unavailable, "unavailable"),
            (ConfigFileState::InvalidEnvironment, "invalid_environment"),
        ];
        for (state, expected) in config_states {
            assert_eq!(state.as_str(), expected);
        }

        let directory_states = [
            (StateDirectoryState::Directory, "directory"),
            (StateDirectoryState::Missing, "missing"),
            (StateDirectoryState::NotDirectory, "not_directory"),
            (StateDirectoryState::Inaccessible, "inaccessible"),
            (StateDirectoryState::Unavailable, "unavailable"),
            (
                StateDirectoryState::InvalidEnvironment,
                "invalid_environment",
            ),
        ];
        for (state, expected) in directory_states {
            assert_eq!(state.as_str(), expected);
        }
    }

    #[test]
    fn xdg_roots_take_precedence_and_paths_have_expected_shapes() {
        let temporary = TestDirectory::new("xdg-precedence");
        let config_root = temporary.path().join("config-root");
        let state_root = temporary.path().join("state-root");
        let home = temporary.path().join("ignored-home");
        let status = inspect_native_status(&environment(
            Some(&config_root),
            Some(&state_root),
            Some(&home),
        ));

        assert_eq!(status.permission_mode(), PermissionMode::Ask);
        assert_eq!(
            status.config_file_path(),
            Some(config_root.join("machine-god/config.json").as_path())
        );
        assert_eq!(
            status.state_directory_path(),
            Some(state_root.join("machine-god").as_path())
        );
        assert_eq!(status.config_file_state(), ConfigFileState::Missing);
        assert_eq!(status.state_directory_state(), StateDirectoryState::Missing);
        assert!(!status.config_file_path().unwrap().starts_with(&home));
        assert!(!status.state_directory_path().unwrap().starts_with(&home));
    }

    #[test]
    fn config_and_state_resolve_their_roots_independently() {
        let temporary = TestDirectory::new("independent-roots");
        let state_root = temporary.path().join("state-root");
        let home = temporary.path().join("home");
        let status = inspect_native_status(&environment(None, Some(&state_root), Some(&home)));

        assert_eq!(
            status.config_file_path(),
            Some(home.join(".config/machine-god/config.json").as_path())
        );
        assert_eq!(
            status.state_directory_path(),
            Some(state_root.join("machine-god").as_path())
        );
    }

    #[test]
    fn empty_xdg_values_fall_back_to_home() {
        let temporary = TestDirectory::new("empty-xdg");
        let home = temporary.path().join("home");
        let status = inspect_native_status(&NativeEnvironment::new(
            Some(OsString::new()),
            Some(OsString::new()),
            Some(home.as_os_str().to_owned()),
        ));

        assert_eq!(
            status.config_file_path(),
            Some(home.join(".config/machine-god/config.json").as_path())
        );
        assert_eq!(
            status.state_directory_path(),
            Some(home.join(".local/state/machine-god").as_path())
        );
    }

    #[test]
    fn relative_xdg_values_are_invalid_without_home_fallback() {
        let temporary = TestDirectory::new("relative-xdg");
        let home = temporary.path().join("home");
        let status = inspect_native_status(&NativeEnvironment::new(
            Some(OsString::from("relative-config")),
            Some(OsString::from("relative-state")),
            Some(home.as_os_str().to_owned()),
        ));

        assert_eq!(status.config_file_path(), None);
        assert_eq!(
            status.config_file_state(),
            ConfigFileState::InvalidEnvironment
        );
        assert_eq!(status.state_directory_path(), None);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::InvalidEnvironment
        );
    }

    #[test]
    fn missing_or_empty_home_makes_both_paths_unavailable() {
        for home in [None, Some(OsString::new())] {
            let status = inspect_native_status(&NativeEnvironment::new(None, None, home));

            assert_eq!(status.config_file_path(), None);
            assert_eq!(status.config_file_state(), ConfigFileState::Unavailable);
            assert_eq!(status.state_directory_path(), None);
            assert_eq!(
                status.state_directory_state(),
                StateDirectoryState::Unavailable
            );
        }
    }

    #[test]
    fn relative_home_is_invalid() {
        let status = inspect_native_status(&NativeEnvironment::new(
            None,
            None,
            Some(OsString::from("relative-home")),
        ));

        assert_eq!(
            status.config_file_state(),
            ConfigFileState::InvalidEnvironment
        );
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::InvalidEnvironment
        );
    }

    #[test]
    fn regular_file_and_directory_are_recognized_without_parsing() {
        let temporary = TestDirectory::new("expected-kinds");
        let config_root = temporary.path().join("config-root");
        let state_root = temporary.path().join("state-root");
        let config_file = config_root.join("machine-god/config.json");
        let state_directory = state_root.join("machine-god");
        fs::create_dir_all(config_file.parent().unwrap()).unwrap();
        fs::create_dir_all(&state_directory).unwrap();
        fs::write(&config_file, b"this is deliberately not parsed as JSON").unwrap();

        let status =
            inspect_native_status(&environment(Some(&config_root), Some(&state_root), None));

        assert_eq!(status.config_file_state(), ConfigFileState::File);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::Directory
        );
    }

    #[test]
    fn wrong_path_kinds_are_reported() {
        let temporary = TestDirectory::new("wrong-kinds");
        let config_root = temporary.path().join("config-root");
        let state_root = temporary.path().join("state-root");
        let config_file = config_root.join("machine-god/config.json");
        let state_directory = state_root.join("machine-god");
        fs::create_dir_all(&config_file).unwrap();
        fs::create_dir_all(state_directory.parent().unwrap()).unwrap();
        fs::write(&state_directory, b"not a directory").unwrap();

        let status =
            inspect_native_status(&environment(Some(&config_root), Some(&state_root), None));

        assert_eq!(status.config_file_state(), ConfigFileState::NotFile);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::NotDirectory
        );
    }

    #[cfg(unix)]
    #[test]
    fn final_symlinks_are_not_followed() {
        use std::os::unix::fs::symlink;

        let temporary = TestDirectory::new("symlinks");
        let config_root = temporary.path().join("config-root");
        let state_root = temporary.path().join("state-root");
        let targets = temporary.path().join("targets");
        let target_file = targets.join("config.json");
        let target_directory = targets.join("state");
        fs::create_dir_all(&target_directory).unwrap();
        fs::write(&target_file, b"{}").unwrap();

        let config_file = config_root.join("machine-god/config.json");
        let state_directory = state_root.join("machine-god");
        fs::create_dir_all(config_file.parent().unwrap()).unwrap();
        fs::create_dir_all(state_directory.parent().unwrap()).unwrap();
        symlink(&target_file, &config_file).unwrap();
        symlink(&target_directory, &state_directory).unwrap();

        let status =
            inspect_native_status(&environment(Some(&config_root), Some(&state_root), None));

        assert_eq!(status.config_file_state(), ConfigFileState::NotFile);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::NotDirectory
        );
    }

    #[cfg(unix)]
    #[test]
    fn metadata_errors_other_than_not_found_are_inaccessible() {
        let temporary = TestDirectory::new("inaccessible");
        let too_long = "x".repeat(300);
        let config_root = temporary.path().join(&too_long);
        let state_root = temporary.path().join(too_long);

        let status =
            inspect_native_status(&environment(Some(&config_root), Some(&state_root), None));

        assert_eq!(status.config_file_state(), ConfigFileState::Inaccessible);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::Inaccessible
        );
    }

    #[test]
    fn inspection_does_not_create_resolved_paths_or_ancestors() {
        let temporary = TestDirectory::new("no-write");
        let config_root = temporary.path().join("absent-config-root");
        let state_root = temporary.path().join("absent-state-root");

        let status =
            inspect_native_status(&environment(Some(&config_root), Some(&state_root), None));

        assert_eq!(status.config_file_state(), ConfigFileState::Missing);
        assert_eq!(status.state_directory_state(), StateDirectoryState::Missing);
        assert!(!config_root.exists());
        assert!(!state_root.exists());
    }

    #[test]
    fn debug_output_redacts_environment_values_and_resolved_paths() {
        let temporary = TestDirectory::new("debug-redaction");
        let secret = temporary.path().join("do-not-print-this-secret");
        let environment = environment(Some(&secret), Some(&secret), Some(&secret));
        let environment_debug = format!("{environment:?}");
        assert!(environment_debug.contains("has_xdg_config_home: true"));
        assert!(environment_debug.contains("has_xdg_state_home: true"));
        assert!(environment_debug.contains("has_home: true"));
        assert!(!environment_debug.contains("do-not-print-this-secret"));

        let status = inspect_native_status(&environment);
        let status_debug = format!("{status:?}");
        assert!(status_debug.contains("has_config_file_path: true"));
        assert!(status_debug.contains("has_state_directory_path: true"));
        assert!(status_debug.contains("config_file_state: Missing"));
        assert!(!status_debug.contains("do-not-print-this-secret"));
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_selected_roots_are_invalid_without_fallback() {
        use std::os::unix::ffi::OsStringExt;

        let temporary = TestDirectory::new("non-unicode");
        let mut invalid_bytes = temporary.path().as_os_str().as_encoded_bytes().to_vec();
        invalid_bytes.extend_from_slice(b"/invalid-");
        invalid_bytes.push(0xff);
        let invalid = OsString::from_vec(invalid_bytes);
        let home = temporary.path().join("valid-home");
        let status = inspect_native_status(&NativeEnvironment::new(
            Some(invalid.clone()),
            Some(invalid),
            Some(home.as_os_str().to_owned()),
        ));

        assert_eq!(status.config_file_path(), None);
        assert_eq!(
            status.config_file_state(),
            ConfigFileState::InvalidEnvironment
        );
        assert_eq!(status.state_directory_path(), None);
        assert_eq!(
            status.state_directory_state(),
            StateDirectoryState::InvalidEnvironment
        );
    }
}
