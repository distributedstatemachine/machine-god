//! Canonical child transcript observations, with custody in the outer manager.
use super::NativeInteractiveSession;
use crate::{
    NativeManagedHistoryError, NativeManagedHistoryOutcome, NativeManagedHistoryRequest,
    NativeObservedManagedAgent,
};

impl NativeInteractiveSession {
    /// Queues the exact child's canonical transcript without changing foreground
    /// selection or admitting child execution. Poll this owner through completion.
    /// # Errors
    /// Rejects transitions, shutdown, foreign/stale observations and history pressure.
    pub fn request_managed_history(
        &mut self,
        observed: NativeObservedManagedAgent,
    ) -> Result<NativeManagedHistoryRequest, NativeManagedHistoryError> {
        self.navigation_available()
            .map_err(|_| NativeManagedHistoryError::Closed)?;
        let request = self
            .managed
            .as_mut()
            .ok_or(NativeManagedHistoryError::Closed)?
            .agents
            .request_history(observed)?;
        self.notify();
        Ok(request)
    }

    /// Cancel only this read; actual worker settlement remains owned.
    pub fn cancel_managed_history(&mut self, request: &NativeManagedHistoryRequest) -> bool {
        let cancelled = self
            .managed
            .as_mut()
            .is_some_and(|owner| owner.agents.cancel_history(request));
        self.notify();
        cancelled
    }

    #[must_use]
    pub fn take_managed_history_outcome(&mut self) -> Option<NativeManagedHistoryOutcome> {
        self.managed.as_mut()?.agents.take_history_outcome()
    }
}
