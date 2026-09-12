//! Per-observation stored-auth selection, without loading any credentials.

use super::NativeMcpControllerOptions;
use crate::mcp::{
    auth::NativeMcpAuthProfile,
    config::{MAX_SERVERS, McpTransportConfig},
    headers::McpResolvedHeaders,
    startup::{NativeMcpStartupAuthSource, NativeMcpStartupAuthentication, NativeMcpStartupError},
    store::NativeMcpConfigSnapshot,
};
use machine_god_core::CancellationToken;
use std::sync::Arc;

pub(super) fn selections(
    options: &NativeMcpControllerOptions,
    snapshot: &Arc<NativeMcpConfigSnapshot>,
    cancellation: CancellationToken,
) -> Result<Vec<NativeMcpStartupAuthentication>, NativeMcpStartupError> {
    if options.startup.authentication.len() > MAX_SERVERS {
        return Err(NativeMcpStartupError::Limit);
    }
    let mut selected = options.startup.authentication.clone();
    let Some(service) = &options.stored_authentication else {
        return Ok(selected);
    };
    let profile = Arc::new(NativeMcpAuthProfile::new(
        options.management.config_store(),
        snapshot.clone(),
        options.startup.owner_cancellation.clone(),
        cancellation,
    ));
    for server in snapshot.config().servers() {
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
            source: NativeMcpStartupAuthSource::ProfileStored {
                service: service.clone(),
                profile: profile.clone(),
            },
        });
    }
    Ok(selected)
}
