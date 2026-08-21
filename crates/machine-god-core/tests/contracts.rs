use futures_core::Stream;
use futures_util::{StreamExt, stream};
use machine_god_core::{
    BoxFuture, BuildError, CancellationToken, Capability, ContentBlock, Engine, EngineBuilder,
    EngineError, EngineEvent, EventSink, EventSinkError, ModelEvent, ModelEventStream,
    ModelProvider, ModelRequest, PermissionDecision, PermissionError, PermissionGrantScope,
    PermissionHandler, PermissionRequest, PreparedToolCall, ProviderError, ProviderErrorKind, Role,
    Session, SessionId, SessionIncarnationId, SessionRecord, SessionRevision, SessionStore,
    SessionStoreError, StopReason, TokenUsage, Tool, ToolCall, ToolCallId, ToolContext, ToolError,
    ToolErrorKind, ToolName, ToolOutput, ToolSpec, Turn, TurnEvent, TurnHandle,
};
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::task::{Context, Poll, Wake, Waker};

trait EngineTestSessions {
    fn create_test_session(&self, id: SessionId) -> Session;
}

impl EngineTestSessions for Engine {
    fn create_test_session(&self, id: SessionId) -> Session {
        let incarnation = test_incarnation(&id);
        self.create_session(id, incarnation)
            .expect("test session identity does not conflict")
    }
}

fn test_incarnation(id: &SessionId) -> SessionIncarnationId {
    SessionIncarnationId::new(format!("test-incarnation-{id}"))
        .expect("test session identity is valid")
}

#[derive(Debug)]
struct StaticProvider {
    events: Vec<Result<ModelEvent, ProviderError>>,
}

impl StaticProvider {
    fn completed() -> Self {
        Self {
            events: vec![
                Ok(ModelEvent::TextDelta {
                    text: "hello".to_owned(),
                }),
                Ok(ModelEvent::Stop {
                    reason: StopReason::Completed,
                }),
            ],
        }
    }
}

impl ModelProvider for StaticProvider {
    fn name(&self) -> &'static str {
        "static"
    }

    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        let events = self.events.clone();
        Box::pin(async move { Ok(Box::pin(stream::iter(events)) as ModelEventStream) })
    }
}

#[derive(Clone, Debug, Default)]
struct MemoryStore {
    record: Arc<Mutex<Option<SessionRecord>>>,
}

impl SessionStore for MemoryStore {
    fn load(
        &self,
        _id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        let record = self.record.lock().unwrap().clone();
        Box::pin(async move { Ok(record) })
    }

    fn save(
        &self,
        mut record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        let result = {
            let mut stored = self.record.lock().unwrap();
            let accepted = match (&*stored, expected_revision) {
                (None, None) => true,
                (Some(current), Some(expected)) => {
                    current.id == record.id
                        && current.incarnation_id == record.incarnation_id
                        && current.revision == expected
                }
                _ => false,
            };
            if accepted {
                let revision = SessionRevision(
                    stored
                        .as_ref()
                        .map_or(1, |current| current.revision.0.saturating_add(1)),
                );
                record.revision = revision;
                *stored = Some(record);
                Ok(revision)
            } else {
                Err(SessionStoreError::new(
                    machine_god_core::SessionStoreErrorKind::Conflict,
                    "revision_conflict",
                    "stored revision changed",
                    true,
                ))
            }
        };
        Box::pin(async move { result })
    }
}

#[derive(Debug)]
struct AllowOnce;

impl PermissionHandler for AllowOnce {
    fn authorize(
        &self,
        _request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        Box::pin(async {
            Ok(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })
        })
    }
}

#[derive(Debug)]
struct StaticTool(&'static str);

impl Tool for StaticTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new(self.0).unwrap(),
            description: "test".to_owned(),
            input_schema: json!({"type": "object"}),
        }
    }

    fn execute(
        &self,
        _context: ToolContext,
        _arguments: serde_json::Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async { Ok(ToolOutput::success(json!({"ok": true}))) })
    }
}

#[derive(Debug)]
struct ToolThenRecoverProvider {
    calls: Arc<AtomicUsize>,
    tool_name: ToolName,
}

impl ModelProvider for ToolThenRecoverProvider {
    fn name(&self) -> &'static str {
        "tool-then-recover"
    }

    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        let round = self.calls.fetch_add(1, Ordering::Relaxed);
        let events = if round == 0 {
            vec![
                Ok(ModelEvent::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("call-1").unwrap(),
                        name: self.tool_name.clone(),
                        arguments: json!({"raw": true}),
                    },
                }),
                Ok(ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                }),
            ]
        } else {
            vec![
                Ok(ModelEvent::TextDelta {
                    text: "recovered".to_owned(),
                }),
                Ok(ModelEvent::Stop {
                    reason: StopReason::Completed,
                }),
            ]
        };
        Box::pin(async move { Ok(Box::pin(stream::iter(events)) as ModelEventStream) })
    }
}

#[derive(Clone, Debug, Default)]
struct CountingPermissionHandler(Arc<AtomicUsize>);

impl PermissionHandler for CountingPermissionHandler {
    fn authorize(
        &self,
        _request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {
            Ok(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })
        })
    }
}

#[derive(Clone, Debug, Default)]
struct FailingPrepareTool {
    executions: Arc<AtomicUsize>,
}

impl Tool for FailingPrepareTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("preflight").unwrap(),
            description: "test".to_owned(),
            input_schema: json!({"type": "object"}),
        }
    }

    fn prepare(&self, _call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        Err(ToolError::new(
            ToolErrorKind::InvalidInput,
            "hostile_code",
            "hostile preflight diagnostic",
            false,
        ))
    }

    fn execute(
        &self,
        _context: ToolContext,
        _arguments: serde_json::Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        self.executions.fetch_add(1, Ordering::Relaxed);
        Box::pin(async { Ok(ToolOutput::success(json!({"unexpected": true}))) })
    }
}

fn engine_with(provider: impl ModelProvider) -> Engine {
    Engine::builder()
        .provider(provider)
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .build()
        .unwrap()
}

fn prompt(session: &machine_god_core::Session, text: &str) -> Turn {
    futures_executor::block_on(session.prompt(text)).unwrap()
}

#[test]
fn extension_traits_are_object_safe() {
    fn provider(_: Arc<dyn ModelProvider>) {}
    fn store(_: Arc<dyn SessionStore>) {}
    fn permissions(_: Arc<dyn PermissionHandler>) {}
    fn sink(_: Arc<dyn EventSink>) {}
    fn tool(_: Arc<dyn Tool>) {}

    provider(Arc::new(StaticProvider::completed()));
    store(Arc::new(MemoryStore::default()));
    permissions(Arc::new(AllowOnce));
    sink(Arc::new(machine_god_core::NoopEventSink));
    tool(Arc::new(StaticTool("compile_surface")));
}

#[test]
fn default_tool_preflight_preserves_policy_and_execution_inputs() {
    let tool = StaticTool("default_preflight");
    let call = ToolCall {
        id: ToolCallId::new("call-default").unwrap(),
        name: ToolName::new("default_preflight").unwrap(),
        arguments: json!({"path": "source.rs"}),
    };

    let prepared = tool.prepare(call.clone()).unwrap();

    assert_eq!(prepared.arguments(), &call.arguments);
    assert_eq!(
        prepared.capability(),
        &Capability::Tool {
            name: call.name,
            call_id: call.id,
            arguments: call.arguments,
        }
    );
}

#[test]
fn prepare_error_skips_policy_and_execution_then_recovers_next_round() {
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let provider = ToolThenRecoverProvider {
        calls: Arc::clone(&provider_calls),
        tool_name: ToolName::new("preflight").unwrap(),
    };
    let permissions = CountingPermissionHandler::default();
    let tool = FailingPrepareTool::default();
    let store = MemoryStore::default();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(permissions.clone())
        .tool(tool.clone())
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("preflight-error").unwrap());

    let events = futures_executor::block_on(prompt(&session, "read").collect::<Vec<_>>());

    assert!(events.iter().all(Result::is_ok));
    assert_eq!(provider_calls.load(Ordering::Relaxed), 2);
    assert_eq!(permissions.0.load(Ordering::Relaxed), 0);
    assert_eq!(tool.executions.load(Ordering::Relaxed), 0);
    let record = store.record.lock().unwrap().clone().unwrap();
    assert!(record.messages.iter().any(|message| {
        message.content.iter().any(|block| {
            matches!(
                block,
                ContentBlock::ToolResult { output, .. }
                    if output.is_error
                        && output.content == json!({
                            "code": "tool_error",
                            "message": "tool execution failed",
                            "retryable": false,
                        })
            )
        })
    }));
}

