//! Human feature data retains its actual command lifetime through presentation.

use super::{CancelOnDrop, ControlFuture, ControlPermit, Error, Receipt};
use crate::{
    McpFeatureAction, McpFeatureRequest, NativeConversationRuntime,
    mcp::{
        control::McpFeatureReply,
        feature::McpFeatureOutcome,
        runtime::{
            NativeMcpFeatureError, NativeMcpFeatureResult, NativeMcpHumanCommand, NativeMcpRuntime,
            NativeMcpRuntimeError,
        },
    },
};
use machine_god_core::CancellationToken;
use std::{fmt, sync::Arc};

/// Complete bounded external data, not a model turn or continuing permission.
/// The original human owner and result slot remain retained until receipt drop.
pub struct NativeMcpHumanFeatureReceipt {
    action: McpFeatureAction,
    server: Box<str>,
    result: NativeMcpFeatureResult,
    _owner: NativeMcpHumanCommand,
    _cancellation: CancelOnDrop,
}
impl NativeMcpHumanFeatureReceipt {
    #[must_use]
    pub const fn action(&self) -> McpFeatureAction {
        self.action
    }
    #[must_use]
    pub fn server(&self) -> &str {
        &self.server
    }
    /// Data only; this does not assert that its original generation is still live.
    #[must_use]
    pub fn reply(&self) -> &McpFeatureReply {
        self.result.reply()
    }
    /// # Errors
    /// Rejects cancellation or retirement of the exact original native selection.
    pub fn revalidate(&self) -> Result<(), NativeMcpFeatureError> {
        self.result.revalidate()
    }
    /// Protocol failures and unresolved input handoffs are not completed actions.
    #[must_use]
    pub fn failed(&self) -> bool {
        matches!(self.reply(), McpFeatureReply::Response(response)
            if matches!(response.outcome(), McpFeatureOutcome::ProtocolFailure { .. }
                | McpFeatureOutcome::UnvalidatedInputRequired))
    }
}
impl fmt::Debug for NativeMcpHumanFeatureReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpHumanFeatureReceipt { <redacted> }")
    }
}

pub(super) fn run(
    conversation: Arc<NativeConversationRuntime>,
    runtime: Arc<NativeMcpRuntime>,
    request: McpFeatureRequest,
    cancellation: CancellationToken,
) -> ControlFuture {
    let cancelled = CancelOnDrop(cancellation.clone());
    Box::pin(async move {
        if cancellation.is_cancelled() {
            return Err(Error::McpFeature(NativeMcpRuntimeError::Cancelled.into()));
        }
        let _permit = ControlPermit::acquire(&conversation)?;
        let source = crate::interactive_session::transition::principal(&conversation);
        let owner = runtime.human_command();
        let result = owner
            .feature_interactive(&request, cancellation, &source)
            .await
            .map_err(Error::McpFeature)?;
        result.revalidate().map_err(Error::McpFeature)?;
        Ok(Receipt::McpFeature(NativeMcpHumanFeatureReceipt {
            action: request.action(),
            server: request.server().into(),
            result,
            _owner: owner,
            _cancellation: cancelled,
        }))
    })
}
