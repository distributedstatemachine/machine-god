use super::super::records::{
    JournalFailure, JournalHead, JournalIntent, JournalMutation, JournalPageRef, JournalRecord,
    JournalWork, JournalWorkRef,
};
use super::super::{JournalError as Error, JournalLimits};
use super::validation;
use machine_god_core::{
    ManagedAgentMode as Mode, ManagedAgentState as State, ManagedEvent, ManagedEventKind,
    ManagedQueueStatus as Status,
};

pub(super) fn event(head: &JournalHead, kind: ManagedEventKind) -> Result<JournalRecord, Error> {
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Invalid)?
        .as_millis()
        .try_into()
        .map_err(|_| Error::Exhausted)?;
    Ok(JournalRecord::Event(ManagedEvent {
        sequence: head.next_sequence,
        revision: head.revision,
        id: format!("event-{}", head.next_sequence),
        timestamp_ms,
        kind,
    }))
}

pub(super) fn apply(
    head: &mut JournalHead,
    mutation: JournalMutation,
    limits: JournalLimits,
) -> Result<Vec<JournalRecord>, Error> {
    let mut kind = match &mutation {
        JournalMutation::Enqueue(work) => Some(ManagedEventKind::MessageQueued {
            message_id: work.id.clone(),
        }),
        JournalMutation::HeadState {
            work_id,
            status,
            failure,
        } => Some(ManagedEventKind::WorkTransition {
            work_item_id: work_id.clone(),
            previous: head.queue.first().map(|work| work.status),
            current: *status,
            reason: failure.clone(),
        }),
        JournalMutation::Configure(_) => Some(ManagedEventKind::Configured),
        JournalMutation::Milestone {
            operation_id,
            work_id,
            name,
            notice,
            ..
        } => Some(ManagedEventKind::MilestoneRecorded {
            operation_id: operation_id.clone(),
            source_child_id: head.id.clone(),
            target_parent_id: notice
                .as_ref()
                .map(|notice| notice.target.parent.id.clone())
                .or_else(|| head.parent_id.clone()),
            notice_emitted: notice.is_some(),
            work_item_id: work_id.clone(),
            name: name.clone(),
        }),
        JournalMutation::Relationship { parent_id, .. } => {
            Some(ManagedEventKind::RelationshipChanged {
                previous_parent_id: head.parent_id.clone(),
                parent_id: parent_id.clone(),
            })
        }
        JournalMutation::Intent(_)
        | JournalMutation::CancelIdle
        | JournalMutation::ResolveHead { .. }
        | JournalMutation::Archive
        | JournalMutation::Reopen(_)
        | JournalMutation::Recover => Some(ManagedEventKind::LifecycleChanged {
            previous: head.status,
            current: head.status,
        }),
        JournalMutation::SuppressedNotice(_) | JournalMutation::AppendHistory(_) => None,
    };
    let mut records = apply_inner(head, mutation, limits)?;
    if let Some(ManagedEventKind::LifecycleChanged { current, .. }) = &mut kind {
        *current = head.status;
    }
    if let Some(kind) = kind {
        records.insert(0, event(head, kind)?);
    }
    Ok(records)
}

pub(super) fn enqueue(
    head: &mut JournalHead,
    work: JournalWork,
    limits: JournalLimits,
) -> Result<Vec<JournalRecord>, Error> {
    validation::work(&work)?;
    if head.status == State::Archived
        || head.intent.is_some()
        || (head.mode == Mode::OneOff && (head.status != State::Idle || !head.queue.is_empty()))
    {
        return Err(Error::Conflict);
    }
    if head.queue.len() >= limits.queue_entries {
        return Err(Error::Limit);
    }
    if head.queue.iter().any(|item| item.id == work.id) {
        return Err(Error::Conflict);
    }
    if work.configuration != head.configuration {
        return Err(Error::Conflict);
    }
    head.queue.push(JournalWorkRef {
        id: work.id.clone(),
        status: Status::Pending,
        page: JournalPageRef {
            child_id: head.id.clone(),
            owner: head.transcript.clone(),
            generation: head.generation,
            sequence: 0,
            length: 0,
            digest: [0; 32],
        },
    });
    if head.queue.len() == 1 {
        head.status = State::Queued;
    }
    Ok(vec![JournalRecord::WorkAccepted(work)])
}

