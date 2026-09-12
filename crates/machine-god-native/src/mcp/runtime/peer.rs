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
    pub(super) fn readiness(&self) -> NativeMcpPeerReadiness {
        match self {
            #[cfg(test)]
            Self::Script(peer) => NativeMcpPeerReadiness::Script(peer.closed.clone()),
            Self::Stdio(peer) => NativeMcpPeerReadiness::Stdio(peer.readiness()),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => NativeMcpPeerReadiness::Http(peer.readiness()),
        }
    }
    pub(super) async fn feature(
        &mut self,
        request: &crate::McpFeatureRequest,
        server: &str,
        catalogs: &[crate::mcp::catalog::McpDescriptorCatalog],
        authority: crate::mcp::control::McpFeatureControlAuthority,
        options: crate::mcp::control::McpFeatureOperationOptions,
        deadline: std::time::Instant,
    ) -> super::features::Result<crate::mcp::control::McpFeatureReply> {
        self.feature_round(request, server, catalogs, authority, options, deadline)
            .await
            .map(crate::mcp::control::McpFeatureRound::into_reply)
    }
    pub(super) async fn feature_round(
        &mut self,
        request: &crate::McpFeatureRequest,
        server: &str,
        catalogs: &[crate::mcp::catalog::McpDescriptorCatalog],
        authority: crate::mcp::control::McpFeatureControlAuthority,
        options: crate::mcp::control::McpFeatureOperationOptions,
        deadline: std::time::Instant,
    ) -> super::features::Result<crate::mcp::control::McpFeatureRound> {
        match self {
            #[cfg(test)]
            Self::Script(peer) => {
                peer.feature_round(request, server, catalogs, &authority, options)
            }
            Self::Stdio(peer) => peer
                .feature_round(request, server, catalogs, authority, options, deadline)
                .await
                .map_err(|error| match error {
                    crate::mcp::peer::McpPeerError::Feature(error) => error.into(),
                    _ => Error::Unavailable.into(),
                }),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer
                .feature_round(request, server, catalogs, authority, options, deadline)
                .await
                .map_err(|error| match error {
                    crate::mcp::http_peer::McpHttpPeerError::Feature(error) => error.into(),
                    _ => Error::Unavailable.into(),
                }),
        }
    }

    pub(super) async fn resume_feature(
        &mut self,
        round: crate::mcp::control::McpFeatureRound,
        responses: crate::mcp::mrtr::McpValidatedResponses,
        deadline: std::time::Instant,
    ) -> super::features::Result<crate::mcp::control::McpFeatureRound> {
        match self {
            #[cfg(test)]
            Self::Script(peer) => peer.resume_feature(round, responses),
            Self::Stdio(peer) => peer
                .resume_feature(round, responses, deadline)
                .await
                .map_err(|error| match error {
                    crate::mcp::peer::McpPeerError::Feature(error) => error.into(),
                    _ => Error::Unavailable.into(),
                }),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer
                .resume_feature(round, responses, deadline)
                .await
                .map_err(|error| match error {
                    crate::mcp::http_peer::McpHttpPeerError::Feature(error) => error.into(),
                    _ => Error::Unavailable.into(),
                }),
        }
    }

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

pub(super) enum NativeMcpPeerReadiness {
    #[cfg(test)]
    Script(Arc<std::sync::atomic::AtomicBool>),
    Stdio(crate::mcp::peer::McpStdioPeerReadiness),
    #[cfg(feature = "mcp-http")]
    Http(crate::mcp::http_peer::McpHttpPeerReadiness),
}
impl NativeMcpPeerReadiness {
    pub(super) fn is_ready(&self) -> bool {
        match self {
            #[cfg(test)]
            Self::Script(closed) => !closed.load(std::sync::atomic::Ordering::Acquire),
            Self::Stdio(peer) => peer.is_ready(),
            #[cfg(feature = "mcp-http")]
            Self::Http(peer) => peer.is_ready(),
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
