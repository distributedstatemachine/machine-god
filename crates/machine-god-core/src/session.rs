use crate::{
    BoxFuture, CancellationToken, Capability, ContentBlock, EngineError, EngineEvent,
    InferenceOptions, Message, ModelEvent, ModelEventStream, ModelRequest, PermissionDecision,
    PermissionRequest, PermissionRequestId, PermissionRisk, PreparedToolAuthorization,
    PreparedToolCall, Role, SessionId, SessionIncarnationId, SessionStoreError,
    SessionStoreErrorKind, StopReason, TokenUsage, ToolCall, ToolContext, ToolExecution, ToolName,
    ToolOutput, ToolSpec, TurnEvent, TurnId, TurnToolRegistration,
};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::{Future, poll_fn};
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::engine::{EngineInner, SessionRegistration, SessionRegistry};

/// Optimistic-concurrency revision assigned by a [`SessionStore`]. Zero is the
/// unsaved in-memory sentinel and is invalid in records returned by a store.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SessionRevision(pub u64);

/// Durable provider-neutral session state.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionRecord {
    pub id: SessionId,
    /// Globally unique identity for this logical lifetime of `id`.
    pub incarnation_id: SessionIncarnationId,
    pub revision: SessionRevision,
    /// First never-reserved turn sequence number. Reconciliation never permits
    /// this allocator position to decrease.
    #[serde(default = "initial_turn_sequence")]
    pub next_turn_sequence: u64,
    pub messages: Vec<Message>,
    pub metadata: BTreeMap<String, Value>,
}

impl SessionRecord {
    #[must_use]
    pub fn empty(id: SessionId, incarnation_id: SessionIncarnationId) -> Self {
        Self {
            id,
            incarnation_id,
            revision: SessionRevision::default(),
            next_turn_sequence: initial_turn_sequence(),
            messages: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }
}

pub(crate) fn validate_record_limits(
    record: &SessionRecord,
    limits: crate::EngineLimits,
) -> Result<(), EngineError> {
    validate_record(record, limits).map_err(|failure| EngineError::Protocol(failure.message))
}

pub(crate) fn redact_store_error(error: SessionStoreError) -> SessionStoreError {
    let SessionStoreError {
        kind,
        code,
        message,
        retryable,
    } = error;
    drop((code, message));
    SessionStoreError::new(kind, "store_failed", "session store failed", retryable)
}

fn redact_event_sink_error(error: crate::EventSinkError) -> crate::EventSinkError {
    let crate::EventSinkError { code, message } = error;
    drop((code, message));
    crate::EventSinkError::new("event_sink_failed", "event sink failed")
}

const fn initial_turn_sequence() -> u64 {
    1
}

/// Object-safe persistence boundary with optimistic concurrency.
///
/// A store must preserve [`SessionRecord::incarnation_id`] for the complete
/// logical lifetime of a record and reject a save that attempts to change it.
pub trait SessionStore: Send + Sync + 'static {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>>;

    /// Persists `record` only if its currently stored revision equals
    /// `expected_revision`. A new record uses `None`. A successful save must
    /// return a nonzero revision strictly greater than the record's previous
    /// revision.
    fn save(
        &self,
        record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>>;
}

/// User input and optional inference controls for one turn.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Prompt {
    pub text: String,
    pub options: InferenceOptions,
}

impl From<String> for Prompt {
    fn from(text: String) -> Self {
        Self {
            text,
            options: InferenceOptions::default(),
        }
    }
}

impl From<&str> for Prompt {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

pub(crate) trait DrainJsonValues {
    fn drain_json_values(&mut self);
}

pub(crate) struct JsonOwnerGuard<T: DrainJsonValues> {
    value: Option<T>,
}

impl<T: DrainJsonValues> JsonOwnerGuard<T> {
    pub(crate) fn new(value: T) -> Self {
        Self { value: Some(value) }
    }

    pub(crate) fn get(&self) -> &T {
        self.value.as_ref().expect("JSON owner guard is armed")
    }

    pub(crate) fn into_inner(mut self) -> T {
        self.value.take().expect("JSON owner guard is armed")
    }
}

impl<T: DrainJsonValues> Drop for JsonOwnerGuard<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.as_mut() {
            value.drain_json_values();
        }
    }
}

impl DrainJsonValues for InferenceOptions {
    fn drain_json_values(&mut self) {
        for value in std::mem::take(&mut self.metadata).into_values() {
            drop_json_value_iterative(value);
        }
    }
}

impl DrainJsonValues for Prompt {
    fn drain_json_values(&mut self) {
        self.options.drain_json_values();
    }
}

impl DrainJsonValues for SessionRecord {
    fn drain_json_values(&mut self) {
        for value in std::mem::take(&mut self.metadata).into_values() {
            drop_json_value_iterative(value);
        }
        for message in &mut self.messages {
            for block in &mut message.content {
                match block {
                    ContentBlock::Json { value } => {
                        drop_json_value_iterative(std::mem::take(value));
                    }
                    ContentBlock::ToolCall { call } => {
                        drop_json_value_iterative(std::mem::take(&mut call.arguments));
                    }
                    ContentBlock::ToolResult { output, .. } => {
                        drop_json_value_iterative(std::mem::take(&mut output.content));
                    }
                    ContentBlock::Text { .. } => {}
                }
            }
        }
    }
}

impl DrainJsonValues for ModelEvent {
    fn drain_json_values(&mut self) {
        if let ModelEvent::ToolCall { call } = self {
            drop_json_value_iterative(std::mem::take(&mut call.arguments));
        }
    }
}

impl DrainJsonValues for ToolOutput {
    fn drain_json_values(&mut self) {
        drop_json_value_iterative(std::mem::take(&mut self.content));
    }
}

impl DrainJsonValues for ToolExecution {
    fn drain_json_values(&mut self) {
        self.drain_owned_json();
    }
}

/// A clonable handle to a provider-neutral session.
#[derive(Clone)]
pub struct Session {
    pub(crate) engine: Arc<EngineInner>,
    state: Arc<SessionState>,
}

pub(crate) struct SessionState {
    data: Mutex<SessionData>,
    active_turn: AtomicBool,
    _registry_membership: SessionRegistration,
}

struct SessionData {
    record: Arc<SessionRecord>,
    persisted: bool,
}

impl SessionState {
    pub(crate) fn new_registered(
        record: SessionRecord,
        persisted: bool,
        registry: std::sync::Weak<SessionRegistry>,
    ) -> Arc<Self> {
        let id = record.id.clone();
        Arc::new_cyclic(move |state| Self {
            data: Mutex::new(SessionData {
                record: Arc::new(record),
                persisted,
            }),
            active_turn: AtomicBool::new(false),
            _registry_membership: SessionRegistration::new(registry, id, state.clone()),
        })
    }