#[allow(clippy::too_many_lines)] // Exhaustive typed transaction validation without effects.
fn apply_inner(
    head: &mut JournalHead,
    mutation: JournalMutation,
    limits: JournalLimits,
) -> Result<Vec<JournalRecord>, Error> {
    if head.status == State::Archived
        && !matches!(
            mutation,
            JournalMutation::Reopen(_) | JournalMutation::Recover
        )
        && !matches!(&mutation, JournalMutation::AppendHistory(records) if records.iter().all(|record| matches!(record, JournalRecord::NoticeAcknowledged { .. })))
    {
        return Err(Error::Conflict);
    }
    match mutation {
        JournalMutation::Enqueue(work) => return enqueue(head, work, limits),
        JournalMutation::HeadState {
            work_id,
            status,
            failure,
        } => {
            if failure
                .as_ref()
                .is_some_and(|text| text.is_empty() || text.len() > 4096 || text.contains('\0'))
            {
                return Err(Error::Invalid);
            }
            change_state(head, &work_id, status, failure.clone())?;
            return Ok(vec![JournalRecord::WorkState {
                work_id,
                status,
                failure,
            }]);
        }
        JournalMutation::Intent(intent) => {
            head.intent = Some(intent);
        }
        JournalMutation::CancelIdle => {
            if head.intent != Some(JournalIntent::Cancel) || !head.queue.is_empty() {
                return Err(Error::Conflict);
            }
            head.intent = None;
            head.status = if head.mode == Mode::Persistent {
                State::Idle
            } else {
                State::Cancelled
            };
        }
        JournalMutation::ResolveHead { work_id, retry } => {
            resolve(head, &work_id, retry)?;
            return Ok(vec![JournalRecord::WorkResolved { work_id, retry }]);
        }
        JournalMutation::Configure(config) => {
            validation::configuration(&config)?;
            head.configuration = config.clone();
            return Ok(vec![JournalRecord::Configuration(config)]);
        }
        JournalMutation::Relationship {
            parent_id,
            parent_owner,
            parent_generation,
        } => {
            validation::relationship(
                parent_id.as_deref(),
                parent_owner.as_ref(),
                parent_generation,
            )?;
            head.parent_id = parent_id;
            head.parent_owner = parent_owner;
            head.parent_generation = parent_generation;
        }
        JournalMutation::SuppressedNotice(sequence) => {
            if sequence != head.next_sequence || sequence <= head.notice_cursor {
                return Err(Error::Conflict);
            }
            head.notice_cursor = sequence;
        }
        JournalMutation::Milestone {
            notice,
            consume_sequence,
            work_id,
            name,
            ..
        } => {
            if head.queue.first().is_none_or(|work| {
                work.id != work_id
                    || !matches!(work.status, Status::Running | Status::AwaitingApproval)
            }) {
                return Err(Error::Conflict);
            }
            if let Some(notice) = notice {
                if !consume_sequence
                    || notice.source.work_id != work_id
                    || notice.event != (crate::managed::notices::NoticeEvent::Milestone { name })
                {
                    return Err(Error::Invalid);
                }
                return apply_inner(
                    head,
                    JournalMutation::AppendHistory(vec![JournalRecord::Notice(notice)]),
                    limits,
                );
            }
            if consume_sequence {
                let sequence = head.next_sequence;
                return apply_inner(head, JournalMutation::SuppressedNotice(sequence), limits);
            }
        }
        JournalMutation::AppendHistory(records) => {
            validate_history(&records)?;
            for record in &records {
                if let JournalRecord::Notice(notice) = record {
                    if notice.source.source.id != head.id
                        || notice.source.source.generation.get() != head.generation
                        || notice.source_sequence.get() != head.next_sequence
                        || notice.source_sequence.get() <= head.notice_cursor
                    {
                        return Err(Error::Conflict);
                    }
                    head.notice_cursor = notice.source_sequence.get();
                }
            }
            return Ok(records);
        }
        JournalMutation::Archive => {
            if head.intent != Some(JournalIntent::Archive)
                || head
                    .queue
                    .iter()
                    .any(|item| matches!(item.status, Status::Running | Status::AwaitingApproval))
            {
                return Err(Error::Conflict);
            }
            for item in &mut head.queue {
                item.status = Status::Interrupted;
            }
            head.status = State::Archived;
            head.intent = None;
        }
        JournalMutation::Reopen(transcript) => {
            if head.status != State::Archived {
                return Err(Error::Conflict);
            }
            head.generation = head.generation.checked_add(1).ok_or(Error::Exhausted)?;
            head.transcript = transcript;
            head.status = if head.queue.is_empty() {
                State::Idle
            } else {
                State::Interrupted
            };
            head.intent = None;
        }
        JournalMutation::Recover => {
            recover(head);
        }
    }
    Ok(Vec::new())
}

