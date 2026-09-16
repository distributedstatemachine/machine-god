//! Owned, caller-polled activation of one explicitly selected MCP profile.

#[cfg(feature = "mcp-http")]
mod authentication;
#[cfg(all(feature = "mcp-http", any(test, feature = "ai-gateway-http")))]
pub(crate) use authentication::{ControlFence, Selection as NativeMcpAuthSelection};
#[cfg(feature = "mcp-http")]
pub use authentication::{NativeMcpAuthenticationError, NativeMcpAuthenticationReceipt};
mod catalog;
mod cleanup;
mod configuration;
mod operation;
mod readiness;
mod state;
#[cfg(test)]
mod tests;

use super::{
    lifetime::McpPeerLifetime,
    management::NativeMcpManagementService,
    runtime::{NativeMcpRuntime, NativeMcpRuntimeClock, NativeMcpRuntimeError},
    startup::{NativeMcpStartupError, NativeMcpStartupPhase, NativeMcpStartupReceipt},
    stdio_startup::NativeMcpStdioStartup,
    store::NativeMcpConfigStoreError,
};
use crate::{NativeOwnedWorkerScope, background_process::ValidatedBackgroundEnvironment};
use machine_god_core::{BoxFuture, CancellationToken, ToolName};
use std::{
    ffi::OsString,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

/// Reusable captured startup authorities. Configuration and its lifetime token
/// come from each exact native store observation, never from this template.
#[derive(Clone)]
pub struct NativeMcpControllerStartupOptions {
    pub captured_environment: Vec<(OsString, OsString)>,
    pub stdio: Option<Arc<NativeMcpStdioStartup>>,
    pub clock: Arc<dyn NativeMcpRuntimeClock>,
    pub catalog_epoch: Instant,
    pub owner_cancellation: CancellationToken,
    #[cfg(feature = "mcp-http")]
    pub network: Option<Arc<super::network::NativeMcpNetwork>>,
    #[cfg(feature = "mcp-http")]
    pub authentication: Vec<super::startup::NativeMcpStartupAuthentication>,
    pub peer_lifetime: McpPeerLifetime,
    pub max_retained_bytes: usize,
}

/// Exact host-owned allocations. The controller never creates another worker
/// scope, archive, store, runtime or ambient environment/resolver selection.
pub struct NativeMcpControllerOptions {
    pub runtime: Arc<NativeMcpRuntime>,
    pub management: Arc<NativeMcpManagementService>,
    pub workers: NativeOwnedWorkerScope,
    pub reserved_tool_names: Box<[ToolName]>,
    pub startup: NativeMcpControllerStartupOptions,
    /// One explicitly owned profile credential service. Each loaded configuration
    /// selects stored credentials for its remote servers unless a caller supplied
    /// an exact per-server override. Controller close also closes this service.
    #[cfg(feature = "mcp-http")]
    pub stored_authentication: Option<Arc<super::auth::NativeMcpAuthService>>,
    /// Includes active, pending, retired and outstanding result generations.
    /// Positive and at most eight; four is suitable for an ordinary host.
    pub max_retained_generations: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpControllerError {
    Invalid,
    Busy,
    Limit,
    Closed,
    Cancelled,
    Deadline,
    Unavailable,
    Store(NativeMcpConfigStoreError),
    Startup(NativeMcpStartupError),
    Runtime(NativeMcpRuntimeError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpControllerPublication {
    Published,
    Unchanged,
}

/// A point-in-time observation, not continuing runtime authority. Retaining an
/// outcome retains its bounded generation reservation and cleanup observations.
#[derive(Clone)]
pub struct NativeMcpControllerReceipt {
    data: state::Receipt,
    generation: Arc<state::Generation>,
}
impl NativeMcpControllerReceipt {
    #[must_use]
    pub fn startup(&self) -> Option<&NativeMcpStartupReceipt> {
        self.data.startup.as_ref()
    }
    #[must_use]
    pub const fn publication(&self) -> NativeMcpControllerPublication {
        self.data.publication
    }
    /// True only if close was observed after this operation published. False
    /// does not promise that a later close or reload cannot retire the result.
    #[must_use]
    pub const fn closed_after_publication(&self) -> bool {
        self.data.closed
    }
    #[must_use]
    pub fn cleanup_complete(&self) -> bool {
        self.generation.cleanup_complete()
    }
}

#[derive(Clone)]
pub struct NativeMcpControllerFailure {
    data: state::Failure,
    generation: Option<Arc<state::Generation>>,
}
impl NativeMcpControllerFailure {
    #[must_use]
    pub const fn kind(&self) -> NativeMcpControllerError {
        self.data.kind
    }
    #[must_use]
    pub fn startup(&self) -> Option<&NativeMcpStartupReceipt> {
        self.data.startup.as_ref()
    }
    #[must_use]
    /// True only with a retained generation's complete cleanup evidence.
    /// Pre-admission and controller-wide settlement errors provide no such
    /// evidence and conservatively return false.
    pub fn cleanup_complete(&self) -> bool {
        self.generation
            .as_ref()
            .is_some_and(|generation| generation.cleanup_complete())
    }
}
impl fmt::Display for NativeMcpControllerFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("native MCP controller operation failed")
    }
}
impl std::error::Error for NativeMcpControllerFailure {}

/// Local cleanup observation; it never asserts HTTP DELETE or token revocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeMcpControllerCleanup {
    pub complete: bool,
    pub pending_generations: usize,
    pub pending_peers: usize,
}

pub struct NativeMcpController {
    inner: Arc<state::Inner>,
}
type Result<T> = std::result::Result<T, NativeMcpControllerFailure>;

impl NativeMcpController {
    /// Managed admission may initialize a never-started controller once. A
    /// retained failed generation still requires an explicit reload, not retry.
    pub(crate) fn needs_initial_startup(&self) -> bool {
        !state::lock(&self.inner.state).activation_attempted
    }

    pub(crate) fn activation_failure(&self) -> Option<NativeMcpControllerError> {
        state::lock(&self.inner.state).activation_failure
    }

    /// Settles only an unpublished failed initial activation. It preserves the
    /// open controller and credential service for explicit interactive repair.
    #[cfg(any(test, feature = "ai-gateway-http"))]
    pub(crate) fn settle_failed_startup(
        &self,
        deadline: Instant,
        cancellation: CancellationToken,
        completion: Option<crate::NativeOwnedWorkerCompletion>,
    ) -> BoxFuture<'static, Result<NativeMcpControllerCleanup>> {
        cleanup::settle_failed_startup(
            Arc::downgrade(&self.inner),
            deadline,
            cancellation,
            completion,
        )
    }
    #[cfg(all(feature = "mcp-http", any(test, feature = "ai-gateway-http")))]
    pub(crate) fn prepare_authentication(
        &self,
        server: String,
        fence: Arc<authentication::ControlFence>,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'static, Result<authentication::Selection>> {
        authentication::prepare(
            Arc::downgrade(&self.inner),
            server,
            fence,
            cancellation,
            deadline,
        )
    }
    /// Shares this owner's exact credential coordinator without loading records,
    /// refreshing, opening a browser or extending engine-drop authority.
    #[cfg(feature = "mcp-http")]
    #[must_use]
    pub fn authentication_service(&self) -> Option<Arc<super::auth::NativeMcpAuthService>> {
        self.inner.options.stored_authentication.clone()
    }
    pub(crate) fn selects_runtime(&self, runtime: &NativeMcpRuntime) -> bool {
        std::ptr::eq(self.inner.options.runtime.as_ref(), runtime)
    }

    /// Observes the explicitly selected monotonic clock now, not at construction.
    /// Call from the first operation poll. This remains available after close so
    /// a caller can select a fresh bounded cleanup deadline.
    /// # Errors
    /// Rejects zero before observing time, and unrepresentable deadline addition.
    pub fn deadline_after(&self, timeout: Duration) -> Result<Instant> {
        if timeout.is_zero() {
            return Err(state::failure(NativeMcpControllerError::Invalid));
        }
        self.inner
            .options
            .startup
            .clock
            .now()
            .checked_add(timeout)
            .ok_or_else(|| state::failure(NativeMcpControllerError::Limit))
    }

    /// Construction is inert, including no clock read or worker admission.
    /// # Errors
    /// Rejects invalid resource limits, duplicate reserved names and malformed
    /// captured environment. Per-server authentication is checked after loading.
    pub fn new(options: NativeMcpControllerOptions) -> Result<Self> {
        if !(1..=8).contains(&options.max_retained_generations)
            || !(1..=256 * 1024 * 1024).contains(&options.startup.max_retained_bytes)
            || options.reserved_tool_names.len() > 4096
        {
            return Err(state::failure(NativeMcpControllerError::Limit));
        }
        let mut names = std::collections::BTreeSet::new();
        for name in &options.reserved_tool_names {
            if !names.insert(name.as_str()) {
                return Err(state::failure(NativeMcpControllerError::Invalid));
            }
        }
        ValidatedBackgroundEnvironment::new(options.startup.captured_environment.clone())
            .map_err(|_| state::failure(NativeMcpControllerError::Invalid))?;
        Ok(Self {
            inner: Arc::new(state::Inner::new(options)),
        })
    }

    /// Activates initial All or required-only Ask startup once.
    /// # Errors
    /// Rejects deferred phase, repeated startup, competing mutations, cancellation,
    /// stale observations and failed required-server admission.
    #[must_use]
    pub fn start(
        &self,
        phase: NativeMcpStartupPhase,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'static, Result<NativeMcpControllerReceipt>> {
        operation::request(
            Arc::downgrade(&self.inner),
            state::Kind::Start(phase),
            cancellation,
            Some(deadline),
        )
    }

    /// Full all-selected replacement; failure preserves the old active generation.
    /// # Errors
    /// Rejects competing mutations, cancellation, stale source/publication and
    /// any selected-server failure, including optional servers.
    #[must_use]
    pub fn reload(
        &self,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'static, Result<NativeMcpControllerReceipt>> {
        operation::request(
            Arc::downgrade(&self.inner),
            state::Kind::Reload,
            cancellation,
            Some(deadline),
        )
    }

    /// Coalesces optional Ask discovery. Cancelling one waiter never cancels
    /// another; close or the original loader deadline still bounds the job.
    /// # Errors
    /// Rejects missing/closed startup, stale configuration/publication, competing
    /// mutations and cancelled waits. One attempt is retained per Ask generation.
    #[must_use]
    pub fn activate_deferred(
        &self,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'static, Result<NativeMcpControllerReceipt>> {
        operation::request(
            Arc::downgrade(&self.inner),
            state::Kind::Deferred,
            cancellation,
            Some(deadline),
        )
    }

    /// Activates initial startup using configured per-attempt budgets without an
    /// overall startup cap. Housekeeping remains independently time-bounded.
    /// # Errors
    /// Preserves the same admission, source and required-readiness errors as start.
    #[must_use]
    pub fn start_configured(
        &self,
        phase: NativeMcpStartupPhase,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeMcpControllerReceipt>> {
        operation::request(
            Arc::downgrade(&self.inner),
            state::Kind::Start(phase),
            cancellation,
            None,
        )
    }

    /// Replaces all selected peers with configured per-attempt/restart budgets.
    /// # Errors
    /// Failed replacement preserves the old active generation, as with reload.
    #[must_use]
    pub fn reload_configured(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeMcpControllerReceipt>> {
        operation::request(
            Arc::downgrade(&self.inner),
            state::Kind::Reload,
            cancellation,
            None,
        )
    }

    /// Coalesces deferred discovery without imposing an overall loader timeout.
    /// Cancelling a waiter does not cancel the shared owner-controlled loader.
    /// # Errors
    /// Preserves deferred admission, exact-generation and configured-attempt errors.
    #[must_use]
    pub fn activate_deferred_configured(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeMcpControllerReceipt>> {
        operation::request(
            Arc::downgrade(&self.inner),
            state::Kind::Deferred,
            cancellation,
            None,
        )
    }

    /// Refreshes expiring authentication before a new operation selects peers.
    /// Reuses the exact active profile, with configured per-attempt budgets;
    /// never activates saved changes or replays a previously selected request.
    /// Concurrent callers share one owner-controlled refresh job.
    /// # Errors
    /// Rejects revoked credentials, changed configuration, competing mutations,
    /// unavailable startup and cancellation. A failed credential refresh may
    /// leave the original publication present but no longer authenticated.
    #[must_use]
    pub fn refresh_authentication_configured(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeMcpControllerReceipt>> {
        operation::request(
            Arc::downgrade(&self.inner),
            state::Kind::Refresh,
            cancellation,
            None,
        )
    }

    /// The principal owner calls this only after its admission or turn has ended.
    /// Cancelling a public refresh observer alone must
    /// not cancel a shared job; this owner boundary instead retires that abandoned
    /// preparation and drives its original future so private peer custody drops.
    /// The published runtime and authentication service remain usable.
    pub(crate) fn settle_abandoned_preparation(&self) -> BoxFuture<'static, ()> {
        let inner = Arc::downgrade(&self.inner);
        Box::pin(async move {
            let Some(inner) = inner.upgrade() else {
                return;
            };
            let running = {
                let state = state::lock(&inner.state);
                state
                    .running
                    .as_ref()
                    .filter(|job| matches!(job.kind, state::Kind::Refresh | state::Kind::Deferred))
                    .cloned()
            };
            if let Some(running) = running {
                if running.kind == state::Kind::Refresh {
                    running.cancellation.cancel();
                }
                // Deferred activation belongs to the published generation and
                // caches its result. Drive its configured attempt to completion
                // rather than poisoning later work with an owner-made cancel.
                // Poll outside the state lock: cancellation and future drops
                // may wake caller code or release process/TLS ownership.
                let _ = running.future.await;
                inner.release_completed();
            }
        })
    }

    /// Irrevocable cutoff, not a reap/socket-completion receipt. No locks are
    /// held while cancellation or runtime retirement wakes caller code.
    pub fn close(&self) {
        self.inner.close();
    }

    /// Closes, drives retained abandoned work, then drains/observes local peers.
    /// It does not close the host's shared worker scope. Use a separate cleanup
    /// token, not an already-cancelled model turn token.
    /// # Errors
    /// Rejects overlapping settlement and cancellation/deadline exhaustion;
    /// undrained ownership stays retained for another explicitly owned attempt.
    #[must_use]
    pub fn settle(
        &self,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeMcpControllerCleanup>> {
        cleanup::settle(Arc::downgrade(&self.inner), deadline, cancellation)
    }
}
impl Drop for NativeMcpController {
    fn drop(&mut self) {
        self.close();
    }
}

macro_rules! redacted { ($($ty:ty),+ $(,)?) => {$(
    impl fmt::Debug for $ty {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(concat!(stringify!($ty), " { <redacted> }"))
        }
    }
)+}; }
redacted!(
    NativeMcpController,
    NativeMcpControllerOptions,
    NativeMcpControllerStartupOptions,
    NativeMcpControllerReceipt,
    NativeMcpControllerFailure
);
