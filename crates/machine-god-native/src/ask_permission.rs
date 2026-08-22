use std::fmt;
use std::sync::Arc;

use machine_god_core::{
    BoxFuture, PermissionDecision, PermissionError, PermissionGrantScope, PermissionHandler,
    PermissionRequest,
};

/// Stable code returned when the host prompt fails to produce a decision.
pub const ASK_PERMISSION_PROMPT_ERROR_CODE: &str = "permission_prompt_failed";

/// Redacted message returned when the host prompt fails to produce a decision.
pub const ASK_PERMISSION_PROMPT_ERROR_MESSAGE: &str = "permission prompt failed";

/// Stable denial reason returned when the host rejects a prompt.
pub const ASK_PERMISSION_DENIED_REASON: &str = "permission denied";

/// A structured decision produced by a native permission prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionPromptDecision {
    /// Return a positive decision scoped to this request.
    AllowOnce,
    /// Return a turn-scoped positive decision.
    AllowTurn,
    /// Return a session-scoped positive decision.
    AllowSession,
    /// Reject the request.
    Deny,
}

/// A redacted failure to obtain a decision from a native permission prompt.
///
/// The type intentionally carries no source error or host-provided text, so
/// terminal, UI, or transport diagnostics cannot cross the policy boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PermissionPromptError;

impl PermissionPromptError {
    /// Creates a redacted prompt failure.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl fmt::Display for PermissionPromptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(ASK_PERMISSION_PROMPT_ERROR_MESSAGE)
    }
}

impl std::error::Error for PermissionPromptError {}

/// Executor-neutral host boundary for presenting a permission prompt.
///
/// The request is passed by value without transformation. Implementations must
/// keep the prompt work in the returned future: dropping that future must not
/// leave detached prompt work running. Host-specific terminal, UI, environment,
/// filesystem, or network authority belongs behind this interface.
pub trait PermissionPrompter: Send + Sync + 'static {
    /// Returns the host's decision for `request`.
    fn prompt(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>>;
}

/// Fail-closed [`PermissionHandler`] backed by an explicitly injected prompt.
///
/// Core validates and bounds every [`PermissionRequest`] before invoking the
/// handler. This adapter forwards that owned value directly and does not clone,
/// serialize, or traverse its capability or reason. It performs no prompt work
/// until the authorization future is polled and creates no detached task.
#[derive(Clone)]
pub struct AskPermissionHandler {
    prompter: Arc<dyn PermissionPrompter>,
}

impl AskPermissionHandler {
    /// Creates a handler backed by an owned `prompter`.
    #[must_use]
    pub fn new(prompter: impl PermissionPrompter) -> Self {
        Self {
            prompter: Arc::new(prompter),
        }
    }

    /// Creates a handler backed by a shared `prompter`.
    #[must_use]
    pub fn shared_prompter(prompter: Arc<dyn PermissionPrompter>) -> Self {
        Self { prompter }
    }
}

impl fmt::Debug for AskPermissionHandler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AskPermissionHandler")
            .finish_non_exhaustive()
    }
}

impl PermissionHandler for AskPermissionHandler {
    fn authorize(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        Box::pin(async move {
            let decision = self.prompter.prompt(request).await.map_err(|_| {
                PermissionError::new(
                    ASK_PERMISSION_PROMPT_ERROR_CODE,
                    ASK_PERMISSION_PROMPT_ERROR_MESSAGE,
                )
            })?;

            Ok(match decision {
                PermissionPromptDecision::AllowOnce => PermissionDecision::Allow {
                    scope: PermissionGrantScope::Once,
                },
                PermissionPromptDecision::AllowTurn => PermissionDecision::Allow {
                    scope: PermissionGrantScope::Turn,
                },
                PermissionPromptDecision::AllowSession => PermissionDecision::Allow {
                    scope: PermissionGrantScope::Session,
                },
                PermissionPromptDecision::Deny => PermissionDecision::Deny {
                    reason: ASK_PERMISSION_DENIED_REASON.to_owned(),
                },
            })
        })
    }
}