#[test]
fn prepared_tool_call_reclaims_deep_json_iteratively() {
    fn deep_value(depth: usize) -> serde_json::Value {
        let mut value = serde_json::Value::Null;
        for _ in 0..depth {
            value = serde_json::Value::Array(vec![value]);
        }
        value
    }

    let prepared = PreparedToolCall::new(
        Capability::Custom {
            name: "deep".to_owned(),
            details: deep_value(20_000),
        },
        deep_value(20_000),
    );
    drop(prepared);
}

#[test]
fn builder_requires_every_authority_boundary() {
    assert_eq!(
        EngineBuilder::new().build().unwrap_err(),
        BuildError::MissingProvider
    );
    assert_eq!(
        EngineBuilder::new()
            .provider(StaticProvider::completed())
            .build()
            .unwrap_err(),
        BuildError::MissingSessionStore
    );
    assert_eq!(
        EngineBuilder::new()
            .provider(StaticProvider::completed())
            .session_store(MemoryStore::default())
            .build()
            .unwrap_err(),
        BuildError::MissingPermissionHandler
    );
}

#[test]
fn duplicate_tools_fail_and_specs_are_deterministic() {
    let error = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .tool(StaticTool("same"))
        .tool(StaticTool("same"))
        .build()
        .unwrap_err();
    assert_eq!(error, BuildError::DuplicateTool("same".to_owned()));

    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .tool(StaticTool("zeta"))
        .tool(StaticTool("alpha"))
        .build()
        .unwrap();
    let names: Vec<_> = engine
        .tool_specs()
        .into_iter()
        .map(|spec| spec.name.to_string())
        .collect();
    assert_eq!(names, ["alpha", "zeta"]);
}

#[test]
fn load_session_restores_provider_neutral_record() {
    let id = SessionId::new("stored").unwrap();
    let incarnation_id = test_incarnation(&id);
    let store = MemoryStore {
        record: Arc::new(Mutex::new(Some(SessionRecord {
            id: id.clone(),
            incarnation_id: incarnation_id.clone(),
            revision: SessionRevision(7),
            next_turn_sequence: 8,
            messages: vec![machine_god_core::Message::text(Role::User, "prior")],
            metadata: BTreeMap::new(),
        }))),
    };
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store)
        .permission_handler(AllowOnce)
        .build()
        .unwrap();

    let session = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();
    assert_eq!(session.incarnation_id(), incarnation_id);
    assert_eq!(session.record().revision, SessionRevision(7));
    assert_eq!(session.record().messages.len(), 1);
}

#[test]
fn persisted_sessions_require_an_explicit_incarnation_id() {
    let legacy_record = json!({
        "id": "legacy-session",
        "revision": 1,
        "next_turn_sequence": 1,
        "messages": [],
        "metadata": {}
    });

    assert!(serde_json::from_value::<SessionRecord>(legacy_record).is_err());
}

#[test]
fn load_session_rejects_a_mismatched_store_record() {
    let store = MemoryStore {
        record: Arc::new(Mutex::new(Some({
            let id = SessionId::new("different").unwrap();
            SessionRecord::empty(id.clone(), test_incarnation(&id))
        }))),
    };
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store)
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let result =
        futures_executor::block_on(engine.load_session(SessionId::new("requested").unwrap()));
    assert!(matches!(result, Err(EngineError::Protocol(_))));
}

#[test]
fn load_session_rejects_a_zero_persisted_revision() {
    let id = SessionId::new("stored-zero-revision").unwrap();
    let store = MemoryStore {
        record: Arc::new(Mutex::new(Some(SessionRecord::empty(
            id.clone(),
            test_incarnation(&id),
        )))),
    };
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store)
        .permission_handler(AllowOnce)
        .build()
        .unwrap();

    assert!(matches!(
        futures_executor::block_on(engine.load_session(id)),
        Err(EngineError::Protocol(message))
            if message.contains("stored session revision must be positive")
    ));
}

#[test]
fn load_session_rejects_a_higher_revision_with_a_lower_turn_sequence() {
    let id = SessionId::new("load-sequence-regression").unwrap();
    let store = MemoryStore {
        record: Arc::new(Mutex::new(Some(SessionRecord {
            id: id.clone(),
            incarnation_id: test_incarnation(&id),
            revision: SessionRevision(5),
            next_turn_sequence: 10,
            messages: vec![machine_god_core::Message::text(Role::User, "current")],
            metadata: BTreeMap::new(),
        }))),
    };
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store.clone())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = futures_executor::block_on(engine.load_session(id.clone()))
        .unwrap()
        .unwrap();
    let regressed = SessionRecord {
        id: id.clone(),
        incarnation_id: session.incarnation_id(),
        revision: SessionRevision(6),
        next_turn_sequence: 9,
        messages: vec![machine_god_core::Message::text(
            Role::User,
            "newer metadata",
        )],
        metadata: BTreeMap::from([("version".to_owned(), json!(6))]),
    };
    *store.record.lock().unwrap() = Some(regressed);

    assert!(matches!(
        futures_executor::block_on(engine.load_session(id)),
        Err(EngineError::Protocol(message)) if message.contains("turn sequence regressed")
    ));
    assert_eq!(session.record().revision, SessionRevision(5));
    assert_eq!(session.record().next_turn_sequence, 10);

    let advanced = SessionRecord {
        id: session.id(),
        incarnation_id: session.incarnation_id(),
        revision: SessionRevision(7),
        next_turn_sequence: 10,
        messages: vec![machine_god_core::Message::text(Role::Assistant, "changed")],
        metadata: BTreeMap::from([("version".to_owned(), json!(7))]),
    };
    *store.record.lock().unwrap() = Some(advanced.clone());
    let loaded = futures_executor::block_on(engine.load_session(session.id()))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.record(), advanced);
}

#[test]
fn turn_events_are_ordered_and_release_session_at_terminal_event() {
    let session = engine_with(StaticProvider::completed())
        .create_test_session(SessionId::new("ordered").unwrap());
    let mut turn = prompt(&session, "hi");
    assert!(session.has_active_turn());
    assert_eq!(
        futures_executor::block_on(session.prompt("overlap")).unwrap_err(),
        EngineError::SessionBusy
    );

    let events = futures_executor::block_on(async {
        let mut output = Vec::new();
        while let Some(event) = turn.next().await {
            output.push(event.unwrap());
        }
        output
    });
    assert_eq!(events.len(), 4);
    assert!(matches!(events[0].payload, TurnEvent::Started));
    assert!(matches!(
        events[1].payload,
        TurnEvent::Model {
            event: ModelEvent::TextDelta { .. }
        }
    ));
    assert!(matches!(
        events[2].payload,
        TurnEvent::Model {
            event: ModelEvent::Stop { .. }
        }
    ));
    assert!(matches!(events[3].payload, TurnEvent::Completed { .. }));
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert!(!session.has_active_turn());
    assert!(futures_executor::block_on(session.prompt("next")).is_ok());
}

#[test]
fn usage_preserves_stream_state_and_reaches_terminal_completion() {
    let usage = TokenUsage {
        input_tokens: 13,
        output_tokens: 5,
        cached_input_tokens: 8,
    };
    let provider = StaticProvider {
        events: vec![
            Ok(ModelEvent::Usage { usage }),
            Ok(ModelEvent::Stop {
                reason: StopReason::Completed,
            }),
        ],
    };
    let session =
        engine_with(provider).create_test_session(SessionId::new("usage-then-stop").unwrap());
    let turn = prompt(&session, "measure");
    let events = futures_executor::block_on(turn.collect::<Vec<_>>());
    let events: Vec<_> = events.into_iter().collect::<Result<_, _>>().unwrap();

    assert_eq!(events.len(), 4);
    assert!(matches!(
        events[1].payload,
        TurnEvent::Model {
            event: ModelEvent::Usage { usage: observed }
        } if observed == usage
    ));
    assert!(matches!(
        events[2].payload,
        TurnEvent::Model {
            event: ModelEvent::Stop {
                reason: StopReason::Completed
            }
        }
    ));
    assert!(matches!(
        events[3].payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            usage: observed,
        } if observed == usage
    ));
    assert!(!session.has_active_turn());
    assert!(futures_executor::block_on(session.prompt("next")).is_ok());
}

