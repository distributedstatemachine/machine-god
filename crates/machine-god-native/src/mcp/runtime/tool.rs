use super::{NativeMcpRuntimeToolCall, route::ToolRoute};
use crate::tool_output_serializer::{CompactToolOutputLimits, measure_json_value_compact};
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, PreparedToolCall, Tool, ToolCall, ToolContext,
    ToolError, ToolErrorKind, ToolExecution, ToolInputLimits, ToolOutput, ToolOutputLimits,
    ToolSpec,
};
use serde_json::Value;
use std::sync::Arc;

pub(super) struct RuntimeTool(pub Arc<ToolRoute>);

impl Tool for RuntimeTool {
    fn complete_input_limits(&self) -> Option<ToolInputLimits> {
        self.0.policy.complete_input_limits
    }

    fn complete_output_limits(&self) -> Option<ToolOutputLimits> {
        self.0.policy.complete_output_limits
    }

    fn spec(&self) -> ToolSpec {
        self.0.spec.clone()
    }

    fn persist_arguments<'a>(
        &'a self,
        context: ToolContext,
        arguments: &'a Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        Box::pin(async move {
            validate_arguments(arguments, &cancellation)?;
            let turn = self
                .0
                .contexts
                .snapshot_for_tool(&context)
                .map_err(|_| unavailable())?;
            let server = self.0.server.upgrade().ok_or_else(unavailable)?;
            self.0.binding.live().map_err(|_| unavailable())?;
            if server.cancellation.is_cancelled() {
                return Err(unavailable());
            }
            if self.0.policy.complete_input_limits.is_none() {
                return Ok(None);
            }
            let result = self
                .0
                .executor
                .persist_arguments(context, &self.0.name, arguments, cancellation.clone())
                .await?;
            turn.revalidate().map_err(|_| unavailable())?;
            self.0.binding.live().map_err(|_| unavailable())?;
            if cancellation.is_cancelled() || server.cancellation.is_cancelled() {
                return Err(unavailable());
            }
            Ok(result)
        })
    }

    fn prepare(&self, _call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        Err(unavailable())
    }

    fn prepare_for_turn(
        &self,
        context: &ToolContext,
        call: ToolCall,
    ) -> Result<PreparedToolCall, ToolError> {
        self.0
            .contexts
            .snapshot_for_tool(context)
            .map_err(|_| unavailable())?;
        self.0.binding.live().map_err(|_| unavailable())?;
        let server = self.0.server.upgrade().ok_or_else(unavailable)?;
        if server.cancellation.is_cancelled()
            || call.name != self.0.name
            || call.id != context.call_id
        {
            return Err(unavailable());
        }
        validate_arguments(&call.arguments, &CancellationToken::new())?;
        // Shared typed permission preparation owns pinned schema/header
        // validation. Preserve original nulls, exact numbers and literal keys.
        Ok(PreparedToolCall::new(
            Capability::Tool {
                name: call.name,
                call_id: call.id,
                arguments: call.arguments.clone(),
            },
            call.arguments,
        ))
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            self.execute_for_turn(context, arguments, cancellation)
                .await
                .map(ToolExecution::into_output)
        })
    }

    fn execute_for_turn(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        Box::pin(async move {
            let call = NativeMcpRuntimeToolCall::claim(
                self.0.clone(),
                context,
                arguments,
                cancellation.clone(),
            )
            .await?;
            let turn = self
                .0
                .contexts
                .snapshot_for_tool(call.context())
                .map_err(|_| unavailable())?;
            let server = self.0.server.upgrade().ok_or_else(unavailable)?;
            let execution = self.0.executor.execute(call).await?;
            turn.revalidate().map_err(|_| unavailable())?;
            self.0.binding.live().map_err(|_| unavailable())?;
            if cancellation.is_cancelled() || server.cancellation.is_cancelled() {
                return Err(unavailable());
            }
            Ok(execution)
        })
    }
}

pub(super) fn validate_arguments(
    arguments: &Value,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    if !arguments.is_object() {
        return Err(unavailable());
    }
    measure_json_value_compact(
        arguments,
        CompactToolOutputLimits {
            output_bytes: 64 * 1024,
            json_depth: 64,
            json_nodes: 4096,
        },
        cancellation,
    )
    .map_err(|_| unavailable())?;
    Ok(())
}

pub(super) fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "mcp_runtime_unavailable",
        "The exact MCP runtime route is unavailable",
        false,
    )
}
