use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use serde::Deserialize;
use serde_json::value::RawValue;

use super::ai_gateway::{AI_GATEWAY_DEFAULT_MODEL, valid_model};
use super::{NativeEnvironment, PermissionMode, ResolvedPath, resolve_config_file};

/// Current configuration schema version used by this native host.
pub const CONFIG_SCHEMA_VERSION: u32 = 3;

/// Maximum number of bytes retained while loading a native configuration.
pub const MAX_CONFIG_BYTES: usize = 64 * 1024;

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
    provider: NativeProviderKind,
    transport: NativeTransportKind,
    model: String,
    credential_source: NativeCredentialSourceKind,
}

impl NativeConfig {
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
            provider: NativeProviderKind::VercelAiGateway,
            transport: NativeTransportKind::AiGatewayHttp,
            model: AI_GATEWAY_DEFAULT_MODEL.to_owned(),
            credential_source: NativeCredentialSourceKind::Environment,
        }
    }
}

impl fmt::Debug for NativeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeConfig")
            .field("schema_version", &self.schema_version)
            .field("permission_mode", &self.permission_mode)
            .field("provider", &self.provider)
            .field("transport", &self.transport)
            .field("model", &"<redacted>")
            .field("credential_source", &self.credential_source)
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

    fn built_in_defaults() -> Self {
        Self {
            config: NativeConfig::default(),
            origin: ConfigOrigin::BuiltInDefaults,
        }
    }

    fn from_file(config: NativeConfig) -> Self {
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

/// Captures the process environment and synchronously loads native configuration.
///
/// # Errors
///
/// Returns [`NativeConfigError`] under the same conditions as
/// [`load_native_config`].
pub fn load_process_config() -> Result<LoadedNativeConfig, NativeConfigError> {
    load_native_config(&NativeEnvironment::from_process())
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
    std::str::from_utf8(&bytes)
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::InvalidFormat))?;
    let schema_version = validate_schema_version(&bytes)?;
    let config = match schema_version {
        1 => parse_v1_config(&bytes)?,
        2 => parse_v2_config(&bytes)?,
        3 => parse_v3_config(&bytes)?,
        _ => unreachable!("validated schema version is supported"),
    };
    Ok(LoadedNativeConfig::from_file(config))
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
        provider: NativeProviderKind::VercelAiGateway,
        transport: NativeTransportKind::AiGatewayHttp,
        model: AI_GATEWAY_DEFAULT_MODEL.to_owned(),
        credential_source: NativeCredentialSourceKind::Environment,
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
        provider: NativeProviderKind::VercelAiGateway,
        transport: NativeTransportKind::AiGatewayHttp,
        model: wire.model,
        credential_source: NativeCredentialSourceKind::Environment,
    })
}

fn parse_v3_config(bytes: &[u8]) -> Result<NativeConfig, NativeConfigError> {
    let wire: WireNativeConfigV3 = serde_json::from_slice(bytes)
        .map_err(|_| NativeConfigError::new(NativeConfigErrorKind::InvalidFormat))?;
    debug_assert_eq!(wire.schema_version, CONFIG_SCHEMA_VERSION);
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
        provider: NativeProviderKind::VercelAiGateway,
        transport: NativeTransportKind::AiGatewayHttp,
        model: wire.model,
        credential_source: NativeCredentialSourceKind::Environment,
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

fn read_bounded(file: &mut File) -> Result<Vec<u8>, NativeConfigError> {
    let mut bytes = vec![0_u8; MAX_CONFIG_BYTES + 1];
    let mut length = 0;
    loop {
        match file.read(&mut bytes[length..]) {
            Ok(0) => break,
            Ok(read) => {
                length += read;
                if length > MAX_CONFIG_BYTES {
                    return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(NativeConfigError::new(NativeConfigErrorKind::Unreadable)),
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
struct WireSchemaEnvelope<'a> {
    #[serde(borrow)]
    schema_version: &'a RawValue,
}

#[cfg(test)]
mod tests {
    use super::{
        CONFIG_SCHEMA_VERSION, ConfigOrigin, MAX_CONFIG_BYTES, NativeConfig, NativeConfigErrorKind,
        NativeCredentialSourceKind, NativeProviderKind, NativeTransportKind, load_native_config,
    };
    use crate::ai_gateway::valid_model;
    use crate::{
        AI_GATEWAY_DEFAULT_MODEL, AI_GATEWAY_MAX_MODEL_BYTES, NativeEnvironment, PermissionMode,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

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
        assert_eq!(CONFIG_SCHEMA_VERSION, 3);
        assert_eq!(loaded.origin(), ConfigOrigin::File);
        assert_config(loaded.config(), CONFIG_SCHEMA_VERSION, "custom/model");
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
        temporary.write_config(br#"{"schema_version":4,"permission_mode":"ask"}"#);

        let error = load_native_config(&temporary.environment()).unwrap_err();
        assert_eq!(
            error.kind(),
            NativeConfigErrorKind::UnsupportedSchemaVersion
        );
    }

    #[test]
    fn future_and_arbitrary_size_integer_versions_are_classified_before_v1_fields() {
        for (index, document) in [
            br#"{"schema_version":4,"permission_mode":"future","new_field":true}"#.as_slice(),
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
            String::new(),
            oversized,
            "contains space".to_owned(),
            "contains\nnewline".to_owned(),
            "contains\u{7f}delete".to_owned(),
            "non-ascii-é".to_owned(),
        ];

        for (index, model) in candidates.into_iter().enumerate() {
            let temporary = TestDirectory::new(&format!("model-{index}"));
            temporary.write_config(&valid_v2_document(&model));
            let result = load_native_config(&temporary.environment());
            assert_eq!(result.is_ok(), valid_model(&model), "model index {index}");
            match result {
                Ok(loaded) => assert_eq!(loaded.config().model(), model),
                Err(error) => {
                    assert_eq!(error.kind(), NativeConfigErrorKind::InvalidFormat);
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
