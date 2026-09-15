//! Bounded fair scheduling for managed native runs, independent of any executor.
//! Only the native manager creates residents/runs from actual core Turns.
#[path = "scheduler/state.rs"]
mod state;
#[cfg(test)]
#[path = "scheduler/tests.rs"]
mod tests;
#[path = "scheduler/waiting.rs"]
mod waiting;

use machine_god_core::{CancellationToken, Turn, TurnHandle, TurnWitness};
use state::{Inner, RunIdentity, State};
use std::{
    fmt,
    future::Future,
    num::{NonZeroU64, NonZeroUsize},
    sync::{Arc, Mutex, Weak},
};
pub(crate) use waiting::{Acquire, DependencyWait};

const MAX_EXECUTIONS: usize = 256;
const MAX_RESIDENTS: usize = 4096;
const MAX_WAITERS: usize = 4096;

/// Live resource limits, never a lifetime creation or history-retention cap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SchedulerLimits {
    executions: NonZeroUsize,
    residents: NonZeroUsize,
    waiters: NonZeroUsize,
}
impl SchedulerLimits {
    pub(crate) fn new(
        executions: usize,
        residents: usize,
        waiters: usize,
    ) -> Result<Self, SchedulerError> {
        if executions == 0
            || residents == 0
            || waiters == 0
            || executions > residents
            || executions > MAX_EXECUTIONS
            || residents > MAX_RESIDENTS
            || waiters > MAX_WAITERS
        {
            return Err(SchedulerError::InvalidLimits);
        }
        Ok(Self {
            executions: NonZeroUsize::new(executions).unwrap(),
            residents: NonZeroUsize::new(residents).unwrap(),
            waiters: NonZeroUsize::new(waiters).unwrap(),
        })
    }
}
impl Default for SchedulerLimits {
    fn default() -> Self {
        Self::new(4, 64, 64).expect("valid defaults")
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SchedulerError {
    InvalidLimits,
    Capacity,
    Exhausted,
    Foreign,
    Stale,
    Busy,
    Cancelled,
    DependencyCycle,
    Unschedulable,
    NotSettling,
}
impl fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("managed scheduler rejected operation")
    }
}
impl std::error::Error for SchedulerError {}

/// Only scalar observations; not execution authority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SchedulerSnapshot {
    pub(crate) residents: usize,
    pub(crate) executing: usize,
    pub(crate) waiters: usize,
    pub(crate) queued: usize,
    pub(crate) dependencies: usize,
    pub(crate) settling: usize,
}
#[derive(Clone)]
pub(crate) struct ManagedScheduler {
    inner: Arc<Inner>,
}
impl ManagedScheduler {
    pub(crate) fn new(limits: SchedulerLimits) -> Self {
        Self {
            inner: Arc::new(Inner {
                limits,
                state: Mutex::new(State::default()),
            }),
        }
    }
    /// Native manager admission. No threads, timers, models or I/O are started.
    pub(super) fn reserve_resident(&self) -> Result<ResidentLease, SchedulerError> {
        let id = self.inner.reserve_resident()?;
        Ok(ResidentLease {
            inner: self.inner.clone(),
            id,
        })
    }
    /// Binds a fresh work generation to an actual reserved, unpolled core Turn.
    /// The actual worker/finalizer must retain the separate settlement owner.
    pub(super) fn register_run(
        &self,
        resident: &ResidentLease,
        work_generation: NonZeroU64,
        turn: &Turn,
    ) -> Result<(RunLease, RunSettlement), SchedulerError> {
        if !Arc::ptr_eq(&resident.inner, &self.inner) {
            return Err(SchedulerError::Foreign);
        }
        let witness = turn.witness();
        if !witness.is_live() {
            return Err(SchedulerError::Cancelled);
        }
        let identity =
            self.inner
                .register_run(resident.id, work_generation, witness, turn.handle())?;
        Ok((
            RunLease {
                inner: self.inner.clone(),
                identity: identity.clone(),
                finished: false,
            },
            RunSettlement {
                inner: self.inner.clone(),
                identity,
                completed: false,
            },
        ))
    }
    pub(crate) fn snapshot(&self) -> SchedulerSnapshot {
        self.inner.snapshot()
    }

    pub(super) fn resident_is_idle(&self, resident: &ResidentLease) -> bool {
        Arc::ptr_eq(&resident.inner, &self.inner) && self.inner.resident_is_idle(resident.id)
    }
}
impl fmt::Debug for ManagedScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedScheduler")
            .field("limits", &self.inner.limits)
            .finish_non_exhaustive()
    }
}

