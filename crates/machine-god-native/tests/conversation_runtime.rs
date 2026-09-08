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

fn routed_runtime(
    routes: &Arc<machine_god_native::NativeConversationModelRoutes>,
    session: &str,
    incarnation: &str,
) -> Result<NativeConversationRuntime, NativeConversationRuntimeError> {
    let mut record = record(None);
    record.id = SessionId::new(session).unwrap();
    record.incarnation_id = SessionIncarnationId::new(incarnation).unwrap();
    let id = record.id.clone();
    let store = InMemorySessionStore::configured(
        BTreeMap::from([(id.clone(), record)]),
        SessionStoreScript::default(),
        256,
    );
    let engine = Engine::builder()
        .session_store(store)
        .provider(ScriptedModelProvider::new("test", [finished()]))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(id)).unwrap().unwrap();
    NativeConversationRuntime::new_with_model_routes(
        NativeConversation::from_session(session).unwrap(),
        preferences("private/original"),
        None,
        routes,
    )
}

fn route_context(runtime: &NativeConversationRuntime) -> machine_god_core::ToolContext {
    machine_god_core::ToolContext {
        session_id: runtime.id(),
        session_incarnation_id: runtime.record().incarnation_id,
        turn_id: machine_god_core::TurnId::new("turn").unwrap(),
        call_id: machine_god_core::ToolCallId::new("call").unwrap(),
    }
}

#[test]
fn model_routes_isolate_incarnations_and_preserve_live_selection_until_turn_settlement() {
    use machine_god_native::{NativeConversationModelRouteError, NativeConversationModelRoutes};
    let routes = Arc::new(NativeConversationModelRoutes::new());
    let first = routed_runtime(&routes, "same-session", "first").unwrap();
    let second = routed_runtime(&routes, "same-session", "second").unwrap();
    let context = route_context(&first);
    assert_eq!(
        routes.snapshot(&context).as_deref(),
        Some("private/original")
    );
    assert!(matches!(
        routed_runtime(&routes, "same-session", "first"),
        Err(NativeConversationRuntimeError::ModelRoute(
            NativeConversationModelRouteError::Duplicate
        ))
    ));
    first.enqueue("run".into()).unwrap();
    let turn = block_on(first.start_next(100)).unwrap().unwrap();
    first
        .set_model_preferences(preferences("private/next"))
        .unwrap();
    assert_eq!(routes.snapshot(&context).as_deref(), Some("private/next"));
    assert_eq!(
        turn.model_snapshot().preferences().model(),
        "private/original"
    );
    assert_eq!(
        routes.snapshot(&route_context(&second)).as_deref(),
        Some("private/original")
    );
    let mut wrong = context.clone();
    wrong.session_id = SessionId::new("wrong-session").unwrap();
    assert!(routes.snapshot(&wrong).is_none());
    wrong = context.clone();
    wrong.session_incarnation_id = SessionIncarnationId::new("wrong-incarnation").unwrap();
    assert!(routes.snapshot(&wrong).is_none());
    assert!(!format!("{routes:?}").contains("private"));
    drop(first);
    assert_eq!(routes.snapshot(&context).as_deref(), Some("private/next"));
    complete(turn);
    assert!(routes.snapshot(&context).is_none());
    assert!(routed_runtime(&routes, "same-session", "first").is_ok());
}

#[test]
fn model_route_capacity_is_bounded_and_reusable_after_drop() {
    use machine_god_native::{
        MAX_NATIVE_CONVERSATION_MODEL_ROUTES, NativeConversationModelRouteError,
        NativeConversationModelRoutes,
    };
    let routes = Arc::new(NativeConversationModelRoutes::new());
    let mut runtimes: Vec<_> = (0..MAX_NATIVE_CONVERSATION_MODEL_ROUTES)
        .map(|index| routed_runtime(&routes, &format!("session-{index}"), "life").unwrap())
        .collect();
    assert!(matches!(
        routed_runtime(&routes, "overflow", "life"),
        Err(NativeConversationRuntimeError::ModelRoute(
            NativeConversationModelRouteError::Capacity
        ))
    ));
    let retired = route_context(runtimes.last().unwrap());
    drop(runtimes.pop());
    assert!(routes.snapshot(&retired).is_none());
    assert!(routed_runtime(&routes, "replacement", "life").is_ok());
    assert_eq!(
        routes.snapshot(&route_context(&runtimes[0])).as_deref(),
        Some("private/original")
    );
}

