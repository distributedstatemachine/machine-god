use super::{Session, SessionState};
use crate::{
    CancellationToken, Message, PermissionRequest, PermissionRequestId, SessionId,
    SessionIncarnationId, TurnId,
};
use std::fmt;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct ActivePermissionContext {
    request_id: PermissionRequestId,
    session_id: SessionId,
    incarnation_id: SessionIncarnationId,
    turn_id: TurnId,
    cursor: (usize, usize),
    pending: Weak<Message>,
    cancellation: CancellationToken,
    open: AtomicBool,
}

pub(super) struct PermissionContextScope {
    state: Arc<SessionState>,
    active: Arc<ActivePermissionContext>,
}

impl PermissionContextScope {
    pub(super) fn open(
        state: &Arc<SessionState>,
        request: &PermissionRequest,
        cursor: (usize, usize),
        pending: &Arc<Message>,
        cancellation: &CancellationToken,
    ) -> Self {
        let active = Arc::new(ActivePermissionContext {
            request_id: request.id.clone(),
            session_id: request.session_id.clone(),
            incarnation_id: request.session_incarnation_id.clone(),
            turn_id: request.turn_id.clone(),
            cursor,
            pending: Arc::downgrade(pending),
            cancellation: cancellation.clone(),
            open: AtomicBool::new(true),
        });
        *state
            .permission_context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::downgrade(&active));
        Self {
            state: state.clone(),
            active,
        }
    }
}
impl Drop for PermissionContextScope {
    fn drop(&mut self) {
        self.active.open.store(false, Ordering::Release);
        let mut current = self
            .state
            .permission_context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current
            .as_ref()
            .is_some_and(|value| value.ptr_eq(&Arc::downgrade(&self.active)))
        {
            current.take();
        }
    }
}

/// Immutable identity of the exact currently authorizing provider call. The
/// one-call assistant projection retains its original input, not an archive or
/// prepared-argument projection. It conveys no user provenance or authority.
pub struct PermissionInvocationSnapshot {
    active: Weak<ActivePermissionContext>,
    pending: Arc<Message>,
    cursor: (usize, usize),
}
impl PermissionInvocationSnapshot {
    #[must_use]
    pub fn pending_assistant(&self) -> &Message {
        &self.pending
    }
    #[must_use]
    pub const fn source_cursor(&self) -> (usize, usize) {
        self.cursor
    }
    /// False after authorization returns/drops or actual turn cancellation.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.active.upgrade().is_some_and(|active| {
            active.open.load(Ordering::Acquire) && !active.cancellation.is_cancelled()
        })
    }
}
impl fmt::Debug for PermissionInvocationSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermissionInvocationSnapshot")
            .finish_non_exhaustive()
    }
}
impl Session {
    /// Observes only the exact currently authorizing request, without scanning
    /// IDs, cloning the transcript, or exposing ambient/user authority. The
    /// returned call is a shared original one-call assistant projection; its
    /// canonical message/block position remains distinct from reused call IDs.
    #[must_use]
    pub fn permission_invocation_snapshot(
        &self,
        request: &PermissionRequest,
    ) -> Option<PermissionInvocationSnapshot> {
        let active = self
            .state
            .permission_context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()?
            .upgrade()?;
        if active.request_id != request.id
            || active.session_id != request.session_id
            || active.incarnation_id != request.session_incarnation_id
            || active.turn_id != request.turn_id
            || !active.open.load(Ordering::Acquire)
            || active.cancellation.is_cancelled()
        {
            return None;
        }
        let snapshot = PermissionInvocationSnapshot {
            pending: active.pending.upgrade()?,
            cursor: active.cursor,
            active: Arc::downgrade(&active),
        };
        snapshot.is_live().then_some(snapshot)
    }
}
