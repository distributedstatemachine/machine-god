//! Bounded, effect-free projection of actual native observations and inbox views.
//!
//! None of these values is execution authority. The driver retains the original
//! native token and exact session/turn owner, and settles replies through the
//! inbox. In particular, terminal engine events never finalize an ACP prompt.

use std::{
    fmt,
    io::{self, Write},
};

use machine_god_core::{EngineEvent, ModelEvent, TurnEvent};
use serde_json::{Value, json};

use super::protocol::{self, ACP_MAX_FRAME_BYTES, AcpMessage};

mod prompts;
#[cfg(test)]
mod tests;

pub use prompts::{
    NativeAcpClientRequest, NativeAcpReplyKind, decode_reply, project_permission, project_prompt,
};

/// Payload-free projection failure; no native identity or client text is retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    Limit,
    InvalidResponse,
    InvalidSource,
    Unsupported,
}
impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ACP projection unavailable")
    }
}
impl std::error::Error for ProjectionError {}

/// Project a session/update `update` body, never a final prompt result.
///
/// # Errors
/// Rejects payloads exceeding wire bounds before cloning or serializing them.
pub fn project_event(event: &EngineEvent) -> Result<Option<Value>, ProjectionError> {
    let mut update = match &event.payload {
        TurnEvent::Model {
            event: ModelEvent::TextDelta { text },
        } => text_chunk("agent_message_chunk", text)?,
        TurnEvent::Model {
            event: ModelEvent::ReasoningDelta { text },
        } => text_chunk("agent_thought_chunk", text)?,
        TurnEvent::Model {
            event: ModelEvent::ToolCall { call },
        } => tool_call(call, "tool_call", "pending")?,
        TurnEvent::ToolStarted { call } => tool_call(call, "tool_call_update", "in_progress")?,
        TurnEvent::ToolFinished { call_id, output } => {
            protocol::validate_value(&output.content, 5).map_err(|_| ProjectionError::Limit)?;
            let text = match &output.content {
                Value::String(text) => text.clone(),
                value => serialize_bounded(value, ACP_MAX_FRAME_BYTES)?,
            };
            let mut update = json!({
                "sessionUpdate":"tool_call_update",
                "toolCallId":call_id.as_str(),
                "status":if output.is_error { "failed" } else { "completed" },
                "content":[{"type":"content","content":{"type":"text","text":text}}]
            });
            // json!'s generic serializer normalizes arbitrary-precision tokens.
            // Move/clone existing exact Value trees directly into the map.
            update["rawOutput"] = output.content.clone();
            update
        }
        // Usage, provider stop and engine completion are not native persistence
        // completion. The driver's finalized outcome owns the prompt response.
        _ => return Ok(None),
    };
    update["_meta"] = json!({"machineGod":{
        "sessionIncarnationId":event.session_incarnation_id.as_str(),
        "turnId":event.turn_id.as_str(),
        "sequence":event.sequence
    }});
    checked(update).map(Some)
}

/// Supported native initialization capabilities. Client filesystem/terminal
/// flags never change the host's native execution authority.
#[must_use]
pub fn initialize_result() -> Value {
    json!({
        "protocolVersion":protocol::ACP_PROTOCOL_VERSION,
        "agentCapabilities":{
            "loadSession":true,
            "promptCapabilities":{"image":false,"audio":false,"embeddedContext":true},
            "mcpCapabilities":{"http":cfg!(feature = "mcp-http")},
            "sessionCapabilities":{"list":{},"resume":{},"close":{}}
        },
        "agentInfo":{"name":"machine-god","title":"Machine God","version":env!("CARGO_PKG_VERSION")},
        "authMethods":[]
    })
}

fn text_chunk(kind: &str, text: &str) -> Result<Value, ProjectionError> {
    bounded_text(text)?;
    Ok(json!({"sessionUpdate":kind,"content":{"type":"text","text":text}}))
}

fn tool_call(
    call: &machine_god_core::ToolCall,
    update: &str,
    status: &str,
) -> Result<Value, ProjectionError> {
    protocol::validate_value(&call.arguments, 5).map_err(|_| ProjectionError::Limit)?;
    let mut value = json!({
        "sessionUpdate":update, "toolCallId":call.id.as_str(),
        "title":call.name.as_str(), "kind":tool_kind(call.name.as_str()),
        "status":status
    });
    value["rawInput"] = call.arguments.clone();
    Ok(value)
}

fn tool_kind(name: &str) -> &'static str {
    match name {
        "read_file" | "list_files" | "file_info" | "read_tool_result" => "read",
        "write_file" | "edit_file" | "copy_file" | "create_folder" => "edit",
        "delete_file" => "delete",
        "rename_file" => "move",
        "grep_files" | "glob_files" | "semantic_search" | "web_search" => "search",
        "terminal" => "execute",
        "web_fetch" => "fetch",
        _ => "other",
    }
}

fn bounded_text(text: &str) -> Result<(), ProjectionError> {
    if text.len() > ACP_MAX_FRAME_BYTES - 4096 {
        Err(ProjectionError::Limit)
    } else {
        Ok(())
    }
}

fn serialize_bounded(value: &Value, limit: usize) -> Result<String, ProjectionError> {
    struct Writer {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit - self.bytes.len() {
                return Err(io::Error::other("ACP projection limit"));
            }
            let needed = self.bytes.len() + bytes.len();
            if needed > self.bytes.capacity() {
                let capacity = needed
                    .max(self.bytes.capacity().saturating_mul(2))
                    .min(self.limit);
                self.bytes.reserve_exact(capacity - self.bytes.len());
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    protocol::validate_value(value, 5).map_err(|_| ProjectionError::Limit)?;
    let mut writer = Writer {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| ProjectionError::Limit)?;
    String::from_utf8(writer.bytes).map_err(|_| ProjectionError::InvalidSource)
}

// Reuse the actual envelope encoder's lexical/retention/output admission. The
// placeholder method is longer than either session/update or elicitation/create
// and reserves the longest allowed RPC ID; the driver still encodes its frame.
fn checked(value: Value) -> Result<Value, ProjectionError> {
    let envelope = AcpMessage::Request {
        id: protocol::AcpId::String("i".repeat(protocol::ACP_MAX_ID_BYTES)),
        method: "session/request_permission".into(),
        params: Some(value),
    };
    protocol::encode_frame(&envelope).map_err(|_| ProjectionError::Limit)?;
    let AcpMessage::Request {
        params: Some(value),
        ..
    } = envelope
    else {
        unreachable!("constructed request");
    };
    Ok(value)
}
