//! Bounded, redacted runtime status inspection for native hosts.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

use machine_god_core::EngineLimits;

use super::PermissionMode;
use super::ai_gateway::valid_model;
use super::config::{LoadedNativeConfig, NativeConfigErrorKind, load_process_config};

/// Inclusive maximum byte length of the optional hexadecimal build revision.
pub const MAX_NATIVE_RUNTIME_BUILD_REVISION_BYTES: usize = 12;
/// Inclusive maximum UTF-8 byte length of the canonical workspace path.
pub const MAX_NATIVE_RUNTIME_WORKSPACE_PATH_BYTES: usize = 4 * 1024;
/// Stable update channel reported by this build.
pub const NATIVE_RUNTIME_UPDATE_CHANNEL: &str = "stable";
/// Stable build channel reported by this build.
pub const NATIVE_RUNTIME_BUILD_CHANNEL: &str = "stable";
/// Sandbox mode honestly implemented by the current native host.
pub const NATIVE_RUNTIME_SANDBOX: &str = "none";
/// Fixed help shown when no supported AI Gateway credential is present.
pub const NATIVE_RUNTIME_MISSING_AUTH_HELP: &str =
    "Machine God needs access to Vercel AI Gateway. Set VERCEL_OIDC_TOKEN or AI_GATEWAY_API_KEY.";

const MAX_RUNTIME_CREDENTIAL_BYTES: usize = 4 * 1024;

/// Non-secret credential source selected for a runtime status snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeRuntimeCredentialSource {
    /// The preferred Vercel OIDC environment credential.
    VercelOidcToken,
    /// The fallback AI Gateway API-key environment credential.
    AiGatewayApiKey,
    /// Neither supported credential was present.
    Missing,
}

impl NativeRuntimeCredentialSource {
    /// Returns the pinned human- and machine-readable status label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VercelOidcToken => "VERCEL_OIDC_TOKEN",
            Self::AiGatewayApiKey => "AI_GATEWAY_API_KEY",
            Self::Missing => "missing",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CredentialValue {
    Absent,
    Valid,
    InvalidEnvironment,
    InvalidBearerToken,
}

/// Preclassified, redacted snapshot of supported credential environment values.
///
/// Construction validates and immediately discards the supplied credential
/// bytes. The snapshot retains only absent, valid, or fixed invalid states.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeRuntimeCredentialEnvironment {
    vercel_oidc_token: CredentialValue,
    ai_gateway_api_key: CredentialValue,
}

impl NativeRuntimeCredentialEnvironment {
    /// Classifies explicitly injected credential values without retaining them.
    #[must_use]
    pub fn new(vercel_oidc_token: Option<OsString>, ai_gateway_api_key: Option<OsString>) -> Self {
        Self {
            vercel_oidc_token: classify_credential(vercel_oidc_token),
            ai_gateway_api_key: classify_credential(ai_gateway_api_key),
        }
    }

    /// Captures and immediately classifies the two supported process values.
    #[must_use]
    pub fn from_process() -> Self {
        Self::new(
            std::env::var_os("VERCEL_OIDC_TOKEN"),
            std::env::var_os("AI_GATEWAY_API_KEY"),
        )
    }
}

impl fmt::Debug for NativeRuntimeCredentialEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeRuntimeCredentialEnvironment(<redacted>)")
    }
}

/// Explicit, effect-free inputs for a runtime status snapshot.
///
/// The workspace must already be canonical when supplied by a host. The
/// production inspector obtains that guarantee through `std::fs::canonicalize`.
pub struct NativeRuntimeStatusInput {
    model: String,
    permission_mode: PermissionMode,
    credentials: NativeRuntimeCredentialEnvironment,
    canonical_workspace: PathBuf,
    build_revision: String,
}

impl NativeRuntimeStatusInput {
    /// Creates an explicit runtime-status input.
    ///
    /// An invalid build revision is deliberately normalized to an empty value.
    #[must_use]
    pub fn new(
        model: impl Into<String>,
        permission_mode: PermissionMode,
        credentials: NativeRuntimeCredentialEnvironment,
        canonical_workspace: impl Into<PathBuf>,
        build_revision: Option<&str>,
    ) -> Self {
        Self {
            model: model.into(),
            permission_mode,
            credentials,
            canonical_workspace: canonical_workspace.into(),
            build_revision: build_revision
                .filter(|revision| valid_build_revision(revision))
                .unwrap_or_default()
                .to_owned(),
        }
    }
}

