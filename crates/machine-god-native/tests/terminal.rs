#![cfg(unix)]

use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use machine_god_core::{
    CancellationToken, Capability, ProcessEnvironment, Tool, ToolError, ToolErrorKind, ToolOutput,
};
use machine_god_native::{
    TerminalCapturedOutput, TerminalConfigErrorKind, TerminalExecution, TerminalExecutionOutcome,
    TerminalExecutionRequest, TerminalExecutionStatus, TerminalExecutor, TerminalExecutorError,
    TerminalExecutorErrorKind, TerminalLimits, TerminalTool,
};
use serde_json::{Value, json};

mod terminal_test_support;

use terminal_test_support::{TemporaryDirectory, call, context, poll_once, poll_ready};

const PRIVATE_ENVIRONMENT_KEY: &str = "MACHINE_GOD_TERMINAL_PRIVATE_KEY";
const PRIVATE_ENVIRONMENT_VALUE: &str = "PRIVATE_ENVIRONMENT_VALUE_DO_NOT_REFLECT";

#[derive(Clone, Copy)]
enum Mode {
    Exited(i32),
    Signaled(i32),
    TimedOut,
    OutputLimit,
    Error(TerminalExecutorErrorKind),
    Pending,
    CancelThenExit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RequestRecord {
    program: String,
    arguments: Vec<String>,
    command: String,
    cwd: String,
    environment_profile: String,
    environment_sha256: String,
    environment: Vec<(OsString, OsString)>,
    deadline: Instant,
    directory_identity: String,
    debug: String,
}

#[derive(Default)]
struct State {
    calls: AtomicUsize,
    polls: AtomicUsize,
    drops: AtomicUsize,
    requests: Mutex<Vec<RequestRecord>>,
}

#[derive(Clone)]
struct FakeExecutor {
    mode: Mode,
    state: Arc<State>,
    stdout: Vec<u8>,
    stdout_total: usize,
    stderr: Vec<u8>,
    stderr_total: usize,
}

impl FakeExecutor {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            state: Arc::new(State::default()),
            stdout: Vec::new(),
            stdout_total: 0,
            stderr: Vec::new(),
            stderr_total: 0,
        }
    }

    fn with_output(mut self, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        self.stdout_total = stdout.len();
        self.stderr_total = stderr.len();
        self.stdout = stdout;
        self.stderr = stderr;
        self
    }

    fn with_totals(mut self, stdout_total: usize, stderr_total: usize) -> Self {
        self.stdout_total = stdout_total;
        self.stderr_total = stderr_total;
        self
    }

    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    fn polls(&self) -> usize {
        self.state.polls.load(Ordering::SeqCst)
    }

    fn drops(&self) -> usize {
        self.state.drops.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<RequestRecord> {
        self.state.requests.lock().unwrap().clone()
    }
}

impl TerminalExecutor for FakeExecutor {
    fn execute(
        &self,
        request: TerminalExecutionRequest,
        cancellation: CancellationToken,
    ) -> TerminalExecution {
        let directory = rustix::fs::fstat(request.directory_fd()).unwrap();
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        self.state.requests.lock().unwrap().push(RequestRecord {
            program: request.program().to_owned(),
            arguments: request.arguments().into_iter().map(str::to_owned).collect(),
            command: request.command().to_owned(),
            cwd: request.cwd().to_owned(),
            environment_profile: request.environment_profile().to_owned(),
            environment_sha256: request.environment_sha256().to_owned(),
            environment: request.environment().to_vec(),
            deadline: request.deadline(),
            directory_identity: format!("{}:{}", directory.st_dev, directory.st_ino),
            debug: format!("{request:?}"),
        });
        Box::pin(FakeExecution {
            mode: self.mode,
            cancellation,
            state: Arc::clone(&self.state),
            stdout: self.stdout.clone(),
            stdout_total: self.stdout_total,
            stderr: self.stderr.clone(),
            stderr_total: self.stderr_total,
        })
    }
}

struct FakeExecution {
    mode: Mode,
    cancellation: CancellationToken,
    state: Arc<State>,
    stdout: Vec<u8>,
    stdout_total: usize,
    stderr: Vec<u8>,
    stderr_total: usize,
}

