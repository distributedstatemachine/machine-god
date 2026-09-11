//! Explicit provider-neutral stop-after-tool orchestration, never JSON commands.
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Engine, EngineEvent, ModelEvent, Session,
    SessionId, SessionIncarnationId, SessionRecord, SessionStoreError, SessionStoreErrorKind,
    StopReason, TokenUsage, Tool, ToolCall, ToolCallId, ToolError, ToolErrorKind, ToolExecution,
    ToolName, ToolOutput, ToolOutputLimits, Turn, TurnEvent, TurnToolRegistration,
};
use machine_god_testkit::{
    InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler, SessionStoreScript,
    SessionStoreStep,
};
use serde_json::{Value, json};
use std::sync::Arc;

#[path = "tool_turn_completion/fixture.rs"]
mod fixture;
use fixture::*;

struct RejectFinished;
impl machine_god_core::EventSink for RejectFinished {
    fn emit(
        &self,
        event: EngineEvent,
    ) -> BoxFuture<'_, Result<(), machine_god_core::EventSinkError>> {
        Box::pin(async move {
            if matches!(event.payload, TurnEvent::ToolFinished { .. }) {
                Err(machine_god_core::EventSinkError::new("fixture", "rejected"))
            } else {
                Ok(())
            }
        })
    }
}

#[test]
fn observer_failure_preserves_saved_result_without_successful_stop() {
    let fixture = Fixture::with_sink(
        FinishTool::new(|_| Ok(ToolExecution::output(ToolOutput::success("done")).finish_turn())),
        2,
        InMemorySessionStore::new(),
        Arc::new(RejectFinished),
    );
    let events = block_on(fixture.start().collect::<Vec<_>>());
    assert!(matches!(
        events.last(),
        Some(Err(machine_god_core::EngineError::EventSink(_)))
    ));
    assert!(!events.iter().any(|event| matches!(
        event,
        Ok(EngineEvent {
            payload: TurnEvent::Completed { .. },
            ..
        })
    )));
    assert_eq!(fixture.output(0), ToolOutput::success("done"));
    assert_eq!(fixture.output(1).content["code"], "tool_result_unknown");
    fixture.assert_calls(1);
    assert_eq!(fixture.provider.requests().len(), 1);
}

#[test]
fn completion_wins_failed_persistence_beats_pending_cancellation() {
    let store = InMemorySessionStore::configured(
        std::collections::BTreeMap::new(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Error(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "failed",
                    "fixture",
                    false,
                )),
            ]),
        },
        32,
    );
    let mut tool = FinishTool::new(|_| {
        Ok(ToolExecution::output(ToolOutput::success("committed")).finish_turn())
    });
    tool.completion_wins = true;
    tool.cancel_on_poll = true;
    let fixture = Fixture::with_store(tool, 2, store);
    let events = fixture.run();
    assert!(
        matches!(&events.last().unwrap().payload, TurnEvent::Failed { code, .. } if code == "store_failed")
    );
    assert_eq!(finished(&events), 0);
    fixture.assert_calls(1);
    assert_eq!(fixture.output(0).content["code"], "tool_result_unknown");
}

#[test]
fn pending_result_save_remains_cancellable_despite_stop_directive() {
    let store = InMemorySessionStore::configured(
        std::collections::BTreeMap::new(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Pending,
            ]),
        },
        32,
    );
    let fixture = Fixture::with_store(
        FinishTool::new(|_| Ok(ToolExecution::output(ToolOutput::success("done")).finish_turn())),
        2,
        store,
    );
    let mut turn = fixture.start();
    loop {
        if matches!(next(&mut turn).payload, TurnEvent::ToolStarted { .. }) {
            break;
        }
    }
    block_on(async {
        assert!(futures_util::poll!(turn.next()).is_pending());
    });
    assert!(turn.handle().cancel());
    let events = collect(turn);
    completed(&events, &StopReason::Cancelled);
    assert_eq!(finished(&events), 0);
    fixture.assert_calls(1);
    assert_eq!(fixture.output(0).content["code"], "tool_result_unknown");
}

