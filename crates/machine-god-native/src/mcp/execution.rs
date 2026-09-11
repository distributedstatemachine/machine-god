//! Concrete MCP exchange, exact result admission and explicitly owned archives.

use super::{
    runtime::{
        NativeMcpRuntimeToolCall, NativeMcpToolCompletionPolicy, NativeMcpToolExecutionPolicy,
        NativeMcpToolExecutor,
    },
    tool_result::{
        McpToolResponseContext, McpToolResponseDisposition, McpToolResultError,
        McpToolResultLimits, NativeMcpToolResultAdmission,
    },
};
use crate::native_tool_result_archive::{
    NativeToolArgumentsArchiveLimits, NativeToolResultArchiveAdapter, NativeToolResultArchiveLimits,
};
use crate::tool_output_serializer::{CompactToolOutputLimits, measure_json_value_compact};
use machine_god_core::{
    BoxFuture, CancellationToken, ToolContext, ToolError, ToolErrorKind, ToolExecution,
    ToolInputLimits, ToolName, ToolOutput, ToolOutputLimits,
};
use serde_json::Value;
use std::num::NonZeroUsize;
use std::sync::Arc;

mod projection;

const INPUT_BYTES: usize = 64 * 1024;
const INPUT_NODES: usize = 4096;
const OUTPUT_BYTES: usize = 4 * 1024 * 1024 + 16 * 1024 + 29;
const OUTPUT_NODES: usize = 262_144;
const POLICY: NativeMcpToolExecutionPolicy = NativeMcpToolExecutionPolicy {
    form: false,
    url: false,
    progress: true,
    completion: NativeMcpToolCompletionPolicy::CompletionWinsAfterFirstPoll,
    complete_input_limits: Some(ToolInputLimits {
        max_argument_bytes: NonZeroUsize::new(INPUT_BYTES).expect("positive bound"),
        max_argument_nodes: NonZeroUsize::new(INPUT_NODES).expect("positive bound"),
        max_prepared_argument_bytes: NonZeroUsize::new(68 * 1024).expect("positive bound"),
        max_prepared_argument_nodes: NonZeroUsize::new(4224).expect("positive bound"),
    }),
    complete_output_limits: Some(ToolOutputLimits {
        max_serialized_bytes: NonZeroUsize::new(OUTPUT_BYTES).expect("positive bound"),
        max_json_nodes: NonZeroUsize::new(OUTPUT_NODES).expect("positive bound"),
    }),
};

/// Required native archive authority and immutable complete-response admission.
/// Construction performs no I/O, spawning, clock reads or runtime discovery.
/// This owner does not retain an engine/runtime or advertise an input responder.
pub struct NativeMcpArchivedToolExecutor {
    archive: Arc<NativeToolResultArchiveAdapter>,
    admission: NativeMcpToolResultAdmission,
}
impl std::fmt::Debug for NativeMcpArchivedToolExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeMcpArchivedToolExecutor { <redacted> }")
    }
}
impl NativeMcpArchivedToolExecutor {
    /// Retains explicit archive authority, without preparing its directory.
    /// # Errors
    /// Propagates invalid response-admission limits.
    pub fn new(archive: Arc<NativeToolResultArchiveAdapter>) -> Result<Self, McpToolResultError> {
        Ok(Self {
            archive,
            admission: NativeMcpToolResultAdmission::new(McpToolResultLimits::default())?,
        })
    }

    /// Matching runtime policy; never infer responder support from remote hints.
    #[must_use]
    pub const fn execution_policy(&self) -> NativeMcpToolExecutionPolicy {
        POLICY
    }
}

impl NativeMcpToolExecutor for NativeMcpArchivedToolExecutor {
    fn execute(
        &self,
        mut call: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        Box::pin(async move {
            call.revalidate()?;
            let response = call.first_exchange().await?;
            call.revalidate()?;
            let context = McpToolResponseContext::new(
                call.context().clone(),
                call.tool_name().clone(),
                Arc::from(call.server_name()),
                call.descriptor().clone(),
                call.runtime().clone(),
                response.protocol(),
                response.request_id().clone(),
            )
            .map_err(|_| invalid_response())?;
            let admitted = self
                .admission
                .admit(context, response.bytes())
                .map_err(|_| invalid_response())?;
            drop(response);
            // The original invocation/options and exact live route remain owned
            // by call. Response provenance alone never grants another exchange.
            call.revalidate()?;
            let (output, finish) = match admitted {
                McpToolResponseDisposition::Complete(output) => (output, false),
                McpToolResponseDisposition::ProtocolFailure(failure) => {
                    (projection::protocol_failure(&failure)?, false)
                }
                McpToolResponseDisposition::InputRequired(required) => {
                    (projection::input_required(&required)?, true)
                }
            };
            call.revalidate()?;
            // Publication owns completion once polled. Never cancel it after
            // publication or replace a returned durable receipt with a retry.
            let execution = self
                .archive
                .publish(
                    call.context().clone(),
                    output,
                    NativeToolResultArchiveLimits {
                        compact_bytes: OUTPUT_BYTES,
                        json_nodes: OUTPUT_NODES,
                    },
                )
                .await?;
            Ok(if finish {
                execution.finish_turn()
            } else {
                execution
            })
        })
    }

    fn persist_arguments<'a>(
        &'a self,
        context: ToolContext,
        _tool: &'a ToolName,
        arguments: &'a Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        Box::pin(async move {
            if !arguments.is_object() || cancellation.is_cancelled() {
                return Err(invalid_response());
            }
            measure_json_value_compact(
                arguments,
                CompactToolOutputLimits {
                    output_bytes: INPUT_BYTES,
                    json_depth: 64,
                    json_nodes: INPUT_NODES,
                },
                &cancellation,
            )
            .map_err(|_| invalid_response())?;
            self.archive
                .publish_arguments(
                    context,
                    arguments.clone(),
                    cancellation,
                    NativeToolArgumentsArchiveLimits {
                        compact_bytes: INPUT_BYTES,
                        json_nodes: INPUT_NODES,
                    },
                )
                .await
        })
    }
}

fn invalid_response() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "mcp_tool_response_rejected",
        "The MCP tool response could not be admitted",
        false,
    )
}
