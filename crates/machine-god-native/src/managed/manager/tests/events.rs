use super::*;
mod source_errors;
use machine_god_core::{
    Capability, FilesystemAccess, ManagedToolActivity, ManagedToolPhase, PermissionDecision,
    PermissionGrantScope, ToolCall, ToolCallId, ToolError, ToolErrorKind, ToolName, ToolOutput,
    ToolSpec,
};
use machine_god_core::{ManagedEvent, ManagedEventKind, ManagedReceipt};
use machine_god_testkit::{
    InMemorySessionStore, PermissionStep, RecordingEventSink, ScriptedPermissionHandler,
    ScriptedPreparedTool, ToolPrepareStep, ToolStep,
};

fn receipt(result: machine_god_core::ManagedSubagentResult) -> ManagedReceipt {
    assert!(result.ok, "{result:?}");
    let Some(ManagedRequested::Receipt(receipt)) = result.requested else {
        panic!("mutation receipt required");
    };
    receipt
}

fn events(fixture: &mut Fixture) -> Vec<ManagedEvent> {
    let mut cursor = None;
    let mut events = Vec::new();
    for _ in 0..100 {
        let mut request = serde_json::json!({"id":"child-1","sections":["events"],"limit":1});
        if let Some(cursor) = cursor {
            request["cursor"] = serde_json::Value::String(cursor);
        }
        let result = fixture.command(serde_json::json!({"inspect":request}));
        assert!(result.ok, "{result:?}");
        let Some(ManagedRequested::Inspection(page)) = result.requested else {
            panic!("inspection required");
        };
        assert!(!page.restart_required);
        assert!(page.events.len() <= 1);
        events.extend(page.events);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return events;
        }
    }
    panic!("bounded history failed to terminate");
}

#[test]
#[allow(clippy::too_many_lines)] // One real command history across paging, archive and restart.
fn real_create_message_and_lifecycle_receipts_resolve_to_paged_events() {
    let mut fixture = Fixture::new(vec![super::completed()]);
    let created = receipt(
        fixture.command(serde_json::json!({"create":{"name":"worker","mode":"persistent"}})),
    );
    let initial = events(&mut fixture);
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].sequence, created.event_sequence);
    assert_eq!(initial[0].kind, ManagedEventKind::Created);

    let queued = receipt(
        fixture.command(serde_json::json!({"message":{"send":{"id":"child-1","content":"first"}}})),
    );
    fixture.drive(|f| {
        f.manager.children[0].snapshot.head.status == ManagedAgentState::Idle
            && !f.manager.children[0].busy()
    });
    let configured = receipt(
        fixture.command(serde_json::json!({"configure":{"id":"child-1","name":"renamed"}})),
    );
    let detached = receipt(
        fixture.command(serde_json::json!({"relationship":{"id":"child-1","action":"detach"}})),
    );
    let cancelled = receipt(
        fixture.command(serde_json::json!({"lifecycle":{"id":"child-1","action":"cancel"}})),
    );
    let closed = receipt(
        fixture.command(serde_json::json!({"lifecycle":{"id":"child-1","action":"close"}})),
    );
    fixture.drive(|f| f.manager.children.is_empty() && f.manager.retiring.is_empty());
    let archived = events(&mut fixture);
    let kind = |receipt: &ManagedReceipt| {
        &archived
            .iter()
            .find(|event| event.sequence == receipt.event_sequence)
            .unwrap()
            .kind
    };
    assert!(matches!(
        kind(&queued),
        ManagedEventKind::MessageQueued { .. }
    ));
    assert_eq!(kind(&configured), &ManagedEventKind::Configured);
    assert!(matches!(
        kind(&detached),
        ManagedEventKind::RelationshipChanged {
            parent_id: None,
            ..
        }
    ));
    assert!(matches!(
        kind(&cancelled),
        ManagedEventKind::LifecycleChanged {
            current: ManagedAgentState::Idle,
            ..
        }
    ));
    assert!(matches!(
        kind(&closed),
        ManagedEventKind::LifecycleChanged {
            current: ManagedAgentState::Archived,
            ..
        }
    ));
    assert!(archived.iter().any(|event| matches!(
        event.kind,
        ManagedEventKind::WorkTransition {
            current: ManagedQueueStatus::Running,
            ..
        }
    )));
    assert!(archived.iter().any(|event| matches!(
        event.kind,
        ManagedEventKind::WorkTransition {
            current: ManagedQueueStatus::Completed,
            ..
        }
    )));
    assert!(
        archived
            .windows(2)
            .all(|pair| pair[0].sequence > pair[1].sequence)
    );
    fixture.restart_manager();
    assert_eq!(events(&mut fixture), archived);
    let reopened = receipt(
        fixture.command(serde_json::json!({"lifecycle":{"id":"child-1","action":"reopen"}})),
    );
    let retained = events(&mut fixture);
    assert_eq!(retained[0].sequence, reopened.event_sequence);
    assert!(matches!(
        retained[0].kind,
        ManagedEventKind::LifecycleChanged {
            previous: ManagedAgentState::Archived,
            current: ManagedAgentState::Idle
        }
    ));
    assert_eq!(&retained[1..], archived.as_slice());
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}

