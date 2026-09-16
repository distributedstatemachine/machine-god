//! Native managed observation/admission. UI pages never replace the parent runtime.
mod drafts;
mod processes;
mod state;
mod view;
use super::NativeInteractiveSession;
use crate::{
    NativeManagedCatalogCursor, NativeManagedCatalogError, NativeManagedCatalogFilter,
    NativeManagedCatalogOutcome, NativeManagedCatalogRequest, NativeManagedCommandResponse,
    NativeObservedManagedAgent,
};
pub use drafts::NativeManagedDraftView;
use machine_god_core::{CancellationToken, ManagedSubagentCommand, ManagedSubagentError};
pub(super) use state::Navigation;
pub use view::{
    NativeManagedEditorIdentity, NativeManagedFrameIdentity, NativeManagedNavigationAction,
    NativeManagedNavigationError, NativeManagedNavigationRoute, NativeManagedNavigationView,
    NativeManagedProcessScope,
};

impl NativeInteractiveSession {
    /// Retains unsent child text/cursor under the exact current editor identity.
    /// It grants no command admission and never executes or persists a message.
    /// # Errors
    /// Rejects stale/non-child editors, invalid text/cursors and bounded capacity.
    pub fn edit_managed_draft(
        &mut self,
        editor: &NativeManagedEditorIdentity,
        value: &str,
        cursor: usize,
    ) -> Result<(), NativeManagedNavigationError> {
        self.navigation_available()?;
        let result = self
            .navigation
            .as_mut()
            .ok_or(NativeManagedNavigationError::Unavailable)?
            .edit_draft(editor, value, cursor);
        self.notify();
        result
    }

    /// Replaces only the current form field under its original editor identity.
    /// This cannot execute a command or change the selected target. Submission
    /// still requires acknowledgement of the resulting frame.
    /// # Errors
    /// Rejects retired editors, pending work, non-form routes and invalid fields.
    pub fn edit_managed_form(
        &mut self,
        editor: &NativeManagedEditorIdentity,
        value: &str,
    ) -> Result<(), NativeManagedNavigationError> {
        self.navigation_available()?;
        let result = self
            .navigation
            .as_mut()
            .ok_or(NativeManagedNavigationError::Unavailable)?
            .edit_form(editor, value);
        self.notify();
        result
    }

    /// Requests up to 64 raw heads, including nonresident/archived histories.
    /// Filtering can yield an empty page with a continuation; it never causes
    /// an unbounded scan. Drive this owner and then consume the exact outcome.
    /// # Errors
    /// Missing managed ownership, shutdown, pending result, invalid cursor/bound.
    pub fn request_managed_catalog(
        &mut self,
        filter: NativeManagedCatalogFilter,
        cursor: Option<NativeManagedCatalogCursor>,
        limit: usize,
    ) -> Result<NativeManagedCatalogRequest, NativeManagedCatalogError> {
        if self.shutting_down || self.closed {
            return Err(NativeManagedCatalogError::Closed);
        }
        if self
            .navigation
            .as_ref()
            .is_some_and(|navigation| navigation.owns_catalog())
        {
            return Err(NativeManagedCatalogError::Busy);
        }
        let request = self
            .managed
            .as_mut()
            .ok_or(NativeManagedCatalogError::Closed)?
            .agents
            .request_catalog(filter, cursor, limit)?;
        self.notify();
        Ok(request)
    }

    #[must_use]
    pub fn take_managed_catalog_outcome(&mut self) -> Option<NativeManagedCatalogOutcome> {
        if self
            .navigation
            .as_ref()
            .is_some_and(|navigation| navigation.owns_catalog())
        {
            return None;
        }
        self.managed.as_mut()?.agents.take_catalog_outcome()
    }

    /// Human admission against a previously observed exact target. This still
    /// captures the actual foreground's policy/workspace and lifecycle permit;
    /// the row supplies only a stale-target fence, never execution authority.
    /// # Errors
    /// Rejects foreign observations, target mismatch, unavailable foreground,
    /// cancellation and aggregate mailbox pressure. Later head changes return
    /// a rejected `StaleGeneration` result from the owned queued command.
    pub fn request_observed_managed_command(
        &mut self,
        observed: NativeObservedManagedAgent,
        command: ManagedSubagentCommand,
        cancellation: CancellationToken,
    ) -> Result<NativeManagedCommandResponse, ManagedSubagentError> {
        if self.shutting_down || self.closed || self.pending.is_some() || self.transition.is_some()
        {
            return Err(ManagedSubagentError::Unavailable);
        }
        let owner = self
            .managed
            .as_mut()
            .ok_or(ManagedSubagentError::Unavailable)?;
        let foreground = owner
            .foreground
            .as_ref()
            .ok_or(ManagedSubagentError::Unavailable)?;
        let response = owner.agents.request_observed_human_command(
            foreground,
            observed,
            command,
            cancellation,
        )?;
        self.notify();
        Ok(response)
    }
}

