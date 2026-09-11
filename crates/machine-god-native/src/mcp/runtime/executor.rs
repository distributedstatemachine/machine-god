use super::{NativeMcpRuntimeError as Error, Result, call::NativeMcpRuntimeToolCall};
use machine_god_core::{
    BoxFuture, CancellationToken, ToolContext, ToolError, ToolExecution, ToolInputLimits, ToolName,
    ToolOutputLimits,
};
use serde_json::Value;

/// Immutable host policy, never inferred from a server capability or tool hint.
/// Form/URL flags assert that the supplied executor owns an actual responder.
/// Archive limits opt into core's existing independent persistence contracts.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeMcpToolExecutionPolicy {
    pub form: bool,
    pub url: bool,
    pub progress: bool,
    pub complete_input_limits: Option<ToolInputLimits>,
    pub complete_output_limits: Option<ToolOutputLimits>,
}
impl NativeMcpToolExecutionPolicy {
    pub(super) fn validate(self) -> Result<Self> {
        if self.complete_input_limits.is_some_and(|limits| {
            limits.max_argument_bytes.get() > 64 * 1024
                || limits.max_argument_nodes.get() > 4096
                || limits.max_prepared_argument_bytes.get() > 68 * 1024
                || limits.max_prepared_argument_nodes.get() > 4224
        }) || self.complete_output_limits.is_some_and(|limits| {
            limits.max_serialized_bytes.get() > 16 * 1024 * 1024
                || limits.max_json_nodes.get() > 1024 * 1024
        }) {
            return Err(Error::Limit);
        }
        Ok(self)
    }
}

/// Explicit native result, archive and interaction owner. Production hosts must
/// supply this implementation; the runtime has no fake/default result policy.
/// The owned call exposes one exact proof-bearing exchange, not arbitrary I/O.
pub trait NativeMcpToolExecutor: Send + Sync + 'static {
    fn execute(
        &self,
        call: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, std::result::Result<ToolExecution, ToolError>>;

    /// Only explicitly selected input-archive effects are permitted here.
    /// The original arguments are unchanged; returned references grant nothing.
    fn persist_arguments<'a>(
        &'a self,
        _context: ToolContext,
        _tool: &'a ToolName,
        _arguments: &'a Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'a, std::result::Result<Option<Value>, ToolError>> {
        Box::pin(async { Ok(None) })
    }
}
