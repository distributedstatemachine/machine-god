use machine_god_core::{
    BoxFuture, CancellationToken, SubagentAuthority, SubagentAuthorityError,
    SubagentAuthorityErrorKind, SubagentOutcome, SubagentRequest,
};
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::DEFAULT_RECORD_CAPACITY;

/// One ordered response from a scripted subagent authority.
#[derive(Clone)]
pub enum SubagentStep {
    /// Return the supplied completed child outcome.
    Complete(SubagentOutcome),
    /// Return the supplied authority failure.
    Error(SubagentAuthorityError),
    /// Remain pending until the invocation is cancelled.
    Pending,
}

impl fmt::Debug for SubagentStep {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Complete(_) => formatter.write_str("Complete(..)"),
            Self::Error(error) => formatter.debug_tuple("Error").field(&error.kind()).finish(),
            Self::Pending => formatter.write_str("Pending"),
        }
    }
}

/// One subagent request captured before its scripted behavior starts.
#[derive(Clone)]
pub struct RecordedSubagentRequest {
    pub request: SubagentRequest,
    pub cancellation: CancellationToken,
}

impl fmt::Debug for RecordedSubagentRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordedSubagentRequest")
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

struct SubagentState {
    steps: VecDeque<SubagentStep>,
    requests: Vec<RecordedSubagentRequest>,
}

struct SubagentInner {
    record_capacity: usize,
    state: Mutex<SubagentState>,
}

/// A cloneable, strict subagent authority with bounded request recording.
///
/// Calls after script exhaustion, or after the recording bound is reached,
/// return fixed authority failures without consuming another step. Debug
/// formatting deliberately omits scripted outcomes and request payloads.
#[derive(Clone)]
pub struct ScriptedSubagentAuthority {
    inner: Arc<SubagentInner>,
}

impl ScriptedSubagentAuthority {
    /// Creates an authority with the default request-recording bound.
    #[must_use]
    pub fn new(steps: impl IntoIterator<Item = SubagentStep>) -> Self {
        Self::with_record_capacity(steps, DEFAULT_RECORD_CAPACITY)
    }

    /// Creates an authority with an explicit request-recording bound.
    #[must_use]
    pub fn with_record_capacity(
        steps: impl IntoIterator<Item = SubagentStep>,
        record_capacity: usize,
    ) -> Self {
        Self {
            inner: Arc::new(SubagentInner {
                record_capacity,
                state: Mutex::new(SubagentState {
                    steps: steps.into_iter().collect(),
                    requests: Vec::new(),
                }),
            }),
        }
    }

    /// Returns a consistent snapshot in authority-call order.
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedSubagentRequest> {
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

    fn record_and_select(
        &self,
        request: SubagentRequest,
        cancellation: CancellationToken,
    ) -> Result<SubagentStep, SubagentAuthorityError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.requests.len() >= self.inner.record_capacity {
            return Err(SubagentAuthorityError::new(
                SubagentAuthorityErrorKind::ResourceLimit,
            ));
        }
        state.requests.push(RecordedSubagentRequest {
            request,
            cancellation,
        });
        let Some(step) = state.steps.pop_front() else {
            return Err(SubagentAuthorityError::new(
                SubagentAuthorityErrorKind::Failed,
            ));
        };
        Ok(step)
    }
}

impl fmt::Debug for ScriptedSubagentAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        formatter
            .debug_struct("ScriptedSubagentAuthority")
            .field("record_capacity", &self.inner.record_capacity)
            .field("recorded_request_count", &state.requests.len())
            .field("remaining_step_count", &state.steps.len())
            .finish()
    }
}

