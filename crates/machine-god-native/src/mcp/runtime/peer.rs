use super::{NativeMcpRuntimeError as Error, Result};
use crate::mcp::{
    peer::McpStdioPeer,
    protocol::NegotiatedProtocol,
    submission::{McpSubmission, McpSubmissionRuntime, McpToolReservation},
};
use std::sync::Arc;

/// Concrete already negotiated peers. Admission/launch/network selection belong
/// to the native startup owner; metadata cannot manufacture this authority.
pub enum NativeMcpOwnedPeer {
    #[cfg(test)]
    Script(super::tests::script::ScriptPeer),
    Stdio(McpStdioPeer),
    #[cfg(feature = "mcp-http")]
    Http(Box<crate::mcp::http_peer::McpHttpPeer>),
}
impl std::fmt::Debug for NativeMcpOwnedPeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeMcpOwnedPeer { <redacted> }")
    }
}
impl NativeMcpOwnedPeer {
    pub(super) fn supports_tools(&self) -> bool {
        match self {
            #[cfg(test)]
            Self::Script(peer) => peer.tools,
            Self::Stdio(peer) => peer.capabilities().tools(),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer.capabilities().tools(),
        }
    }
    pub(super) async fn call(
        &mut self,
        submission: McpSubmission,
        deadline: std::time::Instant,
    ) -> Result<Box<[u8]>> {
        match self {
            #[cfg(test)]
            Self::Script(peer) => peer.call(submission).await,
            Self::Stdio(peer) => peer
                .call_frame(submission, deadline)
                .await
                .map(|frame| frame.bytes().into())
                .map_err(|_| Error::Unavailable),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => {
                let head = submission.http_head().ok_or(Error::Invalid)?;
                peer.call(submission, head, deadline)
                    .await
                    .map(|frame| frame.bytes().into())
                    .map_err(|_| Error::Unavailable)
            }
        }
    }
    pub(super) fn completion(&self) -> NativeMcpPeerCompletion {
        match self {
            #[cfg(test)]
            Self::Script(peer) => NativeMcpPeerCompletion::Script(peer.closed.clone()),
            Self::Stdio(peer) => NativeMcpPeerCompletion::Stdio(peer.completion()),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => NativeMcpPeerCompletion::Http(peer.completion()),
        }
    }
    pub(super) fn protocol(&self) -> NegotiatedProtocol {
        match self {
            #[cfg(test)]
            Self::Script(_) => NegotiatedProtocol {
                transport: crate::mcp::protocol::TransportKind::Stdio,
                version: crate::mcp::protocol::ProtocolVersion::Modern,
            },
            Self::Stdio(peer) => peer.protocol(),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer.protocol(),
        }
    }
    pub(super) fn admit_runtimes(
        &mut self,
        runtimes: Vec<Arc<McpSubmissionRuntime>>,
    ) -> Result<()> {
        match self {
            #[cfg(test)]
            Self::Script(peer) => {
                peer.runtimes = runtimes;
                Ok(())
            }
            Self::Stdio(peer) => peer.admit_runtimes(runtimes).map_err(|_| Error::Invalid),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer.admit_runtimes(runtimes).map_err(|_| Error::Invalid),
        }
    }
    pub(super) fn reserve(&mut self) -> Result<McpToolReservation> {
        match self {
            #[cfg(test)]
            Self::Script(peer) => peer.reserve(),
            Self::Stdio(peer) => peer.reserve_tool().map_err(|_| Error::Unavailable),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer.reserve_tool().map_err(|_| Error::Unavailable),
        }
    }
    pub(super) fn close(&mut self) {
        match self {
            #[cfg(test)]
            Self::Script(peer) => peer.close(),
            Self::Stdio(peer) => peer.close(),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer.close(),
        }
    }
}

/// Cleanup observation only. The host retains native worker/reap ownership and
/// must not infer complete cleanup merely from local transport closure.
#[derive(Clone)]
pub enum NativeMcpPeerCompletion {
    #[cfg(test)]
    Script(Arc<std::sync::atomic::AtomicBool>),
    Stdio(crate::NativeOwnedWorkerCompletion),
    #[cfg(feature = "mcp-http")]
    Http(crate::mcp::http_peer::McpHttpPeerCompletion),
}
impl NativeMcpPeerCompletion {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        match self {
            #[cfg(test)]
            Self::Script(completion) => completion.load(std::sync::atomic::Ordering::Acquire),
            Self::Stdio(completion) => completion.is_complete(),
            #[cfg(feature = "mcp-http")]
            Self::Http(completion) => completion.is_complete(),
        }
    }
}
impl std::fmt::Debug for NativeMcpPeerCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeMcpPeerCompletion { <redacted> }")
    }
}
