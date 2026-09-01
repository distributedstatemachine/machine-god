//! Prepared, process-group-owned background shell execution.

#![allow(
    dead_code,
    reason = "lower-level process lifecycle primitives remain directly integration-tested"
)]

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::future::Future;
use std::num::NonZeroU32;
#[cfg(unix)]
use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::PathBuf;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::pin::Pin;
use std::time::Duration;

use machine_god_core::CancellationToken;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_core::Cancelled;

#[cfg(target_os = "linux")]
use rustix::fd::AsRawFd;
#[cfg(unix)]
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(unix)]
use rustix::fs::{FileType, Mode, OFlags};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::ffi::OsStringExt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::process::{CommandExt, ExitStatusExt};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::ChildStderr;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::Arc;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::task::{Context, Poll, Wake, Waker};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::thread;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::Instant;

/// Fixed production shell.
pub const BACKGROUND_PROCESS_PROGRAM: &str = "/bin/sh";
/// Maximum UTF-8 command bytes retained in one request.
pub const MAX_BACKGROUND_PROCESS_COMMAND_BYTES: usize = 32 * 1024;
/// Maximum display working-directory bytes retained in one request.
pub const MAX_BACKGROUND_PROCESS_CWD_BYTES: usize = 4 * 1024;
/// Maximum injected environment entries.
pub const MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES: usize = 512;
/// Maximum bytes in one injected environment key.
pub const MAX_BACKGROUND_PROCESS_ENVIRONMENT_KEY_BYTES: usize = 1024;
/// Maximum bytes in one injected environment value.
pub const MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES: usize = 16 * 1024;
/// Maximum aggregate key and value bytes in the injected environment.
pub const MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES: usize = 256 * 1024;
/// Grace between TERM and KILL during explicit or implicit stop.
pub const BACKGROUND_PROCESS_TERM_GRACE: Duration = Duration::from_millis(250);

#[cfg(any(target_os = "linux", target_os = "macos"))]
const GROUP_DISAPPEARANCE_GRACE: Duration = Duration::from_secs(5);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const OBSERVATION_INITIAL_INTERVAL: Duration = Duration::from_millis(2);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const OBSERVATION_MAX_INTERVAL: Duration = Duration::from_millis(32);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const GROUP_SNAPSHOT_INITIAL_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const GROUP_SNAPSHOT_MAX_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const GROUP_SNAPSHOT_TIMEOUT: Duration = Duration::from_millis(250);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_GROUP_SNAPSHOT_BYTES: usize = 64 * 1024;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static OBSERVED_LEADER: AtomicU32 = AtomicU32::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static LEADER_OBSERVATIONS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static WAITID_FAILURE_LEADER: AtomicU32 = AtomicU32::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static WAITID_FAILURES: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static GROUP_SNAPSHOT_FAILURE_GROUP: AtomicU32 = AtomicU32::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static GROUP_SNAPSHOT_FAILURES: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static GROUP_SNAPSHOT_SPAWN_FAILURE_GROUP: AtomicU32 = AtomicU32::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static GROUP_SNAPSHOT_SPAWN_FAILURES: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static OBSERVED_GROUP_SIGNAL: AtomicU32 = AtomicU32::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static GROUP_SIGNAL_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_HELPER_PATH_BYTES: usize = 4 * 1024;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_HELPER_ARGUMENTS: usize = 16;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_HELPER_ARGUMENT_BYTES: usize = 1024;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const HELPER_READY_BYTE: u8 = 0xa7;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const HELPER_READY_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RELEASE_FRAME_MAGIC: &[u8; 8] = b"MGBG\0\0\0\x01";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SAFE_BOOTSTRAP_LANGUAGE: &str = "C";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SAFE_BOOTSTRAP_MODE_KEY: &str = "MACHINE_GOD_BACKGROUND_HELPER_MODE";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SAFE_BOOTSTRAP_MODE_VALUE: &str = "1";

/// Stable background-process failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackgroundProcessErrorKind {
    /// This operating system has no active adapter.
    Unsupported,
    /// The exact bounded request is invalid.
    InvalidRequest,
    /// The gated process could not be created or validated.
    Spawn,
    /// The private start gate could not be released.
    Release,
    /// Process observation or reaping failed.
    Wait,
    /// Process-group cleanup could not be proven complete.
    Cleanup,
    /// An internal ownership invariant was violated.
    Invariant,
}

/// Fixed, data-free native process failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct BackgroundProcessError {
    kind: BackgroundProcessErrorKind,
}

impl BackgroundProcessError {
    /// Returns the stable error category.
    #[must_use]
    pub const fn kind(self) -> BackgroundProcessErrorKind {
        self.kind
    }

    const fn new(kind: BackgroundProcessErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for BackgroundProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundProcessError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for BackgroundProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("background process operation failed")
    }
}

impl Error for BackgroundProcessError {}

/// Exact, bounded request for one prepared background command.
pub struct BackgroundProcessRequest {
    command: String,
    cwd: String,
    environment: Vec<(OsString, OsString)>,
    #[cfg(unix)]
    directory: OwnedFd,
    #[cfg(target_os = "linux")]
    descriptor_path: PathBuf,
}

impl BackgroundProcessRequest {
    /// Retains an already-open directory descriptor with an exact command and
    /// environment snapshot.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-request error when any bound or descriptor
    /// identity check fails.
    #[cfg(unix)]
    pub fn from_directory(
        command: String,
        cwd: String,
        environment: Vec<(OsString, OsString)>,
        directory: OwnedFd,
    ) -> Result<Self, BackgroundProcessError> {
        validate_request(&command, &cwd, &environment)?;
        validate_directory(directory.as_fd())?;
        #[cfg(target_os = "linux")]
        let descriptor_path = validated_descriptor_path(directory.as_fd())?;
        Ok(Self {
            command,
            cwd,
            environment,
            directory,
            #[cfg(target_os = "linux")]
            descriptor_path,
        })
    }

