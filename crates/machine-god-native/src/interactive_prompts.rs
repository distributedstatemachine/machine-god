//! Owned human interaction without acquiring an input, output, or worker.

mod payload;
mod projection;
mod state;
#[cfg(test)]
mod tests;

use std::fmt;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};

use machine_god_core::{BackgroundOutputOwner, BoxFuture, PermissionRequest, ToolContext};

use crate::mcp::interaction::{
    McpElicitationAnswer, McpElicitationAnswerInput, McpElicitationPresenter,
    McpElicitationPromptError, McpElicitationPromptRequest, McpUrlRecoveryAnswer,
    McpUrlRecoveryPromptRequest,
};
use crate::{
    PermissionPromptDecision, PermissionPromptError, PermissionPrompter, QuestionPromptError,
    QuestionPromptOutcome, QuestionPromptRequest, QuestionPrompter,
};
use payload::AcceptedResponse;
use payload::Payload;
pub use projection::{
    NativeInteractivePromptKind, NativeInteractivePromptPage, NativeInteractivePromptSummary,
};
use state::Shared;

/// Includes queued, displayed, and replied-but-not-yet-consumed requests.
pub const MAX_NATIVE_INTERACTIVE_PROMPTS: usize = 64;
/// Simultaneously registered principals, not a lifetime creation limit.
pub const MAX_NATIVE_INTERACTIVE_PROMPT_PRINCIPALS: usize = 64;
/// A payload-free inbox page has its own conservative retained byte bound.
pub const MAX_NATIVE_INTERACTIVE_PROMPT_PAGE_BYTES: usize = 192 * 1024;
/// Hard aggregate retained request payload bound; response bytes are separately bounded.
pub const MAX_NATIVE_INTERACTIVE_PROMPT_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
/// Independent aggregate replied-but-unconsumed response bound.
pub const MAX_NATIVE_INTERACTIVE_PROMPT_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Explicit admission bounds. They do not change the underlying tool contracts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeInteractivePromptLimits {
    pending: usize,
    payload_bytes: usize,
    response_bytes: usize,
}

impl NativeInteractivePromptLimits {
    /// # Errors
    /// Rejects zero or limits above the published hard caps.
    pub fn new(
        max_pending: usize,
        max_payload_bytes: usize,
    ) -> Result<Self, NativeInteractivePromptError> {
        if !(1..=MAX_NATIVE_INTERACTIVE_PROMPTS).contains(&max_pending)
            || !(1..=MAX_NATIVE_INTERACTIVE_PROMPT_PAYLOAD_BYTES).contains(&max_payload_bytes)
        {
            return Err(NativeInteractivePromptError::Limit);
        }
        Ok(Self {
            pending: max_pending,
            payload_bytes: max_payload_bytes,
            response_bytes: MAX_NATIVE_INTERACTIVE_PROMPT_RESPONSE_BYTES,
        })
    }
    /// Lower the aggregate retained response bound without changing requests.
    /// # Errors
    /// Rejects zero or a value above the published hard ceiling.
    pub fn with_response_bytes(
        mut self,
        max_response_bytes: usize,
    ) -> Result<Self, NativeInteractivePromptError> {
        if !(1..=MAX_NATIVE_INTERACTIVE_PROMPT_RESPONSE_BYTES).contains(&max_response_bytes) {
            return Err(NativeInteractivePromptError::Limit);
        }
        self.response_bytes = max_response_bytes;
        Ok(self)
    }
}

impl Default for NativeInteractivePromptLimits {
    fn default() -> Self {
        Self {
            pending: MAX_NATIVE_INTERACTIVE_PROMPTS,
            payload_bytes: 8 * 1024 * 1024,
            response_bytes: MAX_NATIVE_INTERACTIVE_PROMPT_RESPONSE_BYTES,
        }
    }
}

/// Fixed errors never retain request, response, identity, or host diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeInteractivePromptError {
    Closed,
    Stale,
    Busy,
    Limit,
    InvalidResponse,
    Exhausted,
}

