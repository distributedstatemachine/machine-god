//! Shared direct-child URL handoff; callers own URL admission and consent.
use super::{
    NativeBackgroundOpenError, NativeBackgroundOpenOutcome, NativeBackgroundUrlExecutable,
};
use crate::{
    NativeOwnedWorkerScope,
    background_commands::url::BackgroundServerUrl,
    background_process::{BackgroundProcessError, TmuxChild, ValidatedBackgroundEnvironment},
};
use machine_god_core::{BoxFuture, CancellationToken};
use rustix::fs::{FileType, Mode, OFlags};
use std::{
    ffi::OsString,
    fs::File,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

/// Native effect checkpoint retained through the direct child's actual reap.
/// External URLs cannot construct this additional authority.
pub(crate) trait LauncherGuard: Send + Sync {
    fn check(&self) -> Result<(), NativeBackgroundOpenError>;
}

/// Each caller moves its own admitted representation without copying URL bytes.
pub(crate) enum LauncherUrl {
    Background(BackgroundServerUrl),
    Mcp(Box<str>),
}

impl LauncherUrl {
    fn as_str(&self) -> &str {
        match self {
            Self::Background(url) => url.as_str(),
            Self::Mcp(url) => url,
        }
    }
}
#[derive(Clone)]
pub(crate) struct OwnedUrlLauncher {
    pub(super) inner: Arc<OpenerInner>,
}
pub(super) struct OpenerInner {
    program: PathBuf,
    pub(super) executable: Arc<File>,
    environment: ValidatedBackgroundEnvironment,
    workers: NativeOwnedWorkerScope,
    pub(super) active: Arc<AtomicBool>,
    #[cfg(test)]
    pub(super) arguments: Vec<OsString>,
}

impl OwnedUrlLauncher {
    #[cfg(test)]
    pub(crate) fn set_test_arguments(&mut self, arguments: Vec<OsString>) {
        Arc::get_mut(&mut self.inner).unwrap().arguments = arguments;
    }
    pub(crate) fn new(
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

    pub(crate) fn open(
        &self,
        url: LauncherUrl,
        cancellation: CancellationToken,
        revoked: CancellationToken,
        deadline: Option<Instant>,
    ) -> BoxFuture<'static, Result<NativeBackgroundOpenOutcome, NativeBackgroundOpenError>> {
        self.open_guarded(url, cancellation, revoked, deadline, None)
    }

    pub(crate) fn open_guarded(
        &self,
        url: LauncherUrl,
        cancellation: CancellationToken,
        revoked: CancellationToken,
        deadline: Option<Instant>,
        checkpoint: Option<Arc<dyn LauncherGuard>>,
    ) -> BoxFuture<'static, Result<NativeBackgroundOpenOutcome, NativeBackgroundOpenError>> {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            if cancellation.is_cancelled() || revoked.is_cancelled() {
                return Err(NativeBackgroundOpenError::Cancelled);
            }
            let now = Instant::now();
            let capped = now
                .checked_add(OPERATION_TIMEOUT)
                .ok_or(NativeBackgroundOpenError::TimedOut)?;
            let deadline = deadline.map_or(capped, |requested| requested.min(capped));
            if now >= deadline {
                return Err(NativeBackgroundOpenError::TimedOut);
            }
            if let Some(checkpoint) = &checkpoint {
                checkpoint.check()?;
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
                deadline,
                checkpoint,
            };
            let workers = inner.workers.clone();
            let result = workers
                .run(move || launch(inner, url.as_str(), &operation, admission))
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
    checkpoint: Option<Arc<dyn LauncherGuard>>,
}

struct LauncherSpawnFailure(NativeBackgroundOpenError);

impl From<BackgroundProcessError> for LauncherSpawnFailure {
    fn from(_: BackgroundProcessError) -> Self {
        Self(NativeBackgroundOpenError::Unavailable)
    }
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
            self.checkpoint
                .as_ref()
                .map_or(Ok(()), |guard| guard.check())
        }
    }
}

fn launch(
    inner: Arc<OpenerInner>,
    url: &str,
    operation: &Operation,
    admission: Admission,
) -> Result<NativeBackgroundOpenOutcome, NativeBackgroundOpenError> {
    operation.check()?;
    verify_executable(&inner)?;
    let mut command = Command::new(&inner.program);
    #[cfg(test)]
    command.args(&inner.arguments);
    command
        .arg(url)
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
    let mut child = TmuxChild::spawn_checked(&mut command, || {
        operation.check().map_err(LauncherSpawnFailure)
    })
    .map_err(|failure| failure.0)?;
    child.retain_until_reaped(Box::new((admission, inner, operation.checkpoint.clone())));
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
