//! One owned, bounded automatic permission review over injected Gateway authority.

mod evidence;
mod policy;
mod secrets;

use std::fmt;
use std::future::{Future, poll_fn};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

use machine_god_core::{
    BoxFuture, CancellationToken, Message, ModelEvent, ProviderError, ProviderErrorKind, SessionId,
    StopReason, ToolCallId,
};
use serde::Deserialize;

use crate::ai_gateway::{build_gateway_transport_request, decode_gateway_review_stream};
use crate::{AiGatewayLimits, AiGatewayTransport};

pub const NATIVE_PERMISSION_REVIEW_MODEL: &str = "zai/glm-5.2";
pub const NATIVE_PERMISSION_REVIEW_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_NATIVE_PERMISSION_REVIEW_PACKET_BYTES: usize = 16 * 1024;
pub const MAX_NATIVE_PERMISSION_REVIEW_RATIONALE_BYTES: usize = 240;

/// Informational impact; this never overrides the model's decision.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NativeAutoPermissionRisk {
    Low,
    Medium,
    High,
    Critical,
}
/// Informational support from proven human scope, not a host-side allow veto.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NativeAutoPermissionAuthorization {
    Unknown,
    Low,
    Medium,
    High,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NativeAutoPermissionDecision {
    Allow,
    Ask,
}

#[derive(Clone, Eq, PartialEq)]
pub struct NativeAutoPermissionAssessment {
    risk: NativeAutoPermissionRisk,
    authorization: NativeAutoPermissionAuthorization,
    decision: NativeAutoPermissionDecision,
    rationale: String,
}
impl NativeAutoPermissionAssessment {
    /// # Errors
    /// Rejects an empty rationale or one exceeding the pinned 240-byte bound.
    pub fn new(
        risk: NativeAutoPermissionRisk,
        authorization: NativeAutoPermissionAuthorization,
        decision: NativeAutoPermissionDecision,
        rationale: &str,
    ) -> Result<Self, NativeAutoPermissionReviewError> {
        if rationale.is_empty() || rationale.len() > MAX_NATIVE_PERMISSION_REVIEW_RATIONALE_BYTES {
            return Err(NativeAutoPermissionReviewError::InvalidResponse);
        }
        Ok(Self {
            risk,
            authorization,
            decision,
            rationale: rationale.to_owned(),
        })
    }
    #[must_use]
    pub const fn risk(&self) -> NativeAutoPermissionRisk {
        self.risk
    }
    #[must_use]
    pub const fn authorization(&self) -> NativeAutoPermissionAuthorization {
        self.authorization
    }
    #[must_use]
    pub const fn decision(&self) -> NativeAutoPermissionDecision {
        self.decision
    }
    #[must_use]
    pub fn rationale(&self) -> &str {
        &self.rationale
    }
}
impl fmt::Debug for NativeAutoPermissionAssessment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeAutoPermissionAssessment")
            .field("risk", &self.risk)
            .field("authorization", &self.authorization)
            .field("decision", &self.decision)
            .finish_non_exhaustive()
    }
}

/// Fixed categories retain no provider diagnostics or action content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAutoPermissionReviewError {
    InvalidInput,
    InvalidResponse,
    TransientFailure,
    PermanentFailure,
    TimedOut,
    Cancelled,
}
impl fmt::Display for NativeAutoPermissionReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "automatic permission review failed ({self:?})")
    }
}
impl std::error::Error for NativeAutoPermissionReviewError {}
type Error = NativeAutoPermissionReviewError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAutoPermissionOrigin {
    Root,
    Subagent,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAutoPermissionPhase {
    Initial,
    Preflight,
    Reactive,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAutoPermissionSandboxScope {
    Restricted,
    Broader,
}
/// Exact borrowed preimage, supplied by native preparation, never read here.
#[derive(Clone, Copy)]
pub enum NativeAutoPermissionFilePreimage<'a> {
    Absent,
    File(&'a [u8]),
    EmptyDirectory,
}
#[derive(Clone, Copy)]
pub struct NativeAutoPermissionTarget<'a> {
    pub role: &'a str,
    pub path: &'a str,
}

/// Distinctly typed proven root-user projection. Validation checks framing,
/// not provenance: only the native owner of canonical user input may supply it.
#[derive(Clone, Copy)]
pub struct NativeAutoPermissionRootContext<'a> {
    projection: &'a str,
}
impl<'a> NativeAutoPermissionRootContext<'a> {
    /// Accepts pinned canonical root context, excluding historical permission feedback
    /// from the returned reviewer projection. No generic message-role inference occurs.
    /// # Errors
    /// Rejects malformed or oversized (1,024-byte) canonical context.
    pub fn from_proven_projection(projection: &'a str) -> Result<Self, Error> {
        Ok(Self {
            projection: evidence::root_projection(projection)?,
        })
    }
    #[must_use]
    pub const fn projection(&self) -> &'a str {
        self.projection
    }
}

