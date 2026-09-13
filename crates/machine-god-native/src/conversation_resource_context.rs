//! Inert provider-only ACP instruction context, separate from skill identity.

use machine_god_core::{MAX_SESSION_USER_CONTEXT_BYTES, SessionRecord, SessionUserContext};
use serde_json::{Value, json};
use std::fmt;

pub const NATIVE_RESOURCE_PROMPT_CONTEXT_KEY: &str = "machine_god.resource_prompt_context";

#[derive(Clone, Eq, PartialEq)]
pub struct NativeResourcePromptContext {
    text: String,
}

impl fmt::Debug for NativeResourcePromptContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeResourcePromptContext(..)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeResourcePromptContextError {
    InvalidMetadata,
    ResourceLimit,
    CheckpointMismatch,
}

impl fmt::Display for NativeResourcePromptContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("resource prompt context is invalid")
    }
}
impl std::error::Error for NativeResourcePromptContextError {}

impl NativeResourcePromptContext {
    /// Admits inert text, not a filesystem path or permission grant.
    /// # Errors
    /// Rejects text above the core user-context byte limit.
    pub fn new(text: String) -> Result<Self, NativeResourcePromptContextError> {
        if text.len() > MAX_SESSION_USER_CONTEXT_BYTES {
            return Err(NativeResourcePromptContextError::ResourceLimit);
        }
        Ok(Self { text })
    }
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn to_value(&self, sequence: u64, first_user_message: usize) -> Value {
        json!({"schema_version":1,"turn_sequence":sequence,"first_user_message":first_user_message,"text":self.text})
    }

    /// Converts only the already admitted advisory bytes; does not modify user history.
    #[must_use]
    pub fn into_user_context(self, user_message_index: usize) -> SessionUserContext {
        SessionUserContext {
            user_message_index,
            text: self.text,
        }
    }
}

/// Combines separately owned native advisory contexts without changing either
/// metadata identity or canonical user text. The only added bytes are two
/// newlines between nonempty contexts; resource instructions follow skills.
/// # Errors
/// Rejects combined text above the core limit before allocating the output.
pub fn compose_user_context(
    skill_text: Option<&str>,
    resource_text: Option<&str>,
    user_message_index: usize,
) -> Result<Option<SessionUserContext>, NativeResourcePromptContextError> {
    let skill = skill_text.filter(|text| !text.is_empty());
    let resource = resource_text.filter(|text| !text.is_empty());
    if skill.is_none() && resource.is_none() {
        return Ok(None);
    }
    let separator = if skill.is_some() && resource.is_some() {
        2
    } else {
        0
    };
    let length = skill
        .map_or(0, str::len)
        .checked_add(resource.map_or(0, str::len))
        .and_then(|length| length.checked_add(separator))
        .filter(|length| *length <= MAX_SESSION_USER_CONTEXT_BYTES)
        .ok_or(NativeResourcePromptContextError::ResourceLimit)?;
    let mut text = String::with_capacity(length);
    if let Some(skill) = skill {
        text.push_str(skill);
    }
    if separator != 0 {
        text.push_str("\n\n");
    }
    if let Some(resource) = resource {
        text.push_str(resource);
    }
    Ok(Some(SessionUserContext {
        user_message_index,
        text,
    }))
}

/// Reads only bounded inert continuation bytes for the exact saved checkpoint.
/// # Errors
/// Rejects malformed, unsupported, oversized or checkpoint-mismatched metadata.
pub fn saved_resource_text(
    record: &SessionRecord,
    checkpoint: Option<(u64, usize)>,
) -> Result<Option<&str>, NativeResourcePromptContextError> {
    let Some(value) = record.metadata.get(NATIVE_RESOURCE_PROMPT_CONTEXT_KEY) else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or(NativeResourcePromptContextError::InvalidMetadata)?;
    if object.len() != 4 || object.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(NativeResourcePromptContextError::InvalidMetadata);
    }
    let sequence = object
        .get("turn_sequence")
        .and_then(Value::as_u64)
        .ok_or(NativeResourcePromptContextError::InvalidMetadata)?;
    let first_user = object
        .get("first_user_message")
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or(NativeResourcePromptContextError::InvalidMetadata)?;
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .ok_or(NativeResourcePromptContextError::InvalidMetadata)?;
    if text.len() > MAX_SESSION_USER_CONTEXT_BYTES {
        return Err(NativeResourcePromptContextError::ResourceLimit);
    }
    if sequence == 0 || checkpoint != Some((sequence, first_user)) {
        return Err(NativeResourcePromptContextError::CheckpointMismatch);
    }
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{SessionId, SessionIncarnationId};

    #[test]
    fn only_exact_checkpoint_reconstitutes_inert_context() {
        let context = NativeResourcePromptContext::new("source text".to_owned()).unwrap();
        let mut record = SessionRecord::empty(
            SessionId::new("acp-resource").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        );
        record.metadata.insert(
            NATIVE_RESOURCE_PROMPT_CONTEXT_KEY.to_owned(),
            context.to_value(1, 0),
        );
        assert_eq!(
            saved_resource_text(&record, Some((1, 0))).unwrap(),
            Some("source text")
        );
        assert_eq!(
            saved_resource_text(&record, Some((2, 0))),
            Err(NativeResourcePromptContextError::CheckpointMismatch)
        );
        assert_eq!(
            saved_resource_text(&record, None),
            Err(NativeResourcePromptContextError::CheckpointMismatch)
        );
        assert!(!format!("{context:?}").contains("source text"));
        record
            .metadata
            .get_mut(NATIVE_RESOURCE_PROMPT_CONTEXT_KEY)
            .unwrap()["schema_version"] = json!(0);
        assert_eq!(
            saved_resource_text(&record, Some((1, 0))),
            Err(NativeResourcePromptContextError::InvalidMetadata)
        );
    }

    #[test]
    fn context_projection_retains_bound_and_message_index() {
        let text = "x".repeat(MAX_SESSION_USER_CONTEXT_BYTES);
        let context = NativeResourcePromptContext::new(text.clone()).unwrap();
        let projected = context.into_user_context(3);
        assert_eq!(projected.user_message_index, 3);
        assert_eq!(projected.text, text);
        assert!(
            NativeResourcePromptContext::new("x".repeat(MAX_SESSION_USER_CONTEXT_BYTES + 1))
                .is_err()
        );
    }

    #[test]
    fn composed_context_counts_separator_and_preserves_exact_boundary() {
        let skill = "s".repeat(MAX_SESSION_USER_CONTEXT_BYTES - 3);
        let context = compose_user_context(Some(&skill), Some("r"), 7)
            .unwrap()
            .unwrap();
        assert_eq!(context.text.len(), MAX_SESSION_USER_CONTEXT_BYTES);
        assert!(context.text.ends_with("\n\nr"));
        assert_eq!(context.user_message_index, 7);
        assert!(compose_user_context(Some(&skill), Some("rr"), 7).is_err());
        assert_eq!(
            compose_user_context(None, Some("r"), 0)
                .unwrap()
                .unwrap()
                .text,
            "r"
        );
        assert_eq!(
            compose_user_context(Some("s"), None, 0)
                .unwrap()
                .unwrap()
                .text,
            "s"
        );
        assert!(compose_user_context(Some(""), None, 0).unwrap().is_none());
        assert!(
            compose_user_context(
                None,
                Some(&"r".repeat(MAX_SESSION_USER_CONTEXT_BYTES + 1)),
                0
            )
            .is_err()
        );
    }
}
