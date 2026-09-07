use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use futures_core::Stream;
use futures_executor::block_on;
use futures_util::{StreamExt, task::noop_waker};
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Engine, EngineError, EngineLimits,
    InferenceOptions, Message, ModelEvent, PermissionDecision, PermissionGrantScope, Role, Session,
    SessionId, SessionIncarnationId, SessionRecord, SessionRevision, SessionStore,
    SessionStoreError, SessionStoreErrorKind, StopReason, Tool, ToolCall, ToolCallId, ToolContext,
    ToolError, ToolExecution, ToolName, ToolOutput, ToolSpec, Turn, TurnEvent,
    TurnToolRegistration,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, RecordedSessionStoreCall,
    ScriptedModelProvider, ScriptedPermissionHandler, ScriptedTool, SessionStoreScript,
    SessionStoreStep, ToolStep,
};
use serde_json::{Value, json};

fn call(id: &str, tool: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new(tool).unwrap(),
        arguments: json!({"path": "retained/evidence"}),
    }
}

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: ToolName::new(name).unwrap(),
        description: "test tool".to_owned(),
        input_schema: json!({"type": "object"}),
    }
}

fn record() -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new("continuation-session").unwrap(),
        SessionIncarnationId::new("continuation-incarnation").unwrap(),
    );
    record.revision = SessionRevision(7);
    record.next_turn_sequence = 12;
    record.messages = vec![
        Message::text(Role::User, "original user input"),
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::ToolCall {
                    call: call("known", "action"),
                },
                ContentBlock::ToolCall {
                    call: call("uncertain", "action"),
                },
            ],
        },
        Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id: ToolCallId::new("known").unwrap(),
                output: ToolOutput::success(
                    json!({"archive": "retained-result", "confirmed": true}),
                ),
            }],
        },
        Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id: ToolCallId::new("uncertain").unwrap(),
                output: ToolOutput {
                    is_error: true,
                    content: json!({"code": "tool_result_unknown", "message": "tool result status is unknown"}),
                },
            }],
        },
    ];
    record
        .metadata
        .insert("host.owned".to_owned(), json!({"preserve": true}));
    record
}

fn finish() -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::TextDelta {
            text: "continued answer".to_owned(),
        },
        ModelEvent::Stop {
            reason: StopReason::Completed,
        },
    ])
}

fn tool_round(calls: Vec<ToolCall>) -> ModelProviderStep {
    ModelProviderStep::events(
        calls
            .into_iter()
            .map(|call| ModelEvent::ToolCall { call })
            .chain([ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            }]),
    )
}

fn allow_turn() -> PermissionStep {
    PermissionStep::Decision(PermissionDecision::Allow {
        scope: PermissionGrantScope::Turn,
    })
}

fn store(record: SessionRecord, script: SessionStoreScript) -> InMemorySessionStore {
    InMemorySessionStore::configured(BTreeMap::from([(record.id.clone(), record)]), script, 100)
}

fn engine(
    store: impl SessionStore,
    provider: ScriptedModelProvider,
    permission: ScriptedPermissionHandler,
    tool: ScriptedTool,
    limits: EngineLimits,
) -> Engine {
    Engine::builder()
        .session_store(store)
        .provider(provider)
        .permission_handler(permission)
        .tool(tool)
        .limits(limits)
        .build()
        .unwrap()
}

fn load(engine: &Engine) -> Session {
    block_on(engine.load_session(record().id)).unwrap().unwrap()
}

fn no_tools() -> ScriptedTool {
    ScriptedTool::new(spec("action"), [])
}

fn pending<T>(future: &mut BoxFuture<'_, T>) {
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&noop_waker()))
            .is_pending()
    );
}

fn complete(mut turn: Turn) {
    let events = block_on(async { turn.by_ref().collect::<Vec<_>>().await });
    assert!(events.iter().all(Result::is_ok));
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
}

