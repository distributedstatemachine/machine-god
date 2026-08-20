use futures_core::Stream;
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, BuildError, CancellationToken, ContentBlock, Engine, EngineError, EngineEvent,
    EngineLimits, InferenceOptions, Message, ModelEvent, ModelEventStream, ModelProvider,
    ModelRequest, PermissionDecision, PermissionError, PermissionGrantScope, PermissionHandler,
    PermissionRequest, Prompt, ProviderError, ProviderErrorKind, Role, SessionId, SessionRecord,
    SessionRevision, SessionStore, SessionStoreError, SessionStoreErrorKind, StopReason,
    TokenUsage, Tool, ToolCall, ToolCallId, ToolContext, ToolError, ToolErrorKind, ToolName,
    ToolOutput, ToolSpec, Turn, TurnEvent,
};
use machine_god_testkit::{
    EventSinkStep, InMemorySessionStore, ModelProviderStep, PermissionStep, RecordingEventSink,
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

fn unknown_result_size() -> usize {
    serde_json::to_vec(&ToolOutput {
        content: json!({
            "code": "tool_result_unknown",
            "message": "tool result status is unknown",
        }),
        is_error: true,
    })
    .unwrap()
    .len()
}

fn nested_array(container_depth: usize) -> Value {
    let mut value = json!("leaf");
    for _ in 0..container_depth {
        value = Value::Array(vec![value]);
    }
    value
}

fn flat_array_nodes(total_nodes: usize) -> Value {
    assert!(total_nodes > 0);
    Value::Array(vec![Value::Null; total_nodes - 1])
}

fn assert_unknown_result(message: &Message, expected_call: &str) {
    assert_eq!(message.role, Role::Tool);
    let ContentBlock::ToolResult { call_id, output } = &message.content[0] else {
        panic!("expected tool result placeholder")
    };
    assert_eq!(call_id.as_str(), expected_call);
    assert!(output.is_error);
    assert_eq!(output.content["code"], "tool_result_unknown");
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
        "events" => limits.max_model_events_per_turn = value,
        "calls_round" => limits.max_tool_calls_per_round = value,
        "calls_turn" => limits.max_tool_calls_per_turn = value,
        "json_depth" => limits.max_json_depth = value,
        "json_nodes" => limits.max_json_nodes = value,
        "text" => limits.max_assistant_text_bytes = value,
        "reasoning" => limits.max_reasoning_bytes = value,
        "stop_detail" => limits.max_stop_detail_bytes = value,
        "prompt" => limits.max_prompt_bytes = value,
        "session_metadata" => limits.max_session_metadata_bytes = value,
        "inference_options" => limits.max_inference_options_bytes = value,
        "transcript_messages" => limits.max_transcript_messages = value,
        "transcript_bytes" => limits.max_transcript_bytes = value,
        "tool_catalog" => limits.max_tool_catalog_bytes = value,
        "arguments" => limits.max_tool_argument_bytes = value,
        "result" => limits.max_serialized_tool_result_bytes = value,
        "cumulative" => limits.max_cumulative_tool_result_bytes = value,
        "denial_reason" => limits.max_permission_denial_reason_bytes = value,
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
        [ToolStep::Output(ToolOutput::success("x".repeat(4_096)))],
    );
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(tool.clone())
        .limits(with_limit(
            EngineLimits::default(),
            "result",
            unknown_result_size(),
        ))
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
    assert_eq!(output.content["code"], "tool_result_unknown");
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
    let result_limit = exact.max(unknown_result_size());
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
    let mut limits = with_limit(EngineLimits::default(), "result", result_limit);
    limits = with_limit(limits, "cumulative", unknown_result_size());
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

    let cumulative_value = ToolOutput::success("z".repeat(unknown_result_size()));
    let cumulative_size = serde_json::to_vec(&cumulative_value).unwrap().len();
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
            ToolStep::Output(cumulative_value.clone()),
            ToolStep::Output(cumulative_value),
        ],
    );
    let store = InMemorySessionStore::new();
    let mut limits = with_limit(EngineLimits::default(), "result", cumulative_size);
    limits = with_limit(limits, "cumulative", cumulative_size * 2 - 1);
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
    assert_eq!(output.content["code"], "tool_result_unknown");
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
fn pending_final_commit_is_cancellable_and_releases_the_lease() {
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
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(inner.record(&id).unwrap().messages.len(), 1);
    assert!(!session.has_active_turn());
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

#[test]
fn provider_originated_cancelled_completion_cannot_bypass_pending_sink() {
    let provider = ScriptedModelProvider::new(
        "provider-cancelled",
        [events([ModelEvent::Stop {
            reason: StopReason::Cancelled,
        }])],
    );
    let sink = RecordingEventSink::scripted([
        EventSinkStep::Accept,
        EventSinkStep::Accept,
        EventSinkStep::Pending,
    ]);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .event_sink(sink.clone())
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("provider-cancelled").unwrap());
    let mut turn = futures_executor::block_on(session.prompt("go")).unwrap();
    assert!(matches!(next(&mut turn).payload, TurnEvent::Started));
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::Model {
            event: ModelEvent::Stop {
                reason: StopReason::Cancelled
            }
        }
    ));
    poll_pending(&mut turn);
    assert!(turn.handle().cancel());
    poll_pending(&mut turn);
    assert!(session.has_active_turn());
    assert!(matches!(
        sink.events()[2].payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    drop(turn);
    assert!(!session.has_active_turn());
}

#[test]
fn model_event_and_stop_detail_limits_enforce_exact_boundaries() {
    for (id, event_limit, completes) in [("event-boundary", 3, true), ("event-over", 2, false)] {
        let provider = ScriptedModelProvider::new(
            id,
            [events([
                ModelEvent::TextDelta {
                    text: String::new(),
                },
                ModelEvent::Usage {
                    usage: TokenUsage::default(),
                },
                ModelEvent::Stop {
                    reason: StopReason::Completed,
                },
            ])],
        );
        let engine = Engine::builder()
            .provider(provider)
            .session_store(InMemorySessionStore::new())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .limits(with_limit(EngineLimits::default(), "events", event_limit))
            .build()
            .unwrap();
        let session = engine.create_session(SessionId::new(id).unwrap());
        let output = collect(&session);
        if completes {
            assert!(matches!(
                output.last().unwrap().payload,
                TurnEvent::Completed { .. }
            ));
        } else {
            assert!(matches!(
                &output.last().unwrap().payload,
                TurnEvent::Failed { code, .. } if code == "model_event_limit"
            ));
        }
        assert!(!session.has_active_turn());
    }

    for (id, detail, completes) in [
        ("stop-detail-boundary", "1234", true),
        ("stop-detail-over", "12345", false),
    ] {
        let provider = ScriptedModelProvider::new(
            id,
            [events([ModelEvent::Stop {
                reason: StopReason::Other(detail.to_owned()),
            }])],
        );
        let engine = Engine::builder()
            .provider(provider)
            .session_store(InMemorySessionStore::new())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .limits(with_limit(EngineLimits::default(), "stop_detail", 4))
            .build()
            .unwrap();
        let output = collect(&engine.create_session(SessionId::new(id).unwrap()));
        assert_eq!(
            matches!(output.last().unwrap().payload, TurnEvent::Completed { .. }),
            completes
        );
    }
}

#[test]
fn prompt_and_transcript_limits_fail_before_unbounded_cloning_or_persistence() {
    let provider = ScriptedModelProvider::new("prompt-limit", []);
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(EngineLimits::default(), "prompt", 4))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("prompt-limit").unwrap());
    assert!(matches!(
        futures_executor::block_on(session.prompt("12345")),
        Err(EngineError::Protocol(message)) if message.contains("prompt")
    ));
    assert!(provider.requests().is_empty());
    assert!(store.calls().is_empty());
    assert!(!session.has_active_turn());

    let id = SessionId::new("hostile-history").unwrap();
    let hostile = SessionRecord {
        id: id.clone(),
        revision: SessionRevision(1),
        next_turn_sequence: 2,
        messages: vec![
            Message::text(Role::User, "one"),
            Message::text(Role::Assistant, "two"),
        ],
        metadata: BTreeMap::new(),
    };
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
            id.clone(),
            hostile,
        )])))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(
            EngineLimits::default(),
            "transcript_messages",
            1,
        ))
        .build()
        .unwrap();
    assert!(matches!(
        futures_executor::block_on(engine.load_session(id)),
        Err(EngineError::Protocol(message)) if message.contains("message limit")
    ));

    let id = SessionId::new("hostile-history-bytes").unwrap();
    let hostile = SessionRecord {
        id: id.clone(),
        revision: SessionRevision(1),
        next_turn_sequence: 2,
        messages: vec![Message::text(Role::User, "larger than four bytes")],
        metadata: BTreeMap::new(),
    };
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
            id.clone(),
            hostile,
        )])))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(EngineLimits::default(), "transcript_bytes", 4))
        .build()
        .unwrap();
    assert!(matches!(
        futures_executor::block_on(engine.load_session(id)),
        Err(EngineError::Protocol(message)) if message.contains("serialized byte limit")
    ));

    let provider = ScriptedModelProvider::new(
        "growing-history",
        [events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(
            EngineLimits::default(),
            "transcript_messages",
            1,
        ))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("growing-history").unwrap());
    let output = collect(&session);
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "transcript_message_limit"
    ));
    assert_eq!(provider.requests().len(), 1);
}