impl Future for FakeExecution {
    type Output = Result<TerminalExecutionOutcome, TerminalExecutorError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.state.polls.fetch_add(1, Ordering::SeqCst);
        if matches!(self.mode, Mode::Pending) {
            return Poll::Pending;
        }
        if matches!(self.mode, Mode::CancelThenExit) {
            assert!(self.cancellation.cancel());
        }
        if let Mode::Error(kind) = self.mode {
            return Poll::Ready(Err(TerminalExecutorError::new(kind)));
        }
        let status = match self.mode {
            Mode::Exited(code) => TerminalExecutionStatus::Exited(code),
            Mode::CancelThenExit => TerminalExecutionStatus::Exited(0),
            Mode::Signaled(signal) => TerminalExecutionStatus::Signaled(signal),
            Mode::TimedOut => TerminalExecutionStatus::TimedOut,
            Mode::OutputLimit => TerminalExecutionStatus::OutputLimit,
            Mode::Error(_) | Mode::Pending => unreachable!(),
        };
        Poll::Ready(TerminalExecutionOutcome::new(
            status,
            TerminalCapturedOutput::new(self.stdout.clone(), self.stdout_total as u64).unwrap(),
            TerminalCapturedOutput::new(self.stderr.clone(), self.stderr_total as u64).unwrap(),
            Duration::from_millis(7),
        ))
    }
}

