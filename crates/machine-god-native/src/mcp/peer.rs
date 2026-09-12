//! Owned stdio negotiation and serialized, exactly correlated request routing.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use machine_god_core::{BoxFuture, CancellationToken};

use super::lifetime::McpPeerLifetime;
use super::pagination::{McpCatalogKind, McpCatalogLimits, McpRawCatalog};
use super::protocol::{NegotiatedProtocol, NegotiationFailure, RpcEnvelope, RpcId};
use super::stdio::{McpStdioConnection, McpStdioError, McpStdioLaunch};
use super::submission::{
    McpPendingToolReservation, McpSubmission, McpSubmissionRuntime, McpToolReservation,
};
use crate::{NativeOwnedWorkerCompletion, NativeOwnedWorkerScope};

mod capabilities;
mod feature;
mod routing;
mod startup;
mod subscription;
#[cfg(test)]
mod tests;
pub use capabilities::McpPeerCapabilities;

/// Explicit host-selected asynchronous timer. Implementations must be inert
/// before polling and must retain ownership of any timer work after abandonment.
pub trait McpPeerTimer: Send + Sync {
    /// Monotonic observation in the native `Instant` domain. Existing timers
    /// retain their native clock; configured hosts may inject the same domain.
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()>;
}

pub type McpStdioCompletionObserver =
    Arc<dyn Fn(NativeOwnedWorkerCompletion) -> bool + Send + Sync>;

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
    Feature(super::feature::McpFeatureCodecError),
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

