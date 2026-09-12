//! Concrete weak controller binding; lookup data alone never launches discovery.

use super::{NativeMcpRuntime, NativeMcpRuntimeError, Result};
use crate::mcp::{
    context::NativeMcpTurnContext,
    controller::{NativeMcpController, NativeMcpControllerError},
    submission::McpSubmissionRegistry,
};
use futures_util::future::{Either, select};
use machine_god_core::CancellationToken;
use std::sync::Arc;

impl NativeMcpRuntime {
    /// The actual host binds once, before publishing its engine. The controller
    /// owns this runtime; the reverse link cannot extend controller ownership.
    pub(crate) fn bind_controller(&self, controller: &Arc<NativeMcpController>) -> Result<()> {
        if !controller.selects_runtime(self) {
            return Err(NativeMcpRuntimeError::Invalid);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
        if state.closed {
            return Err(NativeMcpRuntimeError::Unavailable);
        }
        self.controller
            .set(Arc::downgrade(controller))
            .map_err(|_| NativeMcpRuntimeError::Invalid)
    }

    pub(super) async fn activate_for_turn(
        &self,
        context: &NativeMcpTurnContext,
        registry: &Arc<McpSubmissionRegistry>,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        revalidate(context, registry, cancellation)?;
        let pinned = {
            let state = self
                .state
                .lock()
                .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
            if state.closed {
                return Err(NativeMcpRuntimeError::Unavailable);
            }
            let registry = Arc::downgrade(registry);
            state.turns.iter().any(|pin| pin.registry.ptr_eq(&registry))
        };
        if !pinned && let Some(controller) = self.controller.get() {
            let controller = controller
                .upgrade()
                .ok_or(NativeMcpRuntimeError::Unavailable)?;
            // Poll exact turn retirement first. Dropping this waiter never
            // cancels another turn's coalesced controller-owned discovery.
            match select(
                context.cancelled(),
                Box::pin(async {
                    controller
                        .refresh_authentication_configured(cancellation.clone())
                        .await?;
                    controller
                        .activate_deferred_configured(cancellation.clone())
                        .await
                }),
            )
            .await
            {
                Either::Left(_) => return Err(NativeMcpRuntimeError::Cancelled),
                Either::Right((result, _)) => {
                    result.map_err(|error| match error.kind() {
                        NativeMcpControllerError::Cancelled => NativeMcpRuntimeError::Cancelled,
                        _ => NativeMcpRuntimeError::Unavailable,
                    })?;
                }
            }
        }
        // No new publication is pinned until the original turn and registry
        // have survived the asynchronous boundary and caller cancellation.
        if !pinned {
            revalidate(context, registry, cancellation)?;
            self.refresh_for_turn(context, cancellation).await?;
        }
        revalidate(context, registry, cancellation)
    }
}

fn revalidate(
    context: &NativeMcpTurnContext,
    registry: &Arc<McpSubmissionRegistry>,
    cancellation: &CancellationToken,
) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(NativeMcpRuntimeError::Cancelled);
    }
    let exact = context
        .registry()
        .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
    if !Arc::ptr_eq(&exact, registry) {
        return Err(NativeMcpRuntimeError::Invalid);
    }
    registry
        .revalidate()
        .map_err(|_| NativeMcpRuntimeError::Unavailable)
}
