//! Native ownership of conversation admission and durable paused-turn state.

use std::fmt;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll};

use futures_core::Stream;
use machine_god_core::{
    BoxFuture, ContentBlock, EngineError, EngineEvent, InferenceOptions, Prompt, Role, Session,
    SessionId, SessionRecord, SessionRevision, SessionStoreErrorKind, SessionTurnPreparation,
    StopReason, ToolContext, Turn, TurnEvent, TurnHandle,
};
use serde_json::{Value, json};

use crate::conversation_lifecycle::{LifecycleGate, LifecyclePermit, LifecyclePhase};
use crate::conversation_observations::{ObservationBatch, ObservationSession};
use crate::permission_context::{ContextRegistration, ContextSession};
use crate::workspace_context::{
    ConversationWorkspaceBinding, WorkspaceAdmission, WorkspaceContextRegistration,
};

use crate::{
    NATIVE_CONTEXT_PREFERENCES_KEY, NATIVE_CONVERSATION_HISTORY_KEY, NATIVE_MODEL_PREFERENCES_KEY,
    NATIVE_SESSION_METADATA_KEY, NativeContextError, NativeContextPreferences,
    NativeConversationHistory, NativeConversationHistoryError, NativeConversationObservations,
    NativeHistoryBackground, NativeHistoryFileEvidence, NativeHistoryFileSource,
    NativeHistoryFileStatus, NativeHistoryState, NativeModelPreferences,
    NativeModelPreferencesError, NativeModelSnapshot, NativeObservationError,
    NativeSessionLifecycle, NativeSessionLifecycleError, NativeSessionMetadata,
    NativeSessionMetadataError, NativeSessionMetadataMutationError, rename_native_session,
};

/// Native-only metadata entry. Its contents are not permission grants.
pub const NATIVE_CONVERSATION_CHECKPOINT_KEY: &str = "machine_god.conversation_checkpoint";

/// Fixed, redacted conversation failure. Persistence failures can follow publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeConversationError {
    Busy,
    NoCheckpoint,
    InvalidCheckpoint,
    InvalidHistory(NativeConversationHistoryError),
    Observation(NativeObservationError),
    PermissionContext(crate::NativePermissionContextError),
    WorkspaceContext(crate::NativeWorkspaceContextError),
    InvalidContext(NativeContextError),
    InvalidModelPreferences(NativeModelPreferencesError),
    InvalidMetadata(NativeSessionMetadataError),
    Lifecycle(NativeSessionLifecycleError),
    Conflict,
    HostClosed,
    Persistence,
    Engine,
}

impl fmt::Display for NativeConversationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => f.write_str("conversation is busy"),
            Self::NoCheckpoint => f.write_str("conversation has no paused turn"),
            Self::InvalidCheckpoint => f.write_str("conversation checkpoint is invalid"),
            Self::InvalidHistory(error) => error.fmt(f),
            Self::Observation(error) => error.fmt(f),
            Self::PermissionContext(error) => error.fmt(f),
            Self::WorkspaceContext(error) => error.fmt(f),
            Self::InvalidContext(error) => error.fmt(f),
            Self::InvalidModelPreferences(error) => error.fmt(f),
            Self::InvalidMetadata(error) => error.fmt(f),
            Self::Lifecycle(error) => error.fmt(f),
            Self::Conflict => f.write_str("conversation changed concurrently"),
            Self::HostClosed => f.write_str("conversation host is closed"),
            Self::Persistence => f.write_str("conversation persistence failed"),
            Self::Engine => f.write_str("conversation engine failed"),
        }
    }
}

impl std::error::Error for NativeConversationError {}

/// Observation of a durable interrupted turn, not authorization to replay tools.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativePausedTurn {
    pub turn_sequence: u64,
    pub has_uncertain_tool_results: bool,
}

#[derive(Clone, Copy)]
struct Checkpoint {
    turn_sequence: u64,
    first_user_message: usize,
    paused: bool,
}

impl Checkpoint {
    fn decode(record: &SessionRecord) -> Result<Option<Self>, NativeConversationError> {
        let Some(value) = record.metadata.get(NATIVE_CONVERSATION_CHECKPOINT_KEY) else {
            return Ok(None);
        };
        let object = value
            .as_object()
            .ok_or(NativeConversationError::InvalidCheckpoint)?;
        if object.len() != 4 || object.get("schema_version").and_then(Value::as_u64) != Some(1) {
            return Err(NativeConversationError::InvalidCheckpoint);
        }
        let turn_sequence = object
            .get("turn_sequence")
            .and_then(Value::as_u64)
            .filter(|sequence| *sequence > 0)
            .ok_or(NativeConversationError::InvalidCheckpoint)?;
        let first_user_message = object
            .get("first_user_message")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok())
            .ok_or(NativeConversationError::InvalidCheckpoint)?;
        let paused = match object.get("state").and_then(Value::as_str) {
            Some("running") => false,
            Some("paused") => true,
            _ => return Err(NativeConversationError::InvalidCheckpoint),
        };
        if turn_sequence.checked_add(1) != Some(record.next_turn_sequence)
            || record
                .messages
                .get(first_user_message)
                .map(|message| message.role)
                != Some(Role::User)
            || record.messages[first_user_message + 1..]
                .iter()
                .any(|message| message.role == Role::User)
        {
            return Err(NativeConversationError::InvalidCheckpoint);
        }
        Ok(Some(Self {
            turn_sequence,
            first_user_message,
            paused,
        }))
    }

    fn encode(self) -> Value {
        json!({
            "schema_version": 1,
            "turn_sequence": self.turn_sequence,
            "first_user_message": self.first_user_message,
            "state": if self.paused { "paused" } else { "running" },
        })
    }
}

/// Owns a live core session and serializes native admission/finalization.
///
/// The host retains provider, prompt, workspace and terminal resources. This
/// owner never reads a clock or environment, and never automatically retries a
/// provider or reexecutes a historical tool call. Other handles to the same core
/// session still obey core's revision and turn leases.
pub struct NativeConversation {
    lifecycle: std::sync::OnceLock<Arc<LifecycleGate>>,
    session: Session,
    active: Arc<AtomicBool>,
    observations: Option<Arc<ObservationSession>>,
    permissions: Option<Arc<crate::NativePermissionSession>>,
    permission_contexts: Option<Arc<ContextSession>>,
    workspace: Option<ConversationWorkspaceBinding>,
}

