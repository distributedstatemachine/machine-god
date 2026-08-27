//! Bounded, permission-gated foreground shell execution.

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::future::{Future, poll_fn};
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use std::collections::VecDeque;
#[cfg(target_os = "linux")]
use std::io::Read;
#[cfg(target_os = "linux")]
use std::os::unix::process::{CommandExt, ExitStatusExt};
#[cfg(target_os = "linux")]
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, AtomicU64};
#[cfg(target_os = "linux")]
use std::sync::{Mutex, MutexGuard};
#[cfg(target_os = "linux")]
use std::task::{Context, Waker};
#[cfg(target_os = "linux")]
use std::thread::{self, JoinHandle};

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, PreparedToolCall, ProcessEnvironment, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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
        if retained > MAX_TERMINAL_RETAINED_OUTPUT_BYTES
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
    /// Creates an inert future. Dropping it must synchronously terminate, reap,
    /// and join every owned process, pipe, and worker.
    fn execute(
        &self,
        request: TerminalExecutionRequest,
        cancellation: CancellationToken,
    ) -> TerminalExecution;
}

struct EnvironmentSnapshot {
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
        #[cfg(not(target_os = "linux"))]
        {
            let _ = root;
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
                TerminalLimits::default(),
                !cfg!(target_os = "linux"),
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
    entries.sort_by(|left, right| {
        raw_os_bytes(&left.0)
            .cmp(raw_os_bytes(&right.0))
            .then_with(|| raw_os_bytes(&left.1).cmp(raw_os_bytes(&right.1)))
    });

    let mut aggregate = 0_usize;
    let mut previous_key: Option<&[u8]> = None;
    let mut hasher = Sha256::new();
    for (key, value) in &entries {
        let key = raw_os_bytes(key);
        let value = raw_os_bytes(value);
        if key.is_empty()
            || key.len() > MAX_TERMINAL_ENVIRONMENT_KEY_BYTES
            || value.len() > MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES
            || key.contains(&b'=')
            || key.contains(&0)
            || value.contains(&0)
            || previous_key == Some(key)
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
        || cwd.starts_with('~')
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
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<TerminalExecutionRequest, ToolError> {
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
        _deadline: Instant,
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
) -> Result<TerminalExecutionOutcome, ToolError> {
    let mut cancelled = Box::pin(cancellation.cancelled());
    poll_fn(|context| {
        if cancelled.as_mut().poll(context).is_ready() || cancellation.is_cancelled() {
            return Poll::Ready(Err(cancelled_error()));
        }
        match Pin::new(&mut future).poll(context) {
            Poll::Ready(_) if cancellation.is_cancelled() => Poll::Ready(Err(cancelled_error())),
            Poll::Ready(Ok(mut outcome)) => {
                if Instant::now() >= deadline
                    && !matches!(outcome.status, TerminalExecutionStatus::TimedOut)
                {
                    outcome.status = TerminalExecutionStatus::TimedOut;
                }
                Poll::Ready(Ok(outcome))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(map_executor_error(error))),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
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

struct ActivePermit {
    active: Arc<AtomicUsize>,
}

impl ActivePermit {
    fn acquire(active: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                (value < limit).then(|| value + 1)
            })
            .ok()?;
        Some(Self {
            active: Arc::clone(active),
        })
    }
}

impl Drop for ActivePermit {
    fn drop(&mut self) {
        let previous = self.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "terminal permit underflowed");
    }
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
            check_deadline(deadline)?;
            let permit = ActivePermit::acquire(&self.active, self.limits.max_active_executions)
                .ok_or_else(busy)?;
            check_cancellation(&cancellation)?;
            let request = self.execution_request(parsed.clone(), deadline, &cancellation)?;
            check_cancellation(&cancellation)?;
            let future = self.executor.execute(request, cancellation.clone());
            let outcome = await_executor(future, &cancellation, deadline).await?;
            drop(permit);
            render_output(&parsed.cwd, &outcome)
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
            worker.abort_and_join();
        }
    }
}

#[cfg(target_os = "linux")]
struct SystemWorkerHandle {
    shared: Arc<Mutex<SystemWorkerState>>,
    thread: Option<JoinHandle<()>>,
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct SystemWorkerState {
    abort: bool,
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
        let worker_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("machine-god-terminal".to_owned())
            .spawn(move || system_worker(request, cancellation, worker_shared))
            .map_err(|_| executor_error(TerminalExecutorErrorKind::Spawn))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
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
        let suppressed = {
            let mut state = lock_worker(&self.shared);
            state.abort = true;
            state.waker.take()
        };
        drop(suppressed);
        self.join_aborted();
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
        let thread = self.thread.take().expect("terminal worker thread exists");
        if thread.thread().id() == thread::current().id() {
            // Publication happens only after the command, process group,
            // pipes, and reader threads are completely cleaned up. An inline
            // reentrant wake may consume the result on this worker; only that
            // resource-free notification tail may self-detach.
            drop(thread);
            return true;
        }
        thread.join().is_ok()
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
            {
                let mut state = lock_worker(&self.shared);
                state.abort = true;
                state.waker = None;
            }
            self.join_aborted();
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
    drop(request);
    let waker = {
        let mut state = lock_worker(&shared);
        state.outcome = Some(outcome);
        state.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
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
    let started = Instant::now();
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
        } else {
            Some(command.spawn())
        }
    };
    let Some(spawn) = spawn else {
        return Err(executor_error(TerminalExecutorErrorKind::Cancelled));
    };
    let mut child = spawn.map_err(|_| executor_error(TerminalExecutorErrorKind::Spawn))?;
    let group = rustix::process::Pid::from_raw(
        i32::try_from(child.id())
            .map_err(|_| executor_error(TerminalExecutorErrorKind::Invariant))?,
    )
    .ok_or_else(|| executor_error(TerminalExecutorErrorKind::Invariant))?;
    let stdout = child.stdout.take().ok_or_else(|| {
        terminate_and_reap(&mut child, group);
        executor_error(TerminalExecutorErrorKind::Pipe)
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        terminate_and_reap(&mut child, group);
        executor_error(TerminalExecutorErrorKind::Pipe)
    })?;

    let stop_readers = Arc::new(AtomicBool::new(false));
    let produced = Arc::new(AtomicU64::new(0));
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout_reader = spawn_reader(
        "machine-god-terminal-stdout",
        stdout,
        Arc::clone(&stop_readers),
        Arc::clone(&produced),
        Arc::clone(&overflow),
    );
    let stdout_reader = match stdout_reader {
        Ok(reader) => reader,
        Err(error) => {
            stop_readers.store(true, Ordering::Release);
            terminate_and_reap(&mut child, group);
            return Err(error);
        }
    };
    let stderr_reader = spawn_reader(
        "machine-god-terminal-stderr",
        stderr,
        Arc::clone(&stop_readers),
        Arc::clone(&produced),
        Arc::clone(&overflow),
    );
    let stderr_reader = match stderr_reader {
        Ok(reader) => reader,
        Err(error) => {
            stop_readers.store(true, Ordering::Release);
            terminate_and_reap(&mut child, group);
            let _ = stdout_reader.join();
            return Err(error);
        }
    };

    let terminal_status = loop {
        if cancellation.is_cancelled() || lock_worker(shared).abort {
            terminate_and_reap(&mut child, group);
            stop_readers.store(true, Ordering::Release);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(executor_error(TerminalExecutorErrorKind::Cancelled));
        }
        if Instant::now() >= request.deadline() {
            terminate_and_reap(&mut child, group);
            break TerminalExecutionStatus::TimedOut;
        }
        if overflow.load(Ordering::Acquire) {
            terminate_and_reap(&mut child, group);
            break TerminalExecutionStatus::OutputLimit;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                terminate_remaining_group(group);
                break exit_status(status);
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(_) => {
                terminate_and_reap(&mut child, group);
                stop_readers.store(true, Ordering::Release);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(executor_error(TerminalExecutorErrorKind::Wait));
            }
        }
    };

    stop_readers.store(true, Ordering::Release);
    let stdout = stdout_reader
        .join()
        .map_err(|_| executor_error(TerminalExecutorErrorKind::Pipe))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| executor_error(TerminalExecutorErrorKind::Pipe))??;
    TerminalExecutionOutcome::new(terminal_status, stdout, stderr, started.elapsed())
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
    overflow: Arc<AtomicBool>,
) -> Result<JoinHandle<Result<TerminalCapturedOutput, TerminalExecutorError>>, TerminalExecutorError>
{
    let flags = rustix::fs::fcntl_getfl(&reader)
        .map_err(|_| executor_error(TerminalExecutorErrorKind::Pipe))?;
    rustix::fs::fcntl_setfl(&reader, flags | OFlags::NONBLOCK)
        .map_err(|_| executor_error(TerminalExecutorErrorKind::Pipe))?;
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || read_pipe(reader, &stop, &produced, &overflow))
        .map_err(|_| executor_error(TerminalExecutorErrorKind::Pipe))
}

