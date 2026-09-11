//! Typed client advertisements are protocol data, never permission or consent.
use serde::{Serialize, Serializer, ser::SerializeMap};

use super::ProtocolVersion;

#[derive(Clone, Copy, Debug)]
pub struct McpClientMetadata {
    version: ProtocolVersion,
    progress_token: Option<u64>,
    form: bool,
    url: bool,
}
impl McpClientMetadata {
    /// Legacy requests omit metadata unless they carry a progress token.
    #[must_use]
    pub fn for_protocol(
        version: ProtocolVersion,
        progress_token: Option<u64>,
        form: bool,
        url: bool,
    ) -> Option<Self> {
        (version == ProtocolVersion::Modern || progress_token.is_some()).then_some(Self {
            version,
            progress_token,
            form,
            url,
        })
    }
}
impl Serialize for McpClientMetadata {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if self.version == ProtocolVersion::Modern {
            map.serialize_entry(
                "io.modelcontextprotocol/protocolVersion",
                ProtocolVersion::Modern.as_str(),
            )?;
            map.serialize_entry(
                "io.modelcontextprotocol/clientInfo",
                &serde_json::json!({"name":"machine-god", "version":env!("CARGO_PKG_VERSION")}),
            )?;
            let mut modes = serde_json::Map::new();
            for (enabled, name) in [(self.form, "form"), (self.url, "url")] {
                if enabled {
                    modes.insert(name.into(), serde_json::json!({}));
                }
            }
            let capabilities = if modes.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::json!({"elicitation":serde_json::Value::Object(modes)})
            };
            map.serialize_entry("io.modelcontextprotocol/clientCapabilities", &capabilities)?;
        }
        if let Some(token) = self.progress_token {
            map.serialize_entry("progressToken", &token)?;
        }
        map.end()
    }
}