#[cfg(feature = "ai-gateway-http")]
#[test]
fn search_captures_model_before_capacity_wait_and_rejects_unregistered_context() {
    use machine_god_core::{NetworkTarget, Tool};
    use machine_god_native::{
        NativeConversationModelRoutes, WebSearchDeadline, WebSearchLimits, WebSearchRequest,
        WebSearchResponse, WebSearchTool, WebSearchTransport, WebSearchTransportError,
    };
    use std::sync::Mutex;
    struct Deadline;
    impl WebSearchDeadline for Deadline {
        fn wait_until(&self, _: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
            Box::pin(std::future::pending())
        }
    }
    #[derive(Default)]
    struct Transport(Mutex<Vec<WebSearchRequest>>);
    impl WebSearchTransport for Transport {
        fn search(
            &self,
            request: WebSearchRequest,
            _: CancellationToken,
        ) -> BoxFuture<'_, Result<WebSearchResponse, WebSearchTransportError>> {
            Box::pin(async move {
                let first = {
                    let mut requests = self.0.lock().unwrap();
                    requests.push(request);
                    requests.len() == 1
                };
                if first {
                    std::future::pending::<()>().await;
                }
                WebSearchResponse::new(Vec::new(), false)
            })
        }
    }
    let routes = Arc::new(NativeConversationModelRoutes::new());
    let runtime = routed_runtime(&routes, "search-session", "search-life").unwrap();
    let transport = Arc::new(Transport::default());
    let tool = WebSearchTool::with_bounded_transport(
        NetworkTarget {
            scheme: "https".into(),
            host: "example.com".into(),
            port: None,
        },
        transport.clone(),
        Arc::new(Deadline),
        WebSearchLimits::new(std::time::Duration::from_secs(30), 1).unwrap(),
    )
    .unwrap()
    .with_model_routes(routes.clone());
    let context = route_context(&runtime);
    let args = json!({"query":"test query"});
    let mut first = tool.execute(context.clone(), args.clone(), CancellationToken::new());
    assert!(transport.0.lock().unwrap().is_empty());
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(first.as_mut().poll(&mut cx).is_pending());
    let mut second = tool.execute(context.clone(), args.clone(), CancellationToken::new());
    assert!(second.as_mut().poll(&mut cx).is_pending());
    runtime
        .set_model_preferences(preferences("private/next"))
        .unwrap();
    assert_eq!(transport.0.lock().unwrap().len(), 1);
    drop(first);
    block_on(second).unwrap();
    assert_eq!(
        transport.0.lock().unwrap()[1].worker_model(),
        Some("private/original")
    );
    block_on(tool.execute(context.clone(), args.clone(), CancellationToken::new())).unwrap();
    assert_eq!(
        transport.0.lock().unwrap()[2].worker_model(),
        Some("private/next")
    );
    let mut wrong = context.clone();
    wrong.session_incarnation_id = SessionIncarnationId::new("unregistered").unwrap();
    assert!(block_on(tool.execute(wrong, args.clone(), CancellationToken::new())).is_err());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(block_on(tool.execute(context.clone(), args.clone(), cancelled)).is_err());
    assert!(
        block_on(tool.execute(
            context.clone(),
            json!({"query":"test query","worker_model":"forged"}),
            CancellationToken::new()
        ))
        .is_err()
    );
    drop(runtime);
    assert!(block_on(tool.execute(context, args, CancellationToken::new())).is_err());
    assert_eq!(transport.0.lock().unwrap().len(), 3);
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

mod preference_persistence {
    use super::*;
    use machine_god_native::{
        NativeModelPreferenceCommit, NativeUserConfigError, NativeUserConfigStore,
    };
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;

