use futures_util::FutureExt;
use machine_god_core::{
    CancellationToken, Capability, SessionId, SessionIncarnationId, Tool, ToolCall, ToolCallId,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec, TurnId,
};
use machine_god_testkit::{
    RecordedToolPreparation, RecordedToolPreparationWithContext, ScriptedPreparedTool,
    ToolPrepareStep, ToolStep,
};
use serde_json::json;

fn call(index: usize) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(format!("call-{index}")).unwrap(),
        name: ToolName::new("fixture").unwrap(),
        arguments: json!({"raw": index}),
    }
}

fn context(index: usize) -> ToolContext {
    ToolContext {
        session_id: SessionId::new(format!("session-{index}")).unwrap(),
        session_incarnation_id: SessionIncarnationId::new(format!("incarnation-{index}")).unwrap(),
        turn_id: TurnId::new(format!("turn-{index}")).unwrap(),
        call_id: call(index).id,
    }
}

fn step(index: usize) -> ToolPrepareStep {
    ToolPrepareStep::NoAuthority {
        arguments: json!({"prepared": index}),
    }
}

fn fixture(
    steps: impl IntoIterator<Item = ToolPrepareStep>,
    capacity: usize,
) -> ScriptedPreparedTool {
    ScriptedPreparedTool::with_record_capacity(
        ToolSpec {
            name: call(0).name,
            description: "context recording fixture".to_owned(),
            input_schema: json!({"type": "object"}),
        },
        steps,
        [ToolStep::Output(ToolOutput::success("done"))],
        capacity,
    )
}

#[test]
fn exact_context_is_recorded_without_inferring_it_from_the_call() {
    let tool = fixture((0..5).map(step), 5);
    let base = context(0);
    let alternate = context(1);
    let contexts = [
        base.clone(),
        ToolContext {
            session_id: alternate.session_id,
            ..base.clone()
        },
        ToolContext {
            session_incarnation_id: alternate.session_incarnation_id,
            ..base.clone()
        },
        ToolContext {
            turn_id: alternate.turn_id,
            ..base.clone()
        },
        ToolContext {
            call_id: alternate.call_id,
            ..base
        },
    ];
    let erased: &dyn Tool = &tool;
    for (index, expected) in contexts.iter().enumerate() {
        let prepared = erased.prepare_for_turn(expected, call(0)).unwrap();
        assert_eq!(prepared.arguments(), &json!({"prepared": index}));
    }
    let records: Vec<RecordedToolPreparationWithContext> = tool.preparations_with_context();
    assert_eq!(records.len(), contexts.len());
    for (record, expected) in records.iter().zip(contexts) {
        assert_eq!(record.context, Some(expected));
        assert_eq!(record.call, call(0));
    }
    assert!(tool.invocations().is_empty());
    assert_eq!(tool.remaining_steps(), (0, 1));
}

#[test]
fn direct_and_contextual_preparation_share_order_and_strict_responses() {
    let failure = ToolError::new(ToolErrorKind::InvalidInput, "fixture", "rejected", false);
    let capability = Capability::Custom {
        name: "fixture".to_owned(),
        details: json!({}),
    };
    let tool = fixture(
        [
            ToolPrepareStep::Prepared {
                capability: capability.clone(),
                arguments: json!({"prepared": 0}),
            },
            ToolPrepareStep::Error(failure.clone()),
            step(2),
        ],
        3,
    );
    // This public struct literal must remain source compatible.
    let legacy = RecordedToolPreparation { call: call(0) };
    let first = tool.prepare(legacy.call.clone()).unwrap();
    assert_eq!(first.capability(), Some(&capability));
    assert_eq!(first.arguments(), &json!({"prepared": 0}));
    assert_eq!(
        tool.prepare_for_turn(&context(1), call(1)).unwrap_err(),
        failure
    );
    let third = tool.prepare(call(2)).unwrap();
    assert!(third.capability().is_none());
    assert_eq!(third.arguments(), &json!({"prepared": 2}));
    let mut records = tool.preparations_with_context();
    assert_eq!(records[0].context, None);
    assert_eq!(records[1].context, Some(context(1)));
    assert_eq!(records[2].context, None);
    for (old, new) in tool.preparations().iter().zip(&records) {
        assert_eq!(old.call, new.call);
    }
    records[1].context = None;
    assert_eq!(
        tool.preparations_with_context()[1].context,
        Some(context(1))
    );
    assert_eq!(tool.remaining_steps(), (0, 1));
    assert!(tool.invocations().is_empty());
}