    pub(crate) fn validate_loaded(record: &SessionRecord) -> Result<(), EngineError> {
        if record.revision == SessionRevision(0) {
            return Err(EngineError::Protocol(
                "stored session revision must be positive".to_owned(),
            ));
        }
        if record.next_turn_sequence == 0 {
            return Err(EngineError::Protocol(
                "stored next turn sequence must be positive".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_identity(&self, record: &SessionRecord) -> Result<(), EngineError> {
        let canonical = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record
            .clone();
        Self::validate_record_identity(&canonical, record)
    }

    pub(crate) fn reconcile_loaded(&self, record: SessionRecord) -> Result<(), EngineError> {
        const MAX_RECONCILE_RETRIES: usize = 32;

        Self::validate_loaded(&record)?;
        let record = Arc::new(record);
        for _ in 0..MAX_RECONCILE_RETRIES {
            let (canonical, _) = self.snapshot();
            Self::validate_record_identity(&canonical, &record)?;
            Self::validate_sequence_progress(&canonical, &record)?;
            if record.revision < canonical.revision {
                return Err(EngineError::Protocol(format!(
                    "session store returned stale revision {} behind canonical revision {}",
                    record.revision.0, canonical.revision.0
                )));
            }
            let equal_revision_diverged =
                record.revision == canonical.revision && *record != *canonical;
            if equal_revision_diverged {
                return Err(EngineError::Protocol(
                    "session store returned different records at the same revision".to_owned(),
                ));
            }

            let mut data = self
                .data
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !Arc::ptr_eq(&data.record, &canonical) {
                drop(data);
                continue;
            }
            if record.revision > canonical.revision {
                data.record = Arc::clone(&record);
            }
            data.persisted = true;
            return Ok(());
        }
        Err(EngineError::Protocol(
            "session load reconciliation exceeded its retry bound".to_owned(),
        ))
    }

    fn reconcile_saved(&self, record: Arc<SessionRecord>) -> Result<(), EngineError> {
        const MAX_RECONCILE_RETRIES: usize = 32;

        for _ in 0..MAX_RECONCILE_RETRIES {
            let (canonical, _) = self.snapshot();
            Self::validate_record_identity(&canonical, &record)?;
            Self::validate_sequence_progress(&canonical, &record)?;
            let equal_revision_diverged =
                canonical.revision == record.revision && *canonical != *record;
            if equal_revision_diverged {
                return Err(EngineError::Protocol(
                    "successful save diverged from canonical record at the same revision"
                        .to_owned(),
                ));
            }

            let mut data = self
                .data
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !Arc::ptr_eq(&data.record, &canonical) {
                drop(data);
                continue;
            }
            if canonical.revision < record.revision {
                data.record = record;
                data.persisted = true;
            } else if canonical.revision == record.revision {
                data.persisted = true;
            }
            return Ok(());
        }
        Err(EngineError::Protocol(
            "session save reconciliation exceeded its retry bound".to_owned(),
        ))
    }

    fn snapshot(&self) -> (Arc<SessionRecord>, bool) {
        let data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (Arc::clone(&data.record), data.persisted)
    }

    fn snapshot_is_current(&self, record: &Arc<SessionRecord>, persisted: bool) -> bool {
        let data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::ptr_eq(&data.record, record) && data.persisted == persisted
    }

    fn mark_missing_if_current(&self, record: &Arc<SessionRecord>, persisted: bool) {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if Arc::ptr_eq(&data.record, record) && data.persisted == persisted {
            data.persisted = false;
        }
    }

    fn validate_sequence_progress(
        canonical: &SessionRecord,
        candidate: &SessionRecord,
    ) -> Result<(), EngineError> {
        if candidate.next_turn_sequence < canonical.next_turn_sequence {
            return Err(EngineError::Protocol(format!(
                "session turn sequence regressed from {} to {}",
                canonical.next_turn_sequence, candidate.next_turn_sequence
            )));
        }
        Ok(())
    }

    fn validate_record_identity(
        canonical: &SessionRecord,
        candidate: &SessionRecord,
    ) -> Result<(), EngineError> {
        if candidate.id != canonical.id {
            return Err(EngineError::Protocol(format!(
                "session store returned ID {} for requested ID {}",
                candidate.id, canonical.id
            )));
        }
        if candidate.incarnation_id != canonical.incarnation_id {
            return Err(EngineError::SessionIncarnationConflict);
        }
        Ok(())
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("id", &self.id())
            .field("has_active_turn", &self.has_active_turn())
            .finish_non_exhaustive()
    }
}

impl Session {
    pub(crate) fn from_state(engine: Arc<EngineInner>, state: Arc<SessionState>) -> Self {
        Self { engine, state }
    }

    #[must_use]
    pub fn id(&self) -> SessionId {
        self.state
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record
            .id
            .clone()
    }

    /// Returns the durable identity of this logical session lifetime.
    #[must_use]
    pub fn incarnation_id(&self) -> SessionIncarnationId {
        self.state
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record
            .incarnation_id
            .clone()
    }

    /// Returns a consistent snapshot of in-memory session state.
    #[must_use]
    pub fn record(&self) -> SessionRecord {
        let (record, _) = self.state.snapshot();
        (*record).clone()
    }

    #[must_use]
    pub fn has_active_turn(&self) -> bool {
        self.state.active_turn.load(Ordering::Acquire)
    }

    /// Atomically reserves a durable turn ID and user message, then creates a
    /// bounded multi-round provider/tool stream. A session permits at most one
    /// live turn.
    ///
    /// Dropping a live returned stream requests shared cancellation before
    /// releasing the session immediately. No provider, policy, store, or tool
    /// future is detached from the returned stream.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::SessionBusy`] when this session, or any clone of
    /// it, already owns a live turn. Store errors are returned if the durable
    /// turn sequence cannot be reserved.
    #[must_use]
    pub fn prompt(
        &self,
        prompt: impl Into<Prompt>,
    ) -> BoxFuture<'static, Result<Turn, EngineError>> {
        let session = self.clone();
        let prompt = JsonOwnerGuard::new(prompt.into());
        Box::pin(async move { session.start_prompt(prompt).await })
    }

    async fn start_prompt(&self, prompt: JsonOwnerGuard<Prompt>) -> Result<Turn, EngineError> {
        if prompt.get().text.len() > self.engine.limits.max_prompt_bytes.get() {
            return Err(EngineError::Protocol(
                "prompt exceeded the configured byte limit".to_owned(),
            ));
        }
        validate_inference_options(&prompt.get().options, self.engine.limits)
            .map_err(|failure| EngineError::Protocol(failure.message))?;
        self.state
            .active_turn
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| EngineError::SessionBusy)?;
        let lease = TurnLease {
            state: Arc::clone(&self.state),
        };

        let prompt = prompt.into_inner();
        let (turn_id, record) = self.reserve_turn_and_prompt(prompt.text).await?;
        let session_id = record.id.clone();
        let session_incarnation_id = record.incarnation_id.clone();
        let cancellation = CancellationToken::new();
        let gate = EmissionGate::default();
        let workflow = Box::pin(run_turn(
            Arc::clone(&self.engine),
            Arc::clone(&self.state),
            session_id.clone(),
            turn_id.clone(),
            record,
            prompt.options,
            cancellation.clone(),
            gate.emitter(),
        ));

        Ok(Turn {
            session_id,
            session_incarnation_id,
            id: turn_id.clone(),
            handle: TurnHandle {
                turn_id,
                cancellation: cancellation.clone(),
            },
            cancellation,
            sink: Arc::clone(&self.engine.event_sink),
            state: TurnState::Running(workflow),
            gate,
            delivery: None,
            sequence: 0,
            usage: TokenUsage::default(),
            terminal_seen: false,
            locally_synthesized_cancellation: false,
            cancellation_waiter: None,
            lease: Some(lease),
        })
    }

    async fn reserve_turn_and_prompt(
        &self,
        prompt_text: String,
    ) -> Result<(TurnId, SessionRecord), EngineError> {
        const MAX_CONFLICT_RETRIES: usize = 32;

        for _ in 0..MAX_CONFLICT_RETRIES {
            let (snapshot, persisted) = self.state.snapshot();
            validate_record_limits(&snapshot, self.engine.limits)?;
            let expected_revision = persisted.then_some(snapshot.revision);
            let mut candidate = (*snapshot).clone();
            let turn_sequence = candidate.next_turn_sequence;
            candidate.next_turn_sequence = turn_sequence.checked_add(1).ok_or_else(|| {
                EngineError::Protocol("session turn sequence is exhausted".to_owned())
            })?;
            let turn_id = TurnId::new(format!("turn-{turn_sequence}"))
                .map_err(|error| EngineError::Protocol(error.to_string()))?;
            candidate
                .messages
                .push(Message::text(Role::User, &prompt_text));
            let candidate = JsonOwnerGuard::new(candidate);
            validate_record_limits(candidate.get(), self.engine.limits)?;
            if !self.state.snapshot_is_current(&snapshot, persisted) {
                continue;
            }
            let mut candidate = candidate.into_inner();
            let previous_revision = candidate.revision;

            match self
                .engine
                .session_store
                .save(candidate.clone(), expected_revision)
                .await
            {
                Ok(revision) if revision > previous_revision => {
                    candidate.revision = revision;
                    let candidate = Arc::new(candidate);
                    self.state.reconcile_saved(Arc::clone(&candidate))?;
                    return Ok((turn_id, (*candidate).clone()));
                }
                Ok(_) => {
                    return Err(EngineError::Protocol(
                        "session store returned a non-increasing revision".to_owned(),
                    ));
                }
                Err(error) if error.kind == SessionStoreErrorKind::Conflict => {
                    let id = candidate.id.clone();
                    let Some(current) = self
                        .engine
                        .session_store
                        .load(id.clone())
                        .await
                        .map_err(redact_store_error)?
                    else {
                        if snapshot.id != id {
                            return Err(EngineError::Protocol(
                                "session identity changed during turn reservation".to_owned(),
                            ));
                        }
                        self.state.mark_missing_if_current(&snapshot, persisted);
                        continue;
                    };
                    let current = JsonOwnerGuard::new(current);
                    if current.get().id != id {
                        return Err(EngineError::Protocol(format!(
                            "session store returned ID {} for requested ID {id}",
                            current.get().id
                        )));
                    }
                    if current.get().incarnation_id != snapshot.incarnation_id {
                        return Err(EngineError::SessionIncarnationConflict);
                    }
                    validate_record_limits(current.get(), self.engine.limits)?;
                    self.state.reconcile_loaded(current.into_inner())?;
                }
                Err(error) => return Err(redact_store_error(error).into()),
            }
        }
        Err(SessionStoreError::new(
            SessionStoreErrorKind::Conflict,
            "turn_reservation_contended",
            "turn reservation exceeded its conflict retry bound",
            true,
        )
        .into())
    }
}
type DeliveryFuture = BoxFuture<'static, Result<(), crate::EventSinkError>>;
type WorkflowFuture = BoxFuture<'static, WorkflowExit>;

enum TurnState {
    Running(WorkflowFuture),
    EmitTerminal(Box<TurnEvent>),
    Done,
}

#[derive(Debug)]
enum WorkflowExit {
    Completed {
        reason: StopReason,
        usage: TokenUsage,
    },
    Failed(TurnFailure),
    Cancelled,
}

#[derive(Debug)]
struct TurnFailure {
    component: String,
    code: String,
    message: String,
    retryable: bool,
}

impl TurnFailure {
    fn protocol(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            component: "protocol".to_owned(),
            code: code.to_owned(),
            message: message.into(),
            retryable: false,
        }
    }

    fn limit(code: &'static str, message: &'static str) -> Self {
        Self {
            component: "limits".to_owned(),
            code: code.to_owned(),
            message: message.to_owned(),
            retryable: false,
        }
    }

    fn provider(error: &crate::ProviderError) -> Self {
        Self {
            component: "provider".to_owned(),
            code: "provider_failed".to_owned(),
            message: "model provider failed".to_owned(),
            retryable: error.retryable,
        }
    }

    fn missing_stop() -> Self {
        Self {
            component: "provider".to_owned(),
            code: "missing_stop".to_owned(),
            message: "provider stream ended without a terminal stop".to_owned(),
            retryable: false,
        }
    }

    fn store(error: &SessionStoreError) -> Self {
        Self {
            component: "store".to_owned(),
            code: "store_failed".to_owned(),
            message: "session store failed".to_owned(),
            retryable: error.retryable,
        }
    }

    fn permission(_error: &crate::PermissionError) -> Self {
        Self {
            component: "permission".to_owned(),
            code: "permission_failed".to_owned(),
            message: "permission policy failed".to_owned(),
            retryable: false,
        }
    }

    fn into_event(self) -> TurnEvent {
        TurnEvent::Failed {
            component: self.component,
            code: self.code,
            message: self.message,
            retryable: self.retryable,
        }
    }
}

#[derive(Default)]
struct EmissionGate {
    inner: Arc<Mutex<EmissionState>>,
}

#[derive(Default)]
struct EmissionState {
    pending: Option<StagedTurnEvent>,
    acknowledged: bool,
    usage: TokenUsage,
    terminal_established: bool,
}

struct StagedTurnEvent {
    payload: TurnEvent,
    establishes_terminal: bool,
}

#[derive(Clone)]
struct TurnEmitter {
    inner: Arc<Mutex<EmissionState>>,
}

struct EmitFuture {
    inner: Arc<Mutex<EmissionState>>,
    event: Option<StagedTurnEvent>,
    staged: bool,
}

impl EmissionGate {
    fn emitter(&self) -> TurnEmitter {
        TurnEmitter {
            inner: Arc::clone(&self.inner),
        }
    }

