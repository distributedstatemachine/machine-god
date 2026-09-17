//! Cancellation of accepted work needs no runtime. Its frozen policy and actual
//! attempt identity come from durable originals, never current configuration.
use super::super::notices::{
    ManagedNotice, NoticeEvent, NoticeHistoryRef, NoticePrincipal, NoticeTarget, NoticeTerminal,
    WorkNoticeIdentity,
};
use super::{JournalMutation, JournalRecord, JournalSnapshot, ManagedJournal, durability::Failure};
use crate::managed::store::{JournalError, JournalIntent};
use machine_god_core::{ManagedEventKind, ManagedQueueStatus};
use std::num::NonZeroU64;

pub(super) async fn prepare(
    journal: &ManagedJournal,
    snapshot: &JournalSnapshot,
    mutation: &JournalMutation,
) -> Result<Option<JournalMutation>, Failure> {
    let JournalMutation::HeadState {
        work_id,
        status: ManagedQueueStatus::Cancelled,
        failure: None,
    } = mutation
    else {
        return Ok(None);
    };
    let reference = snapshot
        .head
        .queue
        .first()
        .filter(|work| work.id == *work_id)
        .ok_or(Failure::Rejected(JournalError::Conflict))?;
    let work = journal
        .read_work(reference.page.clone())
        .await
        .map_err(Failure::Rejected)?;
    // Close settles its turn as Cancelled too, but must stop notifications.
    let notice = if snapshot.head.intent == Some(JournalIntent::Cancel)
        && work.configuration.notifications.terminal.cancelled
        && snapshot.head.parent_owner.is_some()
    {
        let (attempt, terminal) = attempt(journal, snapshot, work_id).await?;
        if terminal {
            None
        } else {
            let parent = snapshot.head.parent_owner.as_ref().unwrap();
            let sequence = positive(snapshot.head.next_sequence)?;
            Some(ManagedNotice {
                source: WorkNoticeIdentity {
                    source: NoticePrincipal {
                        id: snapshot.head.id.clone(),
                        generation: positive(snapshot.head.generation)?,
                    },
                    work_id: work_id.clone(),
                    work_generation: attempt,
                },
                source_sequence: sequence,
                target: NoticeTarget {
                    parent: NoticePrincipal {
                        id: parent.session_id.to_string(),
                        generation: positive(
                            snapshot
                                .head
                                .parent_generation
                                .ok_or(Failure::Rejected(JournalError::Invalid))?,
                        )?,
                    },
                    parent_incarnation: parent.incarnation.clone(),
                    relationship_generation: positive(snapshot.head.revision)?,
                },
                event: NoticeEvent::Terminal {
                    outcome: NoticeTerminal::Cancelled,
                },
                history: Some(NoticeHistoryRef {
                    record_id: format!("notice-{}", sequence.get()),
                    source_sequence: sequence,
                }),
            })
        }
    } else {
        None
    };
    Ok(Some(JournalMutation::CancelHead {
        work_id: work_id.clone(),
        notice,
    }))
}

/// Newest-first pages carry their event before work/state records. The initial
/// Pending -> Running revision is the live notice tracker's exact attempt ID;
/// approval transitions must not replace it. Before initial execution, use the
/// acceptance (or explicit retry) revision without inventing a start event.
async fn attempt(
    journal: &ManagedJournal,
    snapshot: &JournalSnapshot,
    work_id: &str,
) -> Result<(NonZeroU64, bool), Failure> {
    let mut cursor = None;
    let mut revision = None;
    let mut terminal = None;
    loop {
        let page = journal
            .history(snapshot.clone(), cursor, 100)
            .await
            .map_err(Failure::Rejected)?;
        for record in page.records {
            match record {
                JournalRecord::Notice(notice)
                    if notice.source.source.generation.get() == snapshot.head.generation
                        && notice.source.work_id == work_id
                        && matches!(notice.event, NoticeEvent::Terminal { .. }) =>
                {
                    terminal = Some(notice.source.work_generation);
                }
                JournalRecord::Event(event) => {
                    let event_revision = positive(event.revision)?;
                    revision = Some(event_revision);
                    if matches!(event.kind, ManagedEventKind::WorkTransition {
                        work_item_id, previous: Some(ManagedQueueStatus::Pending),
                        current: ManagedQueueStatus::Running, ..
                    } if work_item_id == work_id)
                    {
                        return Ok((event_revision, terminal == Some(event_revision)));
                    }
                }
                JournalRecord::WorkResolved {
                    work_id: id,
                    retry: true,
                } if id == work_id => {
                    let attempt = revision.ok_or(Failure::Rejected(JournalError::Invalid))?;
                    return Ok((attempt, terminal == Some(attempt)));
                }
                JournalRecord::WorkAccepted(work) if work.id == work_id => {
                    let attempt = revision.ok_or(Failure::Rejected(JournalError::Invalid))?;
                    return Ok((attempt, terminal == Some(attempt)));
                }
                _ => {}
            }
        }
        cursor = Some(page.next.ok_or(Failure::Rejected(JournalError::Invalid))?);
    }
}

fn positive(value: u64) -> Result<NonZeroU64, Failure> {
    NonZeroU64::new(value).ok_or(Failure::Rejected(JournalError::Invalid))
}
