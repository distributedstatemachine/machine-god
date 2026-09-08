//! Invocation-aware policy retains revocable admission through observer waits.
use futures_core::Stream;
use futures_util::{StreamExt, task::noop_waker};
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, Engine, EngineEvent, EventSink,
    EventSinkError, FilesystemAccess, ModelEvent, PermissionAuthorization, PermissionDecision,
    PermissionError, PermissionExecutionAdmission, PermissionGrantScope, PermissionHandler,
    PermissionInvocation, PermissionRequest, PreparedToolCall, Session, SessionId,
    SessionIncarnationId, StopReason, Tool, ToolCall, ToolCallId, ToolContext, ToolError,
    ToolInputLimits, ToolName, ToolOutput, ToolSpec, Turn, TurnEvent, TurnHandle,
};
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};
use serde_json::{Value, json};
use std::{
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

#[derive(Default)]
struct State {
    revoked: AtomicBool,
    admitted: AtomicUsize,
    dropped: AtomicUsize,
    constructed: AtomicUsize,
    executed: AtomicUsize,
    invocations: AtomicUsize,
    legacy: AtomicUsize,
    release: AtomicBool,
    waiting: AtomicBool,
    panic_drop: AtomicBool,
    pause_execution: AtomicBool,
    cancel_on_admit: Mutex<Option<TurnHandle>>,
    events: Mutex<Vec<EngineEvent>>,
}

struct Admission {
    state: Arc<State>,
    panic: bool,
}
impl PermissionExecutionAdmission for Admission {
    fn admit(self: Box<Self>) -> Result<(), PermissionError> {
        self.state.admitted.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panic, "fixture admission panic");
        if let Some(handle) = self.state.cancel_on_admit.lock().unwrap().take() {
            let _ = handle.cancel();
        }
        if self.state.revoked.load(Ordering::SeqCst) {
            return Err(PermissionError::new(
                "revoked",
                "private revocation details",
            ));
        }
        Ok(())
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        self.state.dropped.fetch_add(1, Ordering::SeqCst);
        assert!(
            !self.state.panic_drop.load(Ordering::SeqCst),
            "fixture admission drop panic"
        );
    }
}

fn allow() -> PermissionDecision {
    PermissionDecision::Allow {
        scope: PermissionGrantScope::Session,
    }
}
struct Policy {
    state: Arc<State>,
    deny: bool,
    panic: bool,
}
impl PermissionHandler for Policy {
    fn authorize(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        panic!("invocation override must be selected")
    }
    fn authorize_invocation<'a>(
        &'a self,
        request: PermissionRequest,
        invocation: PermissionInvocation<'a>,
    ) -> BoxFuture<'a, Result<PermissionAuthorization, PermissionError>> {
        Box::pin(async move {
            self.state.invocations.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.session_id.as_str(), "session");
            assert_eq!(request.session_incarnation_id.as_str(), "life");
            assert_eq!(invocation.tool_name.as_str(), "fixture");
            assert_eq!(invocation.call_id.as_str(), "call");
            assert_eq!(invocation.arguments, &json!({"normalized":"actual"}));
            assert_eq!(format!("{invocation:?}"), "PermissionInvocation { .. }");
            let authorization = PermissionAuthorization::new(if self.deny {
                PermissionDecision::Deny {
                    reason: "denied".into(),
                }
            } else {
                allow()
            })
            .with_admission(Admission {
                state: self.state.clone(),
                panic: self.panic,
            });
            assert_eq!(
                format!("{authorization:?}"),
                "PermissionAuthorization { .. }"
            );
            Ok(authorization)
        })
    }
}
struct LegacyPolicy(Arc<State>);
impl PermissionHandler for LegacyPolicy {
    fn authorize(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        self.0.legacy.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(allow()) })
    }
}

