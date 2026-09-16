use super::{
    Active, Context, ManagedManager, ManagedMessage, ManagedQueueStatus, ManagedRuntimeError,
    ManagedSubagentCommand, Poll, Retiring, command, durability,
};

impl ManagedManager {
    #[allow(clippy::too_many_lines)] // One bounded FIFO selection policy; no head bypass.
    pub(super) fn admit_next(
        &mut self,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Result<bool, ManagedRuntimeError> {
        // At most one catalog read before giving ordinary durable work its next
        // turn. A retained page is bounded and cannot delay child execution.
        if !self.catalog.yield_to_work() && (self.begin_observation() || self.begin_catalog()) {
            return Ok(true);
        }
        if let Some(index) = self
            .children
            .iter()
            .position(|child| child.closing && !child.busy() && child.actual_settled)
        {
            let mut child = self.children.remove(index);
            self.retire_child_notice(&mut child);
            self.retiring.push(Retiring {
                prepared: child.prepared,
                settlement: child.settlement,
            });
            return Ok(true);
        }
        // Drain child custody first. Each write has a bounded payload; no UI ACK is involved.
        let control_ready = self.pending_job.as_ref().is_some_and(|(job, _, _)| {
            target_id(job.command()).is_none_or(|target| {
                self.children
                    .iter()
                    .find(|child| child.snapshot.head.id == target)
                    .is_none_or(|child| child.pending.is_empty())
            })
        });
        for offset in 0..if control_ready {
            0
        } else {
            self.children.len()
        } {
            let index = (self.round_robin + offset) % self.children.len();
            let child = &mut self.children[index];
            if let Some(write) = child.pending.pop_front() {
                self.active = Some(Active::Child {
                    id: child.snapshot.head.id.clone(),
                    future: durability::mutate(
                        self.journal.clone(),
                        self.retry.clone(),
                        child.snapshot.clone(),
                        write.mutation.clone(),
                    ),
                    mutation: write.mutation,
                    after: write.after,
                });
                return Ok(true);
            }
        }
        if let Some(index) = self.approvals.iter().position(|approval| approval.approved) {
            let mut approval = self.approvals.remove(index);
            let environment = self.environment(now_ms, approval.operation.clone(), None);
            self.active = Some(Active::Command {
                target: Some(approval.snapshot.head.id.clone()),
                operation: approval.operation,
                future: command::approved_relationship(
                    approval.job.take().unwrap(),
                    environment,
                    approval.snapshot,
                    approval.mutation,
                ),
            });
            return Ok(true);
        }
        // Candidate requests precede new resident allocations, but never block
        // accepted child work, its durable writes, or notice cleanup below.
        if !self.closing && self.waiting_foreground() && self.evict_idle_child(None) {
            return Ok(true);
        }
        let ready = if self.pending_job.is_none() {
            self.ready_jobs.pop_front()
        } else {
            None
        };
        let job = if self.pending_job.is_some() {
            self.pending_job.take()
        } else if let Some((job, timeout, operation)) = ready {
            Some((job, Some(timeout), operation))
        } else if !self.closing {
            match self.mailbox.poll_next(cx) {
                Poll::Ready(Some(job)) => {
                    let sequence = self.next_operation;
                    self.next_operation = sequence
                        .checked_add(1)
                        .ok_or(ManagedRuntimeError::Capacity)?;
                    Some((job, None, format!("operation-{sequence}")))
                }
                Poll::Ready(None) => {
                    self.request_shutdown();
                    None
                }
                Poll::Pending => None,
            }
        } else {
            None
        };
        if let Some((job, wait_finished, operation)) = job {
            self.remember_principal(job.lease().principal());
            if let ManagedSubagentCommand::Message(ManagedMessage::Milestone(request)) =
                job.command()
            {
                let name = request.name.clone();
                self.milestone(job, &name, operation);
                return Ok(true);
            }
            let target = match job.command() {
                ManagedSubagentCommand::Inspect(value) => Some(value.id.clone()),
                ManagedSubagentCommand::Message(ManagedMessage::Send(value)) => {
                    Some(value.id.clone())
                }
                ManagedSubagentCommand::Create(_)
                | ManagedSubagentCommand::Message(ManagedMessage::Milestone(_)) => None,
                ManagedSubagentCommand::Configure(value) => Some(value.id.clone()),
                ManagedSubagentCommand::Relationship(value) => Some(value.id.clone()),
                ManagedSubagentCommand::Lifecycle(value) => Some(value.id.clone()),
            };
            let needs_resident = matches!(
                job.command(),
                ManagedSubagentCommand::Create(_)
                    | ManagedSubagentCommand::Message(ManagedMessage::Send(_))
                    | ManagedSubagentCommand::Lifecycle(machine_god_core::ManagedLifecycle {
                        action: machine_god_core::ManagedLifecycleAction::Resume
                            | machine_god_core::ManagedLifecycleAction::Reopen
                            | machine_god_core::ManagedLifecycleAction::Close,
                        ..
                    })
            ) && target.as_ref().is_none_or(|target| {
                !self
                    .children
                    .iter()
                    .any(|child| &child.snapshot.head.id == target)
            });
            if needs_resident
                && (!self.has_capacity() || self.waiting_foreground())
                && job.lease().is_live()
            {
                self.pending_job = Some((job, wait_finished, operation));
                if !self.waiting_foreground() && self.evict_idle_child(target.as_deref()) {
                    return Ok(true);
                }
                // Accepted child FIFO work can continue while the original
                // preacceptance head waits for actual residency retirement.
            } else {
                let environment = self.environment(now_ms, operation.clone(), wait_finished);
                self.active = Some(Active::Command {
                    future: command::execute(job, environment),
                    target,
                    operation,
                });
                return Ok(true);
            }
        }
        if !self.closing {
            for offset in 0..self.children.len() {
                let index = (self.round_robin + offset) % self.children.len();
                let child = &self.children[index];
                if !child.busy()
                    && child.actual_settled
                    && child.snapshot.head.intent.is_none()
                    && !child.closing
                    && let Some(work) = child
                        .snapshot
                        .head
                        .queue
                        .first()
                        .filter(|work| work.status == ManagedQueueStatus::Pending)
                {
                    self.active = Some(Active::Load {
                        id: child.snapshot.head.id.clone(),
                        future: self.journal.read_work(work.page.clone()),
                    });
                    return Ok(true);
                }
            }
        }
        if self.begin_delivery() {
            return Ok(true);
        }
        if self.begin_replay() {
            return Ok(true);
        }
        Ok(self.begin_observation() || self.begin_catalog())
    }
    pub(in crate::managed::manager) fn has_capacity(&self) -> bool {
        let reserved = self.reserved_foregrounds();
        let resident =
            self.children.len() + self.retiring.len() + self.foregrounds.len() + reserved;
        resident < self.limits.residents
            && self
                .parents
                .iter()
                .filter(|parent| parent.context.strong_count() > 0)
                .count()
                + reserved
                < 64
            && (resident + 1)
                .checked_mul(self.journal.resident_reservation_bytes())
                .and_then(|bytes| bytes.checked_add(1024 * 1024))
                .is_some_and(|bytes| bytes <= self.limits.buffered_bytes)
    }
    fn evict_idle_child(&mut self, protected: Option<&str>) -> bool {
        // One unsettled retiree can release the required slot. Do not evict the
        // entire idle population while that original cleanup is still pending.
        if !self.retiring.is_empty() || self.has_capacity() {
            return false;
        }
        let Some(index) = self.children.iter().position(|child| {
            !child.busy()
                && child.actual_settled
                && !child.control_requested
                && !child
                    .snapshot
                    .head
                    .queue
                    .iter()
                    .any(|work| work.status == ManagedQueueStatus::Pending)
                && !child.prepared.runtime.notice_cleanup_pending()
                && protected != Some(child.snapshot.head.id.as_str())
        }) else {
            return false;
        };
        let mut child = self.children.remove(index);
        self.retire_child_notice(&mut child);
        child.prepared.resources.begin_close();
        self.retiring.push(Retiring {
            prepared: child.prepared,
            settlement: child.settlement,
        });
        true
    }
    pub(super) fn capture_mailbox(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Result<(), ManagedRuntimeError> {
        if self.closing || self.pending_job.is_some() {
            return Ok(());
        }
        match self.mailbox.poll_next(cx) {
            Poll::Ready(Some(job)) => {
                let sequence = self.next_operation;
                self.next_operation = sequence
                    .checked_add(1)
                    .ok_or(ManagedRuntimeError::Capacity)?;
                self.pending_job = Some((job, None, format!("operation-{sequence}")));
            }
            Poll::Ready(None) => self.request_shutdown(),
            Poll::Pending => {}
        }
        Ok(())
    }
    fn environment(
        &self,
        now_ms: i64,
        operation: String,
        wait_finished: Option<bool>,
    ) -> command::Environment {
        command::Environment {
            journal: self.journal.clone(),
            owner: self.journal_owner.clone(),
            gate: self.retry.clone(),
            cursors: self.cursors.clone(),
            factory: self.factory.clone(),
            capacity: self.has_capacity(),
            residents: self
                .children
                .iter()
                .map(|child| {
                    (
                        child.snapshot.head.id.clone(),
                        child.snapshot.head.transcript.clone(),
                        child.busy(),
                    )
                })
                .collect(),
            now_ms,
            operation,
            wait_finished,
            repaired_heads: self.repaired_heads.clone(),
            notices: self.notices.clone(),
        }
    }
}
pub(super) fn target_id(command: &ManagedSubagentCommand) -> Option<&str> {
    match command {
        ManagedSubagentCommand::Create(_)
        | ManagedSubagentCommand::Message(ManagedMessage::Milestone(_)) => None,
        ManagedSubagentCommand::Inspect(value) => Some(&value.id),
        ManagedSubagentCommand::Message(ManagedMessage::Send(value)) => Some(&value.id),
        ManagedSubagentCommand::Configure(value) => Some(&value.id),
        ManagedSubagentCommand::Relationship(value) => Some(&value.id),
        ManagedSubagentCommand::Lifecycle(value) => Some(&value.id),
    }
}
