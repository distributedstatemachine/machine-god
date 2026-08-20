use crate::{
    BoxFuture, CancellationToken, Capability, ContentBlock, EngineError, EngineEvent,
    InferenceOptions, Message, ModelEvent, ModelEventStream, ModelRequest, PermissionDecision,
    PermissionRequest, PermissionRequestId, PermissionRisk, Role, SessionId, SessionStoreError,
    SessionStoreErrorKind, StopReason, TokenUsage, ToolCall, ToolContext, ToolOutput, TurnEvent,
    TurnId,
};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
    pub fn empty(id: SessionId) -> Self {
        Self {
            id,
            revision: SessionRevision::default(),
            next_turn_sequence: initial_turn_sequence(),
            messages: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }
}

const fn initial_turn_sequence() -> u64 {
    1
}

/// Object-safe persistence boundary with optimistic concurrency.
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
    record: SessionRecord,
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
            data: Mutex::new(SessionData { record, persisted }),
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

    pub(crate) fn reconcile_loaded(&self, record: SessionRecord) -> Result<(), EngineError> {
        Self::validate_loaded(&record)?;
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if record.id != data.record.id {
            return Err(EngineError::Protocol(format!(
                "session store returned ID {} for requested ID {}",
                record.id, data.record.id
            )));
        }
        Self::validate_sequence_progress(&data.record, &record)?;
        if record.revision > data.record.revision {
            data.record = record;
            data.persisted = true;
        } else if record.revision < data.record.revision {
            return Err(EngineError::Protocol(format!(
                "session store returned stale revision {} behind canonical revision {}",
                record.revision.0, data.record.revision.0
            )));
        } else if record.revision == data.record.revision && record != data.record {
            return Err(EngineError::Protocol(
                "session store returned different records at the same revision".to_owned(),
            ));
        } else {
            data.persisted = true;
        }
        Ok(())
    }

    fn reconcile_saved(&self, record: &SessionRecord) -> Result<(), EngineError> {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::validate_sequence_progress(&data.record, record)?;
        if data.record.revision < record.revision {
            data.record.clone_from(record);
            data.persisted = true;
        } else if data.record.revision == record.revision && data.record != *record {
            return Err(EngineError::Protocol(
                "successful save diverged from canonical record at the same revision".to_owned(),
            ));
        } else if data.record.revision == record.revision {
            data.persisted = true;
        }
        Ok(())
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

    /// Returns a consistent snapshot of in-memory session state.
    #[must_use]
    pub fn record(&self) -> SessionRecord {
        self.state
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record
            .clone()
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
        let prompt = prompt.into();
        Box::pin(async move { session.start_prompt(prompt).await })
    }

    async fn start_prompt(&self, prompt: Prompt) -> Result<Turn, EngineError> {
        self.state
            .active_turn
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| EngineError::SessionBusy)?;
        let lease = TurnLease {
            state: Arc::clone(&self.state),
        };

        let (turn_id, record) = self.reserve_turn_and_prompt(prompt.text).await?;
        let session_id = record.id.clone();
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
            let (snapshot, expected_revision) = {
                let data = self
                    .state
                    .data
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (
                    data.record.clone(),
                    data.persisted.then_some(data.record.revision),
                )
            };
            let mut candidate = snapshot.clone();
            let turn_sequence = candidate.next_turn_sequence;
            candidate.next_turn_sequence = turn_sequence.checked_add(1).ok_or_else(|| {
                EngineError::Protocol("session turn sequence is exhausted".to_owned())
            })?;
            let turn_id = TurnId::new(format!("turn-{turn_sequence}"))
                .map_err(|error| EngineError::Protocol(error.to_string()))?;
            candidate
                .messages
                .push(Message::text(Role::User, &prompt_text));
            let previous_revision = candidate.revision;

            match self
                .engine
                .session_store
                .save(candidate.clone(), expected_revision)
                .await
            {
                Ok(revision) if revision > previous_revision => {
                    candidate.revision = revision;
                    self.state.reconcile_saved(&candidate)?;
                    return Ok((turn_id, candidate));
                }
                Ok(_) => {
                    return Err(EngineError::Protocol(
                        "session store returned a non-increasing revision".to_owned(),
                    ));
                }
                Err(error) if error.kind == SessionStoreErrorKind::Conflict => {
                    let id = candidate.id.clone();
                    let Some(current) = self.engine.session_store.load(id.clone()).await? else {
                        let mut data = self
                            .state
                            .data
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if data.record.id != id {
                            return Err(EngineError::Protocol(
                                "session identity changed during turn reservation".to_owned(),
                            ));
                        }
                        if data.record == snapshot && data.persisted == expected_revision.is_some()
                        {
                            data.persisted = false;
                        }
                        continue;
                    };
                    if current.id != id {
                        return Err(EngineError::Protocol(format!(
                            "session store returned ID {} for requested ID {id}",
                            current.id
                        )));
                    }
                    self.state.reconcile_loaded(current)?;
                }
                Err(error) => return Err(error.into()),
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
    EmitTerminal(TurnEvent),
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

    fn provider(error: crate::ProviderError) -> Self {
        Self {
            component: "provider".to_owned(),
            code: error.code,
            message: error.message,
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

    fn store(error: SessionStoreError) -> Self {
        Self {
            component: "store".to_owned(),
            code: error.code,
            message: error.message,
            retryable: error.retryable,
        }
    }

    fn permission(error: crate::PermissionError) -> Self {
        Self {
            component: "permission".to_owned(),
            code: error.code,
            message: error.message,
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
    let mut model_rounds = 0usize;
    let mut tool_calls = 0usize;
    let mut cumulative_tool_result_bytes = 0usize;
    let mut seen_call_ids = BTreeSet::new();
    let mut usage = TokenUsage::default();
    let mut assistant_bytes = 0usize;
    let mut reasoning_bytes = 0usize;

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

        let request = ModelRequest {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            messages: record.messages.clone(),
            tools: engine.tool_specs(),
            options: options.clone(),
        };
        check_cancelled(&cancellation)?;
        let provider_start = engine.provider.stream(request, cancellation.clone());
        let mut stream = await_cancellable(provider_start, &cancellation)
            .await?
            .map_err(TurnFailure::provider)?;

        let mut assistant_text = String::new();
        let mut calls = Vec::new();
        let mut round_call_ids = BTreeSet::new();
        let mut round_usage = TokenUsage::default();
        let stop_reason = loop {
            let item = next_model_item(&mut stream, &cancellation).await?;
            match item {
                Some(Ok(ModelEvent::Stop { reason })) => break reason,
                Some(Ok(event)) => {
                    match &event {
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
                            if engine.tool(&call.name).is_none() {
                                return Err(TurnFailure::protocol(
                                    "unknown_tool",
                                    format!("provider requested unregistered tool {}", call.name),
                                )
                                .into());
                            }
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
                    emitter.emit(TurnEvent::Model { event }).await;
                }
                Some(Err(error)) => return Err(TurnFailure::provider(error).into()),
                None => {
                    return Err(TurnFailure::missing_stop().into());
                }
            }
        };

        usage = checked_usage_add(usage, round_usage)?;
        validate_round_stop(&calls, &stop_reason)?;

        if calls.is_empty() {
            emitter.establish_terminal();
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
                false,
            )
            .await?;
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

        emitter
            .emit(TurnEvent::Model {
                event: ModelEvent::Stop {
                    reason: stop_reason,
                },
            })
            .await;
        record = commit_message(
            &engine,
            &session_state,
            &record,
            Message {
                role: Role::Assistant,
                content: assistant_message_content(&assistant_text, &calls),
            },
            &cancellation,
            true,
        )
        .await?;

        for (round_index, call) in calls.into_iter().enumerate() {
            check_cancelled(&cancellation)?;
            let ordinal = tool_calls
                .checked_add(round_index)
                .and_then(|index| index.checked_add(1))
                .ok_or_else(|| {
                    TurnFailure::limit("tool_call_limit", "tool call count overflowed")
                })?;
            let permission_id = PermissionRequestId::new(format!("permission-{ordinal}"))
                .map_err(|error| TurnFailure::protocol("permission_id", error.to_string()))?;
            let request = PermissionRequest {
                id: permission_id.clone(),
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                capability: Capability::Tool {
                    name: call.name.clone(),
                    call_id: call.id.clone(),
                    arguments: call.arguments.clone(),
                },
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
                .map_err(TurnFailure::permission)?;
            emitter
                .emit(TurnEvent::PermissionResolved {
                    request_id: permission_id,
                    decision: decision.clone(),
                })
                .await;

            let (output, emit_finished) = match decision {
                PermissionDecision::Deny { reason } => (
                    ToolOutput {
                        content: json!({
                            "code": "permission_denied",
                            "message": bounded_text(&reason, 1_024),
                        }),
                        is_error: true,
                    },
                    false,
                ),
                PermissionDecision::Allow { .. } => {
                    let tool = engine.tool(&call.name).ok_or_else(|| {
                        TurnFailure::protocol(
                            "unknown_tool",
                            format!("tool {} disappeared after round validation", call.name),
                        )
                    })?;
                    emitter
                        .emit(TurnEvent::ToolStarted { call: call.clone() })
                        .await;
                    let execution = tool.execute(
                        ToolContext {
                            session_id: session_id.clone(),
                            turn_id: turn_id.clone(),
                            call_id: call.id.clone(),
                        },
                        call.arguments.clone(),
                        cancellation.clone(),
                    );
                    let result = await_cancellable(execution, &cancellation).await?;
                    let output = match result {
                        Ok(output) => output,
                        Err(error) => ToolOutput {
                            content: json!({
                                "code": bounded_text(&error.code, 256),
                                "message": bounded_text(&error.message, 1_024),
                                "retryable": error.retryable,
                            }),
                            is_error: true,
                        },
                    };
                    (output, true)
                }
            };

            let result_bytes = serialized_json_size_bounded(
                &output,
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
                Some(result_bytes) => {
                    match cumulative_tool_result_bytes.checked_add(result_bytes) {
                        Some(total) if total <= limits.max_cumulative_tool_result_bytes.get() => {
                            cumulative_tool_result_bytes = total;
                            None
                        }
                        _ => Some(TurnFailure::limit(
                            "cumulative_tool_result_size_limit",
                            "turn exceeded the cumulative tool result size limit",
                        )),
                    }
                }
            };
            if let Some(failure) = size_failure {
                if emit_finished {
                    emitter.establish_terminal();
                    let marker = ToolOutput {
                        content: json!({
                            "code": "tool_result_discarded",
                            "message": "tool executed but its result exceeded a configured size bound",
                        }),
                        is_error: true,
                    };
                    let _committed = commit_message(
                        &engine,
                        &session_state,
                        &record,
                        Message {
                            role: Role::Tool,
                            content: vec![ContentBlock::ToolResult {
                                call_id: call.id.clone(),
                                output: marker,
                            }],
                        },
                        &cancellation,
                        false,
                    )
                    .await?;
                }
                return Err(failure.into());
            }

            record = commit_message(
                &engine,
                &session_state,
                &record,
                Message {
                    role: Role::Tool,
                    content: vec![ContentBlock::ToolResult {
                        call_id: call.id.clone(),
                        output: output.clone(),
                    }],
                },
                &cancellation,
                true,
            )
            .await?;
            if emit_finished {
                emitter
                    .emit(TurnEvent::ToolFinished {
                        call_id: call.id,
                        output,
                    })
                    .await;
            }
        }
        tool_calls = new_total;
    }
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

fn serialized_json_size_bounded(
    value: &impl Serialize,
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
    honor_cancellation: bool,
) -> Result<SessionRecord, WorkflowAbort> {
    const MAX_CONFLICT_RETRIES: usize = 32;

    for _ in 0..MAX_CONFLICT_RETRIES {
        if honor_cancellation {
            check_cancelled(cancellation)?;
        }
        let (snapshot, expected_revision) = {
            let data = state
                .data
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if data.record.id != base.id {
                return Err(TurnFailure::protocol(
                    "session_identity_changed",
                    "session identity changed during transcript commit",
                )
                .into());
            }
            if data.record.messages != base.messages {
                return Err(TurnFailure::protocol(
                    "transcript_diverged",
                    "durable transcript diverged before message commit",
                )
                .into());
            }
            (
                data.record.clone(),
                data.persisted.then_some(data.record.revision),
            )
        };
        let mut candidate = snapshot;
        candidate.messages.push(message.clone());
        let previous_revision = candidate.revision;
        let save = engine
            .session_store
            .save(candidate.clone(), expected_revision);
        match await_operation(save, cancellation, honor_cancellation).await? {
            Ok(revision) if revision > previous_revision => {
                candidate.revision = revision;
                state.reconcile_saved(&candidate).map_err(|error| {
                    TurnFailure::protocol("save_reconciliation", error.to_string())
                })?;
                return Ok(candidate);
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
                let Some(current) = await_operation(load, cancellation, honor_cancellation)
                    .await?
                    .map_err(TurnFailure::store)?
                else {
                    return Err(TurnFailure::protocol(
                        "transcript_missing_after_conflict",
                        "durable transcript disappeared after a commit conflict",
                    )
                    .into());
                };
                if current.id != base.id {
                    return Err(TurnFailure::protocol(
                        "session_identity_changed",
                        format!(
                            "session store returned ID {} for requested ID {}",
                            current.id, base.id
                        ),
                    )
                    .into());
                }
                SessionState::validate_loaded(&current).map_err(|error| {
                    TurnFailure::protocol("invalid_conflict_record", error.to_string())
                })?;
                if current.messages != base.messages {
                    return Err(TurnFailure::protocol(
                        "transcript_diverged",
                        "durable transcript diverged during message commit",
                    )
                    .into());
                }
                state.reconcile_loaded(current).map_err(|error| {
                    TurnFailure::protocol("conflict_reconciliation", error.to_string())
                })?;
            }
            Err(error) => return Err(TurnFailure::store(error).into()),
        }
    }
    Err(TurnFailure::store(SessionStoreError::new(
        SessionStoreErrorKind::Conflict,
        "message_commit_contended",
        "message commit exceeded its conflict retry bound",
        true,
    ))
    .into())
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
    cancellation_waiter: Option<u64>,
    lease: Option<TurnLease>,
}

impl fmt::Debug for Turn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Turn")
            .field("session_id", &self.session_id)
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
        self.state = TurnState::EmitTerminal(TurnEvent::Completed {
            reason: StopReason::Cancelled,
            usage: self.usage,
        });
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
        let delivers_cancellation = matches!(
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
            let event = EngineEvent {
                session_id: self.session_id.clone(),
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
                Poll::Ready(Some(Err(EngineError::EventSink(error))))
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
                                    TurnState::EmitTerminal(TurnEvent::Completed { reason, usage });
                            }
                            Poll::Ready(WorkflowExit::Failed(failure)) => {
                                self.terminal_seen = true;
                                self.state = TurnState::EmitTerminal(failure.into_event());
                            }
                            Poll::Ready(WorkflowExit::Cancelled) => {
                                self.establish_cancellation();
                            }
                        }
                    }
                }
                TurnState::EmitTerminal(event) => self.stage(event),
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
