//! Per-run attribution under the existing host/collector domain.

use super::{
    NativeOwnedWorkerCleanup, NativeOwnedWorkerCompletion, NativeOwnedWorkerScope,
    NativeOwnedWorkerSpawnError, NativeOwnedWorkerTicket, ScopeState, ScopeStatus, ScopeTicket,
    current_worker_ticket,
};
use std::cell::RefCell;
use std::fmt;
use std::sync::{Arc, Mutex, Weak};

const MAX_RUNS: usize = 64;
const MAX_CLEANUP_RUNS: usize = 64;

#[derive(Clone, Copy, Default)]
pub(super) enum RunClass {
    #[default]
    Ordinary,
    Cleanup,
}

impl RunClass {
    const fn limit(self) -> usize {
        match self {
            Self::Ordinary => MAX_RUNS,
            Self::Cleanup => MAX_CLEANUP_RUNS,
        }
    }

    fn count(self, status: &mut ScopeStatus) -> &mut usize {
        match self {
            Self::Ordinary => &mut status.runs,
            Self::Cleanup => &mut status.cleanup_runs,
        }
    }
}

type RunAdmission = (Arc<ScopeTicket>, Option<Arc<dyn Send + Sync>>);

/// One admitted run's completion cohort. It owns neither a runtime nor worker.
/// The manager binds this non-clone value to its actual scheduler run. Closing
/// (including Drop) ends admission, not already-enrolled cleanup custody.
pub struct NativeOwnedWorkerRun {
    scope: NativeOwnedWorkerScope,
    host: Weak<ScopeState>,
}

impl fmt::Debug for NativeOwnedWorkerRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeOwnedWorkerRun")
            .finish_non_exhaustive()
    }
}

impl NativeOwnedWorkerRun {
    /// Attributes exactly this synchronous poll and restores previous metadata
    /// on return or unwind. Never hold a thread-local guard across an await.
    pub fn with_poll<T>(&self, operation: impl FnOnce() -> T) -> T {
        RunAttribution::with(
            Some(RunAttribution {
                host: self.host.clone(),
                state: Arc::downgrade(&self.scope.state),
            }),
            operation,
        )
    }

    /// Stops new work; admitted worker/TLS/reap obligations still own tickets.
    pub fn close(&self) {
        self.scope.close();
    }

    /// Observes only this run, independently of sibling runs and service work.
    #[must_use]
    pub fn completion(&self) -> NativeOwnedWorkerCompletion {
        self.scope.completion()
    }
}

impl Drop for NativeOwnedWorkerRun {
    fn drop(&mut self) {
        self.close();
    }
}

impl NativeOwnedWorkerScope {
    /// Explicit trusted nested ownership, never inferred from a foreign scope.
    /// The target gets a new host ticket but uses the exact original run cohort.
    pub(crate) fn with_inherited_run_from<T>(
        &self,
        source: &Self,
        operation: impl FnOnce() -> T,
    ) -> Result<T, NativeOwnedWorkerSpawnError> {
        let inherited = if let Some(mut run) = RunAttribution::current() {
            if !Weak::ptr_eq(&run.host, &Arc::downgrade(&source.state)) {
                return Err(NativeOwnedWorkerSpawnError);
            }
            run.host = Arc::downgrade(&self.state);
            Some(run)
        } else {
            if !current_worker_ticket()
                .is_some_and(|ticket| Arc::ptr_eq(&ticket.0.state, &source.state))
            {
                return Err(NativeOwnedWorkerSpawnError);
            }
            None
        };
        Ok(RunAttribution::with(inherited, operation))
    }
    /// Admits a bounded, inert run cohort without starting any native worker.
    /// At most 64 open or unsettled ordinary cohorts share one host. Settled observers
    /// do not consume capacity, so this is not a lifetime creation limit.
    ///
    /// # Errors
    /// Rejects a closed host or exhausted resident cohort capacity.
    pub fn begin_run(&self) -> Result<NativeOwnedWorkerRun, NativeOwnedWorkerSpawnError> {
        self.begin_run_inner(None, RunClass::Ordinary)
    }

