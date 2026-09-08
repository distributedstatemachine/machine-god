use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use serde::Deserialize;
use serde_json::value::RawValue;

use super::ai_gateway::{AI_GATEWAY_DEFAULT_MODEL, valid_model};
use super::{
    NativeConfiguredPermissionDecision, NativeConfiguredPermissionRule,
    NativeConfiguredPermissionRules, NativeSandboxMode,
};
use super::{NativeEnvironment, PermissionMode, ResolvedPath, resolve_config_file};
use super::{NativeModelPreferences, NativeReasoningEffort};

/// Current configuration schema version used by this native host.
pub const CONFIG_SCHEMA_VERSION: u32 = 5;

/// Maximum number of bytes retained while loading a native configuration.
pub const MAX_CONFIG_BYTES: usize = 64 * 1024;

const MAX_CONFIG_INTERRUPTED_READ_ATTEMPTS: usize = 16;

/// Provider selected by a native host configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeProviderKind {
    /// Vercel AI Gateway.
    VercelAiGateway,
}

impl NativeProviderKind {
    /// Returns the stable, machine-readable provider name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VercelAiGateway => "vercel_ai_gateway",
        }
    }
}

/// Transport selected by a native host configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTransportKind {
    /// Native AI Gateway HTTP transport.
    AiGatewayHttp,
}

impl NativeTransportKind {
    /// Returns the stable, machine-readable transport name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AiGatewayHttp => "ai_gateway_http",
        }
    }
}

/// Credential source selected by a native host configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeCredentialSourceKind {
    /// Discover credentials from the native host environment snapshot.
    Environment,
}

impl NativeCredentialSourceKind {
    /// Returns the stable, machine-readable credential-source name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
        }
    }
}

/// Validated native host configuration.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeConfig {
    schema_version: u32,
    permission_mode: PermissionMode,
    sandbox_mode: NativeSandboxMode,
    permission_rules: NativeConfiguredPermissionRules,
    provider: NativeProviderKind,
    transport: NativeTransportKind,
    model: String,
    credential_source: NativeCredentialSourceKind,
    effort: NativeReasoningEffort,
    fast_mode: bool,
}

impl NativeConfig {
    /// Returns the requested reasoning effort (legacy files project automatic).
    #[must_use]
    pub const fn effort(&self) -> &NativeReasoningEffort {
        &self.effort
    }

    /// Returns requested fast mode, independently of current model capabilities.
    #[must_use]
    pub const fn fast_mode(&self) -> bool {
        self.fast_mode
    }

    /// Returns the complete validated default model selection.
    ///
    /// # Panics
    /// Panics only if an internal constructor violates the validated model invariant.
    #[must_use]
    pub fn model_preferences(&self) -> NativeModelPreferences {
        NativeModelPreferences::new(&self.model, self.effort.clone(), self.fast_mode)
            .expect("configuration model preferences are validated")
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn with_model_preferences(&self, preferences: &NativeModelPreferences) -> Self {
        let mut config = self.clone();
        config.schema_version = CONFIG_SCHEMA_VERSION;
        preferences.model().clone_into(&mut config.model);
        config.effort = preferences.effort().clone();
        config.fast_mode = preferences.requested_fast();
        config
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn serialize_current(&self) -> Result<Vec<u8>, NativeConfigError> {
        #[derive(serde::Serialize)]
        struct View<'a> {
            schema_version: u32,
            permission_mode: &'a str,
            sandbox_mode: &'a str,
            permission_rules: &'a NativeConfiguredPermissionRules,
            provider: &'a str,
            transport: &'a str,
            model: &'a str,
            credential_source: &'a str,
            effort: &'a str,
            fast_mode: bool,
        }
        struct BoundedBytes(Vec<u8>);
        impl io::Write for BoundedBytes {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if bytes.len() > MAX_CONFIG_BYTES.saturating_sub(self.0.len()) {
                    return Err(io::ErrorKind::FileTooLarge.into());
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut encoded = BoundedBytes(Vec::new());
        serde_json::to_writer(
            &mut encoded,
            &View {
                schema_version: CONFIG_SCHEMA_VERSION,
                permission_mode: self.permission_mode.as_str(),
                sandbox_mode: self.sandbox_mode.as_str(),
                permission_rules: &self.permission_rules,
                provider: self.provider.as_str(),
                transport: self.transport.as_str(),
                model: &self.model,
                credential_source: self.credential_source.as_str(),
                effort: self.effort.label(),
                fast_mode: self.fast_mode,
            },
        )
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::TooLarge))?;
        Ok(encoded.0)
    }
    /// Returns the schema version observed in the loaded configuration.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the configured permission behavior.
    #[must_use]
    pub const fn permission_mode(&self) -> PermissionMode {
        self.permission_mode
    }

    /// Returns the requested sandbox preference, not effective enforcement.
    #[must_use]
    pub const fn sandbox_mode(&self) -> NativeSandboxMode {
        self.sandbox_mode
    }

    /// Returns ordered configured patterns, separately from saved exact rules.
    #[must_use]
    pub const fn permission_rules(&self) -> &NativeConfiguredPermissionRules {
        &self.permission_rules
    }

    /// Returns the configured provider.
    #[must_use]
    pub const fn provider(&self) -> NativeProviderKind {
        self.provider
    }

    /// Returns the configured transport.
    #[must_use]
    pub const fn transport(&self) -> NativeTransportKind {
        self.transport
    }

    /// Returns the configured model identifier.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Returns the configured credential source.
    #[must_use]
    pub const fn credential_source(&self) -> NativeCredentialSourceKind {
        self.credential_source
    }
}

impl Default for NativeConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            permission_mode: PermissionMode::Ask,
            sandbox_mode: NativeSandboxMode::Os,
            permission_rules: NativeConfiguredPermissionRules::default(),
            provider: NativeProviderKind::VercelAiGateway,
            transport: NativeTransportKind::AiGatewayHttp,
            model: AI_GATEWAY_DEFAULT_MODEL.to_owned(),
            credential_source: NativeCredentialSourceKind::Environment,
            effort: NativeReasoningEffort::default(),
            fast_mode: false,
        }
    }
}

impl fmt::Debug for NativeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeConfig")
            .field("schema_version", &self.schema_version)
            .field("permission_mode", &self.permission_mode)
            .field("sandbox_mode", &self.sandbox_mode)
            .field("permission_rules", &"<redacted>")
            .field("provider", &self.provider)
            .field("transport", &self.transport)
            .field("model", &"<redacted>")
            .field("credential_source", &self.credential_source)
            .field("effort", &"<redacted>")
            .field("fast_mode", &self.fast_mode)
            .finish()
    }
}

