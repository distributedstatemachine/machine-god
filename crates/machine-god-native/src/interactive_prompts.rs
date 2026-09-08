//! Owned human interaction without acquiring an input, output, or worker.

mod payload;
mod state;
#[cfg(test)]
mod tests;

use std::fmt;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use machine_god_core::{BackgroundOutputOwner, BoxFuture, PermissionRequest, ToolContext};

use crate::{
    PermissionPromptDecision, PermissionPromptError, PermissionPrompter, QuestionPromptError,
    QuestionPromptOutcome, QuestionPromptRequest, QuestionPrompter,
};
use payload::Payload;
use state::Shared;

/// Includes queued, displayed, and replied-but-not-yet-consumed requests.
pub const MAX_NATIVE_INTERACTIVE_PROMPTS: usize = 8;
/// Hard aggregate retained request payload bound; response bytes are separately bounded.
pub const MAX_NATIVE_INTERACTIVE_PROMPT_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

/// Explicit admission bounds. They do not change the underlying tool contracts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeInteractivePromptLimits {
    max_pending: usize,
    max_payload_bytes: usize,
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
            max_pending,
            max_payload_bytes,
        })
    }
}

impl Default for NativeInteractivePromptLimits {
    fn default() -> Self {
        Self {
            max_pending: MAX_NATIVE_INTERACTIVE_PROMPTS,
            max_payload_bytes: 8 * 1024 * 1024,
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

/// Opaque UI activation epoch. Re-activating even the same principal advances it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeInteractivePromptScope(u64);

/// A clone retains only an inert bridge identity, never a prompt or grant.
#[derive(Clone)]
pub struct NativeInteractivePromptToken {
    identity: Arc<()>,
    scope: NativeInteractivePromptScope,
    generation: u64,
}

impl NativeInteractivePromptToken {
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
            Payload::Permission(value) => Some(value),
            Payload::Question { .. } => None,
        }
    }
    #[must_use]
    pub fn question(&self) -> Option<(&ToolContext, &QuestionPromptRequest)> {
        match self.payload.as_ref() {
            Payload::Question { context, request } => Some((context, request)),
            Payload::Permission(_) => None,
        }
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
}
impl fmt::Debug for NativeInteractivePromptResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractivePromptResponse { .. }")
    }
}

/// Shared trait endpoint. It owns no input reader, renderer, session, or worker.
pub struct NativeInteractivePromptBridge {
    shared: Arc<Shared>,
}

impl NativeInteractivePromptBridge {
    /// Creates one uniquely owned inbox and shared prompt endpoint without I/O.
    /// # Errors
    /// Rejects invalid limits defensively; no scope is activated automatically.
    pub fn new(
        limits: NativeInteractivePromptLimits,
    ) -> Result<(Arc<Self>, NativeInteractivePromptInbox), NativeInteractivePromptError> {
        let limits =
            NativeInteractivePromptLimits::new(limits.max_pending, limits.max_payload_bytes)?;
        let shared = Arc::new(Shared {
            identity: Arc::new(()),
            limits,
            state: Mutex::new(state::State::default()),
        });
        Ok((
            Arc::new(Self {
                shared: Arc::clone(&shared),
            }),
            NativeInteractivePromptInbox { shared },
        ))
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
        let scope = self.shared.scope();
        let shared = Arc::clone(&self.shared);
        let payload = Arc::new(Payload::Permission(request));
        Box::pin(async move {
            match shared.request(scope, payload).await {
                Ok(NativeInteractivePromptResponse::Permission(decision)) => Ok(decision),
                Ok(NativeInteractivePromptResponse::Question(_)) | Err(_) => {
                    Err(PermissionPromptError::new())
                }
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
        let scope = self.shared.scope();
        let shared = Arc::clone(&self.shared);
        let payload = Arc::new(Payload::Question { context, request });
        Box::pin(async move {
            match shared.request(scope, payload).await {
                Ok(NativeInteractivePromptResponse::Question(outcome)) => Ok(outcome),
                Ok(NativeInteractivePromptResponse::Permission(_)) | Err(_) => {
                    Err(QuestionPromptError::new())
                }
            }
        })
    }
}

/// Sole UI endpoint. The CLI routes its existing input owner here; cloning the
/// shared prompter does not create another inbox or input reader.
pub struct NativeInteractivePromptInbox {
    shared: Arc<Shared>,
}
impl fmt::Debug for NativeInteractivePromptInbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractivePromptInbox { .. }")
    }
}
impl NativeInteractivePromptInbox {
    /// Replaces the exact UI principal and invalidates every prior response,
    /// including a response accepted but not yet consumed by its future.
    /// # Errors
    /// Rejects a closed inbox or exhausted scope counter without rebinding.
    pub fn activate(
        &mut self,
        owner: BackgroundOutputOwner,
    ) -> Result<NativeInteractivePromptScope, NativeInteractivePromptError> {
        self.shared.activate(owner)
    }
    /// Invalidates current admission without closing the reusable inbox.
    pub fn deactivate(&mut self) {
        self.shared.deactivate(false);
    }
    /// Returns the same displayed token until answered/cancelled/dropped. An
    /// empty open inbox registers a wake without self-waking on observation.
    pub fn poll_prompt(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<NativeInteractivePromptView>> {
        self.shared.poll_prompt(cx)
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
    /// for questions. Engine cancellation still has its existing precedence.
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
        self.shared.deactivate(true);
    }
}
impl Drop for NativeInteractivePromptInbox {
    fn drop(&mut self) {
        self.close();
    }
}
