use std::collections::{BTreeMap, VecDeque};
use std::future::poll_fn;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_executor::block_on;
use futures_util::task::noop_waker;
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Engine, EngineError, EngineLimits, Message,
    ModelEventStream, ModelProvider, ModelRequest, PermissionDecision, PermissionError,
    PermissionHandler, PermissionRequest, ProviderError, Role, Session, SessionId,
    SessionIncarnationId, SessionRecord, SessionRevision, SessionStore, SessionStoreError,
    SessionStoreErrorKind, ToolCall, ToolCallId, ToolName, ToolOutput,
};
use serde_json::{Value, json};

#[derive(Clone, Copy, Default)]
enum SaveBehavior {
    #[default]
    Commit,
    WaitBeforeCommit,
    CommitThenWait,
    CommitThenError,
    ReturnRevision(SessionRevision),
    Fail,
}

#[derive(Default)]
struct Store {
    record: Mutex<Option<SessionRecord>>,
    saves: Mutex<VecDeque<SaveBehavior>>,
    loads: Mutex<VecDeque<Result<Option<SessionRecord>, SessionStoreError>>>,
    save_calls: AtomicUsize,
    save_polls: AtomicUsize,
    save_drops: AtomicUsize,
    load_calls: AtomicUsize,
    load_drops: AtomicUsize,
    wait_load: AtomicBool,
    release: AtomicBool,
}

impl Store {
    fn record(&self) -> Option<SessionRecord> {
        self.record.lock().unwrap().clone()
    }

    fn behavior(&self, behavior: SaveBehavior) {
        self.saves.lock().unwrap().push_back(behavior);
    }

    fn override_load(&self, record: Option<SessionRecord>) {
        self.loads.lock().unwrap().push_back(Ok(record));
    }

    async fn wait(&self) {
        poll_fn(|_| {
            if self.release.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
    }
}

struct SaveDrop<'a>(&'a AtomicUsize);

impl Drop for SaveDrop<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn failure(kind: SessionStoreErrorKind) -> SessionStoreError {
    SessionStoreError::new(kind, "sensitive-code", "sensitive-detail", true)
}

impl SessionStore for Store {
    fn load(
        &self,
        _id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            let _drop = SaveDrop(&self.load_drops);
            if self.wait_load.load(Ordering::Acquire) {
                self.wait().await;
            }
            self.loads
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(self.record()))
        })
    }

    fn save(
        &self,
        mut record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        self.save_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            self.save_polls.fetch_add(1, Ordering::SeqCst);
            let _drop = SaveDrop(&self.save_drops);
            let behavior = self.saves.lock().unwrap().pop_front().unwrap_or_default();
            if let SaveBehavior::ReturnRevision(revision) = behavior {
                return Ok(revision);
            }
            if matches!(behavior, SaveBehavior::Fail) {
                return Err(failure(SessionStoreErrorKind::Unavailable));
            }
            if matches!(behavior, SaveBehavior::WaitBeforeCommit) {
                self.wait().await;
            }
            let revision = {
                let mut current = self.record.lock().unwrap();
                if current.as_ref().map(|record| record.revision) != expected_revision {
                    return Err(failure(SessionStoreErrorKind::Conflict));
                }
                record.revision = SessionRevision(record.revision.0 + 1);
                let revision = record.revision;
                *current = Some(record);
                revision
            };
            if matches!(behavior, SaveBehavior::CommitThenWait) {
                self.wait().await;
            }
            if matches!(behavior, SaveBehavior::CommitThenError) {
                return Err(failure(SessionStoreErrorKind::Unavailable));
            }
            Ok(revision)
        })
    }
}

struct NoEffects;

impl ModelProvider for NoEffects {
    fn name(&self) -> &'static str {
        "no-effects"
    }

    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        panic!("metadata operation must not call the provider")
    }
}

impl PermissionHandler for NoEffects {
    fn authorize(
        &self,
        _request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        panic!("metadata operation must not call permission policy")
    }
}

fn engine(store: Arc<Store>, limits: EngineLimits) -> Engine {
    Engine::builder()
        .provider(NoEffects)
        .permission_handler(NoEffects)
        .shared_session_store(store)
        .limits(limits)
        .build()
        .unwrap()
}

