#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, Engine, EngineEvent, Message,
    ModelEvent, PermissionDecision, PermissionGrantScope, PreparedToolCall, Role, SessionId,
    SessionIncarnationId, StopReason, Tool, ToolCall, ToolCallId, ToolContext, ToolError, ToolName,
    ToolOutput, ToolSpec, TurnEvent,
};
use machine_god_native::{RENAME_FILE_TOOL_NAME, RenameFileTool};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::{Value, json};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

struct CancelAfterReadyRenameTool {
    inner: RenameFileTool,
}

impl Tool for CancelAfterReadyRenameTool {
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
                "rename execution was unexpectedly cancelled before becoming ready"
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
                "mg-rename-file-engine-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create temporary directory: {error}"),
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
            Err(error) => panic!("failed to remove temporary directory: {error}"),
        }
    }
}

fn provider(old_path: &str, new_path: &str, name: &str) -> ScriptedModelProvider {
    let call = ToolCall {
        id: ToolCallId::new("rename-file-call").unwrap(),
        name: ToolName::new(RENAME_FILE_TOOL_NAME).unwrap(),
        arguments: json!({"old_path": old_path, "new_path": new_path}),
    };
    ScriptedModelProvider::new(
        name,
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall { call },
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
            .prompt("rename the requested workspace file")
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

fn assert_completed(events: &[EngineEvent]) {
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));
}

fn second_request_tool_output(provider: &ScriptedModelProvider) -> (Message, ToolOutput) {
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request.tools.len(), 1);
    assert_eq!(
        requests[0].request.tools[0].name.as_str(),
        RENAME_FILE_TOOL_NAME
    );
    let message = requests[1].request.messages[2].clone();
    assert_eq!(message.role, Role::Tool);
    let ContentBlock::ToolResult { output, .. } = &message.content[0] else {
        panic!("expected a durable tool result")
    };
    (message.clone(), output.clone())
}

fn assert_exact_capability(policy: &ScriptedPermissionHandler, old_path: &str, new_path: &str) {
    let requests = policy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].capability,
        Capability::FilesystemRename {
            old_path: old_path.to_owned(),
            new_path: new_path.to_owned(),
        }
    );
}

fn assert_no_tool_execution_events(events: &[EngineEvent]) {
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
    )));
}

#[test]
fn engine_denial_uses_normalized_two_endpoint_capability_without_filesystem_effect() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.path().join("source");
    fs::write(&source, b"private bytes").unwrap();
    let provider = provider("./source", "./destination", "native-rename-denied");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Deny {
            reason: "denied by test policy".to_owned(),
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(RenameFileTool::open(temporary.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-rename-denied");

    assert_completed(&events);
    assert_exact_capability(&policy, "source", "destination");
    assert_no_tool_execution_events(&events);
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(
        output,
        ToolOutput {
            content: json!({
                "code": "permission_denied",
                "message": "tool execution was denied by policy",
            }),
            is_error: true,
        }
    );
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    assert_eq!(fs::read(source).unwrap(), b"private bytes");
    assert!(!temporary.path().join("destination").exists());
}

#[test]
fn engine_allow_resolves_policy_before_events_and_durably_returns_exact_result() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("old-parent")).unwrap();
    fs::create_dir(temporary.path().join("new-parent")).unwrap();
    let source = temporary.path().join("old-parent/source");
    fs::write(&source, b"rename me").unwrap();
    let provider = provider(
        "./old-parent//source",
        "./new-parent//destination",
        "native-rename-allowed",
    );
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(RenameFileTool::open(temporary.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-rename-allowed");

    assert_completed(&events);
    assert_exact_capability(&policy, "old-parent/source", "new-parent/destination");
    let resolved = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::PermissionResolved { .. }))
        .unwrap();
    let started = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolStarted { .. }))
        .unwrap();
    let finished = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
        .unwrap();
    assert!(resolved < started && started < finished);
    let expected = ToolOutput::success(json!({
        "old_path": "old-parent/source",
        "new_path": "new-parent/destination"
    }));
    assert!(matches!(
        &events[finished].payload,
        TurnEvent::ToolFinished { output, .. } if output == &expected
    ));
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(output, expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    assert!(!source.exists());
    assert_eq!(
        fs::read(temporary.path().join("new-parent/destination")).unwrap(),
        b"rename me"
    );
}

#[test]
fn engine_invalid_preflight_skips_policy_filesystem_and_tool_events() {
    let workspace = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    let outside_target = outside.path().join("outside");
    fs::write(&outside_target, b"outside sentinel").unwrap();
    let outside_name = outside.path().file_name().unwrap().to_string_lossy();
    let provider = provider(
        &format!("../{outside_name}/outside"),
        "destination",
        "native-rename-preflight-invalid",
    );
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(RenameFileTool::open(workspace.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-rename-preflight-invalid");

    assert_completed(&events);
    assert!(policy.requests().is_empty());
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::PermissionRequested { .. }
            | TurnEvent::PermissionResolved { .. }
            | TurnEvent::ToolStarted { .. }
            | TurnEvent::ToolFinished { .. }
    )));
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(
        output.content["code"],
        Value::String("tool_error".to_owned())
    );
    assert!(output.is_error);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    assert_eq!(fs::read(&outside_target).unwrap(), b"outside sentinel");
}

#[test]
fn engine_allowed_execution_error_is_generic_nonretryable_and_durable() {
    let temporary = TemporaryDirectory::new();
    let provider = provider(
        "missing/source",
        "destination",
        "native-rename-execution-error",
    );
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(RenameFileTool::open(temporary.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-rename-execution-error");

    assert_completed(&events);
    assert_exact_capability(&policy, "missing/source", "destination");
    let expected = ToolOutput {
        content: json!({
            "code": "tool_error",
            "message": "tool execution failed",
            "retryable": false
        }),
        is_error: true,
    };
    assert!(events.iter().any(|event| matches!(
        &event.payload,
        TurnEvent::ToolFinished { output, .. } if output == &expected
    )));
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(output, expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    assert!(!temporary.path().join("destination").exists());
}

#[test]
fn same_poll_cancellation_after_rename_commit_keeps_durable_unknown_result() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.path().join("committed");
    fs::write(&source, b"rename me").unwrap();
    let provider = provider(
        "committed",
        "destination",
        "native-rename-same-poll-cancellation",
    );
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(CancelAfterReadyRenameTool {
            inner: RenameFileTool::open(temporary.path()).unwrap(),
        })
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-rename-same-poll-cancellation");

    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        })
    ));
    assert_exact_capability(&policy, "committed", "destination");
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
    assert!(!source.exists());
    assert_eq!(
        fs::read(temporary.path().join("destination")).unwrap(),
        b"rename me"
    );

    let record = store.record(&session_id).unwrap();
    assert_eq!(record.messages.len(), 3);
    assert_eq!(record.messages[2].role, Role::Tool);
    let ContentBlock::ToolResult { call_id, output } = &record.messages[2].content[0] else {
        panic!("expected durable unknown tool result placeholder")
    };
    assert_eq!(call_id.as_str(), "rename-file-call");
    assert_eq!(
        output,
        &ToolOutput {
            content: json!({
                "code": "tool_result_unknown",
                "message": "tool result status is unknown",
            }),
            is_error: true,
        }
    );
}