    static NEXT: AtomicU64 = AtomicU64::new(1);
    struct ConfigFixture(PathBuf);
    impl ConfigFixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "mg-preference-commit-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
        fn root(&self) -> PathBuf {
            self.0.join("machine-god")
        }
        fn store(&self) -> NativeUserConfigStore {
            NativeUserConfigStore::new(self.root())
        }
        fn seed(&self, bytes: &[u8]) {
            fs::create_dir(self.root()).unwrap();
            fs::set_permissions(self.root(), fs::Permissions::from_mode(0o700)).unwrap();
            fs::write(self.root().join("config.json"), bytes).unwrap();
        }
    }
    impl Drop for ConfigFixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn gated_runtime() -> (NativeConversationRuntime, Arc<Gate>) {
        let initial = record(None);
        let store =
            InMemorySessionStore::from_records(BTreeMap::from([(initial.id.clone(), initial)]));
        let gate = Arc::new(Gate::default());
        let runtime = runtime_with_store(
            Arc::new(GatedStore {
                store,
                gate: gate.clone(),
            }),
            ScriptedModelProvider::new("test", []),
            preferences("private/original"),
            None,
        );
        (runtime, gate)
    }

    fn assert_saved(commit: &NativeModelPreferenceCommit, generation: u64) {
        assert_eq!(commit.generation, generation);
        assert!(
            matches!(commit.session, Ok(NativeModelPreferencePersistence::Saved { generation: saved, .. }) if saved == generation)
        );
    }

    #[test]
    fn commit_is_inert_captures_on_first_poll_and_reports_both_targets() {
        let fixture = ConfigFixture::new();
        let user = fixture.store();
        let (runtime, store, provider) = setup(None, [], SessionStoreScript::default());
        let calls = store.calls().len();
        drop(runtime.persist_model_preferences(&user, 100));
        assert_eq!(store.calls().len(), calls);
        assert!(!fixture.root().exists());
        let commit = runtime.persist_model_preferences(&user, 100);
        runtime
            .set_model_preferences(preferences("private/accepted"))
            .unwrap();
        let commit = block_on(commit);
        assert_saved(&commit, 1);
        assert_eq!(
            commit
                .user_defaults
                .as_ref()
                .unwrap()
                .config()
                .model_preferences(),
            runtime.model_preferences()
        );
        assert_eq!(
            user.load().unwrap().loaded(),
            commit.user_defaults.as_ref().unwrap()
        );
        assert!(!runtime.status().model_preferences_pending);
        assert!(provider.requests().is_empty());
        assert!(!format!("{commit:?}").contains("private/accepted"));
        let calls = store.calls().len();
        let again = block_on(runtime.persist_model_preferences(&user, 0));
        assert_eq!(
            again.session,
            Ok(NativeModelPreferencePersistence::Unchanged)
        );
        assert!(again.user_defaults.is_ok());
        assert_eq!(store.calls().len(), calls);
    }

    #[test]
    fn session_failure_does_not_suppress_user_defaults_or_revert_selection() {
        let fixture = ConfigFixture::new();
        let user = fixture.store();
        let (runtime, _, _) = setup(
            None,
            [],
            SessionStoreScript {
                saves: Some(vec![SessionStoreStep::Error(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "failed",
                    "private",
                    true,
                ))]),
                ..SessionStoreScript::default()
            },
        );
        let before = runtime.record();
        runtime
            .set_model_preferences(preferences("private/accepted"))
            .unwrap();
        let commit = block_on(runtime.persist_model_preferences(&user, 100));
        assert_eq!(
            commit.session,
            Err(NativeConversationRuntimeError::Conversation(
                NativeConversationError::Persistence
            ))
        );
        assert!(commit.user_defaults.is_ok());
        assert_eq!(
            user.load().unwrap().loaded().config().model_preferences(),
            runtime.model_preferences()
        );
        assert_eq!(runtime.record(), before);
        assert!(runtime.status().model_preferences_pending);
        assert!(!runtime.status().active);
    }

    #[test]
    fn invalid_user_defaults_do_not_suppress_session_persistence() {
        let fixture = ConfigFixture::new();
        let bytes = br#"{"schema_version":99}"#;
        fixture.seed(bytes);
        let user = fixture.store();
        let (runtime, _, _) = setup(None, [], SessionStoreScript::default());
        let commit = block_on(runtime.persist_model_preferences(&user, 100));
        assert_saved(&commit, 0);
        assert!(matches!(
            commit.user_defaults,
            Err(NativeUserConfigError::InvalidConfig(_))
        ));
        assert_eq!(fs::read(fixture.root().join("config.json")).unwrap(), bytes);
        assert!(!fixture.root().join(".config.lock").exists());
        assert!(!runtime.status().model_preferences_pending);
    }

    #[test]
    fn user_publication_failure_is_independent_of_successful_session_save() {
        let fixture = ConfigFixture::new();
        fixture.seed(br#"{"schema_version":1,"permission_mode":"ask"}"#);
        fs::write(fixture.root().join(".config.tmp"), b"retained").unwrap();
        let user = fixture.store();
        let (runtime, _, _) = setup(None, [], SessionStoreScript::default());
        let commit = block_on(runtime.persist_model_preferences(&user, 100));
        assert_saved(&commit, 0);
        assert_eq!(
            commit.user_defaults,
            Err(NativeUserConfigError::Persistence)
        );
        assert_eq!(
            fs::read(fixture.root().join(".config.tmp")).unwrap(),
            b"retained"
        );
        assert_eq!(user.load().unwrap().loaded().config().schema_version(), 1);
        assert!(!runtime.status().model_preferences_pending);
    }

    #[test]
    fn active_session_defers_only_session_target_and_preserves_active_job() {
        let fixture = ConfigFixture::new();
        let user = fixture.store();
        let (runtime, _, _) = setup(None, [finished()], SessionStoreScript::default());
        runtime.enqueue("active".into()).unwrap();
        let turn = block_on(runtime.start_next(100)).unwrap().unwrap();
        runtime
            .set_model_preferences(preferences("private/next"))
            .unwrap();
        let commit = block_on(runtime.persist_model_preferences(&user, 100));
        assert_eq!(commit.generation, 1);
        assert_eq!(
            commit.session,
            Ok(NativeModelPreferencePersistence::Deferred)
        );
        assert_eq!(
            commit.user_defaults.unwrap().config().model(),
            "private/next"
        );
        assert_eq!(
            turn.model_snapshot().preferences().model(),
            "private/original"
        );
        assert!(runtime.status().active);
        complete(turn);
        let commit = block_on(runtime.persist_model_preferences(&user, 200));
        assert_saved(&commit, 1);
        assert!(!runtime.status().model_preferences_pending);
    }

    #[test]
    fn both_targets_report_failures_without_rollback() {
        let fixture = ConfigFixture::new();
        fixture.seed(b"invalid JSON");
        let user = fixture.store();
        let (runtime, _, _) = setup(
            None,
            [],
            SessionStoreScript {
                saves: Some(vec![SessionStoreStep::Error(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "failed",
                    "private",
                    true,
                ))]),
                ..SessionStoreScript::default()
            },
        );
        let expected = preferences("private/accepted");
        runtime.set_model_preferences(expected.clone()).unwrap();
        let commit = block_on(runtime.persist_model_preferences(&user, 100));
        assert!(commit.session.is_err());
        assert!(commit.user_defaults.is_err());
        assert_eq!(runtime.model_preferences(), expected);
        assert!(runtime.status().model_preferences_pending);
    }

    #[test]
    fn change_during_pending_session_save_keeps_both_receipts_on_captured_generation() {
        let fixture = ConfigFixture::new();
        let user = fixture.store();
        let (runtime, gate) = gated_runtime();
        let mut commit = runtime.persist_model_preferences(&user, 100);
        assert!(
            commit
                .as_mut()
                .poll(&mut Context::from_waker(&noop_waker()))
                .is_pending()
        );
        runtime
            .set_model_preferences(preferences("private/newer"))
            .unwrap();
        gate.release();
        let commit = block_on(commit);
        assert_saved(&commit, 0);
        assert_eq!(
            commit.user_defaults.unwrap().config().model(),
            "private/original"
        );
        assert_eq!(
            runtime.record().metadata[NATIVE_MODEL_PREFERENCES_KEY]["model"],
            "private/original"
        );
        assert_eq!(runtime.model_preferences().model(), "private/newer");
        assert_eq!(runtime.status().model_preferences_generation, 1);
        assert!(runtime.status().model_preferences_pending);
    }

    #[test]
    fn concurrent_user_change_during_session_save_causes_conflict_not_late_overwrite() {
        let fixture = ConfigFixture::new();
        let user = fixture.store();
        let (runtime, gate) = gated_runtime();
        let mut commit = runtime.persist_model_preferences(&user, 100);
        assert!(
            commit
                .as_mut()
                .poll(&mut Context::from_waker(&noop_waker()))
                .is_pending()
        );
        let other = fixture.store();
        block_on(
            other.set_model_preferences(&other.load().unwrap(), &preferences("private/concurrent")),
        )
        .unwrap();
        gate.release();
        let commit = block_on(commit);
        assert_saved(&commit, 0);
        assert_eq!(commit.user_defaults, Err(NativeUserConfigError::Conflict));
        assert_eq!(
            user.load().unwrap().loaded().config().model(),
            "private/concurrent"
        );
    }

    #[test]
    fn dropping_pending_session_save_never_detaches_a_user_writer() {
        let fixture = ConfigFixture::new();
        let user = fixture.store();
        let (runtime, gate) = gated_runtime();
        let before = runtime.record();
        let mut commit = runtime.persist_model_preferences(&user, 100);
        assert!(
            commit
                .as_mut()
                .poll(&mut Context::from_waker(&noop_waker()))
                .is_pending()
        );
        assert!(!fixture.root().exists());
        drop(commit);
        gate.release();
        assert!(!fixture.root().exists());
        assert_eq!(runtime.record(), before);
        assert!(!runtime.status().active);
        assert!(runtime.status().model_preferences_pending);
    }
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
    assert!(runtime.paused_turn().unwrap().is_some());
    assert!(provider.requests().is_empty());
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
    assert!(runtime.paused_turn().unwrap().is_none());
}