    fn take_pending(&self) -> Option<StagedTurnEvent> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending
            .take()
    }

    fn acknowledge(&self) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .acknowledged = true;
    }

    fn clear(&self) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending = None;
        state.acknowledged = false;
        state.terminal_established = false;
    }

    fn usage(&self) -> TokenUsage {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .usage
    }

    fn terminal_established(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .terminal_established
    }
}

impl TurnEmitter {
    fn set_usage(&self, usage: TokenUsage) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .usage = usage;
    }

    fn establish_terminal(&self) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .terminal_established = true;
    }

    async fn emit(&self, payload: TurnEvent) {
        EmitFuture {
            inner: Arc::clone(&self.inner),
            event: Some(StagedTurnEvent {
                payload,
                establishes_terminal: false,
            }),
            staged: false,
        }
        .await;
    }

    async fn emit_established_terminal(&self, payload: TurnEvent) {
        EmitFuture {
            inner: Arc::clone(&self.inner),
            event: Some(StagedTurnEvent {
                payload,
                establishes_terminal: true,
            }),
            staged: false,
        }
        .await;
    }
}

impl Future for EmitFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if !this.staged {
            let event = this.event.take().expect("unstaged event is available");
            let mut state = this
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert!(state.pending.is_none(), "only one turn event may be staged");
            assert!(
                !state.acknowledged,
                "an acknowledgement cannot precede an event"
            );
            state.pending = Some(event);
            this.staged = true;
            return Poll::Pending;
        }
        let mut state = this
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.acknowledged {
            state.acknowledged = false;
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

enum WorkflowAbort {
    Cancelled,
    Failed(TurnFailure),
}

impl From<TurnFailure> for WorkflowAbort {
    fn from(failure: TurnFailure) -> Self {
        Self::Failed(failure)
    }
}

struct CompletedTurn {
    reason: StopReason,
    usage: TokenUsage,
}

#[derive(Default)]
struct TurnToolCatalog {
    tools: BTreeMap<ToolName, Arc<TurnToolRegistration>>,
}

impl TurnToolCatalog {
    fn model_specs(&self, engine: &EngineInner) -> Vec<ToolSpec> {
        let mut specs = Vec::with_capacity(
            engine
                .tool_specs_ref()
                .len()
                .saturating_add(self.tools.len()),
        );
        specs.extend(engine.tool_specs_ref().iter().cloned());
        specs.extend(
            self.tools
                .values()
                .map(|registration| registration.spec().clone()),
        );
        specs
    }

    fn tool(&self, engine: &EngineInner, name: &ToolName) -> Option<Arc<dyn crate::Tool>> {
        engine
            .tool(name)
            .or_else(|| self.tools.get(name).map(|registration| registration.tool()))
    }

    fn validate_registration(
        &self,
        engine: &EngineInner,
        registration: &TurnToolRegistration,
        limits: crate::EngineLimits,
    ) -> Result<bool, TurnFailure> {
        let spec = registration.spec();
        if engine.tool(&spec.name).is_some() {
            return Err(TurnFailure::protocol(
                "turn_tool_name_collision",
                "turn-local tool duplicated a statically registered tool name",
            ));
        }
        if let Some(existing) = self.tools.get(&spec.name) {
            if existing.spec() == spec {
                return Ok(false);
            }
            return Err(TurnFailure::protocol(
                "turn_tool_name_collision",
                "turn-local tool name was registered with a different specification",
            ));
        }

        validate_json_roots(
            engine
                .tool_specs_ref()
                .iter()
                .map(|spec| &spec.input_schema)
                .chain(
                    self.tools
                        .values()
                        .map(|registration| &registration.spec().input_schema),
                )
                .chain(std::iter::once(&spec.input_schema)),
            limits,
        )
        .map_err(json_limit_failure)?;

        let specs = engine
            .tool_specs_ref()
            .iter()
            .chain(self.tools.values().map(|registration| registration.spec()))
            .chain(std::iter::once(spec))
            .collect::<Vec<_>>();
        let serialized = serialized_json_size_bounded(&specs, limits.max_tool_catalog_bytes.get())
            .map_err(|error| {
                TurnFailure::protocol(
                    "turn_tool_catalog_serialization",
                    format!("turn-local tool catalog could not be serialized: {error}"),
                )
            })?;
        if serialized.is_none() {
            return Err(TurnFailure::limit(
                "tool_catalog_size_limit",
                "turn-local tool registration exceeded the configured catalog size limit",
            ));
        }
        Ok(true)
    }

    fn insert(&mut self, registration: Arc<TurnToolRegistration>) {
        let name = registration.spec().name.clone();
        let previous = self.tools.insert(name, registration);
        assert!(
            previous.is_none(),
            "validated turn tool insertion is unique"
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_turn(
    engine: Arc<EngineInner>,
    session_state: Arc<SessionState>,
    session_id: SessionId,
    turn_id: TurnId,
    record: SessionRecord,
    options: InferenceOptions,
    cancellation: CancellationToken,
    emitter: TurnEmitter,
) -> WorkflowExit {
    match run_turn_inner(
        engine,
        session_state,
        session_id,
        turn_id,
        record,
        options,
        cancellation,
        emitter,
    )
    .await
    {
        Ok(completed) => WorkflowExit::Completed {
            reason: completed.reason,
            usage: completed.usage,
        },
        Err(WorkflowAbort::Cancelled) => WorkflowExit::Cancelled,
        Err(WorkflowAbort::Failed(failure)) => WorkflowExit::Failed(failure),
    }
}

// Keeping the state transitions in one async frame makes cancellation checks
// and event acknowledgement boundaries directly auditable.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_turn_inner(
    engine: Arc<EngineInner>,
    session_state: Arc<SessionState>,
    session_id: SessionId,
    turn_id: TurnId,
    mut record: SessionRecord,
    options: InferenceOptions,
    cancellation: CancellationToken,
    emitter: TurnEmitter,
) -> Result<CompletedTurn, WorkflowAbort> {
    emitter.emit(TurnEvent::Started).await;

    let limits = engine.limits;
    let session_incarnation_id = record.incarnation_id.clone();
    let mut model_rounds = 0usize;
    let mut model_events = 0usize;
    let mut tool_calls = 0usize;
    let mut cumulative_tool_result_bytes = 0usize;
    let mut seen_call_ids = BTreeSet::new();
    let mut usage = TokenUsage::default();
    let mut assistant_bytes = 0usize;
    let mut reasoning_bytes = 0usize;
    let mut turn_tools = TurnToolCatalog::default();

    loop {
        check_cancelled(&cancellation)?;
        model_rounds = model_rounds.checked_add(1).ok_or_else(|| {
            TurnFailure::limit("model_round_limit", "model round count overflowed")
        })?;
        if model_rounds > limits.max_model_rounds.get() {
            return Err(TurnFailure::limit(
                "model_round_limit",
                "turn exceeded the configured model round limit",
            )
            .into());
        }

        validate_record(&record, limits)?;
        validate_inference_options(&options, limits)?;
        let request = ModelRequest {
            session_id: session_id.clone(),
            session_incarnation_id: session_incarnation_id.clone(),
            turn_id: turn_id.clone(),
            messages: record.messages.clone(),
            tools: turn_tools.model_specs(&engine),
            options: options.clone(),
        };
        check_cancelled(&cancellation)?;
        let provider_start = engine.provider.stream(request, cancellation.clone());
        let mut stream = await_cancellable(provider_start, &cancellation)
            .await?
            .map_err(|error| TurnFailure::provider(&error))?;

        let mut assistant_text = String::new();
        let mut calls = Vec::new();
        let mut round_call_ids = BTreeSet::new();
        let mut round_usage = TokenUsage::default();
        let stop_reason = loop {
            let item = next_model_item(&mut stream, &cancellation)
                .await?
                .map(|result| result.map(JsonOwnerGuard::new));
            if item.is_some() {
                model_events = model_events.checked_add(1).ok_or_else(|| {
                    TurnFailure::limit("model_event_limit", "model event count overflowed")
                })?;
                if model_events > limits.max_model_events_per_turn.get() {
                    return Err(TurnFailure::limit(
                        "model_event_limit",
                        "turn exceeded the configured model event limit",
                    )
                    .into());
                }
            }
            match item {
                Some(Ok(event)) if matches!(event.get(), ModelEvent::Stop { .. }) => {
                    let ModelEvent::Stop { reason } = event.into_inner() else {
                        unreachable!("stop event guard changed variant")
                    };
                    validate_stop_reason(&reason, limits)?;
                    break reason;
                }
                Some(Ok(event)) => {
                    match event.get() {
                        ModelEvent::TextDelta { text } => {
                            assistant_bytes =
                                assistant_bytes.checked_add(text.len()).ok_or_else(|| {
                                    TurnFailure::limit(
                                        "assistant_text_size_limit",
                                        "assistant text size overflowed",
                                    )
                                })?;
                            if assistant_bytes > limits.max_assistant_text_bytes.get() {
                                return Err(TurnFailure::limit(
                                    "assistant_text_size_limit",
                                    "turn exceeded the configured assistant text size limit",
                                )
                                .into());
                            }
                            assistant_text.push_str(text);
                        }
                        ModelEvent::ReasoningDelta { text } => {
                            reasoning_bytes =
                                reasoning_bytes.checked_add(text.len()).ok_or_else(|| {
                                    TurnFailure::limit(
                                        "reasoning_size_limit",
                                        "reasoning byte count overflowed",
                                    )
                                })?;
                            if reasoning_bytes > limits.max_reasoning_bytes.get() {
                                return Err(TurnFailure::limit(
                                    "reasoning_size_limit",
                                    "turn exceeded the configured reasoning size limit",
                                )
                                .into());
                            }
                        }
                        ModelEvent::ToolCall { call } => {
                            let round_count = calls.len().checked_add(1).ok_or_else(|| {
                                TurnFailure::limit(
                                    "tool_calls_per_round_limit",
                                    "model-round tool call count overflowed",
                                )
                            })?;
                            if round_count > limits.max_tool_calls_per_round.get() {
                                return Err(TurnFailure::limit(
                                    "tool_calls_per_round_limit",
                                    "model round exceeded the configured tool-call limit",
                                )
                                .into());
                            }
                            let turn_count =
                                tool_calls.checked_add(round_count).ok_or_else(|| {
                                    TurnFailure::limit(
                                        "tool_call_limit",
                                        "turn tool call count overflowed",
                                    )
                                })?;
                            if turn_count > limits.max_tool_calls_per_turn.get() {
                                return Err(TurnFailure::limit(
                                    "tool_call_limit",
                                    "turn exceeded the configured tool call limit",
                                )
                                .into());
                            }
                            if seen_call_ids.contains(&call.id)
                                || !round_call_ids.insert(call.id.clone())
                            {
                                return Err(TurnFailure::protocol(
                                    "duplicate_tool_call_id",
                                    format!("tool-call ID {} was repeated in one turn", call.id),
                                )
                                .into());
                            }
                            if turn_tools.tool(&engine, &call.name).is_none() {
                                return Err(TurnFailure::protocol(
                                    "unknown_tool",
                                    format!("provider requested unregistered tool {}", call.name),
                                )
                                .into());
                            }
                            validate_json_value(&call.arguments, limits)?;
                            let argument_bytes = serialized_json_size_bounded(
                                &call.arguments,
                                limits.max_tool_argument_bytes.get(),
                            )
                            .map_err(|error| {
                                TurnFailure::protocol(
                                    "tool_argument_serialization",
                                    format!("tool arguments could not be serialized: {error}"),
                                )
                            })?;
                            if argument_bytes.is_none() {
                                return Err(TurnFailure::limit(
                                    "tool_argument_size_limit",
                                    "tool arguments exceeded the configured serialized size limit",
                                )
                                .into());
                            }
                            calls.push(call.clone());
                        }
                        ModelEvent::Usage { usage: reported } => {
                            round_usage = *reported;
                            emitter.set_usage(checked_usage_add(usage, round_usage)?);
                        }
                        ModelEvent::Stop { .. } => unreachable!("stop handled above"),
                    }
                    emitter
                        .emit(TurnEvent::Model {
                            event: event.into_inner(),
                        })
                        .await;
                }
                Some(Err(error)) => return Err(TurnFailure::provider(&error).into()),
                None => {
                    return Err(TurnFailure::missing_stop().into());
                }
            }
        };

        usage = checked_usage_add(usage, round_usage)?;
        validate_round_stop(&calls, &stop_reason)?;

        if calls.is_empty() {
            let assistant = Message {
                role: Role::Assistant,
                content: assistant_message_content(&assistant_text, &calls),
            };
            let _committed = commit_message(
                &engine,
                &session_state,
                &record,
                assistant,
                &cancellation,
                CommitCancellation::FinalSaveSuccessWins,
            )
            .await?;
            emitter.establish_terminal();
            emitter
                .emit_established_terminal(TurnEvent::Model {
                    event: ModelEvent::Stop {
                        reason: stop_reason.clone(),
                    },
                })
                .await;
            return Ok(CompletedTurn {
                reason: stop_reason,
                usage,
            });
        }

        let new_total = tool_calls
            .checked_add(calls.len())
            .ok_or_else(|| TurnFailure::limit("tool_call_limit", "tool call count overflowed"))?;
        if new_total > limits.max_tool_calls_per_turn.get() {
            return Err(TurnFailure::limit(
                "tool_call_limit",
                "turn exceeded the configured tool call limit",
            )
            .into());
        }
        for call in &calls {
            seen_call_ids.insert(call.id.clone());
        }

        let placeholder = unknown_tool_result();
        let Some(placeholder_bytes) = serialized_json_size_bounded(
            &placeholder,
            limits.max_serialized_tool_result_bytes.get(),
        )
        .map_err(|_| {
            TurnFailure::protocol(
                "tool_result_serialization",
                "tool result placeholder could not be serialized",
            )
        })?
        else {
            return Err(TurnFailure::limit(
                "tool_result_size_limit",
                "configured result limit cannot hold the required result placeholder",
            )
            .into());
        };
        let placeholder_total = placeholder_bytes.checked_mul(calls.len()).ok_or_else(|| {
            TurnFailure::limit(
                "cumulative_tool_result_size_limit",
                "placeholder result byte count overflowed",
            )
        })?;
        let placeholder_cumulative = cumulative_tool_result_bytes
            .checked_add(placeholder_total)
            .ok_or_else(|| {
                TurnFailure::limit(
                    "cumulative_tool_result_size_limit",
                    "cumulative placeholder result byte count overflowed",
                )
            })?;
        if placeholder_cumulative > limits.max_cumulative_tool_result_bytes.get() {
            return Err(TurnFailure::limit(
                "cumulative_tool_result_size_limit",
                "configured cumulative result limit cannot hold required result placeholders",
            )
            .into());
        }

        emitter
            .emit(TurnEvent::Model {
                event: ModelEvent::Stop {
                    reason: stop_reason,
                },
            })
            .await;
        let placeholder_start = record.messages.len().checked_add(1).ok_or_else(|| {
            TurnFailure::limit(
                "transcript_message_limit",
                "placeholder message index overflowed",
            )
        })?;
        let mut round_messages = Vec::with_capacity(calls.len().saturating_add(1));
        round_messages.push(Message {
            role: Role::Assistant,
            content: assistant_message_content(&assistant_text, &calls),
        });
        round_messages.extend(
            calls
                .iter()
                .map(|call| tool_result_message(call.id.clone(), placeholder.clone())),
        );
        record = commit_messages(
            &engine,
            &session_state,
            &record,
            round_messages,
            &cancellation,
            CommitCancellation::Cancellable,
        )
        .await?;
        cumulative_tool_result_bytes = placeholder_cumulative;

        for (round_index, call) in calls.into_iter().enumerate() {
            check_cancelled(&cancellation)?;
            let call_id = call.id.clone();
            let call_name = call.name.clone();
            let tool = turn_tools.tool(&engine, &call_name).ok_or_else(|| {
                TurnFailure::protocol(
                    "unknown_tool",
                    format!("tool {call_name} disappeared after round validation"),
                )
            })?;
            let preparation = tool.prepare(call.clone());
            check_cancelled(&cancellation)?;

            let (output, next_round_tool, emit_finished) = match preparation {
                Err(error) => (tool_error_output(&error), None, false),
                Ok(prepared) => {
                    validate_prepared_tool_call(&prepared, limits)?;
                    let denied = match prepared.authorization() {
                        PreparedToolAuthorization::NoAuthorityRequired => false,
                        PreparedToolAuthorization::PermissionRequired(capability) => {
                            let ordinal = tool_calls
                                .checked_add(round_index)
                                .and_then(|index| index.checked_add(1))
                                .ok_or_else(|| {
                                    TurnFailure::limit(
                                        "tool_call_limit",
                                        "tool call count overflowed",
                                    )
                                })?;
                            let permission_id = permission_request_id(
                                &session_id,
                                &session_incarnation_id,
                                &turn_id,
                                ordinal,
                            )?;
                            let request = PermissionRequest {
                                id: permission_id.clone(),
                                session_id: session_id.clone(),
                                session_incarnation_id: session_incarnation_id.clone(),
                                turn_id: turn_id.clone(),
                                capability: capability.clone(),
                                risk: PermissionRisk::Critical,
                                reason: "model requested this registered tool".to_owned(),
                            };
                            emitter
                                .emit(TurnEvent::PermissionRequested {
                                    request: request.clone(),
                                })
                                .await;
                            let authorization = engine.permission_handler.authorize(request);
                            let decision = await_cancellable(authorization, &cancellation)
                                .await?
                                .map_err(|error| TurnFailure::permission(&error))?;
                            let decision = match decision {
                                PermissionDecision::Allow { scope } => {
                                    PermissionDecision::Allow { scope }
                                }
                                PermissionDecision::Deny { reason } => PermissionDecision::Deny {
                                    reason: bounded_text(
                                        &reason,
                                        limits.max_permission_denial_reason_bytes.get(),
                                    ),
                                },
                            };
                            emitter
                                .emit(TurnEvent::PermissionResolved {
                                    request_id: permission_id,
                                    decision: decision.clone(),
                                })
                                .await;
                            matches!(decision, PermissionDecision::Deny { .. })
                        }
                    };

                    if denied {
                        (
                            ToolOutput {
                                content: json!({
                                    "code": "permission_denied",
                                    "message": "tool execution was denied by policy",
                                }),
                                is_error: true,
                            },
                            None,
                            false,
                        )
                    } else {
                        emitter
                            .emit(TurnEvent::ToolStarted { call: call.clone() })
                            .await;
                        let execution = tool.execute_for_turn(
                            ToolContext {
                                session_id: session_id.clone(),
                                session_incarnation_id: session_incarnation_id.clone(),
                                turn_id: turn_id.clone(),
                                call_id: call_id.clone(),
                            },
                            prepared.into_arguments(),
                            cancellation.clone(),
                        );
                        let result = await_tool_execution(execution, &cancellation).await?;
                        let (output, next_round_tool) = match result {
                            Ok(execution) => execution.into_parts(),
                            Err(error) => (tool_error_output(&error), None),
                        };
                        (output, next_round_tool, true)
                    }
                }
            };
            let output = JsonOwnerGuard::new(output);

            if let Err(failure) = validate_json_value(&output.get().content, limits) {
                emitter.establish_terminal();
                return Err(failure.into());
            }

            let result_bytes = serialized_json_size_bounded(
                output.get(),
                limits.max_serialized_tool_result_bytes.get(),
            )
            .map_err(|error| {
                TurnFailure::protocol(
                    "tool_result_serialization",
                    format!("tool result could not be serialized: {error}"),
                )
            })?;
            let size_failure = match result_bytes {
                None => Some(TurnFailure::limit(
                    "tool_result_size_limit",
                    "tool result exceeded the configured serialized size limit",
                )),
                Some(result_bytes) => match cumulative_tool_result_bytes
                    .checked_sub(placeholder_bytes)
                    .and_then(|without_placeholder| without_placeholder.checked_add(result_bytes))
                {
                    Some(total) if total <= limits.max_cumulative_tool_result_bytes.get() => {
                        cumulative_tool_result_bytes = total;
                        None
                    }
                    _ => Some(TurnFailure::limit(
                        "cumulative_tool_result_size_limit",
                        "turn exceeded the cumulative tool result size limit",
                    )),
                },
            };
            if let Some(failure) = size_failure {
                emitter.establish_terminal();
                return Err(failure.into());
            }

            let insert_next_round_tool = if let Some(registration) = next_round_tool.as_ref() {
                if output.get().is_error {
                    emitter.establish_terminal();
                    return Err(TurnFailure::protocol(
                        "turn_tool_registration_on_error",
                        "an error tool output cannot register a turn-local tool",
                    )
                    .into());
                }
                turn_tools.validate_registration(&engine, registration, limits)?
            } else {
                false
            };

            let placeholder_index =
                placeholder_start.checked_add(round_index).ok_or_else(|| {
                    TurnFailure::limit(
                        "transcript_message_limit",
                        "placeholder message index overflowed",
                    )
                })?;
            record = replace_message(
                &engine,
                &session_state,
                &record,
                placeholder_index,
                tool_result_message(call_id.clone(), output.get().clone()),
                &cancellation,
                true,
            )
            .await?;
            if insert_next_round_tool {
                turn_tools
                    .insert(next_round_tool.expect("validated turn-local registration is present"));
            }
            if emit_finished {
                emitter
                    .emit(TurnEvent::ToolFinished {
                        call_id,
                        output: output.into_inner(),
                    })
                    .await;
            }
        }
        tool_calls = new_total;
    }
}

fn unknown_tool_result() -> ToolOutput {
    ToolOutput {
        content: json!({
            "code": "tool_result_unknown",
            "message": "tool result status is unknown",
        }),
        is_error: true,
    }
}

fn tool_error_output(error: &crate::ToolError) -> ToolOutput {
    ToolOutput {
        content: json!({
            "code": "tool_error",
            "message": "tool execution failed",
            "retryable": error.retryable,
        }),
        is_error: true,
    }
}

fn tool_result_message(call_id: crate::ToolCallId, output: ToolOutput) -> Message {
    Message {
        role: Role::Tool,
        content: vec![ContentBlock::ToolResult { call_id, output }],
    }
}

fn permission_request_id(
    session_id: &SessionId,
    session_incarnation_id: &SessionIncarnationId,
    turn_id: &TurnId,
    ordinal: usize,
) -> Result<PermissionRequestId, WorkflowAbort> {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut hasher = Sha256::new();
    hasher.update(b"machine-god:permission-request:v2\0");
    for component in [
        session_id.as_str(),
        session_incarnation_id.as_str(),
        turn_id.as_str(),
    ] {
        let length = u64::try_from(component.len()).map_err(|_| {
            TurnFailure::protocol("permission_id", "permission identity length overflowed")
        })?;
        hasher.update(length.to_be_bytes());
        hasher.update(component.as_bytes());
    }
    let ordinal = u64::try_from(ordinal)
        .map_err(|_| TurnFailure::protocol("permission_id", "permission ordinal overflowed"))?;
    hasher.update(ordinal.to_be_bytes());

    let digest = hasher.finalize();
    let mut value = String::with_capacity("permission-sha256-".len() + digest.len() * 2);
    value.push_str("permission-sha256-");
    for byte in digest {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    PermissionRequestId::new(value)
        .map_err(|error| TurnFailure::protocol("permission_id", error.to_string()).into())
}

fn assistant_message_content(text: &str, calls: &[ToolCall]) -> Vec<ContentBlock> {
    let mut content = Vec::with_capacity(usize::from(!text.is_empty()) + calls.len());
    if !text.is_empty() {
        content.push(ContentBlock::Text {
            text: text.to_owned(),
        });
    }
    content.extend(
        calls
            .iter()
            .cloned()
            .map(|call| ContentBlock::ToolCall { call }),
    );
    content
}

struct JsonByteCounter {
    bytes: usize,
    limit: usize,
    exceeded: bool,
}

enum JsonChildren<'a> {
    Array(std::slice::Iter<'a, Value>),
    Object(serde_json::map::Values<'a>),
}

impl<'a> Iterator for JsonChildren<'a> {
    type Item = &'a Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Array(children) => children.next(),
            Self::Object(children) => children.next(),
        }
    }
}

struct JsonFrame<'a> {
    container_depth: usize,
    children: JsonChildren<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JsonLimitViolation {
    Depth,
    Nodes,
}

struct JsonValidationBudget {
    nodes: usize,
    max_nodes: usize,
    max_container_depth: usize,
}

impl JsonValidationBudget {
    fn new(limits: crate::EngineLimits) -> Self {
        Self {
            nodes: 0,
            max_nodes: limits.max_json_nodes.get(),
            max_container_depth: limits.max_json_depth.get(),
        }
    }

    fn validate(&mut self, root: &Value) -> Result<(), JsonLimitViolation> {
        let mut frames = Vec::<JsonFrame<'_>>::new();
        let mut current = Some((root, 0usize));

        loop {
            if let Some((value, parent_depth)) = current.take() {
                self.nodes = self.nodes.checked_add(1).ok_or(JsonLimitViolation::Nodes)?;
                if self.nodes > self.max_nodes {
                    return Err(JsonLimitViolation::Nodes);
                }

                let children = match value {
                    Value::Array(values) => Some(JsonChildren::Array(values.iter())),
                    Value::Object(values) => Some(JsonChildren::Object(values.values())),
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
                };
                if let Some(children) = children {
                    let container_depth = parent_depth
                        .checked_add(1)
                        .ok_or(JsonLimitViolation::Depth)?;
                    if container_depth > self.max_container_depth {
                        return Err(JsonLimitViolation::Depth);
                    }
                    frames.push(JsonFrame {
                        container_depth,
                        children,
                    });
                }
            }

            loop {
                let Some(frame) = frames.last_mut() else {
                    return Ok(());
                };
                if let Some(child) = frame.children.next() {
                    current = Some((child, frame.container_depth));
                    break;
                }
                frames.pop();
            }
        }
    }
}

pub(crate) fn validate_json_roots<'a>(
    roots: impl IntoIterator<Item = &'a Value>,
    limits: crate::EngineLimits,
) -> Result<(), JsonLimitViolation> {
    let mut budget = JsonValidationBudget::new(limits);
    for root in roots {
        budget.validate(root)?;
    }
    Ok(())
}

enum OwnedJsonChildren {
    Array(std::vec::IntoIter<Value>),
    Object(serde_json::map::IntoValues),
}

impl Iterator for OwnedJsonChildren {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Array(children) => children.next(),
            Self::Object(children) => children.next(),
        }
    }
}

