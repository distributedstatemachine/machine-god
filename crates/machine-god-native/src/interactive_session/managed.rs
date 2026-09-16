//! Managed preparation and cleanup stay with the native interactive owner.
pub(in crate::interactive_session) mod staged;
use super::{
    Arc, BoxFuture, Context, NativeConversation, NativeConversationRuntime,
    NativeConversationRuntimeError, NativeInteractiveError, NativeInteractiveInitialSession,
    NativeInteractiveOutcome, NativeInteractiveSession, NativeInteractiveSessionOptions,
    NativeModelCatalog, NativeReferenceHost, NativeRuntimeQuiescence, Poll, Transition,
};
use crate::managed::manager::{
    ManagedForegroundReservation, ManagedForegroundSelection, factory::PreparedManagedRuntime,
};

pub(super) enum Prepared {
    Ordinary(Arc<NativeConversationRuntime>),
    Managed(Box<PreparedManagedRuntime>, ManagedForegroundReservation),
}

pub(super) struct Selection {
    pub reservation: ManagedForegroundReservation,
    pub workspace: Option<crate::NativeWorkspaceScopeSnapshot>,
    pub policy: Option<crate::NativePermissionPolicySnapshot>,
    pub catalog: Option<Arc<NativeModelCatalog>>,
    pub initial: bool,
    pub now_ms: i64,
    pub cancellation: machine_god_core::CancellationToken,
}

type Enrolled = (
    Arc<NativeConversationRuntime>,
    Option<ManagedForegroundSelection>,
);
type EnrollmentError = (
    NativeInteractiveError,
    Box<PreparedManagedRuntime>,
    ManagedForegroundReservation,
);

/// Allocated only for managed hosts, keeping ordinary hosts and ACP futures small.
pub(super) struct Owner {
    pub agents: crate::NativeManagedAgents,
    pub foreground: Option<ManagedForegroundSelection>,
    error: Option<crate::NativeManagedAgentsError>,
    foreground_closed: bool,
}
impl Owner {
    pub(super) fn new(agents: crate::NativeManagedAgents) -> Box<Self> {
        Box::new(Self {
            agents,
            foreground: None,
            error: None,
            foreground_closed: false,
        })
    }
}

impl NativeInteractiveSession {
    /// Queues an explicitly requested human management command against this
    /// actual foreground's captured policy/workspace. This is not model admission
    /// and cannot produce or substitute a tool-call witness. Queueing is not a
    /// durability receipt: co-poll this owner and await the returned response.
    /// Dropping the response alone is not durable cancellation of accepted work.
    /// # Errors
    /// Rejects unavailable/transitioning ownership, cancelled requests, invalid
    /// commands and exhausted shared queue or payload capacity before submission.
    pub fn request_managed_command(
        &mut self,
        command: machine_god_core::ManagedSubagentCommand,
        cancellation: machine_god_core::CancellationToken,
    ) -> Result<crate::NativeManagedCommandResponse, machine_god_core::ManagedSubagentError> {
        use machine_god_core::ManagedSubagentError;
        if self.shutting_down || self.closed || self.transition.is_some() || self.pending.is_some()
        {
            return Err(ManagedSubagentError::Unavailable);
        }
        let owner = self
            .managed
            .as_mut()
            .ok_or(ManagedSubagentError::Unavailable)?;
        let selected = owner
            .foreground
            .as_ref()
            .ok_or(ManagedSubagentError::Unavailable)?;
        let response = owner
            .agents
            .request_human_command(selected, command, cancellation)?;
        self.notify();
        Ok(response)
    }

    /// Reports whether native runtimes own registration in this exact inbox.
    /// Presentation must not register or retire a second lease in managed mode.
    /// # Errors
    /// Rejects an inbox other than the one explicitly bound during composition.
    pub fn manages_prompt_inbox(
        &self,
        inbox: &crate::NativeInteractivePromptInbox,
    ) -> Result<bool, NativeInteractiveError> {
        self.managed.as_ref().map_or(Ok(false), |owner| {
            owner
                .agents
                .manages_prompt_inbox(inbox)
                .map_err(NativeInteractiveError::Managed)
        })
    }

