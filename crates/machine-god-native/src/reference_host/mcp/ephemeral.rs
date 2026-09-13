//! Explicit session startup selection without profile or credential authority.

use super::{NativeReferenceHostBuildError, NativeReferenceHostMcpOptions, error};
use crate::{
    NativeOwnedWorkerScope,
    mcp::{
        ephemeral::{NativeMcpEphemeralOptions, NativeMcpEphemeralOwner},
        lifetime::McpPeerLifetime,
        runtime::{NativeMcpRuntime, NativeMcpRuntimeClock},
        stdio_startup::NativeMcpStdioStartup,
    },
};
use machine_god_core::{CancellationToken, ToolName};
use std::{ffi::OsString, fmt, sync::Arc, time::Instant};

/// Inert captured inputs for one dedicated ACP session host. Unlike profile
/// startup, this type has no authentication service, stored headers or config
/// store. Configuration is supplied later to the composed exact owner.
#[derive(Clone)]
pub struct NativeReferenceHostMcpEphemeralStartupOptions {
    pub captured_environment: Vec<(OsString, OsString)>,
    pub stdio: Option<Arc<NativeMcpStdioStartup>>,
    pub clock: Arc<dyn NativeMcpRuntimeClock>,
    pub catalog_epoch: Instant,
    pub owner_cancellation: CancellationToken,
    #[cfg(feature = "mcp-http")]
    pub network: Option<Arc<crate::mcp::network::NativeMcpNetwork>>,
    pub peer_lifetime: McpPeerLifetime,
    pub max_retained_bytes: usize,
    pub max_retained_generations: usize,
}

impl NativeReferenceHostMcpOptions {
    /// Selects ephemeral session activation with this host's exact runtime and
    /// workers. Construction does not connect, read a profile or start workers.
    /// # Errors
    /// Rejects a profile startup selection or a different clock allocation.
    /// Composition also rejects subsequently selected profile management/auth.
    pub fn with_ephemeral_startup(
        mut self,
        startup: NativeReferenceHostMcpEphemeralStartupOptions,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        if self.ephemeral.is_some() {
            return Err(error());
        }
        self.ephemeral = Some(startup);
        self.validate_controller(false)?;
        Ok(self)
    }
}

impl NativeReferenceHostMcpEphemeralStartupOptions {
    pub(super) fn compose(
        self,
        runtime: Arc<NativeMcpRuntime>,
        workers: &NativeOwnedWorkerScope,
        reserved_tool_names: &[ToolName],
    ) -> Result<Arc<NativeMcpEphemeralOwner>, NativeReferenceHostBuildError> {
        NativeMcpEphemeralOwner::new(NativeMcpEphemeralOptions {
            runtime,
            workers: workers.clone(),
            reserved_tool_names: reserved_tool_names.into(),
            captured_environment: self.captured_environment,
            stdio: self.stdio,
            clock: self.clock,
            catalog_epoch: self.catalog_epoch,
            owner_cancellation: self.owner_cancellation,
            #[cfg(feature = "mcp-http")]
            network: self.network,
            peer_lifetime: self.peer_lifetime,
            max_retained_bytes: self.max_retained_bytes,
            max_retained_generations: self.max_retained_generations,
        })
        .map(Arc::new)
        .map_err(|_| error())
    }
}

impl fmt::Debug for NativeReferenceHostMcpEphemeralStartupOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeReferenceHostMcpEphemeralStartupOptions { <redacted> }")
    }
}