#[test]
fn event_cursor_rejects_a_changed_head_instead_of_skipping_a_new_mutation() {
    let mut fixture = Fixture::new(vec![]);
    receipt(fixture.command(serde_json::json!({"create":{"name":"worker","mode":"persistent"}})));
    let first = fixture
        .command(serde_json::json!({"inspect":{"id":"child-1","sections":["events"],"limit":1}}));
    let cursor = first
        .cursor
        .expect("control record remains on the next page");
    receipt(fixture.command(serde_json::json!({"configure":{"id":"child-1","name":"changed"}})));
    let next = fixture.command(serde_json::json!({"inspect":{"id":"child-1","sections":["events"],"limit":1,"cursor":cursor}}));
    let Some(ManagedRequested::Inspection(page)) = next.requested else {
        panic!("inspection required");
    };
    assert!(page.restart_required);
    assert!(page.events.is_empty());
    assert_eq!(events(&mut fixture).len(), 2);
}

fn tool_round(calls: &[(&str, &str)]) -> ModelProviderStep {
    ModelProviderStep::events(
        calls
            .iter()
            .map(|(id, name)| ModelEvent::ToolCall {
                call: ToolCall {
                    id: ToolCallId::new(*id).unwrap(),
                    name: ToolName::new(*name).unwrap(),
                    arguments: serde_json::json!({}),
                },
            })
            .chain([ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            }]),
    )
}

fn activity_tool(
    name: &str,
    preparations: Vec<ToolPrepareStep>,
    executions: Vec<ToolStep>,
) -> ScriptedPreparedTool {
    ScriptedPreparedTool::new(
        ToolSpec {
            name: ToolName::new(name).unwrap(),
            description: "scripted activity without filesystem effects".into(),
            input_schema: serde_json::json!({"type":"object"}),
        },
        preparations,
        executions,
    )
}

fn tool_activity(fixture: &mut Fixture) -> Vec<ManagedToolActivity> {
    let mut cursor = None;
    let mut activity = Vec::new();
    for _ in 0..200 {
        let mut query = serde_json::json!({"id":"child-1","sections":["tool_activity"],"limit":1});
        if let Some(cursor) = cursor {
            query["cursor"] = serde_json::Value::String(cursor);
        }
        let result = fixture.command(serde_json::json!({"inspect":query}));
        assert!(result.ok, "{result:?}");
        let Some(ManagedRequested::Inspection(page)) = result.requested else {
            panic!("inspection required");
        };
        assert!(!page.restart_required);
        assert!(page.tool_activity.len() <= 1);
        activity.extend(page.tool_activity);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return activity;
        }
    }
    panic!("bounded tool history failed to terminate");
}