struct FixtureTool {
    state: Arc<State>,
    defers: bool,
    no_authority: bool,
}
impl Tool for FixtureTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("fixture").unwrap(),
            description: "fixture".into(),
            input_schema: json!({}),
        }
    }
    fn complete_input_limits(&self) -> Option<ToolInputLimits> {
        Some(ToolInputLimits {
            max_argument_bytes: NonZeroUsize::new(1024).unwrap(),
            max_argument_nodes: NonZeroUsize::new(128).unwrap(),
            max_prepared_argument_bytes: NonZeroUsize::new(1024).unwrap(),
            max_prepared_argument_nodes: NonZeroUsize::new(128).unwrap(),
        })
    }
    fn persist_arguments<'a>(
        &'a self,
        _: ToolContext,
        arguments: &'a Value,
        _: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        Box::pin(async move {
            assert_eq!(arguments, &json!({"unnormalized":"original"}));
            Ok(Some(json!({"archived":"projection"})))
        })
    }
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        assert_eq!(call.arguments, json!({"unnormalized":"original"}));
        let args = json!({"normalized":"actual"});
        let prepared = if self.no_authority {
            PreparedToolCall::without_authority(args)
        } else {
            PreparedToolCall::new(
                Capability::Filesystem {
                    access: FilesystemAccess::Write,
                    path: "file".into(),
                },
                args,
            )
        };
        Ok(if self.defers {
            prepared.completion_wins_after_first_poll()
        } else {
            prepared
        })
    }
    fn execute(
        &self,
        _: ToolContext,
        arguments: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        self.state.constructed.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert_eq!(arguments, json!({"normalized":"actual"}));
            if self.state.pause_execution.load(Ordering::SeqCst) {
                std::future::poll_fn(|_| {
                    self.state.waiting.store(true, Ordering::SeqCst);
                    if self.state.release.load(Ordering::SeqCst) {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
                .await;
            }
            self.state.executed.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput::success("done"))
        })
    }
}

#[derive(Clone, Copy)]
enum WaitAt {
    Never,
    Resolved,
    Started,
}
struct Sink {
    state: Arc<State>,
    at: WaitAt,
}
impl EventSink for Sink {
    fn emit(&self, event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        let wait = matches!(
            (&self.at, &event.payload),
            (WaitAt::Resolved, TurnEvent::PermissionResolved { .. })
                | (WaitAt::Started, TurnEvent::ToolStarted { .. })
        );
        Box::pin(async move {
            self.state.events.lock().unwrap().push(event);
            if wait {
                std::future::poll_fn(|_| {
                    self.state.waiting.store(true, Ordering::SeqCst);
                    if self.state.release.load(Ordering::SeqCst) {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
                .await;
            }
            Ok(())
        })
    }
}
#[derive(Clone, Copy)]
enum PolicyMode {
    Guarded,
    Deny,
    Panic,
    Legacy,
}
#[derive(Clone, Copy)]
enum ToolMode {
    Ordinary,
    Defers,
    NoAuthority,
}
fn session(
    state: &Arc<State>,
    at: WaitAt,
    policy_mode: PolicyMode,
    tool_mode: ToolMode,
) -> Session {
    let provider = ScriptedModelProvider::new(
        "fixture",
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("call").unwrap(),
                        name: ToolName::new("fixture").unwrap(),
                        arguments: json!({"unnormalized":"original"}),
                    },
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    );
    let policy: Arc<dyn PermissionHandler> = if matches!(policy_mode, PolicyMode::Legacy) {
        Arc::new(LegacyPolicy(state.clone()))
    } else {
        Arc::new(Policy {
            state: state.clone(),
            deny: matches!(policy_mode, PolicyMode::Deny),
            panic: matches!(policy_mode, PolicyMode::Panic),
        })
    };
    Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .shared_permission_handler(policy)
        .event_sink(Sink {
            state: state.clone(),
            at,
        })
        .tool(FixtureTool {
            state: state.clone(),
            defers: matches!(tool_mode, ToolMode::Defers),
            no_authority: matches!(tool_mode, ToolMode::NoAuthority),
        })
        .build()
        .unwrap()
        .create_session(
            SessionId::new("session").unwrap(),
            SessionIncarnationId::new("life").unwrap(),
        )
        .unwrap()
}
fn turn(session: &Session) -> Turn {
    futures_executor::block_on(session.prompt("root user request")).unwrap()
}
fn drain(turn: Turn) -> Vec<EngineEvent> {
    futures_executor::block_on(turn.collect::<Vec<_>>())
        .into_iter()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn wait(turn: &mut Turn, state: &State) {
    let waker = noop_waker();
    let mut context = Context::from_waker(&waker);
    for _ in 0..32 {
        if Pin::new(&mut *turn).poll_next(&mut context).is_pending() {
            assert!(state.waiting.load(Ordering::SeqCst));
            return;
        }
    }
    panic!("fixture did not reach observer wait")
}
fn unknown(session: &Session) {
    assert!(session.record().messages.iter().any(|m| m.content.iter().any(|c|
        matches!(c, ContentBlock::ToolResult {output, ..} if output.content["code"] == "tool_result_unknown"))));
}

#[test]
fn guarded_success_borrows_prepared_not_original_or_archived_arguments() {
    for mode in [ToolMode::Ordinary, ToolMode::Defers] {
        let state = Arc::new(State::default());
        let session = session(&state, WaitAt::Never, PolicyMode::Guarded, mode);
        let events = drain(turn(&session));
        assert!(matches!(
            events.last().unwrap().payload,
            TurnEvent::Completed {
                reason: StopReason::Completed,
                ..
            }
        ));
        assert_eq!(state.invocations.load(Ordering::SeqCst), 1);
        assert_eq!(state.admitted.load(Ordering::SeqCst), 1);
        assert_eq!(state.dropped.load(Ordering::SeqCst), 1);
        assert_eq!(state.constructed.load(Ordering::SeqCst), 1);
        assert_eq!(state.executed.load(Ordering::SeqCst), 1);
        assert!(session.record().messages.iter().any(|m| m.content.iter().any(|c|
            matches!(c, ContentBlock::ToolCall {call} if call.arguments == json!({"archived":"projection"})))));
    }
}

#[test]
fn revocation_during_either_observer_wait_prevents_execution_construction() {
    for at in [WaitAt::Resolved, WaitAt::Started] {
        for mode in [ToolMode::Ordinary, ToolMode::Defers] {
            let state = Arc::new(State::default());
            let session = session(&state, at, PolicyMode::Guarded, mode);
            let mut turn = turn(&session);
            wait(&mut turn, &state);
            assert_eq!(state.admitted.load(Ordering::SeqCst), 0);
            state.revoked.store(true, Ordering::SeqCst);
            state.release.store(true, Ordering::SeqCst);
            let events = drain(turn);
            assert!(
                matches!(&events.last().unwrap().payload, TurnEvent::Failed { component, code, message, .. }
                if component == "permission" && code == "permission_failed" && message == "permission policy failed")
            );
            assert_eq!(state.admitted.load(Ordering::SeqCst), 1);
            assert_eq!(state.dropped.load(Ordering::SeqCst), 1);
            assert_eq!(state.constructed.load(Ordering::SeqCst), 0);
            assert_eq!(state.executed.load(Ordering::SeqCst), 0);
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e.payload, TurnEvent::ToolFinished { .. }))
            );
            unknown(&session);
        }
    }
}

