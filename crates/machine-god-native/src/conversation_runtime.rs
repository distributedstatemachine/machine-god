//! Native queue and model-selection ownership around one conversation.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_core::Stream;
use machine_god_core::{
    BoxFuture, CancellationToken, EngineEvent, InferenceOptions, MAX_SAFE_JSON_DEPTH, Prompt,
    SessionId, SessionIncarnationId, SessionRecord, SessionRevision, TurnEvent, TurnHandle,
};

use crate::conversation::{ConversationInput, PendingInput};
use crate::conversation_lifecycle::{
    LifecycleError, LifecycleGate, LifecyclePermit, LifecyclePhase, LifecycleQuiescence,
};
use crate::conversation_model_routes::{CurrentModel, ModelRouteRegistration};
use crate::tool_output_serializer::{
    CompactJsonScratch, CompactToolOutputLimits, measure_json_value_compact_with_scratch,
};
use crate::{
    LoadedNativeConfig, NativeContextPreferences, NativeConversation, NativeConversationError,
    NativeConversationHistory, NativeConversationModelRouteError, NativeConversationModelRoutes,
    NativeConversationTurn, NativeHistoryBackground, NativeHistoryFileEvidence,
    NativeModelCapabilities, NativeModelCatalog, NativeModelPreferences,
    NativeModelPreferencesError, NativeModelSnapshot, NativePausedTurn,
    NativePermissionPolicySnapshot, NativeUserConfigError, NativeUserConfigStore,
};

/// Independent native queue bounds; core still applies its configured turn limits.
pub const MAX_NATIVE_QUEUED_JOBS: usize = 64;
pub const MAX_NATIVE_QUEUED_PROMPT_BYTES: usize = 256 * 1024;
pub const MAX_NATIVE_QUEUED_OPTIONS_BYTES: usize = 64 * 1024;
pub const MAX_NATIVE_QUEUED_INPUT_BYTES: usize = 4 * 1024 * 1024;

/// Process-local queue identity, never a durable core turn identity or grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeQueuedJobId(u64);

impl NativeQueuedJobId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeConversationRuntimeError {
    Busy,
    Quiescing,
    Retired,
    StaleQuiescence,
    QueueLimit,
    InputLimit,
    IdentityExhausted,
    InvalidModelPreferences(NativeModelPreferencesError),
    ModelRoute(NativeConversationModelRouteError),
    Conversation(NativeConversationError),
}

impl fmt::Display for NativeConversationRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => f.write_str("conversation runtime is busy"),
            Self::Quiescing => f.write_str("conversation runtime is quiescing"),
            Self::Retired => f.write_str("conversation runtime is retired"),
            Self::StaleQuiescence => f.write_str("conversation quiescence ownership is stale"),
            Self::QueueLimit => f.write_str("conversation queue limit exceeded"),
            Self::InputLimit => f.write_str("conversation queued input limit exceeded"),
            Self::IdentityExhausted => f.write_str("conversation runtime identity exhausted"),
            Self::InvalidModelPreferences(error) => error.fmt(f),
            Self::ModelRoute(error) => error.fmt(f),
            Self::Conversation(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for NativeConversationRuntimeError {}

impl From<LifecycleError> for NativeConversationRuntimeError {
    fn from(error: LifecycleError) -> Self {
        match error {
            LifecycleError::Busy => Self::Busy,
            LifecycleError::Quiescing => Self::Quiescing,
            LifecycleError::Retired => Self::Retired,
            LifecycleError::Exhausted => Self::IdentityExhausted,
            LifecycleError::Stale => Self::StaleQuiescence,
        }
    }
}

/// Process-local admission state, independent of durable session identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeConversationRuntimePhase {
    Open,
    Quiescing,
    Retired,
}

impl From<NativeConversationError> for NativeConversationRuntimeError {
    fn from(error: NativeConversationError) -> Self {
        Self::Conversation(error)
    }
}

/// Runtime observations, not cross-process durability evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeConversationRuntimeStatus {
    pub phase: NativeConversationRuntimePhase,
    pub queued_jobs: usize,
    pub queued_input_bytes: usize,
    pub active: bool,
    pub model_preferences_pending: bool,
    pub model_preferences_generation: u64,
}

/// Session-target outcome only. User defaults require an independent write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeModelPreferencePersistence {
    Unchanged,
    Deferred,
    Saved {
        generation: u64,
        revision: SessionRevision,
    },
}

/// Independent outcomes for the same captured runtime preference generation.
/// A successful target never implies that the other target was saved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeModelPreferenceCommit {
    pub generation: u64,
    pub session: Result<NativeModelPreferencePersistence, NativeConversationRuntimeError>,
    pub user_defaults: Result<LoadedNativeConfig, NativeUserConfigError>,
}

enum PreferenceSave {
    Observed(NativeModelPreferencePersistence),
    Write(RuntimeLease),
}

struct QueuedJob {
    id: NativeQueuedJobId,
    input: PendingInput,
    bytes: usize,
    checkpoint: Option<u64>,
}

type TakenJob = (
    QueuedJob,
    NativeModelSnapshot,
    u64,
    Option<crate::NativePermissionPolicySnapshot>,
    Option<crate::NativeWorkspaceScopeSnapshot>,
);

