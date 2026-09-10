//! Bounded inert skill text retained with one native conversation checkpoint.

use std::fmt;

use machine_god_core::MAX_SESSION_USER_CONTEXT_BYTES;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_core::{SessionRecord, SessionUserContext};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use serde_json::{Value, json};

/// Reserved continuation data, never filesystem or permission authority.
pub const NATIVE_SKILL_PROMPT_CONTEXT_KEY: &str = "machine_god.skill_prompt_context";

/// Already materialized text supplied by an explicitly composed native host.
/// This value contains no catalog selection, path capability or tool grant.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeSkillPromptContext {
    text: String,
}

impl fmt::Debug for NativeSkillPromptContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeSkillPromptContext(..)")
    }
}

/// Content-free rejection of invalid continuation data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillPromptContextError {
    InvalidMetadata,
    UnsupportedVersion,
    ResourceLimit,
    CheckpointMismatch,
}

impl fmt::Display for NativeSkillPromptContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidMetadata => "skill prompt context is malformed",
            Self::UnsupportedVersion => "skill prompt context version is unsupported",
            Self::ResourceLimit => "skill prompt context exceeds its limit",
            Self::CheckpointMismatch => "skill prompt context does not match its checkpoint",
        })
    }
}

impl std::error::Error for NativeSkillPromptContextError {}

impl NativeSkillPromptContext {
    /// Accepts exact UTF-8 data without scanning files or interpreting instructions.
    ///
    /// # Errors
    /// Rejects text above the core's 65,536-byte user-context limit. Core also
    /// checks composed metadata and provider payload bounds during admission.
    pub fn new(text: String) -> Result<Self, NativeSkillPromptContextError> {
        if text.len() > MAX_SESSION_USER_CONTEXT_BYTES {
            return Err(NativeSkillPromptContextError::ResourceLimit);
        }
        Ok(Self { text })
    }

    /// Exact inert bytes, intentionally visible only through explicit access.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn to_value(&self, sequence: u64, first_user_message: usize) -> Value {
        json!({
            "schema_version": 1,
            "turn_sequence": sequence,
            "first_user_message": first_user_message,
            "text": self.text,
        })
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn into_user_context(self, user_message_index: usize) -> SessionUserContext {
        SessionUserContext {
            user_message_index,
            text: self.text,
        }
    }
}

/// Validates the reserved shallow value before copying any payload. The caller
/// has independently checked the checkpoint against canonical user history.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn saved_text(
    record: &SessionRecord,
    checkpoint: Option<(u64, usize)>,
) -> Result<Option<&str>, NativeSkillPromptContextError> {
    let Some(value) = record.metadata.get(NATIVE_SKILL_PROMPT_CONTEXT_KEY) else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or(NativeSkillPromptContextError::InvalidMetadata)?;
    if object.len() != 4 {
        return Err(NativeSkillPromptContextError::InvalidMetadata);
    }
    let version = object
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or(NativeSkillPromptContextError::InvalidMetadata)?;
    if version != 1 {
        return Err(NativeSkillPromptContextError::UnsupportedVersion);
    }
    let sequence = object
        .get("turn_sequence")
        .and_then(Value::as_u64)
        .ok_or(NativeSkillPromptContextError::InvalidMetadata)?;
    let first_user_message = object
        .get("first_user_message")
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or(NativeSkillPromptContextError::InvalidMetadata)?;
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .ok_or(NativeSkillPromptContextError::InvalidMetadata)?;
    if text.len() > MAX_SESSION_USER_CONTEXT_BYTES {
        return Err(NativeSkillPromptContextError::ResourceLimit);
    }
    if sequence == 0 || checkpoint != Some((sequence, first_user_message)) {
        return Err(NativeSkillPromptContextError::CheckpointMismatch);
    }
    Ok(Some(text))
}
