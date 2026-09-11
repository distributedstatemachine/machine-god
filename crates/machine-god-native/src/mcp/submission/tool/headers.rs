//! Pinned SEP-2243 header annotation admission and request projection.

use super::{BTreeMap, McpSubmissionError, McpToolRequest, ProtocolVersion, RawValue, Result};
use crate::mcp::submission::McpSubmissionHttpHead;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::BTreeSet;

type Object<'a> = BTreeMap<String, &'a RawValue>;
const MAX_DEPTH: usize = 64;
const MAX_FIELD: usize = 16 * 1024;
const MAX_FIELDS: usize = 256;
const MAX_FIELD_BYTES: usize = 768 * 1024;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub(super) fn validate_schema(schema: &super::McpSchema) -> Result<()> {
    validate_root(&object(schema.raw_json())?)
}

pub(super) fn project(
    request: &McpToolRequest,
    base: &McpSubmissionHttpHead,
) -> Result<McpSubmissionHttpHead> {
    let protocol = request.options.protocol;
    let version = base
        .headers()
        .find(|(name, _)| *name == "mcp-protocol-version");
    if protocol.sends_http_protocol_header() {
        if version.map(|(_, value)| value) != Some(protocol.version.as_str().as_bytes()) {
            return Err(McpSubmissionError::Invalid);
        }
    } else if version.is_some() {
        return Err(McpSubmissionError::Invalid);
    }
    if base.headers().any(|(name, _)| {
        matches!(name, "mcp-method" | "mcp-name") || name.starts_with("mcp-param-")
    }) {
        return Err(McpSubmissionError::Invalid);
    }
    let mut projected = Fields {
        values: Vec::new(),
        count: base.headers().len(),
        bytes: base
            .headers()
            .map(|(name, value)| name.len() + value.len())
            .sum(),
    };
    if protocol.version == ProtocolVersion::Modern {
        let root = object(request.schema.raw_json())?;
        validate_root(&root)?;
        projected.push("Mcp-Method", b"tools/call")?;
        projected.push("Mcp-Name", &encode(request.runtime.binding.remote_tool())?)?;
        let arguments = object(
            std::str::from_utf8(&request.arguments).map_err(|_| McpSubmissionError::Invalid)?,
        )?;
        if let Some(properties) = root.get("properties") {
            append_properties(&object(properties.get())?, &arguments, &mut projected, 0)?;
        }
    }
    let mut fields: Vec<_> = base.headers().collect();
    fields.extend(
        projected
            .values
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_slice())),
    );
    McpSubmissionHttpHead::new(base.endpoint(), &fields)
}

struct Fields {
    values: Vec<(String, Vec<u8>)>,
    count: usize,
    bytes: usize,
}
impl Fields {
    fn push(&mut self, name: &str, value: &[u8]) -> Result<()> {
        if self.count >= MAX_FIELDS
            || name.len() > MAX_FIELD
            || value.len() > MAX_FIELD
            || name.len() + value.len() > MAX_FIELD_BYTES.saturating_sub(self.bytes)
        {
            return Err(McpSubmissionError::Limit);
        }
        self.count += 1;
        self.bytes += name.len() + value.len();
        self.values.push((name.into(), value.into()));
        Ok(())
    }
}

