//! Explicit, caller-polled construction of unpublished native MCP server batches.

#[cfg(feature = "mcp-http")]
mod authentication;
mod batch;
mod build;
mod control;
mod phase;
#[cfg(test)]
mod tests;

#[cfg(feature = "mcp-http")]
pub use authentication::{
    NativeMcpStartupAuthChallenge, NativeMcpStartupAuthSource, NativeMcpStartupAuthentication,
};
pub use batch::{
    NativeMcpStartupBatch, NativeMcpStartupCompletion, NativeMcpStartupFailure,
    NativeMcpStartupReceipt, NativeMcpStartupRequirement, NativeMcpStartupServerReceipt,
    NativeMcpStartupState,
};
pub use phase::NativeMcpStartupPhase;

use super::lifetime::McpPeerLifetime;
use super::{
    config::McpConfig, runtime::NativeMcpRuntimeClock, stdio_startup::NativeMcpStdioStartup,
};
use crate::{NativeOwnedWorkerScope, background_process::ValidatedBackgroundEnvironment};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    ffi::OsString,
    fmt,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Instant,
};

const MAX_RETAINED_BYTES: usize = 256 * 1024 * 1024;

/// Immutable native authorities, all selected before constructing this service.
/// Supplying a configuration does not imply stdio, network or OAuth authority.
pub struct NativeMcpStartupOptions {
    pub configuration: Arc<McpConfig>,
    pub captured_environment: Vec<(OsString, OsString)>,
    pub stdio: Option<Arc<NativeMcpStdioStartup>>,
    pub workers: NativeOwnedWorkerScope,
    pub clock: Arc<dyn NativeMcpRuntimeClock>,
    /// Shared origin for relative timestamps in all fetched catalogs.
    /// Must not be later than this clock at the first build poll.
    pub catalog_epoch: Instant,
    pub owner_cancellation: CancellationToken,
    pub configuration_cancellation: CancellationToken,
    #[cfg(feature = "mcp-http")]
    pub network: Option<Arc<super::network::NativeMcpNetwork>>,
    #[cfg(feature = "mcp-http")]
    pub authentication: Vec<NativeMcpStartupAuthentication>,
    /// Explicit peer lifetime, independent of each startup/operation deadline.
    pub peer_lifetime: McpPeerLifetime,
    /// Aggregate retained candidate data; positive and at most 256 MiB.
    pub max_retained_bytes: usize,
}

/// Fixed diagnostics; names are separately bounded configuration metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpStartupError {
    Invalid,
    Limit,
    Unavailable,
    Authentication,
    Cancelled,
    Deadline,
    Catalog,
    RequiredUnavailable,
    DeferredBatch,
    CleanupPending,
}
impl fmt::Display for NativeMcpStartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native MCP startup unavailable")
    }
}
impl std::error::Error for NativeMcpStartupError {}
type Result<T> = std::result::Result<T, NativeMcpStartupError>;

