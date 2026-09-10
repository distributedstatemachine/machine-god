//! Explicit desktop URL handoff with bounded, host-owned direct-child cleanup.

use crate::NativeOwnedWorkerScope;
use crate::background_commands::url::BackgroundServerUrl;
use crate::background_process::{TmuxChild, ValidatedBackgroundEnvironment};
use machine_god_core::{BoxFuture, CancellationToken};
use rustix::fs::{FileType, Mode, OFlags};
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Component, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

/// Fixed pre-launch failures never expose the URL, environment or executable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeBackgroundOpenError {
    InvalidAuthority,
    Busy,
    Cancelled,
    TimedOut,
    Unavailable,
}

impl fmt::Display for NativeBackgroundOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("background URL opener unavailable")
    }
}
impl std::error::Error for NativeBackgroundOpenError {}

/// A launcher receipt, not a network-availability or browser-lifetime claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeBackgroundOpenOutcome {
    /// The direct launcher exited successfully after accepting the URL argument.
    Opened,
    /// The launcher reported failure; this does not prove that no browser opened.
    LauncherFailed,
    /// Launch began, but its successful handoff could not be established.
    Indeterminate,
}

/// Explicit host capability; construction never discovers a browser or environment.
///
/// The caller supplies and protects the executable installation for the entire
/// capability lifetime. Identity revalidation is not protection against an
/// attacker allowed to replace that installation between validation and exec.
#[derive(Clone)]
pub struct NativeBackgroundUrlOpener {
    inner: Arc<OpenerInner>,
}

/// Retained launcher installation, independent of any particular host lifetime.
#[derive(Clone)]
pub struct NativeBackgroundUrlExecutable {
    program: PathBuf,
    executable: Arc<File>,
}
impl fmt::Debug for NativeBackgroundUrlExecutable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeBackgroundUrlExecutable")
            .finish_non_exhaustive()
    }
}
impl NativeBackgroundUrlExecutable {
    /// Validates spelling without inspecting the retained executable.
    /// # Errors
    /// Rejects nonabsolute, parent-relative, NUL-containing or oversized paths.
    pub fn new(program: PathBuf, executable: File) -> Result<Self, NativeBackgroundOpenError> {
        if !program.is_absolute()
            || program.as_os_str().len() > 4096
            || program.as_os_str().as_bytes().contains(&0)
            || program
                .components()
                .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        {
            return Err(NativeBackgroundOpenError::InvalidAuthority);
        }
        Ok(Self {
            program,
            executable: Arc::new(executable),
        })
    }
}

struct OpenerInner {
    program: PathBuf,
    executable: Arc<File>,
    environment: ValidatedBackgroundEnvironment,
    workers: NativeOwnedWorkerScope,
    active: Arc<AtomicBool>,
    #[cfg(test)]
    arguments: Vec<OsString>,
}

impl fmt::Debug for NativeBackgroundUrlOpener {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeBackgroundUrlOpener")
            .finish_non_exhaustive()
    }
}

impl NativeBackgroundUrlOpener {
    /// Binds explicit executable and environment authority without filesystem I/O.
    ///
    /// The supplied program must accept one HTTP(S) URL argument. Its installation
    /// must remain protected against replacement, as for the clipboard capability.
    /// Clones share one operation admission until actual direct-child reaping.
    ///
    /// # Errors
    /// Rejects invalid program spelling or an invalid/oversized environment.
    pub fn new(
        program: PathBuf,
        executable: File,
        environment: Vec<(OsString, OsString)>,
        workers: NativeOwnedWorkerScope,
    ) -> Result<Self, NativeBackgroundOpenError> {
        Self::from_executable(
            NativeBackgroundUrlExecutable::new(program, executable)?,
            environment,
            workers,
        )
    }

    pub(crate) fn from_executable(
        executable: NativeBackgroundUrlExecutable,
        environment: Vec<(OsString, OsString)>,
        workers: NativeOwnedWorkerScope,
    ) -> Result<Self, NativeBackgroundOpenError> {
        let environment = ValidatedBackgroundEnvironment::new(environment)
            .map_err(|_| NativeBackgroundOpenError::InvalidAuthority)?;
        Ok(Self {
            inner: Arc::new(OpenerInner {
                program: executable.program,
                executable: executable.executable,
                environment,
                workers,
                active: Arc::new(AtomicBool::new(false)),
                #[cfg(test)]
                arguments: Vec::new(),
            }),
        })
    }

