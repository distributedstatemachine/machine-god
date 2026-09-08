#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::BTreeMap;
use std::future::poll_fn;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll};
use std::time::Instant;

use futures_executor::block_on;
use futures_util::{
    StreamExt,
    task::{AtomicWaker, noop_waker},
};
use machine_god_core::{
    BoxFuture, CancellationToken, Engine, InferenceOptions, ModelEvent, Prompt, SessionId,
    SessionIncarnationId, SessionRecord, SessionRevision, SessionStore, SessionStoreError,
    SessionStoreErrorKind, StopReason, TurnEvent,
};
use machine_god_native::{
    AI_GATEWAY_INFERENCE_OPTIONS_KEY, AiGatewayModelCatalogAccessMode,
    AiGatewayModelCatalogProvider, AiGatewayModelCatalogRequestAccess,
    AiGatewayModelCatalogTransport, AiGatewayModelCatalogTransportError,
    AiGatewayModelCatalogTransportResponse, MAX_NATIVE_QUEUED_INPUT_BYTES, MAX_NATIVE_QUEUED_JOBS,
    MAX_NATIVE_QUEUED_OPTIONS_BYTES, MAX_NATIVE_QUEUED_PROMPT_BYTES, NATIVE_MODEL_PREFERENCES_KEY,
    NATIVE_SESSION_METADATA_KEY, NativeConversation, NativeConversationError,
    NativeConversationRuntime, NativeConversationRuntimeError, NativeConversationRuntimeTurn,
    NativeModelCatalog, NativeModelPreferencePersistence, NativeModelPreferences,
    NativeReasoningEffort, NativeSessionMetadata,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
    SessionStoreScript, SessionStoreStep,
};
use serde_json::{Value, json};

fn preferences(model: &str) -> NativeModelPreferences {
    NativeModelPreferences::new(model, NativeReasoningEffort::parse("high").unwrap(), true).unwrap()
}

fn record(saved: Option<&NativeModelPreferences>) -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new("private:session").unwrap(),
        SessionIncarnationId::new("private:life").unwrap(),
    );
    record.revision = SessionRevision(1);
    record.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.to_owned(),
        NativeSessionMetadata::default().to_value(),
    );
    if let Some(saved) = saved {
        record
            .metadata
            .insert(NATIVE_MODEL_PREFERENCES_KEY.to_owned(), saved.to_value());
    }
    record
}

fn finished() -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::TextDelta {
            text: "answer".to_owned(),
        },
        ModelEvent::Stop {
            reason: StopReason::Completed,
        },
    ])
}

fn runtime_with_store(
    store: Arc<dyn SessionStore>,
    provider: ScriptedModelProvider,
    startup: NativeModelPreferences,
    process: Option<&str>,
) -> NativeConversationRuntime {
    let engine = Engine::builder()
        .shared_session_store(store)
        .provider(provider)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(SessionId::new("private:session").unwrap()))
        .unwrap()
        .unwrap();
    NativeConversationRuntime::new(
        NativeConversation::from_session(session).unwrap(),
        startup,
        process,
    )
    .unwrap()
}

fn setup(
    saved: Option<&NativeModelPreferences>,
    steps: impl IntoIterator<Item = ModelProviderStep>,
    script: SessionStoreScript,
) -> (
    NativeConversationRuntime,
    InMemorySessionStore,
    ScriptedModelProvider,
) {
    let record = record(saved);
    let store = InMemorySessionStore::configured(
        BTreeMap::from([(record.id.clone(), record)]),
        script,
        256,
    );
    let provider = ScriptedModelProvider::new("test", steps);
    let runtime = runtime_with_store(
        Arc::new(store.clone()),
        provider.clone(),
        preferences("private/original"),
        None,
    );
    (runtime, store, provider)
}

fn complete(turn: NativeConversationRuntimeTurn) {
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
}