impl fmt::Display for NativeInteractivePromptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("interactive prompt unavailable")
    }
}
impl std::error::Error for NativeInteractivePromptError {}

/// Opaque registration epoch, independent of UI navigation and presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeInteractivePromptScope(u64);

/// A clone retains only an inert bridge identity, never a prompt or grant.
#[derive(Clone)]
pub struct NativeInteractivePromptToken {
    identity: Arc<()>,
    scope: NativeInteractivePromptScope,
    generation: u64,
    owner: BackgroundOutputOwner,
}

impl NativeInteractivePromptToken {
    #[must_use]
    pub const fn owner(&self) -> &BackgroundOutputOwner {
        &self.owner
    }
    #[must_use]
    pub const fn scope(&self) -> NativeInteractivePromptScope {
        self.scope
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}
impl PartialEq for NativeInteractivePromptToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
            && self.scope == other.scope
            && self.generation == other.generation
            && self.owner == other.owner
    }
}
impl Eq for NativeInteractivePromptToken {}
impl fmt::Debug for NativeInteractivePromptToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractivePromptToken { .. }")
    }
}

/// Exact immutable payload for presentation, not permission authority. Retained
/// views may outlive their request, but replying with an obsolete token fails.
pub struct NativeInteractivePromptView {
    token: NativeInteractivePromptToken,
    payload: Arc<Payload>,
}
impl NativeInteractivePromptView {
    #[must_use]
    pub const fn token(&self) -> &NativeInteractivePromptToken {
        &self.token
    }
    #[must_use]
    pub fn permission(&self) -> Option<&PermissionRequest> {
        match self.payload.as_ref() {
            Payload::Permission { request, .. } => Some(request),
            _ => None,
        }
    }
    #[must_use]
    pub fn question(&self) -> Option<(&ToolContext, &QuestionPromptRequest)> {
        match self.payload.as_ref() {
            Payload::Question { context, request } => Some((context, request)),
            _ => None,
        }
    }
    #[must_use]
    pub fn elicitation(&self) -> Option<&McpElicitationPromptRequest> {
        match self.payload.as_ref() {
            Payload::Elicitation { request } => Some(request),
            _ => None,
        }
    }
    #[must_use]
    pub fn url_recovery(&self) -> Option<&McpUrlRecoveryPromptRequest> {
        match self.payload.as_ref() {
            Payload::UrlRecovery { request } => Some(request),
            _ => None,
        }
    }
    /// Whether native preparation supplied an exact-action proposal source.
    /// This observation neither validates a stale view nor creates authority.
    #[must_use]
    pub fn can_save_rule(&self) -> bool {
        matches!(
            self.payload.as_ref(),
            Payload::Permission { rule: Some(_), .. }
        )
    }
}
impl fmt::Debug for NativeInteractivePromptView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractivePromptView { .. }")
    }
}

/// An input-owner response. `AllowSession` remains a volatile controller grant;
/// this enum never confirms or publishes a saved rule.
pub enum NativeInteractivePromptResponse {
    Permission(PermissionPromptDecision),
    Question(QuestionPromptOutcome),
    Elicitation(McpElicitationAnswerInput),
    UrlRecovery(McpUrlRecoveryAnswer),
}
impl fmt::Debug for NativeInteractivePromptResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractivePromptResponse { .. }")
    }
}

/// Shared trait endpoint. It owns no input reader, renderer, session, or worker.
pub struct NativeInteractivePromptBridge {
    shared: Arc<Shared>,
    principal: Option<PrincipalKey>,
}

mod registration;
pub(crate) use registration::NativeInteractivePromptRegistration;
#[cfg(any(
    test,
    all(
        feature = "ai-gateway-http",
        any(target_os = "linux", target_os = "macos")
    )
))]
pub(crate) use registration::NativeInteractivePromptReservation;

#[derive(Clone)]
struct PrincipalKey {
    scope: NativeInteractivePromptScope,
    owner: BackgroundOutputOwner,
    live: Arc<std::sync::atomic::AtomicBool>,
}
impl PartialEq for PrincipalKey {
    fn eq(&self, other: &Self) -> bool {
        self.scope == other.scope
            && self.owner == other.owner
            && Arc::ptr_eq(&self.live, &other.live)
    }
}
impl Eq for PrincipalKey {}