impl fmt::Debug for NativeRuntimeStatusInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRuntimeStatusInput")
            .field("model_bytes", &self.model.len())
            .field("permission_mode", &self.permission_mode)
            .field("credentials", &self.credentials)
            .field("workspace_bytes", &path_bytes(&self.canonical_workspace))
            .field("build_revision_bytes", &self.build_revision.len())
            .finish()
    }
}

/// Provider-neutral native runtime status snapshot.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeRuntimeStatus {
    model: String,
    permission_mode: PermissionMode,
    build_revision: String,
    credential_source: NativeRuntimeCredentialSource,
    workspace: PathBuf,
    agent_step_limit: usize,
}

impl NativeRuntimeStatus {
    /// Returns the validated configured model identifier.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Returns the stable update channel.
    #[must_use]
    pub const fn update_channel(&self) -> &'static str {
        NATIVE_RUNTIME_UPDATE_CHANNEL
    }

    /// Returns the stable build channel.
    #[must_use]
    pub const fn build_channel(&self) -> &'static str {
        NATIVE_RUNTIME_BUILD_CHANNEL
    }

    /// Returns the optional validated hexadecimal build revision.
    #[must_use]
    pub fn build_revision(&self) -> &str {
        &self.build_revision
    }

    /// Returns the selected non-secret credential source.
    #[must_use]
    pub const fn credential_source(&self) -> NativeRuntimeCredentialSource {
        self.credential_source
    }

    /// Returns the selected credential source label.
    #[must_use]
    pub const fn auth_label(&self) -> &'static str {
        self.credential_source.as_str()
    }

    /// Environment credentials do not support refresh through machine-god.
    #[must_use]
    pub const fn auth_refreshable(&self) -> bool {
        false
    }

    /// Returns fixed setup help only when no credential is present.
    #[must_use]
    pub const fn auth_help(&self) -> Option<&'static str> {
        match self.credential_source {
            NativeRuntimeCredentialSource::Missing => Some(NATIVE_RUNTIME_MISSING_AUTH_HELP),
            NativeRuntimeCredentialSource::VercelOidcToken
            | NativeRuntimeCredentialSource::AiGatewayApiKey => None,
        }
    }

    /// Returns the configured permission mode.
    #[must_use]
    pub const fn permission_mode(&self) -> PermissionMode {
        self.permission_mode
    }

    /// Returns the current implemented sandbox mode.
    #[must_use]
    pub const fn sandbox(&self) -> &'static str {
        NATIVE_RUNTIME_SANDBOX
    }

    /// Returns the bounded canonical Unicode workspace path.
    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// A standalone status inspection has no loaded session history.
    #[must_use]
    pub const fn history_turns(&self) -> usize {
        0
    }

    /// A standalone status inspection has no runtime permission grants.
    #[must_use]
    pub const fn session_permission_grants(&self) -> usize {
        0
    }

    /// Returns the default engine model-round limit used as the agent step limit.
    #[must_use]
    pub const fn agent_step_limit(&self) -> usize {
        self.agent_step_limit
    }
}

impl fmt::Debug for NativeRuntimeStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRuntimeStatus")
            .field("model_bytes", &self.model.len())
            .field("permission_mode", &self.permission_mode)
            .field("build_revision_bytes", &self.build_revision.len())
            .field("credential_source", &self.credential_source)
            .field("workspace_bytes", &path_bytes(&self.workspace))
            .field("agent_step_limit", &self.agent_step_limit)
            .finish()
    }
}