    pub(super) fn compose_candidate(
        &self,
        transition: &mut Transition,
        conversation: NativeConversation,
    ) -> Result<super::transition::Phase, NativeInteractiveError> {
        let guard = transition
            .guard
            .as_ref()
            .ok_or(NativeInteractiveError::Unavailable)?;
        let snapshot = guard.selection_snapshot()?;
        let host = self.host.clone();
        let options = self.options.clone();
        let policy = snapshot.permission_policy().cloned();
        let catalog = snapshot.model_catalog().cloned();
        let now_ms = transition.request.now_ms;
        if transition.request.staged.is_some() {
            let owner = self
                .managed
                .as_ref()
                .ok_or(NativeInteractiveError::Configuration)?;
            let workspace = guard
                .workspace_snapshot()?
                .ok_or(NativeInteractiveError::Configuration)?;
            let policy = policy.ok_or(NativeInteractiveError::Configuration)?;
            let candidate = transition.request.staged.take().expect("retained stage");
            return Ok(super::transition::Phase::ComposingStaged(staged::compose(
                &owner.agents,
                host,
                &options,
                *candidate,
                conversation,
                staged::Selection {
                    workspace,
                    policy,
                    catalog,
                    process_model: None,
                    now_ms,
                    cancellation: transition.preparation_cancel.clone(),
                },
            )));
        }
        Ok(super::transition::Phase::Composing(match &self.managed {
            Some(owner) => prepare(
                &owner.agents,
                host,
                &options,
                conversation,
                Selection {
                    reservation: transition
                        .managed_reservation
                        .take()
                        .ok_or(NativeInteractiveError::Unavailable)?,
                    workspace: Some(
                        guard
                            .workspace_snapshot()?
                            .ok_or(NativeInteractiveError::Configuration)?,
                    ),
                    policy,
                    catalog,
                    initial: false,
                    now_ms,
                    cancellation: transition.preparation_cancel.clone(),
                },
            ),
            None => Box::pin(async move {
                super::transition::compose(
                    &host,
                    &options,
                    conversation,
                    policy,
                    catalog,
                    false,
                    now_ms,
                )
                .await
                .map(Prepared::Ordinary)
            }),
        }))
    }