    /// Retains only the manager's journal-owner lease, never a runtime/session.
    /// Independent host tickets keep this custody after successful promotion.
    pub(crate) fn begin_run_with_keepalive(
        &self,
        keepalive: Arc<dyn Send + Sync>,
    ) -> Result<NativeOwnedWorkerRun, NativeOwnedWorkerSpawnError> {
        self.begin_run_inner(Some(keepalive), RunClass::Ordinary)
    }

    /// Explicit trusted settlement reserve; ordinary runs cannot borrow it.
    /// This uses the same actual worker/keepalive custody, not another pool.
    pub(crate) fn begin_cleanup_run_with_keepalive(
        &self,
        keepalive: Arc<dyn Send + Sync>,
    ) -> Result<NativeOwnedWorkerRun, NativeOwnedWorkerSpawnError> {
        self.begin_run_inner(Some(keepalive), RunClass::Cleanup)
    }

    /// Observation only, not a reservation. Callers retry exact admission under
    /// their original deadline; a closed host is an error rather than a wake loop.
    #[cfg(any(test, feature = "ai-gateway-http"))]
    pub(crate) fn wait_for_cleanup_capacity(
        &self,
    ) -> machine_god_core::BoxFuture<'_, Result<(), NativeOwnedWorkerSpawnError>> {
        Box::pin(self.state.wait_until(|| {
            let status = self
                .state
                .status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if status.closed {
                Some(Err(NativeOwnedWorkerSpawnError))
            } else if status.cleanup_runs < MAX_CLEANUP_RUNS {
                Some(Ok(()))
            } else {
                None
            }
        }))
    }

    fn begin_run_inner(
        &self,
        keepalive: Option<Arc<dyn Send + Sync>>,
        class: RunClass,
    ) -> Result<NativeOwnedWorkerRun, NativeOwnedWorkerSpawnError> {
        let mut status = self
            .state
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if status.closed || *class.count(&mut status) == class.limit() {
            return Err(NativeOwnedWorkerSpawnError);
        }
        *class.count(&mut status) += 1;
        Ok(NativeOwnedWorkerRun {
            scope: NativeOwnedWorkerScope {
                state: Arc::new(ScopeState {
                    parent: Some(Arc::downgrade(&self.state)),
                    run_class: class,
                    status: Mutex::new(ScopeStatus {
                        keepalive,
                        ..ScopeStatus::default()
                    }),
                    ..ScopeState::default()
                }),
            },
            host: Arc::downgrade(&self.state),
        })
    }
}

impl ScopeState {
    pub(super) fn release_run_capacity(&self) {
        let Some(parent) = self.parent.as_ref() else {
            return;
        };
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if status.released || !status.closed || status.tickets != 0 {
            return;
        }
        let keepalive = status.keepalive.take();
        drop(status);
        // No scope or accounting lock spans destruction of the journal lease.
        drop(keepalive);
        let parent = parent.upgrade();
        if let Some(parent) = &parent {
            *self.run_class.count(
                &mut parent
                    .status
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ) -= 1;
        }
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .released = true;
        if let Some(parent) = parent {
            parent.notify_progress();
        }
    }
}

#[derive(Clone)]
pub(super) struct RunAttribution {
    host: Weak<ScopeState>,
    state: Weak<ScopeState>,
}

#[derive(Clone)]
enum RunContext {
    Inherit,
    Selected(Option<RunAttribution>),
}
thread_local! {
    // Selected(None) freezes an unbound operation instead of adopting a later caller.
    static RUN: RefCell<RunContext> = const { RefCell::new(RunContext::Inherit) };
}

impl RunAttribution {
    pub(super) fn current() -> Option<Self> {
        match RUN.try_with(|slot| slot.borrow().clone()).ok() {
            Some(RunContext::Selected(attribution)) => attribution,
            _ => current_worker_ticket().and_then(|ticket| ticket.attribution()),
        }
    }