#[test]
fn dropping_turn_releases_shared_session_lease() {
    let session = engine_with(StaticProvider::completed())
        .create_test_session(SessionId::new("drop-turn").unwrap());
    let clone = session.clone();
    let turn = prompt(&session, "first");
    assert_eq!(
        futures_executor::block_on(clone.prompt("busy")).unwrap_err(),
        EngineError::SessionBusy
    );
    drop(turn);
    assert!(futures_executor::block_on(clone.prompt("released")).is_ok());
}

#[test]
fn persisted_turn_ids_continue_across_reload_and_stale_handles() {
    let store = MemoryStore::default();
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store.clone())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let id = SessionId::new("durable-turns").unwrap();

    let created = engine.create_test_session(id.clone());
    let first = prompt(&created, "first");
    assert_eq!(first.id().as_str(), "turn-1");
    drop(first);

    let reloaded = futures_executor::block_on(engine.load_session(id.clone()))
        .unwrap()
        .unwrap();
    let second = prompt(&reloaded, "second");
    assert_eq!(second.id().as_str(), "turn-2");
    drop(second);

    let stale_a = futures_executor::block_on(engine.load_session(id.clone()))
        .unwrap()
        .unwrap();
    let stale_b = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();
    let third = prompt(&stale_a, "third");
    assert_eq!(third.id().as_str(), "turn-3");
    drop(third);
    let fourth = prompt(&stale_b, "fourth after conflict retry");
    assert_eq!(fourth.id().as_str(), "turn-4");
    drop(fourth);

    let persisted = store.record.lock().unwrap().clone().unwrap();
    assert_eq!(persisted.next_turn_sequence, 5);
    assert_eq!(persisted.revision, SessionRevision(4));
}

#[test]
fn initial_create_saves_the_zero_sentinel_as_a_positive_revision() {
    let store = MemoryStore::default();
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store.clone())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("initial-save").unwrap());
    assert_eq!(session.record().revision, SessionRevision(0));

    let turn = prompt(&session, "first");
    assert_eq!(turn.id().as_str(), "turn-1");
    assert_eq!(session.record().revision, SessionRevision(1));
    let stored = store.record.lock().unwrap().clone().unwrap();
    assert_eq!(stored.revision, SessionRevision(1));
    assert_eq!(stored.next_turn_sequence, 2);
}

#[test]
fn independently_created_handles_share_one_live_turn_lease() {
    let engine = engine_with(StaticProvider::completed());
    let id = SessionId::new("create-create-lease").unwrap();
    let first = engine.create_test_session(id.clone());
    let second = engine.create_test_session(id);

    let turn = prompt(&first, "first");
    assert_eq!(
        futures_executor::block_on(second.prompt("overlap")).unwrap_err(),
        EngineError::SessionBusy
    );
    drop(turn);
    assert!(futures_executor::block_on(second.prompt("after release")).is_ok());
}

#[test]
fn live_session_id_rejects_a_different_incarnation_for_create_and_load() {
    let id = SessionId::new("live-incarnation-conflict").unwrap();
    let first_incarnation = SessionIncarnationId::new("logical-lifetime-one").unwrap();
    let second_incarnation = SessionIncarnationId::new("logical-lifetime-two").unwrap();
    let stored = SessionRecord {
        id: id.clone(),
        incarnation_id: second_incarnation.clone(),
        revision: SessionRevision(1),
        next_turn_sequence: 1,
        messages: Vec::new(),
        metadata: BTreeMap::new(),
    };
    let store = MemoryStore {
        record: Arc::new(Mutex::new(Some(stored))),
    };
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store)
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let first = engine
        .create_session(id.clone(), first_incarnation.clone())
        .unwrap();

    assert!(matches!(
        engine.create_session(id.clone(), second_incarnation),
        Err(EngineError::SessionIncarnationConflict)
    ));
    assert!(matches!(
        futures_executor::block_on(engine.load_session(id.clone())),
        Err(EngineError::SessionIncarnationConflict)
    ));
    assert_eq!(first.incarnation_id(), first_incarnation);
    assert_eq!(first.record().id, id);
}

#[test]
fn independently_loaded_handles_share_one_live_turn_lease() {
    let store = MemoryStore::default();
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store)
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let id = SessionId::new("load-load-lease").unwrap();
    let seed = engine.create_test_session(id.clone());
    drop(prompt(&seed, "persist"));
    drop(seed);

    let first = futures_executor::block_on(engine.load_session(id.clone()))
        .unwrap()
        .unwrap();
    let second = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();
    let turn = prompt(&first, "first");
    assert_eq!(
        futures_executor::block_on(second.prompt("overlap")).unwrap_err(),
        EngineError::SessionBusy
    );
    drop(turn);
}

#[test]
fn racing_load_and_create_converge_on_state_without_losing_persisted_record() {
    let store = MemoryStore::default();
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store)
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let id = SessionId::new("load-create-lease").unwrap();
    let seed = engine.create_test_session(id.clone());
    drop(prompt(&seed, "persist"));
    drop(seed);

    let load = engine.load_session(id.clone());
    let created_while_load_pending = engine.create_test_session(id);
    let loaded = futures_executor::block_on(load).unwrap().unwrap();
    assert_eq!(
        created_while_load_pending.record().revision,
        SessionRevision(1)
    );
    assert_eq!(created_while_load_pending.record().next_turn_sequence, 2);

    let turn = prompt(&loaded, "loaded wins lease");
    assert_eq!(
        futures_executor::block_on(created_while_load_pending.prompt("overlap")).unwrap_err(),
        EngineError::SessionBusy
    );
    drop(turn);

    let created_after_load = engine.create_test_session(loaded.id());
    let turn = prompt(&created_after_load, "create reuses loaded state");
    assert_eq!(
        futures_executor::block_on(loaded.prompt("overlap again")).unwrap_err(),
        EngineError::SessionBusy
    );
    drop(turn);
}

#[derive(Clone, Debug, Default)]
struct RecordingProvider {
    requests: Arc<Mutex<Vec<ModelRequest>>>,
}

impl ModelProvider for RecordingProvider {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn stream(
        &self,
        request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async {
            Ok(Box::pin(stream::iter([Ok(ModelEvent::Stop {
                reason: StopReason::Completed,
            })])) as ModelEventStream)
        })
    }
}

#[derive(Clone, Debug)]
struct InterleavedSaveStore {
    loaded: Arc<Mutex<Option<SessionRecord>>>,
    save_ready: Arc<AtomicBool>,
    save_revision: SessionRevision,
}

#[derive(Clone, Debug)]
struct GatedCorruptLoadStore {
    corrupt: SessionRecord,
    load_ready: Arc<AtomicBool>,
}

impl SessionStore for GatedCorruptLoadStore {
    fn load(
        &self,
        _id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        let load_ready = Arc::clone(&self.load_ready);
        let corrupt = self.corrupt.clone();
        Box::pin(std::future::poll_fn(move |_context| {
            if load_ready.load(Ordering::Acquire) {
                Poll::Ready(Ok(Some(corrupt.clone())))
            } else {
                Poll::Pending
            }
        }))
    }

    fn save(
        &self,
        _record: SessionRecord,
        _expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async { Ok(SessionRevision(1)) })
    }
}

#[test]
fn corrupt_load_cannot_poison_a_create_racing_the_load() {
    let id = SessionId::new("corrupt-load-create").unwrap();
    let store = GatedCorruptLoadStore {
        corrupt: SessionRecord {
            id: id.clone(),
            incarnation_id: test_incarnation(&id),
            revision: SessionRevision(7),
            next_turn_sequence: 0,
            messages: vec![machine_god_core::Message::text(Role::User, "corrupt")],
            metadata: BTreeMap::new(),
        },
        load_ready: Arc::new(AtomicBool::new(false)),
    };
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store.clone())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let mut loading = engine.load_session(id.clone());
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);

    assert!(matches!(loading.as_mut().poll(&mut context), Poll::Pending));
    let created = engine.create_test_session(id);
    store.load_ready.store(true, Ordering::Release);
    assert!(matches!(
        loading.as_mut().poll(&mut context),
        Poll::Ready(Err(EngineError::Protocol(message)))
            if message.contains("turn sequence must be positive")
    ));

    assert_eq!(created.record().revision, SessionRevision(0));
    assert_eq!(created.record().next_turn_sequence, 1);
    let turn = prompt(&created, "first safe turn");
    assert_eq!(turn.id().as_str(), "turn-1");
}

