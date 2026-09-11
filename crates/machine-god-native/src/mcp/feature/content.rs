//! Shared bounded resource/prompt/tool content admission after strict framing.
use super::{Error, McpFeatureCodecLimits, Result, charge};
use crate::mcp::catalog::fields;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::value::RawValue;
use std::fmt;

// Pinned tools and resources/prompts intentionally differ on URI/icon bounds,
// empty required strings and nested resource annotations. Share structure, not
// an accidentally stricter feature policy for every method.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Policy {
    Feature,
    Tool,
}
impl Policy {
    fn uri_limit(self, limits: McpFeatureCodecLimits) -> usize {
        if self == Self::Tool {
            limits.max_content_field_bytes
        } else {
            64 * 1024
        }
    }
    fn aggregate_limit(self, limits: McpFeatureCodecLimits) -> usize {
        if self == Self::Tool {
            limits.max_response_bytes
        } else {
            limits.max_content_bytes
        }
    }
    fn required(self, object: &fields::Object<'_>, key: &str, maximum: usize) -> Result<Box<str>> {
        if self == Self::Tool {
            required_allow_empty(object, key, maximum)
        } else {
            fields::required(object, key, maximum).map_err(Into::into)
        }
    }
}

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

fn optional_metadata(object: &fields::Object<'_>, annotations: bool) -> Result<usize> {
    let mut bytes = 0;
    if let Some(raw) = object.get("annotations").filter(|_| annotations) {
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
    resource_with_policy(raw, limits, total_content, Policy::Feature)
}

fn resource_with_policy(
    raw: &RawValue,
    limits: McpFeatureCodecLimits,
    total_content: &mut usize,
    policy: Policy,
) -> Result<McpResourceContent> {
    let limits = limits.validate()?;
    if raw.get().len() > limits.max_response_bytes {
        return Err(Error::Limit);
    }
    let object = fields::object(raw)?;
    let uri = policy.required(&object, "uri", policy.uri_limit(limits))?;
    let mime = fields::optional(&object, "mimeType", 4096)?;
    let mut bytes = optional_metadata(&object, policy == Policy::Feature)?;
    let aggregate_limit = policy.aggregate_limit(limits);
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
    charge(&mut bytes, uri.len(), aggregate_limit)?;
    charge(
        &mut bytes,
        mime.as_ref().map_or(0, |mime| mime.len()),
        aggregate_limit,
    )?;
    charge(&mut bytes, data.len(), aggregate_limit)?;
    charge(total_content, bytes, aggregate_limit)?;
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
    admit_with_policy(raw, limits, total_content, Policy::Feature)
}

/// Tool policy requires its caller to bound the complete result independently.
pub(crate) fn admit_with_policy(
    raw: &RawValue,
    limits: McpFeatureCodecLimits,
    total_content: &mut usize,
    policy: Policy,
) -> Result<McpContent> {
    let limits = limits.validate()?;
    if raw.get().len() > limits.max_response_bytes {
        return Err(Error::Limit);
    }
    let object = fields::object(raw)?;
    optional_metadata(&object, true)?;
    let kind = match fields::required(&object, "type", 32)?.as_ref() {
        "text" => {
            required_allow_empty(&object, "text", limits.max_content_field_bytes)?;
            McpContentKind::Text
        }
        name @ ("image" | "audio") => {
            let data = required_allow_empty(&object, "data", limits.max_content_field_bytes)?;
            policy.required(&object, "mimeType", 4096)?;
            base64(&data)?;
            if name == "image" {
                McpContentKind::Image
            } else {
                McpContentKind::Audio
            }
        }
        "resource_link" => {
            policy.required(&object, "uri", policy.uri_limit(limits))?;
            // Content links allow 4096 name bytes, unlike catalog names (256).
            policy.required(&object, "name", 4096)?;
            fields::optional(&object, "title", 4096)?;
            fields::optional(&object, "description", 64 * 1024)?;
            fields::optional(&object, "mimeType", 4096)?;
            if let Some(icons) = object.get("icons") {
                if policy == Policy::Tool {
                    fields::icons_allow_empty(icons, policy.uri_limit(limits))?;
                } else {
                    fields::icons(icons, policy.uri_limit(limits))?;
                }
            }
            if let Some(size) = object.get("size") {
                fields::size(size.get())?;
            }
            McpContentKind::ResourceLink
        }
        "resource" => {
            resource_with_policy(
                object.get("resource").ok_or(Error::InvalidResponse)?,
                limits,
                &mut 0,
                policy,
            )?;
            McpContentKind::Resource
        }
        _ => return Err(Error::InvalidResponse),
    };
    charge(
        total_content,
        compact_size(raw, policy.aggregate_limit(limits))?,
        policy.aggregate_limit(limits),
    )?;
    Ok(McpContent {
        raw: raw.to_owned(),
        kind,
    })
}

pub(crate) fn compact_size(raw: &RawValue, limit: usize) -> Result<usize> {
    let value = machine_god_core::json::from_str(raw.get()).map_err(|_| Error::InvalidResponse)?;
    compact_value_size(&value, limit)
}

pub(crate) fn compact_value_size(value: &serde_json::Value, limit: usize) -> Result<usize> {
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
    let mut counter = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, value).map_err(|_| Error::Limit)?;
    Ok(counter.bytes)
}
