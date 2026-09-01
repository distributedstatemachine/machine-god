//! Prepared, process-group-owned background shell execution.

#![allow(
    dead_code,
    reason = "lower-level process lifecycle primitives remain directly integration-tested"
)]

#[cfg(unix)]
use std::collections::{BTreeSet, HashSet};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::future::Future;
#[cfg(target_os = "linux")]
use std::mem::MaybeUninit;
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
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
use std::sync::atomic::AtomicU32;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::{Arc, Condvar, Mutex, OnceLock};
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
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_CAPTURED_GROUP_MEMBERS: usize = 32 * 1024;
#[cfg(target_os = "linux")]
const MAX_LINUX_PROC_ENTRIES: usize = 128 * 1024;
#[cfg(target_os = "linux")]
const MAX_LINUX_PROC_STAT_BYTES: usize = 4 * 1024;
#[cfg(target_os = "linux")]
const MAX_LINUX_PROC_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
#[cfg(target_os = "linux")]
const MAX_LINUX_MOUNTINFO_BYTES: usize = 1024 * 1024;
#[cfg(target_os = "linux")]
const MAX_LINUX_MOUNTINFO_ENTRIES: usize = 4 * 1024;
#[cfg(target_os = "linux")]
const LINUX_PROC_SUPER_MAGIC: u64 = 0x9fa0;
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
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static GROUP_SIGNAL_EPERM_GROUP: AtomicU32 = AtomicU32::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static GROUP_SIGNAL_EPERM_REMAINING: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static OBSERVED_GROUP_SNAPSHOT: AtomicU32 = AtomicU32::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static GROUP_SNAPSHOT_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static CANCEL_RELEASE_BEFORE_COMMIT_PID: AtomicU32 = AtomicU32::new(0);
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
const CHILD_REAP_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RELEASE_FRAME_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RELEASE_FRAME_MAGIC: &[u8; 8] = b"MGBG\0\0\0\x01";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RELEASE_COMMIT_BYTE: u8 = 0x6d;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_CHILD_REAP_AUTHORITIES: usize = 64;
#[cfg(any(target_os = "linux", target_os = "macos"))]
static ACTIVE_CHILD_REAP_AUTHORITIES: AtomicUsize = AtomicUsize::new(0);
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
    /// Release was cancelled before its explicit commit and cleanup succeeded.
    Cancelled,
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ChildReapPermit {
    reaper: Arc<ChildReaper>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for ChildReapPermit {
    fn drop(&mut self) {
        ACTIVE_CHILD_REAP_AUTHORITIES.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct QuarantinedChild {
    child: Child,
    _permit: ChildReapPermit,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ChildReaper {
    children: Mutex<Vec<QuarantinedChild>>,
    wake: Condvar,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
static CHILD_REAPER: OnceLock<Result<Arc<ChildReaper>, ()>> = OnceLock::new();

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn child_reaper() -> Result<&'static Arc<ChildReaper>, BackgroundProcessError> {
    CHILD_REAPER
        .get_or_init(|| {
            let reaper = Arc::new(ChildReaper {
                children: Mutex::new(Vec::with_capacity(MAX_CHILD_REAP_AUTHORITIES)),
                wake: Condvar::new(),
            });
            let worker_reaper = Arc::clone(&reaper);
            thread::Builder::new()
                .name("machine-god-bg-child-reaper".to_owned())
                .spawn(move || run_child_reaper(&worker_reaper))
                .map(|_| reaper)
                .map_err(|_| ())
        })
        .as_ref()
        .map_err(|()| spawn_error())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_child_reaper(reaper: &ChildReaper) {
    loop {
        let mut children = reaper
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while children.is_empty() {
            children = reaper
                .wake
                .wait(children)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let mut index = 0;
        while index < children.len() {
            match children[index].child.try_wait() {
                Ok(Some(_)) => {
                    children.swap_remove(index);
                }
                Ok(None) | Err(_) => index += 1,
            }
        }
        let _ = reaper
            .wake
            .wait_timeout(children, OBSERVATION_MAX_INTERVAL)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn reserve_child_reap_authority() -> Result<ChildReapPermit, BackgroundProcessError> {
    let reaper = Arc::clone(child_reaper()?);
    ACTIVE_CHILD_REAP_AUTHORITIES
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
            (active < MAX_CHILD_REAP_AUTHORITIES).then_some(active + 1)
        })
        .map_err(|_| spawn_error())?;
    Ok(ChildReapPermit { reaper })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn quarantine_child(child: Child, permit: ChildReapPermit) {
    let reaper = Arc::clone(&permit.reaper);
    let mut children = reaper
        .children
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    debug_assert!(children.len() < MAX_CHILD_REAP_AUTHORITIES);
    children.push(QuarantinedChild {
        child,
        _permit: permit,
    });
    reaper.wake.notify_one();
}

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
        self.prepare_cancellable(request, &CancellationToken::new())
    }

    /// Spawns a private-gated process-group leader while observing cooperative
    /// cancellation through helper readiness and cleanup.
    ///
    /// # Errors
    ///
    /// Returns a fixed spawn, invariant, or unsupported failure. Cancellation
    /// is reported as a spawn failure at this native boundary after the helper
    /// and its process group have been terminated and reaped.
    pub fn prepare_cancellable(
        &self,
        request: BackgroundProcessRequest,
        cancellation: &CancellationToken,
    ) -> Result<PreparedBackgroundProcess, BackgroundProcessError> {
        prepare_system(self, request, cancellation)
    }
}

/// Runs the inherited-directory helper protocol and replaces the helper with
/// the fixed shell. Hosts call this only for their private helper mode, before
/// ordinary CLI parsing or worker creation.
///
/// On macOS the parent maps a clone of the retained directory descriptor to
/// stdout. Linux starts the helper in the retained directory. On both systems,
/// the requested command and environment remain inert bytes in a bounded stdin
/// frame until release. The complete payload is inert until a distinct final
/// commit byte arrives; EOF at any earlier point aborts without executing a
/// user command.
///
/// # Errors
///
/// Returns a fixed release, invalid-request, or spawn failure. A successful
/// call does not return because the helper is replaced with `/bin/sh`.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn run_background_process_helper() -> Result<(), BackgroundProcessError> {
    #[cfg(target_os = "macos")]
    use std::io::stdout;
    use std::io::{Write, stderr, stdin};

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
    let mut commit = [0_u8; 1];
    read_release_bytes(&mut input, &mut commit)?;
    if commit[0] != RELEASE_COMMIT_BYTE {
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
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    snapshot_authority: Option<GroupSnapshotAuthority>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    reap_permit: Option<ChildReapPermit>,
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
        release_prepared(&mut self, &CancellationToken::new())
    }

    /// Releases the private start gate while observing cooperative
    /// cancellation. Frame transmission is nonblocking and bounded by a fixed
    /// deadline even when the ready helper stops reading its gate.
    ///
    /// # Errors
    ///
    /// Returns a fixed release or cleanup failure. Cancellation, timeout, or
    /// an incomplete frame closes the gate and reaps the prepared group before
    /// returning.
    pub fn release_cancellable(
        mut self,
        cancellation: &CancellationToken,
    ) -> Result<OwnedBackgroundProcess, BackgroundProcessError> {
        release_prepared(&mut self, cancellation)
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
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    snapshot_authority: Option<GroupSnapshotAuthority>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    reap_permit: Option<ChildReapPermit>,
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
    let mut keys = BTreeSet::new();
    for (key, value) in environment {
        let key = key.as_os_str().as_bytes();
        let value = value.as_os_str().as_bytes();
        if key.is_empty()
            || key.len() > MAX_BACKGROUND_PROCESS_ENVIRONMENT_KEY_BYTES
            || value.len() > MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES
            || key.contains(&b'=')
            || key.contains(&0)
            || value.contains(&0)
            || !keys.insert(key)
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
fn require_exclusive_child_reaping(
    cancellation: &CancellationToken,
) -> Result<(), BackgroundProcessError> {
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
    require_exclusive_child_reaping_with(probe, cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn require_exclusive_child_reaping_with(
    mut probe: Command,
    cancellation: &CancellationToken,
) -> Result<(), BackgroundProcessError> {
    let mut permit = Some(reserve_child_reap_authority()?);
    let mut child = Some(probe.spawn().map_err(|_| spawn_error())?);
    let deadline = Instant::now() + CHILD_REAP_PROBE_TIMEOUT;
    let mut cancellation = CancellationParker::new(cancellation);
    loop {
        if cancellation.is_cancelled() {
            break;
        }
        match child.as_mut().ok_or_else(invariant_error)?.try_wait() {
            Ok(Some(status)) if status.success() => {
                drop(child.take());
                drop(permit.take());
                return Ok(());
            }
            Ok(None) if Instant::now() < deadline => cancellation.park_timeout(std::cmp::min(
                OBSERVATION_INITIAL_INTERVAL,
                deadline.saturating_duration_since(Instant::now()),
            )),
            Ok(Some(_) | None) | Err(_) => break,
        }
    }
    terminate_and_reap_or_quarantine(&mut child, &mut permit);
    Err(spawn_error())
}

#[cfg(target_os = "linux")]
struct GroupSnapshotAuthority {
    proc_root: OwnedFd,
    mount_id: u64,
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct GroupSnapshotAuthority;

#[cfg(target_os = "linux")]
impl GroupSnapshotAuthority {
    fn open() -> Result<Self, BackgroundProcessError> {
        let proc_root = rustix::fs::open(
            "/proc",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| spawn_error())?;
        let mount_id = linux_fd_mount_id(proc_root.as_fd()).map_err(|()| spawn_error())?;
        let authority = Self {
            proc_root,
            mount_id,
        };
        authority.validate().map_err(|()| spawn_error())?;
        Ok(authority)
    }

    fn validate(&self) -> Result<(), ()> {
        let filesystem = rustix::fs::fstatfs(self.proc_root.as_fd()).map_err(|_| ())?;
        if u64::try_from(filesystem.f_type).ok() != Some(LINUX_PROC_SUPER_MAGIC)
            || linux_fd_mount_id(self.proc_root.as_fd())? != self.mount_id
        {
            return Err(());
        }
        let mountinfo_fd = rustix::fs::openat(
            self.proc_root.as_fd(),
            "self/mountinfo",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| ())?;
        if linux_fd_mount_id(mountinfo_fd.as_fd())? != self.mount_id {
            return Err(());
        }
        let mountinfo = std::fs::File::from(mountinfo_fd);
        validate_linux_proc_mountinfo(std::io::BufReader::new(mountinfo), self.mount_id)
    }
}

#[cfg(target_os = "linux")]
fn linux_fd_mount_id(fd: BorrowedFd<'_>) -> Result<u64, ()> {
    use rustix::fs::{AtFlags, StatxFlags};

    let status =
        rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID).map_err(|_| ())?;
    if status.stx_mask & StatxFlags::MNT_ID.bits() == 0 {
        return Err(());
    }
    Ok(status.stx_mnt_id)
}

#[cfg(target_os = "linux")]
fn validate_linux_proc_mountinfo(
    reader: impl std::io::BufRead,
    expected_mount_id: u64,
) -> Result<(), ()> {
    let mut reader = std::io::Read::take(
        reader,
        u64::try_from(MAX_LINUX_MOUNTINFO_BYTES + 1).map_err(|_| ())?,
    );
    let mut line = Vec::with_capacity(256);
    let mut bytes = 0_usize;
    let mut entries = 0_usize;
    let mut proc_mounts = 0_usize;
    loop {
        line.clear();
        let read = std::io::BufRead::read_until(&mut reader, b'\n', &mut line).map_err(|_| ())?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read)
            .filter(|value| *value <= MAX_LINUX_MOUNTINFO_BYTES)
            .ok_or(())?;
        entries = entries
            .checked_add(1)
            .filter(|value| *value <= MAX_LINUX_MOUNTINFO_ENTRIES)
            .ok_or(())?;
        let content = line.strip_suffix(b"\n").ok_or(())?;
        if content.is_empty() {
            return Err(());
        }
        if parse_linux_mountinfo_line(content, expected_mount_id)? {
            proc_mounts = proc_mounts.checked_add(1).ok_or(())?;
        }
    }
    if bytes == 0 || proc_mounts != 1 {
        return Err(());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_mount_authority(
    bytes: &[u8],
    expected_mount_id: u64,
) -> Result<(), BackgroundProcessError> {
    validate_linux_proc_mountinfo(std::io::Cursor::new(bytes), expected_mount_id)
        .map_err(|()| spawn_error())
}

#[cfg(target_os = "linux")]
fn parse_linux_mountinfo_line(line: &[u8], expected_mount_id: u64) -> Result<bool, ()> {
    let mut fields = line
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty());
    let mount_id = fields.next().and_then(parse_linux_positive_u64).ok_or(())?;
    fields
        .next()
        .and_then(parse_linux_nonnegative_u64)
        .ok_or(())?;
    let device = fields.next().ok_or(())?;
    if !device.contains(&b':') {
        return Err(());
    }
    let root = fields.next().ok_or(())?;
    let mountpoint = fields.next().ok_or(())?;
    let mount_options = fields.next().ok_or(())?;
    let mut separator = false;
    for field in fields.by_ref() {
        if field == b"-" {
            separator = true;
            break;
        }
    }
    if !separator {
        return Err(());
    }
    let filesystem = fields.next().ok_or(())?;
    fields.next().ok_or(())?;
    let super_options = fields.next().ok_or(())?;
    if fields.next().is_some() {
        return Err(());
    }
    if linux_proc_authority_overmount(mountpoint) {
        return Err(());
    }
    if mountpoint != b"/proc" {
        return Ok(false);
    }
    if mount_id != expected_mount_id
        || root != b"/"
        || filesystem != b"proc"
        || !linux_proc_options_are_unrestricted(mount_options)
        || !linux_proc_options_are_unrestricted(super_options)
    {
        return Err(());
    }
    Ok(true)
}

#[cfg(target_os = "linux")]
fn linux_proc_options_are_unrestricted(options: &[u8]) -> bool {
    options
        .split(|byte| *byte == b',')
        .all(|option| !option.starts_with(b"hidepid") || option == b"hidepid=0")
}

#[cfg(target_os = "linux")]
fn linux_numeric_proc_mountpoint(mountpoint: &[u8]) -> bool {
    mountpoint
        .strip_prefix(b"/proc/")
        .and_then(|suffix| suffix.split(|byte| *byte == b'/').next())
        .is_some_and(|component| !component.is_empty() && component.iter().all(u8::is_ascii_digit))
}

#[cfg(target_os = "linux")]
fn linux_proc_authority_overmount(mountpoint: &[u8]) -> bool {
    linux_numeric_proc_mountpoint(mountpoint)
        || matches!(
            mountpoint,
            b"/proc/self"
                | b"/proc/self/mountinfo"
                | b"/proc/thread-self"
                | b"/proc/thread-self/mountinfo"
        )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn prepare_system(
    adapter: &SystemBackgroundProcessAdapter,
    request: BackgroundProcessRequest,
    cancellation: &CancellationToken,
) -> Result<PreparedBackgroundProcess, BackgroundProcessError> {
    if cancellation.is_cancelled() {
        return Err(spawn_error());
    }
    // Recheck immediately before the only effect so a closed or substituted
    // descriptor-backed path fails before spawning.
    validate_directory(request.directory.as_fd()).map_err(|_| spawn_error())?;
    let helper = adapter
        .helper
        .as_ref()
        .ok_or_else(|| BackgroundProcessError::new(BackgroundProcessErrorKind::Unsupported))?;
    #[cfg(target_os = "linux")]
    let snapshot_authority = GroupSnapshotAuthority::open()?;
    #[cfg(target_os = "macos")]
    let snapshot_authority = GroupSnapshotAuthority;
    require_exclusive_child_reaping(cancellation)?;
    #[cfg(target_os = "linux")]
    let retained_cwd = retained_linux_cwd(&request)?;
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
    if cancellation.is_cancelled() {
        return Err(spawn_error());
    }
    let mut reap_permit = Some(reserve_child_reap_authority()?);
    let mut child = Some(command.spawn().map_err(|_| spawn_error())?);
    let pid = NonZeroU32::new(child.as_ref().ok_or_else(invariant_error)?.id())
        .ok_or_else(invariant_error)?;
    let group =
        rustix::process::Pid::from_raw(i32::try_from(pid.get()).map_err(|_| invariant_error())?)
            .ok_or_else(invariant_error)?;
    if rustix::process::getpgid(Some(group)) != Ok(group) {
        let _ = cleanup_child(
            &mut child,
            &mut reap_permit,
            group,
            Duration::ZERO,
            &snapshot_authority,
        );
        return Err(invariant_error());
    }
    let Some(gate) = child.as_mut().and_then(|child| child.stdin.take()) else {
        let _ = cleanup_child(
            &mut child,
            &mut reap_permit,
            group,
            Duration::ZERO,
            &snapshot_authority,
        );
        return Err(spawn_error());
    };
    let Some(ready) = child.as_mut().and_then(|child| child.stderr.take()) else {
        let _ = cleanup_child(
            &mut child,
            &mut reap_permit,
            group,
            Duration::ZERO,
            &snapshot_authority,
        );
        return Err(spawn_error());
    };
    if await_helper_ready(ready, cancellation).is_err() {
        cleanup_child(
            &mut child,
            &mut reap_permit,
            group,
            Duration::ZERO,
            &snapshot_authority,
        )?;
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
        child,
        gate: Some(gate),
        command: Some(command),
        environment: Some(environment),
        group,
        snapshot_authority: Some(snapshot_authority),
        reap_permit,
        pid,
    })
}

#[cfg(target_os = "linux")]
fn retained_linux_cwd(
    request: &BackgroundProcessRequest,
) -> Result<PathBuf, BackgroundProcessError> {
    let descriptor_path =
        validated_descriptor_path(request.directory.as_fd()).map_err(|_| spawn_error())?;
    if descriptor_path != request.descriptor_path {
        return Err(spawn_error());
    }
    Ok(descriptor_path)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn await_helper_ready(
    mut ready: ChildStderr,
    cancellation: &CancellationToken,
) -> Result<(), BackgroundProcessError> {
    use std::io::Read;

    let flags = rustix::fs::fcntl_getfl(&ready).map_err(|_| spawn_error())?;
    rustix::fs::fcntl_setfl(&ready, flags | OFlags::NONBLOCK).map_err(|_| spawn_error())?;
    let deadline = Instant::now() + HELPER_READY_TIMEOUT;
    let mut cancellation = CancellationParker::new(cancellation);
    let mut byte = [0_u8; 1];
    loop {
        if cancellation.is_cancelled() {
            return Err(spawn_error());
        }
        match ready.read(&mut byte) {
            Ok(1) if byte[0] == HELPER_READY_BYTE => {
                return if cancellation.is_cancelled() {
                    Err(spawn_error())
                } else {
                    Ok(())
                };
            }
            Ok(1 | 0) => return Err(spawn_error()),
            Ok(_) => unreachable!("one-byte ready buffer has bounded reads"),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                cancellation.park_timeout(std::cmp::min(
                    OBSERVATION_INITIAL_INTERVAL,
                    deadline.saturating_duration_since(Instant::now()),
                ));
            }
            Err(_) => return Err(spawn_error()),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn prepare_system(
    _adapter: &SystemBackgroundProcessAdapter,
    request: BackgroundProcessRequest,
    _cancellation: &CancellationToken,
) -> Result<PreparedBackgroundProcess, BackgroundProcessError> {
    drop(request);
    Err(BackgroundProcessError::new(
        BackgroundProcessErrorKind::Unsupported,
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn release_prepared(
    prepared: &mut PreparedBackgroundProcess,
    cancellation: &CancellationToken,
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
            write_release_frame_bounded(
                &mut gate,
                &command,
                &environment,
                cancellation,
                prepared.pid,
            )
            .map_err(ReleaseWriteFailure::into_process_error)
        });
    if let Err(release_error) = release {
        drop(gate);
        return match abort_prepared(prepared) {
            Ok(()) => Err(release_error),
            Err(cleanup_error) => Err(cleanup_error),
        };
    }
    drop(gate);
    if prepared.child.is_none()
        || prepared.snapshot_authority.is_none()
        || prepared.reap_permit.is_none()
    {
        return Err(invariant_error());
    }
    let child = prepared.child.take().ok_or_else(invariant_error)?;
    let snapshot_authority = prepared
        .snapshot_authority
        .take()
        .ok_or_else(invariant_error)?;
    let reap_permit = prepared.reap_permit.take().ok_or_else(invariant_error)?;
    Ok(OwnedBackgroundProcess {
        child: Some(child),
        group: prepared.group,
        snapshot_authority: Some(snapshot_authority),
        reap_permit: Some(reap_permit),
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
fn write_release_frame_bounded(
    output: &mut ChildStdin,
    command: &str,
    environment: &[(OsString, OsString)],
    cancellation: &CancellationToken,
    pid: NonZeroU32,
) -> Result<(), ReleaseWriteFailure> {
    #[cfg(not(test))]
    let _ = pid;
    let flags = rustix::fs::fcntl_getfl(&*output).map_err(|_| ReleaseWriteFailure::Release)?;
    rustix::fs::fcntl_setfl(&*output, flags | OFlags::NONBLOCK)
        .map_err(|_| ReleaseWriteFailure::Release)?;
    let mut output = BoundedGateWriter {
        output,
        cancellation_token: cancellation.clone(),
        cancellation: CancellationParker::new(cancellation),
        deadline: Instant::now() + RELEASE_FRAME_TIMEOUT,
    };
    if write_release_frame(&mut output, command, environment).is_err() {
        return Err(if output.cancellation_token.is_cancelled() {
            ReleaseWriteFailure::Cancelled
        } else {
            ReleaseWriteFailure::Release
        });
    }
    #[cfg(test)]
    if CANCEL_RELEASE_BEFORE_COMMIT_PID.load(Ordering::Acquire) == pid.get() {
        output.cancellation_token.cancel();
    }
    if output.cancellation_token.is_cancelled() || Instant::now() >= output.deadline {
        return Err(if output.cancellation_token.is_cancelled() {
            ReleaseWriteFailure::Cancelled
        } else {
            ReleaseWriteFailure::Release
        });
    }
    std::io::Write::write_all(&mut output, &[RELEASE_COMMIT_BYTE]).map_err(|_| {
        if output.cancellation_token.is_cancelled() {
            ReleaseWriteFailure::Cancelled
        } else {
            ReleaseWriteFailure::Release
        }
    })?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
enum ReleaseWriteFailure {
    Cancelled,
    Release,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl ReleaseWriteFailure {
    const fn into_process_error(self) -> BackgroundProcessError {
        BackgroundProcessError::new(match self {
            Self::Cancelled => BackgroundProcessErrorKind::Cancelled,
            Self::Release => BackgroundProcessErrorKind::Release,
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct BoundedGateWriter<'a> {
    output: &'a mut ChildStdin,
    cancellation_token: CancellationToken,
    cancellation: CancellationParker,
    deadline: Instant,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl std::io::Write for BoundedGateWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        loop {
            if self.cancellation_token.is_cancelled() {
                // `Write::write_all` retries `Interrupted`; use the same fixed
                // terminal category as the bounded deadline so cancellation
                // cannot turn into a retry loop.
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            if Instant::now() >= self.deadline {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            match self.output.write(bytes) {
                Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
                Ok(written) => return Ok(written),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    self.cancellation.park_timeout(std::cmp::min(
                        OBSERVATION_INITIAL_INTERVAL,
                        self.deadline.saturating_duration_since(Instant::now()),
                    ));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_frame_length(output: &mut impl std::io::Write, length: usize) -> std::io::Result<()> {
    let length = u32::try_from(length).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    output.write_all(&length.to_be_bytes())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn release_prepared(
    _prepared: &mut PreparedBackgroundProcess,
    _cancellation: &CancellationToken,
) -> Result<OwnedBackgroundProcess, BackgroundProcessError> {
    Err(BackgroundProcessError::new(
        BackgroundProcessErrorKind::Unsupported,
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn abort_prepared(prepared: &mut PreparedBackgroundProcess) -> Result<(), BackgroundProcessError> {
    drop(prepared.gate.take());
    if prepared.child.is_none() {
        return Ok(());
    }
    let authority = prepared
        .snapshot_authority
        .as_ref()
        .ok_or_else(invariant_error)?;
    cleanup_child(
        &mut prepared.child,
        &mut prepared.reap_permit,
        prepared.group,
        Duration::ZERO,
        authority,
    )
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
enum BoundedReap {
    Reaped(ExitStatus),
    LostAuthority,
    TimedOut,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn poll_child_reap(
    child: &mut Option<Child>,
    deadline: Instant,
) -> Result<BoundedReap, BackgroundProcessError> {
    loop {
        let child = child.as_mut().ok_or_else(invariant_error)?;
        match child.try_wait() {
            Ok(Some(status)) => return Ok(BoundedReap::Reaped(status)),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(std::cmp::min(
                    OBSERVATION_INITIAL_INTERVAL,
                    deadline.saturating_duration_since(Instant::now()),
                ));
            }
            Ok(None) => return Ok(BoundedReap::TimedOut),
            Err(_) => return Ok(BoundedReap::LostAuthority),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn discharge_reaped_child(child: &mut Option<Child>, reap_permit: &mut Option<ChildReapPermit>) {
    drop(child.take());
    drop(reap_permit.take());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn quarantine_owned_child(
    child: &mut Option<Child>,
    reap_permit: &mut Option<ChildReapPermit>,
) -> Result<(), BackgroundProcessError> {
    if child.is_none() || reap_permit.is_none() {
        return Err(invariant_error());
    }
    let child = child.take().ok_or_else(invariant_error)?;
    let permit = reap_permit.take().ok_or_else(invariant_error)?;
    quarantine_child(child, permit);
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn terminate_and_reap_or_quarantine(
    child: &mut Option<Child>,
    reap_permit: &mut Option<ChildReapPermit>,
) {
    if let Some(child) = child.as_mut() {
        let _ = child.kill();
    }
    match poll_child_reap(child, Instant::now() + CHILD_REAP_PROBE_TIMEOUT) {
        Ok(BoundedReap::Reaped(_) | BoundedReap::LostAuthority) | Err(_) => {
            discharge_reaped_child(child, reap_permit);
        }
        Ok(BoundedReap::TimedOut) => {
            let _ = quarantine_owned_child(child, reap_permit);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn reap_child_bounded(
    child: &mut Option<Child>,
    reap_permit: &mut Option<ChildReapPermit>,
    expected: Option<BackgroundProcessExit>,
    failures: &mut CleanupFailures,
) {
    if child.is_none() || reap_permit.is_none() {
        failures.record(invariant_error());
        return;
    }
    match poll_child_reap(child, Instant::now() + CHILD_REAP_PROBE_TIMEOUT) {
        Ok(BoundedReap::Reaped(status)) => {
            if expected.is_some_and(|expected| exit_status(status) != expected) {
                failures.record(invariant_error());
            }
            discharge_reaped_child(child, reap_permit);
        }
        Ok(BoundedReap::LostAuthority) => {
            failures.record(wait_error());
            discharge_reaped_child(child, reap_permit);
        }
        Ok(BoundedReap::TimedOut) => {
            if child.as_mut().is_none_or(|child| child.kill().is_err()) {
                failures.record(cleanup_error());
            }
            match poll_child_reap(child, Instant::now() + CHILD_REAP_PROBE_TIMEOUT) {
                Ok(BoundedReap::Reaped(status)) => {
                    if expected.is_some_and(|expected| exit_status(status) != expected) {
                        failures.record(invariant_error());
                    }
                    discharge_reaped_child(child, reap_permit);
                }
                Ok(BoundedReap::LostAuthority) => {
                    failures.record(wait_error());
                    discharge_reaped_child(child, reap_permit);
                }
                Ok(BoundedReap::TimedOut) => {
                    failures.record(cleanup_error());
                    if let Err(error) = quarantine_owned_child(child, reap_permit) {
                        failures.record(error);
                    }
                }
                Err(error) => failures.record(error),
            }
        }
        Err(error) => failures.record(error),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_child(
    child: &mut Option<Child>,
    reap_permit: &mut Option<ChildReapPermit>,
    group: rustix::process::Pid,
    term_grace: Duration,
    authority: &GroupSnapshotAuthority,
) -> Result<(), BackgroundProcessError> {
    cleanup_child_with_expected(
        child,
        reap_permit,
        group,
        term_grace,
        None,
        false,
        authority,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_owned_child(
    owned: &mut OwnedBackgroundProcess,
    term_grace: Duration,
    expected: Option<BackgroundProcessExit>,
    force_cleanup: bool,
) -> Result<(), BackgroundProcessError> {
    if owned.child.is_none() {
        return Err(invariant_error());
    }
    let authority = owned
        .snapshot_authority
        .as_ref()
        .ok_or_else(invariant_error)?;
    cleanup_child_with_expected(
        &mut owned.child,
        &mut owned.reap_permit,
        owned.group,
        term_grace,
        expected,
        force_cleanup,
        authority,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_child_with_expected(
    child: &mut Option<Child>,
    reap_permit: &mut Option<ChildReapPermit>,
    group: rustix::process::Pid,
    term_grace: Duration,
    expected: Option<BackgroundProcessExit>,
    force_cleanup: bool,
    authority: &GroupSnapshotAuthority,
) -> Result<(), BackgroundProcessError> {
    let mut failures = CleanupFailures::default();
    let mut force_signals = force_cleanup;
    let mut captured_members = CapturedMemberUnion::new();
    let mut group_phase = cleanup_group_signal_phase(
        group,
        rustix::process::Signal::TERM,
        &mut force_signals,
        &mut failures,
        authority,
        &mut captured_members,
    );
    if group_phase == CleanupSignalPhase::LostAuthority {
        discharge_reaped_child(child, reap_permit);
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
            authority,
            &mut captured_members,
        );
        if group_phase == CleanupSignalPhase::LostAuthority {
            discharge_reaped_child(child, reap_permit);
            return failures.finish();
        }
        // A successful group KILL already targets the retained leader. Avoid a
        // redundant numeric child signal, and retain wait authority while the
        // bounded group-disappearance proof runs.
        if (group_phase != CleanupSignalPhase::Quiescent
            || captured_members.iter().any(|member| member.pid != group))
            && let Err(error) = require_original_group_quiescent(group, authority, captured_members)
        {
            failures.record(error);
        }
    }
    reap_child_bounded(child, reap_permit, expected, &mut failures);
    failures.finish()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_group_signal_phase(
    group: rustix::process::Pid,
    signal: rustix::process::Signal,
    force_signals: &mut bool,
    failures: &mut CleanupFailures,
    authority: &GroupSnapshotAuthority,
    captured_members: &mut CapturedMemberUnion,
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
    let (only_exited_leader, phase_only_leader) = match group_members(authority, group) {
        Ok(members) => {
            let phase_only_leader = only_group_leader_remains(&members, group);
            let only_exited_leader = leader_exited && phase_only_leader;
            if let Err(error) = captured_members.retain(members) {
                *force_signals = true;
                failures.record(error);
            }
            (only_exited_leader, phase_only_leader)
        }
        Err(error) => {
            *force_signals = true;
            failures.record(error);
            (false, false)
        }
    };
    // Keep the leader unreaped throughout the signal phases. Any observation
    // failure forces both group signals instead of becoming a cleanup short
    // circuit; a clean sole-zombie proof avoids unnecessary signalling.
    if (*force_signals || !only_exited_leader)
        && let Err(error) = signal_group_or_confirm_exited_leader(group, signal, phase_only_leader)
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
    phase_proved_only_leader: bool,
) -> Result<(), LeaderObservationFailure> {
    #[cfg(test)]
    if OBSERVED_GROUP_SIGNAL.load(Ordering::Acquire) == group.as_raw_nonzero().get().cast_unsigned()
    {
        GROUP_SIGNAL_ATTEMPTS.fetch_add(1, Ordering::AcqRel);
    }
    #[cfg(test)]
    let signal_result = if consume_injected_failure(
        &GROUP_SIGNAL_EPERM_GROUP,
        &GROUP_SIGNAL_EPERM_REMAINING,
        group.as_raw_nonzero().get().cast_unsigned(),
    ) {
        Err(rustix::io::Errno::PERM)
    } else {
        rustix::process::kill_process_group(group, signal)
    };
    #[cfg(not(test))]
    let signal_result = rustix::process::kill_process_group(group, signal);
    match signal_result {
        Err(rustix::io::Errno::PERM)
            if phase_proved_only_leader && observe_leader(group)?.is_some() =>
        {
            // EPERM is not evidence of success or disappearance. The separate
            // NOWAIT observation and process-group snapshot from this exact
            // signal phase already prove that the only remaining member is the
            // exited retained leader; do not add another global table scan.
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
    authority: &GroupSnapshotAuthority,
    mut captured: CapturedMemberUnion,
) -> Result<(), BackgroundProcessError> {
    // Capture the complete post-KILL membership once. The retained, unreaped
    // leader keeps the original numeric group identity from being reused while
    // descriptor-relative procfs (Linux) or `ps` (macOS) supplies the bounded
    // capture. Subsequent waits inspect only those captured PIDs. One final
    // global scan proves that no member raced into or escaped the capture.
    let deadline = Instant::now() + GROUP_DISAPPEARANCE_GRACE;
    thread::sleep(GROUP_SNAPSHOT_INITIAL_INTERVAL);
    let mut observed_failure = false;
    match group_members(authority, group) {
        Ok(members) => {
            if captured.retain(members).is_err() {
                observed_failure = true;
            }
        }
        Err(_) => observed_failure = true,
    }
    let mut captured = RetainedMemberWait::new(captured.into_members(), group);
    let mut observation = ObservationBackoff::with_bounds(
        GROUP_SNAPSHOT_INITIAL_INTERVAL,
        GROUP_SNAPSHOT_MAX_INTERVAL,
    );
    #[cfg(target_os = "linux")]
    let mut stat_bytes = Vec::with_capacity(MAX_LINUX_PROC_STAT_BYTES + 1);
    loop {
        match observe_leader(group) {
            Err(LeaderObservationFailure::LostAuthority) => return Err(wait_error()),
            Err(LeaderObservationFailure::Operation(_)) => observed_failure = true,
            Ok(_) => {}
        }
        #[cfg(target_os = "macos")]
        let captured_poll = captured.poll(|member| captured_group_member_exists(authority, member));
        #[cfg(target_os = "linux")]
        let captured_poll = captured
            .poll(|member| captured_group_member_exists(authority, member, &mut stat_bytes));
        match captured_poll {
            Ok(true) => break,
            Ok(false) => {}
            Err(_) => {
                observed_failure = true;
                sleep_through(deadline.saturating_duration_since(Instant::now()));
                break;
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        observation.sleep_and_advance();
    }
    let members = group_members(authority, group)?;
    if observed_failure || !only_group_leader_remains(&members, group) {
        Err(cleanup_error())
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct CapturedGroupMember {
    pid: rustix::process::Pid,
    identity: Option<u64>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct CapturedMemberUnion {
    members: Vec<CapturedGroupMember>,
    index: HashSet<CapturedGroupMember>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CapturedMemberUnion {
    fn new() -> Self {
        Self {
            members: Vec::new(),
            index: HashSet::new(),
        }
    }

    fn retain(&mut self, observed: Vec<CapturedGroupMember>) -> Result<(), BackgroundProcessError> {
        for member in observed {
            if self.index.contains(&member) {
                continue;
            }
            if self.members.len() == MAX_CAPTURED_GROUP_MEMBERS {
                return Err(cleanup_error());
            }
            self.index.insert(member);
            self.members.push(member);
        }
        Ok(())
    }

    fn iter(&self) -> impl Iterator<Item = &CapturedGroupMember> {
        self.members.iter()
    }

    fn into_members(self) -> Vec<CapturedGroupMember> {
        self.members
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct RetainedMemberWait {
    unresolved: Vec<CapturedGroupMember>,
    cursor: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl RetainedMemberWait {
    fn new(mut captured: Vec<CapturedGroupMember>, leader: rustix::process::Pid) -> Self {
        captured.retain(|member| member.pid != leader);
        Self {
            unresolved: captured,
            cursor: 0,
        }
    }

    fn poll(
        &mut self,
        mut exists: impl FnMut(&CapturedGroupMember) -> Result<bool, BackgroundProcessError>,
    ) -> Result<bool, BackgroundProcessError> {
        while self.cursor < self.unresolved.len() {
            if exists(&self.unresolved[self.cursor])? {
                return Ok(false);
            }
            self.cursor += 1;
        }
        Ok(true)
    }
}

#[cfg(target_os = "macos")]
fn captured_group_member_exists(
    _authority: &GroupSnapshotAuthority,
    member: &CapturedGroupMember,
) -> Result<bool, BackgroundProcessError> {
    match rustix::process::getpgid(Some(member.pid)) {
        Ok(_) => Ok(true),
        Err(rustix::io::Errno::SRCH) => Ok(false),
        Err(_) => Err(cleanup_error()),
    }
}

#[cfg(target_os = "macos")]
fn group_members(
    _authority: &GroupSnapshotAuthority,
    group: rustix::process::Pid,
) -> Result<Vec<CapturedGroupMember>, BackgroundProcessError> {
    use std::io::Read;

    #[cfg(test)]
    record_group_snapshot_for_test(group);

    #[cfg(test)]
    if consume_injected_failure(
        &GROUP_SNAPSHOT_SPAWN_FAILURE_GROUP,
        &GROUP_SNAPSHOT_SPAWN_FAILURES,
        group.as_raw_nonzero().get().cast_unsigned(),
    ) {
        return Err(cleanup_error());
    }
    let mut command = Command::new("/bin/ps");
    command.args(["-o", "pid=", "-g"]);
    command
        .arg(group.as_raw_nonzero().get().to_string())
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut reap_permit = Some(reserve_child_reap_authority().map_err(|_| cleanup_error())?);
    let mut child = Some(command.spawn().map_err(|_| cleanup_error())?);
    let output = child
        .as_mut()
        .and_then(|child| child.stdout.take())
        .ok_or_else(cleanup_error)?;
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
        terminate_and_reap_or_quarantine(&mut child, &mut reap_permit);
        return Err(cleanup_error());
    };
    let deadline = Instant::now() + GROUP_SNAPSHOT_TIMEOUT;
    let status = loop {
        match child.as_mut().ok_or_else(cleanup_error)?.try_wait() {
            Ok(Some(status)) => {
                discharge_reaped_child(&mut child, &mut reap_permit);
                break Ok(status);
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(OBSERVATION_INITIAL_INTERVAL);
            }
            Ok(None) | Err(_) => {
                terminate_and_reap_or_quarantine(&mut child, &mut reap_permit);
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
    parse_group_members(&bytes).map(|members| {
        members
            .into_iter()
            .map(|pid| CapturedGroupMember {
                pid,
                identity: None,
            })
            .collect()
    })
}

#[cfg(target_os = "linux")]
fn group_members(
    authority: &GroupSnapshotAuthority,
    group: rustix::process::Pid,
) -> Result<Vec<CapturedGroupMember>, BackgroundProcessError> {
    use std::io::Read;

    #[cfg(test)]
    record_group_snapshot_for_test(group);

    #[cfg(test)]
    if consume_injected_failure(
        &GROUP_SNAPSHOT_SPAWN_FAILURE_GROUP,
        &GROUP_SNAPSHOT_SPAWN_FAILURES,
        group.as_raw_nonzero().get().cast_unsigned(),
    ) {
        return Err(cleanup_error());
    }

    authority.validate().map_err(|()| cleanup_error())?;
    let scan_fd = rustix::fs::openat(
        authority.proc_root.as_fd(),
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| cleanup_error())?;
    let mut directory_buffer = [MaybeUninit::<u8>::uninit(); 8 * 1024];
    let mut entries = rustix::fs::RawDir::new(scan_fd, &mut directory_buffer);
    let mut inspected_entries = 0_usize;
    let mut inspected_bytes = 0_usize;
    let mut members = Vec::new();
    let mut stat_bytes = Vec::with_capacity(MAX_LINUX_PROC_STAT_BYTES + 1);
    while let Some(entry) = entries.next() {
        inspected_entries = inspected_entries
            .checked_add(1)
            .filter(|count| *count <= MAX_LINUX_PROC_ENTRIES)
            .ok_or_else(cleanup_error)?;
        let entry = entry.map_err(|_| cleanup_error())?;
        let name = entry.file_name();
        let Some(pid) = parse_linux_proc_directory_pid(name.to_bytes())? else {
            continue;
        };
        let pid_fd = match rustix::fs::openat(
            authority.proc_root.as_fd(),
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => continue,
            Err(_) => return Err(cleanup_error()),
        };
        let stat_fd = match rustix::fs::openat(
            pid_fd.as_fd(),
            "stat",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => continue,
            Err(_) => return Err(cleanup_error()),
        };
        let mut stat = std::fs::File::from(stat_fd);
        stat_bytes.clear();
        stat.by_ref()
            .take((MAX_LINUX_PROC_STAT_BYTES + 1) as u64)
            .read_to_end(&mut stat_bytes)
            .map_err(|_| cleanup_error())?;
        if stat_bytes.len() > MAX_LINUX_PROC_STAT_BYTES {
            return Err(cleanup_error());
        }
        inspected_bytes = inspected_bytes
            .checked_add(stat_bytes.len())
            .filter(|bytes| *bytes <= MAX_LINUX_PROC_SNAPSHOT_BYTES)
            .ok_or_else(cleanup_error)?;
        let parsed = parse_linux_proc_stat(&stat_bytes, pid)?;
        if parsed.group == Some(group) {
            if members.len() == MAX_CAPTURED_GROUP_MEMBERS {
                return Err(cleanup_error());
            }
            members.push(CapturedGroupMember {
                pid,
                identity: Some(parsed.start_time),
            });
        }
    }

    // A proof is accepted only if the same retained procfs authority still
    // has the admitted identity, options, and mount topology after the scan.
    authority.validate().map_err(|()| cleanup_error())?;

    #[cfg(test)]
    if consume_injected_failure(
        &GROUP_SNAPSHOT_FAILURE_GROUP,
        &GROUP_SNAPSHOT_FAILURES,
        group.as_raw_nonzero().get().cast_unsigned(),
    ) {
        return Err(cleanup_error());
    }
    Ok(members)
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_directory_pid(
    name: &[u8],
) -> Result<Option<rustix::process::Pid>, BackgroundProcessError> {
    if !name.first().is_some_and(u8::is_ascii_digit) {
        return Ok(None);
    }
    parse_linux_positive_i32(name)
        .and_then(rustix::process::Pid::from_raw)
        .map(Some)
        .ok_or_else(cleanup_error)
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_stat(
    bytes: &[u8],
    expected_pid: rustix::process::Pid,
) -> Result<LinuxProcStat, BackgroundProcessError> {
    if bytes.is_empty() || bytes.len() > MAX_LINUX_PROC_STAT_BYTES {
        return Err(cleanup_error());
    }
    let open = bytes
        .iter()
        .position(|byte| *byte == b'(')
        .ok_or_else(cleanup_error)?;
    let close = bytes
        .iter()
        .rposition(|byte| *byte == b')')
        .filter(|close| *close > open)
        .ok_or_else(cleanup_error)?;
    let pid = bytes[..open]
        .strip_suffix(b" ")
        .and_then(parse_linux_positive_i32)
        .and_then(rustix::process::Pid::from_raw)
        .ok_or_else(cleanup_error)?;
    if pid != expected_pid
        || !bytes[close + 1..]
            .first()
            .is_some_and(u8::is_ascii_whitespace)
    {
        return Err(cleanup_error());
    }
    let mut fields = bytes[close + 1..]
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty());
    let state = fields.next().ok_or_else(cleanup_error)?;
    if state.len() != 1 {
        return Err(cleanup_error());
    }
    parse_linux_nonnegative_i32(fields.next().ok_or_else(cleanup_error)?)
        .ok_or_else(cleanup_error)?;
    let group = fields
        .next()
        .and_then(parse_linux_nonnegative_i32)
        .ok_or_else(cleanup_error)?;
    for _ in 0..16 {
        fields.next().ok_or_else(cleanup_error)?;
    }
    let start_time = fields
        .next()
        .and_then(parse_linux_nonnegative_u64)
        .ok_or_else(cleanup_error)?;
    Ok(LinuxProcStat {
        group: rustix::process::Pid::from_raw(group),
        start_time,
    })
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LinuxProcStat {
    group: Option<rustix::process::Pid>,
    start_time: u64,
}

#[cfg(target_os = "linux")]
fn captured_group_member_exists(
    authority: &GroupSnapshotAuthority,
    member: &CapturedGroupMember,
    bytes: &mut Vec<u8>,
) -> Result<bool, BackgroundProcessError> {
    use std::io::Read;

    let name = member.pid.as_raw_nonzero().get().to_string();
    let pid_fd = match rustix::fs::openat(
        authority.proc_root.as_fd(),
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(false),
        Err(_) => return Err(cleanup_error()),
    };
    let stat_fd = match rustix::fs::openat(
        pid_fd.as_fd(),
        "stat",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(false),
        Err(_) => return Err(cleanup_error()),
    };
    bytes.clear();
    std::fs::File::from(stat_fd)
        .take((MAX_LINUX_PROC_STAT_BYTES + 1) as u64)
        .read_to_end(bytes)
        .map_err(|_| cleanup_error())?;
    if bytes.len() > MAX_LINUX_PROC_STAT_BYTES {
        return Err(cleanup_error());
    }
    let parsed = parse_linux_proc_stat(bytes, member.pid)?;
    Ok(member.identity == Some(parsed.start_time))
}

#[cfg(target_os = "linux")]
fn parse_linux_positive_i32(bytes: &[u8]) -> Option<i32> {
    let value = parse_linux_nonnegative_i32(bytes)?;
    (value > 0).then_some(value)
}

#[cfg(target_os = "linux")]
fn parse_linux_nonnegative_i32(bytes: &[u8]) -> Option<i32> {
    if bytes.is_empty() {
        return None;
    }
    bytes.iter().try_fold(0_i32, |value, byte| {
        let digit = byte.checked_sub(b'0').filter(|digit| *digit < 10)?;
        value.checked_mul(10)?.checked_add(i32::from(digit))
    })
}

#[cfg(target_os = "linux")]
fn parse_linux_positive_u64(bytes: &[u8]) -> Option<u64> {
    let value = parse_linux_nonnegative_u64(bytes)?;
    (value > 0).then_some(value)
}

#[cfg(target_os = "linux")]
fn parse_linux_nonnegative_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    bytes.iter().try_fold(0_u64, |value, byte| {
        let digit = byte.checked_sub(b'0').filter(|digit| *digit < 10)?;
        value.checked_mul(10)?.checked_add(u64::from(digit))
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn parse_group_members(bytes: &[u8]) -> Result<Vec<rustix::process::Pid>, BackgroundProcessError> {
    let text = std::str::from_utf8(bytes).map_err(|_| cleanup_error())?;
    let mut members = Vec::new();
    for field in text.split_ascii_whitespace() {
        if members.len() == MAX_CAPTURED_GROUP_MEMBERS {
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
    members: &[CapturedGroupMember],
    leader: rustix::process::Pid,
) -> bool {
    members.len() == 1 && members[0].pid == leader
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
fn record_group_snapshot_for_test(group: rustix::process::Pid) {
    if OBSERVED_GROUP_SNAPSHOT.load(Ordering::Acquire)
        == group.as_raw_nonzero().get().cast_unsigned()
    {
        GROUP_SNAPSHOT_ATTEMPTS.fetch_add(1, Ordering::AcqRel);
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn reset_group_snapshots_for_test(group: NonZeroU32) {
    OBSERVED_GROUP_SNAPSHOT.store(group.get(), Ordering::Release);
    GROUP_SNAPSHOT_ATTEMPTS.store(0, Ordering::Release);
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn group_snapshots_for_test() -> usize {
    GROUP_SNAPSHOT_ATTEMPTS.load(Ordering::Acquire)
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn inject_group_signal_eperm_for_test(group: NonZeroU32, count: usize) {
    inject_failures(
        &GROUP_SIGNAL_EPERM_GROUP,
        &GROUP_SIGNAL_EPERM_REMAINING,
        group,
        count,
    );
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn inject_waitid_failures_for_test(leader: NonZeroU32, count: usize) {
    inject_failures(&WAITID_FAILURE_LEADER, &WAITID_FAILURES, leader, count);
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn cancel_release_before_commit_for_test(pid: NonZeroU32) {
    CANCEL_RELEASE_BEFORE_COMMIT_PID.store(pid.get(), Ordering::Release);
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn clear_release_before_commit_cancellation_for_test() {
    CANCEL_RELEASE_BEFORE_COMMIT_PID.store(0, Ordering::Release);
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
    parse_group_members(bytes).map(|members| members == [leader])
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

#[cfg(all(test, target_os = "linux"))]
mod linux_proc_tests {
    use super::*;

    const UNRESTRICTED_PROC: &[u8] =
        b"36 25 0:32 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw,hidepid=0\n";

    fn pid(raw: i32) -> rustix::process::Pid {
        rustix::process::Pid::from_raw(raw).expect("positive test pid")
    }

    #[test]
    fn proc_stat_uses_the_final_parenthesis_after_a_hostile_comm() {
        let stat =
            b"321 (worker ) name (with parentheses)) S 12 77 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 4242";

        assert_eq!(
            parse_linux_proc_stat(stat, pid(321)).unwrap(),
            LinuxProcStat {
                group: Some(pid(77)),
                start_time: 4242,
            }
        );
    }

    #[test]
    fn proc_stat_rejects_pid_mismatch_and_ambiguous_fields() {
        assert!(parse_linux_proc_stat(b"322 (worker) S 12 77 77", pid(321)).is_err());
        assert!(parse_linux_proc_stat(b"321 (worker) S 12", pid(321)).is_err());
        assert!(parse_linux_proc_stat(b"321 (worker) S 1x 77", pid(321)).is_err());
        assert!(parse_linux_proc_stat(b"321 (worker) S 12 +77", pid(321)).is_err());
        assert!(parse_linux_proc_stat(b"321 (worker) SS 12 77", pid(321)).is_err());
    }

    #[test]
    fn proc_stat_accepts_kernel_processes_without_a_process_group() {
        assert_eq!(
            parse_linux_proc_stat(
                b"2 (kthreadd) S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 99",
                pid(2)
            )
            .unwrap(),
            LinuxProcStat {
                group: None,
                start_time: 99,
            }
        );
    }

    #[test]
    fn proc_stat_enforces_its_record_bound() {
        let oversized = vec![b'x'; MAX_LINUX_PROC_STAT_BYTES + 1];

        assert!(parse_linux_proc_stat(&oversized, pid(321)).is_err());
    }

    #[test]
    fn proc_directory_pid_parser_ignores_kernel_names_and_rejects_numeric_ambiguity() {
        assert_eq!(parse_linux_proc_directory_pid(b"self").unwrap(), None);
        assert_eq!(
            parse_linux_proc_directory_pid(b"321").unwrap(),
            Some(pid(321))
        );
        assert!(parse_linux_proc_directory_pid(b"321x").is_err());
        assert!(parse_linux_proc_directory_pid(b"0").is_err());
        assert!(parse_linux_proc_directory_pid(b"999999999999999999999").is_err());
    }

    #[test]
    fn proc_mount_authority_requires_one_unrestricted_root_procfs() {
        assert!(parse_linux_proc_mount_authority(UNRESTRICTED_PROC, 36).is_ok());
        assert!(
            parse_linux_proc_mount_authority(b"36 25 0:32 / /proc rw,nosuid - proc proc rw\n", 36,)
                .is_ok()
        );
        assert!(parse_linux_proc_mount_authority(UNRESTRICTED_PROC, 37).is_err());

        for restricted in [
            b"36 25 0:32 / /proc rw - proc proc rw,hidepid=1\n".as_slice(),
            b"36 25 0:32 / /proc rw,hidepid=2 - proc proc rw\n".as_slice(),
            b"36 25 0:32 / /proc rw - proc proc rw,hidepid=invisible\n".as_slice(),
            b"36 25 0:32 / /proc rw - proc proc rw,hidepid=unknown\n".as_slice(),
            b"36 25 0:32 /subtree /proc rw - proc proc rw\n".as_slice(),
            b"36 25 0:32 / /proc rw - tmpfs tmpfs rw\n".as_slice(),
            b"36 25 0:32 / /other rw - proc proc rw\n".as_slice(),
            b"36 25 0:32 / /proc rw - proc proc rw".as_slice(),
        ] {
            assert!(parse_linux_proc_mount_authority(restricted, 36).is_err());
        }
    }

    #[test]
    fn proc_mount_authority_rejects_numeric_pid_path_overmounts_and_ambiguity() {
        let numeric_overmount = [
            UNRESTRICTED_PROC,
            b"37 36 0:33 / /proc/321 rw - tmpfs tmpfs rw\n",
        ]
        .concat();
        assert!(parse_linux_proc_mount_authority(&numeric_overmount, 36).is_err());

        let mountinfo_overmount = [
            UNRESTRICTED_PROC,
            b"37 36 0:33 / /proc/self/mountinfo rw - tmpfs tmpfs rw\n",
        ]
        .concat();
        assert!(parse_linux_proc_mount_authority(&mountinfo_overmount, 36).is_err());

        let duplicate = [UNRESTRICTED_PROC, UNRESTRICTED_PROC].concat();
        assert!(parse_linux_proc_mount_authority(&duplicate, 36).is_err());
        assert!(linux_numeric_proc_mountpoint(b"/proc/321/stat"));
        assert!(!linux_numeric_proc_mountpoint(b"/proc/self"));
        assert!(!linux_numeric_proc_mountpoint(b"/proc/321x"));
        assert!(linux_proc_authority_overmount(b"/proc/self/mountinfo"));
    }

    #[test]
    fn host_proc_mount_satisfies_pre_release_authority_contract() {
        GroupSnapshotAuthority::open().expect("test host must expose unrestricted procfs");
    }

    #[test]
    fn proc_mount_authority_rejects_post_admission_topology_change() {
        assert!(parse_linux_proc_mount_authority(UNRESTRICTED_PROC, 36).is_ok());
        let changed = [
            UNRESTRICTED_PROC,
            b"37 36 0:33 / /proc/999/stat rw - tmpfs tmpfs rw\n",
        ]
        .concat();
        assert!(parse_linux_proc_mount_authority(&changed, 36).is_err());
    }

    #[test]
    fn proc_mount_authority_enforces_incremental_input_bounds() {
        let truncated = b"36 25 0:32 / /proc rw - proc proc rw";
        assert!(parse_linux_proc_mount_authority(truncated, 36).is_err());

        let oversized = vec![b'x'; MAX_LINUX_MOUNTINFO_BYTES + 1];
        assert!(parse_linux_proc_mount_authority(&oversized, 36).is_err());
    }
}

#[cfg(test)]
mod process_regression_tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "machine-god-background-process-unit-{}-{sequence}-{label}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create isolated process test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn environment_duplicate_validation_handles_exact_shared_prefix_bound() {
        let directory = TestDirectory::new("environment-prefix");
        let prefix = "k".repeat(508);
        let environment = (0..MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES)
            .map(|index| {
                (
                    OsString::from(format!("{prefix}{index:03}")),
                    OsString::new(),
                )
            })
            .collect::<Vec<_>>();
        BackgroundProcessRequest::open(
            "true".to_owned(),
            "workspace".to_owned(),
            environment.clone(),
            &directory.0,
        )
        .expect("the exact shared-prefix bound is valid");

        let mut duplicate = environment;
        duplicate[MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES - 1].0 = duplicate[0].0.clone();
        let error = BackgroundProcessRequest::open(
            "true".to_owned(),
            "workspace".to_owned(),
            duplicate,
            &directory.0,
        )
        .expect_err("a duplicate at the exact bound is rejected");
        assert_eq!(error.kind(), BackgroundProcessErrorKind::InvalidRequest);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn stalled_child_reap_probe_is_cancelled_and_reaped_promptly() {
        let directory = TestDirectory::new("cancel-reap-probe");
        let pid_file = directory.0.join("probe.pid");
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("printf '%s' \"$$\" > \"$1\"; exec /bin/sleep 30")
            .arg("machine-god-stalled-reap-probe")
            .arg(&pid_file)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let worker = thread::spawn(move || {
            require_exclusive_child_reaping_with(command, &worker_cancellation)
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !pid_file.exists() {
            assert!(Instant::now() < deadline, "probe did not publish its PID");
            thread::sleep(Duration::from_millis(2));
        }
        let pid = fs::read_to_string(&pid_file)
            .expect("read probe PID")
            .parse::<i32>()
            .expect("parse probe PID");

        let started = Instant::now();
        assert!(cancellation.cancel());
        assert_eq!(
            worker
                .join()
                .expect("join cancelled probe")
                .expect_err("cancelled probe fails")
                .kind(),
            BackgroundProcessErrorKind::Spawn
        );
        assert!(started.elapsed() < Duration::from_millis(250));
        assert_eq!(
            rustix::process::test_kill_process(
                rustix::process::Pid::from_raw(pid).expect("positive probe PID")
            ),
            Err(rustix::io::Errno::SRCH)
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn ready_helper_not_reading_maximum_frame_is_bounded_and_reaped() {
        let directory = TestDirectory::new("stalled-release-frame");
        let helper_pid = directory.0.join("helper.pid");
        let prepare = || {
            let helper = BackgroundProcessHelper::new(
                PathBuf::from("/bin/sh"),
                vec![
                    OsString::from("-c"),
                    OsString::from(
                        "printf '%s' \"$$\" > \"$1\"; printf '\\247' >&2; exec /bin/sleep 30",
                    ),
                    OsString::from("machine-god-stalled-release-helper"),
                    helper_pid.as_os_str().to_owned(),
                ],
            )
            .expect("bounded stalled release helper");
            let environment = (0..15)
                .map(|index| {
                    (
                        OsString::from(format!("K{index}")),
                        OsString::from("v".repeat(MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES)),
                    )
                })
                .collect();
            let request = BackgroundProcessRequest::open(
                "x".repeat(MAX_BACKGROUND_PROCESS_COMMAND_BYTES),
                "workspace".to_owned(),
                environment,
                &directory.0,
            )
            .expect("maximum stalled frame request");
            SystemBackgroundProcessAdapter::with_helper(helper)
                .prepare(request)
                .expect("helper becomes ready")
        };

        let prepared = prepare();
        let cancelled_pid = prepared.pid();
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let worker = thread::spawn(move || prepared.release_cancellable(&worker_cancellation));
        thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        assert!(cancellation.cancel());
        assert_eq!(
            worker
                .join()
                .expect("join cancelled release")
                .expect_err("cancelled release fails")
                .kind(),
            BackgroundProcessErrorKind::Cancelled
        );
        assert!(started.elapsed() < Duration::from_millis(250));
        assert_eq!(
            rustix::process::test_kill_process(
                rustix::process::Pid::from_raw(
                    i32::try_from(cancelled_pid.get()).expect("helper PID fits signed range")
                )
                .expect("positive helper PID")
            ),
            Err(rustix::io::Errno::SRCH)
        );

        let prepared = prepare();
        let pid = prepared.pid();

        let started = Instant::now();
        let error = prepared
            .release_cancellable(&CancellationToken::new())
            .expect_err("non-reading helper cannot consume the complete frame");
        assert_eq!(error.kind(), BackgroundProcessErrorKind::Release);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(
            rustix::process::test_kill_process(
                rustix::process::Pid::from_raw(
                    i32::try_from(pid.get()).expect("helper PID fits signed range")
                )
                .expect("positive helper PID")
            ),
            Err(rustix::io::Errno::SRCH)
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn retained_member_wait_amortizes_vanished_prefix_and_one_live_witness() {
        let leader = rustix::process::Pid::from_raw(1).expect("positive leader");
        let trailing = rustix::process::Pid::from_raw(4095).expect("positive witness");
        let captured = (2..=4095)
            .map(|raw| CapturedGroupMember {
                pid: rustix::process::Pid::from_raw(raw).expect("positive captured PID"),
                identity: None,
            })
            .collect();
        let mut wait = RetainedMemberWait::new(captured, leader);
        let mut observations = 0_usize;
        for _ in 0..100 {
            assert!(
                !wait
                    .poll(|member| {
                        observations += 1;
                        Ok(member.pid == trailing)
                    })
                    .expect("injected observation succeeds")
            );
        }
        assert_eq!(observations, 4093 + 100);
        assert!(
            wait.poll(|_| {
                observations += 1;
                Ok(false)
            })
            .expect("final witness disappearance succeeds")
        );
        assert_eq!(observations, 4093 + 101);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn partial_signal_delivery_retains_member_that_escaped_before_later_snapshot() {
        let leader = rustix::process::Pid::from_raw(71).expect("positive leader");
        let escaped = CapturedGroupMember {
            pid: rustix::process::Pid::from_raw(72).expect("positive descendant"),
            identity: Some(9001),
        };
        let leader_member = CapturedGroupMember {
            pid: leader,
            identity: Some(9000),
        };
        let mut retained = CapturedMemberUnion::new();
        retained
            .retain(vec![leader_member, escaped])
            .expect("initial member union fits");
        // The later pre-KILL snapshot no longer contains the descendant: it
        // escaped after partial TERM delivery. The retained union must not
        // discard it merely because the original group no longer reports it.
        retained
            .retain(vec![leader_member])
            .expect("duplicate member union fits");

        let mut wait = RetainedMemberWait::new(retained.into_members(), leader);
        assert!(
            !wait
                .poll(|member| Ok(*member == escaped))
                .expect("injected escaped-member observation succeeds")
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn captured_member_union_has_one_exact_32768_member_bound() {
        let member = |raw: i32| CapturedGroupMember {
            pid: rustix::process::Pid::from_raw(raw).expect("positive test PID"),
            identity: Some(u64::try_from(raw).expect("positive identity")),
        };
        let first_half: Vec<_> = (1..=16_384).map(member).collect();
        let second_half: Vec<_> = (16_385..=32_768).map(member).collect();
        let mut captured = CapturedMemberUnion::new();
        captured
            .retain(first_half.clone())
            .expect("first disjoint half fits");
        captured
            .retain(second_half.clone())
            .expect("exact aggregate bound fits");
        let mut reversed = second_half;
        reversed.reverse();
        reversed.extend(first_half.into_iter().rev());
        captured
            .retain(reversed)
            .expect("reversed duplicates do not consume aggregate capacity");
        assert_eq!(captured.iter().count(), MAX_CAPTURED_GROUP_MEMBERS);
        assert_eq!(
            captured
                .retain(vec![member(32_769)])
                .expect_err("one disjoint churn member exceeds the exact bound")
                .kind(),
            BackgroundProcessErrorKind::Cleanup
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn retained_stable_identity_detects_a_member_that_escaped_the_group() {
        use std::io::Read;

        let authority = GroupSnapshotAuthority::open().expect("open proc authority");
        let raw = i32::try_from(std::process::id()).expect("current PID fits signed range");
        let pid = rustix::process::Pid::from_raw(raw).expect("positive current PID");
        let stat_fd = rustix::fs::openat(
            authority.proc_root.as_fd(),
            raw.to_string(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .and_then(|pid_fd| {
            rustix::fs::openat(
                pid_fd.as_fd(),
                "stat",
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
        })
        .expect("open current stat");
        let mut bytes = Vec::new();
        std::fs::File::from(stat_fd)
            .read_to_end(&mut bytes)
            .expect("read current stat");
        let stat = parse_linux_proc_stat(&bytes, pid).expect("parse current identity");
        let escaped = CapturedGroupMember {
            pid,
            identity: Some(stat.start_time),
        };
        let mut observation_bytes = Vec::with_capacity(MAX_LINUX_PROC_STAT_BYTES + 1);
        assert!(
            captured_group_member_exists(&authority, &escaped, &mut observation_bytes)
                .expect("escaped member observation succeeds")
        );
        assert_ne!(
            stat.group,
            Some(rustix::process::Pid::from_raw(1).expect("positive comparison group"))
        );

        let reused = CapturedGroupMember {
            identity: Some(stat.start_time.wrapping_add(1)),
            ..escaped
        };
        assert!(
            !captured_group_member_exists(&authority, &reused, &mut observation_bytes)
                .expect("PID reuse observation succeeds")
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn cancelled_helper_readiness_wakes_and_reaps_before_timeout() {
        let directory = TestDirectory::new("cancel-readiness");
        let helper_pid = directory.0.join("helper.pid");
        let user_marker = directory.0.join("user-ran");
        let helper = BackgroundProcessHelper::new(
            PathBuf::from("/bin/sh"),
            vec![
                OsString::from("-c"),
                OsString::from(
                    "printf '%s' \"$$\" > \"$1.tmp\"; /bin/mv \"$1.tmp\" \"$1\"; exec /bin/sleep 30",
                ),
                OsString::from("machine-god-stalled-helper"),
                helper_pid.as_os_str().to_owned(),
            ],
        )
        .expect("bounded stalled helper");
        let adapter = SystemBackgroundProcessAdapter::with_helper(helper);
        let request = BackgroundProcessRequest::open(
            "printf bad > user-ran".to_owned(),
            "workspace".to_owned(),
            Vec::new(),
            &directory.0,
        )
        .expect("bounded process request");
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let worker =
            thread::spawn(move || adapter.prepare_cancellable(request, &worker_cancellation));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !helper_pid.exists() {
            assert!(
                Instant::now() < deadline,
                "helper did not reach stalled readiness"
            );
            thread::sleep(Duration::from_millis(2));
        }
        let pid = fs::read_to_string(&helper_pid)
            .expect("read helper PID")
            .parse::<i32>()
            .expect("parse helper PID");

        let started = Instant::now();
        assert!(cancellation.cancel());
        let error = worker
            .join()
            .expect("join cancelled helper preparation")
            .expect_err("cancelled helper preparation fails");
        assert_eq!(error.kind(), BackgroundProcessErrorKind::Spawn);
        assert!(
            started.elapsed() < Duration::from_millis(750),
            "cancellation retained helper readiness capacity until its timeout"
        );
        let pid = rustix::process::Pid::from_raw(pid).expect("positive helper PID");
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH),
            "cancelled helper survived cleanup"
        );
        assert!(!user_marker.exists(), "the gated user command ran");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn eperm_reuses_phase_snapshot_and_stays_within_four_global_scans() {
        let directory = TestDirectory::new("eperm-phase-snapshot");
        let mut reap_permit =
            Some(reserve_child_reap_authority().expect("reserve test child reap authority"));
        let mut child = Some(
            Command::new("/bin/sh")
                .args(["-c", "exit 7"])
                .current_dir(&directory.0)
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .expect("spawn exited test process group"),
        );
        let leader = NonZeroU32::new(child.as_ref().expect("test child retained").id())
            .expect("positive process-group leader");
        let group = rustix::process::Pid::from_raw(
            i32::try_from(leader.get()).expect("test PID fits signed range"),
        )
        .expect("positive process group");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(observe_leader(group), Ok(Some(_))) {
            assert!(Instant::now() < deadline, "leader did not exit");
            thread::sleep(Duration::from_millis(2));
        }
        #[cfg(target_os = "linux")]
        let authority = GroupSnapshotAuthority::open().expect("open group snapshot authority");
        #[cfg(target_os = "macos")]
        let authority = GroupSnapshotAuthority;
        reset_group_snapshots_for_test(leader);
        inject_group_signal_eperm_for_test(leader, 2);

        cleanup_child_with_expected(
            &mut child,
            &mut reap_permit,
            group,
            Duration::ZERO,
            Some(BackgroundProcessExit::Exited(7)),
            true,
            &authority,
        )
        .expect("phase evidence accepts EPERM for an exited sole leader");

        assert_eq!(
            group_snapshots_for_test(),
            4,
            "EPERM must reuse TERM and KILL phase snapshots"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn lingering_group_cleanup_uses_constant_global_snapshots() {
        let directory = TestDirectory::new("snapshot-count");
        let marker = directory.0.join("descendant.pid");
        let command = "trap '' TERM; /bin/sh -c 'trap \"\" TERM; printf \"%s\" \"$$\" > descendant.pid; while :; do :; done' & while [ ! -s descendant.pid ]; do :; done; exit 7";
        let mut reap_permit =
            Some(reserve_child_reap_authority().expect("reserve test child reap authority"));
        let mut child = Some(
            Command::new("/bin/sh")
                .args(["-c", command])
                .current_dir(&directory.0)
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .expect("spawn test process group"),
        );
        let leader = NonZeroU32::new(child.as_ref().expect("test child retained").id())
            .expect("positive process-group leader");
        let group = rustix::process::Pid::from_raw(
            i32::try_from(leader.get()).expect("test PID fits signed range"),
        )
        .expect("positive process group");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() {
            assert!(Instant::now() < deadline, "descendant did not become ready");
            thread::sleep(Duration::from_millis(2));
        }
        #[cfg(target_os = "linux")]
        let authority = GroupSnapshotAuthority::open().expect("open group snapshot authority");
        #[cfg(target_os = "macos")]
        let authority = GroupSnapshotAuthority;
        reset_group_snapshots_for_test(leader);

        cleanup_child_with_expected(
            &mut child,
            &mut reap_permit,
            group,
            Duration::ZERO,
            Some(BackgroundProcessExit::Exited(7)),
            false,
            &authority,
        )
        .expect("clean lingering original-group descendant");

        assert_eq!(
            group_snapshots_for_test(),
            4,
            "cleanup must use TERM, KILL, post-KILL, and final global proofs only"
        );
    }
}
