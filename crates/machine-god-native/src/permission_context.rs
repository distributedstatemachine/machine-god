//! Exact native admission provenance and weak automatic-review context routing.

mod provenance;
mod text;

use crate::{NativeAutoPermissionRootContext, NativeModelSnapshot, NativePermissionPolicySnapshot};
use machine_god_core::{
    Message, PermissionInvocationSnapshot, PermissionRequest, Session, ToolCallId, Turn, TurnHandle,
};
use std::fmt;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

pub const NATIVE_PERMISSION_CONTEXT_KEY: &str = "machine_god.permission_root_provenance";
pub const MAX_NATIVE_PERMISSION_CONTEXT_SESSIONS: usize = 64;
pub const MAX_NATIVE_PERMISSION_ROOT_REQUEST_BYTES: usize = crate::MAX_NATIVE_QUEUED_PROMPT_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativePermissionContextError {
    InvalidProvenance,
    Unavailable,
    Limit,
}
impl fmt::Display for NativePermissionContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidProvenance => "native permission provenance is invalid",
            Self::Unavailable => "native permission review context is unavailable",
            Self::Limit => "native permission context limit exceeded",
        })
    }
}
impl std::error::Error for NativePermissionContextError {}
type Error = NativePermissionContextError;

/// At most 64 weak exact-session routes; this does not own sessions or start work.
#[derive(Default)]
pub struct NativePermissionContexts {
    routes: Mutex<Vec<Weak<ContextSession>>>,
}
impl NativePermissionContexts {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub(crate) fn register(&self, session: &Session) -> Result<Arc<ContextSession>, Error> {
        provenance::validate(&session.record_snapshot())?;
        let mut routes = self
            .routes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        routes.retain(|route| route.strong_count() != 0);
        if routes.len() == MAX_NATIVE_PERMISSION_CONTEXT_SESSIONS
            || routes.iter().filter_map(Weak::upgrade).any(|owner| {
                owner.session.id() == session.id()
                    && owner.session.incarnation_id() == session.incarnation_id()
            })
        {
            return Err(Error::Limit);
        }
        let owner = Arc::new(ContextSession {
            session: session.clone(),
            active: Mutex::new(None),
        });
        routes.push(Arc::downgrade(&owner));
        Ok(owner)
    }
    /// Reads the exact currently authorizing invocation. The immutable result
    /// shares only that pending call, not its canonical transcript or siblings.
    /// # Errors
    /// Rejects missing/closed/foreign routes or requests outside authorization.
    pub fn snapshot(
        &self,
        request: &PermissionRequest,
    ) -> Result<NativePermissionReviewContext, Error> {
        let owner = self
            .routes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(Weak::upgrade)
            .find(|owner| {
                owner.session.id() == request.session_id
                    && owner.session.incarnation_id() == request.session_incarnation_id
            })
            .ok_or(Error::Unavailable)?;
        let turn = owner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(Error::Unavailable)?;
        if turn.handle.id() != &request.turn_id || !turn.live() {
            return Err(Error::Unavailable);
        }
        let source = owner
            .session
            .permission_invocation_snapshot(request)
            .ok_or(Error::Unavailable)?;
        let context = NativePermissionReviewContext { turn, source };
        context
            .is_live()
            .then_some(context)
            .ok_or(Error::Unavailable)
    }
}
impl fmt::Debug for NativePermissionContexts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativePermissionContexts")
            .finish_non_exhaustive()
    }
}

pub(crate) struct ContextSession {
    session: Session,
    active: Mutex<Option<Weak<ContextTurn>>>,
}
impl ContextSession {
    pub(crate) fn begin(
        self: &Arc<Self>,
        turn: &Turn,
        root: Option<String>,
        model: Option<NativeModelSnapshot>,
        source_model: Option<String>,
        policy: Option<NativePermissionPolicySnapshot>,
    ) -> Result<ContextRegistration, Error> {
        if turn.session_id() != &self.session.id()
            || turn.session_incarnation_id() != &self.session.incarnation_id()
        {
            return Err(Error::Unavailable);
        }
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|turn| turn.live())
        {
            return Err(Error::Unavailable);
        }
        let state = Arc::new(ContextTurn {
            handle: turn.handle(),
            open: AtomicBool::new(true),
            root,
            model,
            source_model,
            policy,
        });
        *active = Some(Arc::downgrade(&state));
        Ok(ContextRegistration {
            owner: self.clone(),
            state,
        })
    }
}
struct ContextTurn {
    handle: TurnHandle,
    open: AtomicBool,
    root: Option<String>,
    model: Option<NativeModelSnapshot>,
    source_model: Option<String>,
    policy: Option<NativePermissionPolicySnapshot>,
}
impl ContextTurn {
    fn live(&self) -> bool {
        self.open.load(Ordering::Acquire) && !self.handle.is_cancelled()
    }
}
pub(crate) struct ContextRegistration {
    owner: Arc<ContextSession>,
    state: Arc<ContextTurn>,
}
impl Drop for ContextRegistration {
    fn drop(&mut self) {
        self.state.open.store(false, Ordering::Release);
        let mut active = self
            .owner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active
            .as_ref()
            .is_some_and(|route| route.ptr_eq(&Arc::downgrade(&self.state)))
        {
            active.take();
        }
    }
}

