//! A staged MCP owner retains the manager's original granted residency ticket.
use super::{
    Arc, BoxFuture, ManagedForegroundReservation, ManagedRestorationAuthority, NativeConversation,
    NativeManagedAgents, NativeManagedAgentsError, NativeModelPreferences,
    NativePermissionPolicySnapshot, NativeWorkspaceScopeSnapshot, NonZeroU64, NoticePrincipal,
    PreparedManagedRuntime, fmt, map_error,
};
use crate::mcp::ephemeral::NativeMcpEphemeralConfiguration;
use crate::reference_host::managed_factory::StagedParentMcp;
use machine_god_core::CancellationToken;

pub(crate) struct NativeManagedStagedParent {
    stage: StagedParentMcp,
    reservation: ManagedForegroundReservation,
}
pub(crate) struct NativeManagedStagedFailure {
    pub error: NativeManagedAgentsError,
    pub candidate: NativeManagedStagedParent,
}
impl fmt::Debug for NativeManagedStagedParent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeManagedStagedParent { .. }")
    }
}
impl fmt::Debug for NativeManagedStagedFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeManagedStagedFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
impl NativeManagedStagedParent {
    pub(crate) fn ready(&self) -> Result<(), NativeManagedAgentsError> {
        self.stage.ready().map_err(map_error)
    }
    pub(crate) fn settle(&mut self) -> BoxFuture<'_, Result<(), NativeManagedAgentsError>> {
        Box::pin(async move { self.stage.settle().await.map_err(map_error) })
    }
}

impl NativeManagedAgents {
    /// Native selection supplies explicitly captured network authority while
    /// retaining the parent's original workspace/helper/environment binding.
    /// Neither a waiting ticket nor a foreign manager can start peers.
    pub(crate) fn stage_foreground_mcp(
        &self,
        reservation: ManagedForegroundReservation,
        #[cfg(feature = "mcp-http")] network: Option<Arc<crate::mcp::network::NativeMcpNetwork>>,
        configuration: NativeMcpEphemeralConfiguration,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeManagedStagedParent, NativeManagedAgentsError>> {
        let granted = self.manager.validate_foreground_reservation(&reservation);
        let seed = self.parent_mcp.select_ephemeral_network(
            #[cfg(feature = "mcp-http")]
            network,
        );
        let startup = seed.map(|seed| {
            self.factory.stage_parent_mcp(
                self.journal.owner_lease(),
                seed,
                configuration,
                cancellation,
            )
        });
        Box::pin(async move {
            granted.map_err(map_error)?;
            reservation.validate_preparation().map_err(map_error)?;
            let stage = startup
                .map_err(|_| NativeManagedAgentsError::Configuration)?
                .await
                .map_err(map_error)?;
            Ok(NativeManagedStagedParent { stage, reservation })
        })
    }

    pub(crate) fn prepare_staged_foreground(
        &self,
        candidate: NativeManagedStagedParent,
        conversation: NativeConversation,
        workspace: NativeWorkspaceScopeSnapshot,
        policy: NativePermissionPolicySnapshot,
        preferences: NativeModelPreferences,
    ) -> BoxFuture<
        'static,
        Result<(PreparedManagedRuntime, ManagedForegroundReservation), NativeManagedStagedFailure>,
    > {
        let granted = self
            .manager
            .validate_foreground_reservation(&candidate.reservation);
        let factory = Arc::downgrade(&self.factory);
        Box::pin(async move {
            if let Err(error) = granted.and_then(|()| candidate.reservation.validate_preparation())
            {
                return Err(NativeManagedStagedFailure {
                    error: map_error(error),
                    candidate,
                });
            }
            let Some(factory) = factory.upgrade() else {
                return Err(NativeManagedStagedFailure {
                    error: NativeManagedAgentsError::Unavailable,
                    candidate,
                });
            };
            let principal = NoticePrincipal {
                id: conversation.id().to_string(),
                generation: NonZeroU64::MIN,
            };
            let NativeManagedStagedParent { stage, reservation } = candidate;
            let preparation = factory.prepare_staged_parent(
                stage,
                conversation,
                ManagedRestorationAuthority {
                    workspace,
                    policy,
                    preferences,
                },
                principal,
            );
            drop(factory);
            match preparation.await {
                Ok(prepared) => Ok((prepared, reservation)),
                Err(failure) => Err(NativeManagedStagedFailure {
                    error: map_error(failure.error),
                    candidate: NativeManagedStagedParent {
                        stage: failure.stage,
                        reservation,
                    },
                }),
            }
        })
    }
}