/// Reclaims a JSON tree without recursive `Value::drop` calls.
pub(crate) fn drop_json_value_iterative(root: Value) {
    let mut frames = Vec::<OwnedJsonChildren>::new();
    let mut current = Some(root);

    loop {
        if let Some(value) = current.take() {
            match value {
                Value::Array(values) => frames.push(OwnedJsonChildren::Array(values.into_iter())),
                Value::Object(values) => {
                    frames.push(OwnedJsonChildren::Object(values.into_values()));
                }
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }

        loop {
            let Some(frame) = frames.last_mut() else {
                return;
            };
            if let Some(child) = frame.next() {
                current = Some(child);
                break;
            }
            frames.pop();
        }
    }
}

fn json_limit_failure(violation: JsonLimitViolation) -> TurnFailure {
    match violation {
        JsonLimitViolation::Depth => TurnFailure::limit(
            "json_depth_limit",
            "JSON value exceeded the configured container depth limit",
        ),
        JsonLimitViolation::Nodes => TurnFailure::limit(
            "json_node_limit",
            "JSON values exceeded the configured node limit",
        ),
    }
}

fn validate_json_value(value: &Value, limits: crate::EngineLimits) -> Result<(), TurnFailure> {
    validate_json_roots(std::iter::once(value), limits).map_err(json_limit_failure)
}

// A serialized capability adds enum tags, field names, and validated IDs
// around its operation data. Reserving a fixed 1 KiB envelope preserves the
// legacy default at the exact raw-argument boundary without granting prepared
// execution arguments one additional byte.
const PREPARED_CAPABILITY_ENVELOPE_BYTES: usize = 1024;

fn validate_prepared_tool_call(
    prepared: &PreparedToolCall,
    limits: crate::EngineLimits,
) -> Result<(), TurnFailure> {
    validate_json_value(prepared.arguments(), limits)?;
    let prepared_argument_bytes =
        serialized_json_size_bounded(prepared.arguments(), limits.max_tool_argument_bytes.get())
            .map_err(|error| {
                TurnFailure::protocol(
                    "tool_argument_serialization",
                    format!("prepared tool arguments could not be serialized: {error}"),
                )
            })?;
    if prepared_argument_bytes.is_none() {
        return Err(TurnFailure::limit(
            "tool_argument_size_limit",
            "prepared tool arguments exceeded the configured serialized size limit",
        ));
    }

    if let PreparedToolAuthorization::PermissionRequired(capability) = prepared.authorization() {
        if let Some(value) = capability_json_value(capability) {
            validate_json_value(value, limits)?;
        }
        // Saturation keeps the derived bound representable even for a
        // host-supplied `usize::MAX` argument limit.
        let capability_limit = limits
            .max_tool_argument_bytes
            .get()
            .saturating_add(PREPARED_CAPABILITY_ENVELOPE_BYTES);
        let capability_bytes =
            serialized_json_size_bounded(capability, capability_limit).map_err(|error| {
                TurnFailure::protocol(
                    "tool_argument_serialization",
                    format!("prepared tool capability could not be serialized: {error}"),
                )
            })?;
        if capability_bytes.is_none() {
            return Err(TurnFailure::limit(
                "tool_argument_size_limit",
                "prepared tool capability exceeded the configured serialized size limit",
            ));
        }
    }
    Ok(())
}

fn capability_json_value(capability: &Capability) -> Option<&Value> {
    match capability {
        Capability::Tool { arguments, .. } => Some(arguments),
        Capability::Custom { details, .. } => Some(details),
        Capability::Filesystem { .. }
        | Capability::FilesystemRename { .. }
        | Capability::FilesystemCopy { .. }
        | Capability::OpenFile { .. }
        | Capability::Process { .. }
        | Capability::Network { .. }
        | Capability::Vision { .. } => None,
    }
}

impl Write for JsonByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let next = self
            .bytes
            .checked_add(buffer.len())
            .ok_or_else(|| io::Error::other("serialized JSON byte count overflowed"))?;
        if next > self.limit {
            self.exceeded = true;
            return Err(io::Error::other("serialized JSON exceeded its byte limit"));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn serialized_json_size_bounded<T: Serialize + ?Sized>(
    value: &T,
    limit: usize,
) -> Result<Option<usize>, serde_json::Error> {
    let mut counter = JsonByteCounter {
        bytes: 0,
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut counter, value);
    if counter.exceeded {
        Ok(None)
    } else {
        result?;
        Ok(Some(counter.bytes))
    }
}

fn validate_transcript(
    messages: &[Message],
    limits: crate::EngineLimits,
) -> Result<(), TurnFailure> {
    if messages.len() > limits.max_transcript_messages.get() {
        return Err(TurnFailure::limit(
            "transcript_message_limit",
            "transcript exceeded the configured message limit",
        ));
    }
    let bytes = serialized_json_size_bounded(messages, limits.max_transcript_bytes.get()).map_err(
        |_| {
            TurnFailure::protocol(
                "transcript_serialization",
                "transcript could not be serialized",
            )
        },
    )?;
    if bytes.is_none() {
        return Err(TurnFailure::limit(
            "transcript_size_limit",
            "transcript exceeded the configured serialized byte limit",
        ));
    }
    Ok(())
}

fn validate_record(record: &SessionRecord, limits: crate::EngineLimits) -> Result<(), TurnFailure> {
    let mut json_budget = JsonValidationBudget::new(limits);
    for value in record.metadata.values() {
        json_budget.validate(value).map_err(json_limit_failure)?;
    }
    for message in &record.messages {
        for block in &message.content {
            match block {
                ContentBlock::Json { value } => {
                    json_budget.validate(value).map_err(json_limit_failure)?;
                }
                ContentBlock::ToolCall { call } => {
                    json_budget
                        .validate(&call.arguments)
                        .map_err(json_limit_failure)?;
                }
                ContentBlock::ToolResult { output, .. } => {
                    json_budget
                        .validate(&output.content)
                        .map_err(json_limit_failure)?;
                }
                ContentBlock::Text { .. } => {}
            }
        }
    }
    validate_transcript(&record.messages, limits)?;
    let metadata_bytes =
        serialized_json_size_bounded(&record.metadata, limits.max_session_metadata_bytes.get())
            .map_err(|_| {
                TurnFailure::protocol(
                    "session_metadata_serialization",
                    "session metadata could not be serialized",
                )
            })?;
    if metadata_bytes.is_none() {
        return Err(TurnFailure::limit(
            "session_metadata_size_limit",
            "session metadata exceeded the configured serialized byte limit",
        ));
    }
    Ok(())
}

fn validate_inference_options(
    options: &InferenceOptions,
    limits: crate::EngineLimits,
) -> Result<(), TurnFailure> {
    let mut json_budget = JsonValidationBudget::new(limits);
    for value in options.metadata.values() {
        json_budget.validate(value).map_err(json_limit_failure)?;
    }
    let bytes = serialized_json_size_bounded(options, limits.max_inference_options_bytes.get())
        .map_err(|_| {
            TurnFailure::protocol(
                "inference_options_serialization",
                "inference options could not be serialized",
            )
        })?;
    if bytes.is_none() {
        return Err(TurnFailure::limit(
            "inference_options_size_limit",
            "inference options exceeded the configured serialized byte limit",
        ));
    }
    Ok(())
}

fn validate_round_stop(calls: &[ToolCall], stop_reason: &StopReason) -> Result<(), WorkflowAbort> {
    if calls.is_empty() && *stop_reason == StopReason::ToolCalls {
        return Err(TurnFailure::protocol(
            "tool_stop_without_calls",
            "tool-calls stop requires at least one tool call",
        )
        .into());
    }
    if !calls.is_empty() && *stop_reason != StopReason::ToolCalls {
        return Err(TurnFailure::protocol(
            "calls_with_incompatible_stop",
            "tool calls require a tool-calls stop reason",
        )
        .into());
    }
    Ok(())
}

fn validate_stop_reason(
    reason: &StopReason,
    limits: crate::EngineLimits,
) -> Result<(), WorkflowAbort> {
    if let StopReason::Other(detail) = reason
        && detail.len() > limits.max_stop_detail_bytes.get()
    {
        return Err(TurnFailure::limit(
            "stop_detail_size_limit",
            "provider stop detail exceeded the configured byte limit",
        )
        .into());
    }
    Ok(())
}

fn checked_usage_add(total: TokenUsage, round: TokenUsage) -> Result<TokenUsage, WorkflowAbort> {
    Ok(TokenUsage {
        input_tokens: total
            .input_tokens
            .checked_add(round.input_tokens)
            .ok_or_else(|| {
                TurnFailure::protocol("usage_overflow", "input token usage overflowed")
            })?,
        output_tokens: total
            .output_tokens
            .checked_add(round.output_tokens)
            .ok_or_else(|| {
                TurnFailure::protocol("usage_overflow", "output token usage overflowed")
            })?,
        cached_input_tokens: total
            .cached_input_tokens
            .checked_add(round.cached_input_tokens)
            .ok_or_else(|| {
                TurnFailure::protocol("usage_overflow", "cached input token usage overflowed")
            })?,
    })
}

fn bounded_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), WorkflowAbort> {
    if cancellation.is_cancelled() {
        Err(WorkflowAbort::Cancelled)
    } else {
        Ok(())
    }
}

async fn await_cancellable<T>(
    mut future: BoxFuture<'_, T>,
    cancellation: &CancellationToken,
) -> Result<T, WorkflowAbort> {
    poll_fn(|context| {
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(WorkflowAbort::Cancelled));
        }
        let result = future.as_mut().poll(context);
        if cancellation.is_cancelled() {
            Poll::Ready(Err(WorkflowAbort::Cancelled))
        } else {
            result.map(Ok)
        }
    })
    .await
}

