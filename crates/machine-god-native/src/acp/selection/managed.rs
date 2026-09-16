//! ACP pre-selection owns the actual manager, reservation and ready MCP stage.
use super::{
    AcpSessionError, Arc, BoxFuture, CancellationToken, Context, NativeAcpPreparedHost,
    NativeAcpSession, NativeAcpSessionSelection, NativeAcpWorkspaceIdentity,
    NativeInteractiveSessionOptions, NativeMcpEphemeralConfiguration, NativePermissionContexts,
    NativeReferenceHost, Poll,
};
use crate::interactive_session::NativeManagedStageFailure;
use crate::managed::manager::ManagedForegroundReservation;
use crate::reference_host::NativeManagedStagedParent;
use crate::{
    NativeInteractiveInitialSession, NativeManagedAgents, NativeManagedAgentsError,
    NativeManagedInteractiveStartup, NativeResumeTarget, NativeSessionOrigin,
};

type StageResult = Result<NativeManagedStagedParent, NativeManagedAgentsError>;
enum State {
    Idle,
    Reserving {
        reservation: ManagedForegroundReservation,
        configuration: NativeMcpEphemeralConfiguration,
    },
    Starting(BoxFuture<'static, StageResult>),
    Ready(Box<NativeManagedStagedParent>),
    Opening {
        replay: bool,
    },
    Rejected(Box<NativeManagedStageFailure>),
    Finished,
}

pub(super) struct Preparation {
    startup: NativeManagedInteractiveStartup,
    state: State,
    cancellation: CancellationToken,
}

impl NativeAcpPreparedHost {
    /// Binds ACP startup to the original managed host and its captured ephemeral
    /// parent selection. No reservation, peer or conversation is created here.
    /// # Errors
    /// Returns the original manager on invalid binding for owned settlement.
    pub fn new_managed(
        host: Arc<NativeReferenceHost>,
        options: NativeInteractiveSessionOptions,
        permission_contexts: Arc<NativePermissionContexts>,
        workspace: NativeAcpWorkspaceIdentity,
        agents: NativeManagedAgents,
    ) -> Result<Self, (AcpSessionError, Box<NativeManagedAgents>)> {
        let mut value = Self {
            host,
            options: options.with_origin(NativeSessionOrigin::Acp),
            permission_contexts,
            workspace,
            managed: None,
        };
        if let Err(error) = value.validate_binding() {
            return Err((error, Box::new(agents)));
        }
        if !value.host.managed_agents_selected() || !agents.has_ephemeral_parent() {
            return Err((AcpSessionError::InvalidConfiguration, Box::new(agents)));
        }
        let startup =
            NativeManagedInteractiveStartup::new(value.host.clone(), value.options.clone(), agents)
                .map_err(|(error, agents)| (error.into(), agents))?;
        value.managed = Some(Box::new(Preparation {
            startup,
            state: State::Idle,
            cancellation: CancellationToken::new(),
        }));
        Ok(value)
    }

    pub(super) fn ready(&self) -> Result<(), AcpSessionError> {
        if let Some(managed) = &self.managed {
            managed.ready()
        } else {
            self.host
                .mcp_ephemeral_owner()
                .ok_or(AcpSessionError::InvalidConfiguration)?
                .ready()
                .map_err(|_| AcpSessionError::Unavailable)
        }
    }
}

impl Preparation {
    pub(super) fn start(
        &mut self,
        configuration: NativeMcpEphemeralConfiguration,
        cancellation: CancellationToken,
    ) -> Result<(), AcpSessionError> {
        if !matches!(self.state, State::Idle) {
            return Err(AcpSessionError::Busy);
        }
        let reservation = self.startup.reserve_parent_stage()?;
        self.cancellation = cancellation;
        self.state = State::Reserving {
            reservation,
            configuration,
        };
        Ok(())
    }

