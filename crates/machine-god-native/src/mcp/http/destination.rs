use super::{McpEndpoint, McpHttpError, Result};
use rustls::{ClientConfig, RootCertStore};
use std::{collections::BTreeSet, fmt, net::SocketAddr, sync::Arc};

/// Explicit endpoint-bound DNS result/address authority, never ambient resolution.
pub struct McpHttpDestination {
    endpoint: McpEndpoint,
    pub(super) addresses: Box<[SocketAddr]>,
}
impl fmt::Debug for McpHttpDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpHttpDestination { <redacted> }")
    }
}
impl McpHttpDestination {
    /// The trusted resolver/host selects these addresses for this exact endpoint.
    /// Private HTTPS addresses are permitted; plaintext addresses must be loopback.
    /// No DNS request or connection occurs here. Addresses are attempted once each
    /// before any HTTP write, never as application-request replay.
    ///
    /// # Errors
    /// Requires 1–32 unique, non-unspecified/non-multicast addresses, matching ports,
    /// literal-IP agreement and loopback confinement for plaintext/localhost.
    pub fn new(endpoint: McpEndpoint, addresses: &[SocketAddr]) -> Result<Self> {
        if addresses.is_empty() || addresses.len() > 32 {
            return Err(McpHttpError::Limit);
        }
        let mut unique = BTreeSet::new();
        for address in addresses {
            let ip = address.ip();
            let host_matches = match endpoint.host() {
                url::Host::Ipv4(host) => ip == std::net::IpAddr::V4(host),
                url::Host::Ipv6(host) => ip == std::net::IpAddr::V6(host),
                url::Host::Domain("localhost") => ip.is_loopback(),
                url::Host::Domain(_) => true,
            };
            if address.port() != endpoint.port()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (!endpoint.is_tls() && !ip.is_loopback())
                || !host_matches
                || !unique.insert(*address)
            {
                return Err(McpHttpError::Invalid);
            }
        }
        Ok(Self {
            endpoint,
            addresses: addresses.into(),
        })
    }
    #[must_use]
    pub fn endpoint(&self) -> &McpEndpoint {
        &self.endpoint
    }
}

/// Certificate-verifying TLS, built from explicit trust anchors, not custom verifiers.
#[derive(Clone)]
pub struct McpHttpTrust(pub(super) Arc<ClientConfig>);
impl fmt::Debug for McpHttpTrust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpHttpTrust { <redacted> }")
    }
}
impl McpHttpTrust {
    /// Keeps SNI and hostname verification, offers only HTTP/1.1, and disables
    /// early data, client credentials, resumption and key logging.
    ///
    /// # Errors
    /// Requires 1–512 trust anchors and at most 4 MiB of anchor bytes.
    pub fn new(roots: RootCertStore) -> Result<Self> {
        if roots.is_empty() || roots.len() > 512 {
            return Err(McpHttpError::Limit);
        }
        let mut size = 0usize;
        for root in &roots.roots {
            size = size
                .checked_add(
                    root.subject.len()
                        + root.subject_public_key_info.len()
                        + root
                            .name_constraints
                            .as_ref()
                            .map_or(0, |value| value.len()),
                )
                .ok_or(McpHttpError::Limit)?;
            if size > 4 * 1024 * 1024 {
                return Err(McpHttpError::Limit);
            }
        }
        let mut config = ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| McpHttpError::Tls)?
        .with_root_certificates(roots)
        .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        config.enable_early_data = false;
        config.resumption = rustls::client::Resumption::disabled();
        Ok(Self(Arc::new(config)))
    }
}
