use super::{NativePrincipalMcpRequester, Selection, select_captured};
use machine_god_core::{
    AdmittedToolInvocation, BoxFuture, CancellationToken, PreparedToolCall, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolExecution, ToolInputLimits, ToolOutput,
    ToolOutputLimits, ToolSpec,
};
use serde_json::Value;
use std::sync::Arc;

/// Fixed engine entrypoints authenticate before invoking ordinary MCP adapters.
/// Feature dispatch retains the concrete native adapter's full execution receipt.
pub(crate) struct NativePrincipalMcpTool {
    requester: NativePrincipalMcpRequester,
    shape: Arc<dyn Tool>,
    features: bool,
}
impl NativePrincipalMcpTool {
    pub(super) fn fixed(requester: NativePrincipalMcpRequester, shape: Arc<dyn Tool>) -> Self {
        Self {
            requester,
            shape,
            features: false,
        }
    }
    pub(super) fn features(requester: NativePrincipalMcpRequester, shape: Arc<dyn Tool>) -> Self {
        Self {
            requester,
            shape,
            features: true,
        }
    }
    fn selected_tool(&self, selected: &Selection) -> Arc<dyn Tool> {
        if self.features {
            selected.features.clone()
        } else {
            self.shape.clone()
        }
    }
}
impl Tool for NativePrincipalMcpTool {
    fn spec(&self) -> ToolSpec {
        self.shape.spec()
    }
    fn complete_input_limits(&self) -> Option<ToolInputLimits> {
        self.shape.complete_input_limits()
    }
    fn complete_output_limits(&self) -> Option<ToolOutputLimits> {
        self.shape.complete_output_limits()
    }
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        self.shape.prepare(call)
    }
    fn prepare_for_turn(
        &self,
        context: &ToolContext,
        call: ToolCall,
    ) -> Result<PreparedToolCall, ToolError> {
        let registry = self.requester.0.upgrade().ok_or_else(unavailable)?;
        let selection = registry
            .select_context(context)
            .map_err(|_| unavailable())?;
        drop(registry);
        self.selected_tool(&selection)
            .prepare_for_turn(context, call)
    }
    fn persist_arguments<'a>(
        &'a self,
        context: ToolContext,
        arguments: &'a Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        let route = self.requester.capture(&context);
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            let selection = select_captured(route).map_err(|_| unavailable())?;
            self.selected_tool(&selection)
                .persist_arguments(context, arguments, cancellation)
                .await
        })
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async { Err(unavailable()) })
    }
    fn execute_for_turn(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        Box::pin(async { Err(unavailable()) })
    }
    fn execute_admitted(
        &self,
        invocation: AdmittedToolInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        let route = self.requester.capture(invocation.context());
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            let registry = self.requester.0.upgrade().ok_or_else(unavailable)?;
            let lease = registry
                .principals
                .claim_tool(&invocation)
                .map_err(|_| unavailable())?;
            let selection = select_captured(route).map_err(|_| unavailable())?;
            drop(registry);
            let tool = self.selected_tool(&selection);
            if !selection.matches_lease(&lease) {
                return Err(unavailable());
            }
            let result = tool.execute_admitted(invocation, cancellation).await;
            // Preserve complete output, persisted projection, next-round tool,
            // finish-turn and completion-wins receipts without reinterpreting them.
            drop(selection);
            drop(lease);
            result
        })
    }
}
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "mcp_principal_unavailable",
        "The MCP principal is unavailable",
        false,
    )
}
fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "mcp_principal_cancelled",
        "The MCP operation was cancelled",
        false,
    )
}
impl std::fmt::Debug for NativePrincipalMcpTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativePrincipalMcpTool")
            .finish_non_exhaustive()
    }
}