/// Unique registration custody. Dropping or explicitly retiring this lease
/// invalidates only this exact registration, including unconsumed answers.
pub struct NativeInteractivePromptPrincipal {
    shared: Arc<Shared>,
    key: PrincipalKey,
}
impl NativeInteractivePromptPrincipal {
    #[must_use]
    pub const fn owner(&self) -> &BackgroundOutputOwner {
        &self.key.owner
    }
    #[must_use]
    pub const fn scope(&self) -> NativeInteractivePromptScope {
        self.key.scope
    }
    /// An inert fixed-principal endpoint. Its clones never own registration custody.
    #[must_use]
    pub fn bridge(&self) -> Arc<NativeInteractivePromptBridge> {
        Arc::new(NativeInteractivePromptBridge {
            shared: Arc::clone(&self.shared),
            principal: Some(self.key.clone()),
        })
    }
    /// Idempotent exact retirement; it cannot retire a replacement registration.
    pub fn retire(&mut self) {
        self.shared.retire(&self.key);
    }
}
impl Drop for NativeInteractivePromptPrincipal {
    fn drop(&mut self) {
        self.retire();
    }
}
impl fmt::Debug for NativeInteractivePromptPrincipal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractivePromptPrincipal { .. }")
    }
}
impl fmt::Debug for NativeInteractivePromptBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractivePromptBridge { .. }")
    }
}

impl PermissionPrompter for NativeInteractivePromptBridge {
    fn prompt(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        self.prompt_with_rule(request, None)
    }

    fn prompt_with_rule(
        &self,
        request: PermissionRequest,
        rule: Option<crate::NativePermissionRulePrompt>,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        let shared = Arc::clone(&self.shared);
        let payload = Arc::new(Payload::Permission {
            request,
            rule: rule.map(Box::new),
        });
        let principal = shared.capture(self.principal.as_ref(), &payload);
        Box::pin(async move {
            match shared.request(principal, payload).await {
                Ok(AcceptedResponse::Permission(decision)) => Ok(decision),
                Ok(_) | Err(_) => Err(PermissionPromptError::new()),
            }
        })
    }
}

impl QuestionPrompter for NativeInteractivePromptBridge {
    fn prompt(
        &self,
        _request: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        // There is deliberately no ambient "current question" identity fallback.
        Box::pin(async { Err(QuestionPromptError::new()) })
    }
    fn prompt_with_context(
        &self,
        context: ToolContext,
        request: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        let shared = Arc::clone(&self.shared);
        let payload = Arc::new(Payload::Question { context, request });
        let principal = shared.capture(self.principal.as_ref(), &payload);
        Box::pin(async move {
            match shared.request(principal, payload).await {
                Ok(AcceptedResponse::Question(outcome)) => Ok(outcome),
                Ok(_) | Err(_) => Err(QuestionPromptError::new()),
            }
        })
    }
}

impl McpElicitationPresenter for NativeInteractivePromptBridge {
    fn present(
        &self,
        request: McpElicitationPromptRequest,
        cancellation: machine_god_core::CancellationToken,
    ) -> BoxFuture<'_, Result<McpElicitationAnswer, McpElicitationPromptError>> {
        let future = self.mcp_prompt(Payload::Elicitation { request }, cancellation);
        Box::pin(async move {
            match future.await? {
                AcceptedResponse::Elicitation(answer) => Ok(answer),
                _ => Err(McpElicitationPromptError::InvalidResponse),
            }
        })
    }
    fn recover_url(
        &self,
        request: McpUrlRecoveryPromptRequest,
        cancellation: machine_god_core::CancellationToken,
    ) -> BoxFuture<'_, Result<McpUrlRecoveryAnswer, McpElicitationPromptError>> {
        let future = self.mcp_prompt(Payload::UrlRecovery { request }, cancellation);
        Box::pin(async move {
            match future.await? {
                AcceptedResponse::UrlRecovery(answer) => Ok(answer),
                _ => Err(McpElicitationPromptError::InvalidResponse),
            }
        })
    }
}

