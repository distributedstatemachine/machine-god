//! Weak allocation-authenticated routes; no owning engine or runtime edges.

use super::scheduler::RunRef;
use crate::conversation_routes::RoutePublication;
use crate::file_undo::{FileUndoTracker, NativeUndoBudget};
use crate::{
    NativeModelPreferences, NativePermissionPolicySnapshot, NativeWorkspaceAuthority,
    NativeWorkspaceScopeSnapshot,
};
use machine_god_core::{
    AdmittedToolInvocation, BackgroundOutputOwner, ManagedSubagentInvocation, Session, SessionId,
    SessionIncarnationId, SessionWitness, ToolContext, Turn, TurnId, TurnWitness,
};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

const MAX_RESIDENT: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrincipalError {
    Unavailable,
    Limit,
    Stale,
}
type Result<T> = std::result::Result<T, PrincipalError>;

struct Registry {
    routes: Mutex<Vec<Route>>,
    budget: Arc<NativeUndoBudget>,
    limit: usize,
}
struct Route {
    publication: RoutePublication,
    principal: Weak<NativePrincipal>,
    session: SessionWitness,
    retired: Arc<AtomicBool>,
}

pub(crate) struct NativePrincipalRegistry(Arc<Registry>);
impl NativePrincipalRegistry {
    pub(crate) fn new(limit: usize, budget: Arc<NativeUndoBudget>) -> Result<Self> {
        if !(1..=MAX_RESIDENT).contains(&limit) {
            return Err(PrincipalError::Limit);
        }
        Ok(Self(Arc::new(Registry {
            routes: Mutex::new(Vec::new()),
            budget,
            limit,
        })))
    }

    pub(crate) fn requester(&self) -> NativePrincipalRequester {
        NativePrincipalRequester(Arc::downgrade(&self.0))
    }

    #[cfg(test)]
    pub(crate) fn register(
        &self,
        session: &Session,
        generation: u64,
        workspace: &NativeWorkspaceAuthority,
    ) -> Result<Arc<NativePrincipal>> {
        self.register_with_publication(session, generation, workspace, RoutePublication::default())
    }

    pub(crate) fn register_with_publication(
        &self,
        session: &Session,
        generation: u64,
        workspace: &NativeWorkspaceAuthority,
        publication: RoutePublication,
    ) -> Result<Arc<NativePrincipal>> {
        if generation == 0 {
            return Err(PrincipalError::Stale);
        }
        let witness = session.witness();
        let owner = BackgroundOutputOwner::new(session.id(), session.incarnation_id());
        // Fork immutable descriptor selections outside the registry lock.
        let workspace = workspace
            .fork_selection()
            .map_err(|_| PrincipalError::Unavailable)?;
        let undo = Arc::new(
            FileUndoTracker::for_principal(self.0.budget.clone(), owner.clone(), generation)
                .map_err(|_| PrincipalError::Unavailable)?,
        );
        let principal = Arc::new(NativePrincipal {
            publication: publication.clone(),
            owner,
            generation,
            session: witness.clone(),
            retired: Arc::new(AtomicBool::new(false)),
            active: Mutex::new(None),
            registry: Arc::downgrade(&self.0),
            workspace,
            undo,
        });
        let mut routes = self
            .0
            .routes
            .lock()
            .map_err(|_| PrincipalError::Unavailable)?;
        routes.retain(|route| {
            route.principal.strong_count() != 0
                && route.session.is_live()
                && !route.retired.load(Ordering::Acquire)
        });
        if routes.len() >= self.0.limit {
            return Err(PrincipalError::Limit);
        }
        if routes.iter().any(|route| {
            route.session.same_session(&witness) && publication.conflicts_with(&route.publication)
        }) {
            return Err(PrincipalError::Stale);
        }
        routes.push(Route {
            publication,
            principal: Arc::downgrade(&principal),
            session: witness,
            retired: principal.retired.clone(),
        });
        Ok(principal)
    }

    #[cfg(test)]
    pub(crate) fn claim(
        &self,
        invocation: &ManagedSubagentInvocation,
    ) -> Result<NativeManagedCallLease> {
        self.0
            .claim(invocation.context(), |turn| invocation.claim(turn))
    }
}

