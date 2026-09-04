//! Prepared, process-group-owned background shell execution.

#![allow(
    dead_code,
    reason = "lower-level process lifecycle primitives remain directly integration-tested"
)]

#[cfg(unix)]
use std::collections::BTreeSet;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::HashSet;
use std::error::Error;
#[cfg(target_os = "linux")]
use std::ffi::OsStr;
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
use std::sync::Arc;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
use std::sync::atomic::{AtomicI32, AtomicU32};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::{Condvar, Mutex, OnceLock};
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
const RETRY_INITIAL_INTERVAL: Duration = Duration::from_millis(4);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RETRY_MAX_INTERVAL: Duration = Duration::from_millis(32);
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
static GROUP_SNAPSHOT_TEST_LOCK: Mutex<()> = Mutex::new(());
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static CANCEL_RELEASE_BEFORE_COMMIT_PID: AtomicU32 = AtomicU32::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static RELEASE_FAILURE_WITH_CANCELLATION_PID: AtomicU32 = AtomicU32::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static TRY_WAIT_FAILURE_PID: AtomicU32 = AtomicU32::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static TRY_WAIT_FAILURES: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
static TRY_WAIT_ERRNO: AtomicI32 = AtomicI32::new(0);
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
const BACKGROUND_OUTPUT_READ_BYTES: usize = 16 * 1024;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BACKGROUND_OUTPUT_READS_PER_OBSERVATION: usize = 16;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const BACKGROUND_OUTPUT_FINAL_READS: usize = 128;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RELEASE_FRAME_MAGIC: &[u8; 8] = b"MGBG\0\0\0\x01";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RELEASE_FRAME_WRITE_CHUNK_BYTES: usize = 16 * 1024;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const RELEASE_FRAME_PIPE_WRITE_BYTES: usize = if libc::PIPE_BUF < RELEASE_FRAME_WRITE_CHUNK_BYTES {
    libc::PIPE_BUF
} else {
    RELEASE_FRAME_WRITE_CHUNK_BYTES
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_RELEASE_FRAME_PAYLOAD_BYTES: usize = RELEASE_FRAME_MAGIC.len()
    + 4
    + MAX_BACKGROUND_PROCESS_COMMAND_BYTES
    + 4
    + 8 * MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES
    + MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_RELEASE_FRAME_RETRY_ATTEMPTS: usize = 32;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_RELEASE_FRAME_WRITE_ATTEMPTS: usize = MAX_RELEASE_FRAME_PAYLOAD_BYTES
    .div_ceil(RELEASE_FRAME_PIPE_WRITE_BYTES)
    + 1
    + MAX_RELEASE_FRAME_RETRY_ATTEMPTS;
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
    /// Preparation or release was cancelled before commit and cleanup succeeded.
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
            match try_wait_child(&mut children[index].child) {
                Ok(Some(_)) | Err(ChildTryWaitError::LostAuthority) => {
                    children.swap_remove(index);
                }
                Ok(None) | Err(ChildTryWaitError::Interrupted | ChildTryWaitError::Operation) => {
                    index += 1;
                }
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

/// Immutable validated environment shared by every request from one host.
///
/// Validation and environment-frame encoding happen once at the authority
/// boundary. Clones retain the same entry and frame allocations.
#[derive(Clone)]
pub(crate) struct ValidatedBackgroundEnvironment {
    entries: Arc<[(OsString, OsString)]>,
    release_frame: Arc<Vec<u8>>,
}

impl ValidatedBackgroundEnvironment {
    #[cfg(unix)]
    pub(crate) fn new(
        environment: Vec<(OsString, OsString)>,
    ) -> Result<Self, BackgroundProcessError> {
        validate_environment(&environment)?;
        let release_frame = encode_environment_frame(&environment)?;
        Ok(Self {
            entries: environment.into(),
            release_frame: Arc::new(release_frame),
        })
    }

    pub(crate) fn entries(&self) -> &[(OsString, OsString)] {
        &self.entries
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn release_frame(&self) -> &[u8] {
        self.release_frame.as_slice()
    }

    #[cfg(all(test, unix))]
    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.entries, &other.entries)
            && Arc::ptr_eq(&self.release_frame, &other.release_frame)
    }
}

/// Exact, bounded request for one prepared background command.
pub struct BackgroundProcessRequest {
    command: String,
    cwd: String,
    environment: ValidatedBackgroundEnvironment,
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
        let environment = ValidatedBackgroundEnvironment::new(environment)?;
        Self::from_directory_with_environment(command, cwd, environment, directory)
    }

    /// Retains a directory with an already-validated shared environment.
    ///
    /// This host-only path validates the per-command fields and descriptor but
    /// deliberately reuses the environment validation and encoding result.
    #[cfg(unix)]
    pub(crate) fn from_directory_with_environment(
        command: String,
        cwd: String,
        environment: ValidatedBackgroundEnvironment,
        directory: OwnedFd,
    ) -> Result<Self, BackgroundProcessError> {
        validate_command_and_cwd(&command, &cwd)?;
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
        self.environment.entries()
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

/// Exit status reported only after bounded owned-member cleanup succeeds and
/// the retained direct child is reaped.
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

/// Native process adapter. Linux and macOS use an explicitly supplied instance
/// of this executable as a tiny gated helper. Linux starts it in the retained
/// directory; macOS gives it an inherited directory descriptor. Other targets
/// return a fixed unsupported error without spawning.
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
    /// Returns a fixed `Cancelled` failure when cancellation is observed before
    /// helper creation, or after termination and reap prove that an already
    /// created helper is gone. A readiness or spawn failure observed before a
    /// racing cancellation remains `Spawn`. Failed or ambiguous cleanup is
    /// reported as `Cleanup`, `Wait`, or `Invariant`, and an unavailable
    /// platform adapter is reported as `Unsupported`.
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
    let output = rustix::io::dup(stderr().as_fd()).map_err(|_| spawn_error())?;
    let error = Command::new(BACKGROUND_PROCESS_PROGRAM)
        .arg("-c")
        .arg(command)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output))
        .stderr(Stdio::inherit())
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
    environment: Option<ValidatedBackgroundEnvironment>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    group: rustix::process::Pid,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    snapshot_authority: Option<GroupSnapshotAuthority>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    reap_permit: Option<ChildReapPermit>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    output: Option<ChildStderr>,
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

/// Exclusive ownership of one released background process's bounded cleanup
/// set.
///
/// On Linux and macOS that set contains the retained direct child, members of
/// its original process group, and members captured by bounded cleanup
/// snapshots. A descendant that changes process group or session before any
/// snapshot observes it is outside this ownership set.
pub struct OwnedBackgroundProcess {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    child: Option<Child>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    group: rustix::process::Pid,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    snapshot_authority: Option<GroupSnapshotAuthority>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    reap_permit: Option<ChildReapPermit>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    output: Option<ChildStderr>,
    pid: NonZeroU32,
}

impl OwnedBackgroundProcess {
    /// Returns the validated, nonzero direct-child PID.
    #[must_use]
    pub const fn pid(&self) -> NonZeroU32 {
        self.pid
    }

    /// Waits for the direct child, cleans the bounded owned-member set, proves
    /// original-group quiescence with bounded process-group snapshots while
    /// the leader identity remains retained, and then reaps the leader.
    ///
    /// # Errors
    ///
    /// Returns a fixed wait, cleanup, or invariant failure.
    pub fn wait(mut self) -> Result<BackgroundProcessExit, BackgroundProcessError> {
        wait_owned(&mut self)
    }

