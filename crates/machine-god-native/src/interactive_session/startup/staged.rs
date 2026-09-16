//! ACP's first parent uses the same ready-stage custody as later transitions.
use super::{
    Arc, BoxFuture, CancellationToken, Context, NativeInteractiveError,
    NativeInteractiveInitialSession, NativeInteractiveSession, NativeInteractiveSessionOptions,
    NativeManagedInteractiveStartup, NativeReferenceHost, OpenFailure, OpenResult, Poll, State,
    managed,
};
use crate::managed::manager::ManagedForegroundReservation;
use crate::reference_host::NativeManagedStagedParent;

impl NativeManagedInteractiveStartup {
    pub(crate) fn reserve_parent_stage(
        &mut self,
    ) -> Result<ManagedForegroundReservation, NativeInteractiveError> {
        if self.closing {
            return Err(NativeInteractiveError::Closed);
        }
        let State::Idle(owner) = &mut self.state else {
            return Err(NativeInteractiveError::Busy);
        };
        let reservation = owner
            .agents
            .reserve_foreground()
            .map_err(NativeInteractiveError::Managed)?;
        self.notify();
        Ok(reservation)
    }

    pub(crate) fn poll_parent_stage_reservation(
        &self,
        reservation: &ManagedForegroundReservation,
        cx: &Context<'_>,
    ) -> Poll<Result<(), NativeInteractiveError>> {
        let State::Idle(owner) = &self.state else {
            return Poll::Ready(Err(NativeInteractiveError::Busy));
        };
        owner
            .agents
            .poll_foreground_reservation(reservation, cx)
            .map_err(NativeInteractiveError::Managed)
    }

    pub(crate) fn start_parent_stage(
        &self,
        reservation: ManagedForegroundReservation,
        configuration: crate::mcp::ephemeral::NativeMcpEphemeralConfiguration,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeManagedStagedParent, crate::NativeManagedAgentsError>>
    {
        match &self.state {
            State::Idle(owner) if !self.closing => {
                owner
                    .agents
                    .stage_captured_foreground_mcp(reservation, configuration, cancellation)
            }
            _ => Box::pin(async { Err(crate::NativeManagedAgentsError::Unavailable) }),
        }
    }

    /// Opening consumes one already-ready selection, not an implicit empty MCP
    /// configuration. Synchronous rejection returns original cleanup custody.
    pub(crate) fn request_open_staged(
        &mut self,
        initial: NativeInteractiveInitialSession,
        candidate: NativeManagedStagedParent,
        now_ms: i64,
    ) -> Result<(), managed::staged::Failure> {
        let valid = (|| {
            if self.closing {
                return Err(NativeInteractiveError::Closed);
            }
            let State::Idle(owner) = &self.state else {
                return Err(NativeInteractiveError::Busy);
            };
            owner
                .agents
                .validate_staged_foreground(&candidate)
                .map_err(NativeInteractiveError::Managed)
        })();
        if let Err(error) = valid {
            return Err(managed::staged::Failure::staged(error, candidate));
        }
        let State::Idle(owner) = std::mem::replace(&mut self.state, State::Finished) else {
            unreachable!("validated idle owner");
        };
        let cancellation = CancellationToken::new();
        self.state = State::Opening {
            future: Box::pin(open_owned(
                self.host.clone(),
                self.options.clone(),
                initial,
                now_ms,
                owner,
                candidate,
                cancellation.clone(),
            )),
            cancellation,
        };
        self.notify();
        Ok(())
    }
}

async fn rejected(
    owner: Box<managed::Owner>,
    mut failure: managed::staged::Failure,
) -> OpenFailure {
    match failure.settle().await {
        Ok(()) => OpenFailure {
            error: failure.error,
            owner,
            cleanup: None,
        },
        Err(error) => OpenFailure {
            error: NativeInteractiveError::Managed(error),
            owner,
            cleanup: Some(Box::new(failure)),
        },
    }
}

async fn open_owned(
    host: Arc<NativeReferenceHost>,
    options: NativeInteractiveSessionOptions,
    initial: NativeInteractiveInitialSession,
    now_ms: i64,
    owner: Box<managed::Owner>,
    candidate: NativeManagedStagedParent,
    cancellation: CancellationToken,
) -> OpenResult {
    let authority = (|| {
        if cancellation.is_cancelled() {
            return Err(NativeInteractiveError::Closed);
        }
        options.validate_for_host(&host)?;
        owner
            .agents
            .validate_staged_foreground(&candidate)
            .map_err(NativeInteractiveError::Managed)?;
        host.managed_foreground_authority(None, None)
            .map_err(NativeInteractiveError::Managed)
    })();
    let (workspace, policy) = match authority {
        Ok(authority) => authority,
        Err(error) => {
            return Err(rejected(owner, managed::staged::Failure::staged(error, candidate)).await);
        }
    };
    let kind = match initial {
        NativeInteractiveInitialSession::Fresh => crate::NativeInteractiveTransition::New,
        NativeInteractiveInitialSession::Resume(target) => {
            crate::NativeInteractiveTransition::Resume(target)
        }
    };
    let conversation = match super::super::transition::prepare(&host, &options, kind, now_ms).await
    {
        Ok(conversation) => conversation,
        Err(error) => {
            return Err(rejected(owner, managed::staged::Failure::staged(error, candidate)).await);
        }
    };
    // No shared reference to the manager survives an await: the original owner
    // stays in this future and preparation holds only its checked weak factory.
    let preparation = managed::staged::compose(
        &owner.agents,
        host.clone(),
        &options,
        candidate,
        conversation,
        managed::staged::Selection {
            workspace,
            policy,
            catalog: options.catalog.clone(),
            process_model: options.process_model.clone(),
            now_ms,
            cancellation,
        },
    );
    let prepared = match preparation.await {
        Ok(prepared) => prepared,
        Err(failure) => return Err(rejected(owner, failure).await),
    };
    let mut owner = Some(owner);
    let (current, foreground) = match managed::enroll(&mut owner, prepared) {
        Ok(selected) => selected,
        Err((error, prepared, reservation)) => {
            return Err(rejected(
                owner.take().expect("failed enrollment retains manager"),
                managed::staged::Failure::prepared(error, prepared, reservation),
            )
            .await);
        }
    };
    if let Err(error) = managed::activate_initial(&host, &current, &mut owner, now_ms).await {
        return Err(OpenFailure {
            error,
            owner: owner.take().expect("failed activation retains manager"),
            cleanup: None,
        });
    }
    Ok(NativeInteractiveSession::from_runtime(
        host, options, current, owner, foreground,
    ))
}
