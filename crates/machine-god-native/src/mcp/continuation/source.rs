//! Concrete native operation selection; no callback can invent input authority.

use super::{ContinuationInput, rejected};
use crate::mcp::{
    browser_launcher::{
        NativeMcpBrowserLaunchError, NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLauncher,
    },
    interaction::{McpElicitationAnswer, McpElicitationPromptRequest},
    mrtr::{McpElicitationRequest, McpInputRequired},
    runtime::{NativeMcpRuntimeFeatureCall, NativeMcpRuntimeToolCall},
};
use machine_god_core::{BoxFuture, CancellationToken, ToolError};
use std::sync::Arc;

pub(super) enum InputSource<'a> {
    Tool {
        call: &'a NativeMcpRuntimeToolCall,
        input: &'a ContinuationInput,
    },
    Feature(&'a NativeMcpRuntimeFeatureCall),
}
impl InputSource<'_> {
    pub(super) fn required(&self) -> &McpInputRequired {
        match self {
            Self::Tool { input, .. } => input.required.required(),
            Self::Feature(call) => call.input().as_ref(),
        }
    }

    pub(super) fn revalidate(&self) -> Result<(), ToolError> {
        match self {
            Self::Tool { call, .. } => call.revalidate(),
            Self::Feature(call) => call.revalidate(),
        }
    }

    pub(super) fn check_interaction_deadline(&self) -> Result<(), ToolError> {
        match self {
            Self::Tool { call, .. } => call.check_interaction_deadline(),
            Self::Feature(call) => call.check_interaction_deadline(),
        }
    }

    pub(super) fn interaction_cancelled(&self) -> BoxFuture<'static, ()> {
        match self {
            Self::Tool { call, .. } => call.interaction_cancelled(),
            Self::Feature(call) => call.interaction_cancelled(),
        }
    }

    pub(super) fn prompt(
        &self,
        request: Arc<McpElicitationRequest>,
    ) -> Result<McpElicitationPromptRequest, ToolError> {
        match self {
            Self::Tool { call, .. } => McpElicitationPromptRequest::new(
                call.context().clone(),
                Arc::from(call.server_name()),
                call.tool_name().clone(),
                request,
            )
            .map_err(|_| rejected()),
            Self::Feature(call) => call.prompt(request),
        }
    }

    pub(super) fn launch_url(
        &self,
        request: &Arc<McpElicitationRequest>,
        answer: &McpElicitationAnswer,
        launcher: &NativeMcpBrowserLauncher,
        cancellation: CancellationToken,
    ) -> Result<
        BoxFuture<'static, Result<NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLaunchError>>,
        ToolError,
    > {
        match self {
            Self::Tool { call, input } => {
                call.launch_url(input, request, answer, launcher, cancellation)
            }
            Self::Feature(call) => call.launch_url(request, answer, launcher, cancellation),
        }
    }
}
