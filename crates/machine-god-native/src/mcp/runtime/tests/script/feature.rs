use super::*;
use crate::mcp::{
    control::{self, McpFeatureControlAuthority, McpFeatureOperationOptions, McpFeatureReply},
    feature::{McpFeatureCatalogLoad, McpFeatureExchangeOptions},
    peer::McpPeerCapabilities,
    protocol::{NegotiatedProtocol, TransportKind, WireLimits, parse_envelope},
};

impl ScriptPeer {
    pub(in crate::mcp::runtime) fn feature(
        &mut self,
        request: &McpFeatureRequest,
        server: &str,
        catalogs: &[McpDescriptorCatalog],
        authority: &McpFeatureControlAuthority,
        options: McpFeatureOperationOptions,
    ) -> crate::mcp::runtime::features::Result<McpFeatureReply> {
        let init = parse_envelope(br#"{"jsonrpc":"2.0","id":0,"result":{"resultType":"complete","capabilities":{"resources":{},"prompts":{},"completions":{}}}}"#, WireLimits::default()).unwrap();
        let capabilities = McpPeerCapabilities::admit(&init, ProtocolVersion::Modern).unwrap();
        let mut load: Option<McpFeatureCatalogLoad> = None;
        loop {
            if self.closed.load(Ordering::Acquire) || !authority.is_live() {
                return Err(NativeMcpRuntimeError::Cancelled.into());
            }
            let id = self.next;
            self.next = self
                .next
                .checked_add(1)
                .ok_or(NativeMcpRuntimeError::Limit)?;
            let exchange = control::prepare(
                request,
                server,
                catalogs,
                McpFeatureExchangeOptions::new(
                    NegotiatedProtocol {
                        transport: TransportKind::Stdio,
                        version: ProtocolVersion::Modern,
                    },
                    id,
                    capabilities,
                )?,
                load.as_ref().and_then(McpFeatureCatalogLoad::next_cursor),
                options,
            )?;
            if control::is_list(request) && load.is_none() {
                load = Some(McpFeatureCatalogLoad::new(
                    &exchange,
                    options.pagination,
                    options.descriptors,
                )?);
            }
            {
                let mut writes = self.writes.lock().unwrap();
                writes.extend_from_slice(exchange.wire_json().get().as_bytes());
                writes.push(b'\n');
            }
            let response = self
                .response
                .as_ref()
                .ok_or(NativeMcpRuntimeError::Unavailable)?(id);
            let reply = control::admit(
                &mut load,
                &exchange,
                &response,
                options.epoch,
                options.epoch,
            )?;
            if !authority.is_live() {
                return Err(NativeMcpRuntimeError::Cancelled.into());
            }
            if let Some(reply) = reply {
                return Ok(reply);
            }
        }
    }
}