/// Pure retained selections. No file, process, resolver, credential or clock
/// acquisition occurs in construction or in creation of an unpolled build.
pub struct NativeMcpStartup {
    configuration: Arc<McpConfig>,
    #[cfg(feature = "mcp-http")]
    environment: ValidatedBackgroundEnvironment,
    stdio: Option<Arc<NativeMcpStdioStartup>>,
    workers: NativeOwnedWorkerScope,
    clock: Arc<control::Clock>,
    catalog_epoch: Instant,
    owner: CancellationToken,
    configuration_generation: CancellationToken,
    #[cfg(feature = "mcp-http")]
    network: Option<Arc<super::network::NativeMcpNetwork>>,
    #[cfg(feature = "mcp-http")]
    authentication: Vec<NativeMcpStartupAuthentication>,
    #[cfg(feature = "mcp-http")]
    challenges: Arc<Mutex<authentication::Challenges>>,
    #[cfg(feature = "mcp-http")]
    authentication_leases: Mutex<Vec<Arc<authentication::RetainedLease>>>,
    lifetime: McpPeerLifetime,
    max_retained_bytes: usize,
    pending: Arc<AtomicBool>,
    cleanup: Mutex<Vec<NativeMcpStartupCompletion>>,
}
impl NativeMcpStartup {
    /// # Errors
    /// Rejects over-budget environment, candidates or ambiguous/foreign auth
    /// selections. Selected missing authorities fail their server only on poll.
    pub fn new(options: NativeMcpStartupOptions) -> Result<Self> {
        if !(1..=MAX_RETAINED_BYTES).contains(&options.max_retained_bytes) {
            return Err(NativeMcpStartupError::Limit);
        }
        let environment = ValidatedBackgroundEnvironment::new(options.captured_environment)
            .map_err(|_| NativeMcpStartupError::Invalid)?;
        #[cfg(not(feature = "mcp-http"))]
        drop(environment);
        #[cfg(feature = "mcp-http")]
        authentication::validate(&options.configuration, &options.authentication)?;
        Ok(Self {
            configuration: options.configuration,
            #[cfg(feature = "mcp-http")]
            environment,
            stdio: options.stdio,
            workers: options.workers,
            clock: Arc::new(control::Clock(options.clock)),
            catalog_epoch: options.catalog_epoch,
            owner: options.owner_cancellation,
            configuration_generation: options.configuration_cancellation,
            #[cfg(feature = "mcp-http")]
            network: options.network,
            #[cfg(feature = "mcp-http")]
            authentication: options.authentication,
            #[cfg(feature = "mcp-http")]
            challenges: Arc::new(Mutex::new(authentication::Challenges::default())),
            #[cfg(feature = "mcp-http")]
            authentication_leases: Mutex::default(),
            lifetime: options.peer_lifetime,
            max_retained_bytes: options.max_retained_bytes,
            pending: Arc::new(AtomicBool::new(false)),
            cleanup: Mutex::new(Vec::new()),
        })
    }

    /// Builds the selected phase sequentially in configuration order. Outcomes
    /// and cleanup observations remain available even on cancellation/failure.
    /// No runtime publication, browser launch or application tool call occurs.
    #[must_use]
    pub fn build(
        &self,
        phase: NativeMcpStartupPhase,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'_, NativeMcpStartupBatch> {
        Box::pin(build::build(self, phase, cancellation, Some(deadline)))
    }

    /// Uses each server's configured attempt/restart budgets without an overall
    /// startup cap. Owner/caller cancellation and an explicit peer expiry still
    /// apply; cleanup stages have independent finite housekeeping deadlines.
    #[must_use]
    pub fn build_configured(
        &self,
        phase: NativeMcpStartupPhase,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, NativeMcpStartupBatch> {
        Box::pin(build::build(self, phase, cancellation, None))
    }

    /// Observes only the original successfully selected credential leases and
    /// their clocks/profile lifetimes. Does not load configuration, access the
    /// credential store, refresh a token or mutate a runtime publication.
    /// # Errors
    /// Rejects retired owner/configuration/credential/profile authority. An
    /// expired credential requests refresh instead of permitting anonymous use.
    pub fn authentication_refresh_due(&self) -> Result<bool> {
        control::check_optional(
            &self.clock,
            &[self.owner.clone(), self.configuration_generation.clone()],
            self.lifetime.deadline(),
        )?;
        #[cfg(feature = "mcp-http")]
        return self.retained_authentication_refresh_due();
        #[cfg(not(feature = "mcp-http"))]
        Ok(false)
    }

    /// Retains observations even when a polled build future is abandoned.
    /// Completed entries are pruned; at most two full server sets remain tracked.
    #[must_use]
    pub fn cleanup_observations(&self) -> Vec<NativeMcpStartupCompletion> {
        let mut cleanup = self
            .cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cleanup.retain(|entry| !entry.is_complete());
        cleanup.clone()
    }

    fn record_server(&self, completion: NativeMcpStartupCompletion) -> Result<()> {
        let mut cleanup = self
            .cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cleanup.retain(|entry| !entry.is_complete());
        if cleanup.len() == 2 * super::config::MAX_SERVERS {
            return Err(NativeMcpStartupError::CleanupPending);
        }
        cleanup.push(completion);
        Ok(())
    }
}

macro_rules! redacted {
    ($($ty:ty),+ $(,)?) => {$(
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($ty), " { <redacted> }"))
            }
        }
    )+};
}
redacted!(NativeMcpStartupOptions, NativeMcpStartup);
