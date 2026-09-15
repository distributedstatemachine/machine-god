//! Bridge actual conversation turns to principal and shared scheduler admission.

use super::mcp::{NativePrincipalMcpOwner, NativePrincipalMcpTurn};
use super::principal::{NativePrincipal, NativePrincipalRegistry, NativePrincipalTurn};
use super::scheduler::{Acquire, ManagedScheduler, ResidentLease, RunLease, RunRef, RunSettlement};
use super::store::JournalOwner;
use crate::owned_worker::NativeOwnedWorkerRun;
use crate::{
    NativeConversationError, NativeModelPreferences, NativePermissionPolicySnapshot,
    NativeWorkspaceAuthority,
};
use machine_god_core::{Session, Turn};
use std::{
    fmt,
    future::Future,
    num::NonZeroU64,
    pin::Pin,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

type Result<T> = std::result::Result<T, NativeConversationError>;

#[path = "conversation/admission.rs"]
mod admission;
pub(crate) use admission::ManagedAdmission;

struct Owner {
    principal: Arc<NativePrincipal>,
    scheduler: ManagedScheduler,
    resident: ResidentLease,
    active: Mutex<Active>,
    closed: AtomicBool,
    mcp: OnceLock<Weak<NativePrincipalMcpOwner>>,
    workers: OnceLock<WorkerBinding>,
}

struct WorkerBinding {
    scope: crate::NativeOwnedWorkerScope,
    keepalive: Arc<dyn Send + Sync>,
}

#[derive(Clone)]
pub(crate) struct ManagedRunCleanup {
    run: RunRef,
    cohort: Arc<NativeOwnedWorkerRun>,
}

impl ManagedRunCleanup {
    pub(crate) fn with_poll<T>(&self, operation: impl FnOnce() -> T) -> T {
        self.cohort.with_poll(operation)
    }

    pub(crate) fn completion(&self) -> crate::NativeOwnedWorkerCompletion {
        self.cohort.completion()
    }
}

#[derive(Default)]
struct Active {
    run: Option<RunRef>,
    settlement: Option<RunSettlement>,
    cleanup: Option<ManagedRunCleanup>,
    admission: Option<Arc<NativeOwnedWorkerRun>>,
}

/// The outer manager keeps this owner independently of the creating tool/turn.
/// It owns no session, conversation or engine, so its reverse binding is acyclic.
pub(crate) struct ManagedConversationOwner(Arc<Owner>);

impl ManagedConversationOwner {
    pub(crate) fn new(
        registry: &NativePrincipalRegistry,
        session: &Session,
        scheduler: ManagedScheduler,
        generation: u64,
        workspace: &NativeWorkspaceAuthority,
    ) -> Result<Self> {
        let resident = scheduler
            .reserve_resident()
            .map_err(|_| NativeConversationError::ManagedAdmission)?;
        let principal = registry
            .register(session, generation, workspace)
            .map_err(|_| NativeConversationError::ManagedAdmission)?;
        Ok(Self(Arc::new(Owner {
            principal,
            scheduler,
            resident,
            active: Mutex::new(Active::default()),
            closed: AtomicBool::new(false),
            mcp: OnceLock::new(),
            workers: OnceLock::new(),
        })))
    }

    pub(crate) fn principal(&self) -> &Arc<NativePrincipal> {
        &self.0.principal
    }

    pub(crate) fn binding(&self) -> ManagedConversationBinding {
        ManagedConversationBinding(Arc::downgrade(&self.0))
    }

    /// Binds only this exact principal's independently owned MCP registration.
    /// The manager retains the actual MCP owner, runtime and permission bundle.
    pub(crate) fn configure_mcp(&self, mcp: &Arc<NativePrincipalMcpOwner>) -> Result<()> {
        if self.0.closed.load(Ordering::Acquire)
            || !self.0.scheduler.resident_is_idle(&self.0.resident)
            || !mcp.matches_principal(&self.0.principal)
        {
            return Err(NativeConversationError::ManagedAdmission);
        }
        self.0
            .mcp
            .set(Arc::downgrade(mcp))
            .map_err(|_| NativeConversationError::ManagedAdmission)
    }

    /// Actual collector tickets retain the journal lease independently of the
    /// manager and of service promotion's execution-quota refund.
    pub(crate) fn configure_workers(
        &self,
        scope: crate::NativeOwnedWorkerScope,
        owner: JournalOwner,
    ) -> Result<()> {
        self.configure_worker_binding(scope, Arc::new(owner))
    }

    fn configure_worker_binding(
        &self,
        scope: crate::NativeOwnedWorkerScope,
        keepalive: Arc<dyn Send + Sync>,
    ) -> Result<()> {
        if self.0.closed.load(Ordering::Acquire)
            || !self.0.scheduler.resident_is_idle(&self.0.resident)
        {
            return Err(NativeConversationError::ManagedAdmission);
        }
        self.0
            .workers
            .set(WorkerBinding { scope, keepalive })
            .map_err(|_| NativeConversationError::ManagedAdmission)
    }

    pub(crate) fn cleanup_for(&self, run: &RunRef) -> Result<ManagedRunCleanup> {
        self.binding().cleanup_for(run)
    }

    /// Metadata-only target for authenticated dependency waits.
    pub(crate) fn run(&self) -> Option<RunRef> {
        self.0.active.lock().ok()?.run.clone()
    }

    /// Transfer actual finalizer custody to the manager, never to a response.
    /// The manager must retain it until all original worker/TLS/reap obligations
    /// finish, then call `complete`. Taking it does not release scheduler capacity.
    pub(crate) fn take_settlement(&self) -> Option<(RunRef, RunSettlement)> {
        let mut active = self.0.active.lock().ok()?;
        let reference = active.run.clone()?;
        active
            .settlement
            .take()
            .map(|settlement| (reference, settlement))
    }

    /// End new principal authority while retaining original cleanup custody.
    pub(crate) fn retire(&self) {
        self.0.closed.store(true, Ordering::Release);
        self.0.principal.retire();
        let run = self
            .0
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .run
            .clone();
        if let Some(run) = run {
            run.cancel();
        }
    }
}

impl ManagedConversationBinding {
    /// Weak lookup only; retaining the binding cannot prolong a principal.
    pub(crate) fn cleanup_for(&self, run: &RunRef) -> Result<ManagedRunCleanup> {
        self.0
            .upgrade()
            .ok_or(NativeConversationError::ManagedAdmission)?
            .active
            .lock()
            .map_err(|_| NativeConversationError::ManagedAdmission)?
            .cleanup
            .as_ref()
            .filter(|cleanup| cleanup.run.same_run(run))
            .cloned()
            .ok_or(NativeConversationError::ManagedAdmission)
    }
}

impl Drop for ManagedConversationOwner {
    fn drop(&mut self) {
        self.retire();
        // Last-owner destruction retires the resident after cancelling the turn.
        // An abandoned settlement remains quarantined, never falsely completed.
    }
}

pub(crate) struct ManagedConversationBinding(Weak<Owner>);

impl ManagedConversationBinding {
    pub(crate) fn validate(&self) -> Result<()> {
        self.0
            .upgrade()
            .filter(|owner| {
                !owner.closed.load(Ordering::Acquire)
                    && owner.principal.is_live()
                    && owner.scheduler.resident_is_idle(&owner.resident)
            })
            .map(|_| ())
            .ok_or(NativeConversationError::ManagedAdmission)
    }

    pub(crate) fn begin(
        &self,
        turn: &Turn,
        work_generation: NonZeroU64,
        policy: NativePermissionPolicySnapshot,
        preferences: NativeModelPreferences,
        cohort: Option<Arc<NativeOwnedWorkerRun>>,
    ) -> Result<ManagedConversationTurn> {
        let owner = self
            .0
            .upgrade()
            .ok_or(NativeConversationError::ManagedAdmission)?;
        let mut slot = owner
            .active
            .lock()
            .map_err(|_| NativeConversationError::ManagedAdmission)?;
        if owner.closed.load(Ordering::Acquire) || slot.settlement.is_some() {
            return Err(NativeConversationError::ManagedAdmission);
        }
        // Registration is inert: no provider polling or caller callbacks occur.
        let (run, settlement) = owner
            .scheduler
            .register_run(&owner.resident, work_generation, turn)
            .map_err(|_| NativeConversationError::ManagedAdmission)?;
        let reference = run.reference();
        let cleanup = cohort.map(|cohort| ManagedRunCleanup {
            run: reference.clone(),
            cohort,
        });
        // Even a rejected post-publication registration can own admission I/O.
        // Preserve the exact run/settlement for the manager rather than refunding
        // actual cleanup on an error path.
        slot.run = Some(reference.clone());
        slot.settlement = Some(settlement);
        slot.cleanup.clone_from(&cleanup);
        let registration = (|| {
            let principal = owner
                .principal
                .begin_turn(turn, policy, preferences, Some(reference.clone()))
                .map_err(|_| NativeConversationError::ManagedAdmission)?;
            let mcp = owner
                .mcp
                .get()
                .map(|mcp| {
                    mcp.upgrade()
                        .ok_or(NativeConversationError::ManagedAdmission)?
                        .begin_turn(&principal)
                        .map_err(|_| NativeConversationError::ManagedAdmission)
                })
                .transpose()?;
            Ok::<_, NativeConversationError>((principal, mcp))
        })();
        let Ok((principal, mcp)) = registration else {
            drop(slot);
            run.finish();
            return Err(NativeConversationError::ManagedAdmission);
        };
        let acquisition = run.acquire();
        slot.run = Some(reference);
        drop(slot);
        Ok(ManagedConversationTurn {
            principal: Some(principal),
            mcp,
            acquisition: Some(acquisition),
            run: Some(run),
            cleanup,
            close_cleanup: false,
        })
    }
}

/// Owned beside the actual core turn. This is execution custody, not settlement.
pub(crate) struct ManagedConversationTurn {
    principal: Option<NativePrincipalTurn>,
    mcp: Option<NativePrincipalMcpTurn>,
    acquisition: Option<Acquire>,
    run: Option<RunLease>,
    cleanup: Option<ManagedRunCleanup>,
    close_cleanup: bool,
}

impl ManagedConversationTurn {
    pub(crate) fn activate_cleanup(&mut self) {
        self.close_cleanup = true;
    }
    pub(crate) fn cleanup(&self) -> Option<ManagedRunCleanup> {
        self.cleanup.clone()
    }
    /// Acquire only once before the first core poll. Later dependency waits own
    /// their fair reacquisition; polling them must not create another grant.
    pub(crate) fn poll_admission(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let Some(acquisition) = &mut self.acquisition else {
            return Poll::Ready(Ok(()));
        };
        match Pin::new(acquisition).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.acquisition.take();
                Poll::Ready(result.map_err(|_| NativeConversationError::ManagedAdmission))
            }
        }
    }

    pub(crate) fn finish_execution(&mut self) {
        self.mcp.take();
        self.principal.take();
        self.acquisition.take();
        if let Some(run) = self.run.take() {
            run.finish();
        }
    }
}

impl Drop for ManagedConversationTurn {
    fn drop(&mut self) {
        self.mcp.take();
        self.principal.take();
        self.acquisition.take();
        // RunLease drop cancels the actual turn. Never manufacture settlement.
        self.run.take();
        if let Some(cleanup) = self.cleanup.take().filter(|_| self.close_cleanup) {
            cleanup.cohort.close();
        }
    }
}

impl fmt::Debug for ManagedConversationOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedConversationOwner")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "conversation/tests.rs"]
mod tests;
