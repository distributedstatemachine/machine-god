//! Semantic admission of an already bounded strict JSON tree.
use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::{
    MAX_SERVER_NAME_BYTES, McpConfigError, McpOAuthConfig, McpRemoteConfig, McpServerConfig,
    McpStdioConfig, McpTransportConfig,
};

type Result<T> = std::result::Result<T, McpConfigError>;
type Strings = BTreeMap<Box<str>, Box<str>>;

pub(super) fn object(value: &Value) -> Result<&Map<String, Value>> {
    value.as_object().ok_or(McpConfigError::Invalid)
}

pub(super) fn fields(object: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(McpConfigError::Invalid);
    }
    Ok(())
}

pub(super) fn text(value: &str, max: usize, empty: bool) -> Result<()> {
    if value.len() > max {
        return Err(McpConfigError::Limit);
    }
    if (!empty && value.trim().is_empty()) || value.chars().any(char::is_control) {
        return Err(McpConfigError::Invalid);
    }
    Ok(())
}

pub(super) fn name(value: &str) -> Result<()> {
    text(value, MAX_SERVER_NAME_BYTES, false)?;
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(McpConfigError::Invalid);
    }
    Ok(())
}

fn string(value: &Value, max: usize, empty: bool) -> Result<Box<str>> {
    let value = value.as_str().ok_or(McpConfigError::Invalid)?;
    text(value, max, empty)?;
    Ok(value.into())
}

fn optional(object: &Map<String, Value>, key: &str) -> Result<Option<Box<str>>> {
    let limit = if matches!(key, "resource" | "issuer" | "client_metadata_url") {
        4096
    } else {
        16 * 1024
    };
    object
        .get(key)
        .filter(|v| !v.is_null())
        .map(|v| string(v, limit, false))
        .transpose()
}

fn boolean(object: &Map<String, Value>, key: &str, default: bool) -> Result<bool> {
    object
        .get(key)
        .map_or(Ok(default), |v| v.as_bool().ok_or(McpConfigError::Invalid))
}

fn integer(
    object: &Map<String, Value>,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64> {
    let value = object
        .get(key)
        .map_or(Ok(default), |v| v.as_u64().ok_or(McpConfigError::Invalid))?;
    if !(min..=max).contains(&value) {
        return Err(McpConfigError::Invalid);
    }
    Ok(value)
}

fn environment_name(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    {
        return Err(McpConfigError::Invalid);
    }
    Ok(())
}

fn strings(value: Option<&Value>, max: usize) -> Result<Strings> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let object = object(value)?;
    if object.len() > max {
        return Err(McpConfigError::Limit);
    }
    object
        .iter()
        .map(|(key, value)| {
            text(key, 16 * 1024, false)?;
            Ok((key.as_str().into(), string(value, 16 * 1024, true)?))
        })
        .collect()
}

fn list(value: &Value, max: usize) -> Result<Box<[Box<str>]>> {
    let items = value.as_array().ok_or(McpConfigError::Invalid)?;
    if items.len() > max {
        return Err(McpConfigError::Limit);
    }
    items.iter().map(|v| string(v, 16 * 1024, true)).collect()
}

pub(super) fn server(name_value: &str, value: &Value) -> Result<McpServerConfig> {
    const COMMON: &[&str] = &[
        "type",
        "enabled",
        "required",
        "startup_timeout_ms",
        "operation_timeout_ms",
        "env",
        "environment",
    ];
    name(name_value)?;
    let object = object(value)?;
    let kind = object
        .get("type")
        .map_or(Ok("local"), |v| v.as_str().ok_or(McpConfigError::Invalid))?;
    let remote = kind == "http";
    if !remote && !matches!(kind, "local" | "stdio") {
        return Err(McpConfigError::Invalid);
    }
    let active = if remote {
        &["url", "headers", "header_env", "bearer_token_env", "oauth"][..]
    } else {
        &["command", "args", "restart_limit"][..]
    };
    if object
        .keys()
        .any(|key| !COMMON.contains(&key.as_str()) && !active.contains(&key.as_str()))
    {
        return Err(McpConfigError::Invalid);
    }
    let environment = strings(object.get("environment").or_else(|| object.get("env")), 256)?;
    for key in environment.keys() {
        environment_name(key)?;
    }
    let transport = if remote {
        let config = remote_config(object)?;
        McpTransportConfig::Http(config)
    } else {
        McpTransportConfig::Stdio(stdio_config(object)?)
    };
    Ok(McpServerConfig {
        name: name_value.into(),
        enabled: boolean(object, "enabled", true)?,
        required: boolean(object, "required", false)?,
        startup_timeout_ms: u32::try_from(integer(
            object,
            "startup_timeout_ms",
            10_000,
            1,
            u64::from(u32::MAX),
        )?)
        .map_err(|_| McpConfigError::Invalid)?,
        operation_timeout_ms: u32::try_from(integer(
            object,
            "operation_timeout_ms",
            60_000,
            1,
            u64::from(u32::MAX),
        )?)
        .map_err(|_| McpConfigError::Invalid)?,
        environment,
        transport,
    })
}