impl Drop for FakeExecution {
    fn drop(&mut self) {
        self.state.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn environment() -> Vec<(OsString, OsString)> {
    vec![
        (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
        (
            OsString::from(PRIVATE_ENVIRONMENT_KEY),
            OsString::from(PRIVATE_ENVIRONMENT_VALUE),
        ),
    ]
}

fn limits(max_active: usize) -> TerminalLimits {
    TerminalLimits::new(Duration::from_secs(5), max_active).unwrap()
}

fn tool(root: &std::path::Path, executor: &FakeExecutor) -> TerminalTool {
    TerminalTool::with_executor(
        root,
        environment(),
        Arc::new(executor.clone()),
        TerminalLimits::default(),
    )
    .unwrap()
}

fn execute(
    tool: &TerminalTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_ready(tool.execute(context(), arguments, cancellation))
}

fn exact_arguments(command: &str, cwd: &str) -> Value {
    json!({
        "action": "exec",
        "command": command,
        "cwd": cwd,
        "profile": "clean",
    })
}

fn assert_invalid_input(error: &ToolError) {
    assert_eq!(error.kind, ToolErrorKind::InvalidInput);
    assert!(!error.retryable);
}

#[test]
fn spec_and_defaults_are_strict_and_prepare_exact_process_identity() {
    let temporary = TemporaryDirectory::new("contract");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);

    let spec = tool.spec();
    assert_eq!(spec.name.as_str(), "terminal");
    assert_eq!(
        spec.description,
        "Run one foreground shell command from a workspace-relative directory"
    );
    assert_eq!(spec.input_schema["type"], "object");
    assert_eq!(spec.input_schema["required"], json!(["action", "command"]));
    assert_eq!(spec.input_schema["additionalProperties"], false);
    let properties = spec.input_schema["properties"].as_object().unwrap();
    assert_eq!(properties.len(), 4);
    assert_eq!(properties["action"]["type"], "string");
    assert_eq!(properties["action"]["enum"], json!(["exec"]));
    assert_eq!(properties["command"]["type"], "string");
    assert_eq!(properties["cwd"]["type"], "string");
    assert_eq!(properties["profile"]["type"], "string");
    assert_eq!(properties["profile"]["enum"], json!(["clean"]));

    let prepared = tool
        .prepare(call(
            "terminal",
            json!({ "action": "exec", "command": "printf '%s' hello" }),
        ))
        .unwrap();
    assert_eq!(
        prepared.arguments(),
        &exact_arguments("printf '%s' hello", ".")
    );
    let Capability::Process {
        program,
        arguments,
        working_directory,
        environment,
    } = prepared.capability()
    else {
        panic!("terminal must prepare a process capability")
    };
    assert_eq!(program, "/bin/sh");
    assert_eq!(arguments, &["-c", "printf '%s' hello"]);
    assert_eq!(working_directory, ".");
    assert_eq!(environment.profile, "construction_snapshot");
    assert_eq!(environment.sha256.len(), 64);
    assert!(
        environment
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    assert_eq!(
        serde_json::to_value(prepared.capability()).unwrap(),
        json!({
            "type": "process",
            "program": "/bin/sh",
            "arguments": ["-c", "printf '%s' hello"],
            "working_directory": ".",
            "environment": {
                "profile": "construction_snapshot",
                "sha256": environment.sha256,
            }
        })
    );
}

#[test]
fn strict_schema_command_and_cwd_boundaries_reject_without_executor_effects() {
    let temporary = TemporaryDirectory::new("invalid");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);
    let over_component = "x".repeat(256);
    let over_cwd = "x".repeat(4_097);
    let over_command = "x".repeat(32 * 1_024 + 1);
    let too_many_components = std::iter::repeat_n("x", 257).collect::<Vec<_>>().join("/");
    assert_invalid_input(
        &tool
            .prepare(call(
                "web_search",
                json!({ "action": "exec", "command": "true" }),
            ))
            .unwrap_err(),
    );
    let invalid = vec![
        json!(null),
        json!([]),
        json!({}),
        json!({ "action": "exec" }),
        json!({ "command": "true" }),
        json!({ "action": "start", "command": "true" }),
        json!({ "action": 1, "command": "true" }),
        json!({ "action": "exec", "command": 1 }),
        json!({ "action": "exec", "command": "" }),
        json!({ "action": "exec", "command": "true", "cwd": 1 }),
        json!({ "action": "exec", "command": "true", "profile": "login" }),
        json!({ "action": "exec", "command": "true", "extra": true }),
        json!({ "action": "exec", "command": over_command }),
        json!({ "action": "exec", "command": "true", "cwd": "" }),
        json!({ "action": "exec", "command": "true", "cwd": "/absolute" }),
        json!({ "action": "exec", "command": "true", "cwd": "~" }),
        json!({ "action": "exec", "command": "true", "cwd": "a//b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a/" }),
        json!({ "action": "exec", "command": "true", "cwd": "./a" }),
        json!({ "action": "exec", "command": "true", "cwd": "a/./b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a/../b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a\0b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a\nb" }),
        json!({ "action": "exec", "command": "true", "cwd": "a\u{202e}b" }),
        json!({ "action": "exec", "command": "true", "cwd": over_component }),
        json!({ "action": "exec", "command": "true", "cwd": over_cwd }),
        json!({ "action": "exec", "command": "true", "cwd": too_many_components }),
    ];
    for arguments in invalid {
        assert_invalid_input(
            &tool
                .prepare(call("terminal", arguments.clone()))
                .unwrap_err(),
        );
        assert_invalid_input(&execute(&tool, arguments, CancellationToken::new()).unwrap_err());
    }
    assert_eq!(executor.calls(), 0);
    assert_eq!(executor.polls(), 0);
}

#[test]
fn exact_command_and_cwd_boundaries_prepare_successfully() {
    let temporary = TemporaryDirectory::new("exact-bounds");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);
    let exact_command = "x".repeat(32 * 1_024);
    let exact_component = "x".repeat(255);
    let exact_components = std::iter::repeat_n("x", 256).collect::<Vec<_>>().join("/");
    for (command, cwd) in [
        (exact_command.as_str(), "."),
        ("true", exact_component.as_str()),
        ("true", exact_components.as_str()),
    ] {
        let prepared = tool
            .prepare(call(
                "terminal",
                json!({
                    "action": "exec",
                    "command": command,
                    "cwd": cwd,
                    "profile": "clean",
                }),
            ))
            .unwrap();
        assert_eq!(prepared.arguments(), &exact_arguments(command, cwd));
    }
    assert_eq!(executor.calls(), 0);
}

#[test]
fn environment_digest_is_order_stable_and_raw_values_never_reflect() {
    let temporary = TemporaryDirectory::new("environment");
    let first_executor = FakeExecutor::new(Mode::Exited(0));
    let first = tool(temporary.path(), &first_executor);
    let mut reversed_environment = environment();
    reversed_environment.reverse();
    let second_executor = FakeExecutor::new(Mode::Exited(0));
    let second = TerminalTool::with_executor(
        temporary.path(),
        reversed_environment,
        Arc::new(second_executor),
        TerminalLimits::default(),
    )
    .unwrap();
    let changed_executor = FakeExecutor::new(Mode::Exited(0));
    let changed = TerminalTool::with_executor(
        temporary.path(),
        vec![(
            OsString::from(PRIVATE_ENVIRONMENT_KEY),
            OsString::from("different"),
        )],
        Arc::new(changed_executor),
        TerminalLimits::default(),
    )
    .unwrap();
    let arguments = json!({ "action": "exec", "command": "true" });
    let first_capability = first
        .prepare(call("terminal", arguments.clone()))
        .unwrap()
        .capability()
        .clone();
    let second_capability = second
        .prepare(call("terminal", arguments.clone()))
        .unwrap()
        .capability()
        .clone();
    let changed_capability = changed
        .prepare(call("terminal", arguments))
        .unwrap()
        .capability()
        .clone();
    assert_eq!(first_capability, second_capability);
    assert_ne!(first_capability, changed_capability);

    let rendered = format!("{first_capability:?}");
    assert!(!rendered.contains(PRIVATE_ENVIRONMENT_KEY));
    assert!(!rendered.contains(PRIVATE_ENVIRONMENT_VALUE));

    let output = execute(
        &first,
        exact_arguments("true", "."),
        CancellationToken::new(),
    )
    .unwrap();
    assert!(!output.is_error);
    let requests = first_executor.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].program, "/bin/sh");
    assert_eq!(requests[0].arguments, ["-c", "true"]);
    assert_eq!(requests[0].command, "true");
    assert_eq!(requests[0].cwd, ".");
    assert_eq!(requests[0].environment_profile, "construction_snapshot");
    let mut expected_environment = environment();
    expected_environment.sort();
    assert_eq!(requests[0].environment, expected_environment);
    let now = Instant::now();
    assert!(requests[0].deadline > now);
    assert!(requests[0].deadline <= now + Duration::from_secs(120));
    assert!(!requests[0].debug.contains(PRIVATE_ENVIRONMENT_KEY));
    assert!(!requests[0].debug.contains(PRIVATE_ENVIRONMENT_VALUE));
}

