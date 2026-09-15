//! Retained exact candidate for managed runtime creation and explicit repair.

use super::{
    LifecycleOperation, NativeSessionLifecycle, NativeSessionLifecycleError,
    NativeSessionLifecycleErrorKind, map_store_error,
};
use crate::NativeSessionMetadata;
use machine_god_core::{
    BoxFuture, Session, SessionId, SessionIncarnationId, SessionRecord, SessionReservation,
};

impl NativeSessionLifecycle {
    /// Allocate once, with no filesystem effects or host-lifetime vote.
    pub(crate) fn allocate_identity(
        &self,
    ) -> BoxFuture<'static, Result<(SessionId, SessionIncarnationId), NativeSessionLifecycleError>>
    {
        let operation = self.operation();
        Box::pin(async move {
            Ok((
                operation.next_session_id()?,
                operation.next_incarnation_id()?,
            ))
        })
    }

    /// Reserve the exact candidate before any initial publication. The caller
    /// retains this receipt across errors instead of regenerating its identity.
    pub(crate) fn prepare_initial(
        &self,
        id: SessionId,
        incarnation: SessionIncarnationId,
        metadata: NativeSessionMetadata,
    ) -> BoxFuture<'static, Result<NativeInitialSession, NativeSessionLifecycleError>> {
        let operation = self.operation();
        Box::pin(async move {
            operation.validate_initial_metadata(&metadata)?;
            if operation.load_record(id.clone()).await?.is_some() {
                return Err(NativeSessionLifecycleError::new(
                    NativeSessionLifecycleErrorKind::AlreadyExists,
                ));
            }
            let candidate = SessionRecord::empty(id, incarnation);
            let reservation = operation.reserve_candidate(&candidate)?;
            Ok(NativeInitialSession {
                operation,
                candidate,
                metadata,
                _reservation: reservation,
                attempted: false,
            })
        })
    }
}

/// No Session/Engine ownership: the reservation and requester cannot resurrect
/// a closed host. Dropping an observer never rolls back an attempted publication.
pub(crate) struct NativeInitialSession {
    operation: LifecycleOperation,
    candidate: SessionRecord,
    metadata: NativeSessionMetadata,
    _reservation: SessionReservation,
    attempted: bool,
}

impl NativeInitialSession {
    pub(crate) fn publish(
        &mut self,
    ) -> BoxFuture<'_, Result<Session, NativeSessionLifecycleError>> {
        Box::pin(async move {
            if self.attempted {
                return Err(conflict());
            }
            self.attempted = true;
            let expected = self
                .operation
                .session_store
                .create_record_with_metadata(
                    self.candidate.id.clone(),
                    self.candidate.incarnation_id.clone(),
                    &self.metadata,
                )
                .map_err(map_store_error)?;
            self.load_exact(expected).await
        })
    }

    /// Explicit synchronization, not readback-as-confirmation. A caller keeps
    /// this same receipt on errors; neither this nor publish retries creation.
    pub(crate) fn reconcile(
        &mut self,
    ) -> BoxFuture<'_, Result<Option<Session>, NativeSessionLifecycleError>> {
        Box::pin(async move {
            if !self.attempted {
                return Err(conflict());
            }
            let Some(expected) = self
                .operation
                .session_store
                .confirm_initial_record(
                    self.candidate.id.clone(),
                    self.candidate.incarnation_id.clone(),
                    &self.metadata,
                )
                .map_err(map_store_error)?
            else {
                return Ok(None);
            };
            self.load_exact(expected).await.map(Some)
        })
    }

    async fn load_exact(
        &self,
        expected: SessionRecord,
    ) -> Result<Session, NativeSessionLifecycleError> {
        let session = self.operation.load_canonical(expected.id.clone()).await?;
        if session.record() != expected || session.has_active_turn() {
            return Err(conflict());
        }
        Ok(session)
    }
}

fn conflict() -> NativeSessionLifecycleError {
    NativeSessionLifecycleError::new(NativeSessionLifecycleErrorKind::Conflict)
}

impl std::fmt::Debug for NativeInitialSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeInitialSession")
            .finish_non_exhaustive()
    }
}
