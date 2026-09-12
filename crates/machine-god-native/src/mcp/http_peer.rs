//! Owned HTTP MCP negotiation, modern POST exchanges and queued notifications.

use super::{
    headers::McpResolvedHeaders,
    http::{
        McpHttpClock, McpHttpConnection, McpHttpControl, McpHttpDestination, McpHttpError,
        McpHttpLimits, McpHttpObservation, McpHttpTrust,
    },
    pagination::{McpCatalogKind, McpCatalogLimits, McpRawCatalog},
    peer::McpPeerCapabilities,
    protocol::{
        NegotiatedProtocol, NegotiationFailure, ProtocolVersion, RpcEnvelope, RpcId, RpcKind,
        TransportKind, WireLimits,
    },
    submission::{
        McpPendingToolReservation, McpSubmission, McpSubmissionHttpHead, McpSubmissionRuntime,
        McpToolReservation,
    },
};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
mod feature;
mod head;
mod routing;
mod startup;
mod stream;
mod subscription;
#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, McpHttpPeerError>;
pub type McpHttpCompletionObserver = Arc<dyn Fn(McpHttpPeerCompletion) -> bool + Send + Sync>;

/// Bounded challenge observations for separately authorized authentication.
pub struct McpHttpAuthentication {
    pub status: u16,
    pub challenges: Box<[Box<[u8]>]>,
}
/// Fixed diagnostics; remote bodies and challenge data are never formatted.
pub enum McpHttpPeerError {
    Transport(McpHttpError),
    Negotiation(NegotiationFailure),
    Authentication(McpHttpAuthentication),
    Invalid,
    Protocol,
    Correlation,
    Limit,
    Cancelled,
    Deadline,
    Closed,
    Redirect,
    Feature(super::feature::McpFeatureCodecError),
}
impl fmt::Display for McpHttpPeerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP HTTP peer operation failed")
    }
}
impl std::error::Error for McpHttpPeerError {}
impl From<McpHttpError> for McpHttpPeerError {
    fn from(value: McpHttpError) -> Self {
        Self::Transport(value)
    }
}

/// Explicit selected authority. Owner cancellation or an explicitly selected
/// expiry bounds the peer, independently of finite startup/request deadlines.
pub struct McpHttpPeerOptions {
    pub destination: McpHttpDestination,
    pub trust: Option<McpHttpTrust>,
    pub headers: McpResolvedHeaders,
    pub clock: Arc<dyn McpHttpClock>,
    pub transport: TransportKind,
    pub lifetime: super::lifetime::McpPeerLifetime,
}

