use super::{
    Active, Context, ManagedManager, ManagedMessage, ManagedQueueStatus, ManagedRuntimeError,
    ManagedSubagentCommand, Poll, Retiring, command, durability,
};

impl ManagedManager {
    pub(super) fn admit_next(&mut self, cx: &mut Context<'_>, now_ms: i64) -> bool {
        const LANES: usize = 8;
        // At most one catalog read before giving ordinary durable work its next
        // turn. A retained page is bounded and cannot delay child execution.
        if !self.catalog.yield_to_work() && (self.begin_observation() || self.begin_catalog()) {
            return true;
        }
        if let Some(index) = self.children.iter().position(|child| {
            child.closing
                && !child.busy()
                && child.actual_settled
                && !child.prepared.runtime.notice_cleanup_pending()
        }) {
            let mut child = self.children.remove(index);
            self.retire_child_notice(&mut child);
            let completion = child.control.take().map(|job| {
                let receipt = command::receipt(
                    child.control_operation.as_deref().unwrap(),
                    &child.snapshot,
                    super::ManagedOutcome::LifecycleChanged,
                );
                (job, receipt)
            });
            self.retiring.push(Retiring {
                prepared: child.prepared,
                settlement: child.settlement,
                completion,
            });
            return true;
        }
        // Candidate reservations precede new allocations, not accepted work.
        if !self.closing && self.waiting_foreground() && self.evict_idle_child(None) {
            return true;
        }
        // Rotate on actual admission, not polling frequency. Each ready lane
        // receives a turn within eight admissions (plus the shared read allowance).
        // A parked mailbox head cannot hide another lane's accepted custody.
        for offset in 0..LANES {
            let lane = (self.next_admission + offset) % LANES;
            let admitted = match lane {
                0 => self.admit_child_write(),
                1 => self.admit_approval(now_ms),
                2 => self.admit_ready_wait(now_ms),
                3 => self.admit_mailbox(now_ms),
                4 => self.admit_child_start(),
                5 => self.begin_delivery(cx),
                6 => self.begin_replay(cx),
                7 => self.begin_saved_reopen(now_ms),
                _ => unreachable!("bounded admission lane"),
            };
            if admitted {
                self.next_admission = (lane + 1) % LANES;
                return true;
            }
        }
        self.begin_observation() || self.begin_catalog()
    }

