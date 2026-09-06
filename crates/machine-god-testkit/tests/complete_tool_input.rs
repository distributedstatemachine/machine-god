//! Complete inputs remain immutable while bounded historical arguments are saved.
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, Engine, EngineEvent, EngineLimits,
    PermissionDecision, PermissionGrantScope, PreparedToolCall, Session, SessionId,
    SessionIncarnationId, SessionStoreError, SessionStoreErrorKind, StopReason, Tool, ToolCall,
    ToolCallId, ToolContext, ToolError, ToolErrorKind, ToolInputLimits, ToolName, ToolOutput,
    ToolSpec, TurnEvent,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler, SessionStoreScript, SessionStoreStep,
};
use serde_json::{Value, json};
use std::num::NonZeroUsize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn nz(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap()
}
fn policy(bytes: usize, nodes: usize) -> ToolInputLimits {
    ToolInputLimits {
        max_argument_bytes: nz(bytes),
        max_argument_nodes: nz(nodes),
        max_prepared_argument_bytes: nz(bytes + 2048),
        max_prepared_argument_nodes: nz(nodes),
    }
}
fn reference() -> Value {
    json!({"type":"tool_arguments_archive","handle":"fixture"})
}
type Publication =
    dyn Fn(&Value, &CancellationToken) -> Result<Option<Value>, ToolError> + Send + Sync;

#[derive(Clone)]
struct InputTool {
    policy: Option<ToolInputLimits>,
    publication: Arc<Publication>,
    publications: Arc<AtomicUsize>,
    prepared: Arc<Mutex<Vec<Value>>>,
    executed: Arc<Mutex<Vec<Value>>>,
    execution_records: Arc<Mutex<Vec<machine_god_core::SessionRecord>>>,
    replacement: Option<Value>,
    permission: bool,
    pending: bool,
    store: InMemorySessionStore,
}
impl InputTool {
    fn new() -> Self {
        Self {
            policy: Some(policy(256 * 1024, 100_000)),
            publication: Arc::new(|_, _| Ok(Some(reference()))),
            publications: Arc::new(AtomicUsize::new(0)),
            prepared: Arc::default(),
            executed: Arc::default(),
            execution_records: Arc::default(),
            replacement: None,
            permission: false,
            pending: false,
            store: InMemorySessionStore::new(),
        }
    }
}
impl Tool for InputTool {
    fn complete_input_limits(&self) -> Option<ToolInputLimits> {
        self.policy
    }
    fn persist_arguments<'a>(
        &'a self,
        context: ToolContext,
        arguments: &'a Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        Box::pin(async move {
            assert_eq!(context.session_id.as_str(), "input");
            assert_eq!(context.session_incarnation_id.as_str(), "incarnation");
            assert!(!context.call_id.as_str().is_empty());
            self.publications.fetch_add(1, Ordering::SeqCst);
            if self.pending {
                std::future::pending::<()>().await;
            }
            (self.publication)(arguments, &cancellation)
        })
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("complete").unwrap(),
            description: "fixture".into(),
            input_schema: json!({}),
        }
    }
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        self.prepared.lock().unwrap().push(call.arguments.clone());
        let arguments = self.replacement.clone().unwrap_or(call.arguments);
        Ok(if self.permission {
            PreparedToolCall::new(
                Capability::Tool {
                    name: call.name,
                    call_id: call.id,
                    arguments: arguments.clone(),
                },
                arguments,
            )
        } else {
            PreparedToolCall::without_authority(arguments)
        })
    }
    fn execute(
        &self,
        _: ToolContext,
        arguments: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            let record = self
                .store
                .record(&SessionId::new("input").unwrap())
                .unwrap();
            assert!(record.messages.iter().any(|message| message.content.iter().any(|block|
                matches!(block, ContentBlock::ToolResult {output,..} if output.content["code"]=="tool_result_unknown"))));
            self.execution_records.lock().unwrap().push(record);
            self.executed.lock().unwrap().push(arguments);
            Ok(ToolOutput::success("done"))
        })
    }
}