struct CatalogTransport;
impl AiGatewayModelCatalogTransport for CatalogTransport {
    fn wait_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
    fn get(
        &self,
        _: AiGatewayModelCatalogRequestAccess,
        _: Instant,
        _: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<AiGatewayModelCatalogTransportResponse, AiGatewayModelCatalogTransportError>,
    > {
        Box::pin(async {
            Ok(AiGatewayModelCatalogTransportResponse::new(200, serde_json::to_vec(&json!({"data":[
            {"id":"private/original","type":"language","reasoning_options":[{"type":"effort","values":["high"]}],"fast_options":[{"type":"toggle"}]},
            {"id":"private/next","type":"language"}
        ]})).unwrap()))
        })
    }
}

fn catalog() -> Arc<NativeModelCatalog> {
    Arc::new(
        block_on(
            AiGatewayModelCatalogProvider::new(
                AiGatewayModelCatalogAccessMode::PublicOnly,
                Arc::new(CatalogTransport),
            )
            .list_model_details(CancellationToken::new()),
        )
        .unwrap(),
    )
}

#[test]
fn queued_and_future_jobs_follow_changes_but_active_snapshot_and_request_do_not() {
    let (runtime, store, provider) = setup(
        None,
        [finished(), finished(), finished()],
        SessionStoreScript::default(),
    );
    runtime.set_model_catalog(catalog());
    let first = runtime.enqueue("first".into()).unwrap();
    let second = runtime.enqueue("second".into()).unwrap();
    let calls = store.calls().len();
    let start = runtime.start_next(100);
    assert_eq!(runtime.status().queued_jobs, 2);
    assert!(!runtime.status().active);
    drop(start);
    assert_eq!(store.calls().len(), calls);
    let turn = block_on(runtime.start_next(100)).unwrap().unwrap();
    assert_eq!(turn.queued_id(), first);
    assert_eq!(
        turn.model_snapshot().preferences().model(),
        "private/original"
    );
    assert!(provider.requests().is_empty());
    runtime
        .set_model_preferences(preferences("private/next"))
        .unwrap();
    runtime.enqueue("third".into()).unwrap();
    assert!(runtime.status().model_preferences_pending);
    assert_eq!(
        block_on(runtime.flush_model_preferences(200)).unwrap(),
        NativeModelPreferencePersistence::Deferred
    );
    assert_eq!(
        block_on(runtime.start_next(200)).unwrap_err(),
        NativeConversationRuntimeError::Busy
    );
    complete(turn);
    assert!(!runtime.status().active);
    let turn = block_on(runtime.start_next(200)).unwrap().unwrap();
    assert_eq!(turn.queued_id(), second);
    complete(turn);
    complete(block_on(runtime.start_next(300)).unwrap().unwrap());
    assert!(block_on(runtime.start_next(400)).unwrap().is_none());
    assert!(!runtime.status().model_preferences_pending);
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[0].request.options.model.as_deref(),
        Some("private/original")
    );
    assert_eq!(
        requests[0].request.options.metadata[AI_GATEWAY_INFERENCE_OPTIONS_KEY],
        json!({"schema_version":1,"reasoning_effort":"high","fast":true})
    );
    for request in &requests[1..] {
        assert_eq!(
            request.request.options.model.as_deref(),
            Some("private/next")
        );
        assert_eq!(
            request.request.options.metadata[AI_GATEWAY_INFERENCE_OPTIONS_KEY],
            json!({"schema_version":1,"reasoning_effort":"auto","fast":false})
        );
    }
}

#[test]
fn construction_restores_saved_controls_and_only_explicit_process_model_wins() {
    let saved = preferences("private/saved");
    for process in [None, Some("private/process")] {
        let record = record(Some(&saved));
        let store = InMemorySessionStore::from_records(BTreeMap::from([(
            record.id.clone(),
            record.clone(),
        )]));
        let runtime = runtime_with_store(
            Arc::new(store.clone()),
            ScriptedModelProvider::new("test", []),
            NativeModelPreferences::default(),
            process,
        );
        let restored = runtime.model_preferences();
        assert_eq!(restored.model(), process.unwrap_or("private/saved"));
        assert_eq!(restored.effort(), saved.effort());
        assert!(restored.requested_fast());
        assert_eq!(
            runtime.status().model_preferences_pending,
            process.is_some()
        );
        assert_eq!(store.record(&runtime.id()).unwrap(), record);
    }
    let (runtime, _, _) = setup(None, [], SessionStoreScript::default());
    assert_eq!(runtime.model_preferences().model(), "private/original");
    assert!(runtime.status().model_preferences_pending);
    assert!(
        !runtime
            .record()
            .metadata
            .contains_key(NATIVE_MODEL_PREFERENCES_KEY)
    );
}

