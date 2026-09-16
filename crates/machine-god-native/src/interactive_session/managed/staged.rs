//! Adopt an already-ready parent MCP instance without starting it a second time.
//! Every failure returns original cleanup custody, including its residency charge.
use super::{
    Arc, BoxFuture, ManagedForegroundReservation, NativeConversation, NativeInteractiveError,
    NativeInteractiveSessionOptions, NativeModelCatalog, NativeReferenceHost, Prepared,
    PreparedManagedRuntime,
};
use crate::reference_host::{NativeManagedStagedFailure, NativeManagedStagedParent};
use crate::{NativeManagedAgents, NativePermissionPolicySnapshot, NativeWorkspaceScopeSnapshot};
use machine_god_core::CancellationToken;

pub(in crate::interactive_session) type Settled =
    (Failure, Result<(), crate::NativeManagedAgentsError>);

pub(in crate::interactive_session) fn settle_owned(
    mut failure: Failure,
) -> BoxFuture<'static, Settled> {
    Box::pin(async move {
        let result = failure.settle().await;
        (failure, result)
    })
}

impl super::NativeInteractiveSession {
    pub(crate) fn reserve_parent_stage(
        &mut self,
    ) -> Result<ManagedForegroundReservation, NativeInteractiveError> {
        if self.shutting_down || self.closed || self.pending.is_some() || self.transition.is_some()
        {
            return Err(NativeInteractiveError::Busy);
        }
        let reservation = self
            .managed
            .as_mut()
            .ok_or(NativeInteractiveError::Configuration)?
            .agents
            .reserve_foreground()
            .map_err(NativeInteractiveError::Managed)?;
        self.notify();
        Ok(reservation)
    }

    pub(crate) fn poll_parent_stage_reservation(
        &self,
        reservation: &ManagedForegroundReservation,
        cx: &std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), NativeInteractiveError>> {
        let Some(owner) = &self.managed else {
            return std::task::Poll::Ready(Err(NativeInteractiveError::Configuration));
        };
        owner
            .agents
            .poll_foreground_reservation(reservation, cx)
            .map_err(NativeInteractiveError::Managed)
    }

    pub(crate) fn start_parent_stage(
        &self,
        reservation: ManagedForegroundReservation,
        #[cfg(feature = "mcp-http")] network: Option<Arc<crate::mcp::network::NativeMcpNetwork>>,
        configuration: crate::mcp::ephemeral::NativeMcpEphemeralConfiguration,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeManagedStagedParent, crate::NativeManagedAgentsError>>
    {
        match &self.managed {
            Some(owner) => owner.agents.stage_foreground_mcp(
                reservation,
                #[cfg(feature = "mcp-http")]
                network,
                configuration,
                cancellation,
            ),
            None => Box::pin(async { Err(crate::NativeManagedAgentsError::Configuration) }),
        }
    }

    /// Transfers a ready candidate only into its original idle selection lane.
    /// Rejection returns the original stage; no caller may discard cleanup merely
    /// because this method rejected synchronous admission.
    pub(crate) fn request_staged_transition(
        &mut self,
        kind: crate::NativeInteractiveTransition,
        candidate: NativeManagedStagedParent,
        now_ms: i64,
        cancellation: CancellationToken,
    ) -> Result<crate::NativeInteractiveRequestReceipt, Failure> {
        let valid = (|| {
            if cancellation.is_cancelled() {
                return Err(NativeInteractiveError::Closed);
            }
            if self.pending.is_some() || self.transition.is_some() {
                return Err(NativeInteractiveError::Busy);
            }
            self.managed
                .as_ref()
                .ok_or(NativeInteractiveError::Configuration)?
                .agents
                .validate_staged_foreground(&candidate)
                .map_err(NativeInteractiveError::Managed)?;
            self.request_transition(kind, now_ms)
        })();
        match valid {
            Ok(receipt) => {
                self.pending.as_mut().expect("accepted request").staged = Some(Box::new(candidate));
                self.pending
                    .as_mut()
                    .expect("accepted request")
                    .staged_cancellation = Some(cancellation);
                Ok(receipt)
            }
            Err(error) => Err(Failure::staged(error, candidate)),
        }
    }
}

pub(in crate::interactive_session) struct Selection {
    pub workspace: NativeWorkspaceScopeSnapshot,
    pub policy: NativePermissionPolicySnapshot,
    pub catalog: Option<Arc<NativeModelCatalog>>,
    pub process_model: Option<String>,
    pub now_ms: i64,
    pub cancellation: CancellationToken,
}

enum Custody {
    Stage(Box<NativeManagedStagedParent>),
    Runtime {
        prepared: Box<PreparedManagedRuntime>,
        _reservation: ManagedForegroundReservation,
    },
}

/// An error is not permission to drop an unresolved candidate or its ticket.
/// The transition retains this owner until settlement succeeds, or fences on
/// failure. Polling a borrowed wrapper cannot abandon the accepted cleanup.
pub(crate) struct Failure {
    pub error: NativeInteractiveError,
    custody: Custody,
    settled: Option<Result<(), crate::NativeManagedAgentsError>>,
}
impl Failure {
    pub(in crate::interactive_session) fn prepared(
        error: NativeInteractiveError,
        prepared: Box<PreparedManagedRuntime>,
        reservation: ManagedForegroundReservation,
    ) -> Self {
        Self {
            error,
            custody: Custody::Runtime {
                prepared,
                _reservation: reservation,
            },
            settled: None,
        }
    }

