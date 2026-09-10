//! Concrete native interactive ownership; presentation never owns lifecycle work.

use crate::{
    NativeConversation, NativeConversationError, NativeConversationRuntime,
    NativeConversationRuntimeError, NativeConversationRuntimeTurn, NativeModelCatalog,
    NativeModelPreferences, NativeQueuedJobId, NativeReferenceHost, NativeResumeTarget,
    NativeRuntimeQuiescence, NativeSessionResumeError, NativeTerminalHandoffReceipt,
    NativeTerminalResetReceipt, NativeTerminalTransitionError,
};
use machine_god_core::{BackgroundOutputOwner, BoxFuture, EngineEvent, Prompt};
use std::{
    fmt,
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll, Waker},
};

mod clipboard;
mod controls;
pub use clipboard::{
    NativeInteractiveCopyError, NativeInteractiveCopyId, NativeInteractiveCopyOutcome,
    NativeInteractiveCopyReceipt,
};
pub use controls::{
    NativeInteractiveControl, NativeInteractiveControlError, NativeInteractiveControlId,
    NativeInteractiveControlOutcome, NativeInteractiveControlReceipt,
};
mod driver;
#[cfg(test)]
mod tests;
mod transition;
use transition::{Request, Transition};

/// Explicit verified workspace inputs. These are workspace defaults, not a
/// previous session's saved selection or a filesystem authority constructor.
#[derive(Clone)]
pub struct NativeInteractiveSessionOptions {
    workspace: PathBuf,
    defaults: NativeModelPreferences,
    process_model: Option<String>,
    catalog: Option<Arc<NativeModelCatalog>>,
    clipboard: Option<(
        crate::NativeClipboardExecutable,
        Vec<(std::ffi::OsString, std::ffi::OsString)>,
    )>,
    background_url: Option<(
        crate::NativeBackgroundUrlExecutable,
        Vec<(std::ffi::OsString, std::ffi::OsString)>,
    )>,
}
impl fmt::Debug for NativeInteractiveSessionOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractiveSessionOptions { .. }")
    }
}
impl NativeInteractiveSessionOptions {
    /// The workspace must be the host's verified canonical tool root.
    /// # Errors
    /// Rejects noncanonical lexical workspace paths.
    pub fn new(
        workspace: PathBuf,
        workspace_defaults: NativeModelPreferences,
    ) -> Result<Self, NativeInteractiveError> {
        crate::NativeSessionMetadata::new(&workspace, 0, crate::NativeSessionOrigin::Cli)
            .map_err(|_| NativeInteractiveError::Configuration)?;
        Ok(Self {
            workspace,
            defaults: workspace_defaults,
            process_model: None,
            catalog: None,
            clipboard: None,
            background_url: None,
        })
    }
    /// Applies only to initial startup; fresh transitions use workspace defaults.
    /// # Errors
    /// Rejects an invalid model identifier.
    pub fn with_process_model_override(
        mut self,
        model: &str,
    ) -> Result<Self, NativeInteractiveError> {
        let mut checked = self.defaults.clone();
        checked
            .set_model(model)
            .map_err(|_| NativeInteractiveError::Configuration)?;
        self.process_model = Some(model.to_owned());
        Ok(self)
    }
    #[must_use]
    pub fn with_catalog(mut self, catalog: Arc<NativeModelCatalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }
    /// Retains explicit clipboard authority without inspecting or starting it.
    /// Missing or invalid optional clipboard authority never prevents startup.
    #[must_use]
    pub fn with_clipboard(
        mut self,
        executable: crate::NativeClipboardExecutable,
        environment: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    ) -> Self {
        self.clipboard = Some((executable, environment));
        self
    }

    /// Retains explicit desktop launcher authority without inspecting or starting it.
    /// Invalid optional authority disables URL opening, not interactive startup.
    #[must_use]
    pub fn with_background_url_opener(
        mut self,
        executable: crate::NativeBackgroundUrlExecutable,
        environment: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    ) -> Self {
        self.background_url = Some((executable, environment));
        self
    }
}

#[derive(Clone, Debug)]
pub enum NativeInteractiveInitialSession {
    Fresh,
    Resume(NativeResumeTarget),
}

