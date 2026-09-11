//! Bounded, effect-free profile MCP configuration.
//!
//! The wire grammar follows the pinned fx profile `mcp` object, including local/
//! stdio command vectors, environment aliases and HTTP/SSE OAuth settings.
//! Unlike the producer, unknown and inactive transport fields, invalid environment
//! names, control characters in active values, ambiguous headers and duplicate
//! JSON keys are errors. As in the producer, `environment` takes precedence over
//! `env`, and a command vector takes precedence over `args`; ignored alias values
//! count toward JSON budgets but are not semantically interpreted.
//! Server names use the producer command grammar with a 128-byte native bound.
//! URL strings retain exact bytes with bounded structural checks; HTTP requires
//! an explicit port and exactly `localhost`, `127.0.0.1` or `[::1]`. Complete URI parsing,
//! origin and DNS admission belong to the effect-bearing HTTP authority, including
//! OAuth endpoints; this codec makes no network-policy claim. No environment
//! references are resolved here.
//!
//! Input and canonical output are each limited to 1 MiB, with 64 servers,
//! JSON depth 8, 16,384 nodes and 512 KiB of decoded keys/string values. Individual
//! strings are at most 16 KiB; command/URL strings 4096 bytes, argument and
//! environment lists 256 entries, combined headers 128, and OAuth scopes 64.
//! Validated values retain compact boxed strings/slices. Serialization uses a
//! bounded writer; mutations preflight the entire candidate before publication.

use std::collections::BTreeMap;
use std::fmt;

use serde::ser::{Serialize, SerializeMap, Serializer};

mod json;
mod parse;
#[cfg(test)]
mod tests;

/// Maximum encoded profile size, including canonical serialization.
pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
/// Maximum number of configured servers (enabled or disabled).
pub const MAX_SERVERS: usize = 64;
/// Maximum decoded key and string-value bytes in one configuration.
pub const MAX_STRING_BYTES: usize = 512 * 1024;
/// Maximum server alias length in bytes.
pub const MAX_SERVER_NAME_BYTES: usize = 128;

/// Redacted configuration failure: never contains configuration bytes or names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpConfigError {
    /// Malformed JSON, duplicate keys, invalid fields or invalid field values.
    Invalid,
    /// A finite input, output, tree or retained-data bound was exceeded.
    Limit,
    /// Insertion would overwrite an existing server; use explicit replacement.
    AlreadyExists,
}

impl fmt::Display for McpConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid MCP configuration",
            Self::Limit => "MCP configuration limit exceeded",
            Self::AlreadyExists => "MCP server already exists",
        })
    }
}
impl std::error::Error for McpConfigError {}

/// Validated configuration, preserving producer server insertion order.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct McpConfig {
    servers: Vec<McpServerConfig>,
}

/// Validated profile server; decoding grants no execution or credential authority.
#[derive(Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    name: Box<str>,
    enabled: bool,
    required: bool,
    startup_timeout_ms: u32,
    operation_timeout_ms: u32,
    environment: BTreeMap<Box<str>, Box<str>>,
    transport: McpTransportConfig,
}

/// Transport-specific validated data, without a live transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpTransportConfig {
    /// Explicit process command and argument vector (never shell syntax).
    Stdio(McpStdioConfig),
    /// Streamable HTTP endpoint.
    Http(McpRemoteConfig),
    /// Deprecated HTTP+SSE endpoint.
    Sse(McpRemoteConfig),
}

/// Validated stdio process configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct McpStdioConfig {
    command: Box<str>,
    args: Box<[Box<str>]>,
    restart_limit: u8,
}

/// Validated remote transport configuration. Secret-bearing values are redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct McpRemoteConfig {
    url: Box<str>,
    headers: BTreeMap<Box<str>, Box<str>>,
    header_env: BTreeMap<Box<str>, Box<str>>,
    bearer_token_env: Option<Box<str>>,
    oauth: Option<McpOAuthConfig>,
}

/// Explicit OAuth inputs. Environment references are names, not resolved secrets.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub struct McpOAuthConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    resource: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    issuer: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret_env: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_metadata_url: Option<Box<str>>,
    scopes: Box<[Box<str>]>,
}

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($ty), " { <redacted> }"))
            }
        }
    )+};
}
redacted_debug!(
    McpConfig,
    McpServerConfig,
    McpStdioConfig,
    McpRemoteConfig,
    McpOAuthConfig
);

impl McpConfig {
    /// Constructs an empty configuration without effects.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes a bounded profile object. An absent `mcp` means no servers.
    ///
    /// # Errors
    /// Rejects malformed, duplicate, unknown, invalid or over-budget input.
    pub fn decode(bytes: &[u8]) -> Result<Self, McpConfigError> {
        let (value, order) = json::decode_ordered(bytes)?;
        let root = parse::object(&value)?;
        parse::fields(root, &["mcp"])?;
        let mut servers = Vec::new();
        if let Some(value) = root.get("mcp") {
            let entries = parse::object(value)?;
            if entries.len() > MAX_SERVERS {
                return Err(McpConfigError::Limit);
            }
            for name in order {
                let value = entries.get(&name).ok_or(McpConfigError::Invalid)?;
                servers.push(parse::server(&name, value)?);
            }
        }
        let config = Self { servers };
        json::decode(&config.encode()?)?;
        Ok(config)
    }

