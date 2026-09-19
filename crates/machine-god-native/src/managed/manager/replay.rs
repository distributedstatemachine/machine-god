//! Bounded, inert replay of confirmed originals, never status-derived events.
use super::super::{
    notices::{ManagedNotice, NoticeError, NoticeRelationship},
    prompt_context::ParentNoticeContext,
    store::{JournalCatalogCursor, JournalError, JournalHistoryCursor, JournalTranscript},
};
use super::{
    Active, BoxFuture, Context, JournalMutation, JournalRecord, JournalSnapshot, ManagedAgentState,
    ManagedJournal, ManagedManager, ManagerBlock, durability,
};
use machine_god_core::ManagedNotifications;
use std::sync::{Arc, Weak};

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Replay {
    catalog: Option<JournalCatalogCursor>,
    source: Option<(JournalSnapshot, Option<JournalHistoryCursor>)>,
    catalog_done: bool,
    pending: Option<Pending>,
    validation: Option<Validation>,
    retry: Option<BoxFuture<'static, ()>>,
    rescan: bool,
    pub(super) done: bool,
}
struct Pending {
    snapshot: JournalSnapshot,
    original: ManagedNotice,
    transcript: JournalTranscript,
    fresh: bool,
}
// One resumable join against an exact source snapshot, never a history-sized
// future holding the manager's serialized journal lane.
struct Validation {
    snapshot: JournalSnapshot,
    original: ManagedNotice,
    cursor: Option<JournalHistoryCursor>,
    original_seen: bool,
    parent_checked: bool,
    parent: Option<JournalTranscript>,
}
pub(super) struct Outcome {
    replay: Replay,
    snapshot: Option<JournalSnapshot>,
    error: bool,
}
impl ManagedManager {
    pub(super) fn begin_replay(&mut self, cx: &mut Context<'_>) -> bool {
        if self.closing || self.active.is_some() {
            return false;
        }
        if self.replay_reset {
            // Coalesce writes and registrations into a subsequent sweep. A
            // busy sibling must not repeatedly erase another source's progress.
            if self.replay.done {
                self.replay = Replay::default();
            } else {
                // Before the first catalog read, the upcoming sweep already
                // includes every prior write and parent registration.
                self.replay.rescan |= self.replay.catalog.is_some() || self.replay.catalog_done;
                self.replay.retry = None;
                if let Some(pending) = &mut self.replay.pending {
                    pending.fresh = false;
                }
            }
            self.replay_reset = false;
        }
        if let Some(retry) = &mut self.replay.retry {
            if retry.as_mut().poll(cx).is_pending() {
                return false;
            }
            self.replay.retry = None;
        }
        if self
            .replay
            .pending
            .as_ref()
            .is_some_and(|pending| pending.fresh)
        {
            return self.publish_replay();
        }
        if self.replay.done {
            return false;
        }
        let targets = self
            .parents
            .iter()
            .filter(|parent| parent.active)
            .filter_map(|parent| parent.context.upgrade())
            .filter(|context| !context.is_retired())
            .map(|context| Arc::downgrade(&context))
            .collect();
        let state = std::mem::take(&mut self.replay);
        self.active = Some(Active::Replay(advance(
            self.journal.clone(),
            self.retry.clone(),
            state,
            targets,
        )));
        true
    }
    fn publish_replay(&mut self) -> bool {
        if let Some(pending) = self.replay.pending.as_ref() {
            let notice = &pending.original;
            let transcript = &pending.transcript;
            let eligible = self
                .parents
                .iter()
                .filter(|parent| parent.active)
                .filter_map(|parent| parent.context.upgrade())
                .any(|context| {
                    !context.is_retired()
                        && context.matches_transcript(transcript)
                        && context.principal() == &notice.target.parent
                        && context
                            .delivery()
                            .is_none_or(|delivery| !delivery.originals().contains(notice))
                });
            if eligible {
                let relationship = NoticeRelationship {
                    generation: notice.target.relationship_generation,
                    parent: Some(notice.target.parent.clone()),
                    parent_incarnation: Some(notice.target.parent_incarnation.clone()),
                };
                match self.notices.register_work(
                    &notice.source,
                    ManagedNotifications::default(),
                    &relationship,
                    notice.source_sequence.get(),
                ) {
                    Ok(work) => {
                        let result = self.notices.restore_notice(&work, notice);
                        let _ = self.notices.stop_work(&work);
                        let _ = self.notices.release_work(&work);
                        if matches!(result, Err(NoticeError::Capacity)) {
                            return false;
                        }
                    }
                    // A live tracker can own a confirmed original whose
                    // publication was deferred under inbox pressure. Restore
                    // that exact envelope without changing its timer/cursor.
                    Err(NoticeError::Busy) => {
                        if matches!(
                            self.notices.restore_durable_original(notice),
                            Err(NoticeError::Capacity)
                        ) {
                            return false;
                        }
                    }
                    Err(_) => return false,
                }
            }
            self.replay.pending.take();
            return true;
        }
        false
    }
    pub(super) fn finish_replay(&mut self, mut outcome: Outcome) {
        if let Some(snapshot) = outcome.snapshot
            && let Some(child) = self
                .children
                .iter_mut()
                .find(|child| child.snapshot.head.id == snapshot.head.id)
        {
            child.snapshot = snapshot;
        }
        if outcome.error {
            // Failed read-only validation owns no mutation custody. Park its
            // retry separately so it cannot hide a queued cancel or shutdown.
            let gate = self.retry.clone();
            outcome.replay.retry = Some(Box::pin(async move {
                gate.blocked(ManagerBlock::ReadValidation).await;
            }));
        }
        self.replay = outcome.replay;
        if !outcome.error && !self.closing {
            // Publish before releasing this serialized admission. If capacity
            // delays visibility, any intervening write requires a fresh read.
            self.publish_replay();
        }
    }
}
fn advance(
    journal: ManagedJournal,
    gate: Arc<durability::RetryGate>,
    mut replay: Replay,
    targets: Vec<Weak<ParentNoticeContext>>,
) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let mut snapshot = None;
        let result = step(&journal, &gate, &mut replay, &targets, &mut snapshot).await;
        let stale = matches!(result, Err(JournalError::Conflict))
            && (replay.source.is_some() || replay.validation.is_some() || replay.pending.is_some());
        if stale {
            // The exact source changed, not the catalog frontier. Visit other
            // sources before revisiting this one in the next sweep; ordinary
            // cursor staleness owns no uncertain mutation or manual retry fence.
            replay.source = None;
            replay.validation = None;
            replay.pending = None;
            replay.rescan = true;
        }
        Outcome {
            replay,
            snapshot,
            error: result.is_err() && !stale,
        }
    })
}
async fn step(
    journal: &ManagedJournal,
    gate: &Arc<durability::RetryGate>,
    replay: &mut Replay,
    targets: &[Weak<ParentNoticeContext>],
    repaired: &mut Option<JournalSnapshot>,
) -> Result<(), JournalError> {
    if let Some(pending) = &mut replay.pending {
        // One bounded exact-head read suffices: all original/ACK/lineage
        // evidence was validated against this same immutable snapshot.
        journal.history(pending.snapshot.clone(), None, 1).await?;
        pending.fresh = true;
        return Ok(());
    }
    if replay.validation.is_some() {
        return validate_page(journal, replay, targets).await;
    }
    if replay.source.is_none() {
        if replay.catalog_done {
            if replay.rescan {
                *replay = Replay::default();
            } else {
                replay.done = true;
            }
            return Ok(());
        }
        let page = journal.catalog(replay.catalog.clone(), 1).await?;
        if let Some(entry) = page.entries.first() {
            let mut snapshot = journal.inspect(entry.id.clone()).await?;
            if snapshot.recovery_required() && snapshot.head.recovery_changes_work() {
                snapshot = durability::mutate(
                    journal.clone(),
                    gate.clone(),
                    snapshot,
                    JournalMutation::Recover,
                )
                .await
                .map_err(|_| JournalError::Persistence)?;
                *repaired = Some(snapshot.clone());
            }
            if snapshot.head.status != ManagedAgentState::Archived {
                replay.source = Some((snapshot, None));
            }
        }
        replay.catalog_done = page.next.is_none();
        replay.catalog = page.next;
        return Ok(());
    }
    let (snapshot, cursor) = replay.source.as_ref().unwrap();
    let page = journal.history(snapshot.clone(), cursor.clone(), 1).await?;
    if let Some(JournalRecord::Notice(original)) = page.records.first()
        && original.source.source.generation.get() == snapshot.head.generation
        && targets
            .iter()
            .filter_map(Weak::upgrade)
            .any(|context| context.principal() == &original.target.parent && !context.is_retired())
    {
        replay.validation = Some(Validation {
            snapshot: snapshot.clone(),
            original: original.clone(),
            cursor: None,
            original_seen: false,
            parent_checked: false,
            parent: None,
        });
    }
    if let Some(next) = page.next {
        replay.source.as_mut().unwrap().1 = Some(next);
    } else {
        replay.source = None;
    }
    Ok(())
}
async fn validate_page(
    journal: &ManagedJournal,
    replay: &mut Replay,
    targets: &[Weak<ParentNoticeContext>],
) -> Result<(), JournalError> {
    let validation = replay.validation.as_mut().unwrap();
    let page = journal
        .history(validation.snapshot.clone(), validation.cursor.clone(), 100)
        .await?;
    let original = &validation.original;
    let mut acknowledged = false;
    for record in page.records {
        match record {
            JournalRecord::NoticeAcknowledged {
                identity, target, ..
            } if identity == original.identity() && target == original.target => {
                acknowledged = true;
                break;
            }
            JournalRecord::Notice(notice) if notice == *original => {
                validation.original_seen = true;
            }
            JournalRecord::Control(control)
                if control.revision == original.target.relationship_generation.get() =>
            {
                validation.parent_checked = true;
                validation.parent = (control.parent_generation
                    == Some(original.target.parent.generation.get())
                    && control.parent_id.as_deref() == Some(original.target.parent.id.as_str()))
                .then_some(control.parent_owner)
                .flatten();
            }
            _ => {}
        }
    }
    if acknowledged {
        replay.validation = None;
    } else if (validation.original_seen && validation.parent_checked) || page.next.is_none() {
        if !validation.original_seen {
            return Err(JournalError::Invalid);
        }
        let validation = replay.validation.take().unwrap();
        if let Some(transcript) = validation.parent
            && transcript.incarnation == validation.original.target.parent_incarnation
            && targets.iter().filter_map(Weak::upgrade).any(|context| {
                !context.is_retired()
                    && context.principal() == &validation.original.target.parent
                    && context.matches_transcript(&transcript)
            })
        {
            replay.pending = Some(Pending {
                snapshot: validation.snapshot,
                original: validation.original,
                transcript,
                fresh: true,
            });
        }
    } else {
        validation.cursor = page.next;
    }
    Ok(())
}