    /// Inert until polled. The selected native target supplies `revoked`; raw
    /// transcript strings cannot bypass the validated URL representation.
    pub(crate) fn open(
        &self,
        url: BackgroundServerUrl,
        cancellation: CancellationToken,
        revoked: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeBackgroundOpenOutcome, NativeBackgroundOpenError>> {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            if cancellation.is_cancelled() || revoked.is_cancelled() {
                return Err(NativeBackgroundOpenError::Cancelled);
            }
            inner
                .active
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| NativeBackgroundOpenError::Busy)?;
            let admission = Admission(Arc::clone(&inner.active));
            let abandoned = CancellationToken::new();
            let guard = CancelOnDrop(abandoned.clone());
            let operation = Operation {
                cancellation,
                revoked,
                abandoned,
                deadline: Instant::now() + OPERATION_TIMEOUT,
            };
            let workers = inner.workers.clone();
            let result = workers
                .run(move || launch(inner, &url, &operation, admission))
                .await
                // A worker failure need not prove that its closure never ran.
                // Conservatively retain publication uncertainty at this boundary.
                .unwrap_or(Ok(NativeBackgroundOpenOutcome::Indeterminate));
            drop(guard);
            result
        })
    }
}

struct Admission(Arc<AtomicBool>);
impl Drop for Admission {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct Operation {
    cancellation: CancellationToken,
    revoked: CancellationToken,
    abandoned: CancellationToken,
    deadline: Instant,
}
impl Operation {
    fn check(&self) -> Result<(), NativeBackgroundOpenError> {
        if self.cancellation.is_cancelled()
            || self.revoked.is_cancelled()
            || self.abandoned.is_cancelled()
        {
            Err(NativeBackgroundOpenError::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(NativeBackgroundOpenError::TimedOut)
        } else {
            Ok(())
        }
    }
}

fn launch(
    inner: Arc<OpenerInner>,
    url: &BackgroundServerUrl,
    operation: &Operation,
    admission: Admission,
) -> Result<NativeBackgroundOpenOutcome, NativeBackgroundOpenError> {
    operation.check()?;
    verify_executable(&inner)?;
    let mut command = Command::new(&inner.program);
    #[cfg(test)]
    command.args(&inner.arguments);
    command
        .arg(url.as_str())
        .env_clear()
        .envs(
            inner
                .environment
                .entries()
                .iter()
                .map(|(key, value)| (key, value)),
        )
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    operation.check()?;
    let mut child =
        TmuxChild::spawn(&mut command).map_err(|_| NativeBackgroundOpenError::Unavailable)?;
    child.retain_until_reaped(Box::new((admission, inner)));
    drop(command);
    // Once spawn begins, cancellation cannot promise that the URL was not opened.
    let mut interval = Duration::from_millis(2);
    loop {
        if operation.check().is_err() {
            return Ok(NativeBackgroundOpenOutcome::Indeterminate);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(if status.success() {
                    NativeBackgroundOpenOutcome::Opened
                } else {
                    NativeBackgroundOpenOutcome::LauncherFailed
                });
            }
            Ok(None) => {
                std::thread::sleep(
                    interval.min(operation.deadline.saturating_duration_since(Instant::now())),
                );
                interval = (interval * 2).min(Duration::from_millis(32));
            }
            Err(_) => return Ok(NativeBackgroundOpenOutcome::Indeterminate),
        }
    }
}

fn verify_executable(inner: &OpenerInner) -> Result<(), NativeBackgroundOpenError> {
    let retained = rustix::fs::fstat(&*inner.executable)
        .map_err(|_| NativeBackgroundOpenError::Unavailable)?;
    let named = rustix::fs::open(
        &inner.program,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|_| NativeBackgroundOpenError::Unavailable)?;
    let current = rustix::fs::fstat(named).map_err(|_| NativeBackgroundOpenError::Unavailable)?;
    if FileType::from_raw_mode(retained.st_mode) != FileType::RegularFile
        || retained.st_mode & 0o111 == 0
        || retained.st_dev != current.st_dev
        || retained.st_ino != current.st_ino
        || retained.st_rdev != current.st_rdev
        || retained.st_mode != current.st_mode
        || retained.st_size != current.st_size
        || retained.st_mtime != current.st_mtime
        || retained.st_mtime_nsec != current.st_mtime_nsec
        || retained.st_ctime != current.st_ctime
        || retained.st_ctime_nsec != current.st_ctime_nsec
    {
        return Err(NativeBackgroundOpenError::Unavailable);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