#[test]
fn direct_execute_revalidates_complete_canonical_arguments_without_effects() {
    let temporary = TemporaryDirectory::new("revalidate");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);
    for arguments in [
        json!({ "action": "exec", "command": "true" }),
        json!({ "action": "exec", "command": "true", "cwd": "." }),
        json!({ "action": "exec", "command": "true", "profile": "clean" }),
        json!({ "action": "exec", "command": "true", "cwd": ".", "profile": "clean", "extra": 1 }),
        json!({ "action": "exec", "command": "true", "cwd": "a//b", "profile": "clean" }),
    ] {
        assert_invalid_input(&execute(&tool, arguments, CancellationToken::new()).unwrap_err());
    }
    assert_eq!(executor.calls(), 0);
}

#[test]
fn descriptor_relative_cwd_failures_never_reach_the_injected_executor() {
    let temporary = TemporaryDirectory::new("cwd-failures");
    std::fs::create_dir(temporary.path().join("directory")).unwrap();
    std::fs::write(temporary.path().join("regular-file"), b"not a directory").unwrap();
    std::os::unix::fs::symlink("directory", temporary.path().join("symlink")).unwrap();
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);

    for cwd in ["missing", "regular-file", "symlink"] {
        let error = execute(
            &tool,
            exact_arguments("true", cwd),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::Unavailable);
        assert_eq!(error.code, "terminal_cwd_unavailable");
        assert_eq!(error.message, "terminal working directory is unavailable");
        assert!(!error.retryable);
    }
    assert_eq!(executor.calls(), 0);
    assert_eq!(executor.polls(), 0);
}

#[test]
fn retained_root_rename_and_path_replacement_cannot_redirect_request_identity() {
    let temporary = TemporaryDirectory::new("retained-root");
    let root = temporary.path().join("workspace");
    let moved = temporary.path().join("workspace-moved");
    std::fs::create_dir(&root).unwrap();
    let original = rustix::fs::stat(&root).unwrap();
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(&root, &executor);

    std::fs::rename(&root, &moved).unwrap();
    std::fs::create_dir(&root).unwrap();
    let replacement = rustix::fs::stat(&root).unwrap();
    assert_ne!(
        (original.st_dev, original.st_ino),
        (replacement.st_dev, replacement.st_ino)
    );

    let output = execute(
        &tool,
        exact_arguments("true", "."),
        CancellationToken::new(),
    )
    .unwrap();
    assert!(!output.is_error);
    let requests = executor.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].directory_identity,
        format!("{}:{}", original.st_dev, original.st_ino)
    );
    assert_ne!(
        requests[0].directory_identity,
        format!("{}:{}", replacement.st_dev, replacement.st_ino)
    );
}

#[test]
fn executor_is_inert_until_poll_and_drop_owns_the_pending_execution() {
    let temporary = TemporaryDirectory::new("future");
    let executor = FakeExecutor::new(Mode::Pending);
    let tool = tool(temporary.path(), &executor);
    let mut execution = Box::pin(tool.execute(
        context(),
        exact_arguments("sleep forever", "."),
        CancellationToken::new(),
    ));
    assert_eq!(executor.calls(), 0);
    assert_eq!(executor.polls(), 0);
    assert!(poll_once(execution.as_mut()).is_pending());
    assert_eq!(executor.calls(), 1);
    assert_eq!(executor.polls(), 1);
    assert_eq!(executor.drops(), 0);
    drop(execution);
    assert_eq!(executor.drops(), 1);
}