struct RuntimeState {
    preferences: NativeModelPreferences,
    generation: u64,
    saved_generation: Option<u64>,
    catalog: Option<Arc<NativeModelCatalog>>,
    queue: VecDeque<QueuedJob>,
    bytes: usize,
    next_id: u64,
    active: bool,
    active_handle: Option<TurnHandle>,
    active_cancel_dispatched: bool,
}

impl CurrentModel for Mutex<RuntimeState> {
    fn current_model(&self) -> String {
        self.lock()
            .expect("runtime state poisoned")
            .preferences
            .model()
            .to_owned()
    }
}

/// One session's FIFO and requested model state, without a detached worker.
/// Pending jobs share the current selection; a taken job owns a fixed snapshot.
pub struct NativeConversationRuntime {
    conversation: Arc<NativeConversation>,
    state: Arc<Mutex<RuntimeState>>,
    model_route: Option<Arc<ModelRouteRegistration>>,
    lifecycle: Arc<LifecycleGate>,
}

impl fmt::Debug for NativeConversationRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeConversationRuntime")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl NativeConversationRuntime {
    /// File controls may run during a turn but cannot outlive lifecycle admission.
    #[cfg(feature = "ai-gateway-http")]
    pub(crate) fn acquire_file_control(
        &self,
    ) -> Result<LifecyclePermit, NativeConversationRuntimeError> {
        self.lifecycle.acquire().map_err(Into::into)
    }

    /// Restores saved preferences when present, otherwise uses explicit startup
    /// defaults. A process override replaces only the model, not saved controls.
    /// Construction performs no persistence, catalog or provider work.
    ///
    /// # Errors
    /// Rejects busy/corrupt conversations and invalid process model overrides.
    pub fn new(
        conversation: NativeConversation,
        startup: NativeModelPreferences,
        process_model_override: Option<&str>,
    ) -> Result<Self, NativeConversationRuntimeError> {
        let saved = conversation.model_preferences()?;
        let mut preferences = saved.clone().unwrap_or(startup);
        if let Some(model) = process_model_override {
            preferences
                .set_model(model)
                .map_err(NativeConversationRuntimeError::InvalidModelPreferences)?;
        }
        let saved_generation = (saved.as_ref() == Some(&preferences)).then_some(0);
        let lifecycle = LifecycleGate::new();
        conversation.bind_lifecycle(&lifecycle)?;
        Ok(Self {
            conversation: Arc::new(conversation),
            lifecycle,
            model_route: None,
            state: Arc::new(Mutex::new(RuntimeState {
                preferences,
                generation: 0,
                saved_generation,
                catalog: None,
                queue: VecDeque::new(),
                bytes: 0,
                next_id: 1,
                active: false,
                active_handle: None,
                active_cancel_dispatched: false,
            })),
        })
    }

    /// Constructs a runtime and registers its current selection for secondary
    /// search workers. Main jobs still capture immutable admission snapshots.
    ///
    /// # Errors
    /// Rejects invalid preferences, duplicate incarnations or route capacity.
    ///
    /// # Panics
    /// Panics if an earlier panic poisoned the routing mutex.
    pub fn new_with_model_routes(
        conversation: NativeConversation,
        startup: NativeModelPreferences,
        process_model_override: Option<&str>,
        routes: &Arc<NativeConversationModelRoutes>,
    ) -> Result<Self, NativeConversationRuntimeError> {
        let mut runtime = Self::new(conversation, startup, process_model_override)?;
        let source: Arc<dyn CurrentModel> = runtime.state.clone();
        runtime.model_route = Some(
            routes
                .register(
                    runtime.conversation.id(),
                    runtime.conversation.incarnation_id(),
                    Arc::downgrade(&source),
                )
                .map_err(NativeConversationRuntimeError::ModelRoute)?,
        );
        Ok(runtime)
    }

    #[must_use]
    pub fn id(&self) -> SessionId {
        self.conversation.id()
    }

    /// Reads exact session identity without cloning the transcript.
    #[must_use]
    pub fn incarnation_id(&self) -> SessionIncarnationId {
        self.conversation.incarnation_id()
    }

    #[must_use]
    pub fn record(&self) -> SessionRecord {
        self.conversation.record()
    }

    /// Pins canonical memory without cloning transcript or metadata payloads.
    /// Later commits cannot retarget this observation; writers remain unblocked.
    #[must_use]
    pub fn record_snapshot(&self) -> Arc<SessionRecord> {
        self.conversation.record_snapshot()
    }

    /// Closes new admission and requests cancellation of an owned active turn.
    /// The caller must keep polling its admission/turn and native finalizer.
    /// Dropping the returned guard reopens admission without changing inputs.
    /// # Errors
    /// Rejects another quiescence owner, retired state or generation exhaustion.
    /// # Panics
    /// Panics if an earlier panic poisoned runtime state.
    pub fn begin_quiescence(
        &self,
    ) -> Result<NativeRuntimeQuiescence, NativeConversationRuntimeError> {
        let inner = self.lifecycle.begin_quiescence()?;
        let handle = {
            let mut state = self.state.lock().expect("runtime state poisoned");
            cancellation_to_dispatch(&mut state)
        };
        if let Some(handle) = handle {
            let _ = handle.cancel();
        }
        Ok(NativeRuntimeQuiescence {
            inner,
            conversation: Arc::clone(&self.conversation),
            state: Arc::clone(&self.state),
            model_route: self.model_route.clone(),
        })
    }

    /// Process-local policy controls. Taken turns retain their mode snapshot.
    #[must_use]
    pub fn permissions(&self) -> Option<&Arc<crate::NativePermissionSession>> {
        self.conversation.permissions()
    }

    /// Observes persisted context selection without starting queued work.
    /// # Errors
    /// Rejects active runtime work or invalid saved context.
    /// # Panics
    /// Panics if an earlier panic poisoned the runtime state mutex.
    pub fn context_preferences(
        &self,
    ) -> Result<NativeContextPreferences, NativeConversationRuntimeError> {
        let _lease = self.acquire_idle(false)?;
        Ok(self.conversation.context_preferences()?)
    }

    /// Observes a paused checkpoint, not permission to replay historical effects.
    /// # Errors
    /// Rejects active runtime work or invalid saved checkpoint state.
    /// # Panics
    /// Panics if an earlier panic poisoned the runtime state mutex.
    pub fn paused_turn(&self) -> Result<Option<NativePausedTurn>, NativeConversationRuntimeError> {
        let _lease = self.acquire_idle(false)?;
        Ok(self.conversation.paused_turn()?)
    }

    /// Observes native typed history without starting queued work.
    /// # Errors
    /// Rejects active work and malformed historical facts.
    /// # Panics
    /// Panics if an earlier panic poisoned runtime state.
    pub fn history(&self) -> Result<NativeConversationHistory, NativeConversationRuntimeError> {
        let _lease = self.acquire_idle(false)?;
        Ok(self.conversation.history()?)
    }

    /// Saves pending tool observations without consuming queued input or model
    /// changes. The borrowed future is inert until polled.
    /// # Panics
    /// Polling panics if an earlier panic poisoned runtime state.
    #[must_use]
    pub fn flush_history_observations(
        &self,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<Option<SessionRevision>, NativeConversationRuntimeError>> {
        Box::pin(async move {
            let _lease = self.acquire_idle(false)?;
            Ok(self.conversation.flush_history_observations(now_ms).await?)
        })
    }

    /// Saves explicitly observed file history under runtime admission.
    /// The future is inert before polling; queued inputs and selection survive.
    /// # Panics
    /// Polling panics if an earlier panic poisoned runtime state.
    #[must_use]
    pub fn record_history_file(
        &self,
        first_user_message: usize,
        turn_sequence: u64,
        evidence: NativeHistoryFileEvidence,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<SessionRevision, NativeConversationRuntimeError>> {
        Box::pin(async move {
            let _lease = self.acquire_idle(false)?;
            Ok(self
                .conversation
                .record_history_file(first_user_message, turn_sequence, evidence, now_ms)
                .await?)
        })
    }

    /// Saves descriptive background observations against an exact history attempt.
    /// This neither starts a process nor grants authority over a historical ID.
    /// # Panics
    /// Polling panics if an earlier panic poisoned runtime state.
    #[must_use]
    pub fn set_history_background(
        &self,
        first_user_message: usize,
        turn_sequence: u64,
        background: Option<NativeHistoryBackground>,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<SessionRevision, NativeConversationRuntimeError>> {
        Box::pin(async move {
            let _lease = self.acquire_idle(false)?;
            Ok(self
                .conversation
                .set_history_background(first_user_message, turn_sequence, background, now_ms)
                .await?)
        })
    }

    /// Persists a title through the same runtime admission as jobs and saves.
    /// Pending jobs and requested model settings are preserved. No work starts
    /// until this borrowed future is polled; dropping it releases admission.
    /// # Panics
    /// Polling panics if an earlier panic poisoned the runtime state mutex.
    #[must_use]
    pub fn rename<'a>(
        &'a self,
        title: &'a str,
        now_ms: i64,
    ) -> BoxFuture<'a, Result<SessionRevision, NativeConversationRuntimeError>> {
        Box::pin(async move {
            let _lease = self.acquire_idle(false)?;
            Ok(self.conversation.rename(title, now_ms).await?)
        })
    }

    /// Persists manual context selection, never deleting history or queued input.
    /// This borrowed future is inert until polled. A pending publication owns
    /// runtime admission; failure/drop retains native reconciliation semantics.
    /// # Panics
    /// Polling panics if an earlier panic poisoned the runtime state mutex.
    #[must_use]
    pub fn compact(
        &self,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<bool, NativeConversationRuntimeError>> {
        Box::pin(async move {
            let _lease = self.acquire_idle(false)?;
            Ok(self.conversation.compact(now_ms).await?)
        })
    }

    /// Persists the automatic context limit for future job admission. Zero
    /// disables automatic compaction without clearing an existing manual cut.
    /// This borrowed future performs no effects until polled.
    /// # Panics
    /// Polling panics if an earlier panic poisoned the runtime state mutex.
    #[must_use]
    pub fn set_max_history_turns(
        &self,
        maximum: usize,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<SessionRevision, NativeConversationRuntimeError>> {
        Box::pin(async move {
            let _lease = self.acquire_idle(false)?;
            Ok(self
                .conversation
                .set_max_history_turns(maximum, now_ms)
                .await?)
        })
    }

    /// # Panics
    /// Panics if an earlier panic poisoned the runtime state mutex.
    #[must_use]
    pub fn status(&self) -> NativeConversationRuntimeStatus {
        let phase = match self.lifecycle.phase() {
            LifecyclePhase::Open => NativeConversationRuntimePhase::Open,
            LifecyclePhase::Quiescing => NativeConversationRuntimePhase::Quiescing,
            LifecyclePhase::Retired => NativeConversationRuntimePhase::Retired,
        };
        let state = self.state.lock().expect("runtime state poisoned");
        NativeConversationRuntimeStatus {
            phase,
            queued_jobs: state.queue.len(),
            queued_input_bytes: state.bytes,
            active: state.active,
            model_preferences_pending: state.saved_generation != Some(state.generation),
            model_preferences_generation: state.generation,
        }
    }

    /// # Panics
    /// Panics if an earlier panic poisoned the runtime state mutex.
    #[must_use]
    pub fn model_preferences(&self) -> NativeModelPreferences {
        self.state
            .lock()
            .expect("runtime state poisoned")
            .preferences
            .clone()
    }

    /// Accepts new runtime settings immediately, even while a turn/save is active.
    /// All pending/future jobs observe the replacement; a taken job cannot change.
    /// The returned generation is acceptance, not a persistence receipt.
    ///
    /// # Errors
    /// Rejects generation exhaustion without changing the accepted selection.
    ///
    /// # Panics
    /// Panics if an earlier panic poisoned the runtime state mutex.
    pub fn set_model_preferences(
        &self,
        preferences: NativeModelPreferences,
    ) -> Result<u64, NativeConversationRuntimeError> {
        let _permit = self.lifecycle.acquire()?;
        let mut state = self.state.lock().expect("runtime state poisoned");
        let generation = state
            .generation
            .checked_add(1)
            .ok_or(NativeConversationRuntimeError::IdentityExhausted)?;
        state.preferences = preferences;
        state.generation = generation;
        Ok(generation)
    }

    /// Replaces explicitly fetched capabilities for future admissions only.
    ///
    /// # Errors
    /// Rejects quiescing/retired ownership or exhausted admission capacity.
    ///
    /// # Panics
    /// Panics if an earlier panic poisoned the runtime state mutex.
    pub fn set_model_catalog(
        &self,
        catalog: Arc<NativeModelCatalog>,
    ) -> Result<(), NativeConversationRuntimeError> {
        let _permit = self.lifecycle.acquire()?;
        let previous = self
            .state
            .lock()
            .expect("runtime state poisoned")
            .catalog
            .replace(catalog);
        drop(previous);
        Ok(())
    }

    /// Queues bounded owned input without saving or invoking a provider. Caller
    /// inference controls other than model/Gateway settings survive admission.
    ///
    /// # Errors
    /// Rejects input/queue bounds or exhausted process-local job identities.
    pub fn enqueue(
        &self,
        prompt: Prompt,
    ) -> Result<NativeQueuedJobId, NativeConversationRuntimeError> {
        let input = PendingInput(Some(ConversationInput::Prompt(prompt)));
        let bytes = input_bytes(&input)?;
        self.insert(input, bytes, None)
    }

    /// Queues explicit continuation only with an idle, empty queue and a valid
    /// paused checkpoint. The checkpoint identity is checked again when taken.
    ///
    /// # Errors
    /// Rejects busy/missing checkpoints, input bounds and identity exhaustion.
    pub fn enqueue_continuation(
        &self,
        options: InferenceOptions,
    ) -> Result<NativeQueuedJobId, NativeConversationRuntimeError> {
        let input = PendingInput(Some(ConversationInput::Continue(options)));
        let bytes = input_bytes(&input)?;
        let _lease = self.acquire_idle(true)?;
        let checkpoint = self
            .conversation
            .paused_turn()?
            .ok_or(NativeConversationError::NoCheckpoint)?;
        self.insert(input, bytes, Some(checkpoint.turn_sequence))
    }

    fn insert(
        &self,
        input: PendingInput,
        bytes: usize,
        checkpoint: Option<u64>,
    ) -> Result<NativeQueuedJobId, NativeConversationRuntimeError> {
        let _permit = self.lifecycle.acquire()?;
        let mut state = self.state.lock().expect("runtime state poisoned");
        // An ordinary prompt may arrive while continuation checks the native
        // checkpoint outside this mutex. Do not put recovery behind new input.
        if checkpoint.is_some() && !state.queue.is_empty() {
            return Err(NativeConversationRuntimeError::Busy);
        }
        if state.queue.len() == MAX_NATIVE_QUEUED_JOBS
            || bytes > MAX_NATIVE_QUEUED_INPUT_BYTES - state.bytes
        {
            return Err(NativeConversationRuntimeError::QueueLimit);
        }
        let next = state
            .next_id
            .checked_add(1)
            .ok_or(NativeConversationRuntimeError::IdentityExhausted)?;
        let id = NativeQueuedJobId(state.next_id);
        state.queue.push_back(QueuedJob {
            id,
            input,
            bytes,
            checkpoint,
        });
        state.next_id = next;
        state.bytes += bytes;
        Ok(id)
    }

    /// Removes only still-pending input. Destruction occurs outside the mutex.
    /// Returns false without mutation while admission is unavailable.
    ///
    /// # Panics
    /// Panics on a poisoned mutex or an internally inconsistent queue index.
    #[must_use]
    pub fn cancel_queued(&self, id: NativeQueuedJobId) -> bool {
        let Ok(_permit) = self.lifecycle.acquire() else {
            return false;
        };
        let removed = {
            let mut state = self.state.lock().expect("runtime state poisoned");
            let Some(index) = state.queue.iter().position(|job| job.id == id) else {
                return false;
            };
            let job = state.queue.remove(index).expect("located queued job");
            state.bytes -= job.bytes;
            job
        };
        drop(removed);
        true
    }

    /// Clears pending input, not the owned active turn or durable transcript.
    /// Returns zero without mutation while admission is unavailable.
    ///
    /// # Panics
    /// Panics if an earlier panic poisoned the runtime state mutex.
    #[must_use]
    pub fn clear_queued(&self) -> usize {
        let Ok(_permit) = self.lifecycle.acquire() else {
            return 0;
        };
        let removed = {
            let mut state = self.state.lock().expect("runtime state poisoned");
            state.bytes = 0;
            std::mem::take(&mut state.queue)
        };
        removed.len()
    }

    /// Takes one FIFO job on first poll, atomically captures current preferences
    /// and capabilities, then admits it through the conversation owner. Failed
    /// or dropped admissions are never automatically requeued (publication may
    /// have occurred). An unpolled future leaves the queue untouched.
    ///
    /// # Panics
    /// Polling panics on a poisoned mutex or internally missing owned input.
    #[must_use]
    pub fn start_next(
        &self,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<Option<NativeConversationRuntimeTurn>, NativeConversationRuntimeError>>
    {
        Box::pin(async move {
            let lease = self.acquire_idle(false)?;
            let Some((job, snapshot, generation, policy, workspace)) =
                self.take_job(&lease.permit)?
            else {
                return Ok(None);
            };
            if let Some(expected) = job.checkpoint
                && self
                    .conversation
                    .paused_turn()?
                    .map(|value| value.turn_sequence)
                    != Some(expected)
            {
                return Err(NativeConversationError::Conflict.into());
            }
            // Dropped or failed admission can be publication-uncertain; only a
            // confirmed reservation below restores a saved-generation receipt.
            self.state
                .lock()
                .expect("runtime state poisoned")
                .saved_generation = None;
            let turn = self
                .conversation
                .start_with_policy_admitted(
                    job.input,
                    Some(snapshot.clone()),
                    policy,
                    now_ms,
                    &lease.permit,
                    workspace,
                )
                .await?;
            let handle = turn.handle();
            let cancellation = {
                let mut state = self.state.lock().expect("runtime state poisoned");
                state.saved_generation = Some(generation);
                state.active_handle = Some(handle.clone());
                if lease.permit.was_quiesced() {
                    cancellation_to_dispatch(&mut state)
                } else {
                    None
                }
            };
            // A quiescence request can precede the core reservation's handle.
            // Publish the handle first, then recheck: either this path or the
            // requesting path observes it before any provider work is polled.
            if let Some(handle) = cancellation {
                let _ = handle.cancel();
            }
            Ok(Some(NativeConversationRuntimeTurn {
                core: Some(turn),
                model_route: self.model_route.clone(),
                lease: Some(lease),
                id: job.id,
                snapshot,
            }))
        })
    }

    fn take_job(
        &self,
        permit: &LifecyclePermit,
    ) -> Result<Option<TakenJob>, NativeConversationRuntimeError> {
        let mut state = self.state.lock().expect("runtime state poisoned");
        let Some(job) = state.queue.pop_front() else {
            return Ok(None);
        };
        state.bytes -= job.bytes;
        let unsupported = NativeModelCapabilities::default();
        let capabilities = state
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.details(state.preferences.model()))
            .map_or(&unsupported, |entry| entry.capabilities());
        let snapshot = NativeModelSnapshot::new(&state.preferences, capabilities);
        let policy = self
            .conversation
            .permissions()
            .map(|owner| owner.snapshot_admitted(permit))
            .transpose()
            .map_err(|_| NativeConversationError::Engine)?;
        let workspace = self.conversation.capture_workspace_scope()?;
        Ok(Some((job, snapshot, state.generation, policy, workspace)))
    }

    /// Flushes one captured preference generation while idle. Concurrent runtime
    /// changes stay accepted and dirty; this does not loop or rewrite an active
    /// turn. A deferred/failed/dropped flush is not a successful persistence claim.
    ///
    /// # Panics
    /// Polling panics if an earlier panic poisoned the runtime state mutex.
    #[must_use]
    pub fn flush_model_preferences(
        &self,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<NativeModelPreferencePersistence, NativeConversationRuntimeError>>
    {
        self.flush_model_preferences_with_access(now_ms, None)
    }

    pub(crate) fn flush_model_preferences_with_access(
        &self,
        now_ms: i64,
        access: Option<Arc<dyn machine_god_core::SessionStoreAccess>>,
    ) -> BoxFuture<'_, Result<NativeModelPreferencePersistence, NativeConversationRuntimeError>>
    {
        Box::pin(async move {
            let permit = self.lifecycle.acquire()?;
            let (preferences, generation, save) = self.prepare_preference_save(permit);
            self.save_preferences(preferences, generation, save, now_ms, access)
                .await
        })
    }

    /// Attempts session and explicitly authorized user-default persistence for
    /// one captured selection. Neither target's failure suppresses the other.
    /// The user snapshot is read before awaiting the session save, so a pending
    /// older session write cannot silently overwrite changed user configuration.
    ///
    /// This borrowed future is inert until polled. It does not change selection,
    /// retry conflicts, or spawn work. Dropping a pending session save abandons
    /// this operation without starting a detached user writer or claiming a receipt.
    /// The user store's bounded synchronous I/O contract still applies.
    ///
    /// # Errors
    /// Lifecycle admission failure rejects both targets before any user/store
    /// work. Once admitted, the returned commit keeps independent target errors.
    ///
    /// # Panics
    /// Polling panics if an earlier panic poisoned the runtime state mutex.
    #[must_use]
    pub fn persist_model_preferences<'a>(
        &'a self,
        user_store: &'a NativeUserConfigStore,
        now_ms: i64,
    ) -> BoxFuture<'a, Result<NativeModelPreferenceCommit, NativeConversationRuntimeError>> {
        Box::pin(async move {
            let _operation = self.lifecycle.acquire()?;
            let permit = self.lifecycle.acquire()?;
            let (preferences, generation, save) = self.prepare_preference_save(permit);
            let user_snapshot = user_store.load();
            let session = self
                .save_preferences(preferences.clone(), generation, save, now_ms, None)
                .await;
            let user_defaults = match user_snapshot {
                Ok(snapshot) => {
                    user_store
                        .set_model_preferences(&snapshot, &preferences)
                        .await
                }
                Err(error) => Err(error),
            };
            Ok(NativeModelPreferenceCommit {
                generation,
                session,
                user_defaults,
            })
        })
    }

    fn prepare_preference_save(
        &self,
        permit: LifecyclePermit,
    ) -> (NativeModelPreferences, u64, PreferenceSave) {
        let mut state = self.state.lock().expect("runtime state poisoned");
        let save = if state.saved_generation == Some(state.generation) {
            PreferenceSave::Observed(NativeModelPreferencePersistence::Unchanged)
        } else if state.active {
            PreferenceSave::Observed(NativeModelPreferencePersistence::Deferred)
        } else {
            state.active = true;
            PreferenceSave::Write(RuntimeLease {
                state: Arc::clone(&self.state),
                permit,
            })
        };
        (state.preferences.clone(), state.generation, save)
    }

    async fn save_preferences(
        &self,
        preferences: NativeModelPreferences,
        generation: u64,
        save: PreferenceSave,
        now_ms: i64,
        access: Option<Arc<dyn machine_god_core::SessionStoreAccess>>,
    ) -> Result<NativeModelPreferencePersistence, NativeConversationRuntimeError> {
        let lease = match save {
            PreferenceSave::Observed(outcome) => return Ok(outcome),
            PreferenceSave::Write(lease) => lease,
        };
        let revision = self
            .conversation
            .set_model_preferences_with_access(preferences, now_ms, access)
            .await?;
        self.state
            .lock()
            .expect("runtime state poisoned")
            .saved_generation = Some(generation);
        drop(lease);
        Ok(NativeModelPreferencePersistence::Saved {
            generation,
            revision,
        })
    }

    fn acquire_idle(
        &self,
        require_empty: bool,
    ) -> Result<RuntimeLease, NativeConversationRuntimeError> {
        let permit = self.lifecycle.acquire()?;
        let mut state = self.state.lock().expect("runtime state poisoned");
        if state.active || (require_empty && !state.queue.is_empty()) {
            return Err(NativeConversationRuntimeError::Busy);
        }
        state.active = true;
        Ok(RuntimeLease {
            state: Arc::clone(&self.state),
            permit,
        })
    }

    pub(crate) fn acquire_workspace_control(
        &self,
    ) -> Result<NativeWorkspaceControlLease, NativeConversationRuntimeError> {
        let lease = self.acquire_idle(true)?;
        let conversation = self
            .conversation
            .acquire_workspace_control()
            .map_err(|error| match error {
                NativeConversationError::Busy => NativeConversationRuntimeError::Busy,
                error => NativeConversationRuntimeError::Conversation(error),
            })?;
        Ok(NativeWorkspaceControlLease {
            _conversation: conversation,
            _lease: lease,
        })
    }
}

/// Native-owner-only idle workspace control admission, retained through worker cleanup.
pub(crate) struct NativeWorkspaceControlLease {
    _conversation: crate::conversation::AdmissionLease,
    _lease: RuntimeLease,
}

impl Drop for NativeConversationRuntime {
    fn drop(&mut self) {
        // Destruction disposes owned input even when a separate quiescence
        // guard retains the gate. It is not a new host queue mutation.
        let removed = {
            let mut state = self.state.lock().expect("runtime state poisoned");
            state.bytes = 0;
            std::mem::take(&mut state.queue)
        };
        drop(removed);
    }
}

/// Exclusive reversible admission fence. This owns no worker and never drives
/// a separately owned turn, retries persistence, or establishes a save receipt.
pub struct NativeRuntimeQuiescence {
    inner: LifecycleQuiescence,
    conversation: Arc<NativeConversation>,
    state: Arc<Mutex<RuntimeState>>,
    model_route: Option<Arc<ModelRouteRegistration>>,
}
impl fmt::Debug for NativeRuntimeQuiescence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeRuntimeQuiescence { .. }")
    }
}
impl NativeRuntimeQuiescence {
    /// Copies actual settled selections while this exact guard keeps admission
    /// closed. Does not copy grants, saved rules, history or persistence receipts.
    /// # Errors
    /// Returns `Busy` until every admitted operation has released ownership,
    /// or rejects stale/retired ownership and unavailable permission routing.
    /// # Panics
    /// Panics if an earlier panic poisoned runtime or permission state.
    pub fn selection_snapshot(
        &self,
    ) -> Result<NativeQuiescentSelectionSnapshot, NativeConversationRuntimeError> {
        self.inner.check_idle()?;
        let permission_policy = self
            .conversation
            .permissions()
            .map(|owner| owner.snapshot_quiescent(&self.inner))
            .transpose()
            .map_err(|_| NativeConversationError::Engine)?;
        let state = self.state.lock().expect("runtime state poisoned");
        Ok(NativeQuiescentSelectionSnapshot {
            model_preferences: state.preferences.clone(),
            model_catalog: state.catalog.clone(),
            permission_policy,
        })
    }

