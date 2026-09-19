//! Source-side delivery reconciliation. Parent cleanup remains its own lifecycle operation.
mod validation;
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
    pub active: bool,
    pub completed: Option<NoticeCheckpoint>,
    pub clear: Option<NoticeDelivery>,
    pub pending: Option<Pending>,
    retry: Option<BoxFuture<'static, ()>>,
    in_flight: bool,
    pub clearing:
        Option<BoxFuture<'static, (NoticeDelivery, Result<(), NativeConversationRuntimeError>)>>,
}
pub(super) struct Pending {
    delivery: NoticeDelivery,
    progress: validation::Progress,
}
pub(super) struct Outcome {
    pub context: Weak<ParentNoticeContext>,
    pub delivery: NoticeDelivery,
    pub snapshots: Vec<Repair>,
    pub result: Result<Vec<NoticeIdentity>, ManagedRuntimeError>,
    pending: Option<Pending>,
}

/// Exact publication provenance, not a refreshed observation of arbitrary state.
pub(super) struct Repair {
    pub before: crate::managed::store::JournalHead,
    pub snapshot: JournalSnapshot,
}

pub(super) struct ClearTarget {
    pub runtime: Arc<crate::NativeConversationRuntime>,
    pub drain: Option<crate::conversation_runtime::NativeNoticeDrain>,
}
impl ClearTarget {
    async fn clear(&self, delivery: &NoticeDelivery) -> Result<(), NativeConversationRuntimeError> {
        match &self.drain {
            Some(drain) => self.runtime.drain_notice_delivery(delivery, drain).await,
            None => self.runtime.clear_notice_delivery(delivery).await,
        }
        .map(|_| ())
    }
}
impl ManagedManager {
    /// Root supplies the context bound to an actual session witness. Weak
    /// registration never owns a parent runtime or creates a prompt.
    pub(crate) fn register_parent_context(
        &mut self,
        context: &Arc<ParentNoticeContext>,
    ) -> Result<(), ManagedRuntimeError> {
        self.retain_parent_context(context, true)
    }

    pub(super) fn stage_parent_context(
        &mut self,
        context: &Arc<ParentNoticeContext>,
    ) -> Result<(), ManagedRuntimeError> {
        self.retain_parent_context(context, false)
    }