fn assert_controls_busy(runtime: &NativeConversationRuntime) {
    assert_eq!(
        runtime.context_preferences().unwrap_err(),
        NativeConversationRuntimeError::Busy
    );
    assert_eq!(
        runtime.paused_turn().unwrap_err(),
        NativeConversationRuntimeError::Busy
    );
    assert_eq!(
        block_on(runtime.rename("blocked", 500)).unwrap_err(),
        NativeConversationRuntimeError::Busy
    );
    assert_eq!(
        block_on(runtime.compact(500)).unwrap_err(),
        NativeConversationRuntimeError::Busy
    );
    assert_eq!(
        block_on(runtime.set_max_history_turns(2, 500)).unwrap_err(),
        NativeConversationRuntimeError::Busy
    );
}

#[test]
fn runtime_controls_preserve_history_queue_and_dirty_selection() {
    let (runtime, store, provider) = setup(
        None,
        [finished(), finished(), finished()],
        SessionStoreScript::default(),
    );
    for now in [100, 200] {
        runtime.enqueue(format!("question {now}").into()).unwrap();
        let turn = block_on(runtime.start_next(now)).unwrap().unwrap();
        let calls = store.calls().len();
        assert_controls_busy(&runtime);
        assert_eq!(store.calls().len(), calls);
        complete(turn);
    }
    let original = runtime.record();
    runtime.enqueue("queued question".into()).unwrap();
    runtime
        .set_model_preferences(preferences("private/next"))
        .unwrap();
    let before = runtime.status();
    let calls = store.calls().len();
    drop(runtime.rename("new title", 300));
    drop(runtime.compact(300));
    drop(runtime.set_max_history_turns(2, 300));
    assert_eq!(store.calls().len(), calls);
    assert_eq!(runtime.record(), original);
    assert!(!runtime.status().active);
    block_on(runtime.rename("new title", 300)).unwrap();
    assert!(block_on(runtime.compact(400)).unwrap());
    block_on(runtime.set_max_history_turns(2, 450)).unwrap();
    let context = runtime.context_preferences().unwrap();
    assert_eq!(context.first_retained_message(), 2);
    assert_eq!(context.max_history_turns(), 2);
    assert_eq!(runtime.record().messages, original.messages);
    assert_eq!(
        runtime.record().metadata[NATIVE_SESSION_METADATA_KEY]["title"],
        "new title"
    );
    assert_eq!(runtime.status().queued_jobs, before.queued_jobs);
    assert_eq!(
        runtime.status().model_preferences_pending,
        before.model_preferences_pending
    );
    assert!(runtime.status().model_preferences_pending);
    assert_eq!(runtime.model_preferences().model(), "private/next");
    let persisted = store.record(&runtime.id()).unwrap();
    let calls = store.calls().len();
    assert!(!block_on(runtime.compact(500)).unwrap());
    assert_eq!(store.calls().len(), calls);
    assert_eq!(store.record(&runtime.id()).unwrap(), persisted);
    complete(block_on(runtime.start_next(600)).unwrap().unwrap());
    assert_eq!(
        runtime.record().messages[..original.messages.len()],
        original.messages
    );
    let requests = provider.requests();
    let request = &requests.last().unwrap().request;
    assert_eq!(request.options.model.as_deref(), Some("private/next"));
    assert_eq!(request.messages[1..3], original.messages[2..]);
    assert_eq!(runtime.status().queued_jobs, 0);
}

