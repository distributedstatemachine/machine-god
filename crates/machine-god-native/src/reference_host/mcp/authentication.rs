//! Inert selection of profile authentication using the actual host worker owner.

use super::{NativeReferenceHostBuildError, error};
use crate::{
    NativeOwnedWorkerScope,
    mcp::{
        auth::{
            McpAuthClock, McpAuthDestination, McpAuthEntropy, McpAuthError, McpAuthInvalidated,
            McpAuthInvalidation, McpAuthNetwork, NativeMcpAuthService, NativeMcpCredentialStore,
        },
        clock::TokioMcpClock,
        controller::NativeMcpControllerStartupOptions,
        management::NativeMcpManagementService,
        network::NativeMcpNetwork,
    },
};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{sync::Arc, time::Instant};

#[derive(Clone)]
pub(super) struct Options {
    clock: Arc<dyn McpAuthClock>,
    entropy: Arc<dyn McpAuthEntropy>,
}

impl Options {
    pub fn captured(clock: Arc<TokioMcpClock>) -> Self {
        Self {
            clock,
            entropy: Arc::new(SystemEntropy),
        }
    }

    pub fn compose(
        self,
        management: &NativeMcpManagementService,
        startup: &NativeMcpControllerStartupOptions,
        workers: &NativeOwnedWorkerScope,
    ) -> Result<Arc<NativeMcpAuthService>, NativeReferenceHostBuildError> {
        let store = management.config_store();
        let directory = store.path().parent().ok_or_else(error)?;
        let credentials = NativeMcpCredentialStore::new(directory.to_owned())
            .map(Arc::new)
            .map_err(|_| error())?;
        Ok(Arc::new(NativeMcpAuthService::new(
            credentials,
            Arc::new(SelectedNetwork(startup.network.clone())),
            self.clock,
            self.entropy,
            Arc::new(GenerationInvalidation),
            workers.clone(),
        )))
    }
}

struct SystemEntropy;
impl McpAuthEntropy for SystemEntropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), McpAuthError> {
        if getrandom::fill(bytes).is_err() {
            bytes.fill(0);
            return Err(McpAuthError::Unavailable);
        }
        Ok(())
    }
}

// No DNS authority still permits local credential status/removal. Any attempted
// OAuth request fails explicitly, without capturing another resolver or network.
struct SelectedNetwork(Option<Arc<NativeMcpNetwork>>);
impl McpAuthNetwork for SelectedNetwork {
    fn admit<'a>(
        &'a self,
        url: &'a str,
        cancellation: &'a CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<McpAuthDestination, McpAuthError>> {
        Box::pin(async move {
            let network = self.0.as_ref().ok_or(McpAuthError::Unavailable)?;
            McpAuthNetwork::admit(network.as_ref(), url, cancellation, deadline).await
        })
    }
}

// The same token is retained by each authenticated route and peer writer.
// Invalidation never mints another identity, publishes a catalog or replays work.
struct GenerationInvalidation;
impl McpAuthInvalidation for GenerationInvalidation {
    fn invalidate(&self, event: McpAuthInvalidated) {
        event.generation.cancel();
    }
}
