use super::*;
use crate::NativeSessionOrigin;
use futures_executor::block_on;
use machine_god_core::{
    Engine, EngineLimits, Message, Role, SessionId, SessionIncarnationId, SessionRecord,
    SessionStore, SessionStoreError,
};
use machine_god_testkit::{
    InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler, SessionStoreScript,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    os::unix::ffi::OsStringExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

#[derive(Clone)]
struct Store {
    inner: InMemorySessionStore,
    mode: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}
impl SessionStore for Store {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.inner.load(id)
    }
    fn save(
        &self,
        record: SessionRecord,
        expected: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        struct SaveDrop(Arc<AtomicUsize>);
        impl Drop for SaveDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        Box::pin(async move {
            let _drop = SaveDrop(Arc::clone(&self.dropped));
            let mode = self.mode.swap(0, Ordering::SeqCst);
            if mode == 1 {
                return Err(private_error());
            }
            if mode == 2 {
                return std::future::pending().await;
            }
            let revision = self.inner.save(record, expected).await?;
            if mode == 3 {
                return Err(private_error());
            }
            if mode == 4 {
                return std::future::pending().await;
            }
            Ok(revision)
        })
    }
}
fn private_error() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Unavailable,
        "SECRET",
        "SECRET",
        false,
    )
}
fn fixture(
    metadata: Option<NativeSessionMetadata>,
    limits: EngineLimits,
) -> (Session, Store, ScriptedModelProvider) {
    fixture_at_revision(metadata, limits, SessionRevision(1))
}
fn fixture_at_revision(
    metadata: Option<NativeSessionMetadata>,
    limits: EngineLimits,
    revision: SessionRevision,
) -> (Session, Store, ScriptedModelProvider) {
    let id = SessionId::new("rebind-test").unwrap();
    let mut record = SessionRecord::empty(
        id.clone(),
        SessionIncarnationId::new("rebind-incarnation").unwrap(),
    );
    record.revision = revision;
    record.next_turn_sequence = 11;
    record.messages = vec![
        Message::text(Role::User, "original request"),
        Message::text(Role::Assistant, "original reply"),
    ];
    record
        .metadata
        .insert("checkpoint".into(), json!({"turn":9,"paused":true}));
    record
        .metadata
        .insert("preferences".into(), json!({"model":"kept","fast":true}));
    if let Some(metadata) = metadata {
        record
            .metadata
            .insert(NATIVE_SESSION_METADATA_KEY.into(), metadata.to_value());
    }
    let store = Store {
        inner: InMemorySessionStore::configured(
            BTreeMap::from([(id.clone(), record)]),
            SessionStoreScript::default(),
            512,
        ),
        mode: Arc::default(),
        dropped: Arc::default(),
    };
    let provider = ScriptedModelProvider::new("rebind-test", []);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(limits)
        .build()
        .unwrap();
    let session = block_on(engine.load_session(id)).unwrap().unwrap();
    (session, store, provider)
}
fn known() -> NativeSessionMetadata {
    NativeSessionMetadata::new(Path::new("/a"), 100, NativeSessionOrigin::Cli).unwrap()
}
fn mutate(
    session: &Session,
    path: &Path,
    now: i64,
) -> Result<SessionRevision, NativeSessionMetadataMutationError> {
    block_on(rebind_native_session_workspace(
        session,
        session.record_snapshot().revision,
        path,
        now,
    ))
}
fn history(session: &Session) -> NativeWorkspaceRebindingHistory {
    NativeWorkspaceRebindingHistory::from_metadata(&session.record_snapshot().metadata).unwrap()
}

