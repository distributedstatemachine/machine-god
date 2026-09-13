use super::AcpSessionError;
use machine_god_core::Prompt;
use serde_json::Value;

/// Bounds retained joined prompt text independently of the wire frame limit.
pub const MAX_ACP_PROMPT_BYTES: usize = 1024 * 1024;
pub const MAX_ACP_PROMPT_BLOCKS: usize = 4096;

/// Decodes ACP's prompt array. Resource text is supplied content, not authority
/// to read a URI. No file, URL, environment or editor operation occurs here.
/// # Errors
/// Rejects malformed blocks, unsupported content, empty text and bounded-input
/// violations. Unsupported content is never silently discarded.
pub fn decode_prompt(params: &Value) -> Result<Prompt, AcpSessionError> {
    let blocks = params
        .get("prompt")
        .and_then(Value::as_array)
        .ok_or(AcpSessionError::InvalidPrompt)?;
    if blocks.len() > MAX_ACP_PROMPT_BLOCKS {
        return Err(AcpSessionError::Limit);
    }
    let mut text = String::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let value = block
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or(AcpSessionError::InvalidPrompt)?;
                append(&mut text, "", value)?;
            }
            Some("resource") => {
                let resource = block
                    .get("resource")
                    .and_then(Value::as_object)
                    .ok_or(AcpSessionError::InvalidPrompt)?;
                let uri = resource
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or(AcpSessionError::InvalidPrompt)?;
                if uri.is_empty() || uri.len() > 4096 || uri.chars().any(char::is_control) {
                    return Err(AcpSessionError::InvalidPrompt);
                }
                let value = resource
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or(AcpSessionError::UnsupportedContent)?;
                // The URI is presentation only. Never derive a filesystem or
                // network capability from editor-controlled embedded content.
                let label = format!("File: {uri}\n");
                append(&mut text, &label, value)?;
            }
            Some(_) => return Err(AcpSessionError::UnsupportedContent),
            None => return Err(AcpSessionError::InvalidPrompt),
        }
    }
    if text.is_empty() {
        return Err(AcpSessionError::InvalidPrompt);
    }
    Ok(Prompt::from(text))
}

fn append(text: &mut String, prefix: &str, value: &str) -> Result<(), AcpSessionError> {
    let separator = usize::from(!text.is_empty());
    if text
        .len()
        .saturating_add(separator)
        .saturating_add(prefix.len())
        .saturating_add(value.len())
        > MAX_ACP_PROMPT_BYTES
    {
        return Err(AcpSessionError::Limit);
    }
    if separator != 0 {
        text.push('\n');
    }
    text.push_str(prefix);
    text.push_str(value);
    Ok(())
}