struct Fixture {
    session: Session,
    provider: ScriptedModelProvider,
    tool: InputTool,
    permissions: ScriptedPermissionHandler,
}
impl Fixture {
    fn new(tool: InputTool, limits: EngineLimits, arguments: Vec<Value>) -> Self {
        Self::turns(tool, limits, arguments, 1)
    }
    fn turns(tool: InputTool, limits: EngineLimits, arguments: Vec<Value>, turns: usize) -> Self {
        let steps = (0..turns)
            .flat_map(|turn| {
                let mut events = arguments
                    .iter()
                    .enumerate()
                    .map(
                        |(index, arguments)| machine_god_core::ModelEvent::ToolCall {
                            call: ToolCall {
                                id: ToolCallId::new(format!("call-{turn}-{index}")).unwrap(),
                                name: ToolName::new("complete").unwrap(),
                                arguments: arguments.clone(),
                            },
                        },
                    )
                    .collect::<Vec<_>>();
                events.push(machine_god_core::ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                });
                [
                    ModelProviderStep::events(events),
                    ModelProviderStep::events([machine_god_core::ModelEvent::Stop {
                        reason: StopReason::Completed,
                    }]),
                ]
            })
            .collect::<Vec<_>>();
        drop(arguments);
        let provider = ScriptedModelProvider::new("input", steps);
        let permissions = ScriptedPermissionHandler::new((0..32).map(|_| {
            PermissionStep::Decision(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })
        }));
        let engine = Engine::builder()
            .provider(provider.clone())
            .session_store(tool.store.clone())
            .permission_handler(permissions.clone())
            .tool(tool.clone())
            .limits(limits)
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("input").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            )
            .unwrap();
        Self {
            session,
            provider,
            tool,
            permissions,
        }
    }
    fn run(&self) -> Vec<EngineEvent> {
        futures_executor::block_on(async {
            self.session
                .prompt("go")
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
    }
    fn assert_failure(&self, code: &str) {
        let events = self.run();
        assert!(
            matches!(&events.last().unwrap().payload, TurnEvent::Failed {code:actual,..} if actual==code),
            "{events:?}"
        );
        assert!(self.tool.executed.lock().unwrap().is_empty());
        assert!(self.permissions.requests().is_empty());
    }
}
fn completed(events: &[EngineEvent]) {
    assert!(
        matches!(
            events.last().unwrap().payload,
            TurnEvent::Completed {
                reason: StopReason::Completed,
                ..
            }
        ),
        "{events:?}"
    );
}

#[test]
fn complete_input_reaches_prepare_permission_execution_events_but_history_is_bounded() {
    let full = json!("x".repeat(70_000));
    let mut tool = InputTool::new();
    tool.permission = true;
    let expected = full.clone();
    tool.publication = Arc::new(move |arguments, _| {
        assert_eq!(arguments, &expected);
        Ok(Some(reference()))
    });
    let fixture = Fixture::new(tool, EngineLimits::default(), vec![full.clone()]);
    let events = fixture.run();
    completed(&events);
    assert_eq!(*fixture.tool.prepared.lock().unwrap(), vec![full.clone()]);
    assert_eq!(*fixture.tool.executed.lock().unwrap(), vec![full.clone()]);
    assert!(
        matches!(&fixture.permissions.requests()[0].capability,Capability::Tool {arguments,..} if arguments==&full)
    );
    assert!(events.iter().any(
        |event| matches!(&event.payload,TurnEvent::ToolStarted {call} if call.arguments==full)
    ));
    assert!(events.iter().any(|event| matches!(&event.payload,TurnEvent::Model {event:machine_god_core::ModelEvent::ToolCall {call}} if call.arguments==full)));
    let requests = fixture.provider.requests();
    let ContentBlock::ToolCall { call } = &requests[1].request.messages[1].content[0] else {
        panic!("call")
    };
    assert_eq!(call.arguments, reference());
    assert_eq!(call.id.as_str(), "call-0-0");
    assert!(
        serde_json::to_vec(&requests[1].request.messages)
            .unwrap()
            .len()
            < 1024
    );
    let record = fixture
        .tool
        .store
        .record(&SessionId::new("input").unwrap())
        .unwrap();
    assert_eq!(
        &record.messages[..record.messages.len() - 1],
        requests[1].request.messages
    );
}

#[test]
fn all_round_projections_and_placeholders_are_saved_before_first_effect() {
    let fixture = Fixture::new(
        InputTool::new(),
        EngineLimits::default(),
        vec![json!("x".repeat(70_000)); 2],
    );
    completed(&fixture.run());
    let records = fixture.tool.execution_records.lock().unwrap();
    let blocks = records[0]
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .collect::<Vec<_>>();
    assert_eq!(
        blocks
            .iter()
            .filter(
                |block| matches!(block,ContentBlock::ToolCall {call} if call.arguments==reference())
            )
            .count(),
        2
    );
    assert_eq!(blocks.iter().filter(|block| matches!(block,ContentBlock::ToolResult {output,..} if output.content["code"]=="tool_result_unknown")).count(),2);
}

#[test]
fn restarted_engine_uses_historical_projection_without_hydrating_or_executing_it() {
    let fixture = Fixture::new(
        InputTool::new(),
        EngineLimits::default(),
        vec![json!("x".repeat(70_000))],
    );
    completed(&fixture.run());
    let provider = ScriptedModelProvider::new(
        "restart",
        [ModelProviderStep::events([
            machine_god_core::ModelEvent::Stop {
                reason: StopReason::Completed,
            },
        ])],
    );
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(fixture.tool.store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = futures_executor::block_on(engine.load_session(SessionId::new("input").unwrap()))
        .unwrap()
        .unwrap();
    let events = futures_executor::block_on(async {
        session
            .prompt("after restart")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    });
    completed(&events);
    let requests = provider.requests();
    let ContentBlock::ToolCall { call } = &requests[0].request.messages[1].content[0] else {
        panic!("call")
    };
    assert_eq!(call.arguments, reference());
    assert_eq!(fixture.tool.executed.lock().unwrap().len(), 1);
    assert!(
        serde_json::to_vec(&requests[0].request.messages)
            .unwrap()
            .len()
            < 1024
    );
}

#[test]
fn publication_pre_poll_pending_cancellation_and_drop_have_no_action_effects() {
    for stage in ["pre_poll", "pending", "drop"] {
        let mut tool = InputTool::new();
        tool.pending = true;
        let fixture = Fixture::new(tool, EngineLimits::default(), vec![json!({})]);
        let mut turn = futures_executor::block_on(fixture.session.prompt("go")).unwrap();
        loop {
            let event = futures_executor::block_on(turn.next()).unwrap().unwrap();
            if matches!(
                event.payload,
                TurnEvent::Model {
                    event: machine_god_core::ModelEvent::Stop { .. }
                }
            ) {
                break;
            }
        }
        assert_eq!(fixture.tool.publications.load(Ordering::SeqCst), 0);
        if stage != "pre_poll" {
            futures_executor::block_on(async {
                assert!(futures_util::poll!(turn.next()).is_pending());
            });
            assert_eq!(fixture.tool.publications.load(Ordering::SeqCst), 1);
        }
        if stage == "drop" {
            drop(turn);
        } else {
            assert!(turn.handle().cancel());
            let events = futures_executor::block_on(turn.collect::<Vec<_>>())
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(matches!(
                events.last().unwrap().payload,
                TurnEvent::Completed {
                    reason: StopReason::Cancelled,
                    ..
                }
            ));
        }
        assert!(fixture.tool.prepared.lock().unwrap().is_empty());
        assert!(fixture.tool.executed.lock().unwrap().is_empty());
        assert_eq!(
            fixture
                .tool
                .store
                .record(&SessionId::new("input").unwrap())
                .unwrap()
                .messages
                .len(),
            1
        );
    }
}

#[test]
fn raw_byte_and_node_limits_accept_exact_and_reject_one_over() {
    let full = json!("x".repeat(100));
    for (bytes, ok) in [(102, true), (101, false)] {
        let mut tool = InputTool::new();
        tool.policy = Some(policy(bytes, 1));
        let fixture = Fixture::new(tool, EngineLimits::default(), vec![full.clone()]);
        if ok {
            completed(&fixture.run());
        } else {
            fixture.assert_failure("tool_argument_size_limit");
        }
    }
    for (nodes, ok) in [(9, true), (8, false)] {
        let mut tool = InputTool::new();
        tool.policy = Some(policy(100, nodes));
        let fixture = Fixture::new(
            tool,
            EngineLimits::default(),
            vec![json!([0, 0, 0, 0, 0, 0, 0, 0])],
        );
        if ok {
            completed(&fixture.run());
        } else {
            fixture.assert_failure("json_node_limit");
        }
    }
}

#[test]
fn complete_input_policy_does_not_enlarge_ordinary_arguments_or_inline_persistence() {
    let mut tool = InputTool::new();
    tool.policy = None;
    Fixture::new(
        tool,
        EngineLimits::default(),
        vec![json!("x".repeat(70_000))],
    )
    .assert_failure("tool_argument_size_limit");
    let mut tool = InputTool::new();
    tool.publication = Arc::new(|_, _| Ok(None));
    Fixture::new(
        tool,
        EngineLimits::default(),
        vec![json!("x".repeat(70_000))],
    )
    .assert_failure("tool_argument_size_limit");
    let mut tool = InputTool::new();
    tool.policy = None;
    Fixture::new(tool, EngineLimits::default(), vec![json!({})])
        .assert_failure("unexpected_persisted_tool_arguments");
    let mut tool = InputTool::new();
    tool.policy = None;
    tool.publication = Arc::new(|_, _| Ok(None));
    completed(&Fixture::new(tool, EngineLimits::default(), vec![json!({})]).run());
}

#[test]
fn cumulative_complete_budgets_precede_second_call_event_and_reset_each_turn() {
    let raw = json!([0, 0, 0]); // seven compact bytes, four nodes
    for (bytes, nodes, code) in [
        (13, 8, "cumulative_complete_tool_argument_size_limit"),
        (14, 7, "cumulative_complete_tool_argument_node_limit"),
    ] {
        let fixture = Fixture::new(
            InputTool::new(),
            EngineLimits {
                max_cumulative_complete_tool_argument_bytes: nz(bytes),
                max_cumulative_complete_tool_argument_nodes: nz(nodes),
                ..EngineLimits::default()
            },
            vec![raw.clone(), raw.clone()],
        );
        let events = fixture.run();
        assert!(
            matches!(&events.last().unwrap().payload,TurnEvent::Failed {code:actual,..} if actual==code)
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.payload,
                    TurnEvent::Model {
                        event: machine_god_core::ModelEvent::ToolCall { .. }
                    }
                ))
                .count(),
            1
        );
        assert_eq!(fixture.tool.publications.load(Ordering::SeqCst), 0);
    }
    let fixture = Fixture::turns(
        InputTool::new(),
        EngineLimits {
            max_cumulative_complete_tool_argument_bytes: nz(14),
            max_cumulative_complete_tool_argument_nodes: nz(8),
            ..EngineLimits::default()
        },
        vec![raw.clone(), raw],
        2,
    );
    completed(&fixture.run());
    completed(&fixture.run());
}