#[test]
fn inert_then_atomic_rebind_preserves_every_non_workspace_fact() {
    let (session, store, provider) = fixture(Some(known()), EngineLimits::default());
    let before = session.record();
    let calls = store.inner.calls().len();
    let future = rebind_native_session_workspace(&session, before.revision, Path::new("/b"), 110);
    assert_eq!(store.inner.calls().len(), calls);
    drop(future);
    assert_eq!(session.record(), before);
    assert_eq!(
        mutate(&session, Path::new("/b"), 110).unwrap(),
        SessionRevision(2)
    );
    let after = session.record();
    assert_eq!(after, store.inner.record(&session.id()).unwrap());
    assert_eq!(after.messages, before.messages);
    assert_eq!(after.next_turn_sequence, before.next_turn_sequence);
    assert_eq!(after.id, before.id);
    assert_eq!(after.incarnation_id, before.incarnation_id);
    for key in ["checkpoint", "preferences"] {
        assert_eq!(after.metadata[key], before.metadata[key]);
    }
    let metadata = NativeSessionMetadata::from_metadata(&after.metadata).unwrap();
    assert_eq!(metadata.workspace(), Some(Path::new("/b")));
    assert_eq!(metadata.origin_workspace(), Some(Path::new("/a")));
    assert_eq!(metadata.created_at_ms(), Some(100));
    assert_eq!(metadata.updated_at_ms(), Some(110));
    let history = history(&session);
    assert_eq!(history.entries().len(), 1);
    assert_eq!(
        history.entries()[0].previous_workspace(),
        Some(Path::new("/a"))
    );
    assert_eq!(history.entries()[0].next_workspace(), Path::new("/b"));
    assert_eq!(history.entries()[0].at_ms(), 110);
    assert_eq!(history.entries()[0].previous_revision(), SessionRevision(1));
    assert!(provider.requests().is_empty());
}

#[test]
fn repeated_rebind_keeps_origin_and_title_mutations_keep_history() {
    let (session, _, _) = fixture(Some(known()), EngineLimits::default());
    mutate(&session, Path::new("/b"), 110).unwrap();
    block_on(rename_native_session(&session, "title", 120)).unwrap();
    mutate(&session, Path::new("/c"), 130).unwrap();
    let record = session.record();
    let metadata = NativeSessionMetadata::from_metadata(&record.metadata).unwrap();
    assert_eq!(metadata.origin_workspace(), Some(Path::new("/a")));
    assert_eq!(metadata.title(), Some("title"));
    assert_eq!(
        history(&session).entries()[1].previous_revision(),
        SessionRevision(3)
    );
    assert_eq!(
        history(&session).entries()[1].previous_workspace(),
        Some(Path::new("/b"))
    );
}

#[test]
fn legacy_origin_and_previous_unknown_stay_unknown() {
    let (session, _, _) = fixture(None, EngineLimits::default());
    mutate(&session, Path::new("/b"), 110).unwrap();
    let metadata = NativeSessionMetadata::from_metadata(&session.record().metadata).unwrap();
    assert_eq!(metadata.origin_workspace(), None);
    assert_eq!(metadata.origin(), None);
    assert_eq!(metadata.created_at_ms(), None);
    assert_eq!(history(&session).entries()[0].previous_workspace(), None);
    let legacy = BTreeMap::from([(
        NATIVE_SESSION_METADATA_KEY.into(),
        json!({"schema_version":1,"workspace_hex":"2f61"}),
    )]);
    let (session, _, _) = fixture(
        Some(NativeSessionMetadata::from_metadata(&legacy).unwrap()),
        EngineLimits::default(),
    );
    mutate(&session, Path::new("/c"), 110).unwrap();
    assert_eq!(
        NativeSessionMetadata::from_metadata(&session.record().metadata)
            .unwrap()
            .origin_workspace(),
        None
    );
    assert_eq!(
        history(&session).entries()[0].previous_workspace(),
        Some(Path::new("/a"))
    );
}

#[test]
fn both_workspace_paths_roundtrip_non_unicode_without_loss() {
    let a = PathBuf::from(std::ffi::OsString::from_vec(b"/a/\xff".to_vec()));
    let b = PathBuf::from(std::ffi::OsString::from_vec(b"/b/\xfe".to_vec()));
    let metadata = NativeSessionMetadata::new(&a, 0, NativeSessionOrigin::Cli).unwrap();
    let (session, _, _) = fixture(Some(metadata), EngineLimits::default());
    mutate(&session, &b, 1).unwrap();
    let metadata = NativeSessionMetadata::from_metadata(&session.record().metadata).unwrap();
    assert_eq!(metadata.origin_workspace(), Some(a.as_path()));
    assert_eq!(metadata.workspace(), Some(b.as_path()));
    assert_eq!(
        history(&session).entries()[0].previous_workspace(),
        Some(a.as_path())
    );
    assert_eq!(history(&session).entries()[0].next_workspace(), b.as_path());
}