#[test]
#[allow(clippy::too_many_lines)]
fn metadata_options_and_tool_catalog_limits_enforce_serialized_boundaries() {
    let metadata = BTreeMap::from([(
        "nested".to_owned(),
        json!({"values": ["one", {"two": "three"}]}),
    )]);
    let metadata_size = serde_json::to_vec(&metadata).unwrap().len();
    let id = SessionId::new("metadata-boundary").unwrap();
    let record = SessionRecord {
        id: id.clone(),
        revision: SessionRevision(1),
        next_turn_sequence: 1,
        messages: Vec::new(),
        metadata: metadata.clone(),
    };
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
            id.clone(),
            record,
        )])))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(
            EngineLimits::default(),
            "session_metadata",
            metadata_size,
        ))
        .build()
        .unwrap();
    assert_eq!(
        futures_executor::block_on(engine.load_session(id))
            .unwrap()
            .unwrap()
            .record()
            .metadata,
        metadata
    );

    let id = SessionId::new("metadata-over").unwrap();
    let record = SessionRecord {
        id: id.clone(),
        revision: SessionRevision(1),
        next_turn_sequence: 1,
        messages: Vec::new(),
        metadata: metadata.clone(),
    };
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
            id.clone(),
            record,
        )])))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(
            EngineLimits::default(),
            "session_metadata",
            metadata_size - 1,
        ))
        .build()
        .unwrap();
    assert!(matches!(
        futures_executor::block_on(engine.load_session(id.clone())),
        Err(EngineError::Protocol(message)) if message.contains("metadata")
    ));
    assert!(engine.create_session(id).record().metadata.is_empty());

    let options = InferenceOptions {
        model: Some("model-with-a-name".to_owned()),
        max_output_tokens: Some(42),
        temperature: Some(0.5),
        metadata: BTreeMap::from([("nested".to_owned(), json!({"secret": [1, 2, 3]}))]),
    };
    let options_size = serde_json::to_vec(&options).unwrap().len();
    let provider = ScriptedModelProvider::new(
        "options-boundary",
        [events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(
            EngineLimits::default(),
            "inference_options",
            options_size,
        ))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("options-boundary").unwrap());
    let output = futures_executor::block_on(async {
        session
            .prompt(Prompt {
                text: "go".to_owned(),
                options: options.clone(),
            })
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    });
    assert!(matches!(
        output.last(),
        Some(Ok(EngineEvent {
            payload: TurnEvent::Completed { .. },
            ..
        }))
    ));
    assert_eq!(provider.requests()[0].request.options, options);

    let provider = ScriptedModelProvider::new("options-over", []);
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(
            EngineLimits::default(),
            "inference_options",
            options_size - 1,
        ))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("options-over").unwrap());
    assert!(matches!(
        futures_executor::block_on(session.prompt(Prompt {
            text: "go".to_owned(),
            options,
        })),
        Err(EngineError::Protocol(message)) if message.contains("inference options")
    ));
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());

    let tool_spec = spec("catalog-boundary");
    let catalog_size = serde_json::to_vec(&vec![tool_spec.clone()]).unwrap().len();
    Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .tool(ScriptedTool::new(tool_spec.clone(), []))
        .limits(with_limit(
            EngineLimits::default(),
            "tool_catalog",
            catalog_size,
        ))
        .build()
        .unwrap();
    assert!(matches!(
        Engine::builder()
            .provider(ScriptedModelProvider::new("unused", []))
            .session_store(InMemorySessionStore::new())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .tool(ScriptedTool::new(tool_spec, []))
            .limits(with_limit(
                EngineLimits::default(),
                "tool_catalog",
                catalog_size - 1,
            ))
            .build(),
        Err(BuildError::ToolCatalogTooLarge)
    ));
}

#[derive(Debug)]
struct OneShotSpecTool {
    spec: Mutex<Option<ToolSpec>>,
}

impl OneShotSpecTool {
    fn new(spec: ToolSpec) -> Self {
        Self {
            spec: Mutex::new(Some(spec)),
        }
    }
}