impl fmt::Debug for NativeConversation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeConversation")
            .field("busy", &self.is_busy())
            .finish_non_exhaustive()
    }
}

impl NativeConversation {
    pub(crate) fn bind_lifecycle(
        &self,
        gate: &Arc<LifecycleGate>,
    ) -> Result<(), NativeConversationError> {
        if self.is_busy() {
            return Err(NativeConversationError::Busy);
        }
        if let Some(current) = self.lifecycle.get() {
            return if Arc::ptr_eq(current, gate) {
                Ok(())
            } else {
                Err(NativeConversationError::Engine)
            };
        }
        if let Some(owner) = &self.permissions {
            owner
                .bind_lifecycle(gate)
                .map_err(|_| NativeConversationError::Engine)?;
        }
        self.lifecycle
            .set(Arc::clone(gate))
            .map_err(|_| NativeConversationError::Engine)
    }

    pub(crate) fn retire_lifecycle_routes(&self) {
        if self
            .lifecycle
            .get()
            .is_none_or(|gate| gate.phase() != LifecyclePhase::Retired)
        {
            return;
        }
        if let Some(owner) = &self.permissions {
            owner.retire();
        }
        if let Some(owner) = &self.permission_contexts {
            owner.retire();
        }
        if let Some(binding) = &self.workspace {
            binding.owner.retire();
        }
        if let Some(owner) = &self.observations {
            owner.retire();
        }
    }

    fn acquire_lifecycle(&self) -> Result<Option<LifecyclePermit>, NativeConversationError> {
        self.lifecycle
            .get()
            .map(|gate| gate.acquire().map_err(|_| NativeConversationError::Busy))
            .transpose()
    }

    fn acquire_admission(&self) -> Result<AdmissionLease, NativeConversationError> {
        if self
            .lifecycle
            .get()
            .is_some_and(|gate| gate.phase() == LifecyclePhase::Retired)
        {
            return Err(NativeConversationError::Busy);
        }
        AdmissionLease::acquire(&self.active)
    }

    pub(crate) fn acquire_workspace_control(
        &self,
    ) -> Result<AdmissionLease, NativeConversationError> {
        let lease = self.acquire_admission()?;
        if self.session.has_active_turn() {
            return Err(NativeConversationError::Busy);
        }
        Ok(lease)
    }

    pub(crate) fn capture_workspace_scope(
        &self,
    ) -> Result<Option<crate::NativeWorkspaceScopeSnapshot>, NativeConversationError> {
        self.workspace
            .as_ref()
            .map(|binding| {
                binding.authority.snapshot().map_err(|_| {
                    NativeConversationError::WorkspaceContext(
                        crate::NativeWorkspaceContextError::Unavailable,
                    )
                })
            })
            .transpose()
    }
    /// Adopts a validated live session without effects or inferred metadata.
    ///
    /// # Errors
    /// Rejects an active session or malformed native metadata or preferences.
    pub fn from_session(session: Session) -> Result<Self, NativeConversationError> {
        if session.has_active_turn() {
            return Err(NativeConversationError::Busy);
        }
        let record = session.record();
        NativeSessionMetadata::from_metadata(&record.metadata)
            .map_err(NativeConversationError::InvalidMetadata)?;
        validated_history(&record)?;
        validated_context_preferences(&record)?;
        NativeModelPreferences::from_metadata(&record.metadata)
            .map_err(NativeConversationError::InvalidModelPreferences)?;
        Ok(Self {
            lifecycle: std::sync::OnceLock::new(),
            session,
            active: Arc::new(AtomicBool::new(false)),
            observations: None,
            permissions: None,
            permission_contexts: None,
            workspace: None,
        })
    }

    /// Connects explicitly configured native producers to this exact incarnation.
    /// Construction registers bounded process-local state only; no I/O occurs.
    /// # Errors
    /// Rejects active work, duplicate registration, or exhausted routing capacity.
    pub fn with_observations(
        mut self,
        observations: &Arc<NativeConversationObservations>,
    ) -> Result<Self, NativeConversationError> {
        if self.is_busy() || self.observations.is_some() {
            return Err(NativeConversationError::Busy);
        }
        self.observations = Some(
            observations
                .register(self.id(), self.incarnation_id())
                .map_err(NativeConversationError::Observation)?,
        );
        Ok(self)
    }

    /// Connects exact native admissions to automatic permission review. Missing
    /// historical provenance remains unknown; construction performs no I/O.
    /// # Errors
    /// Rejects busy or duplicate ownership, invalid provenance, and capacity.
    pub fn with_permission_contexts(
        mut self,
        contexts: &Arc<crate::NativePermissionContexts>,
    ) -> Result<Self, NativeConversationError> {
        if self.is_busy() || self.permission_contexts.is_some() {
            return Err(NativeConversationError::Busy);
        }
        self.permission_contexts = Some(
            contexts
                .register(&self.session)
                .map_err(NativeConversationError::PermissionContext)?,
        );
        Ok(self)
    }

    /// Binds this exact incarnation to host-owned workspace authority without
    /// I/O or permission-handler requirements. The manager is read only when a
    /// new turn is admitted, never during tool or permission-context lookup.
    ///
    /// # Errors
    /// Rejects busy/duplicate binding and exhausted or duplicate session routing.
    pub fn with_workspace_contexts(
        mut self,
        authority: crate::NativeWorkspaceAuthority,
        contexts: &Arc<crate::NativeWorkspaceContexts>,
    ) -> Result<Self, NativeConversationError> {
        if self.is_busy() || self.workspace.is_some() {
            return Err(NativeConversationError::Busy);
        }
        self.workspace = Some(ConversationWorkspaceBinding {
            authority,
            owner: contexts
                .register(&self.session)
                .map_err(NativeConversationError::WorkspaceContext)?,
        });
        Ok(self)
    }

    /// Connects this exact session to a native policy handler. The engine must
    /// use the same controller and its tools the matching preparation adapters.
    /// # Errors
    /// Rejects busy/duplicate ownership, malformed rules and routing exhaustion.
    pub fn with_permission_controller(
        mut self,
        controller: &crate::NativePermissionController,
        policy: crate::NativePermissionPolicySnapshot,
    ) -> Result<Self, NativeConversationError> {
        if self.is_busy() || self.permissions.is_some() {
            return Err(NativeConversationError::Busy);
        }
        self.permissions = Some(
            controller
                .register(self.session.clone(), policy)
                .map_err(|_| NativeConversationError::Engine)?,
        );
        Ok(self)
    }

