//! Engine-level admission and persistence tests for explicitly complete outputs.
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Engine, EngineEvent, EngineLimits, ModelEvent,
    PreparedToolCall, Session, SessionId, SessionIncarnationId, SessionStoreError,
    SessionStoreErrorKind, StopReason, Tool, ToolCall, ToolCallId, ToolContext, ToolError,
    ToolExecution, ToolName, ToolOutput, ToolOutputLimits, ToolSpec, Turn, TurnEvent,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
    SessionStoreScript, SessionStoreStep,
};
use serde_json::{Value, json};
use std::num::NonZeroUsize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::task::{Poll, Waker};

fn nz(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap()
}
fn size(output: &ToolOutput) -> usize {
    serde_json::to_vec(output).unwrap().len()
}
fn reference() -> ToolOutput {
    ToolOutput::success(json!({"result_id":"archive-call","bytes":70000}))
}
fn admission(bytes: usize, nodes: usize) -> ToolOutputLimits {
    ToolOutputLimits {
        max_serialized_bytes: nz(bytes),
        max_json_nodes: nz(nodes),
    }
}

#[derive(Default)]
struct Gate {
    pending: AtomicBool,
    waker: Mutex<Option<Waker>>,
}
impl Gate {
    fn release(&self) {
        self.pending.store(false, Ordering::SeqCst);
        let waker = self.waker.lock().unwrap().take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

#[derive(Clone)]
struct CompleteTool {
    factory: Arc<dyn Fn() -> ToolExecution + Send + Sync>,
    admission: Option<ToolOutputLimits>,
    completion_wins: bool,
    cancel_on_poll: bool,
    polls: Arc<AtomicUsize>,
    gate: Arc<Gate>,
}
impl CompleteTool {
    fn pair(complete: ToolOutput, durable: ToolOutput) -> Self {
        Self::factory(move || {
            ToolExecution::with_persisted_output(complete.clone(), durable.clone())
        })
    }
    fn factory(factory: impl Fn() -> ToolExecution + Send + Sync + 'static) -> Self {
        Self {
            factory: Arc::new(factory),
            admission: Some(admission(256 * 1024, 100_000)),
            completion_wins: false,
            cancel_on_poll: false,
            polls: Arc::new(AtomicUsize::new(0)),
            gate: Arc::new(Gate::default()),
        }
    }
}
impl Tool for CompleteTool {
    fn complete_output_limits(&self) -> Option<ToolOutputLimits> {
        self.admission
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("complete").unwrap(),
            description: "fixture".into(),
            input_schema: json!({"type":"object"}),
        }
    }
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        let prepared = PreparedToolCall::without_authority(call.arguments);
        Ok(if self.completion_wins {
            prepared.completion_wins_after_first_poll()
        } else {
            prepared
        })
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async { panic!("engine must use execute_for_turn") })
    }
    fn execute_for_turn(
        &self,
        _: ToolContext,
        _: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        Box::pin(async move {
            self.polls.fetch_add(1, Ordering::SeqCst);
            futures_util::future::poll_fn(|context| {
                if self.gate.pending.load(Ordering::SeqCst) {
                    *self.gate.waker.lock().unwrap() = Some(context.waker().clone());
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            if self.cancel_on_poll {
                cancellation.cancel();
            }
            Ok((self.factory)())
        })
    }
}

struct Fixture {
    session: Session,
    provider: ScriptedModelProvider,
    store: InMemorySessionStore,
}
impl Fixture {
    fn new(tool: CompleteTool, limits: EngineLimits, calls: usize) -> Self {
        Self::configured(tool, limits, calls, InMemorySessionStore::new(), 1)
    }
    fn configured(
        tool: CompleteTool,
        limits: EngineLimits,
        calls: usize,
        store: InMemorySessionStore,
        turns: usize,
    ) -> Self {
        let steps = (0..turns)
            .flat_map(|turn| {
                let mut requests = (0..calls)
                    .map(|index| ModelEvent::ToolCall {
                        call: ToolCall {
                            id: ToolCallId::new(format!("turn-{turn}-call-{index}")).unwrap(),
                            name: ToolName::new("complete").unwrap(),
                            arguments: json!({}),
                        },
                    })
                    .collect::<Vec<_>>();
                requests.push(ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                });
                [
                    ModelProviderStep::events(requests),
                    ModelProviderStep::events([ModelEvent::Stop {
                        reason: StopReason::Completed,
                    }]),
                ]
            })
            .collect::<Vec<_>>();
        let provider = ScriptedModelProvider::new("complete-output", steps);
        let engine = Engine::builder()
            .provider(provider.clone())
            .session_store(store.clone())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .tool(tool)
            .limits(limits)
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("complete-output").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            )
            .unwrap();
        Self {
            session,
            provider,
            store,
        }
    }
    fn run(&self) -> Vec<EngineEvent> {
        futures_executor::block_on(async {
            self.session
                .prompt("go")
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
    }
    fn durable(&self, index: usize) -> ToolOutput {
        let record = self
            .store
            .record(&SessionId::new("complete-output").unwrap())
            .unwrap();
        let ContentBlock::ToolResult { output, .. } = &record.messages[index + 2].content[0] else {
            panic!("tool result")
        };
        output.clone()
    }
    fn assert_failure(&self, events: &[EngineEvent], code: &str, completed: usize) {
        assert!(
            matches!(&events.last().unwrap().payload,TurnEvent::Failed {code:actual,..} if actual==code),
            "{events:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
                .count(),
            completed
        );
        let unknown = self.durable(completed);
        assert!(unknown.is_error);
        assert_eq!(unknown.content["code"], "tool_result_unknown");
        assert_eq!(self.provider.requests().len(), 1);
    }
}
fn assert_completed(events: &[EngineEvent]) {
    assert!(
        matches!(
            events.last().unwrap().payload,
            TurnEvent::Completed {
                reason: StopReason::Completed,
                ..
            }
        ),
        "{events:?}"
    );
}

#[test]
fn complete_event_exceeds_inline_limit_but_transcript_and_provider_only_receive_reference() {
    let full = ToolOutput::success("x".repeat(70_000));
    let durable = reference();
    let fixture = Fixture::new(
        CompleteTool::pair(full.clone(), durable.clone()),
        EngineLimits::default(),
        1,
    );
    let events = fixture.run();
    assert_completed(&events);
    assert!(events.iter().any(
        |event| matches!(&event.payload,TurnEvent::ToolFinished {output,..} if output==&full)
    ));
    assert_eq!(fixture.durable(0), durable);
    let requests = fixture.provider.requests();
    assert_eq!(requests.len(), 2);
    let ContentBlock::ToolResult { output, .. } = &requests[1].request.messages[2].content[0]
    else {
        panic!("provider reference")
    };
    assert_eq!(output, &durable);
    assert!(
        serde_json::to_vec(&requests[1].request.messages)
            .unwrap()
            .len()
            < 1024
    );
}

#[test]
fn persisted_representation_requires_explicit_tool_opt_in() {
    let mut tool = CompleteTool::pair(ToolOutput::success("small"), reference());
    tool.admission = None;
    let fixture = Fixture::new(tool, EngineLimits::default(), 1);
    fixture.assert_failure(&fixture.run(), "complete_tool_result_not_enabled", 0);
}

#[test]
fn opt_in_does_not_enlarge_ordinary_inline_output_admission() {
    let tool =
        CompleteTool::factory(|| ToolExecution::output(ToolOutput::success("x".repeat(70_000))));
    let fixture = Fixture::new(tool, EngineLimits::default(), 1);
    fixture.assert_failure(&fixture.run(), "tool_result_size_limit", 0);
}

#[test]
fn exact_serialized_complete_output_boundary_counts_wrapper_and_escaped_bytes() {
    let output = ToolOutput::success("\u{0001}".repeat(200));
    let exact = size(&output);
    for delta in [0, 1] {
        let mut tool = CompleteTool::pair(output.clone(), reference());
        tool.admission = Some(admission(exact - delta, 100));
        let fixture = Fixture::new(tool, EngineLimits::default(), 1);
        let events = fixture.run();
        if delta == 0 {
            assert_completed(&events);
        } else {
            fixture.assert_failure(&events, "complete_tool_result_size_limit", 0);
        }
    }
}

#[test]
fn exact_complete_node_boundary_is_independent_of_inline_nodes() {
    let output = ToolOutput::success(Value::Array(vec![Value::Null; 100]));
    for maximum in [101, 100] {
        let mut tool = CompleteTool::pair(output.clone(), reference());
        tool.admission = Some(admission(4096, maximum));
        let fixture = Fixture::new(
            tool,
            EngineLimits {
                max_json_nodes: nz(16),
                ..EngineLimits::default()
            },
            1,
        );
        let events = fixture.run();
        if maximum == 101 {
            assert_completed(&events);
        } else {
            fixture.assert_failure(&events, "json_node_limit", 0);
        }
    }
}

fn nested(depth: usize) -> Value {
    let mut value = Value::Null;
    for _ in 0..depth {
        value = Value::Array(vec![value]);
    }
    value
}
#[test]
fn complete_output_obeys_unchanged_depth_limit_at_exact_boundary() {
    for depth in [4, 5] {
        let fixture = Fixture::new(
            CompleteTool::pair(ToolOutput::success(nested(depth)), reference()),
            EngineLimits {
                max_json_depth: nz(4),
                ..EngineLimits::default()
            },
            1,
        );
        let events = fixture.run();
        if depth == 4 {
            assert_completed(&events);
        } else {
            fixture.assert_failure(&events, "json_depth_limit", 0);
        }
    }
}

#[test]
fn complete_aggregate_is_exact_and_independent_from_durable_aggregate() {
    let output = ToolOutput::success("x".repeat(5000));
    let exact = 2 * size(&output);
    for delta in [0, 1] {
        let fixture = Fixture::new(
            CompleteTool::pair(output.clone(), reference()),
            EngineLimits {
                max_cumulative_complete_tool_result_bytes: nz(exact - delta),
                max_cumulative_tool_result_bytes: nz(1024),
                ..EngineLimits::default()
            },
            2,
        );
        let events = fixture.run();
        if delta == 0 {
            assert_completed(&events);
        } else {
            fixture.assert_failure(&events, "cumulative_complete_tool_result_size_limit", 1);
        }
    }
}

#[test]
fn complete_aggregate_resets_each_turn_and_defaults_remain_enforced() {
    let output = ToolOutput::success("x".repeat(5000));
    let fixture = Fixture::configured(
        CompleteTool::pair(output.clone(), reference()),
        EngineLimits {
            max_cumulative_complete_tool_result_bytes: nz(size(&output)),
            ..EngineLimits::default()
        },
        1,
        InMemorySessionStore::new(),
        2,
    );
    assert_completed(&fixture.run());
    assert_completed(&fixture.run());
    assert_eq!(fixture.provider.requests().len(), 4);
    let mut tool = CompleteTool::pair(ToolOutput::success("x".repeat(300 * 1024)), reference());
    tool.admission = Some(admission(512 * 1024, 100_000));
    let fixture = Fixture::new(tool, EngineLimits::default(), 1);
    fixture.assert_failure(
        &fixture.run(),
        "cumulative_complete_tool_result_size_limit",
        0,
    );
}

#[test]
fn reference_replacement_failure_does_not_emit_complete_success() {
    let store = InMemorySessionStore::configured(
        std::collections::BTreeMap::new(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Error(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "replace_failed",
                    "fixture",
                    false,
                )),
            ]),
        },
        32,
    );
    let tool = CompleteTool::pair(ToolOutput::success("x".repeat(70_000)), reference());
    let fixture = Fixture::configured(tool.clone(), EngineLimits::default(), 1, store, 1);
    fixture.assert_failure(&fixture.run(), "store_failed", 0);
    assert_eq!(tool.polls.load(Ordering::SeqCst), 1);
}