#[test]
fn pre_cancel_and_same_poll_cancel_win_without_publishing_output() {
    let temporary = TemporaryDirectory::new("cancellation");
    let pre_executor = FakeExecutor::new(Mode::Exited(0));
    let pre_tool = tool(temporary.path(), &pre_executor);
    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    let error = execute(&pre_tool, exact_arguments("true", "."), cancellation).unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(pre_executor.calls(), 0);

    let race_executor = FakeExecutor::new(Mode::CancelThenExit);
    let race_tool = tool(temporary.path(), &race_executor);
    let error = execute(
        &race_tool,
        exact_arguments("true", "."),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(race_executor.calls(), 1);
    assert_eq!(race_executor.drops(), 1);
}

#[test]
fn statuses_streams_loss_and_truncation_render_as_bounded_protocol() {
    let temporary = TemporaryDirectory::new("outcomes");
    std::fs::create_dir(temporary.path().join("nested")).unwrap();
    let cases = [
        (Mode::Exited(0), "exited", Some(0), None, false),
        (Mode::Exited(23), "exited", Some(23), None, true),
        (Mode::Signaled(15), "signaled", None, Some(15), true),
        (Mode::TimedOut, "timed_out", None, None, true),
        (Mode::OutputLimit, "output_limit", None, None, true),
    ];
    for (mode, status, exit_code, signal, is_error) in cases {
        let produced_stdout = if matches!(mode, Mode::OutputLimit) {
            1024 * 1024 + 1
        } else {
            70_000
        };
        let executor = FakeExecutor::new(mode)
            .with_output(b"hello\n".to_vec(), vec![b'e', 0xff, b'\n'])
            .with_totals(produced_stdout, 9);
        let tool = tool(temporary.path(), &executor);
        let output = execute(
            &tool,
            exact_arguments("private command", "nested"),
            CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(output.is_error, is_error);
        assert_eq!(output.content["action"], "exec");
        assert_eq!(output.content["cwd"], "nested");
        assert_eq!(output.content["status"], status);
        assert_eq!(output.content["exit_code"], json!(exit_code));
        assert_eq!(output.content["signal"], json!(signal));
        assert_eq!(output.content["stdout"], "hello\n");
        assert_eq!(output.content["stderr"], "e�\n");
        assert_eq!(output.content["stdout_bytes"], produced_stdout);
        assert_eq!(output.content["stderr_bytes"], 9);
        assert_eq!(output.content["stdout_truncated"], true);
        assert_eq!(output.content["stderr_truncated"], true);
        assert_eq!(output.content["stdout_lossy"], false);
        assert_eq!(output.content["stderr_lossy"], true);
        assert_eq!(output.content["duration_ms"], 7);
        assert!(serde_json::to_vec(&output).unwrap().len() <= 48 * 1_024);
    }
}

#[test]
fn serialized_result_is_trimmed_below_the_contract_ceiling() {
    let temporary = TemporaryDirectory::new("serialized-cap");
    let executor = FakeExecutor::new(Mode::Exited(0)).with_output(
        std::iter::repeat_n(b'\\', 32 * 1_024).collect(),
        std::iter::repeat_n(b'"', 32 * 1_024).collect(),
    );
    let tool = tool(temporary.path(), &executor);
    let output = execute(
        &tool,
        exact_arguments("true", "."),
        CancellationToken::new(),
    )
    .unwrap();
    let serialized = serde_json::to_vec(&output).unwrap();
    assert!(serialized.len() <= 48 * 1_024, "{}", serialized.len());
    assert_eq!(output.content["stdout_truncated"], true);
    assert_eq!(output.content["stderr_truncated"], true);
}

#[test]
fn fixed_executor_failures_are_redacted_and_classified() {
    let temporary = TemporaryDirectory::new("failures");
    for kind in [
        TerminalExecutorErrorKind::Unsupported,
        TerminalExecutorErrorKind::Busy,
        TerminalExecutorErrorKind::Spawn,
        TerminalExecutorErrorKind::Wait,
        TerminalExecutorErrorKind::Pipe,
        TerminalExecutorErrorKind::Invariant,
        TerminalExecutorErrorKind::InvalidResponse,
        TerminalExecutorErrorKind::Cancelled,
    ] {
        let executor = FakeExecutor::new(Mode::Error(kind));
        let tool = tool(temporary.path(), &executor);
        let error = execute(
            &tool,
            exact_arguments("PRIVATE_COMMAND_DO_NOT_REFLECT", "."),
            CancellationToken::new(),
        )
        .unwrap_err();
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("PRIVATE_COMMAND_DO_NOT_REFLECT"));
        assert!(!rendered.contains(PRIVATE_ENVIRONMENT_VALUE));
        if matches!(kind, TerminalExecutorErrorKind::Busy) {
            assert_eq!(error.code, "terminal_busy");
            assert!(error.retryable);
        }
    }
}

#[test]
fn fail_fast_concurrency_releases_permit_after_drop() {
    let temporary = TemporaryDirectory::new("capacity");
    let executor = FakeExecutor::new(Mode::Pending);
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        limits(1),
    )
    .unwrap();
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("first", "."),
        CancellationToken::new(),
    ));
    assert!(poll_once(first.as_mut()).is_pending());
    let error = execute(
        &tool,
        exact_arguments("second", "."),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Unavailable);
    assert_eq!(error.code, "terminal_busy");
    assert!(error.retryable);
    assert_eq!(executor.calls(), 1);
    drop(first);

    let mut third = Box::pin(tool.execute(
        context(),
        exact_arguments("third", "."),
        CancellationToken::new(),
    ));
    assert!(poll_once(third.as_mut()).is_pending());
    assert_eq!(executor.calls(), 2);
    drop(third);
}

#[test]
fn completed_execution_releases_capacity_before_output_publication() {
    let temporary = TemporaryDirectory::new("capacity-complete");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        limits(1),
    )
    .unwrap();
    for command in ["first", "second"] {
        let output = execute(
            &tool,
            exact_arguments(command, "."),
            CancellationToken::new(),
        )
        .unwrap();
        assert!(!output.is_error);
    }
    assert_eq!(executor.calls(), 2);
}