/// Raw bytes preserve exact schema numbers; the envelope is routing data only.
pub struct McpHttpPeerFrame {
    bytes: Box<[u8]>,
    envelope: RpcEnvelope,
}
impl McpHttpPeerFrame {
    #[cfg(test)]
    fn parse(bytes: Box<[u8]>) -> Result<Self> {
        Self::parse_with_limits(bytes, WireLimits::default())
    }
    fn parse_with_limits(bytes: Box<[u8]>, limits: WireLimits) -> Result<Self> {
        let envelope = super::protocol::parse_envelope(&bytes, limits)
            .map_err(|_| McpHttpPeerError::Protocol)?;
        Ok(Self { bytes, envelope })
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    #[must_use]
    pub fn envelope(&self) -> &RpcEnvelope {
        &self.envelope
    }
    #[must_use]
    pub fn into_envelope(self) -> RpcEnvelope {
        self.envelope
    }
}

#[derive(Default)]
struct Completion {
    closed: CancellationToken,
    exchanges: Mutex<Vec<McpHttpObservation>>,
}
pub(crate) struct McpHttpPeerReadiness {
    closed: CancellationToken,
    cancellation: CancellationToken,
    lifetime: super::lifetime::McpPeerLifetime,
    clock: Arc<dyn McpHttpClock>,
    authentication: Option<Arc<super::auth::McpAuthLease>>,
}
impl McpHttpPeerReadiness {
    pub(crate) fn is_ready(&self) -> bool {
        !self.closed.is_cancelled()
            && !self.cancellation.is_cancelled()
            && !self.lifetime.is_expired(self.clock.now())
            && self
                .authentication
                .as_ref()
                .is_none_or(|lease| lease.access_token().is_ok())
    }
}
/// Local owner completion, not evidence that remote operations were revoked.
#[derive(Clone)]
pub struct McpHttpPeerCompletion(Arc<Completion>);
impl McpHttpPeerCompletion {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.0.closed.is_cancelled()
            && self
                .0
                .exchanges
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .all(McpHttpObservation::is_complete)
    }
    #[must_use]
    pub fn completed(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.0.closed.cancelled().await;
            let exchanges = self
                .0
                .exchanges
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            for exchange in exchanges {
                exchange.completed().await;
            }
        })
    }
}
/// One serialized application lane. All
/// socket work is driven by the caller; no detached listener task is spawned.
pub struct McpHttpPeer {
    options: McpHttpPeerOptions,
    destination: McpHttpDestination,
    protocol: NegotiatedProtocol,
    capabilities: McpPeerCapabilities,
    cancellation: CancellationToken,
    completion: McpHttpPeerCompletion,
    feature_identity: Arc<()>,
    next_id: Option<i64>,
    reserved: McpPendingToolReservation,
    runtimes: Vec<Arc<McpSubmissionRuntime>>,
    notifications: VecDeque<McpHttpPeerFrame>,
    notification_bytes: usize,
    operation_events: usize,
    closed: bool,
    configured_timeouts: bool,
    response_limits: WireLimits,
    feature_authority: Option<super::control::McpFeatureControlAuthority>,
    authentication: Option<Arc<super::auth::McpAuthLease>>,
    subscription: Option<subscription::Subscription>,
}
impl McpHttpPeer {
    pub(crate) fn readiness(&self) -> McpHttpPeerReadiness {
        McpHttpPeerReadiness {
            closed: self.completion.0.closed.clone(),
            cancellation: self.cancellation.clone(),
            lifetime: self.options.lifetime,
            clock: self.options.clock.clone(),
            authentication: self.authentication.clone(),
        }
    }
    /// Executes a native-selected typed feature request with exact fixed HTTP
    /// projection and guarded transport writes. No application replay occurs.
    /// # Errors
    /// Rejects invalid data, stale authority, failed correlation or transport.
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
    /// Configured startup with bounded pre-effect cleanup observation. Returns
    /// the selected attempt deadline for initial tools catalog loading; neither
    /// observation nor successful startup grants application execution authority.
    /// `first_attempt_deadline` preserves time already spent on auth/DNS and
    /// can only shorten the discovery budget.
    /// # Errors
    /// Rejects invalid timeout, observer capacity, transport or negotiation.
    pub async fn connect_observed(
        options: McpHttpPeerOptions,
        cancellation: CancellationToken,
        outer_deadline: Instant,
        startup_timeout: Duration,
        first_attempt_deadline: Option<Instant>,
        observer: McpHttpCompletionObserver,
    ) -> Result<(Self, Instant)> {
        Self::connect_selected_observed(
            options,
            cancellation,
            Some(outer_deadline),
            startup_timeout,
            first_attempt_deadline,
            observer,
            None,
        )
        .await
    }
    /// Uses fresh finite configured attempt deadlines without an additional
    /// overall startup timeout. Peer ownership and explicit expiry still apply.
    /// # Errors
    /// Rejects invalid timeout, observer capacity, transport or negotiation.
    pub async fn connect_configured_observed(
        options: McpHttpPeerOptions,
        cancellation: CancellationToken,
        startup_timeout: Duration,
        first_attempt_deadline: Option<Instant>,
        observer: McpHttpCompletionObserver,
    ) -> Result<(Self, Instant)> {
        Self::connect_selected_observed(
            options,
            cancellation,
            None,
            startup_timeout,
            first_attempt_deadline,
            observer,
            None,
        )
        .await
    }
    // The concrete native startup supplies the exact lease that resolved these
    // headers. Install it before discovery, not after the first socket effect.
    pub(crate) async fn connect_selected_observed(
        options: McpHttpPeerOptions,
        cancellation: CancellationToken,
        outer_deadline: Option<Instant>,
        startup_timeout: Duration,
        first_attempt_deadline: Option<Instant>,
        observer: McpHttpCompletionObserver,
        authentication: Option<Arc<super::auth::McpAuthLease>>,
    ) -> Result<(Self, Instant)> {
        startup::connect_observed(
            options,
            cancellation,
            outer_deadline,
            startup_timeout,
            first_attempt_deadline,
            observer,
            authentication,
        )
        .await
    }
    /// Performs actual bounded modern discovery.
    /// # Errors
    /// Rejects invalid authority, malformed negotiation, authentication, redirects,
    /// cancellation, deadlines and resource exhaustion. No application replay.
    pub async fn connect(
        options: McpHttpPeerOptions,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> Result<Self> {
        startup::connect(options, cancellation, deadline).await
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
    pub fn completion(&self) -> McpHttpPeerCompletion {
        self.completion.clone()
    }
    /// Stops local streams and invalidates reservations without network effects.
    pub fn close(&mut self) {
        self.closed = true;
        self.close_subscription();
        self.reserved = McpPendingToolReservation::default();
        self.runtimes.clear();
        self.notifications.clear();
        self.notification_bytes = 0;
        self.completion.0.closed.cancel();
    }
    /// Registers exact executable allocation identities, not execution grants.
    /// # Errors
    /// Rejects closed peers, duplicate identities and over 2,048 allocations.
    pub fn admit_runtimes(&mut self, runtimes: Vec<Arc<McpSubmissionRuntime>>) -> Result<()> {
        self.available()?;
        if runtimes.len() > 2048
            || runtimes.iter().enumerate().any(|(index, value)| {
                runtimes[..index]
                    .iter()
                    .any(|prior| Arc::ptr_eq(prior, value))
            })
        {
            return Err(McpHttpPeerError::Limit);
        }
        self.runtimes = runtimes;
        Ok(())
    }
    /// # Errors
    /// Rejects a second reservation, retirement or integer exhaustion.
    pub fn reserve_tool_id(&mut self) -> Result<RpcId> {
        self.available()?;
        if self.reserved.is_live() {
            return Err(McpHttpPeerError::Limit);
        }
        let id = self.allocate()?;
        self.reserved = McpPendingToolReservation::manual(id.clone());
        Ok(id)
    }
    /// Reserves an application ID whose ownership moves through the typed
    /// permission request. Abandonment releases the unsent slot without I/O.
    /// # Errors
    /// Rejects 64 live leases, a manual reservation, retirement or ID exhaustion.
    pub fn reserve_tool(&mut self) -> Result<McpToolReservation> {
        self.available()?;
        if !self.reserved.has_capacity() {
            return Err(McpHttpPeerError::Limit);
        }
        let id = self.allocate()?;
        self.reserved.reserve(id).ok_or(McpHttpPeerError::Limit)
    }
    /// Discards only a manual reservation; owned request leases are unaffected.
    pub fn discard_tool_id(&mut self) {
        self.reserved.discard_manual();
    }
    /// Selected immutable base head for typed pre-permission projection.
    /// # Errors
    /// Rejects closed/cancelled peers and invalid protocol header composition.
    pub fn request_head(&self) -> Result<McpSubmissionHttpHead> {
        self.check_owner()?;
        self.make_head(None)
    }
    /// Uses the reserved ID and an admitted runtime allocation. The projected
    /// head must preserve every selected base field; final bytes remain guarded.
    /// # Errors
    /// Rejects changed authority, stale proofs, correlation, I/O and deadlines.
    pub async fn call(
        &mut self,
        submission: McpSubmission,
        head: Arc<McpSubmissionHttpHead>,
        deadline: Instant,
    ) -> Result<McpHttpPeerFrame> {
        routing::call(self, submission, head, deadline).await
    }
    /// Collects raw pages; schema admission/publication remain runtime-owned.
    /// # Errors
    /// Rejects malformed pages, unsupported capabilities and bounded failures.
    pub async fn catalog(
        &mut self,
        kind: McpCatalogKind,
        limits: McpCatalogLimits,
        epoch: Instant,
        deadline: Instant,
    ) -> Result<McpRawCatalog> {
        routing::catalog(self, kind, limits, epoch, deadline).await
    }
    /// Takes an already queued notification; does not acquire network authority.
    pub fn take_notification(&mut self) -> Option<McpHttpPeerFrame> {
        let frame = self.notifications.pop_front()?;
        self.notification_bytes -= frame.bytes.len();
        Some(frame)
    }
    /// Starts one modern POST subscription, retaining the actual SSE stream.
    /// The returned ID is not readiness: the owner must install it in the shared
    /// refresh policy and admit its exact acknowledgement before relying on it.
    /// # Errors
    /// Rejects an existing listener, stale authority or failed request/head.
    pub async fn start_subscription(
        &mut self,
        filters: &super::catalog_refresh::McpSubscriptionFilters,
        deadline: Instant,
    ) -> Result<RpcId> {
        subscription::start(self, filters, deadline).await
    }
    /// Polls one envelope without replacing or replaying the listen request.
    /// A caller deadline returns `None` with the original read still retained;
    /// `active_subscription` distinguishes this from terminal completion.
    /// Dropping this future also retains the exact partial read and socket.
    /// # Errors
    /// Rejects retired authority, malformed events or finite stream exhaustion.
    pub async fn poll_subscription(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<McpHttpPeerFrame>> {
        subscription::poll(self, deadline).await
    }
    /// The retained request identity, including while awaiting acknowledgement.
    #[must_use]
    pub fn active_subscription(&self) -> Option<RpcId> {
        self.subscription.as_ref().map(|state| state.id.clone())
    }
    /// Releases only the subscription socket, without any network request.
    pub fn close_subscription(&mut self) {
        self.subscription = None;
    }
    fn check(&self, deadline: Instant) -> Result<()> {
        self.check_owner()?;
        if self.options.clock.now() >= deadline {
            return Err(McpHttpPeerError::Deadline);
        }
        Ok(())
    }
    fn check_owner(&self) -> Result<()> {
        if self.closed {
            return Err(McpHttpPeerError::Closed);
        }
        if self.cancellation.is_cancelled() {
            return Err(McpHttpPeerError::Cancelled);
        }
        if self
            .authentication
            .as_ref()
            .is_some_and(|lease| lease.access_token().is_err())
        {
            return Err(McpHttpPeerError::Cancelled);
        }
        if self.options.lifetime.is_expired(self.options.clock.now()) {
            return Err(McpHttpPeerError::Deadline);
        }
        Ok(())
    }
    fn available(&self) -> Result<()> {
        self.check_owner()?;
        if self.reserved.blocks_control() {
            return Err(McpHttpPeerError::Limit);
        }
        Ok(())
    }
    fn allocate(&mut self) -> Result<RpcId> {
        let id = self.next_id.ok_or(McpHttpPeerError::Limit)?;
        self.next_id = id.checked_add(1);
        Ok(RpcId::Integer(id))
    }
    fn make_head(&self, method: Option<&str>) -> Result<McpSubmissionHttpHead> {
        let mut headers: Vec<_> = self.options.headers.iter().collect();
        headers.push((
            "mcp-protocol-version",
            self.protocol.version.as_str().as_bytes(),
        ));
        if let Some(method) = method {
            headers.push(("mcp-method", method.as_bytes()));
        }
        McpSubmissionHttpHead::new(self.destination.endpoint(), &headers)
            .map_err(|_| McpHttpPeerError::Invalid)
    }
    fn connection(
        &self,
        head: Arc<McpSubmissionHttpHead>,
        deadline: Instant,
    ) -> Result<McpHttpConnection> {
        self.check(deadline)?;
        let connect = if self.configured_timeouts {
            McpHttpConnection::from_configured_head
        } else {
            McpHttpConnection::from_prepared_head
        };
        let mut connection = connect(
            self.destination.clone(),
            head,
            self.options.trust.clone(),
            McpHttpLimits::default(),
            self.cancellation.clone(),
            self.options.lifetime.constrain(deadline),
            self.options.clock.clone(),
        )?;
        if let Some(authority) = &self.feature_authority {
            connection.guard_feature(authority.clone());
        }
        if let Some(lease) = &self.authentication {
            connection.guard_authentication(lease.clone());
        }
        let mut exchanges = std::mem::take(
            &mut *self
                .completion
                .0
                .exchanges
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        exchanges.retain(|exchange| !exchange.is_complete());
        let full = exchanges.len() >= 8;
        if full {
            *self
                .completion
                .0
                .exchanges
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = exchanges;
            return Err(McpHttpPeerError::Limit);
        }
        exchanges.push(connection.observation());
        *self
            .completion
            .0
            .exchanges
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = exchanges;
        Ok(connection)
    }
    fn retain(&mut self, frame: McpHttpPeerFrame) -> Result<()> {
        if self.notifications.len() == 64
            || frame.bytes.len() > (1024 * 1024usize).saturating_sub(self.notification_bytes)
        {
            return Err(McpHttpPeerError::Limit);
        }
        self.notification_bytes += frame.bytes.len();
        self.notifications.push_back(frame);
        Ok(())
    }
}
impl Drop for McpHttpPeer {
    fn drop(&mut self) {
        self.close();
    }
}

macro_rules! redacted_debug { ($($ty:ty),+ $(,)?) => {$(impl fmt::Debug for $ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(concat!(stringify!($ty), " { <redacted> }")) }
})+}; }
redacted_debug!(
    McpHttpAuthentication,
    McpHttpPeerError,
    McpHttpPeerOptions,
    McpHttpPeerFrame,
    McpHttpPeerCompletion,
    McpHttpPeer
);
