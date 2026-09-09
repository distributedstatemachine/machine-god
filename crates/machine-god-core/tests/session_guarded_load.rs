use std::collections::VecDeque;
use std::future::{Future, poll_fn};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use futures_executor::block_on;
use machine_god_core::{
    BoxFuture, CancellationToken, Engine, EngineError, EngineLimits, Message, ModelEventStream,
    ModelProvider, ModelRequest, PermissionDecision, PermissionError, PermissionHandler,
    PermissionRequest, ProviderError, Role, Session, SessionId, SessionIncarnationId,
    SessionRecord, SessionRevision, SessionStore, SessionStoreError, SessionStoreErrorKind,
};
use serde_json::{Value, json};

type LoadResult = Result<Option<SessionRecord>, SessionStoreError>;

#[derive(Default)]
struct Store {
    responses: Mutex<VecDeque<LoadResult>>,
    loads: AtomicUsize,
    saves: AtomicUsize,
    delay_next: AtomicBool,
    release: AtomicBool,
}

impl Store {
    fn push(&self, record: SessionRecord) {
        self.responses.lock().unwrap().push_back(Ok(Some(record)));
    }
}

impl SessionStore for Store {
    fn load(&self, _: SessionId) -> BoxFuture<'_, LoadResult> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(None));
        let delayed = self.delay_next.swap(false, Ordering::SeqCst);
        Box::pin(async move {
            if delayed {
                poll_fn(|_| {
                    if self.release.load(Ordering::SeqCst) {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
                .await;
            }
            response
        })
    }

    fn save(
        &self,
        record: SessionRecord,
        _: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(SessionRevision(record.revision.0 + 1)) })
    }
}

struct NoEffects;

impl ModelProvider for NoEffects {
    fn name(&self) -> &'static str {
        "guarded-load-no-effects"
    }

    fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        panic!("load must not invoke the provider")
    }
}

impl PermissionHandler for NoEffects {
    fn authorize(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        panic!("load must not invoke permission handling")
    }
}

fn engine(store: &Arc<Store>) -> Engine {
    Engine::builder()
        .provider(NoEffects)
        .permission_handler(NoEffects)
        .shared_session_store(store.clone())
        .build()
        .unwrap()
}

struct Access {
    underlying: Arc<dyn SessionStore>,
    calls: AtomicUsize,
}
impl machine_god_core::SessionStoreAccess for Access {
    fn underlying_store(&self) -> &Arc<dyn SessionStore> {
        &self.underlying
    }
}
impl SessionStore for Access {
    fn load(&self, id: SessionId) -> BoxFuture<'_, LoadResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.underlying.load(id)
    }
    fn save(
        &self,
        record: SessionRecord,
        revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.underlying.save(record, revision)
    }
}

#[test]
fn explicit_store_access_is_inert_identity_checked_and_never_installed_as_default() {
    let store = Arc::new(Store::default());
    let engine = engine(&store);
    let saved = record(1);
    let access = Arc::new(Access {
        underlying: store.clone(),
        calls: AtomicUsize::new(0),
    });
    let wrong = Arc::new(Access {
        underlying: Arc::new(Store::default()),
        calls: AtomicUsize::new(0),
    });
    let load = |access: Arc<Access>| {
        engine.requester().load_session_at_revision_with_access(
            saved.id.clone(),
            saved.incarnation_id.clone(),
            saved.revision,
            access,
        )
    };
    drop(load(access.clone()));
    assert_eq!(access.calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        block_on(load(wrong.clone())),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(wrong.calls.load(Ordering::SeqCst), 0);
    store.push(saved.clone());
    let session = block_on(load(access.clone())).unwrap().unwrap();
    assert_eq!(access.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        block_on(session.update_metadata_with_access(
            saved.revision,
            std::collections::BTreeMap::default(),
            wrong.clone()
        )),
        Err(EngineError::Protocol(_))
    ));
    assert!(matches!(
        block_on(session.check_metadata_revision_with_access(saved.revision, wrong.clone())),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(wrong.calls.load(Ordering::SeqCst), 0);
    store.push(saved.clone());
    let ordinary = block_on(engine.load_session(saved.id)).unwrap().unwrap();
    assert_eq!(ordinary.record(), session.record());
    assert_eq!(access.calls.load(Ordering::SeqCst), 1);
}

fn record(revision: u64) -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new("guarded-session").unwrap(),
        SessionIncarnationId::new("guarded-incarnation").unwrap(),
    );
    record.revision = SessionRevision(revision);
    record.metadata.insert("retained".into(), json!(revision));
    record
        .messages
        .push(Message::text(Role::User, "retained message"));
    record
}

fn guarded(
    engine: &Engine,
    expected: &SessionRecord,
) -> BoxFuture<'static, Result<Option<Session>, EngineError>> {
    engine.load_session_at_revision(
        expected.id.clone(),
        expected.incarnation_id.clone(),
        expected.revision,
    )
}

fn poll<T>(future: &mut (impl Future<Output = T> + Unpin)) -> Poll<T> {
    std::pin::Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
}

