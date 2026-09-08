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
    StopReason, Turn, TurnEvent, TurnHandle,
};
use serde_json::{Value, json};

use crate::{
    NATIVE_CONTEXT_PREFERENCES_KEY, NATIVE_MODEL_PREFERENCES_KEY, NATIVE_SESSION_METADATA_KEY,
    NativeContextError, NativeContextPreferences, NativeModelPreferences,
    NativeModelPreferencesError, NativeModelSnapshot, NativeSessionLifecycle,
    NativeSessionLifecycleError, NativeSessionMetadata, NativeSessionMetadataError,
    NativeSessionMetadataMutationError, rename_native_session,
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
    session: Session,
    active: Arc<AtomicBool>,
}

impl fmt::Debug for NativeConversation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeConversation")
            .field("busy", &self.is_busy())
            .finish_non_exhaustive()
    }
}

impl NativeConversation {
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
        Checkpoint::decode(&record)?;
        validated_context_preferences(&record)?;
        NativeModelPreferences::from_metadata(&record.metadata)
            .map_err(NativeConversationError::InvalidModelPreferences)?;
        Ok(Self {
            session,
            active: Arc::new(AtomicBool::new(false)),
        })
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

    /// Returns canonical history, never a compacted provider projection.
    #[must_use]
    pub fn record(&self) -> SessionRecord {
        self.session.record()
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
            let _lease = AdmissionLease::acquire(&self.active)?;
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
        let _lease = AdmissionLease::acquire(&self.active)?;
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
        Box::pin(async move {
            let _lease = AdmissionLease::acquire(&self.active)?;
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
            self.session
                .update_metadata(record.revision, record.metadata)
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
        let _lease = AdmissionLease::acquire(&self.active)?;
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
            let _lease = AdmissionLease::acquire(&self.active)?;
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
            let _lease = AdmissionLease::acquire(&self.active)?;
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
        let _lease = AdmissionLease::acquire(&self.active)?;
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
        mut input: PendingInput,
        model: Option<NativeModelSnapshot>,
        now_ms: i64,
    ) -> Result<NativeConversationTurn, NativeConversationError> {
        let lease = AdmissionLease::acquire(&self.active)?;
        if self.session.has_active_turn() {
            return Err(NativeConversationError::Busy);
        }
        let mut record = self.session.record();
        let previous = Checkpoint::decode(&record)?;
        NativeModelPreferences::from_metadata(&record.metadata)
            .map_err(NativeConversationError::InvalidModelPreferences)?;
        let preferences = NativeContextPreferences::from_metadata(&record.metadata)
            .map_err(NativeConversationError::InvalidContext)?;
        // Disabled context is the original full-history path, including for
        // embedders whose admitted records exceed native projection bounds.
        let context = if preferences == NativeContextPreferences::default() {
            None
        } else {
            preferences
                .projection(&record)
                .map_err(NativeConversationError::InvalidContext)?
        };
        let checkpoint = Checkpoint {
            turn_sequence: record.next_turn_sequence,
            first_user_message: if matches!(&input.0, Some(ConversationInput::Prompt(_))) {
                record.messages.len()
            } else {
                previous
                    .ok_or(NativeConversationError::NoCheckpoint)?
                    .first_user_message
            },
            paused: false,
        };
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
        if let Some(model) = model {
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
        Ok(NativeConversationTurn {
            handle: turn.handle(),
            core: Some(turn),
            session: self.session.clone(),
            checkpoint,
            lease: Some(lease),
            finalization: None,
            terminal: None,
            done: false,
        })
    }
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

enum ConversationInput {
    Prompt(Prompt),
    Continue(InferenceOptions),
}

// Own untrusted inference JSON safely even when a future is never polled or
// admission rejects before core can install its own iterative-drop guard.
struct PendingInput(Option<ConversationInput>);
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

struct AdmissionLease(Arc<AtomicBool>);
impl AdmissionLease {
    fn acquire(active: &Arc<AtomicBool>) -> Result<Self, NativeConversationError> {
        active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| NativeConversationError::Busy)?;
        Ok(Self(Arc::clone(active)))
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
        self.core.take();
        self.finalization.take();
        self.lease.take();
        self.done = true;
    }

    fn stage_finalization(
        &mut self,
        terminal: Result<EngineEvent, NativeConversationError>,
    ) -> Result<(), NativeConversationError> {
        self.core.take();
        let mut record = self.session.record();
        let checkpoint = Checkpoint::decode(&record)?.ok_or(NativeConversationError::Conflict)?;
        if checkpoint.turn_sequence != self.checkpoint.turn_sequence
            || checkpoint.first_user_message != self.checkpoint.first_user_message
        {
            return Err(NativeConversationError::Conflict);
        }
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
        self.finalization = Some(
            self.session
                .update_metadata(record.revision, record.metadata),
        );
        self.terminal = Some(terminal);
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
                    return Poll::Ready(Some(Ok(event)));
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