    /// Explicit process-local permission controls, independent of model state.
    #[must_use]
    pub fn permissions(&self) -> Option<&Arc<crate::NativePermissionSession>> {
        self.permissions.as_ref()
    }

    /// Creates an empty conversation with metadata in its initial durable record.
    /// The borrowed future performs no work before polling.
    #[must_use]
    pub fn create(
        lifecycle: &NativeSessionLifecycle,
        metadata: NativeSessionMetadata,
    ) -> BoxFuture<'_, Result<Self, NativeConversationError>> {
        Box::pin(async move {
            let session = lifecycle
                .create_generated_with_metadata(metadata)
                .await
                .map_err(NativeConversationError::Lifecycle)?;
            Self::from_session(session)
        })
    }

    /// Loads a conversation without restoring process-local approvals or undo.
    #[must_use]
    pub fn resume(
        lifecycle: &NativeSessionLifecycle,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Self, NativeConversationError>> {
        Box::pin(async move {
            let session = lifecycle
                .resume(id)
                .await
                .map_err(NativeConversationError::Lifecycle)?;
            Self::from_session(session)
        })
    }

    #[must_use]
    pub fn id(&self) -> SessionId {
        self.session.id()
    }

    #[must_use]
    pub fn incarnation_id(&self) -> machine_god_core::SessionIncarnationId {
        self.session.incarnation_id()
    }

    /// Returns canonical history, never a compacted provider projection.
    #[must_use]
    pub fn record(&self) -> SessionRecord {
        self.session.record()
    }

    /// Pins canonical memory without cloning transcript or metadata payloads.
    /// This is not a store receipt or uncertain-write reconciliation.
    #[must_use]
    pub fn record_snapshot(&self) -> Arc<SessionRecord> {
        self.session.record_snapshot()
    }

    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.active.load(Ordering::Acquire) || self.session.has_active_turn()
    }

    /// Persists a title without allowing it to overlap turn finalization.
    /// The borrowed future is inert until polled and preserves checkpoints.
    #[must_use]
    pub fn rename<'a>(
        &'a self,
        title: &'a str,
        now_ms: i64,
    ) -> BoxFuture<'a, Result<SessionRevision, NativeConversationError>> {
        Box::pin(async move {
            let _lease = self.acquire_admission()?;
            rename_native_session(&self.session, title, now_ms)
                .await
                .map_err(|error| match error {
                    NativeSessionMetadataMutationError::InvalidMetadata(error) => {
                        NativeConversationError::InvalidMetadata(error)
                    }
                    NativeSessionMetadataMutationError::Busy => NativeConversationError::Busy,
                    NativeSessionMetadataMutationError::Conflict => {
                        NativeConversationError::Conflict
                    }
                    NativeSessionMetadataMutationError::HostClosed => {
                        NativeConversationError::HostClosed
                    }
                    NativeSessionMetadataMutationError::Persistence => {
                        NativeConversationError::Persistence
                    }
                    _ => NativeConversationError::Engine,
                })
        })
    }

    /// Observes saved model preferences while idle. Missing historical settings
    /// remain `None`; host defaults and process overrides are not inferred.
    ///
    /// # Errors
    /// Returns `Busy` or rejects malformed saved model preferences.
    pub fn model_preferences(
        &self,
    ) -> Result<Option<NativeModelPreferences>, NativeConversationError> {
        let _lease = self.acquire_admission()?;
        if self.session.has_active_turn() {
            return Err(NativeConversationError::Busy);
        }
        NativeModelPreferences::from_metadata(&self.session.record().metadata)
            .map_err(NativeConversationError::InvalidModelPreferences)
    }

    /// Persists requested model settings and an explicit timestamp while idle.
    /// The borrowed future is inert before polling and preserves history,
    /// checkpoints, context selection and unrelated metadata. This writes only
    /// the session; runtime queues and user defaults are separate host targets.
    #[must_use]
    pub fn set_model_preferences(
        &self,
        preferences: NativeModelPreferences,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<SessionRevision, NativeConversationError>> {
        self.set_model_preferences_with_access(preferences, now_ms, None)
    }

    pub(crate) fn set_model_preferences_with_access(
        &self,
        preferences: NativeModelPreferences,
        now_ms: i64,
        access: Option<Arc<dyn machine_god_core::SessionStoreAccess>>,
    ) -> BoxFuture<'_, Result<SessionRevision, NativeConversationError>> {
        Box::pin(async move {
            let _lease = self.acquire_admission()?;
            if self.session.has_active_turn() {
                return Err(NativeConversationError::Busy);
            }
            let mut record = self.session.record();
            NativeModelPreferences::from_metadata(&record.metadata)
                .map_err(NativeConversationError::InvalidModelPreferences)?;
            let mut metadata = NativeSessionMetadata::from_metadata(&record.metadata)
                .map_err(NativeConversationError::InvalidMetadata)?;
            metadata
                .touch(now_ms)
                .map_err(NativeConversationError::InvalidMetadata)?;
            record
                .metadata
                .insert(NATIVE_SESSION_METADATA_KEY.to_owned(), metadata.to_value());
            record.metadata.insert(
                NATIVE_MODEL_PREFERENCES_KEY.to_owned(),
                preferences.to_value(),
            );
            match access {
                Some(access) => self.session.update_metadata_with_access(
                    record.revision,
                    record.metadata,
                    access,
                ),
                None => self
                    .session
                    .update_metadata(record.revision, record.metadata),
            }
            .await
            .map_err(map_engine_error)
        })
    }

    /// Observes validated context preferences while native admission is idle.
    /// Missing preferences retain full history and do not infer host defaults.
    ///
    /// # Errors
    /// Returns `Busy` or rejects malformed preferences and invalid selections.
    pub fn context_preferences(&self) -> Result<NativeContextPreferences, NativeConversationError> {
        let _lease = self.acquire_admission()?;
        if self.session.has_active_turn() {
            return Err(NativeConversationError::Busy);
        }
        validated_context_preferences(&self.session.record())
    }

    /// Persists a manual selection retaining the entire final logical group,
    /// including every assistant/tool round and no-input continuation in it.
    /// Canonical messages and archives are never removed or rewritten.
    /// The borrowed future is inert until polled; `false` is a no-write no-op.
    #[must_use]
    pub fn compact(&self, now_ms: i64) -> BoxFuture<'_, Result<bool, NativeConversationError>> {
        Box::pin(async move {
            let _lease = self.acquire_admission()?;
            if self.session.has_active_turn() {
                return Err(NativeConversationError::Busy);
            }
            let record = self.session.record();
            let mut preferences = NativeContextPreferences::from_metadata(&record.metadata)
                .map_err(NativeConversationError::InvalidContext)?;
            if !preferences
                .force_compact(&record)
                .map_err(NativeConversationError::InvalidContext)?
            {
                return Ok(false);
            }
            self.persist_context_preferences(record, &preferences, now_ms)
                .await?;
            Ok(true)
        })
    }

    /// Persists the automatic history-group limit, preserving the manual cut.
    /// Zero disables automatic selection, not a previously saved manual cut.
    /// This borrowed, inert future holds admission through the metadata save.
    #[must_use]
    pub fn set_max_history_turns(
        &self,
        maximum: usize,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<SessionRevision, NativeConversationError>> {
        Box::pin(async move {
            let _lease = self.acquire_admission()?;
            if self.session.has_active_turn() {
                return Err(NativeConversationError::Busy);
            }
            let record = self.session.record();
            let mut preferences = NativeContextPreferences::from_metadata(&record.metadata)
                .map_err(NativeConversationError::InvalidContext)?;
            preferences
                .set_max_history_turns(maximum)
                .map_err(NativeConversationError::InvalidContext)?;
            validate_selected_context(&preferences, &record)?;
            self.persist_context_preferences(record, &preferences, now_ms)
                .await
        })
    }

    async fn persist_context_preferences(
        &self,
        mut record: SessionRecord,
        preferences: &NativeContextPreferences,
        now_ms: i64,
    ) -> Result<SessionRevision, NativeConversationError> {
        let mut metadata = NativeSessionMetadata::from_metadata(&record.metadata)
            .map_err(NativeConversationError::InvalidMetadata)?;
        metadata
            .touch(now_ms)
            .map_err(NativeConversationError::InvalidMetadata)?;
        record
            .metadata
            .insert(NATIVE_SESSION_METADATA_KEY.to_owned(), metadata.to_value());
        record.metadata.insert(
            NATIVE_CONTEXT_PREFERENCES_KEY.to_owned(),
            preferences.to_value(),
        );
        self.session
            .update_metadata(record.revision, record.metadata)
            .await
            .map_err(map_engine_error)
    }

    /// Observes paused state only while no native turn/finalizer is active.
    /// A running checkpoint left after dropped work is interrupted, not replayed.
    ///
    /// # Errors
    /// Returns `Busy` or rejects invalid native checkpoint state.
    pub fn paused_turn(&self) -> Result<Option<NativePausedTurn>, NativeConversationError> {
        let _lease = self.acquire_admission()?;
        if self.session.has_active_turn() {
            return Err(NativeConversationError::Busy);
        }
        let record = self.session.record();
        Ok(
            Checkpoint::decode(&record)?.map(|checkpoint| NativePausedTurn {
                turn_sequence: checkpoint.turn_sequence,
                has_uncertain_tool_results: record.messages[checkpoint.first_user_message..]
                    .iter()
                    .flat_map(|message| &message.content)
                    .any(|block| {
                        matches!(block, ContentBlock::ToolResult { output, .. }
                        if output.is_error && output.content.get("code").and_then(Value::as_str)
                            == Some("tool_result_unknown"))
                    }),
            }),
        )
    }

    /// Observes explicitly recorded native facts without reconstructing missing history.
    /// # Errors
    /// Rejects active work or invalid typed facts/checkpoint relationships.
    pub fn history(&self) -> Result<NativeConversationHistory, NativeConversationError> {
        let _lease = self.acquire_admission()?;
        if self.session.has_active_turn() {
            return Err(NativeConversationError::Busy);
        }
        validated_history(&self.session.record())
    }

    /// Persists pending tool observations after dropped or failed finalization.
    /// No provider or tool is started. Pending facts are retired only after a
    /// confirmed save; an unpolled future does nothing.
    #[must_use]
    pub fn flush_history_observations(
        &self,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<Option<SessionRevision>, NativeConversationError>> {
        Box::pin(async move {
            let _lease = self.acquire_admission()?;
            if self.session.has_active_turn() {
                return Err(NativeConversationError::Busy);
            }
            let Some(observations) = &self.observations else {
                return Ok(None);
            };
            let batch = observations.snapshot();
            if batch.entries().is_empty() {
                return Ok(None);
            }
            let record = self.session.record();
            let mut history = validated_history(&record)?;
            merge_observations(&mut history, &batch, &record)?;
            let revision = self.persist_history(record, history, now_ms).await?;
            observations.acknowledge(&batch);
            Ok(Some(revision))
        })
    }

    /// Records an explicitly observed file fact against an exact historical attempt.
    /// The borrowed future is inert before polling and grants no file authority.
    #[must_use]
    pub fn record_history_file(
        &self,
        first_user_message: usize,
        turn_sequence: u64,
        evidence: NativeHistoryFileEvidence,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<SessionRevision, NativeConversationError>> {
        self.update_history(now_ms, move |history| {
            history.upsert_file(first_user_message, turn_sequence, evidence)
        })
    }

    /// Records or clears explicit background observations, not process ownership.
    /// A changed attempt identity is rejected rather than attached to a later turn.
    #[must_use]
    pub fn set_history_background(
        &self,
        first_user_message: usize,
        turn_sequence: u64,
        background: Option<NativeHistoryBackground>,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<SessionRevision, NativeConversationError>> {
        self.update_history(now_ms, move |history| {
            history.set_background(first_user_message, turn_sequence, background)
        })
    }

    fn update_history<'a>(
        &'a self,
        now_ms: i64,
        update: impl FnOnce(
            &mut NativeConversationHistory,
        ) -> Result<(), NativeConversationHistoryError>
        + Send
        + 'a,
    ) -> BoxFuture<'a, Result<SessionRevision, NativeConversationError>> {
        Box::pin(async move {
            let _lease = self.acquire_admission()?;
            if self.session.has_active_turn() {
                return Err(NativeConversationError::Busy);
            }
            let record = self.session.record();
            let mut history = validated_history(&record)?;
            let batch = self.observations.as_ref().map(ObservationSession::snapshot);
            if let Some(batch) = &batch {
                merge_observations(&mut history, batch, &record)?;
            }
            update(&mut history).map_err(NativeConversationError::InvalidHistory)?;
            let revision = self.persist_history(record, history, now_ms).await?;
            if let (Some(owner), Some(batch)) = (&self.observations, &batch) {
                owner.acknowledge(batch);
            }
            Ok(revision)
        })
    }

    async fn persist_history(
        &self,
        mut record: SessionRecord,
        history: NativeConversationHistory,
        now_ms: i64,
    ) -> Result<SessionRevision, NativeConversationError> {
        let mut metadata = NativeSessionMetadata::from_metadata(&record.metadata)
            .map_err(NativeConversationError::InvalidMetadata)?;
        metadata
            .touch(now_ms)
            .map_err(NativeConversationError::InvalidMetadata)?;
        record
            .metadata
            .insert(NATIVE_SESSION_METADATA_KEY.to_owned(), metadata.to_value());
        record.metadata.insert(
            NATIVE_CONVERSATION_HISTORY_KEY.to_owned(),
            history.to_value(),
        );
        // New observations may name a canonical call. Validate their exact
        // message/block references before publication, not only on resume.
        validated_history(&record)?;
        self.session
            .update_metadata(record.revision, record.metadata)
            .await
            .map_err(map_engine_error)
    }

    /// Reserves user input, its fresh turn identity and native checkpoint in one
    /// exact-revision publication. Provider work begins only when the returned
    /// stream is polled. `now_ms` is an explicit host clock observation.
    #[must_use]
    pub fn prompt(
        &self,
        prompt: Prompt,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<NativeConversationTurn, NativeConversationError>> {
        let input = PendingInput(Some(ConversationInput::Prompt(prompt)));
        Box::pin(async move { self.start(input, None, now_ms).await })
    }

    /// Atomically publishes the job's requested model preferences with its user
    /// input and checkpoint, and pins the snapshot's effective controls for all
    /// provider rounds. Other inference options are preserved. The future is
    /// inert before polling, including when dropped with untrusted metadata.
    #[must_use]
    pub fn prompt_with_model(
        &self,
        prompt: Prompt,
        model: NativeModelSnapshot,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<NativeConversationTurn, NativeConversationError>> {
        let input = PendingInput(Some(ConversationInput::Prompt(prompt)));
        Box::pin(async move { self.start(input, Some(model), now_ms).await })
    }

    /// Explicitly continues a durable paused turn with a fresh core attempt
    /// budget. Preserves user input and historical results, including unknown
    /// receipts; only newly requested tools can execute with fresh authorization.
    #[must_use]
    pub fn continue_turn(
        &self,
        options: InferenceOptions,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<NativeConversationTurn, NativeConversationError>> {
        let input = PendingInput(Some(ConversationInput::Continue(options)));
        Box::pin(async move { self.start(input, None, now_ms).await })
    }

    /// Continues with the newly admitted job's explicit snapshot, not inferred
    /// historical settings. Saves requested preferences with the new checkpoint
    /// and uses effective controls without appending another user message.
    #[must_use]
    pub fn continue_turn_with_model(
        &self,
        options: InferenceOptions,
        model: NativeModelSnapshot,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<NativeConversationTurn, NativeConversationError>> {
        let input = PendingInput(Some(ConversationInput::Continue(options)));
        Box::pin(async move { self.start(input, Some(model), now_ms).await })
    }

    async fn start(
        &self,
        input: PendingInput,
        model: Option<NativeModelSnapshot>,
        now_ms: i64,
    ) -> Result<NativeConversationTurn, NativeConversationError> {
        let policy = self
            .permissions
            .as_ref()
            .map(|owner| owner.snapshot())
            .transpose()
            .map_err(|_| NativeConversationError::Engine)?;
        self.start_with_policy(input, model, policy, now_ms).await
    }

    // Keep reservation, exact-turn registrations, and rollback-owned leases in
    // one linear admission scope; no provider work is polled between them.
    pub(crate) async fn start_with_policy(
        &self,
        input: PendingInput,
        model: Option<NativeModelSnapshot>,
        policy: Option<crate::NativePermissionPolicySnapshot>,
        now_ms: i64,
    ) -> Result<NativeConversationTurn, NativeConversationError> {
        let permit = self.acquire_lifecycle()?;
        let mut turn = self
            .start_with_policy_inner(
                input,
                model,
                policy,
                now_ms,
                permit.as_ref(),
                WorkspaceAdmission::Current,
            )
            .await?;
        turn.lease.as_mut().expect("admitted turn retains lease").1 = permit;
        Ok(turn)
    }

    pub(crate) async fn start_with_policy_admitted(
        &self,
        input: PendingInput,
        model: Option<NativeModelSnapshot>,
        policy: Option<crate::NativePermissionPolicySnapshot>,
        now_ms: i64,
        permit: &LifecyclePermit,
        workspace: Option<crate::NativeWorkspaceScopeSnapshot>,
    ) -> Result<NativeConversationTurn, NativeConversationError> {
        if self
            .lifecycle
            .get()
            .is_none_or(|gate| !permit.belongs_to(gate))
        {
            return Err(NativeConversationError::Engine);
        }
        self.start_with_policy_inner(
            input,
            model,
            policy,
            now_ms,
            Some(permit),
            WorkspaceAdmission::Taken(workspace),
        )
        .await
    }

    #[allow(clippy::too_many_lines)]
    async fn start_with_policy_inner(
        &self,
        mut input: PendingInput,
        model: Option<NativeModelSnapshot>,
        policy: Option<crate::NativePermissionPolicySnapshot>,
        now_ms: i64,
        permit: Option<&LifecyclePermit>,
        workspace: WorkspaceAdmission,
    ) -> Result<NativeConversationTurn, NativeConversationError> {
        let lease = self.acquire_admission()?;
        if self.session.has_active_turn() {
            return Err(NativeConversationError::Busy);
        }
        let workspace = match workspace {
            WorkspaceAdmission::Current => self.capture_workspace_scope()?,
            WorkspaceAdmission::Taken(workspace) => workspace,
        };
        if self.workspace.is_some() != workspace.is_some() {
            return Err(NativeConversationError::WorkspaceContext(
                crate::NativeWorkspaceContextError::Unavailable,
            ));
        }
        let mut record = self.session.record();
        let previous = Checkpoint::decode(&record)?;
        let mut history = validated_history(&record)?;
        let batch = self.observations.as_ref().map(ObservationSession::snapshot);
        if let Some(batch) = &batch {
            // Reconcile the old attempt before continuation advances its identity
            // and before constructing the provider-only context projection.
            merge_observations(&mut history, batch, &record)?;
            record.metadata.insert(
                NATIVE_CONVERSATION_HISTORY_KEY.to_owned(),
                history.to_value(),
            );
        }
        let source_cursor = (record.messages.len(), 0);
        NativeModelPreferences::from_metadata(&record.metadata)
            .map_err(NativeConversationError::InvalidModelPreferences)?;
        let context = provider_context(&record)?;
        let checkpoint = begin_history(
            &record,
            &mut history,
            previous,
            matches!(&input.0, Some(ConversationInput::Prompt(_))),
        )?;
        let mut metadata = NativeSessionMetadata::from_metadata(&record.metadata)
            .map_err(NativeConversationError::InvalidMetadata)?;
        metadata
            .touch(now_ms)
            .map_err(NativeConversationError::InvalidMetadata)?;
        record
            .metadata
            .insert(NATIVE_SESSION_METADATA_KEY.to_owned(), metadata.to_value());
        record.metadata.insert(
            NATIVE_CONVERSATION_CHECKPOINT_KEY.to_owned(),
            checkpoint.encode(),
        );
        record.metadata.insert(
            NATIVE_CONVERSATION_HISTORY_KEY.to_owned(),
            history.to_value(),
        );
        let root_context = if self.permission_contexts.is_some() {
            let prompt = match input.0.as_ref().expect("input is consumed once") {
                ConversationInput::Prompt(prompt) => Some(prompt.text.as_str()),
                ConversationInput::Continue(_) => None,
            };
            crate::permission_context::prepare_provenance(
                &mut record,
                prompt,
                checkpoint.first_user_message,
            )
            .map_err(NativeConversationError::PermissionContext)?
        } else {
            None
        };
        if let Some(model) = &model {
            apply_model_snapshot(&mut input, &mut record, model);
        }
        let source_model = match input.0.as_ref().expect("input is consumed once") {
            ConversationInput::Prompt(prompt) => prompt.options.model.clone(),
            ConversationInput::Continue(options) => options.model.clone(),
        };
        let preparation = SessionTurnPreparation {
            expected_revision: record.revision,
            metadata: Some(record.metadata),
            context,
        };
        let turn = match input.0.take().expect("input is consumed once") {
            ConversationInput::Prompt(prompt) => {
                self.session.prompt_prepared(prompt, preparation).await
            }
            ConversationInput::Continue(options) => {
                self.session
                    .continue_turn_prepared(options, preparation)
                    .await
            }
        }
        .map_err(map_engine_error)?;
        let workspace_context = self
            .workspace
            .as_ref()
            .zip(workspace)
            .map(|(binding, scope)| {
                binding
                    .owner
                    .begin(&turn, scope)
                    .map_err(NativeConversationError::WorkspaceContext)
            })
            .transpose()?;
        let permission_context = self
            .permission_contexts
            .as_ref()
            .map(|owner| {
                owner
                    .begin(&turn, root_context, model, source_model, policy.clone())
                    .map_err(NativeConversationError::PermissionContext)
            })
            .transpose()?;
        let permission_turn = self.bind_permission_turn(&turn, policy, permit)?;
        if let Some(owner) = &self.permissions {
            if let Some(permit) = permit {
                owner.reconcile_rules_admitted(permit).await
            } else {
                owner.reconcile_rules().await
            }
            .map_err(|_| NativeConversationError::Engine)?;
        }
        if let Some(owner) = &self.observations {
            owner
                .begin_attempt(
                    turn.handle().id().clone(),
                    checkpoint.first_user_message,
                    checkpoint.turn_sequence,
                )
                .map_err(NativeConversationError::Observation)?;
            if let Some(batch) = &batch {
                owner.acknowledge(batch);
            }
        }
        Ok(NativeConversationTurn {
            handle: turn.handle(),
            core: Some(turn),
            session: self.session.clone(),
            checkpoint,
            lease: Some(lease),
            finalization: None,
            terminal: None,
            observations: self.observations.clone(),
            source_cursor,
            observation_batch: None,
            permission_turn,
            permission_context,
            workspace_context,
            done: false,
        })
    }

    fn bind_permission_turn(
        &self,
        turn: &Turn,
        policy: Option<crate::NativePermissionPolicySnapshot>,
        permit: Option<&LifecyclePermit>,
    ) -> Result<Option<crate::NativePermissionTurn>, NativeConversationError> {
        let registration = match (&self.permissions, policy) {
            (Some(owner), Some(policy)) => Some(
                if let Some(permit) = permit {
                    owner.begin_turn_admitted(turn, policy, permit)
                } else {
                    owner.begin_turn(turn, policy)
                }
                .map_err(|_| NativeConversationError::Engine)?,
            ),
            (None, None) => None,
            _ => return Err(NativeConversationError::Engine),
        };
        Ok(registration)
    }
}

fn apply_model_snapshot(
    input: &mut PendingInput,
    record: &mut SessionRecord,
    model: &NativeModelSnapshot,
) {
    record.metadata.insert(
        NATIVE_MODEL_PREFERENCES_KEY.to_owned(),
        model.preferences().to_value(),
    );
    let options = match input.0.as_mut().expect("input is consumed once") {
        ConversationInput::Prompt(prompt) => &mut prompt.options,
        ConversationInput::Continue(options) => options,
    };
    model.apply_to(options);
}

fn provider_context(
    record: &SessionRecord,
) -> Result<Option<machine_god_core::SessionContextProjection>, NativeConversationError> {
    let preferences = NativeContextPreferences::from_metadata(&record.metadata)
        .map_err(NativeConversationError::InvalidContext)?;
    // Disabled context retains the original full-history path and its bounds.
    if preferences == NativeContextPreferences::default() {
        Ok(None)
    } else {
        preferences
            .projection(record)
            .map_err(NativeConversationError::InvalidContext)
    }
}

fn begin_history(
    record: &SessionRecord,
    history: &mut NativeConversationHistory,
    previous: Option<Checkpoint>,
    new_prompt: bool,
) -> Result<Checkpoint, NativeConversationError> {
    let checkpoint = Checkpoint {
        turn_sequence: record.next_turn_sequence,
        first_user_message: if new_prompt {
            record.messages.len()
        } else {
            previous
                .ok_or(NativeConversationError::NoCheckpoint)?
                .first_user_message
        },
        paused: false,
    };
    // Old native checkpoints establish unfinished work without inventing its cause.
    if let Some(previous) = previous
        && history.group(previous.first_user_message).is_none()
    {
        history
            .begin(previous.first_user_message, previous.turn_sequence)
            .map_err(NativeConversationError::InvalidHistory)?;
    }
    history
        .begin(checkpoint.first_user_message, checkpoint.turn_sequence)
        .map_err(NativeConversationError::InvalidHistory)?;
    Ok(checkpoint)
}

fn merge_observations(
    history: &mut NativeConversationHistory,
    batch: &ObservationBatch,
    record: &SessionRecord,
) -> Result<(), NativeConversationError> {
    for entry in batch.entries() {
        let mut file = entry.file().clone();
        if file.model_view_covers_full_file() && !has_saved_successful_output(record, file.source())
        {
            let source = file
                .source()
                .cloned()
                .ok_or(NativeConversationError::Conflict)?;
            let destination = file.new_path().map(str::to_owned);
            let status = file.status();
            file = file
                .with_execution(source, status, destination.as_deref(), false)
                .map_err(NativeConversationError::InvalidHistory)?;
        }
        let group = history
            .group(entry.first_user_message())
            .ok_or(NativeConversationError::Conflict)?;
        // A store can publish and then return an error. Recognize previously
        // published evidence without downgrading settled results or staleness.
        let published = group.files().iter().any(|saved| {
            saved.source().is_some()
                && saved.source() == file.source()
                && saved.path() == file.path()
                && saved.action() == file.action()
                && saved.new_path() == file.new_path()
                && (saved.status() == file.status()
                    || file.status() == NativeHistoryFileStatus::Unknown)
                && (saved.stale() || !file.stale())
                && (saved.model_view_covers_full_file() || !file.model_view_covers_full_file())
        });
        if published {
            continue;
        }
        history
            .upsert_file(entry.first_user_message(), entry.turn_sequence(), file)
            .map_err(NativeConversationError::InvalidHistory)?;
    }
    Ok(())
}

// Core appends one result placeholder per call directly after the assistant
// message, then replaces that exact slot after validating and saving output.
// A completed native read alone does not prove the provider can see its bytes.
fn has_saved_successful_output(
    record: &SessionRecord,
    source: Option<&NativeHistoryFileSource>,
) -> bool {
    let Some(source) = source else {
        return false;
    };
    let Some(message) = record.messages.get(source.assistant_message()) else {
        return false;
    };
    if message.role != Role::Assistant {
        return false;
    }
    let Some(ContentBlock::ToolCall { call }) = message.content.get(source.content_block()) else {
        return false;
    };
    if &call.id != source.call_id() || &call.name != source.tool_name() {
        return false;
    }
    let ordinal = message.content[..source.content_block()]
        .iter()
        .filter(|block| matches!(block, ContentBlock::ToolCall { .. }))
        .count();
    let Some(index) = source
        .assistant_message()
        .checked_add(1)
        .and_then(|index| index.checked_add(ordinal))
    else {
        return false;
    };
    let Some(message) = record.messages.get(index) else {
        return false;
    };
    message.role == Role::Tool
        && matches!(message.content.as_slice(),
        [ContentBlock::ToolResult { call_id, output }]
            if call_id == source.call_id() && !output.is_error)
}

pub(crate) fn validated_history(
    record: &SessionRecord,
) -> Result<NativeConversationHistory, NativeConversationError> {
    let history = NativeConversationHistory::from_record(record)
        .map_err(NativeConversationError::InvalidHistory)?;
    let checkpoint = Checkpoint::decode(record)?;
    for group in history.groups() {
        if group.state() == NativeHistoryState::Running
            && checkpoint.is_none_or(|checkpoint| {
                checkpoint.paused
                    || checkpoint.first_user_message != group.first_user_message()
                    || checkpoint.turn_sequence != group.turn_sequence()
            })
        {
            return Err(NativeConversationError::InvalidCheckpoint);
        }
    }
    if let Some(checkpoint) = checkpoint
        && let Some(group) = history.group(checkpoint.first_user_message)
        && (group.turn_sequence() != checkpoint.turn_sequence
            || group.state() == NativeHistoryState::Completed
            || (group.state() == NativeHistoryState::Running) == checkpoint.paused)
    {
        return Err(NativeConversationError::InvalidCheckpoint);
    }
    Ok(history)
}

fn validated_context_preferences(
    record: &SessionRecord,
) -> Result<NativeContextPreferences, NativeConversationError> {
    let preferences = NativeContextPreferences::from_metadata(&record.metadata)
        .map_err(NativeConversationError::InvalidContext)?;
    validate_selected_context(&preferences, record)?;
    Ok(preferences)
}

fn validate_selected_context(
    preferences: &NativeContextPreferences,
    record: &SessionRecord,
) -> Result<(), NativeConversationError> {
    if preferences != &NativeContextPreferences::default() {
        preferences
            .validate_selection(record)
            .map_err(NativeConversationError::InvalidContext)?;
    }
    Ok(())
}

pub(crate) enum ConversationInput {
    Prompt(Prompt),
    Continue(InferenceOptions),
}

// Own untrusted inference JSON safely even when a future is never polled or
// admission rejects before core can install its own iterative-drop guard.
pub(crate) struct PendingInput(pub(crate) Option<ConversationInput>);
impl Drop for PendingInput {
    fn drop(&mut self) {
        let metadata = match &mut self.0 {
            Some(ConversationInput::Prompt(prompt)) => &mut prompt.options.metadata,
            Some(ConversationInput::Continue(options)) => &mut options.metadata,
            None => return,
        };
        let mut pending: Vec<_> = std::mem::take(metadata).into_values().collect();
        while let Some(value) = pending.pop() {
            match value {
                Value::Array(values) => pending.extend(values),
                Value::Object(values) => pending.extend(values.into_values()),
                _ => {}
            }
        }
    }
}

pub(crate) struct AdmissionLease(Arc<AtomicBool>, Option<LifecyclePermit>);
impl AdmissionLease {
    fn acquire(active: &Arc<AtomicBool>) -> Result<Self, NativeConversationError> {
        active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| NativeConversationError::Busy)?;
        Ok(Self(Arc::clone(active), None))
    }
}
impl Drop for AdmissionLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Owned stream that settles native checkpoint persistence before forwarding a
/// terminal outcome. Drop cancels owned core work and leaves durable recovery
/// evidence; it never starts an asynchronous cleanup task.
pub struct NativeConversationTurn {
    core: Option<Turn>,
    handle: TurnHandle,
    session: Session,
    checkpoint: Checkpoint,
    lease: Option<AdmissionLease>,
    finalization: Option<BoxFuture<'static, Result<SessionRevision, EngineError>>>,
    terminal: Option<Result<EngineEvent, NativeConversationError>>,
    observations: Option<Arc<ObservationSession>>,
    source_cursor: (usize, usize),
    observation_batch: Option<ObservationBatch>,
    permission_turn: Option<crate::NativePermissionTurn>,
    permission_context: Option<ContextRegistration>,
    workspace_context: Option<WorkspaceContextRegistration>,
    done: bool,
}

impl fmt::Debug for NativeConversationTurn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeConversationTurn")
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl NativeConversationTurn {
    #[must_use]
    pub fn handle(&self) -> TurnHandle {
        self.handle.clone()
    }

    fn finish(&mut self) {
        self.workspace_context.take();
        self.permission_context.take();
        self.permission_turn.take();
        self.core.take();
        if let Some(owner) = &self.observations {
            owner.finish_attempt(self.handle.id());
        }
        self.finalization.take();
        self.observation_batch.take();
        self.lease.take();
        self.done = true;
    }

    fn stage_finalization(
        &mut self,
        terminal: Result<EngineEvent, NativeConversationError>,
    ) -> Result<(), NativeConversationError> {
        self.workspace_context.take();
        self.permission_context.take();
        self.core.take();
        if let Some(owner) = &self.observations {
            owner.finish_attempt(self.handle.id());
        }
        let mut record = self.session.record();
        let checkpoint = Checkpoint::decode(&record)?.ok_or(NativeConversationError::Conflict)?;
        if checkpoint.turn_sequence != self.checkpoint.turn_sequence
            || checkpoint.first_user_message != self.checkpoint.first_user_message
        {
            return Err(NativeConversationError::Conflict);
        }
        let mut history = validated_history(&record)?;
        let batch = self.observations.as_ref().map(ObservationSession::snapshot);
        if let Some(batch) = &batch {
            merge_observations(&mut history, batch, &record)?;
        }
        let state = match &terminal {
            Ok(EngineEvent {
                payload:
                    TurnEvent::Completed {
                        reason: StopReason::Cancelled,
                        ..
                    },
                ..
            }) => NativeHistoryState::Cancelled,
            Ok(EngineEvent {
                payload: TurnEvent::Completed { .. },
                ..
            }) => NativeHistoryState::Completed,
            _ => NativeHistoryState::Failed,
        };
        history
            .finish(
                checkpoint.first_user_message,
                checkpoint.turn_sequence,
                state,
            )
            .map_err(NativeConversationError::InvalidHistory)?;
        record.metadata.insert(
            NATIVE_CONVERSATION_HISTORY_KEY.to_owned(),
            history.to_value(),
        );
        if matches!(&terminal, Ok(EngineEvent {
            payload: TurnEvent::Completed { reason, .. }, ..
        }) if *reason != StopReason::Cancelled)
        {
            record.metadata.remove(NATIVE_CONVERSATION_CHECKPOINT_KEY);
        } else {
            record.metadata.insert(
                NATIVE_CONVERSATION_CHECKPOINT_KEY.to_owned(),
                Checkpoint {
                    paused: true,
                    ..checkpoint
                }
                .encode(),
            );
        }
        validated_history(&record)?;
        self.finalization = Some(
            self.session
                .update_metadata(record.revision, record.metadata),
        );
        self.observation_batch = batch;
        self.terminal = Some(terminal);
        Ok(())
    }

    fn bind_observation(&mut self, event: &EngineEvent) -> Result<(), NativeConversationError> {
        let Some(owner) = &self.observations else {
            return Ok(());
        };
        let TurnEvent::ToolStarted { call } = &event.payload else {
            return Ok(());
        };
        let (message, block) = self
            .session
            .find_tool_call(self.source_cursor, &call.name, &call.id)
            .ok_or(NativeConversationError::Conflict)?;
        let source =
            NativeHistoryFileSource::new(message, block, call.id.clone(), call.name.clone())
                .map_err(NativeConversationError::InvalidHistory)?;
        owner
            .bind_call(
                &ToolContext {
                    session_id: event.session_id.clone(),
                    session_incarnation_id: event.session_incarnation_id.clone(),
                    turn_id: event.turn_id.clone(),
                    call_id: call.id.clone(),
                },
                source,
            )
            .map_err(NativeConversationError::Observation)?;
        self.source_cursor = (message, block + 1);
        Ok(())
    }
}

impl Stream for NativeConversationTurn {
    type Item = Result<EngineEvent, NativeConversationError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if self.done {
                return Poll::Ready(None);
            }
            if let Some(finalization) = &mut self.finalization {
                match finalization.as_mut().poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(result) => {
                        let terminal = self.terminal.take().expect("finalizer retains terminal");
                        if result.is_ok()
                            && let (Some(owner), Some(batch)) =
                                (&self.observations, &self.observation_batch)
                        {
                            owner.acknowledge(batch);
                        }
                        self.finish();
                        return Poll::Ready(Some(match result {
                            Ok(_) => terminal,
                            Err(error) => Err(map_engine_error(error)),
                        }));
                    }
                }
            }
            let event = match self.core.as_mut() {
                Some(core) => match Pin::new(core).poll_next(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(event) => event,
                },
                None => None,
            };
            let terminal = match event {
                Some(Ok(event))
                    if !matches!(
                        event.payload,
                        TurnEvent::Completed { .. } | TurnEvent::Failed { .. }
                    ) =>
                {
                    match self.bind_observation(&event) {
                        Ok(()) => return Poll::Ready(Some(Ok(event))),
                        Err(error) => Err(error),
                    }
                }
                Some(event) => event.map_err(map_engine_error),
                None => Err(NativeConversationError::Engine),
            };
            if let Err(error) = self.stage_finalization(terminal) {
                self.finish();
                return Poll::Ready(Some(Err(error)));
            }
        }
    }
}

impl Drop for NativeConversationTurn {
    fn drop(&mut self) {
        self.finish();
    }
}

fn map_engine_error(error: EngineError) -> NativeConversationError {
    match error {
        EngineError::SessionBusy => NativeConversationError::Busy,
        EngineError::HostClosed => NativeConversationError::HostClosed,
        EngineError::SessionIncarnationConflict => NativeConversationError::Conflict,
        EngineError::Store(error) if error.kind == SessionStoreErrorKind::Conflict => {
            NativeConversationError::Conflict
        }
        EngineError::Store(_) => NativeConversationError::Persistence,
        _ => NativeConversationError::Engine,
    }
}