#[test]
fn no_op_is_exact_and_never_saves_or_touches_time() {
    let (session, store, _) = fixture(Some(known()), EngineLimits::default());
    let before = session.record();
    let calls = store.inner.calls().len();
    assert_eq!(
        mutate(&session, Path::new("/a"), -999).unwrap(),
        before.revision
    );
    assert_eq!(session.record(), before);
    assert_eq!(store.inner.calls().len(), calls);
    assert_eq!(
        block_on(rebind_native_session_workspace(
            &session,
            SessionRevision(0),
            Path::new("/a"),
            110
        )),
        Err(NativeSessionMetadataMutationError::Conflict)
    );
    assert_eq!(store.inner.calls().len(), calls);
}

#[test]
fn first_poll_not_construction_fences_revision_and_busy_no_ops() {
    let (session, store, _) = fixture(Some(known()), EngineLimits::default());
    let future =
        rebind_native_session_workspace(&session, SessionRevision(1), Path::new("/b"), 110);
    block_on(rename_native_session(&session, "new revision", 105)).unwrap();
    let calls = store.inner.calls().len();
    assert_eq!(
        block_on(future),
        Err(NativeSessionMetadataMutationError::Conflict)
    );
    assert_eq!(store.inner.calls().len(), calls);
    let turn = block_on(session.prompt("busy")).unwrap();
    for path in ["/a", "/b"] {
        assert_eq!(
            mutate(&session, Path::new(path), 120),
            Err(NativeSessionMetadataMutationError::Busy)
        );
    }
    drop(turn);
}

#[test]
fn failure_before_save_and_pending_drop_publish_nothing_and_release_lease() {
    for mode in [1, 2] {
        let (session, store, _) = fixture(Some(known()), EngineLimits::default());
        let before = session.record();
        store.mode.store(mode, Ordering::SeqCst);
        let mut future =
            rebind_native_session_workspace(&session, before.revision, Path::new("/b"), 110);
        let poll = future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()));
        if mode == 1 {
            assert_eq!(
                poll,
                Poll::Ready(Err(NativeSessionMetadataMutationError::Persistence))
            );
        } else {
            assert!(poll.is_pending());
            assert_eq!(
                mutate(&session, Path::new("/a"), 0),
                Err(NativeSessionMetadataMutationError::Busy)
            );
        }
        drop(future);
        assert_eq!(store.dropped.load(Ordering::SeqCst), 1);
        assert_eq!(session.record(), before);
        assert_eq!(store.inner.record(&session.id()).unwrap(), before);
        assert_eq!(
            mutate(&session, Path::new("/b"), 110).unwrap(),
            SessionRevision(2)
        );
    }
}

#[test]
fn uncertain_publication_never_reports_stale_no_op_and_does_not_duplicate_event() {
    for mode in [3, 4] {
        let (session, store, _) = fixture(Some(known()), EngineLimits::default());
        store.mode.store(mode, Ordering::SeqCst);
        let mut future =
            rebind_native_session_workspace(&session, SessionRevision(1), Path::new("/b"), 110);
        let poll = future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()));
        if mode == 3 {
            assert_eq!(
                poll,
                Poll::Ready(Err(NativeSessionMetadataMutationError::Persistence))
            );
        } else {
            assert!(poll.is_pending());
        }
        drop(future);
        assert_eq!(session.record_snapshot().revision, SessionRevision(1));
        assert_eq!(
            store.inner.record(&session.id()).unwrap().revision,
            SessionRevision(2)
        );
        // In-memory /a looks unchanged but checked no-op must reload and conflict.
        assert_eq!(
            mutate(&session, Path::new("/a"), 0),
            Err(NativeSessionMetadataMutationError::Conflict)
        );
        let calls = store.inner.calls().len();
        assert_eq!(
            mutate(&session, Path::new("/b"), 0).unwrap(),
            SessionRevision(2)
        );
        assert_eq!(store.inner.calls().len(), calls);
        assert_eq!(history(&session).entries().len(), 1);
        assert!(
            !format!(
                "{} {:?}",
                NativeSessionMetadataMutationError::Persistence,
                history(&session)
            )
            .contains("SECRET")
        );
    }
}

