use serde_json::Value;

use super::{McpPeerError, Result, RpcEnvelope};
use crate::mcp::protocol::ProtocolVersion;

/// Admitted capability data, never tool or application execution authority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct McpPeerCapabilities {
    flags: u16,
}
impl McpPeerCapabilities {
    #[must_use]
    pub const fn tools(self) -> bool {
        self.flags & 1 != 0
    }
    #[must_use]
    pub const fn tools_list_changed(self) -> bool {
        self.flags & 2 != 0
    }
    #[must_use]
    pub const fn resources(self) -> bool {
        self.flags & 4 != 0
    }
    #[must_use]
    pub const fn resources_list_changed(self) -> bool {
        self.flags & 8 != 0
    }
    #[must_use]
    pub const fn resources_subscribe(self) -> bool {
        self.flags & 16 != 0
    }
    #[must_use]
    pub const fn prompts(self) -> bool {
        self.flags & 32 != 0
    }
    #[must_use]
    pub const fn prompts_list_changed(self) -> bool {
        self.flags & 64 != 0
    }
    #[must_use]
    pub const fn completions(self) -> bool {
        self.flags & 128 != 0
    }
    pub(crate) fn admit(response: &RpcEnvelope, version: ProtocolVersion) -> Result<Self> {
        let result = response
            .result()
            .and_then(Value::as_object)
            .ok_or(McpPeerError::InvalidResult)?;
        if let Some(value) = result.get("resultType") {
            if value.as_str() != Some("complete") {
                return Err(McpPeerError::InvalidResult);
            }
        } else if version == ProtocolVersion::Modern {
            return Err(McpPeerError::InvalidResult);
        }
        let Some(capabilities) = result.get("capabilities") else {
            return Ok(Self::default());
        };
        let capabilities = capabilities
            .as_object()
            .ok_or(McpPeerError::InvalidResult)?;
        let mut admitted = Self::default();
        for (name, present, flags) in [
            ("tools", 1, &[("listChanged", 2)][..]),
            ("resources", 4, &[("listChanged", 8), ("subscribe", 16)][..]),
            ("prompts", 32, &[("listChanged", 64)][..]),
            ("completions", 128, &[][..]),
        ] {
            let Some(value) = capabilities.get(name) else {
                continue;
            };
            let value = value.as_object().ok_or(McpPeerError::InvalidResult)?;
            admitted.flags |= present;
            for &(flag, bit) in flags {
                match value.get(flag).map(Value::as_bool) {
                    Some(Some(true)) => admitted.flags |= bit,
                    Some(None) => return Err(McpPeerError::InvalidResult),
                    None | Some(Some(false)) => {}
                }
            }
        }
        Ok(admitted)
    }
}
