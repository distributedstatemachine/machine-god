use std::fmt;
use std::path::Path;

mod rebind_history;
#[cfg(test)]
mod rebind_tests;
pub use rebind_history::{
    MAX_NATIVE_WORKSPACE_REBINDINGS, NATIVE_WORKSPACE_REBINDINGS_KEY, NativeWorkspaceRebinding,
    NativeWorkspaceRebindingHistory,
};

use machine_god_core::{BoxFuture, EngineError, Session, SessionRevision, SessionStoreErrorKind};

use crate::{NATIVE_SESSION_METADATA_KEY, NativeSessionMetadata, NativeSessionMetadataError};

/// Redacted outcome of a native metadata mutation. Persistence failure can be
/// ambiguous; reload the session before deciding whether to retry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeSessionMetadataMutationError {
    InvalidMetadata(NativeSessionMetadataError),
    Busy,
    Conflict,
    HostClosed,
    Persistence,
    Engine,
    InvalidHistory,
    HistoryLimit,
}

impl fmt::Display for NativeSessionMetadataMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetadata(error) => error.fmt(formatter),
            Self::Busy => formatter.write_str("session is busy"),
            Self::Conflict => formatter.write_str("session changed before metadata publication"),
            Self::HostClosed => formatter.write_str("session host is closed"),
            Self::Persistence => formatter.write_str("session metadata persistence failed"),
            Self::Engine => formatter.write_str("session metadata mutation failed"),
            Self::InvalidHistory => {
                formatter.write_str("session workspace rebinding history is invalid")
            }
            Self::HistoryLimit => {
                formatter.write_str("session workspace rebinding history is full")
            }
        }
    }
}

impl std::error::Error for NativeSessionMetadataMutationError {}

/// Atomically records a workspace association change and its bounded history.
///
/// The explicit workspace is descriptive metadata, not tool/root authority.
/// Construction is inert. First poll requires the expected canonical revision;
/// core's exclusive metadata transaction checks it again after reconciling any
/// uncertain save. An unchanged workspace uses the same checked lease without
/// saving, advancing time/revision, or creating an event. Origin stays unknown
/// for legacy records and otherwise remains the explicitly recorded original.
///
/// # Errors
/// Rejects invalid metadata/history, full history, regressing update time,
/// busy/changed state, exhausted revisions, or failed publication. Persistence
/// failure or dropping pending work may follow publication; reconcile rather
/// than blindly retrying. No provider, environment, filesystem root or clock is
/// consulted by this adapter; persistence runs only through the core session.
#[must_use]
pub fn rebind_native_session_workspace<'a>(
    session: &'a Session,
    expected_revision: SessionRevision,
    workspace: &'a Path,
    now_ms: i64,
) -> BoxFuture<'a, Result<SessionRevision, NativeSessionMetadataMutationError>> {
    Box::pin(async move {
        let record = session.record_snapshot();
        if record.revision != expected_revision {
            return Err(NativeSessionMetadataMutationError::Conflict);
        }
        let mut metadata = NativeSessionMetadata::from_metadata(&record.metadata)
            .map_err(NativeSessionMetadataMutationError::InvalidMetadata)?;
        let mut history = NativeWorkspaceRebindingHistory::from_metadata(&record.metadata)?;
        history.validate_current(&metadata, record.revision)?;
        let previous = metadata.workspace().map(Path::to_owned);
        if !metadata
            .rebind_workspace(workspace, now_ms)
            .map_err(NativeSessionMetadataMutationError::InvalidMetadata)?
        {
            return session
                .check_metadata_revision(expected_revision)
                .await
                .map_err(map_engine_error);
        }
        expected_revision
            .0
            .checked_add(1)
            .ok_or(NativeSessionMetadataMutationError::Engine)?;
        history.append(previous, workspace.to_owned(), now_ms, expected_revision)?;
        let mut entries = record.metadata.clone();
        entries.insert(NATIVE_SESSION_METADATA_KEY.to_owned(), metadata.to_value());
        entries.insert(
            NATIVE_WORKSPACE_REBINDINGS_KEY.to_owned(),
            history.to_value(),
        );
        session
            .update_metadata(expected_revision, entries)
            .await
            .map_err(map_engine_error)
    })
}

/// Persists a validated title using the session's exclusive metadata transaction.
///
/// Construction is effect-inert. First poll reads the canonical record and stages
/// the title plus injected update time, preserving unrelated metadata. Core then
/// excludes prompt admission and publishes only against that exact revision.
/// The returned revision proves successful persistence, not just a display change.
/// No provider, tool, permission prompt, environment or clock is consulted.
///
/// # Errors
/// Returns a fixed category for invalid input, busy/conflicting state, closed
/// host or failed persistence. Persistence failure may be post-publication;
/// callers must reconcile instead of claiming that no title was saved.
#[must_use]
pub fn rename_native_session<'a>(
    session: &'a Session,
    title: &'a str,
    now_ms: i64,
) -> BoxFuture<'a, Result<SessionRevision, NativeSessionMetadataMutationError>> {
    Box::pin(async move {
        let mut record = session.record();
        let mut metadata = NativeSessionMetadata::from_metadata(&record.metadata)
            .map_err(NativeSessionMetadataMutationError::InvalidMetadata)?;
        metadata
            .rename(title, now_ms)
            .map_err(NativeSessionMetadataMutationError::InvalidMetadata)?;
        record
            .metadata
            .insert(NATIVE_SESSION_METADATA_KEY.to_owned(), metadata.to_value());
        session
            .update_metadata(record.revision, record.metadata)
            .await
            .map_err(map_engine_error)
    })
}

