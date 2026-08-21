use futures_util::{FutureExt, StreamExt};
use machine_god_core::{
    CancellationToken, Capability, ContentBlock, Engine, EngineError, EngineEvent, EventSink,
    InferenceOptions, Message, ModelEvent, ModelProvider, ModelRequest, PermissionDecision,
    PermissionError, PermissionGrantScope, PermissionHandler, PermissionRequest,
    PermissionRequestId, PermissionRisk, ProviderError, ProviderErrorKind, Role, Session,
    SessionId, SessionIncarnationId, SessionRecord, SessionRevision, SessionStore,
    SessionStoreError, SessionStoreErrorKind, StopReason, TokenUsage, Tool, ToolCall, ToolCallId,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec, TurnEvent, TurnId,
};
use machine_god_testkit::{
    EventSinkStep, InMemorySessionStore, ModelProviderStep, PermissionStep,
    RecordedSessionStoreCall, RecordingEventSink, ScriptedModelProvider, ScriptedPermissionHandler,
    ScriptedPreparedTool, ScriptedTool, SessionStoreScript, SessionStoreStep, ToolPrepareStep,
    ToolStep,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

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

fn deny_permissions() -> ScriptedPermissionHandler {
    ScriptedPermissionHandler::new([])
}

fn model_request(name: &str) -> ModelRequest {
    ModelRequest {
        session_id: SessionId::new(name).unwrap(),
        session_incarnation_id: SessionIncarnationId::new(format!("request-incarnation-{name}"))
            .unwrap(),
        turn_id: TurnId::new("turn-1").unwrap(),
        messages: Vec::new(),
        tools: Vec::new(),
        options: InferenceOptions::default(),
    }
}

fn event(sequence: u64) -> EngineEvent {
    EngineEvent {
        session_id: SessionId::new("event-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("event-incarnation").unwrap(),
        turn_id: TurnId::new("turn-1").unwrap(),
        sequence,
        payload: TurnEvent::Started,
    }
}

fn permission_request() -> PermissionRequest {
    PermissionRequest {
        id: PermissionRequestId::new("permission-1").unwrap(),
        session_id: SessionId::new("permission-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("permission-incarnation").unwrap(),
        turn_id: TurnId::new("turn-1").unwrap(),
        capability: Capability::Custom {
            name: "fixture".to_owned(),
            details: json!({"safe": true}),
        },
        risk: PermissionRisk::Low,
        reason: "exercise policy boundary".to_owned(),
    }
}

fn tool_spec() -> ToolSpec {
    ToolSpec {
        name: ToolName::new("fixture_tool").unwrap(),
        description: "A deterministic fixture".to_owned(),
        input_schema: json!({"type": "object"}),
    }
}

fn tool_context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("tool-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("tool-incarnation").unwrap(),
        turn_id: TurnId::new("turn-1").unwrap(),
        call_id: ToolCallId::new("call-1").unwrap(),
    }
}

fn prepared_call(index: usize) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(format!("prepared-call-{index}")).unwrap(),
        name: tool_spec().name,
        arguments: json!({"raw": index}),
    }
}

fn prepared_context(index: usize) -> ToolContext {
    ToolContext {
        call_id: ToolCallId::new(format!("prepared-execution-{index}")).unwrap(),
        ..tool_context()
    }
}

fn prepared_step(index: usize) -> ToolPrepareStep {
    ToolPrepareStep::Prepared {
        capability: Capability::Custom {
            name: "prepared-fixture".to_owned(),
            details: json!({"index": index}),
        },
        arguments: json!({"normalized": index}),
    }
}

