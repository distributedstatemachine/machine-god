//! Bounded, permission-gated foreground shell execution.

use std::error::Error;
#[cfg(unix)]
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt;
use std::future::{Future, poll_fn};
use std::io;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, PreparedToolCall, ProcessEnvironment, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Value, json};
#[cfg(unix)]
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
use std::io::Read;
#[cfg(target_os = "linux")]
use std::os::unix::process::{CommandExt, ExitStatusExt};
#[cfg(target_os = "linux")]
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, AtomicU64};

#[cfg(target_os = "linux")]
use rustix::fd::AsRawFd;
#[cfg(unix)]
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(unix)]
use rustix::fs::{FileType, Mode, OFlags};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

/// Model-visible tool name.
pub const TERMINAL_TOOL_NAME: &str = "terminal";
/// Maximum UTF-8 command size.
pub const MAX_TERMINAL_COMMAND_BYTES: usize = 32 * 1024;
/// Maximum canonical working-directory size.
pub const MAX_TERMINAL_CWD_BYTES: usize = 4 * 1024;
/// Maximum working-directory component count.
pub const MAX_TERMINAL_CWD_COMPONENTS: usize = 256;
/// Maximum bytes in one working-directory component.
pub const MAX_TERMINAL_CWD_COMPONENT_BYTES: usize = 255;
/// Maximum serialized canonical argument size.
pub const MAX_TERMINAL_SERIALIZED_ARGUMENT_BYTES: usize = 64 * 1024;
/// Maximum aggregate raw output retained for presentation.
pub const MAX_TERMINAL_RETAINED_OUTPUT_BYTES: usize = 64 * 1024;
/// Maximum aggregate produced output before termination.
pub const MAX_TERMINAL_PRODUCED_OUTPUT_BYTES: u64 = 1024 * 1024;
/// Maximum complete serialized tool output.
pub const MAX_TERMINAL_SERIALIZED_RESULT_BYTES: usize = 48 * 1024;
/// Maximum environment entries retained at construction.
pub const MAX_TERMINAL_ENVIRONMENT_ENTRIES: usize = 512;
/// Maximum bytes in one environment key.
pub const MAX_TERMINAL_ENVIRONMENT_KEY_BYTES: usize = 1024;
/// Maximum bytes in one environment value.
pub const MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES: usize = 16 * 1024;
/// Maximum aggregate key and value bytes in the environment.
pub const MAX_TERMINAL_ENVIRONMENT_BYTES: usize = 256 * 1024;
/// Fixed production shell.
pub const TERMINAL_PROGRAM: &str = "/bin/sh";
/// Stable environment profile installed by this slice.
pub const TERMINAL_ENVIRONMENT_PROFILE: &str = "construction_snapshot";
/// Default absolute execution timeout.
pub const TERMINAL_DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// Maximum test-configurable execution timeout.
pub const TERMINAL_MAX_TIMEOUT: Duration = Duration::from_secs(600);
/// Default simultaneous execution limit.
pub const TERMINAL_DEFAULT_MAX_ACTIVE_EXECUTIONS: usize = 4;
/// Hard simultaneous execution limit.
pub const TERMINAL_MAX_ACTIVE_EXECUTIONS: usize = 16;

const TERMINAL_DESCRIPTION: &str =
    "Run one foreground shell command from a workspace-relative directory";
const PIPE_RETAINED_BYTES: usize = MAX_TERMINAL_RETAINED_OUTPUT_BYTES / 2;
#[cfg(target_os = "linux")]
const PIPE_READ_BYTES: usize = 16 * 1024;
#[cfg(target_os = "linux")]
const POST_STOP_READ_LIMIT: u8 = 64;
#[cfg(target_os = "linux")]
const PROCESS_GROUP_TERM_GRACE: Duration = Duration::from_millis(250);
#[cfg(target_os = "linux")]
const PROCESS_GROUP_KILL_OBSERVATION_GRACE: Duration = Duration::from_millis(250);

/// Stable terminal construction-error category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TerminalConfigErrorKind {
    UnsupportedPlatform,
    InvalidRoot,
    InvalidEnvironment,
    InvalidLimits,
}

/// Fixed, redacted terminal construction failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct TerminalConfigError {
    kind: TerminalConfigErrorKind,
}

impl TerminalConfigError {
    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(self) -> TerminalConfigErrorKind {
        self.kind
    }

    const fn new(kind: TerminalConfigErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for TerminalConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalConfigError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for TerminalConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            TerminalConfigErrorKind::UnsupportedPlatform => {
                "native terminal execution is unsupported on this platform"
            }
            TerminalConfigErrorKind::InvalidRoot => "native terminal workspace root is invalid",
            TerminalConfigErrorKind::InvalidEnvironment => {
                "native terminal environment snapshot is invalid"
            }
            TerminalConfigErrorKind::InvalidLimits => "native terminal limits are invalid",
        })
    }
}

impl Error for TerminalConfigError {}

/// Bounded timeout and active-execution configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalLimits {
    timeout: Duration,
    max_active_executions: usize,
}

impl TerminalLimits {
    /// Constructs explicit terminal limits.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-limits failure outside the documented bounds.
    pub fn new(
        timeout: Duration,
        max_active_executions: usize,
    ) -> Result<Self, TerminalConfigError> {
        if timeout < Duration::from_millis(1)
            || timeout > TERMINAL_MAX_TIMEOUT
            || !(1..=TERMINAL_MAX_ACTIVE_EXECUTIONS).contains(&max_active_executions)
        {
            return Err(TerminalConfigError::new(
                TerminalConfigErrorKind::InvalidLimits,
            ));
        }
        Ok(Self {
            timeout,
            max_active_executions,
        })
    }

    /// Returns the absolute execution timeout.
    #[must_use]
    pub const fn timeout(self) -> Duration {
        self.timeout
    }

    /// Returns the simultaneous execution limit.
    #[must_use]
    pub const fn max_active_executions(self) -> usize {
        self.max_active_executions
    }
}

impl Default for TerminalLimits {
    fn default() -> Self {
        Self {
            timeout: TERMINAL_DEFAULT_TIMEOUT,
            max_active_executions: TERMINAL_DEFAULT_MAX_ACTIVE_EXECUTIONS,
        }
    }
}

/// Fixed failure category returned by an injected terminal executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TerminalExecutorErrorKind {
    Unsupported,
    Busy,
    Spawn,
    Wait,
    Pipe,
    Invariant,
    InvalidResponse,
    Cancelled,
}

/// Fixed, data-free executor failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct TerminalExecutorError {
    kind: TerminalExecutorErrorKind,
}

impl TerminalExecutorError {
    /// Constructs a fixed executor failure.
    #[must_use]
    pub const fn new(kind: TerminalExecutorErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable category.
    #[must_use]
    pub const fn kind(self) -> TerminalExecutorErrorKind {
        self.kind
    }
}

impl fmt::Debug for TerminalExecutorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalExecutorError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for TerminalExecutorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal executor failed")
    }
}

impl Error for TerminalExecutorError {}

/// Status of one completed foreground execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TerminalExecutionStatus {
    Exited(i32),
    Signaled(i32),
    TimedOut,
    OutputLimit,
}

/// One independently captured output stream.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalCapturedOutput {
    bytes: Vec<u8>,
    total_bytes: u64,
}

impl TerminalCapturedOutput {
    /// Constructs a bounded retained stream and its exact produced-byte count.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-response failure for inconsistent or oversized data.
    pub fn new(bytes: Vec<u8>, total_bytes: u64) -> Result<Self, TerminalExecutorError> {
        if bytes.len() > PIPE_RETAINED_BYTES || total_bytes < bytes.len() as u64 {
            return Err(TerminalExecutorError::new(
                TerminalExecutorErrorKind::InvalidResponse,
            ));
        }
        Ok(Self { bytes, total_bytes })
    }

    /// Returns retained head/tail bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the exact number of produced bytes observed by the executor.
    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Reports whether bytes were omitted from the retained presentation.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.total_bytes > self.bytes.len() as u64
    }
}

impl fmt::Debug for TerminalCapturedOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalCapturedOutput")
            .field("retained_bytes", &self.bytes.len())
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

/// Complete bounded response from one terminal executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalExecutionOutcome {
    status: TerminalExecutionStatus,
    stdout: TerminalCapturedOutput,
    stderr: TerminalCapturedOutput,
    duration: Duration,
}

impl TerminalExecutionOutcome {
    /// Constructs a validated executor outcome.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-response failure when stream totals or status
    /// contradict the terminal bounds.
    pub fn new(
        status: TerminalExecutionStatus,
        stdout: TerminalCapturedOutput,
        stderr: TerminalCapturedOutput,
        duration: Duration,
    ) -> Result<Self, TerminalExecutorError> {
        let retained = stdout.bytes.len().saturating_add(stderr.bytes.len());
        let produced = stdout.total_bytes.saturating_add(stderr.total_bytes);
        let status_valid = match status {
            TerminalExecutionStatus::Exited(code) => (0..=255).contains(&code),
            TerminalExecutionStatus::Signaled(signal) => (1..=255).contains(&signal),
            TerminalExecutionStatus::TimedOut | TerminalExecutionStatus::OutputLimit => true,
        };
        if !status_valid
            || duration > TERMINAL_MAX_TIMEOUT
            || retained > MAX_TERMINAL_RETAINED_OUTPUT_BYTES
            || (matches!(status, TerminalExecutionStatus::OutputLimit)
                != (produced > MAX_TERMINAL_PRODUCED_OUTPUT_BYTES))
        {
            return Err(TerminalExecutorError::new(
                TerminalExecutorErrorKind::InvalidResponse,
            ));
        }
        Ok(Self {
            status,
            stdout,
            stderr,
            duration,
        })
    }

    #[must_use]
    pub const fn status(&self) -> TerminalExecutionStatus {
        self.status
    }

    #[must_use]
    pub const fn stdout(&self) -> &TerminalCapturedOutput {
        &self.stdout
    }

    #[must_use]
    pub const fn stderr(&self) -> &TerminalCapturedOutput {
        &self.stderr
    }

    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }
}

/// Owned exact request passed to a trusted executor.
pub struct TerminalExecutionRequest {
    command: String,
    cwd: String,
    environment: Vec<(OsString, OsString)>,
    environment_sha256: String,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    activity: Arc<ExecutionActivity>,
    #[cfg(target_os = "linux")]
    started: Instant,
    deadline: Instant,
    #[cfg(unix)]
    directory: OwnedFd,
    #[cfg(target_os = "linux")]
    proc_path: PathBuf,
}

impl TerminalExecutionRequest {
    #[must_use]
    pub const fn program(&self) -> &'static str {
        TERMINAL_PROGRAM
    }

    #[must_use]
    pub fn arguments(&self) -> [&str; 2] {
        ["-c", &self.command]
    }

    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    #[must_use]
    pub const fn environment_profile(&self) -> &'static str {
        TERMINAL_ENVIRONMENT_PROFILE
    }

    #[must_use]
    pub fn environment_sha256(&self) -> &str {
        &self.environment_sha256
    }

    /// Returns the exact raw snapshot. Debug output remains redacted.
    #[must_use]
    pub fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    #[cfg(unix)]
    #[must_use]
    pub fn directory_fd(&self) -> BorrowedFd<'_> {
        self.directory.as_fd()
    }

    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn proc_path(&self) -> &Path {
        &self.proc_path
    }
}

impl fmt::Debug for TerminalExecutionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TerminalExecutionRequest { .. }")
    }
}

/// Inert sendable future returned by a trusted executor.
pub type TerminalExecution =
    BoxFuture<'static, Result<TerminalExecutionOutcome, TerminalExecutorError>>;

/// Trusted process boundary for one exact foreground request.
pub trait TerminalExecutor: Send + Sync + 'static {
    /// Creates an inert future. The executor owns every process, pipe, and worker
    /// it creates and must complete all controllable resource cleanup when the
    /// future is dropped. Every task `Waker` supplied while polling the returned
    /// future transparently retains this execution's one active slot. A stored
    /// or in-flight notification callback may therefore outlive cleanup when
    /// joining it could deadlock, but the executor must release stored Wakers
    /// when they are no longer usable and the tail must retain no process
    /// resource or other execution authority. Executors should honor
    /// [`TerminalExecutionRequest::deadline`] so they can return bounded partial
    /// output. The tool also owns an independent deadline wake for controllable
    /// userspace phases; neither boundary can preempt a blocked host syscall.
    fn execute(
        &self,
        request: TerminalExecutionRequest,
        cancellation: CancellationToken,
    ) -> TerminalExecution;
}

struct EnvironmentSnapshot {
    #[cfg(unix)]
    entries: Vec<(OsString, OsString)>,
    sha256: String,
}

/// Native foreground terminal tool confined to one retained workspace root.
pub struct TerminalTool {
    #[cfg(unix)]
    root: OwnedFd,
    environment: Arc<EnvironmentSnapshot>,
    executor: Arc<dyn TerminalExecutor>,
    limits: TerminalLimits,
    active: Arc<AtomicUsize>,
    system_unsupported: bool,
}