fn stdio_config(object: &Map<String, Value>) -> Result<McpStdioConfig> {
    let value = object.get("command").ok_or(McpConfigError::Invalid)?;
    let (command, args) = if let Some(command) = value.as_str() {
        text(command, 4096, false)?;
        (
            Box::<str>::from(command),
            object
                .get("args")
                .map_or_else(|| Ok(Box::default()), |v| list(v, 256))?,
        )
    } else {
        let items = value.as_array().ok_or(McpConfigError::Invalid)?;
        if items.len() > 257 {
            return Err(McpConfigError::Limit);
        }
        let command = string(items.first().ok_or(McpConfigError::Invalid)?, 4096, false)?;
        let args = items[1..]
            .iter()
            .map(|v| string(v, 16 * 1024, true))
            .collect::<Result<_>>()?;
        // The producer intentionally gives a command vector precedence over args.
        (command, args)
    };
    Ok(McpStdioConfig {
        command,
        args,
        restart_limit: u8::try_from(integer(object, "restart_limit", 1, 0, 255)?)
            .map_err(|_| McpConfigError::Invalid)?,
    })
}

fn remote_config(object: &Map<String, Value>) -> Result<McpRemoteConfig> {
    let url = string(
        object.get("url").ok_or(McpConfigError::Invalid)?,
        4096,
        false,
    )?;
    endpoint_shape(&url)?;
    let headers = strings(object.get("headers"), 128)?;
    let header_env = strings(object.get("header_env"), 128)?;
    if headers.len() + header_env.len() > 128 {
        return Err(McpConfigError::Limit);
    }
    let mut names: Vec<&str> = Vec::new();
    for key in headers.keys().chain(header_env.keys()) {
        header_name(key)?;
        if names.iter().any(|prior| prior.eq_ignore_ascii_case(key)) {
            return Err(McpConfigError::Invalid);
        }
        names.push(key);
    }
    for value in header_env.values() {
        environment_name(value)?;
    }
    let bearer_token_env = optional(object, "bearer_token_env")?;
    if let Some(value) = &bearer_token_env {
        environment_name(value)?;
    }
    let oauth = object.get("oauth").map(oauth_config).transpose()?;
    Ok(McpRemoteConfig {
        url,
        headers,
        header_env,
        bearer_token_env,
        oauth,
    })
}

fn header_name(value: &str) -> Result<()> {
    const RESERVED: &[&str] = &[
        "authorization",
        "accept",
        "accept-encoding",
        "connection",
        "content-length",
        "content-type",
        "host",
        "last-event-id",
        "mcp-method",
        "mcp-name",
        "mcp-protocol-version",
        "mcp-session-id",
        "transfer-encoding",
    ];
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        || RESERVED.iter().any(|key| key.eq_ignore_ascii_case(value))
        || value
            .get(..10)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("mcp-param-"))
    {
        return Err(McpConfigError::Invalid);
    }
    Ok(())
}

fn oauth_config(value: &Value) -> Result<McpOAuthConfig> {
    let object = object(value)?;
    fields(
        object,
        &[
            "resource",
            "issuer",
            "client_id",
            "client_secret_env",
            "client_metadata_url",
            "scopes",
        ],
    )?;
    let config = McpOAuthConfig {
        resource: optional(object, "resource")?,
        issuer: optional(object, "issuer")?,
        client_id: optional(object, "client_id")?,
        client_secret_env: optional(object, "client_secret_env")?,
        client_metadata_url: optional(object, "client_metadata_url")?,
        scopes: object
            .get("scopes")
            .map_or_else(|| Ok(Box::default()), |v| list(v, 64))?,
    };
    if let Some(value) = &config.client_secret_env {
        environment_name(value)?;
        if config.client_id.is_none() {
            return Err(McpConfigError::Invalid);
        }
    }
    if let Some(value) = &config.resource {
        endpoint_shape(value)?;
    }
    if let Some(value) = &config.client_metadata_url {
        endpoint_shape(value)?;
        let (scheme, rest) = value.split_once("://").ok_or(McpConfigError::Invalid)?;
        let path = rest
            .split('?')
            .next()
            .unwrap_or_default()
            .split_once('/')
            .map(|(_, path)| path);
        if !scheme.eq_ignore_ascii_case("https")
            || path.is_none_or(|p| p.trim_matches('/').is_empty())
        {
            return Err(McpConfigError::Invalid);
        }
    }
    Ok(config)
}

// Deliberately a structural configuration check, not a URI parser or an origin
// policy. The effect-bearing HTTP authority must use an established URI parser
// and validate endpoint/resource/issuer/client metadata before sending anything.
fn endpoint_shape(value: &str) -> Result<()> {
    text(value, 4096, false)?;
    if value.chars().any(char::is_whitespace) || value.contains(['#', '\\']) {
        return Err(McpConfigError::Invalid);
    }
    let (scheme, rest) = value.split_once("://").ok_or(McpConfigError::Invalid)?;
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(McpConfigError::Invalid);
    }
    if scheme.eq_ignore_ascii_case("https") {
        return Ok(());
    }
    if scheme.eq_ignore_ascii_case("http") {
        let (host, port) = authority.rsplit_once(':').ok_or(McpConfigError::Invalid)?;
        if (host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]")
            && !port.is_empty()
            && port.bytes().all(|b| b.is_ascii_digit())
            && port.parse::<u16>().is_ok()
        {
            return Ok(());
        }
    }
    Err(McpConfigError::Invalid)
}
