//! Prepared, process-group-owned background shell execution.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::num::NonZeroU32;
#[cfg(unix)]
use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::PathBuf;
use std::time::Duration;

use machine_god_core::CancellationToken;

#[cfg(target_os = "linux")]
use rustix::fd::AsRawFd;
#[cfg(unix)]
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(unix)]
use rustix::fs::{FileType, Mode, OFlags};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::process::{CommandExt, ExitStatusExt};
#[cfg(target_os = "macos")]
use std::process::ChildStderr;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
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
const OBSERVATION_INTERVAL: Duration = Duration::from_millis(2);
#[cfg(target_os = "linux")]
const GATE_WRAPPER: &str = "IFS= read -r _machine_god_gate || exit 125\nexec /bin/sh -c \"$1\"";
#[cfg(target_os = "linux")]
const GATE_ARGV_ZERO: &str = "machine-god-background-gate";
#[cfg(target_os = "macos")]
const MAX_HELPER_PATH_BYTES: usize = 4 * 1024;
#[cfg(target_os = "macos")]
const MAX_HELPER_ARGUMENTS: usize = 16;
#[cfg(target_os = "macos")]
const MAX_HELPER_ARGUMENT_BYTES: usize = 1024;
#[cfg(target_os = "macos")]
const HELPER_READY_BYTE: u8 = 0xa7;
#[cfg(target_os = "macos")]
const HELPER_READY_TIMEOUT: Duration = Duration::from_secs(2);

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

/// Explicit executable and private arguments used by the macOS inherited-FD
/// launch helper.
#[cfg(target_os = "macos")]
#[derive(Clone, Debug)]
pub struct BackgroundProcessHelper {
    program: PathBuf,
    arguments: Vec<OsString>,
}

#[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
    helper: Option<BackgroundProcessHelper>,
}

impl SystemBackgroundProcessAdapter {
    /// Constructs the active macOS adapter with an explicitly supplied helper.
    #[cfg(target_os = "macos")]
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

/// Runs the macOS inherited-directory helper protocol and replaces the helper
/// with the fixed shell. Hosts call this only for their private helper mode,
/// before ordinary CLI parsing or worker creation.
///
/// The parent maps a clone of the retained directory descriptor to stdout and
/// sends the bounded command frame over stdin only when releasing the process.
/// An EOF before that frame aborts without executing a user command.
///
/// # Errors
///
/// Returns a fixed release, invalid-request, or spawn failure. A successful
/// call does not return because the helper is replaced with `/bin/sh`.
#[cfg(target_os = "macos")]
pub fn run_background_process_helper() -> Result<(), BackgroundProcessError> {
    use std::io::{Read, Write, stderr, stdin, stdout};

    let stdout = stdout();
    validate_directory(stdout.as_fd()).map_err(|_| spawn_error())?;
    rustix::process::fchdir(stdout.as_fd()).map_err(|_| spawn_error())?;
    let mut ready = stderr().lock();
    ready
        .write_all(&[HELPER_READY_BYTE])
        .and_then(|()| ready.flush())
        .map_err(|_| spawn_error())?;
    drop(ready);

    let mut input = stdin().lock();
    let mut length = [0_u8; 4];
    input
        .read_exact(&mut length)
        .map_err(|_| BackgroundProcessError::new(BackgroundProcessErrorKind::Release))?;
    let length = usize::try_from(u32::from_be_bytes(length)).map_err(|_| invalid_request())?;
    if !(1..=MAX_BACKGROUND_PROCESS_COMMAND_BYTES).contains(&length) {
        return Err(invalid_request());
    }
    let mut command = vec![0_u8; length];
    input
        .read_exact(&mut command)
        .map_err(|_| BackgroundProcessError::new(BackgroundProcessErrorKind::Release))?;
    let mut trailing = [0_u8; 1];
    if input.read(&mut trailing).map_err(|_| invalid_request())? != 0 {
        return Err(invalid_request());
    }
    let command = String::from_utf8(command).map_err(|_| invalid_request())?;
    if command.contains('\0') {
        return Err(invalid_request());
    }
    drop((input, stdout));
    let error = Command::new(BACKGROUND_PROCESS_PROGRAM)
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .exec();
    drop(error);
    Err(spawn_error())
}

/// Returns the fixed unsupported result on platforms without the helper
/// protocol, allowing a host to keep one private-mode dispatch path.
#[cfg(not(target_os = "macos"))]
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
    #[cfg(target_os = "macos")]
    command: Option<String>,
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

#[cfg(target_os = "linux")]
fn prepare_system(
    _adapter: &SystemBackgroundProcessAdapter,
    request: BackgroundProcessRequest,
) -> Result<PreparedBackgroundProcess, BackgroundProcessError> {
    // Recheck immediately before the only effect so a closed or substituted
    // descriptor-backed path fails before spawning.
    validate_directory(request.directory.as_fd()).map_err(|_| spawn_error())?;
    let descriptor_path =
        validated_descriptor_path(request.directory.as_fd()).map_err(|_| spawn_error())?;
    if descriptor_path != request.descriptor_path {
        return Err(spawn_error());
    }

    let mut command = Command::new(BACKGROUND_PROCESS_PROGRAM);
    command
        .arg("-c")
        .arg(GATE_WRAPPER)
        .arg(GATE_ARGV_ZERO)
        .arg(&request.command)
        .current_dir(&request.descriptor_path)
        .env_clear()
        .envs(request.environment.iter().cloned())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = command.spawn().map_err(|_| spawn_error())?;
    // `spawn` has completed the descriptor-backed chdir in the child. Retain
    // the request through that boundary, then explicitly discharge it.
    drop(request);
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
    Ok(PreparedBackgroundProcess {
        child: Some(child),
        gate: Some(gate),
        group,
        pid,
    })
}

#[cfg(target_os = "macos")]
fn prepare_system(
    adapter: &SystemBackgroundProcessAdapter,
    request: BackgroundProcessRequest,
) -> Result<PreparedBackgroundProcess, BackgroundProcessError> {
    validate_directory(request.directory.as_fd()).map_err(|_| spawn_error())?;
    let helper = adapter
        .helper
        .as_ref()
        .ok_or_else(|| BackgroundProcessError::new(BackgroundProcessErrorKind::Unsupported))?;
    let directory = rustix::io::dup(request.directory.as_fd()).map_err(|_| spawn_error())?;
    let mut command = Command::new(&helper.program);
    command
        .args(&helper.arguments)
        .env_clear()
        .envs(request.environment.iter().cloned())
        .stdin(Stdio::piped())
        .stdout(Stdio::from(directory))
        .stderr(Stdio::piped())
        .process_group(0);
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
    Ok(PreparedBackgroundProcess {
        child: Some(child),
        gate: Some(gate),
        command: Some(request.command),
        group,
        pid,
    })
}

#[cfg(target_os = "macos")]
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
                thread::sleep(OBSERVATION_INTERVAL);
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
    use std::io::Write;

