//! Bounded canonical detail text. Rendering never resolves archives or runs tools.
use super::NativeManagedHistoryMode;
use machine_god_core::{ContentBlock, Role};
use std::{borrow::Cow, io};

pub(super) fn text(
    block: &ContentBlock,
    role: Role,
    mode: NativeManagedHistoryMode,
) -> Result<Option<Cow<'_, str>>, ()> {
    if let ContentBlock::Text { text } = block
        && (matches!(role, Role::User | Role::Assistant) || mode.is_full())
    {
        return Ok(Some(Cow::Borrowed(text)));
    }
    if !mode.is_full() {
        return Ok(None);
    }
    // Public borrowed views can also be built by embedding hosts. Validate
    // their JSON before serde recursion, not only after byte serialization.
    let value = match block {
        ContentBlock::Json { value } => Some(value),
        ContentBlock::ToolCall { call } => Some(&call.arguments),
        ContentBlock::ToolResult { output, .. } => Some(&output.content),
        _ => None,
    };
    if let Some(value) = value {
        let mut remaining = crate::session_store::MAX_FILE_SESSION_BYTES;
        validate(value, 0, &mut remaining)?;
    }
    let mut output = Bounded(Vec::new());
    serde_json::to_writer(&mut output, block).map_err(|_| ())?;
    String::from_utf8(output.0)
        .map(Cow::Owned)
        .map(Some)
        .map_err(|_| ())
}

fn validate(value: &serde_json::Value, depth: usize, remaining: &mut usize) -> Result<(), ()> {
    *remaining = remaining.checked_sub(1).ok_or(())?;
    if matches!(
        value,
        serde_json::Value::Array(_) | serde_json::Value::Object(_)
    ) && depth >= machine_god_core::MAX_SAFE_JSON_DEPTH
    {
        return Err(());
    }
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .try_for_each(|value| validate(value, depth + 1, remaining)),
        serde_json::Value::Object(values) => values
            .values()
            .try_for_each(|value| validate(value, depth + 1, remaining)),
        _ => Ok(()),
    }
}

struct Bounded(Vec<u8>);
impl io::Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self
            .0
            .len()
            .checked_add(bytes.len())
            .filter(|next| *next <= crate::session_store::MAX_FILE_SESSION_BYTES)
            .ok_or_else(|| io::Error::other("history detail bound"))?;
        if next > self.0.capacity() {
            let capacity = next
                .max(self.0.capacity().saturating_mul(2))
                .min(crate::session_store::MAX_FILE_SESSION_BYTES);
            self.0
                .try_reserve_exact(capacity - self.0.len())
                .map_err(|_| io::Error::other("history detail capacity"))?;
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_host_constructed_excessive_json_depth_before_serializing() {
        let mut value = serde_json::json!(null);
        for _ in 0..=machine_god_core::MAX_SAFE_JSON_DEPTH {
            value = serde_json::Value::Array(vec![value]);
        }
        assert!(
            text(
                &ContentBlock::Json { value },
                Role::Tool,
                NativeManagedHistoryMode::Full
            )
            .is_err()
        );
    }
    #[test]
    fn detail_is_complete_canonical_data_and_has_a_hard_scratch_bound() {
        let block = ContentBlock::Json {
            value: serde_json::json!({"value": "α🙂\u{1b}private".repeat(2048)}),
        };
        assert!(
            text(
                &block,
                Role::Assistant,
                NativeManagedHistoryMode::Conversation
            )
            .unwrap()
            .is_none()
        );
        let full = text(&block, Role::Assistant, NativeManagedHistoryMode::Full)
            .unwrap()
            .unwrap();
        assert_eq!(serde_json::from_str::<ContentBlock>(&full).unwrap(), block);
        assert!(!full.contains('\u{1b}'));
        let mut output = Bounded(Vec::new());
        io::Write::write_all(
            &mut output,
            &vec![b'a'; crate::session_store::MAX_FILE_SESSION_BYTES],
        )
        .unwrap();
        assert!(io::Write::write_all(&mut output, b"x").is_err());
        assert_eq!(output.0.len(), crate::session_store::MAX_FILE_SESSION_BYTES);
    }
}
