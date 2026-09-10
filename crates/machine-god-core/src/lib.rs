#![doc = include_str!("../../../docs/core-api.md")]
#![forbid(unsafe_code)]

mod background;
mod cancel;
mod engine;
mod error;
mod event;
mod id;
mod json_bounds;
mod model;
mod model_catalog;
mod permission;
mod session;
mod session_context;
mod subagent;
mod terminal;
mod terminal_action;
mod tool;

pub use background::{
    BackgroundClock, BackgroundCompletionRecord, BackgroundHandle, BackgroundOutputOwner,
    BackgroundProcessOutcome, BackgroundProcessRetainer, BackgroundProcessSpawner,
    BackgroundRecordLease, BackgroundRetentionPermit, BackgroundRunningRecord,
    BackgroundStartError, BackgroundStartErrorKind, BackgroundStartRequest, BackgroundStore,
    BackgroundSupervisor, MAX_BACKGROUND_COMMAND_BYTES, MAX_BACKGROUND_CWD_BYTES,
    OwnedBackgroundProcess, PreparedBackgroundProcess,
};
pub use cancel::{CancellationToken, Cancelled};
pub use engine::{Engine, EngineBuilder, EngineLimits, EngineRequester, MAX_SAFE_JSON_DEPTH};
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
    AvailableModel, InvalidModelId, InvalidModelIdReason, MAX_MODEL_ID_BYTES, ModelCatalog,
    ModelCatalogAccess, ModelCatalogProvider, PublicCatalogReason, validate_model_id,
};
pub use permission::{
    Capability, FilesystemAccess, NetworkTarget, PermissionAuthorization, PermissionDecision,
    PermissionExecutionAdmission, PermissionGrantScope, PermissionHandler, PermissionInvocation,
    PermissionRequest, PermissionRisk, ProcessEnvironment, ProcessInput,
};
pub use session::PermissionInvocationSnapshot;
pub use session::{
    Prompt, Session, SessionRecord, SessionReservation, SessionRevision, SessionStore,
    SessionStoreAccess, Turn, TurnHandle, TurnMetadataEditor, TurnMetadataSnapshot,
};
pub use session_context::{
    MAX_CONTEXT_SUMMARY_BYTES, MAX_SESSION_USER_CONTEXT_BYTES, SessionContextProjection,
    SessionTurnPreparation, SessionUserContext,
};
pub use subagent::{
    MAX_CONCURRENT_SUBAGENTS, MAX_CONCURRENT_SUBAGENTS_PER_PARENT_TURN,
    MAX_SUBAGENT_ARGUMENT_BYTES, MAX_SUBAGENT_JSON_DEPTH, MAX_SUBAGENT_JSON_NODES,
    MAX_SUBAGENT_NAME_BYTES, MAX_SUBAGENT_OUTCOME_BYTES, MAX_SUBAGENT_OUTPUT_BYTES,
    MAX_SUBAGENT_PROMPT_BYTES, SUBAGENT_TOOL_NAME, SubagentAuthority, SubagentAuthorityError,
    SubagentAuthorityErrorKind, SubagentOutcome, SubagentRequest, SubagentTool,
};
pub use terminal::{
    MAX_TERMINAL_HYPERLINK_BYTES, MAX_TERMINAL_HYPERLINKS, MAX_TERMINAL_ID_BYTES,
    MAX_TERMINAL_MONITOR_ID_BYTES, MAX_TERMINAL_MONITOR_PATTERN_BYTES, MAX_TERMINAL_SCREEN_CELLS,
    MAX_TERMINAL_SCREEN_TEXT_BYTES, MAX_TERMINAL_WRITE_BYTES, MAX_TERMINAL_WRITE_ITEMS,
    TerminalActorRole, TerminalAttention, TerminalAttentionState, TerminalBackend, TerminalCell,
    TerminalCellKind, TerminalCellStyle, TerminalClosePolicy, TerminalColor, TerminalContractError,
    TerminalCursor, TerminalCursorShape, TerminalDimensions, TerminalEventQuery, TerminalGap,
    TerminalHyperlink, TerminalLifecycle, TerminalModes, TerminalMonitorCondition,
    TerminalMonitorDefinition, TerminalMonitorEvent, TerminalMonitorEventReason, TerminalMonitorId,
    TerminalMonitorLifetime, TerminalMonitorOperation, TerminalMonitorState, TerminalNamedKey,
    TerminalNotifySchedule, TerminalProfile, TerminalReturnCondition, TerminalSchedule,
    TerminalScreen, TerminalScreenCursor, TerminalScreenUnavailableReason, TerminalSessionId,
    TerminalSignal, TerminalWaitRequest, TerminalWriteLease, TerminalWriteLeaseIntent,
    TerminalWritePayload, TerminalWriteRequest,
};
pub use terminal_action::{
    MAX_TERMINAL_ACTION_COMMAND_BYTES, MAX_TERMINAL_ACTION_OUTPUT_BYTES,
    MAX_TERMINAL_ACTION_RESULTS, MAX_TERMINAL_ACTION_TEXT_BYTES,
    MAX_TERMINAL_CHECKPOINT_PAYLOAD_BYTES, MAX_TERMINAL_EXEC_DURATION,
    MAX_TERMINAL_EXEC_STREAM_BYTES, MAX_TERMINAL_INITIAL_MONITORS, TerminalAction,
    TerminalActionErrorCode, TerminalActionRequest, TerminalActionResponse, TerminalActionResult,
    TerminalAllowedControls, TerminalCheckpointEnvelope, TerminalExecCapturedOutput,
    TerminalExecRequest, TerminalExecResult, TerminalExecStatus, TerminalListFilters,
    TerminalMonitorSummary, TerminalPersistenceLevel, TerminalRawRange, TerminalReturnOutcome,
    TerminalScreenRecovery, TerminalSessionFacts, TerminalShellSpec, TerminalStartRequest,
};
pub use tool::{
    PreparedToolAuthorization, PreparedToolCall, Tool, ToolCall, ToolContext, ToolExecution,
    ToolInputLimits, ToolOutput, ToolOutputLimits, ToolSpec, TurnToolRegistration,
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
