use std::{fmt::Debug, time::Duration};

use machine_god_core::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn round_trip<T: Serialize + DeserializeOwned + Eq + Debug>(value: &T) {
    let bytes = serde_json::to_vec(value).unwrap();
    let decoded: T = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(&decoded, value);
}

fn id() -> TerminalSessionId {
    TerminalSessionId::new("terminal.alpha-1").unwrap()
}
fn cursor(offset: u64) -> TerminalCursor {
    TerminalCursor::new(1, offset).unwrap()
}
fn monitor_id() -> TerminalMonitorId {
    TerminalMonitorId::new("ready").unwrap()
}
fn definition() -> TerminalMonitorDefinition {
    TerminalMonitorDefinition {
        condition: TerminalMonitorCondition::OutputContains {
            pattern: "ready".into(),
        },
        check_schedule: None,
        notify: TerminalNotifySchedule::OnMatch,
        lifetime: TerminalMonitorLifetime::UntilSessionEnd,
    }
}
fn facts() -> TerminalSessionFacts {
    TerminalSessionFacts {
        session_id: id(),
        lifecycle: TerminalLifecycle::Running,
        attention: TerminalAttentionState::default(),
        backend: TerminalBackend::Native,
        persistence: TerminalPersistenceLevel::Durable,
        output_cursor: cursor(10),
        unread_range: Some(TerminalRawRange {
            start: cursor(0),
            end: cursor(10),
        }),
        raw_gap: None,
        screen_recovery: TerminalScreenRecovery::Available {
            checkpoint: TerminalCheckpointEnvelope {
                engine_schema_revision: 1,
                applied_cursor: cursor(8),
                payload_len: 1,
                checksum: [0; 32],
            },
        },
        active_monitor_count: 1,
        next_actions: TerminalAllowedControls::default(),
    }
}
fn screen() -> TerminalScreen {
    TerminalScreen {
        dimensions: TerminalDimensions::new(1, 1).unwrap(),
        cursor: TerminalScreenCursor {
            row: 0,
            column: 0,
            visible: true,
            shape: TerminalCursorShape::Block,
            blinking: false,
        },
        modes: TerminalModes::default(),
        cells: vec![TerminalCell {
            kind: TerminalCellKind::Single,
            text: "λ".into(),
            style: TerminalCellStyle::default(),
            hyperlink_id: None,
        }],
        hyperlinks: Vec::new(),
    }
}
fn event(event_id: u64) -> TerminalMonitorEvent {
    TerminalMonitorEvent {
        event_id,
        monitor_id: monitor_id(),
        reason: TerminalMonitorEventReason::Matched,
        lifecycle: TerminalLifecycle::Running,
        cursor: cursor(5),
        created_at_ms: 10,
    }
}
fn exec_result() -> TerminalExecResult {
    TerminalExecResult {
        status: TerminalExecStatus::Exited { exit_code: 0 },
        stdout: TerminalExecCapturedOutput {
            bytes: b"ok".to_vec(),
            total_bytes: 2,
        },
        stderr: TerminalExecCapturedOutput {
            bytes: Vec::new(),
            total_bytes: 0,
        },
        duration: Duration::new(1, 23),
    }
}
fn requests() -> Vec<TerminalActionRequest> {
    vec![
        TerminalActionRequest::Exec {
            request: TerminalExecRequest {
                command: "echo ok".into(),
                cwd: "/prepared".into(),
                profile: Some(TerminalProfile::Clean),
            },
        },
        TerminalActionRequest::Start {
            request: TerminalStartRequest::interactive("/prepared").unwrap(),
        },
        TerminalActionRequest::Read {
            session_id: id(),
            cursor: cursor(0),
        },
        TerminalActionRequest::Screen { session_id: id() },
        TerminalActionRequest::Write {
            session_id: id(),
            request: TerminalWriteRequest {
                lease: TerminalWriteLeaseIntent::Use,
                payload: Some(TerminalWritePayload::Text {
                    text: "hello".into(),
                }),
            },
        },
        TerminalActionRequest::Wait {
            session_id: id(),
            request: TerminalWaitRequest {
                condition: TerminalReturnCondition::Exit,
                safety_ceiling_ms: 1000,
            },
        },
        TerminalActionRequest::Monitor {
            session_id: id(),
            operation: TerminalMonitorOperation::Add {
                definition: definition(),
            },
        },
        TerminalActionRequest::Inspect {
            session_id: id(),
            events: TerminalEventQuery {
                after_event_id: 0,
                acknowledge_event_id: None,
                max_events: 64,
            },
        },
        TerminalActionRequest::List {
            filters: TerminalListFilters {
                task_id: Some("task".into()),
                workspace_root: Some("/prepared".into()),
                lifecycle: Some(TerminalLifecycle::Running),
                backend: Some(TerminalBackend::Native),
            },
        },
        TerminalActionRequest::Resize {
            session_id: id(),
            dimensions: TerminalDimensions::new(24, 80).unwrap(),
        },
        TerminalActionRequest::Signal {
            session_id: id(),
            signal: TerminalSignal::Interrupt,
        },
        TerminalActionRequest::Close {
            session_id: id(),
            policy: TerminalClosePolicy::Graceful,
        },
    ]
}
fn results() -> Vec<TerminalActionResult> {
    vec![
        TerminalActionResult::Exec {
            result: exec_result(),
        },
        TerminalActionResult::Start {
            session: facts(),
            outcome: TerminalReturnOutcome::Started {},
        },
        TerminalActionResult::Read {
            session: facts(),
            output: vec![b'a'; 10],
            raw_range: Some(TerminalRawRange {
                start: cursor(0),
                end: cursor(10),
            }),
        },
        TerminalActionResult::Screen {
            session: facts(),
            snapshot: screen(),
        },
        TerminalActionResult::Write {
            session: facts(),
            accepted_bytes: 5,
        },
        TerminalActionResult::Wait {
            session: facts(),
            outcome: TerminalReturnOutcome::SafetyCeiling {},
        },
        TerminalActionResult::Monitor {
            session: facts(),
            monitor_id: Some(monitor_id()),
        },
        TerminalActionResult::Inspect {
            session: facts(),
            shell: "/bin/bash".into(),
            cwd: "/prepared".into(),
            command: None,
            monitors: vec![TerminalMonitorSummary {
                monitor_id: monitor_id(),
                state: TerminalMonitorState::Active,
            }],
            events: vec![event(1)],
            event_gap_through: 0,
            next_event_id: 2,
        },
        TerminalActionResult::List {
            sessions: vec![facts()],
        },
        TerminalActionResult::Resize {
            session: facts(),
            dimensions: TerminalDimensions::new(24, 80).unwrap(),
        },
        TerminalActionResult::Signal {
            session: facts(),
            signal: TerminalSignal::Interrupt,
        },
        TerminalActionResult::Close {
            session: facts(),
            policy: TerminalClosePolicy::Graceful,
        },
    ]
}

