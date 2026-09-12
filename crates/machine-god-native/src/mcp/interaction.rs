//! Typed human presentation on the existing native interactive prompt inbox.
//! Answers are validated data, never browser/continuation/execution authority.

use super::mrtr::{McpElicitationAction, McpElicitationRequest};
use machine_god_core::{BoxFuture, CancellationToken, ToolContext, ToolName};
use serde_json::value::RawValue;
use std::{fmt, sync::Arc};

pub const MAX_MCP_ELICITATION_ANSWER_BYTES: usize = 128 * 1024;

pub struct McpElicitationPromptRequest {
    context: ToolContext,
    server: Arc<str>,
    tool: ToolName,
    request: Arc<McpElicitationRequest>,
}
impl McpElicitationPromptRequest {
    /// Inert ownership transfer. Inbox admission checks the captured UI scope.
    /// # Errors
    /// Rejects empty or over-256-byte server identities. `ToolName` already
    /// enforces its 128-byte bound. The executor supplies these identities
    /// from selection, never from the server's message.
    pub fn new(
        context: ToolContext,
        server: Arc<str>,
        tool: ToolName,
        request: Arc<McpElicitationRequest>,
    ) -> Result<Self, McpElicitationPromptError> {
        if server.is_empty() || server.len() > 256 {
            return Err(McpElicitationPromptError::InvalidSource);
        }
        Ok(Self {
            context,
            server,
            tool,
            request,
        })
    }
    #[must_use]
    pub fn server(&self) -> &str {
        &self.server
    }
    #[must_use]
    pub const fn tool(&self) -> &ToolName {
        &self.tool
    }
    #[must_use]
    pub const fn context(&self) -> &ToolContext {
        &self.context
    }
    #[must_use]
    pub const fn request(&self) -> &Arc<McpElicitationRequest> {
        &self.request
    }
    #[must_use]
    pub fn retained_byte_charge(&self) -> usize {
        // Count all retained identifiers. Saturation cannot admit an
        // overflow because every inbox limit is strictly below usize::MAX.
        [
            self.context.session_id.as_str(),
            self.context.session_incarnation_id.as_str(),
            self.context.turn_id.as_str(),
            self.context.call_id.as_str(),
            self.server(),
            self.tool.as_str(),
        ]
        .iter()
        .fold(
            self.request.retained_byte_charge().saturating_add(256),
            |total, text| total.saturating_add(text.len()),
        )
    }
}
impl fmt::Debug for McpElicitationPromptRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpElicitationPromptRequest { .. }")
    }
}

/// An inert recovery question for an already selected URL request. Construction
/// does not assert a failed launch or authorize another browser attempt.
#[derive(Debug)]
pub struct McpUrlRecoveryPromptRequest {
    source: McpElicitationPromptRequest,
}
impl McpUrlRecoveryPromptRequest {
    /// # Errors
    /// Rejects non-URL elicitation requests.
    pub fn new(source: McpElicitationPromptRequest) -> Result<Self, McpElicitationPromptError> {
        if source.request().mode() != super::mrtr::McpElicitationMode::Url {
            return Err(McpElicitationPromptError::InvalidSource);
        }
        Ok(Self { source })
    }
    #[must_use]
    pub const fn source(&self) -> &McpElicitationPromptRequest {
        &self.source
    }
    #[must_use]
    pub fn retained_byte_charge(&self) -> usize {
        self.source.retained_byte_charge().saturating_add(64)
    }
}

/// Human choice data, not browser or continuation authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpUrlRecoveryAnswer {
    ContinueManually,
    RetryBrowser,
    Cancel,
}

/// Bounded UI input. Construction is not validation against a queued request.
pub struct McpElicitationAnswerInput {
    raw: Box<RawValue>,
}
impl McpElicitationAnswerInput {
    /// # Errors
    /// Rejects response bytes beyond the fixed raw-input ceiling before retention.
    pub fn new(raw: Box<RawValue>) -> Result<Self, McpElicitationPromptError> {
        if raw.get().len() > MAX_MCP_ELICITATION_ANSWER_BYTES {
            return Err(McpElicitationPromptError::Limit);
        }
        Ok(Self { raw })
    }
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
    pub(crate) fn cancel() -> Self {
        Self {
            raw: RawValue::from_string("{\"action\":\"cancel\"}".into())
                .expect("fixed cancel JSON"),
        }
    }
}
impl fmt::Debug for McpElicitationAnswerInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpElicitationAnswerInput { .. }")
    }
}

/// Canonical answer validated against the exact displayed request. Its type is
/// still human response data, not proof of browser completion or permission.
pub struct McpElicitationAnswer {
    action: McpElicitationAction,
    raw: Box<RawValue>,
}
impl McpElicitationAnswer {
    #[must_use]
    pub const fn action(&self) -> McpElicitationAction {
        self.action
    }
    #[must_use]
    pub fn canonical_json(&self) -> &RawValue {
        &self.raw
    }
    #[must_use]
    pub fn retained_byte_charge(&self) -> usize {
        self.raw.get().len() + 64
    }
    pub(crate) fn validate(
        request: &McpElicitationRequest,
        input: &McpElicitationAnswerInput,
    ) -> Result<Self, McpElicitationPromptError> {
        let (action, raw) = request
            .validate_response(input.raw_json())
            .map_err(|_| McpElicitationPromptError::InvalidResponse)?;
        Ok(Self { action, raw })
    }
}
impl fmt::Debug for McpElicitationAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpElicitationAnswer")
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpElicitationPromptError {
    Unavailable,
    Cancelled,
    Limit,
    InvalidResponse,
    InvalidSource,
    Inbox(crate::NativeInteractivePromptError),
}
impl fmt::Display for McpElicitationPromptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP human input unavailable")
    }
}
impl std::error::Error for McpElicitationPromptError {}

/// Explicitly injected human endpoint. No ambient context or model-text fallback.
pub trait McpElicitationPresenter: Send + Sync {
    fn present(
        &self,
        request: McpElicitationPromptRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpElicitationAnswer, McpElicitationPromptError>>;

    fn recover_url(
        &self,
        request: McpUrlRecoveryPromptRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpUrlRecoveryAnswer, McpElicitationPromptError>> {
        unavailable(request, cancellation)
    }
}

fn unavailable<T: Send + 'static, R: 'static>(
    request: T,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<R, McpElicitationPromptError>> {
    Box::pin(async move {
        drop(request);
        Err(if cancellation.is_cancelled() {
            McpElicitationPromptError::Cancelled
        } else {
            McpElicitationPromptError::Unavailable
        })
    })
}

#[derive(Default, Debug)]
pub struct UnavailableMcpElicitationPresenter;
impl McpElicitationPresenter for UnavailableMcpElicitationPresenter {
    fn present(
        &self,
        request: McpElicitationPromptRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpElicitationAnswer, McpElicitationPromptError>> {
        unavailable(request, cancellation)
    }
}