    /// Opens a managed interactive owner from this exact host and an explicitly
    /// opened private journal directory. No child is restored or started merely
    /// by opening. The owner co-polls child work independently of presentation.
    #[must_use]
    pub fn open_managed(
        mut host: NativeReferenceHost,
        directory: rustix::fd::OwnedFd,
        options: NativeInteractiveSessionOptions,
        initial: NativeInteractiveInitialSession,
        now_ms: i64,
    ) -> BoxFuture<'static, Result<Self, NativeInteractiveError>> {
        Box::pin(async move {
            options.validate_for_host(&host)?;
            let agents = host
                .open_managed_agents(directory, options.defaults.clone(), options.origin)
                .await
                .map_err(NativeInteractiveError::Managed)?;
            Self::open_with_agents(
                Arc::new(host),
                options,
                initial,
                now_ms,
                Some(Owner::new(agents)),
            )
            .await
        })
    }

    pub(crate) fn quiesce_current(
        &mut self,
    ) -> Result<NativeRuntimeQuiescence, NativeConversationRuntimeError> {
        let guard = match &mut self.managed {
            Some(owner) => owner.agents.quiesce_foreground(
                owner
                    .foreground
                    .as_ref()
                    .ok_or(NativeConversationRuntimeError::Retired)?,
            ),
            None => self.current.begin_quiescence(),
        }?;
        // The manager was already polled before this fence. Schedule one more
        // pass so invalidated human waits release their lifecycle permits even
        // when no model, I/O or deadline can otherwise wake the driver.
        if self.managed.is_some() {
            self.notify();
        }
        Ok(guard)
    }

    pub(super) fn retire_candidate(&mut self, transition: &mut Transition) {
        transition.managed_reservation.take();
        if let Some(selected) = transition.managed_candidate.take()
            && let Some(agents) = &mut self.managed
        {
            agents.agents.retire_foreground(&selected);
        }
    }

    pub(super) fn poll_candidate_reservation(
        &mut self,
        transition: &mut Transition,
        cx: &Context<'_>,
    ) -> Poll<Result<(), NativeInteractiveError>> {
        let Some(owner) = &mut self.managed else {
            return Poll::Ready(Ok(()));
        };
        if let Some(candidate) = &transition.request.staged {
            // Staging already holds the exact granted ticket. Never reserve a
            // second slot, or wait for quota occupied by this same candidate.
            return Poll::Ready(
                owner
                    .agents
                    .validate_staged_foreground(candidate)
                    .map_err(NativeInteractiveError::Managed),
            );
        }
        if transition.managed_reservation.is_none() {
            transition.managed_reservation = Some(
                owner
                    .agents
                    .reserve_foreground()
                    .map_err(NativeInteractiveError::Managed)?,
            );
            cx.waker().wake_by_ref();
        }
        owner
            .agents
            .poll_foreground_reservation(
                transition
                    .managed_reservation
                    .as_ref()
                    .expect("retained reservation"),
                cx,
            )
            .map_err(NativeInteractiveError::Managed)
    }

    pub(crate) fn foreground_turn_settled(&self) -> bool {
        match &self.managed {
            Some(owner) => owner
                .foreground
                .as_ref()
                .is_some_and(|selected| owner.agents.foreground_turn_settled(selected)),
            None => true,
        }
    }

    pub(super) fn finish_foreground_shutdown(&mut self) {
        self.notify();
        if let Some(owner) = &mut self.managed {
            owner.foreground_closed = true;
        } else {
            self.closed = true;
            if self.outcome.is_none() {
                self.outcome = Some(NativeInteractiveOutcome::Shutdown);
            }
        }
    }

    pub(super) fn foreground_is_closed(&self) -> bool {
        self.managed
            .as_ref()
            .is_some_and(|owner| owner.foreground_closed)
    }

    pub(super) fn begin_managed_shutdown(&mut self) {
        if self.shutting_down
            && let Some(owner) = &mut self.managed
        {
            owner.agents.request_shutdown();
        }
    }

    pub(crate) fn poll_managed(&mut self, cx: &mut Context<'_>, now_ms: i64) {
        let Some(owner) = &mut self.managed else {
            return;
        };
        if owner.foreground_closed {
            match owner.agents.poll_shutdown(cx, now_ms) {
                Poll::Ready(Ok(())) => {
                    self.closed = true;
                    if self.outcome.is_none() {
                        self.outcome = Some(NativeInteractiveOutcome::Shutdown);
                    }
                }
                Poll::Ready(Err(error)) => {
                    self.shutdown_error = Some(NativeInteractiveError::Managed(error));
                }
                Poll::Pending => {}
            }
        } else if let Poll::Ready(Err(error)) = owner.agents.poll_progress(cx, now_ms) {
            owner.error = Some(error);
        }
    }

    /// ACP's outer selection has already consumed the original quiescence
    /// guard. Do not attempt to acquire another guard on a retired runtime.
    pub(crate) fn request_retired_shutdown(&mut self) -> Result<(), NativeInteractiveError> {
        if self.current.status().phase != crate::NativeConversationRuntimePhase::Retired
            || self.admission.is_some()
            || self.turn.is_some()
            || self.control.is_some()
            || self.transition.is_some()
        {
            return Err(NativeInteractiveError::Busy);
        }
        self.request_shutdown();
        self.begin_managed_shutdown();
        self.finish_foreground_shutdown();
        Ok(())
    }

    /// ACP controls follow this foreground's ephemeral instance, never the
    /// host-global seed or a sibling's runtime.
    pub(crate) fn acp_mcp_ready(&self) -> Result<(), NativeInteractiveError> {
        let ephemeral = match &self.managed {
            Some(owner) => owner
                .foreground
                .as_ref()
                .and_then(|foreground| owner.agents.foreground_mcp_controls(foreground))
                .and_then(|controls| controls.ephemeral),
            None => self.host.mcp_ephemeral_owner(),
        }
        .ok_or(NativeInteractiveError::Unavailable)?;
        ephemeral
            .ready()
            .map_err(|_| NativeInteractiveError::Unavailable)
    }

    pub(crate) fn acp_mcp_runtime(&self) -> Option<Arc<crate::mcp::runtime::NativeMcpRuntime>> {
        match &self.managed {
            Some(owner) => {
                let controls = owner
                    .agents
                    .foreground_mcp_controls(owner.foreground.as_ref()?)?;
                controls.ephemeral?;
                controls.runtime
            }
            None => self
                .host
                .mcp_ephemeral_owner()
                .and_then(|_| self.host.mcp_runtime()),
        }
    }

    /// Bounded native observations; navigation labels confer no control authority.
    #[must_use]
    pub fn managed_progress(&self) -> Option<crate::NativeManagedAgentsProgress> {
        self.managed
            .as_ref()
            .map(|owner| owner.agents.progress_snapshot())
    }

    /// Bounded native observations; navigation labels confer no control authority.
    #[must_use]
    pub fn managed_agents(&self) -> Vec<crate::NativeManagedAgentView> {
        self.managed
            .as_ref()
            .map_or_else(Vec::new, |owner| owner.agents.agents())
    }

    #[must_use]
    pub fn managed_error(&self) -> Option<crate::NativeManagedAgentsError> {
        self.managed.as_ref().and_then(|owner| owner.error)
    }

    /// Last selected controller activation failure, cleared by a successful
    /// explicit reload. A startup warning is not a failed interactive owner.
    #[must_use]
    pub fn mcp_startup_failure(&self) -> Option<crate::mcp::controller::NativeMcpControllerError> {
        let controller = match &self.managed {
            Some(owner) => {
                owner
                    .agents
                    .foreground_mcp_controls(owner.foreground.as_ref()?)?
                    .controller
            }
            None => self.host.mcp_controller(),
        }?;
        controller.activation_failure()
    }

    /// Explicit repair retry does not retry a model turn or allocate another child.
    pub fn retry_managed_reconciliation(&mut self) {
        if let Some(owner) = &mut self.managed {
            owner.agents.retry_reconciliation();
            owner.error = None;
            self.notify();
        }
    }
}