#[test]
fn all_twelve_requests_results_and_responses_round_trip() {
    let requests = requests();
    let results = results();
    assert_eq!(requests.len(), 12);
    assert_eq!(results.len(), 12);
    for (index, (request, result)) in requests.iter().zip(&results).enumerate() {
        request.validate().unwrap();
        result.validate_for(request).unwrap();
        round_trip(request);
        round_trip(result);
        let response = TerminalActionResponse::Success {
            result: Box::new(result.clone()),
        };
        response.validate_for(request).unwrap();
        round_trip(&response);
        for (other_index, other) in requests.iter().enumerate() {
            if index != other_index {
                assert!(result.validate_for(other).is_err());
            }
        }
    }
}

#[test]
fn full_start_forms_remain_effect_free_and_reuse_existing_vocabulary() {
    for backend in [TerminalBackend::Native, TerminalBackend::Tmux] {
        for profile in [TerminalProfile::User, TerminalProfile::Clean] {
            let mut start = TerminalStartRequest::interactive("/this/path/does/not/exist").unwrap();
            start.backend = backend;
            start.profile = Some(profile);
            start.command = Some("exit 7".into());
            start.return_when = Some(TerminalReturnCondition::Started);
            start.dimensions = Some(TerminalDimensions::new(44, 132).unwrap());
            start.initial_monitors = vec![definition(); MAX_TERMINAL_INITIAL_MONITORS];
            start.validate().unwrap();
            round_trip(&start);
            start.profile = None;
            for shell in [
                TerminalShellSpec::UserLogin {},
                TerminalShellSpec::Executable {
                    path: "/missing/bash".into(),
                    clean_start: true,
                },
            ] {
                start.shell = Some(shell);
                start.validate().unwrap();
                round_trip(&start);
            }
        }
    }
}

