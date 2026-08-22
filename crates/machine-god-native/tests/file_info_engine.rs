#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs::{self, FileTimes};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use futures_util::StreamExt;
use machine_god_core::{
    Capability, ContentBlock, Engine, EngineEvent, FilesystemAccess, Message, ModelEvent,
    PermissionDecision, PermissionGrantScope, Role, SessionId, SessionIncarnationId, StopReason,
    ToolCall, ToolCallId, ToolName, ToolOutput, TurnEvent,
};
use machine_god_native::{FILE_INFO_TOOL_NAME, FileInfoTool};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::{Value, json};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-file-info-engine-{}-{identifier}",
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

fn provider(arguments: Value, name: &str) -> ScriptedModelProvider {
    let call = ToolCall {
        id: ToolCallId::new("file-info-call").unwrap(),
        name: ToolName::new(FILE_INFO_TOOL_NAME).unwrap(),
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
            .prompt("inspect metadata for the requested path")
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
        FILE_INFO_TOOL_NAME
    );
    let message = requests[1].request.messages[2].clone();
    assert_eq!(message.role, Role::Tool);
    let ContentBlock::ToolResult { output, .. } = &message.content[0] else {
        panic!("expected a durable tool result")
    };
    (message.clone(), output.clone())
}

fn assert_exact_capability(policy: &ScriptedPermissionHandler, path: &str) {
    let requests = policy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Metadata,
            path: path.to_owned(),
        }
    );
}

fn assert_no_tool_execution_events(events: &[EngineEvent]) {
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
    )));
}

fn serialized_request(provider: &ScriptedModelProvider) -> String {
    serde_json::to_string(&provider.requests()[1].request).unwrap()
}

#[test]
fn engine_denial_uses_exact_metadata_authority_before_filesystem_effects_or_tool_events() {
    let temporary = TemporaryDirectory::new();
    let secret = "DENIED_FILE_INFO_SECRET";
    fs::write(temporary.path().join("secret.txt"), secret).unwrap();
    let provider = provider(json!({ "path": "./secret.txt" }), "native-info-denied");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Deny {
            reason: "denied by test policy".to_owned(),
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(FileInfoTool::open(temporary.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-info-denied");

    assert_completed(&events);
    assert_exact_capability(&policy, "secret.txt");
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
    assert!(!serialized_request(&provider).contains(secret));
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    assert_eq!(
        fs::read_to_string(temporary.path().join("secret.txt")).unwrap(),
        secret
    );
}

#[test]
fn engine_allow_resolves_policy_before_exact_result_events_provider_and_durable_record() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("nested")).unwrap();
    let path = temporary.path().join("nested/report.json");
    fs::write(&path, b"metadata content").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            FileTimes::new()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::new(1_700_000_456, 123_456_789)),
        )
        .unwrap();
    let provider = provider(
        json!({ "path": "./nested//report.json" }),
        "native-info-allowed",
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
        .tool(FileInfoTool::open(temporary.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-info-allowed");

    assert_completed(&events);
    assert_exact_capability(&policy, "nested/report.json");
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
        "path": "nested/report.json",
        "kind": "file",
        "size_bytes": 16,
        "modified": {
            "unix_seconds": 1_700_000_456_i64,
            "nanoseconds": 123_456_789_u32
        },
        "extension": "json"
    }));
    assert!(matches!(
        &events[finished].payload,
        TurnEvent::ToolFinished { output, .. } if output == &expected
    ));
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(output, expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}

#[test]
fn engine_invalid_preflight_skips_policy_filesystem_and_tool_events() {
    let workspace = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    let secret = "PREFLIGHT_FILE_INFO_SECRET";
    fs::write(outside.path().join("secret.txt"), secret).unwrap();
    let outside_name = outside.path().file_name().unwrap().to_string_lossy();
    let requested = format!("../{outside_name}/secret.txt");
    let provider = provider(
        json!({ "path": requested }),
        "native-info-preflight-invalid",
    );
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(FileInfoTool::open(workspace.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-info-preflight-invalid");

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
    assert!(!serialized_request(&provider).contains(secret));
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    assert_eq!(
        fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
        secret
    );
}