impl TerminalTool {
    /// Opens a workspace root and snapshots the process environment.
    ///
    /// # Errors
    ///
    /// Returns a fixed configuration failure when unsupported or when the root
    /// or environment cannot be retained within the frozen bounds.
    pub fn open(root: &Path) -> Result<Self, TerminalConfigError> {
        Self::open_with_limits(root, TerminalLimits::default())
    }

    /// Opens a workspace root with explicit bounded system-execution limits.
    ///
    /// The process environment is captured once at construction and the fixed
    /// production executor is used for every execution.
    ///
    /// # Errors
    ///
    /// Returns a fixed configuration failure when unsupported, when `limits`
    /// are invalid, or when the root or environment cannot be retained within
    /// the frozen bounds.
    pub fn open_with_limits(
        root: &Path,
        limits: TerminalLimits,
    ) -> Result<Self, TerminalConfigError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (root, limits);
            Err(TerminalConfigError::new(
                TerminalConfigErrorKind::UnsupportedPlatform,
            ))
        }
        #[cfg(target_os = "linux")]
        {
            let root = open_workspace_root(root)?;
            Self::from_parts(
                root,
                std::env::vars_os().collect(),
                Arc::new(SystemTerminalExecutor),
                limits,
                false,
            )
        }
    }

    /// Constructs a bounded tool around an explicitly injected executor.
    ///
    /// # Errors
    ///
    /// Returns a fixed configuration failure for an invalid root, environment,
    /// or limit configuration.
    #[cfg(unix)]
    pub fn with_executor(
        root: &Path,
        environment: Vec<(OsString, OsString)>,
        executor: Arc<dyn TerminalExecutor>,
        limits: TerminalLimits,
    ) -> Result<Self, TerminalConfigError> {
        let root = open_workspace_root(root)?;
        Self::from_parts(root, environment, executor, limits, false)
    }

    #[cfg(unix)]
    #[allow(dead_code)] // Used by reference-host composition on integration.
    pub(crate) fn from_root_descriptor(root: OwnedFd) -> Result<Self, TerminalConfigError> {
        #[cfg(target_os = "linux")]
        let environment = std::env::vars_os().collect();
        #[cfg(not(target_os = "linux"))]
        let environment = Vec::new();
        Self::from_parts(
            root,
            environment,
            Arc::new(SystemTerminalExecutor),
            TerminalLimits::default(),
            !cfg!(target_os = "linux"),
        )
    }

    #[cfg(unix)]
    fn from_parts(
        root: OwnedFd,
        environment: Vec<(OsString, OsString)>,
        executor: Arc<dyn TerminalExecutor>,
        limits: TerminalLimits,
        system_unsupported: bool,
    ) -> Result<Self, TerminalConfigError> {
        validate_limits(limits)?;
        let environment = Arc::new(snapshot_environment(environment)?);
        Ok(Self {
            root,
            environment,
            executor,
            limits,
            active: Arc::new(AtomicUsize::new(0)),
            system_unsupported,
        })
    }
}

#[cfg(unix)]
fn validate_limits(limits: TerminalLimits) -> Result<(), TerminalConfigError> {
    TerminalLimits::new(limits.timeout, limits.max_active_executions).map(|_| ())
}

#[cfg(unix)]
fn raw_os_bytes(value: &OsStr) -> &[u8] {
    value.as_bytes()
}

#[cfg(unix)]
fn snapshot_environment(
    mut entries: Vec<(OsString, OsString)>,
) -> Result<EnvironmentSnapshot, TerminalConfigError> {
    if entries.len() > MAX_TERMINAL_ENVIRONMENT_ENTRIES {
        return Err(invalid_environment());
    }
    let mut aggregate = 0_usize;
    for (key, value) in &entries {
        let key = raw_os_bytes(key);
        let value = raw_os_bytes(value);
        if key.is_empty()
            || key.len() > MAX_TERMINAL_ENVIRONMENT_KEY_BYTES
            || value.len() > MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES
            || key.contains(&b'=')
            || key.contains(&0)
            || value.contains(&0)
        {
            return Err(invalid_environment());
        }
        aggregate = aggregate
            .checked_add(key.len())
            .and_then(|total| total.checked_add(value.len()))
            .ok_or_else(invalid_environment)?;
        if aggregate > MAX_TERMINAL_ENVIRONMENT_BYTES {
            return Err(invalid_environment());
        }
    }

    entries.sort_by(|left, right| {
        raw_os_bytes(&left.0)
            .cmp(raw_os_bytes(&right.0))
            .then_with(|| raw_os_bytes(&left.1).cmp(raw_os_bytes(&right.1)))
    });

    let mut previous_key: Option<&[u8]> = None;
    let mut hasher = Sha256::new();
    for (key, value) in &entries {
        let key = raw_os_bytes(key);
        let value = raw_os_bytes(value);
        if previous_key == Some(key) {
            return Err(invalid_environment());
        }
        hasher.update((key.len() as u64).to_be_bytes());
        hasher.update(key);
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
        previous_key = Some(key);
    }

    Ok(EnvironmentSnapshot {
        entries,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

#[cfg(unix)]
fn open_workspace_root(root: &Path) -> Result<OwnedFd, TerminalConfigError> {
    let lexical_root = root.components().collect::<PathBuf>();
    if !lexical_root.is_absolute() {
        return Err(TerminalConfigError::new(
            TerminalConfigErrorKind::InvalidRoot,
        ));
    }
    let root = rustix::fs::open(
        lexical_root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| TerminalConfigError::new(TerminalConfigErrorKind::InvalidRoot))?;
    ensure_directory(root.as_fd())
        .map_err(|_| TerminalConfigError::new(TerminalConfigErrorKind::InvalidRoot))?;
    Ok(root)
}

#[cfg(unix)]
fn invalid_environment() -> TerminalConfigError {
    TerminalConfigError::new(TerminalConfigErrorKind::InvalidEnvironment)
}

fn terminal_name() -> ToolName {
    ToolName::new(TERMINAL_TOOL_NAME).expect("terminal is a valid tool name")
}

fn parse_arguments(
    arguments: &Value,
    require_complete: bool,
) -> Result<TerminalArguments, ToolError> {
    let Value::Object(object) = arguments else {
        return Err(invalid_arguments());
    };
    let expected_len = if require_complete { 4 } else { object.len() };
    if (require_complete && expected_len != 4)
        || (!require_complete && !(2..=4).contains(&expected_len))
        || object
            .keys()
            .any(|key| !matches!(key.as_str(), "action" | "command" | "cwd" | "profile"))
        || !serialized_value_fits(arguments, MAX_TERMINAL_SERIALIZED_ARGUMENT_BYTES)
    {
        return Err(invalid_arguments());
    }
    if object.get("action").and_then(Value::as_str) != Some("exec") {
        return Err(invalid_arguments());
    }
    let command = object
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(invalid_arguments)?;
    if command.is_empty() || command.len() > MAX_TERMINAL_COMMAND_BYTES || command.contains('\0') {
        return Err(invalid_command());
    }
    let cwd = match object.get("cwd") {
        Some(Value::String(cwd)) => cwd.as_str(),
        None if !require_complete => ".",
        _ => return Err(invalid_arguments()),
    };
    validate_cwd(cwd)?;
    match object.get("profile") {
        Some(Value::String(profile)) if profile == "clean" => {}
        None if !require_complete => {}
        _ => return Err(invalid_arguments()),
    }
    Ok(TerminalArguments {
        command: command.to_owned(),
        cwd: cwd.to_owned(),
    })
}

fn canonical_arguments(arguments: &TerminalArguments) -> Value {
    json!({
        "action": "exec",
        "command": arguments.command,
        "cwd": arguments.cwd,
        "profile": "clean"
    })
}

fn validate_cwd(cwd: &str) -> Result<(), ToolError> {
    if cwd == "." {
        return Ok(());
    }
    let bytes = cwd.as_bytes();
    if cwd.is_empty()
        || cwd.len() > MAX_TERMINAL_CWD_BYTES
        || cwd.starts_with('/')
        || cwd.ends_with('/')
        || cwd.contains('\\')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || cwd.chars().any(is_forbidden_path_character)
    {
        return Err(invalid_cwd());
    }
    let mut count = 0_usize;
    for component in cwd.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component == "~"
            || component.len() > MAX_TERMINAL_CWD_COMPONENT_BYTES
        {
            return Err(invalid_cwd());
        }
        count = count.checked_add(1).ok_or_else(invalid_cwd)?;
        if count > MAX_TERMINAL_CWD_COMPONENTS {
            return Err(invalid_cwd());
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

#[cfg(unix)]
impl TerminalTool {
    fn execution_request(
        &self,
        arguments: TerminalArguments,
        started: Instant,
        deadline: Instant,
        activity: Arc<ExecutionActivity>,
        cancellation: &CancellationToken,
    ) -> Result<TerminalExecutionRequest, ToolError> {
        #[cfg(not(target_os = "linux"))]
        let _ = started;
        check_cancellation(cancellation)?;
        let mut directory = finish_precommit(
            rustix::fs::openat(
                self.root.as_fd(),
                ".",
                directory_open_flags(),
                Mode::empty(),
            ),
            cancellation,
        )?;
        ensure_directory(directory.as_fd())?;
        if arguments.cwd != "." {
            for component in arguments.cwd.split('/') {
                check_cancellation(cancellation)?;
                directory = finish_precommit(
                    rustix::fs::openat(
                        directory.as_fd(),
                        component,
                        directory_open_flags(),
                        Mode::empty(),
                    ),
                    cancellation,
                )?;
                ensure_directory(directory.as_fd())?;
            }
        }
        check_cancellation(cancellation)?;
        #[cfg(target_os = "linux")]
        let proc_path = validated_proc_path(directory.as_fd(), cancellation)?;
        Ok(TerminalExecutionRequest {
            command: arguments.command,
            cwd: arguments.cwd,
            environment: self.environment.entries.clone(),
            environment_sha256: self.environment.sha256.clone(),
            activity,
            #[cfg(target_os = "linux")]
            started,
            deadline,
            directory,
            #[cfg(target_os = "linux")]
            proc_path,
        })
    }
}

#[cfg(not(unix))]
impl TerminalTool {
    fn execution_request(
        &self,
        _arguments: TerminalArguments,
        _started: Instant,
        _deadline: Instant,
        _activity: Arc<ExecutionActivity>,
        _cancellation: &CancellationToken,
    ) -> Result<TerminalExecutionRequest, ToolError> {
        Err(unsupported_platform())
    }
}

#[cfg(unix)]
fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

#[cfg(unix)]
fn ensure_directory(directory: BorrowedFd<'_>) -> Result<(), ToolError> {
    let metadata = rustix::fs::fstat(directory).map_err(|_| execution_unavailable())?;
    if metadata.st_nlink == 0 || !FileType::from_raw_mode(metadata.st_mode).is_dir() {
        Err(cwd_unavailable())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn finish_precommit<T>(
    result: Result<T, rustix::io::Errno>,
    cancellation: &CancellationToken,
) -> Result<T, ToolError> {
    check_cancellation(cancellation)?;
    result.map_err(|error| {
        if error == rustix::io::Errno::NOENT
            || error == rustix::io::Errno::NOTDIR
            || error == rustix::io::Errno::LOOP
        {
            cwd_unavailable()
        } else {
            execution_unavailable()
        }
    })
}

#[cfg(target_os = "linux")]
fn validated_proc_path(
    directory: BorrowedFd<'_>,
    cancellation: &CancellationToken,
) -> Result<PathBuf, ToolError> {
    let metadata = rustix::fs::fstat(directory).map_err(|_| execution_unavailable())?;
    check_cancellation(cancellation)?;
    let path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        directory.as_raw_fd()
    ));
    let proc_metadata = rustix::fs::stat(&path).map_err(|_| execution_unavailable())?;
    check_cancellation(cancellation)?;
    if metadata.st_dev != proc_metadata.st_dev
        || metadata.st_ino != proc_metadata.st_ino
        || !FileType::from_raw_mode(proc_metadata.st_mode).is_dir()
    {
        return Err(execution_unavailable());
    }
    Ok(path)
}

async fn await_executor(
    mut future: TerminalExecution,
    cancellation: &CancellationToken,
    deadline: Instant,
    started: Instant,
    timer: DeadlineTimer,
    activity: Arc<ExecutionActivity>,
) -> Result<TerminalExecutionOutcome, ToolError> {
    let mut cancelled = Box::pin(cancellation.cancelled());
    let notifier = Arc::new(ActivityNotifier::new(Arc::clone(&activity)));
    let notifier_waker = Waker::from(Arc::clone(&notifier));
    let result = poll_fn(|context| {
        notifier.bind(context.waker(), &notifier_waker);
        let mut shared_context = Context::from_waker(&notifier_waker);
        if cancelled.as_mut().poll(&mut shared_context).is_ready() || cancellation.is_cancelled() {
            return Poll::Ready(Err(cancelled_error()));
        }
        let execution = Pin::new(&mut future).poll(&mut shared_context);
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(cancelled_error()));
        }
        match execution {
            Poll::Ready(Ok(mut outcome)) => {
                if matches!(outcome.status, TerminalExecutionStatus::OutputLimit) {
                    let _ = activity.claim_output_limit();
                } else if Instant::now() >= deadline {
                    if matches!(activity.close_timeout(), ExecutionCause::OutputLimit) {
                        return Poll::Ready(Err(executor_invariant()));
                    }
                    if !matches!(outcome.status, TerminalExecutionStatus::TimedOut) {
                        outcome.status = TerminalExecutionStatus::TimedOut;
                        outcome.duration = bounded_duration(started.elapsed());
                    }
                }
                Poll::Ready(Ok(outcome))
            }
            Poll::Ready(Err(error)) => {
                let error = map_executor_error(error);
                if matches!(activity.cause(), ExecutionCause::OutputLimit) {
                    return Poll::Ready(Err(error));
                }
                if Instant::now() < deadline {
                    return Poll::Ready(Err(error));
                }
                if matches!(activity.close_timeout(), ExecutionCause::OutputLimit) {
                    return Poll::Ready(Err(error));
                }
                let timeout = empty_outcome(
                    TerminalExecutionStatus::TimedOut,
                    bounded_duration(started.elapsed()),
                )
                .map_err(map_executor_error);
                if cancellation.is_cancelled() {
                    return Poll::Ready(Err(cancelled_error()));
                }
                Poll::Ready(timeout)
            }
            Poll::Pending => {
                if timer.poll_expired(&shared_context).is_pending() {
                    return Poll::Pending;
                }
                if cancellation.is_cancelled() {
                    return Poll::Ready(Err(cancelled_error()));
                }
                let deadline_execution = Pin::new(&mut future).poll(&mut shared_context);
                if cancellation.is_cancelled() {
                    return Poll::Ready(Err(cancelled_error()));
                }
                if let Poll::Ready(Ok(outcome)) = &deadline_execution
                    && matches!(outcome.status, TerminalExecutionStatus::OutputLimit)
                {
                    let _ = activity.claim_output_limit();
                    return Poll::Ready(Ok(outcome.clone()));
                }
                #[cfg(test)]
                activity.run_before_timeout_close();
                if matches!(activity.close_timeout(), ExecutionCause::OutputLimit) {
                    return match deadline_execution {
                        Poll::Pending => Poll::Pending,
                        Poll::Ready(Err(error)) => Poll::Ready(Err(map_executor_error(error))),
                        // A non-output outcome after an output claim cannot
                        // represent the validated bounded overflow result.
                        Poll::Ready(Ok(_)) => Poll::Ready(Err(executor_invariant())),
                    };
                }
                let timeout = empty_outcome(
                    TerminalExecutionStatus::TimedOut,
                    bounded_duration(started.elapsed()),
                )
                .map_err(map_executor_error);
                if cancellation.is_cancelled() {
                    return Poll::Ready(Err(cancelled_error()));
                }
                Poll::Ready(timeout)
            }
        }
    })
    .await;
    notifier.close();
    result
}

fn bounded_duration(duration: Duration) -> Duration {
    std::cmp::min(duration, TERMINAL_MAX_TIMEOUT)
}

fn empty_outcome(
    status: TerminalExecutionStatus,
    duration: Duration,
) -> Result<TerminalExecutionOutcome, TerminalExecutorError> {
    TerminalExecutionOutcome::new(
        status,
        TerminalCapturedOutput::new(Vec::new(), 0)?,
        TerminalCapturedOutput::new(Vec::new(), 0)?,
        duration,
    )
}

fn render_timeout(cwd: &str, started: Instant) -> Result<ToolOutput, ToolError> {
    let outcome = empty_outcome(
        TerminalExecutionStatus::TimedOut,
        bounded_duration(started.elapsed()),
    )
    .map_err(map_executor_error)?;
    render_output(cwd, &outcome)
}

fn render_output(cwd: &str, outcome: &TerminalExecutionOutcome) -> Result<ToolOutput, ToolError> {
    let stdout_lossy = std::str::from_utf8(outcome.stdout.bytes()).is_err();
    let stderr_lossy = std::str::from_utf8(outcome.stderr.bytes()).is_err();
    let mut stdout = String::from_utf8_lossy(outcome.stdout.bytes()).into_owned();
    let mut stderr = String::from_utf8_lossy(outcome.stderr.bytes()).into_owned();
    let mut stdout_truncated = outcome.stdout.truncated();
    let mut stderr_truncated = outcome.stderr.truncated();
    loop {
        let (status, exit_code, signal) = protocol_status(outcome.status);
        let output = ToolOutput {
            content: json!({
                "action": "exec",
                "cwd": cwd,
                "status": status,
                "exit_code": exit_code,
                "signal": signal,
                "stdout": stdout,
                "stderr": stderr,
                "stdout_bytes": outcome.stdout.total_bytes,
                "stderr_bytes": outcome.stderr.total_bytes,
                "stdout_truncated": stdout_truncated,
                "stderr_truncated": stderr_truncated,
                "stdout_lossy": stdout_lossy,
                "stderr_lossy": stderr_lossy,
                "duration_ms": u64::try_from(outcome.duration.as_millis()).unwrap_or(u64::MAX)
            }),
            is_error: !matches!(outcome.status, TerminalExecutionStatus::Exited(0)),
        };
        if serialized_value_fits(&output, MAX_TERMINAL_SERIALIZED_RESULT_BYTES) {
            return Ok(output);
        }
        if stdout.len() >= stderr.len() && !stdout.is_empty() {
            shrink_head_tail(&mut stdout);
            stdout_truncated = true;
        } else if !stderr.is_empty() {
            shrink_head_tail(&mut stderr);
            stderr_truncated = true;
        } else {
            return Err(executor_invariant());
        }
    }
}

fn shrink_head_tail(value: &mut String) {
    let target = value
        .len()
        .saturating_sub(std::cmp::max(1, value.len() / 8));
    let head_target = target / 2;
    let tail_target = target - head_target;
    let mut head = head_target;
    while !value.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = value.len().saturating_sub(tail_target);
    while tail < value.len() && !value.is_char_boundary(tail) {
        tail += 1;
    }
    value.replace_range(head..tail, "");
}

fn protocol_status(status: TerminalExecutionStatus) -> (&'static str, Option<i32>, Option<i32>) {
    match status {
        TerminalExecutionStatus::Exited(code) => ("exited", Some(code), None),
        TerminalExecutionStatus::Signaled(signal) => ("signaled", None, Some(signal)),
        TerminalExecutionStatus::TimedOut => ("timed_out", None, None),
        TerminalExecutionStatus::OutputLimit => ("output_limit", None, None),
    }
}

fn serialized_value_fits(value: &(impl serde::Serialize + ?Sized), limit: usize) -> bool {
    serde_json::to_writer(&mut JsonByteCounter { written: 0, limit }, value).is_ok()
}

struct JsonByteCounter {
    written: usize,
    limit: usize,
}

impl io::Write for JsonByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.written = self
            .written
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("JSON size overflow"))?;
        if self.written > self.limit {
            return Err(io::Error::other("JSON size limit"));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl fmt::Debug for TerminalTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TerminalTool { .. }")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum ExecutionCause {
    Open = 0,
    OutputLimit = 1,
    TimedOut = 2,
}

struct ExecutionActivity {
    active: Arc<AtomicUsize>,
    cause: AtomicU8,
    #[cfg(test)]
    before_timeout_close: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl ExecutionActivity {
    fn acquire(active: &Arc<AtomicUsize>, limit: usize) -> Option<Arc<Self>> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                (value < limit).then(|| value + 1)
            })
            .ok()?;
        Some(Arc::new(Self {
            active: Arc::clone(active),
            cause: AtomicU8::new(ExecutionCause::Open as u8),
            #[cfg(test)]
            before_timeout_close: Mutex::new(None),
        }))
    }

    fn cause(&self) -> ExecutionCause {
        match self.cause.load(Ordering::Acquire) {
            value if value == ExecutionCause::Open as u8 => ExecutionCause::Open,
            value if value == ExecutionCause::OutputLimit as u8 => ExecutionCause::OutputLimit,
            value if value == ExecutionCause::TimedOut as u8 => ExecutionCause::TimedOut,
            _ => unreachable!("terminal execution cause is valid"),
        }
    }

    fn claim_output_limit(&self) -> bool {
        match self.cause.compare_exchange(
            ExecutionCause::Open as u8,
            ExecutionCause::OutputLimit as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => true,
            Err(value) => value == ExecutionCause::OutputLimit as u8,
        }
    }

    fn close_timeout(&self) -> ExecutionCause {
        match self.cause.compare_exchange(
            ExecutionCause::Open as u8,
            ExecutionCause::TimedOut as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => ExecutionCause::TimedOut,
            Err(_) => self.cause(),
        }
    }

    #[cfg(test)]
    fn install_before_timeout_close(&self, hook: impl FnOnce() + Send + 'static) {
        let mut installed = self
            .before_timeout_close
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(installed.replace(Box::new(hook)).is_none());
    }

    #[cfg(test)]
    fn run_before_timeout_close(&self) {
        let hook = self
            .before_timeout_close
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(hook) = hook {
            hook();
        }
    }
}