    pub(super) fn with<T>(attribution: Option<Self>, operation: impl FnOnce() -> T) -> T {
        struct Restore<'a>(&'a RefCell<RunContext>, RunContext);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                drop(
                    self.0
                        .replace(std::mem::replace(&mut self.1, RunContext::Inherit)),
                );
            }
        }
        RUN.with(|slot| {
            let _restore = Restore(slot, slot.replace(RunContext::Selected(attribution)));
            operation()
        })
    }

    pub(super) fn matches(&self, state: &Arc<ScopeState>) -> bool {
        Weak::ptr_eq(&self.state, &Arc::downgrade(state))
    }

    pub(super) fn admit(
        &self,
        host: &Arc<ScopeState>,
    ) -> Result<RunAdmission, NativeOwnedWorkerSpawnError> {
        if !Weak::ptr_eq(&self.host, &Arc::downgrade(host)) {
            return Err(NativeOwnedWorkerSpawnError);
        }
        let state = self.state.upgrade().ok_or(NativeOwnedWorkerSpawnError)?;
        let mut status = state
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if status.closed {
            return Err(NativeOwnedWorkerSpawnError);
        }
        status.tickets = status
            .tickets
            .checked_add(1)
            .ok_or(NativeOwnedWorkerSpawnError)?;
        let keepalive = status.keepalive.clone();
        drop(status);
        Ok((
            Arc::new(ScopeTicket {
                state,
                keepalive: None,
            }),
            keepalive,
        ))
    }
}

pub(super) struct RunEnrollment(Mutex<Option<Arc<ScopeTicket>>>);
impl RunEnrollment {
    pub(super) fn new(ticket: Option<Arc<ScopeTicket>>) -> Self {
        Self(Mutex::new(ticket))
    }
    fn snapshot(&self) -> Option<Arc<ScopeTicket>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub(super) fn clear(&self) {
        let ticket = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(ticket);
    }
}

#[derive(Clone)]
pub(super) struct TicketWitness {
    host: Weak<ScopeTicket>,
    run: Weak<RunEnrollment>,
}
impl TicketWitness {
    pub(super) fn upgrade(&self) -> Option<NativeOwnedWorkerTicket> {
        Some(NativeOwnedWorkerTicket(
            self.host.upgrade()?,
            self.run.upgrade()?,
        ))
    }
}
impl NativeOwnedWorkerTicket {
    pub(super) fn attribution(&self) -> Option<RunAttribution> {
        let run = self.1.snapshot()?;
        Some(RunAttribution {
            host: Arc::downgrade(&self.0.state),
            state: Arc::downgrade(&run.state),
        })
    }
    pub(super) fn witness(&self) -> TicketWitness {
        TicketWitness {
            host: Arc::downgrade(&self.0),
            run: Arc::downgrade(&self.1),
        }
    }
    pub(super) fn snapshot(&self) -> Self {
        Self(
            Arc::clone(&self.0),
            Arc::new(RunEnrollment::new(self.1.snapshot())),
        )
    }
    pub(super) fn owns(&self, state: &Arc<ScopeState>) -> bool {
        Arc::ptr_eq(&self.0.state, state)
            || self
                .1
                .snapshot()
                .is_some_and(|run| Arc::ptr_eq(&run.state, state))
    }
}

/// Called only at the successful initialization boundary of a dedicated
/// long-lived service worker. Its collector/host ticket remains untouched.
pub(crate) fn promote_current_worker_to_service() {
    if let Some(handoff) = current_service_handoff() {
        handoff.promote();
    }
}