    /// Waits for owned operations to finish or drop, including native finalizers
    /// and independent policy/user-default saves. Construction is inert and only
    /// one waiter can borrow this guard. Idle does not imply successful saving.
    #[must_use]
    pub fn wait_idle(&mut self) -> BoxFuture<'_, Result<(), NativeConversationRuntimeError>> {
        Box::pin(async move { self.inner.wait_idle().await.map_err(Into::into) })
    }
    /// Irreversibly closes this runtime and detaches its exact native routes.
    /// Discards queued, never-taken input only after the idle check succeeds.
    /// Existing aliases remain observational but cannot admit work or mutations.
    /// # Errors
    /// Rejects outstanding owned work or stale ownership. Failed consumption
    /// releases this fence, reopening admission without discarding queued input.
    /// # Panics
    /// Panics if an earlier panic poisoned runtime or routing state.
    pub fn retire(mut self) -> Result<(), NativeConversationRuntimeError> {
        self.try_retire()
    }

    /// Attempts irreversible retirement without consuming this admission fence.
    /// On success, detaches exact routes and discards never-taken queued input.
    /// On failure, retains ownership so the caller can keep driving outstanding
    /// work without reopening admission or losing transition receipts.
    /// # Errors
    /// Rejects outstanding owned work or stale/already-retired ownership.
    /// # Panics
    /// Panics if an earlier panic poisoned runtime or routing state.
    pub fn try_retire(&mut self) -> Result<(), NativeConversationRuntimeError> {
        self.inner.try_retire()?;
        self.conversation.retire_lifecycle_routes();
        if let Some(route) = &self.model_route {
            route.retire();
        }
        let removed = {
            let mut state = self.state.lock().expect("runtime state poisoned");
            state.bytes = 0;
            std::mem::take(&mut state.queue)
        };
        drop(removed);
        Ok(())
    }
}

