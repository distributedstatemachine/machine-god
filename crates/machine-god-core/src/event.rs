use crate::{
    BoxFuture, EventSinkError, ModelEvent, PermissionDecision, PermissionRequest, SessionId,
    StopReason, TokenUsage, ToolCall, ToolOutput, TurnId,
};
use serde::{Deserialize, Serialize};

/// Lifecycle payload carried by an [`EngineEvent`].
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnEvent {
    Started,
    Model {
        event: ModelEvent,
    },
    PermissionRequested {
        request: PermissionRequest,
    },
    PermissionResolved {
        request_id: crate::PermissionRequestId,
        decision: PermissionDecision,
    },
    ToolStarted {
        call: ToolCall,
    },
    ToolFinished {
        call_id: crate::ToolCallId,
        output: ToolOutput,
    },
    Completed {
        reason: StopReason,
        usage: TokenUsage,
    },
    Failed {
        component: String,
        code: String,
        message: String,
        retryable: bool,
    },
}

/// One ordered event from a turn.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EngineEvent {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub sequence: u64,
    pub payload: TurnEvent,
}

/// Optional object-safe observer. Returning an error terminates the turn.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>>;
}

/// An observer that deliberately discards events without acquiring authority.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        Box::pin(async { Ok(()) })
    }
}
