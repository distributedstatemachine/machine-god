use super::super::records::{JournalHead, JournalPageRef, JournalRecord, JournalWork};
use super::super::{JournalError as Error, JournalLimits};
use machine_god_core::{ManagedConfiguration, ManagedEventKind};

pub(super) const MAX_RECORDS: usize = 101;
pub(super) fn id(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > 255
        || matches!(value, "." | "..")
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
fn text(value: &str, limit: usize, empty: bool) -> Result<(), Error> {
    if (!empty && value.is_empty()) || value.len() > limit || value.contains('\0') {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
pub(super) fn configuration(config: &ManagedConfiguration) -> Result<(), Error> {
    text(&config.name, 128, false)?;
    if let Some(model) = &config.model {
        text(model, 256, false)?;
    }
    if let Some(effort) = &config.effort {
        text(effort, 64, false)?;
        if !effort
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(Error::Invalid);
        }
    }
    let policy = &config.notifications;
    if policy.milestones.len() > 32 || policy.stop_conditions.len() > 8 {
        return Err(Error::Limit);
    }
    for milestone in &policy.milestones {
        text(milestone, 128, false)?;
    }
    let mut normalized = policy.clone();
    normalized.normalize().map_err(|_| Error::Invalid)?;
    if normalized != *policy {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub(super) fn work(work: &JournalWork) -> Result<(), Error> {
    id(&work.id)?;
    id(&work.source_id)?;
    text(&work.content, 65_536, false)?;
    crate::skills_invocation::validate_references(&work.skills).map_err(|_| Error::Limit)?;
    configuration(&work.configuration)
}
pub(super) fn reference(value: &JournalPageRef, limits: JournalLimits) -> Result<(), Error> {
    id(&value.child_id)?;
    if value.generation == 0
        || value.sequence == 0
        || value.length == 0
        || value.length > limits.page_bytes
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub(super) fn head(head: &JournalHead, limits: JournalLimits) -> Result<(), Error> {
    id(&head.id)?;
    configuration(&head.configuration)?;
    if head.version != 1
        || head.owner_epoch == 0
        || head.generation == 0
        || head.revision == 0
        || head.next_sequence == 0
        || head.last_event_sequence == 0
        || head.last_event_sequence >= head.next_sequence
        || head.queue.len() > limits.queue_entries
        || head.parent_id.is_some() != head.parent_owner.is_some()
    {
        return Err(Error::Invalid);
    }
    if let Some(parent) = &head.parent_id {
        id(parent)?;
        if parent == &head.id {
            return Err(Error::Invalid);
        }
    }
    if let Some(failure) = &head.failure {
        id(&failure.work_id)?;
        text(&failure.reason, 4096, false)?;
    }
    for (index, work) in head.queue.iter().enumerate() {
        id(&work.id)?;
        if head.queue[..index].iter().any(|old| old.id == work.id) {
            return Err(Error::Invalid);
        }
        reference(&work.page, limits)?;
        if work.page.child_id != head.id
            || (work.page.generation == head.generation && work.page.owner != head.transcript)
            || work.page.generation > head.generation
            || work.page.sequence >= head.next_sequence
        {
            return Err(Error::Invalid);
        }
    }
    if let Some(tail) = &head.history_tail {
        reference(tail, limits)?;
        if tail.child_id != head.id
            || (tail.generation == head.generation && tail.owner != head.transcript)
            || tail.generation > head.generation
            || tail.sequence >= head.next_sequence
        {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}
#[allow(clippy::too_many_lines)] // Exhaustive bounded durable record schema validation.
pub(super) fn records(records: &[JournalRecord]) -> Result<(), Error> {
    if records.is_empty() || records.len() > MAX_RECORDS {
        return Err(Error::Limit);
    }
    for record in records {
        match record {
            JournalRecord::NoticeAcknowledged {
                identity,
                target,
                checkpoint,
            } => {
                use crate::managed::notices::NoticeKind;
                id(&identity.source.source.id)?;
                id(&identity.source.work_id)?;
                id(&target.parent.id)?;
                if checkpoint.session_id.as_str() != target.parent.id
                    || checkpoint.turn_sequence == 0
                {
                    return Err(Error::Invalid);
                }
                match &identity.kind {
                    NoticeKind::Milestone { name } => text(name, 128, false)?,
                    NoticeKind::Interval {
                        first_tick,
                        last_tick,
                    } if first_tick > last_tick => return Err(Error::Invalid),
                    _ => {}
                }
            }
            JournalRecord::Notice(value) => {
                use crate::managed::notices::NoticeEvent;
                id(&value.source.source.id)?;
                id(&value.source.work_id)?;
                id(&value.target.parent.id)?;
                if let Some(history) = &value.history {
                    id(&history.record_id)?;
                }
                match &value.event {
                    NoticeEvent::Milestone { name } => text(name, 128, false)?,
                    NoticeEvent::Interval {
                        first_tick,
                        last_tick,
                        coalesced_intervals,
                        gap,
                        ..
                    } => {
                        if last_tick < first_tick
                            || last_tick.get() - first_tick.get() + 1 != coalesced_intervals.get()
                            || *gap != (coalesced_intervals.get() > 1)
                        {
                            return Err(Error::Invalid);
                        }
                    }
                    NoticeEvent::Started | NoticeEvent::Terminal { .. } => {}
                }
            }
            JournalRecord::WorkAccepted(value) => work(value)?,
            JournalRecord::Configuration(value) => configuration(value)?,
            JournalRecord::WorkState {
                work_id, failure, ..
            } => {
                id(work_id)?;
                if let Some(reason) = failure {
                    text(reason, 4096, false)?;
                }
            }
            JournalRecord::WorkResolved { work_id, .. } => id(work_id)?,
            JournalRecord::Control(value) => {
                if value.revision == 0 || value.parent_id.is_some() != value.parent_owner.is_some()
                {
                    return Err(Error::Invalid);
                }
                if let Some(parent) = &value.parent_id {
                    id(parent)?;
                }
                if let Some(failure) = &value.failure {
                    id(&failure.work_id)?;
                    text(&failure.reason, 4096, false)?;
                }
            }
            JournalRecord::History(value) => {
                if let Some(work) = &value.work_id {
                    id(work)?;
                }
                for value in [&value.user, &value.assistant].into_iter().flatten() {
                    // Model text is data; unlike input labels, NUL is valid
                    // inside a bounded serialized assistant/history field.
                    if value.len() > 16 * 1024 {
                        return Err(Error::Invalid);
                    }
                }
            }
            JournalRecord::Tool(value) => {
                if let Some(work) = &value.work_id {
                    id(work)?;
                }
                text(&value.tool_name, 256, false)?;
            }
            JournalRecord::Event(value) => {
                id(&value.id)?;
                if value.sequence == 0 || value.revision == 0 {
                    return Err(Error::Invalid);
                }
                match &value.kind {
                    ManagedEventKind::Created
                    | ManagedEventKind::Configured
                    | ManagedEventKind::LifecycleChanged { .. } => {}
                    ManagedEventKind::MessageQueued { message_id } => id(message_id)?,
                    ManagedEventKind::RelationshipChanged {
                        previous_parent_id,
                        parent_id,
                    } => {
                        for parent in [previous_parent_id, parent_id].into_iter().flatten() {
                            id(parent)?;
                        }
                    }
                    ManagedEventKind::WorkTransition {
                        work_item_id,
                        reason,
                        ..
                    } => {
                        id(work_item_id)?;
                        if let Some(reason) = reason {
                            text(reason, 4096, true)?;
                        }
                    }
                    ManagedEventKind::MilestoneRecorded {
                        operation_id,
                        source_child_id,
                        target_parent_id,
                        work_item_id,
                        name,
                        notice_emitted,
                    } => {
                        text(operation_id, 128, false)?;
                        if operation_id
                            .bytes()
                            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
                        {
                            return Err(Error::Invalid);
                        }
                        if *notice_emitted && target_parent_id.is_none() {
                            return Err(Error::Invalid);
                        }
                        if let Some(parent) = target_parent_id {
                            id(parent)?;
                        }
                        for id_value in [source_child_id, work_item_id] {
                            id(id_value)?;
                        }
                        text(name, 128, false)?;
                    }
                }
            }
        }
    }
    Ok(())
}
