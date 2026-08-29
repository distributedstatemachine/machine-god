#![cfg(unix)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, Engine, EngineEvent, FilesystemAccess,
    Message, ModelEvent, PermissionDecision, PermissionGrantScope, PreparedToolCall, Role,
    SessionId, SessionIncarnationId, StopReason, Tool, ToolCall, ToolCallId, ToolContext,
    ToolError, ToolName, ToolOutput, ToolSpec, TurnEvent,
};
use machine_god_native::{SKILL_TOOL_NAME, SkillTool};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::{Value, json};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

struct CancelAfterReadySkillTool {
    inner: SkillTool,
}

impl Tool for CancelAfterReadySkillTool {
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
                "skill execution was unexpectedly cancelled before becoming ready"
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
                "mg-skill-engine-{}-{identifier}",
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

fn write_resource(root: &Path, name: &str, resource: &str, contents: &str) {
    let path = root.join("skills").join(name).join(resource);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn provider(arguments: Value, name: &str) -> ScriptedModelProvider {
    let call = ToolCall {
        id: ToolCallId::new("skill-call").unwrap(),
        name: ToolName::new(SKILL_TOOL_NAME).unwrap(),
        arguments,
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
            .prompt("read the requested skill resource")
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

fn second_request_tool_message(provider: &ScriptedModelProvider) -> Message {
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request.tools.len(), 1);
    assert_eq!(requests[0].request.tools[0].name.as_str(), SKILL_TOOL_NAME);
    requests[1].request.messages[2].clone()
}

fn assert_exact_capability(policy: &ScriptedPermissionHandler, path: &str) {
    let requests = policy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Read,
            path: path.to_owned(),
        }
    );
}

fn assert_no_execution_events(events: &[EngineEvent]) {
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
    )));
}

#[test]
fn engine_denial_uses_normalized_permission_without_observing_resource_content() {
    let workspace = TemporaryDirectory::new();
    let secret = "DENIED_SKILL_RESOURCE_SECRET";
    write_resource(
        workspace.path(),
        "release-checks",
        "references/linux.md",
        secret,
    );
    let provider = provider(
        json!({
            "name": "release-checks",
            "resource": "./references//linux.md",
        }),
        "skill-denied",
    );
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Deny {
            reason: "denied by test policy".to_owned(),
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(SkillTool::open(workspace.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "skill-denied");

    assert_completed(&events, &StopReason::Completed);
    assert_exact_capability(&policy, "skills/release-checks/references/linux.md");
    assert_no_execution_events(&events);
    let message = second_request_tool_message(&provider);
    assert_eq!(
        tool_result(&message),
        &ToolOutput {
            content: json!({
                "code": "permission_denied",
                "message": "tool execution was denied by policy",
            }),
            is_error: true,
        }
    );
    assert!(
        !serde_json::to_string(&provider.requests()[1].request)
            .unwrap()
            .contains(secret)
    );
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    assert_eq!(
        fs::read_to_string(
            workspace
                .path()
                .join("skills/release-checks/references/linux.md")
        )
        .unwrap(),
        secret
    );
}

#[test]
fn engine_allow_returns_exact_page_to_provider_and_durable_session() {
    let workspace = TemporaryDirectory::new();
    let contents = "alpha\nβeta\n";
    write_resource(
        workspace.path(),
        "release-checks",
        "references/linux.md",
        contents,
    );
    let provider = provider(
        json!({
            "name": "release-checks",
            "resource": "./references//linux.md",
            "offset": 6,
        }),
        "skill-allowed",
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
        .tool(SkillTool::open(workspace.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "skill-allowed");

    assert_completed(&events, &StopReason::Completed);
    assert_exact_capability(&policy, "skills/release-checks/references/linux.md");
    let resolved = events
        .iter()
        .position(|event| {
            matches!(
                event.payload,
                TurnEvent::PermissionResolved {
                    decision: PermissionDecision::Allow { .. },
                    ..
                }
            )
        })
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
        "name": "release-checks",
        "resource": "references/linux.md",
        "offset": 6,
        "next_offset": 12,
        "total_bytes": 12,
        "content": "βeta\n",
        "truncated": false,
    }));
    assert!(matches!(
        &events[finished].payload,
        TurnEvent::ToolFinished { output, .. } if output == &expected
    ));
    let message = second_request_tool_message(&provider);
    assert_eq!(tool_result(&message), &expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}

#[test]
fn engine_preflight_error_skips_permission_and_resource_execution() {
    let workspace = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    let secret = "PREFLIGHT_SKILL_RESOURCE_SECRET";
    fs::write(outside.path().join("secret.md"), secret).unwrap();
    let provider = provider(
        json!({
            "name": "release-checks",
            "resource": "../secret.md",
        }),
        "skill-invalid",
    );
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(SkillTool::open(workspace.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "skill-invalid");

    assert_completed(&events, &StopReason::Completed);
    assert!(policy.requests().is_empty());
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::PermissionRequested { .. }
            | TurnEvent::PermissionResolved { .. }
            | TurnEvent::ToolStarted { .. }
            | TurnEvent::ToolFinished { .. }
    )));
    let message = second_request_tool_message(&provider);
    assert_eq!(
        tool_result(&message).content["code"],
        Value::String("tool_error".to_owned())
    );
    assert!(tool_result(&message).is_error);
    assert!(
        !serde_json::to_string(&provider.requests()[1].request)
            .unwrap()
            .contains(secret)
    );
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    assert_eq!(
        fs::read_to_string(outside.path().join("secret.md")).unwrap(),
        secret
    );
}

#[test]
fn same_poll_cancellation_after_read_keeps_durable_unknown_result() {
    let workspace = TemporaryDirectory::new();
    let secret = "CANCELLED_SKILL_RESOURCE_SECRET";
    write_resource(workspace.path(), "release-checks", "SKILL.md", secret);
    let provider = provider(json!({"name": "release-checks"}), "skill-cancelled");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(CancelAfterReadySkillTool {
            inner: SkillTool::open(workspace.path()).unwrap(),
        })
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "skill-cancelled");

    assert_completed(&events, &StopReason::Cancelled);
    assert_exact_capability(&policy, "skills/release-checks/SKILL.md");
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

    let record = store.record(&session_id).unwrap();
    assert_eq!(record.messages.len(), 3);
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
    assert!(!format!("{:?}", record.messages).contains(secret));
    assert_eq!(
        fs::read_to_string(workspace.path().join("skills/release-checks/SKILL.md")).unwrap(),
        secret
    );
}
