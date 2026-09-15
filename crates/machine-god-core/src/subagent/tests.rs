use super::*;
use serde_json::json;

#[test]
fn complete_result_envelopes_and_history_pages_are_bounded() {
    let mut result = ManagedSubagentResult {
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
    };
    result.validate().unwrap();
    result.status = ManagedResultStatus::Configured;
    assert!(result.validate().is_err());
    result.status = ManagedResultStatus::Created;
    result.error_code = Some(ManagedFailureCode::InvalidState);
    assert!(result.validate().is_err());
    result.ok = false;
    result.status = ManagedResultStatus::Rejected;
    result.requested = None;
    result.validate().unwrap();
    let history = ManagedHistoryItem {
        kind: ManagedHistoryKind::Conversation,
        work_id: None,
        user: Some("u".repeat(16384)),
        assistant: Some("a".repeat(16384)),
        user_truncated: false,
        assistant_truncated: false,
    };
    let mut inspection = ManagedInspection {
        child_id: "child".into(),
        generation: 1,
        history: vec![history.clone()],
        ..ManagedInspection::default()
    };
    result.ok = true;
    result.error_code = None;
    result.status = ManagedResultStatus::Inspected;
    result.requested = Some(ManagedRequested::Inspection(Box::new(inspection.clone())));
    result.validate().unwrap();
    inspection.history.push(history);
    result.requested = Some(ManagedRequested::Inspection(Box::new(inspection)));
    assert!(result.validate().is_err());
}