impl SubagentAuthority for ScriptedSubagentAuthority {
    fn run(
        &self,
        request: SubagentRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<SubagentOutcome, SubagentAuthorityError>> {
        let step = self.record_and_select(request, cancellation.clone());
        Box::pin(async move {
            match step? {
                SubagentStep::Complete(outcome) => Ok(outcome),
                SubagentStep::Error(error) => Err(error),
                SubagentStep::Pending => {
                    cancellation.cancelled().await;
                    Err(SubagentAuthorityError::new(
                        SubagentAuthorityErrorKind::Cancelled,
                    ))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ScriptedSubagentAuthority, SubagentStep};
    use core::task::{Context, Poll};
    use futures_executor::block_on;
    use futures_util::task::noop_waker_ref;
    use machine_god_core::{
        CancellationToken, SessionId, SessionIncarnationId, SubagentAuthority,
        SubagentAuthorityError, SubagentAuthorityErrorKind, SubagentOutcome, SubagentRequest,
        ToolCallId, ToolContext, TurnId,
    };

    fn context(call_id: &str) -> ToolContext {
        ToolContext {
            session_id: SessionId::new("subagent-test-session").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("subagent-test-incarnation").unwrap(),
            turn_id: TurnId::new("subagent-test-turn").unwrap(),
            call_id: ToolCallId::new(call_id).unwrap(),
        }
    }

    fn request(call_id: &str, name: &str, prompt: &str) -> SubagentRequest {
        SubagentRequest::new(context(call_id), name, prompt).expect("test request is bounded")
    }

    fn outcome(text: &str) -> SubagentOutcome {
        SubagentOutcome::new(text).expect("test outcome is bounded")
    }

    #[test]
    fn completed_steps_are_strict_and_requests_are_recorded_in_order() {
        let authority = ScriptedSubagentAuthority::new([
            SubagentStep::Complete(outcome("first result")),
            SubagentStep::Complete(outcome("second result")),
        ]);
        let first = request("call-1", "first", "inspect one");
        let second = request("call-2", "second", "inspect two");

        let first_result =
            block_on(authority.run(first.clone(), CancellationToken::new())).unwrap();
        let second_result =
            block_on(authority.run(second.clone(), CancellationToken::new())).unwrap();

        assert_eq!(first_result.text(), "first result");
        assert_eq!(second_result.text(), "second result");
        assert_eq!(authority.remaining_steps(), 0);
        let recorded = authority.requests();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].request, first);
        assert_eq!(recorded[1].request, second);
    }

    #[test]
    fn scripted_error_and_exhaustion_return_fixed_kinds() {
        let authority = ScriptedSubagentAuthority::new([SubagentStep::Error(
            SubagentAuthorityError::new(SubagentAuthorityErrorKind::Unavailable),
        )]);

        let scripted = block_on(authority.run(
            request("call-scripted", "scripted", "first"),
            CancellationToken::new(),
        ))
        .unwrap_err();
        let exhausted = block_on(authority.run(
            request("call-exhausted", "exhausted", "second"),
            CancellationToken::new(),
        ))
        .unwrap_err();

        assert_eq!(scripted.kind(), SubagentAuthorityErrorKind::Unavailable);
        assert_eq!(exhausted.kind(), SubagentAuthorityErrorKind::Failed);
        assert_eq!(authority.requests().len(), 2);
    }

    #[test]
    fn record_capacity_failure_does_not_consume_a_step() {
        let authority = ScriptedSubagentAuthority::with_record_capacity(
            [
                SubagentStep::Complete(outcome("first")),
                SubagentStep::Complete(outcome("retained")),
            ],
            1,
        );
        block_on(authority.run(
            request("call-first", "first", "first"),
            CancellationToken::new(),
        ))
        .unwrap();

        let error = block_on(authority.run(
            request("call-overflow", "overflow", "overflow"),
            CancellationToken::new(),
        ))
        .unwrap_err();

        assert_eq!(error.kind(), SubagentAuthorityErrorKind::ResourceLimit);
        assert_eq!(authority.requests().len(), 1);
        assert_eq!(authority.remaining_steps(), 1);
    }

    #[test]
    fn pending_step_waits_for_cancellation_and_returns_cancelled() {
        let authority = ScriptedSubagentAuthority::new([SubagentStep::Pending]);
        let cancellation = CancellationToken::new();
        let mut pending = authority.run(
            request("call-pending", "pending", "wait"),
            cancellation.clone(),
        );
        let mut context = Context::from_waker(noop_waker_ref());
        assert!(matches!(pending.as_mut().poll(&mut context), Poll::Pending));

        assert!(cancellation.cancel());
        let error = block_on(pending).unwrap_err();

        assert_eq!(error.kind(), SubagentAuthorityErrorKind::Cancelled);
        assert!(authority.requests()[0].cancellation.is_cancelled());
    }

    #[test]
    fn clones_share_strict_state_and_debug_redacts_payloads() {
        let authority = ScriptedSubagentAuthority::new([SubagentStep::Complete(outcome(
            "PRIVATE_OUTCOME_SENTINEL",
        ))]);
        let clone = authority.clone();
        let authority_before_run_debug = format!("{authority:?}");
        block_on(clone.run(
            request(
                "call-private",
                "PRIVATE_NAME_SENTINEL",
                "PRIVATE_PROMPT_SENTINEL",
            ),
            CancellationToken::new(),
        ))
        .unwrap();

        let authority_debug = format!("{authority:?}");
        let request_debug = format!("{:?}", authority.requests()[0]);
        for secret in [
            "PRIVATE_OUTCOME_SENTINEL",
            "PRIVATE_NAME_SENTINEL",
            "PRIVATE_PROMPT_SENTINEL",
        ] {
            assert!(!authority_debug.contains(secret));
            assert!(!authority_before_run_debug.contains(secret));
            assert!(!request_debug.contains(secret));
        }
        assert_eq!(authority.requests().len(), 1);
        assert_eq!(authority.remaining_steps(), 0);
    }
}
