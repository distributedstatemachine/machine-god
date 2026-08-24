#![cfg(target_os = "linux")]

use std::error::Error;
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use machine_god_core::{
    CancellationToken, Capability, SessionId, SessionIncarnationId, Tool, ToolCall, ToolCallId,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    MAX_CONCURRENT_OPEN_FILE_LAUNCHES, MAX_OPEN_FILE_PATH_BYTES,
    MAX_OPEN_FILE_PATH_COMPONENT_BYTES, MAX_OPEN_FILE_PATH_COMPONENTS,
    MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_OPEN_FILE_SERIALIZED_RESULT_BYTES,
    OPEN_FILE_LAUNCH_TIMEOUT, OPEN_FILE_TOOL_NAME, OpenFileLaunch, OpenFileLaunchOutcome,
    OpenFileLaunchRequest, OpenFileLauncher, OpenFileTool, OpenFileToolOpenErrorKind,
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
            let path = std::env::temp_dir()
                .join(format!("mg-open-file-{}-{identifier}", std::process::id()));
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

#[derive(Clone, Copy)]
enum LaunchMode {
    Ready(OpenFileLaunchOutcome),
    Pending,
    ResultUnknownAfterCancellation,
}

#[derive(Clone, Debug)]
struct LaunchRecord {
    path: String,
    proc_path: PathBuf,
    device: u64,
    inode: u64,
    identity_matches: bool,
    request_debug: String,
}

#[derive(Default)]
struct LauncherState {
    started: usize,
    polled: usize,
    dropped: usize,
    records: Vec<LaunchRecord>,
}

#[derive(Clone)]
struct FakeLauncher {
    mode: LaunchMode,
    state: Arc<Mutex<LauncherState>>,
}

impl FakeLauncher {
    fn new(mode: LaunchMode) -> Self {
        Self {
            mode,
            state: Arc::new(Mutex::new(LauncherState::default())),
        }
    }

    fn with_state<R>(&self, inspect: impl FnOnce(&LauncherState) -> R) -> R {
        inspect(&self.state.lock().unwrap())
    }
}

impl OpenFileLauncher for FakeLauncher {
    fn launch(
        &self,
        request: OpenFileLaunchRequest,
        cancellation: CancellationToken,
    ) -> OpenFileLaunch {
        Box::pin(FakeLaunchFuture {
            mode: self.mode,
            request: Some(request),
            cancellation,
            state: Arc::clone(&self.state),
            started: false,
        })
    }
}

struct FakeLaunchFuture {
    mode: LaunchMode,
    request: Option<OpenFileLaunchRequest>,
    cancellation: CancellationToken,
    state: Arc<Mutex<LauncherState>>,
    started: bool,
}

impl Future for FakeLaunchFuture {
    type Output = OpenFileLaunchOutcome;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();
        this.state.lock().unwrap().polled += 1;
        if !this.started {
            let request = this
                .request
                .as_ref()
                .expect("an unstarted fake launch retains its request");
            let target = rustix::fs::fstat(request.target_fd()).unwrap();
            let proc_target = rustix::fs::stat(request.proc_path()).unwrap();
            let record = LaunchRecord {
                path: request.path().to_owned(),
                proc_path: request.proc_path().to_owned(),
                device: target.st_dev,
                inode: target.st_ino,
                identity_matches: target.st_dev == proc_target.st_dev
                    && target.st_ino == proc_target.st_ino,
                request_debug: format!("{request:?}"),
            };
            let mut state = this.state.lock().unwrap();
            state.started += 1;
            state.records.push(record);
            this.started = true;
        }
        let outcome = match this.mode {
            LaunchMode::Ready(outcome) => Some(outcome),
            LaunchMode::ResultUnknownAfterCancellation if this.cancellation.is_cancelled() => {
                Some(OpenFileLaunchOutcome::ResultUnknown)
            }
            LaunchMode::Pending | LaunchMode::ResultUnknownAfterCancellation => None,
        };
        match outcome {
            Some(outcome) => {
                drop(this.request.take());
                Poll::Ready(outcome)
            }
            None => Poll::Pending,
        }
    }
}