fn reject_start(start: &TerminalStartRequest) {
    assert!(start.validate().is_err());
    assert!(
        serde_json::from_value::<TerminalStartRequest>(serde_json::to_value(start).unwrap())
            .is_err()
    );
}

#[test]
fn startup_relationships_and_bounds_are_enforced_on_wire_and_values() {
    let original = TerminalStartRequest::interactive("/prepared").unwrap();
    let mut start = original.clone();
    start.command = Some("command".into());
    reject_start(&start);
    start.return_when = Some(TerminalReturnCondition::Exit);
    reject_start(&start);
    start.wait_ceiling_ms = Some(1);
    start.validate().unwrap();
    for ceiling in [0, u64::MAX] {
        start.wait_ceiling_ms = Some(ceiling);
        reject_start(&start);
    }
    start.wait_ceiling_ms = Some(i64::MAX.unsigned_abs());
    start.validate().unwrap();
    start.command = Some("a".repeat(MAX_TERMINAL_ACTION_COMMAND_BYTES));
    start.validate().unwrap();
    start.command.as_mut().unwrap().push('a');
    reject_start(&start);
    for command in ["", "a\0b"] {
        start.command = Some(command.into());
        reject_start(&start);
    }
    start = original.clone();
    start.profile = Some(TerminalProfile::Clean);
    start.shell = Some(TerminalShellSpec::UserLogin {});
    reject_start(&start);
    start = original.clone();
    start.initial_monitors = vec![definition(); MAX_TERMINAL_INITIAL_MONITORS + 1];
    reject_start(&start);
    for bad_cwd in [
        String::new(),
        "relative".into(),
        "/a\0b".into(),
        "/".repeat(MAX_TERMINAL_ACTION_TEXT_BYTES + 1),
    ] {
        assert!(TerminalStartRequest::interactive(bad_cwd).is_err());
    }
    assert!(TerminalStartRequest::interactive("/".repeat(MAX_TERMINAL_ACTION_TEXT_BYTES)).is_ok());
    for path in [
        String::new(),
        "x\0y".into(),
        "x".repeat(MAX_TERMINAL_ACTION_TEXT_BYTES + 1),
    ] {
        start = original.clone();
        start.shell = Some(TerminalShellSpec::Executable {
            path,
            clean_start: false,
        });
        reject_start(&start);
    }
}