#[test]
fn complete_engine_turn_is_deterministic_and_fully_inspectable() {
    let usage = TokenUsage {
        input_tokens: 7,
        output_tokens: 2,
        cached_input_tokens: 3,
    };
    let provider = ScriptedModelProvider::new(
        "fixture",
        [ModelProviderStep::events([
            ModelEvent::TextDelta {
                text: "hello".to_owned(),
            },
            ModelEvent::Usage { usage },
            ModelEvent::Stop {
                reason: StopReason::Completed,
            },
        ])],
    );
    let store = InMemorySessionStore::new();
    let sink = RecordingEventSink::new();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(deny_permissions())
        .event_sink(sink.clone())
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("engine-flow").unwrap());

    let turn = futures_executor::block_on(session.prompt("hello model")).unwrap();
    let events = futures_executor::block_on(turn.collect::<Vec<_>>());
    let events: Vec<_> = events.into_iter().collect::<Result<_, _>>().unwrap();

    assert_eq!(events.len(), 5);
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4]
    );
    assert!(matches!(events[0].payload, TurnEvent::Started));
    assert!(matches!(
        events[4].payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            usage: observed,
        } if observed == usage
    ));
    assert_eq!(sink.events(), events);

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].request.session_id, session.id());
    assert_eq!(requests[0].request.turn_id.as_str(), "turn-1");
    assert!(matches!(
        requests[0].request.messages.as_slice(),
        [Message {
            role: Role::User,
            content,
        }] if matches!(content.as_slice(), [ContentBlock::Text { text }] if text == "hello model")
    ));
    assert!(!requests[0].cancellation.is_cancelled());
    assert_eq!(provider.remaining_steps(), 0);

    let stored = store.record(&session.id()).unwrap();
    assert_eq!(stored.revision, SessionRevision(2));
    assert_eq!(stored.next_turn_sequence, 2);
    assert_eq!(stored.messages.len(), 2);
    assert_eq!(stored.messages[1].role, Role::Assistant);
    assert_eq!(store.calls().len(), 2);
    assert!(matches!(
        &store.calls()[0],
        RecordedSessionStoreCall::Save {
            expected_revision: None,
            ..
        }
    ));
    assert!(matches!(
        &store.calls()[1],
        RecordedSessionStoreCall::Save {
            expected_revision: Some(SessionRevision(1)),
            ..
        }
    ));
}

#[test]
fn provider_start_errors_become_structured_terminal_turn_events() {
    let provider = ScriptedModelProvider::new(
        "failure",
        [ModelProviderStep::StartError(ProviderError::new(
            ProviderErrorKind::Unavailable,
            "offline",
            "fixture provider is unavailable",
            true,
        ))],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(deny_permissions())
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("start-error").unwrap());
    let turn = futures_executor::block_on(session.prompt("fail")).unwrap();
    let events = futures_executor::block_on(turn.collect::<Vec<_>>());
    let events: Vec<_> = events.into_iter().collect::<Result<_, _>>().unwrap();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[1].payload,
        TurnEvent::Failed {
            component,
            code,
            retryable: true,
            ..
        } if component == "provider" && code == "provider_failed"
    ));
}