/// One connection owner and one mutable request lane. IDs never repeat during
/// its connection lifetime. Notifications remain untrusted bounded data.
pub struct McpStdioPeer {
    connection: McpStdioConnection,
    protocol: NegotiatedProtocol,
    capabilities: McpPeerCapabilities,
    timer: Arc<dyn McpPeerTimer>,
    cancellation: CancellationToken,
    lifetime: McpPeerLifetime,
    feature_identity: Arc<()>,
    next_id: Option<i64>,
    reserved: McpPendingToolReservation,
    notifications: VecDeque<RpcEnvelope>,
    notification_bytes: usize,
    pending_replies: routing::Replies,
    subscription: Box<subscription::State>,
    closed: bool,
}
pub(crate) struct McpStdioPeerReadiness {
    connection: super::stdio::McpStdioConnectionReadiness,
    cancellation: CancellationToken,
    lifetime: McpPeerLifetime,
    timer: Arc<dyn McpPeerTimer>,
}
impl McpStdioPeerReadiness {
    pub(crate) fn is_ready(&self) -> bool {
        !self.cancellation.is_cancelled()
            && !self.lifetime.is_expired(self.timer.now())
            && self.connection.is_ready()
    }
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
    pub(crate) fn readiness(&self) -> McpStdioPeerReadiness {
        McpStdioPeerReadiness {
            connection: self.connection.readiness(),
            cancellation: self.cancellation.clone(),
            lifetime: self.lifetime,
            timer: self.timer.clone(),
        }
    }
    /// Narrow trusted native ownership without resetting an existing expiry.
    pub(crate) fn restrict_lifetime(&mut self, lifetime: McpPeerLifetime) {
        self.lifetime = match self.lifetime {
            McpPeerLifetime::OwnerControlled => lifetime,
            McpPeerLifetime::Until(deadline) => {
                McpPeerLifetime::Until(lifetime.constrain(deadline))
            }
        };
    }
    /// Executes only the seven typed feature actions against native-selected
    /// catalogs. IDs are consumed once; failed or abandoned sends are not replayed.
    /// # Errors
    /// Rejects invalid identity/data, insufficient wire bounds, expired authority,
    /// malformed responses, correlation failure, cancellation or deadlines.
    pub async fn feature(
        &mut self,
        request: &crate::McpFeatureRequest,
        server: &str,
        catalogs: &[super::catalog::McpDescriptorCatalog],
        authority: super::control::McpFeatureControlAuthority,
        options: super::control::McpFeatureOperationOptions,
        deadline: Instant,
    ) -> Result<super::control::McpFeatureReply> {
        self.feature_round(request, server, catalogs, authority, options, deadline)
            .await
            .map(super::control::McpFeatureRound::into_reply)
    }
    pub(crate) async fn feature_round(
        &mut self,
        request: &crate::McpFeatureRequest,
        server: &str,
        catalogs: &[super::catalog::McpDescriptorCatalog],
        authority: super::control::McpFeatureControlAuthority,
        options: super::control::McpFeatureOperationOptions,
        deadline: Instant,
    ) -> Result<super::control::McpFeatureRound> {
        feature::execute(
            self, request, server, catalogs, authority, options, deadline,
        )
        .await
    }
    pub(crate) async fn resume_feature(
        &mut self,
        round: super::control::McpFeatureRound,
        responses: super::mrtr::McpValidatedResponses,
        deadline: Instant,
    ) -> Result<super::control::McpFeatureRound> {
        feature::resume(self, round, responses, deadline).await
    }
    /// Configured startup with pre-effect completion observation. The returned
    /// deadline also bounds initial tools catalog loading. Discovery does not
    /// downgrade or restart the selected process.
    /// Complete-startup retry policy remains with the caller, after cleanup.
    /// # Errors
    /// Rejects invalid timeout, failed observation, startup or negotiation.
    pub async fn connect_observed(
        factory: &mut dyn McpStdioLaunchFactory,
        host: NativeOwnedWorkerScope,
        timer: Arc<dyn McpPeerTimer>,
        cancellation: CancellationToken,
        outer_deadline: Instant,
        startup_timeout: Duration,
        observer: McpStdioCompletionObserver,
    ) -> Result<(Self, Instant)> {
        startup::connect_observed(
            factory,
            host,
            timer,
            cancellation,
            Some(outer_deadline),
            startup_timeout,
            observer,
        )
        .await
    }
    /// Uses each configured attempt's full finite timeout, without an overall
    /// startup cap. Cancellation and observed cleanup still bound ownership.
    /// # Errors
    /// Rejects invalid timeout, failed observation, startup or negotiation.
    pub async fn connect_configured_observed(
        factory: &mut dyn McpStdioLaunchFactory,
        host: NativeOwnedWorkerScope,
        timer: Arc<dyn McpPeerTimer>,
        cancellation: CancellationToken,
        startup_timeout: Duration,
        observer: McpStdioCompletionObserver,
    ) -> Result<(Self, Instant)> {
        startup::connect_observed(
            factory,
            host,
            timer,
            cancellation,
            None,
            startup_timeout,
            observer,
        )
        .await
    }
    /// Drives modern discovery over one actual owned connection, without restart.
    /// `discovery_timeout` is a subdeadline, never the overall deadline.
    ///
    /// # Errors
    /// Rejects malformed success, uncorrelated replies, resource exhaustion,
    /// cancelled/deadline startup, or unsupported discovery responses.
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
        *self.subscription = subscription::State::default();
        self.reserved = McpPendingToolReservation::default();
        self.connection.close();
        self.pending_replies.clear();
    }
    /// Registers exact executable allocations without granting permission.
    ///
    /// # Errors
    /// Rejects excessive registrations or a closed connection.
    pub fn admit_runtimes(&self, runtimes: Vec<Arc<McpSubmissionRuntime>>) -> Result<()> {
        self.check_lifetime()?;
        self.connection.admit_runtimes(runtimes).map_err(Into::into)
    }
    pub(crate) fn prepare_runtime_set(
        &self,
        runtimes: Vec<Arc<McpSubmissionRuntime>>,
    ) -> Result<crate::mcp::stdio::PreparedStdioRuntimeSet<'_>> {
        self.check_lifetime()?;
        self.connection
            .prepare_runtime_set(runtimes)
            .map_err(Into::into)
    }
    /// Reserves the one pending application ID before native proof preparation.
    /// A failed preparation may discard the reservation; its ID is never reused.
    ///
    /// # Errors
    /// Rejects a second reservation, closed peer or integer exhaustion.
    pub fn reserve_tool_id(&mut self) -> Result<RpcId> {
        self.check_available()?;
        if self.reserved.is_live() {
            return Err(McpPeerError::Capacity);
        }
        let id = self.allocate()?;
        self.reserved = McpPendingToolReservation::manual(id.clone());
        Ok(id)
    }
    /// Reserves an application ID whose ownership moves through the typed
    /// permission request. Abandonment releases the unsent slot without I/O.
    ///
    /// # Errors
    /// Rejects 64 live leases, a manual reservation, closed peer or ID exhaustion.
    pub fn reserve_tool(&mut self) -> Result<McpToolReservation> {
        self.check_available()?;
        if !self.reserved.has_capacity() {
            return Err(McpPeerError::Capacity);
        }
        let id = self.allocate()?;
        self.reserved.reserve(id).ok_or(McpPeerError::Capacity)
    }
    /// Discards only a manual reservation; owned request leases are unaffected.
    pub fn discard_tool_id(&mut self) {
        self.reserved.discard_manual();
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
        self.call_frame(submission, deadline)
            .await
            .map(super::stdio::McpStdioFrame::into_envelope)
    }
    /// Retains the exact correlated response bytes for result/schema admission.
    /// # Errors
    /// Uses the same proof, correlation, deadline and no-replay rules as `call`.
    pub async fn call_frame(
        &mut self,
        submission: McpSubmission,
        deadline: Instant,
    ) -> Result<super::stdio::McpStdioFrame> {
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
    /// Observes one untrusted notification without acquiring continuation authority.
    /// Dropping or timing out this idle observation preserves a healthy peer's
    /// worker-owned input and pending fixed unsupported-request replies. Retained
    /// replies keep their original finite write deadlines across observations.
    /// # Errors
    /// Rejects owner cancellation/expiry, malformed or foreign frames, EOF,
    /// failed reply writes and exhausted bounds. An idle timeout alone is nonfatal.
    pub async fn next_notification(&mut self, deadline: Instant) -> Result<RpcEnvelope> {
        routing::next_notification(self, deadline).await
    }
    fn check_owner(&self) -> Result<()> {
        if self.closed {
            return Err(McpPeerError::Closed);
        }
        if self.cancellation.is_cancelled() {
            return Err(McpPeerError::Cancelled);
        }
        self.check_lifetime()
    }
    fn check_available(&self) -> Result<()> {
        if self.closed || self.cancellation.is_cancelled() {
            return Err(McpPeerError::Closed);
        }
        self.check_lifetime()?;
        if self.reserved.blocks_control() {
            return Err(McpPeerError::Capacity);
        }
        Ok(())
    }
    fn check_lifetime(&self) -> Result<()> {
        if self
            .lifetime
            .deadline()
            .is_some_and(|deadline| self.timer.now() >= deadline)
        {
            return Err(McpPeerError::Deadline);
        }
        Ok(())
    }
    fn allocate(&mut self) -> Result<RpcId> {
        let id = self.next_id.ok_or(McpPeerError::Capacity)?;
        self.next_id = id.checked_add(1);
        Ok(RpcId::Integer(id))
    }
}