/// Non-clone residency owner. Dropping this while work settles does not reuse
/// capacity; only actual settlement completes that release.
pub(crate) struct ResidentLease {
    inner: Arc<Inner>,
    id: u64,
}
impl Drop for ResidentLease {
    fn drop(&mut self) {
        self.inner.retire_resident(self.id);
    }
}
impl fmt::Debug for ResidentLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResidentLease").finish_non_exhaustive()
    }
}

/// Non-clone actual work-generation owner, retained by the native driver.
/// Ordinary provider Pending keeps its execution slot. Only an explicitly
/// authenticated dependency wait may temporarily release it.
pub(crate) struct RunLease {
    inner: Arc<Inner>,
    identity: Arc<RunIdentity>,
    finished: bool,
}
impl RunLease {
    pub(crate) fn reference(&self) -> RunRef {
        RunRef {
            inner: Arc::downgrade(&self.inner),
            identity: Arc::downgrade(&self.identity),
        }
    }
    pub(crate) fn acquire(&self) -> Acquire {
        self.reference().acquire()
    }
    /// Requests actual-turn cancellation and gives finalization a settlement
    /// lane, independent of all occupied model execution slots.
    pub(crate) fn cancel(&self) {
        self.inner.stop(self.identity.id, None, true);
    }
    /// Driver calls only after normal model/turn execution has ended.
    pub(crate) fn finish(mut self) {
        self.inner.stop(self.identity.id, None, false);
        self.finished = true;
    }
}
impl Drop for RunLease {
    fn drop(&mut self) {
        if !self.finished {
            self.inner.stop(self.identity.id, None, true);
        }
    }
}
impl fmt::Debug for RunLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunLease").finish_non_exhaustive()
    }
}

/// Opaque weak exact run reference; no ID constructor, owning runtime or manager.
#[derive(Clone)]
pub(crate) struct RunRef {
    inner: Weak<Inner>,
    identity: Weak<RunIdentity>,
}
impl RunRef {
    pub(crate) fn same_run(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.inner, &other.inner) && Weak::ptr_eq(&self.identity, &other.identity)
    }

    /// Cancels this original run even if its execution owner is elsewhere.
    pub(crate) fn cancel(&self) {
        if let Ok((inner, identity)) = self.resolve() {
            inner.stop(identity.id, None, true);
        }
    }

    pub(crate) fn work_generation(&self) -> Option<NonZeroU64> {
        self.identity
            .upgrade()
            .map(|identity| identity.work_generation)
    }
    pub(crate) fn matches_turn(&self, witness: &TurnWitness) -> bool {
        self.identity
            .upgrade()
            .is_some_and(|identity| identity.turn.same_turn(witness))
    }
    pub(crate) fn is_executing(&self) -> bool {
        self.resolve()
            .is_ok_and(|(inner, identity)| inner.is_executing(identity.id))
    }
    pub(crate) fn acquire(&self) -> Acquire {
        Acquire::new(self.clone())
    }
    /// Native private call admission supplies this reference only after claiming
    /// the core invocation and binding its actual principal/work generation.
    /// Completion, timeout and observer errors all reacquire FIFO quota before
    /// returning. Abandonment instead cancels the actual turn, never resumes it.
    pub(crate) fn dependency_wait<F: Future>(
        &self,
        target: Self,
        observation: F,
        cancellation: CancellationToken,
    ) -> DependencyWait<F> {
        DependencyWait::new(self.clone(), target, observation, cancellation)
    }
    fn resolve(&self) -> Result<(Arc<Inner>, Arc<RunIdentity>), SchedulerError> {
        let inner = self.inner.upgrade().ok_or(SchedulerError::Stale)?;
        let identity = self.identity.upgrade().ok_or(SchedulerError::Stale)?;
        Ok((inner, identity))
    }
}
impl fmt::Debug for RunRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunRef").finish_non_exhaustive()
    }
}

/// Separate actual-worker/finalizer ownership, NOT a response observer.
/// Dropping without complete quarantines the bounded residency slot; it cannot
/// assert that worker/TLS/process reap has finished.
pub(crate) struct RunSettlement {
    inner: Arc<Inner>,
    identity: Arc<RunIdentity>,
    completed: bool,
}
impl RunSettlement {
    pub(crate) fn complete(mut self) -> Result<(), SchedulerError> {
        self.inner.complete(self.identity.id)?;
        self.completed = true;
        Ok(())
    }
}
impl Drop for RunSettlement {
    fn drop(&mut self) {
        if !self.completed {
            self.inner.stop(self.identity.id, None, true);
        }
    }
}
impl fmt::Debug for RunSettlement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunSettlement").finish_non_exhaustive()
    }
}
