//! Native ACP semantics. The wire host supplies explicitly prepared authority.

use crate::{
    NativeConversationRuntime, NativeInteractiveError, NativeInteractiveInitialSession,
    NativeInteractiveOutcome, NativeInteractiveSession, NativeInteractiveSessionOptions,
    NativeModelPreferencePersistence, NativeQueuedJobId, NativeReferenceHost, NativeResumeTarget,
    NativeSessionCatalogCursor, NativeSessionCatalogPage, NativeSessionCatalogReadError,
    NativeSessionCatalogScope, NativeSessionOrigin, PermissionMode,
};
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, EngineEvent, SessionId,
};
use std::{
    fmt,
    sync::Arc,
    task::{Context, Poll},
};

mod history;
mod prompt;
pub use history::NativeAcpHistory;
pub use prompt::{
    MAX_ACP_PROMPT_BLOCKS, MAX_ACP_PROMPT_BYTES, MAX_ACP_RESOURCE_URI_BYTES, NativeAcpPrompt,
    NativeAcpResourceOmission, NativeAcpResourceOmissionReason, decode_prompt_input,
};

/// ACP selects native records only. A load replays presentation; a resume does not.
#[derive(Clone, Debug)]
pub enum NativeAcpSessionSelection {
    New,
    Load(SessionId),
    Resume(SessionId),
}

/// Accepted native configuration change; model persistence is a separate receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAcpConfigChange {
    Model { generation: u64 },
    Mode(PermissionMode),
}

/// Data-free ACP semantic errors. Do not expose nested host diagnostics on the wire.
#[derive(Debug)]
pub enum AcpSessionError {
    InvalidPrompt,
    UnsupportedContent,
    Limit,
    WrongSession,
    Busy,
    Closed,
    Cancelled,
    InvalidConfiguration,
    Unavailable,
    Native(NativeInteractiveError),
}
impl fmt::Display for AcpSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidPrompt => "ACP prompt is invalid",
            Self::UnsupportedContent => "ACP prompt content is unsupported",
            Self::Limit => "ACP resource limit exceeded",
            Self::WrongSession => "ACP session identity does not match",
            Self::Busy => "ACP session is busy",
            Self::Closed => "ACP session is closed",
            Self::Cancelled => "ACP operation was cancelled",
            Self::InvalidConfiguration => "ACP configuration is invalid",
            Self::Unavailable | Self::Native(_) => "ACP native operation is unavailable",
        })
    }
}
impl std::error::Error for AcpSessionError {}
impl From<NativeInteractiveError> for AcpSessionError {
    fn from(error: NativeInteractiveError) -> Self {
        Self::Native(error)
    }
}