#[test]
fn continuation_is_inert_reserves_a_fresh_id_and_preserves_exact_history() {
    let original = record();
    let store = store(original.clone(), SessionStoreScript::default());
    let provider = ScriptedModelProvider::new("continuation", [finish()]);
    let permission = ScriptedPermissionHandler::new([]);
    let tool = no_tools();
    let engine = engine(
        store.clone(),
        provider.clone(),
        permission.clone(),
        tool.clone(),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let options = InferenceOptions {
        model: Some("selected/model".to_owned()),
        max_output_tokens: Some(17),
        temperature: Some(0.25),
        metadata: BTreeMap::from([("host.option".to_owned(), json!(true))]),
    };
    let operation = session.continue_turn(options.clone());
    assert!(!session.has_active_turn());
    assert_eq!(store.calls().len(), 1);
    assert!(provider.requests().is_empty());
    let turn = block_on(operation).unwrap();
    assert_eq!(turn.id().as_str(), "turn-12");
    let mut reserved = original.clone();
    reserved.revision = SessionRevision(8);
    reserved.next_turn_sequence = 13;
    assert_eq!(session.record(), reserved);
    assert_eq!(store.record(&session.id()).unwrap(), reserved);
    assert!(provider.requests().is_empty());
    complete(turn);
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].request.messages, original.messages);
    assert_eq!(requests[0].request.options, options);
    assert_eq!(requests[0].request.turn_id.as_str(), "turn-12");
    assert_eq!(
        requests[0].request.session_incarnation_id,
        original.incarnation_id
    );
    assert!(permission.requests().is_empty());
    assert!(tool.invocations().is_empty());
    let after = session.record();
    assert_eq!(
        &after.messages[..original.messages.len()],
        original.messages.as_slice()
    );
    assert_eq!(after.messages.len(), original.messages.len() + 1);
    assert_eq!(after.metadata, original.metadata);
}

#[test]
fn reserved_ids_remain_consumed_after_turn_drop_and_engine_reload() {
    let store = store(record(), SessionStoreScript::default());
    let first = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        EngineLimits::default(),
    );
    let session = load(&first);
    let turn = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
    assert_eq!(turn.id().as_str(), "turn-12");
    let handle = turn.handle();
    drop(turn);
    assert!(handle.is_cancelled());
    drop(session);
    drop(first);
    let second = engine(
        store,
        ScriptedModelProvider::new("unused", []),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        EngineLimits::default(),
    );
    let session = load(&second);
    let turn = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
    assert_eq!(turn.id().as_str(), "turn-13");
    assert_eq!(session.record().messages, record().messages);
    drop(turn);
}

#[test]
fn continuation_shares_the_prompt_and_metadata_lease() {
    let store = store(
        record(),
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Pending,
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
            ]),
            ..SessionStoreScript::default()
        },
    );
    let engine = engine(
        store,
        ScriptedModelProvider::new("unused", []),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let clone = load(&engine);
    let mut continuation = session.continue_turn(InferenceOptions::default());
    pending(&mut continuation);
    assert!(clone.has_active_turn());
    assert!(matches!(
        block_on(clone.prompt("blocked")),
        Err(EngineError::SessionBusy)
    ));
    assert!(matches!(
        block_on(clone.update_metadata(SessionRevision(7), BTreeMap::new())),
        Err(EngineError::SessionBusy)
    ));
    assert!(matches!(
        block_on(clone.continue_turn(InferenceOptions::default())),
        Err(EngineError::SessionBusy)
    ));
    drop(continuation);
    assert!(!session.has_active_turn());
    let prompt = block_on(session.prompt("ordinary prompt unchanged")).unwrap();
    assert!(matches!(
        block_on(clone.continue_turn(InferenceOptions::default())),
        Err(EngineError::SessionBusy)
    ));
    assert_eq!(
        session.record().messages.last().unwrap(),
        &Message::text(Role::User, "ordinary prompt unchanged")
    );
    drop(prompt);
    let turn = block_on(clone.continue_turn(InferenceOptions::default())).unwrap();
    assert_eq!(turn.id().as_str(), "turn-13");
    drop(turn);
}

#[test]
fn empty_history_fails_before_reservation_for_new_and_loaded_sessions() {
    for persisted in [false, true] {
        let mut empty = record();
        empty.messages.clear();
        let store = store(empty.clone(), SessionStoreScript::default());
        let engine = engine(
            store.clone(),
            ScriptedModelProvider::new("unused", []),
            ScriptedPermissionHandler::new([]),
            no_tools(),
            EngineLimits::default(),
        );
        let session = if persisted {
            load(&engine)
        } else {
            engine
                .create_session(empty.id, empty.incarnation_id)
                .unwrap()
        };
        let error = block_on(session.continue_turn(InferenceOptions::default())).unwrap_err();
        assert!(
            matches!(error, EngineError::Protocol(message) if message == "cannot continue a session with empty history")
        );
        assert!(
            !store
                .calls()
                .iter()
                .any(|call| matches!(call, RecordedSessionStoreCall::Save { .. }))
        );
        assert!(!session.has_active_turn());
    }
}

