use crate::{
    BoxFuture, CancellationToken, EngineError, EngineEvent, InferenceOptions, Message, ModelEvent,
    ModelEventStream, ModelRequest, Role, SessionId, SessionStoreError, SessionStoreErrorKind,
    StopReason, TokenUsage, TurnEvent, TurnId,
};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::engine::EngineInner;

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
    /// First never-reserved turn sequence number.
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
}

struct SessionData {
    record: SessionRecord,
    persisted: bool,
}

impl SessionState {
    pub(crate) fn new(record: SessionRecord, persisted: bool) -> Self {
        Self {
            data: Mutex::new(SessionData { record, persisted }),
            active_turn: AtomicBool::new(false),
        }
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

    /// Reserves a durable turn ID and starts one provider stream. A session
    /// permits at most one live turn.
    ///
    /// Dropping a live returned stream requests provider cancellation before
    /// releasing the session immediately. Provider and tool-loop orchestration
    /// will extend this foundation without changing the public streaming shape.
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

        let (turn_id, record) = self.reserve_turn_id().await?;
        let session_id = record.id.clone();
        let mut messages = record.messages;
        messages.push(Message::text(Role::User, prompt.text));
        let request = ModelRequest {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            messages,
            tools: self.engine.tool_specs(),
            options: prompt.options,
        };
        let cancellation = CancellationToken::new();
        let provider = Arc::clone(&self.engine.provider);
        let provider_cancellation = cancellation.clone();
        let starting =
            Box::pin(async move { provider.stream(request, provider_cancellation).await });

        Ok(Turn {
            session_id,
            id: turn_id.clone(),
            handle: TurnHandle {
                turn_id,
                cancellation: cancellation.clone(),
            },
            cancellation,
            sink: Arc::clone(&self.engine.event_sink),
            state: TurnState::EmitStarted(Some(starting)),
            delivery: None,
            sequence: 0,
            usage: TokenUsage::default(),
            terminal_seen: false,
            cancellation_waiter: None,
            lease: Some(lease),
        })
    }

    async fn reserve_turn_id(&self) -> Result<(TurnId, SessionRecord), EngineError> {
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
            "turn ID reservation exceeded its conflict retry bound",
            true,
        )
        .into())
    }
}

type StartFuture = BoxFuture<'static, Result<ModelEventStream, crate::ProviderError>>;
type DeliveryFuture = BoxFuture<'static, Result<(), crate::EventSinkError>>;

enum TurnState {
    EmitStarted(Option<StartFuture>),
    Starting(StartFuture),
    Streaming(ModelEventStream),
    EmitTerminal(TurnEvent),
    Done,
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

    fn fail_provider(&mut self, error: crate::ProviderError) {
        self.terminal_seen = true;
        self.state = TurnState::EmitTerminal(TurnEvent::Failed {
            component: "provider".to_owned(),
            code: error.code,
            message: error.message,
            retryable: error.retryable,
        });
    }

    fn cancel_pending_delivery(&mut self) -> Option<EngineEvent> {
        if !self.cancellation.is_cancelled() {
            return None;
        }
        let pending = self.delivery.take()?;
        let event = if matches!(
            pending.event.payload,
            TurnEvent::Completed {
                reason: StopReason::Cancelled,
                ..
            }
        ) {
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
        cancellation.register(&mut self.cancellation_waiter, context.waker());
        if let Some(event) = self.cancel_pending_delivery() {
            return Poll::Ready(Some(Ok(event)));
        }
        let Some(delivery) = &mut self.delivery else {
            self.fail_before_terminal();
            return Poll::Ready(Some(Err(EngineError::Protocol(
                "event delivery state was lost".to_owned(),
            ))));
        };
        match delivery.future.as_mut().poll(context) {
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

            if self.cancellation.is_cancelled()
                && !self.terminal_seen
                && !matches!(self.state, TurnState::EmitStarted(_))
            {
                self.terminal_seen = true;
                self.state = TurnState::EmitTerminal(TurnEvent::Completed {
                    reason: StopReason::Cancelled,
                    usage: self.usage,
                });
            } else if !self.terminal_seen {
                let cancellation = self.cancellation.clone();
                cancellation.register(&mut self.cancellation_waiter, context.waker());
            }

            let state = core::mem::replace(&mut self.state, TurnState::Done);
            match state {
                TurnState::EmitStarted(mut starting) => {
                    self.state =
                        TurnState::Starting(starting.take().expect("starting future is available"));
                    self.stage(TurnEvent::Started);
                }
                TurnState::Starting(mut starting) => match starting.as_mut().poll(context) {
                    Poll::Pending => {
                        self.state = TurnState::Starting(starting);
                        return Poll::Pending;
                    }
                    Poll::Ready(Ok(stream)) => self.state = TurnState::Streaming(stream),
                    Poll::Ready(Err(error)) => self.fail_provider(error),
                },
                TurnState::Streaming(mut stream) => match stream.as_mut().poll_next(context) {
                    Poll::Pending => {
                        self.state = TurnState::Streaming(stream);
                        return Poll::Pending;
                    }
                    Poll::Ready(Some(Ok(event))) => {
                        match &event {
                            ModelEvent::Usage { usage } => self.usage = *usage,
                            ModelEvent::Stop { reason } => {
                                self.terminal_seen = true;
                                self.state = TurnState::EmitTerminal(TurnEvent::Completed {
                                    reason: reason.clone(),
                                    usage: self.usage,
                                });
                            }
                            _ => self.state = TurnState::Streaming(stream),
                        }
                        self.stage(TurnEvent::Model { event });
                    }
                    Poll::Ready(Some(Err(error))) => self.fail_provider(error),
                    Poll::Ready(None) => {
                        self.terminal_seen = true;
                        self.state = TurnState::EmitTerminal(TurnEvent::Failed {
                            component: "provider".to_owned(),
                            code: "missing_stop".to_owned(),
                            message: "provider stream ended without a terminal stop".to_owned(),
                            retryable: false,
                        });
                    }
                },
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