#[test]
fn all_waits_monitors_lease_operations_and_payloads_pass_through() {
    for condition in [
        TerminalReturnCondition::Started,
        TerminalReturnCondition::Exit,
        TerminalReturnCondition::Quiet { duration_ms: 1 },
        TerminalReturnCondition::Match {
            pattern: "done".into(),
        },
    ] {
        let request = TerminalActionRequest::Wait {
            session_id: id(),
            request: TerminalWaitRequest {
                condition,
                safety_ceiling_ms: 5,
            },
        };
        request.validate().unwrap();
        round_trip(&request);
    }
    for lease in [
        TerminalWriteLeaseIntent::Acquire,
        TerminalWriteLeaseIntent::Release,
        TerminalWriteLeaseIntent::Revoke,
    ] {
        let request = TerminalActionRequest::Write {
            session_id: id(),
            request: TerminalWriteRequest {
                lease,
                payload: None,
            },
        };
        round_trip(&request);
        assert!(
            TerminalActionResult::Write {
                session: facts(),
                accepted_bytes: 1
            }
            .validate_for(&request)
            .is_err()
        );
        TerminalActionResult::Write {
            session: facts(),
            accepted_bytes: 0,
        }
        .validate_for(&request)
        .unwrap();
    }
    for payload in [
        TerminalWritePayload::Text {
            text: "\0λ".into()
        },
        TerminalWritePayload::Paste {
            text: "hello".into(),
        },
        TerminalWritePayload::Keys {
            keys: vec![TerminalNamedKey::Enter],
        },
        TerminalWritePayload::Controls {
            controls: vec![b'c'],
        },
    ] {
        let request = TerminalActionRequest::Write {
            session_id: id(),
            request: TerminalWriteRequest {
                lease: TerminalWriteLeaseIntent::Use,
                payload: Some(payload),
            },
        };
        request.validate().unwrap();
        round_trip(&request);
    }
    for operation in [
        TerminalMonitorOperation::Add {
            definition: definition(),
        },
        TerminalMonitorOperation::Update {
            monitor_id: monitor_id(),
            definition: definition(),
        },
        TerminalMonitorOperation::Pause {
            monitor_id: monitor_id(),
        },
        TerminalMonitorOperation::Resume {
            monitor_id: monitor_id(),
        },
        TerminalMonitorOperation::Remove {
            monitor_id: monitor_id(),
        },
    ] {
        round_trip(&TerminalActionRequest::Monitor {
            session_id: id(),
            operation,
        });
    }
}

fn reject_result(result: &TerminalActionResult) {
    assert!(result.validate().is_err());
    assert!(
        serde_json::from_value::<TerminalActionResult>(serde_json::to_value(result).unwrap())
            .is_err()
    );
}

#[test]
fn all_thirteen_monitor_conditions_are_inert_data_in_start_and_actions() {
    let conditions = vec![
        TerminalMonitorCondition::ProcessExit,
        TerminalMonitorCondition::ExitCode { exit_code: 7 },
        TerminalMonitorCondition::Signal {
            signal: TerminalSignal::Terminate,
        },
        TerminalMonitorCondition::OutputContains {
            pattern: "ready".into(),
        },
        TerminalMonitorCondition::OutputMatches {
            pattern: "r.*y".into(),
        },
        TerminalMonitorCondition::OutputQuiet { duration_ms: 10 },
        TerminalMonitorCondition::ScreenMatches {
            pattern: "ready".into(),
        },
        TerminalMonitorCondition::TcpReady {
            host: "never-resolve.invalid".into(),
            port: 80,
        },
        TerminalMonitorCondition::HttpReady {
            url: "https://never-fetch.invalid".into(),
        },
        TerminalMonitorCondition::PathExists {
            path: "/never-stat".into(),
        },
        TerminalMonitorCondition::PathChanged {
            path: "/never-watch".into(),
        },
        TerminalMonitorCondition::PathSize {
            path: "/never-open".into(),
            minimum_bytes: 1,
        },
        TerminalMonitorCondition::CustomProbe {
            command: "never-execute".into(),
            cwd: "/never-resolve".into(),
        },
    ];
    assert_eq!(conditions.len(), 13);
    let mut start = TerminalStartRequest::interactive("/prepared").unwrap();
    for condition in conditions {
        let definition = TerminalMonitorDefinition {
            check_schedule: condition
                .requires_polling()
                .then_some(TerminalSchedule { interval_ms: 10 }),
            condition,
            notify: TerminalNotifySchedule::EveryNChecks { count: 2 },
            lifetime: TerminalMonitorLifetime::Duration { duration_ms: 100 },
        };
        let request = TerminalActionRequest::Monitor {
            session_id: id(),
            operation: TerminalMonitorOperation::Add {
                definition: definition.clone(),
            },
        };
        request.validate().unwrap();
        round_trip(&request);
        start.initial_monitors.push(definition);
    }
    start.validate().unwrap();
    round_trip(&TerminalActionRequest::Start { request: start });
}

