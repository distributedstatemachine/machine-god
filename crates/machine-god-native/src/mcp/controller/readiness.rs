use super::{
    NativeMcpController, NativeMcpControllerError, Result,
    state::{failure, lock},
};

impl NativeMcpController {
    /// Observes required peers in the exact active publication, without I/O or
    /// discovery. Failed reload never substitutes its candidate for that view.
    /// This is admission-time readiness, not a promise of future remote health.
    /// # Errors
    /// Rejects absent, closed, replaced, expired or unavailable required peers.
    pub fn required_readiness(&self) -> Result<()> {
        let generation = {
            let state = lock(&self.inner.state);
            if state.closed || self.inner.options.startup.owner_cancellation.is_cancelled() {
                return Err(failure(NativeMcpControllerError::Closed));
            }
            state
                .active
                .clone()
                .ok_or_else(|| failure(NativeMcpControllerError::Unavailable))?
        };
        if generation.cancellation.is_cancelled() {
            return Err(failure(NativeMcpControllerError::Unavailable));
        }
        let (snapshot, checkpoint) = {
            let loaded = lock(&generation.loaded);
            let loaded = loaded
                .as_ref()
                .ok_or_else(|| failure(NativeMcpControllerError::Unavailable))?;
            (
                loaded.snapshot.clone(),
                loaded
                    .checkpoint
                    .clone()
                    .ok_or_else(|| failure(NativeMcpControllerError::Unavailable))?,
            )
        };
        self.inner
            .options
            .runtime
            .required_readiness(&checkpoint, snapshot.config())
            .map_err(|error| failure(NativeMcpControllerError::Runtime(error)))?;
        if generation.cancellation.is_cancelled()
            || self.inner.options.startup.owner_cancellation.is_cancelled()
        {
            return Err(failure(NativeMcpControllerError::Unavailable));
        }
        Ok(())
    }
}