impl NativeInteractivePromptBridge {
    fn mcp_prompt(
        &self,
        payload: Payload,
        cancellation: machine_god_core::CancellationToken,
    ) -> impl std::future::Future<Output = Result<AcceptedResponse, McpElicitationPromptError>>
    + Send
    + 'static
    + use<> {
        let shared = Arc::clone(&self.shared);
        let payload = Arc::new(payload);
        let principal = shared.capture(self.principal.as_ref(), &payload);
        async move {
            use futures_util::future::{Either, select};
            if cancellation.is_cancelled() {
                return Err(McpElicitationPromptError::Cancelled);
            }
            let result = match select(
                Box::pin(cancellation.cancelled()),
                Box::pin(shared.request(principal, payload)),
            )
            .await
            {
                Either::Left(((), pending)) => {
                    drop(pending);
                    return Err(McpElicitationPromptError::Cancelled);
                }
                Either::Right((result, waiter)) => {
                    drop(waiter);
                    result
                }
            };
            if cancellation.is_cancelled() {
                return Err(McpElicitationPromptError::Cancelled);
            }
            result.map_err(McpElicitationPromptError::Inbox)
        }
    }
}

/// Sole UI endpoint. The CLI routes its existing input owner here; cloning the
/// shared prompter does not create another inbox or input reader.
pub struct NativeInteractivePromptInbox {
    shared: Arc<Shared>,
}

/// Registration authority for native runtime construction, without retaining the
/// inbox or any principal. Presentation still has exactly one owning endpoint.
#[derive(Clone)]
pub(crate) struct NativeInteractivePromptRegistrar(Weak<Shared>);

impl NativeInteractivePromptRegistrar {
    #[cfg(any(
        test,
        all(
            feature = "ai-gateway-http",
            any(target_os = "linux", target_os = "macos")
        )
    ))]
    pub(crate) fn reserve(
        &self,
        owner: BackgroundOutputOwner,
    ) -> Result<NativeInteractivePromptReservation, NativeInteractivePromptError> {
        let shared = self
            .0
            .upgrade()
            .ok_or(NativeInteractivePromptError::Closed)?;
        let key = shared.reserve(owner)?;
        Ok(NativeInteractivePromptReservation::new(shared, key))
    }

    #[cfg(all(
        feature = "ai-gateway-http",
        any(target_os = "linux", target_os = "macos")
    ))]
    pub(crate) fn matches(&self, inbox: &NativeInteractivePromptInbox) -> bool {
        self.0.ptr_eq(&Arc::downgrade(&inbox.shared))
    }

    pub(crate) fn register(
        &self,
        owner: BackgroundOutputOwner,
    ) -> Result<NativeInteractivePromptPrincipal, NativeInteractivePromptError> {
        let shared = self
            .0
            .upgrade()
            .ok_or(NativeInteractivePromptError::Closed)?;
        let key = shared.register(owner)?;
        Ok(NativeInteractivePromptPrincipal { shared, key })
    }
}

