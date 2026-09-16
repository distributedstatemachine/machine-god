//! Explicit startup capture, separate from inert host/options construction.

use super::{
    NativeReferenceHostBuildError, NativeReferenceHostMcpEphemeralStartupOptions,
    NativeReferenceHostMcpOptions, error,
};
use crate::{
    NativeReferenceHostTerminalOptions, PreparedNativeRoots, TERMINAL_CAPTURED_HELPER_ARGUMENT,
    mcp::{
        clock::TokioMcpClock,
        context::NativeMcpContexts,
        controller::NativeMcpControllerStartupOptions,
        ephemeral::NativeMcpNetworkRequirement,
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

#[derive(Clone, Copy)]
enum CaptureOwner {
    Profile,
    Ephemeral,
}

impl NativeReferenceHostMcpOptions {
    /// Explicitly captures only replacement ACP transport authority on an owned
    /// startup worker. Retained host helper/environment/workspace inputs do not
    /// need recapturing. Empty and stdio selections acquire no network inputs.
    /// # Errors
    /// Invalid bundled trust or network configuration; unavailable capture stays
    /// absent and required peers subsequently fail normal readiness admission.
    pub fn capture_ephemeral_network(
        requirement: NativeMcpNetworkRequirement,
    ) -> Result<Option<Arc<NativeMcpNetwork>>, NativeReferenceHostBuildError> {
        captured_network(
            network_inputs(requirement),
            Arc::new(TokioMcpClock),
            CancellationToken::new(),
            bundled_trust,
        )
    }

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
        Self::from_captured_startup(
            roots,
            terminal,
            contexts,
            network_inputs(NativeMcpNetworkRequirement::SystemDns),
        )
    }

    /// Captures a dedicated ACP session's production transport authority with
    /// no profile management, stored credential or OAuth-service selection.
    /// Omitted/empty client configuration is still an authoritative selection
    /// published later through the exact composed ephemeral owner.
    /// The admitted selection's requirement skips unused network capture; literal
    /// HTTP peers retain fresh secure entropy and bundled TLS trust without DNS.
    /// # Errors
    /// Same retained-root/process/trust validation as `capture_startup`.
    pub fn capture_ephemeral_startup(
        roots: &PreparedNativeRoots,
        terminal: &NativeReferenceHostTerminalOptions,
        contexts: Arc<NativeMcpContexts>,
        requirement: NativeMcpNetworkRequirement,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        Self::from_captured_ephemeral_startup(
            roots,
            terminal,
            contexts,
            network_inputs(requirement),
        )
    }

    pub(super) fn from_captured_startup(
        roots: &PreparedNativeRoots,
        terminal: &NativeReferenceHostTerminalOptions,
        contexts: Arc<NativeMcpContexts>,
        network_inputs: Option<(McpResolverConfig, [u8; 32])>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        Self::capture(
            roots,
            terminal,
            contexts,
            network_inputs,
            CaptureOwner::Profile,
        )
    }

    pub(super) fn from_captured_ephemeral_startup(
        roots: &PreparedNativeRoots,
        terminal: &NativeReferenceHostTerminalOptions,
        contexts: Arc<NativeMcpContexts>,
        network_inputs: Option<(McpResolverConfig, [u8; 32])>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        Self::capture(
            roots,
            terminal,
            contexts,
            network_inputs,
            CaptureOwner::Ephemeral,
        )
    }

    fn capture(
        roots: &PreparedNativeRoots,
        terminal: &NativeReferenceHostTerminalOptions,
        contexts: Arc<NativeMcpContexts>,
        network_inputs: Option<(McpResolverConfig, [u8; 32])>,
        owner_kind: CaptureOwner,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        let clock = Arc::new(TokioMcpClock);
        let owner = CancellationToken::new();
        let environment = terminal.environment.entries().to_vec();
        let stdio = NativeMcpStdioStartup::new(
            terminal.helper_program.clone(),
            vec![TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
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
        let network =
            captured_network(network_inputs, clock.clone(), owner.clone(), bundled_trust)?;
        if matches!(owner_kind, CaptureOwner::Ephemeral) {
            let startup = NativeReferenceHostMcpEphemeralStartupOptions {
                captured_environment: environment,
                stdio: Some(Arc::new(stdio)),
                clock: clock.clone(),
                catalog_epoch: clock.now(),
                owner_cancellation: owner,
                network,
                peer_lifetime: McpPeerLifetime::OwnerControlled,
                max_retained_bytes: 256 * 1024 * 1024,
                max_retained_generations: 4,
            };
            return Self::new(contexts, clock).with_ephemeral_startup(startup);
        }
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

fn network_inputs(
    requirement: NativeMcpNetworkRequirement,
) -> Option<(McpResolverConfig, [u8; 32])> {
    network_inputs_with(
        requirement,
        || McpResolverConfig::capture_system().ok(),
        || {
            let mut key = [0; 32];
            getrandom::fill(&mut key).ok()?;
            Some(key)
        },
    )
}

fn network_inputs_with(
    requirement: NativeMcpNetworkRequirement,
    capture_resolver: impl FnOnce() -> Option<McpResolverConfig>,
    capture_entropy: impl FnOnce() -> Option<[u8; 32]>,
) -> Option<(McpResolverConfig, [u8; 32])> {
    let resolver = match requirement {
        NativeMcpNetworkRequirement::None => return None,
        NativeMcpNetworkRequirement::LiteralOnly => McpResolverConfig::literal_only(),
        NativeMcpNetworkRequirement::SystemDns => capture_resolver()?,
    };
    Some((resolver, capture_entropy()?))
}

fn captured_network(
    inputs: Option<(McpResolverConfig, [u8; 32])>,
    clock: Arc<TokioMcpClock>,
    owner: CancellationToken,
    capture_trust: impl FnOnce() -> Result<McpHttpTrust, NativeReferenceHostBuildError>,
) -> Result<Option<Arc<NativeMcpNetwork>>, NativeReferenceHostBuildError> {
    inputs
        .map(|(resolver, key)| {
            NativeMcpNetwork::new(resolver, key, Some(capture_trust()?), clock, owner, 32)
                .map(Arc::new)
                .map_err(|_| error())
        })
        .transpose()
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

#[cfg(test)]
mod tests;