    /// Opens and retains an absolute working directory for a later spawn.
    ///
    /// The descriptor, rather than this path, controls the eventual child
    /// working directory. Renaming or replacing the path after this method
    /// returns cannot redirect execution.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-request error for an invalid path, request, or
    /// retained-directory identity.
    #[cfg(unix)]
    pub fn open(
        command: String,
        cwd: String,
        environment: Vec<(OsString, OsString)>,
        directory: &Path,
    ) -> Result<Self, BackgroundProcessError> {
        if !directory.is_absolute() {
            return Err(invalid_request());
        }
        let descriptor = rustix::fs::open(
            directory,
            OFlags::RDONLY
                | OFlags::DIRECTORY
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| invalid_request())?;
        Self::from_directory(command, cwd, environment, descriptor)
    }

    /// Returns the fixed executable.
    #[must_use]
    pub const fn program(&self) -> &'static str {
        debug_assert!(!self.command.is_empty());
        BACKGROUND_PROCESS_PROGRAM
    }

    /// Returns the exact user command retained as a distinct argv element.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Returns the display working directory.
    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Returns the exact injected environment.
    #[must_use]
    pub fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    /// Returns the retained directory descriptor.
    #[cfg(unix)]
    #[must_use]
    pub fn directory_fd(&self) -> BorrowedFd<'_> {
        self.directory.as_fd()
    }

    /// Returns the validated descriptor-backed current-directory path.
    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn descriptor_path(&self) -> &Path {
        &self.descriptor_path
    }
}

impl fmt::Debug for BackgroundProcessRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackgroundProcessRequest { .. }")
    }
}

/// Exit status reported only after the original process group is gone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackgroundProcessExit {
    /// The shell leader exited with this code.
    Exited(i32),
    /// The shell leader was terminated by this signal.
    Signaled(i32),
}

/// Completion of a released process wait with cooperative stop ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackgroundProcessOutcome {
    /// The process leader and its original group completed normally.
    Completed(BackgroundProcessExit),
    /// The stop token requested TERM/KILL cleanup before completion was
    /// observed.
    Stopped,
}

/// Explicit executable and private arguments used by the safe launch helper.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Debug)]
pub struct BackgroundProcessHelper {
    program: PathBuf,
    arguments: Vec<OsString>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl BackgroundProcessHelper {
    /// Validates an absolute helper executable and its fixed private arguments.
    ///
    /// # Errors
    ///
    /// Returns a fixed invalid-request error when the helper specification is
    /// not within the fixed bounds.
    pub fn new(program: PathBuf, arguments: Vec<OsString>) -> Result<Self, BackgroundProcessError> {
        if !program.is_absolute()
            || program.as_os_str().as_bytes().is_empty()
            || program.as_os_str().as_bytes().len() > MAX_HELPER_PATH_BYTES
            || program.as_os_str().as_bytes().contains(&0)
            || arguments.len() > MAX_HELPER_ARGUMENTS
            || arguments.iter().any(|argument| {
                argument.as_os_str().as_bytes().len() > MAX_HELPER_ARGUMENT_BYTES
                    || argument.as_os_str().as_bytes().contains(&0)
            })
        {
            return Err(invalid_request());
        }
        Ok(Self { program, arguments })
    }
}

/// Native process adapter. Linux launches directly. macOS uses an explicitly
/// supplied instance of this executable as a tiny inherited-directory helper.
/// Other targets return a fixed unsupported error without spawning.
#[derive(Clone, Debug, Default)]
pub struct SystemBackgroundProcessAdapter {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    helper: Option<BackgroundProcessHelper>,
}

impl SystemBackgroundProcessAdapter {
    /// Constructs an active adapter with an explicitly supplied safe bootstrap.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[must_use]
    pub const fn with_helper(helper: BackgroundProcessHelper) -> Self {
        Self {
            helper: Some(helper),
        }
    }

    /// Spawns a private-gated process-group leader. The user command cannot run
    /// until the returned handle is released.
    ///
    /// # Errors
    ///
    /// Returns a fixed spawn, invariant, or unsupported failure.
    pub fn prepare(
        &self,
        request: BackgroundProcessRequest,
    ) -> Result<PreparedBackgroundProcess, BackgroundProcessError> {
        prepare_system(self, request)
    }
}

/// Runs the inherited-directory helper protocol and replaces the helper with
/// the fixed shell. Hosts call this only for their private helper mode, before
/// ordinary CLI parsing or worker creation.
///
/// On macOS the parent maps a clone of the retained directory descriptor to
/// stdout. Linux starts the helper in the retained directory. On both systems,
/// the requested command and environment remain inert bytes in a bounded stdin
/// frame until release. An EOF before a complete frame aborts without executing
/// a user command.
///
/// # Errors
///
/// Returns a fixed release, invalid-request, or spawn failure. A successful
/// call does not return because the helper is replaced with `/bin/sh`.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn run_background_process_helper() -> Result<(), BackgroundProcessError> {
    #[cfg(target_os = "macos")]
    use std::io::stdout;
    use std::io::{Read, Write, stderr, stdin};

    #[cfg(target_os = "macos")]
    let stdout = stdout();
    #[cfg(target_os = "macos")]
    validate_directory(stdout.as_fd()).map_err(|_| spawn_error())?;
    #[cfg(target_os = "macos")]
    rustix::process::fchdir(stdout.as_fd()).map_err(|_| spawn_error())?;
    let mut ready = stderr().lock();
    ready
        .write_all(&[HELPER_READY_BYTE])
        .and_then(|()| ready.flush())
        .map_err(|_| spawn_error())?;
    drop(ready);