impl fmt::Debug for NativeInteractivePromptInbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractivePromptInbox { .. }")
    }
}
impl NativeInteractivePromptInbox {
    pub(crate) fn registrar(&self) -> NativeInteractivePromptRegistrar {
        NativeInteractivePromptRegistrar(Arc::downgrade(&self.shared))
    }
    /// Projects pending owners and kinds without retaining request payloads or
    /// marking any prompt displayed. Installs the sole UI wake for subsequent
    /// changes; this observer is shared with `poll_prompt` and `poll_pending`.
    /// A removed cursor remains usable within this inbox's monotonic sequence.
    /// # Errors
    /// Rejects closure, foreign cursors, or a zero/over-cap page limit.
    pub fn page(
        &mut self,
        cx: &mut Context<'_>,
        after: Option<&NativeInteractivePromptToken>,
        limit: usize,
    ) -> Result<NativeInteractivePromptPage, NativeInteractivePromptError> {
        self.shared.page(cx, after, limit)
    }
    /// Selects an exact unanswered request for presentation, independently of
    /// FIFO default navigation. This is not a terminal flush acknowledgement.
    /// # Errors
    /// Rejects closed, foreign, retired, or already answered tokens.
    pub fn select_prompt(
        &mut self,
        token: &NativeInteractivePromptToken,
    ) -> Result<NativeInteractivePromptView, NativeInteractivePromptError> {
        self.shared.select_prompt(token)
    }
    /// Creates the sole observer and shared aggregate budgets without I/O.
    /// # Errors
    /// Rejects invalid limits; no principal is registered automatically.
    pub fn new(
        limits: NativeInteractivePromptLimits,
    ) -> Result<Self, NativeInteractivePromptError> {
        let limits = NativeInteractivePromptLimits::new(limits.pending, limits.payload_bytes)?
            .with_response_bytes(limits.response_bytes)?;
        Ok(Self {
            shared: Arc::new(Shared {
                identity: Arc::new(()),
                limits,
                state: Mutex::new(state::State::default()),
            }),
        })
    }
    /// Endpoint for shared host tools constructed before session identity exists.
    /// Each call captures a matching live registration from its actual source;
    /// an unknown source cannot bind to a registration created later.
    #[must_use]
    pub fn router(&self) -> Arc<NativeInteractivePromptBridge> {
        Arc::new(NativeInteractivePromptBridge {
            shared: Arc::clone(&self.shared),
            principal: None,
        })
    }
    /// Registers exact session/incarnation custody independently of navigation.
    /// # Errors
    /// Rejects duplicate live owners, capacity, closure, or generation exhaustion.
    pub fn register(
        &mut self,
        owner: BackgroundOutputOwner,
    ) -> Result<NativeInteractivePromptPrincipal, NativeInteractivePromptError> {
        self.registrar().register(owner)
    }
    /// Proposes an exact saved change from the current unanswered native prompt.
    /// The returned token still requires a separate human confirmation.
    /// # Errors
    /// Rejects stale, unprepared, cancelled or reset prompt authority.
    pub fn propose_rule_change(
        &self,
        token: &NativeInteractivePromptToken,
        decision: crate::NativePermissionRuleDecision,
    ) -> Result<crate::NativePermissionRuleProposal, NativeInteractivePromptError> {
        self.shared.propose_rule_change(token, decision)
    }
    /// Returns the same displayed token until answered/cancelled/dropped. An
    /// empty open inbox registers a wake without self-waking on observation.
    pub fn poll_prompt(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<NativeInteractivePromptView>> {
        self.shared.poll_prompt(cx)
    }

    /// Observe removal of an already displayed request without polling or
    /// retaining another payload. Used by connection-scoped correlation owners.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn poll_pending(
        &self,
        token: &NativeInteractivePromptToken,
        cx: &mut Context<'_>,
    ) -> Poll<()> {
        self.shared.poll_pending(token, cx)
    }
    /// Accepts one correctly typed, bounded answer for the displayed token.
    /// This is input acceptance, not execution or persistence confirmation.
    /// # Errors
    /// Rejects stale/non-displayed tokens, duplicate or malformed responses.
    pub fn reply(
        &mut self,
        token: &NativeInteractivePromptToken,
        response: NativeInteractivePromptResponse,
    ) -> Result<(), NativeInteractivePromptError> {
        self.shared.reply(token, response)
    }
    /// Explicit user cancellation resolves Deny for permissions or Cancelled
    /// for questions, or the distinct MCP elicitation cancel action. Engine
    /// cancellation still has its existing precedence.
    /// # Errors
    /// Rejects stale/non-displayed tokens.
    pub fn cancel(
        &mut self,
        token: &NativeInteractivePromptToken,
    ) -> Result<(), NativeInteractivePromptError> {
        self.shared.cancel(token)
    }
    /// Permanently closes admission and invalidates pending/ready responses.
    pub fn close(&mut self) {
        self.shared.close();
    }
}
impl Drop for NativeInteractivePromptInbox {
    fn drop(&mut self) {
        self.close();
    }
}
