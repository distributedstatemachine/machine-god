use crate::{
    BoxFuture, CancellationToken, EngineError, EngineEvent, InferenceOptions, Message, ModelEvent,
    ModelEventStream, ModelRequest, Role, SessionId, SessionStoreError, StopReason, TokenUsage,
    TurnEvent, TurnId,
};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::engine::EngineInner;

/// Optimistic-concurrency revision assigned by a [`SessionStore`].
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SessionRevision(pub u64);

/// Durable provider-neutral session state.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub revision: SessionRevision,
    pub messages: Vec<Message>,
    pub metadata: BTreeMap<String, Value>,
}

impl SessionRecord {
    #[must_use]
    pub fn empty(id: SessionId) -> Self {
        Self {
            id,
            revision: SessionRevision::default(),
            messages: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }
}

/// Object-safe persistence boundary with optimistic concurrency.
pub trait SessionStore: Send + Sync + 'static {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>>;

    /// Persists `record` only if its currently stored revision equals
    /// `expected_revision`. A new record uses `None`.
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

struct SessionState {
    record: Mutex<SessionRecord>,
    active_turn: Arc<AtomicBool>,
    next_turn: AtomicU64,
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
    pub(crate) fn from_record(engine: Arc<EngineInner>, record: SessionRecord) -> Self {
        Self {
            engine,
            state: Arc::new(SessionState {
                record: Mutex::new(record),
                active_turn: Arc::new(AtomicBool::new(false)),
                next_turn: AtomicU64::new(1),
            }),
        }
    }

    #[must_use]
    pub fn id(&self) -> SessionId {
        self.state
            .record
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .id
            .clone()
    }

    /// Returns a consistent snapshot of in-memory session state.
    #[must_use]
    pub fn record(&self) -> SessionRecord {
        self.state
            .record
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub fn has_active_turn(&self) -> bool {
        self.state.active_turn.load(Ordering::Acquire)
    }

    /// Starts one provider stream. A session permits at most one live turn.
    ///
    /// Dropping the returned stream releases the session immediately. Provider
    /// and tool-loop orchestration will extend this foundation without changing
    /// the public streaming shape.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::SessionBusy`] when this session, or any clone of
    /// it, already owns a live turn.
    pub fn prompt(&self, prompt: impl Into<Prompt>) -> Result<Turn, EngineError> {
        self.state
            .active_turn
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| EngineError::SessionBusy)?;
        let lease = TurnLease {
            active: Arc::clone(&self.state.active_turn),
        };

        let turn_number = self.state.next_turn.fetch_add(1, Ordering::Relaxed);
        let turn_id = TurnId::new(format!("turn-{turn_number}"))
            .map_err(|error| EngineError::Protocol(error.to_string()))?;
        let session_id = self.id();
        let prompt = prompt.into();
        let mut messages = self.record().messages;
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
            lease: Some(lease),
        })
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
    active: Arc<AtomicBool>,
}

impl Drop for TurnLease {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
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
        self.state = TurnState::Done;
        self.lease.take();
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
}

impl Stream for Turn {
    type Item = Result<EngineEvent, EngineError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(delivery) = &mut self.delivery {
                match delivery.future.as_mut().poll(context) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Ok(())) => {
                        let Some(delivery) = self.delivery.take() else {
                            self.finish();
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
                        }
                        return Poll::Ready(Some(Ok(event)));
                    }
                    Poll::Ready(Err(error)) => {
                        self.delivery = None;
                        self.finish();
                        return Poll::Ready(Some(Err(EngineError::EventSink(error))));
                    }
                }
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
                self.cancellation.register(context.waker());
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
