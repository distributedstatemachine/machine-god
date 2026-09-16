use super::{
    MAX_SUBAGENT_MESSAGE_BYTES, MAX_SUBAGENT_OUTPUT_BYTES, MAX_SUBAGENT_PAGE_LIMIT, ManagedCursor,
    ManagedInspectWait, ManagedNotifications, ManagedPermissionMode, ManagedRelationshipAction,
    fmt, id_valid, serialized_json_size_bounded, text_valid,
};
use serde::{Deserialize, Serialize};

/// Fixed infrastructure failures; no paths, credentials or child output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedSubagentError {
    InvalidArguments,
    Unavailable,
    Failed,
    ResourceLimit,
    Cancelled,
}
impl fmt::Display for ManagedSubagentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("managed subagent operation rejected")
    }
}
impl std::error::Error for ManagedSubagentError {}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentState {
    Idle,
    Queued,
    Running,
    AwaitingApproval,
    Interrupted,
    Completed,
    Failed,
    Cancelled,
    Archived,
}
impl ManagedInspectWait {
    #[must_use]
    pub fn satisfied(&self, generation: u64, state: ManagedAgentState) -> bool {
        self.after_generation.is_none_or(|after| generation > after)
            && !matches!(
                state,
                ManagedAgentState::Queued
                    | ManagedAgentState::Running
                    | ManagedAgentState::AwaitingApproval
            )
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedOutcome {
    Created,
    MessageQueued,
    RelationshipChanged,
    Configured,
    LifecycleChanged,
    MilestoneEmitted,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedResultStatus {
    Accepted,
    Created,
    MessageQueued,
    RelationshipChanged,
    Configured,
    LifecycleChanged,
    MilestoneEmitted,
    Inspected,
    WaitTimedOut,
    Rejected,
    Idle,
    Queued,
    Running,
    AwaitingApproval,
    Interrupted,
    Completed,
    Failed,
    Cancelled,
    Archived,
}
/// Closed data-free rejection vocabulary. Host error strings are never tags.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedFailureCode {
    InvalidJson,
    InvalidRoot,
    MissingCommand,
    UnknownField,
    InvalidFieldType,
    MissingField,
    InvalidEnum,
    InvalidInteger,
    InvalidBranchSelection,
    InvalidNestedBranchSelection,
    MissingName,
    MissingMode,
    MissingOneOffPrompt,
    MissingInspectId,
    InvalidId,
    InvalidName,
    InvalidModel,
    InvalidPrompt,
    InvalidMessage,
    InvalidOperationId,
    InvalidNotificationPolicy,
    DuplicateMilestone,
    DuplicateStopCondition,
    InvalidInspectSections,
    InvalidInspectWait,
    InvalidCursor,
    InvalidPageLimit,
    InvalidRelationship,
    EmptyConfiguration,
    CallerUnavailable,
    ChildUnavailable,
    ControlNotFound,
    OperationIdRequired,
    OperationConflict,
    OperationReplayExpired,
    StaleGeneration,
    InvalidState,
    RelationshipAuthorizationRequired,
    RelationshipCycle,
    RelationshipAlreadyParented,
    RelationshipMissingParent,
    OneOffNotMessageable,
    MilestoneRequiresActiveWork,
    InvalidMilestoneCaller,
    NoActiveWork,
    UndeclaredMilestone,
    ControlLockBusy,
    ControlLockUnsupported,
    ControlRecordInvalid,
    ControlRecordTooLarge,
    CommunicationCapacityExceeded,
    ControlPathUnsafe,
    ControlCommitIndeterminate,
    SessionNotFound,
    GraphChanged,
    GraphTooDeep,
    InvalidSnapshotQuery,
    GenerationExhausted,
    StoreFailure,
    HostUnavailable,
    ResultTooLarge,
    ResourceLimit,
    PermissionDenied,
    Cancelled,
    DependencyCycle,
    DependencyUnavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedReceipt {
    pub outcome: ManagedOutcome,
    pub generation: u64,
    pub event_sequence: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedRelationshipApproval {
    pub action: ManagedRelationshipAction,
    pub approval_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ManagedRequested {
    Receipt(ManagedReceipt),
    Inspection(Box<ManagedInspection>),
    RelationshipApproval(ManagedRelationshipApproval),
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedSubagentResult {
    pub ok: bool,
    pub operation_id: String,
    pub child_id: Option<String>,
    pub status: ManagedResultStatus,
    pub error_code: Option<ManagedFailureCode>,
    pub retryable: bool,
    pub requested: Option<ManagedRequested>,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedConfiguration {
    pub name: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: ManagedPermissionMode,
    pub notifications: ManagedNotifications,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // Independent selected/truncated wire flags.
pub struct ManagedInspection {
    pub child_id: String,
    pub generation: u64,
    pub restart_required: bool,
    pub status: Option<ManagedAgentState>,
    pub configuration: Option<ManagedConfiguration>,
    pub relationship_selected: bool,
    pub parent_id: Option<String>,
    pub messages: Vec<ManagedQueuedMessage>,
    pub history: Vec<ManagedHistoryItem>,
    pub history_len: Option<u64>,
    pub history_truncated: bool,
    pub history_error: Option<ManagedInspectionSourceError>,
    pub events: Vec<ManagedEvent>,
    pub tool_activity_selected: bool,
    pub tool_activity: Vec<ManagedToolActivity>,
    pub tool_activity_truncated: bool,
    pub tool_activity_error: Option<ManagedInspectionSourceError>,
    pub failure_work_id: Option<String>,
    pub failure_reason: Option<String>,
    pub next_cursor: Option<String>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedInspectionSourceError {
    NotFound,
    Invalid,
    Unavailable,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedQueueStatus {
    Pending,
    Running,
    AwaitingApproval,
    Interrupted,
    Completed,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedQueuedMessage {
    pub id: String,
    pub source_id: String,
    pub content: String,
    pub status: ManagedQueueStatus,
    pub cancellation_reason: Option<String>,
    pub created_at_ms: i64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedHistoryKind {
    Conversation,
    BackgroundCommand,
    Interrupted,
    CompactedSummary,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedHistoryItem {
    pub kind: ManagedHistoryKind,
    pub work_id: Option<String>,
    pub user: Option<String>,
    pub assistant: Option<String>,
    pub user_truncated: bool,
    pub assistant_truncated: bool,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedToolPhase {
    Started,
    Succeeded,
    Failed,
    Denied,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedToolActivity {
    pub sequence: u64,
    pub revision: u64,
    pub timestamp_ms: i64,
    pub work_id: Option<String>,
    pub tool_name: String,
    pub phase: ManagedToolPhase,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedEvent {
    pub sequence: u64,
    pub revision: u64,
    pub id: String,
    pub timestamp_ms: i64,
    pub kind: ManagedEventKind,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedEventKind {
    Created,
    MessageQueued {
        message_id: String,
    },
    RelationshipChanged {
        previous_parent_id: Option<String>,
        parent_id: Option<String>,
    },
    Configured,
    LifecycleChanged {
        previous: ManagedAgentState,
        current: ManagedAgentState,
    },
    WorkTransition {
        work_item_id: String,
        previous: Option<ManagedQueueStatus>,
        current: ManagedQueueStatus,
        reason: Option<String>,
    },
    MilestoneRecorded {
        operation_id: String,
        source_child_id: String,
        target_parent_id: Option<String>,
        notice_emitted: bool,
        work_item_id: String,
        name: String,
    },
}
impl ManagedSubagentResult {
    /// Validates the complete envelope before serialization or publication.
    /// # Errors
    /// Rejects inconsistent envelopes and bounded-result overflow.
    pub fn validate(&self) -> Result<(), ManagedSubagentError> {
        let operation_valid = !self.operation_id.is_empty()
            && self.operation_id.len() <= 128
            && self
                .operation_id
                .bytes()
                .all(|b| !b.is_ascii_control() && !b.is_ascii_whitespace());
        if !operation_valid
            || self.child_id.as_ref().is_some_and(|id| !id_valid(id))
            || self.ok != self.error_code.is_none()
            || self.ok != self.requested.is_some()
            || (!self.ok && self.status != ManagedResultStatus::Rejected)
            || self
                .cursor
                .as_ref()
                .is_some_and(|cursor| ManagedCursor::parse(cursor).is_err())
        {
            return Err(ManagedSubagentError::InvalidArguments);
        }
        if let Some(ManagedRequested::Receipt(receipt)) = &self.requested
            && (self.status != receipt.outcome.status()
                || self.child_id.is_none()
                || self.cursor.is_some())
        {
            return Err(ManagedSubagentError::InvalidArguments);
        }
        if let Some(ManagedRequested::RelationshipApproval(approval)) = &self.requested
            && (self.status != ManagedResultStatus::AwaitingApproval
                || self.child_id.is_none()
                || self.cursor.is_some()
                || !id_valid(&approval.approval_id))
        {
            return Err(ManagedSubagentError::InvalidArguments);
        }
        if let Some(ManagedRequested::Inspection(v)) = &self.requested
            && (!id_valid(&v.child_id)
                || self.child_id.as_ref() != Some(&v.child_id)
                || v.messages
                    .len()
                    .saturating_add(v.history.len())
                    .saturating_add(v.events.len())
                    .saturating_add(v.tool_activity.len())
                    > MAX_SUBAGENT_PAGE_LIMIT
                || v.next_cursor != self.cursor
                || v.messages
                    .iter()
                    .any(|m| !text_valid(&m.content, MAX_SUBAGENT_MESSAGE_BYTES))
                || v.history.iter().any(|h| {
                    h.user.as_ref().is_some_and(|s| s.len() > 16 * 1024)
                        || h.assistant.as_ref().is_some_and(|s| s.len() > 16 * 1024)
                })
                || v.history.iter().fold(0usize, |bytes, h| {
                    bytes
                        .saturating_add(h.user.as_ref().map_or(0, String::len))
                        .saturating_add(h.assistant.as_ref().map_or(0, String::len))
                }) > 32 * 1024)
        {
            return Err(ManagedSubagentError::ResourceLimit);
        }
        // The DTO has fixed structural depth. Count before allocating its JSON.
        if !serialized_json_size_bounded(self, MAX_SUBAGENT_OUTPUT_BYTES)
            .is_ok_and(|size| size.is_some())
        {
            return Err(ManagedSubagentError::ResourceLimit);
        }
        Ok(())
    }
}

impl ManagedOutcome {
    #[must_use]
    pub const fn status(self) -> ManagedResultStatus {
        match self {
            Self::Created => ManagedResultStatus::Created,
            Self::MessageQueued => ManagedResultStatus::MessageQueued,
            Self::RelationshipChanged => ManagedResultStatus::RelationshipChanged,
            Self::Configured => ManagedResultStatus::Configured,
            Self::LifecycleChanged => ManagedResultStatus::LifecycleChanged,
            Self::MilestoneEmitted => ManagedResultStatus::MilestoneEmitted,
        }
    }
}