#[derive(Clone, Debug)]
pub enum NativeInteractiveTransition {
    Clear,
    New,
    Reset,
    Resume(NativeResumeTarget),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeInteractiveRequestId(u64);
impl NativeInteractiveRequestId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeInteractiveRequestReceipt {
    pub id: NativeInteractiveRequestId,
    pub superseded: Option<NativeInteractiveRequestId>,
}

/// Fixed categories. Errors may follow candidate publication; never retry an
/// uncertain terminal operation automatically.
#[derive(Debug)]
pub enum NativeInteractiveError {
    Configuration,
    Unavailable,
    Busy,
    Closed,
    IdentityExhausted,
    /// Exact control error or partial-target receipt is retained separately.
    ControlFailed,
    Runtime(NativeConversationRuntimeError),
    Conversation(NativeConversationError),
    Resume(NativeSessionResumeError),
    Terminal(NativeTerminalTransitionError),
    Undo(crate::FileUndoError),
}
impl fmt::Display for NativeInteractiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native interactive operation unavailable")
    }
}
impl std::error::Error for NativeInteractiveError {}
impl From<NativeConversationRuntimeError> for NativeInteractiveError {
    fn from(value: NativeConversationRuntimeError) -> Self {
        Self::Runtime(value)
    }
}
impl From<NativeConversationError> for NativeInteractiveError {
    fn from(value: NativeConversationError) -> Self {
        Self::Conversation(value)
    }
}

/// Confirmed lifecycle receipt. A reset may explicitly retain indeterminate
/// resources; successful transfer does not upgrade their cleanup outcome.
pub struct NativeInteractiveTransitionReceipt {
    pub request: NativeInteractiveRequestId,
    pub source: BackgroundOutputOwner,
    pub destination: BackgroundOutputOwner,
    pub unchanged: bool,
    pub reset: Option<NativeTerminalResetReceipt>,
    pub handoff: Option<NativeTerminalHandoffReceipt>,
    pub settled_turn: Option<EngineEvent>,
}
impl fmt::Debug for NativeInteractiveTransitionReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractiveTransitionReceipt { .. }")
    }
}

/// One retained control outcome, separately consumed from streamed presentation.
pub enum NativeInteractiveOutcome {
    Turn(Result<EngineEvent, NativeInteractiveError>),
    Transition(NativeInteractiveTransitionReceipt),
    Rejected {
        request: NativeInteractiveRequestId,
        error: NativeInteractiveError,
        settled_turn: Option<EngineEvent>,
        /// Known already-prepared identity; absence does not prove no publication.
        candidate: Option<BackgroundOutputOwner>,
    },
    Superseded {
        request: NativeInteractiveRequestId,
        candidate: Option<BackgroundOutputOwner>,
        error: Option<NativeInteractiveError>,
        settled_turn: Option<EngineEvent>,
    },
    /// The owner is fenced and retains the candidate and any reset receipt.
    Indeterminate {
        request: NativeInteractiveRequestId,
        error: NativeInteractiveError,
        settled_turn: Option<EngineEvent>,
    },
    Shutdown,
}
impl fmt::Debug for NativeInteractiveOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractiveOutcome { .. }")
    }
}