impl NativeInteractiveSession {
    /// Opens an independently routed management projection, retaining the parent
    /// runtime. A native catalog read is queued; opening never executes a model.
    /// # Errors
    /// Rejects missing ownership, transitions, shutdown or an unsettled old request.
    pub fn open_managed_navigation(&mut self) -> Result<(), NativeManagedNavigationError> {
        self.navigation_available()?;
        let mut navigation = self.navigation.take().unwrap_or_default();
        let result = navigation.open(self);
        self.navigation = Some(navigation);
        self.notify();
        result
    }

    pub fn close_managed_navigation(&mut self) {
        if let Some(navigation) = &mut self.navigation {
            navigation.close();
        }
        self.notify();
    }

    #[must_use]
    pub fn managed_navigation(&self) -> Option<NativeManagedNavigationView<'_>> {
        self.navigation.as_ref()?.view()
    }

    /// Called only after the renderer's flush for this exact frame succeeds.
    /// An old flush cannot acknowledge a replaced page/editor or reopen a view.
    /// # Errors
    /// Rejects closed, foreign and superseded frames.
    pub fn acknowledge_managed_frame(
        &mut self,
        frame: &NativeManagedFrameIdentity,
    ) -> Result<(), NativeManagedNavigationError> {
        self.navigation_available()?;
        self.navigation
            .as_mut()
            .ok_or(NativeManagedNavigationError::Unavailable)?
            .acknowledge(frame)
    }

    /// Revokes frame admission when resize/overlapping output changes presentation.
    /// # Errors
    /// Rejects missing navigation or exhausted frame identities.
    pub fn invalidate_managed_frame(&mut self) -> Result<(), NativeManagedNavigationError> {
        let result = self
            .navigation
            .as_mut()
            .ok_or(NativeManagedNavigationError::Unavailable)?
            .invalidate();
        self.notify();
        result
    }

    /// Admits typed human intent only from the exact currently acknowledged frame.
    /// A row observation is checked again by the manager after queueing.
    /// # Errors
    /// Rejects stale/unacknowledged frames, pending work, invalid actions or selection.
    pub fn act_on_managed_frame(
        &mut self,
        frame: &NativeManagedFrameIdentity,
        action: NativeManagedNavigationAction,
    ) -> Result<(), NativeManagedNavigationError> {
        self.act_managed(frame, action, None)
    }

    /// Submits an editor line as typed native intent. Matching retained text is
    /// consumed only after successful navigation or a confirmed command receipt.
    /// Keyboard navigation should use `act_on_managed_frame` and preserve drafts.
    /// # Errors
    /// Uses the same exact-frame, target and lifecycle checks as other actions.
    pub fn submit_managed_frame(
        &mut self,
        frame: &NativeManagedFrameIdentity,
        action: NativeManagedNavigationAction,
        text: &str,
    ) -> Result<(), NativeManagedNavigationError> {
        self.act_managed(frame, action, Some(text))
    }

    fn act_managed(
        &mut self,
        frame: &NativeManagedFrameIdentity,
        action: NativeManagedNavigationAction,
        submitted: Option<&str>,
    ) -> Result<(), NativeManagedNavigationError> {
        self.navigation_available()?;
        let mut navigation = self
            .navigation
            .take()
            .ok_or(NativeManagedNavigationError::Unavailable)?;
        let draft = submitted.and_then(|text| navigation.submitted_draft(text));
        let result = navigation.action(self, frame, action);
        if result.is_ok()
            && let Some(draft) = draft
        {
            navigation.retain_submission(draft);
        }
        self.navigation = Some(navigation);
        self.notify();
        result
    }

    fn navigation_available(&self) -> Result<(), NativeManagedNavigationError> {
        if self.managed.is_none()
            || self.shutting_down
            || self.closed
            || self.pending.is_some()
            || self.transition.is_some()
        {
            Err(NativeManagedNavigationError::Unavailable)
        } else {
            Ok(())
        }
    }

    pub(super) fn poll_navigation(&mut self, cx: &mut std::task::Context<'_>) {
        let Some(mut navigation) = self.navigation.take() else {
            return;
        };
        if self.navigation_available().is_err() {
            navigation.close();
        }
        navigation.poll(self, cx);
        self.navigation = Some(navigation);
    }
}
