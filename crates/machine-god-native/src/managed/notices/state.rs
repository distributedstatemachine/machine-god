use super::{
    Arc, ManagedAgentState, ManagedNotice, ManagedNotifications, NativeMcpRuntimeClock, NonZeroU64,
    NoticeAckToken, NoticeBatch, NoticeBatchEntry, NoticeEmission, NoticeError, NoticeEvent,
    NoticeHistoryRef, NoticeLimits, NoticeObservation, NoticePrincipal, NoticeRelationship,
    NoticeTarget, NoticeTerminal, NoticeUsage, PreparedNotice, StagedNotice, WorkNoticeIdentity,
    WorkNoticeRef,
};
use machine_god_core::ManagedStopCondition;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::Waker,
    time::{Duration, Instant},
};

pub(super) struct Inner {
    pub(super) clock: Arc<dyn NativeMcpRuntimeClock>,
    pub(super) state: Mutex<State>,
    limits: NoticeLimits,
    budget: Arc<Budget>,
}
#[derive(Default)]
struct Budget {
    count: AtomicUsize,
    bytes: AtomicUsize,
}
pub(super) struct NoticeRecord {
    id: u64,
    pub(super) notice: ManagedNotice,
    work: u64,
    encoded_bytes: usize,
    charge: usize,
    budget: Arc<Budget>,
}
impl Drop for NoticeRecord {
    fn drop(&mut self) {
        self.budget.bytes.fetch_sub(self.charge, Ordering::AcqRel);
        self.budget.count.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(super) struct WorkIdentity {
    id: u64,
    source: WorkNoticeIdentity,
}
struct Tracker {
    identity: Arc<WorkIdentity>,
    policy: ManagedNotifications,
    relationship: NoticeRelationship,
    sequence: u64,
    observed_sequence: u64,
    started: bool,
    terminal: Option<NoticeTerminal>,
    milestones: u32,
    ticks: u64,
    next_due: Option<Instant>,
    duration_end: Option<Instant>,
    stopped: bool,
    closed: bool,
    pending: Option<PendingNotice>,
}
struct PendingNotice {
    record: Arc<NoticeRecord>,
    transition: Cursor,
    eligible: bool,
}
#[derive(Clone, Copy)]
struct Cursor {
    sequence: u64,
    observed_sequence: u64,
    started: bool,
    terminal: Option<NoticeTerminal>,
    milestones: u32,
    ticks: u64,
    next_due: Option<Instant>,
    duration_end: Option<Instant>,
}
impl Cursor {
    fn capture(work: &Tracker) -> Self {
        Self {
            sequence: work.sequence,
            observed_sequence: work.observed_sequence,
            started: work.started,
            terminal: work.terminal,
            milestones: work.milestones,
            ticks: work.ticks,
            next_due: work.next_due,
            duration_end: work.duration_end,
        }
    }
    fn apply(self, work: &mut Tracker) {
        work.sequence = self.sequence;
        work.observed_sequence = self.observed_sequence;
        work.started = self.started;
        work.terminal = self.terminal;
        work.milestones = self.milestones;
        work.ticks = self.ticks;
        if !work.stopped && !work.closed {
            work.next_due = self.next_due;
            work.duration_end = self.duration_end;
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    next: u64,
    last_now: Option<Instant>,
    trackers: BTreeMap<u64, Tracker>,
    queue: VecDeque<Arc<NoticeRecord>>,
    pub(super) driver: Option<(u64, Option<Waker>)>,
}
impl State {
    fn next_id(&self) -> Result<u64, NoticeError> {
        self.next.checked_add(1).ok_or(NoticeError::Exhausted)
    }
    pub(super) fn now(&mut self, now: Instant) -> Result<(), NoticeError> {
        if self.last_now.is_some_and(|last| now < last) {
            return Err(NoticeError::ClockRegressed);
        }
        self.last_now = Some(now);
        Ok(())
    }
    pub(super) fn earliest(&self) -> Option<Instant> {
        self.trackers
            .values()
            .filter_map(|work| {
                if work.pending.is_some() {
                    return None;
                }
                let next = work.next_due?;
                Some(work.duration_end.map_or(next, |end| end.min(next)))
            })
            .min()
    }
    pub(super) fn claim_driver(&mut self, ticket: &mut Option<u64>) -> Result<(), NoticeError> {
        if let Some(ticket) = *ticket {
            if self
                .driver
                .as_ref()
                .is_some_and(|(actual, _)| *actual == ticket)
            {
                return Ok(());
            }
            return Err(NoticeError::Stale);
        }
        if self.driver.is_some() {
            return Err(NoticeError::Busy);
        }
        let id = self.next_id()?;
        self.next = id;
        self.driver = Some((id, None));
        *ticket = Some(id);
        Ok(())
    }
}
impl Inner {
    pub(super) fn new(limits: NoticeLimits, clock: Arc<dyn NativeMcpRuntimeClock>) -> Self {
        Self {
            clock,
            state: Mutex::new(State::default()),
            limits,
            budget: Arc::new(Budget::default()),
        }
    }
    fn mutate<R>(
        &self,
        f: impl FnOnce(&mut State) -> Result<R, NoticeError>,
    ) -> Result<R, NoticeError> {
        let (result, waker) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let result = f(&mut state);
            let waker = if result.is_ok() {
                state.driver.as_mut().and_then(|(_, waker)| waker.take())
            } else {
                None
            };
            (result, waker)
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        result
    }
    fn resolve(&self, work: &WorkNoticeRef) -> Result<Arc<WorkIdentity>, NoticeError> {
        if !std::ptr::eq(self, work.inner.as_ptr()) {
            return Err(NoticeError::Stale);
        }
        work.identity.upgrade().ok_or(NoticeError::Stale)
    }
    pub(super) fn register(
        &self,
        source: &WorkNoticeIdentity,
        mut policy: ManagedNotifications,
        relationship: &NoticeRelationship,
        sequence: u64,
    ) -> Result<Arc<WorkIdentity>, NoticeError> {
        valid_principal(&source.source)?;
        valid_id(&source.work_id)?;
        valid_relationship(relationship)?;
        policy.normalize().map_err(|_| NoticeError::InvalidInput)?;
        let now = self.clock.now();
        deadlines(&policy, now)?;
        self.mutate(|state| {
            state.now(now)?;
            if state.trackers.len() == self.limits.trackers {
                return Err(NoticeError::Capacity);
            }
            if state
                .trackers
                .values()
                .any(|work| work.identity.source == *source)
            {
                return Err(NoticeError::Busy);
            }
            let id = state.next_id()?;
            let identity = Arc::new(WorkIdentity {
                id,
                source: source.clone(),
            });
            state.trackers.insert(
                id,
                Tracker {
                    identity: identity.clone(),
                    policy: policy.clone(),
                    relationship: relationship.clone(),
                    sequence,
                    observed_sequence: sequence,
                    started: false,
                    terminal: None,
                    milestones: 0,
                    ticks: 0,
                    next_due: None,
                    duration_end: None,
                    stopped: false,
                    closed: false,
                    pending: None,
                },
            );
            state.next = id;
            Ok(identity)
        })
    }
    fn emit(
        self: &Arc<Self>,
        state: &mut State,
        id: u64,
        sequence: NonZeroU64,
        event: NoticeEvent,
        history: Option<&NoticeHistoryRef>,
        enabled: bool,
    ) -> Result<PreparedNotice, NoticeError> {
        let work = state.trackers.get(&id).ok_or(NoticeError::Stale)?;
        if !enabled {
            return Ok(PreparedNotice::Suppressed);
        }
        let Some(parent) = &work.relationship.parent else {
            return Ok(PreparedNotice::Suppressed);
        };
        let notice = ManagedNotice {
            source: work.identity.source.clone(),
            source_sequence: sequence,
            target: NoticeTarget {
                parent: parent.clone(),
                relationship_generation: work.relationship.generation,
            },
            event,
            history: history.cloned(),
        };
        let transition = Cursor::capture(work);
        let record = self.reserve_record(state, id, notice)?;
        let token = StagedNotice {
            inner: Arc::downgrade(self),
            record: Arc::clone(&record),
        };
        state.trackers.get_mut(&id).unwrap().pending = Some(PendingNotice {
            record,
            transition,
            eligible: true,
        });
        Ok(PreparedNotice::Staged(token))
    }
    fn reserve_record(
        &self,
        state: &mut State,
        work: u64,
        notice: ManagedNotice,
    ) -> Result<Arc<NoticeRecord>, NoticeError> {
        let next = state.next_id()?;
        let encoded_bytes = serde_json::to_vec(&notice)
            .map_err(|_| NoticeError::InvalidInput)?
            .len();
        let charge = encoded_bytes
            .checked_add(std::mem::size_of::<NoticeRecord>())
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<PendingNotice>()))
            .ok_or(NoticeError::Capacity)?;
        if encoded_bytes > self.limits.notice_bytes
            || self.budget.count.load(Ordering::Acquire) >= self.limits.records
            || self
                .budget
                .bytes
                .load(Ordering::Acquire)
                .checked_add(charge)
                .is_none_or(|bytes| bytes > self.limits.retained_bytes)
        {
            return Err(NoticeError::Capacity);
        }
        // Reserve queue storage for all currently staged candidates as well as
        // visible records, so confirmation cannot discover a new queue bound.
        let staged = state
            .trackers
            .values()
            .filter(|work| work.pending.is_some())
            .count();
        state
            .queue
            .try_reserve(staged + 1)
            .map_err(|_| NoticeError::Capacity)?;
        // Insertions are serialized; concurrent final snapshot drops only refund.
        self.budget.count.fetch_add(1, Ordering::AcqRel);
        self.budget.bytes.fetch_add(charge, Ordering::AcqRel);
        let record = Arc::new(NoticeRecord {
            id: next,
            notice,
            work,
            encoded_bytes,
            charge,
            budget: self.budget.clone(),
        });
        state.next = next;
        Ok(record)
    }
    pub(super) fn restore(
        &self,
        reference: &WorkNoticeRef,
        notice: &ManagedNotice,
    ) -> Result<NoticeEmission, NoticeError> {
        let identity = self.resolve(reference)?;
        valid_principal(&notice.target.parent)?;
        valid_history(notice.history.as_ref(), notice.source_sequence)?;
        match &notice.event {
            NoticeEvent::Milestone { name }
                if name.is_empty() || name.len() > 128 || name.contains('\0') =>
            {
                return Err(NoticeError::InvalidInput);
            }
            NoticeEvent::Interval {
                first_tick,
                last_tick,
                coalesced_intervals,
                gap,
                ..
            } => {
                if last_tick
                    .get()
                    .checked_sub(first_tick.get())
                    .and_then(|n| n.checked_add(1))
                    != Some(coalesced_intervals.get())
                    || *gap != (coalesced_intervals.get() > 1)
                {
                    return Err(NoticeError::InvalidInput);
                }
            }
            _ => {}
        }
        self.mutate(|state| {
            let work = state.trackers.get(&identity.id).ok_or(NoticeError::Stale)?;
            if work.closed
                || work.pending.is_some()
                || work.started
                || work.identity.source != notice.source
                || notice.source_sequence.get() > work.sequence
            {
                return Err(NoticeError::InvalidInput);
            }
            if let Some(original) = state.queue.iter().find(|record| {
                record.work == identity.id && overlaps(&record.notice.event, &notice.event)
            }) {
                return if original.notice == *notice {
                    Ok(NoticeEmission::AlreadyRecorded)
                } else {
                    Err(NoticeError::InvalidInput)
                };
            }
            let record = self.reserve_record(state, identity.id, notice.clone())?;
            state.queue.push_back(record);
            // Replay admission is explicitly inert, never a running-work resume.
            state.trackers.get_mut(&identity.id).unwrap().stopped = true;
            Ok(NoticeEmission::Queued)
        })
    }
    pub(super) fn start(
        self: &Arc<Self>,
        reference: &WorkNoticeRef,
        sequence: NonZeroU64,
        history: Option<&NoticeHistoryRef>,
    ) -> Result<PreparedNotice, NoticeError> {
        valid_history(history, sequence)?;
        let identity = self.resolve(reference)?;
        let now = self.clock.now();
        self.mutate(|state| {
            state.now(now)?;
            let work = emitting(state, identity.id)?;
            if work.started {
                return Ok(PreparedNotice::AlreadyRecorded);
            }
            if work.terminal.is_some() {
                return Err(NoticeError::Closed);
            }
            fresh_sequence(work, sequence, false)?;
            let (next_due, duration_end) = deadlines(&work.policy, now)?;
            let enabled = work.policy.started;
            let before = Cursor::capture(work);
            let outcome = self.emit(
                state,
                identity.id,
                sequence,
                NoticeEvent::Started,
                history,
                enabled,
            )?;
            let work = state.trackers.get_mut(&identity.id).unwrap();
            work.started = true;
            work.sequence = sequence.get();
            work.next_due = next_due;
            work.duration_end = duration_end;
            retain_transition(work, before);
            Ok(outcome)
        })
    }
    pub(super) fn milestone(
        self: &Arc<Self>,
        reference: &WorkNoticeRef,
        sequence: NonZeroU64,
        name: &str,
        history: Option<&NoticeHistoryRef>,
    ) -> Result<PreparedNotice, NoticeError> {
        valid_history(history, sequence)?;
        if name.is_empty() || name.len() > 128 || name.contains('\0') {
            return Err(NoticeError::InvalidInput);
        }
        let identity = self.resolve(reference)?;
        self.mutate(|state| {
            let work = emitting(state, identity.id)?;
            if !work.started || work.terminal.is_some() {
                return Err(NoticeError::Closed);
            }
            let index = work
                .policy
                .milestones
                .iter()
                .position(|declared| declared == name)
                .ok_or(NoticeError::UndeclaredMilestone)?;
            let bit = 1u32 << index;
            if work.milestones & bit != 0 {
                return Ok(PreparedNotice::AlreadyRecorded);
            }
            fresh_sequence(work, sequence, false)?;
            let before = Cursor::capture(work);
            let outcome = self.emit(
                state,
                identity.id,
                sequence,
                NoticeEvent::Milestone {
                    name: name.to_owned(),
                },
                history,
                true,
            )?;
            let work = state.trackers.get_mut(&identity.id).unwrap();
            work.milestones |= bit;
            work.sequence = sequence.get();
            retain_transition(work, before);
            Ok(outcome)
        })
    }
    pub(super) fn terminal(
        self: &Arc<Self>,
        reference: &WorkNoticeRef,
        sequence: NonZeroU64,
        outcome: NoticeTerminal,
        history: Option<&NoticeHistoryRef>,
    ) -> Result<PreparedNotice, NoticeError> {
        valid_history(history, sequence)?;
        let identity = self.resolve(reference)?;
        self.mutate(|state| {
            let work = emitting(state, identity.id)?;
            if let Some(original) = work.terminal {
                return if original == outcome {
                    Ok(PreparedNotice::AlreadyRecorded)
                } else {
                    Err(NoticeError::InvalidInput)
                };
            }
            fresh_sequence(work, sequence, false)?;
            let enabled = match outcome {
                NoticeTerminal::Completed => work.policy.terminal.completed,
                NoticeTerminal::Failed => work.policy.terminal.failed,
                NoticeTerminal::Cancelled => work.policy.terminal.cancelled,
            };
            let stop = work
                .policy
                .stop_conditions
                .contains(&ManagedStopCondition::Terminal);
            let before = Cursor::capture(work);
            let emission = self.emit(
                state,
                identity.id,
                sequence,
                NoticeEvent::Terminal { outcome },
                history,
                enabled,
            )?;
            let work = state.trackers.get_mut(&identity.id).unwrap();
            work.terminal = Some(outcome);
            work.sequence = sequence.get();
            if stop {
                work.next_due = None;
                work.duration_end = None;
            }
            retain_transition(work, before);
            Ok(emission)
        })
    }
    pub(super) fn observe(
        self: &Arc<Self>,
        observation: &NoticeObservation,
    ) -> Result<PreparedNotice, NoticeError> {
        valid_history(observation.history.as_ref(), observation.source_sequence)?;
        let identity = self.resolve(&observation.work)?;
        let now = self.clock.now();
        self.mutate(|state| {
            state.now(now)?;
            let work = emitting(state, identity.id)?;
            fresh_sequence(work, observation.source_sequence, true)?;
            if observation.source_sequence.get() < work.observed_sequence {
                return Err(NoticeError::StaleSource);
            }
            let Some(due) = work.next_due else {
                return Ok(PreparedNotice::Suppressed);
            };
            let observed_terminal = matches!(
                observation.state,
                ManagedAgentState::Completed
                    | ManagedAgentState::Failed
                    | ManagedAgentState::Cancelled
            ) && work
                .policy
                .stop_conditions
                .contains(&ManagedStopCondition::Terminal);
            if observed_terminal || work.duration_end.is_some_and(|end| now >= end) {
                let work = state.trackers.get_mut(&identity.id).unwrap();
                work.next_due = None;
                work.duration_end = None;
                work.observed_sequence = observation.source_sequence.get();
                return Ok(PreparedNotice::Suppressed);
            }
            if now < due {
                return Ok(PreparedNotice::Suppressed);
            }
            let interval = work
                .policy
                .report_interval_ms
                .expect("due requires interval");
            let elapsed = now.duration_since(due).as_millis();
            let ticks = elapsed
                .checked_div(u128::from(interval))
                .and_then(|v| v.checked_add(1))
                .and_then(|v| u64::try_from(v).ok())
                .and_then(NonZeroU64::new)
                .ok_or(NoticeError::DeadlineOverflow)?;
            let advance = interval
                .checked_mul(ticks.get())
                .ok_or(NoticeError::DeadlineOverflow)?;
            let next = due
                .checked_add(Duration::from_millis(advance))
                .ok_or(NoticeError::DeadlineOverflow)?;
            let first_tick = work
                .ticks
                .checked_add(1)
                .and_then(NonZeroU64::new)
                .ok_or(NoticeError::DeadlineOverflow)?;
            let last_tick = work
                .ticks
                .checked_add(ticks.get())
                .and_then(NonZeroU64::new)
                .ok_or(NoticeError::DeadlineOverflow)?;
            let event = NoticeEvent::Interval {
                state: observation.state,
                first_tick,
                last_tick,
                coalesced_intervals: ticks,
                gap: ticks.get() > 1,
            };
            let before = Cursor::capture(work);
            let emission = self.emit(
                state,
                identity.id,
                observation.source_sequence,
                event,
                observation.history.as_ref(),
                true,
            )?;
            let work = state.trackers.get_mut(&identity.id).unwrap();
            work.next_due = Some(next);
            work.ticks = last_tick.get();
            work.observed_sequence = observation.source_sequence.get();
            retain_transition(work, before);
            Ok(emission)
        })
    }
    pub(super) fn pending(
        self: &Arc<Self>,
        reference: &WorkNoticeRef,
    ) -> Result<Option<StagedNotice>, NoticeError> {
        let identity = self.resolve(reference)?;
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let work = state.trackers.get(&identity.id).ok_or(NoticeError::Stale)?;
        Ok(work.pending.as_ref().map(|pending| StagedNotice {
            inner: Arc::downgrade(self),
            record: Arc::clone(&pending.record),
        }))
    }
    fn validate_stage(&self, candidate: &StagedNotice, state: &State) -> Result<u64, NoticeError> {
        if !std::ptr::eq(self, candidate.inner.as_ptr()) {
            return Err(NoticeError::Stale);
        }
        let id = candidate.record.work;
        let work = state.trackers.get(&id).ok_or(NoticeError::Stale)?;
        if !work
            .pending
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(&pending.record, &candidate.record))
        {
            return Err(NoticeError::Stale);
        }
        Ok(id)
    }
    pub(super) fn confirm(&self, stage: &StagedNotice) -> Result<NoticeEmission, NoticeError> {
        self.mutate(|state| {
            let id = self.validate_stage(stage, state)?;
            let work = state.trackers.get_mut(&id).unwrap();
            let pending = work.pending.take().unwrap();
            pending.transition.apply(work);
            if pending.eligible && !work.closed && !work.stopped {
                state.queue.push_back(pending.record);
                Ok(NoticeEmission::Queued)
            } else {
                Ok(NoticeEmission::Suppressed)
            }
        })
    }
    pub(super) fn discard(&self, stage: &StagedNotice) -> Result<(), NoticeError> {
        let removed = self.mutate(|state| {
            let id = self.validate_stage(stage, state)?;
            Ok(state.trackers.get_mut(&id).unwrap().pending.take())
        })?;
        drop(removed);
        Ok(())
    }
    pub(super) fn set_relationship(
        &self,
        reference: &WorkNoticeRef,
        relationship: &NoticeRelationship,
    ) -> Result<(), NoticeError> {
        valid_relationship(relationship)?;
        let identity = self.resolve(reference)?;
        self.mutate(|state| {
            let work = live(state, identity.id)?;
            if relationship.generation <= work.relationship.generation {
                return Err(NoticeError::StaleSource);
            }
            state.trackers.get_mut(&identity.id).unwrap().relationship = relationship.clone();
            Ok(())
        })
    }
    pub(super) fn stop(&self, reference: &WorkNoticeRef, close: bool) -> Result<(), NoticeError> {
        let identity = self.resolve(reference)?;
        self.mutate(|state| {
            let work = state
                .trackers
                .get_mut(&identity.id)
                .ok_or(NoticeError::Stale)?;
            work.stopped = true;
            work.closed |= close;
            work.next_due = None;
            work.duration_end = None;
            if close {
                state.queue.retain(|record| record.work != identity.id);
            }
            Ok(())
        })
    }
    pub(super) fn retire_target(&self, target: &NoticePrincipal) -> Result<(), NoticeError> {
        valid_principal(target)?;
        self.mutate(|state| {
            for work in state.trackers.values_mut() {
                if let Some(pending) = &mut work.pending
                    && pending.record.notice.target.parent == *target
                {
                    pending.eligible = false;
                }
                if work.relationship.parent.as_ref() == Some(target) {
                    work.relationship.parent = None;
                }
            }
            state
                .queue
                .retain(|record| record.notice.target.parent != *target);
            Ok(())
        })
    }
    pub(super) fn release(&self, reference: &WorkNoticeRef) -> Result<(), NoticeError> {
        let identity = self.resolve(reference)?;
        self.mutate(|state| {
            let work = state.trackers.get(&identity.id).ok_or(NoticeError::Stale)?;
            if (!work.stopped && (work.terminal.is_none() || work.next_due.is_some()))
                || work.pending.is_some()
                || state.queue.iter().any(|record| record.work == identity.id)
            {
                return Err(NoticeError::Busy);
            }
            state.trackers.remove(&identity.id);
            Ok(())
        })
    }
    pub(super) fn snapshot(
        self: &Arc<Self>,
        target: &NoticePrincipal,
        count: usize,
        bytes: usize,
    ) -> Result<NoticeBatch, NoticeError> {
        valid_principal(target)?;
        if count == 0 || count > 64 || bytes > 64 * 1024 {
            return Err(NoticeError::InvalidInput);
        }
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut groups: BTreeMap<u64, VecDeque<&Arc<NoticeRecord>>> = BTreeMap::new();
        let mut order = Vec::new();
        let mut available = 0;
        for record in state
            .queue
            .iter()
            .filter(|record| record.notice.target.parent == *target)
        {
            if !groups.contains_key(&record.work) {
                order.push(record.work);
            }
            groups.entry(record.work).or_default().push_back(record);
            available += 1;
        }
        let mut entries = Vec::new();
        let mut encoded = 0;
        loop {
            let mut advanced = false;
            for work in &order {
                if entries.len() == count {
                    break;
                }
                let group = groups.get_mut(work).unwrap();
                let Some(record) = group.front() else {
                    continue;
                };
                if record.encoded_bytes > bytes - encoded {
                    continue;
                }
                entries.push(NoticeBatchEntry {
                    record: Arc::clone(record),
                });
                encoded += record.encoded_bytes;
                group.pop_front();
                advanced = true;
            }
            if !advanced || entries.len() == count {
                break;
            }
        }
        Ok(NoticeBatch {
            inner: Arc::downgrade(self),
            target: target.clone(),
            more: entries.len() < available,
            entries,
            bytes: encoded,
        })
    }
    fn valid_batch_owner(&self, batch: &NoticeBatch) -> Result<(), NoticeError> {
        if !std::ptr::eq(self, batch.inner.as_ptr()) {
            return Err(NoticeError::InvalidBatch);
        }
        Ok(())
    }
    pub(super) fn validate_batch(&self, batch: &NoticeBatch) -> Result<(), NoticeError> {
        self.valid_batch_owner(batch)?;
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for entry in &batch.entries {
            if entry.record.notice.target.parent != batch.target
                || !state
                    .queue
                    .iter()
                    .any(|record| Arc::ptr_eq(record, &entry.record))
            {
                return Err(NoticeError::InvalidBatch);
            }
        }
        Ok(())
    }
    pub(super) fn acknowledge(
        &self,
        batch: &NoticeBatch,
        accepted: &[NoticeAckToken],
    ) -> Result<usize, NoticeError> {
        self.valid_batch_owner(batch)?;
        if accepted.len() > batch.entries.len() {
            return Err(NoticeError::InvalidBatch);
        }
        self.mutate(|state| {
            let mut ids = Vec::with_capacity(accepted.len());
            for token in accepted {
                let Some(record) = token.record.upgrade() else {
                    return Err(NoticeError::InvalidBatch);
                };
                let id = record.id;
                if ids.contains(&id)
                    || !batch
                        .entries
                        .iter()
                        .any(|entry| Arc::ptr_eq(&entry.record, &record))
                    || !state
                        .queue
                        .iter()
                        .any(|original| Arc::ptr_eq(original, &record))
                {
                    return Err(NoticeError::InvalidBatch);
                }
                ids.push(id);
            }
            state.queue.retain(|record| !ids.contains(&record.id));
            Ok(ids.len())
        })
    }
    pub(super) fn usage(&self) -> NoticeUsage {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        NoticeUsage {
            trackers: state.trackers.len(),
            pending: state.queue.len(),
            staged: state
                .trackers
                .values()
                .filter(|work| work.pending.is_some())
                .count(),
            retained_records: self.budget.count.load(Ordering::Acquire),
            retained_bytes: self.budget.bytes.load(Ordering::Acquire),
        }
    }
}
fn overlaps(a: &NoticeEvent, b: &NoticeEvent) -> bool {
    match (a, b) {
        (NoticeEvent::Started, NoticeEvent::Started)
        | (NoticeEvent::Terminal { .. }, NoticeEvent::Terminal { .. }) => true,
        (NoticeEvent::Milestone { name: a }, NoticeEvent::Milestone { name: b }) => a == b,
        (
            NoticeEvent::Interval {
                first_tick: a_first,
                last_tick: a_last,
                ..
            },
            NoticeEvent::Interval {
                first_tick: b_first,
                last_tick: b_last,
                ..
            },
        ) => a_first <= b_last && b_first <= a_last,
        _ => false,
    }
}
fn live(state: &State, id: u64) -> Result<&Tracker, NoticeError> {
    let work = state.trackers.get(&id).ok_or(NoticeError::Stale)?;
    if work.closed || work.stopped {
        return Err(NoticeError::Closed);
    }
    Ok(work)
}
fn emitting(state: &State, id: u64) -> Result<&Tracker, NoticeError> {
    let work = live(state, id)?;
    if work.pending.is_some() {
        return Err(NoticeError::Busy);
    }
    Ok(work)
}
fn retain_transition(work: &mut Tracker, before: Cursor) {
    let transition = Cursor::capture(work);
    if let Some(pending) = &mut work.pending {
        pending.transition = transition;
        before.apply(work);
    }
}
fn fresh_sequence(work: &Tracker, sequence: NonZeroU64, equal: bool) -> Result<(), NoticeError> {
    if sequence.get() < work.sequence || (!equal && sequence.get() == work.sequence) {
        return Err(NoticeError::StaleSource);
    }
    Ok(())
}
fn valid_id(value: &str) -> Result<(), NoticeError> {
    if value.is_empty()
        || value.len() > 255
        || matches!(value, "." | "..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(NoticeError::InvalidInput);
    }
    Ok(())
}
fn valid_principal(value: &NoticePrincipal) -> Result<(), NoticeError> {
    valid_id(&value.id)
}
fn valid_relationship(value: &NoticeRelationship) -> Result<(), NoticeError> {
    if let Some(parent) = &value.parent {
        valid_principal(parent)?;
    }
    Ok(())
}
fn valid_history(
    history: Option<&NoticeHistoryRef>,
    sequence: NonZeroU64,
) -> Result<(), NoticeError> {
    if let Some(history) = history {
        valid_id(&history.record_id)?;
        if history.source_sequence > sequence {
            return Err(NoticeError::InvalidInput);
        }
    }
    Ok(())
}
fn deadlines(
    policy: &ManagedNotifications,
    now: Instant,
) -> Result<(Option<Instant>, Option<Instant>), NoticeError> {
    let add = |ms: u64| {
        now.checked_add(Duration::from_millis(ms))
            .ok_or(NoticeError::DeadlineOverflow)
    };
    Ok((
        policy.report_interval_ms.map(add).transpose()?,
        policy.report_duration_ms.map(add).transpose()?,
    ))
}
