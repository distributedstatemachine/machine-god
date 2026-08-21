use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use futures_core::Stream;
use machine_god_core::{
    BoxFuture, CancellationToken, ModelEvent, ModelEventStream, ModelProvider, ModelRequest,
    ProviderError, ProviderErrorKind,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::DEFAULT_RECORD_CAPACITY;

/// Behavior after all scripted model events have been emitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelStreamEnd {
    /// End the stream. Scripts normally include a terminal model stop first.
    Close,
    /// Remain pending until the recorded cancellation token is cancelled.
    Pending,
}

/// One ordered response to a call to [`ModelProvider::stream`].
#[derive(Clone, Debug)]
pub enum ModelProviderStep {
    /// Fail before returning a stream.
    StartError(ProviderError),
    /// Return a stream containing the supplied results.
    Stream {
        events: Vec<Result<ModelEvent, ProviderError>>,
        end: ModelStreamEnd,
    },
    /// Keep provider startup pending until cancellation is requested.
    PendingStart,
}

impl ModelProviderStep {
    /// Creates a finite response stream.
    #[must_use]
    pub fn events(events: impl IntoIterator<Item = ModelEvent>) -> Self {
        Self::Stream {
            events: events.into_iter().map(Ok).collect(),
            end: ModelStreamEnd::Close,
        }
    }

    /// Creates a response stream from successful or failed ordered items.
    #[must_use]
    pub fn results(events: impl IntoIterator<Item = Result<ModelEvent, ProviderError>>) -> Self {
        Self::Stream {
            events: events.into_iter().collect(),
            end: ModelStreamEnd::Close,
        }
    }

    /// Creates a stream that emits `events`, then remains pending until cancelled.
    #[must_use]
    pub fn events_then_pending(events: impl IntoIterator<Item = ModelEvent>) -> Self {
        Self::Stream {
            events: events.into_iter().map(Ok).collect(),
            end: ModelStreamEnd::Pending,
        }
    }

    /// Creates a stream that remains pending until cancelled.
    #[must_use]
    pub fn pending() -> Self {
        Self::events_then_pending([])
    }
}

/// A provider invocation captured before its scripted behavior starts.
#[derive(Clone, Debug)]
pub struct RecordedModelRequest {
    pub request: ModelRequest,
    pub cancellation: CancellationToken,
}

#[derive(Debug)]
struct ProviderState {
    steps: VecDeque<ModelProviderStep>,
    requests: Vec<RecordedModelRequest>,
}

#[derive(Debug)]
struct ProviderInner {
    name: String,
    record_capacity: usize,
    state: Mutex<ProviderState>,
}

/// A cloneable, thread-safe provider that consumes one strict step per request.
///
/// Calls after script exhaustion, or after the recording bound is reached,
/// return a structured non-retryable protocol error without consuming another
/// step.
#[derive(Clone, Debug)]
pub struct ScriptedModelProvider {
    inner: Arc<ProviderInner>,
}

impl ScriptedModelProvider {
    /// Creates a provider with the default recording capacity.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        steps: impl IntoIterator<Item = ModelProviderStep>,
    ) -> Self {
        Self::with_record_capacity(name, steps, DEFAULT_RECORD_CAPACITY)
    }

    /// Creates a provider with an explicit bound on retained requests.
    #[must_use]
    pub fn with_record_capacity(
        name: impl Into<String>,
        steps: impl IntoIterator<Item = ModelProviderStep>,
        record_capacity: usize,
    ) -> Self {
        Self {
            inner: Arc::new(ProviderInner {
                name: name.into(),
                record_capacity,
                state: Mutex::new(ProviderState {
                    steps: steps.into_iter().collect(),
                    requests: Vec::new(),
                }),
            }),
        }
    }

    /// Returns a consistent snapshot in provider-start order.
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedModelRequest> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .requests
            .clone()
    }

    /// Returns the number of unconsumed strict steps.
    #[must_use]
    pub fn remaining_steps(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .steps
            .len()
    }

    fn next_step(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> Result<ModelProviderStep, ProviderError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.requests.len() >= self.inner.record_capacity {
            return Err(provider_fixture_error(
                "testkit_record_capacity_exhausted",
                "scripted provider request recording capacity was exhausted",
            ));
        }
        state.requests.push(RecordedModelRequest {
            request,
            cancellation,
        });
        let Some(step) = state.steps.pop_front() else {
            return Err(provider_fixture_error(
                "testkit_script_exhausted",
                "scripted provider received a request after its script was exhausted",
            ));
        };
        Ok(step)
    }
}