#[test]
fn exhausted_allocator_and_invalid_options_fail_before_save() {
    let mut exhausted = record();
    exhausted.next_turn_sequence = u64::MAX;
    let store = store(exhausted, SessionStoreScript::default());
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        EngineLimits::default(),
    );
    let session = load(&engine);
    assert!(
        matches!(block_on(session.continue_turn(InferenceOptions::default())), Err(EngineError::Protocol(message)) if message == "session turn sequence is exhausted")
    );
    let options = InferenceOptions {
        model: Some("x".repeat(70_000)),
        ..InferenceOptions::default()
    };
    assert!(matches!(
        block_on(session.continue_turn(options)),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(store.calls().len(), 1);
    assert!(!session.has_active_turn());
}

#[test]
fn continuation_does_not_spend_a_user_message_slot_but_respects_output_limits() {
    let initial = record();
    let store = store(initial.clone(), SessionStoreScript::default());
    let limits = EngineLimits {
        max_transcript_messages: NonZeroUsize::new(initial.messages.len()).unwrap(),
        ..EngineLimits::default()
    };
    let provider = ScriptedModelProvider::new("bounded", [finish()]);
    let engine = engine(
        store.clone(),
        provider.clone(),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        limits,
    );
    let session = load(&engine);
    assert!(block_on(session.prompt("would-exceed-history")).is_err());
    let turn = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Failed { .. }
    ));
    assert_eq!(provider.requests().len(), 1);
    assert_eq!(session.record().messages, initial.messages);
    assert_eq!(session.record().next_turn_sequence, 13);
}

#[test]
fn stale_continuation_reload_preserves_newer_evidence_and_allocator() {
    let store = store(record(), SessionStoreScript::default());
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let mut external = record();
    external.next_turn_sequence = 20;
    external
        .messages
        .push(Message::text(Role::Assistant, "newer confirmed evidence"));
    external.metadata.insert("newer".to_owned(), json!(true));
    block_on(store.save(external.clone(), Some(SessionRevision(7)))).unwrap();
    let turn = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
    assert_eq!(turn.id().as_str(), "turn-20");
    assert_eq!(session.record().messages, external.messages);
    assert_eq!(session.record().metadata, external.metadata);
    assert_eq!(session.record().next_turn_sequence, 21);
    drop(turn);
}

#[test]
fn a_conflict_reload_that_removed_history_cannot_continue() {
    let store = store(record(), SessionStoreScript::default());
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let mut external = record();
    external.messages.clear();
    block_on(store.save(external, Some(SessionRevision(7)))).unwrap();
    assert!(
        matches!(block_on(session.continue_turn(InferenceOptions::default())), Err(EngineError::Protocol(message)) if message == "cannot continue a session with empty history")
    );
    assert!(session.record().messages.is_empty());
    assert_eq!(store.calls().len(), 4);
}

#[test]
fn store_failure_is_redacted_and_never_starts_provider_or_duplicates_input() {
    let failure = SessionStoreError::new(
        SessionStoreErrorKind::Unavailable,
        "secret-code",
        "secret-message",
        true,
    );
    let store = store(
        record(),
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Error(failure),
                SessionStoreStep::Pass,
            ]),
            ..SessionStoreScript::default()
        },
    );
    let provider = ScriptedModelProvider::new("unused", []);
    let engine = engine(
        store.clone(),
        provider.clone(),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let error = block_on(session.continue_turn(InferenceOptions::default())).unwrap_err();
    assert!(
        matches!(error, EngineError::Store(error) if error.code == "store_failed" && error.message == "session store failed")
    );
    assert_eq!(session.record(), record());
    assert!(provider.requests().is_empty());
    let turn = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
    assert_eq!(turn.id().as_str(), "turn-12");
    assert_eq!(session.record().messages, record().messages);
    drop(turn);
}

#[derive(Clone)]
struct CommitThenPendingStore {
    inner: InMemorySessionStore,
    pause_next: Arc<AtomicBool>,
}

