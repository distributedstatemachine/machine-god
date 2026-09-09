//! Explicit, bounded clipboard process capability. No clipboard discovery or input selection.

use machine_god_core::{BoxFuture, CancellationToken};
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::NativeOwnedWorkerScope;
use crate::terminal::{
    MAX_TERMINAL_ENVIRONMENT_BYTES, MAX_TERMINAL_ENVIRONMENT_ENTRIES,
    MAX_TERMINAL_ENVIRONMENT_KEY_BYTES, MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES,
};

mod process;
#[cfg(test)]
mod tests;

const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

/// Fixed errors contain no copied text, paths, environment, or process diagnostics.
/// After process launch, failure does not imply that the clipboard is unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeClipboardError {
    /// The explicit path or environment is invalid.
    InvalidAuthority,
    /// A bounded payload or environment exceeds its limit.
    ResourceLimit,
    /// This shared capability still owns a prior operation or its child cleanup.
    Busy,
    /// Executable, cwd, worker, process spawn, or exit observation is unavailable.
    Unavailable,
    /// The caller cancelled or abandoned this operation.
    Cancelled,
    /// The independent operation deadline expired; cleanup remains owned.
    TimedOut,
    /// The exclusively owned stdin pipe could not accept the complete payload.
    WriteFailed,
    /// The direct child exited nonzero or was terminated by a signal.
    ExitFailed,
}

impl fmt::Display for NativeClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidAuthority => "clipboard authority is invalid",
            Self::ResourceLimit => "clipboard resource limit exceeded",
            Self::Busy => "clipboard operation or cleanup is still active",
            Self::Unavailable => "clipboard process is unavailable",
            Self::Cancelled => "clipboard copy was cancelled",
            Self::TimedOut => "clipboard copy timed out",
            Self::WriteFailed => "clipboard input write failed",
            Self::ExitFailed => "clipboard process did not exit successfully",
        })
    }
}
impl std::error::Error for NativeClipboardError {}

/// Explicit program spelling and retained executable authority.
/// The host reserves the installation against replacement for the capability's lifetime.
#[derive(Clone)]
pub struct NativeClipboardExecutable {
    program: Arc<PathBuf>,
    retained: Arc<File>,
    #[cfg(test)]
    arguments: Option<Arc<Vec<OsString>>>,
}

impl fmt::Debug for NativeClipboardExecutable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeClipboardExecutable")
            .finish_non_exhaustive()
    }
}
impl NativeClipboardExecutable {
    /// Inert: validates spelling without opening, inspecting, or executing it.
    ///
    /// # Errors
    /// Rejects nonabsolute, oversized, NUL-containing, or parent-relative paths.
    pub fn new(program: &Path, retained: File) -> Result<Self, NativeClipboardError> {
        validate_path(program)?;
        Ok(Self {
            program: Arc::new(program.to_owned()),
            retained: Arc::new(retained),
            #[cfg(test)]
            arguments: None,
        })
    }
}

/// Shared clipboard capability. Clones share one operation/actual-reap admission.
#[derive(Clone)]
pub struct NativeClipboard {
    inner: Arc<ClipboardInner>,
}

struct ClipboardInner {
    executable: NativeClipboardExecutable,
    working_directory: PathBuf,
    environment: Vec<(OsString, OsString)>,
    scope: NativeOwnedWorkerScope,
    active: Arc<AtomicBool>,
    #[cfg(test)]
    probe: Option<Arc<tests::Probe>>,
}

impl fmt::Debug for NativeClipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeClipboard").finish_non_exhaustive()
    }
}

impl NativeClipboard {
    /// Inert construction using explicit cwd, environment, and host worker scope.
    /// No environment variables, display services, or executable paths are discovered.
    ///
    /// # Errors
    /// Rejects invalid cwd spelling, duplicate/invalid environment keys and exceeded bounds.
    pub fn new(
        executable: NativeClipboardExecutable,
        working_directory: PathBuf,
        mut environment: Vec<(OsString, OsString)>,
        scope: NativeOwnedWorkerScope,
    ) -> Result<Self, NativeClipboardError> {
        validate_path(&working_directory)?;
        validate_environment(&mut environment)?;
        Ok(Self {
            inner: Arc::new(ClipboardInner {
                executable,
                working_directory,
                environment,
                scope,
                active: Arc::new(AtomicBool::new(false)),
                #[cfg(test)]
                probe: None,
            }),
        })
    }

    /// Copies the exact UTF-8 bytes, without separators, newline, or terminator.
    /// Inert until first poll. Dropping a polled future cancels its private job,
    /// never the caller's token. The injected scope retains actual worker/reap
    /// completion independently of this response. No failed copy promises rollback.
    ///
    /// # Errors
    /// Rejects oversize, cancelled, busy or unavailable operations; reports write,
    /// deadline and direct-child exit failures without exposing payload data.
    #[must_use]
    pub fn copy(
        &self,
        text: Arc<str>,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<(), NativeClipboardError>> {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(NativeClipboardError::Cancelled);
            }
            if text.len() > crate::MAX_FILE_SESSION_BYTES {
                return Err(NativeClipboardError::ResourceLimit);
            }
            inner
                .active
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| NativeClipboardError::Busy)?;
            let admission = Admission(Arc::clone(&inner.active));
            let abandoned = CancellationToken::new();
            let guard = CancelOnDrop(abandoned.clone());
            let deadline = Instant::now() + OPERATION_TIMEOUT;
            let scope = inner.scope.clone();
            let result = scope
                .run(move || process::copy(&inner, &text, &cancel, &abandoned, deadline, admission))
                .await
                .map_err(|_| NativeClipboardError::Unavailable)?;
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

fn validate_path(path: &Path) -> Result<(), NativeClipboardError> {
    if path.as_os_str().len() > 4096
        || !path.is_absolute()
        || path.as_os_str().as_bytes().contains(&0)
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(NativeClipboardError::InvalidAuthority);
    }
    Ok(())
}

fn validate_environment(entries: &mut [(OsString, OsString)]) -> Result<(), NativeClipboardError> {
    if entries.len() > MAX_TERMINAL_ENVIRONMENT_ENTRIES {
        return Err(NativeClipboardError::ResourceLimit);
    }
    let mut total = 0_usize;
    for (key, value) in entries.iter() {
        let key = key.as_os_str().as_bytes();
        let value = value.as_os_str().as_bytes();
        if key.is_empty() || key.contains(&b'=') || key.contains(&0) || value.contains(&0) {
            return Err(NativeClipboardError::InvalidAuthority);
        }
        total = total
            .checked_add(key.len())
            .and_then(|n| n.checked_add(value.len()))
            .ok_or(NativeClipboardError::ResourceLimit)?;
        if key.len() > MAX_TERMINAL_ENVIRONMENT_KEY_BYTES
            || value.len() > MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES
            || total > MAX_TERMINAL_ENVIRONMENT_BYTES
        {
            return Err(NativeClipboardError::ResourceLimit);
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(NativeClipboardError::InvalidAuthority);
    }
    Ok(())
}
