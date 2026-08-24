use std::error::Error;
use std::fmt;
use std::io;
use std::path::Path;
use std::time::Duration;

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, PreparedToolCall, Tool, ToolCall, ToolContext,
    ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Value, json};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::OwnedFd;
#[cfg(target_os = "linux")]
use rustix::fd::{AsFd, AsRawFd, BorrowedFd};
#[cfg(target_os = "linux")]
use rustix::fs::{FileType, Mode, OFlags};

#[cfg(target_os = "linux")]
use std::future::Future;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::pin::Pin;
#[cfg(target_os = "linux")]
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "linux")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "linux")]
use std::task::{Context, Poll, Waker};
#[cfg(target_os = "linux")]
use std::thread::{self, JoinHandle};
#[cfg(target_os = "linux")]
use std::time::Instant;

/// Maximum UTF-8 bytes accepted in a requested or canonical file path.
pub const MAX_OPEN_FILE_PATH_BYTES: usize = 4 * 1024;
/// Maximum components accepted in a canonical file path.
pub const MAX_OPEN_FILE_PATH_COMPONENTS: usize = 256;
/// Maximum UTF-8 bytes accepted in one canonical path component.
pub const MAX_OPEN_FILE_PATH_COMPONENT_BYTES: usize = 255;
/// Maximum serialized byte size of accepted arguments.
pub const MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES: usize = 64 * 1024;
/// Maximum serialized [`ToolOutput`] bytes produced by this tool.
pub const MAX_OPEN_FILE_SERIALIZED_RESULT_BYTES: usize = 16 * 1024;
/// Maximum wait before the direct desktop helper is terminated.
pub const OPEN_FILE_LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Registered name of [`OpenFileTool`].
pub const OPEN_FILE_TOOL_NAME: &str = "open_file";

const OPEN_FILE_DESCRIPTION: &str = "Open one existing regular file within the configured workspace in the desktop default application";
const PATH_DESCRIPTION: &str = "Workspace-relative regular-file path to open";

/// Stable category for failure to acquire an open-file workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenFileToolOpenErrorKind {
    /// Native desktop launch is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire an [`OpenFileTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct OpenFileToolOpenError {
    kind: OpenFileToolOpenErrorKind,
}

impl OpenFileToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> OpenFileToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: OpenFileToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for OpenFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenFileToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for OpenFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            OpenFileToolOpenErrorKind::UnsupportedPlatform => {
                "native open_file is unsupported on this platform"
            }
            OpenFileToolOpenErrorKind::InvalidRoot => "native open_file workspace root is invalid",
            OpenFileToolOpenErrorKind::InvalidFileType => {
                "native open_file workspace root is not a directory"
            }
            OpenFileToolOpenErrorKind::Unavailable => {
                "native open_file workspace root is unavailable"
            }
        })
    }
}

impl Error for OpenFileToolOpenError {}

/// Result of one trusted Linux launcher attempt.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OpenFileLaunchOutcome {
    /// The direct helper exited successfully and accepted the request.
    Accepted,
    /// Cancellation won before a helper was successfully spawned.
    Cancelled,
    /// No helper was spawned because the launcher was unavailable.
    Unavailable,
    /// A helper was spawned, so the external effect can no longer be known.
    ResultUnknown,
}

/// Owned target passed to an explicitly injected Linux launcher.
#[cfg(target_os = "linux")]
pub struct OpenFileLaunchRequest {
    path: String,
    proc_path: PathBuf,
    target: OwnedFd,
}

#[cfg(target_os = "linux")]
impl OpenFileLaunchRequest {
    /// Returns the exact canonical workspace-relative path approved by policy.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the descriptor-bound proc path supplied to the system helper.
    #[must_use]
    pub fn proc_path(&self) -> &Path {
        &self.proc_path
    }

    /// Borrows the exact retained target descriptor.
    #[must_use]
    pub fn target_fd(&self) -> BorrowedFd<'_> {
        self.target.as_fd()
    }
}

#[cfg(target_os = "linux")]
impl fmt::Debug for OpenFileLaunchRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenFileLaunchRequest")
            .finish_non_exhaustive()
    }
}

/// Sendable future returned by an injected Linux file launcher.
#[cfg(target_os = "linux")]
pub type OpenFileLaunch = BoxFuture<'static, OpenFileLaunchOutcome>;

