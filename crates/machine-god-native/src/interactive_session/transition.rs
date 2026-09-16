use super::{
    Arc, BackgroundOutputOwner, BoxFuture, EngineEvent, NativeConversation,
    NativeConversationRuntime, NativeInteractiveError, NativeInteractiveRequestId,
    NativeInteractiveSessionOptions, NativeInteractiveTransition, NativeModelCatalog,
    NativeReferenceHost, NativeRuntimeQuiescence, NativeTerminalHandoffReceipt,
    NativeTerminalResetReceipt, NativeTerminalTransitionError,
};
use crate::NativePermissionPolicySnapshot;
use crate::file_undo::FileUndoClearReservation;
use machine_god_core::CancellationToken;

mod preparation;

pub(super) struct Request {
    pub id: NativeInteractiveRequestId,
    pub kind: NativeInteractiveTransition,
    pub now_ms: i64,
    pub staged: Option<Box<crate::reference_host::NativeManagedStagedParent>>,
    pub staged_cancellation: Option<CancellationToken>,
}
pub(super) struct Transition {
    pub request: Request,
    pub guard: Option<NativeRuntimeQuiescence>,
    pub phase: Phase,
    pub terminal: Option<EngineEvent>,
    pub prepared: Option<BackgroundOutputOwner>,
    pub managed_candidate: Option<crate::managed::manager::ManagedForegroundSelection>,
    pub managed_reservation: Option<crate::managed::manager::ManagedForegroundReservation>,
    pub preparation_cancel: CancellationToken,
    pub external_stage: bool,
}
pub(super) enum Phase {
    Draining,
    Waiting(BoxFuture<'static, Result<NativeRuntimeQuiescence, NativeInteractiveError>>),
    Preparing(BoxFuture<'static, Result<NativeConversation, NativeInteractiveError>>),
    Reserving(NativeConversation),
    Composing(BoxFuture<'static, Result<super::managed::Prepared, NativeInteractiveError>>),
    ComposingStaged(
        BoxFuture<'static, Result<super::managed::Prepared, super::managed::staged::Failure>>,
    ),
    SettlingStaged(BoxFuture<'static, super::managed::staged::Settled>),
    StagedFenced(Box<super::managed::staged::Failure>),
    Ready(Arc<NativeConversationRuntime>),
    Committing {
        candidate: Arc<NativeConversationRuntime>,
        undo: Option<FileUndoClearReservation>,
        future: BoxFuture<'static, CommitResult>,
    },
    Fenced {
        candidate: Arc<NativeConversationRuntime>,
        undo: Option<FileUndoClearReservation>,
        reset: Option<NativeTerminalResetReceipt>,
        handoff: Option<NativeTerminalHandoffReceipt>,
    },
}
pub(super) struct CommitResult {
    pub reset: Option<NativeTerminalResetReceipt>,
    pub handoff: Result<Option<NativeTerminalHandoffReceipt>, NativeTerminalTransitionError>,
    pub affected: bool,
}
impl Transition {
    pub fn new(request: Request) -> Self {
        let external_stage = request.staged.is_some();
        let preparation_cancel = request.staged_cancellation.clone().unwrap_or_default();
        Self {
            request,
            guard: None,
            phase: Phase::Draining,
            terminal: None,
            prepared: None,
            managed_candidate: None,
            managed_reservation: None,
            preparation_cancel,
            external_stage,
        }
    }

    pub fn staged_cancelled(&self) -> bool {
        self.external_stage && self.preparation_cancel.is_cancelled()
    }

    pub fn cancel_preparation(&self) {
        if !self.committed() {
            self.preparation_cancel.cancel();
        }
    }
    pub fn committed(&self) -> bool {
        matches!(self.phase, Phase::Committing { .. } | Phase::Fenced { .. })
    }
    pub fn is_fenced(&self) -> bool {
        matches!(self.phase, Phase::Fenced { .. } | Phase::StagedFenced(_))
    }
    pub fn reset_receipt(&self) -> Option<&NativeTerminalResetReceipt> {
        match &self.phase {
            Phase::Fenced { reset, .. } => reset.as_ref(),
            _ => None,
        }
    }
    pub fn handoff_receipt(&self) -> Option<&NativeTerminalHandoffReceipt> {
        match &self.phase {
            Phase::Fenced { handoff, .. } => handoff.as_ref(),
            _ => None,
        }
    }
    pub fn candidate_principal(&self) -> Option<BackgroundOutputOwner> {
        match &self.phase {
            Phase::Ready(candidate)
            | Phase::Committing { candidate, .. }
            | Phase::Fenced { candidate, .. } => Some(principal(candidate)),
            _ => None,
        }
    }
}
pub(super) fn principal(runtime: &NativeConversationRuntime) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(runtime.id(), runtime.incarnation_id())
}
pub(super) fn conversation_principal(conversation: &NativeConversation) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(conversation.id(), conversation.incarnation_id())
}

pub(super) async fn prepare(
    host: &NativeReferenceHost,
    options: &NativeInteractiveSessionOptions,
    kind: NativeInteractiveTransition,
    now_ms: i64,
) -> Result<NativeConversation, NativeInteractiveError> {
    match kind {
        NativeInteractiveTransition::Resume(crate::NativeResumeTarget::Observed(observed)) => {
            crate::session_resume::owned::resume(
                host.session_lifecycle(),
                observed,
                &options.workspace,
                now_ms,
                host.control_workers()
                    .ok_or(NativeInteractiveError::Configuration)?,
                CancellationToken::new(),
            )
            .await
            .map_err(NativeInteractiveError::Resume)
        }
        kind => {
            preparation::Preparation::new(
                host.session_lifecycle().clone(),
                options.clone(),
                kind,
                now_ms,
            )
            .run(
                host.control_workers()
                    .ok_or(NativeInteractiveError::Configuration)?,
            )
            .await
        }
    }
}

pub(super) async fn compose(
    host: &NativeReferenceHost,
    options: &NativeInteractiveSessionOptions,
    conversation: NativeConversation,
    policy: Option<NativePermissionPolicySnapshot>,
    catalog: Option<Arc<NativeModelCatalog>>,
    initial: bool,
    now_ms: i64,
) -> Result<Arc<NativeConversationRuntime>, NativeInteractiveError> {
    let conversation = match policy {
        Some(policy) => {
            host.configure_conversation_permissions_with_policy(conversation, policy)?
        }
        None => host.configure_conversation_permissions(conversation)?,
    };
    let conversation = host.configure_conversation_workspace(conversation)?;
    let conversation = host.configure_conversation_mcp(conversation)?;
    let observations = host
        .observations()
        .ok_or(NativeInteractiveError::Configuration)?;
    let conversation = conversation.with_observations(&observations)?;
    let routes = host
        .model_routes()
        .ok_or(NativeInteractiveError::Configuration)?;
    let runtime = NativeConversationRuntime::new_with_model_routes(
        conversation,
        options.defaults.clone(),
        if initial {
            options.process_model.as_deref()
        } else {
            None
        },
        &routes,
    )?;
    if let Some(catalog) = catalog {
        runtime.set_model_catalog(catalog)?;
    }
    // Persist selected workspace defaults for a fresh candidate before any old
    // terminal effects. Resume's already saved selection remains a checked no-op.
    crate::session_resume::owned::flush_candidate(host, &runtime, now_ms).await?;
    Ok(Arc::new(runtime))
}

pub(super) fn commit(
    host: &NativeReferenceHost,
    kind: &NativeInteractiveTransition,
    source: BackgroundOutputOwner,
    destination: BackgroundOutputOwner,
) -> Result<BoxFuture<'static, CommitResult>, NativeInteractiveError> {
    let requester = host
        .terminal_lifecycle_requester()
        .ok_or(NativeInteractiveError::Configuration)?;
    let reset_required = matches!(
        kind,
        NativeInteractiveTransition::Reset | NativeInteractiveTransition::Resume(_)
    );
    Ok(Box::pin(async move {
        let mut reset = None;
        if reset_required {
            match requester
                .reset_current_workspace(source.clone(), CancellationToken::new())
                .await
            {
                Ok(receipt) => reset = Some(receipt),
                Err(error) => {
                    return CommitResult {
                        reset,
                        affected: error == NativeTerminalTransitionError::Uncertain,
                        handoff: Err(error),
                    };
                }
            }
        }
        if let Err(error) = requester
            .activate_session(destination.clone(), CancellationToken::new())
            .await
        {
            return CommitResult {
                affected: reset.is_some() || error == NativeTerminalTransitionError::Uncertain,
                reset,
                handoff: Err(error),
            };
        }
        // Same-transcript ACP replacement still reset the old access generation
        // and activated a fresh one above. There is no different principal to
        // transfer to, so do not call self-handoff or invent a handoff receipt.
        let handoff = if source == destination && reset.is_some() {
            Ok(None)
        } else {
            requester
                .handoff(source, destination, CancellationToken::new())
                .await
                .map(Some)
        };
        let affected = reset.is_some()
            || handoff.is_ok()
            || matches!(handoff, Err(NativeTerminalTransitionError::Uncertain));
        CommitResult {
            reset,
            handoff,
            affected,
        }
    }))
}