/// Owned immutable review inputs, valid only during the exact authorization.
/// Holding this value cannot keep its session, turn, or admission alive.
pub struct NativePermissionReviewContext {
    turn: Arc<ContextTurn>,
    source: PermissionInvocationSnapshot,
}
impl NativePermissionReviewContext {
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.turn.live() && self.source.is_live()
    }
    #[must_use]
    pub fn pending_assistant(&self) -> &Message {
        self.source.pending_assistant()
    }
    #[must_use]
    pub fn source_cursor(&self) -> (usize, usize) {
        self.source.source_cursor()
    }
    #[must_use]
    pub fn target_call_id(&self) -> &ToolCallId {
        let machine_god_core::ContentBlock::ToolCall { call } =
            &self.pending_assistant().content[0]
        else {
            unreachable!("core creates a one-call view")
        };
        &call.id
    }
    #[must_use]
    pub fn source_model(&self) -> Option<&str> {
        self.turn.source_model.as_deref()
    }
    #[must_use]
    pub fn model_snapshot(&self) -> Option<&NativeModelSnapshot> {
        self.turn.model.as_ref()
    }
    #[must_use]
    pub fn permission_policy(&self) -> Option<&NativePermissionPolicySnapshot> {
        self.turn.policy.as_ref()
    }
    /// # Errors
    /// Missing provenance stays unavailable; no user-role fallback is performed.
    pub fn trusted_root_context(&self) -> Result<NativeAutoPermissionRootContext<'_>, Error> {
        if !self.is_live() {
            return Err(Error::Unavailable);
        }
        NativeAutoPermissionRootContext::from_proven_projection(
            self.turn.root.as_deref().ok_or(Error::Unavailable)?,
        )
        .map_err(|_| Error::InvalidProvenance)
    }
}
impl fmt::Debug for NativePermissionReviewContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativePermissionReviewContext")
            .finish_non_exhaustive()
    }
}

pub(crate) fn prepare_provenance(
    record: &mut machine_god_core::SessionRecord,
    prompt: Option<&str>,
    current_user: usize,
) -> Result<Option<String>, Error> {
    provenance::prepare(record, prompt, current_user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{Engine, SessionId, SessionIncarnationId};
    use machine_god_testkit::{
        InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler,
    };
    #[test]
    fn weak_routes_enforce_capacity_and_release_without_owning_sessions() {
        let engine = Engine::builder()
            .provider(ScriptedModelProvider::new("fixture", []))
            .session_store(InMemorySessionStore::new())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .build()
            .unwrap();
        let contexts = NativePermissionContexts::new();
        let mut owners = Vec::new();
        for i in 0..MAX_NATIVE_PERMISSION_CONTEXT_SESSIONS {
            let session = engine
                .create_session(
                    SessionId::new(format!("session-{i}")).unwrap(),
                    SessionIncarnationId::new("life").unwrap(),
                )
                .unwrap();
            owners.push(contexts.register(&session).unwrap());
            assert!(contexts.register(&session).is_err());
        }
        let session = engine
            .create_session(
                SessionId::new("extra").unwrap(),
                SessionIncarnationId::new("life").unwrap(),
            )
            .unwrap();
        assert!(contexts.register(&session).is_err());
        owners.clear();
        let owner = contexts.register(&session).unwrap();
        assert_eq!(contexts.routes.lock().unwrap().len(), 1);
        drop(owner);
        assert!(contexts.register(&session).is_ok());
    }
}
