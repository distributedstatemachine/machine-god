use machine_god_core::{BoxFuture, EngineEvent, EventSink, EventSinkError};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::DEFAULT_RECORD_CAPACITY;

/// One ordered response from a scripted event observer.
#[derive(Clone, Debug)]
pub enum EventSinkStep {
    Accept,
    Error(EventSinkError),
    /// Never resolve. The engine can still cancel a preterminal delivery.
    Pending,
}

#[derive(Debug)]
enum SinkBehavior {
    AcceptAll,
    Scripted(VecDeque<EventSinkStep>),
}

#[derive(Debug)]
struct SinkState {
    events: Vec<EngineEvent>,
    behavior: SinkBehavior,
}

#[derive(Debug)]
struct SinkInner {
    record_capacity: usize,
    state: Mutex<SinkState>,
}

/// A cloneable event sink that retains a bounded, ordered event snapshot.
#[derive(Clone, Debug)]
pub struct RecordingEventSink {
    inner: Arc<SinkInner>,
}

impl Default for RecordingEventSink {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingEventSink {
    /// Creates a sink that accepts every event up to the default record bound.
    #[must_use]
    pub fn new() -> Self {
        Self::with_record_capacity(DEFAULT_RECORD_CAPACITY)
    }

    /// Creates an accept-all sink with an explicit retained-event bound.
    #[must_use]
    pub fn with_record_capacity(record_capacity: usize) -> Self {
        Self::from_behavior(SinkBehavior::AcceptAll, record_capacity)
    }

    /// Creates a strict sink that consumes one step per event.
    #[must_use]
    pub fn scripted(steps: impl IntoIterator<Item = EventSinkStep>) -> Self {
        Self::scripted_with_record_capacity(steps, DEFAULT_RECORD_CAPACITY)
    }

    /// Creates a strict sink with an explicit retained-event bound.
    #[must_use]
    pub fn scripted_with_record_capacity(
        steps: impl IntoIterator<Item = EventSinkStep>,
        record_capacity: usize,
    ) -> Self {
        Self::from_behavior(
            SinkBehavior::Scripted(steps.into_iter().collect()),
            record_capacity,
        )
    }

    fn from_behavior(behavior: SinkBehavior, record_capacity: usize) -> Self {
        Self {
            inner: Arc::new(SinkInner {
                record_capacity,
                state: Mutex::new(SinkState {
                    events: Vec::new(),
                    behavior,
                }),
            }),
        }
    }

    /// Returns a consistent snapshot in observer-call order.
    #[must_use]
    pub fn events(&self) -> Vec<EngineEvent> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .events
            .clone()
    }

    /// Returns the number of unconsumed strict steps, or `None` for accept-all.
    #[must_use]
    pub fn remaining_steps(&self) -> Option<usize> {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &state.behavior {
            SinkBehavior::AcceptAll => None,
            SinkBehavior::Scripted(steps) => Some(steps.len()),
        }
    }

    fn record_and_select(&self, event: EngineEvent) -> Result<EventSinkStep, EventSinkError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.events.len() >= self.inner.record_capacity {
            return Err(sink_fixture_error(
                "testkit_record_capacity_exhausted",
                "recording event sink capacity was exhausted",
            ));
        }
        state.events.push(event);
        let step = match &mut state.behavior {
            SinkBehavior::AcceptAll => EventSinkStep::Accept,
            SinkBehavior::Scripted(steps) => steps.pop_front().ok_or_else(|| {
                sink_fixture_error(
                    "testkit_script_exhausted",
                    "recording event sink received an event after its script was exhausted",
                )
            })?,
        };
        Ok(step)
    }
}

impl EventSink for RecordingEventSink {
    fn emit(&self, event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        let step = self.record_and_select(event);
        Box::pin(async move {
            match step? {
                EventSinkStep::Accept => Ok(()),
                EventSinkStep::Error(error) => Err(error),
                EventSinkStep::Pending => core::future::pending().await,
            }
        })
    }
}

fn sink_fixture_error(code: &'static str, message: &'static str) -> EventSinkError {
    EventSinkError::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::{EventSinkStep, RecordingEventSink};

    #[test]
    fn reports_remaining_strict_steps() {
        let sink = RecordingEventSink::scripted([EventSinkStep::Accept]);
        assert_eq!(sink.remaining_steps(), Some(1));
        assert!(sink.events().is_empty());
    }
}