    /// Waits while observing a cooperative stop token. Cancellation performs
    /// the same bounded signal, identity-snapshot, and reap protocol as
    /// [`Self::stop`] and returns a distinct stopped outcome.
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

    /// Waits while forwarding bounded chunks from the merged standard-output
    /// and standard-error stream to an explicitly supplied synchronous sink.
    ///
    /// The sink is invoked only by the calling thread. The process pipe remains
    /// nonblocking, is drained under a fixed per-observation syscall budget,
    /// and is drained once more after terminal cleanup. The helper's private
    /// readiness byte is consumed during preparation and is never forwarded.
    ///
    /// # Errors
    ///
    /// Returns a fixed wait, cleanup, or invariant failure.
    pub fn wait_with_stop_and_output(
        mut self,
        stop: &CancellationToken,
        mut output: impl FnMut(&[u8]),
    ) -> Result<BackgroundProcessOutcome, BackgroundProcessError> {
        wait_owned_with_stop_and_output(&mut self, stop, &mut output)
    }

    /// Sends TERM to the original group, waits the fixed grace, sends KILL,
    /// resolves every identity-captured member, proves original-group
    /// quiescence with bounded snapshots, and reaps the retained direct child.
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
    validate_command_and_cwd(command, cwd)?;
    validate_environment(environment)
}

#[cfg(unix)]
fn validate_command_and_cwd(command: &str, cwd: &str) -> Result<(), BackgroundProcessError> {
    if command.is_empty()
        || command.len() > MAX_BACKGROUND_PROCESS_COMMAND_BYTES
        || command.contains('\0')
        || cwd.is_empty()
        || cwd.len() > MAX_BACKGROUND_PROCESS_CWD_BYTES
        || cwd.contains('\0')
    {
        return Err(invalid_request());
    }
    Ok(())
}