impl Registry {
    fn stamp(&self, context: &ToolContext) -> Result<NativePrincipalTurnStamp> {
        self.stamp_for_turn(
            &context.session_id,
            &context.session_incarnation_id,
            &context.turn_id,
        )
    }
    fn stamp_for_turn(
        &self,
        session: &SessionId,
        incarnation: &SessionIncarnationId,
        turn: &TurnId,
    ) -> Result<NativePrincipalTurnStamp> {
        let candidates: Vec<_> = self
            .routes
            .lock()
            .map_err(|_| PrincipalError::Unavailable)?
            .iter()
            .filter_map(|route| route.principal.upgrade())
            .collect();
        let mut selected = None;
        for principal in candidates {
            if principal.owner.session_id() != session
                || principal.owner.session_incarnation_id() != incarnation
            {
                continue;
            }
            let state = principal
                .active
                .lock()
                .map_err(|_| PrincipalError::Unavailable)?
                .as_ref()
                .and_then(Weak::upgrade);
            let Some(state) = state.filter(|state| &state.turn_id == turn && state.live()) else {
                continue;
            };
            if selected.is_some() {
                return Err(PrincipalError::Stale);
            }
            selected = Some(NativePrincipalTurnStamp {
                state: Arc::downgrade(&state),
                generation: principal.generation,
            });
        }
        selected.ok_or(PrincipalError::Stale)
    }
    fn claim(
        &self,
        context: &ToolContext,
        claim: impl Fn(&TurnWitness) -> bool,
    ) -> Result<NativeManagedCallLease> {
        // Candidate ownership drops only after the global lock is released.
        let candidates: Vec<_> = self
            .routes
            .lock()
            .map_err(|_| PrincipalError::Unavailable)?
            .iter()
            .filter_map(|route| route.principal.upgrade())
            .collect();
        for principal in candidates {
            if principal.owner.session_id() != &context.session_id
                || principal.owner.session_incarnation_id() != &context.session_incarnation_id
            {
                continue;
            }
            let active = principal
                .active
                .lock()
                .map_err(|_| PrincipalError::Unavailable)?;
            let Some(state) = active.as_ref().and_then(Weak::upgrade) else {
                continue;
            };
            if state.turn_id != context.turn_id
                || !state.live()
                || !principal.session.owns_turn(&state.witness)
                || state.run.as_ref().is_some_and(|run| {
                    !run.matches_turn(&state.witness)
                        || !run.is_executing()
                        || run.work_generation() != state.work_generation
                })
                || !claim(&state.witness)
            {
                continue;
            }
            return Ok(NativeManagedCallLease { state });
        }
        Err(PrincipalError::Stale)
    }
}

/// Reverse tool edge: does not retain registry, principal or runtime ownership.
#[derive(Clone)]
pub(crate) struct NativePrincipalRequester(Weak<Registry>);
impl NativePrincipalRequester {
    pub(crate) fn stamp_for_turn(
        &self,
        session: &SessionId,
        incarnation: &SessionIncarnationId,
        turn: &TurnId,
    ) -> Result<NativePrincipalTurnStamp> {
        self.0
            .upgrade()
            .ok_or(PrincipalError::Unavailable)?
            .stamp_for_turn(session, incarnation, turn)
    }
    pub(crate) fn stamp(&self, context: &ToolContext) -> Result<NativePrincipalTurnStamp> {
        self.0
            .upgrade()
            .ok_or(PrincipalError::Unavailable)?
            .stamp(context)
    }
    pub(crate) fn claim(
        &self,
        invocation: &ManagedSubagentInvocation,
    ) -> Result<NativeManagedCallLease> {
        self.0
            .upgrade()
            .ok_or(PrincipalError::Unavailable)?
            .claim(invocation.context(), |turn| invocation.claim(turn))
    }
    pub(crate) fn claim_tool(
        &self,
        invocation: &AdmittedToolInvocation,
    ) -> Result<NativeManagedCallLease> {
        self.0
            .upgrade()
            .ok_or(PrincipalError::Unavailable)?
            .claim(invocation.context(), |turn| invocation.claim(turn))
    }
}