#[test]
fn idle_flush_is_inert_persists_one_generation_and_is_a_noop_when_clean() {
    let (runtime, store, provider) = setup(None, [], SessionStoreScript::default());
    runtime
        .set_model_preferences(preferences("private/changed"))
        .unwrap();
    let calls = store.calls().len();
    drop(runtime.flush_model_preferences(100));
    assert_eq!(store.calls().len(), calls);
    assert_eq!(
        block_on(runtime.flush_model_preferences(100)).unwrap(),
        NativeModelPreferencePersistence::Saved {
            generation: 1,
            revision: SessionRevision(2)
        }
    );
    assert_eq!(
        runtime.record().metadata[NATIVE_MODEL_PREFERENCES_KEY],
        preferences("private/changed").to_value()
    );
    let calls = store.calls().len();
    assert_eq!(
        block_on(runtime.flush_model_preferences(0)).unwrap(),
        NativeModelPreferencePersistence::Unchanged
    );
    assert_eq!(store.calls().len(), calls);
    assert!(provider.requests().is_empty());
}

#[test]
fn failed_or_dropped_flush_keeps_accepted_selection_pending_without_rollback() {
    let (runtime, store, _) = setup(
        None,
        [],
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Error(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "failed",
                    "private",
                    true,
                )),
                SessionStoreStep::Pending,
                SessionStoreStep::Pass,
            ]),
            ..SessionStoreScript::default()
        },
    );
    let original = runtime.record();
    let expected = preferences("private/accepted");
    runtime.set_model_preferences(expected.clone()).unwrap();
    assert!(matches!(
        block_on(runtime.flush_model_preferences(100)),
        Err(NativeConversationRuntimeError::Conversation(
            NativeConversationError::Persistence
        ))
    ));
    assert_eq!(runtime.model_preferences(), expected);
    assert!(runtime.status().model_preferences_pending);
    let mut flush = runtime.flush_model_preferences(100);
    let waker = noop_waker();
    assert!(
        flush
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    assert!(runtime.status().active);
    assert_eq!(
        block_on(runtime.flush_model_preferences(100)).unwrap(),
        NativeModelPreferencePersistence::Deferred
    );
    drop(flush);
    assert!(!runtime.status().active);
    assert_eq!(store.record(&runtime.id()).unwrap(), original);
    assert!(matches!(
        block_on(runtime.flush_model_preferences(100)).unwrap(),
        NativeModelPreferencePersistence::Saved { generation: 1, .. }
    ));
    assert!(!runtime.status().model_preferences_pending);
}

#[derive(Default)]
struct Gate {
    open: AtomicBool,
    waker: AtomicWaker,
}
impl Gate {
    fn release(&self) {
        self.open.store(true, Ordering::Release);
        self.waker.wake();
    }
    async fn wait(&self) {
        poll_fn(|cx| {
            self.waker.register(cx.waker());
            if self.open.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
    }
}
struct GatedStore {
    store: InMemorySessionStore,
    gate: Arc<Gate>,
}
impl SessionStore for GatedStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.store.load(id)
    }
    fn save(
        &self,
        record: SessionRecord,
        revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async move {
            self.gate.wait().await;
            self.store.save(record, revision).await
        })
    }
}