fn map_engine_error(error: EngineError) -> NativeSessionMetadataMutationError {
    match error {
        EngineError::HostClosed => NativeSessionMetadataMutationError::HostClosed,
        EngineError::SessionBusy => NativeSessionMetadataMutationError::Busy,
        EngineError::SessionIncarnationConflict => NativeSessionMetadataMutationError::Conflict,
        EngineError::Store(error) if error.kind == SessionStoreErrorKind::Conflict => {
            NativeSessionMetadataMutationError::Conflict
        }
        EngineError::Store(_) => NativeSessionMetadataMutationError::Persistence,
        _ => NativeSessionMetadataMutationError::Engine,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use futures_executor::block_on;
    use machine_god_core::{
        Engine, Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionStoreError,
    };
    use machine_god_testkit::{
        InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler, SessionStoreScript,
        SessionStoreStep,
    };
    use serde_json::json;

    use super::*;

    fn fixture(
        script: SessionStoreScript,
    ) -> (Session, InMemorySessionStore, ScriptedModelProvider) {
        let id = SessionId::new("metadata-command-test").unwrap();
        let mut record = SessionRecord::empty(
            id.clone(),
            SessionIncarnationId::new("metadata-command-incarnation").unwrap(),
        );
        record.revision = SessionRevision(1);
        record.next_turn_sequence = 9;
        record.messages = vec![
            Message::text(Role::User, "original"),
            Message::text(Role::Assistant, "reply"),
        ];
        record
            .metadata
            .insert("unrelated".to_owned(), json!({"keep": [1, 2, 3]}));
        let store =
            InMemorySessionStore::configured(BTreeMap::from([(id.clone(), record)]), script, 20);
        let provider = ScriptedModelProvider::new("metadata-test", []);
        let engine = Engine::builder()
            .provider(provider.clone())
            .session_store(store.clone())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .build()
            .unwrap();
        let session = block_on(engine.load_session(id)).unwrap().unwrap();
        (session, store, provider)
    }

    #[test]
    fn unpolled_rename_is_inert_and_success_is_durable() {
        let (session, store, provider) = fixture(SessionStoreScript::default());
        let original = session.record();
        let calls = store.calls().len();
        let future = rename_native_session(&session, "ignored", 200);
        assert_eq!(store.calls().len(), calls);
        drop(future);
        assert_eq!(session.record(), original);
        let revision = block_on(rename_native_session(&session, "  saved title\n", 200)).unwrap();
        assert_eq!(revision, SessionRevision(2));
        let durable = store.record(&session.id()).unwrap();
        assert_eq!(durable, session.record());
        assert_eq!(durable.messages, original.messages);
        assert_eq!(durable.next_turn_sequence, original.next_turn_sequence);
        assert_eq!(durable.id, original.id);
        assert_eq!(durable.incarnation_id, original.incarnation_id);
        assert_eq!(
            durable.metadata["unrelated"],
            original.metadata["unrelated"]
        );
        let metadata = NativeSessionMetadata::from_metadata(&durable.metadata).unwrap();
        assert_eq!(metadata.title(), Some("saved title"));
        assert_eq!(metadata.updated_at_ms(), Some(200));
        assert_eq!(metadata.created_at_ms(), None);
        assert_eq!(metadata.workspace(), None);
        assert!(provider.requests().is_empty());
    }

    #[test]
    fn invalid_title_and_busy_turn_do_not_write_metadata() {
        let (session, store, provider) = fixture(SessionStoreScript::default());
        let original = session.record();
        let calls = store.calls().len();
        assert_eq!(
            block_on(rename_native_session(&session, "bad\0title", 200)),
            Err(NativeSessionMetadataMutationError::InvalidMetadata(
                NativeSessionMetadataError::InvalidTitle
            ))
        );
        assert_eq!(store.calls().len(), calls);
        assert_eq!(session.record(), original);
        let turn = block_on(session.prompt("active turn")).unwrap();
        let active = session.record();
        let calls = store.calls().len();
        assert_eq!(
            block_on(rename_native_session(&session, "valid", 200)),
            Err(NativeSessionMetadataMutationError::Busy)
        );
        assert_eq!(session.record(), active);
        assert_eq!(store.calls().len(), calls);
        assert!(provider.requests().is_empty());
        drop(turn);
    }

    #[test]
    fn failed_save_is_not_reported_as_a_successful_rename() {
        let (session, store, provider) = fixture(SessionStoreScript {
            saves: Some(vec![SessionStoreStep::Error(SessionStoreError::new(
                SessionStoreErrorKind::Unavailable,
                "SECRET",
                "SECRET",
                false,
            ))]),
            loads: None,
        });
        let original = session.record();
        let error = block_on(rename_native_session(&session, "not saved", 200)).unwrap_err();
        assert_eq!(error, NativeSessionMetadataMutationError::Persistence);
        assert_eq!(store.record(&session.id()).unwrap(), original);
        assert_eq!(session.record(), original);
        assert!(!format!("{error:?} {error}").contains("SECRET"));
        assert!(provider.requests().is_empty());
    }

    #[test]
    fn malformed_existing_namespace_is_not_silently_replaced() {
        let (session, store, _) = fixture(SessionStoreScript::default());
        let mut record = session.record();
        record.metadata.insert(
            NATIVE_SESSION_METADATA_KEY.to_owned(),
            json!({"schema_version": 3}),
        );
        block_on(session.update_metadata(record.revision, record.metadata)).unwrap();
        let calls = store.calls().len();
        let original = session.record();
        assert_eq!(
            block_on(rename_native_session(&session, "valid", 200)),
            Err(NativeSessionMetadataMutationError::InvalidMetadata(
                NativeSessionMetadataError::UnsupportedVersion
            ))
        );
        assert_eq!(store.calls().len(), calls);
        assert_eq!(session.record(), original);
    }
}