#[cfg(unix)]
fn validate_environment(
    environment: &[(OsString, OsString)],
) -> Result<(), BackgroundProcessError> {
    if environment.len() > MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES {
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
fn encode_environment_frame(
    environment: &[(OsString, OsString)],
) -> Result<Vec<u8>, BackgroundProcessError> {
    let framing_bytes = environment
        .len()
        .checked_mul(8)
        .and_then(|bytes| bytes.checked_add(4))
        .ok_or_else(invalid_request)?;
    let payload_bytes = environment
        .iter()
        .try_fold(0_usize, |total, (key, value)| {
            total
                .checked_add(key.as_os_str().as_bytes().len())
                .and_then(|bytes| bytes.checked_add(value.as_os_str().as_bytes().len()))
                .ok_or_else(invalid_request)
        })?;
    let mut encoded = Vec::with_capacity(
        framing_bytes
            .checked_add(payload_bytes)
            .ok_or_else(invalid_request)?,
    );
    push_frame_length(&mut encoded, environment.len())?;
    for (key, value) in environment {
        let key = key.as_os_str().as_bytes();
        let value = value.as_os_str().as_bytes();
        push_frame_length(&mut encoded, key.len())?;
        push_frame_length(&mut encoded, value.len())?;
        encoded.extend_from_slice(key);
        encoded.extend_from_slice(value);
    }
    Ok(encoded)
}

#[cfg(unix)]
fn push_frame_length(output: &mut Vec<u8>, length: usize) -> Result<(), BackgroundProcessError> {
    let length = u32::try_from(length).map_err(|_| invalid_request())?;
    output.extend_from_slice(&length.to_be_bytes());
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
    let child_handle = child.as_mut().ok_or_else(invariant_error)?;
    let outcome =
        poll_exclusive_child_reaping(&mut cancellation, deadline, || try_wait_child(child_handle));
    if outcome == ExclusiveReapingOutcome::Waitable {
        drop(child.take());
        drop(permit.take());
        return Ok(());
    }
    let cleanup_succeeded = terminate_and_reap_or_quarantine(&mut child, &mut permit);
    Err(
        if outcome == ExclusiveReapingOutcome::Cancelled && cleanup_succeeded {
            cancelled_error()
        } else {
            spawn_error()
        },
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExclusiveReapingOutcome {
    Waitable,
    Failed,
    Cancelled,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn poll_exclusive_child_reaping(
    cancellation: &mut CancellationParker,
    deadline: Instant,
    mut try_wait: impl FnMut() -> Result<Option<ExitStatus>, ChildTryWaitError>,
) -> ExclusiveReapingOutcome {
    let mut observation = ObservationBackoff::retry();
    loop {
        if cancellation.is_cancelled() {
            return ExclusiveReapingOutcome::Cancelled;
        }
        match try_wait() {
            Ok(Some(status)) if status.success() => return ExclusiveReapingOutcome::Waitable,
            Ok(None) | Err(ChildTryWaitError::Interrupted) if Instant::now() < deadline => {
                observation.park_until_and_advance(cancellation, deadline);
            }
            Ok(Some(_) | None)
            | Err(
                ChildTryWaitError::Interrupted
                | ChildTryWaitError::LostAuthority
                | ChildTryWaitError::Operation,
            ) => return ExclusiveReapingOutcome::Failed,
        }
    }
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
        return Err(cancelled_error());
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
        return Err(cancelled_error());
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
    let output = retain_helper_output(
        &mut child,
        &mut reap_permit,
        group,
        &snapshot_authority,
        cancellation,
    )?;
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
        output: Some(output),
        pid,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn retain_helper_output(
    child: &mut Option<Child>,
    reap_permit: &mut Option<ChildReapPermit>,
    group: rustix::process::Pid,
    snapshot_authority: &GroupSnapshotAuthority,
    cancellation: &CancellationToken,
) -> Result<ChildStderr, BackgroundProcessError> {
    let Some(ready) = child.as_mut().and_then(|child| child.stderr.take()) else {
        let _ = cleanup_child(
            child,
            reap_permit,
            group,
            Duration::ZERO,
            snapshot_authority,
        );
        return Err(spawn_error());
    };
    match await_helper_ready(ready, cancellation) {
        Ok(output) => Ok(output),
        Err(readiness_error) => {
            cleanup_child(
                child,
                reap_permit,
                group,
                Duration::ZERO,
                snapshot_authority,
            )?;
            Err(readiness_error)
        }
    }
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
) -> Result<ChildStderr, BackgroundProcessError> {
    let flags = rustix::fs::fcntl_getfl(&ready).map_err(|_| spawn_error())?;
    rustix::fs::fcntl_setfl(&ready, flags | OFlags::NONBLOCK).map_err(|_| spawn_error())?;
    await_helper_ready_bounded(
        &mut ready,
        cancellation,
        Instant::now() + HELPER_READY_TIMEOUT,
    )?;
    Ok(ready)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn await_helper_ready_bounded(
    ready: &mut impl std::io::Read,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), BackgroundProcessError> {
    let mut cancellation = CancellationParker::new(cancellation);
    let mut observation = ObservationBackoff::new();
    let mut byte = [0_u8; 1];
    loop {
        if cancellation.is_cancelled() {
            return Err(cancelled_error());
        }
        if Instant::now() >= deadline {
            return Err(spawn_error());
        }
        match ready.read(&mut byte) {
            Ok(1) if byte[0] == HELPER_READY_BYTE => {
                return if cancellation.is_cancelled() {
                    Err(cancelled_error())
                } else {
                    Ok(())
                };
            }
            Ok(1 | 0) => return Err(spawn_error()),
            Ok(_) => unreachable!("one-byte ready buffer has bounded reads"),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                if Instant::now() >= deadline {
                    return Err(spawn_error());
                }
                observation.park_until_and_advance(&mut cancellation, deadline);
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
        || prepared.output.is_none()
    {
        return Err(invariant_error());
    }
    let child = prepared.child.take().ok_or_else(invariant_error)?;
    let snapshot_authority = prepared
        .snapshot_authority
        .take()
        .ok_or_else(invariant_error)?;
    let reap_permit = prepared.reap_permit.take().ok_or_else(invariant_error)?;
    let output = prepared.output.take().ok_or_else(invariant_error)?;
    Ok(OwnedBackgroundProcess {
        child: Some(child),
        group: prepared.group,
        snapshot_authority: Some(snapshot_authority),
        reap_permit: Some(reap_permit),
        output: Some(output),
        pid: prepared.pid,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_release_frame(
    output: &mut impl std::io::Write,
    command: &str,
    environment: &ValidatedBackgroundEnvironment,
) -> std::io::Result<()> {
    let mut output = ReleaseFrameChunkWriter::new(output);
    output.write_bytes(RELEASE_FRAME_MAGIC)?;
    output.write_length(command.len())?;
    output.write_bytes(command.as_bytes())?;
    output.write_bytes(environment.release_frame())?;
    output.finish()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ReleaseFrameChunkWriter<'a, W: std::io::Write + ?Sized> {
    output: &'a mut W,
    buffer: [u8; RELEASE_FRAME_WRITE_CHUNK_BYTES],
    used: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'a, W: std::io::Write + ?Sized> ReleaseFrameChunkWriter<'a, W> {
    fn new(output: &'a mut W) -> Self {
        Self {
            output,
            buffer: [0; RELEASE_FRAME_WRITE_CHUNK_BYTES],
            used: 0,
        }
    }

    fn write_length(&mut self, length: usize) -> std::io::Result<()> {
        let length = u32::try_from(length).map_err(|_| std::io::ErrorKind::InvalidInput)?;
        self.write_bytes(&length.to_be_bytes())
    }

    fn write_bytes(&mut self, mut bytes: &[u8]) -> std::io::Result<()> {
        while !bytes.is_empty() {
            let available = self.buffer.len() - self.used;
            let copied = available.min(bytes.len());
            self.buffer[self.used..self.used + copied].copy_from_slice(&bytes[..copied]);
            self.used += copied;
            bytes = &bytes[copied..];
            if self.used == self.buffer.len() {
                self.flush_buffer()?;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<()> {
        self.flush_buffer()
    }

    fn flush_buffer(&mut self) -> std::io::Result<()> {
        if self.used != 0 {
            self.output.write_all(&self.buffer[..self.used])?;
            self.used = 0;
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_release_frame_bounded(
    output: &mut ChildStdin,
    command: &str,
    environment: &ValidatedBackgroundEnvironment,
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
        failure: None,
        retry: ObservationBackoff::retry(),
        attempts: 0,
        attempt_limit: MAX_RELEASE_FRAME_WRITE_ATTEMPTS,
    };
    if write_release_frame(&mut output, command, environment).is_err() {
        return Err(output.observed_failure());
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
    #[cfg(test)]
    if RELEASE_FAILURE_WITH_CANCELLATION_PID.load(Ordering::Acquire) == pid.get() {
        output.failure = Some(ReleaseWriteFailure::Release);
        output.cancellation_token.cancel();
        return Err(output.observed_failure());
    }
    std::io::Write::write_all(&mut output, &[RELEASE_COMMIT_BYTE])
        .map_err(|_| output.observed_failure())?;
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
struct BoundedGateWriter<'a, W: std::io::Write + ?Sized> {
    output: &'a mut W,
    cancellation_token: CancellationToken,
    cancellation: CancellationParker,
    deadline: Instant,
    failure: Option<ReleaseWriteFailure>,
    retry: ObservationBackoff,
    attempts: usize,
    attempt_limit: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<W: std::io::Write + ?Sized> BoundedGateWriter<'_, W> {
    fn observed_failure(&self) -> ReleaseWriteFailure {
        self.failure.unwrap_or(ReleaseWriteFailure::Release)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<W: std::io::Write + ?Sized> std::io::Write for BoundedGateWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        loop {
            if self.cancellation_token.is_cancelled() {
                // `Write::write_all` retries `Interrupted`; use the same fixed
                // terminal category as the bounded deadline so cancellation
                // cannot turn into a retry loop.
                self.failure = Some(ReleaseWriteFailure::Cancelled);
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            if Instant::now() >= self.deadline {
                self.failure = Some(ReleaseWriteFailure::Release);
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            if self.attempts == self.attempt_limit {
                self.failure = Some(ReleaseWriteFailure::Release);
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            self.attempts += 1;
            let attempted = &bytes[..bytes.len().min(RELEASE_FRAME_PIPE_WRITE_BYTES)];
            match self.output.write(attempted) {
                Ok(0) => {
                    self.failure = Some(ReleaseWriteFailure::Release);
                    return Err(std::io::ErrorKind::WriteZero.into());
                }
                Ok(written) => {
                    if written < attempted.len() && self.attempts < self.attempt_limit {
                        // A nonblocking pipe may report positive progress as
                        // small as one byte. Treat that as backpressure too, so
                        // `Write::write_all` cannot turn repeated short writes
                        // into a tight syscall loop before the hard attempt
                        // budget or deadline is reached.
                        self.retry
                            .park_until_and_advance(&mut self.cancellation, self.deadline);
                    }
                    return Ok(written);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    if self.attempts < self.attempt_limit {
                        self.retry
                            .park_until_and_advance(&mut self.cancellation, self.deadline);
                    }
                }
                Err(error) => {
                    self.failure = Some(ReleaseWriteFailure::Release);
                    return Err(error);
                }
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
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
    drop(prepared.output.take());
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
    wait_owned_with_output(owned, &mut |_| {})
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn wait_owned_with_output(
    owned: &mut OwnedBackgroundProcess,
    output: &mut impl FnMut(&[u8]),
) -> Result<BackgroundProcessExit, BackgroundProcessError> {
    if owned.child.is_none() {
        return Err(invariant_error());
    }
    let mut observation = ObservationBackoff::new();
    let observed = loop {
        let drained = match drain_background_output(
            &mut owned.output,
            BACKGROUND_OUTPUT_READS_PER_OBSERVATION,
            output,
        ) {
            Ok(drained) => drained,
            Err(error) => {
                let cleanup = cleanup_owned_child(owned, Duration::ZERO, None, true);
                return Err(combine_cleanup_failures(error, cleanup));
            }
        };
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
            Ok(None) if drained.progressed() => observation = ObservationBackoff::new(),
            Ok(None) => observation.sleep_and_advance(),
        }
    };
    finish_observed_with_output(owned, observed, output)
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
    wait_owned_with_stop_and_output(owned, stop, &mut |_| {})
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn wait_owned_with_stop_and_output(
    owned: &mut OwnedBackgroundProcess,
    stop: &CancellationToken,
    output: &mut impl FnMut(&[u8]),
) -> Result<BackgroundProcessOutcome, BackgroundProcessError> {
    if owned.child.is_none() {
        return Err(invariant_error());
    }
    let mut observation = ObservationBackoff::new();
    let mut cancellation = CancellationParker::new(stop);
    loop {
        if cancellation.is_cancelled() {
            stop_owned(owned)?;
            finish_background_output(owned, output)?;
            return Ok(BackgroundProcessOutcome::Stopped);
        }
        let drained = match drain_background_output(
            &mut owned.output,
            BACKGROUND_OUTPUT_READS_PER_OBSERVATION,
            output,
        ) {
            Ok(drained) => drained,
            Err(error) => {
                let cleanup = cleanup_owned_child(owned, Duration::ZERO, None, true);
                return Err(combine_cleanup_failures(error, cleanup));
            }
        };
        if cancellation.is_cancelled() {
            stop_owned(owned)?;
            finish_background_output(owned, output)?;
            return Ok(BackgroundProcessOutcome::Stopped);
        }
        match observe_leader(owned.group) {
            Ok(Some(status)) => {
                return finish_observed_with_output(owned, status, output)
                    .map(BackgroundProcessOutcome::Completed);
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
        if drained.progressed() {
            observation = ObservationBackoff::new();
        } else {
            observation.park_and_advance(&mut cancellation);
        }
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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn wait_owned_with_stop_and_output(
    owned: &mut OwnedBackgroundProcess,
    stop: &CancellationToken,
    _output: &mut impl FnMut(&[u8]),
) -> Result<BackgroundProcessOutcome, BackgroundProcessError> {
    wait_owned_with_stop(owned, stop)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn finish_observed(
    owned: &mut OwnedBackgroundProcess,
    observed: BackgroundProcessExit,
) -> Result<BackgroundProcessExit, BackgroundProcessError> {
    finish_observed_with_output(owned, observed, &mut |_| {})
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn finish_observed_with_output(
    owned: &mut OwnedBackgroundProcess,
    observed: BackgroundProcessExit,
    output: &mut impl FnMut(&[u8]),
) -> Result<BackgroundProcessExit, BackgroundProcessError> {
    cleanup_owned_child(owned, Duration::ZERO, Some(observed), false)?;
    finish_background_output(owned, output)?;
    Ok(observed)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackgroundOutputDrain {
    Idle,
    Progress,
    Exhausted,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl BackgroundOutputDrain {
    const fn progressed(self) -> bool {
        !matches!(self, Self::Idle)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn drain_background_output<R: std::io::Read>(
    output: &mut Option<R>,
    maximum_reads: usize,
    sink: &mut impl FnMut(&[u8]),
) -> Result<BackgroundOutputDrain, BackgroundProcessError> {
    let Some(reader) = output.as_mut() else {
        return Ok(BackgroundOutputDrain::Idle);
    };
    let mut buffer = [0_u8; BACKGROUND_OUTPUT_READ_BYTES];
    let mut progressed = false;
    for _ in 0..maximum_reads {
        match std::io::Read::read(reader, &mut buffer) {
            Ok(0) => {
                drop(output.take());
                return Ok(if progressed {
                    BackgroundOutputDrain::Progress
                } else {
                    BackgroundOutputDrain::Idle
                });
            }
            Ok(length) => {
                progressed = true;
                sink(&buffer[..length]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Ok(if progressed {
                    BackgroundOutputDrain::Progress
                } else {
                    BackgroundOutputDrain::Idle
                });
            }
            Err(_) => return Err(wait_error()),
        }
    }
    Ok(BackgroundOutputDrain::Exhausted)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn finish_background_output(
    owned: &mut OwnedBackgroundProcess,
    sink: &mut impl FnMut(&[u8]),
) -> Result<(), BackgroundProcessError> {
    finish_background_output_bounded(&mut owned.output, sink)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn finish_background_output_bounded<R: std::io::Read>(
    output: &mut Option<R>,
    sink: &mut impl FnMut(&[u8]),
) -> Result<(), BackgroundProcessError> {
    let _ = drain_background_output(output, BACKGROUND_OUTPUT_FINAL_READS, sink)?;
    // The leader outcome and owned-process cleanup are already final here.
    // A writer outside the bounded cleanup set may keep this pipe readable or
    // open indefinitely, so reaching the drain budget is truncation rather
    // than evidence that the observed process outcome failed.
    drop(output.take());
    Ok(())
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
    ObservationFailed,
    TimedOut,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildTryWaitError {
    Interrupted,
    LostAuthority,
    Operation,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn try_wait_child(child: &mut Child) -> Result<Option<ExitStatus>, ChildTryWaitError> {
    #[cfg(test)]
    if consume_injected_failure(&TRY_WAIT_FAILURE_PID, &TRY_WAIT_FAILURES, child.id()) {
        let raw = TRY_WAIT_ERRNO.load(Ordering::Acquire);
        return Err(classify_try_wait_error(&std::io::Error::from_raw_os_error(
            raw,
        )));
    }
    child
        .try_wait()
        .map_err(|error| classify_try_wait_error(&error))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn classify_try_wait_error(error: &std::io::Error) -> ChildTryWaitError {
    match error.raw_os_error() {
        Some(libc::EINTR) => ChildTryWaitError::Interrupted,
        Some(libc::ECHILD) => ChildTryWaitError::LostAuthority,
        _ => ChildTryWaitError::Operation,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn poll_child_reap(
    child: &mut Option<Child>,
    deadline: Instant,
) -> Result<BoundedReap, BackgroundProcessError> {
    let child = child.as_mut().ok_or_else(invariant_error)?;
    Ok(poll_child_reap_with(deadline, || try_wait_child(child)))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn poll_child_reap_with(
    deadline: Instant,
    mut try_wait: impl FnMut() -> Result<Option<ExitStatus>, ChildTryWaitError>,
) -> BoundedReap {
    let mut observation = ObservationBackoff::retry();
    loop {
        match try_wait() {
            Ok(Some(status)) => return BoundedReap::Reaped(status),
            Ok(None) | Err(ChildTryWaitError::Interrupted) if Instant::now() < deadline => {
                observation.sleep_until_and_advance(deadline);
            }
            Ok(None) | Err(ChildTryWaitError::Interrupted) => return BoundedReap::TimedOut,
            Err(ChildTryWaitError::LostAuthority) => return BoundedReap::LostAuthority,
            Err(ChildTryWaitError::Operation) => return BoundedReap::ObservationFailed,
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
) -> bool {
    if let Some(child) = child.as_mut() {
        let _ = child.kill();
    }
    match poll_child_reap(child, Instant::now() + CHILD_REAP_PROBE_TIMEOUT) {
        Ok(BoundedReap::Reaped(_)) => {
            discharge_reaped_child(child, reap_permit);
            true
        }
        Ok(BoundedReap::LostAuthority) | Err(_) => {
            discharge_reaped_child(child, reap_permit);
            false
        }
        Ok(BoundedReap::TimedOut | BoundedReap::ObservationFailed) => {
            let _ = quarantine_owned_child(child, reap_permit);
            false
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
        Ok(BoundedReap::ObservationFailed) => {
            failures.record(wait_error());
            if let Err(error) = quarantine_owned_child(child, reap_permit) {
                failures.record(error);
            }
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
                Ok(BoundedReap::ObservationFailed) => {
                    failures.record(wait_error());
                    if let Err(error) = quarantine_owned_child(child, reap_permit) {
                        failures.record(error);
                    }
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
    #[cfg(target_os = "macos")]
    let had_captured_descendants = captured.iter().any(|member| member.pid != group);
    let mut captured = RetainedMemberWait::new(captured.into_members(), group);
    let mut observation = ObservationBackoff::with_bounds(
        GROUP_SNAPSHOT_INITIAL_INTERVAL,
        GROUP_SNAPSHOT_MAX_INTERVAL,
    );
    #[cfg(target_os = "linux")]
    let mut stat_bytes = Vec::with_capacity(MAX_LINUX_PROC_STAT_BYTES + 1);
    let captured_resolved = loop {
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
            Ok(true) => break true,
            Ok(false) => {}
            Err(_) => {
                observed_failure = true;
                sleep_through(deadline.saturating_duration_since(Instant::now()));
                break false;
            }
        }
        if Instant::now() >= deadline {
            break false;
        }
        observation.sleep_and_advance();
    };
    #[cfg(target_os = "macos")]
    if captured_resolved && had_captured_descendants && !observed_failure {
        // Darwin can make `getpgid` report a killed descendant absent while a
        // fresh `ps` still exposes its not-yet-reaped zombie. Give that
        // process-table transition one coarse, deadline-bounded interval
        // before the single final global proof; never turn it into polling.
        sleep_through(std::cmp::min(
            GROUP_SNAPSHOT_MAX_INTERVAL,
            deadline.saturating_duration_since(Instant::now()),
        ));
    }
    let members = group_members(authority, group)?;
    require_group_quiescence_evidence(observed_failure, captured_resolved, &members, group)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn require_group_quiescence_evidence(
    observed_failure: bool,
    captured_resolved: bool,
    members: &[CapturedGroupMember],
    group: rustix::process::Pid,
) -> Result<(), BackgroundProcessError> {
    if observed_failure || !captured_resolved || !only_group_leader_remains(members, group) {
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
    let mut output = child
        .as_mut()
        .and_then(|child| child.stdout.take())
        .ok_or_else(cleanup_error)?;
    let Ok(flags) = rustix::fs::fcntl_getfl(&output) else {
        terminate_and_reap_or_quarantine(&mut child, &mut reap_permit);
        return Err(cleanup_error());
    };
    if rustix::fs::fcntl_setfl(&output, flags | OFlags::NONBLOCK).is_err() {
        terminate_and_reap_or_quarantine(&mut child, &mut reap_permit);
        return Err(cleanup_error());
    }
    let deadline = Instant::now() + GROUP_SNAPSHOT_TIMEOUT;
    let snapshot = collect_group_snapshot_output(&mut output, deadline, || {
        let state = match try_wait_child(child.as_mut().ok_or(())?) {
            Ok(Some(status)) => SnapshotChildState::Exited(status.success()),
            Ok(None) | Err(ChildTryWaitError::Interrupted) => SnapshotChildState::Running,
            Err(ChildTryWaitError::LostAuthority | ChildTryWaitError::Operation) => return Err(()),
        };
        Ok(state)
    });
    let Ok(bytes) = snapshot else {
        terminate_and_reap_or_quarantine(&mut child, &mut reap_permit);
        return Err(cleanup_error());
    };
    discharge_reaped_child(&mut child, &mut reap_permit);
    if bytes.len() > MAX_GROUP_SNAPSHOT_BYTES {
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

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotChildState {
    Running,
    Exited(bool),
}

#[cfg(target_os = "macos")]
fn collect_group_snapshot_output(
    reader: &mut impl std::io::Read,
    deadline: Instant,
    mut child_state: impl FnMut() -> Result<SnapshotChildState, ()>,
) -> Result<Vec<u8>, ()> {
    let mut bytes = Vec::with_capacity(MAX_GROUP_SNAPSHOT_BYTES + 1);
    let mut buffer = [0_u8; 8 * 1024];
    let mut eof = false;
    let mut exited = None;
    let mut observation = ObservationBackoff::retry();
    loop {
        if !eof {
            match reader.read(&mut buffer) {
                Ok(0) => eof = true,
                Ok(read) => {
                    if bytes.len().saturating_add(read) > MAX_GROUP_SNAPSHOT_BYTES {
                        return Err(());
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                    continue;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(_) => return Err(()),
            }
        }
        if exited.is_none()
            && let SnapshotChildState::Exited(success) = child_state()?
        {
            exited = Some(success);
        }
        if eof && exited.is_some() {
            return exited.filter(|success| *success).map(|_| bytes).ok_or(());
        }
        if Instant::now() >= deadline {
            return Err(());
        }
        observation.sleep_until_and_advance(deadline);
    }
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

    let mut name_bytes = [0_u8; 10];
    let name = linux_pid_path(member.pid, &mut name_bytes);
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
fn linux_pid_path(pid: rustix::process::Pid, storage: &mut [u8; 10]) -> &OsStr {
    let mut value = pid.as_raw_nonzero().get().cast_unsigned();
    let mut cursor = storage.len();
    loop {
        cursor -= 1;
        storage[cursor] = b'0' + u8::try_from(value % 10).expect("one decimal digit");
        value /= 10;
        if value == 0 {
            return OsStr::from_bytes(&storage[cursor..]);
        }
    }
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

    const fn retry() -> Self {
        Self::with_bounds(RETRY_INITIAL_INTERVAL, RETRY_MAX_INTERVAL)
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

    fn sleep_until_and_advance(&mut self, deadline: Instant) {
        self.sleep_until_and_advance_with(deadline, thread::sleep);
    }

    fn sleep_until_and_advance_with(&mut self, deadline: Instant, sleep: impl FnOnce(Duration)) {
        let interval = self.interval_until(deadline);
        if !interval.is_zero() {
            sleep(interval);
        }
        self.advance();
    }

    fn park_and_advance(&mut self, cancellation: &mut CancellationParker) {
        cancellation.park_timeout(self.current);
        self.advance();
    }

    fn park_until_and_advance(&mut self, cancellation: &mut CancellationParker, deadline: Instant) {
        cancellation.park_timeout(self.interval_until(deadline));
        self.advance();
    }

    fn interval_until(&self, deadline: Instant) -> Duration {
        std::cmp::min(
            self.current,
            deadline.saturating_duration_since(Instant::now()),
        )
    }

    fn advance(&mut self) {
        self.current = self.current.saturating_mul(2).min(self.maximum);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sleep_through(duration: Duration) {
    sleep_through_with(duration, thread::sleep);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sleep_through_with(duration: Duration, sleep: impl FnOnce(Duration)) {
    if !duration.is_zero() {
        sleep(duration);
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
fn cancelled_error() -> BackgroundProcessError {
    BackgroundProcessError::new(BackgroundProcessErrorKind::Cancelled)
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn final_output_drain_exhaustion_closes_the_pipe_without_overriding_completion() {
        let mut output = Some(std::io::repeat(b'x'));
        let mut captured = 0_usize;

        finish_background_output_bounded(&mut output, &mut |bytes| {
            captured = captured.saturating_add(bytes.len());
        })
        .expect("a bounded readable suffix is truncated after process completion");

        assert_eq!(
            captured,
            BACKGROUND_OUTPUT_FINAL_READS * BACKGROUND_OUTPUT_READ_BYTES
        );
        assert!(
            output.is_none(),
            "the final drain closes its read authority"
        );
    }

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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    struct TestProcessGroupGuard {
        child: Option<Child>,
        reap_permit: Option<ChildReapPermit>,
        group: rustix::process::Pid,
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl Drop for TestProcessGroupGuard {
        fn drop(&mut self) {
            if self.child.is_some() {
                let _ =
                    rustix::process::kill_process_group(self.group, rustix::process::Signal::KILL);
                let _ = terminate_and_reap_or_quarantine(&mut self.child, &mut self.reap_permit);
            }
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

    #[cfg(unix)]
    #[test]
    fn shared_environment_storage_survives_multiple_request_preparations() {
        let directory = TestDirectory::new("shared-environment");
        let environment = ValidatedBackgroundEnvironment::new(vec![(
            OsString::from("PAYLOAD"),
            OsString::from("retained"),
        )])
        .expect("validated environment");
        let open = || {
            rustix::fs::open(
                &directory.0,
                OFlags::RDONLY
                    | OFlags::DIRECTORY
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC
                    | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .expect("directory authority")
        };
        let first = BackgroundProcessRequest::from_directory_with_environment(
            "true".to_owned(),
            "workspace".to_owned(),
            environment.clone(),
            open(),
        )
        .expect("first request");
        let second = BackgroundProcessRequest::from_directory_with_environment(
            "true".to_owned(),
            "workspace".to_owned(),
            environment.clone(),
            open(),
        )
        .expect("second request");

        assert!(environment.shares_storage_with(&first.environment));
        assert!(environment.shares_storage_with(&second.environment));
        assert_eq!(first.environment().as_ptr(), second.environment().as_ptr());
        assert_eq!(
            first.environment()[0].1.as_os_str().as_bytes().as_ptr(),
            second.environment()[0].1.as_os_str().as_bytes().as_ptr()
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn encoded_environment_keeps_release_writes_constant_and_commit_distinct() {
        struct CountingWriter {
            lengths: Vec<usize>,
            bytes: Vec<u8>,
        }

        impl std::io::Write for CountingWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.lengths.push(bytes.len());
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let environment = (0..MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES)
            .map(|index| {
                (
                    OsString::from(format!("K{index:03}")),
                    OsString::from("x".repeat(508)),
                )
            })
            .collect::<Vec<_>>();
        let environment =
            ValidatedBackgroundEnvironment::new(environment).expect("maximum environment");
        let command = "x".repeat(MAX_BACKGROUND_PROCESS_COMMAND_BYTES);
        let mut output = CountingWriter {
            lengths: Vec::new(),
            bytes: Vec::new(),
        };

        write_release_frame(&mut output, &command, &environment).expect("release frame");
        assert_eq!(output.lengths.len(), 19);
        assert!(
            output.lengths[..18]
                .iter()
                .all(|length| *length == RELEASE_FRAME_WRITE_CHUNK_BYTES)
        );
        assert_eq!(output.lengths[18], 4_112);
        assert_eq!(output.bytes.len(), 299_024);
        std::io::Write::write_all(&mut output, &[RELEASE_COMMIT_BYTE]).expect("distinct commit");
        assert_eq!(output.lengths.len(), 20);
        assert_eq!(output.lengths[19], 1);
        assert_eq!(output.bytes.last(), Some(&RELEASE_COMMIT_BYTE));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn maximum_release_frame_obeys_physical_attempt_bound_and_distinct_commit() {
        struct CountingWriter {
            lengths: Vec<usize>,
            bytes: usize,
        }

        impl std::io::Write for CountingWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.lengths.push(bytes.len());
                self.bytes += bytes.len();
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let environment = (0..MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES)
            .map(|index| {
                (
                    OsString::from(format!("K{index:03}")),
                    OsString::from("x".repeat(508)),
                )
            })
            .collect::<Vec<_>>();
        let environment =
            ValidatedBackgroundEnvironment::new(environment).expect("maximum environment");
        let command = "x".repeat(MAX_BACKGROUND_PROCESS_COMMAND_BYTES);
        let cancellation = CancellationToken::new();
        let mut output = CountingWriter {
            lengths: Vec::new(),
            bytes: 0,
        };
        let mut writer = BoundedGateWriter {
            output: &mut output,
            cancellation_token: cancellation.clone(),
            cancellation: CancellationParker::new(&cancellation),
            deadline: Instant::now() + Duration::from_secs(1),
            failure: None,
            retry: ObservationBackoff::retry(),
            attempts: 0,
            attempt_limit: MAX_RELEASE_FRAME_WRITE_ATTEMPTS,
        };

        write_release_frame(&mut writer, &command, &environment).expect("release frame");
        let payload_attempts =
            MAX_RELEASE_FRAME_PAYLOAD_BYTES.div_ceil(RELEASE_FRAME_PIPE_WRITE_BYTES);
        assert_eq!(writer.attempts, payload_attempts);
        std::io::Write::write_all(&mut writer, &[RELEASE_COMMIT_BYTE]).expect("distinct commit");
        assert_eq!(writer.attempts, payload_attempts + 1);
        assert!(writer.attempts <= MAX_RELEASE_FRAME_WRITE_ATTEMPTS);
        drop(writer);

        assert_eq!(output.lengths.len(), payload_attempts + 1);
        assert!(
            output.lengths[..payload_attempts]
                .iter()
                .all(|length| *length <= RELEASE_FRAME_PIPE_WRITE_BYTES)
        );
        assert_eq!(output.lengths[payload_attempts], 1);
        assert_eq!(output.bytes, MAX_RELEASE_FRAME_PAYLOAD_BYTES + 1);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn process_retry_backoff_is_exponential_and_capped() {
        let mut retry = ObservationBackoff::retry();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut waits = Vec::new();

        for _ in 0..6 {
            retry.sleep_until_and_advance_with(deadline, |duration| waits.push(duration));
        }

        assert_eq!(
            waits,
            [
                Duration::from_millis(4),
                Duration::from_millis(8),
                Duration::from_millis(16),
                Duration::from_millis(32),
                Duration::from_millis(32),
                Duration::from_millis(32),
            ]
        );
        assert!(waits.iter().all(|wait| *wait <= RETRY_MAX_INTERVAL));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn release_writer_backs_off_persistent_interruption_and_backpressure() {
        struct RetryingWriter {
            attempts: usize,
            kind: std::io::ErrorKind,
        }

        impl std::io::Write for RetryingWriter {
            fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
                self.attempts += 1;
                Err(self.kind.into())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        for kind in [
            std::io::ErrorKind::Interrupted,
            std::io::ErrorKind::WouldBlock,
        ] {
            let cancellation = CancellationToken::new();
            let mut output = RetryingWriter { attempts: 0, kind };
            let mut writer = BoundedGateWriter {
                output: &mut output,
                cancellation_token: cancellation.clone(),
                cancellation: CancellationParker::new(&cancellation),
                deadline: Instant::now() + Duration::from_secs(1),
                failure: None,
                retry: ObservationBackoff::retry(),
                attempts: 0,
                attempt_limit: 4,
            };

            let error = std::io::Write::write(&mut writer, b"x")
                .expect_err("persistent retryable write reaches its attempt budget");
            let failure = writer.observed_failure();
            let next_backoff = writer.retry.current;
            drop(writer);

            assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
            assert!(matches!(failure, ReleaseWriteFailure::Release));
            assert_eq!(output.attempts, 4, "{kind:?} attempt count");
            assert_eq!(next_backoff, Duration::from_millis(32));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn release_writer_bounds_one_byte_short_writes() {
        struct OneByteWriter {
            attempts: usize,
        }

        impl std::io::Write for OneByteWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.attempts += 1;
                Ok(usize::from(!bytes.is_empty()))
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let cancellation = CancellationToken::new();
        let mut output = OneByteWriter { attempts: 0 };
        let mut writer = BoundedGateWriter {
            output: &mut output,
            cancellation_token: cancellation.clone(),
            cancellation: CancellationParker::new(&cancellation),
            deadline: Instant::now() + Duration::from_secs(1),
            failure: None,
            retry: ObservationBackoff::retry(),
            attempts: 0,
            attempt_limit: 4,
        };
        let bytes = [0_u8; 5];

        let error = std::io::Write::write_all(&mut writer, &bytes)
            .expect_err("one-byte progress reaches the fixed attempt budget");
        let failure = writer.observed_failure();
        let next_backoff = writer.retry.current;
        drop(writer);

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(matches!(failure, ReleaseWriteFailure::Release));
        assert_eq!(output.attempts, 4);
        assert_eq!(next_backoff, Duration::from_millis(32));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn cancellation_after_one_byte_short_write_keeps_cancelled_precedence() {
        struct CancellingWriter {
            attempts: usize,
            cancellation: CancellationToken,
        }

        impl std::io::Write for CancellingWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.attempts += 1;
                self.cancellation.cancel();
                Ok(usize::from(!bytes.is_empty()))
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let cancellation = CancellationToken::new();
        let mut output = CancellingWriter {
            attempts: 0,
            cancellation: cancellation.clone(),
        };
        let mut writer = BoundedGateWriter {
            output: &mut output,
            cancellation_token: cancellation.clone(),
            cancellation: CancellationParker::new(&cancellation),
            deadline: Instant::now() + Duration::from_secs(1),
            failure: None,
            retry: ObservationBackoff::retry(),
            attempts: 0,
            attempt_limit: MAX_RELEASE_FRAME_WRITE_ATTEMPTS,
        };

        let error = std::io::Write::write_all(&mut writer, &[0_u8; 2])
            .expect_err("cancellation wins before the second write attempt");
        let failure = writer.observed_failure();
        drop(writer);

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(matches!(failure, ReleaseWriteFailure::Cancelled));
        assert_eq!(output.attempts, 1);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn child_reap_poll_backs_off_persistent_eintr_and_unreaped_states() {
        for (label, interrupted) in [("EINTR", true), ("unreaped", false)] {
            let timeout = Duration::from_millis(20);
            let started = Instant::now();
            let mut attempts = 0_usize;
            let result = poll_child_reap_with(started + timeout, || {
                attempts += 1;
                if interrupted {
                    Err(ChildTryWaitError::Interrupted)
                } else {
                    Ok(None)
                }
            });

            assert!(matches!(result, BoundedReap::TimedOut));
            assert!(started.elapsed() >= timeout);
            assert!(
                (2..=8).contains(&attempts),
                "{label} reap made {attempts} attempts instead of backing off"
            );
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn exclusive_reaping_probe_backs_off_persistent_eintr_and_unreaped_states() {
        for (label, interrupted) in [("EINTR", true), ("unreaped", false)] {
            let cancellation = CancellationToken::new();
            let mut cancellation = CancellationParker::new(&cancellation);
            let timeout = Duration::from_millis(20);
            let started = Instant::now();
            let mut attempts = 0_usize;
            let result = poll_exclusive_child_reaping(&mut cancellation, started + timeout, || {
                attempts += 1;
                if interrupted {
                    Err(ChildTryWaitError::Interrupted)
                } else {
                    Ok(None)
                }
            });

            assert_eq!(result, ExclusiveReapingOutcome::Failed);
            assert!(started.elapsed() >= timeout);
            assert!(
                (2..=8).contains(&attempts),
                "{label} exclusive-reaping probe made {attempts} attempts instead of backing off"
            );
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn release_io_failure_is_not_relabelled_by_simultaneous_cancellation() {
        let _guard = GROUP_SNAPSHOT_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let directory = TestDirectory::new("release-failure-cancel-race");
        let helper = BackgroundProcessHelper::new(
            PathBuf::from("/bin/sh"),
            vec![
                OsString::from("-c"),
                OsString::from("printf '\\247' >&2; exec /bin/sleep 30"),
            ],
        )
        .expect("bounded inert helper");
        let request = BackgroundProcessRequest::open(
            "true".to_owned(),
            "workspace".to_owned(),
            Vec::new(),
            &directory.0,
        )
        .expect("bounded process request");
        let prepared = SystemBackgroundProcessAdapter::with_helper(helper)
            .prepare(request)
            .expect("helper becomes ready");
        let pid = prepared.pid();
        RELEASE_FAILURE_WITH_CANCELLATION_PID.store(pid.get(), Ordering::Release);
        let cancellation = CancellationToken::new();

        let error = prepared
            .release_cancellable(&cancellation)
            .expect_err("injected failing release does not publish ownership");

        RELEASE_FAILURE_WITH_CANCELLATION_PID.store(0, Ordering::Release);
        assert!(cancellation.is_cancelled());
        assert_eq!(error.kind(), BackgroundProcessErrorKind::Release);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn child_reap_errno_preserves_exact_authority_outcome() {
        let _guard = GROUP_SNAPSHOT_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let spawn = || {
            let permit = reserve_child_reap_authority().expect("reserve child authority");
            let child = Command::new("/bin/sleep")
                .arg("30")
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn retained child");
            (Some(child), Some(permit))
        };
        let finish = |child: &mut Option<Child>, permit: &mut Option<ChildReapPermit>| {
            child
                .as_mut()
                .expect("child authority retained")
                .kill()
                .expect("kill retained child");
            assert!(matches!(
                poll_child_reap(child, Instant::now() + CHILD_REAP_PROBE_TIMEOUT)
                    .expect("final reap observation succeeds"),
                BoundedReap::Reaped(_)
            ));
            discharge_reaped_child(child, permit);
        };

        let (mut child, mut permit) = spawn();
        let pid = NonZeroU32::new(child.as_ref().expect("child retained").id()).unwrap();
        inject_failures(&TRY_WAIT_FAILURE_PID, &TRY_WAIT_FAILURES, pid, 1);
        TRY_WAIT_ERRNO.store(libc::EINTR, Ordering::Release);
        assert!(matches!(
            poll_child_reap(&mut child, Instant::now() + Duration::from_millis(20))
                .expect("interruption has a bounded retry outcome"),
            BoundedReap::TimedOut
        ));
        assert!(child.is_some(), "interruption retained child authority");
        finish(&mut child, &mut permit);

        let (mut child, mut permit) = spawn();
        let pid = NonZeroU32::new(child.as_ref().expect("child retained").id()).unwrap();
        inject_failures(&TRY_WAIT_FAILURE_PID, &TRY_WAIT_FAILURES, pid, 1);
        TRY_WAIT_ERRNO.store(libc::ECHILD, Ordering::Release);
        assert!(matches!(
            poll_child_reap(&mut child, Instant::now() + Duration::from_millis(20))
                .expect("lost authority is a stable observation"),
            BoundedReap::LostAuthority
        ));
        finish(&mut child, &mut permit);

        let (mut child, mut permit) = spawn();
        let pid = NonZeroU32::new(child.as_ref().expect("child retained").id()).unwrap();
        child
            .as_mut()
            .expect("child retained before quarantine")
            .kill()
            .expect("kill child before injected observation failure");
        inject_failures(&TRY_WAIT_FAILURE_PID, &TRY_WAIT_FAILURES, pid, 1);
        TRY_WAIT_ERRNO.store(libc::EIO, Ordering::Release);
        let mut failures = CleanupFailures::default();
        reap_child_bounded(&mut child, &mut permit, None, &mut failures);
        assert_eq!(
            failures
                .finish()
                .expect_err("observation failure remains visible")
                .kind(),
            BackgroundProcessErrorKind::Wait
        );
        assert!(
            child.is_none() && permit.is_none(),
            "operation failure transferred both authorities to quarantine"
        );

        TRY_WAIT_FAILURE_PID.store(0, Ordering::Release);
        TRY_WAIT_FAILURES.store(0, Ordering::Release);
        TRY_WAIT_ERRNO.store(0, Ordering::Release);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn unresolved_escaped_member_rejects_deadline_quiescence() {
        let leader = rustix::process::Pid::from_raw(71).expect("positive leader");
        let members = [CapturedGroupMember {
            pid: leader,
            identity: Some(1),
        }];
        assert_eq!(
            require_group_quiescence_evidence(false, false, &members, leader)
                .expect_err("an escaped retained member is unresolved at the deadline")
                .kind(),
            BackgroundProcessErrorKind::Cleanup
        );
        require_group_quiescence_evidence(false, true, &members, leader)
            .expect("resolved retained members and the sole leader are quiescent");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn group_snapshot_reader_retries_are_bounded_without_a_join() {
        struct RetryingReader {
            reads: usize,
            kind: std::io::ErrorKind,
        }

        impl std::io::Read for RetryingReader {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                self.reads += 1;
                Err(self.kind.into())
            }
        }

        for kind in [
            std::io::ErrorKind::Interrupted,
            std::io::ErrorKind::WouldBlock,
        ] {
            let mut reader = RetryingReader { reads: 0, kind };
            let mut observations = 0_usize;
            let timeout = Duration::from_millis(20);
            let started = Instant::now();
            assert_eq!(
                collect_group_snapshot_output(&mut reader, started + timeout, || {
                    observations += 1;
                    Ok(SnapshotChildState::Exited(true))
                }),
                Err(())
            );
            assert!(started.elapsed() >= timeout);
            assert!(started.elapsed() < Duration::from_millis(250));
            assert!(
                (2..=8).contains(&reader.reads),
                "{kind:?} snapshot made {} reads instead of backing off",
                reader.reads
            );
            assert_eq!(observations, 1, "exited child state is not repolled");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn repeated_pid_path_formatting_allocates_nothing() {
        let pid = rustix::process::Pid::from_raw(i32::MAX).expect("positive PID");
        allocation_counter::measure(|| {});
        let allocations = allocation_counter::measure(|| {
            let mut storage = [0_u8; 10];
            for _ in 0..10_000 {
                std::hint::black_box(linux_pid_path(pid, &mut storage));
            }
        });
        assert_eq!(allocations.count_total, 0, "{allocations:?}");
        assert_eq!(allocations.bytes_total, 0, "{allocations:?}");
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
        let mut observation = ObservationBackoff::retry();
        let pid = loop {
            if let Ok(pid) = fs::read_to_string(&pid_file)
                && let Ok(pid) = pid.parse::<i32>()
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "probe did not publish its PID");
            observation.sleep_until_and_advance(deadline);
        };

        let started = Instant::now();
        assert!(cancellation.cancel());
        assert_eq!(
            worker
                .join()
                .expect("join cancelled probe")
                .expect_err("cancelled probe fails")
                .kind(),
            BackgroundProcessErrorKind::Cancelled
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
    fn readiness_error_is_not_relabelled_by_racing_cancellation() {
        struct ErrorAndCancelReader(CancellationToken);

        impl std::io::Read for ErrorAndCancelReader {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                assert!(self.0.cancel());
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
        }

        let cancellation = CancellationToken::new();
        let mut reader = ErrorAndCancelReader(cancellation.clone());
        let error = await_helper_ready_bounded(
            &mut reader,
            &cancellation,
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("readiness failure wins at its failing read");
        assert_eq!(error.kind(), BackgroundProcessErrorKind::Spawn);
        assert!(cancellation.is_cancelled());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn repeated_interrupted_readiness_is_deadline_bounded_and_backed_off() {
        struct InterruptedReader {
            reads: usize,
        }

        impl std::io::Read for InterruptedReader {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                self.reads += 1;
                Err(std::io::ErrorKind::Interrupted.into())
            }
        }

        let cancellation = CancellationToken::new();
        let mut reader = InterruptedReader { reads: 0 };
        let timeout = Duration::from_millis(20);
        let started = Instant::now();
        let error = await_helper_ready_bounded(&mut reader, &cancellation, started + timeout)
            .expect_err("an interrupted reader cannot outlive readiness deadline");
        let elapsed = started.elapsed();
        assert_eq!(error.kind(), BackgroundProcessErrorKind::Spawn);
        assert!(elapsed >= timeout, "readiness returned before its deadline");
        assert!(
            elapsed < Duration::from_millis(250),
            "interrupted readiness exceeded its bounded deadline: {elapsed:?}"
        );
        assert!(
            reader.reads <= 8,
            "interrupted readiness busy-spun for {} reads",
            reader.reads
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn sleep_through_uses_one_exact_bounded_wait() {
        let requested = Duration::from_secs(5);
        let mut waits = Vec::new();

        sleep_through_with(requested, |duration| waits.push(duration));

        assert_eq!(waits, [requested]);
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
        let mut observation = ObservationBackoff::retry();
        while !helper_pid.exists() {
            assert!(
                Instant::now() < deadline,
                "helper did not reach stalled readiness"
            );
            observation.sleep_until_and_advance(deadline);
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
        assert_eq!(error.kind(), BackgroundProcessErrorKind::Cancelled);
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
        let mut observation = ObservationBackoff::retry();
        while !matches!(observe_leader(group), Ok(Some(_))) {
            assert!(Instant::now() < deadline, "leader did not exit");
            observation.sleep_until_and_advance(deadline);
        }
        #[cfg(target_os = "linux")]
        let authority = GroupSnapshotAuthority::open().expect("open group snapshot authority");
        #[cfg(target_os = "macos")]
        let authority = GroupSnapshotAuthority;
        let snapshot_guard = GROUP_SNAPSHOT_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        drop(snapshot_guard);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn lingering_group_cleanup_uses_constant_global_snapshots() {
        let directory = TestDirectory::new("snapshot-count");
        let marker = directory.0.join("descendant.pid");
        let command = "trap '' TERM; /bin/sh -c 'trap \"\" TERM; printf \"%s\" \"$$\" > descendant.pid; exec /bin/sleep 30' & exit 7";
        let reap_permit =
            reserve_child_reap_authority().expect("reserve test child reap authority");
        let child = Command::new("/bin/sh")
            .args(["-c", command])
            .current_dir(&directory.0)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn test process group");
        let leader = NonZeroU32::new(child.id()).expect("positive process-group leader");
        let group = rustix::process::Pid::from_raw(
            i32::try_from(leader.get()).expect("test PID fits signed range"),
        )
        .expect("positive process group");
        let mut process = TestProcessGroupGuard {
            child: Some(child),
            reap_permit: Some(reap_permit),
            group,
        };
        #[cfg(target_os = "linux")]
        let authority = GroupSnapshotAuthority::open().expect("open group snapshot authority");
        #[cfg(target_os = "macos")]
        let authority = GroupSnapshotAuthority;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observation = ObservationBackoff::retry();
        loop {
            let fixture_is_ready = matches!(
                observe_leader(group),
                Ok(Some(BackgroundProcessExit::Exited(7)))
            ) && marker.exists()
                && group_members(&authority, group)
                    .is_ok_and(|members| members.iter().any(|member| member.pid != group));
            if fixture_is_ready {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "exited leader and descendant did not become process-table visible"
            );
            observation.sleep_until_and_advance(deadline);
        }
        let snapshot_guard = GROUP_SNAPSHOT_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reset_group_snapshots_for_test(leader);

        cleanup_child_with_expected(
            &mut process.child,
            &mut process.reap_permit,
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
        drop(snapshot_guard);
    }
}