async fn await_tool_execution(
    mut future: BoxFuture<'_, Result<ToolExecution, crate::ToolError>>,
    cancellation: &CancellationToken,
) -> Result<Result<ToolExecution, crate::ToolError>, WorkflowAbort> {
    poll_fn(|context| {
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(WorkflowAbort::Cancelled));
        }
        let result = future.as_mut().poll(context);
        if cancellation.is_cancelled() {
            if let Poll::Ready(Ok(mut execution)) = result {
                execution.drain_owned_json();
            }
            Poll::Ready(Err(WorkflowAbort::Cancelled))
        } else {
            result.map(Ok)
        }
    })
    .await
}

async fn await_operation<T>(
    future: BoxFuture<'_, T>,
    cancellation: &CancellationToken,
    honor_cancellation: bool,
) -> Result<T, WorkflowAbort> {
    if honor_cancellation {
        await_cancellable(future, cancellation).await
    } else {
        Ok(future.await)
    }
}

async fn await_final_save(
    store: &dyn crate::SessionStore,
    record: SessionRecord,
    expected_revision: Option<SessionRevision>,
    cancellation: &CancellationToken,
) -> Result<Result<SessionRevision, SessionStoreError>, WorkflowAbort> {
    check_cancelled(cancellation)?;
    let mut future = store.save(record, expected_revision);
    let mut first_poll = true;
    poll_fn(move |context| {
        if !first_poll && cancellation.is_cancelled() {
            return Poll::Ready(Err(WorkflowAbort::Cancelled));
        }
        first_poll = false;
        let result = future.as_mut().poll(context);
        match result {
            Poll::Ready(Ok(revision)) => Poll::Ready(Ok(Ok(revision))),
            Poll::Ready(Err(error)) if cancellation.is_cancelled() => {
                drop(error);
                Poll::Ready(Err(WorkflowAbort::Cancelled))
            }
            Poll::Pending if cancellation.is_cancelled() => {
                Poll::Ready(Err(WorkflowAbort::Cancelled))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Ok(Err(error))),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

async fn await_record_load(
    mut future: BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>>,
    cancellation: &CancellationToken,
    honor_cancellation: bool,
) -> Result<Result<Option<SessionRecord>, SessionStoreError>, WorkflowAbort> {
    if !honor_cancellation {
        return Ok(future.await);
    }
    poll_fn(|context| {
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(WorkflowAbort::Cancelled));
        }
        let result = future.as_mut().poll(context);
        if cancellation.is_cancelled() {
            if let Poll::Ready(Ok(Some(mut record))) = result {
                record.drain_json_values();
            }
            Poll::Ready(Err(WorkflowAbort::Cancelled))
        } else {
            result.map(Ok)
        }
    })
    .await
}

async fn next_model_item(
    stream: &mut ModelEventStream,
    cancellation: &CancellationToken,
) -> Result<Option<Result<ModelEvent, crate::ProviderError>>, WorkflowAbort> {
    poll_fn(|context| {
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(WorkflowAbort::Cancelled));
        }
        let result = stream.as_mut().poll_next(context);
        if cancellation.is_cancelled() {
            if let Poll::Ready(Some(Ok(mut event))) = result {
                event.drain_json_values();
            }
            Poll::Ready(Err(WorkflowAbort::Cancelled))
        } else {
            result.map(Ok)
        }
    })
    .await
}

