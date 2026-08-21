use crate::{
    BoxFuture, CancellationToken, Capability, SessionId, SessionIncarnationId, ToolCallId,
    ToolError, ToolName, TurnId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Model-visible description and JSON Schema input contract for a tool.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolSpec {
    pub name: ToolName,
    pub description: String,
    pub input_schema: Value,
}

/// A complete tool invocation requested by a provider.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: ToolName,
    pub arguments: Value,
}

/// Explicit context passed to a tool invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolContext {
    pub session_id: SessionId,
    pub session_incarnation_id: SessionIncarnationId,
    pub turn_id: TurnId,
    pub call_id: ToolCallId,
}

/// Effect-free result of preparing one model-requested tool call.
///
/// The owned capability is the exact policy input for this invocation. The
/// owned arguments are passed unchanged to [`Tool::execute`] only after policy
/// allows that capability.
pub struct PreparedToolCall {
    capability: Capability,
    arguments: Value,
}

impl PreparedToolCall {
    /// Creates a prepared call from its policy capability and execution input.
    #[must_use]
    pub fn new(capability: Capability, arguments: Value) -> Self {
        Self {
            capability,
            arguments,
        }
    }

    /// Returns the exact capability that will be presented to policy.
    #[must_use]
    pub fn capability(&self) -> &Capability {
        &self.capability
    }

    /// Returns the exact arguments that will be passed to execution.
    #[must_use]
    pub fn arguments(&self) -> &Value {
        &self.arguments
    }

    pub(crate) fn into_arguments(mut self) -> Value {
        std::mem::take(&mut self.arguments)
    }
}

impl fmt::Debug for PreparedToolCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedToolCall")
            .finish_non_exhaustive()
    }
}

impl Drop for PreparedToolCall {
    fn drop(&mut self) {
        match &mut self.capability {
            Capability::Tool { arguments, .. } => {
                crate::session::drop_json_value_iterative(std::mem::take(arguments));
            }
            Capability::Custom { details, .. } => {
                crate::session::drop_json_value_iterative(std::mem::take(details));
            }
            Capability::Filesystem { .. }
            | Capability::Process { .. }
            | Capability::Network { .. } => {}
        }
        crate::session::drop_json_value_iterative(std::mem::take(&mut self.arguments));
    }
}

/// Provider-neutral tool output.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolOutput {
    pub content: Value,
    pub is_error: bool,
}

impl ToolOutput {
    #[must_use]
    pub fn success(content: impl Into<Value>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }
}

/// Object-safe tool implementation supplied explicitly by a host.
pub trait Tool: Send + Sync + 'static {
    fn spec(&self) -> ToolSpec;

    /// Validates and normalizes a call without exercising external authority.
    ///
    /// The default preserves the original critical-risk tool authorization:
    /// policy sees the registered name, call ID, and model arguments, and
    /// execution receives those same arguments.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] when the implementation cannot validate or
    /// normalize the call without exercising external authority.
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        let ToolCall {
            id,
            name,
            arguments,
        } = call;
        let capability = Capability::Tool {
            name,
            call_id: id,
            arguments: arguments.clone(),
        };
        Ok(PreparedToolCall::new(capability, arguments))
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>>;
}