/// Trusted host boundary for one descriptor-bound Linux desktop launch.
///
/// Calling [`OpenFileLauncher::launch`] must be effect-free. Implementations
/// start work only when the returned future is polled, retain the complete
/// request until their direct helper is reaped, and observe `cancellation`.
/// Dropping the future must synchronously stop and reap every owned helper.
/// Implementations may let an already-complete worker return from the tail of
/// an inline wake callback when joining that worker would be a self-join.
#[cfg(target_os = "linux")]
pub trait OpenFileLauncher: Send + Sync + 'static {
    /// Creates an inert owned launch future for `request`.
    fn launch(
        &self,
        request: OpenFileLaunchRequest,
        cancellation: CancellationToken,
    ) -> OpenFileLaunch;
}

/// Native `open_file` tool confined to one retained workspace root.
pub struct OpenFileTool {
    #[cfg(target_os = "linux")]
    root: OwnedFd,
    #[cfg(target_os = "linux")]
    launcher: Arc<dyn OpenFileLauncher>,
    #[cfg(target_os = "macos")]
    _root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl OpenFileTool {
    #[cfg(any(
        target_os = "linux",
        all(target_os = "macos", feature = "ai-gateway-http")
    ))]
    pub(crate) fn from_root_descriptor(root: OwnedFd) -> Self {
        #[cfg(target_os = "linux")]
        {
            Self {
                root,
                launcher: Arc::new(SystemOpenFileLauncher::default()),
            }
        }
        #[cfg(target_os = "macos")]
        {
            Self { _root: root }
        }
    }

    /// Opens and retains an absolute Linux workspace directory.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted failure when the platform is unsupported, the
    /// path is relative, or the root cannot be retained as a real directory.
    pub fn open(root: &Path) -> Result<Self, OpenFileToolOpenError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = root;
            Err(OpenFileToolOpenError::new(
                OpenFileToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(target_os = "linux")]
        {
            let descriptor = open_workspace_root(root)?;
            Ok(Self::from_root_descriptor(descriptor))
        }
    }

    /// Opens a Linux workspace with an explicitly injected trusted launcher.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted failure when the root is not an absolute real
    /// directory that can be retained without following its final component.
    #[cfg(target_os = "linux")]
    pub fn open_with_launcher(
        root: &Path,
        launcher: impl OpenFileLauncher,
    ) -> Result<Self, OpenFileToolOpenError> {
        Ok(Self {
            root: open_workspace_root(root)?,
            launcher: Arc::new(launcher),
        })
    }
}

impl fmt::Debug for OpenFileTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenFileTool")
            .finish_non_exhaustive()
    }
}

struct ValidatedArguments {
    path: String,
}

impl Tool for OpenFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: open_file_name(),
            description: OPEN_FILE_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": PATH_DESCRIPTION
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != open_file_name() {
            return Err(invalid_arguments());
        }
        let arguments = validate_arguments(&call.arguments)?;
        let path = arguments.path;
        let prepared_arguments = json!({ "path": path });
        if !serialized_value_fits(&prepared_arguments, MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES) {
            return Err(invalid_arguments());
        }
        let path = prepared_arguments["path"]
            .as_str()
            .expect("prepared open_file path is a string")
            .to_owned();
        Ok(PreparedToolCall::new(
            Capability::OpenFile { path },
            prepared_arguments,
        ))
    }

    fn execute(
        &self,
        _context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            let arguments = validate_arguments(&arguments)?;

            #[cfg(not(target_os = "linux"))]
            {
                let _ = (arguments, cancellation);
                Err(unsupported_platform())
            }

            #[cfg(target_os = "linux")]
            {
                self.execute_linux(arguments.path, cancellation).await
            }
        })
    }
}

fn validate_arguments(arguments: &Value) -> Result<ValidatedArguments, ToolError> {
    let Value::Object(object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 1 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(path)) = object.get("path") else {
        return Err(invalid_arguments());
    };
    if !serialized_value_fits(arguments, MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES) {
        return Err(invalid_arguments());
    }
    validate_canonical_path(path)?;
    Ok(ValidatedArguments { path: path.clone() })
}