/// Stable category for runtime status inspection failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeRuntimeStatusErrorKind {
    /// The selected configuration failed its bounded validation.
    Configuration(NativeConfigErrorKind),
    /// The injected model identifier is invalid.
    InvalidModel,
    /// The selected credential environment value is not Unicode.
    InvalidCredentialEnvironment,
    /// The selected credential is malformed or oversized.
    InvalidCredential,
    /// The workspace path is unavailable or not an absolute canonical shape.
    WorkspaceUnavailable,
    /// The workspace path exceeds its UTF-8 byte limit.
    WorkspaceResourceLimit,
}

/// Fixed, redacted runtime status inspection failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeRuntimeStatusError {
    kind: NativeRuntimeStatusErrorKind,
}

impl NativeRuntimeStatusError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> NativeRuntimeStatusErrorKind {
        self.kind
    }

    const fn new(kind: NativeRuntimeStatusErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for NativeRuntimeStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRuntimeStatusError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeRuntimeStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeRuntimeStatusErrorKind::Configuration(_) => {
                "native runtime status configuration is invalid"
            }
            NativeRuntimeStatusErrorKind::InvalidModel => "native runtime status model is invalid",
            NativeRuntimeStatusErrorKind::InvalidCredentialEnvironment => {
                "native runtime status credential environment is invalid"
            }
            NativeRuntimeStatusErrorKind::InvalidCredential => {
                "native runtime status credential is invalid"
            }
            NativeRuntimeStatusErrorKind::WorkspaceUnavailable => {
                "native runtime status workspace is unavailable"
            }
            NativeRuntimeStatusErrorKind::WorkspaceResourceLimit => {
                "native runtime status workspace exceeded a resource limit"
            }
        })
    }
}

impl Error for NativeRuntimeStatusError {}

/// Builds a status snapshot from explicit, already captured inputs.
///
/// This operation performs no ambient reads, filesystem access, writes, or
/// network access. Hosts supplying the workspace are responsible for obtaining
/// it canonically; the accepted path is still checked for absolute Unicode
/// shape and its fixed byte bound.
///
/// # Errors
///
/// Returns a fixed typed error for an invalid model, selected credential, or
/// workspace path.
pub fn inspect_native_runtime_status(
    input: NativeRuntimeStatusInput,
) -> Result<NativeRuntimeStatus, NativeRuntimeStatusError> {
    if !valid_model(&input.model) {
        return Err(NativeRuntimeStatusError::new(
            NativeRuntimeStatusErrorKind::InvalidModel,
        ));
    }
    validate_workspace(&input.canonical_workspace)?;
    let credential_source = select_credential_source(input.credentials)?;
    let agent_step_limit = EngineLimits::default().max_model_rounds.get();

    Ok(NativeRuntimeStatus {
        model: input.model,
        permission_mode: input.permission_mode,
        build_revision: input.build_revision,
        credential_source,
        workspace: input.canonical_workspace,
        agent_step_limit,
    })
}

/// Loads validated process configuration, classifies supported credentials,
/// and captures the canonical current workspace without modifying anything.
///
/// Inspection is synchronous and bounded. It performs no writes, state
/// preparation, runtime construction, session loading, or network access.
/// Credential bytes are discarded during classification and never appear in
/// the returned snapshot or error values.
///
/// # Errors
///
/// Returns a fixed typed error when configuration, credentials, or the current
/// workspace cannot produce a valid bounded snapshot.
pub fn inspect_process_runtime_status() -> Result<NativeRuntimeStatus, NativeRuntimeStatusError> {
    inspect_process_runtime_status_with(
        load_process_config,
        NativeRuntimeCredentialEnvironment::from_process,
        std::env::current_dir,
        |path| fs::canonicalize(path),
        option_env!("MACHINE_GOD_BUILD_REVISION"),
    )
}

