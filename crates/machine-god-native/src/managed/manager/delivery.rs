//! Source-side delivery reconciliation. Parent cleanup remains its own lifecycle operation.
use super::super::{
    notices::{ManagedNotice, NoticeIdentity},
    prompt_context::{NoticeCheckpoint, NoticeDelivery, ParentNoticeContext},
};
use super::{
    Active, Arc, BoxFuture, Context, JournalMutation, JournalRecord, JournalSnapshot,
    ManagedJournal, ManagedManager, ManagedRuntimeError, ManagerBlock,
    NativeConversationRuntimeError, Poll, Weak, durability,
};

pub(super) struct Parent {
    pub context: Weak<ParentNoticeContext>,
    pub completed: Option<NoticeCheckpoint>,
    pub clear: Option<NoticeDelivery>,
    pub clearing:
        Option<BoxFuture<'static, (NoticeDelivery, Result<(), NativeConversationRuntimeError>)>>,
}
pub(super) struct Outcome {
    pub context: Weak<ParentNoticeContext>,
    pub delivery: NoticeDelivery,
    pub snapshots: Vec<JournalSnapshot>,
    pub result: Result<Vec<NoticeIdentity>, ManagedRuntimeError>,
}
impl ManagedManager {
    /// Root supplies the context bound to an actual session witness. Weak
    /// registration never owns a parent runtime or creates a prompt.
    pub(crate) fn register_parent_context(
        &mut self,
        context: &Arc<ParentNoticeContext>,
    ) -> Result<(), ManagedRuntimeError> {
        if context.is_retired() {
            return Err(ManagedRuntimeError::Unavailable);
        }
        self.parents
            .retain(|parent| parent.context.strong_count() > 0);
        let weak = Arc::downgrade(context);
        if self
            .parents
            .iter()
            .any(|parent| parent.context.ptr_eq(&weak))
        {
            return Ok(());
        }
        if self
            .parents
            .iter()
            .filter_map(|parent| parent.context.upgrade())
            .any(|parent| !parent.is_retired() && parent.principal() == context.principal())
        {
            // The notice inbox is keyed by this pair. Two live session contexts
            // must not compete for originals addressed to only one incarnation.
            return Err(ManagedRuntimeError::Invalid);
        }
        if self.parents.len() >= 64 {
            return Err(ManagedRuntimeError::Capacity);
        }
        self.parents.push(Parent {
            context: weak,
            completed: None,
            clear: None,
            clearing: None,
        });
        self.replay_reset = true;
        Ok(())
    }
    pub(super) fn begin_delivery(&mut self) -> bool {
        if self.active.is_some() {
            return false;
        }
        for parent in &mut self.parents {
            let Some(context) = parent.context.upgrade() else {
                continue;
            };
            let Some(delivery) = context.delivery() else {
                parent.completed = None;
                parent.clear.take();
                continue;
            };
            if parent.completed.as_ref() == Some(delivery.checkpoint()) {
                continue;
            }
            self.active = Some(Active::Delivery(reconcile(
                self.journal.clone(),
                self.retry.clone(),
                parent.context.clone(),
                delivery,
            )));
            return true;
        }
        false
    }
    pub(super) fn finish_delivery(&mut self, outcome: Outcome) -> Result<(), ManagedRuntimeError> {
        for snapshot in outcome.snapshots {
            if let Some(child) = self
                .children
                .iter_mut()
                .find(|child| child.snapshot.head.id == snapshot.head.id)
            {
                child.snapshot = snapshot;
            }
        }
        let identities = outcome.result?;
        self.replay_reset = true;
        self.notices
            .acknowledge_recovered(outcome.delivery.originals())
            .map_err(|_| ManagedRuntimeError::Invalid)?;
        if let Some(context) = outcome.context.upgrade() {
            if context
                .confirm_source_acknowledgements(&outcome.delivery, &identities)
                .is_err()
            {
                return Ok(());
            }
            if let Some(parent) = self
                .parents
                .iter_mut()
                .find(|parent| parent.context.ptr_eq(&outcome.context))
            {
                parent.completed = Some(outcome.delivery.checkpoint().clone());
                parent.clear = Some(outcome.delivery);
            }
        }
        // Root's foreground driver, or the resident child lifecycle driver,
        // clears this exact outbox only after the confirmed source ACK subset.
        Ok(())
    }
    pub(super) fn poll_delivery_clears(&mut self, cx: &mut Context<'_>) -> bool {
        let mut progress = false;
        for parent in &mut self.parents {
            if let Some(future) = &mut parent.clearing
                && let Poll::Ready((delivery, result)) = future.as_mut().poll(cx)
            {
                parent.clearing.take();
                match result {
                    Ok(()) => {}
                    Err(NativeConversationRuntimeError::Busy) => parent.clear = Some(delivery),
                    Err(_) => {
                        let runtime = self
                            .children
                            .iter()
                            .find(|child| {
                                child
                                    .prepared
                                    .notice_context
                                    .as_ref()
                                    .is_some_and(|context| {
                                        parent.context.ptr_eq(&Arc::downgrade(context))
                                    })
                            })
                            .map(|child| child.prepared.runtime.clone())
                            .or_else(|| {
                                super::foreground::runtime_for_notice(
                                    &self.foregrounds,
                                    &parent.context,
                                )
                            });
                        if let Some(runtime) = runtime {
                            let retry = self.retry.clone();
                            parent.clearing = Some(Box::pin(async move {
                                retry.blocked(ManagerBlock::Journal).await;
                                let result =
                                    runtime.clear_notice_delivery(&delivery).await.map(|_| ());
                                (delivery, result)
                            }));
                        }
                    }
                }
                progress = true;
            }
            if parent.clearing.is_none()
                && parent.clear.is_some()
                && let Some(runtime) = self
                    .children
                    .iter()
                    .find(|child| {
                        !child.busy()
                            && !child.closing
                            && child
                                .prepared
                                .notice_context
                                .as_ref()
                                .is_some_and(|context| {
                                    parent.context.ptr_eq(&Arc::downgrade(context))
                                })
                    })
                    .map(|child| child.prepared.runtime.clone())
                    .or_else(|| {
                        super::foreground::runtime_for_notice(&self.foregrounds, &parent.context)
                    })
            {
                let delivery = parent.clear.take().unwrap();
                parent.clearing = Some(Box::pin(async move {
                    let result = runtime.clear_notice_delivery(&delivery).await.map(|_| ());
                    (delivery, result)
                }));
                progress = true;
            }
        }
        progress
    }
}