/// Explicit prepared action evidence; none of these fields conveys authority.
#[derive(Clone, Copy)]
pub enum NativeAutoPermissionAction<'a> {
    Command {
        command: &'a str,
        resolved_cwd: &'a str,
        background: bool,
        backend: &'a str,
        target_os: &'a str,
        scope: NativeAutoPermissionSandboxScope,
    },
    FileMutation {
        tool_name: &'a str,
        display_path: &'a str,
        preimage: NativeAutoPermissionFilePreimage<'a>,
        postimage: Option<&'a [u8]>,
    },
    Tool {
        tool_name: &'a str,
        arguments_json: &'a str,
        schema_json: Option<&'a str>,
        schema_required: bool,
    },
    SandboxWidening {
        command: &'a str,
        resolved_cwd: &'a str,
        background: bool,
        backend: &'a str,
        target_os: &'a str,
        prior_scope: NativeAutoPermissionSandboxScope,
        requested_scope: NativeAutoPermissionSandboxScope,
        reason: &'a str,
        restricted_result: Option<&'a str>,
        restricted_command_result: Option<&'a str>,
    },
}

/// Borrowed successful-turn context. Only the uniquely identified pending call
/// is forwarded; all assistant prose, JSON attachments and sibling calls are omitted.
#[derive(Clone, Copy)]
pub struct NativeAutoPermissionReview<'a> {
    pub session_id: &'a SessionId,
    pub workspace_root: &'a str,
    pub source_model: &'a str,
    pub pending_assistant: &'a Message,
    pub target_call_id: &'a ToolCallId,
    pub trusted_root_context: NativeAutoPermissionRootContext<'a>,
    pub origin: NativeAutoPermissionOrigin,
    pub phase: NativeAutoPermissionPhase,
    pub targets: &'a [NativeAutoPermissionTarget<'a>],
    pub action: NativeAutoPermissionAction<'a>,
    pub escalation_reason: &'a str,
}

macro_rules! redacted_debug {
    ($($name:ident),+ $(,)?) => {$(impl fmt::Debug for $name<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct(stringify!($name)).finish_non_exhaustive()
        }
    })+};
}
redacted_debug!(
    NativeAutoPermissionFilePreimage,
    NativeAutoPermissionTarget,
    NativeAutoPermissionRootContext,
    NativeAutoPermissionAction,
    NativeAutoPermissionReview
);

pub trait NativePermissionReviewer: Send + Sync + 'static {
    fn review<'a>(
        &'a self,
        review: NativeAutoPermissionReview<'a>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<NativeAutoPermissionAssessment, Error>>;
}

/// Explicit bounded monotonic clock and owned wakeup. Implementations must not
/// block, and dropping the timer must release its waiter without detached tasks.
pub trait NativePermissionReviewClock: Send + Sync + 'static {
    fn now(&self) -> Instant;
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'static, ()>;
}

/// Production monotonic deadline using the host's existing Tokio runtime.
#[cfg(feature = "ai-gateway-http")]
#[derive(Clone, Copy, Debug, Default)]
pub struct TokioPermissionReviewClock;
#[cfg(feature = "ai-gateway-http")]
impl NativePermissionReviewClock for TokioPermissionReviewClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'static, ()> {
        Box::pin(async move { tokio::time::sleep_until(deadline.into()).await })
    }
}

pub struct AiGatewayPermissionReviewer {
    transport: Arc<dyn AiGatewayTransport>,
    clock: Arc<dyn NativePermissionReviewClock>,
}
impl AiGatewayPermissionReviewer {
    /// Inert construction. Credentials/endpoints belong to the injected transport;
    /// source-model effort/fast controls cannot enter this dedicated worker.
    #[must_use]
    pub fn new(
        transport: Arc<dyn AiGatewayTransport>,
        clock: Arc<dyn NativePermissionReviewClock>,
    ) -> Self {
        Self { transport, clock }
    }
}
impl fmt::Debug for AiGatewayPermissionReviewer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiGatewayPermissionReviewer")
            .finish_non_exhaustive()
    }
}
impl NativePermissionReviewer for AiGatewayPermissionReviewer {
    fn review<'a>(
        &'a self,
        review: NativeAutoPermissionReview<'a>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<NativeAutoPermissionAssessment, Error>> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let deadline = self
                .clock
                .now()
                .checked_add(NATIVE_PERMISSION_REVIEW_TIMEOUT)
                .ok_or(Error::TimedOut)?;
            let mut timer = self.clock.wait_until(deadline);
            let mut cancelled = Box::pin(cancellation.cancelled());
            let mut operation = Box::pin(self.attempt(review, &cancellation));
            let outcome = poll_fn(|cx| {
                if cancelled.as_mut().poll(cx).is_ready() || cancellation.is_cancelled() {
                    return Poll::Ready(Err(Error::Cancelled));
                }
                if self.clock.now() >= deadline || timer.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(Err(Error::TimedOut));
                }
                let result = operation.as_mut().poll(cx);
                if cancellation.is_cancelled() {
                    drop(result);
                    return Poll::Ready(Err(Error::Cancelled));
                }
                if self.clock.now() >= deadline || timer.as_mut().poll(cx).is_ready() {
                    drop(result);
                    return Poll::Ready(Err(Error::TimedOut));
                }
                result
            })
            .await;
            drop(operation);
            drop(timer);
            drop(cancelled);
            // Release all callback-owned state before the final race decision.
            if cancellation.is_cancelled() {
                return Err(Error::Cancelled);
            }
            if self.clock.now() >= deadline {
                return Err(Error::TimedOut);
            }
            outcome
        })
    }
}