/// A one-shot, weak token for exactly one original dedicated worker enrollment.
/// Losing a startup receipt or dropping this token never promotes anything.
#[derive(Debug)]
pub(crate) struct NativeOwnedWorkerServiceHandoff(Weak<RunEnrollment>);
#[cfg(target_os = "macos")]
impl NativeOwnedWorkerCleanup {
    /// Freezes only this resource's original enrollment; later replacement of
    /// a containing service cannot retarget the weak handoff receipt.
    pub(crate) fn service_handoff(&self) -> NativeOwnedWorkerServiceHandoff {
        NativeOwnedWorkerServiceHandoff(Arc::downgrade(&self.ticket.1))
    }
}

impl NativeOwnedWorkerServiceHandoff {
    pub(crate) fn promote(self) {
        if let Some(enrollment) = self.0.upgrade() {
            enrollment.clear();
        }
    }
}
pub(crate) fn current_service_handoff() -> Option<NativeOwnedWorkerServiceHandoff> {
    current_worker_ticket().map(|ticket| NativeOwnedWorkerServiceHandoff(Arc::downgrade(&ticket.1)))
}

/// Weak frozen attribution for a queued effect. IDs and whichever caller later
/// polls the future cannot retarget its cleanup to a different run.
pub(crate) struct NativeOwnedWorkerAttribution {
    run: Option<RunAttribution>,
    worker: Option<FrozenTicketWitness>,
}
struct FrozenTicketWitness {
    host: Weak<ScopeTicket>,
    run: Option<Weak<ScopeTicket>>,
}
impl NativeOwnedWorkerAttribution {
    /// Runs first-poll admission under the exact construction context, including
    /// an explicitly unbound caller. The admitted cleanup keeps journal custody
    /// while the closure transfers effects to their actual owned workers.
    pub(crate) fn with_admission<T>(
        &self,
        operation: impl FnOnce() -> T,
    ) -> Result<T, NativeOwnedWorkerSpawnError> {
        match self.admit()? {
            Some(cleanup) => Ok(cleanup.run_on_cleanup_worker(operation)),
            None => Ok(RunAttribution::with(None, operation)),
        }
    }

    pub(crate) fn current() -> Self {
        let run = RunAttribution::current();
        // An embedded driver may explicitly poll run A on a worker belonging
        // to B. Only an exactly matching worker ticket is a continuation of
        // the selected operation; ambient worker identity cannot override A.
        let worker = current_worker_ticket().filter(|ticket| match (&run, ticket.attribution()) {
            (Some(selected), Some(worker)) => {
                Weak::ptr_eq(&selected.host, &worker.host)
                    && Weak::ptr_eq(&selected.state, &worker.state)
            }
            (None, None) => true,
            _ => false,
        });
        Self {
            run,
            worker: worker.map(|ticket| FrozenTicketWitness {
                host: Arc::downgrade(&ticket.0),
                run: ticket.1.snapshot().as_ref().map(Arc::downgrade),
            }),
        }
    }

    pub(crate) fn admit(
        &self,
    ) -> Result<Option<NativeOwnedWorkerCleanup>, NativeOwnedWorkerSpawnError> {
        // A worker-origin callback transfers an existing operation obligation,
        // including rollback after closure, rather than admitting new work.
        if let Some(worker) = self.worker.as_ref() {
            let host = worker.host.upgrade().ok_or(NativeOwnedWorkerSpawnError)?;
            let run = worker
                .run
                .as_ref()
                .map(|run| run.upgrade().ok_or(NativeOwnedWorkerSpawnError))
                .transpose()?;
            return Ok(Some(NativeOwnedWorkerCleanup {
                ticket: NativeOwnedWorkerTicket(host, Arc::new(RunEnrollment::new(run))),
            }));
        }
        let Some(run) = &self.run else {
            return Ok(None);
        };
        let host = run.host.upgrade().ok_or(NativeOwnedWorkerSpawnError)?;
        let scope = NativeOwnedWorkerScope { state: host };
        RunAttribution::with(Some(run.clone()), || scope.admit())
            .map(|ticket| Some(NativeOwnedWorkerCleanup { ticket }))
    }
}

#[cfg(test)]
mod cleanup_tests;
#[cfg(test)]
mod tests;
