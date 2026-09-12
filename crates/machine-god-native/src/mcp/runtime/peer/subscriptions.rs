use super::{Error, NativeMcpOwnedPeer, Result};
use crate::mcp::{
    catalog_refresh::McpSubscriptionFilters,
    peer::McpPeerCapabilities,
    protocol::{RpcEnvelope, RpcId},
};
use std::time::Instant;

impl NativeMcpOwnedPeer {
    /// Removes only already admitted ordinary-exchange data; never polls I/O.
    pub(crate) fn take_notification(&mut self) -> Option<RpcEnvelope> {
        match self {
            #[cfg(test)]
            Self::Script(_) => None,
            Self::Stdio(peer) => peer.take_notification(),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer
                .take_notification()
                .map(crate::mcp::http_peer::McpHttpPeerFrame::into_envelope),
        }
    }

    pub(crate) fn capabilities(&self) -> McpPeerCapabilities {
        match self {
            #[cfg(test)]
            Self::Script(_) => McpPeerCapabilities::default(),
            Self::Stdio(peer) => peer.capabilities(),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer.capabilities(),
        }
    }

    pub(crate) async fn start_subscription(
        &mut self,
        filters: &McpSubscriptionFilters,
        deadline: Instant,
    ) -> Result<RpcId> {
        match self {
            #[cfg(test)]
            Self::Script(_) => Err(Error::Invalid),
            Self::Stdio(peer) => peer
                .start_subscription(filters, deadline)
                .await
                .map_err(|_| Error::Unavailable),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer
                .start_subscription(filters, deadline)
                .await
                .map_err(|_| Error::Unavailable),
        }
    }

    pub(crate) fn active_subscription(&self) -> Option<RpcId> {
        match self {
            #[cfg(test)]
            Self::Script(_) => None,
            Self::Stdio(peer) => peer.active_subscription(),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer.active_subscription(),
        }
    }

    pub(crate) async fn poll_subscription(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<RpcEnvelope>> {
        match self {
            #[cfg(test)]
            Self::Script(_) => Ok(None),
            Self::Stdio(peer) => peer
                .poll_subscription(deadline)
                .await
                .map_err(|_| Error::Unavailable),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer
                .poll_subscription(deadline)
                .await
                .map(|frame| frame.map(crate::mcp::http_peer::McpHttpPeerFrame::into_envelope))
                .map_err(|_| Error::Unavailable),
        }
    }

    pub(crate) async fn close_subscription(&mut self, deadline: Instant) -> Result<()> {
        match self {
            #[cfg(test)]
            Self::Script(_) => Ok(()),
            Self::Stdio(peer) => peer
                .close_subscription(deadline)
                .await
                .map_err(|_| Error::Unavailable),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => {
                peer.close_subscription();
                Ok(())
            }
        }
    }
}