impl Tool for OneShotSpecTool {
    fn spec(&self) -> ToolSpec {
        self.spec
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .expect("one-shot tool spec was requested more than once")
    }

    fn execute(
        &self,
        _context: ToolContext,
        _arguments: Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async {
            Err(ToolError::new(
                ToolErrorKind::Other,
                "unused_one_shot_tool",
                "one-shot schema fixture cannot execute",
                false,
            ))
        })
    }
}

fn one_shot_schema_tool(name: &str, schema: Value) -> OneShotSpecTool {
    OneShotSpecTool::new(ToolSpec {
        name: tool_name(name),
        description: "one-shot schema fixture".to_owned(),
        input_schema: schema,
    })
}

#[derive(Clone, Debug)]
struct OneShotLoadStore {
    record: Arc<Mutex<Option<SessionRecord>>>,
}

impl OneShotLoadStore {
    fn new(record: SessionRecord) -> Self {
        Self {
            record: Arc::new(Mutex::new(Some(record))),
        }
    }
}

impl SessionStore for OneShotLoadStore {
    fn load(
        &self,
        _id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        let record = self
            .record
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Box::pin(async move { Ok(record) })
    }

    fn save(
        &self,
        _record: SessionRecord,
        _expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async {
            Err(SessionStoreError::new(
                SessionStoreErrorKind::Other,
                "unused_one_shot_store",
                "one-shot load fixture cannot save",
                false,
            ))
        })
    }
}

#[derive(Debug)]
struct CancelReadyDeepProvider;

impl ModelProvider for CancelReadyDeepProvider {
    fn name(&self) -> &'static str {
        "cancel-ready-deep"
    }

    fn stream(
        &self,
        _request: ModelRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        let mut event = Some(ModelEvent::ToolCall {
            call: call("cancel-ready-deep", "known", nested_array(DEEP_JSON_DEPTH)),
        });
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::poll_fn(move |_| {
                cancellation.cancel();
                Poll::Ready(event.take().map(Ok))
            })) as ModelEventStream)
        })
    }
}

#[derive(Debug)]
struct CancelReadyDeepTool;

impl Tool for CancelReadyDeepTool {
    fn spec(&self) -> ToolSpec {
        spec("cancel-ready-deep")
    }

    fn execute(
        &self,
        _context: ToolContext,
        _arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        let mut output = Some(ToolOutput::success(nested_array(DEEP_JSON_DEPTH)));
        Box::pin(std::future::poll_fn(move |_| {
            cancellation.cancel();
            Poll::Ready(Ok(output.take().expect("tool output polled after ready")))
        }))
    }
}

#[test]
fn tool_catalog_json_depth_limit_enforces_exact_boundary() {
    let mut exact_spec = spec("schema-exact");
    exact_spec.input_schema = nested_array(2);
    Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .tool(ScriptedTool::new(exact_spec, []))
        .limits(with_limit(EngineLimits::default(), "json_depth", 2))
        .build()
        .unwrap();

    let mut over_spec = spec("schema-over");
    over_spec.input_schema = nested_array(3);
    assert!(matches!(
        Engine::builder()
            .provider(ScriptedModelProvider::new("unused", []))
            .session_store(InMemorySessionStore::new())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .tool(ScriptedTool::new(over_spec, []))
            .limits(with_limit(EngineLimits::default(), "json_depth", 2))
            .build(),
        Err(BuildError::ToolCatalogJsonDepthExceeded)
    ));
}

#[test]
fn inference_options_json_depth_fails_before_persistence_or_provider() {
    let exact_options = InferenceOptions {
        metadata: BTreeMap::from([("nested".to_owned(), nested_array(2))]),
        ..InferenceOptions::default()
    };
    let provider = ScriptedModelProvider::new(
        "options-depth-exact",
        [events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(EngineLimits::default(), "json_depth", 2))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("options-depth-exact").unwrap());
    let observed = futures_executor::block_on(async {
        session
            .prompt(Prompt {
                text: "go".to_owned(),
                options: exact_options,
            })
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    });
    assert!(matches!(
        observed.last(),
        Some(Ok(EngineEvent {
            payload: TurnEvent::Completed { .. },
            ..
        }))
    ));
    assert_eq!(provider.requests().len(), 1);

    let provider = ScriptedModelProvider::new("options-depth-over", []);
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(EngineLimits::default(), "json_depth", 2))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("options-depth-over").unwrap());
    assert!(matches!(
        futures_executor::block_on(session.prompt(Prompt {
            text: "go".to_owned(),
            options: InferenceOptions {
                metadata: BTreeMap::from([("nested".to_owned(), nested_array(3))]),
                ..InferenceOptions::default()
            },
        })),
        Err(EngineError::Protocol(message)) if message.contains("depth limit")
    ));
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
}

#[test]
fn loaded_record_json_depth_checks_metadata_and_transcript_before_publication() {
    let id = SessionId::new("loaded-depth-exact").unwrap();
    let exact = SessionRecord {
        id: id.clone(),
        revision: SessionRevision(1),
        next_turn_sequence: 1,
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Json {
                value: nested_array(2),
            }],
        }],
        metadata: BTreeMap::from([("nested".to_owned(), nested_array(2))]),
    };
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
            id.clone(),
            exact.clone(),
        )])))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(with_limit(EngineLimits::default(), "json_depth", 2))
        .build()
        .unwrap();
    assert_eq!(
        futures_executor::block_on(engine.load_session(id))
            .unwrap()
            .unwrap()
            .record(),
        exact
    );

    for (id_text, metadata, value) in [
        (
            "loaded-metadata-depth-over",
            BTreeMap::from([("nested".to_owned(), nested_array(3))]),
            json!(null),
        ),
        (
            "loaded-transcript-depth-over",
            BTreeMap::new(),
            nested_array(3),
        ),
    ] {
        let id = SessionId::new(id_text).unwrap();
        let record = SessionRecord {
            id: id.clone(),
            revision: SessionRevision(1),
            next_turn_sequence: 1,
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Json { value }],
            }],
            metadata,
        };
        let engine = Engine::builder()
            .provider(ScriptedModelProvider::new("unused", []))
            .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
                id.clone(),
                record,
            )])))
            .permission_handler(ScriptedPermissionHandler::new([]))
            .limits(with_limit(EngineLimits::default(), "json_depth", 2))
            .build()
            .unwrap();
        assert!(matches!(
            futures_executor::block_on(engine.load_session(id.clone())),
            Err(EngineError::Protocol(message)) if message.contains("depth limit")
        ));
        assert!(engine.create_session(id).record().messages.is_empty());
    }
}