fn record() -> SessionRecord {
    let call_id = ToolCallId::new("retained-call").unwrap();
    SessionRecord {
        id: SessionId::new("metadata-session").unwrap(),
        incarnation_id: SessionIncarnationId::new("metadata-incarnation").unwrap(),
        revision: SessionRevision(7),
        next_turn_sequence: 12,
        messages: vec![
            Message::text(Role::User, "retained prompt"),
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolCall {
                    call: ToolCall {
                        id: call_id.clone(),
                        name: ToolName::new("retained-tool").unwrap(),
                        arguments: json!({"path": "evidence"}),
                    },
                }],
            },
            Message {
                role: Role::Tool,
                content: vec![ContentBlock::ToolResult {
                    call_id,
                    output: ToolOutput::success(json!({"confirmed": true})),
                }],
            },
            Message::text(Role::Assistant, "retained answer"),
        ],
        metadata: metadata("old"),
    }
}

fn metadata(value: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([("host.preference".to_owned(), json!(value))])
}

fn loaded() -> (Engine, Session, Arc<Store>) {
    let store = Arc::new(Store::default());
    *store.record.lock().unwrap() = Some(record());
    let engine = engine(Arc::clone(&store), EngineLimits::default());
    let session = block_on(engine.load_session(record().id)).unwrap().unwrap();
    (engine, session, store)
}

