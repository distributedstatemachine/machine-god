//! Shape codecs only. Neither sampling nor roots is implemented by this module.
use super::{
    Error, McpMrtrLimits, Result,
    bounds::{self, Object},
};
use serde_json::value::RawValue;

pub(super) fn params(raw: &RawValue, limits: McpMrtrLimits) -> Result<()> {
    let fields = bounds::object(raw)?;
    let messages = bounds::array(bounds::required(&fields, "messages")?)?;
    let tokens = bounds::number(bounds::required(&fields, "maxTokens")?, limits)?;
    if tokens.nonnegative_usize().is_none_or(|value| value == 0) {
        return Err(Error::InvalidRequest);
    }
    for raw in messages {
        message(raw, limits)?;
    }
    bounds::optional_text(&fields, "systemPrompt", limits.max_string_bytes)?;
    if let Some(context) =
        bounds::optional_text(&fields, "includeContext", limits.max_string_bytes)?
        && !matches!(context.as_ref(), "none" | "thisServer" | "allServers")
    {
        return Err(Error::InvalidRequest);
    }
    if let Some(value) = fields.get("temperature")
        && !matches!(value.get().as_bytes().first(), Some(b'-' | b'0'..=b'9'))
    {
        return Err(Error::InvalidRequest);
    }
    if let Some(value) = fields.get("stopSequences") {
        for item in bounds::array(value)? {
            bounds::text(item, limits.max_string_bytes)?;
        }
    }
    for name in ["metadata", "modelPreferences"] {
        if let Some(raw) = fields.get(name) {
            bounds::object(raw)?;
        }
    }
    if let Some(raw) = fields.get("tools") {
        for raw in bounds::array(raw)? {
            let tool = bounds::object(raw)?;
            bounds::text(bounds::required(&tool, "name")?, limits.max_string_bytes)?;
            bounds::object(bounds::required(&tool, "inputSchema")?)?;
        }
    }
    if let Some(raw) = fields.get("toolChoice") {
        let choice = bounds::object(raw)?;
        if let Some(mode) = bounds::optional_text(&choice, "mode", limits.max_string_bytes)?
            && !matches!(mode.as_ref(), "auto" | "required" | "none")
        {
            return Err(Error::InvalidRequest);
        }
    }
    Ok(())
}
fn role(fields: &Object<'_>, limits: McpMrtrLimits) -> Result<()> {
    if !matches!(
        bounds::text(bounds::required(fields, "role")?, limits.max_string_bytes)?.as_ref(),
        "user" | "assistant"
    ) {
        return Err(Error::InvalidRequest);
    }
    Ok(())
}
fn message(raw: &RawValue, limits: McpMrtrLimits) -> Result<()> {
    let fields = bounds::object(raw)?;
    role(&fields, limits)?;
    content(bounds::required(&fields, "content")?, limits, 0)
}
pub(super) fn result(raw: &RawValue, limits: McpMrtrLimits) -> Result<()> {
    let fields = bounds::object(raw)?;
    role(&fields, limits)?;
    bounds::text(bounds::required(&fields, "model")?, limits.max_string_bytes)?;
    bounds::optional_text(&fields, "stopReason", limits.max_string_bytes)?;
    if let Some(meta) = fields.get("_meta") {
        bounds::object(meta)?;
    }
    content(bounds::required(&fields, "content")?, limits, 0)
}
fn content(raw: &RawValue, limits: McpMrtrLimits, depth: usize) -> Result<()> {
    if depth > limits.max_depth {
        return Err(Error::Limit);
    }
    if raw.get().starts_with('[') {
        for item in bounds::array(raw)? {
            item_content(item, limits, depth)?;
        }
        Ok(())
    } else {
        item_content(raw, limits, depth)
    }
}
fn item_content(raw: &RawValue, limits: McpMrtrLimits, depth: usize) -> Result<()> {
    let fields = bounds::object(raw)?;
    match bounds::text(bounds::required(&fields, "type")?, limits.max_string_bytes)?.as_ref() {
        "text" => {
            bounds::text(bounds::required(&fields, "text")?, limits.max_string_bytes)?;
        }
        "image" | "audio" => {
            // Sampling's pinned codec checks strings, not base64/media policy.
            bounds::text(bounds::required(&fields, "data")?, limits.max_json_bytes)?;
            bounds::text(
                bounds::required(&fields, "mimeType")?,
                limits.max_name_bytes,
            )?;
        }
        "tool_use" => {
            for name in ["id", "name"] {
                bounds::text(bounds::required(&fields, name)?, limits.max_name_bytes)?;
            }
            bounds::object(bounds::required(&fields, "input")?)?;
        }
        "tool_result" => {
            bounds::text(
                bounds::required(&fields, "toolUseId")?,
                limits.max_name_bytes,
            )?;
            let raw = bounds::required(&fields, "content")?;
            bounds::array(raw)?;
            content(raw, limits, depth + 1)?;
        }
        _ => return Err(Error::InvalidRequest),
    }
    Ok(())
}
pub(super) fn roots_result(raw: &RawValue, limits: McpMrtrLimits) -> Result<()> {
    let fields = bounds::object(raw)?;
    for raw in bounds::array(bounds::required(&fields, "roots")?)? {
        let root = bounds::object(raw)?;
        bounds::text(bounds::required(&root, "uri")?, limits.max_string_bytes)?;
        bounds::optional_text(&root, "name", limits.max_string_bytes)?;
        if let Some(meta) = root.get("_meta") {
            bounds::object(meta)?;
        }
    }
    Ok(())
}