#[test]
fn prepared_arguments_and_whole_capability_have_independent_exact_bounds() {
    for (bytes, nodes, code) in [
        (5, 2, Some("tool_argument_size_limit")),
        (6, 1, Some("json_node_limit")),
        (6, 2, None),
    ] {
        let mut tool = InputTool::new();
        let mut bound = policy(100, 10);
        bound.max_prepared_argument_bytes = nz(bytes);
        bound.max_prepared_argument_nodes = nz(nodes);
        tool.policy = Some(bound);
        tool.replacement = Some(json!([null]));
        let fixture = Fixture::new(tool, EngineLimits::default(), vec![json!({})]);
        if let Some(code) = code {
            fixture.assert_failure(code);
        } else {
            completed(&fixture.run());
        }
    }
    let capability = Capability::Tool {
        name: ToolName::new("complete").unwrap(),
        call_id: ToolCallId::new("call-0-0").unwrap(),
        arguments: json!({}),
    };
    let exact = serde_json::to_vec(&capability).unwrap().len();
    for (bytes, ok) in [(exact, true), (exact - 1, false)] {
        let mut tool = InputTool::new();
        let mut bound = policy(100, 10);
        bound.max_prepared_argument_bytes = nz(bytes);
        tool.policy = Some(bound);
        tool.permission = true;
        let fixture = Fixture::new(tool, EngineLimits::default(), vec![json!({})]);
        if ok {
            completed(&fixture.run());
        } else {
            fixture.assert_failure("tool_argument_size_limit");
        }
    }
}