async fn commit_message(
    engine: &Arc<EngineInner>,
    state: &Arc<SessionState>,
    base: &SessionRecord,
    message: Message,
    cancellation: &CancellationToken,
    cancellation_mode: CommitCancellation,
) -> Result<SessionRecord, WorkflowAbort> {
    commit_messages(
        engine,
        state,
        base,
        vec![message],
        cancellation,
        cancellation_mode,
    )
    .await
}

#[derive(Clone, Copy)]
enum CommitCancellation {
    Cancellable,
    FinalSaveSuccessWins,
}

#[allow(clippy::too_many_lines)]
async fn commit_messages(
    engine: &Arc<EngineInner>,
    state: &Arc<SessionState>,
    base: &SessionRecord,
    messages: Vec<Message>,
    cancellation: &CancellationToken,
    cancellation_mode: CommitCancellation,
) -> Result<SessionRecord, WorkflowAbort> {
    const MAX_CONFLICT_RETRIES: usize = 32;

    for _ in 0..MAX_CONFLICT_RETRIES {
        check_cancelled(cancellation)?;
        let (snapshot, persisted) = state.snapshot();
        if snapshot.id != base.id {
            return Err(TurnFailure::protocol(
                "session_identity_changed",
                "session identity changed during transcript commit",
            )
            .into());
        }
        if snapshot.incarnation_id != base.incarnation_id {
            return Err(TurnFailure::protocol(
                "session_incarnation_changed",
                "session incarnation changed during transcript commit",
            )
            .into());
        }
        if snapshot.messages != base.messages {
            return Err(TurnFailure::protocol(
                "transcript_diverged",
                "durable transcript diverged before message commit",
            )
            .into());
        }
        validate_record(&snapshot, engine.limits)?;
        let expected_revision = persisted.then_some(snapshot.revision);
        let mut candidate = (*snapshot).clone();
        candidate.messages.extend(messages.iter().cloned());
        let candidate = JsonOwnerGuard::new(candidate);
        validate_record(candidate.get(), engine.limits)?;
        if !state.snapshot_is_current(&snapshot, persisted) {
            continue;
        }
        let mut candidate = candidate.into_inner();
        let previous_revision = candidate.revision;
        let save_result = match cancellation_mode {
            CommitCancellation::Cancellable => {
                let save = engine
                    .session_store
                    .save(candidate.clone(), expected_revision);
                await_cancellable(save, cancellation).await?
            }
            CommitCancellation::FinalSaveSuccessWins => {
                await_final_save(
                    engine.session_store.as_ref(),
                    candidate.clone(),
                    expected_revision,
                    cancellation,
                )
                .await?
            }
        };
        match save_result {
            Ok(revision) if revision > previous_revision => {
                candidate.revision = revision;
                let candidate = Arc::new(candidate);
                state
                    .reconcile_saved(Arc::clone(&candidate))
                    .map_err(|error| {
                        TurnFailure::protocol("save_reconciliation", error.to_string())
                    })?;
                return Ok((*candidate).clone());
            }
            Ok(_) => {
                return Err(TurnFailure::protocol(
                    "non_increasing_revision",
                    "session store returned a non-increasing revision",
                )
                .into());
            }
            Err(error) if error.kind == SessionStoreErrorKind::Conflict => {
                let load = engine.session_store.load(base.id.clone());
                let Some(current) = await_record_load(load, cancellation, true)
                    .await?
                    .map_err(|error| TurnFailure::store(&error))?
                else {
                    return Err(TurnFailure::protocol(
                        "transcript_missing_after_conflict",
                        "durable transcript disappeared after a commit conflict",
                    )
                    .into());
                };
                let current = JsonOwnerGuard::new(current);
                if current.get().id != base.id {
                    return Err(TurnFailure::protocol(
                        "session_identity_changed",
                        format!(
                            "session store returned ID {} for requested ID {}",
                            current.get().id,
                            base.id
                        ),
                    )
                    .into());
                }
                if current.get().incarnation_id != base.incarnation_id {
                    return Err(TurnFailure::protocol(
                        "session_incarnation_changed",
                        "session incarnation changed during message commit",
                    )
                    .into());
                }
                SessionState::validate_loaded(current.get()).map_err(|error| {
                    TurnFailure::protocol("invalid_conflict_record", error.to_string())
                })?;
                validate_record(current.get(), engine.limits)?;
                if current.get().messages != base.messages {
                    return Err(TurnFailure::protocol(
                        "transcript_diverged",
                        "durable transcript diverged during message commit",
                    )
                    .into());
                }
                state
                    .reconcile_loaded(current.into_inner())
                    .map_err(|error| {
                        TurnFailure::protocol("conflict_reconciliation", error.to_string())
                    })?;
            }
            Err(error) => return Err(TurnFailure::store(&error).into()),
        }
    }
    let error = SessionStoreError::new(
        SessionStoreErrorKind::Conflict,
        "message_commit_contended",
        "message commit exceeded its conflict retry bound",
        true,
    );
    Err(TurnFailure::store(&error).into())
}

