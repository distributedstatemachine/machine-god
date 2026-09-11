//! Explicit endpoint-specific DNS and TLS authority for native MCP.

use super::auth::{McpAuthDestination, McpAuthError, McpAuthNetwork};
use super::endpoint::McpEndpoint;
use super::http::{McpHttpClock, McpHttpDestination, McpHttpTrust};
use crate::bounded_dns::{
    QueryIdSequence, SystemNameServer, SystemResolverSnapshot, load_system_resolver_snapshot,
    validate_system_resolver_snapshot,
};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

mod lookup;
#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, McpNetworkError>;

/// Fixed diagnostics never include resolver, host, endpoint or trust material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpNetworkError {
    Invalid,
    Unavailable,
    Limit,
    Cancelled,
    Deadline,
}
impl fmt::Display for McpNetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid MCP network authority",
            Self::Unavailable => "MCP destination resolution unavailable",
            Self::Limit => "MCP network limit exceeded",
            Self::Cancelled => "MCP destination resolution cancelled",
            Self::Deadline => "MCP destination resolution deadline exceeded",
        })
    }
}
impl std::error::Error for McpNetworkError {}

/// Explicit DNS transports for one nameserver. Neither field performs lookup.
#[derive(Clone, Copy)]
pub struct McpNameServer {
    pub udp: Option<SocketAddr>,
    pub tcp: Option<SocketAddr>,
    pub trust_negative_responses: bool,
}
impl fmt::Debug for McpNameServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpNameServer { <redacted> }")
    }
}

/// Captured resolver selection; no search suffix, hosts-file or ambient fallback.
pub struct McpResolverConfig(SystemResolverSnapshot);
impl fmt::Debug for McpResolverConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpResolverConfig { <redacted> }")
    }
}
impl McpResolverConfig {
    /// Explicitly permits only literal IPs and exact localhost; no DNS fallback.
    #[must_use]
    pub fn literal_only() -> Self {
        Self(SystemResolverSnapshot {
            name_servers: Vec::new(),
            timeout: Duration::from_secs(1),
            attempts: 1,
            concurrent_requests: 1,
            try_tcp_on_error: false,
            recursion_desired: true,
        })
    }

    /// Constructs inert DNS authority with explicit retry and concurrent-server bounds.
    /// # Errors
    /// Accepts 1–32 servers, 1–5 attempts, 1–32 concurrent servers and at most
    /// 30 seconds per query. Addresses require a nonzero port and unicast IP.
    pub fn new(
        servers: &[McpNameServer],
        timeout: Duration,
        attempts: usize,
        concurrent_servers: usize,
    ) -> Result<Self> {
        if servers.is_empty()
            || servers.len() > 32
            || timeout.is_zero()
            || timeout > Duration::from_secs(30)
            || !(1..=5).contains(&attempts)
            || !(1..=32).contains(&concurrent_servers)
        {
            return Err(McpNetworkError::Limit);
        }
        for server in servers {
            if server.udp.is_none() && server.tcp.is_none() {
                return Err(McpNetworkError::Invalid);
            }
            for address in [server.udp, server.tcp].into_iter().flatten() {
                if address.port() == 0
                    || address.ip().is_unspecified()
                    || address.ip().is_multicast()
                {
                    return Err(McpNetworkError::Invalid);
                }
            }
        }
        Ok(Self(SystemResolverSnapshot {
            name_servers: servers
                .iter()
                .map(|server| SystemNameServer {
                    udp: server.udp,
                    tcp: server.tcp,
                    trust_negative_responses: server.trust_negative_responses,
                })
                .collect(),
            timeout,
            attempts,
            concurrent_requests: concurrent_servers,
            try_tcp_on_error: false,
            recursion_desired: true,
        }))
    }

    /// Explicit synchronous system-configuration capture for an owned startup worker.
    /// This is the only system resolver read; construction and requests never recapture.
    /// The caller owns worker completion/cancellation. This method starts no worker.
    /// # Errors
    /// Fails closed on absent, unsupported or over-budget system configuration.
    pub fn capture_system() -> Result<Self> {
        let parsed = load_system_resolver_snapshot().map_err(|_| McpNetworkError::Unavailable)?;
        let snapshot =
            validate_system_resolver_snapshot(&parsed).map_err(|_| McpNetworkError::Unavailable)?;
        for server in &snapshot.name_servers {
            for address in [server.udp, server.tcp].into_iter().flatten() {
                if address.port() == 0
                    || address.ip().is_unspecified()
                    || address.ip().is_multicast()
                {
                    return Err(McpNetworkError::Invalid);
                }
            }
        }
        Ok(Self(snapshot))
    }
}