/// Bounded settled selection values, not an admission or persistence receipt.
pub struct NativeQuiescentSelectionSnapshot {
    model_preferences: NativeModelPreferences,
    model_catalog: Option<Arc<NativeModelCatalog>>,
    permission_policy: Option<NativePermissionPolicySnapshot>,
}
impl fmt::Debug for NativeQuiescentSelectionSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeQuiescentSelectionSnapshot { .. }")
    }
}
impl NativeQuiescentSelectionSnapshot {
    #[must_use]
    pub fn model_preferences(&self) -> &NativeModelPreferences {
        &self.model_preferences
    }
    #[must_use]
    pub fn model_catalog(&self) -> Option<&Arc<NativeModelCatalog>> {
        self.model_catalog.as_ref()
    }
    #[must_use]
    pub fn permission_policy(&self) -> Option<&NativePermissionPolicySnapshot> {
        self.permission_policy.as_ref()
    }
}

struct RuntimeLease {
    state: Arc<Mutex<RuntimeState>>,
    permit: LifecyclePermit,
}
impl Drop for RuntimeLease {
    fn drop(&mut self) {
        let handle = {
            let mut state = self.state.lock().expect("runtime state poisoned");
            state.active = false;
            state.active_cancel_dispatched = false;
            state.active_handle.take()
        };
        drop(handle);
    }
}