impl Drop for ExecutionActivity {
    fn drop(&mut self) {
        let previous = self.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "terminal activity underflowed");
    }
}

struct ActivityNotifier {
    // State precedes the activity so the retained target Waker is destroyed
    // before the last notifier-owned activity reference can be released.
    state: Mutex<ActivityNotifierState>,
    _activity: Arc<ExecutionActivity>,
}

#[derive(Default)]
struct ActivityNotifierState {
    target: Option<Arc<Waker>>,
    notifying: bool,
    observed_while_notifying: bool,
    pending_after_observation: bool,
    lifecycle: ActivityNotifierLifecycle,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum ActivityNotifierLifecycle {
    #[default]
    Open,
    Closed,
}

impl ActivityNotifier {
    fn new(activity: Arc<ExecutionActivity>) -> Self {
        Self {
            state: Mutex::new(ActivityNotifierState::default()),
            _activity: activity,
        }
    }

    fn bind(&self, target: &Waker, notifier_waker: &Waker) {
        let notifier_target = target.will_wake(notifier_waker);
        {
            let mut state = lock_activity_notifier(&self.state);
            if matches!(state.lifecycle, ActivityNotifierLifecycle::Closed) {
                return;
            }
            if state.notifying {
                // This poll observes every notice that preceded the bind. A
                // later notice in the same callback window needs one replay.
                state.observed_while_notifying = true;
                state.pending_after_observation = false;
            }
            if notifier_target {
                // A trusted executor may retain the supplied notifier Waker
                // and the outer host may legally use that Waker to re-poll.
                // That poll is an observation, but binding the Waker would
                // replace the genuine outer target with this notifier itself.
                return;
            }
        }

        // Cloning and destroying an arbitrary Waker may execute foreign code,
        // so only Arc bookkeeping and identity comparison happen under lock.
        let incoming = Arc::new(target.clone());
        let (replaced, unused) = {
            let mut state = lock_activity_notifier(&self.state);
            if matches!(state.lifecycle, ActivityNotifierLifecycle::Closed)
                || state
                    .target
                    .as_deref()
                    .is_some_and(|existing| existing.will_wake(target))
            {
                (None, Some(incoming))
            } else {
                (state.target.replace(incoming), None)
            }
        };
        drop(replaced);
        drop(unused);
    }

    fn close(&self) {
        let target = {
            let mut state = lock_activity_notifier(&self.state);
            state.lifecycle = ActivityNotifierLifecycle::Closed;
            state.observed_while_notifying = false;
            state.pending_after_observation = false;
            state.target.take()
        };
        // Destroying an arbitrary Waker may run foreign code. The notifier's
        // activity remains owned by `self`, including through a callback that
        // was already in flight when the target was removed.
        drop(target);
    }

    fn notify(&self) {
        let mut target = {
            let mut state = lock_activity_notifier(&self.state);
            if matches!(state.lifecycle, ActivityNotifierLifecycle::Closed) {
                return;
            }
            if state.notifying {
                // Notices before a re-poll are represented by the callback
                // already in flight. A notice after that poll must be replayed
                // once the callback returns or the wake could be lost.
                if state.observed_while_notifying {
                    state.pending_after_observation = true;
                }
                return;
            }
            let Some(target) = state.target.as_ref().map(Arc::clone) else {
                return;
            };
            state.notifying = true;
            state.observed_while_notifying = false;
            state.pending_after_observation = false;
            target
        };

        loop {
            let notified = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                target.wake_by_ref();
            }));
            let next = {
                let mut state = lock_activity_notifier(&self.state);
                if matches!(state.lifecycle, ActivityNotifierLifecycle::Closed) || notified.is_err()
                {
                    state.notifying = false;
                    state.observed_while_notifying = false;
                    state.pending_after_observation = false;
                    None
                } else if state.pending_after_observation {
                    state.observed_while_notifying = false;
                    state.pending_after_observation = false;
                    if let Some(target) = state.target.as_ref().map(Arc::clone) {
                        Some(target)
                    } else {
                        state.notifying = false;
                        None
                    }
                } else {
                    state.notifying = false;
                    state.observed_while_notifying = false;
                    None
                }
            };
            drop(target);
            if let Err(payload) = notified {
                std::panic::resume_unwind(payload);
            }
            let Some(next) = next else {
                return;
            };
            target = next;
        }
    }
}

