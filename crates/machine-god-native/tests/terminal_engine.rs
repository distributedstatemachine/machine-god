#![cfg(unix)]

use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use machine_god_core::{
    BackgroundStartError, BackgroundStartRequest, BoxFuture, Capability, ContentBlock, Engine,
    EngineEvent, MAX_BACKGROUND_CWD_BYTES, Message, ModelEvent, PermissionDecision,
    PermissionGrantScope, ProcessEnvironment, Role, SessionId, SessionIncarnationId, StopReason,
    Tool, ToolCall, ToolCallId, ToolName, ToolOutput, TurnEvent,
};
use machine_god_native::{
    MAX_TERMINAL_CWD_COMPONENT_BYTES, MAX_TERMINAL_CWD_COMPONENTS,
    TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE, TerminalBackgroundOutcome, TerminalBackgroundStarter,
    TerminalCapturedOutput, TerminalExecution, TerminalExecutionOutcome, TerminalExecutionRequest,
    TerminalExecutionStatus, TerminalExecutor, TerminalLimits, TerminalTool,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::{Value, json};

mod terminal_test_support;

use terminal_test_support::{TemporaryDirectory, call};

const PRIVATE_COMMAND: &str = "printf '%s' PRIVATE_TERMINAL_COMMAND";
const PRIVATE_ENVIRONMENT_VALUE: &str = "PRIVATE_TERMINAL_ENVIRONMENT_VALUE";

#[derive(Clone, Default)]
struct FakeExecutor {
    calls: Arc<AtomicUsize>,
}

impl TerminalExecutor for FakeExecutor {
    fn execute(
        &self,
        request: TerminalExecutionRequest,
        _cancellation: machine_god_core::CancellationToken,
    ) -> TerminalExecution {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.program(), "/bin/sh");
        assert_eq!(request.arguments(), ["-c", PRIVATE_COMMAND]);
        assert_eq!(request.command(), PRIVATE_COMMAND);
        assert_eq!(request.cwd(), ".");
        assert_eq!(request.environment_profile(), "construction_snapshot");
        Box::pin(async {
            TerminalExecutionOutcome::new(
                TerminalExecutionStatus::Exited(0),
                TerminalCapturedOutput::new(b"terminal output\n".to_vec(), 16).unwrap(),
                TerminalCapturedOutput::new(Vec::new(), 0).unwrap(),
                Duration::from_millis(3),
            )
        })
    }
}

fn provider(name: &str, action: &str) -> ScriptedModelProvider {
    provider_with_arguments(
        name,
        json!({
            "action": action,
            "command": PRIVATE_COMMAND,
        }),
    )
}

fn provider_with_arguments(name: &str, arguments: Value) -> ScriptedModelProvider {
    ScriptedModelProvider::new(
        name,
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("terminal-call").unwrap(),
                        name: ToolName::new("terminal").unwrap(),
                        arguments,
                    },
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

fn canonical_relative_cwd_with_length(length: usize) -> String {
    assert!(length > 0);
    let component_count =
        (length + MAX_TERMINAL_CWD_COMPONENT_BYTES + 1) / (MAX_TERMINAL_CWD_COMPONENT_BYTES + 1);
    assert!(component_count <= MAX_TERMINAL_CWD_COMPONENTS);
    let component_bytes = length - (component_count - 1);
    let minimum_component_bytes = component_bytes / component_count;
    let longer_components = component_bytes % component_count;
    (0..component_count)
        .map(|index| "x".repeat(minimum_component_bytes + usize::from(index < longer_components)))
        .collect::<Vec<_>>()
        .join("/")
}

fn terminal(root: &std::path::Path, executor: FakeExecutor) -> TerminalTool {
    TerminalTool::with_executor(
        root,
        vec![
            (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
            (
                OsString::from("PRIVATE_TERMINAL_ENV"),
                OsString::from(PRIVATE_ENVIRONMENT_VALUE),
            ),
        ],
        Arc::new(executor),
        TerminalLimits::default(),
    )
    .unwrap()
}

#[derive(Clone, Default)]
struct FakeBackgroundStarter {
    calls: Arc<AtomicUsize>,
}

impl TerminalBackgroundStarter for FakeBackgroundStarter {
    fn start(
        &self,
        request: BackgroundStartRequest,
        _cancellation: machine_god_core::CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalBackgroundOutcome, BackgroundStartError>> {
        assert_eq!(request.command(), PRIVATE_COMMAND);
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            TerminalBackgroundOutcome::new(17, None)
        })
    }
}

fn terminal_with_background(
    root: &std::path::Path,
    executor: FakeExecutor,
    starter: FakeBackgroundStarter,
) -> TerminalTool {
    TerminalTool::with_executor_and_background(
        root,
        vec![(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))],
        Arc::new(executor),
        TerminalLimits::default(),
        std::fs::canonicalize(root)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
        ProcessEnvironment {
            profile: TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE.to_owned(),
            sha256: "b".repeat(64),
        },
        Arc::new(starter),
    )
    .unwrap()
}

