use crate::DEFAULT_RECORD_CAPACITY;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
        ScriptedPermissionHandler,
    };
    use futures_executor::block_on;
    use futures_util::StreamExt;
    use machine_god_core::{
        Engine, ManagedOutcome, ManagedReceipt, ManagedRequested, ManagedResultStatus, ModelEvent,
        PermissionDecision, PermissionGrantScope, Session, SessionId, SessionIncarnationId,
        StopReason, SubagentTool, ToolCall, ToolCallId, ToolName,
    };
    use serde_json::json;

    fn result() -> ManagedSubagentResult {
        ManagedSubagentResult {
            ok: true,
            operation_id: "op".into(),
            child_id: Some("child".into()),
            status: ManagedResultStatus::Created,
            error_code: None,
            retryable: false,
            requested: Some(ManagedRequested::Receipt(ManagedReceipt {
                outcome: ManagedOutcome::Created,
                generation: 1,
                event_sequence: 1,
            })),
            cursor: None,
        }
    }
    fn session(authority: ScriptedSubagentAuthority) -> (Engine, Session) {
        let call = ToolCall {
            id: ToolCallId::new("call").unwrap(),
            name: ToolName::new("subagent").unwrap(),
            arguments: json!({"command":{"create":{"name":"PRIVATE_NAME","mode":"persistent"}}}),
        };
        let engine = Engine::builder()
            .provider(ScriptedModelProvider::new(
                "managed-test",
                [
                    ModelProviderStep::events([
                        ModelEvent::ToolCall { call },
                        ModelEvent::Stop {
                            reason: StopReason::ToolCalls,
                        },
                    ]),
                    ModelProviderStep::events([ModelEvent::Stop {
                        reason: StopReason::Completed,
                    }]),
                ],
            ))
            .session_store(InMemorySessionStore::default())
            .permission_handler(ScriptedPermissionHandler::new([PermissionStep::Decision(
                PermissionDecision::Allow {
                    scope: PermissionGrantScope::Once,
                },
            )]))
            .tool(SubagentTool::new(authority))
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("test").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            )
            .unwrap();
        (engine, session)
    }
    #[test]
    fn actual_turn_is_required_and_recording_does_not_retain_authority() {
        let authority = ScriptedSubagentAuthority::new([SubagentStep::Complete(result())]);
        let (engine, session) = session(authority.clone());
        let turn = block_on(session.prompt("task")).unwrap();
        let witness = turn.witness();
        let session_witness = session.witness();
        authority.bind_turn(witness.clone());
        assert!(authority.requests().is_empty());
        assert_eq!(authority.remaining_steps(), 1);
        let events = block_on(turn.collect::<Vec<_>>());
        assert!(events.iter().all(Result::is_ok));
        assert_eq!(authority.requests().len(), 1);
        assert_eq!(authority.remaining_steps(), 0);
        assert!(!format!("{authority:?} {:?}", authority.requests()).contains("PRIVATE_NAME"));
        assert!(!witness.is_live());
        drop(session);
        drop(engine);
        assert!(!session_witness.is_live());
    }
    #[test]
    fn missing_witness_and_zero_capacity_preserve_script() {
        for bind in [false, true] {
            let authority = ScriptedSubagentAuthority::with_record_capacity(
                [SubagentStep::Complete(result())],
                0,
            );
            let (_engine, session) = session(authority.clone());
            let turn = block_on(session.prompt("task")).unwrap();
            if bind {
                authority.bind_turn(turn.witness());
            }
            block_on(turn.collect::<Vec<_>>());
            assert!(authority.requests().is_empty());
            assert_eq!(authority.remaining_steps(), 1);
        }
    }
}

use machine_god_core::{
    BoxFuture, CancellationToken, ManagedSubagentAuthority, ManagedSubagentCommand,
    ManagedSubagentError, ManagedSubagentInvocation, ManagedSubagentResult, ToolContext,
    TurnWitness,
};
use std::{
    collections::VecDeque,
    fmt,
    sync::{Arc, Mutex},
};

/// One manager response, not a foreground child's final answer.
#[derive(Clone)]
pub enum SubagentStep {
    Complete(ManagedSubagentResult),
    Error(ManagedSubagentError),
    Pending,
}
impl fmt::Debug for SubagentStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Complete(_) => "Complete(..)",
            Self::Error(_) => "Error(..)",
            Self::Pending => "Pending",
        })
    }
}
/// Data-only recording deliberately drops the opaque invocation proof.
#[derive(Clone)]
pub struct RecordedSubagentRequest {
    pub context: ToolContext,
    pub command: ManagedSubagentCommand,
    pub cancellation: CancellationToken,
}
impl fmt::Debug for RecordedSubagentRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordedSubagentRequest")
            .finish_non_exhaustive()
    }
}
struct State {
    steps: VecDeque<SubagentStep>,
    requests: Vec<RecordedSubagentRequest>,
    turn: Option<TurnWitness>,
}
struct Inner {
    capacity: usize,
    state: Mutex<State>,
}
/// Strict bounded manager double. Bind an actual Turn witness before polling;
/// forged or stale execution cannot consume scripted behavior. No child runner.
#[derive(Clone)]
pub struct ScriptedSubagentAuthority {
    inner: Arc<Inner>,
}
impl ScriptedSubagentAuthority {
    #[must_use]
    pub fn new(steps: impl IntoIterator<Item = SubagentStep>) -> Self {
        Self::with_record_capacity(steps, DEFAULT_RECORD_CAPACITY)
    }
    #[must_use]
    pub fn with_record_capacity(
        steps: impl IntoIterator<Item = SubagentStep>,
        capacity: usize,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                capacity,
                state: Mutex::new(State {
                    steps: steps.into_iter().collect(),
                    requests: vec![],
                    turn: None,
                }),
            }),
        }
    }
    /// Test host supplies a real turn; the retained observation is weak.
    pub fn bind_turn(&self, turn: TurnWitness) {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .turn = Some(turn);
    }
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedSubagentRequest> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .requests
            .clone()
    }
    #[must_use]
    pub fn remaining_steps(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .steps
            .len()
    }
}
impl fmt::Debug for ScriptedSubagentAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptedSubagentAuthority")
            .field("capacity", &self.inner.capacity)
            .finish_non_exhaustive()
    }
}
impl ManagedSubagentAuthority for ScriptedSubagentAuthority {
    fn execute(
        &self,
        invocation: ManagedSubagentInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ManagedSubagentResult, ManagedSubagentError>> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(ManagedSubagentError::Cancelled);
            }
            let step = {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !state
                    .turn
                    .as_ref()
                    .is_some_and(|turn| invocation.claim(turn))
                {
                    return Err(ManagedSubagentError::Unavailable);
                }
                if state.requests.len() >= self.inner.capacity {
                    return Err(ManagedSubagentError::ResourceLimit);
                }
                state.requests.push(RecordedSubagentRequest {
                    context: invocation.context().clone(),
                    command: invocation.command().clone(),
                    cancellation: cancellation.clone(),
                });
                state
                    .steps
                    .pop_front()
                    .ok_or(ManagedSubagentError::Failed)?
            };
            match step {
                SubagentStep::Complete(result) => Ok(result),
                SubagentStep::Error(error) => Err(error),
                SubagentStep::Pending => {
                    cancellation.cancelled().await;
                    Err(ManagedSubagentError::Cancelled)
                }
            }
        })
    }
}