impl Drop for FakeLaunchFuture {
    fn drop(&mut self) {
        if self.started {
            self.state.lock().unwrap().dropped += 1;
        }
    }
}

#[derive(Clone, Default)]
struct CaptureLauncher {
    request: Arc<Mutex<Option<OpenFileLaunchRequest>>>,
}

impl OpenFileLauncher for CaptureLauncher {
    fn launch(
        &self,
        request: OpenFileLaunchRequest,
        _cancellation: CancellationToken,
    ) -> OpenFileLaunch {
        Box::pin(CaptureLaunchFuture {
            request: Some(request),
            captured: Arc::clone(&self.request),
            started: false,
        })
    }
}

struct CaptureLaunchFuture {
    request: Option<OpenFileLaunchRequest>,
    captured: Arc<Mutex<Option<OpenFileLaunchRequest>>>,
    started: bool,
}

impl Future for CaptureLaunchFuture {
    type Output = OpenFileLaunchOutcome;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.started {
            *self.captured.lock().unwrap() = self.request.take();
            self.started = true;
        }
        Poll::Pending
    }
}

#[derive(Default)]
struct RetentionState {
    original_identity_retained: bool,
    replacement_identity_rejected: bool,
}

#[derive(Clone)]
struct RetentionLauncher {
    selected_parent: PathBuf,
    moved_parent: PathBuf,
    original_device: u64,
    original_inode: u64,
    state: Arc<Mutex<RetentionState>>,
}

impl OpenFileLauncher for RetentionLauncher {
    fn launch(
        &self,
        request: OpenFileLaunchRequest,
        _cancellation: CancellationToken,
    ) -> OpenFileLaunch {
        let selected_parent = self.selected_parent.clone();
        let moved_parent = self.moved_parent.clone();
        let original_device = self.original_device;
        let original_inode = self.original_inode;
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            fs::rename(&selected_parent, &moved_parent).unwrap();
            fs::remove_file(moved_parent.join("report.txt")).unwrap();
            fs::create_dir(&selected_parent).unwrap();
            let replacement = selected_parent.join("report.txt");
            fs::write(&replacement, b"replacement bytes").unwrap();

            let retained = rustix::fs::stat(request.proc_path()).unwrap();
            let replacement = fs::metadata(replacement).unwrap();
            let mut state = state.lock().unwrap();
            state.original_identity_retained =
                retained.st_dev == original_device && retained.st_ino == original_inode;
            state.replacement_identity_rejected =
                replacement.dev() != original_device || replacement.ino() != original_inode;
            drop(request);
            OpenFileLaunchOutcome::Accepted
        })
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    match poll_once(future.as_mut()) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("open_file execution unexpectedly remained pending"),
    }
}

fn tool(root: &Path, launcher: &FakeLauncher) -> OpenFileTool {
    OpenFileTool::open_with_launcher(root, launcher.clone()).unwrap()
}

fn tool_name(name: &str) -> ToolName {
    ToolName::new(name).unwrap()
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("open-file-call").unwrap(),
        name: tool_name(name),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("open-file-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("open-file-incarnation").unwrap(),
        turn_id: TurnId::new("open-file-turn").unwrap(),
        call_id: ToolCallId::new("open-file-call").unwrap(),
    }
}

fn execute(
    tool: &OpenFileTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_ready(tool.execute(context(), arguments, cancellation))
}

fn capture_launch_request(root: &Path, path: &str) -> OpenFileLaunchRequest {
    let launcher = CaptureLauncher::default();
    let tool = OpenFileTool::open_with_launcher(root, launcher.clone()).unwrap();
    let mut execution =
        Box::pin(tool.execute(context(), json!({ "path": path }), CancellationToken::new()));
    assert!(poll_once(execution.as_mut()).is_pending());
    let request = launcher
        .request
        .lock()
        .unwrap()
        .take()
        .expect("the first launcher-future poll captures its request");
    drop(execution);
    request
}

