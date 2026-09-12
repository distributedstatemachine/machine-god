//! Explicit owned browser handoff. URL admission and launch receipts are not consent.

use crate::NativeOwnedWorkerScope;
use crate::background_url_opener::{
    NativeBackgroundOpenError, NativeBackgroundOpenOutcome, NativeBackgroundUrlExecutable,
    launcher::{LauncherUrl, OwnedUrlLauncher},
};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{ffi::OsString, fmt, time::Instant};

const MAX_URL_BYTES: usize = 32 * 1024;

/// Bounded HTTP(S) data, not permission, authentication or continuation authority.
#[derive(Clone)]
pub struct NativeMcpBrowserUrl(Box<str>);

impl NativeMcpBrowserUrl {
    /// Admits a URL without network, filesystem or browser effects.
    ///
    /// # Errors
    /// Rejects oversized URLs, non-HTTP(S) schemes, missing hosts, credentials,
    /// backslashes and whitespace/control characters. Retains the exact input.
    pub fn new(value: &str) -> Result<Self, NativeMcpBrowserLaunchError> {
        if value.len() > MAX_URL_BYTES
            || value
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control() || ch == '\\')
        {
            return Err(NativeMcpBrowserLaunchError::InvalidUrl);
        }
        let parsed = url::Url::parse(value).map_err(|_| NativeMcpBrowserLaunchError::InvalidUrl)?;
        let Some((scheme, rest)) = value.split_once("://") else {
            return Err(NativeMcpBrowserLaunchError::InvalidUrl);
        };
        if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
            || parsed.host().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || rest
                .split(['/', '?', '#'])
                .next()
                .is_none_or(|authority| authority.is_empty() || authority.contains('@'))
        {
            return Err(NativeMcpBrowserLaunchError::InvalidUrl);
        }
        Ok(Self(value.into()))
    }

    /// Borrows observed URL data; callers must apply their own display escaping.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for NativeMcpBrowserUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeMcpBrowserUrl")
            .finish_non_exhaustive()
    }
}

/// Fixed redacted failures before a launcher handoff can be established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpBrowserLaunchError {
    InvalidUrl,
    InvalidAuthority,
    Busy,
    Cancelled,
    TimedOut,
    Unavailable,
}
impl fmt::Display for NativeMcpBrowserLaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MCP browser launcher unavailable")
    }
}
impl std::error::Error for NativeMcpBrowserLaunchError {}

impl From<NativeBackgroundOpenError> for NativeMcpBrowserLaunchError {
    fn from(error: NativeBackgroundOpenError) -> Self {
        match error {
            NativeBackgroundOpenError::InvalidAuthority => Self::InvalidAuthority,
            NativeBackgroundOpenError::Busy => Self::Busy,
            NativeBackgroundOpenError::Cancelled => Self::Cancelled,
            NativeBackgroundOpenError::TimedOut => Self::TimedOut,
            NativeBackgroundOpenError::Unavailable => Self::Unavailable,
        }
    }
}

/// A direct-child observation, never OAuth success or completion/retry proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpBrowserLaunchOutcome {
    /// The direct launcher exited successfully; browser lifetime is unknown.
    Opened,
    /// The launcher failed; a browser may nevertheless have opened.
    LauncherFailed,
    /// A successful handoff could not be established; an effect may have occurred.
    Indeterminate,
}

/// Explicit retained launcher, captured environment and actual host worker owner.
/// Clones share one admission until the direct child is actually reaped.
#[derive(Clone)]
pub struct NativeMcpBrowserLauncher {
    launcher: OwnedUrlLauncher,
}

impl fmt::Debug for NativeMcpBrowserLauncher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeMcpBrowserLauncher")
            .finish_non_exhaustive()
    }
}

impl NativeMcpBrowserLauncher {
    /// Retains an actual existing launcher allocation without acquiring effects.
    #[cfg(any(test, feature = "ai-gateway-http"))]
    #[must_use]
    pub(crate) fn from_shared_launcher(launcher: OwnedUrlLauncher) -> Self {
        Self { launcher }
    }

    /// Inert binding; never discovers an executable or reads the environment.
    /// The host must protect the retained executable installation until cleanup.
    ///
    /// # Errors
    /// Rejects invalid or oversized captured environment entries.
    pub fn new(
        executable: NativeBackgroundUrlExecutable,
        environment: Vec<(OsString, OsString)>,
        workers: NativeOwnedWorkerScope,
    ) -> Result<Self, NativeMcpBrowserLaunchError> {
        Ok(Self {
            launcher: OwnedUrlLauncher::new(executable, environment, workers)?,
        })
    }

    /// Inert until polled. The host must obtain consent separately before polling.
    /// Original caller/owner cancellation remains checked through queued work,
    /// pre-spawn and observation. The supplied deadline is capped to ten seconds
    /// from first poll, using the native direct-child monotonic clock.
    /// No automatic retry occurs. Dropping a polled future retains owned cleanup.
    #[must_use]
    pub fn launch(
        &self,
        url: NativeMcpBrowserUrl,
        cancellation: CancellationToken,
        owner: CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'static, Result<NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLaunchError>>
    {
        let operation =
            self.launcher
                .open(LauncherUrl::Mcp(url.0), cancellation, owner, Some(deadline));
        Box::pin(async move {
            Ok(match operation.await? {
                NativeBackgroundOpenOutcome::Opened => NativeMcpBrowserLaunchOutcome::Opened,
                NativeBackgroundOpenOutcome::LauncherFailed => {
                    NativeMcpBrowserLaunchOutcome::LauncherFailed
                }
                NativeBackgroundOpenOutcome::Indeterminate => {
                    NativeMcpBrowserLaunchOutcome::Indeterminate
                }
            })
        })
    }
}

#[cfg(test)]
mod tests;
