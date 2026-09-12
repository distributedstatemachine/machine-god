use super::{Kind, NativeMcpControllerError, Result};
use crate::mcp::controller::state::{Generation, Inner, failure, lock};
use std::sync::Arc;

#[cfg(test)]
mod tests;

pub(super) struct Observation {
    pub source: Arc<Generation>,
    pub due: bool,
}

/// Observe injected clocks/profile state outside controller and loaded locks.
/// The selector must subsequently compare the exact source allocation again.
pub(super) fn observe(inner: &Inner) -> Result<Observation> {
    let source = {
        let state = lock(&inner.state);
        if state.closed {
            return Err(failure(NativeMcpControllerError::Closed));
        }
        let source = state
            .active
            .clone()
            .ok_or_else(|| failure(NativeMcpControllerError::Unavailable))?;
        if state.running.as_ref().is_some_and(|running| {
            running.kind == Kind::Refresh
                && running
                    .refresh_source
                    .as_ref()
                    .is_some_and(|original| Arc::ptr_eq(original, &source))
        }) {
            // The in-flight refresh may already have committed new credentials
            // and revoked the old lease. Join it rather than inspecting that
            // retired lease or starting a second token exchange.
            return Ok(Observation { source, due: true });
        }
        source
    };
    if source.cancellation.is_cancelled() {
        return Err(failure(NativeMcpControllerError::Unavailable));
    }
    #[cfg(feature = "mcp-http")]
    let due = {
        let startup = lock(&source.loaded)
            .as_ref()
            .ok_or_else(|| failure(NativeMcpControllerError::Unavailable))?
            .startup
            .clone();
        startup
            .authentication_refresh_due()
            .map_err(|error| failure(NativeMcpControllerError::Startup(error)))?
    };
    #[cfg(not(feature = "mcp-http"))]
    let due = false;
    Ok(Observation { source, due })
}

pub(super) fn phase(source: &Generation) -> crate::mcp::startup::NativeMcpStartupPhase {
    use crate::mcp::startup::NativeMcpStartupPhase;
    if lock(&source.deferred)
        .as_ref()
        .is_some_and(std::result::Result::is_ok)
    {
        NativeMcpStartupPhase::All
    } else {
        source.phase
    }
}