#[cfg(target_os = "linux")]
fn read_pipe(
    mut reader: impl Read,
    stop: &AtomicBool,
    produced: &AtomicU64,
    overflow: &AtomicBool,
) -> Result<TerminalCapturedOutput, TerminalExecutorError> {
    let mut capture = PipeCapture::default();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                let previous = produced.fetch_add(length as u64, Ordering::AcqRel);
                if previous.saturating_add(length as u64) > MAX_TERMINAL_PRODUCED_OUTPUT_BYTES {
                    overflow.store(true, Ordering::Release);
                }
                capture.push(&buffer[..length]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(_) => return Err(executor_error(TerminalExecutorErrorKind::Pipe)),
        }
    }
    capture.finish()
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct PipeCapture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
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
        for byte in &bytes[head_bytes..] {
            if self.tail.len() == PIPE_RETAINED_BYTES - head_limit {
                self.tail.pop_front();
            }
            self.tail.push_back(*byte);
        }
    }

    fn finish(self) -> Result<TerminalCapturedOutput, TerminalExecutorError> {
        let mut retained = self.head;
        retained.extend(self.tail);
        TerminalCapturedOutput::new(retained, self.total)
    }
}

#[cfg(target_os = "linux")]
fn terminate_and_reap(child: &mut Child, group: rustix::process::Pid) {
    let _ = rustix::process::kill_process_group(group, rustix::process::Signal::TERM);
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                terminate_remaining_group(group);
                return;
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(2)),
            _ => break,
        }
    }
    let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
fn terminate_remaining_group(group: rustix::process::Pid) {
    if rustix::process::test_kill_process_group(group).is_err() {
        return;
    }
    let _ = rustix::process::kill_process_group(group, rustix::process::Signal::TERM);
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        if rustix::process::test_kill_process_group(group).is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(2));
    }
    let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
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

fn check_deadline(deadline: Instant) -> Result<(), ToolError> {
    if Instant::now() >= deadline {
        Err(fixed_tool_error(
            ToolErrorKind::Execution,
            "terminal_timed_out",
            "terminal execution timed out",
            true,
        ))
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
