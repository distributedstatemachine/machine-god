use super::{
    NativeMcpPublicationCheckpoint, NativeMcpRuntime, NativeMcpRuntimeError as Error, Result,
};
use crate::mcp::config::McpConfig;

impl NativeMcpRuntime {
    pub(crate) fn required_readiness(
        &self,
        checkpoint: &NativeMcpPublicationCheckpoint,
        configuration: &McpConfig,
    ) -> Result<()> {
        let publication = {
            let state = self.state.lock().map_err(|_| Error::Unavailable)?;
            if state.closed {
                return Err(Error::Unavailable);
            }
            checkpoint.check(self, &state)?;
            state.active.clone().ok_or(Error::Unavailable)?
        };
        publication.check()?;
        for required in configuration
            .servers()
            .iter()
            .filter(|server| server.required())
        {
            if !required.enabled() {
                return Err(Error::Unavailable);
            }
            let route = publication
                .servers
                .iter()
                .find(|route| route.name.as_ref() == required.name())
                .ok_or(Error::Unavailable)?;
            route.check_authority()?;
            if !route.readiness.is_ready() {
                return Err(Error::Unavailable);
            }
            route.check_authority()?;
        }
        // Selected clocks and peer observations run outside publication locks.
        // A concurrent replacement/close must not bless the old observed view.
        let state = self.state.lock().map_err(|_| Error::Unavailable)?;
        if state.closed {
            return Err(Error::Unavailable);
        }
        checkpoint.check(self, &state)?;
        publication.check()
    }
}