fn inspect_process_runtime_status_with(
    load_config: impl FnOnce() -> Result<LoadedNativeConfig, super::NativeConfigError>,
    capture_credentials: impl FnOnce() -> NativeRuntimeCredentialEnvironment,
    current_directory: impl FnOnce() -> std::io::Result<PathBuf>,
    canonicalize: impl FnOnce(&Path) -> std::io::Result<PathBuf>,
    build_revision: Option<&str>,
) -> Result<NativeRuntimeStatus, NativeRuntimeStatusError> {
    let loaded = load_config().map_err(|error| {
        NativeRuntimeStatusError::new(NativeRuntimeStatusErrorKind::Configuration(error.kind()))
    })?;
    let credentials = capture_credentials();
    let current_directory = current_directory().map_err(|_| {
        NativeRuntimeStatusError::new(NativeRuntimeStatusErrorKind::WorkspaceUnavailable)
    })?;
    validate_workspace_input(&current_directory)?;
    let canonical_workspace = canonicalize(&current_directory).map_err(|_| {
        NativeRuntimeStatusError::new(NativeRuntimeStatusErrorKind::WorkspaceUnavailable)
    })?;

    inspect_native_runtime_status(NativeRuntimeStatusInput::new(
        loaded.config().model(),
        loaded.config().permission_mode(),
        credentials,
        canonical_workspace,
        build_revision,
    ))
}

fn select_credential_source(
    environment: NativeRuntimeCredentialEnvironment,
) -> Result<NativeRuntimeCredentialSource, NativeRuntimeStatusError> {
    match environment.vercel_oidc_token {
        CredentialValue::Valid => Ok(NativeRuntimeCredentialSource::VercelOidcToken),
        CredentialValue::InvalidEnvironment => Err(NativeRuntimeStatusError::new(
            NativeRuntimeStatusErrorKind::InvalidCredentialEnvironment,
        )),
        CredentialValue::InvalidBearerToken => Err(NativeRuntimeStatusError::new(
            NativeRuntimeStatusErrorKind::InvalidCredential,
        )),
        CredentialValue::Absent => match environment.ai_gateway_api_key {
            CredentialValue::Valid => Ok(NativeRuntimeCredentialSource::AiGatewayApiKey),
            CredentialValue::Absent => Ok(NativeRuntimeCredentialSource::Missing),
            CredentialValue::InvalidEnvironment => Err(NativeRuntimeStatusError::new(
                NativeRuntimeStatusErrorKind::InvalidCredentialEnvironment,
            )),
            CredentialValue::InvalidBearerToken => Err(NativeRuntimeStatusError::new(
                NativeRuntimeStatusErrorKind::InvalidCredential,
            )),
        },
    }
}

fn classify_credential(value: Option<OsString>) -> CredentialValue {
    let Some(value) = value else {
        return CredentialValue::Absent;
    };
    if value.is_empty() {
        return CredentialValue::Absent;
    }
    let value = match value.into_string() {
        Ok(value) => value,
        Err(value) => {
            discard_non_unicode(value);
            return CredentialValue::InvalidEnvironment;
        }
    };
    let mut bytes = value.into_bytes();
    let valid = valid_bearer_token(&bytes);
    bytes.fill(0);
    if valid {
        CredentialValue::Valid
    } else {
        CredentialValue::InvalidBearerToken
    }
}

fn valid_bearer_token(token: &[u8]) -> bool {
    if token.is_empty() || token.len() > MAX_RUNTIME_CREDENTIAL_BYTES {
        return false;
    }
    let Some(first_padding) = token.iter().position(|byte| *byte == b'=') else {
        return token.iter().copied().all(valid_bearer_token_byte);
    };
    first_padding > 0
        && token[..first_padding]
            .iter()
            .copied()
            .all(valid_bearer_token_byte)
        && token[first_padding..].iter().all(|byte| *byte == b'=')
}

const fn valid_bearer_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
}

#[cfg(unix)]
fn discard_non_unicode(value: OsString) {
    let mut bytes = value.into_vec();
    bytes.fill(0);
}

#[cfg(not(unix))]
fn discard_non_unicode(value: OsString) {
    drop(value);
}

fn validate_workspace_input(path: &Path) -> Result<(), NativeRuntimeStatusError> {
    if !path.is_absolute() || path.to_str().is_none() {
        return Err(NativeRuntimeStatusError::new(
            NativeRuntimeStatusErrorKind::WorkspaceUnavailable,
        ));
    }
    if path_bytes(path).is_some_and(|length| length > MAX_NATIVE_RUNTIME_WORKSPACE_PATH_BYTES) {
        return Err(NativeRuntimeStatusError::new(
            NativeRuntimeStatusErrorKind::WorkspaceResourceLimit,
        ));
    }
    Ok(())
}