fn cancellation_to_dispatch(state: &mut RuntimeState) -> Option<TurnHandle> {
    if state.active_cancel_dispatched {
        return None;
    }
    let handle = state.active_handle.clone()?;
    state.active_cancel_dispatched = true;
    Some(handle)
}

/// Owned active job. Its lease covers native checkpoint finalization too.
/// Drop releases core work before opening runtime admission; no worker detaches.
pub struct NativeConversationRuntimeTurn {
    core: Option<NativeConversationTurn>,
    model_route: Option<Arc<ModelRouteRegistration>>,
    lease: Option<RuntimeLease>,
    id: NativeQueuedJobId,
    snapshot: NativeModelSnapshot,
}

impl NativeConversationRuntimeTurn {
    #[must_use]
    pub const fn queued_id(&self) -> NativeQueuedJobId {
        self.id
    }
    #[must_use]
    pub const fn model_snapshot(&self) -> &NativeModelSnapshot {
        &self.snapshot
    }
    #[must_use]
    pub fn handle(&self) -> Option<TurnHandle> {
        self.core.as_ref().map(NativeConversationTurn::handle)
    }
    fn finish(&mut self) {
        self.core.take();
        self.model_route.take();
        self.lease.take();
    }
}

impl fmt::Debug for NativeConversationRuntimeTurn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeConversationRuntimeTurn")
            .field("queued_id", &self.id)
            .field("active", &self.core.is_some())
            .finish_non_exhaustive()
    }
}