#[test]
fn history_capacity_never_silently_discards_and_no_op_still_works() {
    let (session, store, _) = fixture(Some(known()), EngineLimits::default());
    for index in 0..MAX_NATIVE_WORKSPACE_REBINDINGS {
        mutate(&session, Path::new(&format!("/move-{index}")), 110).unwrap();
    }
    let before = session.record();
    assert_eq!(history(&session).entries().len(), 64);
    let calls = store.inner.calls().len();
    assert_eq!(
        mutate(&session, Path::new("/one-too-many"), 120),
        Err(NativeSessionMetadataMutationError::HistoryLimit)
    );
    assert_eq!(
        mutate(&session, Path::new("/move-63"), 0).unwrap(),
        before.revision
    );
    assert_eq!(store.inner.calls().len(), calls);
    assert_eq!(session.record(), before);
}

#[test]
fn invalid_path_time_and_core_metadata_bounds_fail_before_publication() {
    let (session, store, _) = fixture(Some(known()), EngineLimits::default());
    let calls = store.inner.calls().len();
    for path in ["relative", "/a/../b", "/nul\0", "/b/"] {
        assert!(mutate(&session, Path::new(path), 110).is_err());
    }
    assert_eq!(
        mutate(&session, Path::new("/b"), 99),
        Err(NativeSessionMetadataMutationError::InvalidMetadata(
            NativeSessionMetadataError::TimeRegression
        ))
    );
    assert_eq!(store.inner.calls().len(), calls);
    let limits = EngineLimits {
        max_session_metadata_bytes: NonZeroUsize::new(512).unwrap(),
        ..EngineLimits::default()
    };
    let (session, store, _) = fixture(None, limits);
    let before = session.record();
    let calls = store.inner.calls().len();
    assert_eq!(
        mutate(&session, Path::new(&format!("/{}", "x".repeat(400))), 110),
        Err(NativeSessionMetadataMutationError::Engine)
    );
    assert_eq!(store.inner.calls().len(), calls);
    assert_eq!(session.record(), before);
}

#[test]
fn malformed_history_is_not_replaced_even_by_a_no_op() {
    let malformed = [
        Value::Null,
        json!({}),
        json!({"schema_version":2,"entries":[]}),
        json!({"schema_version":1,"entries":[],"extra":0}),
        json!({"schema_version":1,"entries":[{}]}),
        json!({"schema_version":1,"entries":[{"previous_workspace_hex":"2f61","next_workspace_hex":"2f61","at_ms":110,"previous_revision":1}]}),
    ];
    for value in malformed {
        let (session, store, _) = fixture(Some(known()), EngineLimits::default());
        let mut record = session.record();
        record
            .metadata
            .insert(NATIVE_WORKSPACE_REBINDINGS_KEY.into(), value);
        block_on(session.update_metadata(record.revision, record.metadata)).unwrap();
        let before = session.record();
        let calls = store.inner.calls().len();
        assert_eq!(
            mutate(&session, Path::new("/a"), 110),
            Err(NativeSessionMetadataMutationError::InvalidHistory)
        );
        assert_eq!(store.inner.calls().len(), calls);
        assert_eq!(session.record(), before);
    }
}

