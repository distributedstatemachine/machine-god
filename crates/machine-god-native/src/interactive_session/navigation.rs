//! Native managed observation/admission. UI pages never replace the parent runtime.
use super::NativeInteractiveSession;
use crate::{
    NativeManagedCatalogCursor, NativeManagedCatalogError, NativeManagedCatalogFilter,
    NativeManagedCatalogOutcome, NativeManagedCatalogRequest, NativeManagedCommandResponse,
    NativeObservedManagedAgent,
};
use machine_god_core::{CancellationToken, ManagedSubagentCommand, ManagedSubagentError};

impl NativeInteractiveSession {
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
