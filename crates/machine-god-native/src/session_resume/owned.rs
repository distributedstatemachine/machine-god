//! Per-operation access to the exact engine store; no ambient store substitution.
use super::{
    Arc, BoxFuture, EngineError, Error, FileSessionStore, Kind, NativeConversation,
    NativeObservedSession, NativePreparedResume, NativeSessionCatalogQuery, NativeSessionLifecycle,
    Path, SessionId, SessionRevision, SessionStore, SessionStoreErrorKind, map_engine,
    map_mutation, validate_native_record,
};
use crate::{
    NativeOwnedWorkerScope,
    session_store::{FileSessionScanControl, FileSessionScanError},
};
use machine_god_core::{CancellationToken, SessionRecord, SessionStoreAccess, SessionStoreError};
use std::sync::atomic::{AtomicU8, Ordering};

struct Access {
    store: Arc<FileSessionStore>,
    erased: Arc<dyn SessionStore>,
    workers: NativeOwnedWorkerScope,
    control: Arc<FileSessionScanControl>,
    failure: Arc<AtomicU8>,
    #[cfg(test)]
    before_io: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl Access {
    fn new(
        store: Arc<FileSessionStore>,
        workers: NativeOwnedWorkerScope,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            erased: store.clone(),
            store,
            workers,
            control: Arc::new(FileSessionScanControl {
                cancel: cancellation,
                abandoned: CancellationToken::new(),
                #[cfg(test)]
                after_read: None,
            }),
            failure: Arc::new(AtomicU8::new(0)),
            #[cfg(test)]
            before_io: None,
        }
    }
    fn map_failure(&self, error: Error) -> Error {
        match self.failure.load(Ordering::Acquire) {
            1 => Error::new(Kind::Busy),
            2 => Error::new(Kind::Cancelled),
            _ => error,
        }
    }
    fn check(&self) -> Result<(), Error> {
        self.control
            .check()
            .map_err(|_| Error::new(Kind::Cancelled))
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
        let store = self.store.clone();
        let control = self.control.clone();
        let failure = self.failure.clone();
        #[cfg(test)]
        let hook = self.before_io.clone();
        Box::pin(async move {
            self.workers
                .run(move || {
                    #[cfg(test)]
                    if let Some(hook) = hook {
                        hook();
                    }
                    store
                        .load_controlled(&id, &control)
                        .map_err(|error| map_io(error, &failure))
                })
                .await
                .map_err(|_| unavailable())?
        })
    }
    fn save(
        &self,
        record: SessionRecord,
        revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        let store = self.store.clone();
        let control = self.control.clone();
        let failure = self.failure.clone();
        #[cfg(test)]
        let hook = self.before_io.clone();
        Box::pin(async move {
            self.workers
                .run(move || {
                    #[cfg(test)]
                    if let Some(hook) = hook {
                        hook();
                    }
                    store
                        .save_controlled(&mut { record }, revision, &control)
                        .map_err(|error| map_io(error, &failure))
                })
                .await
                .map_err(|_| unavailable())?
        })
    }
}
fn unavailable() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Unavailable,
        "store_failed",
        "session store failed",
        false,
    )
}
fn map_io(error: FileSessionScanError, failure: &AtomicU8) -> SessionStoreError {
    match error {
        FileSessionScanError::Store(error) => error,
        FileSessionScanError::Busy => {
            failure.store(1, Ordering::Release);
            unavailable()
        }
        FileSessionScanError::Cancelled => {
            failure.store(2, Ordering::Release);
            unavailable()
        }
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(crate) fn resume<'a>(
    lifecycle: &'a NativeSessionLifecycle,
    observed: NativeObservedSession,
    workspace: &'a Path,
    now_ms: i64,
    workers: NativeOwnedWorkerScope,
    cancellation: CancellationToken,
) -> BoxFuture<'a, Result<NativeConversation, Error>> {
    Box::pin(async move {
        NativeSessionCatalogQuery::new(1)
            .expect("valid limit")
            .with_workspace(workspace)
            .map_err(|_| Error::new(Kind::InvalidWorkspace))?;
        let store = Arc::clone(lifecycle.session_store());
        let access = Arc::new(Access::new(store.clone(), workers, cancellation));
        run(lifecycle, observed, workspace, now_ms, store, access).await
    })
}