    let mut input = stdin().lock();
    let (command, environment) = read_release_frame(&mut input)?;
    let mut trailing = [0_u8; 1];
    if input.read(&mut trailing).map_err(|_| invalid_request())? != 0 {
        return Err(invalid_request());
    }
    #[cfg(target_os = "macos")]
    drop((input, stdout));
    #[cfg(target_os = "linux")]
    drop(input);
    let error = Command::new(BACKGROUND_PROCESS_PROGRAM)
        .arg("-c")
        .arg(command)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .exec();
    drop(error);
    Err(spawn_error())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_release_frame(
    input: &mut impl std::io::Read,
) -> Result<(String, Vec<(OsString, OsString)>), BackgroundProcessError> {
    let mut magic = [0_u8; RELEASE_FRAME_MAGIC.len()];
    read_release_bytes(input, &mut magic)?;
    if &magic != RELEASE_FRAME_MAGIC {
        return Err(invalid_request());
    }
    let command_length = read_release_length(input, MAX_BACKGROUND_PROCESS_COMMAND_BYTES)?;
    if command_length == 0 {
        return Err(invalid_request());
    }
    let mut command = vec![0_u8; command_length];
    read_release_bytes(input, &mut command)?;
    let command = String::from_utf8(command).map_err(|_| invalid_request())?;

    let environment_count = read_release_length(input, MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES)?;
    let mut environment = Vec::with_capacity(environment_count);
    let mut aggregate = 0_usize;
    for _ in 0..environment_count {
        let key_length = read_release_length(input, MAX_BACKGROUND_PROCESS_ENVIRONMENT_KEY_BYTES)?;
        if key_length == 0 {
            return Err(invalid_request());
        }
        let value_length =
            read_release_length(input, MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES)?;
        aggregate = aggregate
            .checked_add(key_length)
            .and_then(|total| total.checked_add(value_length))
            .filter(|total| *total <= MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES)
            .ok_or_else(invalid_request)?;
        let mut key = vec![0_u8; key_length];
        let mut value = vec![0_u8; value_length];
        read_release_bytes(input, &mut key)?;
        read_release_bytes(input, &mut value)?;
        environment.push((OsString::from_vec(key), OsString::from_vec(value)));
    }
    validate_request(&command, "helper", &environment)?;
    Ok((command, environment))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_release_length(
    input: &mut impl std::io::Read,
    maximum: usize,
) -> Result<usize, BackgroundProcessError> {
    let mut bytes = [0_u8; 4];
    read_release_bytes(input, &mut bytes)?;
    let length = usize::try_from(u32::from_be_bytes(bytes)).map_err(|_| invalid_request())?;
    if length > maximum {
        return Err(invalid_request());
    }
    Ok(length)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_release_bytes(
    input: &mut impl std::io::Read,
    bytes: &mut [u8],
) -> Result<(), BackgroundProcessError> {
    input
        .read_exact(bytes)
        .map_err(|_| BackgroundProcessError::new(BackgroundProcessErrorKind::Release))
}

/// Returns the fixed unsupported result on platforms without the helper
/// protocol, allowing a host to keep one private-mode dispatch path.
///
/// # Errors
///
/// Always returns the fixed unsupported category outside Linux and macOS.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn run_background_process_helper() -> Result<(), BackgroundProcessError> {
    Err(BackgroundProcessError::new(
        BackgroundProcessErrorKind::Unsupported,
    ))
}

/// A spawned process blocked on its private start gate.
pub struct PreparedBackgroundProcess {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    child: Option<Child>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    gate: Option<ChildStdin>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    command: Option<String>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    environment: Option<Vec<(OsString, OsString)>>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    group: rustix::process::Pid,
    pid: NonZeroU32,
}

impl PreparedBackgroundProcess {
    /// Returns the validated, nonzero direct-child PID.
    #[must_use]
    pub const fn pid(&self) -> NonZeroU32 {
        self.pid
    }

    /// Releases the private start gate and transfers process ownership.
    /// Closing the gate immediately after the release byte gives the user
    /// command EOF on standard input.
    ///
    /// # Errors
    ///
    /// Returns a fixed release or cleanup failure and reaps the child when the
    /// gate cannot be released.
    pub fn release(mut self) -> Result<OwnedBackgroundProcess, BackgroundProcessError> {
        release_prepared(&mut self)
    }

    /// Aborts the prepared process without permitting the user command to run,
    /// then signals the group and reaps the direct child.
    ///
    /// # Errors
    ///
    /// Returns a fixed cleanup failure unless group disappearance is proven.
    pub fn abort_and_reap(mut self) -> Result<(), BackgroundProcessError> {
        abort_prepared(&mut self)
    }
}

impl fmt::Debug for PreparedBackgroundProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedBackgroundProcess")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl Drop for PreparedBackgroundProcess {
    fn drop(&mut self) {
        let _ = abort_prepared(self);
    }
}

/// Exclusive ownership of one released background process group.
pub struct OwnedBackgroundProcess {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    child: Option<Child>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    group: rustix::process::Pid,
    pid: NonZeroU32,
}

impl OwnedBackgroundProcess {
    /// Returns the validated, nonzero direct-child PID.
    #[must_use]
    pub const fn pid(&self) -> NonZeroU32 {
        self.pid
    }

    /// Waits for the direct child, cleans any lingering original-group
    /// descendants, reaps the leader, and requires an ESRCH group probe before
    /// reporting its status.
    ///
    /// # Errors
    ///
    /// Returns a fixed wait, cleanup, or invariant failure.
    pub fn wait(mut self) -> Result<BackgroundProcessExit, BackgroundProcessError> {
        wait_owned(&mut self)
    }

    /// Waits while observing a cooperative stop token. Cancellation performs
    /// the same bounded TERM/KILL/reap/ESRCH protocol as [`Self::stop`] and
    /// returns a distinct stopped outcome.
    ///
    /// This is the retainer-facing operation: a retainer can cancel the token
    /// during shutdown even while its worker owns this blocking wait.
    ///
    /// # Errors
    ///
    /// Returns a fixed wait, cleanup, or invariant failure.
    pub fn wait_with_stop(
        mut self,
        stop: &CancellationToken,
    ) -> Result<BackgroundProcessOutcome, BackgroundProcessError> {
        wait_owned_with_stop(&mut self, stop)
    }

    /// Sends TERM to the original group, waits the fixed grace, sends KILL,
    /// reaps the retained direct child, and requires an ESRCH group probe.
    ///
    /// # Errors
    ///
    /// Returns a fixed cleanup failure unless ownership is fully discharged.
    pub fn stop(mut self) -> Result<(), BackgroundProcessError> {
        stop_owned(&mut self)
    }
}

impl fmt::Debug for OwnedBackgroundProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedBackgroundProcess")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl Drop for OwnedBackgroundProcess {
    fn drop(&mut self) {
        let _ = stop_owned(self);
    }
}