#[test]
fn nested_request_validation_cannot_be_bypassed_by_envelope_deserialization() {
    let invalid = [
        TerminalActionRequest::Wait {
            session_id: id(),
            request: TerminalWaitRequest {
                condition: TerminalReturnCondition::Exit,
                safety_ceiling_ms: 0,
            },
        },
        TerminalActionRequest::Write {
            session_id: id(),
            request: TerminalWriteRequest {
                lease: TerminalWriteLeaseIntent::Use,
                payload: None,
            },
        },
        TerminalActionRequest::Write {
            session_id: id(),
            request: TerminalWriteRequest {
                lease: TerminalWriteLeaseIntent::Acquire,
                payload: Some(TerminalWritePayload::Text { text: "x".into() }),
            },
        },
        TerminalActionRequest::List {
            filters: TerminalListFilters {
                task_id: Some("x".repeat(MAX_TERMINAL_ACTION_TEXT_BYTES + 1)),
                ..TerminalListFilters::default()
            },
        },
        TerminalActionRequest::Monitor {
            session_id: id(),
            operation: TerminalMonitorOperation::Add {
                definition: TerminalMonitorDefinition {
                    condition: TerminalMonitorCondition::PathExists {
                        path: "/path".into(),
                    },
                    check_schedule: None,
                    ..definition()
                },
            },
        },
    ];
    for request in invalid {
        assert!(request.validate().is_err());
        assert!(
            serde_json::from_value::<TerminalActionRequest>(serde_json::to_value(request).unwrap())
                .is_err()
        );
    }
}

#[test]
fn cursor_range_gap_and_checkpoint_relationships_are_enforced() {
    for range in [
        TerminalRawRange {
            start: cursor(5),
            end: cursor(4),
        },
        TerminalRawRange {
            start: cursor(0),
            end: TerminalCursor::new(2, 0).unwrap(),
        },
    ] {
        assert!(range.validate().is_err());
    }
    TerminalRawRange {
        start: cursor(4),
        end: cursor(4),
    }
    .validate()
    .unwrap();
    let mut session = facts();
    session.unread_range.as_mut().unwrap().end = cursor(11);
    assert!(session.validate().is_err());
    session = facts();
    session.raw_gap = Some(TerminalGap::new(cursor(5), cursor(11)).unwrap());
    assert!(session.validate().is_err());
    for applied_cursor in [cursor(11), TerminalCursor::new(2, 0).unwrap()] {
        session = facts();
        let TerminalScreenRecovery::Available { checkpoint } = &mut session.screen_recovery else {
            unreachable!()
        };
        checkpoint.applied_cursor = applied_cursor;
        assert!(session.validate().is_err());
    }
    for payload_len in [0, MAX_TERMINAL_CHECKPOINT_PAYLOAD_BYTES + 1] {
        let checkpoint = TerminalCheckpointEnvelope {
            engine_schema_revision: 1,
            applied_cursor: cursor(0),
            payload_len,
            checksum: [0; 32],
        };
        assert!(checkpoint.validate().is_err());
    }
    reject_result(&TerminalActionResult::Read {
        session: facts(),
        output: Vec::new(),
        raw_range: Some(TerminalRawRange {
            start: cursor(0),
            end: cursor(11),
        }),
    });
    reject_result(&TerminalActionResult::Read {
        session: facts(),
        output: vec![0; MAX_TERMINAL_ACTION_OUTPUT_BYTES + 1],
        raw_range: None,
    });
    let mut snapshot = screen();
    snapshot.cursor.column = 1;
    reject_result(&TerminalActionResult::Screen {
        session: facts(),
        snapshot,
    });
}