fn reconcile(
    journal: ManagedJournal,
    gate: Arc<durability::RetryGate>,
    context: Weak<ParentNoticeContext>,
    delivery: NoticeDelivery,
) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let mut snapshots = Vec::new();
        let result = reconcile_sources(&journal, &gate, &delivery, &mut snapshots).await;
        Outcome {
            context,
            delivery,
            snapshots,
            result,
        }
    })
}
pub(super) fn retry(
    journal: ManagedJournal,
    gate: Arc<durability::RetryGate>,
    context: Weak<ParentNoticeContext>,
    delivery: NoticeDelivery,
) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        gate.blocked(ManagerBlock::Journal).await;
        reconcile(journal, gate, context, delivery).await
    })
}
pub(super) async fn reconcile_sources(
    journal: &ManagedJournal,
    gate: &Arc<durability::RetryGate>,
    delivery: &NoticeDelivery,
    snapshots: &mut Vec<JournalSnapshot>,
) -> Result<Vec<NoticeIdentity>, ManagedRuntimeError> {
    let originals = delivery.originals();
    if originals.is_empty() || originals.len() > 64 {
        return Err(ManagedRuntimeError::Invalid);
    }
    let mut processed = Vec::<String>::new();
    let mut acknowledged = Vec::new();
    for original in originals {
        let source = &original.source.source.id;
        if processed.contains(source) {
            continue;
        }
        processed.push(source.clone());
        let selected: Vec<_> = originals
            .iter()
            .filter(|notice| notice.source.source.id == *source)
            .collect();
        let mut snapshot = journal
            .inspect(source.clone())
            .await
            .map_err(|_| ManagedRuntimeError::Persistence)?;
        if snapshot.recovery_required() {
            snapshot = durability::mutate(
                journal.clone(),
                gate.clone(),
                snapshot,
                JournalMutation::Recover,
            )
            .await
            .map_err(|_| ManagedRuntimeError::Persistence)?;
        }
        let acked =
            source_acknowledgements(journal, &snapshot, &selected, delivery.checkpoint()).await?;
        let records: Vec<_> = selected
            .iter()
            .zip(acked)
            .filter(|(_, acked)| !acked)
            .map(|(notice, _)| JournalRecord::NoticeAcknowledged {
                identity: notice.identity(),
                target: notice.target.clone(),
                checkpoint: delivery.checkpoint().clone(),
            })
            .collect();
        if !records.is_empty() {
            snapshot = durability::mutate(
                journal.clone(),
                gate.clone(),
                snapshot,
                JournalMutation::AppendHistory(records),
            )
            .await
            .map_err(|_| ManagedRuntimeError::Persistence)?;
        }
        snapshots.push(snapshot);
        acknowledged.extend(
            selected
                .into_iter()
                .map(super::super::notices::ManagedNotice::identity),
        );
    }
    Ok(acknowledged)
}

/// Validate the immutable originals and their historical recipients before any ACK write.
async fn source_acknowledgements(
    journal: &ManagedJournal,
    snapshot: &JournalSnapshot,
    selected: &[&ManagedNotice],
    expected_checkpoint: &NoticeCheckpoint,
) -> Result<Vec<bool>, ManagedRuntimeError> {
    let mut found = vec![false; selected.len()];
    let mut lineage = vec![false; selected.len()];
    let mut acked = vec![false; selected.len()];
    let mut cursor = None;
    loop {
        let page = journal
            .history(snapshot.clone(), cursor, 100)
            .await
            .map_err(|_| ManagedRuntimeError::Persistence)?;
        for record in page.records {
            match record {
                JournalRecord::Control(control) => {
                    for (index, original) in selected.iter().enumerate() {
                        if control.revision == original.target.relationship_generation.get() {
                            lineage[index] = control.parent_owner.as_ref().is_some_and(|owner| {
                                owner.session_id == expected_checkpoint.session_id
                                    && owner.incarnation == expected_checkpoint.incarnation_id
                            });
                        }
                    }
                }
                JournalRecord::Notice(notice) => {
                    if let Some(index) = selected.iter().position(|original| **original == notice) {
                        found[index] = true;
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
                        acked[index] = true;
                    }
                }
                _ => {}
            }
        }
        if found.iter().all(|value| *value) && lineage.iter().all(|value| *value) {
            return Ok(acked);
        }
        let Some(next) = page.next else {
            return Err(ManagedRuntimeError::Invalid);
        };
        cursor = Some(next);
    }
}