#[test]
fn history_codec_rejects_chain_time_revision_and_field_corruption() {
    let (session, _, _) = fixture(Some(known()), EngineLimits::default());
    mutate(&session, Path::new("/b"), 110).unwrap();
    mutate(&session, Path::new("/c"), 120).unwrap();
    let original = history(&session).to_value();
    for (field, value) in [
        ("previous_workspace_hex", json!("2f78")),
        ("next_workspace_hex", json!("2F")),
        ("at_ms", json!(109)),
        ("previous_revision", json!(1)),
        ("unexpected", json!([])),
    ] {
        let mut broken = original.clone();
        broken["entries"][1][field] = value;
        assert!(
            NativeWorkspaceRebindingHistory::from_metadata(&BTreeMap::from([(
                NATIVE_WORKSPACE_REBINDINGS_KEY.into(),
                broken
            )]))
            .is_err()
        );
    }
    let mut excessive = original;
    excessive["entries"] = json!(vec![Value::Null; 65]);
    assert_eq!(
        NativeWorkspaceRebindingHistory::from_metadata(&BTreeMap::from([(
            NATIVE_WORKSPACE_REBINDINGS_KEY.into(),
            excessive
        )])),
        Err(NativeSessionMetadataMutationError::HistoryLimit)
    );
}

#[test]
fn revision_overflow_fails_before_store_but_checked_no_op_remains_valid() {
    let (session, store, _) = fixture_at_revision(
        Some(known()),
        EngineLimits::default(),
        SessionRevision(u64::MAX),
    );
    let calls = store.inner.calls().len();
    assert_eq!(
        mutate(&session, Path::new("/b"), 110),
        Err(NativeSessionMetadataMutationError::Engine)
    );
    assert_eq!(
        mutate(&session, Path::new("/a"), 0).unwrap(),
        SessionRevision(u64::MAX)
    );
    assert_eq!(store.inner.calls().len(), calls);
    assert!(history(&session).entries().is_empty());
}

#[test]
fn external_store_cas_change_does_not_overwrite_or_append_rebind() {
    let (session, store, _) = fixture(Some(known()), EngineLimits::default());
    let mut foreign = session.record();
    foreign.metadata.insert("foreign".into(), json!("preserve"));
    block_on(store.inner.save(foreign, Some(SessionRevision(1)))).unwrap();
    let durable = store.inner.record(&session.id()).unwrap();
    assert_eq!(
        mutate(&session, Path::new("/b"), 110),
        Err(NativeSessionMetadataMutationError::Conflict)
    );
    assert_eq!(store.inner.record(&session.id()).unwrap(), durable);
    assert!(
        NativeWorkspaceRebindingHistory::from_metadata(&durable.metadata)
            .unwrap()
            .entries()
            .is_empty()
    );
}

#[test]
fn valid_chain_must_match_current_workspace_time_and_revision() {
    for field in ["workspace", "time", "creation", "revision"] {
        let (session, store, _) = fixture(Some(known()), EngineLimits::default());
        mutate(&session, Path::new("/b"), 110).unwrap();
        let mut record = session.record();
        match field {
            "workspace" => {
                record
                    .metadata
                    .get_mut(NATIVE_SESSION_METADATA_KEY)
                    .unwrap()["workspace_hex"] = json!("2f63");
            }
            "time" => {
                record
                    .metadata
                    .get_mut(NATIVE_SESSION_METADATA_KEY)
                    .unwrap()["updated_at_ms"] = json!(105);
            }
            "creation" => {
                record
                    .metadata
                    .get_mut(NATIVE_WORKSPACE_REBINDINGS_KEY)
                    .unwrap()["entries"][0]["at_ms"] = json!(99);
            }
            _ => {
                record
                    .metadata
                    .get_mut(NATIVE_WORKSPACE_REBINDINGS_KEY)
                    .unwrap()["entries"][0]["previous_revision"] = json!(3);
            }
        }
        block_on(session.update_metadata(record.revision, record.metadata)).unwrap();
        let before = session.record();
        let calls = store.inner.calls().len();
        assert_eq!(
            mutate(&session, Path::new("/d"), 120),
            Err(NativeSessionMetadataMutationError::InvalidHistory)
        );
        assert_eq!(store.inner.calls().len(), calls);
        assert_eq!(session.record(), before);
    }
}