impl SessionStore for InterleavedSaveStore {
    fn load(
        &self,
        _id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        let record = self.loaded.lock().unwrap().clone();
        Box::pin(async move { Ok(record) })
    }

    fn save(
        &self,
        _record: SessionRecord,
        _expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        let save_ready = Arc::clone(&self.save_ready);
        let save_revision = self.save_revision;
        Box::pin(std::future::poll_fn(move |_context| {
            if save_ready.load(Ordering::Acquire) {
                Poll::Ready(Ok(save_revision))
            } else {
                Poll::Pending
            }
        }))
    }
}

#[test]
fn delayed_save_does_not_regress_a_newer_concurrent_load() {
    let id = SessionId::new("save-load-interleaving").unwrap();
    let store = InterleavedSaveStore {
        loaded: Arc::new(Mutex::new(None)),
        save_ready: Arc::new(AtomicBool::new(false)),
        save_revision: SessionRevision(2),
    };
    let provider = RecordingProvider::default();
    let requests = Arc::clone(&provider.requests);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = engine.create_test_session(id.clone());
    let mut reserving = session.prompt("reserved against revision zero");
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);

    assert!(matches!(
        reserving.as_mut().poll(&mut context),
        Poll::Pending
    ));
    let newer = SessionRecord {
        id: id.clone(),
        incarnation_id: test_incarnation(&id),
        revision: SessionRevision(3),
        next_turn_sequence: 2,
        messages: vec![machine_god_core::Message::text(Role::User, "newer")],
        metadata: BTreeMap::new(),
    };
    *store.loaded.lock().unwrap() = Some(newer.clone());
    let loaded = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.record(), newer);

    store.save_ready.store(true, Ordering::Release);
    let Poll::Ready(Ok(mut turn)) = reserving.as_mut().poll(&mut context) else {
        panic!("delayed reservation did not complete");
    };
    assert_eq!(turn.id().as_str(), "turn-1");
    assert_eq!(session.record(), newer);

    let _started = futures_executor::block_on(turn.next()).unwrap().unwrap();
    let _model_stop = futures_executor::block_on(turn.next()).unwrap().unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].messages.len(), 1);
    assert_eq!(
        requests[0].messages[0],
        machine_god_core::Message::text(Role::User, "reserved against revision zero")
    );
}

#[test]
fn delayed_save_rejects_equal_revision_divergence_from_a_concurrent_load() {
    let id = SessionId::new("save-load-equal-divergence").unwrap();
    let store = InterleavedSaveStore {
        loaded: Arc::new(Mutex::new(None)),
        save_ready: Arc::new(AtomicBool::new(false)),
        save_revision: SessionRevision(2),
    };
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store.clone())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = engine.create_test_session(id.clone());
    let mut reserving = session.prompt("reserved against revision zero");
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);

    assert!(matches!(
        reserving.as_mut().poll(&mut context),
        Poll::Pending
    ));
    let divergent = stored_record(&id, 2, 2, "concurrent load");
    *store.loaded.lock().unwrap() = Some(divergent.clone());
    let loaded = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.record(), divergent);

    store.save_ready.store(true, Ordering::Release);
    assert!(matches!(
        reserving.as_mut().poll(&mut context),
        Poll::Ready(Err(EngineError::Protocol(message)))
            if message.contains("save diverged") && message.contains("same revision")
    ));
    assert_eq!(session.record(), divergent);
    assert!(!session.has_active_turn());
}

#[test]
fn delayed_higher_revision_save_rejects_turn_sequence_regression() {
    let id = SessionId::new("save-sequence-regression").unwrap();
    let store = InterleavedSaveStore {
        loaded: Arc::new(Mutex::new(None)),
        save_ready: Arc::new(AtomicBool::new(false)),
        save_revision: SessionRevision(7),
    };
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store.clone())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = engine.create_test_session(id.clone());
    let mut reserving = session.prompt("reserve low sequence");
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);

    assert!(matches!(
        reserving.as_mut().poll(&mut context),
        Poll::Pending
    ));
    let canonical = stored_record(&id, 6, 10, "concurrent canonical state");
    *store.loaded.lock().unwrap() = Some(canonical.clone());
    let loaded = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.record(), canonical);

    store.save_ready.store(true, Ordering::Release);
    assert!(matches!(
        reserving.as_mut().poll(&mut context),
        Poll::Ready(Err(EngineError::Protocol(message)))
            if message.contains("turn sequence regressed")
    ));
    assert_eq!(session.record(), canonical);
    assert!(!session.has_active_turn());
}

#[derive(Clone, Debug)]
struct ConflictReloadStore {
    loads: Arc<Mutex<VecDeque<SessionRecord>>>,
}

#[derive(Clone, Debug)]
enum ScriptedLoad {
    Ready(Option<SessionRecord>),
    MissingWhen(Arc<AtomicBool>),
}

#[derive(Clone, Debug)]
struct ScriptedConflictStore {
    loads: Arc<Mutex<VecDeque<ScriptedLoad>>>,
    expected_revisions: Arc<Mutex<Vec<Option<SessionRevision>>>>,
}

impl SessionStore for ScriptedConflictStore {
    fn load(
        &self,
        _id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        let load = self.loads.lock().unwrap().pop_front().unwrap();
        match load {
            ScriptedLoad::Ready(record) => Box::pin(async move { Ok(record) }),
            ScriptedLoad::MissingWhen(ready) => Box::pin(std::future::poll_fn(move |_context| {
                if ready.load(Ordering::Acquire) {
                    Poll::Ready(Ok(None))
                } else {
                    Poll::Pending
                }
            })),
        }
    }

    fn save(
        &self,
        _record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        self.expected_revisions
            .lock()
            .unwrap()
            .push(expected_revision);
        Box::pin(async {
            Err(SessionStoreError::new(
                machine_god_core::SessionStoreErrorKind::Conflict,
                "scripted_conflict",
                "force scripted conflict reload",
                true,
            ))
        })
    }
}

impl SessionStore for ConflictReloadStore {
    fn load(
        &self,
        _id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        let record = self.loads.lock().unwrap().pop_front();
        Box::pin(async move { Ok(record) })
    }

    fn save(
        &self,
        _record: SessionRecord,
        _expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async {
            Err(SessionStoreError::new(
                machine_god_core::SessionStoreErrorKind::Conflict,
                "controlled_conflict",
                "force conflict reload",
                true,
            ))
        })
    }
}

fn stored_record(
    id: &SessionId,
    revision: u64,
    next_turn_sequence: u64,
    text: &str,
) -> SessionRecord {
    SessionRecord {
        id: id.clone(),
        incarnation_id: test_incarnation(id),
        revision: SessionRevision(revision),
        next_turn_sequence,
        messages: vec![machine_god_core::Message::text(Role::User, text)],
        metadata: BTreeMap::new(),
    }
}

fn engine_with_conflict_loads(records: Vec<SessionRecord>) -> Engine {
    Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(ConflictReloadStore {
            loads: Arc::new(Mutex::new(records.into())),
        })
        .permission_handler(AllowOnce)
        .build()
        .unwrap()
}

#[test]
fn missing_conflict_load_cannot_clear_a_newer_concurrent_revision() {
    let id = SessionId::new("missing-conflict-concurrent-load").unwrap();
    let missing_ready = Arc::new(AtomicBool::new(false));
    let expected_revisions = Arc::new(Mutex::new(Vec::new()));
    let current = stored_record(&id, 5, 6, "current");
    let newer = stored_record(&id, 6, 7, "newer");
    let stale = stored_record(&id, 5, 7, "stale");
    let store = ScriptedConflictStore {
        loads: Arc::new(Mutex::new(
            vec![
                ScriptedLoad::Ready(Some(current)),
                ScriptedLoad::MissingWhen(Arc::clone(&missing_ready)),
                ScriptedLoad::Ready(Some(newer.clone())),
                ScriptedLoad::Ready(Some(stale)),
            ]
            .into(),
        )),
        expected_revisions: Arc::clone(&expected_revisions),
    };
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store)
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = futures_executor::block_on(engine.load_session(id.clone()))
        .unwrap()
        .unwrap();
    let mut reserving = session.prompt("reserve");
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);

    assert!(matches!(
        reserving.as_mut().poll(&mut context),
        Poll::Pending
    ));
    let concurrently_loaded = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();
    assert_eq!(concurrently_loaded.record(), newer);

    missing_ready.store(true, Ordering::Release);
    assert!(matches!(
        reserving.as_mut().poll(&mut context),
        Poll::Ready(Err(EngineError::Protocol(message)))
            if message.contains("stale revision")
    ));
    assert_eq!(
        *expected_revisions.lock().unwrap(),
        [Some(SessionRevision(5)), Some(SessionRevision(6))]
    );
    assert_eq!(session.record(), newer);
    assert!(!session.has_active_turn());
}