pub(crate) struct NativePrincipal {
    publication: RoutePublication,
    owner: BackgroundOutputOwner,
    generation: u64,
    session: SessionWitness,
    retired: Arc<AtomicBool>,
    active: Mutex<Option<Weak<TurnState>>>,
    registry: Weak<Registry>,
    workspace: NativeWorkspaceAuthority,
    undo: Arc<FileUndoTracker>,
}
impl NativePrincipal {
    /// Composition may bind resources to a reserved principal, but only an
    /// active publication can register an executable turn.
    pub(crate) fn is_live(&self) -> bool {
        self.live()
    }
    pub(crate) fn ready_to_publish(&self) -> bool {
        let Some(registry) = self.registry.upgrade() else {
            return false;
        };
        let Ok(routes) = registry.routes.lock() else {
            return false;
        };
        self.live()
            && routes
                .iter()
                .any(|route| Arc::ptr_eq(&route.retired, &self.retired))
            && !routes.iter().any(|route| {
                !Arc::ptr_eq(&route.retired, &self.retired)
                    && route.session.same_session(&self.session)
            })
    }
    pub(crate) fn owner(&self) -> &BackgroundOutputOwner {
        &self.owner
    }
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) fn workspace(&self) -> &NativeWorkspaceAuthority {
        &self.workspace
    }
    pub(crate) fn undo(&self) -> &Arc<FileUndoTracker> {
        &self.undo
    }
    #[cfg(test)]
    pub(crate) fn requester(self: &Arc<Self>) -> NativePrincipalWeak {
        NativePrincipalWeak(Arc::downgrade(self))
    }
    fn live(&self) -> bool {
        self.registry.strong_count() != 0
            && self.session.is_live()
            && !self.retired.load(Ordering::Acquire)
            && (self.publication.is_staged() || self.publication.is_active())
    }
    pub(crate) fn begin_turn(
        self: &Arc<Self>,
        turn: &Turn,
        policy: NativePermissionPolicySnapshot,
        preferences: NativeModelPreferences,
        run: Option<RunRef>,
    ) -> Result<NativePrincipalTurn> {
        let witness = turn.witness();
        if !self.live()
            || !self.publication.is_active()
            || !witness.is_live()
            || !self.session.owns_turn(&witness)
            || run.as_ref().is_some_and(|run| !run.matches_turn(&witness))
        {
            return Err(PrincipalError::Stale);
        }
        let workspace = self
            .workspace
            .snapshot()
            .map_err(|_| PrincipalError::Unavailable)?;
        let work_generation = run
            .as_ref()
            .map(|run| run.work_generation().ok_or(PrincipalError::Stale))
            .transpose()?;
        let mut active = self
            .active
            .lock()
            .map_err(|_| PrincipalError::Unavailable)?;
        if !self.live()
            || !self.publication.is_active()
            || active
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|turn| turn.live())
        {
            return Err(PrincipalError::Stale);
        }
        let state = Arc::new(TurnState {
            principal: self.clone(),
            witness,
            turn_id: turn.id().clone(),
            open: AtomicBool::new(true),
            policy,
            preferences,
            run,
            work_generation,
            workspace,
        });
        *active = Some(Arc::downgrade(&state));
        Ok(NativePrincipalTurn { state })
    }

    pub(crate) fn retire(&self) {
        // Stop model/turn authority, not the conversation's lifecycle barrier.
        // The outer owner may still need the original metadata-only admission
        // to acknowledge and clear notice custody before exact route retirement.
        {
            let mut active = self
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.retired.store(true, Ordering::Release);
            *active = None;
        }
        if let Some(registry) = self.registry.upgrade() {
            registry
                .routes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|route| !Arc::ptr_eq(&route.retired, &self.retired));
        }
    }
}

/// Weak UI/native reverse route. Public IDs alone cannot upgrade it.
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct NativePrincipalWeak(Weak<NativePrincipal>);
#[cfg(test)]
impl NativePrincipalWeak {
    pub(crate) fn upgrade(
        &self,
        session: &SessionWitness,
        generation: u64,
    ) -> Result<Arc<NativePrincipal>> {
        self.0
            .upgrade()
            .filter(|principal| {
                principal.live()
                    && principal.generation == generation
                    && principal.session.same_session(session)
            })
            .ok_or(PrincipalError::Stale)
    }
}

struct TurnState {
    principal: Arc<NativePrincipal>,
    witness: TurnWitness,
    turn_id: machine_god_core::TurnId,
    open: AtomicBool,
    policy: NativePermissionPolicySnapshot,
    preferences: NativeModelPreferences,
    run: Option<RunRef>,
    work_generation: Option<std::num::NonZeroU64>,
    workspace: NativeWorkspaceScopeSnapshot,
}
impl TurnState {
    fn live(&self) -> bool {
        self.open.load(Ordering::Acquire)
            && self.witness.is_live()
            && self.principal.live()
            && self.principal.publication.is_active()
            && self
                .principal
                .undo
                .check_principal(&self.principal.owner, self.principal.generation)
                .is_ok()
    }
}

