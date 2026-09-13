use super::AcpSessionError;
use machine_god_core::Prompt;
use serde_json::Value;
use std::path::{Component, PathBuf};

/// Independent retained path and diagnostic bounds, below the wire frame bound.
pub const MAX_ACP_RESOURCE_TARGETS: usize = 64;
pub const MAX_ACP_RESOURCE_OMISSIONS: usize = 32;
pub const MAX_ACP_RESOURCE_URI_BYTES: usize = 4096;

/// A diagnostic contains descriptive input only, never authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAcpResourceOmissionReason {
    UnsafeTarget,
    TargetLimit,
    Unavailable,
    ContextLimit,
}

#[derive(Clone)]
pub struct NativeAcpResourceOmission {
    pub source: String,
    pub reason: NativeAcpResourceOmissionReason,
}
impl std::fmt::Debug for NativeAcpResourceOmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeAcpResourceOmission")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// Canonical user text and separately retained advisory context targets.
/// Constructors perform no filesystem operation and confer no authority.
pub struct NativeAcpPrompt {
    pub(crate) prompt: Prompt,
    pub(crate) resource_targets: Vec<PathBuf>,
    pub(crate) omissions: Vec<NativeAcpResourceOmission>,
    pub(crate) omitted_records: usize,
}

impl NativeAcpPrompt {
    #[must_use]
    pub const fn prompt(&self) -> &Prompt {
        &self.prompt
    }
    #[must_use]
    pub fn resource_targets(&self) -> &[PathBuf] {
        &self.resource_targets
    }
    #[must_use]
    pub fn omissions(&self) -> &[NativeAcpResourceOmission] {
        &self.omissions
    }
    #[must_use]
    pub const fn omitted_records(&self) -> usize {
        self.omitted_records
    }

    /// Charges canonical text and retained target/diagnostic source bytes for FIFO admission.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.prompt.text.len()
            + self
                .resource_targets
                .iter()
                .map(|path| path.as_os_str().len())
                .sum::<usize>()
            + self
                .omissions
                .iter()
                .map(|item| item.source.len())
                .sum::<usize>()
    }

    pub(crate) fn omit(&mut self, source: &str, reason: NativeAcpResourceOmissionReason) {
        if self
            .omissions
            .iter()
            .any(|item| item.source == source && item.reason == reason)
        {
            return;
        }
        if self.omissions.len() < MAX_ACP_RESOURCE_OMISSIONS {
            self.omissions.push(NativeAcpResourceOmission {
                source: source.to_owned(),
                reason,
            });
        } else {
            self.omitted_records = self.omitted_records.saturating_add(1);
        }
    }
}

impl std::fmt::Debug for NativeAcpPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeAcpPrompt")
            .field("resource_targets", &self.resource_targets.len())
            .field("omissions", &self.omissions.len())
            .field("omitted_records", &self.omitted_records)
            .finish_non_exhaustive()
    }
}

/// Decodes modern text/resources without I/O. URI-only resources supply scoped
/// instruction targets; some nonempty canonical text is still required.
/// Unsupported URI targets are explicit bounded omissions, not remote fetches.
/// # Errors
/// Rejects malformed, binary/image, empty or oversized prompts.
pub fn decode_prompt_input(params: &Value) -> Result<NativeAcpPrompt, AcpSessionError> {
    let blocks = params
        .get("prompt")
        .and_then(Value::as_array)
        .ok_or(AcpSessionError::InvalidPrompt)?;
    if blocks.len() > MAX_ACP_PROMPT_BLOCKS {
        return Err(AcpSessionError::Limit);
    }
    let mut result = NativeAcpPrompt {
        prompt: Prompt::from(""),
        resource_targets: Vec::new(),
        omissions: Vec::new(),
        omitted_records: 0,
    };
    let mut text = String::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => append(
                &mut text,
                "",
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or(AcpSessionError::InvalidPrompt)?,
            )?,
            Some("resource") => {
                let resource = block
                    .get("resource")
                    .and_then(Value::as_object)
                    .ok_or(AcpSessionError::InvalidPrompt)?;
                let uri = resource
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or(AcpSessionError::InvalidPrompt)?;
                if uri.is_empty()
                    || uri.len() > MAX_ACP_RESOURCE_URI_BYTES
                    || uri.chars().any(char::is_control)
                {
                    return Err(AcpSessionError::InvalidPrompt);
                }
                if resource.contains_key("blob") {
                    return Err(AcpSessionError::UnsupportedContent);
                }
                match local_file_target(uri) {
                    Some(path) if result.resource_targets.contains(&path) => {}
                    Some(path) if result.resource_targets.len() < MAX_ACP_RESOURCE_TARGETS => {
                        result.resource_targets.push(path);
                    }
                    Some(_) => result.omit(uri, NativeAcpResourceOmissionReason::TargetLimit),
                    None => result.omit(uri, NativeAcpResourceOmissionReason::UnsafeTarget),
                }
                if let Some(value) = resource.get("text") {
                    let value = value.as_str().ok_or(AcpSessionError::InvalidPrompt)?;
                    append(&mut text, &format!("File: {uri}\n"), value)?;
                }
            }
            Some(_) => return Err(AcpSessionError::UnsupportedContent),
            None => return Err(AcpSessionError::InvalidPrompt),
        }
    }
    if text.is_empty() {
        return Err(AcpSessionError::InvalidPrompt);
    }
    result.prompt = Prompt::from(text);
    Ok(result)
}

fn local_file_target(uri: &str) -> Option<PathBuf> {
    let (scheme, encoded) = uri.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("file") || encoded.contains(['?', '#']) {
        return None;
    }
    // Empty authority only. A host, including localhost, is not local authority.
    let encoded = if let Some(authority) = encoded.strip_prefix("//") {
        if !authority.starts_with('/') {
            return None;
        }
        authority
    } else {
        encoded
    };
    if !encoded.starts_with('/') {
        return None;
    }
    let mut decoded = Vec::with_capacity(encoded.len());
    let bytes = encoded.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = if bytes[index] == b'%' {
            let hi = char::from(*bytes.get(index + 1)?).to_digit(16)?;
            let lo = char::from(*bytes.get(index + 2)?).to_digit(16)?;
            index += 3;
            u8::try_from(hi * 16 + lo).ok()?
        } else {
            let byte = bytes[index];
            index += 1;
            byte
        };
        if byte.is_ascii_control() || byte == b'\\' {
            return None;
        }
        decoded.push(byte);
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if decoded.chars().any(char::is_control) {
        return None;
    }
    let path = PathBuf::from(decoded);
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
    {
        return None;
    }
    Some(path.components().collect())
}

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

#[cfg(test)]
mod resource_tests;
