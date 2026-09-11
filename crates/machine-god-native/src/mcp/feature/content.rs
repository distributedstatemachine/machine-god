//! Shared bounded resource/prompt/tool content admission after strict framing.
use super::{Error, McpFeatureCodecLimits, Result, charge};
use crate::mcp::catalog::fields;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::value::RawValue;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpContentKind {
    Text,
    Image,
    Audio,
    ResourceLink,
    Resource,
}
pub enum McpResourceData {
    Text(Box<str>),
    Blob(Box<str>),
}
impl fmt::Debug for McpResourceData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpResourceData { .. }")
    }
}
pub struct McpResourceContent {
    raw: Box<RawValue>,
    uri: Box<str>,
    mime: Option<Box<str>>,
    data: McpResourceData,
}
impl fmt::Debug for McpResourceContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpResourceContent { .. }")
    }
}
impl McpResourceContent {
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
    #[must_use]
    pub fn uri(&self) -> &str {
        &self.uri
    }
    #[must_use]
    pub fn mime_type(&self) -> Option<&str> {
        self.mime.as_deref()
    }
    #[must_use]
    pub fn data(&self) -> &McpResourceData {
        &self.data
    }
}
pub struct McpContent {
    raw: Box<RawValue>,
    kind: McpContentKind,
}
impl fmt::Debug for McpContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpContent")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl McpContent {
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
    #[must_use]
    pub fn kind(&self) -> McpContentKind {
        self.kind
    }
}

fn optional_metadata(object: &fields::Object<'_>) -> Result<usize> {
    let mut bytes = 0;
    if let Some(raw) = object.get("annotations") {
        fields::annotations(raw, false)?;
        bytes += fields::metadata(raw, 32, true)?.get().len();
    }
    if let Some(raw) = object.get("_meta") {
        bytes += fields::metadata(raw, 32, true)?.get().len();
    }
    Ok(bytes)
}
fn required_allow_empty(
    object: &fields::Object<'_>,
    key: &str,
    maximum: usize,
) -> Result<Box<str>> {
    fields::optional(object, key, maximum)?.ok_or(Error::InvalidResponse)
}
fn base64(value: &str) -> Result<()> {
    STANDARD
        .decode(value)
        .map(|_| ())
        .map_err(|_| Error::InvalidResponse)
}

/// Caller first admits the complete bounded duplicate-free envelope. Each item
/// is then charged before retention; no untrusted standalone parser is exposed.
pub(crate) fn resource(
    raw: &RawValue,
    limits: McpFeatureCodecLimits,
    total_content: &mut usize,
) -> Result<McpResourceContent> {
    let limits = limits.validate()?;
    if raw.get().len() > limits.max_response_bytes {
        return Err(Error::Limit);
    }
    let object = fields::object(raw)?;
    let uri = fields::required(&object, "uri", 64 * 1024)?;
    let mime = fields::optional(&object, "mimeType", 4096)?;
    let mut bytes = optional_metadata(&object)?;
    if object.contains_key("text") == object.contains_key("blob") {
        return Err(Error::InvalidResponse);
    }
    let text = object.contains_key("text");
    let data = required_allow_empty(
        &object,
        if text { "text" } else { "blob" },
        limits.max_content_field_bytes,
    )?;
    if !text {
        base64(&data)?;
    }
    charge(&mut bytes, uri.len(), limits.max_content_bytes)?;
    charge(
        &mut bytes,
        mime.as_ref().map_or(0, |mime| mime.len()),
        limits.max_content_bytes,
    )?;
    charge(&mut bytes, data.len(), limits.max_content_bytes)?;
    charge(total_content, bytes, limits.max_content_bytes)?;
    Ok(McpResourceContent {
        raw: raw.to_owned(),
        uri,
        mime,
        data: if text {
            McpResourceData::Text(data)
        } else {
            McpResourceData::Blob(data)
        },
    })
}

/// Shared prompt/tool content schema. Admission grants no provider instruction
/// or external effect authority. `raw` comes from a bounded admitted envelope.
pub(crate) fn admit(
    raw: &RawValue,
    limits: McpFeatureCodecLimits,
    total_content: &mut usize,
) -> Result<McpContent> {
    let limits = limits.validate()?;
    if raw.get().len() > limits.max_response_bytes {
        return Err(Error::Limit);
    }
    let object = fields::object(raw)?;
    optional_metadata(&object)?;
    let kind = match fields::required(&object, "type", 32)?.as_ref() {
        "text" => {
            required_allow_empty(&object, "text", limits.max_content_field_bytes)?;
            McpContentKind::Text
        }
        name @ ("image" | "audio") => {
            let data = required_allow_empty(&object, "data", limits.max_content_field_bytes)?;
            fields::required(&object, "mimeType", 4096)?;
            base64(&data)?;
            if name == "image" {
                McpContentKind::Image
            } else {
                McpContentKind::Audio
            }
        }
        "resource_link" => {
            fields::required(&object, "uri", 64 * 1024)?;
            // Content links allow 4096 name bytes, unlike catalog names (256).
            fields::required(&object, "name", 4096)?;
            fields::optional(&object, "title", 4096)?;
            fields::optional(&object, "description", 64 * 1024)?;
            fields::optional(&object, "mimeType", 4096)?;
            if let Some(icons) = object.get("icons") {
                fields::icons(icons, 64 * 1024)?;
            }
            if let Some(size) = object.get("size") {
                fields::size(size.get())?;
            }
            McpContentKind::ResourceLink
        }
        "resource" => {
            resource(
                object.get("resource").ok_or(Error::InvalidResponse)?,
                limits,
                &mut 0,
            )?;
            McpContentKind::Resource
        }
        _ => return Err(Error::InvalidResponse),
    };
    charge(
        total_content,
        compact_size(raw, limits.max_content_bytes)?,
        limits.max_content_bytes,
    )?;
    Ok(McpContent {
        raw: raw.to_owned(),
        kind,
    })
}

pub(crate) fn compact_size(raw: &RawValue, limit: usize) -> Result<usize> {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit - self.bytes {
                return Err(std::io::ErrorKind::FileTooLarge.into());
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let value = machine_god_core::json::from_str(raw.get()).map_err(|_| Error::InvalidResponse)?;
    let mut counter = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, &value).map_err(|_| Error::Limit)?;
    Ok(counter.bytes)
}