fn validate_canonical_path(path: &str) -> Result<(), ToolError> {
    if path.is_empty()
        || path.len() > MAX_OPEN_FILE_PATH_BYTES
        || path.starts_with('/')
        || path.starts_with('~')
        || path.ends_with('/')
        || path.chars().any(is_forbidden_path_character)
    {
        return Err(invalid_path());
    }

    let mut count = 0_usize;
    for component in path.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.len() > MAX_OPEN_FILE_PATH_COMPONENT_BYTES
        {
            return Err(invalid_path());
        }
        count = count.checked_add(1).ok_or_else(invalid_path)?;
        if count > MAX_OPEN_FILE_PATH_COMPONENTS {
            return Err(invalid_path());
        }
    }
    Ok(())
}

fn is_forbidden_path_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

fn open_file_name() -> ToolName {
    ToolName::new(OPEN_FILE_TOOL_NAME).expect("open_file is a valid tool name")
}

fn serialized_value_fits(value: &(impl serde::Serialize + ?Sized), limit: usize) -> bool {
    let mut writer = JsonByteCounter { written: 0, limit };
    serde_json::to_writer(&mut writer, value).is_ok()
}

struct JsonByteCounter {
    written: usize,
    limit: usize,
}

impl io::Write for JsonByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(written) = self.written.checked_add(bytes.len()) else {
            return Err(io::Error::other("serialized JSON byte count overflowed"));
        };
        if written > self.limit {
            return Err(io::Error::other("serialized JSON exceeded its byte limit"));
        }
        self.written = written;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn open_workspace_root(root: &Path) -> Result<OwnedFd, OpenFileToolOpenError> {
    let lexical_root = root.components().collect::<PathBuf>();
    if !lexical_root.is_absolute() {
        return Err(OpenFileToolOpenError::new(
            OpenFileToolOpenErrorKind::InvalidRoot,
        ));
    }
    let descriptor = rustix::fs::open(
        &lexical_root,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(map_root_open_error)?;
    let metadata = rustix::fs::fstat(&descriptor)
        .map_err(|_| OpenFileToolOpenError::new(OpenFileToolOpenErrorKind::Unavailable))?;
    if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
        return Err(OpenFileToolOpenError::new(
            OpenFileToolOpenErrorKind::InvalidFileType,
        ));
    }
    Ok(descriptor)
}

#[cfg(target_os = "linux")]
fn map_root_open_error(error: rustix::io::Errno) -> OpenFileToolOpenError {
    let kind = if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        OpenFileToolOpenErrorKind::InvalidFileType
    } else {
        OpenFileToolOpenErrorKind::Unavailable
    };
    OpenFileToolOpenError::new(kind)
}

#[cfg(target_os = "linux")]
impl OpenFileTool {
    async fn execute_linux(
        &self,
        path: String,
        cancellation: CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let request = self.prepare_launch_request(path.clone(), &cancellation)?;
        match self.launcher.launch(request, cancellation.clone()).await {
            OpenFileLaunchOutcome::Accepted => success(&path),
            OpenFileLaunchOutcome::Cancelled => Err(cancelled()),
            OpenFileLaunchOutcome::Unavailable => {
                check_cancellation(&cancellation)?;
                Err(launcher_unavailable())
            }
            OpenFileLaunchOutcome::ResultUnknown => Err(result_unknown()),
        }
    }

