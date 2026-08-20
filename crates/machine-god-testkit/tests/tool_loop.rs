use futures_core::Stream;
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, ContentBlock, Engine, EngineEvent, EngineLimits, Message, ModelEvent,
    PermissionDecision, PermissionGrantScope, Role, SessionId, SessionRecord, SessionRevision,
    SessionStore, SessionStoreError, SessionStoreErrorKind, StopReason, TokenUsage, ToolCall,
    ToolCallId, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec, Turn, TurnEvent,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, RecordingEventSink,
    ScriptedModelProvider, ScriptedPermissionHandler, ScriptedTool, SessionStoreScript,
    SessionStoreStep, ToolStep,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

fn tool_name(name: &str) -> ToolName {
    ToolName::new(name).unwrap()
}

fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(id).unwrap(),
        name: tool_name(name),
        arguments,
    }
}

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: tool_name(name),
        description: format!("test tool {name}"),
        input_schema: json!({"type": "object"}),
    }
}

fn allow() -> PermissionStep {
    PermissionStep::Decision(PermissionDecision::Allow {
        scope: PermissionGrantScope::Once,
    })
}

fn events(step: impl IntoIterator<Item = ModelEvent>) -> ModelProviderStep {
    ModelProviderStep::events(step)
}

fn collect(session: &machine_god_core::Session) -> Vec<EngineEvent> {
    futures_executor::block_on(async {
        session
            .prompt("solve it")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
}

#[test]
#[allow(clippy::too_many_lines)]
fn two_round_tool_turn_is_serial_ordered_and_durable() {
    let first_usage = TokenUsage {
        input_tokens: 10,
        output_tokens: 2,
        cached_input_tokens: 3,
    };
    let second_usage = TokenUsage {
        input_tokens: 14,
        output_tokens: 4,
        cached_input_tokens: 5,
    };
    let requested = call("call-1", "lookup", json!({"key": "answer"}));
    let provider = ScriptedModelProvider::new(
        "two-round",
        [
            events([
                ModelEvent::TextDelta {
                    text: "checking ".to_owned(),
                },
                ModelEvent::TextDelta {
                    text: "now".to_owned(),
                },
                ModelEvent::ReasoningDelta {
                    text: "not durable".to_owned(),
                },
                ModelEvent::ToolCall {
                    call: requested.clone(),
                },
                ModelEvent::Usage { usage: first_usage },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            events([
                ModelEvent::TextDelta {
                    text: "answer is 42".to_owned(),
                },
                ModelEvent::Usage {
                    usage: second_usage,
                },
                ModelEvent::Stop {
                    reason: StopReason::Completed,
                },
            ]),
        ],
    );
    let tool = ScriptedTool::new(
        spec("lookup"),
        [ToolStep::Output(ToolOutput::success(json!({"value": 42})))],
    );
    let store = InMemorySessionStore::new();
    let sink = RecordingEventSink::new();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .event_sink(sink.clone())
        .tool(tool.clone())
        .build()
        .unwrap();
    let id = SessionId::new("two-round").unwrap();
    let session = engine.create_session(id.clone());

    let observed = collect(&session);
    assert_eq!(observed, sink.events());
    assert!(matches!(observed[0].payload, TurnEvent::Started));
    assert!(matches!(
        observed.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            usage: TokenUsage {
                input_tokens: 24,
                output_tokens: 6,
                cached_input_tokens: 8,
            }
        }
    ));
    assert_eq!(
        observed
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        (0..observed.len() as u64).collect::<Vec<_>>()
    );

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request.messages.len(), 1);
    assert_eq!(requests[1].request.messages.len(), 3);
    assert_eq!(requests[1].request.messages[0].role, Role::User);
    assert_eq!(requests[1].request.messages[1].role, Role::Assistant);
    assert_eq!(requests[1].request.messages[2].role, Role::Tool);
    assert_eq!(
        requests[1].request.messages[1].content[0],
        ContentBlock::Text {
            text: "checking now".to_owned()
        }
    );
    assert!(
        requests[1].request.messages[1]
            .content
            .iter()
            .all(|block| !matches!(block, ContentBlock::Text { text } if text == "not durable"))
    );
    assert_eq!(tool.invocations().len(), 1);

    let durable = store.record(&id).unwrap();
    assert_eq!(durable.messages.len(), 4);
    assert_eq!(durable.messages[3].role, Role::Assistant);
    let reloaded = futures_executor::block_on(engine.load_session(id))
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.record(), durable);
}

#[test]
fn multiple_tools_are_authorized_executed_and_persisted_in_provider_order() {
    let first = call("first", "alpha", json!({"n": 1}));
    let second = call("second", "beta", json!({"n": 2}));
    let provider = ScriptedModelProvider::new(
        "ordered-tools",
        [
            events([
                ModelEvent::ToolCall {
                    call: first.clone(),
                },
                ModelEvent::ToolCall {
                    call: second.clone(),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    );
    let alpha = ScriptedTool::new(
        spec("alpha"),
        [ToolStep::Output(ToolOutput::success("alpha-output"))],
    );
    let beta = ScriptedTool::new(
        spec("beta"),
        [ToolStep::Output(ToolOutput::success("beta-output"))],
    );
    let permissions = ScriptedPermissionHandler::new([allow(), allow()]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(InMemorySessionStore::new())
        .permission_handler(permissions.clone())
        .tool(beta.clone())
        .tool(alpha.clone())
        .build()
        .unwrap();

    let output = collect(&engine.create_session(SessionId::new("ordered-tools").unwrap()));
    assert!(matches!(
        output.last().unwrap().payload,
        TurnEvent::Completed { .. }
    ));
    assert_eq!(
        permissions
            .requests()
            .into_iter()
            .map(|request| match request.capability {
                machine_god_core::Capability::Tool { call_id, .. } => call_id.to_string(),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(alpha.invocations().len(), 1);
    assert_eq!(beta.invocations().len(), 1);
    let second_request = &provider.requests()[1].request;
    assert_eq!(
        second_request
            .messages
            .iter()
            .map(|message| message.role)
            .collect::<Vec<_>>(),
        [Role::User, Role::Assistant, Role::Tool, Role::Tool]
    );
}

#[test]
fn denial_and_tool_error_become_durable_model_visible_results() {
    let denied = call("denied", "shared", json!({}));
    let failed = call("failed", "shared", json!({}));
    let provider = ScriptedModelProvider::new(
        "error-roundtrip",
        [
            events([
                ModelEvent::ToolCall { call: denied },
                ModelEvent::ToolCall { call: failed },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    );
    let tool = ScriptedTool::new(
        spec("shared"),
        [ToolStep::Error(ToolError::new(
            ToolErrorKind::Execution,
            "failed",
            "expected test failure",
            false,
        ))],
    );
    let permissions = ScriptedPermissionHandler::new([
        PermissionStep::Decision(PermissionDecision::Deny {
            reason: "policy says no".to_owned(),
        }),
        allow(),
    ]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(InMemorySessionStore::new())
        .permission_handler(permissions)
        .tool(tool.clone())
        .build()
        .unwrap();

    let output = collect(&engine.create_session(SessionId::new("error-roundtrip").unwrap()));
    assert!(matches!(
        output.last().unwrap().payload,
        TurnEvent::Completed { .. }
    ));
    assert_eq!(tool.invocations().len(), 1);
    assert_eq!(
        output
            .iter()
            .filter(|event| matches!(event.payload, TurnEvent::ToolStarted { .. }))
            .count(),
        1
    );
    let request = &provider.requests()[1].request;
    for message in &request.messages[2..4] {
        let ContentBlock::ToolResult { output, .. } = &message.content[0] else {
            panic!("expected tool result")
        };
        assert!(output.is_error);
    }
}

fn protocol_failure(
    provider_events: Vec<ModelEvent>,
    tools: Vec<ScriptedTool>,
) -> (Vec<EngineEvent>, ScriptedPermissionHandler) {
    let provider = ScriptedModelProvider::new("invalid", [events(provider_events)]);
    let permissions = ScriptedPermissionHandler::new([]);
    let mut builder = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(permissions.clone());
    for tool in tools {
        builder = builder.tool(tool);
    }
    let engine = builder.build().unwrap();
    let output = collect(&engine.create_session(SessionId::new("invalid").unwrap()));
    (output, permissions)
}

#[test]
fn malformed_rounds_fail_before_permission_or_execution() {
    let (output, permissions) = protocol_failure(
        vec![ModelEvent::Stop {
            reason: StopReason::ToolCalls,
        }],
        Vec::new(),
    );
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "tool_stop_without_calls"
    ));
    assert!(permissions.requests().is_empty());

    let unknown = call("unknown", "missing", json!({}));
    let (output, permissions) = protocol_failure(
        vec![
            ModelEvent::ToolCall { call: unknown },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ],
        Vec::new(),
    );
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "unknown_tool"
    ));
    assert!(permissions.requests().is_empty());

    let duplicate = call("duplicate", "known", json!({}));
    let known = ScriptedTool::new(spec("known"), []);
    let (output, permissions) = protocol_failure(
        vec![
            ModelEvent::ToolCall {
                call: duplicate.clone(),
            },
            ModelEvent::ToolCall { call: duplicate },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ],
        vec![known],
    );
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "duplicate_tool_call_id"
    ));
    assert!(permissions.requests().is_empty());

    let incompatible = call("incompatible", "known", json!({}));
    let known = ScriptedTool::new(spec("known"), []);
    let (output, permissions) = protocol_failure(
        vec![
            ModelEvent::ToolCall { call: incompatible },
            ModelEvent::Stop {
                reason: StopReason::Completed,
            },
        ],
        vec![known],
    );
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "calls_with_incompatible_stop"
    ));
    assert!(permissions.requests().is_empty());
}

#[test]
fn duplicate_call_id_in_a_later_round_fails_before_reexecution() {
    let repeated = call("repeated", "known", json!({}));
    let provider = ScriptedModelProvider::new(
        "cross-round-duplicate",
        [
            events([
                ModelEvent::ToolCall {
                    call: repeated.clone(),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            events([
                ModelEvent::ToolCall { call: repeated },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
        ],
    );
    let tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success("once"))],
    );
    let permissions = ScriptedPermissionHandler::new([allow()]);
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(permissions.clone())
        .tool(tool.clone())
        .build()
        .unwrap();
    let id = SessionId::new("cross-round-duplicate").unwrap();
    let output = collect(&engine.create_session(id.clone()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "duplicate_tool_call_id"
    ));
    assert_eq!(permissions.requests().len(), 1);
    assert_eq!(tool.invocations().len(), 1);
    assert_eq!(store.record(&id).unwrap().messages.len(), 3);
}

fn with_limit(mut limits: EngineLimits, field: &str, value: usize) -> EngineLimits {
    let value = NonZeroUsize::new(value).unwrap();
    match field {
        "rounds" => limits.max_model_rounds = value,
        "calls_round" => limits.max_tool_calls_per_round = value,
        "calls_turn" => limits.max_tool_calls_per_turn = value,
        "text" => limits.max_assistant_text_bytes = value,
        "reasoning" => limits.max_reasoning_bytes = value,
        "arguments" => limits.max_tool_argument_bytes = value,
        "result" => limits.max_serialized_tool_result_bytes = value,
        "cumulative" => limits.max_cumulative_tool_result_bytes = value,
        _ => panic!("unknown limit"),
    }
    limits
}

#[test]
fn per_round_call_budget_fails_before_any_execution() {
    let provider = ScriptedModelProvider::new(
        "round-budget",
        [events([
            ModelEvent::ToolCall {
                call: call("one", "known", json!({})),
            },
            ModelEvent::ToolCall {
                call: call("two", "known", json!({})),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let tool = ScriptedTool::new(spec("known"), []);
    let permissions = ScriptedPermissionHandler::new([]);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(permissions.clone())
        .tool(tool.clone())
        .limits(with_limit(EngineLimits::default(), "calls_round", 1))
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("round-budget").unwrap()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "tool_calls_per_round_limit"
    ));
    assert!(permissions.requests().is_empty());
    assert!(tool.invocations().is_empty());
}

#[test]
fn assistant_reasoning_and_argument_budgets_are_checked() {
    for (id, field, event, code) in [
        (
            "text-budget",
            "text",
            ModelEvent::TextDelta {
                text: "12345".to_owned(),
            },
            "assistant_text_size_limit",
        ),
        (
            "reasoning-budget",
            "reasoning",
            ModelEvent::ReasoningDelta {
                text: "12345".to_owned(),
            },
            "reasoning_size_limit",
        ),
    ] {
        let provider = ScriptedModelProvider::new(
            id,
            [events([
                event,
                ModelEvent::Stop {
                    reason: StopReason::Completed,
                },
            ])],
        );
        let engine = Engine::builder()
            .provider(provider)
            .session_store(InMemorySessionStore::new())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .limits(with_limit(EngineLimits::default(), field, 4))
            .build()
            .unwrap();
        let output = collect(&engine.create_session(SessionId::new(id).unwrap()));
        assert!(matches!(
            &output.last().unwrap().payload,
            TurnEvent::Failed { code: observed, .. } if observed == code
        ));
    }

    let requested = call("large-args", "known", json!({"value": "12345"}));
    let provider = ScriptedModelProvider::new(
        "argument-budget",
        [events([
            ModelEvent::ToolCall { call: requested },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let tool = ScriptedTool::new(spec("known"), []);
    let permissions = ScriptedPermissionHandler::new([]);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(permissions.clone())
        .tool(tool.clone())
        .limits(with_limit(EngineLimits::default(), "arguments", 4))
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("argument-budget").unwrap()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "tool_argument_size_limit"
    ));
    assert!(permissions.requests().is_empty());
    assert!(tool.invocations().is_empty());
}

#[test]
fn oversized_executed_result_leaves_a_durable_unknown_result_marker() {
    let requested = call("large-result", "large", json!({}));
    let provider = ScriptedModelProvider::new(
        "large-result",
        [events([
            ModelEvent::ToolCall { call: requested },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let tool = ScriptedTool::new(
        spec("large"),
        [ToolStep::Output(ToolOutput::success("a large result"))],
    );
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(tool.clone())
        .limits(with_limit(EngineLimits::default(), "result", 8))
        .build()
        .unwrap();
    let id = SessionId::new("large-result").unwrap();
    let output = collect(&engine.create_session(id.clone()));

    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "tool_result_size_limit"
    ));
    assert_eq!(tool.invocations().len(), 1);
    let durable = store.record(&id).unwrap();
    assert_eq!(durable.messages.len(), 3);
    let ContentBlock::ToolResult { output, .. } = &durable.messages[2].content[0] else {
        panic!("expected durable marker")
    };
    assert!(output.is_error);
    assert_eq!(output.content["code"], "tool_result_discarded");
}

#[test]
fn stop_is_immediate_even_when_provider_stream_would_remain_pending() {
    let provider = ScriptedModelProvider::new(
        "stop-pending",
        [ModelProviderStep::events_then_pending([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("stop-pending").unwrap()));
    assert!(matches!(
        output.last().unwrap().payload,
        TurnEvent::Completed { .. }
    ));
}

#[test]
fn model_round_and_turn_call_budgets_stop_before_additional_work() {
    let first = call("first", "known", json!({}));
    let provider = ScriptedModelProvider::new(
        "round-limit",
        [
            events([
                ModelEvent::ToolCall { call: first },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::pending(),
        ],
    );
    let tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success("done"))],
    );
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(tool.clone())
        .limits(with_limit(EngineLimits::default(), "rounds", 1))
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("round-limit").unwrap()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "model_round_limit"
    ));
    assert_eq!(provider.requests().len(), 1);
    assert_eq!(tool.invocations().len(), 1);

    let provider = ScriptedModelProvider::new(
        "turn-call-limit",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("one", "known", json!({})),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            events([
                ModelEvent::ToolCall {
                    call: call("two", "known", json!({})),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
        ],
    );
    let tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success("first"))],
    );
    let permissions = ScriptedPermissionHandler::new([allow()]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(InMemorySessionStore::new())
        .permission_handler(permissions.clone())
        .tool(tool.clone())
        .limits(with_limit(EngineLimits::default(), "calls_turn", 1))
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("turn-call-limit").unwrap()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "tool_call_limit"
    ));
    assert_eq!(provider.requests().len(), 2);
    assert_eq!(permissions.requests().len(), 1);
    assert_eq!(tool.invocations().len(), 1);
}

#[test]
fn exact_result_boundary_is_allowed_and_cumulative_boundary_is_checked() {
    let output_value = ToolOutput::success(json!({"value": 1}));
    let exact = serde_json::to_vec(&output_value).unwrap().len();
    let provider = ScriptedModelProvider::new(
        "exact-result",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("exact", "known", json!({})),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    );
    let tool = ScriptedTool::new(spec("known"), [ToolStep::Output(output_value.clone())]);
    let mut limits = with_limit(EngineLimits::default(), "result", exact);
    limits = with_limit(limits, "cumulative", exact);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(tool)
        .limits(limits)
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("exact-result").unwrap()));
    assert!(matches!(
        output.last().unwrap().payload,
        TurnEvent::Completed { .. }
    ));

    let provider = ScriptedModelProvider::new(
        "cumulative-result",
        [events([
            ModelEvent::ToolCall {
                call: call("one", "known", json!({})),
            },
            ModelEvent::ToolCall {
                call: call("two", "known", json!({})),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let tool = ScriptedTool::new(
        spec("known"),
        [
            ToolStep::Output(output_value.clone()),
            ToolStep::Output(output_value),
        ],
    );
    let store = InMemorySessionStore::new();
    let mut limits = with_limit(EngineLimits::default(), "result", exact);
    limits = with_limit(limits, "cumulative", exact * 2 - 1);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([allow(), allow()]))
        .tool(tool.clone())
        .limits(limits)
        .build()
        .unwrap();
    let id = SessionId::new("cumulative-result").unwrap();
    let output = collect(&engine.create_session(id.clone()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "cumulative_tool_result_size_limit"
    ));
    assert_eq!(tool.invocations().len(), 2);
    let durable = store.record(&id).unwrap();
    assert_eq!(durable.messages.len(), 4);
    let ContentBlock::ToolResult { output, .. } = &durable.messages[3].content[0] else {
        panic!("expected marker")
    };
    assert_eq!(output.content["code"], "tool_result_discarded");
}

#[test]
fn usage_accumulation_overflow_fails_closed() {
    let provider = ScriptedModelProvider::new(
        "usage-overflow",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("first", "known", json!({})),
                },
                ModelEvent::Usage {
                    usage: TokenUsage {
                        input_tokens: u64::MAX,
                        output_tokens: 0,
                        cached_input_tokens: 0,
                    },
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            events([ModelEvent::Usage {
                usage: TokenUsage {
                    input_tokens: 1,
                    output_tokens: 0,
                    cached_input_tokens: 0,
                },
            }]),
        ],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(ScriptedTool::new(
            spec("known"),
            [ToolStep::Output(ToolOutput::success("done"))],
        ))
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("usage-overflow").unwrap()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "usage_overflow"
    ));
}

fn poll_pending(turn: &mut Turn) {
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);
    assert!(matches!(
        Pin::new(turn).poll_next(&mut context),
        Poll::Pending
    ));
}

fn next(turn: &mut Turn) -> EngineEvent {
    futures_executor::block_on(turn.next()).unwrap().unwrap()
}

#[test]
#[allow(clippy::too_many_lines)]
fn cancellation_interrupts_pending_permission_tool_store_and_new_provider_phases() {
    // Permission policy pending.
    let provider = ScriptedModelProvider::new(
        "cancel-permission",
        [events([
            ModelEvent::ToolCall {
                call: call("call", "known", json!({})),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([PermissionStep::Pending]))
        .tool(ScriptedTool::new(spec("known"), []))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("cancel-permission").unwrap());
    let mut turn = futures_executor::block_on(session.prompt("go")).unwrap();
    for _ in 0..4 {
        let _ = next(&mut turn);
    }
    poll_pending(&mut turn);
    assert!(turn.handle().cancel());
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));

    // Tool execution pending.
    let provider = ScriptedModelProvider::new(
        "cancel-tool",
        [events([
            ModelEvent::ToolCall {
                call: call("call", "known", json!({})),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(ScriptedTool::new(spec("known"), [ToolStep::Pending]))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("cancel-tool").unwrap());
    let mut turn = futures_executor::block_on(session.prompt("go")).unwrap();
    for _ in 0..6 {
        let _ = next(&mut turn);
    }
    poll_pending(&mut turn);
    assert!(turn.handle().cancel());
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));

    // Assistant transcript save pending before any permission request.
    let provider = ScriptedModelProvider::new(
        "cancel-store",
        [events([
            ModelEvent::ToolCall {
                call: call("call", "known", json!({})),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let store = InMemorySessionStore::configured(
        BTreeMap::new(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![SessionStoreStep::Pass, SessionStoreStep::Pending]),
        },
        32,
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .tool(ScriptedTool::new(spec("known"), []))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("cancel-store").unwrap());
    let mut turn = futures_executor::block_on(session.prompt("go")).unwrap();
    for _ in 0..3 {
        let _ = next(&mut turn);
    }
    poll_pending(&mut turn);
    assert!(turn.handle().cancel());
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));

    // New provider startup pending after a durably finished tool.
    let provider = ScriptedModelProvider::new(
        "cancel-next-provider",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("call", "known", json!({})),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::PendingStart,
        ],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(ScriptedTool::new(
            spec("known"),
            [ToolStep::Output(ToolOutput::success("done"))],
        ))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("cancel-next-provider").unwrap());
    let mut turn = futures_executor::block_on(session.prompt("go")).unwrap();
    for _ in 0..7 {
        let _ = next(&mut turn);
    }
    poll_pending(&mut turn);
    assert!(turn.handle().cancel());
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
}

#[derive(Clone, Copy, Debug)]
enum ConflictInjection {
    AllocatorOnly,
    Transcript,
}

#[derive(Debug)]
struct ConflictStoreState {
    record: Option<SessionRecord>,
    save_calls: usize,
    injected: bool,
}

#[derive(Clone, Debug)]
struct ConflictOnceStore {
    injection: ConflictInjection,
    state: Arc<Mutex<ConflictStoreState>>,
}

impl ConflictOnceStore {
    fn new(injection: ConflictInjection) -> Self {
        Self {
            injection,
            state: Arc::new(Mutex::new(ConflictStoreState {
                record: None,
                save_calls: 0,
                injected: false,
            })),
        }
    }

    fn record(&self) -> SessionRecord {
        self.state.lock().unwrap().record.clone().unwrap()
    }
}

impl SessionStore for ConflictOnceStore {
    fn load(
        &self,
        _id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        let record = self.state.lock().unwrap().record.clone();
        Box::pin(async move { Ok(record) })
    }

    fn save(
        &self,
        mut record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        let result = {
            let mut state = self.state.lock().unwrap();
            state.save_calls += 1;
            if state.save_calls == 2 && !state.injected {
                state.injected = true;
                let current = state.record.as_mut().unwrap();
                current.revision = SessionRevision(current.revision.0 + 1);
                current.next_turn_sequence += 1;
                current.metadata.insert("external".to_owned(), json!(true));
                if matches!(self.injection, ConflictInjection::Transcript) {
                    current.messages.push(Message::text(Role::User, "external"));
                }
                Err(SessionStoreError::new(
                    SessionStoreErrorKind::Conflict,
                    "injected_conflict",
                    "test conflict",
                    true,
                ))
            } else {
                let current_revision = state.record.as_ref().map(|value| value.revision);
                if current_revision == expected_revision {
                    let revision = SessionRevision(current_revision.map_or(1, |value| value.0 + 1));
                    record.revision = revision;
                    state.record = Some(record);
                    Ok(revision)
                } else {
                    Err(SessionStoreError::new(
                        SessionStoreErrorKind::Conflict,
                        "revision_conflict",
                        "revision changed",
                        true,
                    ))
                }
            }
        };
        Box::pin(async move { result })
    }
}

#[test]
fn allocator_only_conflict_is_merged_but_transcript_divergence_fails_closed() {
    let provider = ScriptedModelProvider::new(
        "allocator-conflict",
        [events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let store = ConflictOnceStore::new(ConflictInjection::AllocatorOnly);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("allocator-conflict").unwrap()));
    assert!(matches!(
        output.last().unwrap().payload,
        TurnEvent::Completed { .. }
    ));
    let durable = store.record();
    assert_eq!(durable.next_turn_sequence, 3);
    assert_eq!(durable.metadata["external"], true);
    assert_eq!(durable.messages.len(), 2);

    let provider = ScriptedModelProvider::new(
        "transcript-conflict",
        [events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let store = ConflictOnceStore::new(ConflictInjection::Transcript);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("transcript-conflict").unwrap()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "transcript_diverged"
    ));
    let durable = store.record();
    assert_eq!(durable.messages.len(), 2);
    assert_eq!(durable.messages[1], Message::text(Role::User, "external"));
}

#[derive(Clone, Debug)]
struct GatedFinalStore {
    inner: InMemorySessionStore,
    save_calls: Arc<AtomicUsize>,
    final_ready: Arc<AtomicBool>,
}

impl SessionStore for GatedFinalStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.inner.load(id)
    }

    fn save(
        &self,
        record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        let call = self.save_calls.fetch_add(1, Ordering::AcqRel) + 1;
        let inner = self.inner.clone();
        let ready = Arc::clone(&self.final_ready);
        Box::pin(async move {
            if call == 2 {
                std::future::poll_fn(|context| {
                    if ready.load(Ordering::Acquire) {
                        Poll::Ready(())
                    } else {
                        let _ = context;
                        Poll::Pending
                    }
                })
                .await;
            }
            inner.save(record, expected_revision).await
        })
    }
}

#[test]
fn final_stop_survives_cancellation_and_completion_follows_durable_commit() {
    let provider = ScriptedModelProvider::new(
        "final-commit",
        [events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let inner = InMemorySessionStore::new();
    let store = GatedFinalStore {
        inner: inner.clone(),
        save_calls: Arc::new(AtomicUsize::new(0)),
        final_ready: Arc::new(AtomicBool::new(false)),
    };
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let id = SessionId::new("final-commit").unwrap();
    let session = engine.create_session(id.clone());
    let mut turn = futures_executor::block_on(session.prompt("go")).unwrap();
    assert!(matches!(next(&mut turn).payload, TurnEvent::Started));
    poll_pending(&mut turn);
    assert_eq!(inner.record(&id).unwrap().messages.len(), 1);
    assert!(turn.handle().cancel());
    poll_pending(&mut turn);
    store.final_ready.store(true, Ordering::Release);
    let model_stop = next(&mut turn);
    assert!(matches!(
        model_stop.payload,
        TurnEvent::Model {
            event: ModelEvent::Stop {
                reason: StopReason::Completed
            }
        }
    ));
    assert_eq!(inner.record(&id).unwrap().messages.len(), 2);
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
}

#[derive(Clone, Debug)]
struct NonIncreasingFinalStore {
    inner: InMemorySessionStore,
    saves: Arc<AtomicUsize>,
}

impl SessionStore for NonIncreasingFinalStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.inner.load(id)
    }

    fn save(
        &self,
        record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        let call = self.saves.fetch_add(1, Ordering::AcqRel) + 1;
        if call == 2 {
            let revision = record.revision;
            Box::pin(async move { Ok(revision) })
        } else {
            self.inner.save(record, expected_revision)
        }
    }
}

#[test]
fn nonincreasing_store_revision_fails_without_claiming_completion() {
    let provider = ScriptedModelProvider::new(
        "bad-revision",
        [events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let inner = InMemorySessionStore::new();
    let store = NonIncreasingFinalStore {
        inner: inner.clone(),
        saves: Arc::new(AtomicUsize::new(0)),
    };
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let id = SessionId::new("bad-revision").unwrap();
    let output = collect(&engine.create_session(id.clone()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "non_increasing_revision"
    ));
    assert!(
        !output
            .iter()
            .any(|event| matches!(event.payload, TurnEvent::Completed { .. }))
    );
    assert_eq!(inner.record(&id).unwrap().messages.len(), 1);
}