impl Wake for ActivityNotifier {
    fn wake(self: Arc<Self>) {
        self.notify();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.notify();
    }
}

fn lock_activity_notifier(
    state: &Mutex<ActivityNotifierState>,
) -> MutexGuard<'_, ActivityNotifierState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct DeadlineTimer {
    shared: Arc<DeadlineTimerShared>,
    thread: Option<JoinHandle<()>>,
}

struct DeadlineTimerShared {
    deadline: Instant,
    _activity: Arc<ExecutionActivity>,
    state: Mutex<DeadlineTimerState>,
    changed: Condvar,
}

#[derive(Default)]
struct DeadlineTimerState {
    stopped: bool,
    expired: bool,
    callback_in_flight: bool,
    waker: Option<Waker>,
}

impl DeadlineTimer {
    fn new(deadline: Instant, activity: Arc<ExecutionActivity>) -> Result<Self, ToolError> {
        let shared = Arc::new(DeadlineTimerShared {
            deadline,
            _activity: activity,
            state: Mutex::new(DeadlineTimerState::default()),
            changed: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("machine-god-terminal-deadline".to_owned())
            .spawn(move || run_deadline_timer(&worker_shared))
            .map_err(|_| execution_unavailable())?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    fn poll_expired(&self, context: &Context<'_>) -> Poll<()> {
        let mut incoming = Some(context.waker().clone());
        let (expired, replaced) = {
            let mut state = lock_deadline(&self.shared.state);
            if state.expired || Instant::now() >= self.shared.deadline {
                state.expired = true;
                (true, state.waker.take())
            } else {
                let replaced = match state.waker.as_ref() {
                    Some(existing) if existing.will_wake(context.waker()) => None,
                    Some(_) => state
                        .waker
                        .replace(incoming.take().expect("deadline incoming waker exists")),
                    None => {
                        state.waker = incoming.take();
                        None
                    }
                };
                (false, replaced)
            }
        };
        drop(replaced);
        drop(incoming);
        if expired {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl Drop for DeadlineTimer {
    fn drop(&mut self) {
        let (suppressed, callback_in_flight) = {
            let mut state = lock_deadline(&self.shared.state);
            state.stopped = true;
            (state.waker.take(), state.callback_in_flight)
        };
        drop(suppressed);
        self.shared.changed.notify_all();
        let Some(thread) = self.thread.take() else {
            return;
        };
        if thread.is_finished() || !callback_in_flight {
            let _ = thread.join();
        } else {
            // The shared execution activity keeps the originating admission
            // occupied through the actual thread return. Joining can deadlock
            // while a taken Waker is running against the polling task lock.
            drop(thread);
        }
    }
}

fn run_deadline_timer(shared: &DeadlineTimerShared) {
    let mut state = lock_deadline(&shared.state);
    loop {
        if state.stopped {
            return;
        }
        let now = Instant::now();
        if now >= shared.deadline {
            state.expired = true;
            let waker = state.waker.take();
            state.callback_in_flight = waker.is_some();
            drop(state);
            if let Some(waker) = waker {
                waker.wake();
                let mut state = lock_deadline(&shared.state);
                state.callback_in_flight = false;
            }
            return;
        }
        let waited = shared
            .changed
            .wait_timeout(state, shared.deadline.saturating_duration_since(now))
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state = waited.0;
    }
}

fn lock_deadline(state: &Mutex<DeadlineTimerState>) -> MutexGuard<'_, DeadlineTimerState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone)]
struct TerminalArguments {
    command: String,
    cwd: String,
}

impl Tool for TerminalTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: terminal_name(),
            description: TERMINAL_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["exec"] },
                    "command": { "type": "string" },
                    "cwd": { "type": "string", "default": "." },
                    "profile": { "type": "string", "enum": ["clean"], "default": "clean" }
                },
                "required": ["action", "command"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != terminal_name() {
            return Err(invalid_arguments());
        }
        let parsed = parse_arguments(&call.arguments, false)?;
        let canonical = canonical_arguments(&parsed);
        Ok(PreparedToolCall::new(
            Capability::Process {
                program: TERMINAL_PROGRAM.to_owned(),
                arguments: vec!["-c".to_owned(), parsed.command],
                working_directory: parsed.cwd,
                environment: ProcessEnvironment {
                    profile: TERMINAL_ENVIRONMENT_PROFILE.to_owned(),
                    sha256: self.environment.sha256.clone(),
                },
            },
            canonical,
        ))
    }

    fn execute(
        &self,
        _context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            let started = Instant::now();
            check_cancellation(&cancellation)?;
            let deadline = started
                .checked_add(self.limits.timeout)
                .ok_or_else(execution_unavailable)?;
            let parsed = parse_arguments(&arguments, true)?;
            if canonical_arguments(&parsed) != arguments {
                return Err(invalid_arguments());
            }
            if self.system_unsupported {
                return Err(unsupported_platform());
            }
            if Instant::now() >= deadline {
                check_cancellation(&cancellation)?;
                return render_timeout(&parsed.cwd, started);
            }
            let Some(activity) =
                ExecutionActivity::acquire(&self.active, self.limits.max_active_executions)
            else {
                check_cancellation(&cancellation)?;
                return if Instant::now() >= deadline {
                    render_timeout(&parsed.cwd, started)
                } else {
                    Err(busy())
                };
            };
            check_cancellation(&cancellation)?;
            let request = self.execution_request(
                parsed.clone(),
                started,
                deadline,
                Arc::clone(&activity),
                &cancellation,
            )?;
            check_cancellation(&cancellation)?;
            if Instant::now() >= deadline {
                let _ = activity.close_timeout();
                return render_timeout(&parsed.cwd, started);
            }
            let timer = DeadlineTimer::new(deadline, Arc::clone(&activity))?;
            let future = self.executor.execute(request, cancellation.clone());
            let outcome = await_executor(
                future,
                &cancellation,
                deadline,
                started,
                timer,
                Arc::clone(&activity),
            )
            .await;
            check_cancellation(&cancellation)?;
            let outcome = outcome?;
            let output = render_output(&parsed.cwd, &outcome)?;
            check_cancellation(&cancellation)?;
            Ok(output)
        })
    }
}

#[allow(dead_code)] // Used by Linux production and reference-host composition.
#[derive(Clone, Copy, Debug)]
struct SystemTerminalExecutor;

impl TerminalExecutor for SystemTerminalExecutor {
    fn execute(
        &self,
        request: TerminalExecutionRequest,
        cancellation: CancellationToken,
    ) -> TerminalExecution {
        #[cfg(target_os = "linux")]
        {
            Box::pin(SystemExecutionFuture::new(request, cancellation))
        }
        #[cfg(not(target_os = "linux"))]
        {
            Box::pin(async move {
                let _ = (request, cancellation);
                Err(TerminalExecutorError::new(
                    TerminalExecutorErrorKind::Unsupported,
                ))
            })
        }
    }
}

#[cfg(target_os = "linux")]
struct SystemExecutionFuture {
    cancellation: CancellationToken,
    cancelled: Pin<Box<machine_god_core::Cancelled>>,
    state: SystemExecutionState,
}

#[cfg(target_os = "linux")]
enum SystemExecutionState {
    Initial(Option<TerminalExecutionRequest>),
    Waiting(SystemWorkerHandle),
    Done,
}

#[cfg(target_os = "linux")]
impl SystemExecutionFuture {
    fn new(request: TerminalExecutionRequest, cancellation: CancellationToken) -> Self {
        Self {
            cancelled: Box::pin(cancellation.cancelled()),
            cancellation,
            state: SystemExecutionState::Initial(Some(request)),
        }
    }

