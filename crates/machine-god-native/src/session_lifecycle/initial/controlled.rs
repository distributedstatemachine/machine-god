//! Explicit nonblocking lock admission for operations already on an owned worker.
use super::{
    BoxFuture, LifecycleOperation, NativeInitialSession, NativeSessionLifecycle,
    NativeSessionLifecycleError, NativeSessionLifecycleErrorKind, NativeSessionMetadata, Session,
    SessionId, SessionIncarnationId, SessionRecord, conflict, map_store_error,
};
use crate::session_store::{FileSessionScanControl, FileSessionScanError};
use machine_god_core::{SessionRevision, SessionStore, SessionStoreAccess, SessionStoreError};
use std::sync::Arc;

struct Access {
    store: Arc<crate::FileSessionStore>,
    erased: Arc<dyn SessionStore>,
    control: Arc<FileSessionScanControl>,
}
impl Access {
    fn new(store: Arc<crate::FileSessionStore>, control: Arc<FileSessionScanControl>) -> Arc<Self> {
        Arc::new(Self {
            erased: store.clone(),
            store,
            control,
        })
    }
}
impl SessionStoreAccess for Access {
    fn underlying_store(&self) -> &Arc<dyn SessionStore> {
        &self.erased
    }
}
impl SessionStore for Access {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        Box::pin(async move {
            self.store
                .load_controlled(&id, &self.control)
                .map_err(store_error)
        })
    }
    fn save(
        &self,
        _: SessionRecord,
        _: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        // This adapter is exclusively for checked canonical adoption, never publication.
        Box::pin(async { Err(unavailable()) })
    }
}
fn unavailable() -> SessionStoreError {
    SessionStoreError::new(
        machine_god_core::SessionStoreErrorKind::Unavailable,
        "controlled_load_unavailable",
        "controlled session access unavailable",
        false,
    )
}
fn store_error(error: FileSessionScanError) -> SessionStoreError {
    match error {
        FileSessionScanError::Store(error) => error,
        FileSessionScanError::Busy | FileSessionScanError::Cancelled => unavailable(),
    }
}
fn controlled_error(error: FileSessionScanError) -> NativeSessionLifecycleError {
    map_store_error(store_error(error))
}

impl NativeSessionLifecycle {
    /// Must run on an owned worker. Contended locks reject without blocking that worker.
    pub(crate) fn prepare_initial_controlled(
        &self,
        id: SessionId,
        incarnation: SessionIncarnationId,
        metadata: NativeSessionMetadata,
        control: &FileSessionScanControl,
    ) -> Result<NativeInitialSession, NativeSessionLifecycleError> {
        let operation = self.operation();
        operation.validate_initial_metadata(&metadata)?;
        if operation
            .session_store
            .load_controlled(&id, control)
            .map_err(controlled_error)?
            .is_some()
        {
            return Err(NativeSessionLifecycleError::new(
                NativeSessionLifecycleErrorKind::AlreadyExists,
            ));
        }
        control.check().map_err(controlled_error)?;
        let candidate = SessionRecord::empty(id, incarnation);
        let reservation = operation.reserve_candidate(&candidate)?;
        Ok(NativeInitialSession {
            operation,
            candidate,
            metadata,
            _reservation: reservation,
            attempted: false,
        })
    }

    pub(crate) async fn resume_controlled(
        &self,
        id: SessionId,
        control: Arc<FileSessionScanControl>,
    ) -> Result<Session, NativeSessionLifecycleError> {
        let operation = self.operation();
        let record = operation
            .session_store
            .load_controlled(&id, &control)
            .map_err(controlled_error)?
            .ok_or_else(|| {
                NativeSessionLifecycleError::new(NativeSessionLifecycleErrorKind::NotFound)
            })?;
        operation.load_controlled_exact(record, control).await
    }
}
impl LifecycleOperation {
    async fn load_controlled_exact(
        &self,
        expected: SessionRecord,
        control: Arc<FileSessionScanControl>,
    ) -> Result<Session, NativeSessionLifecycleError> {
        let access = Access::new(self.session_store.clone(), control);
        let session = self
            .engine
            .load_session_at_revision_with_access(
                expected.id.clone(),
                expected.incarnation_id.clone(),
                expected.revision,
                access,
            )
            .await
            .map_err(super::super::map_engine_error)?
            .ok_or_else(|| {
                NativeSessionLifecycleError::new(NativeSessionLifecycleErrorKind::NotFound)
            })?;
        if session.record() != expected || session.has_active_turn() {
            return Err(conflict());
        }
        Ok(session)
    }
}
impl NativeInitialSession {
    pub(crate) async fn publish_controlled(
        &mut self,
        control: Arc<FileSessionScanControl>,
    ) -> Result<Session, NativeSessionLifecycleError> {
        if self.attempted {
            return Err(conflict());
        }
        self.attempted = true;
        let expected = self
            .operation
            .session_store
            .create_record_with_metadata_controlled(
                self.candidate.id.clone(),
                self.candidate.incarnation_id.clone(),
                &self.metadata,
                &control,
            )
            .map_err(controlled_error)?;
        self.operation
            .load_controlled_exact(expected, control)
            .await
    }
    pub(crate) async fn reconcile_controlled(
        &mut self,
        control: Arc<FileSessionScanControl>,
    ) -> Result<Option<Session>, NativeSessionLifecycleError> {
        if !self.attempted {
            return Err(conflict());
        }
        let Some(expected) = self
            .operation
            .session_store
            .confirm_initial_record_controlled(
                self.candidate.id.clone(),
                self.candidate.incarnation_id.clone(),
                &self.metadata,
                &control,
            )
            .map_err(controlled_error)?
        else {
            return Ok(None);
        };
        self.operation
            .load_controlled_exact(expected, control)
            .await
            .map(Some)
    }
}