#[test]
fn cancellation_and_drop_while_observer_pending_release_without_admission() {
    for at in [WaitAt::Resolved, WaitAt::Started] {
        for cancel in [false, true] {
            for mode in [ToolMode::Ordinary, ToolMode::Defers] {
                let state = Arc::new(State::default());
                let session = session(&state, at, PolicyMode::Guarded, mode);
                let mut turn = turn(&session);
                wait(&mut turn, &state);
                if cancel {
                    assert!(turn.handle().cancel());
                    assert!(matches!(
                        drain(turn).last().unwrap().payload,
                        TurnEvent::Completed {
                            reason: StopReason::Cancelled,
                            ..
                        }
                    ));
                } else {
                    drop(turn);
                }
                assert_eq!(state.admitted.load(Ordering::SeqCst), 0);
                assert_eq!(state.dropped.load(Ordering::SeqCst), 1);
                assert_eq!(state.constructed.load(Ordering::SeqCst), 0);
                unknown(&session);
            }
        }
    }
}

#[test]
fn denial_discards_admission_without_execution() {
    let state = Arc::new(State::default());
    let session = session(&state, WaitAt::Never, PolicyMode::Deny, ToolMode::Ordinary);
    let events = drain(turn(&session));
    assert!(matches!(
        events.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
    assert_eq!(state.admitted.load(Ordering::SeqCst), 0);
    assert_eq!(state.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(state.constructed.load(Ordering::SeqCst), 0);
}

#[test]
fn legacy_and_no_authority_paths_do_not_gain_guard_requirements() {
    for no_authority in [false, true] {
        let state = Arc::new(State::default());
        let session = session(
            &state,
            WaitAt::Never,
            PolicyMode::Legacy,
            if no_authority {
                ToolMode::NoAuthority
            } else {
                ToolMode::Ordinary
            },
        );
        drain(turn(&session));
        assert_eq!(
            state.legacy.load(Ordering::SeqCst),
            usize::from(!no_authority)
        );
        assert_eq!(state.admitted.load(Ordering::SeqCst), 0);
        assert_eq!(state.executed.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn panicking_admission_unwinds_without_effects_and_drop_releases_turn() {
    let state = Arc::new(State::default());
    let session = session(&state, WaitAt::Never, PolicyMode::Panic, ToolMode::Ordinary);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drain(turn(&session)))).is_err()
    );
    assert_eq!(state.admitted.load(Ordering::SeqCst), 1);
    assert_eq!(state.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(state.constructed.load(Ordering::SeqCst), 0);
    unknown(&session);
    assert!(!session.has_active_turn());
}

#[test]
fn cancellation_from_admission_prevents_construction_in_both_execution_modes() {
    for mode in [ToolMode::Ordinary, ToolMode::Defers] {
        let state = Arc::new(State::default());
        let session = session(&state, WaitAt::Never, PolicyMode::Guarded, mode);
        let turn = turn(&session);
        *state.cancel_on_admit.lock().unwrap() = Some(turn.handle());
        assert!(matches!(
            drain(turn).last().unwrap().payload,
            TurnEvent::Completed {
                reason: StopReason::Cancelled,
                ..
            }
        ));
        assert_eq!(state.admitted.load(Ordering::SeqCst), 1);
        assert_eq!(state.constructed.load(Ordering::SeqCst), 0);
        assert_eq!(state.dropped.load(Ordering::SeqCst), 1);
        unknown(&session);
    }
}

#[test]
fn panicking_guard_drop_releases_turn_without_admitting() {
    let state = Arc::new(State::default());
    let session = session(
        &state,
        WaitAt::Started,
        PolicyMode::Guarded,
        ToolMode::Ordinary,
    );
    let mut turn = turn(&session);
    wait(&mut turn, &state);
    state.panic_drop.store(true, Ordering::SeqCst);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(turn))).is_err());
    assert_eq!(state.admitted.load(Ordering::SeqCst), 0);
    assert_eq!(state.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(state.constructed.load(Ordering::SeqCst), 0);
    assert!(!session.has_active_turn());
    unknown(&session);
}

#[test]
fn legacy_default_is_inert_and_returns_no_admission() {
    let observed = Arc::new(State::default());
    let session = session(
        &observed,
        WaitAt::Never,
        PolicyMode::Guarded,
        ToolMode::Ordinary,
    );
    let events = drain(turn(&session));
    let request = events
        .into_iter()
        .find_map(|e| match e.payload {
            TurnEvent::PermissionRequested { request } => Some(request),
            _ => None,
        })
        .unwrap();
    let state = Arc::new(State::default());
    let handler = LegacyPolicy(state.clone());
    let name = ToolName::new("fixture").unwrap();
    let id = ToolCallId::new("call").unwrap();
    let arguments = json!({"normalized":"actual"});
    let invocation = PermissionInvocation {
        tool_name: &name,
        call_id: &id,
        arguments: &arguments,
    };
    drop(handler.authorize_invocation(request.clone(), invocation));
    assert_eq!(state.legacy.load(Ordering::SeqCst), 0);
    let future = handler.authorize_invocation(request, invocation);
    assert_eq!(state.legacy.load(Ordering::SeqCst), 0);
    let result = futures_executor::block_on(future).unwrap();
    assert_eq!(result.decision, allow());
    assert!(result.admission.is_none());
    assert_eq!(state.legacy.load(Ordering::SeqCst), 1);
}

#[test]
fn guarded_pending_execution_preserves_completion_wins_and_cancellable_semantics() {
    for mode in [ToolMode::Ordinary, ToolMode::Defers] {
        let state = Arc::new(State::default());
        state.pause_execution.store(true, Ordering::SeqCst);
        let session = session(&state, WaitAt::Never, PolicyMode::Guarded, mode);
        let mut turn = turn(&session);
        wait(&mut turn, &state);
        assert_eq!(state.admitted.load(Ordering::SeqCst), 1);
        assert_eq!(state.constructed.load(Ordering::SeqCst), 1);
        assert_eq!(state.executed.load(Ordering::SeqCst), 0);
        assert!(turn.handle().cancel());
        if matches!(mode, ToolMode::Defers) {
            wait(&mut turn, &state);
        }
        state.release.store(true, Ordering::SeqCst);
        let events = drain(turn);
        assert!(matches!(
            events.last().unwrap().payload,
            TurnEvent::Completed {
                reason: StopReason::Cancelled,
                ..
            }
        ));
        let completed = matches!(mode, ToolMode::Defers);
        assert_eq!(
            state.executed.load(Ordering::SeqCst),
            usize::from(completed)
        );
        assert_eq!(state.admitted.load(Ordering::SeqCst), 1);
        assert_eq!(
            events
                .iter()
                .any(|e| matches!(e.payload, TurnEvent::ToolFinished { .. })),
            completed
        );
        if completed {
            assert!(session.record().messages.iter().any(|m| m.content.iter().any(|c|
                matches!(c, ContentBlock::ToolResult {output, ..} if output.content == json!("done")))));
        } else {
            unknown(&session);
        }
    }
}