/// Source from which a native configuration was loaded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigOrigin {
    /// No configuration location or file was available, so safe defaults apply.
    BuiltInDefaults,
    /// A configuration file was opened, bounded, and validated.
    File,
}

/// A validated native configuration together with its source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedNativeConfig {
    config: NativeConfig,
    origin: ConfigOrigin,
}

impl LoadedNativeConfig {
    /// Returns the validated native configuration.
    #[must_use]
    pub const fn config(&self) -> &NativeConfig {
        &self.config
    }

    /// Returns the source of the loaded configuration.
    #[must_use]
    pub const fn origin(&self) -> ConfigOrigin {
        self.origin
    }

    pub(crate) fn built_in_defaults() -> Self {
        Self {
            config: NativeConfig::default(),
            origin: ConfigOrigin::BuiltInDefaults,
        }
    }

    pub(crate) fn from_file(config: NativeConfig) -> Self {
        Self {
            config,
            origin: ConfigOrigin::File,
        }
    }
}

/// Stable category for a native configuration load failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeConfigErrorKind {
    /// The selected configuration environment value is invalid.
    InvalidEnvironment,
    /// The selected path is a symlink or another non-regular file type.
    InvalidFileType,
    /// The file could not be safely opened, inspected, or read.
    Unreadable,
    /// The file exceeds [`MAX_CONFIG_BYTES`].
    TooLarge,
    /// The file is not valid UTF-8 JSON matching the strict configuration schema.
    InvalidFormat,
    /// The file uses a schema version this native host does not support.
    UnsupportedSchemaVersion,
}

/// Redacted native configuration load failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeConfigError {
    kind: NativeConfigErrorKind,
}

impl NativeConfigError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> NativeConfigErrorKind {
        self.kind
    }

    const fn new(kind: NativeConfigErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for NativeConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeConfigError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeConfigErrorKind::InvalidEnvironment => {
                "native configuration environment is invalid"
            }
            NativeConfigErrorKind::InvalidFileType => {
                "native configuration path is not a regular file"
            }
            NativeConfigErrorKind::Unreadable => "native configuration file is unreadable",
            NativeConfigErrorKind::TooLarge => "native configuration file is too large",
            NativeConfigErrorKind::InvalidFormat => "native configuration format is invalid",
            NativeConfigErrorKind::UnsupportedSchemaVersion => {
                "native configuration schema version is unsupported"
            }
        })
    }
}

impl Error for NativeConfigError {}

/// Resolves and synchronously loads native configuration without modifying it.
///
/// A missing file or unavailable configuration location returns the safe built-in
/// configuration. A selected but invalid location, an unsafe file type, an I/O
/// failure, an oversized file, or invalid configuration returns a typed error.
///
/// # Errors
///
/// Returns [`NativeConfigError`] when a selected location is invalid, the file
/// cannot be safely read within its bound, or its contents do not match the
/// supported schema.
pub fn load_native_config(
    environment: &NativeEnvironment,
) -> Result<LoadedNativeConfig, NativeConfigError> {
    match resolve_config_file(environment) {
        ResolvedPath::Path(path) => load_config_path(&path),
        ResolvedPath::Unavailable => Ok(LoadedNativeConfig::built_in_defaults()),
        ResolvedPath::InvalidEnvironment => Err(NativeConfigError::new(
            NativeConfigErrorKind::InvalidEnvironment,
        )),
    }
}

/// Captures `XDG_CONFIG_HOME` from the process environment and captures `HOME`
/// only when that value is missing or empty, then synchronously loads native
/// configuration.
///
/// # Errors
///
/// Returns [`NativeConfigError`] under the same conditions as
/// [`load_native_config`].
pub fn load_process_config() -> Result<LoadedNativeConfig, NativeConfigError> {
    load_process_config_with(std::env::var_os)
}

fn load_process_config_with(
    mut read_environment: impl FnMut(&'static str) -> Option<OsString>,
) -> Result<LoadedNativeConfig, NativeConfigError> {
    let xdg_config_home = read_environment("XDG_CONFIG_HOME");
    let home = match xdg_config_home.as_ref() {
        Some(value) if !value.is_empty() => None,
        Some(_) | None => read_environment("HOME"),
    };
    let environment = NativeEnvironment::new(xdg_config_home, None, home);
    load_native_config(&environment)
}

fn load_config_path(path: &Path) -> Result<LoadedNativeConfig, NativeConfigError> {
    let initial_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LoadedNativeConfig::built_in_defaults());
        }
        Err(_) => return Err(NativeConfigError::new(NativeConfigErrorKind::Unreadable)),
    };
    if !initial_metadata.file_type().is_file() {
        return Err(NativeConfigError::new(
            NativeConfigErrorKind::InvalidFileType,
        ));
    }

    let Some(mut file) = open_config_file(path)? else {
        return Ok(LoadedNativeConfig::built_in_defaults());
    };
    let metadata = file
        .metadata()
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::Unreadable))?;
    if !metadata.file_type().is_file() {
        return Err(NativeConfigError::new(
            NativeConfigErrorKind::InvalidFileType,
        ));
    }
    if metadata.len() > MAX_CONFIG_BYTES as u64 {
        return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
    }

    let bytes = read_bounded(&mut file)?;
    parse_config_bytes(&bytes).map(LoadedNativeConfig::from_file)
}

pub(crate) fn parse_config_bytes(bytes: &[u8]) -> Result<NativeConfig, NativeConfigError> {
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
    }
    std::str::from_utf8(bytes)
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::InvalidFormat))?;
    let schema_version = validate_schema_version(bytes)?;
    let config = match schema_version {
        1 => parse_v1_config(bytes)?,
        2 => parse_v2_config(bytes)?,
        3 => parse_v3_config(bytes)?,
        4 => parse_v4_config(bytes)?,
        5 => parse_v5_config(bytes)?,
        _ => unreachable!("validated schema version is supported"),
    };
    Ok(config)
}

fn validate_schema_version(bytes: &[u8]) -> Result<u32, NativeConfigError> {
    let envelope: WireSchemaEnvelope<'_> = serde_json::from_slice(bytes)
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::InvalidFormat))?;
    let version = envelope.schema_version.get();
    if is_json_integer(version) {
        if let Ok(version) = version.parse::<u32>() {
            match version {
                1 => return Ok(1),
                2 => return Ok(2),
                3 => return Ok(3),
                4 => return Ok(4),
                5 => return Ok(5),
                _ => {}
            }
        }
        return Err(NativeConfigError::new(
            NativeConfigErrorKind::UnsupportedSchemaVersion,
        ));
    }
    Err(NativeConfigError::new(NativeConfigErrorKind::InvalidFormat))
}

