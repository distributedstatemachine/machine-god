#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt;
use machine_god_core::{
    Capability, ContentBlock, Engine, EngineEvent, FilesystemAccess, Message, ModelEvent,
    PermissionDecision, PermissionGrantScope, Role, SessionId, SessionIncarnationId, StopReason,
    ToolCall, ToolCallId, ToolName, ToolOutput, TurnEvent,
};
use machine_god_native::{GREP_FILES_TOOL_NAME, GrepFilesTool};
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
                "mg-grep-files-engine-{}-{identifier}",
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
        id: ToolCallId::new("grep-files-call").unwrap(),
        name: ToolName::new(GREP_FILES_TOOL_NAME).unwrap(),
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
            .prompt("search the workspace contents")
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
        GREP_FILES_TOOL_NAME
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
            access: FilesystemAccess::SearchContent,
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
fn engine_denial_uses_exact_search_capability_without_reading_or_emitting_tool_events() {
    let temporary = TemporaryDirectory::new();
    let denied = temporary.path().join("denied");
    fs::create_dir(&denied).unwrap();
    let secret = "DENIED_GREP_CONTENT_SECRET";
    fs::write(denied.join("secret.txt"), secret).unwrap();
    let provider = provider(
        json!({"pattern": "SECRET", "path": "./denied//."}),
        "native-grep-denied",
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
        .tool(GrepFilesTool::open(temporary.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-grep-denied");

    assert_completed(&events);
    assert_exact_capability(&policy, "denied");
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
}

#[test]
fn engine_allow_resolves_policy_before_exact_events_provider_result_and_durable_record() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir_all(temporary.path().join("scope/src")).unwrap();
    fs::write(
        temporary.path().join("scope/src/alpha.rs"),
        b"needle\nneedle",
    )
    .unwrap();
    fs::write(temporary.path().join("scope/src/beta.rs"), b"nothing").unwrap();
    fs::write(temporary.path().join("scope/ignored.txt"), b"needle").unwrap();
    let provider = provider(
        json!({
            "pattern": "needle",
            "path": "./scope//.",
            "include": "./src//*.rs",
            "mode": "count"
        }),
        "native-grep-allowed",
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
        .tool(GrepFilesTool::open(temporary.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-grep-allowed");

    assert_completed(&events);
    assert_exact_capability(&policy, "scope");
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
        "pattern": "needle",
        "path": "scope",
        "include": "src/*.rs",
        "case_insensitive": false,
        "mode": "count",
        "head_limit": 100,
        "offset": 0,
        "context_lines": 0,
        "candidate_files": 2,
        "searched_files": 2,
        "skipped_oversized_files": 0,
        "skipped_non_text_files": 0,
        "matching_lines": 2,
        "matching_files": 1
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
    let secret = "PREFLIGHT_GREP_CONTENT_SECRET";
    fs::write(outside.path().join("secret.txt"), secret).unwrap();
    let outside_name = outside.path().file_name().unwrap().to_string_lossy();
    let provider = provider(
        json!({"pattern": "SECRET", "path": format!("../{outside_name}")}),
        "native-grep-preflight-invalid",
    );
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(GrepFilesTool::open(workspace.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-grep-preflight-invalid");

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
}

#[test]
fn engine_allowed_execution_error_is_generic_retryable_and_durable_after_tool_events() {
    let temporary = TemporaryDirectory::new();
    let provider = provider(
        json!({"pattern": "needle", "path": "missing", "mode": "count"}),
        "native-grep-execution-error",
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
        .tool(GrepFilesTool::open(temporary.path()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-grep-execution-error");

    assert_completed(&events);
    assert_exact_capability(&policy, "missing");
    let finished = events
        .iter()
        .find_map(|event| match &event.payload {
            TurnEvent::ToolFinished { output, .. } => Some(output.clone()),
            _ => None,
        })
        .expect("allowed failing execution emits ToolFinished");
    let expected = ToolOutput {
        content: json!({
            "code": "tool_error",
            "message": "tool execution failed",
            "retryable": false
        }),
        is_error: true,
    };
    assert_eq!(finished, expected);
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(output, expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}