pub(super) fn prepare(
    agents: &crate::NativeManagedAgents,
    host: Arc<NativeReferenceHost>,
    options: &NativeInteractiveSessionOptions,
    conversation: NativeConversation,
    selection: Selection,
) -> BoxFuture<'static, Result<Prepared, NativeInteractiveError>> {
    let Selection {
        reservation,
        workspace,
        policy,
        catalog,
        initial,
        now_ms,
        cancellation,
    } = selection;
    let preparation = host.prepare_managed_foreground(
        agents,
        conversation,
        workspace,
        policy,
        options.defaults.clone(),
    );
    let process_model = initial.then(|| options.process_model.clone()).flatten();
    let phase = options.mcp_startup_phase;
    Box::pin(async move {
        let mut prepared = Box::new(preparation.await.map_err(NativeInteractiveError::Managed)?);
        let startup = start_parent_mcp(&prepared, phase, cancellation.clone());
        let runtime = prepared.runtime.clone();
        let configure = async {
            startup.await?;
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
            crate::session_resume::owned::flush_candidate(&host, &runtime, now_ms).await?;
            Ok::<_, NativeInteractiveError>(())
        }
        .await;
        if let Err(error) = configure {
            close_prepared(&mut prepared).await?;
            if cancellation.is_cancelled() {
                return Err(NativeInteractiveError::Closed);
            }
            return Err(error);
        }
        Ok(Prepared::Managed(prepared, reservation))
    })
}