fn parse_v1_config(bytes: &[u8]) -> Result<NativeConfig, NativeConfigError> {
    let wire: WireNativeConfigV1 = serde_json::from_slice(bytes)
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::InvalidFormat))?;
    debug_assert_eq!(wire.schema_version, 1);
    if wire.permission_mode != "ask" {
        return Err(NativeConfigError::new(NativeConfigErrorKind::InvalidFormat));
    }
    Ok(NativeConfig {
        schema_version: wire.schema_version,
        permission_mode: PermissionMode::Ask,
        sandbox_mode: NativeSandboxMode::Os,
        permission_rules: NativeConfiguredPermissionRules::default(),
        provider: NativeProviderKind::VercelAiGateway,
        transport: NativeTransportKind::AiGatewayHttp,
        model: AI_GATEWAY_DEFAULT_MODEL.to_owned(),
        credential_source: NativeCredentialSourceKind::Environment,
        effort: NativeReasoningEffort::default(),
        fast_mode: false,
    })
}

fn parse_v2_config(bytes: &[u8]) -> Result<NativeConfig, NativeConfigError> {
    let wire: WireNativeConfigV2 = serde_json::from_slice(bytes)
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::InvalidFormat))?;
    debug_assert_eq!(wire.schema_version, 2);
    if wire.permission_mode != "ask"
        || wire.provider != "vercel_ai_gateway"
        || wire.transport != "ai_gateway_http"
        || !valid_model(&wire.model)
    {
        return Err(NativeConfigError::new(NativeConfigErrorKind::InvalidFormat));
    }
    Ok(NativeConfig {
        schema_version: wire.schema_version,
        permission_mode: PermissionMode::Ask,
        sandbox_mode: NativeSandboxMode::Os,
        permission_rules: NativeConfiguredPermissionRules::default(),
        provider: NativeProviderKind::VercelAiGateway,
        transport: NativeTransportKind::AiGatewayHttp,
        model: wire.model,
        credential_source: NativeCredentialSourceKind::Environment,
        effort: NativeReasoningEffort::default(),
        fast_mode: false,
    })
}

fn parse_v3_config(bytes: &[u8]) -> Result<NativeConfig, NativeConfigError> {
    let wire: WireNativeConfigV3 = serde_json::from_slice(bytes)
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::InvalidFormat))?;
    debug_assert_eq!(wire.schema_version, 3);
    if wire.permission_mode != "ask"
        || wire.provider != "vercel_ai_gateway"
        || wire.transport != "ai_gateway_http"
        || !valid_model(&wire.model)
        || wire.credential_source != "environment"
    {
        return Err(NativeConfigError::new(NativeConfigErrorKind::InvalidFormat));
    }
    Ok(NativeConfig {
        schema_version: wire.schema_version,
        permission_mode: PermissionMode::Ask,
        sandbox_mode: NativeSandboxMode::Os,
        permission_rules: NativeConfiguredPermissionRules::default(),
        provider: NativeProviderKind::VercelAiGateway,
        transport: NativeTransportKind::AiGatewayHttp,
        model: wire.model,
        credential_source: NativeCredentialSourceKind::Environment,
        effort: NativeReasoningEffort::default(),
        fast_mode: false,
    })
}

fn parse_v4_config(bytes: &[u8]) -> Result<NativeConfig, NativeConfigError> {
    let wire: WireNativeConfigV4 = serde_json::from_slice(bytes)
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::InvalidFormat))?;
    if wire.permission_mode != "ask"
        || wire.provider != "vercel_ai_gateway"
        || wire.transport != "ai_gateway_http"
        || !valid_model(&wire.model)
        || wire.credential_source != "environment"
    {
        return Err(NativeConfigError::new(NativeConfigErrorKind::InvalidFormat));
    }
    let effort = NativeReasoningEffort::parse(&wire.effort)
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::InvalidFormat))?;
    Ok(NativeConfig {
        schema_version: wire.schema_version,
        model: wire.model,
        permission_mode: PermissionMode::Ask,
        sandbox_mode: NativeSandboxMode::Os,
        permission_rules: NativeConfiguredPermissionRules::default(),
        provider: NativeProviderKind::VercelAiGateway,
        transport: NativeTransportKind::AiGatewayHttp,
        credential_source: NativeCredentialSourceKind::Environment,
        effort,
        fast_mode: wire.fast_mode,
    })
}

fn parse_v5_config(bytes: &[u8]) -> Result<NativeConfig, NativeConfigError> {
    let invalid = || NativeConfigError::new(NativeConfigErrorKind::InvalidFormat);
    let wire: WireNativeConfigV5 = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let permission_mode = match wire.permission_mode.as_str() {
        "ask" => PermissionMode::Ask,
        "auto" => PermissionMode::Auto,
        "yolo" => PermissionMode::Yolo,
        _ => return Err(invalid()),
    };
    let sandbox_mode = match wire.sandbox_mode.as_str() {
        "os" => NativeSandboxMode::Os,
        "none" => NativeSandboxMode::None,
        _ => return Err(invalid()),
    };
    if wire.provider != "vercel_ai_gateway"
        || wire.transport != "ai_gateway_http"
        || wire.credential_source != "environment"
        || !valid_model(&wire.model)
    {
        return Err(invalid());
    }
    let effort = NativeReasoningEffort::parse(&wire.effort).map_err(|_| invalid())?;
    let rules = wire
        .permission_rules
        .into_iter()
        .map(|rule| {
            let action = match rule.action.as_str() {
                "allow" => NativeConfiguredPermissionDecision::Allow,
                "ask" => NativeConfiguredPermissionDecision::Ask,
                "deny" => NativeConfiguredPermissionDecision::Deny,
                _ => return Err(invalid()),
            };
            NativeConfiguredPermissionRule::new(&rule.permission, &rule.pattern, action)
                .map_err(|_| invalid())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let permission_rules = NativeConfiguredPermissionRules::new(rules).map_err(|_| invalid())?;
    Ok(NativeConfig {
        schema_version: wire.schema_version,
        permission_mode,
        sandbox_mode,
        permission_rules,
        provider: NativeProviderKind::VercelAiGateway,
        transport: NativeTransportKind::AiGatewayHttp,
        model: wire.model,
        credential_source: NativeCredentialSourceKind::Environment,
        effort,
        fast_mode: wire.fast_mode,
    })
}

fn is_json_integer(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn open_config_file(path: &Path) -> Result<Option<File>, NativeConfigError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);

    match options.open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        #[cfg(unix)]
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => Err(NativeConfigError::new(
            NativeConfigErrorKind::InvalidFileType,
        )),
        Err(_) => Err(NativeConfigError::new(NativeConfigErrorKind::Unreadable)),
    }
}