    pub(super) fn poll_start(
        &mut self,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Poll<Result<(), AcpSessionError>> {
        // The manager must grant the original ticket; never wait on that ticket
        // without driving its owner. Old ACP sessions are co-polled separately.
        if let Poll::Ready(Err(error)) = self.startup.poll_open(cx, now_ms) {
            return Poll::Ready(Err(error.into()));
        }
        if let State::Reserving { reservation, .. } = &self.state {
            if self.cancellation.is_cancelled() {
                return Poll::Ready(Err(AcpSessionError::Cancelled));
            }
            match self.startup.poll_parent_stage_reservation(reservation, cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error.into())),
                Poll::Ready(Ok(())) => {}
            }
            let State::Reserving {
                reservation,
                configuration,
            } = std::mem::replace(&mut self.state, State::Idle)
            else {
                unreachable!("checked reservation");
            };
            self.state = State::Starting(self.startup.start_parent_stage(
                reservation,
                configuration,
                self.cancellation.clone(),
            ));
        }
        if let State::Starting(future) = &mut self.state {
            let result = match future.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => result,
            };
            self.state = State::Idle;
            match result {
                Ok(candidate) => self.state = State::Ready(Box::new(candidate)),
                Err(_) => return Poll::Ready(Err(AcpSessionError::Unavailable)),
            }
        }
        Poll::Ready(self.ready())
    }

    fn ready(&self) -> Result<(), AcpSessionError> {
        let State::Ready(candidate) = &self.state else {
            return Err(AcpSessionError::Unavailable);
        };
        candidate.ready().map_err(|_| AcpSessionError::Unavailable)
    }

    pub(super) fn open(
        &mut self,
        selection: NativeAcpSessionSelection,
        now_ms: i64,
    ) -> Result<(), AcpSessionError> {
        self.ready()?;
        let State::Ready(candidate) = std::mem::replace(&mut self.state, State::Idle) else {
            unreachable!("checked candidate");
        };
        let replay = matches!(selection, NativeAcpSessionSelection::Load(_));
        let initial = match selection {
            NativeAcpSessionSelection::New => NativeInteractiveInitialSession::Fresh,
            NativeAcpSessionSelection::Load(id) | NativeAcpSessionSelection::Resume(id) => {
                NativeInteractiveInitialSession::Resume(NativeResumeTarget::Exact(id))
            }
        };
        match self
            .startup
            .request_open_staged(initial, *candidate, now_ms)
        {
            Ok(()) => {
                self.state = State::Opening { replay };
                Ok(())
            }
            Err(failure) => {
                self.state = State::Rejected(Box::new(failure));
                Err(AcpSessionError::Unavailable)
            }
        }
    }

    pub(super) fn poll_open(
        &mut self,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Poll<Result<NativeAcpSession, AcpSessionError>> {
        let State::Opening { replay } = self.state else {
            return Poll::Ready(Err(AcpSessionError::Unavailable));
        };
        if self.cancellation.is_cancelled() {
            self.startup.request_shutdown();
        }
        self.startup
            .poll_open(cx, now_ms)
            .map(|result| match result {
                Ok(Some(session)) => {
                    self.state = State::Finished;
                    Ok(NativeAcpSession::from_interactive(session, replay))
                }
                Ok(None) => Err(AcpSessionError::Cancelled),
                Err(error) => Err(error.into()),
            })
    }

    // Successful cleanup releases the ticket before awaiting manager shutdown.
    // Failure returns this entire owner, never an uncharged error-only receipt.
    pub(super) async fn settle(mut self: Box<Self>, now_ms: i64) -> Result<(), Box<Self>> {
        self.cancellation.cancel();
        if let State::Starting(future) = &mut self.state {
            let result = future.await;
            self.state = match result {
                Ok(candidate) => State::Ready(Box::new(candidate)),
                Err(_) => State::Idle,
            };
        }
        let settled = match &mut self.state {
            State::Ready(candidate) => candidate.settle().await,
            State::Rejected(failure) => failure.settle().await,
            _ => Ok(()),
        };
        if settled.is_err() {
            self.startup.request_shutdown();
            return Err(self);
        }
        self.state = State::Finished;
        self.startup.request_shutdown();
        let settled = futures_util::future::poll_fn(|cx| self.startup.poll_open(cx, now_ms)).await;
        match settled {
            Ok(None) => Ok(()),
            _ => Err(self),
        }
    }
}