    pub(in crate::interactive_session) fn staged(
        error: NativeInteractiveError,
        candidate: NativeManagedStagedParent,
    ) -> Self {
        Self {
            error,
            custody: Custody::Stage(Box::new(candidate)),
            settled: None,
        }
    }

    pub(crate) fn settle(&mut self) -> BoxFuture<'_, Result<(), crate::NativeManagedAgentsError>> {
        Box::pin(async move {
            if let Some(result) = self.settled {
                return result;
            }
            let result = match &mut self.custody {
                Custody::Stage(candidate) => candidate.settle().await,
                Custody::Runtime { prepared, .. } => {
                    prepared.owner.retire();
                    prepared.resources.begin_close();
                    futures_util::future::poll_fn(|cx| prepared.resources.poll_closed(cx))
                        .await
                        .map_err(crate::reference_host::managed_error)
                }
            };
            self.settled = Some(result);
            result
        })
    }
}
impl std::fmt::Debug for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StagedForegroundFailure { .. }")
    }
}

pub(in crate::interactive_session) fn compose(
    agents: &NativeManagedAgents,
    host: Arc<NativeReferenceHost>,
    options: &NativeInteractiveSessionOptions,
    candidate: NativeManagedStagedParent,
    conversation: NativeConversation,
    selection: Selection,
) -> BoxFuture<'static, Result<Prepared, Failure>> {
    let Selection {
        workspace,
        policy,
        catalog,
        process_model,
        now_ms,
        cancellation,
    } = selection;
    let preparation = agents.prepare_staged_foreground(
        candidate,
        conversation,
        workspace,
        policy,
        options.defaults.clone(),
        cancellation.clone(),
    );
    Box::pin(async move {
        let (prepared, reservation) =
            preparation
                .await
                .map_err(|NativeManagedStagedFailure { error, candidate }| {
                    Failure::staged(
                        if cancellation.is_cancelled() {
                            NativeInteractiveError::Closed
                        } else {
                            NativeInteractiveError::Managed(error)
                        },
                        candidate,
                    )
                })?;
        let prepared = Box::new(prepared);
        let runtime = &prepared.runtime;
        // The original stage already established readiness. Starting configured
        // MCP here would replace the selected publication or duplicate peers.
        let configure = async {
            if cancellation.is_cancelled() {
                return Err(NativeInteractiveError::Closed);
            }
            if let Some(model) = process_model {
                let mut preferences = runtime.model_preferences();
                preferences
                    .set_model(&model)
                    .map_err(|_| NativeInteractiveError::Configuration)?;
                runtime.set_model_preferences(preferences)?;
            }
            if let Some(catalog) = catalog {
                runtime.set_model_catalog(catalog)?;
            }
            runtime.recover_notice_delivery().await?;
            crate::session_resume::owned::flush_candidate(&host, runtime, now_ms).await?;
            if cancellation.is_cancelled() {
                return Err(NativeInteractiveError::Closed);
            }
            Ok(())
        }
        .await;
        match configure {
            Ok(()) => Ok(Prepared::Managed(prepared, reservation)),
            Err(error) => Err(Failure::prepared(error, prepared, reservation)),
        }
    })
}