#[test]
fn selection_during_pending_flush_is_not_misreported_as_the_saved_generation() {
    let initial = record(None);
    let store = InMemorySessionStore::from_records(BTreeMap::from([(initial.id.clone(), initial)]));
    let gate = Arc::new(Gate::default());
    let runtime = runtime_with_store(
        Arc::new(GatedStore {
            store: store.clone(),
            gate: gate.clone(),
        }),
        ScriptedModelProvider::new("test", []),
        preferences("private/original"),
        None,
    );
    let mut flush = runtime.flush_model_preferences(100);
    let waker = noop_waker();
    assert!(
        flush
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    runtime
        .set_model_preferences(preferences("private/during-save"))
        .unwrap();
    gate.release();
    assert!(matches!(
        block_on(flush).unwrap(),
        NativeModelPreferencePersistence::Saved { generation: 0, .. }
    ));
    assert_eq!(runtime.model_preferences().model(), "private/during-save");
    assert_eq!(
        store.record(&runtime.id()).unwrap().metadata[NATIVE_MODEL_PREFERENCES_KEY]["model"],
        "private/original"
    );
    assert!(runtime.status().model_preferences_pending);
    assert!(matches!(
        block_on(runtime.flush_model_preferences(200)).unwrap(),
        NativeModelPreferencePersistence::Saved { generation: 1, .. }
    ));
    assert!(!runtime.status().model_preferences_pending);
}

#[test]
fn dropped_pending_start_consumes_only_taken_input_never_requeues_uncertain_work() {
    let (runtime, store, provider) = setup(
        Some(&preferences("private/original")),
        [],
        SessionStoreScript {
            saves: Some(vec![SessionStoreStep::Pending]),
            ..SessionStoreScript::default()
        },
    );
    runtime.enqueue("taken".into()).unwrap();
    runtime.enqueue("still queued".into()).unwrap();
    let original = runtime.record();
    let mut start = runtime.start_next(100);
    let waker = noop_waker();
    assert!(
        start
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    assert_eq!(runtime.status().queued_jobs, 1);
    assert!(runtime.status().active);
    drop(start);
    assert!(!runtime.status().active);
    assert!(runtime.status().model_preferences_pending);
    assert_eq!(runtime.status().queued_jobs, 1);
    assert_eq!(store.record(&runtime.id()).unwrap(), original);
    assert!(provider.requests().is_empty());
}

#[test]
fn continuation_requires_idle_empty_queue_and_captures_current_selection() {
    let (runtime, _, provider) = setup(None, [finished()], SessionStoreScript::default());
    assert!(matches!(
        runtime.enqueue_continuation(InferenceOptions::default()),
        Err(NativeConversationRuntimeError::Conversation(
            NativeConversationError::NoCheckpoint
        ))
    ));
    runtime.enqueue("original question".into()).unwrap();
    assert_eq!(
        runtime
            .enqueue_continuation(InferenceOptions::default())
            .unwrap_err(),
        NativeConversationRuntimeError::Busy
    );
    let turn = block_on(runtime.start_next(100)).unwrap().unwrap();
    assert_eq!(
        runtime
            .enqueue_continuation(InferenceOptions::default())
            .unwrap_err(),
        NativeConversationRuntimeError::Busy
    );
    drop(turn);
    runtime
        .enqueue_continuation(InferenceOptions::default())
        .unwrap();
    runtime
        .set_model_preferences(preferences("private/continued"))
        .unwrap();
    complete(block_on(runtime.start_next(200)).unwrap().unwrap());
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].request.options.model.as_deref(),
        Some("private/continued")
    );
    assert_eq!(runtime.record().messages.len(), 2);
}

#[test]
fn native_finalizer_keeps_runtime_busy_without_losing_pending_selection() {
    let (runtime, _, _) = setup(
        None,
        [finished()],
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Pending,
            ]),
            ..SessionStoreScript::default()
        },
    );
    runtime.enqueue("question".into()).unwrap();
    let mut turn = block_on(runtime.start_next(100)).unwrap().unwrap();
    runtime
        .set_model_preferences(preferences("private/next"))
        .unwrap();
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut finalizer_pending = false;
    for _ in 0..32 {
        use futures_core::Stream;
        match std::pin::Pin::new(&mut turn).poll_next(&mut cx) {
            Poll::Pending => {
                finalizer_pending = true;
                break;
            }
            Poll::Ready(Some(Ok(event))) => {
                assert!(!matches!(event.payload, TurnEvent::Completed { .. }));
            }
            event @ Poll::Ready(_) => panic!("unexpected event: {event:?}"),
        }
    }
    assert!(finalizer_pending);
    assert!(runtime.status().active);
    assert_eq!(
        block_on(runtime.flush_model_preferences(200)).unwrap(),
        NativeModelPreferencePersistence::Deferred
    );
    assert_eq!(
        block_on(runtime.start_next(200)).unwrap_err(),
        NativeConversationRuntimeError::Busy
    );
    drop(turn);
    assert!(!runtime.status().active);
    assert!(runtime.status().model_preferences_pending);
}

#[test]
fn dropping_runtime_clears_queue_without_invalidating_the_owned_active_turn() {
    let (runtime, _, provider) = setup(None, [finished()], SessionStoreScript::default());
    runtime.enqueue("active".into()).unwrap();
    runtime.enqueue("never run".into()).unwrap();
    let turn = block_on(runtime.start_next(100)).unwrap().unwrap();
    drop(runtime);
    complete(turn);
    assert_eq!(provider.requests().len(), 1);
}