#[cfg(feature = "ai-gateway-http")]
pub(crate) async fn flush_candidate(
    host: &crate::NativeReferenceHost,
    runtime: &crate::NativeConversationRuntime,
    now_ms: i64,
) -> Result<(), crate::NativeInteractiveError> {
    let access = Arc::new(Access::new(
        host.session_store().clone(),
        host.control_workers()
            .ok_or(crate::NativeInteractiveError::Configuration)?,
        CancellationToken::new(),
    ));
    let guard = CancelOnDrop(access.control.abandoned.clone());
    let result = runtime
        .flush_model_preferences_with_access(now_ms, Some(access.clone()))
        .await;
    drop(guard);
    match result {
        Ok(_) => Ok(()),
        Err(error) => match access.failure.load(Ordering::Acquire) {
            1 => Err(crate::NativeInteractiveError::Resume(Error::new(
                Kind::Busy,
            ))),
            2 => Err(crate::NativeInteractiveError::Resume(Error::new(
                Kind::Cancelled,
            ))),
            _ => Err(error.into()),
        },
    }
}

async fn run(
    lifecycle: &NativeSessionLifecycle,
    observed: NativeObservedSession,
    workspace: &Path,
    now_ms: i64,
    store: Arc<FileSessionStore>,
    access: Arc<Access>,
) -> Result<NativeConversation, Error> {
    let guard = CancelOnDrop(access.control.abandoned.clone());
    let result = prepare_and_adopt(
        lifecycle,
        observed,
        workspace,
        now_ms,
        store,
        access.clone(),
    )
    .await;
    drop(guard);
    result.map_err(|error| access.map_failure(error))
}

async fn prepare_and_adopt(
    lifecycle: &NativeSessionLifecycle,
    observed: NativeObservedSession,
    workspace: &Path,
    now_ms: i64,
    store: Arc<FileSessionStore>,
    access: Arc<Access>,
) -> Result<NativeConversation, Error> {
    access.check()?;
    let record = access
        .load(observed.id.clone())
        .await
        .map_err(|error| map_engine(EngineError::Store(error)))?
        .ok_or_else(|| Error::new(Kind::Conflict))?;
    if record.id != observed.id
        || record.incarnation_id != observed.incarnation_id
        || record.revision != observed.revision
    {
        return Err(Error::new(Kind::Conflict));
    }
    validate_native_record(&record)?;
    drop(record);
    access.check()?;
    let session = lifecycle
        .engine()
        .requester()
        .load_session_at_revision_with_access(
            observed.id.clone(),
            observed.incarnation_id.clone(),
            observed.revision,
            access.clone(),
        )
        .await
        .map_err(map_engine)?
        .ok_or_else(|| Error::new(Kind::Conflict))?;
    let mut prepared = NativePreparedResume {
        session,
        store,
        id: observed.id,
        incarnation_id: observed.incarnation_id,
        revision: observed.revision,
    };
    prepared.check_live()?;
    validate_native_record(&prepared.session.record_snapshot())?;
    access.check()?;
    prepared.revision = crate::session_metadata_commands::rebind_workspace_with_access(
        &prepared.session,
        prepared.revision,
        workspace,
        now_ms,
        Some(access.clone()),
    )
    .await
    .map_err(map_mutation)?;
    access.check()?;
    let conversation = prepared.adopt_with_access(Some(access.clone())).await?;
    access.check()?;
    Ok(conversation)
}

#[cfg(test)]
mod tests;