struct UnusedRegistration;
impl Tool for UnusedRegistration {
    fn spec(&self) -> machine_god_core::ToolSpec {
        machine_god_core::ToolSpec {
            name: ToolName::new("extra").unwrap(),
            description: "fixture".into(),
            input_schema: json!({"type":"object"}),
        }
    }
    fn execute(
        &self,
        _: machine_god_core::ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        panic!("registration must not execute")
    }
}
#[test]
fn accepted_registration_drops_with_stopped_turn_without_another_round() {
    let observed = Arc::new(std::sync::Mutex::new(None));
    let captured = observed.clone();
    let fixture = Fixture::new(
        FinishTool::new(move |_| {
            let registration = Arc::new(TurnToolRegistration::new(UnusedRegistration));
            *captured.lock().unwrap() = Some(Arc::downgrade(&registration));
            Ok(
                ToolExecution::with_next_round_tool(ToolOutput::success("selected"), registration)
                    .finish_turn(),
            )
        }),
        1,
    );
    let events = fixture.run();
    completed(&events, &StopReason::Completed);
    assert!(
        observed
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .is_none()
    );
    assert_eq!(finished(&events), 1);
    assert_eq!(fixture.provider.requests().len(), 1);
}

#[test]
fn explicit_stop_preserves_ordered_prefix_unknown_siblings_and_usage() {
    for stop_at in 0..3 {
        let fixture = Fixture::new(
            FinishTool::new(move |index| {
                let result = ToolExecution::output(ToolOutput::success(json!({"executed":index})));
                Ok(if index == stop_at {
                    result.finish_turn()
                } else {
                    result
                })
            }),
            3,
        );
        let events = fixture.run();
        completed(&events, &StopReason::Completed);
        assert_eq!(fixture.provider.requests().len(), 1);
        fixture.assert_calls(stop_at + 1);
        assert_eq!(finished(&events), stop_at + 1);
        assert_eq!(fixture.record().messages.len(), 5);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.payload,
                    TurnEvent::Model {
                        event: ModelEvent::Stop { .. }
                    }
                ))
                .count(),
            1,
            "no invented provider Stop"
        );
        assert!(matches!(
            events.last().unwrap().payload,
            TurnEvent::Completed {
                usage: TokenUsage {
                    input_tokens: 3,
                    output_tokens: 5,
                    ..
                },
                ..
            }
        ));
        for index in 0..3 {
            let output = fixture.output(index);
            if index <= stop_at {
                assert_eq!(output.content["executed"], index);
                assert!(!output.is_error);
            } else {
                assert_eq!(output.content["code"], "tool_result_unknown");
                assert!(output.is_error);
            }
        }
    }
}

#[test]
fn error_bearing_output_can_end_turn_without_becoming_successful_output() {
    let output = ToolOutput {
        content: json!({"awaiting_input":true}),
        is_error: true,
    };
    let expected = output.clone();
    let fixture = Fixture::new(
        FinishTool::new(move |_| Ok(ToolExecution::output(output.clone()).finish_turn())),
        2,
    );
    let events = fixture.run();
    completed(&events, &StopReason::Completed);
    assert_eq!(fixture.output(0), expected);
    assert_eq!(fixture.provider.requests().len(), 1);
    fixture.assert_calls(1);
}

#[test]
fn ordinary_errors_and_json_fields_never_request_a_stop() {
    for error in [false, true] {
        let fixture = Fixture::new(
            FinishTool::new(move |_| {
                if error {
                    Err(ToolError::new(
                        ToolErrorKind::Unavailable,
                        "finish_turn",
                        "McpInputRequired",
                        false,
                    ))
                } else {
                    Ok(ToolExecution::output(ToolOutput::success(
                        json!({"finish_turn":true,"status_detail":"McpInputRequired"}),
                    )))
                }
            }),
            2,
        );
        let events = fixture.run();
        completed(&events, &StopReason::Completed);
        fixture.assert_calls(2);
        assert_eq!(fixture.provider.requests().len(), 2);
        assert_eq!(finished(&events), 2);
    }
}