    fn prepare_launch_request(
        &self,
        path: String,
        cancellation: &CancellationToken,
    ) -> Result<OpenFileLaunchRequest, ToolError> {
        check_cancellation(cancellation)?;
        let root = finish_precommit_operation(
            rustix::fs::openat(
                self.root.as_fd(),
                ".",
                OFlags::PATH
                    | OFlags::DIRECTORY
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC
                    | OFlags::NONBLOCK,
                Mode::empty(),
            ),
            cancellation,
            |_| unavailable(),
        )?;
        finish_precommit_operation(
            ensure_linked_directory(root.as_fd()),
            cancellation,
            std::convert::identity,
        )?;

        let mut directory = root;
        let mut components = path.split('/').peekable();
        let target = loop {
            check_cancellation(cancellation)?;
            let component = components.next().ok_or_else(invalid_arguments)?;
            if components.peek().is_some() {
                directory = finish_precommit_operation(
                    rustix::fs::openat(
                        directory.as_fd(),
                        component,
                        OFlags::PATH
                            | OFlags::DIRECTORY
                            | OFlags::NOFOLLOW
                            | OFlags::CLOEXEC
                            | OFlags::NONBLOCK,
                        Mode::empty(),
                    ),
                    cancellation,
                    map_path_open_error,
                )?;
            } else {
                break finish_precommit_operation(
                    rustix::fs::openat(
                        directory.as_fd(),
                        component,
                        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                        Mode::empty(),
                    ),
                    cancellation,
                    map_path_open_error,
                )?;
            }
        };

        let metadata =
            finish_precommit_operation(rustix::fs::fstat(&target), cancellation, |_| {
                unavailable()
            })?;
        finish_precommit_operation(
            if !FileType::from_raw_mode(metadata.st_mode).is_file() {
                Err(not_regular_file())
            } else if metadata.st_nlink == 0 {
                Err(unavailable())
            } else {
                Ok(())
            },
            cancellation,
            std::convert::identity,
        )?;

        let proc_path = PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            target.as_raw_fd()
        ));
        let proc_metadata =
            finish_precommit_operation(rustix::fs::stat(&proc_path), cancellation, |_| {
                unavailable()
            })?;
        finish_precommit_operation(
            if proc_metadata.st_dev != metadata.st_dev
                || proc_metadata.st_ino != metadata.st_ino
                || !FileType::from_raw_mode(proc_metadata.st_mode).is_file()
            {
                Err(unavailable())
            } else {
                Ok(())
            },
            cancellation,
            std::convert::identity,
        )?;

        Ok(OpenFileLaunchRequest {
            path,
            proc_path,
            target,
        })
    }
}

#[cfg(target_os = "linux")]
fn finish_precommit_operation<T, E>(
    result: Result<T, E>,
    cancellation: &CancellationToken,
    map_error: impl FnOnce(E) -> ToolError,
) -> Result<T, ToolError> {
    check_cancellation(cancellation)?;
    result.map_err(map_error)
}

#[cfg(target_os = "linux")]
fn ensure_linked_directory(directory: BorrowedFd<'_>) -> Result<(), ToolError> {
    let metadata = rustix::fs::fstat(directory).map_err(|_| unavailable())?;
    if metadata.st_nlink == 0 || !FileType::from_raw_mode(metadata.st_mode).is_dir() {
        Err(unavailable())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn map_path_open_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::NOENT {
        not_found()
    } else if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        path_rejected()
    } else if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        unavailable()
    }
}