#[allow(clippy::too_many_lines)]
async fn replace_message(
    engine: &Arc<EngineInner>,
    state: &Arc<SessionState>,
    base: &SessionRecord,
    index: usize,
    replacement: Message,
    cancellation: &CancellationToken,
    honor_cancellation: bool,
) -> Result<SessionRecord, WorkflowAbort> {
    const MAX_CONFLICT_RETRIES: usize = 32;

    for _ in 0..MAX_CONFLICT_RETRIES {
        if honor_cancellation {
            check_cancelled(cancellation)?;
        }
        let (snapshot, persisted) = state.snapshot();
        if snapshot.id != base.id {
            return Err(TurnFailure::protocol(
                "session_identity_changed",
                "session identity changed during result replacement",
            )
            .into());
        }
        if snapshot.incarnation_id != base.incarnation_id {
            return Err(TurnFailure::protocol(
                "session_incarnation_changed",
                "session incarnation changed during result replacement",
            )
            .into());
        }
        if snapshot.messages != base.messages {
            return Err(TurnFailure::protocol(
                "transcript_diverged",
                "durable transcript diverged before result replacement",
            )
            .into());
        }
        validate_record(&snapshot, engine.limits)?;
        let expected_revision = persisted.then_some(snapshot.revision);
        let mut candidate = (*snapshot).clone();
        let Some(message) = candidate.messages.get_mut(index) else {
            return Err(TurnFailure::protocol(
                "placeholder_missing",
                "tool result placeholder index was missing",
            )
            .into());
        };
        *message = replacement.clone();
        let candidate = JsonOwnerGuard::new(candidate);
        validate_record(candidate.get(), engine.limits)?;
        if !state.snapshot_is_current(&snapshot, persisted) {
            continue;
        }
        let mut candidate = candidate.into_inner();
        let previous_revision = candidate.revision;
        let save = engine
            .session_store
            .save(candidate.clone(), expected_revision);
        match await_operation(save, cancellation, honor_cancellation).await? {
            Ok(revision) if revision > previous_revision => {
                candidate.revision = revision;
                let candidate = Arc::new(candidate);
                state
                    .reconcile_saved(Arc::clone(&candidate))
                    .map_err(|error| {
                        TurnFailure::protocol("save_reconciliation", error.to_string())
                    })?;
                return Ok((*candidate).clone());
            }
            Ok(_) => {
                return Err(TurnFailure::protocol(
                    "non_increasing_revision",
                    "session store returned a non-increasing revision",
                )
                .into());
            }
            Err(error) if error.kind == SessionStoreErrorKind::Conflict => {
                let load = engine.session_store.load(base.id.clone());
                let Some(current) = await_record_load(load, cancellation, honor_cancellation)
                    .await?
                    .map_err(|error| TurnFailure::store(&error))?
                else {
                    return Err(TurnFailure::protocol(
                        "transcript_missing_after_conflict",
                        "durable transcript disappeared after a result conflict",
                    )
                    .into());
                };
                let current = JsonOwnerGuard::new(current);
                if current.get().id != base.id {
                    return Err(TurnFailure::protocol(
                        "session_identity_changed",
                        "session store returned a different session during result replacement",
                    )
                    .into());
                }
                if current.get().incarnation_id != base.incarnation_id {
                    return Err(TurnFailure::protocol(
                        "session_incarnation_changed",
                        "session incarnation changed during result replacement",
                    )
                    .into());
                }
                SessionState::validate_loaded(current.get()).map_err(|error| {
                    TurnFailure::protocol("invalid_conflict_record", error.to_string())
                })?;
                validate_record(current.get(), engine.limits)?;
                if current.get().messages != base.messages {
                    return Err(TurnFailure::protocol(
                        "transcript_diverged",
                        "durable transcript diverged during result replacement",
                    )
                    .into());
                }
                state
                    .reconcile_loaded(current.into_inner())
                    .map_err(|error| {
                        TurnFailure::protocol("conflict_reconciliation", error.to_string())
                    })?;
            }
            Err(error) => return Err(TurnFailure::store(&error).into()),
        }
    }
    let error = SessionStoreError::new(
        SessionStoreErrorKind::Conflict,
        "message_replace_contended",
        "message replacement exceeded its conflict retry bound",
        true,
    );
    Err(TurnFailure::store(&error).into())
}

