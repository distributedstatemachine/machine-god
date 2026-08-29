#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, Engine, EngineEvent, Message,
    ModelEvent, PermissionDecision, PermissionGrantScope, PermissionRisk, PreparedToolCall, Role,
    SessionId, SessionIncarnationId, StopReason, Tool, ToolCall, ToolCallId, ToolContext,
    ToolError, ToolName, ToolOutput, ToolSpec, TurnEvent,
};
use machine_god_native::{MEMORY_TOOL_NAME, MemoryTool};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::{Value, json};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

struct CancelAfterReadyMemoryTool {
    inner: MemoryTool,
}

impl Tool for CancelAfterReadyMemoryTool {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        self.inner.prepare(call)
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        let execution = self.inner.execute(context, arguments, cancellation.clone());
        Box::pin(async move {
            let result = execution.await;
            assert!(
                cancellation.cancel(),
                "memory execution was unexpectedly cancelled before becoming ready"
            );
            result
        })
    }
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-memory-engine-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create a temporary directory: {error}"),
            }
        }
        panic!("failed to allocate a unique temporary directory");
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.path);
        if std::thread::panicking() {
            return;
        }
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove a temporary directory: {error}"),
        }
    }
}

fn memory_call(id: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new(MEMORY_TOOL_NAME).unwrap(),
        arguments,
    }
}

fn single_call_provider(arguments: Value, name: &str) -> ScriptedModelProvider {
    ScriptedModelProvider::new(
        name,
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: memory_call("memory-call", arguments),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    )
}

fn save_then_list_provider(fact: &str, name: &str) -> ScriptedModelProvider {
    ScriptedModelProvider::new(
        name,
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: memory_call("memory-save", json!({"action": "save", "fact": fact})),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: memory_call("memory-list", json!({"action": "list"})),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    )
}

fn collect(engine: &Engine, name: &str) -> (SessionId, Vec<EngineEvent>) {
    let session_id = SessionId::new(name).unwrap();
    let incarnation_id = SessionIncarnationId::new(format!("incarnation-{name}")).unwrap();
    let session = engine
        .create_session(session_id.clone(), incarnation_id)
        .unwrap();
    let events = futures_executor::block_on(async {
        session
            .prompt("apply the requested durable memory operation")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    });
    (session_id, events)
}

fn assert_completed(events: &[EngineEvent], reason: &StopReason) {
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: actual,
            ..
        }) if actual == reason
    ));
}

fn tool_result(message: &Message) -> &ToolOutput {
    assert_eq!(message.role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = message.content.as_slice() else {
        panic!("expected one durable tool result")
    };
    output
}

fn assert_exact_memory_request(policy: &ScriptedPermissionHandler, index: usize, details: Value) {
    let requests = policy.requests();
    assert_eq!(
        requests[index].capability,
        Capability::Custom {
            name: MEMORY_TOOL_NAME.to_owned(),
            details,
        }
    );
    assert_eq!(requests[index].risk, PermissionRisk::Critical);
}

fn event_position(events: &[EngineEvent], predicate: impl Fn(&TurnEvent) -> bool) -> usize {
    events
        .iter()
        .position(|event| predicate(&event.payload))
        .expect("expected engine event")
}

#[test]
fn engine_denial_uses_exact_custom_capability_and_is_effect_free() {
    let state = TemporaryDirectory::new();
    let fact = "Never persist this denied preference";
    let arguments = json!({"action": "save", "fact": fact});
    let provider = single_call_provider(arguments.clone(), "memory-denied");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Deny {
            reason: "denied by test policy".to_owned(),
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(MemoryTool::open(state.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "memory-denied");

    assert_completed(&events, &StopReason::Completed);
    assert_eq!(policy.requests().len(), 1);
    assert_exact_memory_request(&policy, 0, arguments);
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
    )));

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let result_message = requests[1].request.messages[2].clone();
    assert_eq!(
        tool_result(&result_message),
        &ToolOutput {
            content: json!({
                "code": "permission_denied",
                "message": "tool execution was denied by policy",
            }),
            is_error: true,
        }
    );
    assert_eq!(
        store.record(&session_id).unwrap().messages[2],
        result_message
    );
    assert_eq!(fs::read_dir(state.path()).unwrap().count(), 0);
}