#[test]
fn invalid_committed_output_fails_validation_even_after_cancellation() {
    let mut tool = CompleteTool::pair(ToolOutput::success("x".repeat(70_000)), reference());
    tool.admission = Some(admission(1024, 100));
    tool.completion_wins = true;
    tool.cancel_on_poll = true;
    let fixture = Fixture::new(tool, EngineLimits::default(), 1);
    fixture.assert_failure(&fixture.run(), "complete_tool_result_size_limit", 0);
}

#[test]
fn durable_representation_still_obeys_inline_byte_node_depth_and_aggregate_limits() {
    let cases = [
        (
            ToolOutput::success("x".repeat(70_000)),
            EngineLimits::default(),
            "tool_result_size_limit",
            1,
        ),
        (
            ToolOutput::success(Value::Array(vec![Value::Null; 100])),
            EngineLimits {
                max_json_nodes: nz(16),
                ..EngineLimits::default()
            },
            "json_node_limit",
            1,
        ),
        (
            ToolOutput::success(nested(5)),
            EngineLimits {
                max_json_depth: nz(4),
                ..EngineLimits::default()
            },
            "json_depth_limit",
            1,
        ),
        (
            ToolOutput::success("x".repeat(1000)),
            EngineLimits {
                max_cumulative_tool_result_bytes: nz(1500),
                ..EngineLimits::default()
            },
            "cumulative_tool_result_size_limit",
            2,
        ),
    ];
    for (durable, limits, code, calls) in cases {
        let fixture = Fixture::new(
            CompleteTool::pair(ToolOutput::success("full"), durable),
            limits,
            calls,
        );
        fixture.assert_failure(&fixture.run(), code, calls - 1);
    }
}