#[test]
fn provider_argument_json_depth_fails_before_authorization_or_tool_execution() {
    let exact_provider = ScriptedModelProvider::new(
        "argument-depth-exact",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("exact", "known", nested_array(2)),
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
    let exact_permissions = ScriptedPermissionHandler::new([allow()]);
    let exact_tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success("done"))],
    );
    let engine = Engine::builder()
        .provider(exact_provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(exact_permissions.clone())
        .tool(exact_tool.clone())
        .limits(with_limit(EngineLimits::default(), "json_depth", 2))
        .build()
        .unwrap();
    let observed = collect(&engine.create_session(SessionId::new("argument-depth-exact").unwrap()));
    assert!(matches!(
        observed.last().unwrap().payload,
        TurnEvent::Completed { .. }
    ));
    assert_eq!(exact_permissions.requests().len(), 1);
    assert_eq!(exact_tool.invocations().len(), 1);

    let over_provider = ScriptedModelProvider::new(
        "argument-depth-over",
        [events([
            ModelEvent::ToolCall {
                call: call("over", "known", nested_array(3)),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let over_permissions = ScriptedPermissionHandler::new([]);
    let over_tool = ScriptedTool::new(spec("known"), []);
    let engine = Engine::builder()
        .provider(over_provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(over_permissions.clone())
        .tool(over_tool.clone())
        .limits(with_limit(EngineLimits::default(), "json_depth", 2))
        .build()
        .unwrap();
    let observed = collect(&engine.create_session(SessionId::new("argument-depth-over").unwrap()));
    assert!(matches!(
        &observed.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "json_depth_limit"
    ));
    assert!(over_permissions.requests().is_empty());
    assert!(over_tool.invocations().is_empty());
}

#[test]
fn tool_output_json_depth_exact_boundary_and_unknown_overflow_marker() {
    let exact_provider = ScriptedModelProvider::new(
        "output-depth-exact",
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
    let exact_tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success(nested_array(2)))],
    );
    let engine = Engine::builder()
        .provider(exact_provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(exact_tool.clone())
        .limits(with_limit(EngineLimits::default(), "json_depth", 2))
        .build()
        .unwrap();
    let observed = collect(&engine.create_session(SessionId::new("output-depth-exact").unwrap()));
    assert!(matches!(
        observed.last().unwrap().payload,
        TurnEvent::Completed { .. }
    ));
    assert_eq!(exact_tool.invocations().len(), 1);

    let over_provider = ScriptedModelProvider::new(
        "output-depth-over",
        [events([
            ModelEvent::ToolCall {
                call: call("over", "known", json!({})),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let over_tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success(nested_array(3)))],
    );
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(over_provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(over_tool.clone())
        .limits(with_limit(EngineLimits::default(), "json_depth", 2))
        .build()
        .unwrap();
    let id = SessionId::new("output-depth-over").unwrap();
    let observed = collect(&engine.create_session(id.clone()));
    assert!(matches!(
        &observed.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "json_depth_limit"
    ));
    assert_eq!(over_tool.invocations().len(), 1);
    let durable = store.record(&id).unwrap();
    assert_eq!(durable.messages.len(), 3);
    assert_unknown_result(&durable.messages[2], "over");
}

#[test]
fn tool_catalog_json_node_budget_is_aggregate_and_enforces_plus_one() {
    let limits = with_limit(EngineLimits::default(), "json_nodes", 2);
    Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .tool(one_shot_schema_tool("one", Value::Null))
        .tool(one_shot_schema_tool("two", Value::Null))
        .limits(limits)
        .build()
        .unwrap();

    assert!(matches!(
        Engine::builder()
            .provider(ScriptedModelProvider::new("unused", []))
            .session_store(InMemorySessionStore::new())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .tool(one_shot_schema_tool("one", Value::Null))
            .tool(one_shot_schema_tool("two", Value::Null))
            .tool(one_shot_schema_tool("three", Value::Null))
            .limits(limits)
            .build(),
        Err(BuildError::ToolCatalogJsonNodeLimitExceeded)
    ));
}

#[test]
fn inference_metadata_json_node_budget_enforces_exact_and_plus_one_before_effects() {
    let limits = with_limit(EngineLimits::default(), "json_nodes", 2);
    let provider = ScriptedModelProvider::new(
        "options-nodes-exact",
        [events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(limits)
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("options-nodes-exact").unwrap());
    let output = futures_executor::block_on(async {
        session
            .prompt(Prompt {
                text: "go".to_owned(),
                options: InferenceOptions {
                    metadata: BTreeMap::from([
                        ("one".to_owned(), Value::Null),
                        ("two".to_owned(), Value::Null),
                    ]),
                    ..InferenceOptions::default()
                },
            })
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    });
    assert!(matches!(
        output.last(),
        Some(Ok(EngineEvent {
            payload: TurnEvent::Completed { .. },
            ..
        }))
    ));
    assert_eq!(provider.requests().len(), 1);

    let provider = ScriptedModelProvider::new("options-nodes-over", []);
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(limits)
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("options-nodes-over").unwrap());
    assert!(matches!(
        futures_executor::block_on(session.prompt(Prompt {
            text: "go".to_owned(),
            options: InferenceOptions {
                metadata: BTreeMap::from([
                    ("one".to_owned(), Value::Null),
                    ("two".to_owned(), Value::Null),
                    ("three".to_owned(), Value::Null),
                ]),
                ..InferenceOptions::default()
            },
        })),
        Err(EngineError::Protocol(message)) if message.contains("node limit")
    ));
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
}

#[test]
fn loaded_record_json_node_budget_is_aggregate_across_metadata_and_messages() {
    let limits = with_limit(EngineLimits::default(), "json_nodes", 2);
    let id = SessionId::new("record-nodes-exact").unwrap();
    let exact = SessionRecord {
        id: id.clone(),
        revision: SessionRevision(1),
        next_turn_sequence: 1,
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Json { value: Value::Null }],
        }],
        metadata: BTreeMap::from([("one".to_owned(), Value::Null)]),
    };
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
            id.clone(),
            exact,
        )])))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(limits)
        .build()
        .unwrap();
    assert!(
        futures_executor::block_on(engine.load_session(id))
            .unwrap()
            .is_some()
    );

    let id = SessionId::new("record-nodes-over").unwrap();
    let over = SessionRecord {
        id: id.clone(),
        revision: SessionRevision(1),
        next_turn_sequence: 1,
        messages: vec![Message {
            role: Role::User,
            content: vec![
                ContentBlock::Json { value: Value::Null },
                ContentBlock::ToolCall {
                    call: call("three", "known", Value::Null),
                },
            ],
        }],
        metadata: BTreeMap::from([("one".to_owned(), Value::Null)]),
    };
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
            id.clone(),
            over,
        )])))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(limits)
        .build()
        .unwrap();
    assert!(matches!(
        futures_executor::block_on(engine.load_session(id.clone())),
        Err(EngineError::Protocol(message)) if message.contains("node limit")
    ));
    assert!(engine.create_session(id).record().messages.is_empty());
}