#[test]
#[allow(clippy::too_many_lines)] // Real core denial/approval events, durable pages, archive and restart.
fn denied_tool_activity_survives_paging_archive_and_restart_without_execution() {
    let preparation = || ToolPrepareStep::Prepared {
        // Deliberately not Capability::Tool: native tools can preserve their own capability.
        capability: Capability::Filesystem {
            access: FilesystemAccess::Write,
            path: "scripted-only".into(),
        },
        arguments: serde_json::json!({}),
    };
    let blocked = activity_tool("blocked_write", vec![preparation(), preparation()], vec![]);
    let approved = activity_tool(
        "approved_write",
        vec![preparation(), preparation()],
        vec![
            ToolStep::Output(ToolOutput {
                content: serde_json::json!("done"),
                is_error: false,
            }),
            ToolStep::Error(ToolError::new(
                ToolErrorKind::Execution,
                "scripted",
                "failed",
                false,
            )),
        ],
    );
    let broken = activity_tool(
        "invalid_input",
        vec![ToolPrepareStep::Error(ToolError::new(
            ToolErrorKind::InvalidInput,
            "scripted",
            "invalid",
            false,
        ))],
        vec![],
    );
    let deny = || {
        PermissionStep::Decision(PermissionDecision::Deny {
            reason: "rejected".into(),
        })
    };
    let allow = || {
        PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })
    };
    let permissions = ScriptedPermissionHandler::new([deny(), allow(), deny(), allow()]);
    let observed = RecordingEventSink::new();
    let mut fixture = Fixture::with_engine_setup(
        vec![
            // A preparation failure emits no permission event; correlation must not shift.
            tool_round(&[("invalid", "invalid_input"), ("reused", "blocked_write")]),
            tool_round(&[("allowed", "approved_write")]),
            super::completed(),
            tool_round(&[("reused", "blocked_write")]),
            tool_round(&[("allowed", "approved_write")]),
            super::completed(),
        ],
        InMemorySessionStore::default(),
        |engine| {
            engine
                .permission_handler(permissions.clone())
                .event_sink(observed.clone())
                .tool(blocked.clone())
                .tool(approved.clone())
                .tool(broken.clone())
        },
    );
    receipt(fixture.command(
        serde_json::json!({"create":{"name":"worker","mode":"persistent","prompt":"activity"}}),
    ));
    settled_activity(&mut fixture);
    // Reused provider IDs belong to separate actual turns, not duplicate IDs
    // within one core turn (which core correctly rejects).
    receipt(
        fixture.command(serde_json::json!({"message":{"send":{"id":"child-1","content":"again"}}})),
    );
    settled_activity(&mut fixture);
    let activity = tool_activity(&mut fixture);
    assert_eq!(
        activity
            .iter()
            .map(|item| (item.tool_name.as_str(), item.phase))
            .collect::<Vec<_>>(),
        [
            ("approved_write", ManagedToolPhase::Failed),
            ("approved_write", ManagedToolPhase::Started),
            ("blocked_write", ManagedToolPhase::Denied),
            ("approved_write", ManagedToolPhase::Succeeded),
            ("approved_write", ManagedToolPhase::Started),
            ("blocked_write", ManagedToolPhase::Denied),
        ]
    );
    assert!(
        activity[3..]
            .iter()
            .all(|item| item.work_id.as_deref() == Some("work-1"))
    );
    assert!(
        activity[..3]
            .iter()
            .all(|item| item.work_id == activity[0].work_id)
    );
    assert_ne!(activity[0].work_id, activity[3].work_id);
    assert!(activity.chunks(3).all(|turn| {
        turn.windows(2)
            .all(|pair| pair[0].sequence > pair[1].sequence)
    }));
    assert!(blocked.invocations().is_empty());
    assert!(broken.invocations().is_empty());
    assert_eq!(approved.invocations().len(), 2);
    assert_eq!(permissions.requests().len(), 4);
    let actual = observed.events();
    let denied: Vec<_> = actual
        .iter()
        .filter_map(|event| match &event.payload {
            machine_god_core::TurnEvent::ToolDenied { call_id, tool_name } => {
                Some((call_id.as_str(), tool_name.as_str(), &event.turn_id))
            }
            _ => None,
        })
        .collect();
    assert_eq!(denied.len(), 2);
    assert!(
        denied
            .iter()
            .all(|(id, name, _)| *id == "reused" && *name == "blocked_write")
    );
    assert_ne!(denied[0].2, denied[1].2);
    for event in actual {
        let encoded = serde_json::to_value(&event).unwrap();
        assert_eq!(
            serde_json::from_value::<machine_god_core::EngineEvent>(encoded).unwrap(),
            event
        );
    }
    receipt(fixture.command(serde_json::json!({"lifecycle":{"id":"child-1","action":"close"}})));
    fixture.drive(|f| f.manager.children.is_empty() && f.manager.retiring.is_empty());
    assert_eq!(tool_activity(&mut fixture), activity);
    fixture.restart_manager();
    assert_eq!(tool_activity(&mut fixture), activity);
    assert_eq!(fixture.factory.provider.requests().len(), 6);
}

fn settled_activity(fixture: &mut Fixture) {
    fixture.drive(|f| {
        !f.manager.children[0].busy()
            && !matches!(
                f.manager.children[0].snapshot.head.status,
                ManagedAgentState::Queued
                    | ManagedAgentState::Running
                    | ManagedAgentState::AwaitingApproval
            )
    });
    assert_eq!(
        fixture.manager.children[0].snapshot.head.status,
        ManagedAgentState::Idle
    );
}

#[test]
fn permission_failure_or_cancellation_does_not_invent_denied_tool_activity() {
    for cancel in [false, true] {
        let tool = activity_tool(
            "unexecuted",
            vec![ToolPrepareStep::Prepared {
                capability: Capability::Filesystem {
                    access: FilesystemAccess::Write,
                    path: "scripted-only".into(),
                },
                arguments: serde_json::json!({}),
            }],
            vec![],
        );
        let permissions = ScriptedPermissionHandler::new([if cancel {
            PermissionStep::Pending
        } else {
            PermissionStep::Error(machine_god_core::PermissionError::new(
                "unavailable",
                "permission unavailable",
            ))
        }]);
        let mut fixture = Fixture::with_engine_setup(
            vec![tool_round(&[("original", "unexecuted")])],
            InMemorySessionStore::default(),
            |engine| engine.permission_handler(permissions).tool(tool.clone()),
        );
        receipt(fixture.command(
            serde_json::json!({"create":{"name":"worker","mode":"persistent","prompt":"activity"}}),
        ));
        if cancel {
            fixture.drive(|f| {
                f.manager.children[0].snapshot.head.status == ManagedAgentState::AwaitingApproval
            });
            receipt(
                fixture
                    .command(serde_json::json!({"lifecycle":{"id":"child-1","action":"cancel"}})),
            );
            settled_activity(&mut fixture);
        } else {
            fixture.drive(|f| {
                f.manager.children[0].snapshot.head.status == ManagedAgentState::Failed
                    && !f.manager.children[0].busy()
            });
        }
        assert!(tool_activity(&mut fixture).is_empty());
        assert!(tool.invocations().is_empty());
    }
}