#[test]
fn inspect_event_order_gaps_and_page_bounds_are_validated() {
    let base = results().swap_remove(7);
    for changes in [
        json!({"next_event_id":0}),
        json!({"event_gap_through":2}),
        json!({"events":[event(1),event(1)]}),
        json!({"events":[event(2)]}),
        json!({"shell":""}),
        json!({"cwd":"x\0y"}),
        json!({"command":""}),
    ] {
        let mut value = serde_json::to_value(&base).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        assert!(serde_json::from_value::<TerminalActionResult>(value).is_err());
    }
    let mut query = TerminalEventQuery {
        after_event_id: 2,
        acknowledge_event_id: Some(1),
        max_events: 64,
    };
    assert!(
        TerminalActionRequest::Inspect {
            session_id: id(),
            events: query.clone()
        }
        .validate()
        .is_err()
    );
    query.acknowledge_event_id = Some(2);
    let request = TerminalActionRequest::Inspect {
        session_id: id(),
        events: query,
    };
    assert!(base.validate_for(&request).is_err());
    let TerminalActionResult::Inspect {
        mut session,
        shell,
        cwd,
        command,
        monitors,
        ..
    } = base
    else {
        unreachable!()
    };
    session.active_monitor_count = 0;
    let page = TerminalActionResult::Inspect {
        session,
        shell,
        cwd,
        command,
        monitors,
        events: (1..=256).map(event).collect(),
        event_gap_through: 0,
        next_event_id: 257,
    };
    page.validate().unwrap();
    round_trip(&page);
    let mut value = serde_json::to_value(page).unwrap();
    value["events"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(event(257)).unwrap());
    value["next_event_id"] = json!(258);
    assert!(serde_json::from_value::<TerminalActionResult>(value).is_err());
    reject_result(&TerminalActionResult::List {
        sessions: vec![facts(); MAX_TERMINAL_ACTION_RESULTS + 1],
    });
}

#[test]
fn result_correlations_reject_wrong_session_backend_and_mutation_receipts() {
    let mut other = facts();
    other.session_id = TerminalSessionId::new("another").unwrap();
    assert!(
        TerminalActionResult::Screen {
            session: other,
            snapshot: screen()
        }
        .validate_for(&requests()[3])
        .is_err()
    );
    let mut other = facts();
    other.backend = TerminalBackend::Tmux;
    assert!(
        TerminalActionResult::Start {
            session: other,
            outcome: TerminalReturnOutcome::Started {}
        }
        .validate_for(&requests()[1])
        .is_err()
    );
    assert!(
        TerminalActionResult::Resize {
            session: facts(),
            dimensions: TerminalDimensions::new(1, 1).unwrap()
        }
        .validate_for(&requests()[9])
        .is_err()
    );
    assert!(
        TerminalActionResult::Signal {
            session: facts(),
            signal: TerminalSignal::Kill
        }
        .validate_for(&requests()[10])
        .is_err()
    );
    assert!(
        TerminalActionResult::Close {
            session: facts(),
            policy: TerminalClosePolicy::Force
        }
        .validate_for(&requests()[11])
        .is_err()
    );
    reject_result(&TerminalActionResult::Write {
        session: facts(),
        accepted_bytes: u32::try_from(MAX_TERMINAL_WRITE_BYTES + 1).unwrap(),
    });
    for outcome in [
        TerminalReturnOutcome::Exited { exit_code: -1 },
        TerminalReturnOutcome::Exited { exit_code: 256 },
        TerminalReturnOutcome::Signal { signal: 0 },
        TerminalReturnOutcome::Signal { signal: 256 },
    ] {
        reject_result(&TerminalActionResult::Wait {
            session: facts(),
            outcome,
        });
    }
}