impl SessionStore for CommitThenPendingStore {
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
        Box::pin(async move {
            let revision = self.inner.save(record, expected).await?;
            if self.pause_next.swap(false, Ordering::AcqRel) {
                std::future::pending::<()>().await;
            }
            Ok(revision)
        })
    }
}

#[test]
fn dropped_committed_reservation_consumes_its_turn_id_without_repeating_input() {
    let inner = store(record(), SessionStoreScript::default());
    let store = CommitThenPendingStore {
        inner: inner.clone(),
        pause_next: Arc::new(AtomicBool::new(true)),
    };
    let engine = engine(
        store,
        ScriptedModelProvider::new("unused", []),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let mut operation = session.continue_turn(InferenceOptions::default());
    pending(&mut operation);
    assert_eq!(inner.record(&session.id()).unwrap().next_turn_sequence, 13);
    assert_eq!(session.record().next_turn_sequence, 12);
    drop(operation);
    let turn = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
    assert_eq!(turn.id().as_str(), "turn-13");
    assert_eq!(session.record().messages, record().messages);
    drop(turn);
}

#[test]
fn continuation_reloads_after_an_uncertain_metadata_save() {
    let inner = store(record(), SessionStoreScript::default());
    let store = CommitThenPendingStore {
        inner: inner.clone(),
        pause_next: Arc::new(AtomicBool::new(true)),
    };
    let engine = engine(
        store,
        ScriptedModelProvider::new("unused", []),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let metadata = BTreeMap::from([("committed".to_owned(), json!(true))]);
    let mut operation = session.update_metadata(SessionRevision(7), metadata.clone());
    pending(&mut operation);
    drop(operation);
    let turn = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
    assert_eq!(turn.id().as_str(), "turn-12");
    assert_eq!(session.record().metadata, metadata);
    assert_eq!(session.record().messages, record().messages);
    assert!(matches!(
        inner.calls()[2],
        RecordedSessionStoreCall::Load { .. }
    ));
    drop(turn);
}

#[test]
fn cancellation_preserves_actual_completed_and_unknown_results_without_replay() {
    let provider = ScriptedModelProvider::new(
        "two-turns",
        [
            tool_round(vec![call("known", "action"), call("uncertain", "action")]),
            finish(),
        ],
    );
    let tool = ScriptedTool::new(
        spec("action"),
        [
            ToolStep::Output(ToolOutput::success(json!({"confirmed": true}))),
            ToolStep::Pending,
        ],
    );
    let permission = ScriptedPermissionHandler::new([allow_turn(), allow_turn()]);
    let store = InMemorySessionStore::new();
    let engine = engine(
        store,
        provider.clone(),
        permission.clone(),
        tool.clone(),
        EngineLimits::default(),
    );
    let session = engine
        .create_session(record().id, record().incarnation_id)
        .unwrap();
    let mut turn = block_on(session.prompt("original prompt")).unwrap();
    let waker = noop_waker();
    loop {
        match std::pin::Pin::new(&mut turn).poll_next(&mut Context::from_waker(&waker)) {
            Poll::Ready(Some(Ok(_))) => {}
            Poll::Pending => break,
            other @ Poll::Ready(_) => panic!("expected pending second tool, got {other:?}"),
        }
    }
    assert_eq!(tool.invocations().len(), 2);
    let handle = turn.handle();
    assert!(handle.cancel());
    let cancelled = block_on(turn.collect::<Vec<_>>());
    assert!(matches!(
        cancelled.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    let retained = session.record();
    assert!(
        matches!(&retained.messages[2].content[0], ContentBlock::ToolResult { output, .. } if !output.is_error)
    );
    assert!(
        matches!(&retained.messages[3].content[0], ContentBlock::ToolResult { output, .. } if output.content["code"] == "tool_result_unknown")
    );
    let continuation = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
    assert_eq!(continuation.id().as_str(), "turn-2");
    complete(continuation);
    assert_eq!(provider.requests()[1].request.messages, retained.messages);
    assert_eq!(tool.invocations().len(), 2);
    assert_eq!(permission.requests().len(), 2);
    assert!(tool.invocations()[1].cancellation.is_cancelled());
}

#[test]
fn new_provider_calls_need_fresh_authorization_under_the_continuation_turn() {
    let provider = ScriptedModelProvider::new(
        "two-turns",
        [
            tool_round(vec![call("first", "action")]),
            finish(),
            tool_round(vec![call("second", "action")]),
            finish(),
        ],
    );
    let tool = ScriptedTool::new(
        spec("action"),
        [ToolStep::Output(ToolOutput::success(
            json!({"confirmed": true}),
        ))],
    );
    let permission = ScriptedPermissionHandler::new([
        allow_turn(),
        PermissionStep::Decision(PermissionDecision::Deny {
            reason: "fresh decision".to_owned(),
        }),
    ]);
    let engine = engine(
        InMemorySessionStore::new(),
        provider,
        permission.clone(),
        tool.clone(),
        EngineLimits::default(),
    );
    let session = engine
        .create_session(record().id, record().incarnation_id)
        .unwrap();
    complete(block_on(session.prompt("original")).unwrap());
    complete(block_on(session.continue_turn(InferenceOptions::default())).unwrap());
    let requests = permission.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].turn_id.as_str(), "turn-1");
    assert_eq!(requests[1].turn_id.as_str(), "turn-2");
    assert_ne!(requests[0].id, requests[1].id);
    assert_eq!(tool.invocations().len(), 1);
}

struct RegistrationTool(Arc<TurnToolRegistration>);

impl Tool for RegistrationTool {
    fn spec(&self) -> ToolSpec {
        spec("register")
    }

    fn execute(
        &self,
        _context: ToolContext,
        _arguments: Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        panic!("engine uses the turn-aware entry point")
    }

    fn execute_for_turn(
        &self,
        _context: ToolContext,
        _arguments: Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        let registration = Arc::clone(&self.0);
        Box::pin(async move {
            Ok(ToolExecution::with_next_round_tool(
                ToolOutput::success(json!({"selected": "dynamic"})),
                registration,
            ))
        })
    }
}

#[test]
fn continuation_does_not_restore_ephemeral_tool_registrations() {
    let dynamic = ScriptedTool::new(spec("dynamic"), []);
    let provider = ScriptedModelProvider::new(
        "registration",
        [
            tool_round(vec![call("register-call", "register")]),
            finish(),
            finish(),
        ],
    );
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::new())
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([allow_turn()]))
        .tool(RegistrationTool(Arc::new(TurnToolRegistration::new(
            dynamic.clone(),
        ))))
        .build()
        .unwrap();
    let session = engine
        .create_session(record().id, record().incarnation_id)
        .unwrap();
    complete(block_on(session.prompt("select a tool")).unwrap());
    complete(block_on(session.continue_turn(InferenceOptions::default())).unwrap());
    let requests = provider.requests();
    assert!(
        requests[1]
            .request
            .tools
            .iter()
            .any(|tool| tool.name.as_str() == "dynamic")
    );
    assert!(
        !requests[2]
            .request
            .tools
            .iter()
            .any(|tool| tool.name.as_str() == "dynamic")
    );
    assert!(dynamic.invocations().is_empty());
}

