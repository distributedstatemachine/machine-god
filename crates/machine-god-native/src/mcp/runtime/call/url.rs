//! Browser effects retain the exact original tool grant through native spawn.

use super::super::tool::unavailable;
use super::NativeMcpRuntimeToolCall;
use crate::{
    NativeBackgroundOpenError,
    background_url_opener::launcher::LauncherGuard,
    mcp::{
        browser_launcher::{
            NativeMcpBrowserLaunchError, NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLauncher,
            NativeMcpBrowserUrl,
        },
        context::NativeMcpTurnContext,
        continuation::ContinuationInput,
        interaction::McpElicitationAnswer,
        mrtr::{McpElicitationAction, McpElicitationRequest, McpInputRequestPayload},
        runtime::NativeMcpRuntimeClock,
        submission::{McpContinuationCustody, McpSubmissionRuntime},
    },
};
use machine_god_core::{BoxFuture, CancellationToken, ToolError};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

struct UrlGuard {
    custody: Arc<McpContinuationCustody>,
    turn: Arc<NativeMcpTurnContext>,
    binding: Arc<McpSubmissionRuntime>,
    cancellation: CancellationToken,
    server: CancellationToken,
    clock: Arc<dyn NativeMcpRuntimeClock>,
    deadline: Instant,
}
impl LauncherGuard for UrlGuard {
    fn check(&self) -> Result<(), NativeBackgroundOpenError> {
        if self.clock.now() >= self.deadline {
            return Err(NativeBackgroundOpenError::TimedOut);
        }
        if self.cancellation.is_cancelled()
            || self.server.is_cancelled()
            || self.custody.revalidate().is_err()
            || self.turn.revalidate().is_err()
            || self.binding.live().is_err()
        {
            return Err(NativeBackgroundOpenError::Cancelled);
        }
        Ok(())
    }
}

impl NativeMcpRuntimeToolCall {
    pub(crate) fn launch_url(
        &self,
        input: &ContinuationInput,
        request: &Arc<McpElicitationRequest>,
        answer: &McpElicitationAnswer,
        launcher: &NativeMcpBrowserLauncher,
        cancellation: CancellationToken,
    ) -> Result<
        BoxFuture<'static, Result<NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLaunchError>>,
        ToolError,
    > {
        self.revalidate()?;
        self.check_interaction_deadline()?;
        if answer.action() != McpElicitationAction::Accept
            || !Arc::ptr_eq(&self.round, &input.round)
            || !self.round.load(Ordering::Acquire)
            || !input.required.required().requests().iter().any(|item| {
                matches!(item.payload(), McpInputRequestPayload::Elicitation(selected) if Arc::ptr_eq(selected, request))
            }) {
            return Err(unavailable());
        }
        let url = request.url().ok_or_else(unavailable)?;
        let url = match NativeMcpBrowserUrl::new(url) {
            Ok(url) => url,
            Err(error) => return Ok(Box::pin(async move { Err(error) })),
        };
        let deadline = self.interaction_deadline.ok_or_else(unavailable)?;
        let guard = Arc::new(UrlGuard {
            custody: self.custody.clone(),
            turn: self.turn.clone(),
            binding: self.tool.binding.clone(),
            cancellation: self.cancellation.clone(),
            server: self.server.cancellation.clone(),
            clock: self.server.clock.clone(),
            deadline,
        });
        Ok(launcher.launch_guarded(
            url,
            cancellation,
            self.server.cancellation.clone(),
            deadline,
            guard,
        ))
    }
}
