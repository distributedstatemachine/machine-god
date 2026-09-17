//! Bounded, inert replay of confirmed originals, never status-derived events.
use super::super::{
    notices::{ManagedNotice, NoticeError, NoticeRelationship},
    prompt_context::ParentNoticeContext,
    store::{JournalCatalogCursor, JournalHistoryCursor, JournalTranscript},
};
use super::{
    Active, BoxFuture, JournalMutation, JournalRecord, JournalSnapshot, ManagedAgentState,
    ManagedJournal, ManagedManager, ManagerBlock, durability,
};
use machine_god_core::ManagedNotifications;
use std::sync::{Arc, Weak};

#[derive(Default)]
pub(super) struct Replay {
    catalog: Option<JournalCatalogCursor>,
    source: Option<(JournalSnapshot, Option<JournalHistoryCursor>)>,
    catalog_done: bool,
    pending: Option<(ManagedNotice, JournalTranscript)>,
    pub(super) done: bool,
}
pub(super) struct Outcome {
    replay: Replay,
    snapshot: Option<JournalSnapshot>,
    error: bool,
}
impl ManagedManager {
    pub(super) fn begin_replay(&mut self) -> bool {
        if self.closing || self.active.is_some() {
            return false;
        }
        if self.replay_reset {
            self.replay = Replay::default();
            self.replay_reset = false;
        }
        if let Some((notice, transcript)) = self.replay.pending.as_ref() {
            let eligible = self
                .parents
                .iter()
                .filter(|parent| parent.active)
                .filter_map(|parent| parent.context.upgrade())
                .any(|context| {
                    context.matches_transcript(transcript)
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
                    // A live tracker already owns its actual emissions.
                    Err(NoticeError::Busy) => {}
                    Err(_) => return false,
                }
            }
            self.replay.pending.take();
            return true;
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
            false,
        )));
        true
    }
    pub(super) fn finish_replay(&mut self, outcome: Outcome) {
        if let Some(snapshot) = outcome.snapshot
            && let Some(child) = self
                .children
                .iter_mut()
                .find(|child| child.snapshot.head.id == snapshot.head.id)
        {
            child.snapshot = snapshot;
        }
        if outcome.error {
            let targets = self
                .parents
                .iter()
                .filter(|parent| parent.active)
                .filter_map(|parent| parent.context.upgrade())
                .filter(|context| !context.is_retired())
                .map(|context| Arc::downgrade(&context))
                .collect();
            self.active = Some(Active::Replay(advance(
                self.journal.clone(),
                self.retry.clone(),
                outcome.replay,
                targets,
                true,
            )));
        } else {
            self.replay = outcome.replay;
        }
    }
}
fn advance(
    journal: ManagedJournal,
    gate: Arc<durability::RetryGate>,
    mut replay: Replay,
    targets: Vec<Weak<ParentNoticeContext>>,
    retry: bool,
) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        if retry {
            gate.blocked(ManagerBlock::Journal).await;
        }
        let mut snapshot = None;
        let result = step(&journal, &gate, &mut replay, &targets, &mut snapshot).await;
        Outcome {
            replay,
            snapshot,
            error: result.is_err(),
        }
    })
}
async fn step(
    journal: &ManagedJournal,
    gate: &Arc<durability::RetryGate>,
    replay: &mut Replay,
    targets: &[Weak<ParentNoticeContext>],
    repaired: &mut Option<JournalSnapshot>,
) -> Result<(), ()> {
    if replay.source.is_none() {
        if replay.catalog_done {
            replay.done = true;
            return Ok(());
        }
        let page = journal
            .catalog(replay.catalog.clone(), 1)
            .await
            .map_err(|_| ())?;
        if let Some(entry) = page.entries.first() {
            let mut snapshot = journal.inspect(entry.id.clone()).await.map_err(|_| ())?;
            if snapshot.recovery_required() {
                snapshot = durability::mutate(
                    journal.clone(),
                    gate.clone(),
                    snapshot,
                    JournalMutation::Recover,
                )
                .await
                .map_err(|_| ())?;
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
    let page = journal
        .history(snapshot.clone(), cursor.clone(), 1)
        .await
        .map_err(|_| ())?;
    if let Some(JournalRecord::Notice(original)) = page.records.first()
        && original.source.source.generation.get() == snapshot.head.generation
        && targets
            .iter()
            .filter_map(Weak::upgrade)
            .any(|context| context.principal() == &original.target.parent && !context.is_retired())
        && !acknowledged(journal, snapshot, original).await?
        && let Some(transcript) = historical_parent(
            journal,
            snapshot,
            original.target.relationship_generation.get(),
            &original.target.parent,
        )
        .await?
        && transcript.incarnation == original.target.parent_incarnation
        && targets.iter().filter_map(Weak::upgrade).any(|context| {
            context.principal() == &original.target.parent
                && context.matches_transcript(&transcript)
        })
    {
        replay.pending = Some((original.clone(), transcript));
    }
    if let Some(next) = page.next {
        replay.source.as_mut().unwrap().1 = Some(next);
    } else {
        replay.source = None;
    }
    Ok(())
}
async fn historical_parent(
    journal: &ManagedJournal,
    snapshot: &JournalSnapshot,
    revision: u64,
    parent: &super::super::notices::NoticePrincipal,
) -> Result<Option<JournalTranscript>, ()> {
    let mut cursor = None;
    loop {
        let page = journal
            .history(snapshot.clone(), cursor, 100)
            .await
            .map_err(|_| ())?;
        for record in page.records {
            if let JournalRecord::Control(control) = record
                && control.revision == revision
            {
                return Ok((control.parent_generation == Some(parent.generation.get())
                    && control.parent_id.as_deref() == Some(parent.id.as_str()))
                .then_some(control.parent_owner)
                .flatten());
            }
        }
        let Some(next) = page.next else {
            return Ok(None);
        };
        cursor = Some(next);
    }
}
async fn acknowledged(
    journal: &ManagedJournal,
    snapshot: &JournalSnapshot,
    original: &ManagedNotice,
) -> Result<bool, ()> {
    let mut cursor = None;
    loop {
        let page = journal
            .history(snapshot.clone(), cursor, 100)
            .await
            .map_err(|_| ())?;
        for record in page.records {
            match record {
                JournalRecord::NoticeAcknowledged {
                    identity, target, ..
                } if identity == original.identity() && target == original.target => {
                    return Ok(true);
                }
                JournalRecord::Notice(notice) if notice == *original => return Ok(false),
                _ => {}
            }
        }
        let Some(next) = page.next else {
            return Err(());
        };
        cursor = Some(next);
    }
}