#[test]
fn canonical_tool_call_locator_is_read_only_exact_and_cursor_bounded() {
    let (_engine, session, store) = loaded();
    let before = session.record();
    let name = ToolName::new("retained-tool").unwrap();
    let id = ToolCallId::new("retained-call").unwrap();
    assert_eq!(session.find_tool_call((0, 0), &name, &id), Some((1, 0)));
    assert_eq!(session.find_tool_call((1, 0), &name, &id), Some((1, 0)));
    assert_eq!(session.find_tool_call((1, 1), &name, &id), None);
    assert_eq!(session.find_tool_call((0, usize::MAX), &name, &id), None);
    assert_eq!(session.find_tool_call((usize::MAX, 0), &name, &id), None);
    assert_eq!(
        session.find_tool_call((0, 0), &ToolName::new("other").unwrap(), &id),
        None
    );
    assert_eq!(
        session.find_tool_call((0, 0), &name, &ToolCallId::new("other").unwrap()),
        None
    );
    assert_eq!(session.record(), before);
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn canonical_tool_call_locator_distinguishes_reused_historical_ids() {
    let mut initial = record();
    let mut later_call = initial.messages[1].clone();
    later_call.content.insert(
        0,
        ContentBlock::Text {
            text: "before".to_owned(),
        },
    );
    initial
        .messages
        .push(Message::text(Role::User, "next prompt"));
    initial.messages.push(later_call);
    initial.messages.push(initial.messages[2].clone());
    let store = Arc::new(Store::default());
    *store.record.lock().unwrap() = Some(initial.clone());
    let engine = engine(store, EngineLimits::default());
    let session = block_on(engine.load_session(initial.id)).unwrap().unwrap();
    let name = ToolName::new("retained-tool").unwrap();
    let id = ToolCallId::new("retained-call").unwrap();
    assert_eq!(session.find_tool_call((0, 0), &name, &id), Some((1, 0)));
    assert_eq!(session.find_tool_call((1, 1), &name, &id), Some((5, 1)));
    assert_eq!(session.find_tool_call((5, 1), &name, &id), Some((5, 1)));
    assert_eq!(session.find_tool_call((5, 2), &name, &id), None);
}

fn pending<T>(future: &mut BoxFuture<'_, T>) {
    let waker = noop_waker();
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
}

fn assert_conflict(result: Result<SessionRevision, EngineError>) {
    assert!(
        matches!(result, Err(EngineError::Store(error)) if error.kind == SessionStoreErrorKind::Conflict)
    );
}

#[test]
fn unpolled_edits_are_inert_and_success_preserves_all_nonmetadata_fields() {
    let (engine, session, store) = loaded();
    let clone = block_on(engine.load_session(session.id()))
        .unwrap()
        .unwrap();
    let before = session.record();
    let operation = session.update_metadata(before.revision, metadata("new"));
    assert!(!session.has_active_turn());
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.load_calls.load(Ordering::SeqCst), 2);
    assert_eq!(session.record(), before);
    assert_eq!(block_on(operation).unwrap(), SessionRevision(8));
    let mut expected = before;
    expected.revision = SessionRevision(8);
    expected.metadata = metadata("new");
    assert_eq!(session.record(), expected);
    assert_eq!(clone.record(), expected);
    assert_eq!(store.record().unwrap(), expected);
    assert!(!session.has_active_turn());
    assert_conflict(block_on(
        session.update_metadata(SessionRevision(7), metadata("stale")),
    ));
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn zero_revision_creates_metadata_for_a_genuinely_unsaved_session() {
    let store = Arc::new(Store::default());
    let engine = engine(Arc::clone(&store), EngineLimits::default());
    let session = engine
        .create_session(record().id, record().incarnation_id)
        .unwrap();
    let before = session.record();
    assert_eq!(
        block_on(session.update_metadata(SessionRevision(0), metadata("first"))).unwrap(),
        SessionRevision(1)
    );
    let mut expected = before;
    expected.revision = SessionRevision(1);
    expected.metadata = metadata("first");
    assert_eq!(session.record(), expected);
}

#[test]
fn metadata_and_prompt_share_one_lease_and_drop_releases_owned_store_work() {
    let (_, session, store) = loaded();
    store.behavior(SaveBehavior::WaitBeforeCommit);
    let mut operation = session.update_metadata(SessionRevision(7), metadata("pending"));
    pending(&mut operation);
    assert!(session.has_active_turn());
    assert!(matches!(
        block_on(session.clone().prompt("blocked")),
        Err(EngineError::SessionBusy)
    ));
    assert!(matches!(
        block_on(
            session
                .clone()
                .update_metadata(SessionRevision(7), metadata("blocked"))
        ),
        Err(EngineError::SessionBusy)
    ));
    drop(operation);
    assert_eq!(store.save_drops.load(Ordering::SeqCst), 1);
    assert!(!session.has_active_turn());
    assert_eq!(session.record(), record());
    assert_eq!(
        block_on(session.update_metadata(SessionRevision(7), metadata("after-drop"))).unwrap(),
        SessionRevision(8)
    );
    assert_eq!(store.load_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn live_turn_blocks_edits_and_its_drop_allows_them() {
    let (_, session, store) = loaded();
    let turn = block_on(session.prompt("next")).unwrap();
    let handle = turn.handle();
    assert!(matches!(
        block_on(session.update_metadata(SessionRevision(8), metadata("blocked"))),
        Err(EngineError::SessionBusy)
    ));
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 1);
    drop(turn);
    assert!(handle.is_cancelled());
    assert_eq!(
        block_on(session.update_metadata(SessionRevision(8), metadata("allowed"))).unwrap(),
        SessionRevision(9)
    );
}

#[test]
fn dropped_committed_save_cannot_be_overwritten_by_a_stale_patch() {
    let (_, session, store) = loaded();
    store.behavior(SaveBehavior::CommitThenWait);
    let mut operation = session.update_metadata(SessionRevision(7), metadata("committed"));
    pending(&mut operation);
    assert_eq!(session.record().metadata, metadata("old"));
    assert_eq!(store.record().unwrap().metadata, metadata("committed"));
    drop(operation);
    assert_conflict(block_on(
        session.update_metadata(SessionRevision(7), metadata("stale")),
    ));
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 1);
    assert_eq!(session.record().metadata, metadata("committed"));
    assert_eq!(session.record().revision, SessionRevision(8));
    assert!(!session.has_active_turn());
}

#[test]
fn ambiguous_error_is_redacted_and_next_prompt_preserves_committed_metadata() {
    let (_, session, store) = loaded();
    store.behavior(SaveBehavior::CommitThenError);
    let error =
        block_on(session.update_metadata(SessionRevision(7), metadata("committed"))).unwrap_err();
    assert!(
        matches!(&error, EngineError::Store(error) if error.code == "store_failed" && error.message == "session store failed")
    );
    let turn = block_on(session.prompt("next")).unwrap();
    assert_eq!(session.record().metadata, metadata("committed"));
    assert_eq!(session.record().next_turn_sequence, 13);
    assert_eq!(store.record().unwrap().metadata, metadata("committed"));
    assert_eq!(store.load_calls.load(Ordering::SeqCst), 2);
    drop(turn);
}

#[test]
fn conflict_does_not_retry_stale_edits_and_later_edit_reconciles() {
    let (_, session, store) = loaded();
    let mut external = record();
    external.revision = SessionRevision(8);
    external.metadata = metadata("external");
    *store.record.lock().unwrap() = Some(external.clone());
    assert_conflict(block_on(
        session.update_metadata(SessionRevision(7), metadata("stale")),
    ));
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.record().unwrap(), external);
    assert_eq!(
        block_on(session.update_metadata(SessionRevision(8), metadata("fresh"))).unwrap(),
        SessionRevision(9)
    );
    assert_eq!(store.load_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn invalid_save_revisions_force_reload_before_another_write() {
    for revision in [SessionRevision(0), SessionRevision(6), SessionRevision(7)] {
        let (_, session, store) = loaded();
        store.behavior(SaveBehavior::ReturnRevision(revision));
        assert!(matches!(
            block_on(session.update_metadata(SessionRevision(7), metadata("invalid"))),
            Err(EngineError::Protocol(_))
        ));
        assert_eq!(session.record(), record());
        assert_eq!(
            block_on(session.update_metadata(SessionRevision(7), metadata("valid"))).unwrap(),
            SessionRevision(8)
        );
        assert_eq!(store.load_calls.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn missing_persisted_record_after_uncertainty_never_becomes_a_new_record() {
    let (_, session, store) = loaded();
    store.behavior(SaveBehavior::Fail);
    assert!(block_on(session.update_metadata(SessionRevision(7), metadata("failed"))).is_err());
    *store.record.lock().unwrap() = None;
    assert!(matches!(
        block_on(session.prompt("must-not-recreate")),
        Err(EngineError::Protocol(_))
    ));
    assert!(matches!(
        block_on(session.update_metadata(SessionRevision(7), metadata("must-not-recreate"))),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.load_calls.load(Ordering::SeqCst), 3);
    assert!(store.record().is_none());
    assert!(!session.has_active_turn());
}

#[test]
fn dropped_unsaved_edit_can_retry_only_after_observing_absence() {
    let store = Arc::new(Store::default());
    let engine = engine(Arc::clone(&store), EngineLimits::default());
    let session = engine
        .create_session(record().id, record().incarnation_id)
        .unwrap();
    store.behavior(SaveBehavior::WaitBeforeCommit);
    let mut operation = session.update_metadata(SessionRevision(0), metadata("pending"));
    pending(&mut operation);
    drop(operation);
    assert_eq!(
        block_on(session.update_metadata(SessionRevision(0), metadata("fresh"))).unwrap(),
        SessionRevision(1)
    );
    assert_eq!(store.load_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn uncertain_reload_rejects_invalid_identity_revision_allocator_and_content() {
    let mut invalid_records = Vec::new();
    let mut invalid = record();
    invalid.id = SessionId::new("another").unwrap();
    invalid_records.push(invalid);
    let mut invalid = record();
    invalid.incarnation_id = SessionIncarnationId::new("another").unwrap();
    invalid_records.push(invalid);
    for revision in [0, 6] {
        let mut invalid = record();
        invalid.revision = SessionRevision(revision);
        invalid_records.push(invalid);
    }
    for sequence in [0, 11] {
        let mut invalid = record();
        invalid.revision = SessionRevision(8);
        invalid.next_turn_sequence = sequence;
        invalid_records.push(invalid);
    }
    let mut invalid = record();
    invalid.metadata = metadata("divergent-at-same-revision");
    invalid_records.push(invalid);
    let mut invalid = record();
    invalid.metadata = BTreeMap::from([("deep".to_owned(), deep_value(10_000))]);
    invalid_records.push(invalid);
    for invalid in invalid_records {
        let (_, session, store) = loaded();
        store.behavior(SaveBehavior::Fail);
        assert!(block_on(session.update_metadata(SessionRevision(7), metadata("failed"))).is_err());
        store.override_load(Some(invalid));
        assert!(block_on(session.prompt("blocked")).is_err());
        assert_eq!(session.record(), record());
        assert_eq!(store.save_calls.load(Ordering::SeqCst), 1);
        assert!(!session.has_active_turn());
        assert_eq!(
            block_on(session.update_metadata(SessionRevision(7), metadata("recovered"))).unwrap(),
            SessionRevision(8)
        );
        assert_eq!(store.load_calls.load(Ordering::SeqCst), 3);
    }
}

fn deep_value(depth: usize) -> Value {
    let mut value = Value::Null;
    for _ in 0..depth {
        value = Value::Array(vec![value]);
    }
    value
}

#[test]
fn hostile_metadata_is_drained_on_unpolled_drop_busy_and_validation_failure() {
    let (_, session, store) = loaded();
    drop(session.update_metadata(
        SessionRevision(7),
        BTreeMap::from([("deep".to_owned(), deep_value(50_000))]),
    ));
    assert!(matches!(
        block_on(session.update_metadata(
            SessionRevision(7),
            BTreeMap::from([("deep".to_owned(), deep_value(50_000))])
        )),
        Err(EngineError::Protocol(_))
    ));
    let turn = block_on(session.prompt("busy")).unwrap();
    assert!(matches!(
        block_on(session.update_metadata(
            SessionRevision(8),
            BTreeMap::from([("deep".to_owned(), deep_value(50_000))])
        )),
        Err(EngineError::SessionBusy)
    ));
    drop(turn);
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn byte_and_aggregate_json_node_limits_apply_before_save() {
    for limits in [
        EngineLimits {
            max_session_metadata_bytes: NonZeroUsize::new(32).unwrap(),
            ..EngineLimits::default()
        },
        EngineLimits {
            max_json_nodes: NonZeroUsize::new(4).unwrap(),
            ..EngineLimits::default()
        },
    ] {
        let store = Arc::new(Store::default());
        let engine = engine(Arc::clone(&store), limits);
        let session = engine
            .create_session(record().id, record().incarnation_id)
            .unwrap();
        let values = BTreeMap::from([
            (
                "one".to_owned(),
                json!(["long-enough-to-exceed-metadata-limit", 2]),
            ),
            ("two".to_owned(), json!([3, 4])),
        ]);
        assert!(matches!(
            block_on(session.update_metadata(SessionRevision(0), values)),
            Err(EngineError::Protocol(_))
        ));
        assert_eq!(store.save_calls.load(Ordering::SeqCst), 0);
        assert!(!session.has_active_turn());
    }
}

#[test]
fn a_delayed_success_cannot_rewind_a_newer_canonical_load() {
    let (engine, session, store) = loaded();
    store.behavior(SaveBehavior::CommitThenWait);
    let mut operation = session.update_metadata(SessionRevision(7), metadata("committed"));
    pending(&mut operation);
    let mut newer = store.record().unwrap();
    newer.revision = SessionRevision(9);
    newer.metadata = metadata("newer");
    *store.record.lock().unwrap() = Some(newer.clone());
    let loaded = block_on(engine.load_session(session.id()))
        .unwrap()
        .unwrap();
    store.release.store(true, Ordering::Release);
    assert_eq!(block_on(operation).unwrap(), SessionRevision(8));
    assert_eq!(session.record(), newer);
    assert_eq!(loaded.record(), newer);
    assert!(!session.has_active_turn());
}

#[test]
fn unpolled_metadata_future_does_not_keep_host_authority_alive() {
    struct Resource(Arc<AtomicBool>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let dropped = Arc::new(AtomicBool::new(false));
    let store = Arc::new(Store::default());
    let engine = Engine::builder()
        .provider(NoEffects)
        .permission_handler(NoEffects)
        .shared_session_store(store.clone())
        .host_resource(Resource(Arc::clone(&dropped)))
        .build()
        .unwrap();
    let session = engine
        .create_session(record().id, record().incarnation_id)
        .unwrap();
    let operation = session.update_metadata(SessionRevision(0), metadata("never"));
    drop(session);
    drop(engine);
    assert!(dropped.load(Ordering::Acquire));
    assert!(matches!(block_on(operation), Err(EngineError::HostClosed)));
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn dropped_reconciliation_load_keeps_the_requirement_armed_and_releases_lease() {
    let (_, session, store) = loaded();
    store.behavior(SaveBehavior::CommitThenError);
    assert!(block_on(session.update_metadata(SessionRevision(7), metadata("committed"))).is_err());
    store.wait_load.store(true, Ordering::Release);
    let mut retry = session.update_metadata(SessionRevision(8), metadata("fresh"));
    pending(&mut retry);
    assert!(session.has_active_turn());
    assert!(matches!(
        block_on(session.prompt("busy")),
        Err(EngineError::SessionBusy)
    ));
    drop(retry);
    assert!(!session.has_active_turn());
    assert_eq!(store.load_drops.load(Ordering::SeqCst), 2);
    store.wait_load.store(false, Ordering::Release);
    assert_conflict(block_on(
        session.update_metadata(SessionRevision(7), metadata("stale")),
    ));
    assert_eq!(store.load_calls.load(Ordering::SeqCst), 3);
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 1);
    assert_eq!(session.record().metadata, metadata("committed"));
}

#[test]
fn reconciliation_store_failure_is_redacted_and_does_not_disarm_refresh() {
    let (_, session, store) = loaded();
    store.behavior(SaveBehavior::CommitThenError);
    assert!(block_on(session.update_metadata(SessionRevision(7), metadata("committed"))).is_err());
    store
        .loads
        .lock()
        .unwrap()
        .push_back(Err(failure(SessionStoreErrorKind::Corrupt)));
    let error = block_on(session.prompt("blocked")).unwrap_err();
    assert!(
        matches!(error, EngineError::Store(error) if error.kind == SessionStoreErrorKind::Corrupt && error.code == "store_failed" && error.message == "session store failed")
    );
    assert_conflict(block_on(
        session.update_metadata(SessionRevision(7), metadata("stale")),
    ));
    assert_eq!(store.load_calls.load(Ordering::SeqCst), 3);
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn metadata_byte_boundary_is_inclusive() {
    let bound = serde_json::to_vec(&metadata("a")).unwrap().len();
    let store = Arc::new(Store::default());
    let engine = engine(
        Arc::clone(&store),
        EngineLimits {
            max_session_metadata_bytes: NonZeroUsize::new(bound).unwrap(),
            ..EngineLimits::default()
        },
    );
    let session = engine
        .create_session(record().id, record().incarnation_id)
        .unwrap();
    assert_eq!(
        block_on(session.update_metadata(SessionRevision(0), metadata("a"))).unwrap(),
        SessionRevision(1)
    );
    assert!(matches!(
        block_on(session.update_metadata(SessionRevision(1), metadata("aa"))),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn metadata_and_retained_transcript_share_the_json_node_budget() {
    let store = Arc::new(Store::default());
    *store.record.lock().unwrap() = Some(record());
    let engine = engine(
        Arc::clone(&store),
        EngineLimits {
            max_json_nodes: NonZeroUsize::new(5).unwrap(),
            ..EngineLimits::default()
        },
    );
    let session = block_on(engine.load_session(record().id)).unwrap().unwrap();
    assert!(matches!(
        block_on(session.update_metadata(
            SessionRevision(7),
            BTreeMap::from([("array".to_owned(), json!([1]))])
        )),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(store.save_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn divergent_equal_revision_success_fails_and_requires_reload() {
    let (engine, session, store) = loaded();
    store.behavior(SaveBehavior::CommitThenWait);
    let mut operation = session.update_metadata(SessionRevision(7), metadata("committed"));
    pending(&mut operation);
    let mut divergent = store.record().unwrap();
    divergent.metadata = metadata("divergent");
    *store.record.lock().unwrap() = Some(divergent.clone());
    let _loaded = block_on(engine.load_session(session.id()))
        .unwrap()
        .unwrap();
    store.release.store(true, Ordering::Release);
    assert!(matches!(block_on(operation), Err(EngineError::Protocol(_))));
    assert_eq!(session.record(), divergent);
    let turn = block_on(session.prompt("fresh")).unwrap();
    assert_eq!(store.load_calls.load(Ordering::SeqCst), 3);
    assert_eq!(session.record().metadata, metadata("divergent"));
    drop(turn);
}