fn assert_original_descriptor_released(proc_path: &Path, device: u64, inode: u64) {
    match rustix::fs::stat(proc_path) {
        Err(error) if error == rustix::io::Errno::NOENT => {}
        Ok(metadata) => assert!(
            metadata.st_dev != device || metadata.st_ino != inode,
            "the proc fd path still resolves to the original target identity"
        ),
        Err(error) => panic!("unexpected proc fd inspection failure: {error}"),
    }
}

fn assert_tool_error(
    error: ToolError,
    kind: ToolErrorKind,
    code: &str,
    message: &str,
    retryable: bool,
) {
    let display = error.to_string();
    let debug = format!("{error:?}");
    let ToolError {
        kind: actual_kind,
        code: actual_code,
        message: actual_message,
        retryable: actual_retryable,
    } = error;
    assert_eq!(actual_kind, kind);
    assert_eq!(actual_code, code);
    assert_eq!(actual_message, message);
    assert_eq!(actual_retryable, retryable);
    assert_eq!(display, format!("{code}: {message}"));
    assert_eq!(
        debug,
        format!(
            "ToolError {{ kind: {kind:?}, code: \"{code}\", message: \"{message}\", retryable: {retryable} }}"
        )
    );
}

fn assert_invalid_arguments(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "open_file_invalid_arguments",
        "open_file arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "open_file_invalid_path",
        "open_file path is invalid",
        false,
    );
}

#[test]
fn exported_contract_spec_and_constructor_errors_are_exact_and_redacted() {
    assert_eq!(OPEN_FILE_TOOL_NAME, "open_file");
    assert_eq!(MAX_OPEN_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_OPEN_FILE_PATH_COMPONENT_BYTES, 255);
    assert_eq!(MAX_OPEN_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES, 65_536);
    assert_eq!(MAX_OPEN_FILE_SERIALIZED_RESULT_BYTES, 16_384);
    assert_eq!(MAX_CONCURRENT_OPEN_FILE_LAUNCHES, 32);
    assert_eq!(OPEN_FILE_LAUNCH_TIMEOUT, Duration::from_secs(30));

    let temporary = TemporaryDirectory::new();
    let launcher = FakeLauncher::new(LaunchMode::Ready(OpenFileLaunchOutcome::Accepted));
    let spec = tool(temporary.path(), &launcher).spec();
    assert_eq!(spec.name.as_str(), OPEN_FILE_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Open one existing regular file within the configured workspace in the desktop default application"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative regular-file path to open"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    );

    let private = "PRIVATE_OPEN_FILE_ROOT_DO_NOT_REFLECT";
    for (root, kind, display) in [
        (
            PathBuf::from(private),
            OpenFileToolOpenErrorKind::InvalidRoot,
            "native open_file workspace root is invalid",
        ),
        (
            temporary.path().join(private),
            OpenFileToolOpenErrorKind::Unavailable,
            "native open_file workspace root is unavailable",
        ),
    ] {
        let error = OpenFileTool::open_with_launcher(&root, launcher.clone()).unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.to_string(), display);
        assert!(error.source().is_none());
        assert!(!error.to_string().contains(private));
        assert!(!format!("{error:?}").contains(private));
    }

    let file_root = temporary.path().join(private);
    fs::write(&file_root, b"not a directory").unwrap();
    let error = OpenFileTool::open_with_launcher(&file_root, launcher.clone()).unwrap_err();
    assert_eq!(error.kind(), OpenFileToolOpenErrorKind::InvalidFileType);
    assert_eq!(
        error.to_string(),
        "native open_file workspace root is not a directory"
    );
    assert_eq!(
        format!("{error:?}"),
        "OpenFileToolOpenError { kind: InvalidFileType }"
    );

    let link_root = temporary.path().join("root-link");
    symlink(temporary.path(), &link_root).unwrap();
    let error = OpenFileTool::open_with_launcher(&link_root, launcher).unwrap_err();
    assert_eq!(error.kind(), OpenFileToolOpenErrorKind::InvalidFileType);
}