/// Bounded DNS admission shared by HTTP runtime startup and OAuth URL selection.
/// No credential/header state, DNS cache, detached task, or process authority.
pub struct NativeMcpNetwork {
    resolver: McpResolverConfig,
    query_ids: QueryIdSequence,
    trust: Option<McpHttpTrust>,
    clock: Arc<dyn McpHttpClock>,
    owner: CancellationToken,
    permits: Semaphore,
}
impl fmt::Debug for NativeMcpNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpNetwork { <redacted> }")
    }
}
impl NativeMcpNetwork {
    /// Retains explicit authority only. The key must come from host-selected secure
    /// entropy; deterministic keys are appropriate only in fixtures.
    /// # Errors
    /// Requires 1–32 simultaneous admissions. TLS endpoints require selected trust.
    pub fn new(
        resolver: McpResolverConfig,
        query_id_key: [u8; 32],
        trust: Option<McpHttpTrust>,
        clock: Arc<dyn McpHttpClock>,
        owner: CancellationToken,
        max_active: usize,
    ) -> Result<Self> {
        if !(1..=32).contains(&max_active) {
            return Err(McpNetworkError::Limit);
        }
        Ok(Self {
            resolver,
            query_ids: QueryIdSequence::new(query_id_key),
            trust,
            clock,
            owner,
            permits: Semaphore::new(max_active),
        })
    }

    /// The host must retain this cancellation in the subsequent peer/exchange owner;
    /// a resolved address is data, not a self-revoking connection capability.
    #[must_use]
    pub fn owner_cancellation(&self) -> CancellationToken {
        self.owner.clone()
    }

    /// Resolves exactly this endpoint, returning its trust without endpoint headers.
    /// Literal IPs and exact `localhost` use no resolver or hosts file. All other
    /// names use one absolute FQDN, including names with an existing trailing dot.
    /// # Errors
    /// Rejects invalid addresses, missing TLS trust, cancellation and exhausted bounds.
    pub async fn admit_endpoint(
        &self,
        endpoint: &McpEndpoint,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<McpAuthDestination> {
        self.check(cancellation, deadline)?;
        if endpoint.is_tls() && self.trust.is_none() {
            return Err(McpNetworkError::Unavailable);
        }
        let deadline = deadline.min(
            self.clock
                .now()
                .checked_add(Duration::from_secs(30))
                .ok_or(McpNetworkError::Limit)?,
        );
        self.run(cancellation, deadline, async {
            let _permit = self
                .permits
                .acquire()
                .await
                .map_err(|_| McpNetworkError::Unavailable)?;
            self.check(cancellation, deadline)?;
            let addresses = match endpoint.host() {
                url::Host::Ipv4(ip) => vec![IpAddr::V4(ip)],
                url::Host::Ipv6(ip) => vec![IpAddr::V6(ip)],
                url::Host::Domain("localhost") => vec![
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    IpAddr::V6(Ipv6Addr::LOCALHOST),
                ],
                url::Host::Domain(host) => self.lookup(host, cancellation, deadline).await?,
            };
            self.check(cancellation, deadline)?;
            let sockets: Vec<_> = addresses
                .into_iter()
                .map(|ip| SocketAddr::new(ip, endpoint.port()))
                .collect();
            let destination = McpHttpDestination::new(endpoint.clone(), &sockets)
                .map_err(|_| McpNetworkError::Invalid)?;
            Ok(McpAuthDestination {
                destination,
                trust: endpoint.is_tls().then(|| self.trust.clone()).flatten(),
            })
        })
        .await
    }

    fn check(&self, cancellation: &CancellationToken, deadline: Instant) -> Result<()> {
        if self.owner.is_cancelled() || cancellation.is_cancelled() {
            Err(McpNetworkError::Cancelled)
        } else if self.clock.now() >= deadline {
            Err(McpNetworkError::Deadline)
        } else {
            Ok(())
        }
    }

    async fn run<T>(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
        future: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        use std::{
            future::{Future, poll_fn},
            task::Poll,
        };
        self.check(cancellation, deadline)?;
        let mut owner = std::pin::pin!(self.owner.cancelled());
        let mut cancel = std::pin::pin!(cancellation.cancelled());
        let mut timer = self.clock.sleep_until(deadline);
        let mut future = std::pin::pin!(future);
        poll_fn(|cx| {
            if owner.as_mut().poll(cx).is_ready() || cancel.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(McpNetworkError::Cancelled));
            }
            if timer.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(McpNetworkError::Deadline));
            }
            match future.as_mut().poll(cx) {
                Poll::Ready(result) => Poll::Ready(self.check(cancellation, deadline).and(result)),
                Poll::Pending => Poll::Pending,
            }
        })
        .await
    }
}

impl McpAuthNetwork for NativeMcpNetwork {
    fn admit<'a>(
        &'a self,
        url: &'a str,
        cancellation: &'a CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'a, std::result::Result<McpAuthDestination, McpAuthError>> {
        Box::pin(async move {
            let endpoint = McpEndpoint::parse(url).map_err(|_| McpAuthError::Invalid)?;
            self.admit_endpoint(&endpoint, cancellation, deadline)
                .await
                .map_err(|error| match error {
                    McpNetworkError::Invalid => McpAuthError::Invalid,
                    McpNetworkError::Unavailable => McpAuthError::Network,
                    McpNetworkError::Limit => McpAuthError::Limit,
                    McpNetworkError::Cancelled => McpAuthError::Cancelled,
                    McpNetworkError::Deadline => McpAuthError::Deadline,
                })
        })
    }
}
