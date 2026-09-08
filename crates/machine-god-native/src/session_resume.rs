//! Explicit native resume selection and checked candidate adoption.

use crate::{
    FileSessionStore, NativeContextPreferences, NativeConversation, NativeConversationError,
    NativeModelPreferences, NativeSessionCatalog, NativeSessionCatalogEntry,
    NativeSessionCatalogError, NativeSessionCatalogErrorKind, NativeSessionCatalogQuery,
    NativeSessionLifecycle, NativeSessionLifecycleError, NativeSessionLifecycleErrorKind,
    NativeSessionMetadata, NativeSessionMetadataMutationError, rebind_native_session_workspace,
};
use machine_god_core::{
    BoxFuture, EngineError, Session, SessionId, SessionIncarnationId, SessionRevision,
    SessionStore, SessionStoreErrorKind,
};
use std::{fmt, path::Path, sync::Arc};

/// Explicit target selection. Neither form generates a new identity.
#[derive(Clone)]
pub enum NativeResumeTarget {
    Latest,
    Exact(SessionId),
}
impl fmt::Debug for NativeResumeTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Latest => "NativeResumeTarget::Latest",
            Self::Exact(_) => "NativeResumeTarget::Exact(..)",
        })
    }
}

/// Fixed resume failure categories, never an automatic retry instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSessionResumeErrorKind {
    InvalidWorkspace,
    NotFound,
    SelectionIncomplete,
    Conflict,
    Busy,
    Corrupt,
    Unavailable,
    HostClosed,
    Engine,
}
impl NativeSessionResumeErrorKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidWorkspace => "invalid_workspace",
            Self::NotFound => "not_found",
            Self::SelectionIncomplete => "selection_incomplete",
            Self::Conflict => "conflict",
            Self::Busy => "busy",
            Self::Corrupt => "corrupt",
            Self::Unavailable => "unavailable",
            Self::HostClosed => "host_closed",
            Self::Engine => "engine",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSessionResumeError {
    kind: NativeSessionResumeErrorKind,
}
impl NativeSessionResumeError {
    const fn new(kind: NativeSessionResumeErrorKind) -> Self {
        Self { kind }
    }
    #[must_use]
    pub const fn kind(self) -> NativeSessionResumeErrorKind {
        self.kind
    }
}
impl fmt::Display for NativeSessionResumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.kind {
            NativeSessionResumeErrorKind::InvalidWorkspace => "native resume workspace is invalid",
            NativeSessionResumeErrorKind::NotFound => "native resume target was not found",
            NativeSessionResumeErrorKind::SelectionIncomplete => {
                "native resume selection is incomplete"
            }
            NativeSessionResumeErrorKind::Conflict => "native resume target changed",
            NativeSessionResumeErrorKind::Busy => "native resume target is busy",
            NativeSessionResumeErrorKind::Corrupt => "native resume target is corrupt",
            NativeSessionResumeErrorKind::Unavailable => "native resume persistence is unavailable",
            NativeSessionResumeErrorKind::HostClosed => "native resume host is closed",
            NativeSessionResumeErrorKind::Engine => "native resume admission failed",
        })
    }
}
impl std::error::Error for NativeSessionResumeError {}
type Error = NativeSessionResumeError;
type Kind = NativeSessionResumeErrorKind;

/// Opaque canonical candidate. Dropping it starts no work and does not roll back
/// an already confirmed explicit workspace rebind.
pub struct NativePreparedResume {
    session: Session,
    store: Arc<FileSessionStore>,
    id: SessionId,
    incarnation_id: SessionIncarnationId,
    revision: SessionRevision,
}
impl fmt::Debug for NativePreparedResume {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePreparedResume { .. }")
    }
}
impl NativePreparedResume {
    #[must_use]
    pub const fn id(&self) -> &SessionId {
        &self.id
    }
    #[must_use]
    pub const fn incarnation_id(&self) -> &SessionIncarnationId {
        &self.incarnation_id
    }
    #[must_use]
    pub const fn revision(&self) -> SessionRevision {
        self.revision
    }

