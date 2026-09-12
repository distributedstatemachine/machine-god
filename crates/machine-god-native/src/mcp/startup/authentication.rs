use super::{NativeMcpStartup, NativeMcpStartupError as Error, Result};
use crate::mcp::{
    auth::{McpAuthConfig, McpAuthError, McpAuthLease, NativeMcpAuthProfile, NativeMcpAuthService},
    config::{MAX_SERVERS, McpConfig, McpRemoteConfig, McpTransportConfig},
    headers::McpResolvedHeaders,
};
use machine_god_core::CancellationToken;
use std::{fmt, os::unix::ffi::OsStrExt, sync::Arc, time::Instant};

mod challenges;
pub(super) use challenges::Challenges;
pub use challenges::NativeMcpStartupAuthChallenge;

/// Explicit endpoint authentication choice. None of these variants can open a
/// browser. Stored credentials may refresh through the selected native service.
#[derive(Clone)]
pub enum NativeMcpStartupAuthSource {
    Configured,
    Lease(Arc<McpAuthLease>),
    Stored(Arc<NativeMcpAuthService>),
    /// Native controller selection sharing the exact loaded profile and lifetime.
    ProfileStored {
        service: Arc<NativeMcpAuthService>,
        profile: Arc<NativeMcpAuthProfile>,
    },
}
#[derive(Clone)]
pub struct NativeMcpStartupAuthentication {
    pub server: Box<str>,
    pub additional_headers: McpResolvedHeaders,
    pub source: NativeMcpStartupAuthSource,
}

pub(super) fn validate(
    configuration: &McpConfig,
    selections: &[NativeMcpStartupAuthentication],
) -> Result<()> {
    if selections.len() > MAX_SERVERS {
        return Err(Error::Limit);
    }
    for (index, selected) in selections.iter().enumerate() {
        if selections[..index]
            .iter()
            .any(|prior| prior.server == selected.server)
            || configuration
                .server(&selected.server)
                .is_none_or(|server| matches!(server.transport(), McpTransportConfig::Stdio(_)))
        {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}

impl NativeMcpStartup {
    fn lookup(&self, name: &str) -> Option<&[u8]> {
        self.environment
            .entries()
            .iter()
            .find(|(key, _)| key.as_bytes() == name.as_bytes())
            .map(|(_, value)| value.as_bytes())
    }
    fn authentication_selection(&self, name: &str) -> Option<&NativeMcpStartupAuthentication> {
        self.authentication
            .iter()
            .find(|selection| selection.server.as_ref() == name)
    }

    /// Stable exact selected OAuth identity shared by explicit auth commands and
    /// startup. Uses a fixed empty OAuth marker instead of a changing access token
    /// or dormant bearer environment value. It performs no credential load.
    /// # Errors
    /// Rejects non-remote names, missing selected header/client-secret environment
    /// or invalid/ambiguous endpoint header authentication configuration.
    pub fn authentication_config(&self, server: &str) -> Result<McpAuthConfig> {
        let configuration = self.configuration.server(server).ok_or(Error::Invalid)?;
        let remote = match configuration.transport() {
            McpTransportConfig::Http(remote) | McpTransportConfig::Sse(remote) => remote,
            McpTransportConfig::Stdio(_) => return Err(Error::Invalid),
        };
        self.auth_config(remote, self.authentication_selection(server))
    }

    fn auth_config(
        &self,
        remote: &McpRemoteConfig,
        selection: Option<&NativeMcpStartupAuthentication>,
    ) -> Result<McpAuthConfig> {
        let additional: Vec<_> = selection
            .into_iter()
            .flat_map(|selection| selection.additional_headers.iter())
            .collect();
        let identity =
            McpResolvedHeaders::resolve(remote, |name| self.lookup(name), Some(b""), &additional)
                .map_err(|_| Error::Authentication)?
                .authentication_identity_bytes();
        McpAuthConfig::new(remote, &identity, |name| self.lookup(name))
            .map_err(|_| Error::Authentication)
    }

    pub(super) async fn headers(
        &self,
        name: &str,
        remote: &McpRemoteConfig,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<(McpResolvedHeaders, Option<CancellationToken>)> {
        let selection = self.authentication_selection(name);
        let additional: Vec<_> = selection
            .into_iter()
            .flat_map(|selection| selection.additional_headers.iter())
            .collect();
        let lease = match selection.map(|selection| &selection.source) {
            None | Some(NativeMcpStartupAuthSource::Configured) => None,
            Some(NativeMcpStartupAuthSource::Lease(lease)) => {
                let expected = self.auth_config(remote, selection)?;
                if lease.identity().endpoint() != remote.url()
                    || lease.identity() != expected.identity()
                {
                    return Err(Error::Authentication);
                }
                Some(lease.clone())
            }
            Some(NativeMcpStartupAuthSource::Stored(service)) => {
                let expected = self.auth_config(remote, selection)?;
                match service
                    .access_token(expected.identity(), cancellation, deadline)
                    .await
                {
                    Ok(lease) => Some(Arc::new(lease)),
                    Err(McpAuthError::Missing) => None,
                    Err(_) => return Err(Error::Authentication),
                }
            }
            Some(NativeMcpStartupAuthSource::ProfileStored { service, profile }) => {
                let expected = self.auth_config(remote, selection)?;
                match service
                    .access_token_for_profile(
                        expected.identity(),
                        profile.clone(),
                        cancellation,
                        deadline,
                    )
                    .await
                {
                    Ok(lease) => Some(Arc::new(lease)),
                    Err(McpAuthError::Missing) => None,
                    Err(_) => return Err(Error::Authentication),
                }
            }
        };
        let token = lease
            .as_ref()
            .map(|lease| lease.access_token().map_err(|_| Error::Authentication))
            .transpose()?;
        let headers =
            McpResolvedHeaders::resolve(remote, |name| self.lookup(name), token, &additional)
                .map_err(|_| Error::Authentication)?;
        Ok((headers, lease.as_ref().map(|lease| lease.generation())))
    }
}

macro_rules! redacted { ($($ty:ty),+ $(,)?) => {$(impl fmt::Debug for $ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(concat!(stringify!($ty), " { <redacted> }")) }
})+}; }
redacted!(NativeMcpStartupAuthSource, NativeMcpStartupAuthentication);