#[test]
fn queue_count_input_and_aggregate_limits_are_independent_and_recover_after_removal() {
    let (runtime, _, provider) = setup(None, [], SessionStoreScript::default());
    let mut ids = Vec::new();
    for _ in 0..MAX_NATIVE_QUEUED_JOBS {
        ids.push(runtime.enqueue("q".into()).unwrap());
    }
    assert_eq!(
        runtime.enqueue("one too many".into()).unwrap_err(),
        NativeConversationRuntimeError::QueueLimit
    );
    assert!(runtime.cancel_queued(ids[0]));
    assert!(!runtime.cancel_queued(ids[0]));
    let next = runtime.enqueue("reused capacity".into()).unwrap();
    assert!(next.get() > ids.last().unwrap().get());
    assert_eq!(runtime.clear_queued(), MAX_NATIVE_QUEUED_JOBS);
    assert_eq!(runtime.status().queued_input_bytes, 0);
    assert_eq!(
        runtime
            .enqueue("x".repeat(MAX_NATIVE_QUEUED_PROMPT_BYTES + 1).into())
            .unwrap_err(),
        NativeConversationRuntimeError::InputLimit
    );
    for _ in 0..(MAX_NATIVE_QUEUED_INPUT_BYTES / MAX_NATIVE_QUEUED_PROMPT_BYTES - 1) {
        runtime
            .enqueue("x".repeat(MAX_NATIVE_QUEUED_PROMPT_BYTES).into())
            .unwrap();
    }
    assert_eq!(
        runtime
            .enqueue("x".repeat(MAX_NATIVE_QUEUED_PROMPT_BYTES).into())
            .unwrap_err(),
        NativeConversationRuntimeError::QueueLimit
    );
    assert!(runtime.status().queued_input_bytes < MAX_NATIVE_QUEUED_INPUT_BYTES);
    let _ = runtime.clear_queued();
    assert!(provider.requests().is_empty());
}

fn deep_options() -> InferenceOptions {
    let mut value = Value::Null;
    for _ in 0..20_000 {
        value = Value::Array(vec![value]);
    }
    InferenceOptions {
        metadata: BTreeMap::from([("private/deep".to_owned(), value)]),
        ..InferenceOptions::default()
    }
}

#[test]
fn active_cancellation_preserves_pending_job_and_current_model_selection() {
    use futures_core::Stream;
    let (runtime, _, provider) = setup(
        None,
        [ModelProviderStep::pending(), finished()],
        SessionStoreScript::default(),
    );
    runtime.enqueue("cancel me".into()).unwrap();
    runtime.enqueue("next".into()).unwrap();
    let mut turn = block_on(runtime.start_next(100)).unwrap().unwrap();
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    for _ in 0..32 {
        match std::pin::Pin::new(&mut turn).poll_next(&mut cx) {
            Poll::Pending => break,
            Poll::Ready(Some(Ok(event))) => assert!(!matches!(
                event.payload,
                TurnEvent::Completed { .. } | TurnEvent::Failed { .. }
            )),
            event @ Poll::Ready(_) => panic!("unexpected event {event:?}"),
        }
    }
    assert_eq!(provider.requests().len(), 1);
    runtime
        .set_model_preferences(preferences("private/after-cancel"))
        .unwrap();
    assert!(turn.handle().unwrap().cancel());
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(runtime.status().queued_jobs, 1);
    assert!(runtime.status().model_preferences_pending);
    complete(block_on(runtime.start_next(200)).unwrap().unwrap());
    assert_eq!(
        provider.requests()[1].request.options.model.as_deref(),
        Some("private/after-cancel")
    );
}