#[test]
fn prepare_is_effect_free_strict_and_uses_the_exact_open_file_capability() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("report.txt"), b"private").unwrap();
    let launcher = FakeLauncher::new(LaunchMode::Ready(OpenFileLaunchOutcome::Accepted));
    let tool = tool(temporary.path(), &launcher);

    let prepared = tool
        .prepare(call(OPEN_FILE_TOOL_NAME, json!({ "path": "report.txt" })))
        .unwrap();
    assert_eq!(
        prepared.capability(),
        &Capability::OpenFile {
            path: "report.txt".to_owned()
        }
    );
    assert_eq!(prepared.arguments(), &json!({ "path": "report.txt" }));
    assert_eq!(
        serde_json::to_value(prepared.capability()).unwrap(),
        json!({ "type": "open_file", "path": "report.txt" })
    );
    launcher.with_state(|state| {
        assert_eq!(state.started, 0);
        assert_eq!(state.polled, 0);
    });

    for invalid in [
        call("another_tool", json!({ "path": "report.txt" })),
        call(OPEN_FILE_TOOL_NAME, json!(null)),
        call(OPEN_FILE_TOOL_NAME, json!([])),
        call(OPEN_FILE_TOOL_NAME, json!({})),
        call(OPEN_FILE_TOOL_NAME, json!({ "path": null })),
        call(OPEN_FILE_TOOL_NAME, json!({ "path": 1 })),
        call(OPEN_FILE_TOOL_NAME, json!({ "path": ["report.txt"] })),
        call(
            OPEN_FILE_TOOL_NAME,
            json!({ "path": "report.txt", "extra": true }),
        ),
    ] {
        assert_invalid_arguments(tool.prepare(invalid).unwrap_err());
    }
}

#[test]
fn canonical_validation_rejects_aliases_controls_and_directional_text() {
    let temporary = TemporaryDirectory::new();
    let launcher = FakeLauncher::new(LaunchMode::Ready(OpenFileLaunchOutcome::Accepted));
    let tool = tool(temporary.path(), &launcher);
    let invalid = [
        "",
        ".",
        "..",
        "./file",
        "dir/./file",
        "dir/../file",
        "dir//file",
        "dir/",
        "/file",
        "~file",
        "line\nbreak",
        "tab\tfile",
        "nul\0file",
        "bidi\u{061c}file",
        "bidi\u{200e}file",
        "bidi\u{200f}file",
        "bidi\u{2028}file",
        "bidi\u{202e}file",
        "bidi\u{2066}file",
        "bidi\u{2069}file",
    ];
    for path in invalid {
        assert_invalid_path(
            tool.prepare(call(OPEN_FILE_TOOL_NAME, json!({ "path": path })))
                .unwrap_err(),
        );
    }

    for path in [r"directory\file.txt", " leading and trailing ", "λ.txt"] {
        let prepared = tool
            .prepare(call(OPEN_FILE_TOOL_NAME, json!({ "path": path })))
            .unwrap();
        assert_eq!(prepared.arguments(), &json!({ "path": path }));
    }
    assert_eq!(launcher.with_state(|state| state.started), 0);
}