#[test]
fn provider_argument_json_node_limit_precedes_authorization_and_execution() {
    let limits = with_limit(EngineLimits::default(), "json_nodes", 7);
    let provider = ScriptedModelProvider::new(
        "argument-nodes-exact-record",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("exact", "known", flat_array_nodes(4)),
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
    let permissions = ScriptedPermissionHandler::new([allow()]);
    let tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success(Value::Null))],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(permissions.clone())
        .tool(tool.clone())
        .limits(limits)
        .build()
        .unwrap();
    let output =
        collect(&engine.create_session(SessionId::new("argument-nodes-exact-record").unwrap()));
    assert!(matches!(
        output.last().unwrap().payload,
        TurnEvent::Completed { .. }
    ));
    assert_eq!(permissions.requests().len(), 1);
    assert_eq!(tool.invocations().len(), 1);

    let permissions = ScriptedPermissionHandler::new([]);
    let tool = ScriptedTool::new(spec("known"), []);
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new(
            "argument-nodes-over",
            [events([
                ModelEvent::ToolCall {
                    call: call("over", "known", flat_array_nodes(8)),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ])],
        ))
        .session_store(InMemorySessionStore::new())
        .permission_handler(permissions.clone())
        .tool(tool.clone())
        .limits(limits)
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("argument-nodes-over").unwrap()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "json_node_limit"
    ));
    assert!(permissions.requests().is_empty());
    assert!(tool.invocations().is_empty());
}