#[test]
fn stale_reload_cannot_regress_a_positive_record_after_missing_conflict_load() {
    let id = SessionId::new("missing-conflict-stale-reload").unwrap();
    let expected_revisions = Arc::new(Mutex::new(Vec::new()));
    let current = stored_record(&id, 5, 6, "current");
    let stale = stored_record(&id, 4, 6, "stale");
    let store = ScriptedConflictStore {
        loads: Arc::new(Mutex::new(
            vec![
                ScriptedLoad::Ready(Some(current.clone())),
                ScriptedLoad::Ready(None),
                ScriptedLoad::Ready(Some(stale)),
            ]
            .into(),
        )),
        expected_revisions: Arc::clone(&expected_revisions),
    };
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(store)
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();

    assert!(matches!(
        futures_executor::block_on(session.prompt("reserve")),
        Err(EngineError::Protocol(message)) if message.contains("stale revision")
    ));
    assert_eq!(
        *expected_revisions.lock().unwrap(),
        [Some(SessionRevision(5)), None]
    );
    assert_eq!(session.record(), current);
    assert!(!session.has_active_turn());
}

#[test]
fn conflict_reload_rejects_a_zero_turn_sequence() {
    let id = SessionId::new("conflict-zero-sequence").unwrap();
    let engine = engine_with_conflict_loads(vec![stored_record(&id, 1, 0, "corrupt")]);
    let session = engine.create_test_session(id);

    assert!(matches!(
        futures_executor::block_on(session.prompt("reserve")),
        Err(EngineError::Protocol(message))
            if message.contains("turn sequence must be positive")
    ));
    assert!(!session.has_active_turn());
}

#[test]
fn conflict_reload_rejects_a_zero_persisted_revision() {
    let id = SessionId::new("conflict-zero-revision").unwrap();
    let current = stored_record(&id, 5, 6, "current");
    let zero_revision = stored_record(&id, 0, 7, "invalid persisted sentinel");
    let engine = engine_with_conflict_loads(vec![current, zero_revision]);
    let session = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();

    assert!(matches!(
        futures_executor::block_on(session.prompt("reserve")),
        Err(EngineError::Protocol(message))
            if message.contains("stored session revision must be positive")
    ));
    assert_eq!(session.record().revision, SessionRevision(5));
    assert!(!session.has_active_turn());
}

#[test]
fn conflict_reload_rejects_a_higher_revision_with_a_lower_turn_sequence() {
    let id = SessionId::new("conflict-sequence-regression").unwrap();
    let current = stored_record(&id, 5, 10, "current");
    let regressed = stored_record(&id, 6, 9, "higher revision");
    let engine = engine_with_conflict_loads(vec![current, regressed]);
    let session = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();

    assert!(matches!(
        futures_executor::block_on(session.prompt("reserve")),
        Err(EngineError::Protocol(message)) if message.contains("turn sequence regressed")
    ));
    assert_eq!(session.record().revision, SessionRevision(5));
    assert_eq!(session.record().next_turn_sequence, 10);
    assert!(!session.has_active_turn());
}

#[test]
fn conflict_reload_rejects_a_stale_revision() {
    let id = SessionId::new("conflict-stale-revision").unwrap();
    let current = stored_record(&id, 5, 6, "current");
    let stale = stored_record(&id, 4, 6, "stale");
    let engine = engine_with_conflict_loads(vec![current, stale]);
    let session = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();

    assert!(matches!(
        futures_executor::block_on(session.prompt("reserve")),
        Err(EngineError::Protocol(message)) if message.contains("stale revision")
    ));
    assert_eq!(session.record().revision, SessionRevision(5));
    assert!(!session.has_active_turn());
}

#[test]
fn conflict_reload_rejects_equal_revision_divergence() {
    let id = SessionId::new("conflict-equal-divergence").unwrap();
    let current = stored_record(&id, 5, 6, "current");
    let divergent = stored_record(&id, 5, 6, "divergent");
    let engine = engine_with_conflict_loads(vec![current, divergent]);
    let session = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();

    assert!(matches!(
        futures_executor::block_on(session.prompt("reserve")),
        Err(EngineError::Protocol(message)) if message.contains("same revision")
    ));
    assert_eq!(
        session.record().messages[0],
        machine_god_core::Message::text(Role::User, "current")
    );
    assert!(!session.has_active_turn());
}

#[derive(Debug)]
struct PendingProvider;

impl ModelProvider for PendingProvider {
    fn name(&self) -> &'static str {
        "pending"
    }

    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        Box::pin(async {
            Ok(Box::pin(futures_util::stream::pending::<
                Result<ModelEvent, ProviderError>,
            >()) as ModelEventStream)
        })
    }
}

#[derive(Clone, Debug)]
enum BarrierPollOutcome {
    Stop,
    Error,
    End,
}

#[derive(Clone, Debug)]
struct BarrierPollProvider {
    barrier: Arc<Barrier>,
    outcome: BarrierPollOutcome,
}

impl ModelProvider for BarrierPollProvider {
    fn name(&self) -> &'static str {
        "barrier-poll"
    }

    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        let stream = BarrierPollStream {
            barrier: Arc::clone(&self.barrier),
            outcome: Some(self.outcome.clone()),
        };
        Box::pin(async move { Ok(Box::pin(stream) as ModelEventStream) })
    }
}

#[derive(Debug)]
struct BarrierPollStream {
    barrier: Arc<Barrier>,
    outcome: Option<BarrierPollOutcome>,
}