fn assert_conflict(error: &EngineError) {
    assert_eq!(
        error,
        &EngineError::Store(SessionStoreError::new(
            SessionStoreErrorKind::Conflict,
            "store_failed",
            "session store failed",
            false,
        ))
    );
}

#[test]
fn both_entry_points_are_inert_and_matching_loads_converge() {
    let store = Arc::new(Store::default());
    let engine = engine(&store);
    let expected = record(3);
    let canonical = engine
        .create_session(expected.id.clone(), expected.incarnation_id.clone())
        .unwrap();
    let unpolled = guarded(&engine, &expected);
    drop(unpolled);
    let mut load = engine.requester().load_session_at_revision(
        expected.id.clone(),
        expected.incarnation_id.clone(),
        expected.revision,
    );
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
    store.push(expected.clone());
    let loaded = match poll(&mut load) {
        Poll::Ready(Ok(Some(session))) => session,
        other => panic!("unexpected load: {other:?}"),
    };
    assert_eq!(loaded.record(), expected);
    assert_eq!(canonical.record(), expected);
    let newer = record(4);
    store.push(newer.clone());
    let second = block_on(guarded(&engine, &newer)).unwrap().unwrap();
    assert_eq!(second.record(), newer);
    assert_eq!(loaded.record(), newer);
    assert_eq!(canonical.record(), newer);
    assert_eq!(store.saves.load(Ordering::SeqCst), 0);
}