#[test]
fn absolute_deadline_drops_a_permanently_pending_executor_and_releases_capacity() {
    let temporary = TemporaryDirectory::new("pending-timeout");
    let executor = FakeExecutor::new(Mode::Pending);
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        TerminalLimits::new(Duration::from_millis(5), 1).unwrap(),
    )
    .unwrap();
    let started = Instant::now();

    for command in ["first", "second"] {
        let output = futures_executor::block_on(tool.execute(
            context(),
            exact_arguments(command, "."),
            CancellationToken::new(),
        ))
        .unwrap();
        assert!(output.is_error);
        assert_eq!(output.content["status"], "timed_out");
        assert_eq!(output.content["exit_code"], Value::Null);
        assert_eq!(output.content["signal"], Value::Null);
    }
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(executor.calls(), 2);
    assert!(executor.polls() >= 2);
    assert_eq!(executor.drops(), 2);
}

#[test]
fn process_environment_contract_round_trips_exactly() {
    let environment = ProcessEnvironment {
        profile: "construction_snapshot".to_owned(),
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
    };
    let value = serde_json::to_value(&environment).unwrap();
    assert_eq!(
        value,
        json!({
            "profile": "construction_snapshot",
            "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        })
    );
    assert_eq!(
        serde_json::from_value::<ProcessEnvironment>(value).unwrap(),
        environment
    );
}

#[test]
fn limits_enforce_the_exact_public_timeout_and_capacity_boundaries() {
    let defaults = TerminalLimits::default();
    assert_eq!(defaults.timeout(), Duration::from_secs(120));
    assert_eq!(defaults.max_active_executions(), 4);
    assert!(TerminalLimits::new(Duration::from_millis(1), 1).is_ok());
    assert!(TerminalLimits::new(Duration::from_secs(600), 16).is_ok());
    for (timeout, active) in [
        (Duration::ZERO, 1),
        (Duration::from_secs(600) + Duration::from_millis(1), 1),
        (Duration::from_secs(1), 0),
        (Duration::from_secs(1), 17),
    ] {
        let error = TerminalLimits::new(timeout, active).unwrap_err();
        assert_eq!(error.kind(), TerminalConfigErrorKind::InvalidLimits);
        assert_eq!(error.to_string(), "native terminal limits are invalid");
    }
}

#[test]
fn invalid_environment_snapshots_are_bounded_redacted_and_reject_duplicates() {
    let temporary = TemporaryDirectory::new("invalid-environment");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let too_many = (0..513)
        .map(|index| {
            (
                OsString::from(format!("KEY_{index}")),
                OsString::from("value"),
            )
        })
        .collect();
    let aggregate_overflow = (0..17)
        .map(|index| {
            (
                OsString::from(format!("AGGREGATE_{index}")),
                OsString::from("x".repeat(16 * 1_024)),
            )
        })
        .collect();
    let invalid = vec![
        vec![(OsString::new(), OsString::from("value"))],
        vec![(OsString::from("BAD=KEY"), OsString::from("value"))],
        vec![(OsString::from("BAD\0KEY"), OsString::from("value"))],
        vec![(OsString::from("KEY"), OsString::from("BAD\0VALUE"))],
        vec![(OsString::from("k".repeat(1_025)), OsString::from("v"))],
        vec![(
            OsString::from("KEY"),
            OsString::from("v".repeat(16 * 1_024 + 1)),
        )],
        vec![
            (OsString::from("DUPLICATE"), OsString::from("first")),
            (OsString::from("DUPLICATE"), OsString::from("second")),
        ],
        too_many,
        aggregate_overflow,
    ];
    for environment in invalid {
        let error = TerminalTool::with_executor(
            temporary.path(),
            environment,
            Arc::new(executor.clone()),
            TerminalLimits::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), TerminalConfigErrorKind::InvalidEnvironment);
        assert_eq!(
            error.to_string(),
            "native terminal environment snapshot is invalid"
        );
        let rendering = format!("{error:?} {error}");
        assert!(!rendering.contains("BAD"));
        assert!(!rendering.contains("DUPLICATE"));
        assert!(!rendering.contains("second"));
    }
    assert_eq!(executor.calls(), 0);
}

#[test]
fn injected_outcome_contract_rejects_impossible_stream_and_status_reports() {
    let too_large =
        TerminalCapturedOutput::new(vec![b'x'; 64 * 1_024 + 1], 64 * 1_024 + 1).unwrap_err();
    assert_eq!(too_large.kind(), TerminalExecutorErrorKind::InvalidResponse);
    let impossible_total = TerminalCapturedOutput::new(b"two".to_vec(), 2).unwrap_err();
    assert_eq!(
        impossible_total.kind(),
        TerminalExecutorErrorKind::InvalidResponse
    );

    let small = || TerminalCapturedOutput::new(Vec::new(), 0).unwrap();
    let overflow = || TerminalCapturedOutput::new(Vec::new(), 1024 * 1024 + 1).unwrap();
    let missing_overflow = TerminalExecutionOutcome::new(
        TerminalExecutionStatus::OutputLimit,
        small(),
        small(),
        Duration::ZERO,
    )
    .unwrap_err();
    assert_eq!(
        missing_overflow.kind(),
        TerminalExecutorErrorKind::InvalidResponse
    );
    let undeclared_overflow = TerminalExecutionOutcome::new(
        TerminalExecutionStatus::Exited(0),
        overflow(),
        small(),
        Duration::ZERO,
    )
    .unwrap_err();
    assert_eq!(
        undeclared_overflow.kind(),
        TerminalExecutorErrorKind::InvalidResponse
    );
    for (status, duration) in [
        (TerminalExecutionStatus::Exited(-1), Duration::ZERO),
        (TerminalExecutionStatus::Exited(256), Duration::ZERO),
        (TerminalExecutionStatus::Signaled(0), Duration::ZERO),
        (TerminalExecutionStatus::Signaled(256), Duration::ZERO),
        (
            TerminalExecutionStatus::Exited(0),
            Duration::from_secs(600) + Duration::from_millis(1),
        ),
    ] {
        let error = TerminalExecutionOutcome::new(status, small(), small(), duration).unwrap_err();
        assert_eq!(error.kind(), TerminalExecutorErrorKind::InvalidResponse);
    }
}

#[cfg(target_os = "linux")]
fn require_linux_executable(path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;

    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 => true,
        _ => {
            eprintln!("skipping terminal system evidence because {path} is unavailable");
            false
        }
    }
}

