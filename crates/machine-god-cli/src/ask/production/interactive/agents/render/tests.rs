use super::{Lines, detail, detail_count, details, identity_rows, write_identity};
use machine_god_core::{
    ManagedAgentState, ManagedConfiguration, ManagedEvent, ManagedEventKind, ManagedHistoryItem,
    ManagedHistoryKind, ManagedInspection, ManagedInspectionSourceError, ManagedNotifications,
    ManagedPermissionMode, ManagedQueueStatus, ManagedQueuedMessage, ManagedRequested,
    ManagedResultStatus, ManagedSubagentResult, ManagedToolActivity, ManagedToolPhase,
};

fn envelope(inspection: ManagedInspection) -> ManagedSubagentResult {
    ManagedSubagentResult {
        ok: true,
        operation_id: "display-fixture".into(),
        child_id: Some("child".into()),
        status: ManagedResultStatus::Inspected,
        error_code: None,
        retryable: false,
        requested: Some(ManagedRequested::Inspection(Box::new(inspection))),
        cursor: None,
    }
}

fn inspection() -> ManagedInspection {
    ManagedInspection {
        status: Some(ManagedAgentState::Interrupted),
        restart_required: true,
        failure_work_id: Some("failed-work".into()),
        failure_reason: Some("interrupted-work".into()),
        relationship_selected: true,
        parent_id: Some("actual-parent".into()),
        messages: vec![ManagedQueuedMessage {
            id: "queued-message".into(),
            source_id: "source-owner".into(),
            content: "queued-content".into(),
            status: ManagedQueueStatus::Cancelled,
            cancellation_reason: Some("cancel-reason".into()),
            created_at_ms: 12,
        }],
        history: vec![ManagedHistoryItem {
            kind: ManagedHistoryKind::Conversation,
            work_id: Some("history-work".into()),
            user: Some("history-user".into()),
            assistant: Some("history-answer".into()),
            user_truncated: true,
            assistant_truncated: false,
        }],
        history_len: Some(7),
        history_truncated: true,
        history_error: Some(ManagedInspectionSourceError::Unavailable),
        tool_activity: vec![ManagedToolActivity {
            sequence: 8,
            revision: 9,
            timestamp_ms: 13,
            work_id: Some("tool-work".into()),
            tool_name: "read_file".into(),
            phase: ManagedToolPhase::Denied,
        }],
        tool_activity_truncated: true,
        tool_activity_error: Some(ManagedInspectionSourceError::Invalid),
        events: vec![ManagedEvent {
            sequence: 10,
            revision: 11,
            id: "event-id".into(),
            timestamp_ms: 14,
            kind: ManagedEventKind::MilestoneRecorded {
                operation_id: "operation".into(),
                source_child_id: "milestone-child".into(),
                target_parent_id: Some("milestone-parent".into()),
                work_item_id: "milestone-work".into(),
                name: "milestone-name".into(),
                notice_emitted: true,
            },
        }],
        ..ManagedInspection::default()
    }
}

