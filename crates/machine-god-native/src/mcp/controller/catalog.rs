//! Short same-generation catalog checkpoint handoff, without callback locks.

use super::{
    NativeMcpController,
    state::{Generation, Inner, lock},
};
use crate::mcp::runtime::{NativeMcpPublicationCheckpoint, NativeMcpRuntimeError as Error};
use std::sync::{Arc, atomic::Ordering};

struct Handoff<'a> {
    inner: &'a Inner,
    generation: Arc<Generation>,
}
impl Drop for Handoff<'_> {
    fn drop(&mut self) {
        lock(&self.inner.state).catalog_handoff = false;
    }
}
impl NativeMcpController {
    /// The private closure must perform the exact conditional runtime commit.
    /// Readiness observes Busy during the handoff; close is always permitted.
    pub(crate) fn sync_catalog_publication(
        &self,
        expected: &NativeMcpPublicationCheckpoint,
        prospective: &NativeMcpPublicationCheckpoint,
        commit: impl FnOnce() -> Result<NativeMcpPublicationCheckpoint, Error>,
    ) -> Result<NativeMcpPublicationCheckpoint, Error> {
        if !expected.same_runtime(prospective) {
            return Err(Error::Invalid);
        }
        self.inner.release_completed();
        let handoff = {
            let mut state = lock(&self.inner.state);
            if state.closed || self.inner.options.startup.owner_cancellation.is_cancelled() {
                return Err(Error::Unavailable);
            }
            if state.catalog_handoff
                || state.running.is_some()
                || self.inner.settling.load(Ordering::Acquire)
            {
                return Err(Error::Unavailable);
            }
            #[cfg(all(feature = "mcp-http", any(test, feature = "ai-gateway-http")))]
            if state.authenticating.strong_count() != 0 {
                return Err(Error::Unavailable);
            }
            let generation = state.active.as_ref().ok_or(Error::Unavailable)?;
            if generation.cancellation.is_cancelled() {
                return Err(Error::Unavailable);
            }
            let loaded = lock(&generation.loaded);
            if !loaded
                .as_ref()
                .and_then(|loaded| loaded.checkpoint.as_ref())
                .is_some_and(|checkpoint| checkpoint.same_selection(expected))
            {
                return Err(Error::Unavailable);
            }
            drop(loaded);
            let generation = generation.clone();
            state.catalog_handoff = true;
            Handoff {
                inner: &self.inner,
                generation,
            }
        };
        // No owner lock while observing injected clocks, committing transport
        // tables, or completing deferred cancellation callbacks.
        let committed = commit()?;
        {
            let mut loaded = lock(&handoff.generation.loaded);
            // Handoff excludes every loaded-state mutation; close only retires
            // owners and never removes this exact generation's loaded record.
            if let Some(loaded) = loaded.as_mut() {
                loaded.checkpoint = Some(committed.clone());
            }
        }
        // Even if a reentrant close occurred after publication, retain the exact
        // successful checkpoint without resurrecting controller active state.
        drop(handoff);
        Ok(committed)
    }
}

#[cfg(test)]
mod tests;
