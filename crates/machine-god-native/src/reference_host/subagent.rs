//! Managed tool publication using the execution domain's existing archive owner.

use std::{num::NonZeroUsize, sync::Arc};

use machine_god_core::{
    AdmittedToolInvocation, BoxFuture, CancellationToken, MAX_SUBAGENT_ARGUMENT_BYTES,
    MAX_SUBAGENT_JSON_DEPTH, MAX_SUBAGENT_JSON_NODES, MAX_SUBAGENT_OUTPUT_BYTES,
    ManagedSubagentAuthority, PreparedToolCall, SubagentTool, Tool, ToolCall, ToolContext,
    ToolError, ToolErrorKind, ToolExecution, ToolInputLimits, ToolOutput, ToolOutputLimits,
    ToolSpec,
};
use serde_json::Value;

use crate::{
    NativeToolArgumentsArchiveLimits, NativeToolResultArchiveAdapter,
    NativeToolResultArchiveLimits,
    tool_output_serializer::{CompactToolOutputLimits, measure_json_value_compact},
};

// The closed managed result bounds its content; ToolOutput adds its envelope.
const COMPLETE_OUTPUT_BYTES: usize = MAX_SUBAGENT_OUTPUT_BYTES + 32;

pub(super) struct NativeManagedSubagentTool {
    inner: SubagentTool,
    archive: Arc<NativeToolResultArchiveAdapter>,
}

impl NativeManagedSubagentTool {
    pub(super) fn new(
        authority: Arc<dyn ManagedSubagentAuthority>,
        archive: Arc<NativeToolResultArchiveAdapter>,
    ) -> Self {
        Self {
            inner: SubagentTool::shared_authority(authority),
            archive,
        }
    }
}

impl Tool for NativeManagedSubagentTool {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }

    fn complete_input_limits(&self) -> Option<ToolInputLimits> {
        Some(ToolInputLimits {
            max_argument_bytes: bound(MAX_SUBAGENT_ARGUMENT_BYTES),
            max_argument_nodes: bound(MAX_SUBAGENT_JSON_NODES),
            max_prepared_argument_bytes: bound(MAX_SUBAGENT_ARGUMENT_BYTES),
            max_prepared_argument_nodes: bound(MAX_SUBAGENT_JSON_NODES),
        })
    }

    fn complete_output_limits(&self) -> Option<ToolOutputLimits> {
        Some(ToolOutputLimits {
            max_serialized_bytes: bound(COMPLETE_OUTPUT_BYTES),
            max_json_nodes: bound(COMPLETE_OUTPUT_BYTES),
        })
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        self.inner.prepare(call)
    }

    fn prepare_for_turn(
        &self,
        context: &ToolContext,
        call: ToolCall,
    ) -> Result<PreparedToolCall, ToolError> {
        self.inner.prepare_for_turn(context, call)
    }

    fn persist_arguments<'a>(
        &'a self,
        context: ToolContext,
        arguments: &'a Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        Box::pin(async move {
            // This hook precedes action admission. Its only effect is bounded
            // lossless input publication, never child admission or execution.
            measure_json_value_compact(
                arguments,
                CompactToolOutputLimits {
                    output_bytes: MAX_SUBAGENT_ARGUMENT_BYTES,
                    json_depth: MAX_SUBAGENT_JSON_DEPTH,
                    json_nodes: MAX_SUBAGENT_JSON_NODES,
                },
                &cancellation,
            )
            .map_err(|_| {
                ToolError::new(
                    if cancellation.is_cancelled() {
                        ToolErrorKind::Cancelled
                    } else {
                        ToolErrorKind::InvalidInput
                    },
                    "subagent_input_publication_rejected",
                    "managed subagent input publication rejected",
                    false,
                )
            })?;
            self.archive
                .publish_arguments(
                    context,
                    arguments.clone(),
                    cancellation,
                    NativeToolArgumentsArchiveLimits {
                        compact_bytes: MAX_SUBAGENT_ARGUMENT_BYTES,
                        json_nodes: MAX_SUBAGENT_JSON_NODES,
                    },
                )
                .await
        })
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        // The structural route intentionally remains closed.
        self.inner.execute(context, arguments, cancellation)
    }

    fn execute_admitted(
        &self,
        invocation: AdmittedToolInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        let context = invocation.context().clone();
        let execution = self.inner.execute_admitted(invocation, cancellation);
        Box::pin(async move {
            // SubagentTool emits only a typed managed result, never dynamic
            // registrations or finish-turn directives. Do not discard effects
            // from an arbitrary Tool implementation through this concrete seam.
            let output = execution.await?.into_output();
            self.archive
                .publish(
                    context,
                    output,
                    NativeToolResultArchiveLimits {
                        compact_bytes: COMPLETE_OUTPUT_BYTES,
                        json_nodes: COMPLETE_OUTPUT_BYTES,
                    },
                )
                .await
        })
    }
}

fn bound(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("fixed nonzero managed tool bound")
}

#[cfg(test)]
mod tests;