#[test]
fn contextual_exhaustion_records_attempts_but_capacity_rejects_before_consumption() {
    let tool = fixture([step(0)], 3);
    tool.prepare_for_turn(&context(0), call(0)).unwrap();
    for index in 1..3 {
        let error = tool
            .prepare_for_turn(&context(index), call(index))
            .unwrap_err();
        assert_eq!(error.code, "testkit_script_exhausted");
        assert_eq!(error.kind, ToolErrorKind::Other);
    }
    let error = tool.prepare_for_turn(&context(3), call(3)).unwrap_err();
    assert_eq!(error.code, "testkit_record_capacity_exhausted");
    assert_eq!(tool.preparations().len(), 3);
    let records = tool.preparations_with_context();
    assert_eq!(records.len(), 3);
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record.context, Some(context(index)));
        assert_eq!(record.call, call(index));
    }
    assert_eq!(tool.remaining_steps(), (0, 1));
}

#[test]
fn both_entry_points_share_zero_and_finite_preparation_capacity() {
    for capacity in [0, 3] {
        let tool = fixture((0..=capacity).map(step), capacity);
        for index in 0..capacity {
            if index % 2 == 0 {
                tool.prepare(call(index)).unwrap();
            } else {
                tool.prepare_for_turn(&context(index), call(index)).unwrap();
            }
        }
        assert_eq!(
            tool.prepare(call(capacity)).unwrap_err().code,
            "testkit_record_capacity_exhausted"
        );
        assert_eq!(
            tool.prepare_for_turn(&context(capacity), call(capacity))
                .unwrap_err()
                .code,
            "testkit_record_capacity_exhausted"
        );
        assert_eq!(tool.preparations().len(), capacity);
        assert_eq!(tool.preparations_with_context().len(), capacity);
        assert_eq!(tool.remaining_steps(), (1, 1));
        assert!(tool.invocations().is_empty());
    }
}

#[test]
fn execution_recording_still_happens_at_future_construction() {
    for contextual in [false, true] {
        let tool = fixture([step(0)], 1);
        let prepared = tool.prepare_for_turn(&context(0), call(0)).unwrap();
        assert_eq!(tool.remaining_steps(), (0, 1));
        let cancellation = CancellationToken::new();
        let future = if contextual {
            tool.execute_for_turn(
                context(0),
                prepared.arguments().clone(),
                cancellation.clone(),
            )
            .map(drop)
            .boxed()
        } else {
            tool.execute(
                context(0),
                prepared.arguments().clone(),
                cancellation.clone(),
            )
            .map(drop)
            .boxed()
        };
        assert_eq!(tool.remaining_steps(), (0, 0));
        let invocations = tool.invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].context, context(0));
        assert_eq!(invocations[0].arguments, json!({"prepared": 0}));
        cancellation.cancel();
        assert!(invocations[0].cancellation.is_cancelled());
        drop(future);
        assert_eq!(tool.invocations().len(), 1);
        assert_eq!(tool.preparations_with_context().len(), 1);
    }
}

#[test]
fn concurrent_context_records_keep_each_call_paired_with_its_context() {
    const CAPACITY: usize = 8;
    let tool = fixture((0..CAPACITY).map(step), CAPACITY);
    std::thread::scope(|scope| {
        for index in 0..CAPACITY {
            let tool = &tool;
            scope.spawn(move || {
                tool.prepare_for_turn(&context(index), call(index)).unwrap();
            });
        }
    });
    let mut records = tool.preparations_with_context();
    records.sort_by(|a, b| a.call.id.cmp(&b.call.id));
    assert_eq!(records.len(), CAPACITY);
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record.call, call(index));
        assert_eq!(record.context, Some(context(index)));
    }
    assert_eq!(tool.remaining_steps(), (0, 1));
}