#[test]
fn complete_and_durable_error_status_must_match_both_directions() {
    for full_error in [false, true] {
        let fixture = Fixture::new(
            CompleteTool::pair(
                ToolOutput {
                    content: json!("full"),
                    is_error: full_error,
                },
                ToolOutput {
                    content: json!("ref"),
                    is_error: !full_error,
                },
            ),
            EngineLimits::default(),
            1,
        );
        fixture.assert_failure(&fixture.run(), "complete_tool_result_status_mismatch", 0);
    }
    let fixture = Fixture::new(
        CompleteTool::pair(
            ToolOutput {
                content: json!("full error"),
                is_error: true,
            },
            ToolOutput {
                content: json!("error reference"),
                is_error: true,
            },
        ),
        EngineLimits::default(),
        1,
    );
    assert_completed(&fixture.run());
    assert!(fixture.durable(0).is_error);
}

fn start_turn(fixture: &Fixture) -> Turn {
    let mut turn = futures_executor::block_on(fixture.session.prompt("go")).unwrap();
    loop {
        if matches!(next(&mut turn).payload, TurnEvent::ToolStarted { .. }) {
            return turn;
        }
    }
}
fn next(turn: &mut Turn) -> EngineEvent {
    futures_executor::block_on(turn.next()).unwrap().unwrap()
}
fn poll_pending(turn: &mut Turn) {
    futures_executor::block_on(async {
        assert!(futures_util::poll!(turn.next()).is_pending());
    });
}