#[test]
fn pending_provider_start_is_cancelled_without_a_clock() {
    let provider = ScriptedModelProvider::new("pending", [ModelProviderStep::PendingStart]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(InMemorySessionStore::new())
        .permission_handler(deny_permissions())
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("pending-start").unwrap());
    let mut turn = futures_executor::block_on(session.prompt("wait")).unwrap();
    assert!(matches!(
        futures_executor::block_on(turn.next())
            .unwrap()
            .unwrap()
            .payload,
        TurnEvent::Started
    ));
    assert!(turn.next().now_or_never().is_none());
    assert_eq!(provider.requests().len(), 1);
    let handle = turn.handle();
    assert!(handle.cancel());
    let terminal = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(
        terminal.payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert!(futures_executor::block_on(turn.next()).is_none());
    assert!(provider.requests()[0].cancellation.is_cancelled());
}

#[test]
fn pending_sink_exposes_backpressure_and_engine_cancellation() {
    let provider = ScriptedModelProvider::new(
        "pending-sink-provider",
        [ModelProviderStep::events_then_pending([
            ModelEvent::TextDelta {
                text: "blocked".to_owned(),
            },
        ])],
    );
    let sink = RecordingEventSink::scripted([EventSinkStep::Accept, EventSinkStep::Pending]);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(deny_permissions())
        .event_sink(sink.clone())
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("sink-backpressure").unwrap());
    let mut turn = futures_executor::block_on(session.prompt("block")).unwrap();
    assert!(futures_executor::block_on(turn.next()).unwrap().is_ok());
    assert!(turn.next().now_or_never().is_none());
    assert_eq!(sink.events().len(), 2);

    assert!(turn.handle().cancel());
    let terminal = futures_executor::block_on(turn.next()).unwrap().unwrap();
    assert!(matches!(
        terminal.payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert!(!session.has_active_turn());
}

#[test]
fn store_scripts_fail_before_mutation_and_report_exhaustion() {
    let id = SessionId::new("scripted-store").unwrap();
    let error = SessionStoreError::new(
        SessionStoreErrorKind::Unavailable,
        "disk_offline",
        "fixture unavailable",
        true,
    );
    let store = InMemorySessionStore::configured(
        BTreeMap::new(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![SessionStoreStep::Error(error.clone())]),
        },
        4,
    );
    let record = SessionRecord::empty(id.clone(), test_incarnation(&id));
    assert_eq!(
        futures_executor::block_on(store.save(record.clone(), None)).unwrap_err(),
        error
    );
    assert!(store.record(&id).is_none());

    let exhausted = futures_executor::block_on(store.save(record, None)).unwrap_err();
    assert_eq!(exhausted.code, "testkit_script_exhausted");
    assert_eq!(store.calls().len(), 2);
    assert_eq!(store.remaining_steps(), (None, Some(0)));
}

#[test]
fn every_double_reports_its_recording_bound() {
    let provider =
        ScriptedModelProvider::with_record_capacity("bounded", [ModelProviderStep::events([])], 0);
    let result = futures_executor::block_on(
        provider.stream(model_request("bounded"), CancellationToken::new()),
    );
    let Err(error) = result else {
        panic!("zero provider recording capacity must fail");
    };
    assert_eq!(error.code, "testkit_record_capacity_exhausted");

    let sink = RecordingEventSink::with_record_capacity(0);
    let error = futures_executor::block_on(sink.emit(event(0))).unwrap_err();
    assert_eq!(error.code, "testkit_record_capacity_exhausted");

    let permission = ScriptedPermissionHandler::with_record_capacity([], 0);
    let error = futures_executor::block_on(permission.authorize(permission_request())).unwrap_err();
    assert_eq!(error.code, "testkit_record_capacity_exhausted");

    let tool = ScriptedTool::with_record_capacity(tool_spec(), [], 0);
    let error = futures_executor::block_on(tool.execute(
        tool_context(),
        json!({}),
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(error.code, "testkit_record_capacity_exhausted");

    let prepared_tool = ScriptedPreparedTool::with_record_capacity(
        tool_spec(),
        [ToolPrepareStep::Prepared {
            capability: Capability::Custom {
                name: "bounded".to_owned(),
                details: json!({}),
            },
            arguments: json!({}),
        }],
        [ToolStep::Output(ToolOutput::success("unused"))],
        0,
    );
    let error = prepared_tool
        .prepare(ToolCall {
            id: ToolCallId::new("bounded-prepare").unwrap(),
            name: tool_spec().name,
            arguments: json!({}),
        })
        .unwrap_err();
    assert_eq!(error.code, "testkit_record_capacity_exhausted");
    let error = futures_executor::block_on(prepared_tool.execute(
        tool_context(),
        json!({}),
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(error.code, "testkit_record_capacity_exhausted");

    let store = InMemorySessionStore::configured(BTreeMap::new(), SessionStoreScript::default(), 0);
    let error =
        futures_executor::block_on(store.load(SessionId::new("bounded").unwrap())).unwrap_err();
    assert_eq!(error.code, "testkit_record_capacity_exhausted");
}

#[test]
fn permissions_record_decisions_errors_and_exhaustion_in_order() {
    let expected_error = PermissionError::new("policy_failed", "fixture policy failed");
    let handler = ScriptedPermissionHandler::new([
        PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Turn,
        }),
        PermissionStep::Error(expected_error.clone()),
    ]);
    let first = permission_request();
    let mut second = permission_request();
    second.id = PermissionRequestId::new("permission-2").unwrap();
    let mut third = permission_request();
    third.id = PermissionRequestId::new("permission-3").unwrap();

    assert!(matches!(
        futures_executor::block_on(handler.authorize(first)).unwrap(),
        PermissionDecision::Allow {
            scope: PermissionGrantScope::Turn
        }
    ));
    assert_eq!(
        futures_executor::block_on(handler.authorize(second)).unwrap_err(),
        expected_error
    );
    let exhausted = futures_executor::block_on(handler.authorize(third)).unwrap_err();
    assert_eq!(exhausted.code, "testkit_script_exhausted");
    assert_eq!(handler.requests().len(), 3);
}

#[test]
fn tool_records_results_errors_and_cancelled_pending_work() {
    let expected_error = ToolError::new(
        ToolErrorKind::Execution,
        "fixture_failure",
        "fixture failed",
        false,
    );
    let tool = ScriptedTool::new(
        tool_spec(),
        [
            ToolStep::Output(ToolOutput::success(json!({"answer": 42}))),
            ToolStep::Error(expected_error.clone()),
            ToolStep::Pending,
        ],
    );
    assert_eq!(tool.spec(), tool_spec());
    assert_eq!(
        futures_executor::block_on(tool.execute(
            tool_context(),
            json!({"call": 1}),
            CancellationToken::new(),
        ))
        .unwrap()
        .content,
        json!({"answer": 42})
    );
    assert_eq!(
        futures_executor::block_on(tool.execute(
            tool_context(),
            json!({"call": 2}),
            CancellationToken::new(),
        ))
        .unwrap_err(),
        expected_error
    );
    let cancellation = CancellationToken::new();
    let pending = tool.execute(tool_context(), json!({"call": 3}), cancellation.clone());
    assert!(cancellation.cancel());
    let cancelled = futures_executor::block_on(pending).unwrap_err();
    assert_eq!(cancelled.kind, ToolErrorKind::Cancelled);
    let invocations = tool.invocations();
    assert_eq!(invocations.len(), 3);
    assert_eq!(invocations[2].arguments, json!({"call": 3}));
    assert!(invocations[2].cancellation.is_cancelled());
}

#[test]
fn prepared_tool_records_preparation_and_execution_independently() {
    let requested = ToolCall {
        id: ToolCallId::new("prepared-call").unwrap(),
        name: tool_spec().name,
        arguments: json!({"raw": "input"}),
    };
    let prepared_arguments = json!({"normalized": true});
    let tool = ScriptedPreparedTool::new(
        tool_spec(),
        [ToolPrepareStep::Prepared {
            capability: Capability::Custom {
                name: "prepared-fixture".to_owned(),
                details: json!({"safe": true}),
            },
            arguments: prepared_arguments.clone(),
        }],
        [ToolStep::Output(ToolOutput::success("done"))],
    );

    assert!(tool.prepare(requested.clone()).is_ok());
    assert_eq!(tool.preparations().len(), 1);
    assert_eq!(tool.preparations()[0].call, requested);
    assert!(tool.invocations().is_empty());

    futures_executor::block_on(tool.execute(
        tool_context(),
        prepared_arguments.clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(tool.invocations().len(), 1);
    assert_eq!(tool.invocations()[0].arguments, prepared_arguments);
    assert_eq!(tool.remaining_steps(), (0, 0));
}

#[test]
fn prepared_tool_errors_before_execution_and_preserves_execution_script() {
    let expected = ToolError::new(
        ToolErrorKind::InvalidInput,
        "invalid_path",
        "fixture rejected its input",
        false,
    );
    let tool = ScriptedPreparedTool::new(
        tool_spec(),
        [ToolPrepareStep::Error(expected.clone())],
        [ToolStep::Output(ToolOutput::success("must remain unused"))],
    );
    let requested = ToolCall {
        id: ToolCallId::new("prepare-error").unwrap(),
        name: tool_spec().name,
        arguments: json!({}),
    };

    assert_eq!(tool.prepare(requested).unwrap_err(), expected);
    assert_eq!(tool.preparations().len(), 1);
    assert!(tool.invocations().is_empty());
    assert_eq!(tool.remaining_steps(), (0, 1));
}

#[test]
fn prepared_tool_script_exhaustion_records_only_the_attempted_phase() {
    let tool = ScriptedPreparedTool::new(
        tool_spec(),
        [prepared_step(0)],
        [ToolStep::Output(ToolOutput::success("executed"))],
    );

    assert!(tool.prepare(prepared_call(0)).is_ok());
    let prepare_error = tool.prepare(prepared_call(1)).unwrap_err();
    assert_eq!(prepare_error.kind, ToolErrorKind::Other);
    assert_eq!(prepare_error.code, "testkit_script_exhausted");
    assert_eq!(
        prepare_error.message,
        "scripted tool was prepared after its preparation script was exhausted"
    );
    assert!(!prepare_error.retryable);
    assert_eq!(tool.preparations().len(), 2);
    assert!(tool.invocations().is_empty());
    assert_eq!(tool.remaining_steps(), (0, 1));

    futures_executor::block_on(tool.execute(
        prepared_context(0),
        json!({"execution": 0}),
        CancellationToken::new(),
    ))
    .unwrap();
    let execute_error = futures_executor::block_on(tool.execute(
        prepared_context(1),
        json!({"execution": 1}),
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(execute_error.kind, ToolErrorKind::Other);
    assert_eq!(execute_error.code, "testkit_script_exhausted");
    assert_eq!(
        execute_error.message,
        "scripted tool was invoked after its execution script was exhausted"
    );
    assert!(!execute_error.retryable);
    assert_eq!(tool.preparations().len(), 2);
    assert_eq!(tool.invocations().len(), 2);
    assert_eq!(tool.remaining_steps(), (0, 0));
}

#[test]
fn prepared_tool_record_capacity_is_independent_at_n_and_n_plus_one() {
    const CAPACITY: usize = 3;
    let tool = ScriptedPreparedTool::with_record_capacity(
        tool_spec(),
        (0..=CAPACITY).map(prepared_step),
        (0..=CAPACITY)
            .map(|index| ToolStep::Output(ToolOutput::success(json!({"execution": index})))),
        CAPACITY,
    );

    for index in 0..CAPACITY {
        assert!(tool.prepare(prepared_call(index)).is_ok());
    }
    let prepare_error = tool.prepare(prepared_call(CAPACITY)).unwrap_err();
    assert_eq!(prepare_error.code, "testkit_record_capacity_exhausted");
    assert_eq!(tool.preparations().len(), CAPACITY);
    assert!(tool.invocations().is_empty());
    assert_eq!(tool.remaining_steps(), (1, CAPACITY + 1));

    for index in 0..CAPACITY {
        futures_executor::block_on(tool.execute(
            prepared_context(index),
            json!({"execution": index}),
            CancellationToken::new(),
        ))
        .unwrap();
    }
    let execute_error = futures_executor::block_on(tool.execute(
        prepared_context(CAPACITY),
        json!({"execution": CAPACITY}),
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(execute_error.code, "testkit_record_capacity_exhausted");
    assert_eq!(tool.preparations().len(), CAPACITY);
    assert_eq!(tool.invocations().len(), CAPACITY);
    assert_eq!(tool.remaining_steps(), (1, 1));
}

#[test]
fn prepared_tool_concurrent_calls_and_snapshots_remain_bounded_and_consistent() {
    const CALLS: usize = 24;
    let expected_spec = tool_spec();
    let tool = ScriptedPreparedTool::with_record_capacity(
        expected_spec.clone(),
        (0..CALLS).map(prepared_step),
        (0..CALLS).map(|_| ToolStep::Output(ToolOutput::success("executed"))),
        CALLS,
    );
    let barrier = Arc::new(Barrier::new(CALLS * 2 + 2));
    let done = Arc::new(AtomicBool::new(false));
    let snapshot_tool = tool.clone();
    let snapshot_barrier = Arc::clone(&barrier);
    let snapshot_done = Arc::clone(&done);
    let snapshot = std::thread::spawn(move || {
        snapshot_barrier.wait();
        let mut samples = 0usize;
        loop {
            let preparations = snapshot_tool.preparations();
            let preparation_ids: BTreeSet<_> = preparations
                .iter()
                .map(|recorded| recorded.call.id.to_string())
                .collect();
            assert_eq!(preparation_ids.len(), preparations.len());
            assert!(preparations.len() <= CALLS);

            let invocations = snapshot_tool.invocations();
            let execution_ids: BTreeSet<_> = invocations
                .iter()
                .map(|recorded| recorded.context.call_id.to_string())
                .collect();
            assert_eq!(execution_ids.len(), invocations.len());
            assert!(invocations.len() <= CALLS);
            assert!(
                invocations
                    .iter()
                    .all(|recorded| !recorded.cancellation.is_cancelled())
            );

            let remaining = snapshot_tool.remaining_steps();
            assert!(remaining.0 <= CALLS);
            assert!(remaining.1 <= CALLS);
            assert_eq!(snapshot_tool.spec(), expected_spec);
            samples += 1;
            if snapshot_done.load(Ordering::SeqCst) {
                break;
            }
        }
        samples
    });

    let mut workers = Vec::new();
    for index in 0..CALLS {
        let preparation_tool = tool.clone();
        let preparation_barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            preparation_barrier.wait();
            preparation_tool.prepare(prepared_call(index)).map(drop)
        }));

        let execution_tool = tool.clone();
        let execution_barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            execution_barrier.wait();
            futures_executor::block_on(execution_tool.execute(
                prepared_context(index),
                json!({"execution": index}),
                CancellationToken::new(),
            ))
            .map(drop)
        }));
    }
    barrier.wait();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    done.store(true, Ordering::SeqCst);
    assert!(snapshot.join().unwrap() > 0);

    let preparations = tool.preparations();
    let preparation_ids: BTreeSet<_> = preparations
        .iter()
        .map(|recorded| recorded.call.id.to_string())
        .collect();
    let invocations = tool.invocations();
    let execution_ids: BTreeSet<_> = invocations
        .iter()
        .map(|recorded| recorded.context.call_id.to_string())
        .collect();
    assert_eq!(preparations.len(), CALLS);
    assert_eq!(preparation_ids.len(), CALLS);
    assert_eq!(invocations.len(), CALLS);
    assert_eq!(execution_ids.len(), CALLS);
    assert_eq!(tool.remaining_steps(), (0, 0));
}

#[test]
fn provider_inspection_is_safe_during_concurrent_calls() {
    const CALLS: usize = 32;
    let provider = ScriptedModelProvider::new(
        "concurrent",
        (0..CALLS).map(|_| ModelProviderStep::events([])),
    );
    let mut workers = Vec::new();
    for index in 0..CALLS {
        let provider = provider.clone();
        workers.push(std::thread::spawn(move || {
            let name = format!("request-{index}");
            futures_executor::block_on(
                provider.stream(model_request(&name), CancellationToken::new()),
            )
            .map(drop)
        }));
    }
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    let requests = provider.requests();
    let ids: BTreeSet<_> = requests
        .iter()
        .map(|recorded| recorded.request.session_id.to_string())
        .collect();
    assert_eq!(requests.len(), CALLS);
    assert_eq!(ids.len(), CALLS);
    assert_eq!(provider.remaining_steps(), 0);
}

#[test]
fn provider_result_stream_can_inject_an_item_error() {
    let expected = ProviderError::new(
        ProviderErrorKind::Transport,
        "connection_reset",
        "fixture connection reset",
        true,
    );
    let provider = ScriptedModelProvider::new(
        "stream-error",
        [ModelProviderStep::results([Err(expected.clone())])],
    );
    let mut stream = futures_executor::block_on(
        provider.stream(model_request("stream-error"), CancellationToken::new()),
    )
    .unwrap();
    assert_eq!(
        futures_executor::block_on(stream.next())
            .unwrap()
            .unwrap_err(),
        expected
    );
}

#[test]
fn provider_stream_preserves_usage_events() {
    let usage = TokenUsage {
        input_tokens: 7,
        output_tokens: 2,
        cached_input_tokens: 3,
    };
    let provider = ScriptedModelProvider::new(
        "usage",
        [ModelProviderStep::events([ModelEvent::Usage { usage }])],
    );
    let mut stream = futures_executor::block_on(
        provider.stream(model_request("usage"), CancellationToken::new()),
    )
    .unwrap();
    assert!(matches!(
        futures_executor::block_on(stream.next()),
        Some(Ok(ModelEvent::Usage { usage: observed })) if observed == usage
    ));
}

#[test]
fn event_sink_script_records_errors_and_exhausted_calls() {
    let expected = machine_god_core::EventSinkError::new("observer_failed", "fixture observer");
    let sink = RecordingEventSink::scripted([
        EventSinkStep::Accept,
        EventSinkStep::Error(expected.clone()),
    ]);
    futures_executor::block_on(sink.emit(event(0))).unwrap();
    assert_eq!(
        futures_executor::block_on(sink.emit(event(1))).unwrap_err(),
        expected
    );
    let exhausted = futures_executor::block_on(sink.emit(event(2))).unwrap_err();
    assert_eq!(exhausted.code, "testkit_script_exhausted");
    assert_eq!(sink.events().len(), 3);
}

#[test]
fn store_load_and_save_inspection_preserves_exact_inputs() {
    let id = SessionId::new("load-save-inspection").unwrap();
    let mut record = SessionRecord::empty(id.clone(), test_incarnation(&id));
    record.revision = SessionRevision(8);
    record.next_turn_sequence = 5;
    let store = InMemorySessionStore::from_records(BTreeMap::from([(id.clone(), record.clone())]));

    assert_eq!(
        futures_executor::block_on(store.load(id.clone())).unwrap(),
        Some(record.clone())
    );
    record.next_turn_sequence = 6;
    assert_eq!(
        futures_executor::block_on(store.save(record.clone(), Some(SessionRevision(8)))).unwrap(),
        SessionRevision(9)
    );
    assert_eq!(
        store.calls(),
        [
            RecordedSessionStoreCall::Load { id },
            RecordedSessionStoreCall::Save {
                record,
                expected_revision: Some(SessionRevision(8)),
            },
        ]
    );
}

#[test]
fn strict_empty_provider_script_surfaces_through_the_engine() {
    let provider = ScriptedModelProvider::new("exhausted", []);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(deny_permissions())
        .build()
        .unwrap();
    let session = engine.create_test_session(SessionId::new("engine-exhaustion").unwrap());
    let turn = futures_executor::block_on(session.prompt("one too many")).unwrap();
    let events = futures_executor::block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok));
    assert!(matches!(
        &events[1].as_ref().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "provider_failed"
    ));
}

#[test]
fn store_conflicts_are_typed_and_do_not_mutate_state() {
    let id = SessionId::new("store-conflict").unwrap();
    let mut initial = SessionRecord::empty(id.clone(), test_incarnation(&id));
    initial.revision = SessionRevision(3);
    let store = InMemorySessionStore::from_records(BTreeMap::from([(id.clone(), initial.clone())]));
    let mut candidate = initial.clone();
    candidate.next_turn_sequence = 9;
    let error =
        futures_executor::block_on(store.save(candidate, Some(SessionRevision(2)))).unwrap_err();
    assert_eq!(error.kind, SessionStoreErrorKind::Conflict);
    assert_eq!(store.record(&id), Some(initial));
}

#[test]
fn pending_permission_future_is_explicitly_detectable() {
    let handler = ScriptedPermissionHandler::new([PermissionStep::Pending]);
    let pending = handler.authorize(permission_request());
    assert!(pending.now_or_never().is_none());
    assert_eq!(handler.requests().len(), 1);
}

#[test]
fn errors_remain_component_typed() {
    let provider_error = ProviderError::new(
        ProviderErrorKind::InvalidRequest,
        "bad_request",
        "fixture rejected request",
        false,
    );
    let provider = ScriptedModelProvider::new(
        "typed",
        [ModelProviderStep::StartError(provider_error.clone())],
    );
    let result = futures_executor::block_on(
        provider.stream(model_request("typed"), CancellationToken::new()),
    );
    let Err(error) = result else {
        panic!("scripted provider start error must fail");
    };
    assert_eq!(error, provider_error);

    let engine_error: EngineError =
        SessionStoreError::new(SessionStoreErrorKind::Other, "fixture", "fixture", false).into();
    assert!(matches!(engine_error, EngineError::Store(_)));
}
