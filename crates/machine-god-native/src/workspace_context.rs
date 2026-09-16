//! Weak exact-turn routing for host-injected immutable workspace authority.

use std::fmt;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

use machine_god_core::{
    PermissionRequest, Session, SessionId, SessionIncarnationId, SessionWitness, ToolContext, Turn,
    TurnHandle, TurnId, TurnWitness,
};

use crate::{NativeWorkspaceAuthority, NativeWorkspaceScopeSnapshot};

/// Maximum concurrently registered workspace session incarnations.
pub const MAX_NATIVE_WORKSPACE_CONTEXT_SESSIONS: usize = 64;

/// Fixed, redacted workspace context errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeWorkspaceContextError {
    Unavailable,
    Limit,
}

impl fmt::Display for NativeWorkspaceContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "native workspace context is unavailable",
            Self::Limit => "native workspace context limit exceeded",
        })
    }
}
impl std::error::Error for NativeWorkspaceContextError {}
type Result<T> = std::result::Result<T, NativeWorkspaceContextError>;
type Routes = Mutex<Vec<Weak<WorkspaceContextSession>>>;

/// Bounded, weak exact-session routes, independent of permission composition.
/// Construction and lookup perform no filesystem operations.
#[derive(Default)]
pub struct NativeWorkspaceContexts {
    routes: Arc<Routes>,
}

impl NativeWorkspaceContexts {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn register(&self, session: &Session) -> Result<Arc<WorkspaceContextSession>> {
        self.register_with_publication(
            session,
            crate::conversation_routes::RoutePublication::default(),
        )
    }
    pub(crate) fn register_with_publication(
        &self,
        session: &Session,
        publication: crate::conversation_routes::RoutePublication,
    ) -> Result<Arc<WorkspaceContextSession>> {
        let mut routes = self
            .routes
            .lock()
            .map_err(|_| NativeWorkspaceContextError::Unavailable)?;
        routes.retain(|route| route.strong_count() != 0);
        let id = session.id();
        let incarnation = session.incarnation_id();
        if routes.len() >= MAX_NATIVE_WORKSPACE_CONTEXT_SESSIONS
            || routes.iter().filter_map(Weak::upgrade).any(|owner| {
                owner.id == id
                    && owner.incarnation == incarnation
                    && publication.conflicts_with(&owner.publication)
            })
        {
            return Err(NativeWorkspaceContextError::Limit);
        }
        let owner = Arc::new(WorkspaceContextSession {
            publication,
            id,
            incarnation,
            session: session.witness(),
            active: Mutex::new(None),
            retired: AtomicBool::new(false),
            routes: Arc::downgrade(&self.routes),
        });
        routes.push(Arc::downgrade(&owner));
        Ok(owner)
    }

    /// Reads the scope of a live exact native turn, including before permission
    /// authorization begins. Matching IDs is routing, not a permission grant.
    ///
    /// # Errors
    /// Rejects missing, foreign, cancelled, expired, or retired contexts.
    pub fn snapshot_for_tool(&self, context: &ToolContext) -> Result<NativeWorkspaceTurnScope> {
        self.snapshot(
            &context.session_id,
            &context.session_incarnation_id,
            &context.turn_id,
        )
    }

    /// Reads only the exact turn's pinned workspace scope, not mutable policy.
    ///
    /// # Errors
    /// Rejects missing, foreign, cancelled, expired, or retired contexts.
    pub fn snapshot_for_permission(
        &self,
        request: &PermissionRequest,
    ) -> Result<NativeWorkspaceTurnScope> {
        self.snapshot(
            &request.session_id,
            &request.session_incarnation_id,
            &request.turn_id,
        )
    }

    fn snapshot(
        &self,
        id: &SessionId,
        incarnation: &SessionIncarnationId,
        turn: &TurnId,
    ) -> Result<NativeWorkspaceTurnScope> {
        let owner = self
            .routes
            .lock()
            .map_err(|_| NativeWorkspaceContextError::Unavailable)?
            .iter()
            .filter_map(Weak::upgrade)
            .find(|owner| {
                owner.publication.is_active()
                    && &owner.id == id
                    && &owner.incarnation == incarnation
            })
            .ok_or(NativeWorkspaceContextError::Unavailable)?;
        let state = owner
            .active
            .lock()
            .map_err(|_| NativeWorkspaceContextError::Unavailable)?
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(NativeWorkspaceContextError::Unavailable)?;
        if state.handle.id() != turn || !state.live() {
            return Err(NativeWorkspaceContextError::Unavailable);
        }
        Ok(NativeWorkspaceTurnScope { state })
    }
}

impl fmt::Debug for NativeWorkspaceContexts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWorkspaceContexts")
            .finish_non_exhaustive()
    }
}

pub(crate) struct WorkspaceContextSession {
    publication: crate::conversation_routes::RoutePublication,
    id: SessionId,
    incarnation: SessionIncarnationId,
    session: SessionWitness,
    active: Mutex<Option<Weak<WorkspaceContextTurn>>>,
    retired: AtomicBool,
    routes: Weak<Routes>,
}