#[test]
fn stop_preserves_complete_event_and_exact_persisted_archive_reference() {
    let full = ToolOutput::success("x".repeat(70000));
    let complete = full.clone();
    let reference = ToolOutput::success(json!({"fixture_archive":"complete"}));
    let persisted = reference.clone();
    let fixture = Fixture::new(
        FinishTool::new(move |_| {
            Ok(
                ToolExecution::with_persisted_output(complete.clone(), persisted.clone())
                    .finish_turn(),
            )
        }),
        1,
    );
    let events = fixture.run();
    completed(&events, &StopReason::Completed);
    assert_eq!(fixture.output(0), reference);
    assert!(events.iter().any(
        |event| matches!(&event.payload, TurnEvent::ToolFinished { output, .. } if output == &full)
    ));
    assert_eq!(fixture.provider.requests().len(), 1);
}

#[test]
fn same_poll_cancellation_wins_after_ordinary_or_completion_wins_tool() {
    for completion_wins in [false, true] {
        let mut tool = FinishTool::new(|_| {
            Ok(ToolExecution::output(ToolOutput::success("committed")).finish_turn())
        });
        tool.cancel_on_poll = true;
        tool.completion_wins = completion_wins;
        let fixture = Fixture::new(tool, 2);
        let events = fixture.run();
        completed(&events, &StopReason::Cancelled);
        assert_eq!(finished(&events), usize::from(completion_wins));
        if completion_wins {
            assert_eq!(fixture.output(0).content, "committed");
        } else {
            assert_eq!(fixture.output(0).content["code"], "tool_result_unknown");
        }
        fixture.assert_calls(1);
        assert_eq!(fixture.provider.requests().len(), 1);
    }
}

#[test]
fn cancellation_after_tool_finished_is_not_relabelled_completed() {
    let fixture = Fixture::new(
        FinishTool::new(|_| Ok(ToolExecution::output(ToolOutput::success("done")).finish_turn())),
        2,
    );
    let mut turn = fixture.start();
    loop {
        if matches!(next(&mut turn).payload, TurnEvent::ToolFinished { .. }) {
            break;
        }
    }
    assert!(turn.handle().cancel());
    completed(&collect(turn), &StopReason::Cancelled);
    fixture.assert_calls(1);
    assert_eq!(fixture.output(0).content, "done");
}

#[test]
fn failed_result_save_and_output_bounds_never_establish_successful_stop() {
    let store = InMemorySessionStore::configured(
        std::collections::BTreeMap::new(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Error(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "failed",
                    "fixture",
                    false,
                )),
            ]),
        },
        32,
    );
    let fixture = Fixture::with_store(
        FinishTool::new(|_| Ok(ToolExecution::output(ToolOutput::success("done")).finish_turn())),
        2,
        store,
    );
    let events = fixture.run();
    assert!(
        matches!(&events.last().unwrap().payload, TurnEvent::Failed { code, .. } if code == "store_failed")
    );
    assert_eq!(finished(&events), 0);
    fixture.assert_calls(1);
    assert_eq!(fixture.output(0).content["code"], "tool_result_unknown");
    let fixture = Fixture::new(
        FinishTool::new(|_| {
            Ok(ToolExecution::with_persisted_output(
                ToolOutput::success("x".repeat(300_000)),
                ToolOutput::success("archive"),
            )
            .finish_turn())
        }),
        1,
    );
    let events = fixture.run();
    assert!(
        matches!(&events.last().unwrap().payload, TurnEvent::Failed { code, .. } if code == "complete_tool_result_size_limit")
    );
    assert_eq!(finished(&events), 0);
    assert_eq!(fixture.output(0).content["code"], "tool_result_unknown");
}

#[test]
fn finish_directive_does_not_bypass_dynamic_registration_validation() {
    let fixture = Fixture::new(
        FinishTool::new(|_| {
            let conflicting = Arc::new(TurnToolRegistration::new(FinishTool::new(|_| {
                panic!("never executed")
            })));
            Ok(
                ToolExecution::with_next_round_tool(ToolOutput::success("selected"), conflicting)
                    .finish_turn(),
            )
        }),
        1,
    );
    let events = fixture.run();
    assert!(matches!(
        events.last().unwrap().payload,
        TurnEvent::Failed { .. }
    ));
    assert_eq!(finished(&events), 0);
    assert_eq!(fixture.output(0).content["code"], "tool_result_unknown");
    assert_eq!(fixture.provider.requests().len(), 1);
}