#[test]
fn every_path_bound_accepts_its_edge_and_rejects_one_more() {
    let temporary = TemporaryDirectory::new();
    let launcher = FakeLauncher::new(LaunchMode::Ready(OpenFileLaunchOutcome::Accepted));
    let tool = tool(temporary.path(), &launcher);

    let exact_component = format!("{}x", "λ".repeat(127));
    assert_eq!(exact_component.len(), MAX_OPEN_FILE_PATH_COMPONENT_BYTES);
    tool.prepare(call(
        OPEN_FILE_TOOL_NAME,
        json!({ "path": exact_component }),
    ))
    .unwrap();
    let oversized_component = "λ".repeat(128);
    assert_eq!(
        oversized_component.len(),
        MAX_OPEN_FILE_PATH_COMPONENT_BYTES + 1
    );
    assert_invalid_path(
        tool.prepare(call(
            OPEN_FILE_TOOL_NAME,
            json!({ "path": oversized_component }),
        ))
        .unwrap_err(),
    );

    let exact_component_count_path = std::iter::repeat_n("a", MAX_OPEN_FILE_PATH_COMPONENTS)
        .collect::<Vec<_>>()
        .join("/");
    tool.prepare(call(
        OPEN_FILE_TOOL_NAME,
        json!({ "path": exact_component_count_path }),
    ))
    .unwrap();
    let oversized_component_count_path =
        std::iter::repeat_n("a", MAX_OPEN_FILE_PATH_COMPONENTS + 1)
            .collect::<Vec<_>>()
            .join("/");
    assert_invalid_path(
        tool.prepare(call(
            OPEN_FILE_TOOL_NAME,
            json!({ "path": oversized_component_count_path }),
        ))
        .unwrap_err(),
    );

    let mut max_components = vec!["a".repeat(255); 15];
    max_components.push("b".repeat(254));
    max_components.push("c".to_owned());
    let max_path = max_components.join("/");
    assert_eq!(max_path.len(), MAX_OPEN_FILE_PATH_BYTES);
    let max_arguments = json!({ "path": max_path });
    assert!(
        serde_json::to_vec(&max_arguments).unwrap().len()
            <= MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES
    );
    tool.prepare(call(OPEN_FILE_TOOL_NAME, max_arguments.clone()))
        .unwrap();
    let mut over_path = max_arguments["path"].as_str().unwrap().to_owned();
    over_path.push('d');
    assert_invalid_path(
        tool.prepare(call(OPEN_FILE_TOOL_NAME, json!({ "path": over_path })))
            .unwrap_err(),
    );

    let very_large_path = "a".repeat(1024 * 1024);
    assert!(
        serde_json::to_vec(&json!({ "path": &very_large_path }))
            .unwrap()
            .len()
            > MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES
    );
    assert_invalid_path(
        tool.prepare(call(
            OPEN_FILE_TOOL_NAME,
            json!({ "path": &very_large_path }),
        ))
        .unwrap_err(),
    );
    assert_invalid_path(
        execute(
            &tool,
            json!({ "path": very_large_path }),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_eq!(launcher.with_state(|state| state.started), 0);
}

#[test]
fn accepted_launch_uses_the_exact_descriptor_bound_proc_identity_and_result() {
    let temporary = TemporaryDirectory::new();
    let requested = r"nested/space λ\literal.txt";
    fs::create_dir(temporary.path().join("nested")).unwrap();
    fs::write(temporary.path().join(requested), b"private bytes").unwrap();
    let launcher = FakeLauncher::new(LaunchMode::Ready(OpenFileLaunchOutcome::Accepted));
    let tool = tool(temporary.path(), &launcher);

    let output = execute(
        &tool,
        json!({ "path": requested }),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(output, ToolOutput::success(json!({ "path": requested })));
    assert!(serde_json::to_vec(&output).unwrap().len() <= MAX_OPEN_FILE_SERIALIZED_RESULT_BYTES);
    launcher.with_state(|state| {
        assert_eq!(state.started, 1);
        assert_eq!(state.polled, 1);
        assert_eq!(state.dropped, 1);
        assert_eq!(state.records[0].path, requested);
        assert!(state.records[0].identity_matches);
        assert_eq!(
            state.records[0].request_debug,
            "OpenFileLaunchRequest { .. }"
        );
        assert!(
            state.records[0]
                .proc_path
                .starts_with(format!("/proc/{}/fd/", std::process::id()))
        );
    });
}

#[test]
fn filesystem_resolution_rejects_missing_symlinks_directories_and_special_files() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    let outside_sentinel = outside.path().join("outside.txt");
    fs::write(&outside_sentinel, b"outside sentinel").unwrap();
    fs::create_dir(temporary.path().join("directory")).unwrap();
    fs::write(temporary.path().join("target.txt"), b"target").unwrap();
    symlink("target.txt", temporary.path().join("file-link")).unwrap();
    symlink("directory", temporary.path().join("directory-link")).unwrap();
    symlink(&outside_sentinel, temporary.path().join("outside-link")).unwrap();
    let _socket = UnixListener::bind(temporary.path().join("socket")).unwrap();
    let fifo = temporary.path().join("fifo");
    let status = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(status.success());
    let launcher = FakeLauncher::new(LaunchMode::Ready(OpenFileLaunchOutcome::Accepted));
    let tool = tool(temporary.path(), &launcher);

    assert_tool_error(
        execute(
            &tool,
            json!({ "path": "missing" }),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::Unavailable,
        "open_file_not_found",
        "requested file is unavailable",
        false,
    );
    for path in ["directory", "file-link", "outside-link", "socket", "fifo"] {
        assert_tool_error(
            execute(&tool, json!({ "path": path }), CancellationToken::new()).unwrap_err(),
            ToolErrorKind::Execution,
            "open_file_not_regular_file",
            "requested path is not a regular file",
            false,
        );
    }
    assert_tool_error(
        execute(
            &tool,
            json!({ "path": "directory-link/child" }),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::PermissionDenied,
        "open_file_path_rejected",
        "requested file path is not confined",
        false,
    );
    assert_eq!(launcher.with_state(|state| state.started), 0);
    assert_eq!(fs::read(&outside_sentinel).unwrap(), b"outside sentinel");
}

#[test]
fn retained_target_identity_survives_ancestor_rename_unlink_and_path_replacement() {
    let temporary = TemporaryDirectory::new();
    let selected_parent = temporary.path().join("selected");
    let moved_parent = temporary.path().join("moved");
    fs::create_dir(&selected_parent).unwrap();
    let selected = selected_parent.join("report.txt");
    fs::write(&selected, b"original bytes").unwrap();
    let original = fs::metadata(&selected).unwrap();
    let state = Arc::new(Mutex::new(RetentionState::default()));
    let launcher = RetentionLauncher {
        selected_parent,
        moved_parent,
        original_device: original.dev(),
        original_inode: original.ino(),
        state: Arc::clone(&state),
    };
    let tool = OpenFileTool::open_with_launcher(temporary.path(), launcher).unwrap();

    assert_eq!(
        execute(
            &tool,
            json!({ "path": "selected/report.txt" }),
            CancellationToken::new(),
        )
        .unwrap(),
        ToolOutput::success(json!({ "path": "selected/report.txt" }))
    );
    let state = state.lock().unwrap();
    assert!(state.original_identity_retained);
    assert!(state.replacement_identity_rejected);
    assert_eq!(
        fs::read(temporary.path().join("selected/report.txt")).unwrap(),
        b"replacement bytes"
    );
}

#[test]
fn retained_root_survives_rename_but_rejects_unlinked_root_replacement() {
    let parent = TemporaryDirectory::new();
    let workspace = parent.path().join("workspace");
    let retained = parent.path().join("retained");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("report.txt"), b"retained").unwrap();
    let launcher = FakeLauncher::new(LaunchMode::Ready(OpenFileLaunchOutcome::Accepted));
    let retained_tool = tool(&workspace, &launcher);
    fs::rename(&workspace, &retained).unwrap();
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("report.txt"), b"replacement").unwrap();
    assert_eq!(
        execute(
            &retained_tool,
            json!({ "path": "report.txt" }),
            CancellationToken::new(),
        )
        .unwrap(),
        ToolOutput::success(json!({ "path": "report.txt" }))
    );

    let unlinked = parent.path().join("unlinked");
    fs::create_dir(&unlinked).unwrap();
    let never = FakeLauncher::new(LaunchMode::Ready(OpenFileLaunchOutcome::Accepted));
    let stale_tool = tool(&unlinked, &never);
    fs::remove_dir(&unlinked).unwrap();
    fs::create_dir(&unlinked).unwrap();
    fs::write(unlinked.join("decoy"), b"replacement").unwrap();
    assert_tool_error(
        execute(
            &stale_tool,
            json!({ "path": "decoy" }),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::Unavailable,
        "open_file_unavailable",
        "requested file is unavailable",
        true,
    );
    assert_eq!(never.with_state(|state| state.started), 0);
}

#[test]
fn launcher_outcomes_map_to_exact_fixed_errors_without_path_disclosure() {
    let temporary = TemporaryDirectory::new();
    let private = "PRIVATE_LAUNCH_PATH.txt";
    fs::write(temporary.path().join(private), b"private").unwrap();
    for (outcome, kind, code, message, retryable) in [
        (
            OpenFileLaunchOutcome::Cancelled,
            ToolErrorKind::Cancelled,
            "open_file_cancelled",
            "open_file execution was cancelled",
            false,
        ),
        (
            OpenFileLaunchOutcome::Unavailable,
            ToolErrorKind::Unavailable,
            "open_file_launcher_unavailable",
            "native file launcher is unavailable",
            true,
        ),
        (
            OpenFileLaunchOutcome::ResultUnknown,
            ToolErrorKind::Execution,
            "open_file_result_unknown",
            "requested file open status is uncertain",
            false,
        ),
    ] {
        let launcher = FakeLauncher::new(LaunchMode::Ready(outcome));
        let error = execute(
            &tool(temporary.path(), &launcher),
            json!({ "path": private }),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert!(!error.to_string().contains(private));
        assert!(!format!("{error:?}").contains(private));
        assert_tool_error(error, kind, code, message, retryable);
    }
}

#[test]
fn execution_and_launcher_constructors_are_inert_and_drop_closes_the_request() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("report.txt"), b"private").unwrap();
    let pending = FakeLauncher::new(LaunchMode::Pending);
    let tool = tool(temporary.path(), &pending);

    let unpolled = tool.execute(
        context(),
        json!({ "path": "report.txt" }),
        CancellationToken::new(),
    );
    assert_eq!(pending.with_state(|state| state.started), 0);
    drop(unpolled);
    assert_eq!(pending.with_state(|state| state.started), 0);

    let request = capture_launch_request(temporary.path(), "report.txt");
    let original = rustix::fs::fstat(request.target_fd()).unwrap();
    let proc_path = request.proc_path().to_owned();
    let unpolled_launch = pending.launch(request, CancellationToken::new());
    pending.with_state(|state| {
        assert_eq!(state.started, 0);
        assert_eq!(state.polled, 0);
        assert_eq!(state.dropped, 0);
        assert!(state.records.is_empty());
    });
    let retained = rustix::fs::stat(&proc_path).unwrap();
    assert_eq!(
        (retained.st_dev, retained.st_ino),
        (original.st_dev, original.st_ino)
    );
    drop(unpolled_launch);
    pending.with_state(|state| {
        assert_eq!(state.started, 0);
        assert_eq!(state.polled, 0);
        assert_eq!(state.dropped, 0);
        assert!(state.records.is_empty());
    });
    assert_original_descriptor_released(&proc_path, original.st_dev, original.st_ino);

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    assert_tool_error(
        execute(&tool, json!({ "path": "report.txt" }), cancellation).unwrap_err(),
        ToolErrorKind::Cancelled,
        "open_file_cancelled",
        "open_file execution was cancelled",
        false,
    );
    assert_eq!(pending.with_state(|state| state.started), 0);

    let mut execution = Box::pin(tool.execute(
        context(),
        json!({ "path": "report.txt" }),
        CancellationToken::new(),
    ));
    assert!(poll_once(execution.as_mut()).is_pending());
    let record = pending.with_state(|state| {
        assert_eq!(state.started, 1);
        assert_eq!(state.polled, 1);
        assert_eq!(state.dropped, 0);
        state.records[0].clone()
    });
    let retained = rustix::fs::stat(&record.proc_path).unwrap();
    assert_eq!(
        (retained.st_dev, retained.st_ino),
        (record.device, record.inode)
    );
    drop(execution);
    assert_original_descriptor_released(&record.proc_path, record.device, record.inode);
    assert_eq!(pending.with_state(|state| state.dropped), 1);
}

#[test]
fn cancellation_after_launch_is_reported_as_unknown_by_the_injected_lifecycle() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("report.txt"), b"private").unwrap();
    let launcher = FakeLauncher::new(LaunchMode::ResultUnknownAfterCancellation);
    let tool = tool(temporary.path(), &launcher);
    let cancellation = CancellationToken::new();
    let mut execution = Box::pin(tool.execute(
        context(),
        json!({ "path": "report.txt" }),
        cancellation.clone(),
    ));
    assert!(poll_once(execution.as_mut()).is_pending());
    assert!(cancellation.cancel());
    let Poll::Ready(result) = poll_once(execution.as_mut()) else {
        panic!("cancellation-aware fake launcher did not finish")
    };
    assert_tool_error(
        result.unwrap_err(),
        ToolErrorKind::Execution,
        "open_file_result_unknown",
        "requested file open status is uncertain",
        false,
    );
    launcher.with_state(|state| {
        assert_eq!(state.started, 1);
        assert_eq!(state.polled, 2);
        assert_eq!(state.dropped, 1);
    });
}