fn decode(value: Value) -> Result<ManagedSubagentCommand, ManagedSubagentError> {
    let mut envelope = json!({});
    envelope["command"] = value;
    ManagedSubagentCommand::decode(envelope)
}
#[test]
fn all_commands_and_defaults_are_closed_and_typed() {
    for value in [
        json!({"create":{"name":"worker","mode":"one_off","prompt":"task"}}),
        json!({"create":{"name":"worker","mode":"persistent"}}),
        json!({"inspect":{"id":"child","sections":["messages","status","tool_activity","events","configuration","relationship"]}}),
        json!({"message":{"send":{"id":"child","content":"ordinary milestone text"}}}),
        json!({"message":{"milestone":{"name":"ready"}}}),
        json!({"relationship":{"action":"attach","id":"child"}}),
        json!({"relationship":{"action":"detach","id":"child"}}),
        json!({"relationship":{"action":"reparent","id":"child","parent_id":"parent"}}),
        json!({"configure":{"id":"child","effort":"xhigh"}}),
        json!({"lifecycle":{"id":"child","action":"cancel"}}),
        json!({"lifecycle":{"id":"child","action":"resume"}}),
        json!({"lifecycle":{"id":"child","action":"close"}}),
        json!({"lifecycle":{"id":"child","action":"reopen"}}),
    ] {
        let command = decode(value).unwrap();
        assert_eq!(
            ManagedSubagentCommand::decode(command.to_arguments().unwrap()).unwrap(),
            command
        );
    }
    let ManagedSubagentCommand::Create(value) =
        decode(json!({"create":{"name":"a","mode":"persistent"}})).unwrap()
    else {
        panic!()
    };
    assert_eq!(value.permission_mode, None);
    assert_eq!(value.notifications, ManagedNotifications::default());
    let ManagedSubagentCommand::Inspect(value) =
        decode(json!({"inspect":{"id":"child","sections":["status"]}})).unwrap()
    else {
        panic!()
    };
    assert_eq!(value.limit, 50);
}
#[test]
fn invalid_branches_fields_nulls_and_bounds_are_rejected() {
    for value in [
        json!({}),
        json!({"create":{},"inspect":{}}),
        json!({"create":{"name":"a","mode":"one_off"}}),
        json!({"create":{"name":"a","mode":"persistent","unknown":true}}),
        json!({"create":{"name":"a","mode":"persistent","prompt":null}}),
        json!({"create":{"name":"a","mode":"persistent","model":""}}),
        json!({"create":{"name":"a","mode":"persistent","effort":"bad effort"}}),
        json!({"message":{"send":{"id":"child","content":"x"},"milestone":{"name":"m"}}}),
        json!({"message":{"milestone":{"name":"m","id":"other"}}}),
        json!({"relationship":{"action":"detach","id":"child","parent_id":"parent"}}),
        json!({"relationship":{"action":"reparent","id":"child"}}),
        json!({"relationship":{"action":"attach","id":"child","parent_id":"child"}}),
        json!({"configure":{"id":"child"}}),
        json!({"lifecycle":{"id":"../child","action":"close"}}),
        json!({"inspect":{"id":"child","sections":[]}}),
        json!({"inspect":{"id":"child","sections":["status","status"]}}),
        json!({"inspect":{"id":"child","sections":["history"]}}),
        json!({"inspect":{"id":"child","sections":["status"],"limit":101}}),
        json!({"inspect":{"id":"child","sections":["status"],"wait":{"until":"settled","timeout_ms":0}}}),
        json!({"inspect":{"id":"child","sections":["messages"],"wait":{"until":"settled","timeout_ms":1}}}),
        json!({"inspect":{"id":"child","sections":["status"],"cursor":"v1:1:0","wait":{"until":"settled","timeout_ms":1}}}),
    ] {
        assert!(decode(value.clone()).is_err(), "{value}");
    }
    for size in [MAX_SUBAGENT_NAME_BYTES + 1, MAX_SUBAGENT_PROMPT_BYTES + 1] {
        assert!(decode(json!({"create":{"name":"x".repeat(size),"mode":"persistent"}})).is_err());
    }
    assert!(decode(json!({"message":{"send":{"id":"c","content":"x".repeat(MAX_SUBAGENT_MESSAGE_BYTES+1)}}})).is_err());
    let mut hostile = Value::Null;
    for _ in 0..10_000 {
        hostile = Value::Array(vec![hostile]);
    }
    assert_eq!(
        ManagedSubagentCommand::decode(hostile),
        Err(ManagedSubagentError::ResourceLimit)
    );
}
#[test]
fn maximum_prompt_and_notifications_fit_prepared_bounds() {
    let value = json!({"create":{"name":"x".repeat(128),"mode":"one_off","prompt":"\u{0001}".repeat(65536),
        "model":"m".repeat(256),"effort":"e".repeat(64),
        "notifications":{"started":true,"milestones":(0..32).map(|i| format!("{i:0128}")).collect::<Vec<_>>(),
        "report_interval_ms":1,"report_duration_ms":2}}});
    let command = decode(value).unwrap();
    let prepared = command.to_arguments().unwrap();
    assert!(json_bounded(
        &prepared,
        MAX_SUBAGENT_ARGUMENT_BYTES,
        MAX_SUBAGENT_JSON_NODES
    ));
    let ManagedSubagentCommand::Create(v) = command else {
        panic!()
    };
    assert!(
        v.notifications
            .stop_conditions
            .contains(&ManagedStopCondition::DurationElapsed)
    );
}
#[test]
fn notification_validation_checks_duplicates_durations_and_count() {
    for policy in [
        json!({"milestones":["a","a"]}),
        json!({"milestones":(0..33).map(|i| i.to_string()).collect::<Vec<_>>()}),
        json!({"report_interval_ms":0}),
        json!({"report_duration_ms":1}),
        json!({"report_interval_ms":u64::MAX}),
        json!({"stop_conditions":["duration_elapsed"]}),
        json!({"stop_conditions":["terminal","terminal"]}),
    ] {
        assert!(
            decode(json!({"create":{"name":"a","mode":"persistent","notifications":policy}}))
                .is_err()
        );
    }
}
#[test]
fn cursor_and_wait_are_generation_bound() {
    let cursor = ManagedCursor {
        generation: u64::MAX,
        offset: u64::MAX,
    };
    assert_eq!(ManagedCursor::parse(&cursor.encode()).unwrap(), cursor);
    for value in [
        "v1:01:0",
        "v1:1:+2",
        "v1:1:2:3",
        "v1:1:18446744073709551616",
        "v2:1:2",
    ] {
        assert!(ManagedCursor::parse(value).is_err());
    }
    let wait = ManagedInspectWait {
        until: ManagedWaitUntil::Settled,
        after_generation: Some(3),
        timeout_ms: 1,
    };
    assert!(!wait.satisfied(3, ManagedAgentState::Idle));
    assert!(!wait.satisfied(4, ManagedAgentState::AwaitingApproval));
    assert!(wait.satisfied(4, ManagedAgentState::Interrupted));
}
#[test]
fn structural_execution_cannot_call_the_authority() {
    struct Never;
    impl ManagedSubagentAuthority for Never {
        fn execute(
            &self,
            _: ManagedSubagentInvocation,
            _: CancellationToken,
        ) -> BoxFuture<'_, Result<ManagedSubagentResult, ManagedSubagentError>> {
            panic!("forged structural invocation")
        }
    }
    let tool = SubagentTool::new(Never);
    let context = ToolContext {
        session_id: crate::SessionId::new("s").unwrap(),
        session_incarnation_id: crate::SessionIncarnationId::new("i").unwrap(),
        turn_id: crate::TurnId::new("t").unwrap(),
        call_id: crate::ToolCallId::new("c").unwrap(),
    };
    assert!(
        futures_executor::block_on(tool.execute(context, json!({}), CancellationToken::new()))
            .is_err()
    );
    assert!(
        tool.spec().input_schema["properties"]["command"]["oneOf"]
            .as_array()
            .unwrap()
            .len()
            == 6
    );
}