/// Owning admission registration; retain beside the actual turn, not its IDs.
pub(crate) struct NativePrincipalTurn {
    state: Arc<TurnState>,
}
impl NativePrincipalTurn {
    pub(crate) fn turn_id(&self) -> &TurnId {
        &self.state.turn_id
    }
    pub(crate) fn stamp(&self) -> NativePrincipalTurnStamp {
        NativePrincipalTurnStamp {
            state: Arc::downgrade(&self.state),
            generation: self.state.principal.generation,
        }
    }
    #[cfg(test)]
    pub(crate) fn witness(&self) -> &TurnWitness {
        &self.state.witness
    }
}
impl Drop for NativePrincipalTurn {
    fn drop(&mut self) {
        let mut active = self
            .state
            .principal
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.state.open.store(false, Ordering::Release);
        if active
            .as_ref()
            .is_some_and(|active| active.ptr_eq(&Arc::downgrade(&self.state)))
        {
            *active = None;
        }
    }
}

/// Privately minted, nonclone call admission. Resource custody may outlive
/// retirement; it never restores authority after the turn/registry retires.
pub(crate) struct NativeManagedCallLease {
    state: Arc<TurnState>,
}

/// Non-consuming weak preparation/routing identity, never execution authority.
#[derive(Clone)]
pub(crate) struct NativePrincipalTurnStamp {
    state: Weak<TurnState>,
    generation: u64,
}
impl NativePrincipalTurnStamp {
    /// Execution consent keeps no call lease alive, but observes the same run
    /// generation/executing restriction as the original owned model lease.
    pub(crate) fn execution_is_live(&self) -> bool {
        self.is_live()
            && self.state.upgrade().is_some_and(|state| {
                state.run.as_ref().is_none_or(|run| {
                    run.matches_turn(&state.witness)
                        && run.is_executing()
                        && run.work_generation() == state.work_generation
                })
            })
    }

    pub(crate) fn is_live(&self) -> bool {
        self.state.upgrade().is_some_and(|state| {
            state.live()
                && state.principal.generation == self.generation
                && state.principal.session.owns_turn(&state.witness)
                && state.principal.active.lock().is_ok_and(|active| {
                    active
                        .as_ref()
                        .is_some_and(|active| active.ptr_eq(&self.state))
                })
        })
    }
    pub(crate) fn matches_principal(&self, principal: &Arc<NativePrincipal>) -> bool {
        self.is_live()
            && self.state.upgrade().is_some_and(|state| {
                Arc::ptr_eq(&state.principal, principal) && self.generation == principal.generation
            })
    }
    pub(crate) fn matches_turn(&self, witness: &TurnWitness) -> bool {
        self.is_live()
            && self
                .state
                .upgrade()
                .is_some_and(|state| state.witness.same_turn(witness))
    }
    pub(crate) fn same_turn(&self, other: &Self) -> bool {
        self.generation == other.generation && self.state.ptr_eq(&other.state)
    }
}
impl NativeManagedCallLease {
    pub(crate) fn consent_stamp(&self) -> NativePrincipalTurnStamp {
        NativePrincipalTurnStamp {
            state: Arc::downgrade(&self.state),
            generation: self.state.principal.generation,
        }
    }
    pub(crate) fn is_live(&self) -> bool {
        self.state.live()
            && self.state.run.as_ref().is_none_or(|run| {
                run.matches_turn(&self.state.witness)
                    && run.is_executing()
                    && run.work_generation() == self.state.work_generation
            })
    }
    pub(crate) fn principal(&self) -> &Arc<NativePrincipal> {
        &self.state.principal
    }
    pub(crate) fn policy(&self) -> &NativePermissionPolicySnapshot {
        &self.state.policy
    }
    pub(crate) fn run(&self) -> Option<&RunRef> {
        self.state.run.as_ref()
    }
    pub(crate) fn witness(&self) -> &TurnWitness {
        &self.state.witness
    }
    pub(crate) fn workspace(&self) -> &NativeWorkspaceScopeSnapshot {
        &self.state.workspace
    }
    pub(crate) fn preferences(&self) -> &NativeModelPreferences {
        &self.state.preferences
    }
}

#[cfg(test)]
#[path = "principal/tests.rs"]
mod tests;

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_debug!(
    NativePrincipalRegistry,
    NativePrincipalRequester,
    NativePrincipal,
    NativePrincipalTurn,
    NativePrincipalTurnStamp,
    NativeManagedCallLease
);
#[cfg(test)]
redacted_debug!(NativePrincipalWeak);
