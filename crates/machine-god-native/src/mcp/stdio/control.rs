//! Native-only startup/discovery authority; never an arbitrary method channel.
use super::super::protocol::{RpcId, RpcKind};
use super::{McpStdioError, Result, RpcEnvelope, WireLimits, fmt, parse_envelope};

/// Caller owns startup/discovery admission. Application feature calls and tool
/// calls are deliberately excluded; protocol data cannot create this authority.
pub struct McpStdioControl {
    pub(super) bytes: Box<[u8]>,
}
impl fmt::Debug for McpStdioControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpStdioControl { <redacted> }")
    }
}
impl McpStdioControl {
    /// Already admitted control JSON without the stdio delimiter. This is data,
    /// not a generic HTTP method or application-call authorization boundary.
    pub(crate) fn json_bytes(&self) -> &[u8] {
        &self.bytes[..self.bytes.len() - 1]
    }

    /// Exact bounded prebuilt JSON, checked against the startup/discovery lane.
    ///
    /// # Errors
    /// Rejects other methods, malformed envelopes and oversized payloads.
    pub fn discovery(bytes: &[u8]) -> Result<Self> {
        let envelope = admit(bytes)?;
        if envelope.kind() != RpcKind::Request
            || !matches!(
                envelope.method(),
                Some(
                    "server/discover"
                        | "initialize"
                        | "tools/list"
                        | "resources/list"
                        | "resources/templates/list"
                        | "prompts/list"
                )
            )
        {
            return Err(McpStdioError::Invalid);
        }
        Self::framed(bytes)
    }

    /// Legacy initialized and request-cancellation notifications only. The
    /// runtime is responsible for the generation/request being cancelled.
    ///
    /// # Errors
    /// Rejects other notifications or malformed/bounded framing.
    pub fn notification(bytes: &[u8]) -> Result<Self> {
        let envelope = admit(bytes)?;
        if envelope.kind() != RpcKind::Notification
            || !matches!(
                envelope.method(),
                Some("notifications/initialized" | "notifications/cancelled")
            )
        {
            return Err(McpStdioError::Invalid);
        }
        if envelope.method() == Some("notifications/cancelled") {
            let params = envelope
                .params()
                .and_then(serde_json::Value::as_object)
                .ok_or(McpStdioError::Invalid)?;
            let id = params.get("requestId").ok_or(McpStdioError::Invalid)?;
            if !(id.as_i64().is_some() || id.as_str().is_some_and(|value| value.len() <= 1024)) {
                return Err(McpStdioError::Invalid);
            }
        }
        Self::framed(bytes)
    }

    /// Fixed rejection of unsupported server requests; no arbitrary successful
    /// result or permission-bearing continuation can enter this lane.
    ///
    /// # Errors
    /// Rejects null or oversized IDs.
    pub fn unsupported(id: &RpcId) -> Result<Self> {
        let id = match id {
            RpcId::Integer(value) => serde_json::Value::from(*value),
            RpcId::String(value) if value.len() <= 1024 => serde_json::Value::from(value.clone()),
            RpcId::String(_) | RpcId::Null => return Err(McpStdioError::Invalid),
        };
        let value = serde_json::json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32601,"message":"Method not found"}});
        let bytes = serde_json::to_vec(&value).map_err(|_| McpStdioError::Invalid)?;
        admit(&bytes)?;
        Self::framed(&bytes)
    }
    fn framed(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 128 * 1024 {
            return Err(McpStdioError::Capacity);
        }
        let mut framed = Vec::with_capacity(bytes.len() + 1);
        framed.extend_from_slice(bytes);
        framed.push(b'\n');
        Ok(Self {
            bytes: framed.into_boxed_slice(),
        })
    }
}
fn admit(bytes: &[u8]) -> Result<RpcEnvelope> {
    if bytes.contains(&b'\n') || bytes.contains(&b'\r') {
        return Err(McpStdioError::Invalid);
    }
    parse_envelope(
        bytes,
        WireLimits {
            max_frame_bytes: 128 * 1024,
            max_depth: 64,
            max_nodes: 8192,
        },
    )
    .map_err(|_| McpStdioError::Invalid)
}