impl Stream for BarrierPollStream {
    type Item = Result<ModelEvent, ProviderError>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let Some(outcome) = self.outcome.take() else {
            return Poll::Ready(None);
        };
        self.barrier.wait();
        self.barrier.wait();
        match outcome {
            BarrierPollOutcome::Stop => Poll::Ready(Some(Ok(ModelEvent::Stop {
                reason: StopReason::Completed,
            }))),
            BarrierPollOutcome::Error => Poll::Ready(Some(Err(ProviderError::new(
                ProviderErrorKind::Unavailable,
                "racing_error",
                "provider failed while cancellation raced",
                true,
            )))),
            BarrierPollOutcome::End => Poll::Ready(None),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum BarrierStartupOutcome {
    Success,
    Error,
}

#[derive(Clone, Debug)]
struct BarrierStartupProvider {
    barrier: Arc<Barrier>,
    outcome: BarrierStartupOutcome,
}

impl ModelProvider for BarrierStartupProvider {
    fn name(&self) -> &'static str {
        "barrier-startup"
    }

    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        let barrier = Arc::clone(&self.barrier);
        let outcome = self.outcome;
        Box::pin(async move {
            barrier.wait();
            barrier.wait();
            match outcome {
                BarrierStartupOutcome::Success => {
                    Ok(Box::pin(futures_util::stream::pending()) as ModelEventStream)
                }
                BarrierStartupOutcome::Error => Err(ProviderError::new(
                    ProviderErrorKind::Unavailable,
                    "startup_race",
                    "provider startup failed while cancellation raced",
                    true,
                )),
            }
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum RetainedStream {
    Pending,
    Completed,
    TextThenPending,
}

#[derive(Clone, Debug)]
struct RetainingCancellationProvider {
    cancellation: Arc<Mutex<Option<CancellationToken>>>,
    stream: RetainedStream,
}

impl ModelProvider for RetainingCancellationProvider {
    fn name(&self) -> &'static str {
        "retaining-cancellation"
    }

    fn stream(
        &self,
        _request: ModelRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        *self.cancellation.lock().unwrap() = Some(cancellation);
        let stream = self.stream;
        Box::pin(async move {
            let stream: ModelEventStream = match stream {
                RetainedStream::Pending => Box::pin(futures_util::stream::pending()),
                RetainedStream::Completed => {
                    Box::pin(futures_util::stream::iter([Ok(ModelEvent::Stop {
                        reason: StopReason::Completed,
                    })]))
                }
                RetainedStream::TextThenPending => Box::pin(
                    futures_util::stream::iter([Ok(ModelEvent::TextDelta {
                        text: "partial".to_owned(),
                    })])
                    .chain(futures_util::stream::pending()),
                ),
            };
            Ok(stream)
        })
    }
}

#[derive(Debug, Default)]
struct TurnWakeCounter(AtomicUsize);

impl Wake for TurnWakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn assert_cancellation_during_provider_poll_wins(outcome: BarrierPollOutcome, session_id: &str) {
    let barrier = Arc::new(Barrier::new(2));
    let engine = Engine::builder()
        .provider(BarrierPollProvider {
            barrier: Arc::clone(&barrier),
            outcome,
        })
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new(session_id).unwrap());
    let mut turn = prompt(&session, "race");
    let handle = turn.handle();
    let started = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(started.payload, TurnEvent::Started));

    let cancelling = std::thread::spawn(move || {
        barrier.wait();
        assert!(handle.cancel());
        barrier.wait();
    });
    let completed = futures_executor::block_on(turn.next()).unwrap().unwrap();
    cancelling.join().unwrap();

    assert!(matches!(
        completed.payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert!(futures_executor::block_on(turn.next()).is_none());
    assert!(!session.has_active_turn());
}

#[test]
fn cancellation_observed_before_stop_poll_result_wins() {
    assert_cancellation_during_provider_poll_wins(
        BarrierPollOutcome::Stop,
        "cancel-before-stop-result",
    );
}

#[test]
fn cancellation_observed_before_error_poll_result_wins() {
    assert_cancellation_during_provider_poll_wins(
        BarrierPollOutcome::Error,
        "cancel-before-error-result",
    );
}

#[test]
fn cancellation_observed_before_end_poll_result_wins() {
    assert_cancellation_during_provider_poll_wins(
        BarrierPollOutcome::End,
        "cancel-before-end-result",
    );
}

fn assert_cancellation_during_provider_startup_wins(
    outcome: BarrierStartupOutcome,
    session_id: &str,
) {
    let barrier = Arc::new(Barrier::new(2));
    let engine = Engine::builder()
        .provider(BarrierStartupProvider {
            barrier: Arc::clone(&barrier),
            outcome,
        })
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new(session_id).unwrap());
    let mut turn = prompt(&session, "startup race");
    let handle = turn.handle();
    let started = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(started.payload, TurnEvent::Started));

    let cancelling = std::thread::spawn(move || {
        barrier.wait();
        assert!(handle.cancel());
        barrier.wait();
    });
    let completed = futures_executor::block_on(turn.next()).unwrap().unwrap();
    cancelling.join().unwrap();

    assert!(matches!(
        completed.payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert!(futures_executor::block_on(turn.next()).is_none());
    assert!(!session.has_active_turn());
}

#[test]
fn cancellation_observed_before_startup_error_wins() {
    assert_cancellation_during_provider_startup_wins(
        BarrierStartupOutcome::Error,
        "cancel-before-startup-error",
    );
}

#[test]
fn cancellation_observed_before_startup_success_wins() {
    assert_cancellation_during_provider_startup_wins(
        BarrierStartupOutcome::Success,
        "cancel-before-startup-success",
    );
}

#[test]
fn ready_nonterminal_event_does_not_retain_its_poller_waker() {
    let session = engine_with(PendingProvider)
        .create_test_session(SessionId::new("idle-turn-waker").unwrap());
    let mut turn = prompt(&session, "wait");
    let handle = turn.handle();
    let wake_counter = Arc::new(TurnWakeCounter::default());
    let waker = Waker::from(Arc::clone(&wake_counter));
    let mut context = Context::from_waker(&waker);
    let mut next = Box::pin(turn.next());

    assert!(matches!(
        next.as_mut().poll(&mut context),
        Poll::Ready(Some(Ok(EngineEvent {
            payload: TurnEvent::Started,
            ..
        })))
    ));
    drop(next);
    assert_eq!(Arc::strong_count(&wake_counter), 2);
    assert!(handle.cancel());
    assert_eq!(wake_counter.0.load(Ordering::Relaxed), 0);
}

#[test]
fn dropping_a_live_turn_cancels_provider_work_before_releasing_the_lease() {
    let retained = Arc::new(Mutex::new(None));
    let engine = Engine::builder()
        .provider(RetainingCancellationProvider {
            cancellation: Arc::clone(&retained),
            stream: RetainedStream::Pending,
        })
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("drop-cancels-provider").unwrap());
    let mut turn = prompt(&session, "wait");
    let handle = turn.handle();

    let _started = futures_executor::block_on(turn.next()).unwrap().unwrap();
    let mut pending = Box::pin(turn.next());
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);
    assert!(matches!(pending.as_mut().poll(&mut context), Poll::Pending));
    drop(pending);

    let provider_cancellation = retained.lock().unwrap().clone().unwrap();
    assert!(!provider_cancellation.is_cancelled());
    assert!(session.has_active_turn());
    drop(turn);

    assert!(provider_cancellation.is_cancelled());
    assert!(!handle.cancel());
    assert!(!session.has_active_turn());
}

#[test]
fn dropping_a_completed_turn_does_not_cancel_or_wake_again() {
    let retained = Arc::new(Mutex::new(None));
    let engine = Engine::builder()
        .provider(RetainingCancellationProvider {
            cancellation: Arc::clone(&retained),
            stream: RetainedStream::Completed,
        })
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("completed-drop-idempotent").unwrap());
    let mut turn = prompt(&session, "complete");

    let events = futures_executor::block_on(async {
        let mut events = Vec::new();
        while let Some(event) = turn.next().await {
            events.push(event.unwrap());
        }
        events
    });
    assert_eq!(events.len(), 3);
    assert!(!session.has_active_turn());

    let provider_cancellation = retained.lock().unwrap().clone().unwrap();
    let wake_counter = Arc::new(TurnWakeCounter::default());
    let waker = Waker::from(Arc::clone(&wake_counter));
    let mut context = Context::from_waker(&waker);
    let mut cancelled = Box::pin(provider_cancellation.cancelled());
    assert!(matches!(
        cancelled.as_mut().poll(&mut context),
        Poll::Pending
    ));

    drop(turn);
    assert!(!provider_cancellation.is_cancelled());
    assert_eq!(wake_counter.0.load(Ordering::Relaxed), 0);
}

#[test]
fn cancellation_is_idempotent_and_emits_a_terminal_event() {
    let session =
        engine_with(PendingProvider).create_test_session(SessionId::new("cancel-turn").unwrap());
    let incarnation_id = session.incarnation_id();
    let mut turn = prompt(&session, "wait");
    let handle = turn.handle();

    let events = futures_executor::block_on(async {
        let started = turn.next().await.unwrap().unwrap();
        assert!(handle.cancel());
        assert!(!handle.cancel());
        let completed = turn.next().await.unwrap().unwrap();
        (started, completed)
    });
    assert!(matches!(events.0.payload, TurnEvent::Started));
    assert_eq!(events.0.session_incarnation_id, incarnation_id);
    assert!(matches!(
        events.1.payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(events.1.session_incarnation_id, incarnation_id);
    assert!(futures_executor::block_on(turn.next()).is_none());
    assert!(!session.has_active_turn());
}

#[test]
fn provider_end_without_stop_is_a_structured_failure_event() {
    let provider = StaticProvider {
        events: vec![Ok(ModelEvent::TextDelta {
            text: "partial".to_owned(),
        })],
    };
    let session = engine_with(provider).create_test_session(SessionId::new("no-stop").unwrap());
    let events = futures_executor::block_on(prompt(&session, "go").collect::<Vec<_>>());
    let last = events.last().unwrap().as_ref().unwrap();
    assert!(matches!(
        &last.payload,
        TurnEvent::Failed { component, code, .. }
            if component == "provider" && code == "missing_stop"
    ));
}

#[test]
fn provider_errors_remain_structured_in_band() {
    let provider = StaticProvider {
        events: vec![Err(ProviderError::new(
            ProviderErrorKind::RateLimited,
            "rate_limited",
            "try later",
            true,
        ))],
    };
    let session =
        engine_with(provider).create_test_session(SessionId::new("provider-error").unwrap());
    let events = futures_executor::block_on(prompt(&session, "go").collect::<Vec<_>>());
    assert!(matches!(
        &events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Failed { code, retryable: true, .. } if code == "provider_failed"
    ));
}

const SINK_SECRET: &str = "SENTINEL_EVENT_SINK_SECRET";

fn hostile_sink_error() -> EventSinkError {
    EventSinkError::new(
        format!("hostile\n{SINK_SECRET}{}", "c".repeat(65_536)),
        format!("hostile message\n{SINK_SECRET}{}", "m".repeat(65_536)),
    )
}

#[derive(Debug)]
struct HostileRejectingSink;

impl EventSink for HostileRejectingSink {
    fn emit(&self, _event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        Box::pin(async { Err(hostile_sink_error()) })
    }
}

#[derive(Clone, Debug, Default)]
struct CancelAndRejectSink {
    handle: Arc<Mutex<Option<TurnHandle>>>,
}

impl CancelAndRejectSink {
    fn install(&self, handle: TurnHandle) {
        *self.handle.lock().unwrap() = Some(handle);
    }
}

impl EventSink for CancelAndRejectSink {
    fn emit(&self, _event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        let handle = self.handle.lock().unwrap().clone();
        Box::pin(async move {
            assert!(handle.expect("turn handle is installed").cancel());
            Err(hostile_sink_error())
        })
    }
}

#[derive(Clone, Debug, Default)]
struct RejectFirstModelSink {
    deliveries: Arc<AtomicUsize>,
}

impl EventSink for RejectFirstModelSink {
    fn emit(&self, _event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        let delivery = self.deliveries.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            if delivery == 0 {
                Ok(())
            } else {
                Err(EventSinkError::new(
                    "model_event_rejected",
                    "observer rejected the first model event",
                ))
            }
        })
    }
}