    fn retain_parent_context(
        &mut self,
        context: &Arc<ParentNoticeContext>,
        active: bool,
    ) -> Result<(), ManagedRuntimeError> {
        if context.is_retired() {
            return Err(ManagedRuntimeError::Unavailable);
        }
        self.parents.retain(|parent| {
            parent.context.strong_count() > 0
                || parent.clear.is_some()
                || parent.clearing.is_some()
                || parent.pending.is_some()
                || parent.in_flight
        });
        let weak = Arc::downgrade(context);
        if let Some(parent) = self
            .parents
            .iter()
            .find(|parent| parent.context.ptr_eq(&weak))
        {
            return if parent.active == active {
                Ok(())
            } else {
                Err(ManagedRuntimeError::Invalid)
            };
        }
        if self
            .parents
            .iter()
            .filter(|parent| active || !parent.active)
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
            active,
            completed: None,
            clear: None,
            pending: None,
            retry: None,
            in_flight: false,
            clearing: None,
        });
        self.replay_reset |= active;
        Ok(())
    }

    pub(super) fn activate_parent_context(
        &mut self,
        context: &Arc<ParentNoticeContext>,
    ) -> Result<(), ManagedRuntimeError> {
        if context.is_retired() {
            return Err(ManagedRuntimeError::Unavailable);
        }
        let weak = Arc::downgrade(context);
        if self.parents.iter().any(|parent| {
            parent.active
                && !parent.context.ptr_eq(&weak)
                && parent.context.upgrade().is_some_and(|original| {
                    !original.is_retired() && original.principal() == context.principal()
                })
        }) {
            return Err(ManagedRuntimeError::Invalid);
        }
        let parent = self
            .parents
            .iter_mut()
            .find(|parent| parent.context.ptr_eq(&weak))
            .ok_or(ManagedRuntimeError::Invalid)?;
        parent.active = true;
        self.replay_reset = true;
        Ok(())
    }
    pub(super) fn begin_delivery(&mut self, cx: &mut Context<'_>) -> bool {
        if self.active.is_some() {
            return false;
        }
        for parent in &mut self.parents {
            if let Some(retry) = &mut parent.retry {
                if retry.as_mut().poll(cx).is_pending() {
                    continue;
                }
                parent.retry = None;
            }
            let pending = if let Some(pending) = parent.pending.take() {
                pending
            } else {
                let Some(delivery) = parent
                    .context
                    .upgrade()
                    .and_then(|context| context.delivery())
                else {
                    continue;
                };
                if parent.completed.as_ref() == Some(delivery.checkpoint()) {
                    continue;
                }
                Pending {
                    delivery,
                    progress: validation::Progress::default(),
                }
            };
            parent.in_flight = true;
            self.active = Some(Active::Delivery(reconcile(
                self.journal.clone(),
                self.retry.clone(),
                parent.context.clone(),
                pending,
            )));
            return true;
        }
        false
    }
    pub(super) fn finish_delivery(&mut self, outcome: Outcome) -> Result<(), ManagedRuntimeError> {
        if let Some(parent) = self
            .parents
            .iter_mut()
            .find(|parent| parent.context.ptr_eq(&outcome.context))
        {
            parent.in_flight = false;
        }
        for repair in outcome.snapshots {
            // Each source ACK changes replay evidence, even while other
            // originals in this same parent delivery still await validation.
            self.replay_reset = true;
            for pending in &mut self.saved_lifetimes {
                pending.refresh_snapshot(&repair);
            }
            let snapshot = repair.snapshot;
            if let Some(child) = self
                .children
                .iter_mut()
                .find(|child| child.snapshot.head.id == snapshot.head.id)
            {
                child.snapshot = snapshot;
            }
        }
        if let Some(pending) = outcome.pending {
            if let Some(index) = self
                .parents
                .iter()
                .position(|parent| parent.context.ptr_eq(&outcome.context))
            {
                // Retain the exact receipt even if its weak observer retires.
                // Rotate between parents so a long source cannot monopolize delivery.
                let mut parent = self.parents.remove(index);
                parent.pending = Some(pending);
                if outcome.result.is_err() {
                    let gate = self.retry.clone();
                    parent.retry = Some(Box::pin(async move {
                        gate.blocked(ManagerBlock::ReadValidation).await;
                    }));
                }
                self.parents.push(parent);
            }
            return Ok(());
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
            if let Some(delivery) = &parent.clear {
                delivery.register_clear_waker(cx.waker());
                if delivery.is_cleared() {
                    // An external owner can clear and retire before our next
                    // poll. Observe its exact confirmed receipt, not weak-owner
                    // disappearance or an unobservable/uncertain slot.
                    parent.clear.take();
                    parent.clearing.take();
                    parent.completed = None;
                    progress = true;
                    continue;
                }
            }
            if let Some(future) = &mut parent.clearing
                && let Poll::Ready((delivery, result)) = future.as_mut().poll(cx)
            {
                parent.clearing.take();
                match result {
                    Ok(()) => {
                        parent.clear.take();
                        parent.completed = None;
                    }
                    Err(NativeConversationRuntimeError::Busy) => parent.clear = Some(delivery),
                    Err(_) => {
                        let runtime = clear_runtime(
                            &self.children,
                            &self.retiring,
                            &self.saved_lifetimes,
                            &self.foregrounds,
                            &parent.context,
                            false,
                        );
                        if let Some(runtime) = runtime {
                            let retry = self.retry.clone();
                            parent.clearing = Some(Box::pin(async move {
                                retry.blocked(ManagerBlock::NoticeClear).await;
                                let result = runtime.clear(&delivery).await;
                                (delivery, result)
                            }));
                        }
                    }
                }
                progress = true;
            }
            if parent.clearing.is_none()
                && parent.clear.is_some()
                && let Some(runtime) = clear_runtime(
                    &self.children,
                    &self.retiring,
                    &self.saved_lifetimes,
                    &self.foregrounds,
                    &parent.context,
                    true,
                )
            {
                let delivery = parent.clear.as_ref().unwrap().clone();
                parent.clearing = Some(Box::pin(async move {
                    let result = runtime.clear(&delivery).await;
                    (delivery, result)
                }));
                progress = true;
            }
        }
        progress
    }
}

