//! Owned stdio negotiation and serialized, exactly correlated request routing.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use machine_god_core::{BoxFuture, CancellationToken};

use super::pagination::{McpCatalogKind, McpCatalogLimits, McpRawCatalog};
use super::protocol::{NegotiatedProtocol, NegotiationFailure, RpcEnvelope, RpcId};
use super::stdio::{McpStdioConnection, McpStdioError, McpStdioLaunch};
use super::submission::{McpSubmission, McpSubmissionRuntime};
use crate::{NativeOwnedWorkerCompletion, NativeOwnedWorkerScope};

mod capabilities;
mod routing;
mod startup;
#[cfg(test)]
mod tests;
pub use capabilities::McpPeerCapabilities;

/// Explicit host-selected asynchronous timer. Implementations must be inert
/// before polling and must retain ownership of any timer work after abandonment.
pub trait McpPeerTimer: Send + Sync {
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()>;
}

/// Explicit authority to reopen the same selected server during negotiation.
/// The factory must not switch server/configuration identity between attempts.
pub trait McpStdioLaunchFactory: Send {
    /// Called on a polled startup, never while constructing a peer future.
    ///
    /// # Errors
    /// Return a redacted error when selected launch authority is unavailable.
    fn launch(&mut self) -> std::result::Result<McpStdioLaunch, McpStdioError>;
}
impl<F: FnMut() -> std::result::Result<McpStdioLaunch, McpStdioError> + Send> McpStdioLaunchFactory
    for F
{
    fn launch(&mut self) -> std::result::Result<McpStdioLaunch, McpStdioError> {
        self()
    }
}

/// Redacted peer failures. Protocol-error response payloads are returned as data,
/// not copied into diagnostics. Failure never authorizes application replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpPeerError {
    Transport(McpStdioError),
    Negotiation(NegotiationFailure),
    InvalidResult,
    Correlation,
    Capacity,
    Cancelled,
    Deadline,
    Closed,
}
impl fmt::Display for McpPeerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP peer operation failed")
    }
}
impl std::error::Error for McpPeerError {}
impl From<McpStdioError> for McpPeerError {
    fn from(value: McpStdioError) -> Self {
        Self::Transport(value)
    }
}
type Result<T> = std::result::Result<T, McpPeerError>;

/// One connection owner and one mutable request lane. IDs never repeat even
/// across startup restarts. Notifications remain untrusted bounded data.
pub struct McpStdioPeer {
    connection: McpStdioConnection,
    protocol: NegotiatedProtocol,
    capabilities: McpPeerCapabilities,
    timer: Arc<dyn McpPeerTimer>,
    cancellation: CancellationToken,
    next_id: Option<i64>,
    reserved: Option<RpcId>,
    notifications: VecDeque<RpcEnvelope>,
    notification_bytes: usize,
    closed: bool,
}
impl fmt::Debug for McpStdioPeer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpStdioPeer")
            .field("protocol", &self.protocol)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}
impl McpStdioPeer {
    /// Drives discovery/initialize over actual owned pipes. A restart occurs
    /// only after positively settled old-connection cleanup and live control.
    /// `discovery_timeout` is a subdeadline, never the overall deadline.
    ///
    /// # Errors
    /// Rejects malformed success, uncorrelated replies, resource exhaustion,
    /// cancelled/deadline startup, or non-admitted fallback observations.
    pub async fn connect(
        factory: &mut dyn McpStdioLaunchFactory,
        host: NativeOwnedWorkerScope,
        timer: Arc<dyn McpPeerTimer>,
        cancellation: CancellationToken,
        deadline: Instant,
        discovery_timeout: Duration,
    ) -> Result<Self> {
        startup::connect(
            factory,
            host,
            timer,
            cancellation,
            deadline,
            discovery_timeout,
        )
        .await
    }

    #[must_use]
    pub const fn protocol(&self) -> NegotiatedProtocol {
        self.protocol
    }
    #[must_use]
    pub const fn capabilities(&self) -> McpPeerCapabilities {
        self.capabilities
    }
    #[must_use]
    pub fn completion(&self) -> NativeOwnedWorkerCompletion {
        self.connection.completion()
    }
    /// Retires the connection; its completion still includes deferred reap.
    pub fn close(&mut self) {
        self.closed = true;
        self.connection.close();
    }
    /// Registers exact executable allocations without granting permission.
    ///
    /// # Errors
    /// Rejects excessive registrations or a closed connection.
    pub fn admit_runtimes(&self, runtimes: Vec<Arc<McpSubmissionRuntime>>) -> Result<()> {
        self.connection.admit_runtimes(runtimes).map_err(Into::into)
    }
    /// Reserves the one pending application ID before native proof preparation.
    /// A failed preparation may discard the reservation; its ID is never reused.
    ///
    /// # Errors
    /// Rejects a second reservation, closed peer or integer exhaustion.
    pub fn reserve_tool_id(&mut self) -> Result<RpcId> {
        self.check_available()?;
        let id = self.allocate()?;
        self.reserved = Some(id.clone());
        Ok(id)
    }
    /// Discards an unsent reservation, without making its ID reusable.
    pub fn discard_tool_id(&mut self) {
        self.reserved = None;
    }
    /// Sends the exact reserved proof-bearing request once and correlates its
    /// response. Any abandoned polled request closes the connection.
    ///
    /// # Errors
    /// Rejects foreign IDs/proofs, stale responses, cancellation and deadlines.
    pub async fn call(
        &mut self,
        submission: McpSubmission,
        deadline: Instant,
    ) -> Result<RpcEnvelope> {
        routing::call(self, submission, deadline).await
    }
    /// Loads every bounded catalog page without normalizing raw schema numbers.
    /// Publication and executable schema admission remain caller responsibilities.
    ///
    /// # Errors
    /// Rejects malformed pages, unsupported capability, stale IDs or deadlines.
    pub async fn catalog(
        &mut self,
        kind: McpCatalogKind,
        limits: McpCatalogLimits,
        timestamp_origin: Instant,
        deadline: Instant,
    ) -> Result<McpRawCatalog> {
        routing::catalog(self, kind, limits, timestamp_origin, deadline).await
    }
    /// Takes one untrusted notification observed while driving a request. The
    /// peer does not invent subscription or continuation authority from it.
    pub fn take_notification(&mut self) -> Option<RpcEnvelope> {
        let envelope = self.notifications.pop_front()?;
        // Charge is deliberately conservative until the complete queue drains.
        if self.notifications.is_empty() {
            self.notification_bytes = 0;
        }
        Some(envelope)
    }
    fn check_available(&self) -> Result<()> {
        if self.closed || self.cancellation.is_cancelled() {
            return Err(McpPeerError::Closed);
        }
        if self.reserved.is_some() {
            return Err(McpPeerError::Capacity);
        }
        Ok(())
    }
    fn allocate(&mut self) -> Result<RpcId> {
        let id = self.next_id.ok_or(McpPeerError::Capacity)?;
        self.next_id = id.checked_add(1);
        Ok(RpcId::Integer(id))
    }
}
