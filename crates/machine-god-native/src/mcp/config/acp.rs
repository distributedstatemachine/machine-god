//! ACP admission constructs native selections without a profile encode/decode.
use super::{
    MAX_SERVERS, McpConfig, McpConfigError as Error, McpServerConfig, McpStdioConfig,
    McpTransportConfig, json, parse,
};
use crate::mcp::headers::McpResolvedHeaders;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, path::Path, sync::Arc};

type Result<T> = std::result::Result<T, Error>;
pub(crate) type Headers = Vec<(Box<str>, McpResolvedHeaders)>;
pub(crate) type Identities = Vec<Arc<[u8]>>;

pub(crate) fn decode(bytes: Option<&[u8]>) -> Result<(McpConfig, Headers, Identities)> {
    let Some(bytes) = bytes else {
        return Ok((McpConfig::new(), Vec::new(), Vec::new()));
    };
    let value = json::decode(bytes)?;
    let entries = value.as_array().ok_or(Error::Invalid)?;
    if entries.len() > MAX_SERVERS {
        return Err(Error::Limit);
    }
    let mut configuration = McpConfig::new();
    let mut headers = Vec::new();
    let mut identities = Vec::new();
    let mut identity_bytes = 0usize;
    for entry in entries {
        let object = parse::object(entry)?;
        let name = text(object, "name", 128, false)?;
        parse::name(name)?;
        if configuration.server(name).is_some() {
            return Err(Error::AlreadyExists);
        }
        let kind = object
            .get("type")
            .map(|value| value.as_str().ok_or(Error::Invalid))
            .transpose()?;
        let (transport, environment) = match kind {
            None | Some("stdio") => stdio(object)?,
            Some("http") => {
                parse::fields(object, &["name", "type", "url", "headers"])?;
                let url = text(object, "url", 4096, false)?;
                // Reuse the profile's endpoint grammar but never its header grammar.
                let remote_object =
                    Map::from_iter([("url".to_owned(), Value::String(url.to_owned()))]);
                let remote = parse::remote_config(&remote_object)?;
                let mut resolved = Vec::new();
                for entry in array(object, "headers", 128)? {
                    let pair = parse::object(entry)?;
                    parse::fields(pair, &["name", "value"])?;
                    let name = pair
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or(Error::Invalid)?;
                    let value = pair
                        .get("value")
                        .and_then(Value::as_str)
                        .ok_or(Error::Invalid)?;
                    resolved.push((name, value.as_bytes()));
                }
                let resolved =
                    McpResolvedHeaders::from_resolved(&resolved).map_err(|_| Error::Invalid)?;
                headers.push((name.into(), resolved));
                (McpTransportConfig::Http(remote), BTreeMap::new())
            }
            Some(_) => return Err(Error::Invalid),
        };
        // The strict input tree has already charged all retained string bytes.
        // Never call profile serialization on injected environment/header secrets.
        configuration.servers.push(McpServerConfig {
            name: name.into(),
            enabled: true,
            required: true,
            startup_timeout_ms: 10_000,
            operation_timeout_ms: 60_000,
            environment,
            transport,
        });
        let mut identity = b"MG-ACP-MCP-1\0".to_vec();
        serde_json::to_writer(&mut identity, entry).map_err(|_| Error::Invalid)?;
        identity_bytes = identity_bytes
            .checked_add(identity.len())
            .ok_or(Error::Limit)?;
        if identity_bytes > 2 * super::MAX_CONFIG_BYTES {
            return Err(Error::Limit);
        }
        identities.push(identity.into());
    }
    Ok((configuration, headers, identities))
}

type Environment = BTreeMap<Box<str>, Box<str>>;
fn stdio(object: &Map<String, Value>) -> Result<(McpTransportConfig, Environment)> {
    parse::fields(object, &["name", "type", "command", "args", "env"])?;
    let command = text(object, "command", 4096, false)?;
    if !Path::new(command).is_absolute() {
        return Err(Error::Invalid);
    }
    let args = array(object, "args", 256)?
        .iter()
        .map(|value| {
            let value = value.as_str().ok_or(Error::Invalid)?;
            literal(value)?;
            Ok(Box::<str>::from(value))
        })
        .collect::<Result<Box<[_]>>>()?;
    let mut environment = BTreeMap::new();
    for entry in array(object, "env", 256)? {
        let pair = parse::object(entry)?;
        parse::fields(pair, &["name", "value"])?;
        let key = text(pair, "name", 16 * 1024, false)?;
        if !key.bytes().enumerate().all(|(i, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || i > 0 && byte.is_ascii_digit()
        }) {
            return Err(Error::Invalid);
        }
        let value = pair
            .get("value")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid)?;
        literal(value)?;
        if environment.insert(key.into(), value.into()).is_some() {
            return Err(Error::Invalid);
        }
    }
    Ok((
        McpTransportConfig::Stdio(McpStdioConfig {
            command: command.into(),
            args,
            restart_limit: 1,
        }),
        environment,
    ))
}

fn text<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    limit: usize,
    empty: bool,
) -> Result<&'a str> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(Error::Invalid)?;
    parse::text(value, limit, empty)?;
    Ok(value)
}
fn array<'a>(object: &'a Map<String, Value>, key: &str, limit: usize) -> Result<&'a [Value]> {
    let value = object
        .get(key)
        .and_then(Value::as_array)
        .ok_or(Error::Invalid)?;
    if value.len() > limit {
        return Err(Error::Limit);
    }
    Ok(value)
}

fn literal(value: &str) -> Result<()> {
    if value.len() > 16 * 1024 {
        return Err(Error::Limit);
    }
    if value.contains('\0') {
        return Err(Error::Invalid);
    }
    Ok(())
}
