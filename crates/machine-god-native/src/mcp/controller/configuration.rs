//! Inert startup construction shared by activation and explicit auth selection.

use super::{NativeMcpControllerOptions, state::Failure};
use crate::mcp::{
    startup::{NativeMcpStartup, NativeMcpStartupOptions},
    store::NativeMcpConfigSnapshot,
};
use machine_god_core::CancellationToken;
use std::sync::Arc;

pub(super) fn startup(
    options: &NativeMcpControllerOptions,
    snapshot: &Arc<NativeMcpConfigSnapshot>,
    cancellation: CancellationToken,
) -> Result<Arc<NativeMcpStartup>, Failure> {
    let selected = &options.startup;
    Ok(Arc::new(NativeMcpStartup::new(NativeMcpStartupOptions {
        configuration: Arc::new(snapshot.config().clone()),
        captured_environment: selected.captured_environment.clone(),
        stdio: selected.stdio.clone(),
        workers: options.workers.clone(),
        clock: selected.clock.clone(),
        catalog_epoch: selected.catalog_epoch,
        owner_cancellation: selected.owner_cancellation.clone(),
        configuration_cancellation: cancellation.clone(),
        #[cfg(feature = "mcp-http")]
        network: selected.network.clone(),
        #[cfg(feature = "mcp-http")]
        authentication: super::authentication::selections(options, snapshot, cancellation)?,
        peer_lifetime: selected.peer_lifetime,
        max_retained_bytes: selected.max_retained_bytes,
    })?))
}
