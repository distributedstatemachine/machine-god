use super::AcpSessionError;
use machine_god_core::{ContentBlock, Role, SessionRecord};
use serde_json::{Value, json};
use std::{fmt, io, sync::Arc};

/// A cursor over an immutable native checkpoint, not another copied transcript.
/// Each call projects at most one saved block. No tool is executed or inspected.
pub struct NativeAcpHistory {
    record: Arc<SessionRecord>,
    message: usize,
    block: usize,
    failed: bool,
}
impl fmt::Debug for NativeAcpHistory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeAcpHistory").finish_non_exhaustive()
    }
}
impl NativeAcpHistory {
    #[must_use]
    pub fn new(record: Arc<SessionRecord>) -> Self {
        Self {
            record,
            message: 0,
            block: 0,
            failed: false,
        }
    }

    /// Returns one ACP `session/update` update body. System/provider-only JSON
    /// is not editor history. Saved tool calls/results remain inert evidence.
    /// # Errors
    /// Rejects a block whose encoded projection cannot fit a bounded wire frame.
    /// # Panics
    /// Panics only if a statically constructed JSON object is not an object.
    pub fn next_update(&mut self) -> Result<Option<Value>, AcpSessionError> {
        if self.failed {
            return Err(AcpSessionError::Limit);
        }
        while let Some(message) = self.record.messages.get(self.message) {
            let Some(block) = message.content.get(self.block) else {
                self.message += 1;
                self.block = 0;
                continue;
            };
            self.block += 1;
            if message.role == Role::System {
                continue;
            }
            // Bound before making cloned JSON/string fields. Leave headroom for
            // the session/update envelope and JSON escaping of status fields.
            let mut budget = ByteBudget(crate::acp::protocol::ACP_MAX_FRAME_BYTES - 4096);
            if serde_json::to_writer(&mut budget, block).is_err() {
                self.failed = true;
                return Err(AcpSessionError::Limit);
            }
            let update = match block {
                ContentBlock::Text { text } => {
                    let kind = match message.role {
                        Role::User => "user_message_chunk",
                        Role::Assistant => "agent_message_chunk",
                        _ => continue,
                    };
                    json!({"sessionUpdate":kind,"content":{"type":"text","text":text}})
                }
                ContentBlock::ToolCall { call } => {
                    let mut value = json!({
                    "sessionUpdate":"tool_call", "toolCallId":call.id,
                    "title":call.name, "kind":"other", "status":"pending",
                    });
                    value
                        .as_object_mut()
                        .expect("object")
                        .insert("rawInput".into(), call.arguments.clone());
                    value
                }
                ContentBlock::ToolResult { call_id, output } => {
                    let mut value = json!({
                    "sessionUpdate":"tool_call_update", "toolCallId":call_id,
                    "status":if output.is_error {"failed"} else {"completed"},
                    });
                    value
                        .as_object_mut()
                        .expect("object")
                        .insert("rawOutput".into(), output.content.clone());
                    value
                }
                _ => continue,
            };
            return Ok(Some(update));
        }
        Ok(None)
    }
}

struct ByteBudget(usize);
impl io::Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::other("history projection limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
