//! One host-owned read, serialized with the manager's journal operations.
use super::{Active, BoxFuture, ManagedManager, ManagedRuntimeError};

impl ManagedManager {
    /// Trusted native composition only. The caller owns cancellation/result
    /// correlation; the manager owns polling through actual worker settlement.
    /// An observation must not await another manager command or a UI response.
    pub(crate) fn request_observation(
        &mut self,
        future: BoxFuture<'static, ()>,
    ) -> Result<(), ManagedRuntimeError> {
        if self.closing
            || self.observation.is_some()
            || matches!(self.active, Some(Active::Observation(_)))
        {
            return Err(ManagedRuntimeError::Capacity);
        }
        self.observation = Some(future);
        Ok(())
    }

    pub(super) fn begin_observation(&mut self) -> bool {
        let Some(future) = self.observation.take() else {
            return false;
        };
        // Share the catalog fairness allowance. Repeated UI reads cannot take
        // separate priority turns ahead of accepted durable work.
        self.catalog.mark_read();
        self.active = Some(Active::Observation(future));
        true
    }
}