fn recover(head: &mut JournalHead) {
    if head.status == State::Archived {
        return;
    }
    let mut interrupted = false;
    for item in &mut head.queue {
        if matches!(
            item.status,
            Status::Pending | Status::Running | Status::AwaitingApproval
        ) {
            item.status = Status::Interrupted;
            interrupted = true;
        }
    }
    if interrupted {
        head.status = State::Interrupted;
    }
    // Durable intent remains evidence; recovery itself never signals.
}

fn validate_history(records: &[JournalRecord]) -> Result<(), Error> {
    if records.len() > 100 {
        return Err(Error::Limit);
    }
    validation::records(records)?;
    if records.iter().any(|item| {
        !matches!(
            item,
            JournalRecord::History(_)
                | JournalRecord::Event(_)
                | JournalRecord::Tool(_)
                | JournalRecord::Notice(_)
                | JournalRecord::NoticeAcknowledged { .. }
        )
    }) {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn change_state(
    head: &mut JournalHead,
    work_id: &str,
    status: Status,
    failure: Option<String>,
) -> Result<(), Error> {
    let first = head.queue.first_mut().ok_or(Error::Conflict)?;
    if first.id != work_id {
        return Err(Error::Conflict);
    }
    let allowed = match status {
        Status::Running => {
            matches!(first.status, Status::Pending | Status::AwaitingApproval)
                && head.intent.is_none()
        }
        Status::AwaitingApproval => first.status == Status::Running && head.intent.is_none(),
        Status::Interrupted => matches!(
            first.status,
            Status::Pending | Status::Running | Status::AwaitingApproval
        ),
        Status::Completed | Status::Failed => {
            matches!(first.status, Status::Running | Status::AwaitingApproval)
                && head.intent.is_none()
        }
        Status::Cancelled => head.intent.is_some(),
        Status::Pending => false, // Explicit resolution, never an implicit retry.
    };
    if !allowed || (status == Status::Failed) != failure.is_some() {
        return Err(Error::Conflict);
    }
    first.status = status;
    head.failure = failure.map(|reason| JournalFailure {
        work_id: work_id.to_owned(),
        reason,
    });
    match status {
        Status::Completed | Status::Cancelled => {
            head.queue.remove(0);
            let cancelled =
                status == Status::Cancelled && head.intent == Some(JournalIntent::Cancel);
            if cancelled {
                head.intent = None;
                for work in &mut head.queue {
                    work.status = Status::Interrupted;
                }
            }
            head.status = if head.mode == Mode::OneOff {
                if status == Status::Completed {
                    State::Completed
                } else {
                    State::Cancelled
                }
            } else if cancelled {
                State::Idle
            } else {
                next_state(head)
            };
        }
        Status::Running => head.status = State::Running,
        Status::AwaitingApproval => head.status = State::AwaitingApproval,
        Status::Interrupted => head.status = State::Interrupted,
        Status::Failed => head.status = State::Failed,
        Status::Pending => unreachable!(),
    }
    Ok(())
}
fn resolve(head: &mut JournalHead, work_id: &str, retry: bool) -> Result<(), Error> {
    let first = head.queue.first_mut().ok_or(Error::Conflict)?;
    if first.id != work_id
        || head.intent.is_some()
        || !matches!(
            first.status,
            Status::Interrupted | Status::Failed | Status::AwaitingApproval
        )
    {
        return Err(Error::Conflict);
    }
    if retry {
        first.status = Status::Pending;
    } else {
        head.queue.remove(0);
    }
    head.failure = None;
    head.status = next_state(head);
    Ok(())
}
fn next_state(head: &JournalHead) -> State {
    match head.queue.first().map(|item| item.status) {
        None => State::Idle,
        Some(Status::Pending) => State::Queued,
        Some(Status::Failed) => State::Failed,
        Some(_) => State::Interrupted,
    }
}