    /// Returns canonical compact JSON, including defaults and normalized aliases.
    ///
    /// # Errors
    /// Returns `Limit` if encoded or aggregate limits would be exceeded.
    pub fn encode(&self) -> Result<Vec<u8>, McpConfigError> {
        json::encode(&ConfigView(self.servers.iter().collect()))
    }

    /// Returns immutable server configurations in preserved insertion order.
    #[must_use]
    pub fn servers(&self) -> &[McpServerConfig] {
        &self.servers
    }

    /// Finds an exact case-sensitive alias; no fallback or normalization occurs.
    #[must_use]
    pub fn server(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.iter().find(|s| s.name() == name)
    }

    /// Inserts a validated server without replacing existing state.
    ///
    /// # Errors
    /// Duplicate aliases or aggregate bounds leave this configuration unchanged.
    pub fn insert(&mut self, server: McpServerConfig) -> Result<(), McpConfigError> {
        if self.server(server.name()).is_some() {
            return Err(McpConfigError::AlreadyExists);
        }
        self.replace(server)?;
        Ok(())
    }

    /// Inserts or explicitly replaces one server, returning the previous value.
    /// All aggregate checks happen before mutation, including on replacement.
    ///
    /// # Errors
    /// Aggregate limits leave the previous configuration unchanged.
    pub fn replace(
        &mut self,
        server: McpServerConfig,
    ) -> Result<Option<McpServerConfig>, McpConfigError> {
        let position = self.servers.iter().position(|s| s.name() == server.name());
        if position.is_none() && self.servers.len() == MAX_SERVERS {
            return Err(McpConfigError::Limit);
        }
        let mut candidate: Vec<_> = self.servers.iter().collect();
        if let Some(index) = position {
            candidate[index] = &server;
        } else {
            candidate.push(&server);
        }
        validate_view(&ConfigView(candidate))?;
        Ok(if let Some(index) = position {
            Some(std::mem::replace(&mut self.servers[index], server))
        } else {
            self.servers.push(server);
            None
        })
    }

    /// Removes an exact alias, returning its previous validated value.
    pub fn remove(&mut self, name: &str) -> Option<McpServerConfig> {
        self.servers
            .iter()
            .position(|s| s.name() == name)
            .map(|i| self.servers.remove(i))
    }
}

impl McpServerConfig {
    /// Decodes one server object under an explicit alias.
    ///
    /// # Errors
    /// Invalid alias, server fields or resource bounds are rejected.
    pub fn decode(name: &str, bytes: &[u8]) -> Result<Self, McpConfigError> {
        parse::name(name)?;
        let value = json::decode(bytes)?;
        let server = parse::server(name, &value)?;
        validate_view(&ConfigView(vec![&server]))?;
        Ok(server)
    }

    /// Creates a default stdio server from an explicit executable and arguments.
    /// Empty arguments are preserved; no splitting or interpolation occurs.
    ///
    /// # Errors
    /// Rejects invalid/control-bearing strings and per-field or aggregate limits.
    pub fn stdio(name: &str, command: &str, args: &[&str]) -> Result<Self, McpConfigError> {
        parse::name(name)?;
        parse::text(command, 4096, false)?;
        if args.len() > 256 {
            return Err(McpConfigError::Limit);
        }
        let mut total = name.len() + command.len();
        for arg in args {
            parse::text(arg, 16 * 1024, true)?;
            total += arg.len();
            if total > MAX_STRING_BYTES {
                return Err(McpConfigError::Limit);
            }
        }
        let server = Self {
            name: name.into(),
            enabled: true,
            required: false,
            startup_timeout_ms: 10_000,
            operation_timeout_ms: 60_000,
            environment: BTreeMap::new(),
            transport: McpTransportConfig::Stdio(McpStdioConfig {
                command: command.into(),
                args: args.iter().map(|s| Box::<str>::from(*s)).collect(),
                restart_limit: 1,
            }),
        };
        validate_view(&ConfigView(vec![&server]))?;
        Ok(server)
    }

    /// Exact validated server alias.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Whether the configured server participates in startup.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    /// Whether failure of this server is required to fail startup.
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }
    /// Nonzero startup timeout in milliseconds.
    #[must_use]
    pub fn startup_timeout_ms(&self) -> u32 {
        self.startup_timeout_ms
    }
    /// Nonzero operation timeout in milliseconds.
    #[must_use]
    pub fn operation_timeout_ms(&self) -> u32 {
        self.operation_timeout_ms
    }
    /// Explicit environment overrides, without ambient lookup or expansion.
    #[must_use]
    pub fn environment(&self) -> &BTreeMap<Box<str>, Box<str>> {
        &self.environment
    }
    /// Validated transport-specific inputs.
    #[must_use]
    pub fn transport(&self) -> &McpTransportConfig {
        &self.transport
    }
}

