//! Per-observation stored-auth selection, without loading any credentials.

use super::NativeMcpControllerOptions;
use crate::mcp::{
    config::{MAX_SERVERS, McpConfig, McpTransportConfig},
    headers::McpResolvedHeaders,
    startup::{NativeMcpStartupAuthSource, NativeMcpStartupAuthentication, NativeMcpStartupError},
};

pub(super) fn selections(
    options: &NativeMcpControllerOptions,
    configuration: &McpConfig,
) -> Result<Vec<NativeMcpStartupAuthentication>, NativeMcpStartupError> {
    if options.startup.authentication.len() > MAX_SERVERS {
        return Err(NativeMcpStartupError::Limit);
    }
    let mut selected = options.startup.authentication.clone();
    let Some(service) = &options.stored_authentication else {
        return Ok(selected);
    };
    for server in configuration.servers() {
        if matches!(server.transport(), McpTransportConfig::Stdio(_))
            || selected
                .iter()
                .any(|value| value.server.as_ref() == server.name())
        {
            continue;
        }
        if selected.len() == MAX_SERVERS {
            return Err(NativeMcpStartupError::Limit);
        }
        selected.push(NativeMcpStartupAuthentication {
            server: server.name().into(),
            additional_headers: McpResolvedHeaders::from_resolved(&[])
                .map_err(|_| NativeMcpStartupError::Authentication)?,
            source: NativeMcpStartupAuthSource::Stored(service.clone()),
        });
    }
    Ok(selected)
}