#[test]
fn completion_wins_retains_large_event_and_durable_reference_after_same_poll_cancellation() {
    let full = ToolOutput::success("x".repeat(70_000));
    let mut tool = CompleteTool::pair(full.clone(), reference());
    tool.completion_wins = true;
    tool.cancel_on_poll = true;
    let fixture = Fixture::new(tool, EngineLimits::default(), 1);
    let events = fixture.run();
    assert!(events.iter().any(
        |event| matches!(&event.payload,TurnEvent::ToolFinished {output,..} if output==&full)
    ));
    assert!(matches!(
        events.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(fixture.durable(0), reference());
    assert_eq!(fixture.provider.requests().len(), 1);
}

#[test]
fn completion_wins_obeys_pre_poll_and_pending_cancellation_boundaries() {
    let mut tool = CompleteTool::pair(ToolOutput::success("x".repeat(70_000)), reference());
    tool.completion_wins = true;
    let fixture = Fixture::new(tool.clone(), EngineLimits::default(), 1);
    let mut turn = start_turn(&fixture);
    assert!(turn.handle().cancel());
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(tool.polls.load(Ordering::SeqCst), 0);
    let mut tool = CompleteTool::pair(ToolOutput::success("x".repeat(70_000)), reference());
    tool.completion_wins = true;
    tool.gate.pending.store(true, Ordering::SeqCst);
    let fixture = Fixture::new(tool.clone(), EngineLimits::default(), 1);
    let mut turn = start_turn(&fixture);
    poll_pending(&mut turn);
    assert_eq!(tool.polls.load(Ordering::SeqCst), 1);
    assert!(turn.handle().cancel());
    poll_pending(&mut turn);
    tool.gate.release();
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::ToolFinished { .. }
    ));
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(fixture.durable(0), reference());
}

#[test]
fn ordinary_cancellation_does_not_publish_a_complete_event_or_reference() {
    let mut tool = CompleteTool::pair(ToolOutput::success("x".repeat(70_000)), reference());
    tool.cancel_on_poll = true;
    let fixture = Fixture::new(tool, EngineLimits::default(), 1);
    let events = fixture.run();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
    );
    assert!(matches!(
        events.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(fixture.durable(0).content["code"], "tool_result_unknown");
}

const DEEP_CASE: &str = "MACHINE_GOD_COMPLETE_OUTPUT_DEEP_CASE";
#[test]
fn deep_complete_and_durable_rejections_are_stack_safe() {
    for case in ["complete", "durable", "cancel_ready"] {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "deep_complete_output_child", "--nocapture"])
            .env(DEEP_CASE, case)
            .status()
            .unwrap();
        assert!(status.success(), "deep output case {case}");
    }
}
#[test]
fn deep_complete_output_child() {
    let Ok(case) = std::env::var(DEEP_CASE) else {
        return;
    };
    let durable_deep = case == "durable";
    let mut tool = CompleteTool::factory(move || {
        let deep = ToolOutput::success(nested(50_000));
        if durable_deep {
            ToolExecution::with_persisted_output(ToolOutput::success("full"), deep)
        } else {
            ToolExecution::with_persisted_output(deep, reference())
        }
    });
    tool.cancel_on_poll = case == "cancel_ready";
    let fixture = Fixture::new(tool, EngineLimits::default(), 1);
    let events = fixture.run();
    if case == "cancel_ready" {
        assert!(matches!(
            events.last().unwrap().payload,
            TurnEvent::Completed {
                reason: StopReason::Cancelled,
                ..
            }
        ));
    } else {
        fixture.assert_failure(&events, "json_depth_limit", 0);
    }
}