#[test]
fn unpolled_and_rejected_continuation_options_have_iterative_cleanup() {
    let store = store(record(), SessionStoreScript::default());
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        ScriptedPermissionHandler::new([]),
        no_tools(),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let options = || {
        let mut value = Value::Null;
        for _ in 0..50_000 {
            value = Value::Array(vec![value]);
        }
        InferenceOptions {
            metadata: BTreeMap::from([("deep".to_owned(), value)]),
            ..InferenceOptions::default()
        }
    };
    drop(session.continue_turn(options()));
    assert!(matches!(
        block_on(session.continue_turn(options())),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(store.calls().len(), 1);
    assert!(!session.has_active_turn());
}

#[test]
fn continuation_future_does_not_retain_host_authority() {
    struct Resource(Arc<AtomicBool>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let dropped = Arc::new(AtomicBool::new(false));
    let store = store(record(), SessionStoreScript::default());
    let engine = Engine::builder()
        .session_store(store.clone())
        .provider(ScriptedModelProvider::new("unused", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .host_resource(Resource(Arc::clone(&dropped)))
        .build()
        .unwrap();
    let session = load(&engine);
    let operation = session.continue_turn(InferenceOptions::default());
    drop(session);
    drop(engine);
    assert!(dropped.load(Ordering::Acquire));
    assert!(matches!(block_on(operation), Err(EngineError::HostClosed)));
    assert_eq!(store.calls().len(), 1);
}