#[test]
fn projection_still_obeys_ordinary_byte_node_and_depth_limits() {
    for (projection, limits, code) in [
        (
            json!("x".repeat(70_000)),
            EngineLimits::default(),
            "tool_argument_size_limit",
        ),
        (
            json!([0, 0, 0, 0, 0, 0, 0, 0]),
            EngineLimits {
                max_json_nodes: nz(8),
                ..EngineLimits::default()
            },
            "json_node_limit",
        ),
        (
            nested(5),
            EngineLimits {
                max_json_depth: nz(4),
                ..EngineLimits::default()
            },
            "json_depth_limit",
        ),
    ] {
        let mut tool = InputTool::new();
        tool.publication = Arc::new(move |_, _| Ok(Some(projection.clone())));
        Fixture::new(tool, limits, vec![json!({})]).assert_failure(code);
    }
}

#[test]
fn publication_failure_in_second_call_prevents_every_action_and_hides_diagnostics() {
    let mut tool = InputTool::new();
    let count = Arc::new(AtomicUsize::new(0));
    tool.publication = Arc::new(move |_, _| {
        if count.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(Some(reference()))
        } else {
            Err(ToolError::new(
                ToolErrorKind::Execution,
                "private-code",
                "private diagnostic",
                true,
            ))
        }
    });
    let fixture = Fixture::new(tool, EngineLimits::default(), vec![json!({}), json!({})]);
    fixture.assert_failure("tool_argument_publication_failed");
    assert!(fixture.tool.prepared.lock().unwrap().is_empty());
    assert_eq!(
        fixture
            .tool
            .store
            .record(&SessionId::new("input").unwrap())
            .unwrap()
            .messages
            .len(),
        1
    );
}