    /// Consumes this candidate after fresh durable and canonical validation.
    /// The future is inert before polling. Success is a checked admission
    /// observation, not a cross-process lease or an automatic runtime switch.
    #[must_use]
    pub fn adopt(self) -> BoxFuture<'static, Result<NativeConversation, Error>> {
        Box::pin(async move {
            self.check_live()?;
            let record = self
                .store
                .load(self.id.clone())
                .await
                .map_err(|error| {
                    Error::new(if error.kind == SessionStoreErrorKind::Corrupt {
                        Kind::Corrupt
                    } else {
                        Kind::Unavailable
                    })
                })?
                .ok_or_else(|| Error::new(Kind::Conflict))?;
            if record.id != self.id
                || record.incarnation_id != self.incarnation_id
                || record.revision != self.revision
                || record != *self.session.record_snapshot()
            {
                return Err(Error::new(Kind::Conflict));
            }
            validate_native_record(&record)?;
            drop(record);
            self.session
                .check_metadata_revision(self.revision)
                .await
                .map_err(map_engine)?;
            self.check_live()?;
            let conversation =
                NativeConversation::from_session(self.session.clone()).map_err(map_conversation)?;
            self.check_live()?;
            Ok(conversation)
        })
    }
    fn check_live(&self) -> Result<(), Error> {
        if self.session.has_active_turn() {
            return Err(Error::new(Kind::Busy));
        }
        let record = self.session.record_snapshot();
        if record.id != self.id
            || record.incarnation_id != self.incarnation_id
            || record.revision != self.revision
        {
            return Err(Error::new(Kind::Conflict));
        }
        Ok(())
    }
}

/// Selects and loads before returning a candidate, without touching a caller's
/// runtime, provider, tools, queue or background ownership. Native synchronous
/// I/O inherits the retained store's bounded bytes and unbounded lock latency.
#[must_use]
pub fn prepare_native_session_resume<'a>(
    lifecycle: &'a NativeSessionLifecycle,
    target: NativeResumeTarget,
    workspace: &'a Path,
    now_ms: i64,
) -> BoxFuture<'a, Result<NativePreparedResume, Error>> {
    Box::pin(prepare(lifecycle, target, workspace, now_ms, || {}, || {}))
}

async fn prepare(
    lifecycle: &NativeSessionLifecycle,
    target: NativeResumeTarget,
    workspace: &Path,
    now_ms: i64,
    after_selection: impl FnOnce() + Send,
    before_load: impl FnOnce() + Send,
) -> Result<NativePreparedResume, Error> {
    let query = NativeSessionCatalogQuery::new(1)
        .expect("one row is a valid limit")
        .with_workspace(workspace)
        .map_err(|_| Error::new(Kind::InvalidWorkspace))?;
    let store = Arc::clone(lifecycle.session_store());
    let catalog = NativeSessionCatalog::new(Arc::clone(&store));
    let exact = matches!(&target, NativeResumeTarget::Exact(_));
    let entry = match target {
        NativeResumeTarget::Latest => catalog
            .list(query)
            .await
            .map_err(map_catalog)?
            .latest()
            .map_err(|_| Error::new(Kind::SelectionIncomplete))?
            .cloned(),
        NativeResumeTarget::Exact(id) => catalog.exact(id).await.map_err(map_catalog)?,
    }
    .ok_or_else(|| Error::new(Kind::NotFound))?;
    after_selection();
    let record = lifecycle
        .replay(entry.id().clone())
        .await
        .map_err(map_lifecycle)?;
    if record.id != *entry.id()
        || record.incarnation_id != *entry.incarnation_id()
        || record.revision != entry.revision()
    {
        return Err(Error::new(Kind::Conflict));
    }
    validate_native_record(&record)?;
    drop(record);
    before_load();
    let session = lifecycle
        .engine()
        .requester()
        .load_session_at_revision(
            entry.id().clone(),
            entry.incarnation_id().clone(),
            entry.revision(),
        )
        .await
        .map_err(map_engine)?
        .ok_or_else(|| Error::new(Kind::Conflict))?;
    let mut prepared = candidate(session, store, &entry)?;
    validate_native_record(&prepared.session.record_snapshot())?;
    if exact {
        prepared.revision = rebind_native_session_workspace(
            &prepared.session,
            prepared.revision,
            workspace,
            now_ms,
        )
        .await
        .map_err(map_mutation)?;
    }
    prepared.check_live()?;
    Ok(prepared)
}

