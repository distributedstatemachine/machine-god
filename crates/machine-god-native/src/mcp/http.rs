//! Single-use, explicitly addressed HTTP/1.1 exchanges with owned TLS sockets.

mod body;
mod control;
mod destination;
mod io;
mod response;

pub use body::McpHttpBody;
pub use control::McpHttpControl;
pub use destination::{McpHttpDestination, McpHttpTrust};
pub use response::{McpHttpHeaders, McpHttpResponse};

use crate::mcp::{
    endpoint::McpEndpoint,
    submission::{McpSubmission, McpSubmissionHttpHead, McpSubmissionRuntime},
};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, McpHttpError>;

/// Explicit monotonic time/timer authority. Futures must be inert before polling.
pub trait McpHttpClock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()>;
}
struct SystemClock;
impl McpHttpClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}

/// Fixed errors disclose no endpoint, request, header or credential data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpHttpError {
    Invalid,
    Limit,
    Cancelled,
    Deadline,
    Connect,
    Tls,
    Io,
    Protocol,
    Submission,
    Closed,
}
impl fmt::Display for McpHttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid MCP HTTP authority",
            Self::Limit => "MCP HTTP limit exceeded",
            Self::Cancelled => "MCP HTTP operation cancelled",
            Self::Deadline => "MCP HTTP deadline exceeded",
            Self::Connect => "MCP HTTP connection failed",
            Self::Tls => "MCP TLS connection failed",
            Self::Io => "MCP HTTP I/O failed",
            Self::Protocol => "invalid MCP HTTP response",
            Self::Submission => "MCP HTTP submission rejected",
            Self::Closed => "MCP HTTP exchange closed",
        })
    }
}
impl std::error::Error for McpHttpError {}

/// Positive per-exchange budgets; zero is never unlimited.
#[derive(Clone, Copy, Debug)]
pub struct McpHttpLimits {
    pub head_bytes: usize,
    pub header_count: usize,
    pub body_bytes: u64,
    pub wire_bytes: u64,
}
impl Default for McpHttpLimits {
    fn default() -> Self {
        Self {
            head_bytes: 64 * 1024,
            header_count: 128,
            body_bytes: 64 * 1024 * 1024,
            wire_bytes: 128 * 1024 * 1024,
        }
    }
}
impl McpHttpLimits {
    fn validate(self) -> Result<Self> {
        if self.head_bytes == 0
            || self.head_bytes > 256 * 1024
            || self.header_count == 0
            || self.header_count > 256
            || self.body_bytes == 0
            || self.body_bytes > 1024 * 1024 * 1024
            || self.wire_bytes == 0
            || self.wire_bytes > 2 * 1024 * 1024 * 1024
        {
            return Err(McpHttpError::Limit);
        }
        Ok(self)
    }
}

/// Attempt evidence is shared with the owner even if its future is abandoned.
/// Counts are plaintext acknowledgements, not proof of remote execution.
#[derive(Clone, Default)]
pub struct McpHttpObservation(Arc<io::Observation>);
impl fmt::Debug for McpHttpObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpHttpObservation { <redacted> }")
    }
}
impl McpHttpObservation {
    #[must_use]
    pub fn was_attempted(&self) -> bool {
        self.0.attempted.load(std::sync::atomic::Ordering::Acquire)
    }
    #[must_use]
    pub fn acknowledged_bytes(&self) -> usize {
        self.0
            .acknowledged
            .load(std::sync::atomic::Ordering::Acquire)
    }
    /// True after all socket ownership was released, including dropped futures.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.0.complete.is_cancelled()
    }
    /// Does not drive work. The owner must continue polling or drop its exchange.
    #[must_use]
    pub fn completed(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.0.complete.cancelled())
    }
}