    fn finish(
        &mut self,
        outcome: Result<TerminalExecutionOutcome, TerminalExecutorError>,
    ) -> Poll<Result<TerminalExecutionOutcome, TerminalExecutorError>> {
        self.state = SystemExecutionState::Done;
        Poll::Ready(outcome)
    }
}

#[cfg(target_os = "linux")]
impl Future for SystemExecutionFuture {
    type Output = Result<TerminalExecutionOutcome, TerminalExecutorError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            if matches!(self.state, SystemExecutionState::Initial(_))
                && self.cancellation.is_cancelled()
            {
                return self.finish(Err(executor_error(TerminalExecutorErrorKind::Cancelled)));
            }
            if let SystemExecutionState::Initial(request) = &mut self.state {
                let request = request.take().expect("initial terminal request exists");
                match SystemWorkerHandle::spawn(request, self.cancellation.clone()) {
                    Ok(worker) => {
                        self.state = SystemExecutionState::Waiting(worker);
                        continue;
                    }
                    Err(error) => return self.finish(Err(error)),
                }
            }
            if matches!(self.state, SystemExecutionState::Waiting(_)) {
                if self.cancelled.as_mut().poll(context).is_ready() {
                    let state = std::mem::replace(&mut self.state, SystemExecutionState::Done);
                    let SystemExecutionState::Waiting(worker) = state else {
                        unreachable!("terminal state was checked")
                    };
                    worker.abort_and_join();
                    return Poll::Ready(Err(executor_error(TerminalExecutorErrorKind::Cancelled)));
                }
                let output_limit = match &self.state {
                    SystemExecutionState::Waiting(worker) => worker.output_limit_observed(),
                    _ => unreachable!("terminal state was checked"),
                };
                if output_limit {
                    let state = std::mem::replace(&mut self.state, SystemExecutionState::Done);
                    let SystemExecutionState::Waiting(worker) = state else {
                        unreachable!("terminal state was checked")
                    };
                    return Poll::Ready(worker.join_output_limit());
                }
                let outcome = match &mut self.state {
                    SystemExecutionState::Waiting(worker) => worker.poll_outcome(context),
                    _ => unreachable!("terminal state was checked"),
                };
                if let Some(outcome) = outcome {
                    let state = std::mem::replace(&mut self.state, SystemExecutionState::Done);
                    let SystemExecutionState::Waiting(worker) = state else {
                        unreachable!("terminal state was checked")
                    };
                    return Poll::Ready(worker.join_finished(outcome));
                }
                let output_limit = match &self.state {
                    SystemExecutionState::Waiting(worker) => worker.output_limit_observed(),
                    _ => unreachable!("terminal state was checked"),
                };
                if output_limit {
                    continue;
                }
                return Poll::Pending;
            }
            panic!("terminal execution future polled after completion");
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for SystemExecutionFuture {
    fn drop(&mut self) {
        let state = std::mem::replace(&mut self.state, SystemExecutionState::Done);
        if let SystemExecutionState::Waiting(worker) = state {
            if worker.output_limit_observed() {
                let _ = worker.join_output_limit();
            } else {
                worker.abort_and_join();
            }
        }
    }
}

#[cfg(target_os = "linux")]
struct SystemWorkerHandle {
    shared: Arc<Mutex<SystemWorkerState>>,
    activity: Arc<ExecutionActivity>,
    thread: Option<JoinHandle<()>>,
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct SystemWorkerState {
    abort: bool,
    callback_in_flight: bool,
    outcome: Option<Result<TerminalExecutionOutcome, TerminalExecutorError>>,
    waker: Option<Waker>,
}

#[cfg(target_os = "linux")]
impl SystemWorkerHandle {
    fn spawn(
        request: TerminalExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<Self, TerminalExecutorError> {
        let shared = Arc::new(Mutex::new(SystemWorkerState::default()));
        let activity = Arc::clone(&request.activity);
        let worker_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("machine-god-terminal".to_owned())
            .spawn(move || system_worker(request, cancellation, worker_shared))
            .map_err(|_| executor_error(TerminalExecutorErrorKind::Spawn))?;
        Ok(Self {
            shared,
            activity,
            thread: Some(thread),
        })
    }

    fn output_limit_observed(&self) -> bool {
        matches!(self.activity.cause(), ExecutionCause::OutputLimit)
    }

    fn poll_outcome(
        &mut self,
        context: &Context<'_>,
    ) -> Option<Result<TerminalExecutionOutcome, TerminalExecutorError>> {
        let mut incoming = Some(context.waker().clone());
        let (outcome, replaced) = {
            let mut state = lock_worker(&self.shared);
            if let Some(outcome) = state.outcome.take() {
                (Some(outcome), state.waker.take())
            } else {
                let replaced = match state.waker.as_ref() {
                    Some(existing) if existing.will_wake(context.waker()) => None,
                    Some(_) => state
                        .waker
                        .replace(incoming.take().expect("terminal incoming waker exists")),
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

    fn abort_and_join(mut self) {
        let (suppressed, resources_released) = {
            let mut state = lock_worker(&self.shared);
            state.abort = true;
            (state.waker.take(), state.outcome.is_some())
        };
        drop(suppressed);
        if resources_released {
            let _ = self.finish_published();
        } else {
            self.join_aborted();
        }
    }

    fn join_output_limit(mut self) -> Result<TerminalExecutionOutcome, TerminalExecutorError> {
        self.join_without_abort()
    }

    fn join_without_abort(&mut self) -> Result<TerminalExecutionOutcome, TerminalExecutorError> {
        let (published, suppressed) = {
            let mut state = lock_worker(&self.shared);
            (state.outcome.take(), state.waker.take())
        };
        drop(suppressed);
        if let Some(outcome) = published {
            return if self.finish_published() {
                outcome
            } else {
                Err(executor_error(TerminalExecutorErrorKind::Invariant))
            };
        }

        let thread = self.thread.take().expect("terminal worker thread exists");
        if thread.thread().id() == thread::current().id() || thread.join().is_err() {
            return Err(executor_error(TerminalExecutorErrorKind::Invariant));
        }
        lock_worker(&self.shared)
            .outcome
            .take()
            .unwrap_or_else(|| Err(executor_error(TerminalExecutorErrorKind::Invariant)))
    }

    fn join_finished(
        mut self,
        outcome: Result<TerminalExecutionOutcome, TerminalExecutorError>,
    ) -> Result<TerminalExecutionOutcome, TerminalExecutorError> {
        if self.join_published() {
            outcome
        } else {
            Err(executor_error(TerminalExecutorErrorKind::Invariant))
        }
    }

    fn join_published(&mut self) -> bool {
        self.finish_published()
    }

    fn finish_published(&mut self) -> bool {
        let thread = self.thread.take().expect("terminal worker thread exists");
        let callback_in_flight = lock_worker(&self.shared).callback_in_flight;
        if thread.is_finished() || !callback_in_flight {
            thread.join().is_ok()
        } else {
            // The outcome is published only after the command, group, pipes,
            // readers, and request descriptor are released. An unfinished
            // handle can therefore be only an activity-retained Waker
            // notification tail, which must not be joined while that Waker may
            // be waiting for the polling task lock.
            drop(thread);
            true
        }
    }

    fn join_aborted(&mut self) {
        let thread = self.thread.take().expect("terminal worker thread exists");
        if thread.thread().id() == thread::current().id() {
            drop(thread);
        } else {
            let _ = thread.join();
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for SystemWorkerHandle {
    fn drop(&mut self) {
        if self.thread.is_some() {
            if self.output_limit_observed() {
                let _ = self.join_without_abort();
                return;
            }
            let resources_released = {
                let mut state = lock_worker(&self.shared);
                state.abort = true;
                state.waker = None;
                state.outcome.is_some()
            };
            if resources_released {
                let _ = self.finish_published();
            } else {
                self.join_aborted();
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn lock_worker(shared: &Mutex<SystemWorkerState>) -> MutexGuard<'_, SystemWorkerState> {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(target_os = "linux")]
fn system_worker(
    request: TerminalExecutionRequest,
    cancellation: CancellationToken,
    shared: Arc<Mutex<SystemWorkerState>>,
) {
    let outcome = run_system_command(&request, &cancellation, &shared);
    let _activity = Arc::clone(&request.activity);
    drop(request);
    let waker = {
        let mut state = lock_worker(&shared);
        state.outcome = Some(outcome);
        let waker = state.waker.take();
        state.callback_in_flight = waker.is_some();
        waker
    };
    if let Some(waker) = waker {
        waker.wake();
        let mut state = lock_worker(&shared);
        state.callback_in_flight = false;
    }
    drop((cancellation, shared));
}

#[cfg(target_os = "linux")]
#[allow(clippy::too_many_lines)] // Linear ownership protocol is easier to audit in one scope.
fn run_system_command(
    request: &TerminalExecutionRequest,
    cancellation: &CancellationToken,
    shared: &Mutex<SystemWorkerState>,
) -> Result<TerminalExecutionOutcome, TerminalExecutorError> {
    if cancellation.is_cancelled() || lock_worker(shared).abort {
        return Err(executor_error(TerminalExecutorErrorKind::Cancelled));
    }
    if Instant::now() >= request.deadline() {
        let _ = request.activity.close_timeout();
        return empty_outcome(
            TerminalExecutionStatus::TimedOut,
            bounded_duration(request.started.elapsed()),
        );
    }
    let mut command = Command::new(TERMINAL_PROGRAM);
    command
        .arg("-c")
        .arg(request.command())
        .current_dir(request.proc_path())
        .env_clear()
        .envs(request.environment().iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let spawn = {
        let state = lock_worker(shared);
        if cancellation.is_cancelled() || state.abort {
            None
        } else if Instant::now() >= request.deadline() {
            let _ = request.activity.close_timeout();
            return empty_outcome(
                TerminalExecutionStatus::TimedOut,
                bounded_duration(request.started.elapsed()),
            );
        } else {
            Some(command.spawn())
        }
    };
    let Some(spawn) = spawn else {
        return Err(executor_error(TerminalExecutorErrorKind::Cancelled));
    };
    let mut child = match spawn {
        Ok(child) => child,
        Err(_) if cancellation.is_cancelled() || lock_worker(shared).abort => {
            return Err(executor_error(TerminalExecutorErrorKind::Cancelled));
        }
        Err(_) => return Err(executor_error(TerminalExecutorErrorKind::Spawn)),
    };
    let group = rustix::process::Pid::from_raw(
        i32::try_from(child.id())
            .map_err(|_| executor_error(TerminalExecutorErrorKind::Invariant))?,
    )
    .ok_or_else(|| executor_error(TerminalExecutorErrorKind::Invariant))?;
    let Some(stdout) = child.stdout.take() else {
        terminate_and_reap(&mut child, group)?;
        return Err(executor_error(TerminalExecutorErrorKind::Pipe));
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_and_reap(&mut child, group)?;
        return Err(executor_error(TerminalExecutorErrorKind::Pipe));
    };

    let stop_readers = Arc::new(AtomicBool::new(false));
    let produced = Arc::new(AtomicU64::new(0));
    let stdout_reader = spawn_reader(
        "machine-god-terminal-stdout",
        stdout,
        Arc::clone(&stop_readers),
        Arc::clone(&produced),
        Arc::clone(&request.activity),
    );
    let stdout_reader = match stdout_reader {
        Ok(reader) => reader,
        Err(error) => {
            stop_readers.store(true, Ordering::Release);
            terminate_and_reap(&mut child, group)?;
            return Err(error);
        }
    };
    let stderr_reader = spawn_reader(
        "machine-god-terminal-stderr",
        stderr,
        Arc::clone(&stop_readers),
        Arc::clone(&produced),
        Arc::clone(&request.activity),
    );
    let stderr_reader = match stderr_reader {
        Ok(reader) => reader,
        Err(error) => {
            stop_readers.store(true, Ordering::Release);
            let cleanup = terminate_and_reap(&mut child, group);
            let _ = stdout_reader.join();
            cleanup?;
            return Err(error);
        }
    };

    let (mut terminal_status, cleanup) = loop {
        if cancellation.is_cancelled() || lock_worker(shared).abort {
            stop_readers.store(true, Ordering::Release);
            let cleanup = terminate_and_reap(&mut child, group);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            cleanup?;
            return Err(executor_error(TerminalExecutorErrorKind::Cancelled));
        }
        match request.activity.cause() {
            ExecutionCause::OutputLimit => {
                stop_readers.store(true, Ordering::Release);
                break (
                    TerminalExecutionStatus::OutputLimit,
                    terminate_and_reap(&mut child, group),
                );
            }
            ExecutionCause::TimedOut => {
                stop_readers.store(true, Ordering::Release);
                break (
                    TerminalExecutionStatus::TimedOut,
                    terminate_and_reap(&mut child, group),
                );
            }
            ExecutionCause::Open => {}
        }
        if Instant::now() >= request.deadline() {
            stop_readers.store(true, Ordering::Release);
            let cause = request.activity.close_timeout();
            break (
                match cause {
                    ExecutionCause::OutputLimit => TerminalExecutionStatus::OutputLimit,
                    ExecutionCause::Open | ExecutionCause::TimedOut => {
                        TerminalExecutionStatus::TimedOut
                    }
                },
                terminate_and_reap(&mut child, group),
            );
        }
        match observe_leader_exit(group) {
            Ok(Some(status)) => {
                stop_readers.store(true, Ordering::Release);
                break (status, cleanup_observed_leader(&mut child, group, status));
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(_) => {
                stop_readers.store(true, Ordering::Release);
                let cleanup = terminate_and_reap(&mut child, group);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                cleanup?;
                return Err(executor_error(TerminalExecutorErrorKind::Wait));
            }
        }
    };

    stop_readers.store(true, Ordering::Release);
    let stdout = stdout_reader.join();
    let stderr = stderr_reader.join();
    let stdout = stdout.map_err(|_| executor_error(TerminalExecutorErrorKind::Pipe))??;
    let stderr = stderr.map_err(|_| executor_error(TerminalExecutorErrorKind::Pipe))??;
    cleanup?;
    let produced = stdout.total_bytes().saturating_add(stderr.total_bytes());
    if produced > MAX_TERMINAL_PRODUCED_OUTPUT_BYTES {
        let _ = request.activity.claim_output_limit();
    }
    match request.activity.cause() {
        ExecutionCause::OutputLimit => terminal_status = TerminalExecutionStatus::OutputLimit,
        ExecutionCause::TimedOut if produced > MAX_TERMINAL_PRODUCED_OUTPUT_BYTES => {
            return empty_outcome(
                TerminalExecutionStatus::TimedOut,
                bounded_duration(request.started.elapsed()),
            );
        }
        ExecutionCause::Open | ExecutionCause::TimedOut => {}
    }
    TerminalExecutionOutcome::new(
        terminal_status,
        stdout,
        stderr,
        bounded_duration(request.started.elapsed()),
    )
}

#[cfg(target_os = "linux")]
fn exit_status(status: ExitStatus) -> TerminalExecutionStatus {
    if let Some(code) = status.code() {
        TerminalExecutionStatus::Exited(code)
    } else {
        TerminalExecutionStatus::Signaled(status.signal().unwrap_or(0))
    }
}

#[cfg(target_os = "linux")]
fn observe_leader_exit(
    leader: rustix::process::Pid,
) -> Result<Option<TerminalExecutionStatus>, TerminalExecutorError> {
    let status = rustix::process::waitid(
        rustix::process::WaitId::Pid(leader),
        rustix::process::WaitIdOptions::EXITED
            | rustix::process::WaitIdOptions::NOHANG
            | rustix::process::WaitIdOptions::NOWAIT,
    )
    .map_err(|_| executor_error(TerminalExecutorErrorKind::Wait))?;
    status.map(waitid_status).transpose()
}

#[cfg(target_os = "linux")]
fn waitid_status(
    status: rustix::process::WaitIdStatus,
) -> Result<TerminalExecutionStatus, TerminalExecutorError> {
    if let Some(code) = status.exit_status() {
        Ok(TerminalExecutionStatus::Exited(code))
    } else if let Some(signal) = status.terminating_signal() {
        Ok(TerminalExecutionStatus::Signaled(signal))
    } else {
        Err(executor_error(TerminalExecutorErrorKind::Invariant))
    }
}

#[cfg(target_os = "linux")]
trait PipeReader: Read + AsFd + Send + 'static {}
#[cfg(target_os = "linux")]
impl PipeReader for ChildStdout {}
#[cfg(target_os = "linux")]
impl PipeReader for ChildStderr {}

#[cfg(target_os = "linux")]
fn spawn_reader<R: PipeReader>(
    name: &str,
    reader: R,
    stop: Arc<AtomicBool>,
    produced: Arc<AtomicU64>,
    activity: Arc<ExecutionActivity>,
) -> Result<JoinHandle<Result<TerminalCapturedOutput, TerminalExecutorError>>, TerminalExecutorError>
{
    let flags = rustix::fs::fcntl_getfl(&reader)
        .map_err(|_| executor_error(TerminalExecutorErrorKind::Pipe))?;
    rustix::fs::fcntl_setfl(&reader, flags | OFlags::NONBLOCK)
        .map_err(|_| executor_error(TerminalExecutorErrorKind::Pipe))?;
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || read_pipe(reader, &stop, &produced, &activity))
        .map_err(|_| executor_error(TerminalExecutorErrorKind::Pipe))
}

#[cfg(target_os = "linux")]
fn read_pipe(
    mut reader: impl Read,
    stop: &AtomicBool,
    produced: &AtomicU64,
    activity: &ExecutionActivity,
) -> Result<TerminalCapturedOutput, TerminalExecutorError> {
    let mut capture = PipeCapture::default();
    let mut buffer = [0_u8; PIPE_READ_BYTES];
    let mut stopping_reads = 0_u8;
    loop {
        if matches!(activity.cause(), ExecutionCause::OutputLimit) {
            break;
        }
        if stop.load(Ordering::Acquire) {
            stopping_reads = stopping_reads.saturating_add(1);
            if stopping_reads > POST_STOP_READ_LIMIT {
                break;
            }
        }
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                let previous = produced.fetch_add(length as u64, Ordering::AcqRel);
                let total = previous.saturating_add(length as u64);
                capture.push(&buffer[..length]);
                if total > MAX_TERMINAL_PRODUCED_OUTPUT_BYTES {
                    let _ = activity.claim_output_limit();
                    stop.store(true, Ordering::Release);
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                return Err(executor_error(TerminalExecutorErrorKind::Pipe));
            }
        }
    }
    capture.finish()
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct PipeCapture {
    head: Vec<u8>,
    tail: Vec<u8>,
    tail_start: usize,
    total: u64,
}

#[cfg(target_os = "linux")]
impl PipeCapture {
    fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len() as u64);
        let head_limit = PIPE_RETAINED_BYTES / 2;
        let missing_head = head_limit.saturating_sub(self.head.len());
        let head_bytes = std::cmp::min(missing_head, bytes.len());
        self.head.extend_from_slice(&bytes[..head_bytes]);
        let bytes = &bytes[head_bytes..];
        let tail_limit = PIPE_RETAINED_BYTES - head_limit;
        let missing_tail = tail_limit.saturating_sub(self.tail.len());
        let appended = std::cmp::min(missing_tail, bytes.len());
        self.tail.extend_from_slice(&bytes[..appended]);
        let bytes = &bytes[appended..];
        if bytes.is_empty() {
            return;
        }
        debug_assert_eq!(self.tail.len(), tail_limit);
        if bytes.len() >= tail_limit {
            self.tail
                .copy_from_slice(&bytes[bytes.len() - tail_limit..]);
            self.tail_start = 0;
            return;
        }
        let first = std::cmp::min(bytes.len(), tail_limit - self.tail_start);
        self.tail[self.tail_start..self.tail_start + first].copy_from_slice(&bytes[..first]);
        self.tail[..bytes.len() - first].copy_from_slice(&bytes[first..]);
        self.tail_start = (self.tail_start + bytes.len()) % tail_limit;
    }

    fn finish(self) -> Result<TerminalCapturedOutput, TerminalExecutorError> {
        let mut retained = self.head;
        retained.extend_from_slice(&self.tail[self.tail_start..]);
        retained.extend_from_slice(&self.tail[..self.tail_start]);
        TerminalCapturedOutput::new(retained, self.total)
    }
}

#[cfg(target_os = "linux")]
fn terminate_and_reap(
    child: &mut Child,
    group: rustix::process::Pid,
) -> Result<(), TerminalExecutorError> {
    let term = signal_process_group(group, rustix::process::Signal::TERM);
    let kill = match term {
        Ok(true) => {
            sleep_through_grace(PROCESS_GROUP_TERM_GRACE);
            signal_process_group(group, rustix::process::Signal::KILL)
        }
        Ok(false) => Ok(false),
        Err(error) => {
            let _ = child.kill();
            Err(error)
        }
    };
    let reaped = child
        .wait()
        .map(|_| ())
        .map_err(|_| executor_error(TerminalExecutorErrorKind::Wait));
    let disappeared = observe_group_disappearance(group);
    term?;
    kill?;
    reaped?;
    disappeared
}

#[cfg(target_os = "linux")]
fn cleanup_observed_leader(
    child: &mut Child,
    group: rustix::process::Pid,
    observed: TerminalExecutionStatus,
) -> Result<(), TerminalExecutorError> {
    let term = signal_process_group(group, rustix::process::Signal::TERM);
    let kill = match term {
        Ok(true) => {
            // The waitid-NOWAIT leader remains a zombie through every numeric
            // group signal, pinning its PID/PGID identity against reuse. The
            // direct child has already exited, so no foreground process needs a
            // TERM grace before remaining original-group members are killed.
            signal_process_group(group, rustix::process::Signal::KILL)
        }
        Ok(false) => Ok(false),
        Err(error) => Err(error),
    };
    let reaped = child
        .wait()
        .map_err(|_| executor_error(TerminalExecutorErrorKind::Wait));
    let disappeared = observe_group_disappearance(group);
    term?;
    kill?;
    let reaped = reaped?;
    if exit_status(reaped) != observed {
        return Err(executor_error(TerminalExecutorErrorKind::Invariant));
    }
    disappeared
}

#[cfg(target_os = "linux")]
fn signal_process_group(
    group: rustix::process::Pid,
    signal: rustix::process::Signal,
) -> Result<bool, TerminalExecutorError> {
    match rustix::process::kill_process_group(group, signal) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::SRCH) => Ok(false),
        Err(_) => Err(executor_error(TerminalExecutorErrorKind::Wait)),
    }
}

#[cfg(target_os = "linux")]
fn observe_group_disappearance(group: rustix::process::Pid) -> Result<(), TerminalExecutorError> {
    let deadline = Instant::now() + PROCESS_GROUP_KILL_OBSERVATION_GRACE;
    loop {
        match rustix::process::test_kill_process_group(group) {
            Err(rustix::io::Errno::SRCH) => return Ok(()),
            Ok(()) if Instant::now() < deadline => thread::sleep(Duration::from_millis(2)),
            // A signalable group, EPERM, and every other probe failure are all
            // ambiguous. Only ESRCH proves that the original numeric group is
            // no longer observable after the leader has been reaped.
            Ok(()) | Err(_) => {
                return Err(executor_error(TerminalExecutorErrorKind::Wait));
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn sleep_through_grace(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        thread::sleep(std::cmp::min(
            Duration::from_millis(2),
            deadline.saturating_duration_since(Instant::now()),
        ));
    }
}

#[cfg(target_os = "linux")]
fn executor_error(kind: TerminalExecutorErrorKind) -> TerminalExecutorError {
    TerminalExecutorError::new(kind)
}

fn map_executor_error(error: TerminalExecutorError) -> ToolError {
    match error.kind() {
        TerminalExecutorErrorKind::Unsupported => unsupported_platform(),
        TerminalExecutorErrorKind::Busy => busy(),
        TerminalExecutorErrorKind::Cancelled => cancelled_error(),
        TerminalExecutorErrorKind::Spawn => fixed_tool_error(
            ToolErrorKind::Execution,
            "terminal_spawn_failed",
            "terminal process could not be started",
            true,
        ),
        TerminalExecutorErrorKind::Wait => fixed_tool_error(
            ToolErrorKind::Execution,
            "terminal_wait_failed",
            "terminal process could not be reaped",
            false,
        ),
        TerminalExecutorErrorKind::Pipe => fixed_tool_error(
            ToolErrorKind::Execution,
            "terminal_pipe_failed",
            "terminal output capture failed",
            false,
        ),
        TerminalExecutorErrorKind::Invariant | TerminalExecutorErrorKind::InvalidResponse => {
            executor_invariant()
        }
    }
}

fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(cancelled_error())
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    fixed_tool_error(
        ToolErrorKind::InvalidInput,
        "terminal_invalid_arguments",
        "terminal arguments are invalid",
        false,
    )
}

fn invalid_command() -> ToolError {
    fixed_tool_error(
        ToolErrorKind::InvalidInput,
        "terminal_invalid_command",
        "terminal command is invalid",
        false,
    )
}

fn invalid_cwd() -> ToolError {
    fixed_tool_error(
        ToolErrorKind::InvalidInput,
        "terminal_invalid_cwd",
        "terminal working directory is invalid",
        false,
    )
}

#[cfg(unix)]
fn cwd_unavailable() -> ToolError {
    fixed_tool_error(
        ToolErrorKind::Unavailable,
        "terminal_cwd_unavailable",
        "terminal working directory is unavailable",
        false,
    )
}

fn busy() -> ToolError {
    fixed_tool_error(
        ToolErrorKind::Unavailable,
        "terminal_busy",
        "terminal execution capacity is busy",
        true,
    )
}

fn unsupported_platform() -> ToolError {
    fixed_tool_error(
        ToolErrorKind::Unavailable,
        "terminal_unsupported",
        "terminal execution is unsupported on this platform",
        false,
    )
}

fn execution_unavailable() -> ToolError {
    fixed_tool_error(
        ToolErrorKind::Unavailable,
        "terminal_unavailable",
        "terminal execution is unavailable",
        true,
    )
}

fn executor_invariant() -> ToolError {
    fixed_tool_error(
        ToolErrorKind::Execution,
        "terminal_executor_failed",
        "terminal executor failed",
        false,
    )
}

fn cancelled_error() -> ToolError {
    fixed_tool_error(
        ToolErrorKind::Cancelled,
        "terminal_cancelled",
        "terminal execution was cancelled",
        false,
    )
}

fn fixed_tool_error(
    kind: ToolErrorKind,
    code: &'static str,
    message: &'static str,
    retryable: bool,
) -> ToolError {
    ToolError::new(kind, code, message, retryable)
}

#[cfg(test)]
mod tests {
    use super::{
        ActivityNotifier, ExecutionActivity, ExecutionCause, MAX_TERMINAL_PRODUCED_OUTPUT_BYTES,
        TERMINAL_DEFAULT_MAX_ACTIVE_EXECUTIONS, TERMINAL_DEFAULT_TIMEOUT,
        TERMINAL_MAX_ACTIVE_EXECUTIONS, TERMINAL_MAX_TIMEOUT, TerminalCapturedOutput,
        TerminalExecution, TerminalExecutionOutcome, TerminalExecutionRequest,
        TerminalExecutionStatus, TerminalExecutor, TerminalExecutorError,
        TerminalExecutorErrorKind, TerminalLimits, TerminalTool, validate_cwd,
    };
    use machine_god_core::{
        CancellationToken, SessionId, SessionIncarnationId, Tool, ToolCallId, ToolContext,
        ToolErrorKind, TurnId,
    };
    use serde_json::json;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::{Duration, Instant};

    fn empty_capture() -> TerminalCapturedOutput {
        TerminalCapturedOutput::new(Vec::new(), 0).unwrap()
    }

    struct BlockingNotifierTarget {
        calls: AtomicUsize,
        entered: Mutex<bool>,
        changed: Condvar,
        released: AtomicBool,
        in_flight: Arc<AtomicUsize>,
        maximum_in_flight: Arc<AtomicUsize>,
    }

    impl BlockingNotifierTarget {
        fn new(in_flight: Arc<AtomicUsize>, maximum_in_flight: Arc<AtomicUsize>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                entered: Mutex::new(false),
                changed: Condvar::new(),
                released: AtomicBool::new(false),
                in_flight,
                maximum_in_flight,
            }
        }

        fn wait_until_entered(&self) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut entered = self
                .entered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while !*entered {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "notifier callback did not run");
                entered = self
                    .changed
                    .wait_timeout(entered, remaining)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
            }
        }

        fn release(&self) {
            self.released.store(true, Ordering::Release);
            self.changed.notify_all();
        }

        fn notify(&self) {
            self.calls.fetch_add(1, Ordering::AcqRel);
            enter_notifier_callback(&self.in_flight, &self.maximum_in_flight);
            {
                let mut entered = self
                    .entered
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *entered = true;
                self.changed.notify_all();
                while !self.released.load(Ordering::Acquire) {
                    entered = self
                        .changed
                        .wait(entered)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            }
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
        }
    }

    impl Wake for BlockingNotifierTarget {
        fn wake(self: Arc<Self>) {
            self.notify();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.notify();
        }
    }

    struct CountingNotifierTarget {
        calls: AtomicUsize,
        in_flight: Arc<AtomicUsize>,
        maximum_in_flight: Arc<AtomicUsize>,
    }

    impl CountingNotifierTarget {
        fn notify(&self) {
            self.calls.fetch_add(1, Ordering::AcqRel);
            enter_notifier_callback(&self.in_flight, &self.maximum_in_flight);
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
        }
    }

    impl Wake for CountingNotifierTarget {
        fn wake(self: Arc<Self>) {
            self.notify();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.notify();
        }
    }

    fn enter_notifier_callback(in_flight: &AtomicUsize, maximum_in_flight: &AtomicUsize) {
        let current = in_flight.fetch_add(1, Ordering::AcqRel) + 1;
        maximum_in_flight.fetch_max(current, Ordering::AcqRel);
    }

    struct PanicOnceNotifierTarget(AtomicUsize);

    impl PanicOnceNotifierTarget {
        fn notify(&self) {
            assert_ne!(
                self.0.fetch_add(1, Ordering::AcqRel),
                0,
                "intentional notifier target panic"
            );
        }
    }

    impl Wake for PanicOnceNotifierTarget {
        fn wake(self: Arc<Self>) {
            self.notify();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.notify();
        }
    }

    struct DropObservingNotifierTarget {
        active: Arc<AtomicUsize>,
        dropped: Arc<AtomicBool>,
    }

    impl Wake for DropObservingNotifierTarget {
        fn wake(self: Arc<Self>) {}
    }

    impl Drop for DropObservingNotifierTarget {
        fn drop(&mut self) {
            assert_eq!(self.active.load(Ordering::Acquire), 1);
            self.dropped.store(true, Ordering::Release);
        }
    }

    #[test]
    fn terminal_limits_have_exact_defaults_and_bounds() {
        let defaults = TerminalLimits::default();
        assert_eq!(defaults.timeout(), TERMINAL_DEFAULT_TIMEOUT);
        assert_eq!(
            defaults.max_active_executions(),
            TERMINAL_DEFAULT_MAX_ACTIVE_EXECUTIONS
        );

        let minimum = TerminalLimits::new(Duration::from_millis(1), 1).unwrap();
        assert_eq!(minimum.timeout(), Duration::from_millis(1));
        assert_eq!(minimum.max_active_executions(), 1);

        let maximum =
            TerminalLimits::new(TERMINAL_MAX_TIMEOUT, TERMINAL_MAX_ACTIVE_EXECUTIONS).unwrap();
        assert_eq!(maximum.timeout(), TERMINAL_MAX_TIMEOUT);
        assert_eq!(
            maximum.max_active_executions(),
            TERMINAL_MAX_ACTIVE_EXECUTIONS
        );

        assert!(TerminalLimits::new(Duration::ZERO, 1).is_err());
        assert!(TerminalLimits::new(TERMINAL_MAX_TIMEOUT + Duration::from_nanos(1), 1).is_err());
        assert!(TerminalLimits::new(Duration::from_millis(1), 0).is_err());
        assert!(
            TerminalLimits::new(Duration::from_millis(1), TERMINAL_MAX_ACTIVE_EXECUTIONS + 1)
                .is_err()
        );
    }

    #[test]
    fn cwd_rejects_only_an_exact_tilde_component() {
        assert!(validate_cwd("~cache").is_ok());
        assert!(validate_cwd("parent/~cache").is_ok());
        assert!(validate_cwd("~").is_err());
        assert!(validate_cwd("parent/~/child").is_err());
    }

    #[test]
    fn one_execution_activity_owns_exactly_one_slot_across_every_tail() {
        let active = Arc::new(AtomicUsize::new(0));
        let activity = ExecutionActivity::acquire(&active, 1).expect("first activity");
        let worker = Arc::clone(&activity);
        let guardian = Arc::clone(&activity);
        let notifier = Arc::new(ActivityNotifier::new(Arc::clone(&activity)));
        let worker_waker = Waker::from(Arc::clone(&notifier));
        notifier.bind(Waker::noop(), &worker_waker);
        let guardian_waker = worker_waker.clone();
        assert_eq!(active.load(Ordering::Acquire), 1);

        drop(activity);
        assert!(ExecutionActivity::acquire(&active, 1).is_none());
        drop(worker);
        drop(guardian);
        drop(worker_waker);
        assert!(ExecutionActivity::acquire(&active, 1).is_none());
        drop(guardian_waker);
        drop(notifier);

        let recovered = ExecutionActivity::acquire(&active, 1).expect("activity tails finished");
        assert_eq!(active.load(Ordering::Acquire), 1);
        drop(recovered);
        assert_eq!(active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn activity_notifier_replays_notice_after_poll_during_callback() {
        let active = Arc::new(AtomicUsize::new(0));
        let activity = ExecutionActivity::acquire(&active, 1).unwrap();
        let notifier = Arc::new(ActivityNotifier::new(Arc::clone(&activity)));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let maximum_in_flight = Arc::new(AtomicUsize::new(0));
        let blocking = Arc::new(BlockingNotifierTarget::new(
            Arc::clone(&in_flight),
            Arc::clone(&maximum_in_flight),
        ));
        let blocking_waker = Waker::from(Arc::clone(&blocking));
        let notifier_waker = Waker::from(Arc::clone(&notifier));
        notifier.bind(&blocking_waker, &notifier_waker);

        let rebound = Arc::new(CountingNotifierTarget {
            calls: AtomicUsize::new(0),
            in_flight: Arc::clone(&in_flight),
            maximum_in_flight: Arc::clone(&maximum_in_flight),
        });
        let rebound_waker = Waker::from(Arc::clone(&rebound));

        std::thread::scope(|scope| {
            let first_waker = notifier_waker.clone();
            let first = scope.spawn(move || first_waker.wake());
            blocking.wait_until_entered();
            notifier.bind(&rebound_waker, &notifier_waker);

            let mut burst = Vec::new();
            for _ in 0..32 {
                let burst_waker = notifier_waker.clone();
                burst.push(scope.spawn(move || burst_waker.wake()));
            }
            for wake in burst {
                wake.join().unwrap();
            }
            assert_eq!(blocking.calls.load(Ordering::Acquire), 1);
            assert_eq!(rebound.calls.load(Ordering::Acquire), 0);

            blocking.release();
            first.join().unwrap();
        });

        assert_eq!(blocking.calls.load(Ordering::Acquire), 1);
        assert_eq!(rebound.calls.load(Ordering::Acquire), 1);
        assert_eq!(maximum_in_flight.load(Ordering::Acquire), 1);

        notifier_waker.wake_by_ref();
        assert_eq!(rebound.calls.load(Ordering::Acquire), 2);

        drop(activity);
        assert_eq!(active.load(Ordering::Acquire), 1);
        drop(notifier_waker);
        drop(notifier);
        assert_eq!(active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn activity_notifier_coalesces_notices_observed_by_later_repoll() {
        let active = Arc::new(AtomicUsize::new(0));
        let activity = ExecutionActivity::acquire(&active, 1).unwrap();
        let notifier = Arc::new(ActivityNotifier::new(Arc::clone(&activity)));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let maximum_in_flight = Arc::new(AtomicUsize::new(0));
        let blocking = Arc::new(BlockingNotifierTarget::new(
            Arc::clone(&in_flight),
            Arc::clone(&maximum_in_flight),
        ));
        let blocking_waker = Waker::from(Arc::clone(&blocking));
        let notifier_waker = Waker::from(Arc::clone(&notifier));
        notifier.bind(&blocking_waker, &notifier_waker);

        std::thread::scope(|scope| {
            let first_waker = notifier_waker.clone();
            let first = scope.spawn(move || first_waker.wake());
            blocking.wait_until_entered();

            let mut burst = Vec::new();
            for _ in 0..32 {
                let burst_waker = notifier_waker.clone();
                burst.push(scope.spawn(move || burst_waker.wake()));
            }
            for wake in burst {
                wake.join().unwrap();
            }

            notifier.bind(&blocking_waker, &notifier_waker);
            blocking.release();
            first.join().unwrap();
        });

        assert_eq!(blocking.calls.load(Ordering::Acquire), 1);
        assert_eq!(maximum_in_flight.load(Ordering::Acquire), 1);
        notifier_waker.wake_by_ref();
        assert_eq!(blocking.calls.load(Ordering::Acquire), 2);
        assert_eq!(maximum_in_flight.load(Ordering::Acquire), 1);
    }

    #[test]
    fn activity_notifier_self_bind_preserves_the_external_target() {
        let active = Arc::new(AtomicUsize::new(0));
        let activity = ExecutionActivity::acquire(&active, 1).unwrap();
        let notifier = Arc::new(ActivityNotifier::new(Arc::clone(&activity)));
        let notifier_waker = Waker::from(Arc::clone(&notifier));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let maximum_in_flight = Arc::new(AtomicUsize::new(0));
        let target = Arc::new(CountingNotifierTarget {
            calls: AtomicUsize::new(0),
            in_flight,
            maximum_in_flight,
        });
        let target_waker = Waker::from(Arc::clone(&target));

        notifier.bind(&target_waker, &notifier_waker);
        notifier.bind(&notifier_waker, &notifier_waker);
        notifier_waker.wake_by_ref();
        assert_eq!(target.calls.load(Ordering::Acquire), 1);

        notifier.close();
        notifier.bind(&target_waker, &notifier_waker);
        notifier_waker.wake_by_ref();
        assert_eq!(target.calls.load(Ordering::Acquire), 1);

        drop((target_waker, target, activity, notifier));
        assert_eq!(active.load(Ordering::Acquire), 1);
        drop(notifier_waker);
        assert_eq!(active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn activity_notifier_close_suppresses_an_in_flight_replay() {
        let active = Arc::new(AtomicUsize::new(0));
        let activity = ExecutionActivity::acquire(&active, 1).unwrap();
        let notifier = Arc::new(ActivityNotifier::new(Arc::clone(&activity)));
        let notifier_waker = Waker::from(Arc::clone(&notifier));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let maximum_in_flight = Arc::new(AtomicUsize::new(0));
        let blocking = Arc::new(BlockingNotifierTarget::new(
            Arc::clone(&in_flight),
            Arc::clone(&maximum_in_flight),
        ));
        let blocking_waker = Waker::from(Arc::clone(&blocking));
        notifier.bind(&blocking_waker, &notifier_waker);
        let rebound = Arc::new(CountingNotifierTarget {
            calls: AtomicUsize::new(0),
            in_flight,
            maximum_in_flight: Arc::clone(&maximum_in_flight),
        });
        let rebound_waker = Waker::from(Arc::clone(&rebound));

        let first_waker = notifier_waker.clone();
        let first = std::thread::spawn(move || first_waker.wake());
        blocking.wait_until_entered();
        notifier.bind(&rebound_waker, &notifier_waker);
        notifier_waker.wake_by_ref();
        notifier.close();
        notifier.bind(&rebound_waker, &notifier_waker);
        notifier_waker.wake_by_ref();

        assert_eq!(blocking.calls.load(Ordering::Acquire), 1);
        assert_eq!(rebound.calls.load(Ordering::Acquire), 0);
        assert_eq!(maximum_in_flight.load(Ordering::Acquire), 1);
        drop((activity, notifier, notifier_waker));
        assert_eq!(active.load(Ordering::Acquire), 1);
        blocking.release();
        first.join().unwrap();
        assert_eq!(active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn activity_notifier_recovers_after_target_panic() {
        let active = Arc::new(AtomicUsize::new(0));
        let activity = ExecutionActivity::acquire(&active, 1).unwrap();
        let notifier = Arc::new(ActivityNotifier::new(Arc::clone(&activity)));
        let target = Arc::new(PanicOnceNotifierTarget(AtomicUsize::new(0)));
        let target_waker = Waker::from(Arc::clone(&target));
        let notifier_waker = Waker::from(Arc::clone(&notifier));
        notifier.bind(&target_waker, &notifier_waker);

        let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            notifier_waker.wake_by_ref();
        }));
        assert!(first.is_err());
        notifier_waker.wake_by_ref();
        assert_eq!(target.0.load(Ordering::Acquire), 2);
    }

    #[test]
    fn activity_notifier_close_drops_target_before_releasing_activity() {
        let active = Arc::new(AtomicUsize::new(0));
        let activity = ExecutionActivity::acquire(&active, 1).unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let target = Arc::new(DropObservingNotifierTarget {
            active: Arc::clone(&active),
            dropped: Arc::clone(&dropped),
        });
        let target_waker = Waker::from(Arc::clone(&target));
        let notifier = Arc::new(ActivityNotifier::new(Arc::clone(&activity)));
        let notifier_waker = Waker::from(Arc::clone(&notifier));
        notifier.bind(&target_waker, &notifier_waker);

        drop(target_waker);
        drop(target);
        drop(activity);
        assert!(!dropped.load(Ordering::Acquire));
        notifier.close();
        assert!(dropped.load(Ordering::Acquire));
        drop(notifier);
        assert_eq!(active.load(Ordering::Acquire), 1);
        drop(notifier_waker);
        assert_eq!(active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn execution_cause_has_one_linearizable_winner() {
        let active = Arc::new(AtomicUsize::new(0));
        let output = ExecutionActivity::acquire(&active, 1).unwrap();
        assert!(output.claim_output_limit());
        assert_eq!(output.close_timeout(), ExecutionCause::OutputLimit);
        drop(output);

        let timeout = ExecutionActivity::acquire(&active, 1).unwrap();
        assert_eq!(timeout.close_timeout(), ExecutionCause::TimedOut);
        assert!(!timeout.claim_output_limit());
        assert_eq!(timeout.cause(), ExecutionCause::TimedOut);
    }

    #[test]
    fn no_waker_thread_gap_keeps_the_originating_slot_occupied() {
        let active = Arc::new(AtomicUsize::new(0));
        let activity = ExecutionActivity::acquire(&active, 1).unwrap();
        let thread_activity = Arc::clone(&activity);
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let thread_entered = Arc::clone(&entered);
        let thread_release = Arc::clone(&release);
        let thread = std::thread::spawn(move || {
            thread_entered.wait();
            thread_release.wait();
            drop(thread_activity);
        });

        drop(activity);
        entered.wait();
        assert_eq!(active.load(Ordering::Acquire), 1);
        assert!(ExecutionActivity::acquire(&active, 1).is_none());
        release.wait();
        thread.join().unwrap();
        assert_eq!(active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn system_constructor_uses_requested_limits_or_fixed_unsupported() {
        let limits = TerminalLimits::new(Duration::from_millis(7), 2).unwrap();

        #[cfg(target_os = "linux")]
        {
            let root = std::env::current_dir().unwrap();
            let defaults = TerminalTool::open(&root).unwrap();
            assert_eq!(defaults.limits, TerminalLimits::default());

            let configured = TerminalTool::open_with_limits(&root, limits).unwrap();
            assert_eq!(configured.limits, limits);
        }

        #[cfg(not(target_os = "linux"))]
        {
            let absent = std::path::Path::new("/machine-god-terminal-root-must-not-be-opened");
            let Err(default_error) = TerminalTool::open(absent) else {
                panic!("system terminal unexpectedly supported");
            };
            assert_eq!(
                default_error.kind(),
                super::TerminalConfigErrorKind::UnsupportedPlatform
            );

            let Err(configured_error) = TerminalTool::open_with_limits(absent, limits) else {
                panic!("system terminal unexpectedly supported");
            };
            assert_eq!(
                configured_error.kind(),
                super::TerminalConfigErrorKind::UnsupportedPlatform
            );

            let invalid_limits = TerminalLimits {
                timeout: Duration::ZERO,
                max_active_executions: 0,
            };
            let Err(invalid_error) = TerminalTool::open_with_limits(absent, invalid_limits) else {
                panic!("system terminal unexpectedly supported");
            };
            assert_eq!(
                invalid_error.kind(),
                super::TerminalConfigErrorKind::UnsupportedPlatform
            );
        }
    }

    #[test]
    fn outcome_rejects_impossible_status_and_duration_values() {
        for status in [
            TerminalExecutionStatus::Exited(-1),
            TerminalExecutionStatus::Exited(256),
            TerminalExecutionStatus::Signaled(0),
            TerminalExecutionStatus::Signaled(256),
        ] {
            assert!(
                TerminalExecutionOutcome::new(
                    status,
                    empty_capture(),
                    empty_capture(),
                    Duration::ZERO,
                )
                .is_err()
            );
        }
        assert!(
            TerminalExecutionOutcome::new(
                TerminalExecutionStatus::Exited(0),
                empty_capture(),
                empty_capture(),
                TERMINAL_MAX_TIMEOUT + Duration::from_nanos(1),
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn independent_deadline_stops_and_drops_a_pending_executor() {
        let dropped = Arc::new(AtomicBool::new(false));
        let executor = Arc::new(PendingExecutor {
            dropped: Arc::clone(&dropped),
        });
        let root = std::env::current_dir().unwrap();
        let tool = TerminalTool::with_executor(
            &root,
            Vec::new(),
            executor,
            TerminalLimits::new(Duration::from_millis(5), 1).unwrap(),
        )
        .unwrap();
        let started = Instant::now();
        let output = futures_executor::block_on(tool.execute(
            context(),
            json!({
                "action": "exec",
                "command": "ignored",
                "cwd": ".",
                "profile": "clean"
            }),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(output.content["status"], "timed_out");
        assert!(output.is_error);
        assert!(dropped.load(Ordering::Acquire));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn executor_future_drop_cancellation_wins_over_ready_outcome() {
        let root = std::env::current_dir().unwrap();
        let tool = TerminalTool::with_executor(
            &root,
            Vec::new(),
            Arc::new(CancelOnDropExecutor),
            TerminalLimits::default(),
        )
        .unwrap();
        let error = futures_executor::block_on(tool.execute(
            context(),
            json!({
                "action": "exec",
                "command": "ignored",
                "cwd": ".",
                "profile": "clean"
            }),
            CancellationToken::new(),
        ))
        .unwrap_err();

        assert_eq!(error.kind, ToolErrorKind::Cancelled);
        assert_eq!(error.code, "terminal_cancelled");
    }

    #[cfg(unix)]
    #[test]
    fn output_limit_ready_after_deadline_remains_authoritative() {
        let root = std::env::current_dir().unwrap();
        let tool = TerminalTool::with_executor(
            &root,
            Vec::new(),
            Arc::new(DelayedOutputLimitExecutor),
            TerminalLimits::new(Duration::from_millis(1), 1).unwrap(),
        )
        .unwrap();
        let output = futures_executor::block_on(tool.execute(
            context(),
            json!({
                "action": "exec",
                "command": "ignored",
                "cwd": ".",
                "profile": "clean"
            }),
            CancellationToken::new(),
        ))
        .unwrap();

        assert_eq!(output.content["status"], "output_limit");
        assert_eq!(
            output.content["stdout_bytes"],
            MAX_TERMINAL_PRODUCED_OUTPUT_BYTES + 1
        );
    }

    #[cfg(unix)]
    #[test]
    fn output_limit_claimed_after_final_deadline_poll_closes_timeout() {
        let root = std::env::current_dir().unwrap();
        let tool = TerminalTool::with_executor(
            &root,
            Vec::new(),
            Arc::new(FinalPollOutputLimitExecutor),
            TerminalLimits::new(Duration::from_millis(1), 1).unwrap(),
        )
        .unwrap();
        let output = futures_executor::block_on(tool.execute(
            context(),
            json!({
                "action": "exec",
                "command": "ignored",
                "cwd": ".",
                "profile": "clean"
            }),
            CancellationToken::new(),
        ))
        .unwrap();

        assert_eq!(output.content["status"], "output_limit");
        assert_eq!(
            output.content["stdout_bytes"],
            MAX_TERMINAL_PRODUCED_OUTPUT_BYTES + 1
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_output_executor_error_ready_after_deadline_yields_timeout() {
        let root = std::env::current_dir().unwrap();
        let tool = TerminalTool::with_executor(
            &root,
            Vec::new(),
            Arc::new(DelayedErrorExecutor),
            TerminalLimits::new(Duration::from_millis(1), 1).unwrap(),
        )
        .unwrap();
        let output = futures_executor::block_on(tool.execute(
            context(),
            json!({
                "action": "exec",
                "command": "ignored",
                "cwd": ".",
                "profile": "clean"
            }),
            CancellationToken::new(),
        ))
        .unwrap();

        assert_eq!(output.content["status"], "timed_out");
        assert!(output.is_error);
    }

    #[cfg(unix)]
    #[test]
    fn output_claim_preserves_typed_cleanup_error_on_both_sides_of_deadline() {
        for (timeout, delay, kind, code) in [
            (
                Duration::from_secs(1),
                Duration::ZERO,
                TerminalExecutorErrorKind::Wait,
                "terminal_wait_failed",
            ),
            (
                Duration::from_millis(1),
                Duration::from_millis(5),
                TerminalExecutorErrorKind::Pipe,
                "terminal_pipe_failed",
            ),
        ] {
            let root = std::env::current_dir().unwrap();
            let tool = TerminalTool::with_executor(
                &root,
                Vec::new(),
                Arc::new(OutputClaimThenErrorExecutor { delay, kind }),
                TerminalLimits::new(timeout, 1).unwrap(),
            )
            .unwrap();
            let error = futures_executor::block_on(tool.execute(
                context(),
                json!({
                    "action": "exec",
                    "command": "ignored",
                    "cwd": ".",
                    "profile": "clean"
                }),
                CancellationToken::new(),
            ))
            .unwrap_err();

            assert_eq!(error.kind, ToolErrorKind::Execution);
            assert_eq!(error.code, code);
        }
    }

    #[cfg(unix)]
    struct DelayedErrorExecutor;

    #[cfg(unix)]
    struct OutputClaimThenErrorExecutor {
        delay: Duration,
        kind: TerminalExecutorErrorKind,
    }

    #[cfg(unix)]
    impl TerminalExecutor for OutputClaimThenErrorExecutor {
        fn execute(
            &self,
            request: TerminalExecutionRequest,
            _cancellation: CancellationToken,
        ) -> TerminalExecution {
            let delay = self.delay;
            let kind = self.kind;
            Box::pin(async move {
                assert!(request.activity.claim_output_limit());
                std::thread::sleep(delay);
                Err(TerminalExecutorError::new(kind))
            })
        }
    }

    #[cfg(unix)]
    impl TerminalExecutor for DelayedErrorExecutor {
        fn execute(
            &self,
            request: TerminalExecutionRequest,
            _cancellation: CancellationToken,
        ) -> TerminalExecution {
            Box::pin(DelayedErrorExecution { _request: request })
        }
    }

    #[cfg(unix)]
    struct DelayedErrorExecution {
        _request: TerminalExecutionRequest,
    }

    #[cfg(unix)]
    impl Future for DelayedErrorExecution {
        type Output = Result<TerminalExecutionOutcome, TerminalExecutorError>;

        fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            std::thread::sleep(Duration::from_millis(5));
            Poll::Ready(Err(TerminalExecutorError::new(
                TerminalExecutorErrorKind::Spawn,
            )))
        }
    }

    #[cfg(unix)]
    struct DelayedOutputLimitExecutor;

    #[cfg(unix)]
    struct FinalPollOutputLimitExecutor;

    #[cfg(unix)]
    impl TerminalExecutor for FinalPollOutputLimitExecutor {
        fn execute(
            &self,
            request: TerminalExecutionRequest,
            _cancellation: CancellationToken,
        ) -> TerminalExecution {
            let activity = Arc::clone(&request.activity);
            let producer_activity = Arc::clone(&activity);
            activity.install_before_timeout_close(move || {
                std::thread::spawn(move || {
                    assert!(producer_activity.claim_output_limit());
                })
                .join()
                .unwrap();
            });
            Box::pin(FinalPollOutputLimitExecution {
                _request: request,
                polls: 0,
            })
        }
    }

    #[cfg(unix)]
    struct FinalPollOutputLimitExecution {
        _request: TerminalExecutionRequest,
        polls: u8,
    }

    #[cfg(unix)]
    impl Future for FinalPollOutputLimitExecution {
        type Output = Result<TerminalExecutionOutcome, TerminalExecutorError>;

        fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
            self.polls += 1;
            match self.polls {
                1 => {
                    std::thread::sleep(Duration::from_millis(5));
                    Poll::Pending
                }
                2 => {
                    context.waker().wake_by_ref();
                    Poll::Pending
                }
                _ => Poll::Ready(TerminalExecutionOutcome::new(
                    TerminalExecutionStatus::OutputLimit,
                    TerminalCapturedOutput::new(Vec::new(), MAX_TERMINAL_PRODUCED_OUTPUT_BYTES + 1)
                        .unwrap(),
                    empty_capture(),
                    Duration::from_millis(1),
                )),
            }
        }
    }

    #[cfg(unix)]
    impl TerminalExecutor for DelayedOutputLimitExecutor {
        fn execute(
            &self,
            request: TerminalExecutionRequest,
            _cancellation: CancellationToken,
        ) -> TerminalExecution {
            Box::pin(DelayedOutputLimitExecution { _request: request })
        }
    }

    #[cfg(unix)]
    struct DelayedOutputLimitExecution {
        _request: TerminalExecutionRequest,
    }

    #[cfg(unix)]
    impl Future for DelayedOutputLimitExecution {
        type Output = Result<TerminalExecutionOutcome, super::TerminalExecutorError>;

        fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            std::thread::sleep(Duration::from_millis(5));
            Poll::Ready(TerminalExecutionOutcome::new(
                TerminalExecutionStatus::OutputLimit,
                TerminalCapturedOutput::new(Vec::new(), MAX_TERMINAL_PRODUCED_OUTPUT_BYTES + 1)
                    .unwrap(),
                empty_capture(),
                Duration::from_millis(5),
            ))
        }
    }

    #[cfg(unix)]
    struct CancelOnDropExecutor;

    #[cfg(unix)]
    impl TerminalExecutor for CancelOnDropExecutor {
        fn execute(
            &self,
            request: TerminalExecutionRequest,
            cancellation: CancellationToken,
        ) -> TerminalExecution {
            Box::pin(CancelOnDropExecution {
                _request: request,
                cancellation,
                outcome: Some(
                    TerminalExecutionOutcome::new(
                        TerminalExecutionStatus::Exited(0),
                        empty_capture(),
                        empty_capture(),
                        Duration::ZERO,
                    )
                    .unwrap(),
                ),
            })
        }
    }

    #[cfg(unix)]
    struct CancelOnDropExecution {
        _request: TerminalExecutionRequest,
        cancellation: CancellationToken,
        outcome: Option<TerminalExecutionOutcome>,
    }

    #[cfg(unix)]
    impl Future for CancelOnDropExecution {
        type Output = Result<TerminalExecutionOutcome, super::TerminalExecutorError>;

        fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(Ok(self.get_mut().outcome.take().unwrap()))
        }
    }

    #[cfg(unix)]
    impl Drop for CancelOnDropExecution {
        fn drop(&mut self) {
            self.cancellation.cancel();
        }
    }

    #[cfg(unix)]
    struct PendingExecutor {
        dropped: Arc<AtomicBool>,
    }

    #[cfg(unix)]
    impl TerminalExecutor for PendingExecutor {
        fn execute(
            &self,
            request: TerminalExecutionRequest,
            _cancellation: CancellationToken,
        ) -> TerminalExecution {
            Box::pin(PendingExecution {
                _request: request,
                dropped: Arc::clone(&self.dropped),
            })
        }
    }

    #[cfg(unix)]
    struct PendingExecution {
        _request: TerminalExecutionRequest,
        dropped: Arc<AtomicBool>,
    }

    #[cfg(unix)]
    impl Future for PendingExecution {
        type Output = Result<TerminalExecutionOutcome, super::TerminalExecutorError>;

        fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    #[cfg(unix)]
    impl Drop for PendingExecution {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Release);
        }
    }

    #[cfg(unix)]
    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("session").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
            turn_id: TurnId::new("turn").unwrap(),
            call_id: ToolCallId::new("call").unwrap(),
        }
    }
}
