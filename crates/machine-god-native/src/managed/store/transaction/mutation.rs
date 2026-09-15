use super::super::records::{
    JournalFailure, JournalHead, JournalIntent, JournalMutation, JournalPageRef, JournalRecord,
    JournalWork, JournalWorkRef,
};
use super::super::{JournalError as Error, JournalLimits};
use super::validation;
use machine_god_core::{
    ManagedAgentMode as Mode, ManagedAgentState as State, ManagedQueueStatus as Status,
};

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

pub(super) fn apply(
    head: &mut JournalHead,
    mutation: JournalMutation,
    limits: JournalLimits,
) -> Result<Vec<JournalRecord>, Error> {
    if head.status == State::Archived
        && !matches!(
            mutation,
            JournalMutation::Reopen(_) | JournalMutation::Recover
        )
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
        } => {
            if let Some(parent) = &parent_id {
                validation::id(parent)?;
            }
            if parent_id.is_some() != parent_owner.is_some() {
                return Err(Error::Invalid);
            }
            head.parent_id = parent_id;
            head.parent_owner = parent_owner;
        }
        JournalMutation::NoticeCursor(cursor) => {
            if cursor < head.notice_cursor || cursor >= head.next_sequence {
                return Err(Error::Conflict);
            }
            head.notice_cursor = cursor;
        }
        JournalMutation::AppendHistory(records) => {
            validate_history(&records)?;
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
