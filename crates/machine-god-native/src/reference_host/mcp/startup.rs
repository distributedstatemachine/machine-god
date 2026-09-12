//! Explicit startup capture, separate from inert host/options construction.

use super::{NativeReferenceHostBuildError, NativeReferenceHostMcpOptions, error};
use crate::{
    NativeReferenceHostTerminalOptions, PreparedNativeRoots, TERMINAL_PTY_HELPER_ARGUMENT,
    mcp::{
        clock::TokioMcpClock,
        context::NativeMcpContexts,
        controller::NativeMcpControllerStartupOptions,
        http::McpHttpTrust,
        lifetime::McpPeerLifetime,
        network::{McpResolverConfig, NativeMcpNetwork},
        protocol::WireLimits,
        runtime::NativeMcpRuntimeClock,
        stdio_startup::NativeMcpStdioStartup,
    },
};
use machine_god_core::CancellationToken;
use std::sync::Arc;

impl NativeReferenceHostMcpOptions {
    /// Explicitly captures production startup authority on the caller's owned
    /// startup worker. Reads system DNS configuration and secure entropy, and
    /// duplicates the already retained workspace descriptor. The selected helper
    /// and environment are reused exactly; workspace paths are never reopened.
    /// No process, socket, browser, runtime or worker is started here.
    ///
    /// Peers live until their actual host/configuration owner cancels them;
    /// individual startup and operation attempts retain their configured bounds.
    /// # Errors
    /// Returns a redacted MCP configuration error for invalid process/root or
    /// bundled trust authority. Unavailable DNS/entropy capture leaves network
    /// authority absent: empty profiles and stdio remain usable, while remote
    /// startup fails through its ordinary per-server policy. No ambient fallback.
    pub fn capture_startup(
        roots: &PreparedNativeRoots,
        terminal: &NativeReferenceHostTerminalOptions,
        contexts: Arc<NativeMcpContexts>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        Self::from_captured_startup(roots, terminal, contexts, network_inputs())
    }

    pub(super) fn from_captured_startup(
        roots: &PreparedNativeRoots,
        terminal: &NativeReferenceHostTerminalOptions,
        contexts: Arc<NativeMcpContexts>,
        network_inputs: Option<(McpResolverConfig, [u8; 32])>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        let clock = Arc::new(TokioMcpClock);
        let owner = CancellationToken::new();
        let environment = terminal.environment.entries().to_vec();
        let stdio = NativeMcpStdioStartup::new(
            terminal.helper_program.clone(),
            vec![TERMINAL_PTY_HELPER_ARGUMENT.into()],
            environment.clone(),
            Arc::new(roots.try_clone_workspace().map_err(|_| error())?.into()),
            WireLimits {
                max_frame_bytes: 16 * 1024 * 1024,
                max_depth: 64,
                max_nodes: 262_144,
            },
        )
        .map_err(|_| error())?;
        #[cfg(target_os = "macos")]
        let stdio = stdio
            .with_process_inventory_service(
                terminal.helper_program.clone(),
                vec![crate::PROCESS_INVENTORY_SERVICE_ARGUMENT.into()],
            )
            .map_err(|_| error())?;
        let network = network_inputs
            .map(|(resolver, key)| {
                NativeMcpNetwork::new(
                    resolver,
                    key,
                    Some(bundled_trust()?),
                    clock.clone(),
                    owner.clone(),
                    32,
                )
                .map(Arc::new)
                .map_err(|_| error())
            })
            .transpose()?;
        let startup = NativeMcpControllerStartupOptions {
            captured_environment: environment,
            stdio: Some(Arc::new(stdio)),
            clock: clock.clone(),
            catalog_epoch: clock.now(),
            owner_cancellation: owner,
            network,
            authentication: Vec::new(),
            peer_lifetime: McpPeerLifetime::OwnerControlled,
            max_retained_bytes: 256 * 1024 * 1024,
        };
        let authentication = super::authentication::Options::captured(clock.clone());
        let mut options = Self::new(contexts, clock).with_controller_startup(startup);
        options.authentication = Some(authentication);
        Ok(options)
    }
}

fn network_inputs() -> Option<(McpResolverConfig, [u8; 32])> {
    let resolver = McpResolverConfig::capture_system().ok()?;
    let mut key = [0; 32];
    getrandom::fill(&mut key).ok()?;
    Some((resolver, key))
}

pub(super) fn bundled_trust() -> Result<McpHttpTrust, NativeReferenceHostBuildError> {
    let mut roots = rustls::RootCertStore::empty();
    let (valid, invalid) =
        roots.add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
    if valid == 0 || invalid != 0 {
        return Err(error());
    }
    McpHttpTrust::new(roots).map_err(|_| error())
}
