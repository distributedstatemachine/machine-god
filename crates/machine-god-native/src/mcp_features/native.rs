//! Production adapter. The runtime, not JSON data, supplies feature authority.
use super::{
    IterativeJsonValue, McpFeaturePublication, check_cancellation, decode_canonical,
    prepare_request, tool_spec,
};
use crate::{
    NativeToolResultArchiveAdapter, NativeToolResultArchiveLimits,
    mcp::{
        feature::McpFeatureCodecError,
        runtime::{NativeMcpFeatureError, NativeMcpRuntime, NativeMcpRuntimeError},
    },
};
use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolCall, Tool, ToolCall, ToolContext, ToolError,
    ToolErrorKind, ToolExecution, ToolOutput, ToolOutputLimits, ToolSpec,
};
use serde_json::Value;
use std::{
    fmt,
    num::NonZeroUsize,
    sync::{Arc, Weak},
};

mod projection;
#[cfg(test)]
mod tests;

// Full bounded wire data plus trusted-envelope escaping, not the portable
// adapter's 64 KiB limit or the normalized tool-result content limit.
const CONTENT_BYTES: usize = 16 * 1024 * 1024 + 512 * 1024;
const OUTPUT_BYTES: usize = CONTENT_BYTES + 29;
const OUTPUT_NODES: usize = 262_144 + 64;

/// Native feature tool sharing the real result archive and an exact runtime
/// lineage. Construction is inert; the weak reference prevents engine cycles.
pub struct NativeMcpFeaturesTool {
    runtime: Weak<NativeMcpRuntime>,
    archive: Arc<NativeToolResultArchiveAdapter>,
}
impl NativeMcpFeaturesTool {
    #[must_use]
    pub fn new(
        runtime: Weak<NativeMcpRuntime>,
        archive: Arc<NativeToolResultArchiveAdapter>,
    ) -> Self {
        Self { runtime, archive }
    }
}
impl fmt::Debug for NativeMcpFeaturesTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpFeaturesTool { <redacted> }")
    }
}
impl Tool for NativeMcpFeaturesTool {
    fn spec(&self) -> ToolSpec {
        tool_spec()
    }

    fn complete_output_limits(&self) -> Option<ToolOutputLimits> {
        Some(ToolOutputLimits {
            max_serialized_bytes: NonZeroUsize::new(OUTPUT_BYTES).expect("positive bound"),
            max_json_nodes: NonZeroUsize::new(OUTPUT_NODES).expect("positive bound"),
        })
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        Ok(prepare_request(call)?.completion_wins_after_first_poll())
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        let operation = self.execute_for_turn(context, arguments, cancellation);
        Box::pin(async move { operation.await.map(ToolExecution::into_output) })
    }

    fn execute_for_turn(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        let arguments = IterativeJsonValue::new(arguments);
        Box::pin(async move {
            check_cancellation(&cancellation)?;
            let request = decode_canonical(arguments.get())?;
            drop(arguments);
            let runtime = self.runtime.upgrade().ok_or_else(unavailable)?;
            let publication = McpFeaturePublication::from(&request);
            let result = runtime
                .feature_for_turn(context.clone(), &request, cancellation.clone())
                .await
                .map_err(runtime_error)?;
            result.revalidate().map_err(runtime_error)?;
            check_cancellation(&cancellation)?;
            let (output, stop) = projection::project(&publication, result.reply(), &cancellation)?;
            // This checks the original selected publication, never a replacement.
            result.revalidate().map_err(runtime_error)?;
            check_cancellation(&cancellation)?;
            let execution = self
                .archive
                .publish(
                    context,
                    output,
                    NativeToolResultArchiveLimits {
                        compact_bytes: OUTPUT_BYTES,
                        json_nodes: OUTPUT_NODES,
                    },
                )
                .await?;
            // Keep the original witness and bounded result slot through durable
            // publication. Once polled, publication owns completion: cancellation
            // cannot erase its receipt or invite replay of a completed operation.
            drop(result);
            Ok(if stop {
                execution.finish_turn()
            } else {
                execution
            })
        })
    }
}

fn runtime_error(error: NativeMcpFeatureError) -> ToolError {
    match error {
        NativeMcpFeatureError::Runtime(NativeMcpRuntimeError::Cancelled) => super::cancelled(),
        NativeMcpFeatureError::Runtime(NativeMcpRuntimeError::Limit)
        | NativeMcpFeatureError::Codec(McpFeatureCodecError::Limit) => projection::limit(),
        NativeMcpFeatureError::Codec(McpFeatureCodecError::NotFound) => super::not_found(),
        NativeMcpFeatureError::Codec(McpFeatureCodecError::InvalidRequest) => {
            super::invalid_arguments()
        }
        NativeMcpFeatureError::Runtime(
            NativeMcpRuntimeError::Invalid | NativeMcpRuntimeError::Unavailable,
        )
        | NativeMcpFeatureError::Codec(_) => unavailable(),
    }
}
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "mcp_features_native_unavailable",
        "The MCP feature runtime is unavailable",
        false,
    )
}