impl AiGatewayPermissionReviewer {
    async fn attempt(
        &self,
        review: NativeAutoPermissionReview<'_>,
        cancellation: &CancellationToken,
    ) -> Result<NativeAutoPermissionAssessment, Error> {
        let body = evidence::body(review)?;
        // Give the outer arbiter a boundary after bounded preparation and before
        // constructing the sole authority-bearing transport future.
        review_boundary().await;
        let request = build_gateway_transport_request(
            NATIVE_PERMISSION_REVIEW_MODEL,
            review.session_id.as_str(),
            body,
        );
        let source = self
            .transport
            .stream(request, cancellation.clone())
            .await
            .map_err(|error| map_transport(&error))?;
        // Startup may itself make cancellation/deadline observable. Return to
        // the owner before polling the newly acquired byte stream.
        review_boundary().await;
        let limits = AiGatewayLimits {
            max_request_bytes: MAX_NATIVE_PERMISSION_REVIEW_PACKET_BYTES,
            max_chunk_bytes: 64 * 1024,
            max_record_bytes: 16 * 1024,
            max_undecoded_bytes: 16 * 1024,
            max_total_response_bytes: 64 * 1024,
            max_records: 1024,
            max_streamed_tool_calls: 1,
            max_tool_calls: 1,
            max_tool_arguments_bytes: 4096,
            max_json_nodes: 256,
            ..AiGatewayLimits::default()
        };
        let mut stream = decode_gateway_review_stream(source, cancellation, limits);
        let mut assessment = None;
        let mut stopped = false;
        while let Some(event) = poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
            match event.map_err(|error| map_provider(&error))? {
                ModelEvent::TextDelta { text }
                    if text.trim_matches([' ', '\t', '\r', '\n']).is_empty() => {}
                ModelEvent::ReasoningDelta { .. } | ModelEvent::Usage { .. } => {}
                ModelEvent::ToolCall { call }
                    if assessment.is_none() && call.name.as_str() == "permission_decision" =>
                {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Parsed {
                        risk: NativeAutoPermissionRisk,
                        authorization: NativeAutoPermissionAuthorization,
                        decision: NativeAutoPermissionDecision,
                        rationale: String,
                    }
                    let parsed: Parsed = serde_json::from_value(call.arguments)
                        .map_err(|_| Error::InvalidResponse)?;
                    assessment = Some(NativeAutoPermissionAssessment::new(
                        parsed.risk,
                        parsed.authorization,
                        parsed.decision,
                        &parsed.rationale,
                    )?);
                }
                ModelEvent::Stop {
                    reason:
                        StopReason::ToolCalls
                        | StopReason::Completed
                        | StopReason::MaxOutputTokens
                        | StopReason::Other(_),
                } => {
                    stopped = true;
                }
                ModelEvent::Stop {
                    reason: StopReason::ContentFilter,
                } => return Err(Error::PermanentFailure),
                ModelEvent::Stop {
                    reason: StopReason::Cancelled,
                } => return Err(Error::Cancelled),
                _ => return Err(Error::InvalidResponse),
            }
            // One bounded decoded event per owner poll keeps deadline wakeups
            // observable even when the injected byte stream is always ready.
            review_boundary().await;
        }
        if !stopped {
            return Err(Error::InvalidResponse);
        }
        assessment.ok_or(Error::InvalidResponse)
    }
}

async fn review_boundary() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

fn map_provider(error: &ProviderError) -> Error {
    if error.kind == ProviderErrorKind::Other && error.code == "gateway_provider_failure" {
        return Error::TransientFailure;
    }
    if error.kind == ProviderErrorKind::Protocol && error.code.starts_with("gateway_") {
        return Error::InvalidResponse;
    }
    map_transport(error)
}

fn map_transport(error: &ProviderError) -> Error {
    match error.kind {
        ProviderErrorKind::Cancelled => Error::Cancelled,
        ProviderErrorKind::RateLimited | ProviderErrorKind::Unavailable => Error::TransientFailure,
        ProviderErrorKind::Transport if error.retryable => Error::TransientFailure,
        _ => Error::PermanentFailure,
    }
}