fn clear_runtime(
    children: &[super::Child],
    retiring: &[super::Retiring],
    saved: &[super::saved_lifetime::Pending],
    foregrounds: &[super::foreground::Foreground],
    context: &Weak<ParentNoticeContext>,
    idle_child_only: bool,
) -> Option<ClearTarget> {
    children
        .iter()
        .find(|child| {
            (!idle_child_only || (!child.busy() && !child.closing))
                && child
                    .prepared
                    .notice_context
                    .as_ref()
                    .is_some_and(|original| context.ptr_eq(&Arc::downgrade(original)))
        })
        .map(|child| ClearTarget {
            runtime: child.prepared.runtime.clone(),
            drain: None,
        })
        .or_else(|| retiring_runtime(retiring, context))
        .or_else(|| super::saved_lifetime::runtime_for_notice(saved, context))
        .or_else(|| super::foreground::runtime_for_notice(foregrounds, context))
}

fn retiring_runtime(
    retiring: &[super::Retiring],
    context: &Weak<ParentNoticeContext>,
) -> Option<ClearTarget> {
    // A rejected restored candidate can already own a recovered delivery.
    // It has no child row, but retains this exact runtime and context through
    // peer/worker closure. That closure does not retire runtime metadata admission.
    retiring
        .iter()
        .find(|retired| {
            !retired.prepared.runtime.status().active
                && retired
                    .prepared
                    .notice_context
                    .as_ref()
                    .is_some_and(|original| context.ptr_eq(&Arc::downgrade(original)))
        })
        .map(|retired| ClearTarget {
            runtime: retired.prepared.runtime.clone(),
            drain: None,
        })
}

fn reconcile(
    journal: ManagedJournal,
    gate: Arc<durability::RetryGate>,
    context: Weak<ParentNoticeContext>,
    mut pending: Pending,
) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let mut snapshots = Vec::new();
        let step = validation::step(
            &journal,
            &gate,
            &pending.delivery,
            &mut pending.progress,
            &mut snapshots,
        )
        .await;
        let result = match &step {
            Ok(true) => Ok(pending
                .delivery
                .originals()
                .iter()
                .map(ManagedNotice::identity)
                .collect()),
            Ok(false) => Ok(Vec::new()),
            Err(error) => Err(*error),
        };
        Outcome {
            context,
            delivery: pending.delivery.clone(),
            snapshots,
            result,
            pending: (!matches!(step, Ok(true))).then_some(pending),
        }
    })
}

// Test adapter for validating a complete fixed receipt. Production admission
// always calls one `validation::step` and yields to the finite manager round.
#[cfg(test)]
pub(super) async fn reconcile_sources(
    journal: &ManagedJournal,
    gate: &Arc<durability::RetryGate>,
    delivery: &NoticeDelivery,
    snapshots: &mut Vec<Repair>,
) -> Result<Vec<NoticeIdentity>, ManagedRuntimeError> {
    let mut progress = validation::Progress::default();
    while !validation::step(journal, gate, delivery, &mut progress, snapshots).await? {}
    Ok(delivery
        .originals()
        .iter()
        .map(ManagedNotice::identity)
        .collect())
}