impl WorkspaceContextSession {
    pub(crate) fn ready_to_publish(self: &Arc<Self>) -> bool {
        self.routes.upgrade().is_some_and(|routes| {
            let Ok(routes) = routes.lock() else {
                return false;
            };
            routes
                .iter()
                .any(|route| route.ptr_eq(&Arc::downgrade(self)))
                && !routes.iter().filter_map(Weak::upgrade).any(|owner| {
                    !Arc::ptr_eq(&owner, self)
                        && owner.id == self.id
                        && owner.incarnation == self.incarnation
                })
        })
    }
    pub(crate) fn begin(
        self: &Arc<Self>,
        turn: &Turn,
        scope: NativeWorkspaceScopeSnapshot,
        undo: Option<Arc<crate::FileUndoTracker>>,
    ) -> Result<WorkspaceContextRegistration> {
        let witness = turn.witness();
        if turn.session_id() != &self.id
            || turn.session_incarnation_id() != &self.incarnation
            || !witness.is_live()
            || !self.session.owns_turn(&witness)
        {
            return Err(NativeWorkspaceContextError::Unavailable);
        }
        let mut active = self
            .active
            .lock()
            .map_err(|_| NativeWorkspaceContextError::Unavailable)?;
        if !self.publication.is_active()
            || self.retired.load(Ordering::Acquire)
            || active
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|turn| turn.live())
        {
            return Err(NativeWorkspaceContextError::Unavailable);
        }
        let state = Arc::new(WorkspaceContextTurn {
            owner: Arc::downgrade(self),
            handle: turn.handle(),
            witness,
            open: AtomicBool::new(true),
            scope,
            undo,
        });
        *active = Some(Arc::downgrade(&state));
        Ok(WorkspaceContextRegistration {
            owner: self.clone(),
            state,
        })
    }

    pub(crate) fn retire(self: &Arc<Self>) {
        self.publication.retire();
        self.retired.store(true, Ordering::Release);
        let active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .and_then(|turn| turn.upgrade());
        if let Some(turn) = active {
            turn.open.store(false, Ordering::Release);
        }
        if let Some(routes) = self.routes.upgrade() {
            routes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|route| !Weak::ptr_eq(route, &Arc::downgrade(self)));
        }
    }
}

struct WorkspaceContextTurn {
    owner: Weak<WorkspaceContextSession>,
    handle: TurnHandle,
    witness: TurnWitness,
    open: AtomicBool,
    scope: NativeWorkspaceScopeSnapshot,
    undo: Option<Arc<crate::FileUndoTracker>>,
}

impl WorkspaceContextTurn {
    fn live(&self) -> bool {
        self.open.load(Ordering::Acquire)
            && self.witness.is_live()
            && !self.handle.is_cancelled()
            && self.owner.upgrade().is_some_and(|owner| {
                owner.publication.is_active()
                    && owner.session.is_live()
                    && !owner.retired.load(Ordering::Acquire)
            })
    }
}

pub(crate) struct WorkspaceContextRegistration {
    owner: Arc<WorkspaceContextSession>,
    state: Arc<WorkspaceContextTurn>,
}

impl Drop for WorkspaceContextRegistration {
    fn drop(&mut self) {
        self.state.open.store(false, Ordering::Release);
        let mut active = self
            .owner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active
            .as_ref()
            .is_some_and(|route| Weak::ptr_eq(route, &Arc::downgrade(&self.state)))
        {
            active.take();
        }
    }
}

/// Immutable turn scope with a live-registration check, not policy authorization.
/// Retaining this value cannot keep the native turn or its admission alive.
pub struct NativeWorkspaceTurnScope {
    state: Arc<WorkspaceContextTurn>,
}

impl NativeWorkspaceTurnScope {
    pub(crate) fn undo_tracker(&self) -> Result<Option<Arc<crate::FileUndoTracker>>> {
        if !self.is_live() {
            return Err(NativeWorkspaceContextError::Unavailable);
        }
        Ok(self.state.undo.clone())
    }

    #[must_use]
    pub fn is_live(&self) -> bool {
        self.state.live()
    }

    /// Clones only the pinned snapshot; never reads the current manager or opens descriptors.
    ///
    /// # Errors
    /// Rejects an expired, cancelled, or retired native turn.
    pub fn snapshot(&self) -> Result<NativeWorkspaceScopeSnapshot> {
        if !self.is_live() {
            return Err(NativeWorkspaceContextError::Unavailable);
        }
        Ok(self.state.scope.clone())
    }
}

impl fmt::Debug for NativeWorkspaceTurnScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWorkspaceTurnScope")
            .field("live", &self.is_live())
            .finish_non_exhaustive()
    }
}

pub(crate) struct ConversationWorkspaceBinding {
    pub(crate) authority: NativeWorkspaceAuthority,
    pub(crate) owner: Arc<WorkspaceContextSession>,
}

pub(crate) enum WorkspaceAdmission {
    Current,
    Taken(Option<NativeWorkspaceScopeSnapshot>),
}

#[cfg(test)]
mod tests;