#[test]
fn concurrent_calls_keep_paths_descriptors_results_and_lifecycles_isolated() {
    const CALL_COUNT: usize = 32;

    let temporary = TemporaryDirectory::new();
    let mut expected_paths = Vec::with_capacity(CALL_COUNT);
    for index in 0..CALL_COUNT {
        let path = format!("report-{index:02}.txt");
        fs::write(temporary.path().join(&path), format!("private {index}")).unwrap();
        expected_paths.push(path);
    }
    let launcher = FakeLauncher::new(LaunchMode::Ready(OpenFileLaunchOutcome::Accepted));
    let tool = Arc::new(tool(temporary.path(), &launcher));
    let barrier = Arc::new(std::sync::Barrier::new(CALL_COUNT));
    let threads = expected_paths
        .clone()
        .into_iter()
        .map(|path| {
            let tool = Arc::clone(&tool);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                execute(
                    &tool,
                    json!({ "path": path.clone() }),
                    CancellationToken::new(),
                )
                .map(|output| (path, output))
            })
        })
        .collect::<Vec<_>>();

    let mut results = threads
        .into_iter()
        .map(|thread| thread.join().unwrap().unwrap())
        .collect::<Vec<_>>();
    results.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        results,
        expected_paths
            .iter()
            .map(|path| { (path.clone(), ToolOutput::success(json!({ "path": path })),) })
            .collect::<Vec<_>>()
    );
    launcher.with_state(|state| {
        assert_eq!(state.started, CALL_COUNT);
        assert_eq!(state.polled, CALL_COUNT);
        assert_eq!(state.dropped, CALL_COUNT);
        let mut paths = state
            .records
            .iter()
            .map(|record| record.path.clone())
            .collect::<Vec<_>>();
        paths.sort();
        assert_eq!(paths, expected_paths);
        assert!(state.records.iter().all(|record| record.identity_matches));
    });
}