impl Stream for NativeConversationRuntimeTurn {
    type Item = Result<EngineEvent, NativeConversationError>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(core) = &mut self.core else {
            return Poll::Ready(None);
        };
        match Pin::new(core).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(event) => {
                if !matches!(&event, Some(Ok(event)) if !matches!(event.payload, TurnEvent::Completed { .. } | TurnEvent::Failed { .. }))
                {
                    self.finish();
                }
                Poll::Ready(event)
            }
        }
    }
}

impl Drop for NativeConversationRuntimeTurn {
    fn drop(&mut self) {
        self.finish();
    }
}

fn input_bytes(input: &PendingInput) -> Result<usize, NativeConversationRuntimeError> {
    let (text_bytes, options) = match input.0.as_ref().expect("owned queue input") {
        ConversationInput::Prompt(prompt) => (prompt.text.len(), &prompt.options),
        ConversationInput::Continue(options) => (0, options),
    };
    let invalid = NativeConversationRuntimeError::InputLimit;
    if text_bytes > MAX_NATIVE_QUEUED_PROMPT_BYTES
        || options
            .model
            .as_ref()
            .is_some_and(|model| model.len() > MAX_NATIVE_QUEUED_OPTIONS_BYTES)
    {
        return Err(invalid);
    }
    // Reuse the existing iterative serializer before serde traverses admitted
    // trees. Serialized bytes bound aggregate nodes independently of root count.
    let cancellation = CancellationToken::new();
    let mut scratch = CompactJsonScratch::new();
    let mut remaining = MAX_NATIVE_QUEUED_OPTIONS_BYTES;
    for (key, value) in &options.metadata {
        remaining = remaining.checked_sub(key.len() + 3).ok_or(invalid)?;
        let bytes = measure_json_value_compact_with_scratch(
            value,
            &mut scratch,
            CompactToolOutputLimits {
                output_bytes: remaining,
                json_depth: MAX_SAFE_JSON_DEPTH,
                json_nodes: MAX_NATIVE_QUEUED_OPTIONS_BYTES,
            },
            &cancellation,
        )
        .map_err(|_| invalid)?;
        remaining = remaining.checked_sub(bytes).ok_or(invalid)?;
    }
    let mut counter = OptionsBytes(0);
    serde_json::to_writer(&mut counter, options).map_err(|_| invalid)?;
    Ok(text_bytes + counter.0)
}