#[test]
fn round_save_failure_after_publication_prevents_preparation_and_effects() {
    let mut tool = InputTool::new();
    tool.store = InMemorySessionStore::configured(
        std::collections::BTreeMap::new(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Error(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "save",
                    "fixture",
                    false,
                )),
            ]),
        },
        32,
    );
    let fixture = Fixture::new(tool, EngineLimits::default(), vec![json!({})]);
    fixture.assert_failure("store_failed");
    assert_eq!(fixture.tool.publications.load(Ordering::SeqCst), 1);
    assert!(fixture.tool.prepared.lock().unwrap().is_empty());
}

#[test]
fn same_poll_publication_cancellation_discards_projection_without_actions() {
    let mut tool = InputTool::new();
    tool.publication = Arc::new(|_, cancellation| {
        cancellation.cancel();
        Ok(Some(reference()))
    });
    let fixture = Fixture::new(tool, EngineLimits::default(), vec![json!({})]);
    let events = fixture.run();
    assert!(matches!(
        events.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert!(fixture.tool.prepared.lock().unwrap().is_empty());
    assert!(fixture.tool.executed.lock().unwrap().is_empty());
    assert_eq!(
        fixture
            .tool
            .store
            .record(&SessionId::new("input").unwrap())
            .unwrap()
            .messages
            .len(),
        1
    );
}

fn nested(depth: usize) -> Value {
    (0..depth).fold(Value::Null, |value, _| Value::Array(vec![value]))
}

#[test]
fn deep_published_json_is_guarded_before_rejection_and_same_poll_cancellation() {
    for case in ["depth", "not_enabled", "cancel"] {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "deep_input_projection_child", "--nocapture"])
            .env("MACHINE_GOD_DEEP_INPUT_PROJECTION", case)
            .status()
            .unwrap();
        assert!(status.success(), "{case}");
    }
}
#[test]
fn deep_input_projection_child() {
    let Ok(case) = std::env::var("MACHINE_GOD_DEEP_INPUT_PROJECTION") else {
        return;
    };
    let mut tool = InputTool::new();
    if case == "not_enabled" {
        tool.policy = None;
    }
    let cancel = case == "cancel";
    tool.publication = Arc::new(move |_, cancellation| {
        let value = nested(50_000);
        if cancel {
            cancellation.cancel();
        }
        Ok(Some(value))
    });
    let fixture = Fixture::new(tool, EngineLimits::default(), vec![json!({})]);
    if cancel {
        assert!(matches!(
            fixture.run().last().unwrap().payload,
            TurnEvent::Completed {
                reason: StopReason::Cancelled,
                ..
            }
        ));
    } else {
        fixture.assert_failure(if case == "depth" {
            "json_depth_limit"
        } else {
            "unexpected_persisted_tool_arguments"
        });
    }
}