#[derive(Debug)]
struct PendingSink;

impl EventSink for PendingSink {
    fn emit(&self, _event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        Box::pin(std::future::pending())
    }
}

#[derive(Clone, Copy, Debug)]
enum BarrierDeliveryOutcome {
    Success,
    Error,
}

#[derive(Clone, Debug)]
struct BarrierDeliverySink {
    barrier: Arc<Barrier>,
    outcome: BarrierDeliveryOutcome,
}

impl EventSink for BarrierDeliverySink {
    fn emit(&self, _event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        let barrier = Arc::clone(&self.barrier);
        let outcome = self.outcome;
        Box::pin(async move {
            barrier.wait();
            barrier.wait();
            match outcome {
                BarrierDeliveryOutcome::Success => Ok(()),
                BarrierDeliveryOutcome::Error => Err(EventSinkError::new(
                    "delivery_race",
                    "observer failed while cancellation raced",
                )),
            }
        })
    }
}

fn assert_cancellation_during_nonterminal_delivery_wins(
    outcome: BarrierDeliveryOutcome,
    session_id: &str,
) {
    let barrier = Arc::new(Barrier::new(2));
    let engine = Engine::builder()
        .provider(PendingProvider)
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .event_sink(BarrierDeliverySink {
            barrier: Arc::clone(&barrier),
            outcome,
        })
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new(session_id).unwrap());
    let mut turn = prompt(&session, "delivery race");
    let handle = turn.handle();

    let cancelling = std::thread::spawn(move || {
        barrier.wait();
        assert!(handle.cancel());
        barrier.wait();
    });
    let completed = futures_executor::block_on(turn.next()).unwrap().unwrap();
    cancelling.join().unwrap();

    assert!(matches!(
        completed.payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert!(futures_executor::block_on(turn.next()).is_none());
    assert!(!session.has_active_turn());
}

#[test]
fn cancellation_observed_before_nonterminal_delivery_success_wins() {
    assert_cancellation_during_nonterminal_delivery_wins(
        BarrierDeliveryOutcome::Success,
        "cancel-before-delivery-success",
    );
}

#[test]
fn cancellation_observed_before_nonterminal_delivery_error_wins() {
    assert_cancellation_during_nonterminal_delivery_wins(
        BarrierDeliveryOutcome::Error,
        "cancel-before-delivery-error",
    );
}

#[derive(Clone, Debug)]
struct TerminalGateSink {
    terminal_ready: Arc<AtomicBool>,
}

impl EventSink for TerminalGateSink {
    fn emit(&self, event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        let terminal_ready = Arc::clone(&self.terminal_ready);
        let is_provider_terminal = matches!(
            event.payload,
            TurnEvent::Model {
                event: ModelEvent::Stop { .. }
            } | TurnEvent::Failed { .. }
        );
        Box::pin(std::future::poll_fn(move |_context| {
            if !is_provider_terminal || terminal_ready.load(Ordering::Acquire) {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        }))
    }
}

#[derive(Debug)]
struct PendingAfterStartedSink;

impl EventSink for PendingAfterStartedSink {
    fn emit(&self, event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        let is_started = matches!(event.payload, TurnEvent::Started);
        Box::pin(std::future::poll_fn(move |_context| {
            if is_started {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        }))
    }
}

#[test]
fn staged_cancellation_still_bypasses_pending_observer_delivery() {
    let engine = Engine::builder()
        .provider(PendingProvider)
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .event_sink(PendingAfterStartedSink)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("staged-cancel-delivery").unwrap());
    let mut turn = prompt(&session, "cancel");
    let handle = turn.handle();
    let started = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(started.payload, TurnEvent::Started));

    assert!(handle.cancel());
    let cancelled = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(
        cancelled.payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert!(!session.has_active_turn());
}

#[test]
fn cancellation_after_a_delivered_stop_preserves_provider_completion() {
    let provider = StaticProvider {
        events: vec![Ok(ModelEvent::Stop {
            reason: StopReason::Completed,
        })],
    };
    let session =
        engine_with(provider).create_test_session(SessionId::new("stop-then-cancel").unwrap());
    let mut turn = prompt(&session, "complete");
    let handle = turn.handle();

    let started = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(started.payload, TurnEvent::Started));
    let stopped = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(
        stopped.payload,
        TurnEvent::Model {
            event: ModelEvent::Stop {
                reason: StopReason::Completed
            }
        }
    ));
    assert!(handle.cancel());

    let completed = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(
        completed.payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
    assert!(futures_executor::block_on(turn.next()).is_none());
    assert!(!session.has_active_turn());
}

#[test]
fn cancellation_during_stop_delivery_preserves_provider_completion() {
    let terminal_ready = Arc::new(AtomicBool::new(false));
    let engine = Engine::builder()
        .provider(StaticProvider {
            events: vec![Ok(ModelEvent::Stop {
                reason: StopReason::Completed,
            })],
        })
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .event_sink(TerminalGateSink {
            terminal_ready: Arc::clone(&terminal_ready),
        })
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("cancel-pending-stop").unwrap());
    let mut turn = prompt(&session, "complete");
    let handle = turn.handle();
    let started = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(started.payload, TurnEvent::Started));
    let mut next = Box::pin(turn.next());
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);

    assert!(matches!(next.as_mut().poll(&mut context), Poll::Pending));
    assert!(handle.cancel());
    assert!(matches!(next.as_mut().poll(&mut context), Poll::Pending));
    terminal_ready.store(true, Ordering::Release);
    let Poll::Ready(Some(Ok(stopped))) = next.as_mut().poll(&mut context) else {
        panic!("provider Stop was not preserved after cancellation");
    };
    assert!(matches!(
        stopped.payload,
        TurnEvent::Model {
            event: ModelEvent::Stop {
                reason: StopReason::Completed
            }
        }
    ));
    drop(next);

    let completed = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(
        completed.payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
    assert!(!session.has_active_turn());
}

#[test]
fn cancellation_during_provider_failure_delivery_preserves_failure() {
    let terminal_ready = Arc::new(AtomicBool::new(false));
    let engine = Engine::builder()
        .provider(StaticProvider {
            events: vec![Err(ProviderError::new(
                ProviderErrorKind::RateLimited,
                "rate_limited",
                "try later",
                true,
            ))],
        })
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .event_sink(TerminalGateSink {
            terminal_ready: Arc::clone(&terminal_ready),
        })
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("cancel-pending-failure").unwrap());
    let mut turn = prompt(&session, "fail");
    let handle = turn.handle();
    let started = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(started.payload, TurnEvent::Started));
    let mut next = Box::pin(turn.next());
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);

    assert!(matches!(next.as_mut().poll(&mut context), Poll::Pending));
    assert!(handle.cancel());
    assert!(matches!(next.as_mut().poll(&mut context), Poll::Pending));
    terminal_ready.store(true, Ordering::Release);
    let Poll::Ready(Some(Ok(failed))) = next.as_mut().poll(&mut context) else {
        panic!("provider failure was not preserved after cancellation");
    };
    assert!(matches!(
        failed.payload,
        TurnEvent::Failed {
            ref code,
            retryable: true,
            ..
        } if code == "provider_failed"
    ));
    drop(next);
    assert!(futures_executor::block_on(turn.next()).is_none());
    assert!(!session.has_active_turn());
}

