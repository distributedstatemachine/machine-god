use futures_core::Stream;
use futures_util::{StreamExt, stream};
use machine_god_core::{
    BoxFuture, BuildError, CancellationToken, Engine, EngineBuilder, EngineError, EngineEvent,
    EventSink, EventSinkError, ModelEvent, ModelEventStream, ModelProvider, ModelRequest,
    PermissionDecision, PermissionError, PermissionGrantScope, PermissionHandler,
    PermissionRequest, ProviderError, ProviderErrorKind, Role, SessionId, SessionRecord,
    SessionRevision, SessionStore, SessionStoreError, StopReason, Tool, ToolContext, ToolError,
    ToolName, ToolOutput, ToolSpec, Turn, TurnEvent,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

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
                    current.id == record.id && current.revision == expected
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
    let store = MemoryStore {
        record: Arc::new(Mutex::new(Some(SessionRecord {
            id: id.clone(),
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
    assert_eq!(session.record().revision, SessionRevision(7));
    assert_eq!(session.record().messages.len(), 1);
}

#[test]
fn load_session_rejects_a_mismatched_store_record() {
    let store = MemoryStore {
        record: Arc::new(Mutex::new(Some(SessionRecord::empty(
            SessionId::new("different").unwrap(),
        )))),
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
fn turn_events_are_ordered_and_release_session_at_terminal_event() {
    let session =
        engine_with(StaticProvider::completed()).create_session(SessionId::new("ordered").unwrap());
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
fn dropping_turn_releases_shared_session_lease() {
    let session = engine_with(StaticProvider::completed())
        .create_session(SessionId::new("drop-turn").unwrap());
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

    let created = engine.create_session(id.clone());
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

#[test]
fn cancellation_is_idempotent_and_emits_a_terminal_event() {
    let session =
        engine_with(PendingProvider).create_session(SessionId::new("cancel-turn").unwrap());
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
    assert!(matches!(
        events.1.payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
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
    let session = engine_with(provider).create_session(SessionId::new("no-stop").unwrap());
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
    let session = engine_with(provider).create_session(SessionId::new("provider-error").unwrap());
    let events = futures_executor::block_on(prompt(&session, "go").collect::<Vec<_>>());
    assert!(matches!(
        &events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Failed { code, retryable: true, .. } if code == "rate_limited"
    ));
}

#[derive(Debug)]
struct RejectingSink;

impl EventSink for RejectingSink {
    fn emit(&self, _event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        Box::pin(async { Err(EventSinkError::new("closed", "observer closed")) })
    }
}

#[derive(Debug)]
struct PendingSink;

impl EventSink for PendingSink {
    fn emit(&self, _event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        Box::pin(std::future::pending())
    }
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
    let session = engine.create_session(SessionId::new("pending-sink").unwrap());
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
        .event_sink(RejectingSink)
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("sink-error").unwrap());
    let mut turn = prompt(&session, "go");
    let result = futures_executor::block_on(turn.next()).unwrap();
    assert!(matches!(result, Err(EngineError::EventSink(_))));
    assert!(futures_executor::block_on(turn.next()).is_none());
    assert!(!session.has_active_turn());
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
    let session = engine.create_session(SessionId::new("surface").unwrap());
    let turn = prompt(&session, "prompt");
    assert_send_stream(&turn);
}