#[test]
fn dropped_runtime_control_releases_admission_without_publishing_or_losing_queue() {
    let initial = record(Some(&preferences("private/original")));
    let store =
        InMemorySessionStore::from_records(BTreeMap::from([(initial.id.clone(), initial.clone())]));
    let gate = Arc::new(Gate::default());
    let provider = ScriptedModelProvider::new("test", []);
    let runtime = runtime_with_store(
        Arc::new(GatedStore {
            store: store.clone(),
            gate: gate.clone(),
        }),
        provider.clone(),
        preferences("private/original"),
        None,
    );
    let mut rename = runtime.rename("abandoned", 100);
    assert!(
        rename
            .as_mut()
            .poll(&mut Context::from_waker(&noop_waker()))
            .is_pending()
    );
    assert_controls_busy(&runtime);
    runtime.enqueue("kept".into()).unwrap();
    runtime
        .set_model_preferences(preferences("private/during-save"))
        .unwrap();
    assert_eq!(
        block_on(runtime.start_next(200)).unwrap_err(),
        NativeConversationRuntimeError::Busy
    );
    assert_eq!(
        block_on(runtime.flush_model_preferences(200)).unwrap(),
        NativeModelPreferencePersistence::Deferred
    );
    drop(rename);
    assert!(!runtime.status().active);
    assert_eq!(runtime.record(), initial);
    assert_eq!(store.record(&runtime.id()).unwrap(), initial);
    assert!(provider.requests().is_empty());
    gate.release();
    block_on(runtime.rename("committed", 300)).unwrap();
    assert_eq!(
        runtime.record().metadata[NATIVE_SESSION_METADATA_KEY]["title"],
        "committed"
    );
    assert_eq!(runtime.status().queued_jobs, 1);
    assert!(runtime.status().model_preferences_pending);
    assert_eq!(runtime.model_preferences().model(), "private/during-save");
}

#[test]
fn invalid_and_failed_runtime_controls_preserve_canonical_state() {
    let (runtime, store, provider) = setup(
        None,
        [],
        SessionStoreScript {
            saves: Some(vec![SessionStoreStep::Error(SessionStoreError::new(
                SessionStoreErrorKind::Unavailable,
                "failed",
                "save failed",
                true,
            ))]),
            ..SessionStoreScript::default()
        },
    );
    let original = runtime.record();
    runtime.enqueue("kept".into()).unwrap();
    let calls = store.calls().len();
    assert!(block_on(runtime.rename("", 100)).is_err());
    assert!(block_on(runtime.set_max_history_turns(usize::MAX, 100)).is_err());
    assert!(!block_on(runtime.compact(100)).unwrap());
    assert_eq!(store.calls().len(), calls);
    assert!(block_on(runtime.rename("failed", 200)).is_err());
    assert_eq!(runtime.record(), original);
    assert_eq!(store.record(&runtime.id()).unwrap(), original);
    assert!(!runtime.status().active);
    assert_eq!(runtime.status().queued_jobs, 1);
    assert!(runtime.status().model_preferences_pending);
    assert!(provider.requests().is_empty());
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
    assert_controls_busy(&runtime);
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