/// One native session owner. It never discovers profiles or builds an engine.
/// Keep polling after cancel/close until the retained native outcome settles.
pub struct NativeAcpSession {
    inner: NativeInteractiveSession,
    history: Option<NativeAcpHistory>,
    prompt: Option<NativeQueuedJobId>,
    model_save: Option<crate::NativeInteractiveControlId>,
    command_control: Option<(crate::NativeInteractiveControlId, BackgroundOutputOwner)>,
    pub(crate) command_services: super::commands::Services,
    cancelling: bool,
    closing: bool,
}
impl fmt::Debug for NativeAcpSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeAcpSession").finish_non_exhaustive()
    }
}
impl NativeAcpSession {
    /// The caller must prepare an authoritative ephemeral MCP selection before
    /// supplying the host. This constructor does no I/O before its first poll.
    /// The host must satisfy `NativeInteractiveSession::open`'s existing native
    /// worker, terminal/helper, observation, undo and model-route requirements.
    /// # Errors
    /// Rejects unavailable native composition or an invalid native resume target.
    #[must_use]
    pub fn open(
        host: Arc<NativeReferenceHost>,
        options: NativeInteractiveSessionOptions,
        selection: NativeAcpSessionSelection,
        now_ms: i64,
    ) -> BoxFuture<'static, Result<Self, AcpSessionError>> {
        Box::pin(async move {
            let command_services = super::commands::Services::from_host(&host);
            let replay = matches!(selection, NativeAcpSessionSelection::Load(_));
            let initial = match selection {
                NativeAcpSessionSelection::New => NativeInteractiveInitialSession::Fresh,
                NativeAcpSessionSelection::Load(id) | NativeAcpSessionSelection::Resume(id) => {
                    NativeInteractiveInitialSession::Resume(NativeResumeTarget::Exact(id))
                }
            };
            let inner = NativeInteractiveSession::open(
                host,
                options.with_origin(NativeSessionOrigin::Acp),
                initial,
                now_ms,
            )
            .await?;
            let history = replay.then(|| NativeAcpHistory::new(inner.runtime().record_snapshot()));
            Ok(Self {
                inner,
                history,
                prompt: None,
                model_save: None,
                command_control: None,
                command_services,
                cancelling: false,
                closing: false,
            })
        })
    }

    #[must_use]
    pub fn id(&self) -> SessionId {
        self.inner.runtime().id()
    }

    /// The incarnation-bearing identity for prompt, interaction and MCP custody.
    #[must_use]
    pub fn principal(&self) -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(self.id(), self.inner.runtime().incarnation_id())
    }

    #[must_use]
    pub fn runtime(&self) -> &Arc<NativeConversationRuntime> {
        self.inner.runtime()
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    #[must_use]
    pub fn has_pending_prompt(&self) -> bool {
        self.prompt.is_some()
    }

    #[must_use]
    pub fn has_pending_model_save(&self) -> bool {
        self.model_save.is_some()
    }

    #[must_use]
    pub fn has_pending_command_control(&self) -> bool {
        self.command_control.is_some()
    }

    pub(crate) fn check_command_admission(
        &self,
        expected: &SessionId,
    ) -> Result<(), AcpSessionError> {
        self.check_session(expected)?;
        if self.prompt.is_some() || self.model_save.is_some() || self.command_control.is_some() {
            return Err(AcpSessionError::Busy);
        }
        Ok(())
    }

    /// Admits only same-session ACP command operations to the owned native lane.
    /// # Errors
    /// Rejects foreign identity, unsupported controls or occupied prompt/save lanes.
    pub fn request_command_control(
        &mut self,
        expected: &SessionId,
        control: crate::NativeInteractiveControl,
        now_ms: i64,
    ) -> Result<crate::NativeInteractiveControlId, AcpSessionError> {
        self.check_command_admission(expected)?;
        if !matches!(
            &control,
            crate::NativeInteractiveControl::UndoLast
                | crate::NativeInteractiveControl::Compact
                | crate::NativeInteractiveControl::SaveModelSession
                | crate::NativeInteractiveControl::Skills {
                    command: crate::NativeSkillsCommand::List
                }
                | crate::NativeInteractiveControl::Mcp {
                    command: crate::mcp::commands::McpCommand::Feature(_)
                }
        ) {
            return Err(AcpSessionError::Unavailable);
        }
        let principal = self.principal();
        let id = self.inner.request_control(control, now_ms)?;
        self.command_control = Some((id, principal));
        Ok(id)
    }

    /// Drains accepted command receipts even while close/shutdown is fenced.
    /// Model-save receipts have independent custody and are never consumed here.
    #[must_use]
    pub fn take_command_control_outcome(
        &mut self,
    ) -> Option<crate::NativeInteractiveControlOutcome> {
        let (expected, source) = self.command_control.as_ref()?;
        let outcome = self.inner.take_control_outcome()?;
        debug_assert!(
            *expected == outcome.id,
            "native command receipt ID mismatch"
        );
        debug_assert!(
            *source == outcome.source,
            "native command receipt principal mismatch"
        );
        self.command_control = None;
        if self.prompt.is_none() {
            self.cancelling = false;
        }
        Some(outcome)
    }

    /// Accepts session preferences; this is not a durable save receipt.
    /// # Errors
    /// Rejects foreign identity or occupied native work lanes.
    pub fn set_command_model_preferences(
        &mut self,
        expected: &SessionId,
        preferences: crate::NativeModelPreferences,
    ) -> Result<u64, AcpSessionError> {
        self.check_command_admission(expected)?;
        self.inner
            .set_model_preferences(preferences)
            .map_err(Into::into)
    }

    /// A failed shutdown remains owned and fenced, not a successful close.
    #[must_use]
    pub fn shutdown_error(&self) -> Option<&NativeInteractiveError> {
        self.inner.shutdown_error()
    }

    #[must_use]
    pub fn is_fenced(&self) -> bool {
        self.inner.is_fenced()
    }

    /// Returns a streaming, inert view of the captured native load checkpoint.
    #[must_use]
    pub fn take_loaded_history(&mut self) -> Option<NativeAcpHistory> {
        self.history.take()
    }

    fn check_session(&self, expected: &SessionId) -> Result<(), AcpSessionError> {
        if expected != &self.id() {
            return Err(AcpSessionError::WrongSession);
        }
        if self.closing {
            return Err(AcpSessionError::Closed);
        }
        Ok(())
    }

    /// Admits one prompt only; native admission and persistence remain owned.
    /// # Errors
    /// Rejects a stale identity, another pending prompt, or invalid native input.
    pub fn enqueue(
        &mut self,
        expected: &SessionId,
        prompt: NativeAcpPrompt,
    ) -> Result<NativeQueuedJobId, AcpSessionError> {
        self.check_session(expected)?;
        if self.prompt.is_some() || self.command_control.is_some() {
            return Err(AcpSessionError::Busy);
        }
        let job = self.inner.enqueue_acp(prompt)?;
        self.prompt = Some(job);
        Ok(job)
    }

    /// Acceptance is not completion, including cancellation before admission.
    /// # Errors
    /// Rejects a stale or closed session identity.
    pub fn request_cancel(&mut self, expected: &SessionId) -> Result<bool, AcpSessionError> {
        self.check_session(expected)?;
        self.cancelling = self.prompt.is_some() || self.command_control.is_some();
        if self.cancelling {
            self.inner.request_cancel();
        }
        Ok(self.cancelling)
    }

    /// Closes live ownership, never deletes saved session history.
    /// # Errors
    /// Rejects a stale or already closed session identity.
    pub fn request_close(&mut self, expected: &SessionId) -> Result<(), AcpSessionError> {
        self.check_session(expected)?;
        self.closing = true;
        self.inner.request_shutdown();
        Ok(())
    }

    /// Advances owned native work. Output backpressure must not drop this owner.
    pub fn poll_progress(&mut self, cx: &mut Context<'_>, now_ms: i64) -> Poll<()> {
        if self.cancelling {
            self.inner.request_cancel();
        }
        let result = self.inner.poll_progress(cx, now_ms);
        // A cancellation can arrive while a queued prompt has not yet entered
        // native admission. Keep the intent until that owned admission exists.
        if self.cancelling {
            self.inner.request_cancel();
        }
        result
    }

    #[must_use]
    pub fn take_presentation(&mut self) -> Option<EngineEvent> {
        self.inner.take_presentation()
    }

    /// Only a native Turn outcome is prompt completion: streamed terminal events
    /// are not checkpoint/history completion receipts.
    #[must_use]
    pub fn take_outcome(&mut self) -> Option<NativeInteractiveOutcome> {
        let outcome = self.inner.take_outcome()?;
        if matches!(
            outcome,
            NativeInteractiveOutcome::Turn(_) | NativeInteractiveOutcome::Shutdown
        ) {
            self.prompt = None;
            self.cancelling = false;
        }
        Some(outcome)
    }

    /// Changes future taken jobs; does not persist a rule or alter a running turn.
    /// # Errors
    /// Rejects stale identity, unsupported mode or unavailable native policy.
    pub fn set_mode(
        &self,
        expected: &SessionId,
        mode: &str,
    ) -> Result<PermissionMode, AcpSessionError> {
        self.check_session(expected)?;
        let mode = parse_mode(mode)?;
        self.inner
            .runtime()
            .permissions()
            .ok_or(AcpSessionError::Unavailable)?
            .set_mode(mode)
            .map_err(|_| AcpSessionError::Unavailable)?;
        Ok(mode)
    }

    /// Returns the current session mode, not a newly created profile controller.
    /// # Errors
    /// Rejects unavailable or retired native permission ownership.
    pub fn mode(&self) -> Result<PermissionMode, AcpSessionError> {
        self.inner
            .runtime()
            .permissions()
            .ok_or(AcpSessionError::Unavailable)?
            .snapshot()
            .map(|snapshot| snapshot.mode())
            .map_err(|_| AcpSessionError::Unavailable)
    }

    /// Changes the native session model or permission mode. A model acceptance
    /// generation is not a persistence receipt; call `flush_model` before
    /// reporting saved state. Permission modes never write persistent rules.
    /// # Errors
    /// Rejects stale identity, unsupported option/model or unavailable ownership.
    pub fn set_config_option(
        &mut self,
        expected: &SessionId,
        option: &str,
        value: &str,
    ) -> Result<NativeAcpConfigChange, AcpSessionError> {
        self.check_session(expected)?;
        if option == "mode" {
            return self
                .set_mode(expected, value)
                .map(NativeAcpConfigChange::Mode);
        }
        if option != "model" {
            return Err(AcpSessionError::InvalidConfiguration);
        }
        let mut preferences = self.inner.runtime().model_preferences();
        preferences
            .set_model(value)
            .map_err(|_| AcpSessionError::InvalidConfiguration)?;
        self.inner
            .set_model_preferences(preferences)
            .map(|generation| NativeAcpConfigChange::Model { generation })
            .map_err(Into::into)
    }

    /// Accepts an owned session-only save. No user configuration is written.
    /// The publication remains in the native control lane until its receipt.
    /// # Errors
    /// Rejects a foreign identity or an occupied/unavailable native control lane.
    pub fn request_model_save(
        &mut self,
        expected: &SessionId,
        now_ms: i64,
    ) -> Result<crate::NativeInteractiveControlId, AcpSessionError> {
        self.check_session(expected)?;
        if self.model_save.is_some() || self.command_control.is_some() {
            return Err(AcpSessionError::Busy);
        }
        let id = self
            .inner
            .request_control(crate::NativeInteractiveControl::SaveModelSession, now_ms)?;
        self.model_save = Some(id);
        Ok(id)
    }

    /// Returns only the receipt for this facade's exact accepted native save.
    #[must_use]
    pub fn take_model_save_outcome(
        &mut self,
    ) -> Option<Result<NativeModelPreferencePersistence, AcpSessionError>> {
        let expected = self.model_save?;
        let outcome = self.inner.take_control_outcome()?;
        self.model_save = None;
        if outcome.id != expected || outcome.source != self.principal() {
            return Some(Err(AcpSessionError::WrongSession));
        }
        Some(match outcome.result {
            Ok(crate::NativeInteractiveControlReceipt::ModelSession(receipt)) => Ok(receipt),
            Err(crate::NativeInteractiveControlError::Runtime(error)) => Err(
                AcpSessionError::Native(NativeInteractiveError::Runtime(error)),
            ),
            _ => Err(AcpSessionError::Unavailable),
        })
    }

    /// Session-only persistence, never user configuration. Inert before polling;
    /// dropping this wrapper retains any accepted save in the native control lane.
    /// # Errors
    /// Rejects stale identity or a failed native session checkpoint.
    #[must_use]
    pub fn flush_model(
        &mut self,
        expected: &SessionId,
        now_ms: i64,
    ) -> BoxFuture<'_, Result<NativeModelPreferencePersistence, AcpSessionError>> {
        let expected = expected.clone();
        Box::pin(async move {
            self.request_model_save(&expected, now_ms)?;
            futures_util::future::poll_fn(|cx| {
                let _ = self.poll_progress(cx, now_ms);
                self.take_model_save_outcome()
                    .map_or(Poll::Pending, Poll::Ready)
            })
            .await
        })
    }

    /// Uses the injected host's bounded native catalog; workspace scope is a
    /// descriptive filter and does not open or grant filesystem authority.
    #[must_use]
    pub fn list(
        host: &NativeReferenceHost,
        scope: NativeSessionCatalogScope,
        limit: usize,
        continuation: Option<NativeSessionCatalogCursor>,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        let reader = host.session_catalog_reader();
        Box::pin(async move { reader?.list(scope, limit, continuation, cancel).await })
    }
}

fn parse_mode(mode: &str) -> Result<PermissionMode, AcpSessionError> {
    match mode {
        "ask" => Ok(PermissionMode::Ask),
        "auto" => Ok(PermissionMode::Auto),
        "yolo" => Ok(PermissionMode::Yolo),
        _ => Err(AcpSessionError::InvalidConfiguration),
    }
}

#[cfg(test)]
mod tests;
