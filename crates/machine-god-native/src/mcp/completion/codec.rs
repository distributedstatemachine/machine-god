use super::{MAX_ID_BYTES, McpCompletionError, Result};
use crate::mcp::protocol::{RpcKind, WireLimits, parse_envelope};

/// Strict bounded notification data, never a locally selected source identity.
pub struct McpLegacyCompletionNotification {
    pub(super) id: Box<str>,
}
impl McpLegacyCompletionNotification {
    /// Unrelated valid notifications return `None`. Targeted malformed parameters,
    /// duplicate JSON keys and over-budget envelopes are rejected before retention.
    /// # Errors
    /// Rejects invalid notification framing, parameters, and independent JSON bounds.
    pub fn parse(bytes: &[u8]) -> Result<Option<Self>> {
        let frame = parse_envelope(
            bytes,
            WireLimits {
                max_frame_bytes: 128 * 1024,
                max_depth: 64,
                max_nodes: 65_536,
            },
        )
        .map_err(|_| McpCompletionError::Invalid)?;
        if frame.method() != Some("notifications/elicitation/complete") {
            return Ok(None);
        }
        if frame.kind() != RpcKind::Notification {
            return Err(McpCompletionError::Invalid);
        }
        let id = frame
            .params()
            .and_then(|params| params.get("elicitationId"))
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= MAX_ID_BYTES)
            .ok_or(McpCompletionError::Invalid)?;
        Ok(Some(Self { id: id.into() }))
    }

    #[must_use]
    pub fn elicitation_id(&self) -> &str {
        &self.id
    }
}
impl std::fmt::Debug for McpLegacyCompletionNotification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("McpLegacyCompletionNotification { <redacted> }")
    }
}