fn validate_native_record(record: &machine_god_core::SessionRecord) -> Result<(), Error> {
    let metadata = NativeSessionMetadata::from_metadata(&record.metadata)
        .map_err(|_| Error::new(Kind::Corrupt))?;
    crate::NativeWorkspaceRebindingHistory::from_metadata(&record.metadata)
        .and_then(|history| history.validate_current(&metadata, record.revision))
        .map_err(|_| Error::new(Kind::Corrupt))?;
    crate::conversation::validated_history(record).map_err(map_conversation)?;
    let preferences = NativeContextPreferences::from_metadata(&record.metadata)
        .map_err(|_| Error::new(Kind::Corrupt))?;
    if preferences != NativeContextPreferences::default() {
        preferences
            .validate_selection(record)
            .map_err(|_| Error::new(Kind::Corrupt))?;
    }
    NativeModelPreferences::from_metadata(&record.metadata)
        .map_err(|_| Error::new(Kind::Corrupt))?;
    Ok(())
}

fn candidate(
    session: Session,
    store: Arc<FileSessionStore>,
    entry: &NativeSessionCatalogEntry,
) -> Result<NativePreparedResume, Error> {
    let candidate = NativePreparedResume {
        session,
        store,
        id: entry.id().clone(),
        incarnation_id: entry.incarnation_id().clone(),
        revision: entry.revision(),
    };
    candidate.check_live()?;
    Ok(candidate)
}

fn map_catalog(error: NativeSessionCatalogError) -> Error {
    Error::new(match error.kind() {
        NativeSessionCatalogErrorKind::Corrupt => Kind::Corrupt,
        _ => Kind::Unavailable,
    })
}
fn map_lifecycle(error: NativeSessionLifecycleError) -> Error {
    Error::new(match error.kind() {
        NativeSessionLifecycleErrorKind::NotFound => Kind::NotFound,
        NativeSessionLifecycleErrorKind::LiveSession
        | NativeSessionLifecycleErrorKind::Conflict => Kind::Conflict,
        NativeSessionLifecycleErrorKind::Corrupt => Kind::Corrupt,
        NativeSessionLifecycleErrorKind::Unavailable => Kind::Unavailable,
        _ => Kind::Engine,
    })
}
fn map_conversation(error: NativeConversationError) -> Error {
    Error::new(match error {
        NativeConversationError::Busy => Kind::Busy,
        NativeConversationError::Conflict => Kind::Conflict,
        NativeConversationError::HostClosed => Kind::HostClosed,
        NativeConversationError::Persistence => Kind::Unavailable,
        NativeConversationError::Engine => Kind::Engine,
        _ => Kind::Corrupt,
    })
}
fn map_mutation(error: NativeSessionMetadataMutationError) -> Error {
    Error::new(match error {
        NativeSessionMetadataMutationError::Busy => Kind::Busy,
        NativeSessionMetadataMutationError::Conflict => Kind::Conflict,
        NativeSessionMetadataMutationError::HostClosed => Kind::HostClosed,
        NativeSessionMetadataMutationError::Persistence => Kind::Unavailable,
        NativeSessionMetadataMutationError::InvalidHistory => Kind::Corrupt,
        _ => Kind::Engine,
    })
}
fn map_engine(error: EngineError) -> Error {
    Error::new(match error {
        EngineError::SessionBusy => Kind::Busy,
        EngineError::SessionIncarnationConflict => Kind::Conflict,
        EngineError::HostClosed => Kind::HostClosed,
        EngineError::Store(error) if error.kind == SessionStoreErrorKind::Conflict => {
            Kind::Conflict
        }
        EngineError::Store(error) if error.kind == SessionStoreErrorKind::Corrupt => Kind::Corrupt,
        EngineError::Store(_) => Kind::Unavailable,
        _ => Kind::Engine,
    })
}

#[cfg(test)]
mod tests;
