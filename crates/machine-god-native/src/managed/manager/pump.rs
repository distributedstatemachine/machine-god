//! Fair bounded outer polling. No dependency on terminal frames or stdout.
mod admission;
mod child;
mod pressure;
use super::super::store::{JournalError, JournalIntent};
use super::{
    Active, Arc, Child, ChildWrite, Context, JournalMutation, JournalRecord, JournalWork,
    ManagedAgentState, ManagedManager, ManagedQueueStatus, ManagedRuntimeError, ManagerBlock,
    ManagerProgress, Poll, Retiring, VecDeque, WriteAfter, command, durability, projection,
    waiting,
};
use futures_core::Stream;
use machine_god_core::{
    ManagedFailureCode, ManagedHistoryItem, ManagedHistoryKind, ManagedMessage, ManagedOutcome,
    ManagedPermissionMode, ManagedSubagentCommand, ManagedToolActivity, ManagedToolPhase,
    ModelEvent, TurnEvent,
};
use std::pin::Pin;

impl ManagedManager {
    pub(super) fn pump(
        &mut self,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Poll<Result<ManagerProgress, ManagedRuntimeError>> {
        self.catalog.register(cx);
        let mut changed = false;
        for _ in 0..self.limits.work_per_poll {
            self.capture_mailbox(cx)?;
            let mut progress = self.observe_journal_pressure();
            progress |= self.poll_active(cx, now_ms)?;
            progress |= self.observe_journal_pressure();
            progress |= self.poll_foregrounds(cx)?;
            progress |= self.poll_retiring(cx)?;
            progress |= self.poll_foreground_reservations(cx);
            progress |= self.pump_waiters(cx);
            progress |= self.poll_delivery_clears(cx);
            progress |= self.poll_saved_lifetimes(cx, now_ms);
            let len = self.children.len();
            for offset in 0..len {
                let index = (self.round_robin + offset) % len;
                progress |= self.poll_child(index, cx, now_ms)?;
            }
            if len > 0 {
                self.round_robin = (self.round_robin + 1) % len;
            }
            progress |= self.pump_notices(cx)?;
            if self.active.is_none() {
                progress |= self.admit_next(cx, now_ms);
            }
            changed |= progress;
            if !progress {
                return if changed {
                    Poll::Ready(Ok(self.progress()))
                } else {
                    Poll::Pending
                };
            }
        }
        cx.waker().wake_by_ref();
        Poll::Ready(Ok(self.progress()))
    }
    #[allow(clippy::too_many_lines)] // Each outcome retains or transfers exact original custody.
    fn poll_active(
        &mut self,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Result<bool, ManagedRuntimeError> {
        let Some(mut active) = self.active.take() else {
            return Ok(false);
        };
        match &mut active {
            Active::Observation(future) => {
                if future.as_mut().poll(cx).is_pending() {
                    self.active = Some(active);
                    return Ok(false);
                }
            }
            Active::Catalog { future, .. } => {
                let Poll::Ready(result) = future.as_mut().poll(cx) else {
                    self.active = Some(active);
                    return Ok(false);
                };
                let Active::Catalog { request, .. } = active else {
                    unreachable!()
                };
                self.catalog.finish(request, result);
            }
            Active::Replay(future) => {
                let Poll::Ready(outcome) = future.as_mut().poll(cx) else {
                    self.active = Some(active);
                    return Ok(false);
                };
                self.finish_replay(outcome);
            }
            Active::Delivery(future) => {
                let Poll::Ready(outcome) = future.as_mut().poll(cx) else {
                    self.active = Some(active);
                    return Ok(false);
                };
                self.finish_delivery(outcome)?;
            }
            Active::Command {
                future, operation, ..
            } => match future.as_mut().poll(cx) {
                Poll::Pending => {
                    self.active = Some(active);
                    return Ok(false);
                }
                Poll::Ready(outcome) => self.apply_outcome(outcome, operation.clone(), now_ms),
            },
            Active::Child { id, future, .. } => {
                let Poll::Ready(result) = future.as_mut().poll(cx) else {
                    self.active = Some(active);
                    return Ok(false);
                };
                let index = self
                    .children
                    .iter()
                    .position(|child| child.snapshot.head.id == *id)
                    .ok_or(ManagedRuntimeError::Invalid)?;
                let Active::Child {
                    after, mutation, ..
                } = active
                else {
                    unreachable!()
                };
                if let Ok(snapshot) = result {
                    // History cursors are tied to the exact head snapshot.
                    // A confirmed manager write invalidates that read-only
                    // frontier, not an ambiguous publication requiring retry.
                    self.replay_reset = true;
                    let child = &mut self.children[index];
                    child.snapshot = snapshot;
                    match after {
                        WriteAfter::Observe => {}
                        WriteAfter::Start(work) => {
                            let work = *work;
                            child.notice_attempt =
                                std::num::NonZeroU64::new(child.snapshot.head.revision);
                            if let Some(notice) = child.notice.take() {
                                let _ = self.notices.stop_work(&notice);
                                self.retained_notices.push(notice);
                            }
                            child.pressure_interrupted |=
                                !self.journal.ordinary_publication_available();
                            if self.closing || child.pressure_interrupted {
                                child.work = Some(work);
                                Self::finish_child(
                                    child,
                                    ManagedQueueStatus::Interrupted,
                                    self.closing,
                                );
                            } else if Self::start_child(child, work.clone(), now_ms).is_err() {
                                child.work = Some(work);
                                Self::finish_child(child, ManagedQueueStatus::Failed, false);
                            }
                        }
                        WriteAfter::Terminal => {
                            if child.notice_terminal == Some(ManagedQueueStatus::Cancelled) {
                                // Cancellation and its original notice were published
                                // atomically. Retire this live attempt's emitter so
                                // replay, not a second terminal emission, exposes it.
                                if let Some(notice) = child.notice.take() {
                                    let _ = self.notices.stop_work(&notice);
                                    self.retained_notices.push(notice);
                                }
                                child.notice_terminal = None;
                                child.notice_started = false;
                            }
                            child.work.take();
                            child.notice_attempt = None;
                            child.assistant.clear();
                            child.tools.clear();
                        }
                        WriteAfter::Archived => child.closing = true,
                        WriteAfter::Notice { stage, reply } => {
                            if let Some(stage) = stage {
                                self.notices
                                    .confirm_durable(&stage)
                                    .map_err(|_| ManagedRuntimeError::Invalid)?;
                            }
                            if let Some((job, operation)) = reply {
                                job.complete(Ok(command::receipt(
                                    &operation,
                                    &child.snapshot,
                                    ManagedOutcome::MilestoneEmitted,
                                )));
                            }
                        }
                    }
                } else {
                    let child = &self.children[index];
                    let journal = self.journal.clone();
                    let gate = self.retry.clone();
                    let snapshot = child.snapshot.clone();
                    let retry_mutation = mutation.clone();
                    self.active = Some(Active::Child {
                        id: child.snapshot.head.id.clone(),
                        mutation,
                        after,
                        future: Box::pin(async move {
                            gate.blocked(ManagerBlock::Journal).await;
                            durability::mutate(journal, gate, snapshot, retry_mutation).await
                        }),
                    });
                }
            }
            Active::Load { id, future } => {
                let Poll::Ready(result) = future.as_mut().poll(cx) else {
                    self.active = Some(active);
                    return Ok(false);
                };
                let child = self
                    .children
                    .iter_mut()
                    .find(|child| child.snapshot.head.id == *id)
                    .ok_or(ManagedRuntimeError::Invalid)?;
                child.pressure_interrupted |= !self.journal.ordinary_publication_available();
                match result {
                    Ok(_) if child.pressure_interrupted && child.snapshot.head.intent.is_none() => {
                        child.pending.push_back(ChildWrite {
                            mutation: JournalMutation::InterruptForPressure,
                            after: WriteAfter::Observe,
                        });
                    }
                    Ok(work) => child.pending.push_back(ChildWrite {
                        mutation: JournalMutation::HeadState {
                            work_id: work.id.clone(),
                            status: ManagedQueueStatus::Running,
                            failure: None,
                        },
                        after: WriteAfter::Start(Box::new(work)),
                    }),
                    Err(JournalError::Busy | JournalError::Limit) => {
                        return Err(ManagedRuntimeError::Capacity);
                    }
                    Err(_) => return Err(ManagedRuntimeError::Persistence),
                }
            }
        }
        Ok(true)
    }
    #[allow(clippy::too_many_lines)] // One result atomically updates resident and response ownership.
    pub(in crate::managed::manager) fn apply_outcome(
        &mut self,
        mut outcome: command::Outcome,
        operation: String,
        _now_ms: i64,
    ) {
        self.replay_reset |= outcome.replay_changed;
        if let command::Action::SavedLifetime {
            reopen,
            preparation,
        } = outcome.action
        {
            self.saved_lifetimes
                .push(super::saved_lifetime::Pending::new(
                    outcome.job,
                    outcome
                        .snapshot
                        .take()
                        .expect("saved lifecycle owns its exact head"),
                    operation,
                    reopen,
                    preparation,
                ));
            return;
        }
        if let Some(context) = outcome
            .prepared
            .as_ref()
            .and_then(|prepared| prepared.notice_context.as_ref())
        {
            // Resident/context bounds are both 64; dead weak entries are reclaimed.
            let _ = self.register_parent_context(context);
        }
        let approval_snapshot = matches!(outcome.action, command::Action::Approval { .. })
            .then(|| outcome.snapshot.as_ref().unwrap().clone());
        let mut resident = None;
        if let Some(snapshot) = outcome.snapshot.take() {
            resident = self
                .children
                .iter()
                .position(|child| child.snapshot.head.id == snapshot.head.id);
            if let Some(index) = resident {
                self.children[index].snapshot = snapshot;
                if let Some(prepared) = outcome.prepared.take() {
                    let old = std::mem::replace(&mut self.children[index].prepared, prepared);
                    self.retiring.push(Retiring {
                        prepared: old,
                        settlement: None,
                    });
                    self.children[index].selection = Arc::new(projection::SelectionIdentity);
                }
            } else if let Some(prepared) = outcome.prepared.take() {
                resident = Some(self.children.len());
                self.children.push(Child {
                    snapshot,
                    prepared,
                    selection: Arc::new(projection::SelectionIdentity),
                    starting: None,
                    turn: None,
                    work: None,
                    settlement: None,
                    actual_settled: true,
                    admission_pending: false,
                    pending: VecDeque::new(),
                    assistant: String::new(),
                    assistant_truncated: false,
                    tools: Vec::new(),
                    notice: None,
                    closing: false,
                    control: None,
                    control_operation: None,
                    control_requested: false,
                    pressure_interrupted: false,
                    notice_started: false,
                    notice_terminal: None,
                    notice_attempt: None,
                });
            }
        }
        if let Some(mut prepared) = outcome.prepared.take() {
            prepared.resources.begin_close();
            self.retiring.push(Retiring {
                prepared,
                settlement: None,
            });
        }
        match outcome.action {
            command::Action::SavedLifetime { .. } => {
                unreachable!("saved custody transferred above")
            }
            command::Action::Reply(result) => {
                if let Some(index) = resident {
                    self.refresh_relationship(index);
                }
                if let Some(index) = resident
                    && self.children[index].snapshot.head.status == ManagedAgentState::Archived
                {
                    self.children[index].closing = true;
                }
                outcome.job.complete(Ok(result));
            }
            command::Action::Wait(query) => self.add_waiter(outcome.job, query, operation),
            command::Action::Approval { proposal, mutation } => {
                if self.approvals.len() >= self.limits.waiters {
                    outcome.job.complete(Ok(command::rejected(
                        &operation,
                        ManagedFailureCode::ResourceLimit,
                    )));
                } else {
                    let future = self
                        .authorizer
                        .authorize(proposal, outcome.job.cancellation().clone());
                    self.approvals.push(waiting::Approval {
                        job: Some(outcome.job),
                        snapshot: approval_snapshot.unwrap(),
                        mutation,
                        operation,
                        future,
                        approved: false,
                    });
                }
            }
            command::Action::Cancel | command::Action::Archive => {
                if let Some(index) = resident {
                    let child = &mut self.children[index];
                    let _ = child.prepared.runtime.request_active_cancel();
                    let _ = child.prepared.runtime.clear_queued();
                    child.control_requested = true;
                    if Arc::ptr_eq(
                        child.prepared.owner.principal(),
                        outcome.job.lease().principal(),
                    ) {
                        let result = command::receipt(
                            &operation,
                            &child.snapshot,
                            ManagedOutcome::LifecycleChanged,
                        );
                        outcome.job.complete(Ok(result));
                    } else {
                        child.control = Some(outcome.job);
                        child.control_operation = Some(operation);
                    }
                } else {
                    outcome.job.complete(Ok(command::rejected(
                        &operation,
                        ManagedFailureCode::ChildUnavailable,
                    )));
                }
            }
        }
    }
    fn poll_retiring(&mut self, cx: &mut Context<'_>) -> Result<bool, ManagedRuntimeError> {
        let mut index = 0;
        let mut progress = false;
        while index < self.retiring.len() {
            let retired = &mut self.retiring[index];
            retired.prepared.resources.begin_close();
            if let Some((run, _)) = &retired.settlement
                && let Poll::Ready(result) = retired.prepared.resources.poll_turn_settled(cx, run)
            {
                result?;
                retired
                    .settlement
                    .take()
                    .unwrap()
                    .1
                    .complete()
                    .map_err(|_| ManagedRuntimeError::Invalid)?;
            }
            if retired.settlement.is_none()
                && let Poll::Ready(result) = retired.prepared.resources.poll_closed(cx)
            {
                result?;
                // Failed admission can retire a restored candidate with an
                // original saved delivery. Peer closure is not its source ACK
                // or metadata-clear receipt; keep the exact owner available to
                // the delivery lane until both kinds of custody settle.
                if retired.prepared.runtime.notice_cleanup_pending() {
                    index += 1;
                    continue;
                }
                self.retiring.remove(index);
                self.retry.retry_capacity();
                progress = true;
                continue;
            }
            index += 1;
        }
        Ok(progress)
    }
}