/// Inert selected destination, headers and lifetime; consumed by one exchange.
/// There are no spawned tasks, connection pools, redirects or automatic replay.
pub struct McpHttpConnection {
    destination: McpHttpDestination,
    head: Arc<McpSubmissionHttpHead>,
    trust: Option<McpHttpTrust>,
    limits: McpHttpLimits,
    lifetime: io::Lifetime,
    observation: McpHttpObservation,
    completion: io::Completion,
}
impl fmt::Debug for McpHttpConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpHttpConnection { <redacted> }")
    }
}
impl McpHttpConnection {
    pub(crate) fn guard_feature(&mut self, guard: super::control::McpFeatureControlAuthority) {
        self.lifetime.guard_feature(guard);
    }
    /// Selects explicit authority without connecting. HTTPS requires explicit
    /// verified trust; plaintext requires a loopback-only selected destination.
    ///
    /// # Errors
    /// Rejects invalid bounds, headers, expired/over-24-hour deadline or TLS mismatch.
    pub fn new(
        destination: McpHttpDestination,
        headers: &[(&str, &[u8])],
        trust: Option<McpHttpTrust>,
        limits: McpHttpLimits,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> Result<Self> {
        Self::with_clock(
            destination,
            headers,
            trust,
            limits,
            cancellation,
            deadline,
            Arc::new(SystemClock),
        )
    }

    /// Uses explicitly injected time authority for acquisition, writes and reads.
    /// # Errors
    /// Same authority and resource validation as [`Self::new`].
    pub fn with_clock(
        destination: McpHttpDestination,
        headers: &[(&str, &[u8])],
        trust: Option<McpHttpTrust>,
        limits: McpHttpLimits,
        cancellation: CancellationToken,
        deadline: Instant,
        clock: Arc<dyn McpHttpClock>,
    ) -> Result<Self> {
        let head = Arc::new(
            McpSubmissionHttpHead::new(destination.endpoint(), headers)
                .map_err(|_| McpHttpError::Invalid)?,
        );
        Self::from_prepared_head(
            destination,
            head,
            trust,
            limits,
            cancellation,
            deadline,
            clock,
        )
    }

    /// Retains an immutable native-prepared head; it remains data, not a proof.
    /// # Errors
    /// Rejects changed endpoint authority and the bounds enforced by [`Self::new`].
    pub fn from_prepared_head(
        destination: McpHttpDestination,
        head: Arc<McpSubmissionHttpHead>,
        trust: Option<McpHttpTrust>,
        limits: McpHttpLimits,
        cancellation: CancellationToken,
        deadline: Instant,
        clock: Arc<dyn McpHttpClock>,
    ) -> Result<Self> {
        Self::from_bounded_head(
            destination,
            (head, Duration::from_secs(24 * 60 * 60)),
            trust,
            limits,
            cancellation,
            deadline,
            clock,
        )
    }

    /// Explicit configured peer policy; legacy public constructors retain their
    /// 24-hour ceiling. Every exchange remains finite and independently bounded.
    pub(crate) fn from_configured_head(
        destination: McpHttpDestination,
        head: Arc<McpSubmissionHttpHead>,
        trust: Option<McpHttpTrust>,
        limits: McpHttpLimits,
        cancellation: CancellationToken,
        deadline: Instant,
        clock: Arc<dyn McpHttpClock>,
    ) -> Result<Self> {
        Self::from_bounded_head(
            destination,
            (head, Duration::from_millis(u64::from(u32::MAX))),
            trust,
            limits,
            cancellation,
            deadline,
            clock,
        )
    }

    fn from_bounded_head(
        destination: McpHttpDestination,
        policy: (Arc<McpSubmissionHttpHead>, Duration),
        trust: Option<McpHttpTrust>,
        limits: McpHttpLimits,
        cancellation: CancellationToken,
        deadline: Instant,
        clock: Arc<dyn McpHttpClock>,
    ) -> Result<Self> {
        let (head, maximum) = policy;
        let now = clock.now();
        if deadline <= now || deadline.duration_since(now) > maximum {
            return Err(McpHttpError::Deadline);
        }
        if destination.endpoint().is_tls() != trust.is_some()
            || destination.endpoint() != head.endpoint()
        {
            return Err(McpHttpError::Invalid);
        }
        let observation = McpHttpObservation::default();
        Ok(Self {
            destination,
            head,
            trust,
            limits: limits.validate()?,
            lifetime: io::Lifetime::new(cancellation, deadline, clock),
            completion: io::Completion(observation.clone()),
            observation,
        })
    }
    #[must_use]
    pub fn observation(&self) -> McpHttpObservation {
        self.observation.clone()
    }
    #[must_use]
    pub fn endpoint(&self) -> &McpEndpoint {
        self.destination.endpoint()
    }

    /// Sends exactly the pre-admitted request through the retained native proof.
    /// The caller explicitly supplies the one runtime allocation admitted to this
    /// connection; names and numeric generations cannot substitute for it.
    /// Construction is inert. Dropping the future or returned body closes its socket.
    #[must_use]
    pub fn submit(
        self,
        submission: McpSubmission,
        runtime: Arc<McpSubmissionRuntime>,
    ) -> BoxFuture<'static, Result<McpHttpResponse>> {
        Box::pin(async move { io::submit(self, submission, runtime).await })
    }

    /// Explicit startup/control authority, never an arbitrary tool call.
    #[must_use]
    pub fn control(self, control: McpHttpControl) -> BoxFuture<'static, Result<McpHttpResponse>> {
        Box::pin(async move { io::control(self, control).await })
    }
}

#[cfg(test)]
pub(crate) mod tests;
