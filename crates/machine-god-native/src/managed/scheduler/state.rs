use super::{SchedulerError, SchedulerLimits, SchedulerSnapshot, TurnHandle, TurnWitness};
use std::{
    collections::{BTreeMap, VecDeque},
    num::NonZeroU64,
    sync::{Arc, Mutex},
    task::Waker,
};

pub(super) struct Inner {
    pub(super) limits: SchedulerLimits,
    pub(super) state: Mutex<State>,
}
pub(super) struct RunIdentity {
    pub(super) id: u64,
    pub(super) work_generation: NonZeroU64,
    pub(super) turn: TurnWitness,
    pub(super) handle: TurnHandle,
}
struct Resident {
    retired: bool,
    generation: u64,
    run: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Registered,
    Queued(u64),
    Executing(u64),
    Waiting { ticket: u64, target: u64 },
    Settling,
}
impl Phase {
    fn ticket(self) -> Option<u64> {
        match self {
            Self::Queued(t) | Self::Executing(t) | Self::Waiting { ticket: t, .. } => Some(t),
            Self::Registered | Self::Settling => None,
        }
    }
}
struct Slot {
    resident: u64,
    phase: Phase,
    turn: TurnWitness,
    handle: TurnHandle,
    waker: Option<Waker>,
}
pub(super) struct State {
    next: Option<u64>,
    residents: BTreeMap<u64, Resident>,
    runs: BTreeMap<u64, Slot>,
    queue: VecDeque<(u64, u64)>,
    executing: usize,
    waiters: usize,
}
impl Default for State {
    fn default() -> Self {
        Self {
            next: Some(1),
            residents: BTreeMap::new(),
            runs: BTreeMap::new(),
            queue: VecDeque::new(),
            executing: 0,
            waiters: 0,
        }
    }
}
/// All externally supplied waker operations/destruction and cancellation happen
/// after unlocking. Removed slots are carried out instead of dropping in-map.
#[derive(Default)]
struct Effects {
    wake: Vec<Waker>,
    discard: Vec<Waker>,
    cancel: Vec<TurnHandle>,
    removed: Vec<Slot>,
}
impl Effects {
    fn wake(&mut self, waker: Option<Waker>) {
        if let Some(w) = waker {
            self.wake.push(w);
        }
    }
    fn discard(&mut self, waker: Option<Waker>) {
        if let Some(w) = waker {
            self.discard.push(w);
        }
    }
    fn finish(self) {
        for handle in self.cancel {
            let _ = handle.cancel();
        }
        for waker in self.wake {
            waker.wake();
        }
        drop(self.discard);
        drop(self.removed);
    }
}
impl State {
    fn allocate(&mut self) -> Result<u64, SchedulerError> {
        let id = self.next.ok_or(SchedulerError::Exhausted)?;
        self.next = id.checked_add(1);
        Ok(id)
    }
    fn stop(&mut self, id: u64, ticket: Option<u64>, cancel: bool, effects: &mut Effects) {
        let Some(slot) = self.runs.get_mut(&id) else {
            return;
        };
        if ticket.is_some() && ticket != slot.phase.ticket() {
            return;
        }
        match slot.phase {
            Phase::Executing(_) => self.executing -= 1,
            Phase::Queued(_) | Phase::Waiting { .. } => self.waiters -= 1,
            Phase::Registered | Phase::Settling => {}
        }
        slot.phase = Phase::Settling;
        if cancel {
            effects.cancel.push(slot.handle.clone());
        }
        effects.wake(slot.waker.take());
        self.queue.retain(|(run, _)| *run != id);
        self.wake_dependents(id, effects);
    }
    fn wake_dependents(&mut self, id: u64, effects: &mut Effects) {
        for slot in self.runs.values_mut() {
            if matches!(slot.phase, Phase::Waiting { target,.. } if target == id) {
                effects.wake(slot.waker.take());
            }
        }
    }
    fn drive(&mut self, limits: SchedulerLimits, effects: &mut Effects) {
        while self.executing < limits.executions.get() {
            let Some((id, ticket)) = self.queue.pop_front() else {
                break;
            };
            let Some(slot) = self.runs.get_mut(&id) else {
                continue;
            };
            if slot.phase != Phase::Queued(ticket) {
                continue;
            }
            if slot.handle.is_cancelled() {
                self.stop(id, Some(ticket), true, effects);
                continue;
            }
            self.waiters -= 1;
            self.executing += 1;
            slot.phase = Phase::Executing(ticket);
            effects.wake(slot.waker.take());
        }
    }
}
impl Inner {
    pub(super) fn reserve_resident(&self) -> Result<u64, SchedulerError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.residents.len() == self.limits.residents.get() {
            return Err(SchedulerError::Capacity);
        }
        let id = state.allocate()?;
        state.residents.insert(
            id,
            Resident {
                retired: false,
                generation: 0,
                run: None,
            },
        );
        Ok(id)
    }
    pub(super) fn retire_resident(&self, id: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(resident) = state.residents.get_mut(&id) {
            resident.retired = true;
            if resident.run.is_none() {
                state.residents.remove(&id);
            }
        }
    }
    pub(super) fn resident_is_idle(&self, id: u64) -> bool {
        self.state.lock().is_ok_and(|state| {
            state
                .residents
                .get(&id)
                .is_some_and(|resident| !resident.retired && resident.run.is_none())
        })
    }
    pub(super) fn register_run(
        &self,
        resident: u64,
        generation: NonZeroU64,
        turn: TurnWitness,
        handle: TurnHandle,
    ) -> Result<Arc<RunIdentity>, SchedulerError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(owner) = state.residents.get(&resident) else {
            return Err(SchedulerError::Stale);
        };
        if owner.retired || generation.get() <= owner.generation {
            return Err(SchedulerError::Stale);
        }
        if owner.run.is_some() {
            return Err(SchedulerError::Busy);
        }
        if state.runs.values().any(|slot| slot.turn.same_turn(&turn)) {
            return Err(SchedulerError::Busy);
        }
        let id = state.allocate()?;
        let owner = state
            .residents
            .get_mut(&resident)
            .expect("validated resident");
        owner.run = Some(id);
        owner.generation = generation.get();
        state.runs.insert(
            id,
            Slot {
                resident,
                phase: Phase::Registered,
                turn: turn.clone(),
                handle: handle.clone(),
                waker: None,
            },
        );
        Ok(Arc::new(RunIdentity {
            id,
            work_generation: generation,
            turn,
            handle,
        }))
    }
    pub(super) fn is_executing(&self, id: u64) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .runs
            .get(&id)
            .is_some_and(|slot| {
                matches!(slot.phase, Phase::Executing(_)) && !slot.handle.is_cancelled()
            })
    }
    pub(super) fn snapshot(&self) -> SchedulerSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        SchedulerSnapshot {
            residents: state.residents.len(),
            executing: state.executing,
            waiters: state.waiters,
            queued: state.queue.len(),
            dependencies: state
                .runs
                .values()
                .filter(|s| matches!(s.phase, Phase::Waiting { .. }))
                .count(),
            settling: state
                .runs
                .values()
                .filter(|s| s.phase == Phase::Settling)
                .count(),
        }
    }
    pub(super) fn stop(&self, id: u64, ticket: Option<u64>, cancel: bool) {
        let mut effects = Effects::default();
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.stop(id, ticket, cancel, &mut effects);
            state.drive(self.limits, &mut effects);
        }
        effects.finish();
    }
    pub(super) fn complete(&self, id: u64) -> Result<(), SchedulerError> {
        let mut effects = Effects::default();
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(slot) = state.runs.get(&id) else {
                return Err(SchedulerError::Stale);
            };
            if slot.phase != Phase::Settling {
                return Err(SchedulerError::NotSettling);
            }
            let slot = state.runs.remove(&id).expect("validated run");
            let resident = state
                .residents
                .get_mut(&slot.resident)
                .expect("run pins resident");
            debug_assert_eq!(resident.run, Some(id));
            resident.run = None;
            if resident.retired {
                state.residents.remove(&slot.resident);
            }
            effects.removed.push(slot);
            state.wake_dependents(id, &mut effects);
        }
        effects.finish();
        Ok(())
    }
    /// First poll enqueues, including immediately available capacity: no bypass.
    pub(super) fn poll_acquire(
        &self,
        id: u64,
        ticket: &mut Option<u64>,
        waker: Waker,
    ) -> Result<bool, SchedulerError> {
        let mut effects = Effects::default();
        let mut waker = Some(waker);
        let result = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (|| {
                let Some(slot) = state.runs.get(&id) else {
                    return Err(SchedulerError::Stale);
                };
                if slot.handle.is_cancelled() || slot.phase == Phase::Settling {
                    return Err(SchedulerError::Cancelled);
                }
                if ticket.is_none() {
                    if slot.phase != Phase::Registered {
                        return Err(SchedulerError::Busy);
                    }
                    if state.waiters == self.limits.waiters.get() {
                        return Err(SchedulerError::Capacity);
                    }
                    let next = state.allocate()?;
                    *ticket = Some(next);
                    state.waiters += 1;
                    state.runs.get_mut(&id).unwrap().phase = Phase::Queued(next);
                    state.queue.push_back((id, next));
                }
                let ticket = ticket.expect("admitted acquisition");
                let slot = state.runs.get_mut(&id).unwrap();
                match slot.phase {
                    Phase::Executing(t) if t == ticket => return Ok(true),
                    Phase::Queued(t) if t == ticket => {}
                    _ => return Err(SchedulerError::Stale),
                }
                effects.discard(slot.waker.replace(waker.take().unwrap()));
                state.drive(self.limits, &mut effects);
                Ok(state
                    .runs
                    .get(&id)
                    .is_some_and(|slot| slot.phase == Phase::Executing(ticket)))
            })()
        };
        effects.finish();
        drop(waker);
        result
    }
    pub(super) fn begin_wait(
        &self,
        id: u64,
        target: u64,
        waker: Waker,
    ) -> Result<u64, SchedulerError> {
        let mut effects = Effects::default();
        let mut waker = Some(waker);
        let result = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (|| {
                if id == target {
                    return Err(SchedulerError::DependencyCycle);
                }
                let caller = state.runs.get(&id).ok_or(SchedulerError::Stale)?;
                if caller.handle.is_cancelled() {
                    return Err(SchedulerError::Cancelled);
                }
                if !matches!(caller.phase, Phase::Executing(_)) {
                    return Err(SchedulerError::Busy);
                }
                let target_slot = state
                    .runs
                    .get(&target)
                    .ok_or(SchedulerError::Unschedulable)?;
                if target_slot.phase == Phase::Registered {
                    return Err(SchedulerError::Unschedulable);
                }
                let mut cursor = target;
                for _ in 0..=state.runs.len() {
                    if cursor == id {
                        return Err(SchedulerError::DependencyCycle);
                    }
                    match state.runs.get(&cursor).map(|slot| slot.phase) {
                        Some(Phase::Waiting { target, .. }) => cursor = target,
                        _ => break,
                    }
                }
                if state.waiters == self.limits.waiters.get() {
                    return Err(SchedulerError::Capacity);
                }
                let ticket = state.allocate()?;
                // Edge and waiter reservation precede quota release in this lock.
                let caller = state.runs.get_mut(&id).unwrap();
                caller.phase = Phase::Waiting { ticket, target };
                effects.discard(caller.waker.replace(waker.take().unwrap()));
                state.waiters += 1;
                state.executing -= 1;
                state.drive(self.limits, &mut effects);
                Ok(ticket)
            })()
        };
        effects.finish();
        drop(waker);
        result
    }
    pub(super) fn refresh_wait(
        &self,
        id: u64,
        ticket: u64,
        waker: Waker,
    ) -> Result<(), SchedulerError> {
        let mut discard = None;
        let mut waker = Some(waker);
        let result = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.runs.get_mut(&id) {
                Some(slot)
                    if matches!(slot.phase,Phase::Waiting { ticket:t,.. } if t == ticket)
                        && !slot.handle.is_cancelled() =>
                {
                    discard = slot.waker.replace(waker.take().unwrap());
                    Ok(())
                }
                _ => Err(SchedulerError::Cancelled),
            }
        };
        drop(discard);
        drop(waker);
        result
    }
    pub(super) fn poll_reacquire(
        &self,
        id: u64,
        ticket: u64,
        waker: Waker,
    ) -> Result<bool, SchedulerError> {
        let mut effects = Effects::default();
        let mut waker = Some(waker);
        let result = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (|| {
                let slot = state.runs.get_mut(&id).ok_or(SchedulerError::Stale)?;
                if slot.handle.is_cancelled() {
                    return Err(SchedulerError::Cancelled);
                }
                match slot.phase {
                    Phase::Waiting { ticket: t, .. } if t == ticket => {
                        // Same waiter reservation now queues behind all prior grants.
                        slot.phase = Phase::Queued(ticket);
                        effects.discard(slot.waker.replace(waker.take().unwrap()));
                        state.queue.push_back((id, ticket));
                    }
                    Phase::Queued(t) if t == ticket => {
                        effects.discard(slot.waker.replace(waker.take().unwrap()));
                    }
                    Phase::Executing(t) if t == ticket => return Ok(true),
                    _ => return Err(SchedulerError::Cancelled),
                }
                state.drive(self.limits, &mut effects);
                Ok(state
                    .runs
                    .get(&id)
                    .is_some_and(|slot| slot.phase == Phase::Executing(ticket)))
            })()
        };
        effects.finish();
        drop(waker);
        result
    }
}