#[test]
#[allow(clippy::too_many_lines)]
fn engine_save_then_list_orders_policy_and_durably_continues_with_visible_fact() {
    let state = TemporaryDirectory::new();
    let fact = "Prefer exact provider continuation checks";
    let save_arguments = json!({"action": "save", "fact": fact});
    let list_arguments = json!({"action": "list"});
    let provider = save_then_list_provider(fact, "memory-save-list");
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([
        PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        }),
        PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        }),
    ]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(MemoryTool::open(state.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "memory-save-list");

    assert_completed(&events, &StopReason::Completed);
    assert_eq!(policy.requests().len(), 2);
    assert_exact_memory_request(&policy, 0, save_arguments);
    assert_exact_memory_request(&policy, 1, list_arguments);

    let save_requested = event_position(&events, |event| {
        matches!(event, TurnEvent::PermissionRequested { .. })
    });
    let save_resolved = event_position(&events, |event| {
        matches!(
            event,
            TurnEvent::PermissionResolved {
                decision: PermissionDecision::Allow { .. },
                ..
            }
        )
    });
    let save_started = event_position(&events, |event| {
        matches!(
            event,
            TurnEvent::ToolStarted { call } if call.id.as_str() == "memory-save"
        )
    });
    let save_finished = event_position(&events, |event| {
        matches!(
            event,
            TurnEvent::ToolFinished { call_id, .. } if call_id.as_str() == "memory-save"
        )
    });
    let list_requested = events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            matches!(event.payload, TurnEvent::PermissionRequested { .. }).then_some(index)
        })
        .nth(1)
        .expect("expected list permission request");
    let list_resolved = events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            matches!(event.payload, TurnEvent::PermissionResolved { .. }).then_some(index)
        })
        .nth(1)
        .expect("expected list permission resolution");
    let list_started = event_position(&events, |event| {
        matches!(
            event,
            TurnEvent::ToolStarted { call } if call.id.as_str() == "memory-list"
        )
    });
    let list_finished = event_position(&events, |event| {
        matches!(
            event,
            TurnEvent::ToolFinished { call_id, .. } if call_id.as_str() == "memory-list"
        )
    });
    assert!(save_requested < save_resolved && save_resolved < save_started);
    assert!(save_started < save_finished);
    assert!(save_finished < list_requested);
    assert!(list_requested < list_resolved && list_resolved < list_started);
    assert!(list_started < list_finished);

    let expected_save = ToolOutput::success(json!({
        "action": "save",
        "stored": true,
        "count": 1,
    }));
    let expected_list = ToolOutput::success(json!({
        "action": "list",
        "memories": [fact],
        "count": 1,
    }));
    assert!(matches!(
        &events[save_finished].payload,
        TurnEvent::ToolFinished { output, .. } if output == &expected_save
    ));
    assert!(matches!(
        &events[list_finished].payload,
        TurnEvent::ToolFinished { output, .. } if output == &expected_list
    ));

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    let save_message = requests[1].request.messages[2].clone();
    assert_eq!(tool_result(&save_message), &expected_save);
    let final_request = &requests[2].request;
    assert_eq!(final_request.messages[2], save_message);
    let list_message = final_request.messages[4].clone();
    assert_eq!(tool_result(&list_message), &expected_list);

    let record = store.record(&session_id).unwrap();
    assert_eq!(record.messages[2], save_message);
    assert_eq!(record.messages[4], list_message);
}

#[test]
fn same_poll_cancellation_after_save_commit_keeps_durable_unknown_result() {
    let state = TemporaryDirectory::new();
    let fact = "Committed before same-poll cancellation";
    let arguments = json!({"action": "save", "fact": fact});
    let provider = single_call_provider(arguments.clone(), "memory-same-poll-cancellation");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(CancelAfterReadyMemoryTool {
            inner: MemoryTool::open(state.path()).unwrap(),
        })
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "memory-same-poll-cancellation");

    assert_completed(&events, &StopReason::Cancelled);
    assert_eq!(policy.requests().len(), 1);
    assert_exact_memory_request(&policy, 0, arguments);
    assert!(
        events
            .iter()
            .any(|event| matches!(event.payload, TurnEvent::ToolStarted { .. }))
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event.payload, TurnEvent::ToolFinished { .. }))
    );
    assert_eq!(provider.requests().len(), 1);

    let document: Value =
        serde_json::from_slice(&fs::read(state.path().join("memories.json")).unwrap())
            .expect("committed memory document is valid JSON");
    assert_eq!(document, json!({"schema_version": 1, "memories": [fact]}));

    let record = store.record(&session_id).unwrap();
    assert_eq!(record.messages.len(), 3);
    assert_eq!(record.messages[2].role, Role::Tool);
    assert_eq!(
        tool_result(&record.messages[2]),
        &ToolOutput {
            content: json!({
                "code": "tool_result_unknown",
                "message": "tool result status is unknown",
            }),
            is_error: true,
        }
    );
}