#[test]
fn details_include_selected_sources_failures_and_retained_gap_evidence() {
    let result = envelope(inspection());
    let mut rows = Vec::new();
    detail::visit(Some(&result), |row| rows.push(row.to_owned()));
    assert_eq!(detail_count(Some(&result)), rows.len());
    let text = rows.join("\n");
    for expected in [
        "Interrupted",
        "refresh required",
        "failed-work",
        "interrupted-work",
        "actual-parent",
        "queued-message",
        "source-owner",
        "queued-content",
        "cancel-reason",
        "history-work",
        "history-user",
        "history-answer",
        "retained prefix",
        "History entries: 7",
        "Conversation history has a retained gap",
        "Conversation history unavailable: Unavailable",
        "read_file",
        "Denied",
        "tool-work",
        "Tool history has a retained gap",
        "Tool history unavailable: Invalid",
        "Event #10",
        "revision 11",
        "milestone-name",
        "milestone-child",
        "milestone-parent",
        "milestone-work",
        "Notice emitted: yes",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
}

#[test]
fn detached_milestone_does_not_claim_a_parent_or_notice() {
    let mut selected = inspection();
    let ManagedEventKind::MilestoneRecorded {
        target_parent_id,
        notice_emitted,
        ..
    } = &mut selected.events[0].kind
    else {
        panic!("milestone fixture");
    };
    *target_parent_id = None;
    *notice_emitted = false;
    let result = envelope(selected);
    let mut rows = Vec::new();
    detail::visit(Some(&result), |row| rows.push(row.to_owned()));
    let text = rows.join("\n");
    assert!(text.contains("To: none"));
    assert!(text.contains("Notice emitted: no"));
    assert!(!text.contains("milestone-parent"));
}

#[test]
fn configuration_includes_notification_policy_and_detached_relationship() {
    let result = envelope(ManagedInspection {
        relationship_selected: true,
        configuration: Some(ManagedConfiguration {
            name: "configured-name".into(),
            model: Some("configured-model".into()),
            effort: Some("configured-effort".into()),
            permission_mode: ManagedPermissionMode::Ask,
            notifications: ManagedNotifications {
                started: true,
                milestones: vec!["ready".into()],
                report_interval_ms: Some(1000),
                report_duration_ms: Some(2000),
                ..ManagedNotifications::default()
            },
        }),
        ..ManagedInspection::default()
    });
    let mut rows = Vec::new();
    detail::visit(Some(&result), |row| rows.push(row.to_owned()));
    let text = rows.join("\n");
    for expected in [
        "detached",
        "configured-name",
        "configured-model",
        "configured-effort",
        "start=true",
        "1000",
        "2000",
        "Milestone: ready",
        "Stop condition: Terminal",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
}

#[test]
fn scrolling_reaches_every_row_with_a_bounded_sanitized_window() {
    let result = envelope(inspection());
    let mut rows = Vec::new();
    detail::visit(Some(&result), |row| rows.push(row.to_owned()));
    for (offset, expected) in rows.iter().enumerate() {
        let mut lines = Lines {
            bytes: Vec::new(),
            columns: 80,
            limit: 11,
            count: 4,
        };
        details(&mut lines, Some(&result), 6, offset).unwrap();
        assert_eq!(lines.count, 6);
        let expected = crate::ask::production::interactive::composer_view::label(expected, 80, 512);
        assert!(
            lines
                .bytes
                .windows(expected.len())
                .any(|window| window == expected)
        );
        assert!(lines.bytes.len() < 1028);
    }
    let mut source = inspection();
    source.failure_reason = Some("\x1b]52;c;credential\x07\r\n".repeat(2000));
    let result = envelope(source);
    let mut lines = Lines {
        bytes: Vec::new(),
        columns: 200,
        limit: 64,
        count: 0,
    };
    details(&mut lines, Some(&result), 64, 0).unwrap();
    assert!(!lines.bytes.contains(&27));
    assert!(!lines.bytes.contains(&7));
    assert!(lines.bytes.len() < 64 * 1024);
    assert!(lines.count <= 64);
}

#[test]
fn identity_wrapping_never_omits_the_distinguishing_suffix() {
    for columns in [40, 41, 80, 256, u16::MAX] {
        for length in [1, 36, 128, 255] {
            let id = format!("{}Z", "a".repeat(length - 1));
            let mut lines = Lines {
                bytes: Vec::new(),
                columns,
                limit: 64,
                count: 0,
            };
            write_identity(&mut lines, &id).unwrap();
            assert_eq!(lines.count, identity_rows(&id, columns).unwrap());
            let rendered = String::from_utf8(lines.bytes).unwrap();
            assert_eq!(rendered.replace("\r\n", ""), format!("id: {id}"));
        }
    }
    for invalid in ["", "bad\nidentity", "control\x1b", "非ascii"] {
        assert_eq!(identity_rows(invalid, 40), None);
    }
}
