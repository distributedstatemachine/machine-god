#![cfg(target_os = "linux")]

use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_util::{Stream, StreamExt};
use machine_god_core::{
    Capability, ContentBlock, Engine, EngineEvent, Message, ModelEvent, PermissionDecision,
    PermissionGrantScope, Role, SessionId, SessionIncarnationId, StopReason, ToolCall, ToolCallId,
    ToolName, ToolOutput, TurnEvent,
};
use machine_god_native::{
    OPEN_FILE_TOOL_NAME, OpenFileLaunch, OpenFileLaunchOutcome, OpenFileLaunchRequest,
    OpenFileLauncher, OpenFileTool,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::json;

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-open-file-engine-{}-{identifier}",
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

#[derive(Clone)]
struct FakeLauncher {
    outcome: OpenFileLaunchOutcome,
    launch_calls: Arc<AtomicUsize>,
    future_polls: Arc<AtomicUsize>,
}

#[derive(Clone, Default)]
struct PendingLauncher {
    launch_calls: Arc<AtomicUsize>,
    future_polls: Arc<AtomicUsize>,
    future_drops: Arc<AtomicUsize>,
    retained_target: Arc<Mutex<Option<RetainedTarget>>>,
}

#[derive(Clone, Debug)]
struct RetainedTarget {
    proc_path: PathBuf,
    device: u64,
    inode: u64,
}

impl OpenFileLauncher for PendingLauncher {
    fn launch(
        &self,
        request: OpenFileLaunchRequest,
        _cancellation: machine_god_core::CancellationToken,
    ) -> OpenFileLaunch {
        Box::pin(PendingLaunchFuture {
            request: Some(request),
            launch_calls: Arc::clone(&self.launch_calls),
            future_polls: Arc::clone(&self.future_polls),
            future_drops: Arc::clone(&self.future_drops),
            retained_target: Arc::clone(&self.retained_target),
            started: false,
        })
    }
}

struct PendingLaunchFuture {
    request: Option<OpenFileLaunchRequest>,
    launch_calls: Arc<AtomicUsize>,
    future_polls: Arc<AtomicUsize>,
    future_drops: Arc<AtomicUsize>,
    retained_target: Arc<Mutex<Option<RetainedTarget>>>,
    started: bool,
}

impl Future for PendingLaunchFuture {
    type Output = OpenFileLaunchOutcome;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();
        this.future_polls.fetch_add(1, Ordering::SeqCst);
        if !this.started {
            let request = this
                .request
                .as_ref()
                .expect("pending launcher retains its request");
            let target = rustix::fs::fstat(request.target_fd()).unwrap();
            let proc_target = rustix::fs::stat(request.proc_path()).unwrap();
            assert_eq!(
                (proc_target.st_dev, proc_target.st_ino),
                (target.st_dev, target.st_ino)
            );
            *this.retained_target.lock().unwrap() = Some(RetainedTarget {
                proc_path: request.proc_path().to_owned(),
                device: target.st_dev,
                inode: target.st_ino,
            });
            this.launch_calls.fetch_add(1, Ordering::SeqCst);
            this.started = true;
        }
        Poll::Pending
    }
}

