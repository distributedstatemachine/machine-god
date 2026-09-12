//! Human read/get rounds keep native command authority, not model tool grants.

use super::{Result, exchange};
use crate::{
    McpFeatureAction, McpFeatureRequest, NativeBackgroundOpenError,
    background_url_opener::launcher::LauncherGuard,
    mcp::{
        browser_launcher::{
            NativeMcpBrowserLaunchError, NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLauncher,
            NativeMcpBrowserUrl,
        },
        continuation::collect_feature_input,
        control::{McpFeatureControlAuthority, McpFeatureReply, McpFeatureRound},
        interaction::{McpElicitationAnswer, McpElicitationPresenter, McpElicitationPromptRequest},
        mrtr::{
            McpElicitationAction, McpElicitationRequest, McpInputRequestPayload, McpInputRequired,
        },
        runtime::{
            NativeMcpRuntimeClock, NativeMcpRuntimeError, route::ServerRoute, tool::unavailable,
        },
    },
};
use futures_util::future::select;
use machine_god_core::{BackgroundOutputOwner, BoxFuture, CancellationToken, ToolError};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(test)]
mod tests;

pub(in crate::mcp::runtime) struct FeatureInputEndpoint {
    pub presenter: Arc<dyn McpElicitationPresenter>,
    pub launcher: Option<NativeMcpBrowserLauncher>,
}

pub(super) async fn complete(
    mut round: McpFeatureRound,
    server: &Arc<ServerRoute>,
    request: &McpFeatureRequest,
    authority: &McpFeatureControlAuthority,
    source: &BackgroundOutputOwner,
    endpoint: &FeatureInputEndpoint,
) -> Result<McpFeatureReply> {
    for _ in 0..8 {
        let Some(input) = round.input() else { break };
        let call = NativeMcpRuntimeFeatureCall::new(
            input.clone(),
            server.clone(),
            authority.clone(),
            source.clone(),
            request.action(),
        )?;
        let responses = collect_feature_input(
            &call,
            endpoint.presenter.as_ref(),
            endpoint.launcher.as_ref(),
        )
        .await
        .map_err(|_| NativeMcpRuntimeError::Cancelled)?;
        call.revalidate()
            .map_err(|_| NativeMcpRuntimeError::Cancelled)?;
        call.check_interaction_deadline()
            .map_err(|_| NativeMcpRuntimeError::Cancelled)?;
        let Some(responses) = responses else { break };
        // Retire this human round before reacquiring the peer. Each continuation
        // gets a fresh operation deadline, never a fresh publication/descriptor.
        drop(call);
        let mut lane = server.acquire_feature(authority).await?;
        let deadline = lane.deadline;
        let response = lane.peer.resume_feature(round, responses, deadline);
        round = exchange::timed(server, authority, deadline, response).await?;
    }
    // Unhandled input and exhausted rounds remain a failed/unresolved receipt.
    Ok(round.into_reply())
}

pub(crate) struct NativeMcpRuntimeFeatureCall {
    input: Arc<McpInputRequired>,
    server: Arc<ServerRoute>,
    authority: McpFeatureControlAuthority,
    owner: BackgroundOutputOwner,
    action: McpFeatureAction,
    deadline: Instant,
    active: Arc<AtomicBool>,
}
impl NativeMcpRuntimeFeatureCall {
    fn new(
        input: Arc<McpInputRequired>,
        server: Arc<ServerRoute>,
        authority: McpFeatureControlAuthority,
        owner: BackgroundOutputOwner,
        action: McpFeatureAction,
    ) -> Result<Self> {
        if !matches!(
            action,
            McpFeatureAction::ResourceRead | McpFeatureAction::PromptGet
        ) || !authority.is_live()
        {
            return Err(NativeMcpRuntimeError::Cancelled.into());
        }
        let deadline = server
            .clock
            .now()
            .checked_add(Duration::from_secs(30 * 60))
            .ok_or(NativeMcpRuntimeError::Limit)?;
        Ok(Self {
            input,
            server,
            authority,
            owner,
            action,
            deadline,
            active: Arc::new(AtomicBool::new(true)),
        })
    }

    pub(crate) fn input(&self) -> &Arc<McpInputRequired> {
        &self.input
    }

    pub(crate) fn revalidate(&self) -> std::result::Result<(), ToolError> {
        if !self.active.load(Ordering::Acquire)
            || !self.authority.is_live()
            || self.server.check_authority().is_err()
        {
            return Err(unavailable());
        }
        Ok(())
    }

    pub(crate) fn check_interaction_deadline(&self) -> std::result::Result<(), ToolError> {
        (self.server.clock.now() < self.deadline)
            .then_some(())
            .ok_or_else(unavailable)
    }

    pub(crate) fn interaction_cancelled(&self) -> BoxFuture<'static, ()> {
        let authority = self.authority.clone();
        let clock = self.server.clock.clone();
        let deadline = self.deadline;
        Box::pin(async move {
            select(authority.cancelled(), clock.sleep_until(deadline)).await;
        })
    }

    pub(crate) fn prompt(
        &self,
        request: Arc<McpElicitationRequest>,
    ) -> std::result::Result<McpElicitationPromptRequest, ToolError> {
        self.revalidate()?;
        self.check_interaction_deadline()?;
        if !self.contains(&request) {
            return Err(unavailable());
        }
        McpElicitationPromptRequest::new_human_feature(
            self.owner.clone(),
            self.server.name.clone(),
            self.action,
            request,
        )
        .map_err(|_| unavailable())
    }

    fn contains(&self, request: &Arc<McpElicitationRequest>) -> bool {
        self.input.requests().iter().any(|item| {
            matches!(item.payload(), McpInputRequestPayload::Elicitation(selected) if Arc::ptr_eq(selected, request))
        })
    }

    pub(crate) fn launch_url(
        &self,
        request: &Arc<McpElicitationRequest>,
        answer: &McpElicitationAnswer,
        launcher: &NativeMcpBrowserLauncher,
        cancellation: CancellationToken,
    ) -> std::result::Result<
        BoxFuture<
            'static,
            std::result::Result<NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLaunchError>,
        >,
        ToolError,
    > {
        self.revalidate()?;
        self.check_interaction_deadline()?;
        if answer.action() != McpElicitationAction::Accept || !self.contains(request) {
            return Err(unavailable());
        }
        let url = match NativeMcpBrowserUrl::new(request.url().ok_or_else(unavailable)?) {
            Ok(url) => url,
            Err(error) => return Ok(Box::pin(async move { Err(error) })),
        };
        let guard = Arc::new(FeatureUrlGuard {
            authority: self.authority.clone(),
            clock: self.server.clock.clone(),
            deadline: self.deadline,
            active: self.active.clone(),
        });
        Ok(launcher.launch_guarded(
            url,
            cancellation,
            self.server.cancellation.clone(),
            self.deadline,
            guard,
        ))
    }
}
impl Drop for NativeMcpRuntimeFeatureCall {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

struct FeatureUrlGuard {
    authority: McpFeatureControlAuthority,
    clock: Arc<dyn NativeMcpRuntimeClock>,
    deadline: Instant,
    active: Arc<AtomicBool>,
}
impl LauncherGuard for FeatureUrlGuard {
    fn check(&self) -> std::result::Result<(), NativeBackgroundOpenError> {
        if self.clock.now() >= self.deadline {
            return Err(NativeBackgroundOpenError::TimedOut);
        }
        if !self.active.load(Ordering::Acquire) || !self.authority.is_live() {
            return Err(NativeBackgroundOpenError::Cancelled);
        }
        Ok(())
    }
}
