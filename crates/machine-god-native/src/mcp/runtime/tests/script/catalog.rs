use super::*;
use crate::mcp::{pagination::McpRawCatalog, protocol::McpClientMetadata, stdio::McpStdioControl};

impl ScriptPeer {
    /// Dedicated discovery fixture; tool/feature callbacks cannot answer this.
    pub(in crate::mcp::runtime) async fn tools_catalog(
        &mut self,
        limits: McpCatalogLimits,
        epoch: Instant,
        clock: &dyn NativeMcpRuntimeClock,
    ) -> Result<McpRawCatalog> {
        let response = self
            .catalog_response
            .as_ref()
            .ok_or(NativeMcpRuntimeError::Unavailable)?
            .clone();
        let mut builder =
            McpCatalogBuilder::new(McpCatalogKind::Tools, ProtocolVersion::Modern, limits)
                .map_err(|_| NativeMcpRuntimeError::Invalid)?;
        loop {
            if self.closed.load(Ordering::Acquire) {
                return Err(NativeMcpRuntimeError::Unavailable);
            }
            let id = self.next;
            self.next = self
                .next
                .checked_add(1)
                .ok_or(NativeMcpRuntimeError::Limit)?;
            let cursor = builder.next_cursor().map(str::to_owned);
            let mut params = json!({"_meta": McpClientMetadata::for_protocol(ProtocolVersion::Modern, None, false, false)});
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            let wire = serde_json::to_vec(
                &json!({"jsonrpc":"2.0", "id":id, "method":"tools/list", "params":params}),
            )
            .map_err(|_| NativeMcpRuntimeError::Invalid)?;
            McpStdioControl::discovery(&wire).map_err(|_| NativeMcpRuntimeError::Invalid)?;
            {
                let mut writes = self.writes.lock().unwrap();
                writes.extend_from_slice(&wire);
                writes.push(b'\n');
            }
            let bytes = response(id).await?;
            if self.closed.load(Ordering::Acquire) {
                return Err(NativeMcpRuntimeError::Unavailable);
            }
            let now = clock
                .now()
                .checked_duration_since(epoch)
                .ok_or(NativeMcpRuntimeError::Invalid)?
                .as_millis()
                .try_into()
                .map_err(|_| NativeMcpRuntimeError::Limit)?;
            if !builder
                .append_response(&bytes, &RpcId::Integer(id), cursor.as_deref(), now)
                .map_err(|_| NativeMcpRuntimeError::Invalid)?
            {
                break;
            }
        }
        builder.finish().map_err(|_| NativeMcpRuntimeError::Invalid)
    }
}
