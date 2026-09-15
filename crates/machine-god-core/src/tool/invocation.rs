//! Non-serializable execution identity; no host authority or owning host lease.

use crate::session::SessionState;
use crate::{CancellationToken, ToolContext, ToolName};
use serde_json::Value;
use std::fmt;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

/// Weak identity of an actual session allocation, not its durable public IDs.
#[derive(Clone)]
pub struct SessionWitness {
    session: Weak<SessionState>,
}
impl SessionWitness {
    pub(crate) fn new(session: &Arc<SessionState>) -> Self {
        Self {
            session: Arc::downgrade(session),
        }
    }
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.session.strong_count() != 0
    }
    #[must_use]
    pub fn same_session(&self, other: &Self) -> bool {
        self.session.ptr_eq(&other.session)
    }
    #[must_use]
    pub fn owns_turn(&self, turn: &TurnWitness) -> bool {
        turn.scope
            .upgrade()
            .is_some_and(|scope| scope.session.ptr_eq(&self.session))
    }
}
impl fmt::Debug for SessionWitness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionWitness")
            .field("live", &self.is_live())
            .finish_non_exhaustive()
    }
}

pub(crate) struct InvocationTurnScope {
    session: Weak<SessionState>,
    cancellation: CancellationToken,
    open: AtomicBool,
}

impl InvocationTurnScope {
    pub(crate) fn new(session: &Arc<SessionState>, cancellation: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            session: Arc::downgrade(session),
            cancellation,
            open: AtomicBool::new(true),
        })
    }
    pub(crate) fn close(&self) {
        self.open.store(false, Ordering::Release);
    }
    pub(crate) fn witness(self: &Arc<Self>) -> TurnWitness {
        TurnWitness {
            scope: Arc::downgrade(self),
        }
    }
}

/// Weak identity of one actual reserved turn. IDs, serialization and host
/// observations cannot construct a witness. It retains no runtime or session.
#[derive(Clone)]
pub struct TurnWitness {
    scope: Weak<InvocationTurnScope>,
}

impl TurnWitness {
    /// Whether the actual turn is still open and not cancelled.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.scope.upgrade().is_some_and(|scope| {
            scope.open.load(Ordering::Acquire)
                && !scope.cancellation.is_cancelled()
                && scope.session.strong_count() != 0
        })
    }
    /// Compares allocation identity, not public session/turn identifiers.
    #[must_use]
    pub fn same_turn(&self, other: &Self) -> bool {
        self.scope.ptr_eq(&other.scope)
    }
    pub(crate) fn belongs_to(&self, session: &Arc<SessionState>) -> bool {
        self.scope
            .upgrade()
            .is_some_and(|scope| scope.session.ptr_eq(&Arc::downgrade(session)))
    }
}
impl fmt::Debug for TurnWitness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TurnWitness")
            .field("live", &self.is_live())
            .finish_non_exhaustive()
    }
}

struct CallScope {
    turn: TurnWitness,
    claimed: AtomicBool,
}

/// An exact prepared invocation minted only by core's admitted execution path.
/// Its private weak call allocation is distinct even when provider IDs repeat.
/// Claims authenticate identity only; native hosts must additionally validate
/// principal generation, policy and resource authority before accepting work.
pub struct AdmittedToolInvocation {
    context: ToolContext,
    name: ToolName,
    arguments: Value,
    scope: Weak<CallScope>,
}

pub(crate) struct CallScopeOwner(Arc<CallScope>);

impl AdmittedToolInvocation {
    pub(crate) fn new(
        context: ToolContext,
        name: ToolName,
        arguments: Value,
        turn: TurnWitness,
    ) -> (Self, CallScopeOwner) {
        let scope = Arc::new(CallScope {
            turn,
            claimed: AtomicBool::new(false),
        });
        (
            Self {
                context,
                name,
                arguments,
                scope: Arc::downgrade(&scope),
            },
            CallScopeOwner(scope),
        )
    }
    #[must_use]
    pub const fn context(&self) -> &ToolContext {
        &self.context
    }
    #[must_use]
    pub const fn tool_name(&self) -> &ToolName {
        &self.name
    }
    #[must_use]
    pub const fn arguments(&self) -> &Value {
        &self.arguments
    }
    /// Consumes the one-shot claim only for this exact live turn. A failed
    /// foreign/stale check does not consume the legitimate caller's claim.
    #[must_use]
    pub fn claim(&self, turn: &TurnWitness) -> bool {
        self.scope.upgrade().is_some_and(|scope| {
            scope.turn.same_turn(turn)
                && scope.turn.is_live()
                && scope
                    .claimed
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
        })
    }
    /// Discards authentication when adapting to an ordinary structural tool.
    #[must_use]
    pub fn into_parts(self) -> (ToolContext, Value) {
        (self.context, self.arguments)
    }
}

impl fmt::Debug for AdmittedToolInvocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmittedToolInvocation")
            .finish_non_exhaustive()
    }
}

impl Drop for CallScopeOwner {
    fn drop(&mut self) {
        // Explicitly touch the owner: its allocation, not IDs, defines liveness.
        self.0.claimed.store(true, Ordering::Release);
    }
}