fn validate_workspace(path: &Path) -> Result<(), NativeRuntimeStatusError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(NativeRuntimeStatusError::new(
            NativeRuntimeStatusErrorKind::WorkspaceUnavailable,
        ));
    }
    let path = path.to_str().ok_or_else(|| {
        NativeRuntimeStatusError::new(NativeRuntimeStatusErrorKind::WorkspaceUnavailable)
    })?;
    if path.len() > MAX_NATIVE_RUNTIME_WORKSPACE_PATH_BYTES {
        return Err(NativeRuntimeStatusError::new(
            NativeRuntimeStatusErrorKind::WorkspaceResourceLimit,
        ));
    }
    Ok(())
}

fn path_bytes(path: &Path) -> Option<usize> {
    path.to_str().map(str::len)
}

const fn valid_build_revision(revision: &str) -> bool {
    let bytes = revision.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_NATIVE_RUNTIME_BUILD_REVISION_BYTES {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_hexdigit() {
            return false;
        }
        index += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        MAX_NATIVE_RUNTIME_BUILD_REVISION_BYTES, MAX_NATIVE_RUNTIME_WORKSPACE_PATH_BYTES,
        NATIVE_RUNTIME_BUILD_CHANNEL, NATIVE_RUNTIME_MISSING_AUTH_HELP, NATIVE_RUNTIME_SANDBOX,
        NATIVE_RUNTIME_UPDATE_CHANNEL, NativeRuntimeCredentialEnvironment,
        NativeRuntimeCredentialSource, NativeRuntimeStatusErrorKind, NativeRuntimeStatusInput,
        inspect_native_runtime_status, inspect_process_runtime_status_with,
    };
    use crate::{
        AI_GATEWAY_DEFAULT_MODEL, NativeConfigErrorKind, NativeEnvironment, PermissionMode,
        load_native_config,
    };

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "machine-god-runtime-status-{}-{label}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn credentials(
        oidc: Option<&str>,
        api_key: Option<&str>,
    ) -> NativeRuntimeCredentialEnvironment {
        NativeRuntimeCredentialEnvironment::new(
            oidc.map(OsString::from),
            api_key.map(OsString::from),
        )
    }

    fn input(
        credentials: NativeRuntimeCredentialEnvironment,
        workspace: impl Into<PathBuf>,
        revision: Option<&str>,
    ) -> NativeRuntimeStatusInput {
        NativeRuntimeStatusInput::new(
            AI_GATEWAY_DEFAULT_MODEL,
            PermissionMode::Ask,
            credentials,
            workspace,
            revision,
        )
    }

    #[test]
    fn explicit_snapshot_matches_the_product_adapted_pinned_status_fields() {
        let status = inspect_native_runtime_status(input(
            credentials(None, None),
            PathBuf::from("/work/machine-god"),
            Some("a1B2c3D4e5F6"),
        ))
        .unwrap();

        assert_eq!(status.model(), AI_GATEWAY_DEFAULT_MODEL);
        assert_eq!(status.update_channel(), NATIVE_RUNTIME_UPDATE_CHANNEL);
        assert_eq!(status.build_channel(), NATIVE_RUNTIME_BUILD_CHANNEL);
        assert_eq!(status.build_revision(), "a1B2c3D4e5F6");
        assert_eq!(
            status.credential_source(),
            NativeRuntimeCredentialSource::Missing
        );
        assert_eq!(status.auth_label(), "missing");
        assert!(!status.auth_refreshable());
        assert_eq!(status.auth_help(), Some(NATIVE_RUNTIME_MISSING_AUTH_HELP));
        assert_eq!(status.permission_mode(), PermissionMode::Ask);
        assert_eq!(status.sandbox(), NATIVE_RUNTIME_SANDBOX);
        assert_eq!(status.workspace(), Path::new("/work/machine-god"));
        assert_eq!(status.history_turns(), 0);
        assert_eq!(status.session_permission_grants(), 0);
        assert_eq!(status.agent_step_limit(), 8);
    }

    #[test]
    fn credential_precedence_and_missing_help_are_stable() {
        let cases = [
            (Some("oidc-token"), Some("api-key"), "VERCEL_OIDC_TOKEN"),
            (Some(""), Some("api-key"), "AI_GATEWAY_API_KEY"),
            (None, Some("api-key"), "AI_GATEWAY_API_KEY"),
            (None, None, "missing"),
        ];

        for (oidc, api_key, expected) in cases {
            let status = inspect_native_runtime_status(input(
                credentials(oidc, api_key),
                PathBuf::from("/workspace"),
                None,
            ))
            .unwrap();
            assert_eq!(status.auth_label(), expected);
            assert_eq!(status.auth_help().is_some(), expected == "missing");
            assert!(!status.auth_refreshable());
        }
    }

    #[test]
    fn selected_invalid_credentials_fail_closed_without_secret_disclosure() {
        let invalid = [
            credentials(Some("contains space"), Some("valid-fallback")),
            credentials(None, Some("bad=x")),
            credentials(None, Some(&"x".repeat(4 * 1024 + 1))),
        ];
        for environment in invalid {
            let error = inspect_native_runtime_status(input(
                environment,
                PathBuf::from("/workspace"),
                None,
            ))
            .unwrap_err();
            assert_eq!(
                error.kind(),
                NativeRuntimeStatusErrorKind::InvalidCredential
            );
            let rendered = format!("{error:?} {error}");
            for secret in ["contains space", "valid-fallback", "bad=x"] {
                assert!(!rendered.contains(secret));
            }
        }

        let valid_preferred_ignores_invalid_fallback = inspect_native_runtime_status(input(
            credentials(Some("valid-oidc"), Some("bad=x")),
            PathBuf::from("/workspace"),
            None,
        ))
        .unwrap();
        assert_eq!(
            valid_preferred_ignores_invalid_fallback.credential_source(),
            NativeRuntimeCredentialSource::VercelOidcToken
        );
    }

    #[cfg(unix)]
    #[test]
    fn selected_non_unicode_credential_has_a_distinct_fixed_error() {
        use std::os::unix::ffi::OsStringExt;

        let environment = NativeRuntimeCredentialEnvironment::new(
            Some(OsString::from_vec(vec![0xff])),
            Some(OsString::from("valid-fallback")),
        );
        let error =
            inspect_native_runtime_status(input(environment, PathBuf::from("/workspace"), None))
                .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeRuntimeStatusErrorKind::InvalidCredentialEnvironment
        );
        assert_eq!(
            error.to_string(),
            "native runtime status credential environment is invalid"
        );
    }

    #[test]
    fn invalid_model_workspace_and_revision_boundaries_are_enforced() {
        let error = inspect_native_runtime_status(NativeRuntimeStatusInput::new(
            "bad model",
            PermissionMode::Ask,
            credentials(None, None),
            "/workspace",
            None,
        ))
        .unwrap_err();
        assert_eq!(error.kind(), NativeRuntimeStatusErrorKind::InvalidModel);

        let error = inspect_native_runtime_status(input(
            credentials(None, None),
            PathBuf::from("relative"),
            None,
        ))
        .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeRuntimeStatusErrorKind::WorkspaceUnavailable
        );

        let workspace = format!("/{}", "x".repeat(MAX_NATIVE_RUNTIME_WORKSPACE_PATH_BYTES));
        let error = inspect_native_runtime_status(input(credentials(None, None), workspace, None))
            .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeRuntimeStatusErrorKind::WorkspaceResourceLimit
        );

        for invalid_revision in [
            "",
            "not-hex",
            &"a".repeat(MAX_NATIVE_RUNTIME_BUILD_REVISION_BYTES + 1),
        ] {
            let status = inspect_native_runtime_status(input(
                credentials(None, None),
                PathBuf::from("/workspace"),
                Some(invalid_revision),
            ))
            .unwrap();
            assert_eq!(status.build_revision(), "");
        }
    }

    #[test]
    fn process_path_loads_config_defaults_and_canonicalizes_once() {
        let temporary = TemporaryDirectory::new("defaults");
        let environment = NativeEnvironment::new(None, None, None);
        let current_calls = Cell::new(0usize);
        let canonical_calls = Cell::new(0usize);
        let call_order = RefCell::new(Vec::new());
        let lexical = temporary.path().join("child").join("..");
        let canonical = temporary.path().to_path_buf();

        let status = inspect_process_runtime_status_with(
            || {
                call_order.borrow_mut().push("config");
                load_native_config(&environment)
            },
            || {
                call_order.borrow_mut().push("credentials");
                credentials(None, None)
            },
            || {
                call_order.borrow_mut().push("current_directory");
                current_calls.set(current_calls.get() + 1);
                Ok(lexical.clone())
            },
            |observed| {
                call_order.borrow_mut().push("canonicalize");
                canonical_calls.set(canonical_calls.get() + 1);
                assert_eq!(observed, lexical);
                Ok(canonical.clone())
            },
            Some("abc123"),
        )
        .unwrap();

        assert_eq!(current_calls.get(), 1);
        assert_eq!(canonical_calls.get(), 1);
        assert_eq!(
            call_order.into_inner(),
            ["config", "credentials", "current_directory", "canonicalize"]
        );
        assert_eq!(status.model(), AI_GATEWAY_DEFAULT_MODEL);
        assert_eq!(status.permission_mode(), PermissionMode::Ask);
        assert_eq!(status.workspace(), temporary.path());
    }

    #[test]
    fn process_path_reports_the_validated_configured_model() {
        let temporary = TemporaryDirectory::new("configured");
        let config_root = temporary.path().join("config");
        let config_directory = config_root.join("machine-god");
        fs::create_dir_all(&config_directory).unwrap();
        fs::write(
            config_directory.join("config.json"),
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"custom/status-model","credential_source":"environment"}"#,
        )
        .unwrap();
        let environment = NativeEnvironment::new(Some(config_root.into_os_string()), None, None);

        let status = inspect_process_runtime_status_with(
            || load_native_config(&environment),
            || credentials(None, Some("api-key")),
            || Ok(temporary.path().to_path_buf()),
            |path| Ok(path.to_path_buf()),
            None,
        )
        .unwrap();

        assert_eq!(status.model(), "custom/status-model");
        assert_eq!(status.auth_label(), "AI_GATEWAY_API_KEY");
    }

    #[test]
    fn config_and_canonicalization_failures_remain_typed_and_redacted() {
        let temporary = TemporaryDirectory::new("invalid-config");
        let config_root = temporary.path().join("config");
        let config_directory = config_root.join("machine-god");
        fs::create_dir_all(&config_directory).unwrap();
        fs::write(
            config_directory.join("config.json"),
            br#"{"schema_version":3,"permission_mode":"deny","secret":"HIDDEN"}"#,
        )
        .unwrap();
        let environment = NativeEnvironment::new(Some(config_root.into_os_string()), None, None);
        let credential_calls = Cell::new(0usize);
        let current_directory_calls = Cell::new(0usize);
        let error = inspect_process_runtime_status_with(
            || load_native_config(&environment),
            || {
                credential_calls.set(credential_calls.get() + 1);
                credentials(None, None)
            },
            || {
                current_directory_calls.set(current_directory_calls.get() + 1);
                Ok(temporary.path().to_path_buf())
            },
            |path| Ok(path.to_path_buf()),
            None,
        )
        .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeRuntimeStatusErrorKind::Configuration(NativeConfigErrorKind::InvalidFormat)
        );
        assert_eq!(credential_calls.get(), 0);
        assert_eq!(current_directory_calls.get(), 0);
        assert!(!format!("{error:?} {error}").contains("HIDDEN"));

        let error = inspect_process_runtime_status_with(
            || load_native_config(&NativeEnvironment::new(None, None, None)),
            || credentials(None, None),
            || Ok(temporary.path().to_path_buf()),
            |_| Err(std::io::Error::other("/secret/workspace")),
            None,
        )
        .unwrap_err();
        assert_eq!(
            error.kind(),
            NativeRuntimeStatusErrorKind::WorkspaceUnavailable
        );
        assert!(!format!("{error:?} {error}").contains("/secret/workspace"));
    }
}
