//! Explicit host policy for tools whose standalone contract skips permission.

use std::fmt;
use std::sync::Arc;

use machine_god_core::{
    BoxFuture, CancellationToken, EngineLimits, PreparedToolCall, Tool, ToolCall, ToolCallId,
    ToolContext, ToolError, ToolErrorKind, ToolExecution, ToolInputLimits, ToolName, ToolOutput,
    ToolOutputLimits, ToolSpec,
};
use serde_json::Value;

use crate::tool_output_serializer::{CompactToolOutputLimits, measure_json_value_compact};

/// Opt-in policy wrapper over one explicitly supplied trusted tool allocation.
///
/// Calls with concrete capabilities keep them unchanged. Otherwise the host
/// receives an exact tool-name/call-ID/prepared-arguments capability. This does
/// not alter the standalone tool or infer default approval from its risk.
pub struct NativePermissionGovernedTool {
    tool: Arc<dyn Tool>,
    limits: EngineLimits,
}

impl NativePermissionGovernedTool {
    /// Construction is inert; the supplied limits must match the hosting engine.
    #[must_use]
    pub fn new(tool: Arc<dyn Tool>, limits: EngineLimits) -> Self {
        Self { tool, limits }
    }

    fn govern(
        &self,
        prepared: PreparedToolCall,
        name: ToolName,
        call_id: ToolCallId,
    ) -> Result<PreparedToolCall, ToolError> {
        if prepared.capability().is_some() {
            return Ok(prepared);
        }
        let complete = self.tool.complete_input_limits();
        let limits = CompactToolOutputLimits {
            output_bytes: complete.map_or(self.limits.max_tool_argument_bytes.get(), |limits| {
                limits.max_prepared_argument_bytes.get()
            }),
            json_depth: self.limits.max_json_depth.get(),
            json_nodes: complete.map_or(self.limits.max_json_nodes.get(), |limits| {
                limits.max_prepared_argument_nodes.get()
            }),
        };
        // Bound the trusted tool's normalized value before creating the extra
        // capability copy. Core separately checks the complete framed capability.
        measure_json_value_compact(prepared.arguments(), limits, &CancellationToken::new())
            .map_err(|_| {
                ToolError::new(
                    ToolErrorKind::InvalidInput,
                    "permission_preparation_failed",
                    "permission preparation failed",
                    false,
                )
            })?;
        Ok(prepared.require_tool_permission(name, call_id))
    }
}

impl fmt::Debug for NativePermissionGovernedTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativePermissionGovernedTool { .. }")
    }
}

impl Tool for NativePermissionGovernedTool {
    fn spec(&self) -> ToolSpec {
        self.tool.spec()
    }

    fn complete_input_limits(&self) -> Option<ToolInputLimits> {
        self.tool.complete_input_limits()
    }

    fn complete_output_limits(&self) -> Option<ToolOutputLimits> {
        self.tool.complete_output_limits()
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        let name = call.name.clone();
        let call_id = call.id.clone();
        self.govern(self.tool.prepare(call)?, name, call_id)
    }

    fn prepare_for_turn(
        &self,
        context: &ToolContext,
        call: ToolCall,
    ) -> Result<PreparedToolCall, ToolError> {
        let name = call.name.clone();
        let call_id = call.id.clone();
        self.govern(self.tool.prepare_for_turn(context, call)?, name, call_id)
    }

    fn persist_arguments<'a>(
        &'a self,
        context: ToolContext,
        arguments: &'a Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        self.tool
            .persist_arguments(context, arguments, cancellation)
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        self.tool.execute(context, arguments, cancellation)
    }

    fn execute_for_turn(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        self.tool.execute_for_turn(context, arguments, cancellation)
    }
}
