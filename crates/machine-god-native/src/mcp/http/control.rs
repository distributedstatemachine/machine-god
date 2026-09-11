use super::{McpHttpError, Result};
use crate::mcp::{protocol::RpcId, stdio::McpStdioControl, submission::McpSubmissionHttpHead};
use std::fmt;

/// Explicit protocol-only HTTP authority, reusing the stdio control admission.
pub struct McpHttpControl(Kind);
enum Kind {
    Protocol(McpStdioControl),
    Listen,
    TerminateSession,
}
impl fmt::Debug for McpHttpControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpHttpControl { <redacted> }")
    }
}
impl McpHttpControl {
    /// # Errors
    /// Rejects any method outside the bounded discovery/catalog lane.
    pub fn discovery(bytes: &[u8]) -> Result<Self> {
        McpStdioControl::discovery(bytes)
            .map(|value| Self(Kind::Protocol(value)))
            .map_err(|_| McpHttpError::Invalid)
    }
    /// # Errors
    /// Accepts only initialized/cancelled protocol notifications.
    pub fn notification(bytes: &[u8]) -> Result<Self> {
        McpStdioControl::notification(bytes)
            .map(|value| Self(Kind::Protocol(value)))
            .map_err(|_| McpHttpError::Invalid)
    }
    /// # Errors
    /// Rejects null/oversized IDs; no arbitrary success response is accepted.
    pub fn unsupported(id: &RpcId) -> Result<Self> {
        McpStdioControl::unsupported(id)
            .map(|value| Self(Kind::Protocol(value)))
            .map_err(|_| McpHttpError::Invalid)
    }
    /// Explicit GET listener admission. No implicit reconnect/resume is performed.
    #[must_use]
    pub fn listen() -> Self {
        Self(Kind::Listen)
    }
    /// Explicit DELETE session teardown. The runtime must supply its admitted session header.
    #[must_use]
    pub fn terminate_session() -> Self {
        Self(Kind::TerminateSession)
    }
    pub(super) fn encode(&self, head: &McpSubmissionHttpHead) -> Result<Box<[u8]>> {
        match &self.0 {
            Kind::Protocol(control) => head
                .encode(control.json_bytes())
                .map_err(|_| McpHttpError::Invalid),
            Kind::Listen | Kind::TerminateSession => {
                let post = head.encode(b"").map_err(|_| McpHttpError::Invalid)?;
                let method: &[u8] = if matches!(self.0, Kind::Listen) {
                    b"GET "
                } else {
                    b"DELETE "
                };
                let mut bytes = Vec::with_capacity(post.len() + 2);
                bytes.extend_from_slice(method);
                for (index, line) in post[5..].split_inclusive(|byte| *byte == b'\n').enumerate() {
                    if index != 0
                        && (line.starts_with(b"content-type:")
                            || line.starts_with(b"content-length:"))
                    {
                        continue;
                    }
                    if line.starts_with(b"accept:") {
                        bytes.extend_from_slice(b"accept: text/event-stream\r\n");
                    } else {
                        bytes.extend_from_slice(line);
                    }
                }
                Ok(bytes.into_boxed_slice())
            }
        }
    }
}
