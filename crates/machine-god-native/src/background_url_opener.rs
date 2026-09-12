//! Explicit desktop URL handoff with bounded, host-owned direct-child cleanup.

#[cfg(any(test, feature = "ai-gateway-http"))]
use crate::NativeOwnedWorkerScope;
#[cfg(any(test, feature = "ai-gateway-http"))]
use crate::background_commands::url::BackgroundServerUrl;
#[cfg(any(test, feature = "ai-gateway-http"))]
use machine_god_core::{BoxFuture, CancellationToken};
#[cfg(any(test, feature = "ai-gateway-http"))]
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Component, PathBuf};
use std::sync::Arc;

pub(crate) mod launcher;
#[cfg(any(test, feature = "ai-gateway-http"))]
use launcher::{LauncherUrl, OwnedUrlLauncher};

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
#[cfg(any(test, feature = "ai-gateway-http"))]
pub struct NativeBackgroundUrlOpener {
    launcher: OwnedUrlLauncher,
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

#[cfg(any(test, feature = "ai-gateway-http"))]
impl fmt::Debug for NativeBackgroundUrlOpener {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeBackgroundUrlOpener")
            .finish_non_exhaustive()
    }
}

#[cfg(any(test, feature = "ai-gateway-http"))]
impl NativeBackgroundUrlOpener {
    /// Shares the already-bound launcher and its admission without recapturing
    /// executable, environment or worker ownership. This does not grant consent.
    #[must_use]
    pub(crate) fn mcp_launcher(&self) -> crate::mcp::browser_launcher::NativeMcpBrowserLauncher {
        crate::mcp::browser_launcher::NativeMcpBrowserLauncher::from_shared_launcher(
            self.launcher.clone(),
        )
    }

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
        Ok(Self {
            launcher: OwnedUrlLauncher::new(executable, environment, workers)?,
        })
    }
    /// Inert until polled; the native target supplies the retained owner token.
    pub(crate) fn open(
        &self,
        url: BackgroundServerUrl,
        cancellation: CancellationToken,
        revoked: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeBackgroundOpenOutcome, NativeBackgroundOpenError>> {
        self.launcher
            .open(LauncherUrl::Background(url), cancellation, revoked, None)
    }
}
#[cfg(test)]
mod mcp_composition_tests;
#[cfg(test)]
mod tests;
