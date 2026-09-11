//! Weak exact-conversation routing; lookup data cannot register or authorize work.

use super::submission::McpSubmissionRegistry;
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
use super::submission::McpSubmissionTurnRegistration;
use machine_god_core::{
    BoxFuture, PermissionRequest, SessionId, SessionIncarnationId, ToolContext, TurnId,
};
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
use machine_god_core::{Session, Turn};
use std::{
    fmt,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

/// Maximum concurrently registered exact session incarnations.
pub const MAX_NATIVE_MCP_CONTEXT_SESSIONS: usize = 64;
type Routes = Mutex<Vec<Weak<McpContextSession>>>;
type Result<T> = std::result::Result<T, NativeMcpContextError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpContextError {
    Unavailable,
    Duplicate,
    Limit,
}
impl fmt::Display for NativeMcpContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "native MCP turn context unavailable",
            Self::Duplicate => "native MCP context already registered",
            Self::Limit => "native MCP context capacity exceeded",
        })
    }
}
impl std::error::Error for NativeMcpContextError {}

/// An engine may retain this router: all reverse routes are weak, never Sessions.
#[derive(Default)]
pub struct NativeMcpContexts {
    routes: Arc<Routes>,
}
impl NativeMcpContexts {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(any(test, target_os = "linux", target_os = "macos"))]
    pub(crate) fn register(&self, session: &Session) -> Result<Arc<McpContextSession>> {
        let id = session.id();
        let incarnation = session.incarnation_id();
        let mut routes = self
            .routes
            .lock()
            .map_err(|_| NativeMcpContextError::Unavailable)?;
        routes.retain(|route| route.strong_count() != 0);
        // Upgrade outside iterator destruction while locked: route owners have
        // reentrant retirement Drop. Retain all upgrades until after unlocking.
        let owners: Vec<_> = routes.iter().filter_map(Weak::upgrade).collect();
        let result = if owners
            .iter()
            .any(|owner| owner.id == id && owner.incarnation == incarnation)
        {
            Err(NativeMcpContextError::Duplicate)
        } else if routes.len() >= MAX_NATIVE_MCP_CONTEXT_SESSIONS {
            Err(NativeMcpContextError::Limit)
        } else {
            let owner = Arc::new(McpContextSession {
                id,
                incarnation,
                active: Mutex::new(Active::default()),
                retired: AtomicBool::new(false),
                routes: Arc::downgrade(&self.routes),
            });
            routes.push(Arc::downgrade(&owner));
            Ok(owner)
        };
        drop(routes);
        drop(owners);
        result
    }

    /// # Errors
    /// Rejects missing, foreign, cancelled or retired exact turns. Call IDs are
    /// not authority; the returned registry still validates exact preparation.
    pub fn snapshot_for_tool(&self, context: &ToolContext) -> Result<NativeMcpTurnContext> {
        self.snapshot(
            &context.session_id,
            &context.session_incarnation_id,
            &context.turn_id,
        )
    }
    /// # Errors
    /// Rejects missing/foreign/retired turn routes; this does not authorize the request.
    pub fn snapshot_for_permission(
        &self,
        request: &PermissionRequest,
    ) -> Result<NativeMcpTurnContext> {
        self.snapshot(
            &request.session_id,
            &request.session_incarnation_id,
            &request.turn_id,
        )
    }

    /// Retires only the exact turn, including a cancelled turn whose public
    /// snapshot is already unavailable. Its registration remains exclusive
    /// until the actual owner drops it. No registry destructor runs under a
    /// router/session lock.
    #[cfg(any(test, target_os = "linux", target_os = "macos"))]
    pub(crate) fn close_turn(
        &self,
        id: &SessionId,
        incarnation: &SessionIncarnationId,
        turn: &TurnId,
    ) {
        let owners: Vec<_> = self
            .routes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        let state = owners
            .iter()
            .find(|owner| &owner.id == id && &owner.incarnation == incarnation)
            .and_then(|owner| {
                owner
                    .active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .route
                    .as_ref()
                    .and_then(Weak::upgrade)
            });
        if let Some(state) = state.filter(|state| &state.id == turn) {
            state.registry.retire();
        }
    }
    fn snapshot(
        &self,
        id: &SessionId,
        incarnation: &SessionIncarnationId,
        turn: &TurnId,
    ) -> Result<NativeMcpTurnContext> {
        let owners: Vec<_> = self
            .routes
            .lock()
            .map_err(|_| NativeMcpContextError::Unavailable)?
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        let owner = owners
            .iter()
            .find(|owner| &owner.id == id && &owner.incarnation == incarnation)
            .ok_or(NativeMcpContextError::Unavailable)?;
        let state = owner
            .active
            .lock()
            .map_err(|_| NativeMcpContextError::Unavailable)?
            .route
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(NativeMcpContextError::Unavailable)?;
        if &state.id != turn {
            return Err(NativeMcpContextError::Unavailable);
        }
        let snapshot = NativeMcpTurnContext { state };
        snapshot.revalidate()?;
        Ok(snapshot)
    }
}
impl Drop for NativeMcpContexts {
    fn drop(&mut self) {
        let routes = std::mem::take(
            &mut *self
                .routes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for owner in routes.into_iter().filter_map(|route| route.upgrade()) {
            owner.retire();
        }
    }
}

#[derive(Default)]
struct Active {
    route: Option<Weak<McpContextTurn>>,
    #[cfg(any(test, target_os = "linux", target_os = "macos"))]
    last: Option<TurnId>,
}

pub(crate) struct McpContextSession {
    id: SessionId,
    incarnation: SessionIncarnationId,
    active: Mutex<Active>,
    retired: AtomicBool,
    routes: Weak<Routes>,
}
impl McpContextSession {
    #[cfg(any(test, target_os = "linux", target_os = "macos"))]
    pub(crate) fn begin(
        self: &Arc<Self>,
        session: &Session,
        turn: &Turn,
    ) -> Result<McpContextRegistration> {
        if session.id() != self.id || session.incarnation_id() != self.incarnation {
            return Err(NativeMcpContextError::Unavailable);
        }
        let (registry, registration) = McpSubmissionRegistry::register_turn(session, turn)
            .map_err(|_| NativeMcpContextError::Unavailable)?;
        let mut active = self
            .active
            .lock()
            .map_err(|_| NativeMcpContextError::Unavailable)?;
        if self.retired.load(Ordering::Acquire) || self.routes.strong_count() == 0 {
            return Err(NativeMcpContextError::Unavailable);
        }
        // A retained registration remains exclusive even when its core turn has
        // been cancelled. Never replace it based on cancellation alone.
        if active
            .route
            .as_ref()
            .is_some_and(|route| route.strong_count() != 0)
            || active.last.as_ref() == Some(turn.handle().id())
        {
            return Err(NativeMcpContextError::Duplicate);
        }
        let state = Arc::new(McpContextTurn {
            id: turn.handle().id().clone(),
            owner: Arc::downgrade(self),
            registry,
        });
        active.last = Some(state.id.clone());
        active.route = Some(Arc::downgrade(&state));
        Ok(McpContextRegistration {
            owner: self.clone(),
            state,
            registration: Some(registration),
        })
    }

