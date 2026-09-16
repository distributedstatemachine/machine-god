//! Checked same-domain parent replacement without replacing the child manager.
use super::{
    AcpSessionError, Arc, BoxFuture, CancellationToken, Context, NativeAcpSession,
    NativeAcpWorkspaceIdentity, NativeMcpEphemeralConfiguration, NativeReferenceHost, Poll,
};
use crate::interactive_session::NativeManagedStageFailure;
use crate::managed::manager::ManagedForegroundReservation;
use crate::reference_host::NativeManagedStagedParent;
use crate::{NativeManagedAgentsError, NativeWorkspaceAuthorityError, PreparedNativeRoots};
use std::{fmt, sync::Weak};

/// Descriptor-checked reuse of one existing host allocation. A pathname or
/// separately constructed workspace authority cannot forge this selection.
pub struct NativeAcpHostReuse {
    host: Weak<NativeReferenceHost>,
    workspace: NativeAcpWorkspaceIdentity,
    #[cfg(feature = "mcp-http")]
    network: Option<Arc<crate::mcp::network::NativeMcpNetwork>>,
}
impl fmt::Debug for NativeAcpHostReuse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAcpHostReuse { .. }")
    }
}
impl NativeAcpHostReuse {
    /// Checks newly prepared explicit roots against the original retained
    /// descriptors. Runs on the factory's owned worker, opening no extra paths.
    /// Returns `None` for a different workspace/state domain.
    /// # Errors
    /// Invalid metadata or unavailable descriptor validation.
    pub fn capture(
        host: &Arc<NativeReferenceHost>,
        roots: &PreparedNativeRoots,
    ) -> Result<Option<Self>, AcpSessionError> {
        if !host.managed_agents_selected() {
            return Ok(None);
        }
        crate::NativeSessionMetadata::new(
            roots.workspace_root(),
            0,
            crate::NativeSessionOrigin::Acp,
        )
        .map_err(|_| AcpSessionError::InvalidConfiguration)?;
        let scope = host.workspace_scope().ok_or(AcpSessionError::Unavailable)?;
        let primary = roots
            .try_clone_workspace()
            .map_err(|_| AcpSessionError::Unavailable)?;
        let state = roots
            .try_clone_state()
            .map_err(|_| AcpSessionError::Unavailable)?;
        match scope.validate_host_binding(&primary, roots.canonical_workspace_root(), &state) {
            Ok(()) => Ok(Some(Self {
                host: Arc::downgrade(host),
                workspace: NativeAcpWorkspaceIdentity {
                    requested: roots.workspace_root().to_owned(),
                    scope,
                },
                #[cfg(feature = "mcp-http")]
                network: None,
            })),
            Err(NativeWorkspaceAuthorityError::WrongAuthority) => Ok(None),
            Err(_) => Err(AcpSessionError::Unavailable),
        }
    }

    /// Selects only explicitly captured replacement transport authority. The
    /// original host binding and process/workspace capture remain unchanged.
    #[cfg(feature = "mcp-http")]
    #[must_use]
    pub fn with_network(
        mut self,
        network: Option<Arc<crate::mcp::network::NativeMcpNetwork>>,
    ) -> Self {
        self.network = network;
        self
    }

    pub(super) fn matches(
        &self,
        host: &Arc<NativeReferenceHost>,
        workspace: &std::path::Path,
    ) -> bool {
        self.host.ptr_eq(&Arc::downgrade(host))
            && self.workspace.requested == workspace
            && host.has_workspace_primary(&self.workspace.scope)
    }
}

enum State {
    Reserving {
        reservation: ManagedForegroundReservation,
        configuration: NativeMcpEphemeralConfiguration,
        #[cfg(feature = "mcp-http")]
        network: Option<Arc<crate::mcp::network::NativeMcpNetwork>>,
    },
    Starting(BoxFuture<'static, Result<NativeManagedStagedParent, NativeManagedAgentsError>>),
    Ready(Box<NativeManagedStagedParent>),
    Rejected(Box<NativeManagedStageFailure>),
    Transferred,
}
pub(super) struct Stage {
    state: State,
    cancellation: CancellationToken,
}
impl Stage {
    pub(super) fn new(
        session: &mut NativeAcpSession,
        reuse: NativeAcpHostReuse,
        configuration: NativeMcpEphemeralConfiguration,
        cancellation: CancellationToken,
    ) -> Result<Box<Self>, AcpSessionError> {
        let reservation = session.reserve_parent_stage()?;
        let NativeAcpHostReuse {
            host,
            workspace,
            #[cfg(feature = "mcp-http")]
            network,
        } = reuse;
        // Admission consumes the checked token even without HTTP support.
        drop((host, workspace));
        Ok(Box::new(Self {
            state: State::Reserving {
                reservation,
                configuration,
                #[cfg(feature = "mcp-http")]
                network,
            },
            cancellation,
        }))
    }

    pub(super) fn poll_start(
        &mut self,
        session: &NativeAcpSession,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), AcpSessionError>> {
        if let State::Reserving { reservation, .. } = &self.state {
            if self.cancellation.is_cancelled() {
                return Poll::Ready(Err(AcpSessionError::Cancelled));
            }
            match session.poll_parent_stage_reservation(reservation, cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => {}
            }
            let State::Reserving {
                reservation,
                configuration,
                #[cfg(feature = "mcp-http")]
                network,
            } = std::mem::replace(&mut self.state, State::Transferred)
            else {
                unreachable!("checked reservation");
            };
            self.state = State::Starting(session.start_parent_stage(
                reservation,
                #[cfg(feature = "mcp-http")]
                network,
                configuration,
                self.cancellation.clone(),
            ));
        }
        if let State::Starting(future) = &mut self.state {
            let result = match future.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => result,
            };
            self.state = State::Transferred;
            match result {
                Ok(candidate) => self.state = State::Ready(Box::new(candidate)),
                Err(_) => return Poll::Ready(Err(AcpSessionError::Unavailable)),
            }
        }
        Poll::Ready(match &self.state {
            State::Ready(candidate) => candidate.ready().map_err(|_| AcpSessionError::Unavailable),
            _ => Err(AcpSessionError::Unavailable),
        })
    }

    pub(super) fn adopt(
        &mut self,
        session: &mut NativeAcpSession,
        selection: super::NativeAcpSessionSelection,
        now_ms: i64,
    ) -> Result<crate::NativeInteractiveRequestId, AcpSessionError> {
        let State::Ready(_) = self.state else {
            return Err(AcpSessionError::Unavailable);
        };
        let State::Ready(candidate) = std::mem::replace(&mut self.state, State::Transferred) else {
            unreachable!("checked candidate");
        };
        match session.request_staged_selection(
            selection,
            *candidate,
            now_ms,
            self.cancellation.clone(),
        ) {
            Ok(receipt) => Ok(receipt.id),
            Err(failure) => {
                self.state = State::Rejected(Box::new(failure));
                Err(AcpSessionError::Unavailable)
            }
        }
    }

    pub(super) async fn settle(mut self: Box<Self>) -> Result<(), Box<Self>> {
        self.cancellation.cancel();
        if let State::Starting(future) = &mut self.state {
            self.state = match future.await {
                Ok(candidate) => State::Ready(Box::new(candidate)),
                Err(_) => State::Transferred,
            };
        }
        let result = match &mut self.state {
            State::Ready(candidate) => candidate.settle().await,
            State::Rejected(failure) => failure.settle().await,
            _ => Ok(()),
        };
        if result.is_err() { Err(self) } else { Ok(()) }
    }
}