#[test]
fn tool_output_json_node_limit_keeps_post_effect_placeholder() {
    let limits = with_limit(EngineLimits::default(), "json_nodes", 7);
    let provider = ScriptedModelProvider::new(
        "output-nodes-exact-record",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("exact", "known", Value::Null),
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
    let tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success(flat_array_nodes(6)))],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(tool.clone())
        .limits(limits)
        .build()
        .unwrap();
    let output =
        collect(&engine.create_session(SessionId::new("output-nodes-exact-record").unwrap()));
    assert!(matches!(
        output.last().unwrap().payload,
        TurnEvent::Completed { .. }
    ));
    assert_eq!(tool.invocations().len(), 1);

    let provider = ScriptedModelProvider::new(
        "output-nodes-over",
        [events([
            ModelEvent::ToolCall {
                call: call("over", "known", Value::Null),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success(flat_array_nodes(8)))],
    );
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(tool.clone())
        .limits(limits)
        .build()
        .unwrap();
    let id = SessionId::new("output-nodes-over").unwrap();
    let output = collect(&engine.create_session(id.clone()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "json_node_limit"
    ));
    assert_eq!(tool.invocations().len(), 1);
    assert_unknown_result(&store.record(&id).unwrap().messages[2], "over");
}

const DEEP_JSON_CASE_ENV: &str = "MACHINE_GOD_DEEP_JSON_CASE";
const DEEP_JSON_DEPTH: usize = 50_000;

#[test]
fn deep_owned_json_rejections_are_stack_safe_in_subprocesses() {
    for case in [
        "builder_abandon",
        "builder_duplicate",
        "catalog_build",
        "unpolled_prompt",
        "prompt_options",
        "loaded_metadata",
        "loaded_transcript",
        "provider_arguments",
        "provider_cancel_ready",
        "tool_output",
        "tool_cancel_ready",
    ] {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "deep_owned_json_rejection_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(DEEP_JSON_CASE_ENV, case)
            .status()
            .unwrap();
        assert!(status.success(), "deep JSON subprocess failed: {case}");
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn deep_owned_json_rejection_child() {
    let Ok(case) = std::env::var(DEEP_JSON_CASE_ENV) else {
        return;
    };

    match case.as_str() {
        "builder_abandon" => {
            let builder = Engine::builder().tool(one_shot_schema_tool(
                "deep-abandon",
                nested_array(DEEP_JSON_DEPTH),
            ));
            drop(builder);
        }
        "builder_duplicate" => {
            let builder = Engine::builder()
                .tool(one_shot_schema_tool(
                    "deep-duplicate",
                    nested_array(DEEP_JSON_DEPTH),
                ))
                .tool(ScriptedTool::new(spec("deep-duplicate"), []));
            drop(builder);
        }
        "catalog_build" => {
            let result = Engine::builder()
                .provider(ScriptedModelProvider::new("unused", []))
                .session_store(InMemorySessionStore::new())
                .permission_handler(ScriptedPermissionHandler::new([]))
                .tool(one_shot_schema_tool(
                    "deep-catalog",
                    nested_array(DEEP_JSON_DEPTH),
                ))
                .build();
            assert!(matches!(
                result,
                Err(BuildError::ToolCatalogJsonDepthExceeded)
            ));
        }
        "unpolled_prompt" => {
            let engine = Engine::builder()
                .provider(ScriptedModelProvider::new("unused", []))
                .session_store(InMemorySessionStore::new())
                .permission_handler(ScriptedPermissionHandler::new([]))
                .build()
                .unwrap();
            let session = engine.create_session(SessionId::new("deep-unpolled").unwrap());
            let future = session.prompt(Prompt {
                text: "go".to_owned(),
                options: InferenceOptions {
                    metadata: BTreeMap::from([("deep".to_owned(), nested_array(DEEP_JSON_DEPTH))]),
                    ..InferenceOptions::default()
                },
            });
            drop(future);
        }
        "prompt_options" => {
            let provider = ScriptedModelProvider::new("unused", []);
            let store = InMemorySessionStore::new();
            let engine = Engine::builder()
                .provider(provider.clone())
                .session_store(store.clone())
                .permission_handler(ScriptedPermissionHandler::new([]))
                .build()
                .unwrap();
            let session = engine.create_session(SessionId::new("deep-options").unwrap());
            let result = futures_executor::block_on(session.prompt(Prompt {
                text: "go".to_owned(),
                options: InferenceOptions {
                    metadata: BTreeMap::from([("deep".to_owned(), nested_array(DEEP_JSON_DEPTH))]),
                    ..InferenceOptions::default()
                },
            }));
            assert!(matches!(
                result,
                Err(EngineError::Protocol(message)) if message.contains("depth limit")
            ));
            assert!(provider.requests().is_empty());
            assert!(store.calls().is_empty());
        }
        "loaded_metadata" | "loaded_transcript" => {
            let id = SessionId::new(case.as_str()).unwrap();
            let (metadata, messages) = if case == "loaded_metadata" {
                (
                    BTreeMap::from([("deep".to_owned(), nested_array(DEEP_JSON_DEPTH))]),
                    Vec::new(),
                )
            } else {
                (
                    BTreeMap::new(),
                    vec![Message {
                        role: Role::User,
                        content: vec![ContentBlock::Json {
                            value: nested_array(DEEP_JSON_DEPTH),
                        }],
                    }],
                )
            };
            let store = OneShotLoadStore::new(SessionRecord {
                id: id.clone(),
                revision: SessionRevision(1),
                next_turn_sequence: 1,
                messages,
                metadata,
            });
            let engine = Engine::builder()
                .provider(ScriptedModelProvider::new("unused", []))
                .session_store(store)
                .permission_handler(ScriptedPermissionHandler::new([]))
                .build()
                .unwrap();
            assert!(matches!(
                futures_executor::block_on(engine.load_session(id.clone())),
                Err(EngineError::Protocol(message)) if message.contains("depth limit")
            ));
            assert!(engine.create_session(id).record().messages.is_empty());
        }
        "provider_arguments" => {
            let permissions = ScriptedPermissionHandler::new([]);
            let tool = ScriptedTool::new(spec("known"), []);
            let engine = Engine::builder()
                .provider(ScriptedModelProvider::new(
                    "deep-provider-arguments",
                    [events([
                        ModelEvent::ToolCall {
                            call: call("deep", "known", nested_array(DEEP_JSON_DEPTH)),
                        },
                        ModelEvent::Stop {
                            reason: StopReason::ToolCalls,
                        },
                    ])],
                ))
                .session_store(InMemorySessionStore::new())
                .permission_handler(permissions.clone())
                .tool(tool.clone())
                .build()
                .unwrap();
            let output =
                collect(&engine.create_session(SessionId::new("deep-provider-arguments").unwrap()));
            assert!(matches!(
                &output.last().unwrap().payload,
                TurnEvent::Failed { code, .. } if code == "json_depth_limit"
            ));
            assert!(permissions.requests().is_empty());
            assert!(tool.invocations().is_empty());
        }
        "provider_cancel_ready" => {
            let permissions = ScriptedPermissionHandler::new([]);
            let engine = Engine::builder()
                .provider(CancelReadyDeepProvider)
                .session_store(InMemorySessionStore::new())
                .permission_handler(permissions.clone())
                .tool(ScriptedTool::new(spec("known"), []))
                .build()
                .unwrap();
            let output = collect(
                &engine.create_session(SessionId::new("deep-provider-cancel-ready").unwrap()),
            );
            assert!(matches!(
                &output.last().unwrap().payload,
                TurnEvent::Completed {
                    reason: StopReason::Cancelled,
                    ..
                }
            ));
            assert!(permissions.requests().is_empty());
        }
        "tool_output" => {
            let store = InMemorySessionStore::new();
            let tool = ScriptedTool::new(
                spec("known"),
                [ToolStep::Output(ToolOutput::success(nested_array(
                    DEEP_JSON_DEPTH,
                )))],
            );
            let engine = Engine::builder()
                .provider(ScriptedModelProvider::new(
                    "deep-tool-output",
                    [events([
                        ModelEvent::ToolCall {
                            call: call("deep", "known", Value::Null),
                        },
                        ModelEvent::Stop {
                            reason: StopReason::ToolCalls,
                        },
                    ])],
                ))
                .session_store(store.clone())
                .permission_handler(ScriptedPermissionHandler::new([allow()]))
                .tool(tool.clone())
                .build()
                .unwrap();
            let id = SessionId::new("deep-tool-output").unwrap();
            let output = collect(&engine.create_session(id.clone()));
            assert!(matches!(
                &output.last().unwrap().payload,
                TurnEvent::Failed { code, .. } if code == "json_depth_limit"
            ));
            assert_eq!(tool.invocations().len(), 1);
            assert_unknown_result(&store.record(&id).unwrap().messages[2], "deep");
        }
        "tool_cancel_ready" => {
            let store = InMemorySessionStore::new();
            let engine = Engine::builder()
                .provider(ScriptedModelProvider::new(
                    "deep-tool-cancel-ready",
                    [events([
                        ModelEvent::ToolCall {
                            call: call("deep", "cancel-ready-deep", Value::Null),
                        },
                        ModelEvent::Stop {
                            reason: StopReason::ToolCalls,
                        },
                    ])],
                ))
                .session_store(store.clone())
                .permission_handler(ScriptedPermissionHandler::new([allow()]))
                .tool(CancelReadyDeepTool)
                .build()
                .unwrap();
            let id = SessionId::new("deep-tool-cancel-ready").unwrap();
            let output = collect(&engine.create_session(id.clone()));
            assert!(matches!(
                &output.last().unwrap().payload,
                TurnEvent::Completed {
                    reason: StopReason::Cancelled,
                    ..
                }
            ));
            assert_unknown_result(&store.record(&id).unwrap().messages[2], "deep");
        }
        other => panic!("unknown deep JSON subprocess case: {other}"),
    }
}

#[test]
fn placeholder_budgets_fail_before_committing_calls_or_requesting_permission() {
    let provider = ScriptedModelProvider::new(
        "placeholder-budget",
        [events([
            ModelEvent::ToolCall {
                call: call("call", "known", json!({})),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let permissions = ScriptedPermissionHandler::new([]);
    let store = InMemorySessionStore::new();
    let tool = ScriptedTool::new(spec("known"), []);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(permissions.clone())
        .tool(tool.clone())
        .limits(with_limit(EngineLimits::default(), "result", 1))
        .build()
        .unwrap();
    let id = SessionId::new("placeholder-budget").unwrap();
    let output = collect(&engine.create_session(id.clone()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "tool_result_size_limit"
    ));
    assert_eq!(store.record(&id).unwrap().messages.len(), 1);
    assert!(permissions.requests().is_empty());
    assert!(tool.invocations().is_empty());
}

#[test]
fn denial_reason_is_host_only_and_permission_ids_include_turn_identity() {
    let secret = "éééSENTINEL_POLICY_SECRET";
    let provider = ScriptedModelProvider::new(
        "permission-identity",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("first-call", "known", json!({})),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
            events([
                ModelEvent::ToolCall {
                    call: call("second-call", "known", json!({})),
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
    let permissions = ScriptedPermissionHandler::new([
        PermissionStep::Decision(PermissionDecision::Deny {
            reason: secret.to_owned(),
        }),
        PermissionStep::Decision(PermissionDecision::Deny {
            reason: "second denial".to_owned(),
        }),
    ]);
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(permissions.clone())
        .tool(ScriptedTool::new(spec("known"), []))
        .limits(with_limit(EngineLimits::default(), "denial_reason", 5))
        .build()
        .unwrap();
    let id = SessionId::new("permission-identity").unwrap();
    let session = engine.create_session(id.clone());
    let first_events = collect(&session);
    let _second_events = collect(&session);

    let ids = permissions
        .requests()
        .into_iter()
        .map(|request| request.id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert!(ids.iter().all(|id| {
        id.len() == 82
            && id.starts_with("permission-sha256-")
            && id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    }));
    assert!(first_events.iter().any(|event| matches!(
        &event.payload,
        TurnEvent::PermissionResolved {
            decision: PermissionDecision::Deny { reason },
            ..
        } if reason == "éé"
    )));
    assert!(
        !serde_json::to_string(&store.record(&id).unwrap())
            .unwrap()
            .contains(secret)
    );
    assert!(
        !serde_json::to_string(&store.record(&id).unwrap())
            .unwrap()
            .contains("SENTINEL_POLICY_SECRET")
    );
    assert!(
        !serde_json::to_string(&provider.requests()[1].request.messages)
            .unwrap()
            .contains(secret)
    );
    assert!(
        !serde_json::to_string(&provider.requests()[1].request.messages)
            .unwrap()
            .contains("SENTINEL_POLICY_SECRET")
    );
}

#[test]
fn permission_request_ids_are_distinct_across_sessions() {
    let provider = ScriptedModelProvider::new(
        "session-scoped-permission-ids",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("first", "known", json!({})),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
            events([
                ModelEvent::ToolCall {
                    call: call("second", "known", json!({})),
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
    let permissions = ScriptedPermissionHandler::new([
        PermissionStep::Decision(PermissionDecision::Deny {
            reason: "first".to_owned(),
        }),
        PermissionStep::Decision(PermissionDecision::Deny {
            reason: "second".to_owned(),
        }),
    ]);
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(permissions.clone())
        .tool(ScriptedTool::new(spec("known"), []))
        .build()
        .unwrap();
    collect(&engine.create_session(SessionId::new("permission-session-one").unwrap()));
    collect(&engine.create_session(SessionId::new("permission-session-two").unwrap()));

    let requests = permissions.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].turn_id, requests[1].turn_id);
    assert_ne!(requests[0].session_id, requests[1].session_id);
    assert_eq!(
        requests[0].id.as_str(),
        "permission-sha256-e0587c0d089e3b55553560e7d014b8684db1d83eb9d8e3f19ccb635e7e3cc5ca"
    );
    assert_ne!(requests[0].id, requests[1].id);
}

#[derive(Clone, Debug, Default)]
struct IdCachingPermissionHandler {
    state: Arc<Mutex<IdCachingPermissionState>>,
}

#[derive(Debug, Default)]
struct IdCachingPermissionState {
    allowed_id: Option<String>,
    requests: Vec<PermissionRequest>,
}

impl IdCachingPermissionHandler {
    fn requests(&self) -> Vec<PermissionRequest> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .requests
            .clone()
    }
}

impl PermissionHandler for IdCachingPermissionHandler {
    fn authorize(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        let decision = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let request_id = request.id.to_string();
            state.requests.push(request);
            match &state.allowed_id {
                Some(allowed_id) if *allowed_id == request_id => PermissionDecision::Allow {
                    scope: PermissionGrantScope::Session,
                },
                Some(_) => PermissionDecision::Deny {
                    reason: "separate session requires a fresh decision".to_owned(),
                },
                None => {
                    state.allowed_id = Some(request_id);
                    PermissionDecision::Allow {
                        scope: PermissionGrantScope::Session,
                    }
                }
            }
        };
        Box::pin(async move { Ok(decision) })
    }
}

#[test]
fn permission_id_cache_cannot_reuse_allow_across_sessions() {
    let provider = ScriptedModelProvider::new(
        "permission-cache-isolation",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("first", "known", json!({})),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
            events([
                ModelEvent::ToolCall {
                    call: call("second", "known", json!({})),
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
    let permissions = IdCachingPermissionHandler::default();
    let tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success("first allowed"))],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(permissions.clone())
        .tool(tool.clone())
        .build()
        .unwrap();

    collect(&engine.create_session(SessionId::new("cache-session-one").unwrap()));
    collect(&engine.create_session(SessionId::new("cache-session-two").unwrap()));

    let requests = permissions.requests();
    assert_eq!(requests.len(), 2);
    assert_ne!(requests[0].id, requests[1].id);
    assert_eq!(tool.invocations().len(), 1);
}

#[test]
#[allow(clippy::too_many_lines)]
fn provider_permission_and_store_terminal_diagnostics_are_redacted() {
    let secret = "SENTINEL_COMPONENT_SECRET";
    let provider = ScriptedModelProvider::new(
        "provider-secret",
        [ModelProviderStep::StartError(ProviderError::new(
            ProviderErrorKind::Other,
            format!("provider_code\n{secret}"),
            secret,
            false,
        ))],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("provider-secret").unwrap()));
    assert!(!format!("{:?}", output.last().unwrap().payload).contains(secret));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "provider_failed"
    ));

    let provider = ScriptedModelProvider::new(
        "permission-secret",
        [events([
            ModelEvent::ToolCall {
                call: call("call", "known", json!({})),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([PermissionStep::Error(
            PermissionError::new(format!("policy_code\n{secret}"), secret),
        )]))
        .tool(ScriptedTool::new(spec("known"), []))
        .build()
        .unwrap();
    let id = SessionId::new("permission-secret").unwrap();
    let output = collect(&engine.create_session(id.clone()));
    assert!(!format!("{:?}", output.last().unwrap().payload).contains(secret));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "permission_failed"
    ));
    assert_unknown_result(&store.record(&id).unwrap().messages[2], "call");

    let provider = ScriptedModelProvider::new(
        "store-secret",
        [events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let store = InMemorySessionStore::configured(
        BTreeMap::new(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Error(SessionStoreError::new(
                    SessionStoreErrorKind::Other,
                    format!("store_code\n{secret}"),
                    secret,
                    false,
                )),
            ]),
        },
        16,
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let output = collect(&engine.create_session(SessionId::new("store-secret").unwrap()));
    assert!(!format!("{:?}", output.last().unwrap().payload).contains(secret));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { code, .. } if code == "store_failed"
    ));

    let store = InMemorySessionStore::configured(
        BTreeMap::new(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![SessionStoreStep::Error(SessionStoreError::new(
                SessionStoreErrorKind::Unavailable,
                format!("initial_store_code\n{secret}"),
                secret,
                true,
            ))]),
        },
        8,
    );
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(store)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine.create_session(SessionId::new("initial-store-secret").unwrap());
    let error = futures_executor::block_on(session.prompt("go")).unwrap_err();
    assert!(!error.to_string().contains(secret));
    assert!(matches!(
        error,
        EngineError::Store(SessionStoreError {
            code,
            kind: SessionStoreErrorKind::Unavailable,
            retryable: true,
            ..
        }) if code == "store_failed"
    ));

    let store = InMemorySessionStore::configured(
        BTreeMap::new(),
        SessionStoreScript {
            loads: Some(vec![SessionStoreStep::Error(SessionStoreError::new(
                SessionStoreErrorKind::Corrupt,
                format!("load_code\n{secret}"),
                secret,
                false,
            ))]),
            saves: None,
        },
        8,
    );
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("unused", []))
        .session_store(store)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let error = futures_executor::block_on(
        engine.load_session(SessionId::new("load-store-secret").unwrap()),
    )
    .unwrap_err();
    assert!(!error.to_string().contains(secret));
    assert!(matches!(
        error,
        EngineError::Store(SessionStoreError {
            code,
            kind: SessionStoreErrorKind::Corrupt,
            retryable: false,
            ..
        }) if code == "store_failed"
    ));
}

#[test]
fn permission_sink_and_replacement_save_failures_leave_placeholders() {
    let requested = call("call", "known", json!({}));
    let provider = ScriptedModelProvider::new(
        "sink-placeholder",
        [events([
            ModelEvent::ToolCall {
                call: requested.clone(),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let sink = RecordingEventSink::scripted([
        EventSinkStep::Accept,
        EventSinkStep::Accept,
        EventSinkStep::Accept,
        EventSinkStep::Error(machine_god_core::EventSinkError::new(
            "sink_failure",
            "expected",
        )),
    ]);
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .event_sink(sink)
        .tool(ScriptedTool::new(spec("known"), []))
        .build()
        .unwrap();
    let id = SessionId::new("sink-placeholder").unwrap();
    let session = engine.create_session(id.clone());
    let turn = futures_executor::block_on(session.prompt("go")).unwrap();
    let result = futures_executor::block_on(turn.collect::<Vec<_>>());
    assert!(matches!(
        result.last(),
        Some(Err(EngineError::EventSink(_)))
    ));
    assert_unknown_result(&store.record(&id).unwrap().messages[2], "call");

    let provider = ScriptedModelProvider::new(
        "save-placeholder",
        [events([
            ModelEvent::ToolCall { call: requested },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let store = InMemorySessionStore::configured(
        BTreeMap::new(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Error(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "replace_failed",
                    "secret replacement detail",
                    true,
                )),
            ]),
        },
        32,
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(ScriptedTool::new(
            spec("known"),
            [ToolStep::Output(ToolOutput::success("done"))],
        ))
        .build()
        .unwrap();
    let id = SessionId::new("save-placeholder").unwrap();
    let output = collect(&engine.create_session(id.clone()));
    assert!(matches!(
        &output.last().unwrap().payload,
        TurnEvent::Failed { component, .. } if component == "store"
    ));
    assert_unknown_result(&store.record(&id).unwrap().messages[2], "call");
}

#[derive(Clone, Debug)]
struct CancelBeforeReadyTool {
    invoked: Arc<AtomicBool>,
}

impl Tool for CancelBeforeReadyTool {
    fn spec(&self) -> ToolSpec {
        spec("cancel-ready")
    }

    fn execute(
        &self,
        _context: ToolContext,
        _arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        self.invoked.store(true, Ordering::Release);
        cancellation.cancel();
        Box::pin(async { Ok(ToolOutput::success("must be discarded")) })
    }
}

#[test]
fn cancellation_ready_race_keeps_placeholder_and_next_prompt_never_replays() {
    let provider = ScriptedModelProvider::new(
        "cancel-ready",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("race", "cancel-ready", json!({})),
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
    let invoked = Arc::new(AtomicBool::new(false));
    let store = InMemorySessionStore::new();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([allow()]))
        .tool(CancelBeforeReadyTool {
            invoked: Arc::clone(&invoked),
        })
        .build()
        .unwrap();
    let id = SessionId::new("cancel-ready").unwrap();
    let session = engine.create_session(id.clone());
    let first = collect(&session);
    assert!(matches!(
        first.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert!(invoked.load(Ordering::Acquire));
    assert_unknown_result(&store.record(&id).unwrap().messages[2], "race");

    let second = collect(&session);
    assert!(matches!(
        second.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
    assert_eq!(provider.requests().len(), 2);
    assert_unknown_result(&provider.requests()[1].request.messages[2], "race");
}

#[test]
#[allow(clippy::too_many_lines)]
fn multi_call_cancellation_preserves_known_prefix_and_unknown_suffix_for_resume() {
    let provider = ScriptedModelProvider::new(
        "multi-resume",
        [
            events([
                ModelEvent::ToolCall {
                    call: call("first", "known", json!({})),
                },
                ModelEvent::ToolCall {
                    call: call("second", "known", json!({})),
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
    let store = InMemorySessionStore::new();
    let tool = ScriptedTool::new(
        spec("known"),
        [ToolStep::Output(ToolOutput::success("first-result"))],
    );
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(ScriptedPermissionHandler::new([
            allow(),
            PermissionStep::Pending,
        ]))
        .tool(tool.clone())
        .build()
        .unwrap();
    let id = SessionId::new("multi-resume").unwrap();
    let session = engine.create_session(id.clone());
    let mut turn = futures_executor::block_on(session.prompt("go")).unwrap();
    let mut saw_second_request = false;
    for _ in 0..20 {
        let event = next(&mut turn);
        if matches!(
            event.payload,
            TurnEvent::PermissionRequested {
                request: machine_god_core::PermissionRequest {
                    capability: machine_god_core::Capability::Tool { ref call_id, .. },
                    ..
                }
            } if call_id.as_str() == "second"
        ) {
            saw_second_request = true;
            break;
        }
    }
    assert!(saw_second_request);
    poll_pending(&mut turn);
    assert!(turn.handle().cancel());
    assert!(matches!(
        next(&mut turn).payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    let durable = store.record(&id).unwrap();
    assert_eq!(durable.messages.len(), 4);
    let ContentBlock::ToolResult { output, .. } = &durable.messages[2].content[0] else {
        panic!("expected first result")
    };
    assert_eq!(output.content, "first-result");
    assert_unknown_result(&durable.messages[3], "second");
    assert_eq!(tool.invocations().len(), 1);

    let resumed = collect(&session);
    assert!(matches!(
        resumed.last().unwrap().payload,
        TurnEvent::Completed { .. }
    ));
    let resume_request = &provider.requests()[1].request.messages;
    assert_unknown_result(&resume_request[3], "second");
    assert_eq!(tool.invocations().len(), 1);
}