/// Owns the actual host runtime and all started admission/transition futures.
/// Dropping an outer poll wrapper never drops those operations. Dropping this
/// owner itself is abandonment, not a cleanup/persistence receipt.
pub struct NativeInteractiveSession {
    host: Arc<NativeReferenceHost>,
    options: NativeInteractiveSessionOptions,
    current: Arc<NativeConversationRuntime>,
    admission: Option<
        BoxFuture<
            'static,
            Result<Option<NativeConversationRuntimeTurn>, NativeConversationRuntimeError>,
        >,
    >,
    turn: Option<NativeConversationRuntimeTurn>,
    transition: Option<Transition>,
    pending: Option<Request>,
    next_request: u64,
    presentation: Option<EngineEvent>,
    outcome: Option<NativeInteractiveOutcome>,
    control: Option<controls::OwnedControl>,
    control_outcome: Option<NativeInteractiveControlOutcome>,
    next_control: u64,
    clipboard: Result<crate::NativeClipboard, crate::NativeClipboardError>,
    background_opener: Option<crate::NativeBackgroundUrlOpener>,
    copy: Option<clipboard::OwnedCopy>,
    copy_outcome: Option<NativeInteractiveCopyOutcome>,
    next_copy: u64,
    cancel_requested: bool,
    shutting_down: bool,
    closed: bool,
    shutdown_error: Option<NativeInteractiveError>,
    wake: Option<Waker>,
}
impl fmt::Debug for NativeInteractiveSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeInteractiveSession { .. }")
    }
}
impl NativeInteractiveSession {
    /// Constructs the initial runtime through this exact complete host. Startup
    /// is inert before poll; abandoned startup may have persisted a fresh ID.
    #[must_use]
    pub fn open(
        host: Arc<NativeReferenceHost>,
        mut options: NativeInteractiveSessionOptions,
        initial: NativeInteractiveInitialSession,
        now_ms: i64,
    ) -> BoxFuture<'static, Result<Self, NativeInteractiveError>> {
        Box::pin(async move {
            if options.workspace != host.workspace_root()
                || host.undo_tracker().is_none()
                || host.terminal_lifecycle_requester().is_none()
                || host.model_routes().is_none()
                || host.observations().is_none()
            {
                return Err(NativeInteractiveError::Configuration);
            }
            let conversation = transition::prepare(
                &host,
                &options,
                match initial {
                    NativeInteractiveInitialSession::Fresh => NativeInteractiveTransition::New,
                    NativeInteractiveInitialSession::Resume(target) => {
                        NativeInteractiveTransition::Resume(target)
                    }
                },
                now_ms,
            )
            .await?;
            let current = transition::compose(
                &host,
                &options,
                conversation,
                None,
                options.catalog.clone(),
                true,
                now_ms,
            )
            .await?;
            host.terminal_lifecycle_requester()
                .ok_or(NativeInteractiveError::Configuration)?
                .activate_session(
                    transition::principal(&current),
                    machine_god_core::CancellationToken::new(),
                )
                .await
                .map_err(NativeInteractiveError::Terminal)?;
            let clipboard = options.clipboard.take().map_or(
                Err(crate::NativeClipboardError::Unavailable),
                |(executable, environment)| {
                    crate::NativeClipboard::new(
                        executable,
                        options.workspace.clone(),
                        environment,
                        host.control_workers()
                            .ok_or(crate::NativeClipboardError::Unavailable)?,
                    )
                },
            );
            let background_opener =
                options
                    .background_url
                    .take()
                    .and_then(|(executable, environment)| {
                        crate::NativeBackgroundUrlOpener::from_executable(
                            executable,
                            environment,
                            host.control_workers()?,
                        )
                        .ok()
                    });
            Ok(Self {
                host,
                options,
                current,
                admission: None,
                turn: None,
                transition: None,
                pending: None,
                next_request: 1,
                presentation: None,
                outcome: None,
                control: None,
                control_outcome: None,
                next_control: 1,
                clipboard,
                background_opener,
                copy: None,
                copy_outcome: None,
                next_copy: 1,
                cancel_requested: false,
                shutting_down: false,
                closed: false,
                shutdown_error: None,
                wake: None,
            })
        })
    }
    #[must_use]
    pub fn runtime(&self) -> &Arc<NativeConversationRuntime> {
        &self.current
    }
    /// # Errors
    /// Rejects transition/shutdown admission and native queue bounds.
    pub fn enqueue(&mut self, prompt: Prompt) -> Result<NativeQueuedJobId, NativeInteractiveError> {
        if self.closed || self.shutting_down {
            return Err(NativeInteractiveError::Closed);
        }
        if self.transition.is_some() || self.pending.is_some() {
            return Err(NativeInteractiveError::Busy);
        }
        let id = self.current.enqueue(prompt)?;
        self.notify();
        Ok(id)
    }
    /// Last request wins before terminal commit starts. Started preparation is
    /// still driven to its result, including a superseded publication receipt.
    /// # Errors
    /// Rejects shutdown, indeterminate ownership or exhausted request identities.
    pub fn request_transition(
        &mut self,
        kind: NativeInteractiveTransition,
        now_ms: i64,
    ) -> Result<NativeInteractiveRequestReceipt, NativeInteractiveError> {
        if self.closed || self.shutting_down {
            return Err(NativeInteractiveError::Closed);
        }
        if self.is_fenced() {
            return Err(NativeInteractiveError::Busy);
        }
        if self
            .control_outcome
            .as_ref()
            .is_some_and(NativeInteractiveControlOutcome::failed)
        {
            return Err(NativeInteractiveError::ControlFailed);
        }
        let id = NativeInteractiveRequestId(self.next_request);
        self.next_request = self
            .next_request
            .checked_add(1)
            .ok_or(NativeInteractiveError::IdentityExhausted)?;
        let superseded = self.pending.as_ref().map(|request| request.id).or_else(|| {
            self.transition
                .as_ref()
                .filter(|transition| !transition.committed())
                .map(|transition| transition.request.id)
        });
        self.pending = Some(Request { id, kind, now_ms });
        self.cancel_copy();
        self.presentation.take();
        self.notify();
        Ok(NativeInteractiveRequestReceipt { id, superseded })
    }
    pub fn request_shutdown(&mut self) {
        self.cancel_copy();
        self.cancel_background_control();
        self.shutting_down = true;
        self.pending.take();
        self.presentation.take();
        self.notify();
    }
    /// Requests cancellation of the owned admission/current turn without
    /// replacing the session or discarding queued input. Accepted publications
    /// settle first; cancellation never drops their metadata editor.
    /// Returns acceptance, not a settled turn or persistence receipt.
    pub fn request_cancel(&mut self) -> bool {
        if !self.closed && !self.shutting_down && self.cancel_background_control() {
            self.notify();
            return true;
        }
        if self.closed || self.shutting_down || (self.admission.is_none() && self.turn.is_none()) {
            return false;
        }
        self.cancel_requested = true;
        self.notify();
        true
    }

    fn cancel_background_control(&self) -> bool {
        if let Some(token) = self
            .control
            .as_ref()
            .and_then(|control| control.cancellation.as_ref())
        {
            token.cancel();
            return true;
        }
        false
    }
    #[must_use]
    pub fn take_presentation(&mut self) -> Option<EngineEvent> {
        let event = self.presentation.take();
        if event.is_some() {
            self.notify();
        }
        event
    }
    #[must_use]
    pub fn take_outcome(&mut self) -> Option<NativeInteractiveOutcome> {
        let event = self.outcome.take();
        if event.is_some() {
            self.notify();
        }
        event
    }
    #[must_use]
    pub fn is_fenced(&self) -> bool {
        self.transition.as_ref().is_some_and(Transition::is_fenced)
    }
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.closed
    }
    /// Retained shutdown failure independent of an unconsumed earlier outcome.
    #[must_use]
    pub fn shutdown_error(&self) -> Option<&NativeInteractiveError> {
        self.shutdown_error.as_ref()
    }
    #[must_use]
    pub fn retained_reset_receipt(&self) -> Option<&NativeTerminalResetReceipt> {
        self.transition.as_ref().and_then(Transition::reset_receipt)
    }
    #[must_use]
    pub fn retained_handoff_receipt(&self) -> Option<&NativeTerminalHandoffReceipt> {
        self.transition
            .as_ref()
            .and_then(Transition::handoff_receipt)
    }
    #[must_use]
    pub fn retained_candidate(&self) -> Option<BackgroundOutputOwner> {
        self.transition
            .as_ref()
            .and_then(Transition::candidate_principal)
    }
    /// Drives owned work with an explicit admission time. Ready means a retained
    /// presentation/control outcome can be taken; it does not consume that value.
    pub fn poll_progress(&mut self, cx: &mut Context<'_>, now_ms: i64) -> Poll<()> {
        self.drive(cx, now_ms)
    }
    fn notify(&mut self) {
        if let Some(wake) = self.wake.take() {
            wake.wake();
        }
    }
}
