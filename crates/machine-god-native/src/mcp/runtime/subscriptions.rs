//! Exact peer subscription state, shared by startup and caller-driven refresh.
use super::{NativeMcpCatalogState, NativeMcpOwnedPeer, NativeMcpRuntimeError as Error, Result};
use crate::mcp::{
    catalog_refresh::{McpRefreshNotification, McpSubscriptionFilters},
    protocol::{RpcEnvelope, RpcId},
};
use std::time::Instant;

impl NativeMcpCatalogState {
    /// Startup owns the peer and applies its original attempt deadline and
    /// cancellation guards around this future. An ACK is not tool authority.
    pub(crate) async fn start_subscription(
        &mut self,
        peer: &mut NativeMcpOwnedPeer,
        deadline: Instant,
    ) -> Result<()> {
        let uris = self.uris.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        let filters = McpSubscriptionFilters::new(peer.capabilities(), &uris)
            .map_err(|_| Error::Invalid)?;
        if filters.is_empty() {
            return Ok(());
        }
        let id = peer.start_subscription(&filters, deadline).await?;
        let RpcId::Integer(value) = id else {
            return Err(Error::Invalid);
        };
        self.policy
            .install_subscription(&self.generation, value, filters)
            .map_err(|_| Error::Invalid)?;
        self.subscription = Some(id);
        self.acknowledged = false;
        while !self.acknowledged {
            let Some(envelope) = peer.poll_subscription(deadline).await? else {
                return Err(Error::Unavailable);
            };
            if self.observe(&envelope)? {
                peer.close_subscription(deadline).await?;
                return Err(Error::Unavailable);
            }
        }
        Ok(())
    }

    /// Returns whether the exact listener must close; payloads are not retained.
    pub(super) fn observe(&mut self, envelope: &RpcEnvelope) -> Result<bool> {
        match self
            .policy
            .observe(&self.generation, envelope)
            .map_err(|_| Error::Unavailable)?
        {
            McpRefreshNotification::Acknowledged => self.acknowledged = true,
            McpRefreshNotification::CloseUnsupported | McpRefreshNotification::CloseCancelled => {
                self.acknowledged = false;
                return Ok(true);
            }
            McpRefreshNotification::Ignored
            | McpRefreshNotification::Invalidated
            | McpRefreshNotification::AllResourceReads => {}
        }
        Ok(false)
    }
}
