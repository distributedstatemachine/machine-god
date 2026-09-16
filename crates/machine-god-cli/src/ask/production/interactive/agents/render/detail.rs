//! Pure logical rows from one retained bounded native inspection result.
use super::prefix;
use machine_god_core::{
    ManagedEventKind, ManagedInspection, ManagedRequested, ManagedSubagentResult,
};

pub(super) fn visit(result: Option<&ManagedSubagentResult>, mut emit: impl FnMut(&str)) {
    let Some(result) = result else {
        emit("No inspection result yet");
        return;
    };
    emit(&format!(
        "{:?}{}",
        result.status,
        if result.ok { "" } else { " — rejected" }
    ));
    if let Some(error) = result.error_code {
        emit(&format!("{error:?}; /refresh before retry"));
    }
    match &result.requested {
        Some(ManagedRequested::Inspection(inspection)) => inspect(inspection, &mut emit),
        Some(ManagedRequested::Receipt(receipt)) => {
            emit(&format!(
                "{:?} · generation {} · event {}",
                receipt.outcome, receipt.generation, receipt.event_sequence
            ));
            if let Some(id) = &result.child_id {
                emit(&format!("child: {}", prefix(id)));
            }
        }
        Some(ManagedRequested::RelationshipApproval(_)) => {
            emit("Relationship approval pending in the human inbox");
        }
        None => {}
    }
}

fn inspect(value: &ManagedInspection, emit: &mut impl FnMut(&str)) {
    if value.restart_required {
        emit("Inspection changed; /refresh required");
    }
    if let Some(status) = value.status {
        emit(&format!("State: {status:?}"));
    }
    if let Some(work) = &value.failure_work_id {
        emit(&format!("Failed work: {}", prefix(work)));
    }
    if let Some(reason) = &value.failure_reason {
        emit(&format!("Failure: {}", prefix(reason)));
    }
    if value.relationship_selected {
        emit(&format!(
            "Parent: {}",
            value.parent_id.as_deref().map_or("detached", prefix)
        ));
    }
    if let Some(configuration) = &value.configuration {
        configuration_rows(configuration, emit);
    }
    for item in &value.messages {
        emit(&format!(
            "Message {} · {:?} · {}",
            prefix(&item.id),
            item.status,
            item.created_at_ms
        ));
        emit(&format!("From: {}", prefix(&item.source_id)));
        emit(&format!("Content: {}", prefix(&item.content)));
        if let Some(reason) = &item.cancellation_reason {
            emit(&format!("Cancellation: {}", prefix(reason)));
        }
    }
    history_rows(value, emit);
    for item in &value.tool_activity {
        emit(&format!(
            "Tool #{} · {} · {:?} · {}",
            item.sequence,
            prefix(&item.tool_name),
            item.phase,
            item.timestamp_ms
        ));
        if let Some(work) = &item.work_id {
            emit(&format!("Work: {}", prefix(work)));
        }
    }
    if value.tool_activity_truncated {
        emit("Tool history has a retained gap");
    }
    if let Some(error) = value.tool_activity_error {
        emit(&format!("Tool history unavailable: {error:?}"));
    }
    for event in &value.events {
        emit(&format!(
            "Event #{} · revision {} · {}",
            event.sequence, event.revision, event.timestamp_ms
        ));
        event_rows(&event.kind, emit);
    }
}

fn history_rows(value: &ManagedInspection, emit: &mut impl FnMut(&str)) {
    if let Some(count) = value.history_len {
        emit(&format!("History entries: {count}"));
    }
    for item in &value.history {
        emit(&format!("History: {:?}", item.kind));
        if let Some(work) = &item.work_id {
            emit(&format!("Work: {}", prefix(work)));
        }
        if let Some(text) = &item.user {
            emit(&format!(
                "User{}: {}",
                if item.user_truncated {
                    " (retained prefix)"
                } else {
                    ""
                },
                prefix(text)
            ));
        }
        if let Some(text) = &item.assistant {
            emit(&format!(
                "Agent{}: {}",
                if item.assistant_truncated {
                    " (retained prefix)"
                } else {
                    ""
                },
                prefix(text)
            ));
        }
    }
    if value.history_truncated {
        emit("Conversation history has a retained gap");
    }
    if let Some(error) = value.history_error {
        emit(&format!("Conversation history unavailable: {error:?}"));
    }
}

fn configuration_rows(value: &machine_god_core::ManagedConfiguration, emit: &mut impl FnMut(&str)) {
    emit(&format!("Name: {}", prefix(&value.name)));
    emit(&format!(
        "Model: {}",
        value.model.as_deref().map_or("inherited", prefix)
    ));
    emit(&format!(
        "Effort: {}",
        value.effort.as_deref().map_or("inherited", prefix)
    ));
    emit(&format!("Permission: {:?}", value.permission_mode));
    let notices = &value.notifications;
    emit(&format!(
        "Notify: complete={} failure={} cancel={} start={}",
        notices.terminal.completed,
        notices.terminal.failed,
        notices.terminal.cancelled,
        notices.started
    ));
    emit(&format!(
        "Report interval (ms): {:?}",
        notices.report_interval_ms
    ));
    emit(&format!(
        "Report duration (ms): {:?}",
        notices.report_duration_ms
    ));
    for milestone in &notices.milestones {
        emit(&format!("Milestone: {}", prefix(milestone)));
    }
    for condition in &notices.stop_conditions {
        emit(&format!("Stop condition: {condition:?}"));
    }
}

fn event_rows(value: &ManagedEventKind, emit: &mut impl FnMut(&str)) {
    match value {
        ManagedEventKind::Created => emit("Created"),
        ManagedEventKind::Configured => emit("Configured"),
        ManagedEventKind::MessageQueued { message_id } => {
            emit(&format!("Message queued: {}", prefix(message_id)));
        }
        ManagedEventKind::RelationshipChanged {
            previous_parent_id,
            parent_id,
        } => {
            emit(&format!(
                "Previous parent: {}",
                previous_parent_id.as_deref().map_or("detached", prefix)
            ));
            emit(&format!(
                "New parent: {}",
                parent_id.as_deref().map_or("detached", prefix)
            ));
        }
        ManagedEventKind::LifecycleChanged { previous, current } => {
            emit(&format!("Lifecycle: {previous:?} → {current:?}"));
        }
        ManagedEventKind::WorkTransition {
            work_item_id,
            previous,
            current,
            reason,
        } => {
            emit(&format!(
                "Work {}: {previous:?} → {current:?}",
                prefix(work_item_id)
            ));
            if let Some(reason) = reason {
                emit(&format!("Reason: {}", prefix(reason)));
            }
        }
        ManagedEventKind::MilestoneRecorded {
            source_child_id,
            target_parent_id,
            work_item_id,
            name,
            notice_emitted,
            ..
        } => {
            emit(&format!("Milestone: {}", prefix(name)));
            emit(&format!("From: {}", prefix(source_child_id)));
            emit(&format!(
                "To: {}",
                target_parent_id.as_deref().map_or("none", prefix)
            ));
            emit(if *notice_emitted {
                "Notice emitted: yes"
            } else {
                "Notice emitted: no"
            });
            emit(&format!("Work: {}", prefix(work_item_id)));
        }
    }
}