#[test]
fn rejected_revision_or_incarnation_never_reconciles_live_canonical() {
    for wrong_incarnation in [false, true] {
        let store = Arc::new(Store::default());
        let engine = engine(&store);
        let expected = record(3);
        store.push(expected.clone());
        let canonical = block_on(engine.load_session(expected.id.clone()))
            .unwrap()
            .unwrap();
        let mut changed = record(4);
        if wrong_incarnation {
            changed.incarnation_id = SessionIncarnationId::new("replacement-secret").unwrap();
        }
        store.push(changed);
        let error = block_on(guarded(&engine, &expected)).unwrap_err();
        if wrong_incarnation {
            assert_eq!(error, EngineError::SessionIncarnationConflict);
        } else {
            assert_conflict(&error);
        }
        assert_eq!(canonical.record(), expected);
        assert_eq!(store.saves.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn missing_and_rejected_loads_do_not_publish_registry_state() {
    let store = Arc::new(Store::default());
    let engine = engine(&store);
    let expected = record(3);
    assert!(block_on(guarded(&engine, &expected)).unwrap().is_none());
    store.push(record(4));
    assert_conflict(&block_on(guarded(&engine, &expected)).unwrap_err());
    let created = engine
        .create_session(
            expected.id,
            SessionIncarnationId::new("fresh-lifetime").unwrap(),
        )
        .unwrap();
    assert_eq!(created.record().revision, SessionRevision(0));
    assert!(created.record().metadata.is_empty());
}

#[test]
fn matching_guarded_load_rejects_live_turn_without_reconciling() {
    let store = Arc::new(Store::default());
    let engine = engine(&store);
    let initial = record(3);
    store.push(initial.clone());
    let canonical = block_on(guarded(&engine, &initial)).unwrap().unwrap();
    let turn = block_on(canonical.prompt("hold admission")).unwrap();
    let active = canonical.record();
    let mut newer = active.clone();
    newer.revision = SessionRevision(active.revision.0 + 1);
    newer.metadata.insert("later".into(), json!(true));
    store.push(newer.clone());
    assert!(matches!(
        block_on(guarded(&engine, &newer)),
        Err(EngineError::SessionBusy)
    ));
    assert_eq!(canonical.record(), active);
    assert!(canonical.has_active_turn());
    // Ordinary load retains its historical behavior even while the turn lives.
    store.push(active.clone());
    let ordinary = block_on(engine.load_session(active.id.clone()))
        .unwrap()
        .unwrap();
    assert!(ordinary.has_active_turn());
    assert!(matches!(
        block_on(ordinary.prompt("cannot acquire")),
        Err(EngineError::SessionBusy)
    ));
    drop(turn);
    store.push(newer.clone());
    assert_eq!(
        block_on(guarded(&engine, &newer))
            .unwrap()
            .unwrap()
            .record(),
        newer
    );
    assert!(!canonical.has_active_turn());
}

#[test]
fn turn_started_while_store_waits_blocks_guarded_reconciliation() {
    let store = Arc::new(Store::default());
    let engine = engine(&store);
    let expected = record(3);
    store.push(expected.clone());
    let canonical = block_on(guarded(&engine, &expected)).unwrap().unwrap();
    store.push(expected.clone());
    store.delay_next.store(true, Ordering::SeqCst);
    let mut pending = guarded(&engine, &expected);
    assert!(poll(&mut pending).is_pending());
    let turn = block_on(canonical.prompt("wins admission before load returns")).unwrap();
    let active = canonical.record();
    store.release.store(true, Ordering::SeqCst);
    assert!(matches!(block_on(pending), Err(EngineError::SessionBusy)));
    assert_eq!(canonical.record(), active);
    drop(turn);
    assert!(!canonical.has_active_turn());
}

#[test]
fn delayed_changed_store_result_is_checked_before_reconciliation() {
    let store = Arc::new(Store::default());
    let engine = engine(&store);
    let expected = record(3);
    store.push(expected.clone());
    let canonical = block_on(guarded(&engine, &expected)).unwrap().unwrap();
    store.push(record(4));
    store.delay_next.store(true, Ordering::SeqCst);
    let mut pending = guarded(&engine, &expected);
    assert!(poll(&mut pending).is_pending());
    assert_eq!(canonical.record(), expected);
    store.release.store(true, Ordering::SeqCst);
    assert_conflict(&block_on(pending).unwrap_err());
    assert_eq!(canonical.record(), expected);
}

#[test]
fn delayed_matching_load_cannot_rewind_concurrently_advanced_canonical() {
    let store = Arc::new(Store::default());
    let engine = engine(&store);
    let expected = record(3);
    store.push(expected.clone());
    let canonical = block_on(guarded(&engine, &expected)).unwrap().unwrap();
    store.push(expected.clone());
    store.delay_next.store(true, Ordering::SeqCst);
    let mut pending = guarded(&engine, &expected);
    assert!(poll(&mut pending).is_pending());
    let newer = record(4);
    store.push(newer.clone());
    let advanced = block_on(engine.load_session(expected.id)).unwrap().unwrap();
    store.release.store(true, Ordering::SeqCst);
    assert!(matches!(block_on(pending), Err(EngineError::Protocol(_))));
    assert_eq!(canonical.record(), newer);
    assert_eq!(advanced.record(), newer);
}

struct HostDrop(Arc<AtomicUsize>);

impl Drop for HostDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn unpolled_and_pending_guarded_loads_cannot_resurrect_host() {
    for pending in [false, true] {
        let store = Arc::new(Store::default());
        let drops = Arc::new(AtomicUsize::new(0));
        let engine = Engine::builder()
            .provider(NoEffects)
            .permission_handler(NoEffects)
            .shared_session_store(store.clone())
            .host_resource(HostDrop(drops.clone()))
            .build()
            .unwrap();
        let expected = record(3);
        store.push(expected.clone());
        store.delay_next.store(true, Ordering::SeqCst);
        let requester = engine.requester();
        let mut load = requester.load_session_at_revision(
            expected.id,
            expected.incarnation_id,
            expected.revision,
        );
        if pending {
            assert!(poll(&mut load).is_pending());
        }
        drop(engine);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        store.release.store(true, Ordering::SeqCst);
        assert!(matches!(block_on(load), Err(EngineError::HostClosed)));
        assert_eq!(store.loads.load(Ordering::SeqCst), usize::from(pending));
        assert_eq!(store.saves.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn store_failures_are_redacted_and_ordinary_validation_still_applies() {
    let store = Arc::new(Store::default());
    let engine = engine(&store);
    let expected = record(3);
    store
        .responses
        .lock()
        .unwrap()
        .push_back(Err(SessionStoreError::new(
            SessionStoreErrorKind::Unavailable,
            "secret-code",
            "secret-detail",
            true,
        )));
    assert_eq!(
        block_on(guarded(&engine, &expected)).unwrap_err(),
        EngineError::Store(SessionStoreError::new(
            SessionStoreErrorKind::Unavailable,
            "store_failed",
            "session store failed",
            true,
        ))
    );
    let mut malformed = expected.clone();
    malformed.next_turn_sequence = 0;
    let mut wrong_id = expected.clone();
    wrong_id.id = SessionId::new("wrong-id").unwrap();
    let mut zero_revision = expected.clone();
    zero_revision.revision = SessionRevision(0);
    let mut divergent = expected.clone();
    divergent.metadata.insert("changed".into(), json!(true));
    store.push(expected.clone());
    let canonical = block_on(guarded(&engine, &expected)).unwrap().unwrap();
    for invalid in [malformed, wrong_id, zero_revision, divergent] {
        store.push(invalid);
        assert!(matches!(
            block_on(guarded(&engine, &expected)),
            Err(EngineError::Protocol(_))
        ));
        assert_eq!(canonical.record(), expected);
    }
}

#[test]
fn rejected_deep_and_oversized_records_are_bounded_and_dropped_iteratively() {
    let store = Arc::new(Store::default());
    let engine = engine(&store);
    let expected = record(3);
    let mut deep = Value::Null;
    for _ in 0..20_000 {
        deep = Value::Array(vec![deep]);
    }
    let mut invalid = expected.clone();
    invalid.metadata.insert("deep".into(), deep);
    store.push(invalid);
    assert!(matches!(
        block_on(guarded(&engine, &expected)),
        Err(EngineError::Protocol(_))
    ));
    let mut oversized = expected.clone();
    oversized.metadata.insert(
        "large".into(),
        Value::String("x".repeat(EngineLimits::default().max_session_metadata_bytes.get())),
    );
    store.push(oversized);
    assert!(matches!(
        block_on(guarded(&engine, &expected)),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(store.saves.load(Ordering::SeqCst), 0);
}