impl ModelProvider for ScriptedModelProvider {
    fn name(&self) -> &str {
        &self.inner.name
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        let step = self.next_step(request, cancellation.clone());
        Box::pin(async move {
            match step? {
                ModelProviderStep::StartError(error) => Err(error),
                ModelProviderStep::Stream { events, end } => {
                    Ok(
                        Box::pin(ScriptedModelStream::new(events, end, cancellation))
                            as ModelEventStream,
                    )
                }
                ModelProviderStep::PendingStart => {
                    cancellation.cancelled().await;
                    Err(ProviderError::new(
                        ProviderErrorKind::Cancelled,
                        "cancelled",
                        "scripted provider startup was cancelled",
                        false,
                    ))
                }
            }
        })
    }
}

fn provider_fixture_error(code: &'static str, message: &'static str) -> ProviderError {
    ProviderError::new(ProviderErrorKind::Protocol, code, message, false)
}

#[derive(Debug)]
struct ScriptedModelStream {
    events: VecDeque<Result<ModelEvent, ProviderError>>,
    end: ModelStreamEnd,
    cancellation: CancellationToken,
    cancelled: Option<machine_god_core::Cancelled>,
    done: bool,
}

impl ScriptedModelStream {
    fn new(
        events: Vec<Result<ModelEvent, ProviderError>>,
        end: ModelStreamEnd,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            events: events.into(),
            end,
            cancellation,
            cancelled: None,
            done: false,
        }
    }
}

impl Stream for ScriptedModelStream {
    type Item = Result<ModelEvent, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        if self.cancellation.is_cancelled() {
            self.done = true;
            self.cancelled = None;
            return Poll::Ready(None);
        }
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(Some(event));
        }
        match self.end {
            ModelStreamEnd::Close => {
                self.done = true;
                Poll::Ready(None)
            }
            ModelStreamEnd::Pending => {
                let token = self.cancellation.clone();
                let waiter = self.cancelled.get_or_insert_with(|| token.cancelled());
                if Pin::new(waiter).poll(context).is_ready() {
                    self.done = true;
                    self.cancelled = None;
                    Poll::Ready(None)
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ModelProviderStep, ScriptedModelProvider};
    use machine_god_core::{
        CancellationToken, InferenceOptions, ModelProvider, ModelRequest, ProviderError, SessionId,
        SessionIncarnationId, TurnId,
    };

    fn request() -> ModelRequest {
        ModelRequest {
            session_id: SessionId::new("provider-test").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("provider-test-incarnation").unwrap(),
            turn_id: TurnId::new("turn-1").unwrap(),
            messages: Vec::new(),
            tools: Vec::new(),
            options: InferenceOptions::default(),
        }
    }

    #[test]
    fn recovers_a_poisoned_state_lock() {
        let provider = ScriptedModelProvider::new("poison", [ModelProviderStep::pending()]);
        let inner = provider.inner.clone();
        let _ = std::thread::spawn(move || {
            let _guard = inner.state.lock().unwrap();
            panic!("poison provider state");
        })
        .join();

        let result =
            futures_executor::block_on(provider.stream(request(), CancellationToken::new()));
        assert!(result.is_ok());
        assert_eq!(provider.requests().len(), 1);
    }

    #[test]
    fn exhaustion_is_actionable_and_records_the_attempted_request() {
        let provider = ScriptedModelProvider::new("empty", []);
        let error: ProviderError =
            futures_executor::block_on(provider.stream(request(), CancellationToken::new()))
                .err()
                .expect("empty script must fail");
        assert_eq!(error.code, "testkit_script_exhausted");
        assert_eq!(provider.requests().len(), 1);
    }
}