    pub(crate) fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        let state = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .route
            .take()
            .and_then(|route| route.upgrade());
        if let Some(routes) = self.routes.upgrade() {
            routes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|route| !std::ptr::eq(route.as_ptr(), self));
        }
        if let Some(state) = state {
            state.registry.retire();
        }
    }
}
impl Drop for McpContextSession {
    fn drop(&mut self) {
        self.retire();
    }
}

struct McpContextTurn {
    id: TurnId,
    owner: Weak<McpContextSession>,
    registry: Arc<McpSubmissionRegistry>,
}
impl McpContextTurn {
    fn live(&self) -> bool {
        let Some(owner) = self.owner.upgrade() else {
            return false;
        };
        if owner.retired.load(Ordering::Acquire) || owner.routes.strong_count() == 0 {
            return false;
        }
        let active = owner
            .active
            .lock()
            .map(|active| {
                active
                    .route
                    .as_ref()
                    .is_some_and(|route| std::ptr::eq(route.as_ptr(), self))
            })
            .unwrap_or(false);
        active && self.registry.revalidate().is_ok()
    }
}

/// Native turn owner; snapshots retain no copy of this registration.
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
pub(crate) struct McpContextRegistration {
    owner: Arc<McpContextSession>,
    state: Arc<McpContextTurn>,
    registration: Option<McpSubmissionTurnRegistration>,
}
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
impl Drop for McpContextRegistration {
    fn drop(&mut self) {
        {
            let mut active = self
                .owner
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if active
                .route
                .as_ref()
                .is_some_and(|route| Weak::ptr_eq(route, &Arc::downgrade(&self.state)))
            {
                active.route.take();
            }
        }
        // Cancels and destroys arbitrary proof/waker state outside route locks.
        drop(self.registration.take());
    }
}

/// Exact live registry snapshot. Retention cannot keep a conversation or engine alive.
pub struct NativeMcpTurnContext {
    state: Arc<McpContextTurn>,
}
impl NativeMcpTurnContext {
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.state.live()
    }
    /// # Errors
    /// Rejects retired/cancelled/unpublished turns; lookup is not an execution grant.
    pub fn revalidate(&self) -> Result<()> {
        if self.is_live() {
            Ok(())
        } else {
            Err(NativeMcpContextError::Unavailable)
        }
    }
    /// # Errors
    /// Rejects stale snapshots. The registry revalidates again before each effect.
    pub fn registry(&self) -> Result<Arc<McpSubmissionRegistry>> {
        self.revalidate()?;
        Ok(self.state.registry.clone())
    }
    /// Observes registry retirement or actual core-turn cancellation without driving work.
    #[must_use]
    pub fn cancelled(&self) -> BoxFuture<'static, ()> {
        Box::pin(self.state.registry.cancelled_owned())
    }
}

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(concat!(stringify!($ty), " { <redacted> }")) }
        }
    )+};
}
redacted_debug!(NativeMcpContexts, NativeMcpTurnContext);

#[cfg(test)]
mod tests;