#[cfg(target_os = "linux")]
fn read_linux_pid(path: &std::path::Path) -> rustix::process::Pid {
    let pid = std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    rustix::process::Pid::from_raw(pid).expect("terminal process pid is positive")
}

#[cfg(target_os = "linux")]
struct EscapedProcessGuard {
    pid: rustix::process::Pid,
}

#[cfg(target_os = "linux")]
impl EscapedProcessGuard {
    fn new(pid: rustix::process::Pid) -> Self {
        Self { pid }
    }

    fn terminate(&self) -> bool {
        let _ = rustix::process::kill_process_group(self.pid, rustix::process::Signal::TERM);
        if self.wait_until_gone(Duration::from_millis(500)) {
            return true;
        }
        let _ = rustix::process::kill_process_group(self.pid, rustix::process::Signal::KILL);
        self.wait_until_gone(Duration::from_secs(2))
    }

    fn wait_until_gone(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            match rustix::process::test_kill_process(self.pid) {
                Err(error) if error == rustix::io::Errno::SRCH => return true,
                _ if Instant::now() >= deadline => return false,
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for EscapedProcessGuard {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_executor_runs_fixed_shell_in_selected_cwd_with_separate_streams() {
    let temporary = TemporaryDirectory::new("system");
    std::fs::create_dir(temporary.path().join("nested")).unwrap();
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let command = "if IFS= read -r ignored; then exit 99; fi; printf '%s' 'stdout bytes'; printf '%s' 'stderr bytes' >&2; exit 7";

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments(command, "nested"),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(output.is_error);
    assert_eq!(output.content["action"], "exec");
    assert_eq!(output.content["cwd"], "nested");
    assert_eq!(output.content["status"], "exited");
    assert_eq!(output.content["exit_code"], 7);
    assert_eq!(output.content["signal"], Value::Null);
    assert_eq!(output.content["stdout"], "stdout bytes");
    assert_eq!(output.content["stderr"], "stderr bytes");
    assert_eq!(output.content["stdout_bytes"], 12);
    assert_eq!(output.content["stderr_bytes"], 12);
    assert_eq!(output.content["stdout_truncated"], false);
    assert_eq!(output.content["stderr_truncated"], false);
    assert_eq!(output.content["stdout_lossy"], false);
    assert_eq!(output.content["stderr_lossy"], false);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_executor_reports_direct_shell_signal_status() {
    let temporary = TemporaryDirectory::new("system-signal");
    let tool = TerminalTool::open(temporary.path()).unwrap();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments("kill -TERM \"$$\"", "."),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(output.is_error);
    assert_eq!(output.content["status"], "signaled");
    assert_eq!(output.content["exit_code"], Value::Null);
    assert_eq!(output.content["signal"], 15);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_executor_terminates_on_aggregate_output_pressure() {
    if !require_linux_executable("/usr/bin/head") || !std::path::Path::new("/dev/zero").exists() {
        eprintln!("skipping terminal output-limit evidence because head or /dev/zero is absent");
        return;
    }
    let temporary = TemporaryDirectory::new("system-output-limit");
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let started = Instant::now();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments("/usr/bin/head -c 2097152 /dev/zero", "."),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(output.is_error);
    assert_eq!(output.content["status"], "output_limit");
    assert_eq!(output.content["exit_code"], Value::Null);
    assert_eq!(output.content["signal"], Value::Null);
    assert!(output.content["stdout_bytes"].as_u64().unwrap() > 1024 * 1024);
    assert_eq!(output.content["stderr_bytes"], 0);
    assert_eq!(output.content["stdout_truncated"], true);
    assert_eq!(output.content["stdout_lossy"], false);
    assert!(serde_json::to_vec(&output).unwrap().len() <= 48 * 1024);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_timeout_kills_a_term_ignoring_shell_before_publication() {
    if !require_linux_executable("/bin/sleep") {
        return;
    }
    let temporary = TemporaryDirectory::new("system-timeout");
    let tool = TerminalTool::open_with_limits(
        temporary.path(),
        TerminalLimits::new(Duration::from_millis(100), 1).unwrap(),
    )
    .unwrap();
    let started = Instant::now();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments(
            "trap '' TERM; printf '%s' \"$$\" > timeout.pid; while :; do /bin/sleep 1; done",
            ".",
        ),
        CancellationToken::new(),
    ))
    .unwrap();

    let pid = read_linux_pid(&temporary.path().join("timeout.pid"));
    let cleanup = EscapedProcessGuard::new(pid);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(output.is_error);
    assert_eq!(output.content["status"], "timed_out");
    assert_eq!(output.content["exit_code"], Value::Null);
    assert_eq!(output.content["signal"], Value::Null);
    assert_eq!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH),
        "timed-out TERM-ignoring shell survived output publication"
    );
    drop(cleanup);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_reader_cleanup_is_bounded_when_setsid_process_retains_pipe() {
    if !require_linux_executable("/usr/bin/setsid")
        || !require_linux_executable("/bin/sh")
        || !require_linux_executable("/bin/sleep")
    {
        return;
    }
    let temporary = TemporaryDirectory::new("system-setsid-pipe");
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let command = "/usr/bin/setsid /bin/sh -c 'printf \"%s\" \"$$\" > escaped.pid; exec /bin/sleep 30' & i=0; while [ ! -s escaped.pid ] && [ \"$i\" -lt 200 ]; do i=$((i + 1)); /bin/sleep 0.01; done; test -s escaped.pid";
    let started = Instant::now();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments(command, "."),
        CancellationToken::new(),
    ))
    .unwrap();

    let escaped = read_linux_pid(&temporary.path().join("escaped.pid"));
    let cleanup = EscapedProcessGuard::new(escaped);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!output.is_error);
    assert_eq!(output.content["status"], "exited");
    assert_eq!(output.content["exit_code"], 0);
    assert!(rustix::process::test_kill_process(escaped).is_ok());
    assert!(
        cleanup.terminate(),
        "escaped setsid test process did not terminate during explicit cleanup"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_ready_publication_has_already_terminated_background_group_members() {
    let temporary = TemporaryDirectory::new("system-ready-cleanup");
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments("/bin/sleep 60 & printf '%s' \"$!\" > descendant.pid", "."),
        CancellationToken::new(),
    ))
    .unwrap();
    assert!(!output.is_error);

    let pid = std::fs::read_to_string(temporary.path().join("descendant.pid"))
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    let pid = rustix::process::Pid::from_raw(pid).expect("terminal descendant pid is positive");
    assert_eq!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH),
        "terminal output was published before the background group member was gone"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn dropping_linux_system_execution_synchronously_terminates_the_owned_process_group() {
    let temporary = TemporaryDirectory::new("system-drop");
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let mut execution = Box::pin(tool.execute(
        context(),
        exact_arguments(
            "/bin/sleep 60 & descendant=$!; printf '%s %s' \"$$\" \"$descendant\" > owned.pids; wait",
            ".",
        ),
        CancellationToken::new(),
    ));
    assert!(poll_once(execution.as_mut()).is_pending());

    let pid_path = temporary.path().join("owned.pids");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let pids = loop {
        match std::fs::read_to_string(&pid_path) {
            Ok(pids) => {
                let pids = pids
                    .split_whitespace()
                    .map(|pid| pid.parse::<i32>().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(pids.len(), 2);
                break pids;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "terminal child did not publish its pid"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("failed to read terminal child pid: {error}"),
        }
    };
    let pids = pids
        .into_iter()
        .map(|pid| rustix::process::Pid::from_raw(pid).expect("terminal child pid is positive"))
        .collect::<Vec<_>>();
    for pid in &pids {
        assert!(rustix::process::test_kill_process(*pid).is_ok());
    }

    drop(execution);

    for pid in pids {
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH),
            "terminal future drop returned before its process group was gone"
        );
    }
}
