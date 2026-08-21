use crate::{
    BoxFuture, CancellationToken, SessionId, SessionIncarnationId, ToolCallId, ToolError, ToolName,
    TurnId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>>;
}
