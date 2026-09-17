//! One source-history page per manager journal admission.
use super::{
    Arc, JournalMutation, JournalRecord, ManagedJournal, ManagedNotice, ManagedRuntimeError,
    NoticeCheckpoint, NoticeDelivery, Repair, durability,
};
use crate::managed::store::{JournalError, JournalHistoryCursor};

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Progress {
    processed: u64,
    source: Option<Source>,
}
struct Source {
    cursor: Option<JournalHistoryCursor>,
    found: Vec<bool>,
    lineage: Vec<bool>,
    acked: Vec<bool>,
}

/// Returns true only after every source has its exact confirmed ACK. A failed
/// read owns no publication and can be parked outside the serialized lane.
pub(super) async fn step(
    journal: &ManagedJournal,
    gate: &Arc<durability::RetryGate>,
    delivery: &NoticeDelivery,
    progress: &mut Progress,
    repaired: &mut Vec<Repair>,
) -> Result<bool, ManagedRuntimeError> {
    let originals = delivery.originals();
    if originals.is_empty() || originals.len() > 64 {
        return Err(ManagedRuntimeError::Invalid);
    }
    let Some((_, original)) = originals
        .iter()
        .enumerate()
        .find(|(index, _)| progress.processed & (1u64 << index) == 0)
    else {
        return Ok(true);
    };
    let id = &original.source.source.id;
    let selected: Vec<_> = originals
        .iter()
        .filter(|notice| &notice.source.source.id == id)
        .collect();
    // Retain only a digest-bound cursor between admissions, not a full head
    // allocation per registered parent. The cursor rejects a changed head.
    let mut snapshot = journal
        .inspect(id.clone())
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?;
    // Quiescent heads need no work-state recovery. Their original ACK reserve
    // funds the exact ACK publication, including its atomic owner-epoch repair,
    // rather than an unrelated generic recovery publication first.
    if snapshot.recovery_required() && snapshot.head.recovery_changes_work() {
        let before = snapshot.head.clone();
        snapshot = durability::mutate(
            journal.clone(),
            gate.clone(),
            snapshot,
            JournalMutation::Recover,
        )
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?;
        repaired.push(Repair {
            before,
            snapshot: snapshot.clone(),
        });
    }
    if progress.source.is_none() {
        progress.source = Some(Source {
            cursor: None,
            found: vec![false; selected.len()],
            lineage: vec![false; selected.len()],
            acked: vec![false; selected.len()],
        });
    }
    let source = progress.source.as_mut().unwrap();
    let page = match journal
        .history(snapshot.clone(), source.cursor.clone(), 100)
        .await
    {
        Ok(page) => page,
        Err(JournalError::Conflict) => {
            // A sibling command may have changed this source between pages.
            // Restart from its current exact head without authorizing any ACK.
            progress.source = None;
            return Ok(false);
        }
        Err(_) => return Err(ManagedRuntimeError::Persistence),
    };
    source.observe(page.records, &selected, delivery.checkpoint());
    if !source.found.iter().all(|found| *found) || !source.lineage.iter().all(|valid| *valid) {
        source.cursor = Some(page.next.ok_or(ManagedRuntimeError::Invalid)?);
        return Ok(false);
    }
    let records: Vec<_> = selected
        .iter()
        .zip(&source.acked)
        .filter(|(_, acked)| !**acked)
        .map(|(notice, _)| JournalRecord::NoticeAcknowledged {
            identity: notice.identity(),
            target: notice.target.clone(),
            checkpoint: delivery.checkpoint().clone(),
        })
        .collect();
    if !records.is_empty() {
        let before = snapshot.head.clone();
        snapshot = durability::mutate(
            journal.clone(),
            gate.clone(),
            snapshot,
            JournalMutation::AppendHistory(records),
        )
        .await
        .map_err(|_| ManagedRuntimeError::Persistence)?;
        repaired.push(Repair { before, snapshot });
    }
    for (index, notice) in originals.iter().enumerate() {
        if &notice.source.source.id == id {
            progress.processed |= 1u64 << index;
        }
    }
    progress.source = None;
    let complete = u64::MAX >> (64 - originals.len());
    Ok(progress.processed == complete)
}

impl Source {
    fn observe(
        &mut self,
        records: Vec<JournalRecord>,
        selected: &[&ManagedNotice],
        expected_checkpoint: &NoticeCheckpoint,
    ) {
        for record in records {
            match record {
                JournalRecord::Control(control) => {
                    for (index, original) in selected.iter().enumerate() {
                        if control.revision == original.target.relationship_generation.get() {
                            self.lineage[index] = control.parent_generation
                                == Some(original.target.parent.generation.get())
                                && control.parent_id.as_deref()
                                    == Some(original.target.parent.id.as_str())
                                && control.parent_owner.as_ref().is_some_and(|owner| {
                                    owner.session_id == expected_checkpoint.session_id
                                        && owner.incarnation == expected_checkpoint.incarnation_id
                                        && owner.incarnation == original.target.parent_incarnation
                                });
                        }
                    }
                }
                JournalRecord::Notice(notice) => {
                    if let Some(index) = selected.iter().position(|original| **original == notice) {
                        self.found[index] = true;
                    }
                }
                JournalRecord::NoticeAcknowledged {
                    identity,
                    target,
                    checkpoint,
                } if &checkpoint == expected_checkpoint => {
                    if let Some(index) = selected.iter().position(|original| {
                        original.identity() == identity && original.target == target
                    }) {
                        self.acked[index] = true;
                    }
                }
                _ => {}
            }
        }
    }
}