impl McpStdioConfig {
    /// Explicit executable spelling, never a shell command string to evaluate.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }
    /// Exact ordered arguments, including empty strings.
    #[must_use]
    pub fn args(&self) -> &[Box<str>] {
        &self.args
    }
    /// Configured bounded restart count, including zero.
    #[must_use]
    pub fn restart_limit(&self) -> u8 {
        self.restart_limit
    }
}

impl McpRemoteConfig {
    /// Exact, structurally checked endpoint spelling. The HTTP authority must
    /// fully parse and admit it before effects; this is not a validated origin.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
    /// Static request headers; protocol-owned and Authorization headers are absent.
    #[must_use]
    pub fn headers(&self) -> &BTreeMap<Box<str>, Box<str>> {
        &self.headers
    }
    /// Header-name to environment-variable-name mappings, not resolved values.
    #[must_use]
    pub fn header_env(&self) -> &BTreeMap<Box<str>, Box<str>> {
        &self.header_env
    }
    /// Explicit environment name for bearer credentials.
    #[must_use]
    pub fn bearer_token_env(&self) -> Option<&str> {
        self.bearer_token_env.as_deref()
    }
    /// Optional explicit OAuth inputs.
    #[must_use]
    pub fn oauth(&self) -> Option<&McpOAuthConfig> {
        self.oauth.as_ref()
    }
}

impl McpOAuthConfig {
    /// Explicit protected resource identifier, if configured.
    #[must_use]
    pub fn resource(&self) -> Option<&str> {
        self.resource.as_deref()
    }
    /// Explicit issuer, if configured.
    #[must_use]
    pub fn issuer(&self) -> Option<&str> {
        self.issuer.as_deref()
    }
    /// Explicit client identifier, if configured.
    #[must_use]
    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }
    /// Environment name containing the client secret; never resolved by the codec.
    #[must_use]
    pub fn client_secret_env(&self) -> Option<&str> {
        self.client_secret_env.as_deref()
    }
    /// Structurally HTTPS client metadata URL; full parsed-path checks remain
    /// the HTTP authority's responsibility before any request.
    #[must_use]
    pub fn client_metadata_url(&self) -> Option<&str> {
        self.client_metadata_url.as_deref()
    }
    /// Requested scopes in configured order.
    #[must_use]
    pub fn scopes(&self) -> &[Box<str>] {
        &self.scopes
    }
}

struct ConfigView<'a>(Vec<&'a McpServerConfig>);

fn validate_view(view: &ConfigView<'_>) -> Result<(), McpConfigError> {
    // Use the same strict aggregate tree/string budget for programmatic edits,
    // without cloning secret-bearing configuration strings or server state.
    json::decode(&json::encode(view)?)?;
    Ok(())
}
impl Serialize for ConfigView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut root = serializer.serialize_map(Some(1))?;
        root.serialize_entry("mcp", &ServersView(&self.0))?;
        root.end()
    }
}

struct ServersView<'a>(&'a [&'a McpServerConfig]);
impl Serialize for ServersView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for server in self.0 {
            map.serialize_entry(server.name(), server)?;
        }
        map.end()
    }
}

impl Serialize for McpServerConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("enabled", &self.enabled)?;
        map.serialize_entry("required", &self.required)?;
        map.serialize_entry("startup_timeout_ms", &self.startup_timeout_ms)?;
        map.serialize_entry("operation_timeout_ms", &self.operation_timeout_ms)?;
        map.serialize_entry("environment", &self.environment)?;
        match &self.transport {
            McpTransportConfig::Stdio(config) => {
                map.serialize_entry("type", "stdio")?;
                map.serialize_entry("command", &config.command)?;
                map.serialize_entry("args", &config.args)?;
                map.serialize_entry("restart_limit", &config.restart_limit)?;
            }
            McpTransportConfig::Http(config) | McpTransportConfig::Sse(config) => {
                let kind = if matches!(&self.transport, McpTransportConfig::Http(_)) {
                    "http"
                } else {
                    "sse"
                };
                map.serialize_entry("type", kind)?;
                map.serialize_entry("url", &config.url)?;
                map.serialize_entry("headers", &config.headers)?;
                map.serialize_entry("header_env", &config.header_env)?;
                if let Some(value) = &config.bearer_token_env {
                    map.serialize_entry("bearer_token_env", value)?;
                }
                if let Some(value) = &config.oauth {
                    map.serialize_entry("oauth", value)?;
                }
            }
        }
        map.end()
    }
}