struct OptionsBytes(usize);

#[cfg(test)]
#[path = "workspace_context/runtime_tests.rs"]
mod workspace_control_tests;
impl Write for OptionsBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_NATIVE_QUEUED_OPTIONS_BYTES - self.0 {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use machine_god_core::{Engine, SessionIncarnationId};
    use machine_god_testkit::{
        InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler,
    };

    #[test]
    fn snapshot_reads_selection_published_by_previously_admitted_setter_after_fence() {
        let engine = Engine::builder()
            .session_store(InMemorySessionStore::default())
            .provider(ScriptedModelProvider::new("fixture", []))
            .permission_handler(ScriptedPermissionHandler::new([]))
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("settled-selection").unwrap(),
                SessionIncarnationId::new("exact-life").unwrap(),
            )
            .unwrap();
        let runtime = NativeConversationRuntime::new(
            NativeConversation::from_session(session).unwrap(),
            NativeModelPreferences::new(
                "private/before",
                crate::NativeReasoningEffort::default(),
                false,
            )
            .unwrap(),
            None,
        )
        .unwrap();
        let admitted = runtime.lifecycle.acquire().unwrap();
        let guard = runtime.begin_quiescence().unwrap();
        assert!(matches!(
            guard.selection_snapshot(),
            Err(NativeConversationRuntimeError::Busy)
        ));
        // Deterministic interleaving of the synchronous setter's admission and
        // state publication: a successful old admission may finish behind fence.
        let selected = NativeModelPreferences::new(
            "private/after",
            crate::NativeReasoningEffort::default(),
            true,
        )
        .unwrap();
        runtime.state.lock().unwrap().preferences = selected.clone();
        assert!(matches!(
            guard.selection_snapshot(),
            Err(NativeConversationRuntimeError::Busy)
        ));
        drop(admitted);
        assert_eq!(
            guard.selection_snapshot().unwrap().model_preferences(),
            &selected
        );
        assert!(matches!(
            runtime.set_model_preferences(selected),
            Err(NativeConversationRuntimeError::Quiescing)
        ));
    }
}
