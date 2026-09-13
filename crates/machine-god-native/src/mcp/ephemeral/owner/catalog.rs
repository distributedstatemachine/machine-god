//! Same-generation catalog handoff. No owner lock spans a runtime callback.
use super::{MutationState, State, lock};
use crate::mcp::runtime::{
    NativeMcpPublicationCheckpoint, NativeMcpRuntime, NativeMcpRuntimeError as Error,
};
use machine_god_core::CancellationToken;
use std::sync::{Arc, Mutex};
#[cfg(test)]
mod tests;

pub(crate) struct NativeMcpEphemeralCatalog {
    state: Arc<Mutex<State>>,
    owner: CancellationToken,
}
impl NativeMcpEphemeralCatalog {
    pub(super) fn new(state: Arc<Mutex<State>>, owner: CancellationToken) -> Self {
        Self { state, owner }
    }

    pub(crate) fn required_readiness(&self, runtime: &NativeMcpRuntime) -> Result<(), Error> {
        let (generation, checkpoint) = {
            let state = lock(&self.state);
            if state.closed || state.mutation == MutationState::Catalog || self.owner.is_cancelled()
            {
                return Err(Error::Unavailable);
            }
            let active = state.active.as_ref().ok_or(Error::Unavailable)?;
            (active.generation.clone(), active.checkpoint.clone())
        };
        if generation.cancellation.is_cancelled() {
            return Err(Error::Unavailable);
        }
        runtime.required_readiness(&checkpoint, &generation.configuration.configuration)?;
        if generation.cancellation.is_cancelled() || self.owner.is_cancelled() {
            return Err(Error::Unavailable);
        }
        Ok(())
    }

    pub(crate) fn sync_catalog_publication(
        &self,
        expected: &NativeMcpPublicationCheckpoint,
        prospective: &NativeMcpPublicationCheckpoint,
        commit: impl FnOnce() -> Result<NativeMcpPublicationCheckpoint, Error>,
    ) -> Result<NativeMcpPublicationCheckpoint, Error> {
        if !expected.same_runtime(prospective) {
            return Err(Error::Invalid);
        }
        let generation = {
            let mut state = lock(&self.state);
            if state.closed
                || state.mutation != MutationState::Idle
                || state.settling
                || self.owner.is_cancelled()
            {
                return Err(Error::Unavailable);
            }
            let active = state.active.as_ref().ok_or(Error::Unavailable)?;
            if active.generation.cancellation.is_cancelled()
                || !active.checkpoint.same_selection(expected)
            {
                return Err(Error::Unavailable);
            }
            let generation = active.generation.clone();
            state.mutation = MutationState::Catalog;
            generation
        };
        let _handoff = Handoff(self);
        let committed = commit()?;
        let mut state = lock(&self.state);
        if let Some(active) = &mut state.active
            && Arc::ptr_eq(&active.generation, &generation)
        {
            active.checkpoint = committed.clone();
        }
        Ok(committed)
    }
}
struct Handoff<'a>(&'a NativeMcpEphemeralCatalog);
impl Drop for Handoff<'_> {
    fn drop(&mut self) {
        lock(&self.0.state).mutation = MutationState::Idle;
    }
}