fn object(text: &str) -> Result<Object<'_>> {
    serde_json::from_str(text).map_err(|_| McpSubmissionError::Invalid)
}
fn string(raw: &RawValue) -> Result<String> {
    serde_json::from_str(raw.get()).map_err(|_| McpSubmissionError::Invalid)
}
fn token(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

fn validate_root(root: &Object<'_>) -> Result<()> {
    let mut names = BTreeSet::new();
    for (name, value) in root {
        if name == "x-mcp-header" {
            return Err(McpSubmissionError::Invalid);
        }
        if name == "properties" {
            validate_properties(&object(value.get())?, &mut names, 0)?;
        } else if contains_annotation(value, 0)? {
            return Err(McpSubmissionError::Invalid);
        }
    }
    Ok(())
}

fn validate_properties(
    properties: &Object<'_>,
    names: &mut BTreeSet<String>,
    depth: usize,
) -> Result<()> {
    if depth >= MAX_DEPTH {
        return Err(McpSubmissionError::Limit);
    }
    for value in properties.values() {
        if !value.get().starts_with('{') {
            if contains_annotation(value, 0)? {
                return Err(McpSubmissionError::Invalid);
            }
            continue;
        }
        let property = object(value.get())?;
        if let Some(annotation) = property.get("x-mcp-header") {
            let name = string(annotation)?;
            let kind = property
                .get("type")
                .ok_or(McpSubmissionError::Invalid)
                .and_then(|raw| string(raw))?;
            if !token(&name)
                || !matches!(kind.as_str(), "string" | "integer" | "boolean")
                || ["$ref", "allOf", "anyOf", "oneOf"]
                    .iter()
                    .any(|name| property.contains_key(*name))
                || !names.insert(name.to_ascii_lowercase())
            {
                return Err(McpSubmissionError::Invalid);
            }
        }
        for (name, child) in &property {
            match name.as_str() {
                "x-mcp-header" => {}
                "properties" => validate_properties(&object(child.get())?, names, depth + 1)?,
                _ if contains_annotation(child, 0)? => return Err(McpSubmissionError::Invalid),
                _ => {}
            }
        }
    }
    Ok(())
}

fn contains_annotation(raw: &RawValue, depth: usize) -> Result<bool> {
    if depth >= MAX_DEPTH {
        return Err(McpSubmissionError::Limit);
    }
    match raw.get().as_bytes().first() {
        Some(b'{') => {
            let fields = object(raw.get())?;
            if fields.contains_key("x-mcp-header") {
                return Ok(true);
            }
            for child in fields.values() {
                if contains_annotation(child, depth + 1)? {
                    return Ok(true);
                }
            }
        }
        Some(b'[') => {
            let values: Vec<&RawValue> =
                serde_json::from_str(raw.get()).map_err(|_| McpSubmissionError::Invalid)?;
            for child in values {
                if contains_annotation(child, depth + 1)? {
                    return Ok(true);
                }
            }
        }
        _ => {}
    }
    Ok(false)
}

fn append_properties(
    properties: &Object<'_>,
    arguments: &Object<'_>,
    fields: &mut Fields,
    depth: usize,
) -> Result<()> {
    if depth >= MAX_DEPTH {
        return Err(McpSubmissionError::Limit);
    }
    for (name, value) in properties {
        if !value.get().starts_with('{') {
            continue;
        }
        let Some(argument) = arguments.get(name) else {
            continue;
        };
        if argument.get() == "null" {
            continue;
        }
        let property = object(value.get())?;
        if let Some(annotation) = property.get("x-mcp-header") {
            let suffix = string(annotation)?;
            if suffix.len() > MAX_FIELD - "Mcp-Param-".len() {
                return Err(McpSubmissionError::Limit);
            }
            let kind = string(property.get("type").ok_or(McpSubmissionError::Invalid)?)?;
            let value = match kind.as_str() {
                "string" => string(argument)?,
                "boolean" => serde_json::from_str::<bool>(argument.get())
                    .map_err(|_| McpSubmissionError::Invalid)?
                    .to_string(),
                "integer" => {
                    let value: i64 = serde_json::from_str(argument.get())
                        .map_err(|_| McpSubmissionError::Invalid)?;
                    if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
                        return Err(McpSubmissionError::Invalid);
                    }
                    value.to_string()
                }
                _ => return Err(McpSubmissionError::Invalid),
            };
            fields.push(&format!("Mcp-Param-{suffix}"), &encode(&value)?)?;
        }
        if let Some(nested) = property.get("properties") {
            if argument.get().starts_with('{') {
                append_properties(
                    &object(nested.get())?,
                    &object(argument.get())?,
                    fields,
                    depth + 1,
                )?;
            } else if contains_annotation(nested, 0)? {
                return Err(McpSubmissionError::Invalid);
            }
        }
    }
    Ok(())
}

fn encode(value: &str) -> Result<Vec<u8>> {
    let encode = value.starts_with("=?base64?")
        || value.starts_with([' ', '\t'])
        || value.ends_with([' ', '\t'])
        || value.bytes().any(|byte| !(0x20..=0x7e).contains(&byte));
    if encode {
        let length = value
            .len()
            .div_ceil(3)
            .checked_mul(4)
            .and_then(|size| size.checked_add(11))
            .ok_or(McpSubmissionError::Limit)?;
        if length > MAX_FIELD {
            return Err(McpSubmissionError::Limit);
        }
        Ok(format!("=?base64?{}?=", STANDARD.encode(value)).into_bytes())
    } else if value.len() > MAX_FIELD {
        Err(McpSubmissionError::Limit)
    } else {
        Ok(value.as_bytes().to_vec())
    }
}