fn collect(engine: &Engine, name: &str) -> (SessionId, Vec<EngineEvent>) {
    let session_id = SessionId::new(name).unwrap();
    let session = engine
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new(format!("incarnation-{name}")).unwrap(),
        )
        .unwrap();
    let events = futures_executor::block_on(async {
        session
            .prompt("run the requested command")
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

fn second_request_tool_output(provider: &ScriptedModelProvider) -> (Message, ToolOutput) {
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request.tools.len(), 1);
    assert_eq!(requests[0].request.tools[0].name.as_str(), "terminal");
    let message = requests[1].request.messages[2].clone();
    assert_eq!(message.role, Role::Tool);
    let ContentBlock::ToolResult { output, .. } = &message.content[0] else {
        panic!("expected a durable terminal tool result")
    };
    (message.clone(), output.clone())
}

#[test]
fn denial_authorizes_exact_process_identity_without_executor_or_tool_events() {
    let temporary = TemporaryDirectory::new("engine-denied");
    let provider = provider("terminal-denied", "exec");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Deny {
            reason: "denied by test policy".to_owned(),
        })]);
    let executor = FakeExecutor::default();
    let tool_instance = terminal(temporary.path(), executor.clone());
    let expected = tool_instance
        .prepare(call(
            "terminal",
            json!({ "action": "exec", "command": PRIVATE_COMMAND }),
        ))
        .unwrap()
        .capability()
        .expect("terminal requires permission authority")
        .clone();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(tool_instance)
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "terminal-denied-session");

    let requests = policy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].capability, expected);
    assert!(matches!(expected, Capability::Process { .. }));
    let policy_rendering = serde_json::to_string(&requests[0]).unwrap();
    assert!(!policy_rendering.contains(PRIVATE_ENVIRONMENT_VALUE));
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
    )));
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
}

#[test]
fn allow_precedes_one_execution_and_persists_the_exact_bounded_output() {
    let temporary = TemporaryDirectory::new("engine-allow");
    let provider = provider("terminal-allowed", "exec");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let executor = FakeExecutor::default();
    let tool_instance = terminal(temporary.path(), executor.clone());
    let expected = tool_instance
        .prepare(call(
            "terminal",
            json!({ "action": "exec", "command": PRIVATE_COMMAND }),
        ))
        .unwrap()
        .capability()
        .expect("terminal requires permission authority")
        .clone();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(tool_instance)
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "terminal-allowed-session");

    assert_eq!(policy.requests()[0].capability, expected);
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
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(
        output,
        ToolOutput {
            content: json!({
                "action": "exec",
                "cwd": ".",
                "status": "exited",
                "exit_code": 0,
                "signal": null,
                "stdout": "terminal output\n",
                "stderr": "",
                "stdout_bytes": 16,
                "stderr_bytes": 0,
                "stdout_truncated": false,
                "stderr_truncated": false,
                "stdout_lossy": false,
                "stderr_lossy": false,
                "duration_ms": 3,
            }),
            is_error: false,
        }
    );
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}