fn start_parent_mcp(
    prepared: &PreparedManagedRuntime,
    phase: crate::mcp::startup::NativeMcpStartupPhase,
    cancellation: machine_god_core::CancellationToken,
) -> BoxFuture<'static, Result<(), NativeInteractiveError>> {
    let controller = prepared
        .resources
        .mcp_controls()
        .and_then(|controls| controls.controller);
    let binding = prepared.owner.binding();
    Box::pin(async move {
        let Some(controller) = controller else {
            return Ok(());
        };
        let admission = binding.prepare_admission()?;
        let completion = admission.cohort().map(|cohort| cohort.completion());
        let result = admission
            .wrap(controller.start_configured(phase, cancellation.clone()))
            .await;
        drop(admission);
        if let Err(failure) = result {
            use crate::mcp::controller::NativeMcpControllerError;
            let error = || {
                NativeInteractiveError::Conversation(
                    crate::NativeConversationError::McpRequiredUnavailable,
                )
            };
            if cancellation.is_cancelled()
                || matches!(
                    failure.kind(),
                    NativeMcpControllerError::Closed | NativeMcpControllerError::Cancelled
                )
            {
                return Err(error());
            }
            let deadline = controller
                .deadline_after(std::time::Duration::from_secs(30))
                .map_err(|_| error())?;
            // Co-poll retained failed-startup peers and the original admission,
            // while leaving the controller available for explicit /mcp repair.
            let cleanup = controller
                .settle_failed_startup(deadline, cancellation, completion)
                .await
                .map_err(|_| error())?;
            if !cleanup.complete {
                return Err(error());
            }
            return if phase == crate::mcp::startup::NativeMcpStartupPhase::All {
                Ok(())
            } else {
                Err(error())
            };
        }
        if let Some(completion) = completion {
            completion.wait().await;
        }
        Ok(())
    })
}

pub(super) fn enroll(
    agents: &mut Option<Box<Owner>>,
    prepared: Prepared,
) -> Result<Enrolled, EnrollmentError> {
    match prepared {
        Prepared::Ordinary(runtime) => Ok((runtime, None)),
        Prepared::Managed(prepared, reservation) => {
            let Some(agents) = agents else {
                return Err((NativeInteractiveError::Configuration, prepared, reservation));
            };
            let runtime = prepared.runtime.clone();
            match agents.agents.enroll_foreground(prepared, &reservation) {
                Ok(selection) => Ok((runtime, Some(selection))),
                Err((error, prepared)) => Err((
                    NativeInteractiveError::Managed(crate::reference_host::managed_error(error)),
                    prepared,
                    reservation,
                )),
            }
        }
    }
}

pub(super) async fn reserve_initial(
    agents: &mut crate::NativeManagedAgents,
    now_ms: i64,
    cancellation: &machine_god_core::CancellationToken,
) -> Result<ManagedForegroundReservation, NativeInteractiveError> {
    let reservation = agents
        .reserve_foreground()
        .map_err(NativeInteractiveError::Managed)?;
    let ready = futures_util::future::poll_fn(|cx| {
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(crate::NativeManagedAgentsError::Unavailable));
        }
        if let Poll::Ready(Err(error)) = agents.poll_progress(cx, now_ms) {
            return Poll::Ready(Err(error));
        }
        agents.poll_foreground_reservation(&reservation, cx)
    })
    .await;
    if cancellation.is_cancelled() {
        return Err(NativeInteractiveError::Closed);
    }
    ready.map_err(NativeInteractiveError::Managed)?;
    Ok(reservation)
}

pub(super) async fn close_prepared(
    prepared: &mut PreparedManagedRuntime,
) -> Result<(), NativeInteractiveError> {
    prepared.owner.retire();
    prepared.resources.begin_close();
    futures_util::future::poll_fn(|cx| prepared.resources.poll_closed(cx))
        .await
        .map_err(|error| {
            NativeInteractiveError::Managed(crate::reference_host::managed_error(error))
        })
}

pub(super) async fn activate_initial(
    host: &NativeReferenceHost,
    current: &NativeConversationRuntime,
    agents: &mut Option<Box<Owner>>,
    now_ms: i64,
) -> Result<(), NativeInteractiveError> {
    let result = async {
        host.terminal_lifecycle_requester()
            .ok_or(NativeInteractiveError::Configuration)?
            .activate_session(
                super::transition::principal(current),
                machine_god_core::CancellationToken::new(),
            )
            .await
            .map_err(NativeInteractiveError::Terminal)
    }
    .await;
    if result.is_err()
        && let Some(owner) = agents
    {
        // Enrollment already transferred original MCP/admission custody. A
        // failed terminal activation must drive that owner, not just drop it.
        futures_util::future::poll_fn(|cx| owner.agents.poll_shutdown(cx, now_ms))
            .await
            .map_err(NativeInteractiveError::Managed)?;
    }
    result
}