struct PendingDelivery {
    event: EngineEvent,
    future: DeliveryFuture,
}

struct TurnLease {
    state: Arc<SessionState>,
}

impl Drop for TurnLease {
    fn drop(&mut self) {
        self.state.active_turn.store(false, Ordering::Release);
    }
}

/// A clonable cancellation handle for a live turn.
#[derive(Clone, Debug)]
pub struct TurnHandle {
    turn_id: TurnId,
    cancellation: CancellationToken,
}

impl TurnHandle {
    #[must_use]
    pub fn id(&self) -> &TurnId {
        &self.turn_id
    }

    /// Requests cancellation. Returns `true` only for the first request.
    #[must_use]
    pub fn cancel(&self) -> bool {
        self.cancellation.cancel()
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

/// Ordered asynchronous events for one session turn.
pub struct Turn {
    session_id: SessionId,
    session_incarnation_id: SessionIncarnationId,
    id: TurnId,
    handle: TurnHandle,
    cancellation: CancellationToken,
    sink: Arc<dyn crate::EventSink>,
    state: TurnState,
    gate: EmissionGate,
    delivery: Option<PendingDelivery>,
    sequence: u64,
    usage: TokenUsage,
    terminal_seen: bool,
    locally_synthesized_cancellation: bool,
    cancellation_waiter: Option<u64>,
    lease: Option<TurnLease>,
}

impl fmt::Debug for Turn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Turn")
            .field("session_id", &self.session_id)
            .field("session_incarnation_id", &self.session_incarnation_id)
            .field("turn_id", &self.id)
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl Turn {
    #[must_use]
    pub fn id(&self) -> &TurnId {
        &self.id
    }

    #[must_use]
    pub fn handle(&self) -> TurnHandle {
        self.handle.clone()
    }

    fn finish(&mut self) {
        self.cancellation.deregister(&mut self.cancellation_waiter);
        self.state = TurnState::Done;
        self.gate.clear();
        self.lease.take();
    }

    fn fail_before_terminal(&mut self) {
        if !self.terminal_seen {
            let _ = self.cancellation.cancel();
        }
        self.terminal_seen = true;
        self.finish();
    }

    fn stage(&mut self, payload: TurnEvent) {
        let event = EngineEvent {
            session_id: self.session_id.clone(),
            session_incarnation_id: self.session_incarnation_id.clone(),
            turn_id: self.id.clone(),
            sequence: self.sequence,
            payload,
        };
        self.sequence = self.sequence.saturating_add(1);
        let sink = Arc::clone(&self.sink);
        let observed = event.clone();
        self.delivery = Some(PendingDelivery {
            event,
            future: Box::pin(async move { sink.emit(observed).await }),
        });
    }

    fn establish_cancellation(&mut self) {
        self.terminal_seen = true;
        self.locally_synthesized_cancellation = true;
        self.state = TurnState::EmitTerminal(Box::new(TurnEvent::Completed {
            reason: StopReason::Cancelled,
            usage: self.usage,
        }));
    }

    fn establish_cancellation_if_observed(&mut self) -> bool {
        if self.cancellation.is_cancelled() && !self.terminal_seen {
            self.establish_cancellation();
            true
        } else {
            false
        }
    }

    fn cancel_pending_delivery(&mut self) -> Option<EngineEvent> {
        if !self.cancellation.is_cancelled() {
            return None;
        }
        let delivers_cancellation = self.locally_synthesized_cancellation
            && matches!(
                &self.delivery.as_ref()?.event.payload,
                TurnEvent::Completed {
                    reason: StopReason::Cancelled,
                    ..
                }
            );
        if self.terminal_seen && !delivers_cancellation {
            return None;
        }
        let pending = self.delivery.take()?;
        let event = if delivers_cancellation {
            pending.event
        } else {
            self.locally_synthesized_cancellation = true;
            let event = EngineEvent {
                session_id: self.session_id.clone(),
                session_incarnation_id: self.session_incarnation_id.clone(),
                turn_id: self.id.clone(),
                sequence: self.sequence,
                payload: TurnEvent::Completed {
                    reason: StopReason::Cancelled,
                    usage: self.usage,
                },
            };
            self.sequence = self.sequence.saturating_add(1);
            event
        };
        self.terminal_seen = true;
        self.finish();
        Some(event)
    }

    fn poll_delivery(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<EngineEvent, EngineError>>> {
        let cancellation = self.cancellation.clone();
        if self.terminal_seen {
            cancellation.deregister(&mut self.cancellation_waiter);
        } else {
            cancellation.register(&mut self.cancellation_waiter, context.waker());
        }
        if let Some(event) = self.cancel_pending_delivery() {
            return Poll::Ready(Some(Ok(event)));
        }
        let Some(delivery) = &mut self.delivery else {
            self.fail_before_terminal();
            return Poll::Ready(Some(Err(EngineError::Protocol(
                "event delivery state was lost".to_owned(),
            ))));
        };
        let result = delivery.future.as_mut().poll(context);
        if let Some(event) = self.cancel_pending_delivery() {
            return Poll::Ready(Some(Ok(event)));
        }
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(())) => {
                let Some(delivery) = self.delivery.take() else {
                    self.fail_before_terminal();
                    return Poll::Ready(Some(Err(EngineError::Protocol(
                        "event delivery state was lost".to_owned(),
                    ))));
                };
                let event = delivery.event;
                if matches!(
                    event.payload,
                    TurnEvent::Completed { .. } | TurnEvent::Failed { .. }
                ) {
                    self.finish();
                } else {
                    self.gate.acknowledge();
                    let cancellation = self.cancellation.clone();
                    cancellation.deregister(&mut self.cancellation_waiter);
                }
                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(Err(error)) => {
                self.delivery = None;
                self.fail_before_terminal();
                Poll::Ready(Some(Err(EngineError::EventSink(redact_event_sink_error(
                    error,
                )))))
            }
        }
    }
}

impl Stream for Turn {
    type Item = Result<EngineEvent, EngineError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(event) = self.cancel_pending_delivery() {
                return Poll::Ready(Some(Ok(event)));
            }
            if self.delivery.is_some() {
                return self.poll_delivery(context);
            }

            self.usage = self.gate.usage();
            self.terminal_seen |= self.gate.terminal_established();
            self.establish_cancellation_if_observed();
            if !self.terminal_seen {
                let cancellation = self.cancellation.clone();
                cancellation.register(&mut self.cancellation_waiter, context.waker());
            }

            let state = core::mem::replace(&mut self.state, TurnState::Done);
            match state {
                TurnState::Running(mut workflow) => {
                    let result = workflow.as_mut().poll(context);
                    self.usage = self.gate.usage();
                    self.terminal_seen |= self.gate.terminal_established();
                    if let Some(staged) = self.gate.take_pending() {
                        if staged.establishes_terminal {
                            self.terminal_seen = true;
                        }
                        self.state = TurnState::Running(workflow);
                        if !self.establish_cancellation_if_observed() {
                            self.stage(staged.payload);
                        }
                    } else if !self.establish_cancellation_if_observed() {
                        match result {
                            Poll::Pending => {
                                self.state = TurnState::Running(workflow);
                                return Poll::Pending;
                            }
                            Poll::Ready(WorkflowExit::Completed { reason, usage }) => {
                                self.usage = usage;
                                self.terminal_seen = true;
                                self.state =
                                    TurnState::EmitTerminal(Box::new(TurnEvent::Completed {
                                        reason,
                                        usage,
                                    }));
                            }
                            Poll::Ready(WorkflowExit::Failed(failure)) => {
                                self.terminal_seen = true;
                                self.state =
                                    TurnState::EmitTerminal(Box::new(failure.into_event()));
                            }
                            Poll::Ready(WorkflowExit::Cancelled) => {
                                self.establish_cancellation();
                            }
                        }
                    }
                }
                TurnState::EmitTerminal(event) => self.stage(*event),
                TurnState::Done => return Poll::Ready(None),
            }
        }
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        if self.lease.is_some() {
            let _ = self.cancellation.cancel();
        }
        self.finish();
    }
}