#[cfg(unix)]
fn validate_request(
    command: &str,
    cwd: &str,
    environment: &[(OsString, OsString)],
) -> Result<(), BackgroundProcessError> {
    if command.is_empty()
        || command.len() > MAX_BACKGROUND_PROCESS_COMMAND_BYTES
        || command.contains('\0')
        || cwd.is_empty()
        || cwd.len() > MAX_BACKGROUND_PROCESS_CWD_BYTES
        || cwd.contains('\0')
        || environment.len() > MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES
    {
        return Err(invalid_request());
    }
    let mut aggregate = 0_usize;
    for (index, (key, value)) in environment.iter().enumerate() {
        let key = key.as_os_str().as_bytes();
        let value = value.as_os_str().as_bytes();
        if key.is_empty()
            || key.len() > MAX_BACKGROUND_PROCESS_ENVIRONMENT_KEY_BYTES
            || value.len() > MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES
            || key.contains(&b'=')
            || key.contains(&0)
            || value.contains(&0)
            || environment[..index]
                .iter()
                .any(|(previous, _)| previous.as_os_str().as_bytes() == key)
        {
            return Err(invalid_request());
        }
        aggregate = aggregate
            .checked_add(key.len())
            .and_then(|total| total.checked_add(value.len()))
            .ok_or_else(invalid_request)?;
        if aggregate > MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES {
            return Err(invalid_request());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn validate_directory(directory: BorrowedFd<'_>) -> Result<(), BackgroundProcessError> {
    let metadata = rustix::fs::fstat(directory).map_err(|_| invalid_request())?;
    if metadata.st_nlink == 0 || !FileType::from_raw_mode(metadata.st_mode).is_dir() {
        return Err(invalid_request());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validated_descriptor_path(directory: BorrowedFd<'_>) -> Result<PathBuf, BackgroundProcessError> {
    let descriptor_metadata = rustix::fs::fstat(directory).map_err(|_| invalid_request())?;
    let path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        directory.as_raw_fd()
    ));
    let path_metadata = rustix::fs::stat(&path).map_err(|_| invalid_request())?;
    if descriptor_metadata.st_dev != path_metadata.st_dev
        || descriptor_metadata.st_ino != path_metadata.st_ino
        || !FileType::from_raw_mode(path_metadata.st_mode).is_dir()
    {
        return Err(invalid_request());
    }
    Ok(path)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn require_exclusive_child_reaping() -> Result<(), BackgroundProcessError> {
    // `SIGCHLD = SIG_IGN`, `SA_NOCLDWAIT`, and a competing process-wide reaper
    // all make a direct child non-waitable. Exercise the exact wait authority
    // immediately before admission without changing process-wide signal state.
    // The caller must keep that authority exclusive for the returned handle's
    // lifetime; a later ECHILD is handled as irrevocable authority loss.
    let mut probe = Command::new(BACKGROUND_PROCESS_PROGRAM);
    probe
        .arg("-c")
        .arg("exit 0")
        .env_clear()
        .env("LANG", SAFE_BOOTSTRAP_LANGUAGE)
        .env("LC_ALL", SAFE_BOOTSTRAP_LANGUAGE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = probe.spawn().map_err(|_| spawn_error())?;
    match child.wait() {
        Ok(status) if status.success() => Ok(()),
        Ok(_) | Err(_) => Err(spawn_error()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn prepare_system(
    adapter: &SystemBackgroundProcessAdapter,
    request: BackgroundProcessRequest,
) -> Result<PreparedBackgroundProcess, BackgroundProcessError> {
    // Recheck immediately before the only effect so a closed or substituted
    // descriptor-backed path fails before spawning.
    validate_directory(request.directory.as_fd()).map_err(|_| spawn_error())?;
    let helper = adapter
        .helper
        .as_ref()
        .ok_or_else(|| BackgroundProcessError::new(BackgroundProcessErrorKind::Unsupported))?;
    require_exclusive_child_reaping()?;
    #[cfg(target_os = "linux")]
    let retained_cwd = {
        let descriptor_path =
            validated_descriptor_path(request.directory.as_fd()).map_err(|_| spawn_error())?;
        if descriptor_path != request.descriptor_path {
            return Err(spawn_error());
        }
        descriptor_path
    };
    #[cfg(target_os = "macos")]
    let directory = rustix::io::dup(request.directory.as_fd()).map_err(|_| spawn_error())?;

    let mut command = Command::new(&helper.program);
    command
        .args(&helper.arguments)
        .env_clear()
        .env("LANG", SAFE_BOOTSTRAP_LANGUAGE)
        .env("LC_ALL", SAFE_BOOTSTRAP_LANGUAGE)
        .env(SAFE_BOOTSTRAP_MODE_KEY, SAFE_BOOTSTRAP_MODE_VALUE)
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    #[cfg(target_os = "linux")]
    command.current_dir(retained_cwd).stdout(Stdio::null());
    #[cfg(target_os = "macos")]
    command.stdout(Stdio::from(directory));
    command.stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|_| spawn_error())?;
    let pid = NonZeroU32::new(child.id()).ok_or_else(invariant_error)?;
    let group =
        rustix::process::Pid::from_raw(i32::try_from(pid.get()).map_err(|_| invariant_error())?)
            .ok_or_else(invariant_error)?;
    if rustix::process::getpgid(Some(group)) != Ok(group) {
        let _ = cleanup_child(&mut child, group, Duration::ZERO);
        return Err(invariant_error());
    }
    let Some(gate) = child.stdin.take() else {
        let _ = cleanup_child(&mut child, group, Duration::ZERO);
        return Err(spawn_error());
    };
    let Some(ready) = child.stderr.take() else {
        let _ = cleanup_child(&mut child, group, Duration::ZERO);
        return Err(spawn_error());
    };
    if await_helper_ready(ready).is_err() {
        let _ = cleanup_child(&mut child, group, Duration::ZERO);
        return Err(spawn_error());
    }
    // `spawn` has completed the descriptor-backed chdir in the child. Move
    // only the inert release frame out of the retained request.
    let BackgroundProcessRequest {
        command,
        environment,
        ..
    } = request;
    Ok(PreparedBackgroundProcess {
        child: Some(child),
        gate: Some(gate),
        command: Some(command),
        environment: Some(environment),
        group,
        pid,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn await_helper_ready(mut ready: ChildStderr) -> Result<(), BackgroundProcessError> {
    use std::io::Read;

    let flags = rustix::fs::fcntl_getfl(&ready).map_err(|_| spawn_error())?;
    rustix::fs::fcntl_setfl(&ready, flags | OFlags::NONBLOCK).map_err(|_| spawn_error())?;
    let deadline = Instant::now() + HELPER_READY_TIMEOUT;
    let mut byte = [0_u8; 1];
    loop {
        match ready.read(&mut byte) {
            Ok(1) if byte[0] == HELPER_READY_BYTE => return Ok(()),
            Ok(1 | 0) => return Err(spawn_error()),
            Ok(_) => unreachable!("one-byte ready buffer has bounded reads"),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(OBSERVATION_INITIAL_INTERVAL);
            }
            Err(_) => return Err(spawn_error()),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn prepare_system(
    _adapter: &SystemBackgroundProcessAdapter,
    request: BackgroundProcessRequest,
) -> Result<PreparedBackgroundProcess, BackgroundProcessError> {
    drop(request);
    Err(BackgroundProcessError::new(
        BackgroundProcessErrorKind::Unsupported,
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn release_prepared(
    prepared: &mut PreparedBackgroundProcess,
) -> Result<OwnedBackgroundProcess, BackgroundProcessError> {
    let Some(mut gate) = prepared.gate.take() else {
        return Err(invariant_error());
    };
    let release = prepared
        .command
        .take()
        .ok_or_else(invariant_error)
        .and_then(|command| {
            let environment = prepared.environment.take().ok_or_else(invariant_error)?;
            write_release_frame(&mut gate, &command, &environment)
                .map_err(|_| BackgroundProcessError::new(BackgroundProcessErrorKind::Release))
        });
    if release.is_err() {
        drop(gate);
        let cleanup = abort_prepared(prepared);
        return cleanup.and(Err(BackgroundProcessError::new(
            BackgroundProcessErrorKind::Release,
        )));
    }
    drop(gate);
    let child = prepared.child.take().ok_or_else(invariant_error)?;
    Ok(OwnedBackgroundProcess {
        child: Some(child),
        group: prepared.group,
        pid: prepared.pid,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_release_frame(
    output: &mut impl std::io::Write,
    command: &str,
    environment: &[(OsString, OsString)],
) -> std::io::Result<()> {
    output.write_all(RELEASE_FRAME_MAGIC)?;
    write_frame_length(output, command.len())?;
    output.write_all(command.as_bytes())?;
    write_frame_length(output, environment.len())?;
    for (key, value) in environment {
        let key = key.as_os_str().as_bytes();
        let value = value.as_os_str().as_bytes();
        write_frame_length(output, key.len())?;
        write_frame_length(output, value.len())?;
        output.write_all(key)?;
        output.write_all(value)?;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_frame_length(output: &mut impl std::io::Write, length: usize) -> std::io::Result<()> {
    let length = u32::try_from(length).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    output.write_all(&length.to_be_bytes())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn release_prepared(
    _prepared: &mut PreparedBackgroundProcess,
) -> Result<OwnedBackgroundProcess, BackgroundProcessError> {
    Err(BackgroundProcessError::new(
        BackgroundProcessErrorKind::Unsupported,
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn abort_prepared(prepared: &mut PreparedBackgroundProcess) -> Result<(), BackgroundProcessError> {
    drop(prepared.gate.take());
    let Some(mut child) = prepared.child.take() else {
        return Ok(());
    };
    cleanup_child(&mut child, prepared.group, Duration::ZERO)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "keeps one platform-neutral prepared-process cleanup shape"
)]
fn abort_prepared(_prepared: &mut PreparedBackgroundProcess) -> Result<(), BackgroundProcessError> {
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn wait_owned(
    owned: &mut OwnedBackgroundProcess,
) -> Result<BackgroundProcessExit, BackgroundProcessError> {
    if owned.child.is_none() {
        return Err(invariant_error());
    }
    let mut observation = ObservationBackoff::new();
    let observed = loop {
        match observe_leader(owned.group) {
            Err(LeaderObservationFailure::LostAuthority) => {
                drop(owned.child.take());
                return Err(wait_error());
            }
            Err(LeaderObservationFailure::Operation(error)) => {
                let cleanup = cleanup_owned_child(owned, Duration::ZERO, None, true);
                return Err(combine_cleanup_failures(error, cleanup));
            }
            Ok(Some(status)) => break status,
            Ok(None) => observation.sleep_and_advance(),
        }
    };
    finish_observed(owned, observed)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn wait_owned(
    _owned: &mut OwnedBackgroundProcess,
) -> Result<BackgroundProcessExit, BackgroundProcessError> {
    Err(BackgroundProcessError::new(
        BackgroundProcessErrorKind::Unsupported,
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn wait_owned_with_stop(
    owned: &mut OwnedBackgroundProcess,
    stop: &CancellationToken,
) -> Result<BackgroundProcessOutcome, BackgroundProcessError> {
    if owned.child.is_none() {
        return Err(invariant_error());
    }
    let mut observation = ObservationBackoff::new();
    let mut cancellation = CancellationParker::new(stop);
    loop {
        if cancellation.is_cancelled() {
            stop_owned(owned)?;
            return Ok(BackgroundProcessOutcome::Stopped);
        }
        match observe_leader(owned.group) {
            Ok(Some(status)) => {
                return finish_observed(owned, status).map(BackgroundProcessOutcome::Completed);
            }
            Ok(None) => {}
            Err(LeaderObservationFailure::LostAuthority) => {
                drop(owned.child.take());
                return Err(wait_error());
            }
            Err(LeaderObservationFailure::Operation(error)) => {
                let cleanup = cleanup_owned_child(owned, Duration::ZERO, None, true);
                return Err(combine_cleanup_failures(error, cleanup));
            }
        }
        observation.park_and_advance(&mut cancellation);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn wait_owned_with_stop(
    _owned: &mut OwnedBackgroundProcess,
    _stop: &CancellationToken,
) -> Result<BackgroundProcessOutcome, BackgroundProcessError> {
    Err(BackgroundProcessError::new(
        BackgroundProcessErrorKind::Unsupported,
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn finish_observed(
    owned: &mut OwnedBackgroundProcess,
    observed: BackgroundProcessExit,
) -> Result<BackgroundProcessExit, BackgroundProcessError> {
    cleanup_owned_child(owned, Duration::ZERO, Some(observed), false)?;
    Ok(observed)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn stop_owned(owned: &mut OwnedBackgroundProcess) -> Result<(), BackgroundProcessError> {
    if owned.child.is_none() {
        return Ok(());
    }
    cleanup_owned_child(owned, BACKGROUND_PROCESS_TERM_GRACE, None, false)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "keeps one platform-neutral owned-process cleanup shape"
)]
fn stop_owned(_owned: &mut OwnedBackgroundProcess) -> Result<(), BackgroundProcessError> {
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn observe_leader(
    leader: rustix::process::Pid,
) -> Result<Option<BackgroundProcessExit>, LeaderObservationFailure> {
    #[cfg(test)]
    if OBSERVED_LEADER.load(Ordering::Relaxed) == leader.as_raw_nonzero().get().cast_unsigned() {
        LEADER_OBSERVATIONS.fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(test)]
    if consume_injected_failure(
        &WAITID_FAILURE_LEADER,
        &WAITID_FAILURES,
        leader.as_raw_nonzero().get().cast_unsigned(),
    ) {
        return Err(LeaderObservationFailure::Operation(wait_error()));
    }
    let status = rustix::process::waitid(
        rustix::process::WaitId::Pid(leader),
        rustix::process::WaitIdOptions::EXITED
            | rustix::process::WaitIdOptions::NOHANG
            | rustix::process::WaitIdOptions::NOWAIT,
    )
    .map_err(|error| {
        if error == rustix::io::Errno::CHILD {
            LeaderObservationFailure::LostAuthority
        } else {
            LeaderObservationFailure::Operation(wait_error())
        }
    })?;
    status
        .map(waitid_status)
        .transpose()
        .map_err(LeaderObservationFailure::Operation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum LeaderObservationFailure {
    LostAuthority,
    Operation(BackgroundProcessError),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn waitid_status(
    status: rustix::process::WaitIdStatus,
) -> Result<BackgroundProcessExit, BackgroundProcessError> {
    if let Some(code) = status.exit_status() {
        Ok(BackgroundProcessExit::Exited(code))
    } else if let Some(signal) = status.terminating_signal() {
        Ok(BackgroundProcessExit::Signaled(signal))
    } else {
        Err(invariant_error())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn exit_status(status: ExitStatus) -> BackgroundProcessExit {
    status.code().map_or_else(
        || BackgroundProcessExit::Signaled(status.signal().unwrap_or(0)),
        BackgroundProcessExit::Exited,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_child(
    child: &mut Child,
    group: rustix::process::Pid,
    term_grace: Duration,
) -> Result<(), BackgroundProcessError> {
    cleanup_child_with_expected(child, group, term_grace, None, false)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_owned_child(
    owned: &mut OwnedBackgroundProcess,
    term_grace: Duration,
    expected: Option<BackgroundProcessExit>,
    force_cleanup: bool,
) -> Result<(), BackgroundProcessError> {
    let Some(mut child) = owned.child.take() else {
        return Err(invariant_error());
    };
    cleanup_child_with_expected(&mut child, owned.group, term_grace, expected, force_cleanup)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_child_with_expected(
    child: &mut Child,
    group: rustix::process::Pid,
    term_grace: Duration,
    expected: Option<BackgroundProcessExit>,
    force_cleanup: bool,
) -> Result<(), BackgroundProcessError> {
    let mut failures = CleanupFailures::default();
    let mut force_signals = force_cleanup;
    let mut group_phase = cleanup_group_signal_phase(
        group,
        rustix::process::Signal::TERM,
        &mut force_signals,
        &mut failures,
    );
    if group_phase == CleanupSignalPhase::LostAuthority {
        return failures.finish();
    }
    if group_phase != CleanupSignalPhase::Quiescent {
        if !term_grace.is_zero() {
            sleep_through(term_grace);
        }
        group_phase = cleanup_group_signal_phase(
            group,
            rustix::process::Signal::KILL,
            &mut force_signals,
            &mut failures,
        );
        if group_phase == CleanupSignalPhase::LostAuthority {
            return failures.finish();
        }
        // A successful group KILL already targets the retained leader. Avoid a
        // redundant numeric child signal, and retain wait authority while the
        // bounded group-disappearance proof runs.
        if group_phase != CleanupSignalPhase::Quiescent
            && let Err(error) = require_original_group_quiescent(group)
        {
            failures.record(error);
        }
    }
    match child.wait() {
        Ok(status) => {
            if expected.is_some_and(|expected| exit_status(status) != expected) {
                failures.record(invariant_error());
            }
        }
        Err(_) => failures.record(wait_error()),
    }
    failures.finish()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_group_signal_phase(
    group: rustix::process::Pid,
    signal: rustix::process::Signal,
    force_signals: &mut bool,
    failures: &mut CleanupFailures,
) -> CleanupSignalPhase {
    let leader_exited = match observe_leader(group) {
        Ok(status) => status.is_some(),
        Err(LeaderObservationFailure::LostAuthority) => {
            failures.record(wait_error());
            return CleanupSignalPhase::LostAuthority;
        }
        Err(LeaderObservationFailure::Operation(error)) => {
            *force_signals = true;
            failures.record(error);
            false
        }
    };
    let only_exited_leader = match group_members(group) {
        Ok(members) => leader_exited && only_group_leader_remains(&members, group),
        Err(error) => {
            *force_signals = true;
            failures.record(error);
            false
        }
    };
    // Keep the leader unreaped throughout the signal phases. Any observation
    // failure forces both group signals instead of becoming a cleanup short
    // circuit; a clean sole-zombie proof avoids unnecessary signalling.
    if (*force_signals || !only_exited_leader)
        && let Err(error) = signal_group_or_confirm_exited_leader(group, signal)
    {
        match error {
            LeaderObservationFailure::LostAuthority => {
                failures.record(wait_error());
                return CleanupSignalPhase::LostAuthority;
            }
            LeaderObservationFailure::Operation(error) => {
                *force_signals = true;
                failures.record(error);
            }
        }
    }
    if only_exited_leader && !*force_signals {
        CleanupSignalPhase::Quiescent
    } else {
        CleanupSignalPhase::Active
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Eq, PartialEq)]
enum CleanupSignalPhase {
    Quiescent,
    Active,
    LostAuthority,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct CleanupFailures {
    invariant: bool,
    wait: bool,
    cleanup: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CleanupFailures {
    fn record(&mut self, error: BackgroundProcessError) {
        match error.kind() {
            BackgroundProcessErrorKind::Wait => self.wait = true,
            BackgroundProcessErrorKind::Cleanup => self.cleanup = true,
            _ => self.invariant = true,
        }
    }

    fn finish(self) -> Result<(), BackgroundProcessError> {
        // Stable category precedence is independent of which best-effort OS
        // action happens to report its failure first.
        if self.invariant {
            Err(invariant_error())
        } else if self.wait {
            Err(wait_error())
        } else if self.cleanup {
            Err(cleanup_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn combine_cleanup_failures(
    primary: BackgroundProcessError,
    cleanup: Result<(), BackgroundProcessError>,
) -> BackgroundProcessError {
    let mut failures = CleanupFailures::default();
    failures.record(primary);
    if let Err(error) = cleanup {
        failures.record(error);
    }
    failures
        .finish()
        .expect_err("the primary operation supplied one failure")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn signal_group(
    group: rustix::process::Pid,
    signal: rustix::process::Signal,
) -> Result<(), BackgroundProcessError> {
    classify_group_signal(rustix::process::kill_process_group(group, signal))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn signal_group_or_confirm_exited_leader(
    group: rustix::process::Pid,
    signal: rustix::process::Signal,
) -> Result<(), LeaderObservationFailure> {
    #[cfg(test)]
    if OBSERVED_GROUP_SIGNAL.load(Ordering::Acquire) == group.as_raw_nonzero().get().cast_unsigned()
    {
        GROUP_SIGNAL_ATTEMPTS.fetch_add(1, Ordering::AcqRel);
    }
    match rustix::process::kill_process_group(group, signal) {
        Err(rustix::io::Errno::PERM)
            if observe_leader(group)?.is_some()
                && only_group_leader_remains(
                    &group_members(group).map_err(LeaderObservationFailure::Operation)?,
                    group,
                ) =>
        {
            // EPERM is not evidence of success or disappearance. The separate
            // NOWAIT observation and bounded process-group snapshot prove that
            // the only remaining member is the already-exited retained leader.
            Ok(())
        }
        result => classify_group_signal(result).map_err(LeaderObservationFailure::Operation),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn classify_group_signal(
    result: Result<(), rustix::io::Errno>,
) -> Result<(), BackgroundProcessError> {
    match result {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(_) => Err(cleanup_error()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn require_original_group_quiescent(
    group: rustix::process::Pid,
) -> Result<(), BackgroundProcessError> {
    let deadline = Instant::now() + GROUP_DISAPPEARANCE_GRACE;
    let mut observation = ObservationBackoff::with_bounds(
        GROUP_SNAPSHOT_INITIAL_INTERVAL,
        GROUP_SNAPSHOT_MAX_INTERVAL,
    );
    let mut observed_failure = false;
    loop {
        match observe_leader(group) {
            Err(LeaderObservationFailure::LostAuthority) => return Err(wait_error()),
            Err(LeaderObservationFailure::Operation(_)) => observed_failure = true,
            Ok(_) => {}
        }
        match group_members(group) {
            Ok(members) if only_group_leader_remains(&members, group) => {
                return if observed_failure {
                    Err(cleanup_error())
                } else {
                    Ok(())
                };
            }
            Ok(_) => {}
            Err(_) => observed_failure = true,
        }
        if Instant::now() >= deadline {
            return Err(cleanup_error());
        }
        observation.sleep_and_advance();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn group_members(
    group: rustix::process::Pid,
) -> Result<Vec<rustix::process::Pid>, BackgroundProcessError> {
    use std::io::Read;

    #[cfg(test)]
    if consume_injected_failure(
        &GROUP_SNAPSHOT_SPAWN_FAILURE_GROUP,
        &GROUP_SNAPSHOT_SPAWN_FAILURES,
        group.as_raw_nonzero().get().cast_unsigned(),
    ) {
        return Err(cleanup_error());
    }
    let mut command = Command::new("/bin/ps");
    #[cfg(target_os = "linux")]
    command.args(["--no-headers", "-o", "pid", "--pgroup"]);
    #[cfg(target_os = "macos")]
    command.args(["-o", "pid=", "-g"]);
    command
        .arg(group.as_raw_nonzero().get().to_string())
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|_| cleanup_error())?;
    let output = child.stdout.take().ok_or_else(cleanup_error)?;
    let reader = thread::Builder::new()
        .name("machine-god-bg-group-snapshot".to_owned())
        .spawn(move || {
            let mut bytes = Vec::with_capacity(MAX_GROUP_SNAPSHOT_BYTES + 1);
            output
                .take((MAX_GROUP_SNAPSHOT_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
    let Ok(reader) = reader else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(cleanup_error());
    };
    let deadline = Instant::now() + GROUP_SNAPSHOT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(OBSERVATION_INITIAL_INTERVAL);
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(cleanup_error());
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| cleanup_error())?
        .map_err(|_| cleanup_error())?;
    if !status?.success() || bytes.len() > MAX_GROUP_SNAPSHOT_BYTES {
        return Err(cleanup_error());
    }
    #[cfg(test)]
    if consume_injected_failure(
        &GROUP_SNAPSHOT_FAILURE_GROUP,
        &GROUP_SNAPSHOT_FAILURES,
        group.as_raw_nonzero().get().cast_unsigned(),
    ) {
        return Err(cleanup_error());
    }
    parse_group_members(&bytes)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn parse_group_members(bytes: &[u8]) -> Result<Vec<rustix::process::Pid>, BackgroundProcessError> {
    let text = std::str::from_utf8(bytes).map_err(|_| cleanup_error())?;
    let mut members = Vec::new();
    for field in text.split_ascii_whitespace() {
        if members.len() == MAX_GROUP_SNAPSHOT_BYTES / 2 {
            return Err(cleanup_error());
        }
        let raw = field.parse::<i32>().map_err(|_| cleanup_error())?;
        let member = rustix::process::Pid::from_raw(raw).ok_or_else(cleanup_error)?;
        members.push(member);
    }
    Ok(members)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn only_group_leader_remains(
    members: &[rustix::process::Pid],
    leader: rustix::process::Pid,
) -> bool {
    members == [leader]
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn consume_injected_failure(target: &AtomicU32, remaining: &AtomicUsize, actual: u32) -> bool {
    target.load(Ordering::Acquire) == actual
        && remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_sub(1)
            })
            .is_ok()
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn inject_failures(target: &AtomicU32, remaining: &AtomicUsize, pid: NonZeroU32, count: usize) {
    target.store(pid.get(), Ordering::Release);
    remaining.store(count, Ordering::Release);
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn inject_waitid_failures_for_test(leader: NonZeroU32, count: usize) {
    inject_failures(&WAITID_FAILURE_LEADER, &WAITID_FAILURES, leader, count);
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn inject_group_snapshot_failures_for_test(group: NonZeroU32, count: usize) {
    inject_failures(
        &GROUP_SNAPSHOT_FAILURE_GROUP,
        &GROUP_SNAPSHOT_FAILURES,
        group,
        count,
    );
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn inject_group_snapshot_spawn_failures_for_test(group: NonZeroU32, count: usize) {
    inject_failures(
        &GROUP_SNAPSHOT_SPAWN_FAILURE_GROUP,
        &GROUP_SNAPSHOT_SPAWN_FAILURES,
        group,
        count,
    );
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn group_snapshot_is_quiescent_for_test(
    bytes: &[u8],
    leader: i32,
) -> Result<bool, BackgroundProcessError> {
    let leader = rustix::process::Pid::from_raw(leader).ok_or_else(cleanup_error)?;
    parse_group_members(bytes).map(|members| only_group_leader_remains(&members, leader))
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn permission_denied_group_signal_is_failure_for_test() -> bool {
    classify_group_signal(Err(rustix::io::Errno::PERM)).is_err()
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn reset_leader_observations_for_test(leader: NonZeroU32) {
    OBSERVED_LEADER.store(leader.get(), Ordering::Relaxed);
    LEADER_OBSERVATIONS.store(0, Ordering::Relaxed);
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn leader_observations_for_test() -> usize {
    LEADER_OBSERVATIONS.load(Ordering::Relaxed)
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn cancellation_wakeup_latency_for_test(timeout: Duration) -> Duration {
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(0);
    let worker = thread::spawn(move || {
        let mut parker = CancellationParker::new(&worker_cancellation);
        assert!(!parker.is_cancelled());
        ready_sender.send(()).expect("signal registered waiter");
        let started = Instant::now();
        parker.park_timeout(timeout);
        assert!(parker.is_cancelled());
        started.elapsed()
    });
    ready_receiver.recv().expect("await registered waiter");
    assert!(cancellation.cancel());
    worker.join().expect("join cancellation waiter")
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn reset_group_signal_attempts_for_test(group: NonZeroU32) {
    OBSERVED_GROUP_SIGNAL.store(group.get(), Ordering::Release);
    GROUP_SIGNAL_ATTEMPTS.store(0, Ordering::Release);
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn group_signal_attempts_for_test() -> usize {
    GROUP_SIGNAL_ATTEMPTS.load(Ordering::Acquire)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ThreadUnparker(thread::Thread);

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Wake for ThreadUnparker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct CancellationParker {
    cancelled: Pin<Box<Cancelled>>,
    waker: Waker,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CancellationParker {
    fn new(cancellation: &CancellationToken) -> Self {
        Self {
            cancelled: Box::pin(cancellation.cancelled()),
            waker: Waker::from(Arc::new(ThreadUnparker(thread::current()))),
        }
    }

    fn is_cancelled(&mut self) -> bool {
        let waker = self.waker.clone();
        let mut context = Context::from_waker(&waker);
        matches!(self.cancelled.as_mut().poll(&mut context), Poll::Ready(()))
    }

    fn park_timeout(&mut self, timeout: Duration) {
        if !self.is_cancelled() {
            thread::park_timeout(timeout);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ObservationBackoff {
    current: Duration,
    maximum: Duration,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl ObservationBackoff {
    const fn new() -> Self {
        Self::with_bounds(OBSERVATION_INITIAL_INTERVAL, OBSERVATION_MAX_INTERVAL)
    }

    const fn with_bounds(initial: Duration, maximum: Duration) -> Self {
        Self {
            current: initial,
            maximum,
        }
    }

    fn sleep_and_advance(&mut self) {
        thread::sleep(self.current);
        self.advance();
    }

    fn park_and_advance(&mut self, cancellation: &mut CancellationParker) {
        cancellation.park_timeout(self.current);
        self.advance();
    }

    fn advance(&mut self) {
        self.current = self.current.saturating_mul(2).min(self.maximum);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sleep_through(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        thread::sleep(std::cmp::min(
            OBSERVATION_INITIAL_INTERVAL,
            deadline.saturating_duration_since(Instant::now()),
        ));
    }
}

#[cfg(unix)]
fn invalid_request() -> BackgroundProcessError {
    BackgroundProcessError::new(BackgroundProcessErrorKind::InvalidRequest)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_error() -> BackgroundProcessError {
    BackgroundProcessError::new(BackgroundProcessErrorKind::Spawn)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn wait_error() -> BackgroundProcessError {
    BackgroundProcessError::new(BackgroundProcessErrorKind::Wait)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_error() -> BackgroundProcessError {
    BackgroundProcessError::new(BackgroundProcessErrorKind::Cleanup)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn invariant_error() -> BackgroundProcessError {
    BackgroundProcessError::new(BackgroundProcessErrorKind::Invariant)
}
