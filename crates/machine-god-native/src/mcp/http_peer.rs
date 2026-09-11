//! Owned HTTP MCP negotiation, sessions and explicitly polled notification streams.

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
mod head;
mod routing;
mod startup;
mod stream;
#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, McpHttpPeerError>;

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
    SessionExpired,
    ListenerUnsupported,
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

/// Explicit selected authority. The deadline bounds the entire peer lifetime,
/// independently of shorter startup/request deadlines; no ambient resolution.
pub struct McpHttpPeerOptions {
    pub destination: McpHttpDestination,
    pub trust: Option<McpHttpTrust>,
    pub headers: McpResolvedHeaders,
    pub clock: Arc<dyn McpHttpClock>,
    pub transport: TransportKind,
    pub lifetime_deadline: Instant,
}

/// Raw bytes preserve exact schema numbers; the envelope is routing data only.
pub struct McpHttpPeerFrame {
    bytes: Box<[u8]>,
    envelope: RpcEnvelope,
}
impl McpHttpPeerFrame {
    fn parse(bytes: Box<[u8]>) -> Result<Self> {
        let envelope = super::protocol::parse_envelope(&bytes, WireLimits::default())
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
/// A remote DELETE receipt is independent of local socket completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpHttpSessionTeardown {
    NotNeeded,
    NotAttempted,
    Confirmed,
    Unsupported,
    Ambiguous,
}

/// One serialized application lane and at most one persistent listener. All
/// socket work is driven by the caller; no detached listener task is spawned.
pub struct McpHttpPeer {
    options: McpHttpPeerOptions,
    destination: McpHttpDestination,
    protocol: NegotiatedProtocol,
    capabilities: McpPeerCapabilities,
    session: Option<Box<str>>,
    cancellation: CancellationToken,
    completion: McpHttpPeerCompletion,
    listener: Option<stream::Read>,
    listener_resume: stream::Resume,
    listener_reconnects: usize,
    next_id: Option<i64>,
    reserved: McpPendingToolReservation,
    runtimes: Vec<Arc<McpSubmissionRuntime>>,
    notifications: VecDeque<McpHttpPeerFrame>,
    notification_bytes: usize,
    operation_events: usize,
    closed: bool,
}
impl McpHttpPeer {
    /// Performs actual bounded discovery/initialization and legacy notifications.
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
    /// Stops local streams and invalidates reservations; does not issue DELETE.
    pub fn close(&mut self) {
        self.closed = true;
        self.reserved = McpPendingToolReservation::default();
        self.listener.take();
        self.runtimes.clear();
        self.notifications.clear();
        self.notification_bytes = 0;
        self.completion.0.closed.cancel();
    }
    /// Explicit cleanup authority, under its own deadline even after cancellation.
    /// It never retries DELETE or asserts that application effects were revoked.
    pub async fn shutdown(&mut self, deadline: Instant) -> McpHttpSessionTeardown {
        routing::shutdown(self, deadline).await
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
        self.check(self.options.lifetime_deadline)?;
        self.make_head(None, None)
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
    /// Starts the optional legacy Streamable HTTP notification GET. Deprecated
    /// HTTP+SSE already owns its required listener; modern HTTP is stateless.
    /// # Errors
    /// Rejects unsupported GET, expiration, changed session and transport errors.
    pub async fn start_listener(&mut self, deadline: Instant) -> Result<()> {
        routing::start_listener(self, deadline).await
    }
    /// Drives the owned listener until one untrusted notification is available.
    /// # Errors
    /// Rejects retired listeners, malformed events and exhausted bounds.
    pub async fn next_notification(&mut self, deadline: Instant) -> Result<McpHttpPeerFrame> {
        routing::next_notification(self, deadline).await
    }
    pub fn take_notification(&mut self) -> Option<McpHttpPeerFrame> {
        let frame = self.notifications.pop_front()?;
        self.notification_bytes -= frame.bytes.len();
        Some(frame)
    }
    fn check(&self, deadline: Instant) -> Result<()> {
        if self.closed {
            return Err(McpHttpPeerError::Closed);
        }
        if self.cancellation.is_cancelled() {
            return Err(McpHttpPeerError::Cancelled);
        }
        if self.options.clock.now() >= deadline.min(self.options.lifetime_deadline) {
            return Err(McpHttpPeerError::Deadline);
        }
        Ok(())
    }
    fn available(&self) -> Result<()> {
        self.check(self.options.lifetime_deadline)?;
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
    fn make_head(
        &self,
        method: Option<&str>,
        resume: Option<&str>,
    ) -> Result<McpSubmissionHttpHead> {
        let mut headers: Vec<_> = self.options.headers.iter().collect();
        if self.protocol.sends_http_protocol_header() {
            headers.push((
                "mcp-protocol-version",
                self.protocol.version.as_str().as_bytes(),
            ));
        }
        if self.protocol.version == ProtocolVersion::Modern
            && let Some(method) = method
        {
            headers.push(("mcp-method", method.as_bytes()));
        }
        if let Some(session) = &self.session {
            headers.push(("mcp-session-id", session.as_bytes()));
        }
        if let Some(resume) = resume {
            headers.push(("last-event-id", resume.as_bytes()));
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
        let connection = McpHttpConnection::from_prepared_head(
            self.destination.clone(),
            head,
            self.options.trust.clone(),
            McpHttpLimits::default(),
            self.cancellation.clone(),
            deadline.min(self.options.lifetime_deadline),
            self.options.clock.clone(),
        )?;
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
