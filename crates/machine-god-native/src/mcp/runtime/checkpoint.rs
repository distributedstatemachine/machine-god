use super::{
    NativeMcpRuntime, NativeMcpRuntimeError as Error, Result, State, candidate::Publication,
};
use std::{
    fmt,
    sync::{Arc, Weak},
};

/// Observation of one exact runtime publication, including no publication.
/// Retains no peers or runtime owner and grants no execution authority.
#[derive(Clone)]
pub struct NativeMcpPublicationCheckpoint {
    runtime: Weak<()>,
    publication: Option<Weak<Publication>>,
}
impl fmt::Debug for NativeMcpPublicationCheckpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpPublicationCheckpoint { <redacted> }")
    }
}
impl NativeMcpPublicationCheckpoint {
    pub(super) fn check(&self, runtime: &NativeMcpRuntime, state: &State) -> Result<()> {
        if !self.runtime.ptr_eq(&Arc::downgrade(&runtime.identity)) {
            return Err(Error::Invalid);
        }
        let unchanged = match (&self.publication, &state.active) {
            (None, None) => true,
            (Some(expected), Some(active)) => expected.ptr_eq(&Arc::downgrade(active)),
            _ => false,
        };
        if unchanged {
            Ok(())
        } else {
            Err(Error::Unavailable)
        }
    }
}
impl NativeMcpRuntime {
    /// Captures an exact publication for conditional startup/reload. Does not
    /// pin a turn, query a peer, observe time or keep retired peers alive.
    /// # Errors
    /// Rejects a closed or unavailable runtime owner.
    pub fn publication_checkpoint(&self) -> Result<NativeMcpPublicationCheckpoint> {
        let state = self.state.lock().map_err(|_| Error::Unavailable)?;
        if state.closed {
            return Err(Error::Unavailable);
        }
        Ok(NativeMcpPublicationCheckpoint {
            runtime: Arc::downgrade(&self.identity),
            publication: state.active.as_ref().map(Arc::downgrade),
        })
    }
}
