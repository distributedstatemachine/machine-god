#![doc = include_str!("../../../docs/core-api.md")]
#![forbid(unsafe_code)]

mod cancel;
mod engine;
mod error;
mod event;
mod id;
mod model;
mod permission;
mod session;
mod tool;

pub use cancel::{CancellationToken, Cancelled};
pub use engine::{Engine, EngineBuilder};
pub use error::{
    BuildError, EngineError, EventSinkError, PermissionError, ProviderError, ProviderErrorKind,
    SessionStoreError, SessionStoreErrorKind, ToolError, ToolErrorKind,
};
pub use event::{EngineEvent, EventSink, NoopEventSink, TurnEvent};
pub use id::{InvalidId, PermissionRequestId, SessionId, ToolCallId, ToolName, TurnId};
pub use model::{
    ContentBlock, InferenceOptions, Message, ModelEvent, ModelEventStream, ModelProvider,
    ModelRequest, Role, StopReason, TokenUsage,
};
pub use permission::{
    Capability, FilesystemAccess, NetworkTarget, PermissionDecision, PermissionGrantScope,
    PermissionHandler, PermissionRequest, PermissionRisk,
};
pub use session::{
    Prompt, Session, SessionRecord, SessionRevision, SessionStore, Turn, TurnHandle,
};
pub use tool::{Tool, ToolCall, ToolContext, ToolOutput, ToolSpec};

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