pub(crate) fn read_bounded(file: &mut File) -> Result<Vec<u8>, NativeConfigError> {
    read_bounded_from(file)
}

fn read_bounded_from(reader: &mut impl Read) -> Result<Vec<u8>, NativeConfigError> {
    let mut bytes = vec![0_u8; MAX_CONFIG_BYTES + 1];
    let mut length = 0_usize;
    let mut interrupted_attempts = 0_usize;
    loop {
        let remaining = bytes.len() - length;
        match reader.read(&mut bytes[length..]) {
            Ok(0) => break,
            Ok(read) if read <= remaining => {
                length += read;
                if length > MAX_CONFIG_BYTES {
                    return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                interrupted_attempts += 1;
                if interrupted_attempts >= MAX_CONFIG_INTERRUPTED_READ_ATTEMPTS {
                    return Err(NativeConfigError::new(NativeConfigErrorKind::Unreadable));
                }
            }
            Ok(_) | Err(_) => {
                return Err(NativeConfigError::new(NativeConfigErrorKind::Unreadable));
            }
        }
    }
    bytes.truncate(length);
    Ok(bytes)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNativeConfigV1 {
    schema_version: u32,
    permission_mode: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNativeConfigV2 {
    schema_version: u32,
    permission_mode: String,
    provider: String,
    transport: String,
    model: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNativeConfigV3 {
    schema_version: u32,
    permission_mode: String,
    provider: String,
    transport: String,
    model: String,
    credential_source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNativeConfigV4 {
    schema_version: u32,
    permission_mode: String,
    provider: String,
    transport: String,
    model: String,
    credential_source: String,
    effort: String,
    fast_mode: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNativeConfigV5 {
    schema_version: u32,
    permission_mode: String,
    sandbox_mode: String,
    permission_rules: Vec<WirePermissionRule>,
    provider: String,
    transport: String,
    model: String,
    credential_source: String,
    effort: String,
    fast_mode: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePermissionRule {
    permission: String,
    pattern: String,
    action: String,
}

#[derive(Deserialize)]
struct WireSchemaEnvelope<'a> {
    #[serde(borrow)]
    schema_version: &'a RawValue,
}

#[cfg(test)]
mod tests {
    fn valid_v5() -> serde_json::Value {
        serde_json::json!({
            "schema_version":5, "permission_mode":"ask", "sandbox_mode":"os",
            "permission_rules":[], "provider":"vercel_ai_gateway",
            "transport":"ai_gateway_http", "model":"model",
            "credential_source":"environment", "effort":"auto", "fast_mode":false,
        })
    }

    #[test]
    fn v5_preferences_preserve_order_and_redact_rule_contents() {
        for (spelling, mode) in [
            ("ask", PermissionMode::Ask),
            ("auto", PermissionMode::Auto),
            ("yolo", PermissionMode::Yolo),
        ] {
            for (sandbox, expected) in [
                ("os", NativeSandboxMode::Os),
                ("none", NativeSandboxMode::None),
            ] {
                let mut value = valid_v5();
                value["permission_mode"] = spelling.into();
                value["sandbox_mode"] = sandbox.into();
                value["permission_rules"] = serde_json::json!([
                    {"permission":" write_file ","pattern":" private-path/* ","action":"allow"},
                    {"permission":"write_file","pattern":"private-path/*","action":"deny"},
                    {"permission":"write_file","pattern":"","action":"ask"},
                ]);
                let parsed = parse_config_bytes(&serde_json::to_vec(&value).unwrap()).unwrap();
                assert_eq!(parsed.schema_version(), 5);
                assert_eq!(parsed.permission_mode(), mode);
                assert_eq!(mode.as_str(), spelling);
                assert_eq!(parsed.sandbox_mode(), expected);
                assert_eq!(expected.as_str(), sandbox);
                let rules = parsed.permission_rules().rules();
                assert_eq!(rules.len(), 3);
                assert_eq!(rules[0].permission(), "write_file");
                assert_eq!(rules[0].pattern(), "private-path/*");
                assert_eq!(
                    rules[0].decision(),
                    NativeConfiguredPermissionDecision::Allow
                );
                assert_eq!(
                    rules[1].decision(),
                    NativeConfiguredPermissionDecision::Deny
                );
                assert_eq!(rules[2].decision(), NativeConfiguredPermissionDecision::Ask);
                assert!(!format!("{parsed:?}").contains("private-path"));
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                assert_eq!(
                    parse_config_bytes(&parsed.serialize_current().unwrap()).unwrap(),
                    parsed
                );
            }
        }
        assert_eq!(NativeSandboxMode::default(), NativeSandboxMode::Os);
        assert_eq!(
            NativeConfig::default().sandbox_mode(),
            NativeSandboxMode::Os
        );
        assert!(
            NativeConfig::default()
                .permission_rules()
                .rules()
                .is_empty()
        );
    }

    #[test]
    fn v5_requires_exact_fields_types_and_rule_shapes() {
        let valid = valid_v5();
        let object = valid.as_object().unwrap();
        for (key, field) in object {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert_eq!(
                parse_config_bytes(&serde_json::to_vec(&missing).unwrap())
                    .unwrap_err()
                    .kind(),
                NativeConfigErrorKind::InvalidFormat
            );
            let encoded = serde_json::to_string(&valid).unwrap();
            let duplicate = format!(
                "{{{}:{},{}",
                serde_json::to_string(key).unwrap(),
                field,
                &encoded[1..]
            );
            assert_eq!(
                parse_config_bytes(duplicate.as_bytes()).unwrap_err().kind(),
                NativeConfigErrorKind::InvalidFormat
            );
            let mut null = valid.clone();
            null[key] = serde_json::Value::Null;
            assert_eq!(
                parse_config_bytes(&serde_json::to_vec(&null).unwrap())
                    .unwrap_err()
                    .kind(),
                NativeConfigErrorKind::InvalidFormat
            );
        }
        for (key, wrong) in [
            ("schema_version", serde_json::json!(5.0)),
            ("permission_mode", serde_json::json!("AUTO")),
            ("sandbox_mode", serde_json::json!("disabled")),
            ("permission_rules", serde_json::json!({})),
            (
                "permission_rules",
                serde_json::json!([{"permission":"read","pattern":"*","action":"ALLOW"}]),
            ),
            (
                "permission_rules",
                serde_json::json!([{"permission":"read","action":"ask"}]),
            ),
            (
                "permission_rules",
                serde_json::json!([{"permission":"read","pattern":false,"action":"ask"}]),
            ),
            (
                "permission_rules",
                serde_json::json!([{"permission":" ","pattern":"*","action":"allow"}]),
            ),
            (
                "permission_rules",
                serde_json::json!([{"permission":"read","pattern":"*","action":"allow","extra":0}]),
            ),
            ("extra", serde_json::json!(true)),
        ] {
            let mut value = valid.clone();
            value[key] = wrong;
            assert_eq!(
                parse_config_bytes(&serde_json::to_vec(&value).unwrap())
                    .unwrap_err()
                    .kind(),
                NativeConfigErrorKind::InvalidFormat
            );
        }
        let encoded = serde_json::to_string(&valid).unwrap().replace("\"permission_rules\":[]", "\"permission_rules\":[{\"permission\":\"read\",\"pattern\":\"*\",\"action\":\"ask\",\"action\":\"allow\"}]");
        assert_eq!(
            parse_config_bytes(encoded.as_bytes()).unwrap_err().kind(),
            NativeConfigErrorKind::InvalidFormat
        );
    }

    #[test]
    fn legacy_schemas_keep_ask_only_and_reject_policy_fields() {
        for version in 1..=4 {
            let mut value = valid_v5();
            let object = value.as_object_mut().unwrap();
            object.remove("sandbox_mode");
            object.remove("permission_rules");
            object.insert("schema_version".into(), version.into());
            if version < 4 {
                object.remove("effort");
                object.remove("fast_mode");
            }
            if version < 3 {
                object.remove("credential_source");
            }
            if version < 2 {
                object.remove("model");
                object.remove("provider");
                object.remove("transport");
            }
            let parsed = parse_config_bytes(&serde_json::to_vec(&value).unwrap()).unwrap();
            assert_eq!(parsed.schema_version(), version);
            assert_eq!(parsed.sandbox_mode(), NativeSandboxMode::Os);
            assert!(parsed.permission_rules().rules().is_empty());
            for (key, extra) in [
                ("permission_mode", serde_json::json!("auto")),
                ("permission_mode", serde_json::json!("yolo")),
                ("sandbox_mode", serde_json::json!("os")),
                ("permission_rules", serde_json::json!([])),
            ] {
                let mut invalid = value.clone();
                invalid[key] = extra;
                assert_eq!(
                    parse_config_bytes(&serde_json::to_vec(&invalid).unwrap())
                        .unwrap_err()
                        .kind(),
                    NativeConfigErrorKind::InvalidFormat
                );
            }
        }
    }

    #[test]
    fn strict_v4_requires_all_fields_and_rejects_duplicates_and_wrong_controls() {
        let valid = br#"{"schema_version":4,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model","credential_source":"environment","effort":"high","fast_mode":true}"#;
        let parsed = super::parse_config_bytes(valid).unwrap();
        assert_eq!(parsed.schema_version(), 4);
        assert_eq!(parsed.effort().label(), "high");
        assert!(parsed.fast_mode());
        let value: serde_json::Value = serde_json::from_slice(valid).unwrap();
        for field in [
            "schema_version",
            "permission_mode",
            "provider",
            "transport",
            "model",
            "credential_source",
            "effort",
            "fast_mode",
        ] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(super::parse_config_bytes(&serde_json::to_vec(&missing).unwrap()).is_err());
        }
        for (field, replacement) in [
            ("effort", serde_json::json!(false)),
            ("fast_mode", serde_json::json!("true")),
            ("extra", serde_json::json!(true)),
        ] {
            let mut invalid = value.clone();
            invalid[field] = replacement;
            assert!(super::parse_config_bytes(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let duplicate = String::from_utf8(valid.to_vec()).unwrap().replace(
            "\"effort\":\"high\"",
            "\"effort\":\"high\",\"effort\":\"low\"",
        );
        assert!(super::parse_config_bytes(duplicate.as_bytes()).is_err());
    }

    use super::{
        CONFIG_SCHEMA_VERSION, ConfigOrigin, MAX_CONFIG_BYTES,
        MAX_CONFIG_INTERRUPTED_READ_ATTEMPTS, NativeConfig, NativeConfigErrorKind,
        NativeCredentialSourceKind, NativeProviderKind, NativeTransportKind, load_native_config,
        load_process_config_with, parse_config_bytes, read_bounded_from,
    };
    use crate::ai_gateway::valid_model;
    use crate::{
        AI_GATEWAY_DEFAULT_MODEL, AI_GATEWAY_MAX_MODEL_BYTES, NativeConfiguredPermissionDecision,
        NativeEnvironment, NativeSandboxMode, PermissionMode,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::io::{self, Read};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone, Copy, Debug)]
    enum ReadStep {
        Bytes(&'static [u8]),
        Interrupted,
        Overreported,
        End,
    }

    #[derive(Debug)]
    struct ScriptedReader {
        steps: Vec<ReadStep>,
        next: usize,
    }

    impl ScriptedReader {
        fn new(steps: Vec<ReadStep>) -> Self {
            Self { steps, next: 0 }
        }

        fn calls(&self) -> usize {
            self.next
        }
    }

    impl Read for ScriptedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let step = self
                .steps
                .get(self.next)
                .copied()
                .expect("scripted reader was called beyond its bounded fixture");
            self.next += 1;
            match step {
                ReadStep::Bytes(bytes) => {
                    assert!(bytes.len() <= buffer.len());
                    buffer[..bytes.len()].copy_from_slice(bytes);
                    Ok(bytes.len())
                }
                ReadStep::Interrupted => Err(io::Error::from(io::ErrorKind::Interrupted)),
                ReadStep::Overreported => Ok(buffer.len() + 1),
                ReadStep::End => Ok(0),
            }
        }
    }

    #[derive(Debug)]
    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(test_name: &str) -> Self {
            let base = std::env::temp_dir().join("machine-god-native-config-tests");
            fs::create_dir_all(&base).expect("failed to create config test base directory");
            loop {
                let id = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = base.join(format!("{}-{test_name}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("failed to create test directory: {error}"),
                }
            }
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn environment(&self) -> NativeEnvironment {
            NativeEnvironment::new(Some(self.0.as_os_str().to_owned()), None, None)
        }

        fn config_path(&self) -> PathBuf {
            self.0.join("machine-god/config.json")
        }

        fn write_config(&self, bytes: &[u8]) {
            let path = self.config_path();
            fs::create_dir_all(path.parent().expect("config path has parent"))
                .expect("failed to create config parent");
            fs::write(path, bytes).expect("failed to write config fixture");
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("failed to remove config test directory: {error}");
            }
        }
    }

    fn valid_v2_document(model: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "permission_mode": "ask",
            "provider": "vercel_ai_gateway",
            "transport": "ai_gateway_http",
            "model": model,
        }))
        .unwrap()
    }

    fn valid_v3_document(model: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 3,
            "permission_mode": "ask",
            "provider": "vercel_ai_gateway",
            "transport": "ai_gateway_http",
            "model": model,
            "credential_source": "environment",
        }))
        .unwrap()
    }

    fn assert_config(config: &NativeConfig, schema_version: u32, model: &str) {
        assert_eq!(config.schema_version(), schema_version);
        assert_eq!(config.permission_mode(), PermissionMode::Ask);
        assert_eq!(config.provider(), NativeProviderKind::VercelAiGateway);
        assert_eq!(config.provider().as_str(), "vercel_ai_gateway");
        assert_eq!(config.transport(), NativeTransportKind::AiGatewayHttp);
        assert_eq!(config.transport().as_str(), "ai_gateway_http");
        assert_eq!(config.model(), model);
        assert_eq!(
            config.credential_source(),
            NativeCredentialSourceKind::Environment
        );
        assert_eq!(config.credential_source().as_str(), "environment");
    }

    #[test]
    fn process_config_snapshot_requests_home_after_missing_xdg_config_home() {
        let mut requested = Vec::new();
        let loaded = load_process_config_with(|key| {
            requested.push(key);
            match key {
                "XDG_CONFIG_HOME" | "HOME" => None,
                _ => panic!("unexpected environment request: {key}"),
            }
        })
        .unwrap();

        assert_eq!(requested, ["XDG_CONFIG_HOME", "HOME"]);
        assert_eq!(loaded.origin(), ConfigOrigin::BuiltInDefaults);
    }

    #[test]
    fn process_config_snapshot_requests_home_after_empty_xdg_config_home() {
        let mut requested = Vec::new();
        let loaded = load_process_config_with(|key| {
            requested.push(key);
            match key {
                "XDG_CONFIG_HOME" => Some(OsString::new()),
                "HOME" => None,
                _ => panic!("unexpected environment request: {key}"),
            }
        })
        .unwrap();

        assert_eq!(requested, ["XDG_CONFIG_HOME", "HOME"]);
        assert_eq!(loaded.origin(), ConfigOrigin::BuiltInDefaults);
    }

    #[test]
    fn process_config_snapshot_does_not_request_home_after_valid_xdg_config_home() {
        let temporary = TestDirectory::new("process-valid-xdg");
        let xdg_config_home = temporary.path().as_os_str().to_owned();
        let mut requested = Vec::new();
        let loaded = load_process_config_with(|key| {
            requested.push(key);
            match key {
                "XDG_CONFIG_HOME" => Some(xdg_config_home.clone()),
                _ => panic!("unexpected environment request: {key}"),
            }
        })
        .unwrap();

        assert_eq!(requested, ["XDG_CONFIG_HOME"]);
        assert_eq!(loaded.origin(), ConfigOrigin::BuiltInDefaults);
    }

    #[test]
    fn process_config_snapshot_does_not_request_home_after_relative_xdg_config_home() {
        let mut requested = Vec::new();
        let error = load_process_config_with(|key| {
            requested.push(key);
            match key {
                "XDG_CONFIG_HOME" => Some(OsString::from("relative")),
                _ => panic!("unexpected environment request: {key}"),
            }
        })
        .unwrap_err();

        assert_eq!(requested, ["XDG_CONFIG_HOME"]);
        assert_eq!(error.kind(), NativeConfigErrorKind::InvalidEnvironment);
    }

    #[cfg(unix)]
    #[test]
    fn process_config_snapshot_does_not_request_home_after_non_unicode_xdg_config_home() {
        use std::os::unix::ffi::OsStringExt;

        let non_unicode = OsString::from_vec(vec![b'/', 0xff]);
        let mut requested = Vec::new();
        let error = load_process_config_with(|key| {
            requested.push(key);
            match key {
                "XDG_CONFIG_HOME" => Some(non_unicode.clone()),
                _ => panic!("unexpected environment request: {key}"),
            }
        })
        .unwrap_err();

        assert_eq!(requested, ["XDG_CONFIG_HOME"]);
        assert_eq!(error.kind(), NativeConfigErrorKind::InvalidEnvironment);
    }

    #[cfg(windows)]
    #[test]
    fn process_config_snapshot_does_not_request_home_after_non_unicode_xdg_config_home() {
        use std::os::windows::ffi::OsStringExt;

        let non_unicode = OsString::from_wide(&[0xd800]);
        let mut requested = Vec::new();
        let error = load_process_config_with(|key| {
            requested.push(key);
            match key {
                "XDG_CONFIG_HOME" => Some(non_unicode.clone()),
                _ => panic!("unexpected environment request: {key}"),
            }
        })
        .unwrap_err();

        assert_eq!(requested, ["XDG_CONFIG_HOME"]);
        assert_eq!(error.kind(), NativeConfigErrorKind::InvalidEnvironment);
    }

    #[test]
    fn bounded_read_succeeds_before_the_sixteenth_interruption() {
        let mut steps = vec![ReadStep::Interrupted; MAX_CONFIG_INTERRUPTED_READ_ATTEMPTS - 1];
        steps.extend([ReadStep::Bytes(b"config"), ReadStep::End]);
        let mut reader = ScriptedReader::new(steps);

        assert_eq!(read_bounded_from(&mut reader).unwrap(), b"config");
        assert_eq!(reader.calls(), MAX_CONFIG_INTERRUPTED_READ_ATTEMPTS + 1);
    }

    #[test]
    fn bounded_read_maps_the_sixteenth_interruption_to_unreadable() {
        let mut reader = ScriptedReader::new(vec![
            ReadStep::Interrupted;
            MAX_CONFIG_INTERRUPTED_READ_ATTEMPTS
        ]);

        let error = read_bounded_from(&mut reader).unwrap_err();

        assert_eq!(error.kind(), NativeConfigErrorKind::Unreadable);
        assert_eq!(reader.calls(), MAX_CONFIG_INTERRUPTED_READ_ATTEMPTS);
    }

    #[test]
    fn bounded_read_counts_interleaved_interruptions_cumulatively() {
        let first_interruptions = MAX_CONFIG_INTERRUPTED_READ_ATTEMPTS / 2;
        let mut steps = vec![ReadStep::Interrupted; first_interruptions];
        steps.push(ReadStep::Bytes(b"partial"));
        steps.extend(vec![
            ReadStep::Interrupted;
            MAX_CONFIG_INTERRUPTED_READ_ATTEMPTS
                - first_interruptions
        ]);
        let mut reader = ScriptedReader::new(steps);

        let error = read_bounded_from(&mut reader).unwrap_err();

        assert_eq!(error.kind(), NativeConfigErrorKind::Unreadable);
        assert_eq!(reader.calls(), MAX_CONFIG_INTERRUPTED_READ_ATTEMPTS + 1);
    }

    #[test]
    fn bounded_read_maps_overreported_progress_to_unreadable() {
        let mut reader = ScriptedReader::new(vec![ReadStep::Overreported]);

        let error = read_bounded_from(&mut reader).unwrap_err();

        assert_eq!(error.kind(), NativeConfigErrorKind::Unreadable);
        assert_eq!(reader.calls(), 1);
    }

    #[test]
    fn unavailable_and_missing_locations_use_safe_defaults() {
        let unavailable = load_native_config(&NativeEnvironment::new(None, None, None)).unwrap();
        assert_eq!(unavailable.origin(), ConfigOrigin::BuiltInDefaults);
        assert_config(
            unavailable.config(),
            CONFIG_SCHEMA_VERSION,
            AI_GATEWAY_DEFAULT_MODEL,
        );
        assert_eq!(unavailable.config(), &NativeConfig::default());

        let temporary = TestDirectory::new("missing");
        let missing = load_native_config(&temporary.environment()).unwrap();
        assert_eq!(missing.origin(), ConfigOrigin::BuiltInDefaults);
        assert_config(
            missing.config(),
            CONFIG_SCHEMA_VERSION,
            AI_GATEWAY_DEFAULT_MODEL,
        );
        assert!(!temporary.config_path().exists());
    }

    #[test]
    fn exact_v1_schema_loads_with_compatible_defaults_and_retains_its_version() {
        let temporary = TestDirectory::new("valid-v1");
        temporary.write_config(br#"{"schema_version":1,"permission_mode":"ask"}"#);

        let loaded = load_native_config(&temporary.environment()).unwrap();
        assert_eq!(loaded.origin(), ConfigOrigin::File);
        assert_config(loaded.config(), 1, AI_GATEWAY_DEFAULT_MODEL);
    }

    #[test]
    fn exact_v2_schema_loads_all_selected_fields() {
        let temporary = TestDirectory::new("valid-v2");
        temporary.write_config(
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"custom/model"}"#,
        );

        let loaded = load_native_config(&temporary.environment()).unwrap();
        assert_eq!(loaded.origin(), ConfigOrigin::File);
        assert_config(loaded.config(), 2, "custom/model");
    }

    #[test]
    fn exact_v3_schema_loads_all_selected_fields() {
        let temporary = TestDirectory::new("valid-v3");
        temporary.write_config(
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"custom/model","credential_source":"environment"}"#,
        );

        let loaded = load_native_config(&temporary.environment()).unwrap();
        assert_eq!(CONFIG_SCHEMA_VERSION, 5);
        assert_eq!(loaded.origin(), ConfigOrigin::File);
        assert_config(loaded.config(), 3, "custom/model");
    }

    #[test]
    fn config_debug_redacts_the_model_but_reports_structure() {
        let temporary = TestDirectory::new("debug-redaction");
        temporary.write_config(&valid_v3_document("private-model-marker"));

        let loaded = load_native_config(&temporary.environment()).unwrap();
        let config_debug = format!("{:?}", loaded.config());
        let loaded_debug = format!("{loaded:?}");
        for diagnostic in [&config_debug, &loaded_debug] {
            assert!(!diagnostic.contains("private-model-marker"));
            assert!(diagnostic.contains("<redacted>"));
            assert!(diagnostic.contains("VercelAiGateway"));
            assert!(diagnostic.contains("AiGatewayHttp"));
        }
    }

    #[test]
    fn strict_v1_schema_rejects_invalid_json_shapes() {
        let invalid_documents: &[&[u8]] = &[
            br"{}",
            br#"{"schema_version":1}"#,
            br#"{"permission_mode":"ask"}"#,
            br#"{"schema_version":1,"permission_mode":"ask","extra":true}"#,
            br#"{"schema_version":1,"schema_version":1,"permission_mode":"ask"}"#,
            br#"{"schema_version":"1","permission_mode":"ask"}"#,
            br#"{"schema_version":1,"permission_mode":"deny"}"#,
            br#"{"schema_version":1,"permission_mode":"ask","credential_source":"environment"}"#,
            br#"{"schema_version":1,"permission_mode":"ask"} trailing"#,
            b"{\"schema_version\":1,\"permission_mode\":\"ask\xff\"}",
        ];

        for (index, document) in invalid_documents.iter().enumerate() {
            let temporary = TestDirectory::new(&format!("invalid-{index}"));
            temporary.write_config(document);
            let error = load_native_config(&temporary.environment()).unwrap_err();
            assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFormat);
        }
    }

    #[test]
    fn strict_v2_schema_rejects_unknown_duplicate_missing_and_wrong_fields() {
        let invalid_documents: &[&[u8]] = &[
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http"}"#,
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model","extra":true}"#,
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model"}"#,
            br#"{"schema_version":2,"permission_mode":"ask","provider":"other","transport":"ai_gateway_http","model":"model"}"#,
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"other","model":"model"}"#,
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":1}"#,
            br#"{"schema_version":2,"permission_mode":"ask"}"#,
            br#"{"schema_version":1,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model"}"#,
            br#"{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model","credential_source":"environment"}"#,
        ];

        for (index, document) in invalid_documents.iter().enumerate() {
            let temporary = TestDirectory::new(&format!("invalid-v2-{index}"));
            temporary.write_config(document);
            let error = load_native_config(&temporary.environment()).unwrap_err();
            assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFormat);
        }
    }

    #[test]
    fn strict_v3_schema_requires_exact_credential_source_and_shape() {
        let invalid_documents: &[&[u8]] = &[
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model"}"#,
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model","credential_source":"other"}"#,
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model","credential_source":true}"#,
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model","credential_source":"environment","extra":true}"#,
            br#"{"schema_version":3,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model","credential_source":"environment","credential_source":"environment"}"#,
        ];

        for (index, document) in invalid_documents.iter().enumerate() {
            let temporary = TestDirectory::new(&format!("invalid-v3-{index}"));
            temporary.write_config(document);
            let error = load_native_config(&temporary.environment()).unwrap_err();
            assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFormat);
        }
    }

    #[test]
    fn unsupported_schema_version_has_its_own_kind() {
        let temporary = TestDirectory::new("unsupported-version");
        temporary.write_config(br#"{"schema_version":6,"permission_mode":"ask"}"#);

        let error = load_native_config(&temporary.environment()).unwrap_err();
        assert_eq!(
            error.kind(),
            NativeConfigErrorKind::UnsupportedSchemaVersion
        );
    }

    #[test]
    fn future_and_arbitrary_size_integer_versions_are_classified_before_v1_fields() {
        for (index, document) in [
            br#"{"schema_version":6,"permission_mode":"future","new_field":true}"#.as_slice(),
            br#"{"schema_version":18446744073709551616}"#.as_slice(),
            br#"{"schema_version":-1,"future_shape":[]}"#.as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            let temporary = TestDirectory::new(&format!("future-version-{index}"));
            temporary.write_config(document);
            assert_eq!(
                load_native_config(&temporary.environment())
                    .unwrap_err()
                    .kind(),
                NativeConfigErrorKind::UnsupportedSchemaVersion
            );
        }
    }

    #[test]
    fn invalid_utf8_in_an_ignored_future_field_is_still_invalid_format() {
        let temporary = TestDirectory::new("future-invalid-utf8");
        temporary.write_config(b"{\"schema_version\":4,\"future\":\"\xff\"}");

        assert_eq!(
            load_native_config(&temporary.environment())
                .unwrap_err()
                .kind(),
            NativeConfigErrorKind::InvalidFormat
        );
    }

    #[test]
    fn supported_noninteger_schema_versions_are_invalid_format_before_dispatch() {
        for (index, document) in [
            br#"{"schema_version":"2","permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model"}"#.as_slice(),
            br#"{"schema_version":2.0,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model"}"#.as_slice(),
            br#"{"schema_version":true,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"model"}"#.as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            let temporary = TestDirectory::new(&format!("noninteger-version-{index}"));
            temporary.write_config(document);
            assert_eq!(
                load_native_config(&temporary.environment())
                    .unwrap_err()
                    .kind(),
                NativeConfigErrorKind::InvalidFormat
            );
        }
    }

    #[test]
    fn config_model_validation_matches_the_gateway_validator_and_exact_bound() {
        let exactly_maximum = "!".repeat(AI_GATEWAY_MAX_MODEL_BYTES);
        let oversized = "!".repeat(AI_GATEWAY_MAX_MODEL_BYTES + 1);
        let candidates = [
            AI_GATEWAY_DEFAULT_MODEL.to_owned(),
            "!".to_owned(),
            exactly_maximum,
            "é".repeat(AI_GATEWAY_MAX_MODEL_BYTES / 2),
            "\u{85}modèle\u{a0}".to_owned(),
            String::new(),
            oversized,
            " leading space".to_owned(),
            "trailing space ".to_owned(),
            "contains space".to_owned(),
            "contains\nnewline".to_owned(),
            "contains\u{7f}delete".to_owned(),
            "non-ascii-é".to_owned(),
        ];

        for (index, model) in candidates.into_iter().enumerate() {
            let temporary = TestDirectory::new(&format!("model-{index}"));
            for document in [valid_v2_document(&model), valid_v3_document(&model)] {
                temporary.write_config(&document);
                let result = load_native_config(&temporary.environment());
                assert_eq!(result.is_ok(), valid_model(&model), "model index {index}");
                match result {
                    Ok(loaded) => assert_eq!(loaded.config().model(), model),
                    Err(error) => {
                        assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFormat);
                        assert!(!format!("{error:?} {error}").contains(&model) || model.is_empty());
                    }
                }
            }
        }
    }

    #[test]
    fn retained_read_is_bounded_at_limit_plus_one() {
        let valid = valid_v3_document(AI_GATEWAY_DEFAULT_MODEL);
        let temporary = TestDirectory::new("exact-limit");
        let mut exact_limit = Vec::with_capacity(MAX_CONFIG_BYTES);
        exact_limit.extend_from_slice(&valid);
        exact_limit.resize(MAX_CONFIG_BYTES, b' ');
        temporary.write_config(&exact_limit);
        assert_eq!(
            load_native_config(&temporary.environment())
                .unwrap()
                .origin(),
            ConfigOrigin::File
        );

        exact_limit.push(b' ');
        temporary.write_config(&exact_limit);
        let error = load_native_config(&temporary.environment()).unwrap_err();
        assert_eq!(error.kind(), NativeConfigErrorKind::TooLarge);
    }

    #[test]
    fn invalid_environment_and_file_type_are_typed() {
        let invalid_environment = NativeEnvironment::new(
            Some(OsString::from("relative")),
            None,
            Some(OsString::from("/unused")),
        );
        assert_eq!(
            load_native_config(&invalid_environment).unwrap_err().kind(),
            NativeConfigErrorKind::InvalidEnvironment
        );

        let temporary = TestDirectory::new("directory");
        fs::create_dir_all(temporary.config_path()).unwrap();
        assert_eq!(
            load_native_config(&temporary.environment())
                .unwrap_err()
                .kind(),
            NativeConfigErrorKind::InvalidFileType
        );
    }

    #[cfg(unix)]
    #[test]
    fn final_symlink_is_rejected_and_errors_are_redacted() {
        use std::os::unix::fs::symlink;

        let temporary = TestDirectory::new("secret-path");
        let secret_target = temporary.path().join("secret-content");
        fs::write(
            &secret_target,
            br#"{"schema_version":1,"permission_mode":"ask"}"#,
        )
        .unwrap();
        let config_path = temporary.config_path();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        symlink(&secret_target, config_path).unwrap();

        let error = load_native_config(&temporary.environment()).unwrap_err();
        assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFileType);
        assert!(!format!("{error:?}").contains("secret"));
        assert!(!error.to_string().contains("secret"));
    }
}