fn assert_cancellation_does_not_wake_pending_terminal_delivery(
    provider_events: Vec<Result<ModelEvent, ProviderError>>,
    session_id: &str,
) {
    let engine = Engine::builder()
        .provider(StaticProvider {
            events: provider_events,
        })
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .event_sink(PendingAfterStartedSink)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new(session_id).unwrap());
    let mut turn = prompt(&session, "terminal");
    let handle = turn.handle();
    let started = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(started.payload, TurnEvent::Started));

    let wake_counter = Arc::new(TurnWakeCounter::default());
    let waker = Waker::from(Arc::clone(&wake_counter));
    let mut context = Context::from_waker(&waker);
    let mut next = Box::pin(turn.next());

    assert!(matches!(next.as_mut().poll(&mut context), Poll::Pending));
    assert_eq!(wake_counter.0.load(Ordering::Relaxed), 0);
    assert!(handle.cancel());
    assert_eq!(wake_counter.0.load(Ordering::Relaxed), 0);

    for _ in 0..4 {
        assert!(matches!(next.as_mut().poll(&mut context), Poll::Pending));
        assert_eq!(wake_counter.0.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn cancellation_does_not_self_wake_pending_stop_delivery() {
    assert_cancellation_does_not_wake_pending_terminal_delivery(
        vec![Ok(ModelEvent::Stop {
            reason: StopReason::Completed,
        })],
        "cancel-pending-stop-no-self-wake",
    );
}

#[test]
fn cancellation_does_not_self_wake_pending_provider_failure_delivery() {
    assert_cancellation_does_not_wake_pending_terminal_delivery(
        vec![Err(ProviderError::new(
            ProviderErrorKind::Unavailable,
            "terminal_failure",
            "provider failed",
            true,
        ))],
        "cancel-pending-failure-no-self-wake",
    );
}

#[test]
fn pending_delivery_rebinds_cancellation_to_the_latest_poller() {
    let engine = Engine::builder()
        .provider(PendingProvider)
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .event_sink(PendingSink)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("pending-sink-repoll").unwrap());
    let mut turn = prompt(&session, "wait");
    let handle = turn.handle();
    let first_counter = Arc::new(TurnWakeCounter::default());
    let second_counter = Arc::new(TurnWakeCounter::default());
    let first_waker = Waker::from(Arc::clone(&first_counter));
    let second_waker = Waker::from(Arc::clone(&second_counter));
    let mut first_context = Context::from_waker(&first_waker);
    let mut second_context = Context::from_waker(&second_waker);
    let mut next = Box::pin(turn.next());

    assert!(matches!(
        next.as_mut().poll(&mut first_context),
        Poll::Pending
    ));
    assert!(matches!(
        next.as_mut().poll(&mut second_context),
        Poll::Pending
    ));
    assert_eq!(Arc::strong_count(&first_counter), 2);
    assert_eq!(Arc::strong_count(&second_counter), 3);

    assert!(handle.cancel());
    assert_eq!(first_counter.0.load(Ordering::Relaxed), 0);
    assert_eq!(second_counter.0.load(Ordering::Relaxed), 1);
    assert!(matches!(
        next.as_mut().poll(&mut second_context),
        Poll::Ready(Some(Ok(EngineEvent {
            payload: TurnEvent::Completed {
                reason: StopReason::Cancelled,
                ..
            },
            ..
        })))
    ));
    drop(next);
    assert!(!session.has_active_turn());
}

#[test]
fn cancellation_aborts_pending_observer_delivery_and_releases_turn() {
    let engine = Engine::builder()
        .provider(PendingProvider)
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .event_sink(PendingSink)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("pending-sink").unwrap());
    let incarnation_id = session.incarnation_id();
    let mut turn = prompt(&session, "wait");
    let handle = turn.handle();
    let mut next = Box::pin(turn.next());
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);

    assert!(matches!(next.as_mut().poll(&mut context), Poll::Pending));
    assert!(session.has_active_turn());
    assert!(handle.cancel());
    let Poll::Ready(Some(Ok(cancelled))) = next.as_mut().poll(&mut context) else {
        panic!("cancellation did not interrupt the pending event sink");
    };
    assert!(matches!(
        cancelled.payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(cancelled.session_incarnation_id, incarnation_id);
    drop(next);
    assert!(!session.has_active_turn());
    assert!(futures_executor::block_on(turn.next()).is_none());
}

#[test]
fn observer_failure_terminates_and_releases_turn() {
    let engine = Engine::builder()
        .provider(StaticProvider::completed())
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .event_sink(HostileRejectingSink)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("sink-error").unwrap());
    let mut turn = prompt(&session, "go");
    let error = futures_executor::block_on(turn.next())
        .unwrap()
        .unwrap_err();
    let display = error.to_string();
    let debug = format!("{error:?}");
    assert!(!display.contains(SINK_SECRET));
    assert!(!debug.contains(SINK_SECRET));
    assert!(!display.contains('\n'));
    assert!(!debug.contains("hostile message"));
    let EngineError::EventSink(error) = error else {
        panic!("expected an event-sink error")
    };
    assert_eq!(error.code, "event_sink_failed");
    assert_eq!(error.message, "event sink failed");
    assert!(futures_executor::block_on(turn.next()).is_none());
    assert!(!session.has_active_turn());
}

#[test]
fn cancellation_observed_during_ready_sink_error_wins_without_leaking_it() {
    let sink = CancelAndRejectSink::default();
    let engine = Engine::builder()
        .provider(PendingProvider)
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .event_sink(sink.clone())
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("cancel-ready-sink-error").unwrap());
    let mut turn = prompt(&session, "go");
    let handle = turn.handle();
    sink.install(handle.clone());

    let event = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(handle.is_cancelled());
    assert!(matches!(
        event.payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    let debug = format!("{event:?}");
    let serialized = serde_json::to_string(&event).unwrap();
    assert!(!debug.contains(SINK_SECRET));
    assert!(!serialized.contains(SINK_SECRET));
    assert!(futures_executor::block_on(turn.next()).is_none());
    assert!(!session.has_active_turn());
}

#[test]
fn nonterminal_observer_failure_cancels_retained_provider_work() {
    let retained = Arc::new(Mutex::new(None));
    let sink = RejectFirstModelSink::default();
    let deliveries = Arc::clone(&sink.deliveries);
    let engine = Engine::builder()
        .provider(RetainingCancellationProvider {
            cancellation: Arc::clone(&retained),
            stream: RetainedStream::TextThenPending,
        })
        .session_store(MemoryStore::default())
        .permission_handler(AllowOnce)
        .event_sink(sink)
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("sink-failure-cancels").unwrap());
    let mut turn = prompt(&session, "go");
    let handle = turn.handle();

    let started = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(started.payload, TurnEvent::Started));
    let result = futures_executor::block_on(turn.next()).unwrap();
    assert!(matches!(result, Err(EngineError::EventSink(_))));
    assert_eq!(deliveries.load(Ordering::Relaxed), 2);

    let provider_cancellation = retained.lock().unwrap().clone().unwrap();
    assert!(provider_cancellation.is_cancelled());
    assert!(!handle.cancel());
    assert!(!session.has_active_turn());
    assert!(futures_executor::block_on(turn.next()).is_none());
}

// Compile-time assertion that the turn is a sendable stream and can move
// between executor workers without changing its public type.
fn assert_send_stream<T>(_: &T)
where
    T: Stream<Item = Result<EngineEvent, EngineError>> + Send + Unpin,
{
}

#[allow(dead_code)]
fn assert_turn_surface(engine: &Engine) {
    let session = engine.create_test_session(SessionId::new("surface").unwrap());
    let turn = prompt(&session, "prompt");
    assert_send_stream(&turn);
}