#[test]
fn background_start_denial_has_zero_starter_and_foreground_effects() {
    let temporary = TemporaryDirectory::new("engine-start-denied");
    let provider = provider("terminal-start-denied", "start");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Deny {
            reason: "denied by test policy".to_owned(),
        })]);
    let executor = FakeExecutor::default();
    let starter = FakeBackgroundStarter::default();
    let tool = terminal_with_background(temporary.path(), executor.clone(), starter.clone());
    let expected = tool
        .prepare(call(
            "terminal",
            json!({ "action": "start", "command": PRIVATE_COMMAND }),
        ))
        .unwrap()
        .capability()
        .unwrap()
        .clone();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store)
        .permission_handler(policy.clone())
        .tool(tool)
        .build()
        .unwrap();

    let (_, events) = collect(&engine, "terminal-start-denied-session");

    assert_eq!(policy.requests()[0].capability, expected);
    assert_eq!(starter.calls.load(Ordering::SeqCst), 0);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
    )));
    assert_eq!(
        second_request_tool_output(&provider).1,
        ToolOutput {
            content: json!({
                "code": "permission_denied",
                "message": "tool execution was denied by policy",
            }),
            is_error: true,
        }
    );
}

#[test]
fn over_combined_background_cwd_bound_fails_before_permission_or_starter_effects() {
    let temporary = TemporaryDirectory::new("engine-start-cwd-over-bound");
    let workspace = std::fs::canonicalize(temporary.path()).unwrap();
    let workspace = workspace.to_str().expect("test workspace is Unicode");
    let prefix_bytes = workspace.len() + usize::from(workspace != "/");
    let exact_relative_bytes = MAX_BACKGROUND_CWD_BYTES
        .checked_sub(prefix_bytes)
        .expect("temporary workspace leaves room for a relative cwd");
    let over_cwd = canonical_relative_cwd_with_length(exact_relative_bytes + 1);
    let provider = provider_with_arguments(
        "terminal-start-cwd-over-bound",
        json!({
            "action": "start",
            "command": PRIVATE_COMMAND,
            "cwd": over_cwd,
        }),
    );
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([]);
    let executor = FakeExecutor::default();
    let starter = FakeBackgroundStarter::default();
    let tool = terminal_with_background(temporary.path(), executor.clone(), starter.clone());
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store)
        .permission_handler(policy.clone())
        .tool(tool)
        .build()
        .unwrap();

    let (_, events) = collect(&engine, "terminal-start-cwd-over-bound-session");

    assert!(policy.requests().is_empty());
    assert_eq!(starter.calls.load(Ordering::SeqCst), 0);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::PermissionRequested { .. }
            | TurnEvent::PermissionResolved { .. }
            | TurnEvent::ToolStarted { .. }
            | TurnEvent::ToolFinished { .. }
    )));
    assert_eq!(
        second_request_tool_output(&provider).1,
        ToolOutput {
            content: json!({
                "code": "tool_error",
                "message": "tool execution failed",
                "retryable": false,
            }),
            is_error: true,
        }
    );
}

#[test]
fn background_start_runs_once_after_permission_and_persists_display_identity() {
    let temporary = TemporaryDirectory::new("engine-start-allowed");
    let provider = provider("terminal-start-allowed", "start");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let executor = FakeExecutor::default();
    let starter = FakeBackgroundStarter::default();
    let tool = terminal_with_background(temporary.path(), executor.clone(), starter.clone());
    let expected = tool
        .prepare(call(
            "terminal",
            json!({ "action": "start", "command": PRIVATE_COMMAND }),
        ))
        .unwrap()
        .capability()
        .unwrap()
        .clone();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(tool)
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "terminal-start-allowed-session");

    assert_eq!(policy.requests()[0].capability, expected);
    let Capability::Process {
        working_directory,
        environment,
        ..
    } = expected
    else {
        panic!("start must request process permission")
    };
    assert_eq!(working_directory, ".");
    assert_eq!(environment.profile, TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE);
    assert_eq!(starter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    let resolved = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::PermissionResolved { .. }))
        .unwrap();
    let tool_started_index = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolStarted { .. }))
        .unwrap();
    assert!(resolved < tool_started_index);
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(
        output,
        ToolOutput {
            content: json!({
                "action": "start",
                "background_id": 17,
                "pid": null,
                "status": "started",
            }),
            is_error: false,
        }
    );
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}