    fn admit_child_write(&mut self) -> bool {
        for offset in 0..self.children.len() {
            let index = (self.next_write + offset) % self.children.len();
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
                self.next_write = index + 1;
                return true;
            }
        }
        false
    }

    fn admit_approval(&mut self, now_ms: i64) -> bool {
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
            return true;
        }
        false
    }

    fn admit_ready_wait(&mut self, now_ms: i64) -> bool {
        if self
            .ready_jobs
            .front()
            .is_some_and(|(job, _, _)| self.target_has_pending_writes(job_target(job)))
        {
            return false;
        }
        let Some((job, timeout, operation)) = self.ready_jobs.pop_front() else {
            return false;
        };
        let environment = self.environment(now_ms, operation.clone(), Some(timeout));
        self.active = Some(Active::Command {
            target: target_id(job.command()).map(str::to_owned),
            operation,
            future: command::execute(job, environment),
        });
        true
    }

    fn admit_mailbox(&mut self, now_ms: i64) -> bool {
        // Freeze only the captured target; finish its already-buffered writes
        // before loading the command's exact head. Unrelated children keep moving.
        if self
            .pending_job
            .as_ref()
            .is_some_and(|(job, _, _)| self.target_has_pending_writes(job_target(job)))
        {
            return false;
        }
        if let Some((job, wait_finished, operation)) = self.pending_job.take() {
            if let ManagedSubagentCommand::Message(ManagedMessage::Milestone(request)) =
                job.command()
            {
                let name = request.name.clone();
                self.milestone(job, &name, operation);
                return true;
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
                let foreground_waiting = self.waiting_foreground();
                let evicted = !foreground_waiting && self.evict_idle_child(target.as_deref());
                if !foreground_waiting && !self.retiring.is_empty() {
                    self.pending_job = Some((job, wait_finished, operation));
                    if evicted {
                        return true;
                    }
                    // Retain the FIFO head, but let other lanes make progress.
                } else {
                    // This request has no durable acceptance. Holding it here
                    // would hide the cancel/close that could free its capacity.
                    job.complete(Ok(command::rejected(
                        &operation,
                        machine_god_core::ManagedFailureCode::ResourceLimit,
                    )));
                    return true;
                }
            } else {
                let environment = self.environment(now_ms, operation.clone(), wait_finished);
                self.active = Some(Active::Command {
                    future: command::execute(job, environment),
                    target,
                    operation,
                });
                return true;
            }
        }
        false
    }

    fn admit_child_start(&mut self) -> bool {
        if !self.closing && self.journal.ordinary_publication_available() {
            for offset in 0..self.children.len() {
                let index = (self.next_start + offset) % self.children.len();
                let child = &self.children[index];
                if !child.busy()
                    && child.actual_settled
                    && child.snapshot.head.intent.is_none()
                    && !child.closing
                    && !self.command_target_pending(&child.snapshot.head.id)
                    && child
                        .snapshot
                        .head
                        .queue
                        .first()
                        .is_some_and(|work| work.status == ManagedQueueStatus::Pending)
                {
                    // Only an explicitly accepted Pending head can enter here
                    // after pressure interrupted the previous FIFO attempt.
                    self.children[index].pressure_interrupted = false;
                    let child = &self.children[index];
                    let work = child.snapshot.head.queue.first().unwrap();
                    self.active = Some(Active::Load {
                        id: child.snapshot.head.id.clone(),
                        future: self.journal.read_work(work.page.clone()),
                    });
                    self.next_start = index + 1;
                    return true;
                }
            }
        }
        false
    }
    fn target_has_pending_writes(&self, target: Option<&str>) -> bool {
        target.is_some_and(|target| {
            self.children
                .iter()
                .any(|child| child.snapshot.head.id == target && !child.pending.is_empty())
        })
    }
    pub(super) fn command_target_pending(&self, id: &str) -> bool {
        self.pending_job
            .as_ref()
            .map(|(job, _, _)| job)
            .into_iter()
            .chain(self.ready_jobs.front().map(|(job, _, _)| job))
            .any(|job| job_target(job) == Some(id))
    }
    pub(in crate::managed::manager) fn has_capacity(&self) -> bool {
        let reserved = self.reserved_foregrounds();
        let resident = self.children.len()
            + self.retiring.len()
            + self.foregrounds.len()
            + self.saved_lifetimes.len()
            + reserved;
        resident < self.limits.residents
            && self
                .parents
                .iter()
                .filter(|parent| parent.context.strong_count() > 0)
                .count()
                + reserved
                + self
                    .saved_lifetimes
                    .iter()
                    .map(super::super::saved_lifetime::Pending::context_reservation)
                    .sum::<usize>()
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
            completion: None,
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
    pub(in crate::managed::manager) fn environment(
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
            retiring: self
                .retiring
                .iter()
                .map(|retired| {
                    let owner = retired.prepared.owner.principal().owner();
                    crate::managed::store::JournalTranscript {
                        session_id: owner.session_id().clone(),
                        incarnation: owner.session_incarnation_id().clone(),
                    }
                })
                .collect(),
            controls: self
                .children
                .iter()
                .filter(|child| child.control_requested)
                .map(|child| child.snapshot.head.id.clone())
                .chain(
                    self.saved_lifetimes
                        .iter()
                        .map(|pending| pending.id().to_owned()),
                )
                .collect(),
            saved_lifetimes: self
                .saved_lifetimes
                .iter()
                .map(|pending| pending.id().to_owned())
                .collect(),
            now_ms,
            operation,
            wait_finished,
        }
    }
}
pub(super) fn job_target(job: &super::super::ManagedMailboxJob) -> Option<&str> {
    if matches!(
        job.command(),
        ManagedSubagentCommand::Message(ManagedMessage::Milestone(_))
    ) {
        Some(job.lease().principal().owner().session_id().as_str())
    } else {
        target_id(job.command())
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