    let Some(mut gate) = prepared.gate.take() else {
        return Err(invariant_error());
    };
    #[cfg(target_os = "linux")]
    let release = gate.write_all(b"\n");
    #[cfg(target_os = "macos")]
    let release = prepared
        .command
        .take()
        .ok_or_else(invariant_error)
        .and_then(|command| {
            let length = u32::try_from(command.len()).map_err(|_| invariant_error())?;
            gate.write_all(&length.to_be_bytes())
                .and_then(|()| gate.write_all(command.as_bytes()))
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
    let observed = loop {
        match observe_leader(owned.group)? {
            Some(status) => break status,
            None => thread::sleep(OBSERVATION_INTERVAL),
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
    loop {
        if stop.is_cancelled() {
            stop_owned(owned)?;
            return Ok(BackgroundProcessOutcome::Stopped);
        }
        if let Some(status) = observe_leader(owned.group)? {
            return finish_observed(owned, status).map(BackgroundProcessOutcome::Completed);
        }
        thread::sleep(OBSERVATION_INTERVAL);
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
    // The NOWAIT-observed leader remains a zombie and pins the numeric PID and
    // PGID while every remaining original-group descendant is signalled.
    let term = signal_group(owned.group, rustix::process::Signal::TERM);
    let kill = signal_group(owned.group, rustix::process::Signal::KILL);
    let reaped = owned
        .child
        .as_mut()
        .ok_or_else(invariant_error)?
        .wait()
        .map_err(|_| wait_error());
    owned.child = None;
    let gone = require_group_gone(owned.group);
    term?;
    kill?;
    let reaped = reaped?;
    gone?;
    if exit_status(reaped) != observed {
        return Err(invariant_error());
    }
    Ok(observed)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn stop_owned(owned: &mut OwnedBackgroundProcess) -> Result<(), BackgroundProcessError> {
    let Some(mut child) = owned.child.take() else {
        return Ok(());
    };
    cleanup_child(&mut child, owned.group, BACKGROUND_PROCESS_TERM_GRACE)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn stop_owned(_owned: &mut OwnedBackgroundProcess) -> Result<(), BackgroundProcessError> {
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn observe_leader(
    leader: rustix::process::Pid,
) -> Result<Option<BackgroundProcessExit>, BackgroundProcessError> {
    let status = rustix::process::waitid(
        rustix::process::WaitId::Pid(leader),
        rustix::process::WaitIdOptions::EXITED
            | rustix::process::WaitIdOptions::NOHANG
            | rustix::process::WaitIdOptions::NOWAIT,
    )
    .map_err(|_| wait_error())?;
    status.map(waitid_status).transpose()
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
    let term = signal_group(group, rustix::process::Signal::TERM);
    if !term_grace.is_zero() {
        sleep_through(term_grace);
    }
    // Keep `child` unreaped until this final original-group signal so the
    // leader's numeric PID/PGID cannot be recycled into an unrelated group.
    let kill = signal_group(group, rustix::process::Signal::KILL);
    let direct_kill = if kill.is_err() {
        child.kill().map_err(|_| cleanup_error())
    } else {
        Ok(())
    };
    let reaped = child.wait().map(|_| ()).map_err(|_| wait_error());
    let gone = require_group_gone(group);
    term?;
    kill?;
    direct_kill?;
    reaped?;
    gone
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn signal_group(
    group: rustix::process::Pid,
    signal: rustix::process::Signal,
) -> Result<(), BackgroundProcessError> {
    match rustix::process::kill_process_group(group, signal) {
        Ok(()) | Err(rustix::io::Errno::SRCH | rustix::io::Errno::PERM) => Ok(()),
        Err(_) => Err(cleanup_error()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn require_group_gone(group: rustix::process::Pid) -> Result<(), BackgroundProcessError> {
    let deadline = Instant::now() + GROUP_DISAPPEARANCE_GRACE;
    loop {
        match rustix::process::test_kill_process_group(group) {
            Err(rustix::io::Errno::SRCH) => return Ok(()),
            Ok(()) if Instant::now() < deadline => thread::sleep(OBSERVATION_INTERVAL),
            Ok(()) | Err(_) => return Err(cleanup_error()),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sleep_through(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        thread::sleep(std::cmp::min(
            OBSERVATION_INTERVAL,
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