#[test]
fn preference_change_during_admission_preserves_taken_snapshot_and_dirty_generation() {
    let initial = record(None);
    let store = InMemorySessionStore::from_records(BTreeMap::from([(initial.id.clone(), initial)]));
    let gate = Arc::new(Gate::default());
    let provider = ScriptedModelProvider::new("test", [finished(), finished()]);
    let runtime = runtime_with_store(
        Arc::new(GatedStore {
            store,
            gate: gate.clone(),
        }),
        provider.clone(),
        preferences("private/original"),
        None,
    );
    runtime.enqueue("taken".into()).unwrap();
    let mut start = runtime.start_next(100);
    let waker = noop_waker();
    assert!(
        start
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    runtime
        .set_model_preferences(preferences("private/changed"))
        .unwrap();
    runtime.enqueue("next".into()).unwrap();
    gate.release();
    let turn = block_on(start).unwrap().unwrap();
    assert_eq!(
        turn.model_snapshot().preferences().model(),
        "private/original"
    );
    assert!(runtime.status().model_preferences_pending);
    complete(turn);
    complete(block_on(runtime.start_next(200)).unwrap().unwrap());
    assert_eq!(
        provider.requests()[0].request.options.model.as_deref(),
        Some("private/original")
    );
    assert_eq!(
        provider.requests()[1].request.options.model.as_deref(),
        Some("private/changed")
    );
    assert!(!runtime.status().model_preferences_pending);
}

#[test]
fn queued_continuation_cannot_follow_a_replaced_checkpoint() {
    let initial = record(None);
    let store =
        InMemorySessionStore::from_records(BTreeMap::from([(initial.id.clone(), initial.clone())]));
    let provider = ScriptedModelProvider::new("test", []);
    let engine = Engine::builder()
        .session_store(store)
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(initial.id)).unwrap().unwrap();
    let external = NativeConversation::from_session(session.clone()).unwrap();
    let runtime = NativeConversationRuntime::new(
        NativeConversation::from_session(session).unwrap(),
        preferences("private/model"),
        None,
    )
    .unwrap();
    runtime.enqueue("original".into()).unwrap();
    drop(block_on(runtime.start_next(100)).unwrap().unwrap());
    runtime
        .enqueue_continuation(InferenceOptions::default())
        .unwrap();
    drop(block_on(external.prompt("replacement".into(), 200)).unwrap());
    assert_eq!(
        block_on(runtime.start_next(300)).unwrap_err(),
        NativeConversationRuntimeError::Conversation(NativeConversationError::Conflict)
    );
    assert!(!runtime.status().active);
    assert_eq!(runtime.status().queued_jobs, 0);
    assert!(provider.requests().is_empty());
}

#[test]
fn queue_options_enforce_exact_serialized_byte_boundary_without_large_copies() {
    let (runtime, _, _) = setup(None, [], SessionStoreScript::default());
    let mut options = InferenceOptions::default();
    options.metadata.insert("data".to_owned(), json!(""));
    let overhead = serde_json::to_vec(&options).unwrap().len();
    options.metadata.insert(
        "data".to_owned(),
        json!("x".repeat(MAX_NATIVE_QUEUED_OPTIONS_BYTES - overhead)),
    );
    let queued = runtime
        .enqueue(Prompt {
            text: String::new(),
            options,
        })
        .unwrap();
    assert_eq!(
        runtime.status().queued_input_bytes,
        MAX_NATIVE_QUEUED_OPTIONS_BYTES
    );
    assert!(runtime.cancel_queued(queued));
    let mut options = InferenceOptions::default();
    options.metadata.insert(
        "data".to_owned(),
        json!("x".repeat(MAX_NATIVE_QUEUED_OPTIONS_BYTES - overhead + 1)),
    );
    assert_eq!(
        runtime
            .enqueue(Prompt {
                text: String::new(),
                options
            })
            .unwrap_err(),
        NativeConversationRuntimeError::InputLimit
    );
}

#[test]
fn rejected_queue_options_are_bounded_redacted_and_iteratively_destroyed() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let (runtime, store, _) = setup(None, [], SessionStoreScript::default());
            let calls = store.calls().len();
            for options in [
                deep_options(),
                InferenceOptions {
                    metadata: BTreeMap::from([(
                        "private/large".to_owned(),
                        Value::String("x".repeat(MAX_NATIVE_QUEUED_OPTIONS_BYTES)),
                    )]),
                    ..InferenceOptions::default()
                },
            ] {
                let error = runtime
                    .enqueue(Prompt {
                        text: "private/input".to_owned(),
                        options,
                    })
                    .unwrap_err();
                assert_eq!(error, NativeConversationRuntimeError::InputLimit);
                assert!(!format!("{error}: {error:?} {runtime:?}").contains("private"));
            }
            assert_eq!(
                runtime.enqueue_continuation(deep_options()).unwrap_err(),
                NativeConversationRuntimeError::InputLimit
            );
            assert_eq!(runtime.status().queued_jobs, 0);
            assert_eq!(store.calls().len(), calls);
        })
        .unwrap()
        .join()
        .unwrap();
}