impl Drop for PendingLaunchFuture {
    fn drop(&mut self) {
        if self.started {
            self.future_drops.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl FakeLauncher {
    fn new(outcome: OpenFileLaunchOutcome) -> Self {
        Self {
            outcome,
            launch_calls: Arc::new(AtomicUsize::new(0)),
            future_polls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn launch_calls(&self) -> usize {
        self.launch_calls.load(Ordering::SeqCst)
    }

    fn future_polls(&self) -> usize {
        self.future_polls.load(Ordering::SeqCst)
    }
}

impl OpenFileLauncher for FakeLauncher {
    fn launch(
        &self,
        request: OpenFileLaunchRequest,
        _cancellation: machine_god_core::CancellationToken,
    ) -> OpenFileLaunch {
        let outcome = self.outcome;
        let launch_calls = Arc::clone(&self.launch_calls);
        let future_polls = Arc::clone(&self.future_polls);
        Box::pin(async move {
            launch_calls.fetch_add(1, Ordering::SeqCst);
            future_polls.fetch_add(1, Ordering::SeqCst);
            drop(request);
            outcome
        })
    }
}

fn provider(path: &str, name: &str) -> ScriptedModelProvider {
    let call = ToolCall {
        id: ToolCallId::new("open-file-call").unwrap(),
        name: ToolName::new(OPEN_FILE_TOOL_NAME).unwrap(),
        arguments: json!({ "path": path }),
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
            .prompt("open the requested workspace file")
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
        OPEN_FILE_TOOL_NAME
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
        Capability::OpenFile {
            path: path.to_owned()
        }
    );
}

fn next_event(turn: &mut machine_god_core::Turn) -> EngineEvent {
    futures_executor::block_on(turn.next()).unwrap().unwrap()
}

fn assert_turn_pending(turn: &mut machine_god_core::Turn) {
    let waker = futures_util::task::noop_waker();
    assert!(matches!(
        Pin::new(turn).poll_next(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
}

fn assert_original_descriptor_released(target: &RetainedTarget) {
    match rustix::fs::stat(&target.proc_path) {
        Err(error) if error == rustix::io::Errno::NOENT => {}
        Ok(metadata) => assert!(
            metadata.st_dev != target.device || metadata.st_ino != target.inode,
            "the proc fd path still resolves to the original target identity"
        ),
        Err(error) => panic!("unexpected proc fd inspection failure: {error}"),
    }
}

#[test]
fn engine_denial_authorizes_exact_open_file_capability_without_launch_or_tool_events() {
    let temporary = TemporaryDirectory::new();
    let private = "DENIED_OPEN_FILE_PRIVATE_BYTES";
    fs::write(temporary.path().join("secret.txt"), private).unwrap();
    let provider = provider("secret.txt", "native-open-file-denied");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Deny {
            reason: "denied by test policy".to_owned(),
        })]);
    let launcher = FakeLauncher::new(OpenFileLaunchOutcome::Accepted);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(OpenFileTool::open_with_launcher(temporary.path(), launcher.clone()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-open-file-denied");

    assert_completed(&events);
    assert_exact_capability(&policy, "secret.txt");
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
    )));
    assert_eq!(launcher.launch_calls(), 0);
    assert_eq!(launcher.future_polls(), 0);
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
    assert!(
        !serde_json::to_string(&provider.requests()[1].request)
            .unwrap()
            .contains(private)
    );
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    assert_eq!(
        fs::read_to_string(temporary.path().join("secret.txt")).unwrap(),
        private
    );
}

#[test]
fn engine_allow_orders_policy_before_launch_and_durably_returns_exact_result() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("nested")).unwrap();
    fs::write(temporary.path().join("nested/report.txt"), b"report").unwrap();
    let provider = provider("nested/report.txt", "native-open-file-allowed");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let launcher = FakeLauncher::new(OpenFileLaunchOutcome::Accepted);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(OpenFileTool::open_with_launcher(temporary.path(), launcher.clone()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-open-file-allowed");

    assert_completed(&events);
    assert_exact_capability(&policy, "nested/report.txt");
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
    assert_eq!(launcher.launch_calls(), 1);
    assert_eq!(launcher.future_polls(), 1);
    let expected = ToolOutput::success(json!({ "path": "nested/report.txt" }));
    assert!(matches!(
        &events[finished].payload,
        TurnEvent::ToolFinished { output, .. } if output == &expected
    ));
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(output, expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}

#[test]
fn engine_launcher_error_is_generic_retryable_and_durable_after_tool_events() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("report.txt"), b"report").unwrap();
    let provider = provider("report.txt", "native-open-file-launcher-error");
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let launcher = FakeLauncher::new(OpenFileLaunchOutcome::Unavailable);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(OpenFileTool::open_with_launcher(temporary.path(), launcher.clone()).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "native-open-file-launcher-error");

    assert_completed(&events);
    assert_exact_capability(&policy, "report.txt");
    assert_eq!(launcher.launch_calls(), 1);
    assert_eq!(launcher.future_polls(), 1);
    let expected = ToolOutput {
        content: json!({
            "code": "tool_error",
            "message": "tool execution failed",
            "retryable": true,
        }),
        is_error: true,
    };
    let finished = events
        .iter()
        .find_map(|event| match &event.payload {
            TurnEvent::ToolFinished { output, .. } => Some(output.clone()),
            _ => None,
        })
        .expect("allowed failing execution emits ToolFinished");
    assert_eq!(finished, expected);
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(output, expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}

#[test]
fn engine_cancellation_drops_pending_committed_launcher_and_closes_retained_target() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("report.txt"), b"report").unwrap();
    let provider = provider("report.txt", "native-open-file-cancelled");
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let launcher = PendingLauncher::default();
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::new())
        .permission_handler(policy.clone())
        .tool(OpenFileTool::open_with_launcher(temporary.path(), launcher.clone()).unwrap())
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("native-open-file-cancelled").unwrap(),
            SessionIncarnationId::new("incarnation-native-open-file-cancelled").unwrap(),
        )
        .unwrap();
    let mut turn = futures_executor::block_on(session.prompt("open the file")).unwrap();

    for _ in 0..6 {
        let _ = next_event(&mut turn);
    }
    assert_turn_pending(&mut turn);
    assert_exact_capability(&policy, "report.txt");
    assert_eq!(launcher.launch_calls.load(Ordering::SeqCst), 1);
    assert_eq!(launcher.future_polls.load(Ordering::SeqCst), 1);
    assert_eq!(launcher.future_drops.load(Ordering::SeqCst), 0);
    let retained_target = launcher
        .retained_target
        .lock()
        .unwrap()
        .clone()
        .expect("pending execution recorded its proc target");
    let retained = rustix::fs::stat(&retained_target.proc_path).unwrap();
    assert_eq!(
        (retained.st_dev, retained.st_ino),
        (retained_target.device, retained_target.inode)
    );

    assert!(turn.handle().cancel());
    assert!(matches!(
        next_event(&mut turn).payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(launcher.future_drops.load(Ordering::SeqCst), 1);
    assert_original_descriptor_released(&retained_target);
}
