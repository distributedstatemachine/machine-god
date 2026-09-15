//! Bridge actual conversation turns to principal and shared scheduler admission.

use super::principal::{NativePrincipal, NativePrincipalRegistry, NativePrincipalTurn};
use super::scheduler::{Acquire, ManagedScheduler, ResidentLease, RunLease, RunRef, RunSettlement};
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
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

type Result<T> = std::result::Result<T, NativeConversationError>;

struct Owner {
    principal: Arc<NativePrincipal>,
    scheduler: ManagedScheduler,
    resident: ResidentLease,
    active: Mutex<Active>,
    closed: AtomicBool,
}

#[derive(Default)]
struct Active {
    run: Option<RunRef>,
    settlement: Option<RunSettlement>,
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
        })))
    }

    pub(crate) fn principal(&self) -> &Arc<NativePrincipal> {
        &self.0.principal
    }

    pub(crate) fn binding(&self) -> ManagedConversationBinding {
        ManagedConversationBinding(Arc::downgrade(&self.0))
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
}

impl Drop for ManagedConversationOwner {
    fn drop(&mut self) {
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
        let principal =
            match owner
                .principal
                .begin_turn(turn, policy, preferences, Some(reference.clone()))
            {
                Ok(principal) => principal,
                Err(_) => {
                    drop(slot);
                    // No core/provider poll has occurred, so this rejected reservation
                    // has no native execution or cleanup obligation to transfer.
                    run.finish();
                    settlement
                        .complete()
                        .map_err(|_| NativeConversationError::ManagedAdmission)?;
                    return Err(NativeConversationError::ManagedAdmission);
                }
            };
        let acquisition = run.acquire();
        slot.run = Some(reference);
        slot.settlement = Some(settlement);
        drop(slot);
        Ok(ManagedConversationTurn {
            principal: Some(principal),
            acquisition: Some(acquisition),
            run: Some(run),
        })
    }
}

/// Owned beside the actual core turn. This is execution custody, not settlement.
pub(crate) struct ManagedConversationTurn {
    principal: Option<NativePrincipalTurn>,
    acquisition: Option<Acquire>,
    run: Option<RunLease>,
}

impl ManagedConversationTurn {
    pub(crate) fn principal_turn(&self) -> Option<&NativePrincipalTurn> {
        self.principal.as_ref()
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
        self.principal.take();
        self.acquisition.take();
        if let Some(run) = self.run.take() {
            run.finish();
        }
    }
}

impl Drop for ManagedConversationTurn {
    fn drop(&mut self) {
        self.principal.take();
        self.acquisition.take();
        // RunLease drop cancels the actual turn. Never manufacture settlement.
        self.run.take();
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