#[cfg(target_os = "linux")]
fn success(path: &str) -> Result<ToolOutput, ToolError> {
    let output = ToolOutput::success(json!({ "path": path }));
    if serialized_value_fits(&output, MAX_OPEN_FILE_SERIALIZED_RESULT_BYTES) {
        Ok(output)
    } else {
        Err(result_unknown())
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
struct SystemOpenFileLauncher {
    config: Arc<SystemLaunchConfig>,
}

#[cfg(target_os = "linux")]
impl Default for SystemOpenFileLauncher {
    fn default() -> Self {
        Self {
            config: Arc::new(SystemLaunchConfig {
                program: PathBuf::from("/usr/bin/xdg-open"),
                current_dir: PathBuf::from("/"),
                timeout: OPEN_FILE_LAUNCH_TIMEOUT,
                #[cfg(test)]
                before_spawn: None,
                #[cfg(test)]
                force_wait_failure: false,
            }),
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct SystemLaunchConfig {
    program: PathBuf,
    current_dir: PathBuf,
    timeout: Duration,
    #[cfg(test)]
    before_spawn: Option<Arc<BeforeSpawnHook>>,
    #[cfg(test)]
    force_wait_failure: bool,
}

#[cfg(all(test, target_os = "linux"))]
#[derive(Debug)]
struct BeforeSpawnHook {
    reached: std::sync::Barrier,
    release: std::sync::Barrier,
}

#[cfg(all(test, target_os = "linux"))]
impl BeforeSpawnHook {
    fn new() -> Self {
        Self {
            reached: std::sync::Barrier::new(2),
            release: std::sync::Barrier::new(2),
        }
    }

    fn pause_worker(&self) {
        self.reached.wait();
        self.release.wait();
    }
}

#[cfg(target_os = "linux")]
impl OpenFileLauncher for SystemOpenFileLauncher {
    fn launch(
        &self,
        request: OpenFileLaunchRequest,
        cancellation: CancellationToken,
    ) -> OpenFileLaunch {
        Box::pin(SystemLaunchFuture::new(
            request,
            cancellation,
            Arc::clone(&self.config),
        ))
    }
}

#[cfg(target_os = "linux")]
struct SystemLaunchFuture {
    cancellation: CancellationToken,
    cancelled: Pin<Box<machine_god_core::Cancelled>>,
    config: Arc<SystemLaunchConfig>,
    state: SystemLaunchState,
}

#[cfg(target_os = "linux")]
enum SystemLaunchState {
    Initial(Option<OpenFileLaunchRequest>),
    Waiting(WorkerHandle),
    Done,
}

#[cfg(target_os = "linux")]
impl SystemLaunchFuture {
    fn new(
        request: OpenFileLaunchRequest,
        cancellation: CancellationToken,
        config: Arc<SystemLaunchConfig>,
    ) -> Self {
        Self {
            cancelled: Box::pin(cancellation.cancelled()),
            cancellation,
            config,
            state: SystemLaunchState::Initial(Some(request)),
        }
    }

    fn finish(&mut self, outcome: OpenFileLaunchOutcome) -> Poll<OpenFileLaunchOutcome> {
        self.state = SystemLaunchState::Done;
        Poll::Ready(outcome)
    }
}

#[cfg(target_os = "linux")]
impl Future for SystemLaunchFuture {
    type Output = OpenFileLaunchOutcome;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            if matches!(self.state, SystemLaunchState::Initial(_))
                && self.cancellation.is_cancelled()
            {
                return self.finish(OpenFileLaunchOutcome::Cancelled);
            }

            if let SystemLaunchState::Initial(request) = &mut self.state {
                let request = request.take().expect("initial launch request exists");
                let Ok(worker) = WorkerHandle::spawn(
                    request,
                    self.cancellation.clone(),
                    Arc::clone(&self.config),
                ) else {
                    let outcome = if self.cancellation.is_cancelled() {
                        OpenFileLaunchOutcome::Cancelled
                    } else {
                        OpenFileLaunchOutcome::Unavailable
                    };
                    return self.finish(outcome);
                };
                self.state = SystemLaunchState::Waiting(worker);
                continue;
            }

            if matches!(self.state, SystemLaunchState::Waiting(_)) {
                if self.cancelled.as_mut().poll(context).is_ready() {
                    let state = std::mem::replace(&mut self.state, SystemLaunchState::Done);
                    let SystemLaunchState::Waiting(worker) = state else {
                        unreachable!("waiting state was checked")
                    };
                    return Poll::Ready(worker.abort_and_join());
                }

                let outcome = match &mut self.state {
                    SystemLaunchState::Waiting(worker) => worker.poll_outcome(context),
                    _ => unreachable!("waiting state was checked"),
                };
                if let Some(outcome) = outcome {
                    let state = std::mem::replace(&mut self.state, SystemLaunchState::Done);
                    let SystemLaunchState::Waiting(worker) = state else {
                        unreachable!("waiting state was checked")
                    };
                    return Poll::Ready(worker.join_finished(outcome));
                }
                return Poll::Pending;
            }

            panic!("open_file launch future polled after completion");
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for SystemLaunchFuture {
    fn drop(&mut self) {
        let state = std::mem::replace(&mut self.state, SystemLaunchState::Done);
        if let SystemLaunchState::Waiting(worker) = state {
            let _ = worker.abort_and_join();
        }
    }
}

#[cfg(target_os = "linux")]
struct WorkerHandle {
    shared: Arc<Mutex<WorkerState>>,
    thread: Option<JoinHandle<()>>,
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct WorkerState {
    abort: bool,
    outcome: Option<OpenFileLaunchOutcome>,
    notification_complete: bool,
    waker: Option<Waker>,
}

#[cfg(target_os = "linux")]
impl WorkerHandle {
    fn spawn(
        request: OpenFileLaunchRequest,
        cancellation: CancellationToken,
        config: Arc<SystemLaunchConfig>,
    ) -> Result<Self, ()> {
        let shared = Arc::new(Mutex::new(WorkerState::default()));
        let worker_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("machine-god-open-file".to_owned())
            .spawn(move || launch_worker(&request, &cancellation, &config, &worker_shared))
            .map_err(|_| ())?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    fn poll_outcome(&mut self, context: &Context<'_>) -> Option<OpenFileLaunchOutcome> {
        let mut incoming = Some(context.waker().clone());
        let (outcome, replaced) = {
            let mut state = lock_worker_state(&self.shared);
            if let Some(outcome) = state.outcome.take() {
                (Some(outcome), state.waker.take())
            } else {
                let replaced = match state.waker.as_ref() {
                    Some(existing) if existing.will_wake(context.waker()) => None,
                    Some(_) => state
                        .waker
                        .replace(incoming.take().expect("incoming waker exists")),
                    None => {
                        state.waker = incoming.take();
                        None
                    }
                };
                (None, replaced)
            }
        };
        drop(replaced);
        drop(incoming);
        outcome
    }

    fn abort_and_join(mut self) -> OpenFileLaunchOutcome {
        let (published, notification_complete, suppressed_waker) = {
            let mut state = lock_worker_state(&self.shared);
            if let Some(outcome) = state.outcome.take() {
                (Some(outcome), state.notification_complete, None)
            } else {
                state.abort = true;
                (None, false, state.waker.take())
            }
        };
        drop(suppressed_waker);
        let outcome = match published {
            Some(outcome) => self.finish_published_worker(outcome, notification_complete),
            None => self.join_aborted_worker(),
        };
        match outcome {
            OpenFileLaunchOutcome::Cancelled | OpenFileLaunchOutcome::Unavailable => {
                OpenFileLaunchOutcome::Cancelled
            }
            OpenFileLaunchOutcome::Accepted | OpenFileLaunchOutcome::ResultUnknown => {
                OpenFileLaunchOutcome::ResultUnknown
            }
        }
    }

    fn join_finished(mut self, outcome: OpenFileLaunchOutcome) -> OpenFileLaunchOutcome {
        let notification_complete = lock_worker_state(&self.shared).notification_complete;
        self.finish_published_worker(outcome, notification_complete)
    }

    fn finish_published_worker(
        &mut self,
        outcome: OpenFileLaunchOutcome,
        notification_complete: bool,
    ) -> OpenFileLaunchOutcome {
        // Publication happens only after the helper is reaped. The worker may
        // now be executing an arbitrary Waker callback, which can synchronously
        // repoll or block on executor state held by this future's owner. Joining
        // that callback would permit self-join and cross-thread lock cycles.
        let thread = self.thread.take().expect("open_file worker thread exists");
        if !notification_complete || thread.thread().id() == thread::current().id() {
            drop(thread);
            return outcome;
        }
        if thread.join().is_err() {
            OpenFileLaunchOutcome::ResultUnknown
        } else {
            outcome
        }
    }

    fn join_aborted_worker(&mut self) -> OpenFileLaunchOutcome {
        let thread = self.thread.take().expect("open_file worker thread exists");
        if thread.thread().id() == thread::current().id() {
            // A current-thread abort can arise only through an adversarial
            // reentrant callback. Never attempt a self-join.
            drop(thread);
            return lock_worker_state(&self.shared)
                .outcome
                .take()
                .unwrap_or(OpenFileLaunchOutcome::ResultUnknown);
        }
        let joined = thread.join();
        if joined.is_err() {
            return OpenFileLaunchOutcome::ResultUnknown;
        }
        lock_worker_state(&self.shared)
            .outcome
            .take()
            .unwrap_or(OpenFileLaunchOutcome::ResultUnknown)
    }
}

#[cfg(target_os = "linux")]
impl Drop for WorkerHandle {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let (published, notification_complete, suppressed_waker) = {
                let mut state = lock_worker_state(&self.shared);
                if state.outcome.is_some() {
                    (true, state.notification_complete, None)
                } else {
                    state.abort = true;
                    (false, false, state.waker.take())
                }
            };
            drop(suppressed_waker);
            let thread = self.thread.take().expect("worker thread exists");
            let current_thread = thread.thread().id() == thread::current().id();
            if (!published || notification_complete) && !current_thread {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn lock_worker_state(shared: &Mutex<WorkerState>) -> std::sync::MutexGuard<'_, WorkerState> {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(target_os = "linux")]
fn launch_worker(
    request: &OpenFileLaunchRequest,
    cancellation: &CancellationToken,
    config: &SystemLaunchConfig,
    shared: &Mutex<WorkerState>,
) {
    if cancellation.is_cancelled() || lock_worker_state(shared).abort {
        publish_worker_outcome(shared, OpenFileLaunchOutcome::Cancelled);
        return;
    }

    let mut command = Command::new(&config.program);
    command
        .arg(request.proc_path())
        .current_dir(&config.current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(test)]
    if let Some(hook) = &config.before_spawn {
        hook.pause_worker();
    }

    let spawn_result = {
        let state = lock_worker_state(shared);
        if cancellation.is_cancelled() || state.abort {
            None
        } else {
            // The abort transition uses this same lock, so cancellation/drop
            // and the spawn attempt have one serialized final gate.
            Some(command.spawn())
        }
    };
    let Some(spawn_result) = spawn_result else {
        publish_worker_outcome(shared, OpenFileLaunchOutcome::Cancelled);
        return;
    };
    let Ok(mut child) = spawn_result else {
        let outcome = if cancellation.is_cancelled() || lock_worker_state(shared).abort {
            OpenFileLaunchOutcome::Cancelled
        } else {
            OpenFileLaunchOutcome::Unavailable
        };
        publish_worker_outcome(shared, outcome);
        return;
    };

    let deadline = Instant::now() + config.timeout;
    loop {
        if cancellation.is_cancelled() || lock_worker_state(shared).abort {
            terminate_and_reap(&mut child);
            publish_worker_outcome(shared, OpenFileLaunchOutcome::ResultUnknown);
            return;
        }
        #[cfg(test)]
        if config.force_wait_failure {
            terminate_and_reap(&mut child);
            publish_worker_outcome(shared, OpenFileLaunchOutcome::ResultUnknown);
            return;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                publish_worker_outcome(
                    shared,
                    if status.success() {
                        OpenFileLaunchOutcome::Accepted
                    } else {
                        OpenFileLaunchOutcome::ResultUnknown
                    },
                );
                return;
            }
            Ok(None) => {}
            Err(_) => {
                terminate_and_reap(&mut child);
                publish_worker_outcome(shared, OpenFileLaunchOutcome::ResultUnknown);
                return;
            }
        }
        if Instant::now() >= deadline {
            terminate_and_reap(&mut child);
            publish_worker_outcome(shared, OpenFileLaunchOutcome::ResultUnknown);
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(target_os = "linux")]
fn terminate_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
fn publish_worker_outcome(shared: &Mutex<WorkerState>, outcome: OpenFileLaunchOutcome) {
    let waker = {
        let mut state = lock_worker_state(shared);
        state.outcome = Some(outcome);
        state.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
    lock_worker_state(shared).notification_complete = true;
}

#[cfg(target_os = "linux")]
fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "open_file_invalid_arguments",
        "open_file arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "open_file_invalid_path",
        "open_file path is invalid",
        false,
    )
}

#[cfg(not(target_os = "linux"))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "open_file_unsupported_platform",
        "native open_file is unsupported on this platform",
        false,
    )
}

#[cfg(target_os = "linux")]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "open_file_not_found",
        "requested file is unavailable",
        false,
    )
}

#[cfg(target_os = "linux")]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "open_file_permission_denied",
        "requested file cannot be opened",
        false,
    )
}

#[cfg(target_os = "linux")]
fn path_rejected() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "open_file_path_rejected",
        "requested file path is not confined",
        false,
    )
}

#[cfg(target_os = "linux")]
fn not_regular_file() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "open_file_not_regular_file",
        "requested path is not a regular file",
        false,
    )
}

#[cfg(target_os = "linux")]
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "open_file_unavailable",
        "requested file is unavailable",
        true,
    )
}

#[cfg(target_os = "linux")]
fn launcher_unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "open_file_launcher_unavailable",
        "native file launcher is unavailable",
        true,
    )
}

#[cfg(target_os = "linux")]
fn result_unknown() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "open_file_result_unknown",
        "requested file open status is uncertain",
        false,
    )
}

#[cfg(target_os = "linux")]
fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "open_file_cancelled",
        "open_file execution was cancelled",
        false,
    )
}

#[cfg(all(test, target_os = "linux"))]
mod system_tests;