#[test]
fn foreground_result_preserves_status_totals_duration_and_bounds() {
    let mut result = exec_result();
    round_trip(&result);
    assert_eq!(result.duration.subsec_nanos(), 23);
    result.status = TerminalExecStatus::TimedOut {};
    result.validate().unwrap();
    result.status = TerminalExecStatus::OutputLimit {};
    assert!(result.validate().is_err());
    result.stdout.total_bytes = MAX_TERMINAL_ACTION_OUTPUT_BYTES as u64 + 1;
    result.validate().unwrap();
    round_trip(&result);
    assert!(result.stdout.truncated());
    result.status = TerminalExecStatus::Exited { exit_code: 0 };
    assert!(result.validate().is_err());
    result = exec_result();
    result.stdout.total_bytes = 1;
    assert!(result.validate().is_err());
    result = exec_result();
    result.duration = MAX_TERMINAL_EXEC_DURATION + Duration::from_nanos(1);
    assert!(result.validate().is_err());
    result = exec_result();
    result.stdout.bytes = vec![0; MAX_TERMINAL_EXEC_STREAM_BYTES + 1];
    result.stdout.total_bytes = result.stdout.bytes.len() as u64;
    reject_result(&TerminalActionResult::Exec { result });
}

#[test]
fn failure_ids_are_sealed_and_failed_start_can_name_allocated_session() {
    for invalid in ["", "..", "../escape", "a/b", "a\0b"] {
        let value = json!({"status":"failure","action":"start","code":"startup_failed","session_id":invalid,"retryable":false});
        assert!(serde_json::from_value::<TerminalActionResponse>(value).is_err());
    }
    let failure = TerminalActionResponse::Failure {
        action: TerminalAction::Start,
        code: TerminalActionErrorCode::StartupFailed,
        session_id: Some(id()),
        retryable: false,
    };
    failure.validate_for(&requests()[1]).unwrap();
    round_trip(&failure);
    for action in [TerminalAction::Exec, TerminalAction::List] {
        let failure = TerminalActionResponse::Failure {
            action,
            code: TerminalActionErrorCode::Cancelled,
            session_id: Some(id()),
            retryable: false,
        };
        let request = requests()
            .into_iter()
            .find(|request| request.action() == action)
            .unwrap();
        assert!(failure.validate_for(&request).is_err());
    }
    let failure = TerminalActionResponse::Failure {
        action: TerminalAction::Read,
        code: TerminalActionErrorCode::SessionLost,
        session_id: Some(TerminalSessionId::new("wrong").unwrap()),
        retryable: false,
    };
    assert!(failure.validate_for(&requests()[2]).is_err());
}

#[test]
fn unknown_authority_fields_and_invalid_data_never_echo_input() {
    for request in requests() {
        for name in ["actor", "owner", "writer", "authority", "caller", "pid"] {
            let mut value = serde_json::to_value(&request).unwrap();
            value[name] = json!("secret-sentinel");
            let error = serde_json::from_value::<TerminalActionRequest>(value).unwrap_err();
            assert!(!error.to_string().contains("secret-sentinel"));
        }
    }
    let mut value = serde_json::to_value(&requests()[1]).unwrap();
    value["request"]["cwd"] = Value::String("secret-sentinel".into());
    let error = serde_json::from_value::<TerminalActionRequest>(value).unwrap_err();
    assert!(!error.to_string().contains("secret-sentinel"));
    let request = TerminalActionRequest::Exec {
        request: TerminalExecRequest {
            command: "secret-sentinel".into(),
            cwd: "/secret-sentinel".into(),
            profile: None,
        },
    };
    assert!(!format!("{request:?}").contains("secret-sentinel"));
    assert!(
        serde_json::from_str::<TerminalActionRequest>(
            r#"{"action":"screen","session_id":"one","session_id":"two"}"#
        )
        .is_err()
    );
}
