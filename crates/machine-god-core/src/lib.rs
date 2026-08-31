#![doc = include_str!("../../../docs/core-api.md")]
#![forbid(unsafe_code)]

mod cancel;
mod engine;
mod error;
mod event;
mod id;
mod model;
mod model_catalog;
mod permission;
mod session;
mod subagent;
mod tool;

pub use cancel::{CancellationToken, Cancelled};
pub use engine::{Engine, EngineBuilder, EngineLimits, MAX_SAFE_JSON_DEPTH};
pub use error::{
    BuildError, EngineError, EventSinkError, PermissionError, ProviderError, ProviderErrorKind,
    SessionStoreError, SessionStoreErrorKind, ToolError, ToolErrorKind,
};
pub use event::{EngineEvent, EventSink, NoopEventSink, TurnEvent};
pub use id::{
    InvalidId, PermissionRequestId, SessionId, SessionIncarnationId, ToolCallId, ToolName, TurnId,
};
pub use model::{
    ContentBlock, InferenceOptions, Message, ModelEvent, ModelEventStream, ModelProvider,
    ModelRequest, Role, StopReason, TokenUsage,
};
pub use model_catalog::{
    AvailableModel, InvalidModelId, InvalidModelIdReason, ModelCatalog, ModelCatalogAccess,
    ModelCatalogProvider, PublicCatalogReason,
};
pub use permission::{
    Capability, FilesystemAccess, NetworkTarget, PermissionDecision, PermissionGrantScope,
    PermissionHandler, PermissionRequest, PermissionRisk, ProcessEnvironment,
};
pub use session::{
    Prompt, Session, SessionRecord, SessionRevision, SessionStore, Turn, TurnHandle,
};
pub use subagent::{
    MAX_CONCURRENT_SUBAGENTS, MAX_CONCURRENT_SUBAGENTS_PER_PARENT_TURN,
    MAX_SUBAGENT_ARGUMENT_BYTES, MAX_SUBAGENT_JSON_DEPTH, MAX_SUBAGENT_JSON_NODES,
    MAX_SUBAGENT_NAME_BYTES, MAX_SUBAGENT_OUTCOME_BYTES, MAX_SUBAGENT_OUTPUT_BYTES,
    MAX_SUBAGENT_PROMPT_BYTES, SUBAGENT_TOOL_NAME, SubagentAuthority, SubagentAuthorityError,
    SubagentAuthorityErrorKind, SubagentOutcome, SubagentRequest, SubagentTool,
};
pub use tool::{
    PreparedToolAuthorization, PreparedToolCall, Tool, ToolCall, ToolContext, ToolExecution,
    ToolOutput, ToolSpec, TurnToolRegistration,
};

use core::future::Future;
use core::pin::Pin;

/// A sendable, dynamically dispatched future used by object-safe core traits.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Current public API version.
pub const API_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::API_VERSION;

    #[test]
    fn api_version_starts_at_one() {
        assert_eq!(API_VERSION, 1);
    }
}
