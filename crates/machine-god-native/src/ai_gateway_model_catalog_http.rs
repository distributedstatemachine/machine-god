//! Native HTTP GET transport for the bounded AI Gateway model catalog.
//!
//! Machine-god retains at most the inclusive body cap and copies a received
//! data frame in at most 64 KiB steps. Reqwest/Hyper owns raw HTTP framing and
//! may transiently materialize a dependency data frame larger than that step;
//! the first frame that would exceed the retained cap is rejected without
//! appending any of its bytes.

use crate::ai_gateway_http_shared::{authorization_value, root_certificates};
use crate::{
    AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES, AiGatewayBearerToken,
    AiGatewayModelCatalogRequestAccess, AiGatewayModelCatalogTransport,
    AiGatewayModelCatalogTransportError, AiGatewayModelCatalogTransportErrorKind,
    AiGatewayModelCatalogTransportResponse,
};
use hickory_proto::op::{Header, Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{DNSClass, Name as DnsName, RData, RecordType};
use hickory_proto::serialize::binary::{BinDecodable, BinDecoder};
use hickory_resolver::config::{ProtocolConfig, ResolverConfig, ResolverOpts};
use machine_god_core::{BoxFuture, CancellationToken, Cancelled};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_LENGTH, HeaderValue, USER_AGENT,
};
use reqwest::{Certificate, Client, Method, Request, Url};
use sha2::{Digest, Sha256};
use std::error::Error as _;
use std::fmt;
use std::future::{Future, pending, poll_fn};
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::Poll;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Sleep;

/// Pinned production model-catalog endpoint.
pub const AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_ENDPOINT: &str =
    "https://ai-gateway.vercel.sh/coding-agent/v1/models";
/// Default connection-establishment timeout.
pub const AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default per-attempt timeout, subordinate to the provider's shared deadline.
pub const AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Default maximum simultaneous active requests per transport.
pub const AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS: usize = 8;
/// Hard maximum simultaneous active requests per transport.
pub const AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ACTIVE_REQUESTS: usize = 32;
/// Maximum amount copied from a received HTTP data frame in one processing step.
pub const AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_RESPONSE_CHUNK_BYTES: usize = 64 * 1024;
/// Maximum accepted endpoint text size.
pub const AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ENDPOINT_BYTES: usize = 2 * 1024;

const USER_AGENT_VALUE: &str = concat!("machine-god/", env!("CARGO_PKG_VERSION"));
#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
const SYSTEM_RESOLVER_CONFIG_PATH: &str = "/etc/resolv.conf";
#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
const SYSTEM_RESOLVER_MAX_CONFIG_BYTES: usize = 64 * 1024;
const SYSTEM_RESOLVER_MAX_NAMESERVERS: usize = 32;
const SYSTEM_RESOLVER_MAX_SEARCH_DOMAINS: usize = 32;
const SYSTEM_RESOLVER_MAX_NAME_BYTES: usize = 8 * 1024;
const SYSTEM_RESOLVER_MAX_CONNECTIONS: usize = 64;
#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
const SYSTEM_RESOLVER_MAX_INTERRUPTED_READS: usize = 16;
#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
const SYSTEM_RESOLVER_MAX_READ_CALLS: usize =
    SYSTEM_RESOLVER_MAX_CONFIG_BYTES + SYSTEM_RESOLVER_MAX_INTERRUPTED_READS + 2;
const SYSTEM_RESOLVER_MAX_NDOTS: usize = 15;
const SYSTEM_RESOLVER_MAX_ATTEMPTS: usize = 5;
const SYSTEM_RESOLVER_MAX_AVOIDED_PORTS: usize = 1_024;
const SYSTEM_RESOLVER_MAX_DNS_ADDRESSES: usize = 32;
const SYSTEM_RESOLVER_MAX_DNS_CNAME_RECORDS: usize = 7;
const SYSTEM_RESOLVER_MAX_DNS_ANSWER_RECORDS: usize =
    SYSTEM_RESOLVER_MAX_DNS_ADDRESSES + SYSTEM_RESOLVER_MAX_DNS_CNAME_RECORDS;
const SYSTEM_RESOLVER_MAX_DNS_RESOURCE_RECORDS: usize = 4 * SYSTEM_RESOLVER_MAX_DNS_ADDRESSES;
const SYSTEM_RESOLVER_MAX_DNS_MESSAGE_BYTES: usize = 4 * 1_024;

/// Stable construction-error category for the catalog HTTP transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AiGatewayModelCatalogHttpConfigErrorKind {
    /// The endpoint is not the pinned endpoint or a strict loopback test URL.
    InvalidEndpoint,
    /// One or more resource limits are invalid.
    InvalidLimits,
    /// The supplied validated bearer token could not form a header.
    InvalidCredential,
    /// The HTTP backend could not be initialized.
    ClientInitialization,
}

/// Fixed, redacted catalog HTTP construction failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AiGatewayModelCatalogHttpConfigError {
    kind: AiGatewayModelCatalogHttpConfigErrorKind,
}

impl AiGatewayModelCatalogHttpConfigError {
    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(self) -> AiGatewayModelCatalogHttpConfigErrorKind {
        self.kind
    }

    const fn new(kind: AiGatewayModelCatalogHttpConfigErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for AiGatewayModelCatalogHttpConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayModelCatalogHttpConfigError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for AiGatewayModelCatalogHttpConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            AiGatewayModelCatalogHttpConfigErrorKind::InvalidEndpoint => {
                "invalid AI Gateway model catalog endpoint"
            }
            AiGatewayModelCatalogHttpConfigErrorKind::InvalidLimits => {
                "invalid AI Gateway model catalog HTTP limits"
            }
            AiGatewayModelCatalogHttpConfigErrorKind::InvalidCredential => {
                "invalid AI Gateway model catalog credential"
            }
            AiGatewayModelCatalogHttpConfigErrorKind::ClientInitialization => {
                "AI Gateway model catalog HTTP client initialization failed"
            }
        })
    }
}

impl std::error::Error for AiGatewayModelCatalogHttpConfigError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EndpointKind {
    Production,
    LoopbackTest,
}

/// Approved model-catalog HTTP endpoint.
#[derive(Clone, Eq, PartialEq)]
pub struct AiGatewayModelCatalogHttpEndpoint {
    url: Url,
    kind: EndpointKind,
}

impl AiGatewayModelCatalogHttpEndpoint {
    /// Constructs a strict plaintext numeric-loopback endpoint for tests.
    ///
    /// The URL must be canonical ASCII `http`, contain an explicit nonzero
    /// port and absolute path, and contain no userinfo, query, or fragment.
    ///
    /// # Errors
    ///
    /// Returns a redacted error for any noncanonical or non-loopback endpoint.
    pub fn loopback_http(endpoint: &str) -> Result<Self, AiGatewayModelCatalogHttpConfigError> {
        if endpoint.is_empty()
            || endpoint.len() > AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ENDPOINT_BYTES
            || !endpoint.is_ascii()
        {
            return Err(invalid_endpoint());
        }
        let Some(after_scheme) = endpoint.strip_prefix("http://") else {
            return Err(invalid_endpoint());
        };
        let Some((authority, _)) = after_scheme.split_once('/') else {
            return Err(invalid_endpoint());
        };
        let socket = authority
            .parse::<SocketAddr>()
            .map_err(|_| invalid_endpoint())?;
        if socket.port() == 0 || authority != socket.to_string() {
            return Err(invalid_endpoint());
        }
        let url = endpoint.parse::<Url>().map_err(|_| invalid_endpoint())?;
        if url.as_str() != endpoint
            || url.scheme() != "http"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.path().starts_with('/')
        {
            return Err(invalid_endpoint());
        }
        let host = url.host_str().ok_or_else(invalid_endpoint)?;
        let numeric_host = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);
        let address = numeric_host
            .parse::<IpAddr>()
            .map_err(|_| invalid_endpoint())?;
        if address != socket.ip() || url.port() != Some(socket.port()) {
            return Err(invalid_endpoint());
        }
        let loopback = match address {
            IpAddr::V4(address) => address.octets()[0] == 127,
            IpAddr::V6(address) => address.is_loopback(),
        };
        if !loopback {
            return Err(invalid_endpoint());
        }
        Ok(Self {
            url,
            kind: EndpointKind::LoopbackTest,
        })
    }
}

impl Default for AiGatewayModelCatalogHttpEndpoint {
    fn default() -> Self {
        Self {
            url: AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_ENDPOINT
                .parse()
                .expect("the pinned model catalog endpoint is valid"),
            kind: EndpointKind::Production,
        }
    }
}

impl fmt::Debug for AiGatewayModelCatalogHttpEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayModelCatalogHttpEndpoint")
            .field("kind", &self.kind)
            .field("url", &"<redacted>")
            .finish()
    }
}

fn invalid_endpoint() -> AiGatewayModelCatalogHttpConfigError {
    AiGatewayModelCatalogHttpConfigError::new(
        AiGatewayModelCatalogHttpConfigErrorKind::InvalidEndpoint,
    )
}

/// Native catalog connection, attempt, and concurrency bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AiGatewayModelCatalogHttpLimits {
    connect_timeout: Duration,
    request_timeout: Duration,
    max_active_requests: usize,
}

impl AiGatewayModelCatalogHttpLimits {
    /// Constructs explicit nonzero limits no greater than the fixed defaults.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when either timeout is zero or over 30 seconds,
    /// connection timeout exceeds request timeout, or concurrency is outside
    /// `1..=32`.
    pub fn new(
        connect_timeout: Duration,
        request_timeout: Duration,
        max_active_requests: usize,
    ) -> Result<Self, AiGatewayModelCatalogHttpConfigError> {
        if connect_timeout.is_zero()
            || request_timeout.is_zero()
            || connect_timeout > request_timeout
            || connect_timeout > AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_CONNECT_TIMEOUT
            || request_timeout > AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_REQUEST_TIMEOUT
            || !(1..=AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ACTIVE_REQUESTS)
                .contains(&max_active_requests)
        {
            return Err(AiGatewayModelCatalogHttpConfigError::new(
                AiGatewayModelCatalogHttpConfigErrorKind::InvalidLimits,
            ));
        }
        Ok(Self {
            connect_timeout,
            request_timeout,
            max_active_requests,
        })
    }

    /// Returns the connection-establishment timeout.
    #[must_use]
    pub const fn connect_timeout(self) -> Duration {
        self.connect_timeout
    }

    /// Returns the per-attempt timeout subordinate to the provider deadline.
    #[must_use]
    pub const fn request_timeout(self) -> Duration {
        self.request_timeout
    }

    /// Returns the simultaneous active-request bound.
    #[must_use]
    pub const fn max_active_requests(self) -> usize {
        self.max_active_requests
    }
}

impl Default for AiGatewayModelCatalogHttpLimits {
    fn default() -> Self {
        Self {
            connect_timeout: AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_CONNECT_TIMEOUT,
            request_timeout: AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_REQUEST_TIMEOUT,
            max_active_requests: AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS,
        }
    }
}

/// Tokio-hosted HTTP implementation of [`AiGatewayModelCatalogTransport`].
///
/// Production construction synchronously snapshots bounded platform DNS
/// configuration exactly once, performs no network request, and requires no
/// runtime. Numeric-loopback test construction performs no DNS discovery.
/// Polling the returned request future requires a current Tokio runtime with
/// I/O and time enabled. Drop owns cancellation: request, response, resolver,
/// permit, and waiters remain inside that future and are released together.
///
/// # Panics
///
/// Tokio may panic if an active runtime handle lacks its I/O or time driver.
pub struct AiGatewayModelCatalogHttpTransport {
    client: Client,
    endpoint: AiGatewayModelCatalogHttpEndpoint,
    authorization: Option<HeaderValue>,
    limits: AiGatewayModelCatalogHttpLimits,
    permits: Arc<Semaphore>,
}

impl AiGatewayModelCatalogHttpTransport {
    /// Creates a production transport with optional authentication.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted error if the credential header or HTTP backend
    /// cannot be initialized.
    pub fn new(
        token: Option<AiGatewayBearerToken>,
    ) -> Result<Self, AiGatewayModelCatalogHttpConfigError> {
        Self::with_endpoint_and_limits(
            token,
            AiGatewayModelCatalogHttpEndpoint::default(),
            AiGatewayModelCatalogHttpLimits::default(),
        )
    }

    /// Creates a transport with an approved endpoint and explicit limits.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted error if the credential header or HTTP backend
    /// cannot be initialized.
    pub fn with_endpoint_and_limits(
        token: Option<AiGatewayBearerToken>,
        endpoint: AiGatewayModelCatalogHttpEndpoint,
        limits: AiGatewayModelCatalogHttpLimits,
    ) -> Result<Self, AiGatewayModelCatalogHttpConfigError> {
        let authorization = token
            .as_ref()
            .map(authorization_value)
            .transpose()
            .map_err(|_| {
                AiGatewayModelCatalogHttpConfigError::new(
                    AiGatewayModelCatalogHttpConfigErrorKind::InvalidCredential,
                )
            })?;
        drop(token);
        let certificates = root_certificates().map_err(|_| {
            AiGatewayModelCatalogHttpConfigError::new(
                AiGatewayModelCatalogHttpConfigErrorKind::ClientInitialization,
            )
        })?;
        let client = build_client(certificates, endpoint.kind, limits)?;
        Ok(Self {
            client,
            endpoint,
            authorization,
            limits,
            permits: Arc::new(Semaphore::new(limits.max_active_requests)),
        })
    }
}

fn build_client(
    certificates: Vec<Certificate>,
    endpoint_kind: EndpointKind,
    limits: AiGatewayModelCatalogHttpLimits,
) -> Result<Client, AiGatewayModelCatalogHttpConfigError> {
    let resolver = match endpoint_kind {
        EndpointKind::Production => SystemBoundedResolver::new(),
        EndpointKind::LoopbackTest => SystemBoundedResolver::unavailable(),
    };
    client_builder(certificates, limits)
        .dns_resolver(resolver)
        .build()
        .map_err(|_| {
            AiGatewayModelCatalogHttpConfigError::new(
                AiGatewayModelCatalogHttpConfigErrorKind::ClientInitialization,
            )
        })
}

struct ParsedSystemResolverSnapshot {
    config: ResolverConfig,
    options: ResolverOpts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SystemNameServer {
    udp: Option<SocketAddr>,
    tcp: Option<SocketAddr>,
    trust_negative_responses: bool,
}

struct SystemResolverSnapshot {
    name_servers: Vec<SystemNameServer>,
    timeout: Duration,
    attempts: usize,
    concurrent_requests: usize,
    try_tcp_on_error: bool,
    recursion_desired: bool,
}

struct QueryIdSequence {
    key: [u8; 32],
    counter: AtomicU32,
}

impl QueryIdSequence {
    fn new(key: [u8; 32]) -> Self {
        Self::with_counter(key, 0)
    }

    fn with_counter(key: [u8; 32], counter: u32) -> Self {
        Self {
            key,
            counter: AtomicU32::new(counter),
        }
    }

    fn next(&self) -> u16 {
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut digest = Sha256::new();
        digest.update(self.key);
        digest.update(counter.to_be_bytes());
        let digest = digest.finalize();
        u16::from_be_bytes([digest[0], digest[1]])
    }
}

struct SystemBoundedResolver {
    snapshot: Arc<Result<SystemResolverSnapshot, SystemResolverConfigurationUnavailable>>,
    query_ids: Arc<Result<QueryIdSequence, SystemResolverConfigurationUnavailable>>,
}

impl SystemBoundedResolver {
    fn new() -> Self {
        Self::with_sources(load_system_resolver_snapshot, system_resolver_query_id_key)
    }

    fn unavailable() -> Self {
        Self {
            snapshot: Arc::new(Err(SystemResolverConfigurationUnavailable)),
            query_ids: Arc::new(Err(SystemResolverConfigurationUnavailable)),
        }
    }

    #[cfg(test)]
    fn with_loader<F>(loader: F) -> Self
    where
        F: FnOnce() -> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable>,
    {
        Self::with_sources(loader, system_resolver_query_id_key)
    }

    fn with_sources<F, E>(loader: F, entropy: E) -> Self
    where
        F: FnOnce() -> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable>,
        E: FnOnce() -> Result<[u8; 32], SystemResolverConfigurationUnavailable>,
    {
        Self {
            snapshot: Arc::new(
                loader().and_then(|snapshot| validate_system_resolver_snapshot(&snapshot)),
            ),
            query_ids: Arc::new(entropy().map(QueryIdSequence::new)),
        }
    }
}

impl Resolve for SystemBoundedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let snapshot = Arc::clone(&self.snapshot);
        let query_ids = Arc::clone(&self.query_ids);
        let name = absolute_system_lookup_name(&name);
        Box::pin(async move {
            let snapshot = match Arc::as_ref(&snapshot) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return Err(Box::new(*error) as Box<dyn std::error::Error + Send + Sync>);
                }
            };
            let query_ids = match Arc::as_ref(&query_ids) {
                Ok(query_ids) => query_ids,
                Err(error) => {
                    return Err(Box::new(*error) as Box<dyn std::error::Error + Send + Sync>);
                }
            };
            let lookup = lookup_system_addresses(snapshot, query_ids, &name)
                .await
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?;
            let addrs: Addrs = Box::new(
                lookup
                    .into_iter()
                    .map(|address| SocketAddr::new(address, 0)),
            );
            Ok(addrs)
        })
    }
}

fn absolute_system_lookup_name(name: &Name) -> String {
    let host = name.as_str().trim_end_matches('.');
    let mut absolute = String::with_capacity(host.len() + 1);
    absolute.push_str(host);
    absolute.push('.');
    absolute
}

fn system_resolver_query_id_key() -> Result<[u8; 32], SystemResolverConfigurationUnavailable> {
    let mut key = [0_u8; 32];
    getrandom::fill(&mut key).map_err(|_| SystemResolverConfigurationUnavailable)?;
    Ok(key)
}

fn validate_system_resolver_snapshot(
    snapshot: &ParsedSystemResolverSnapshot,
) -> Result<SystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    let configured_servers = snapshot.config.name_servers();
    if configured_servers.is_empty() || configured_servers.len() > SYSTEM_RESOLVER_MAX_NAMESERVERS {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let connection_count = configured_servers.iter().try_fold(0usize, |count, server| {
        if server.connections.is_empty()
            || server.connections.len() > 2
            || server.connections.iter().any(|item| {
                item.port == 0
                    || !matches!(item.protocol, ProtocolConfig::Udp | ProtocolConfig::Tcp)
            })
        {
            return None;
        }
        count.checked_add(server.connections.len())
    });
    if connection_count.is_none_or(|count| count > SYSTEM_RESOLVER_MAX_CONNECTIONS) {
        return Err(SystemResolverConfigurationUnavailable);
    }
    if snapshot.config.search().len() > SYSTEM_RESOLVER_MAX_SEARCH_DOMAINS {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let name_bytes = snapshot
        .config
        .domain()
        .into_iter()
        .chain(snapshot.config.search())
        .try_fold(0usize, |count, name| count.checked_add(name.len()));
    if name_bytes.is_none_or(|count| count > SYSTEM_RESOLVER_MAX_NAME_BYTES)
        || snapshot.options.ndots > SYSTEM_RESOLVER_MAX_NDOTS
        || snapshot.options.timeout.is_zero()
        || snapshot.options.timeout > AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_REQUEST_TIMEOUT
        || !(1..=SYSTEM_RESOLVER_MAX_ATTEMPTS).contains(&snapshot.options.attempts)
        || snapshot.options.num_concurrent_reqs > SYSTEM_RESOLVER_MAX_NAMESERVERS
        || !(1..=AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_ACTIVE_REQUESTS)
            .contains(&snapshot.options.max_active_requests)
        || snapshot.options.avoid_local_udp_ports.len() > SYSTEM_RESOLVER_MAX_AVOIDED_PORTS
        || !snapshot.options.avoid_local_udp_ports.is_empty()
        || snapshot.options.case_randomization
        || snapshot.options.trust_anchor.is_some()
        || !snapshot.options.allow_answers.is_empty()
        || !snapshot.options.deny_answers.is_empty()
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let mut name_servers = Vec::with_capacity(configured_servers.len());
    for server in configured_servers {
        let udp = server
            .connections
            .iter()
            .find(|connection| connection.protocol == ProtocolConfig::Udp)
            .map(|connection| SocketAddr::new(server.ip, connection.port));
        let tcp = server
            .connections
            .iter()
            .find(|connection| connection.protocol == ProtocolConfig::Tcp)
            .map(|connection| SocketAddr::new(server.ip, connection.port));
        if udp.is_some() || tcp.is_some() {
            name_servers.push(SystemNameServer {
                udp,
                tcp,
                trust_negative_responses: server.trust_negative_responses,
            });
        }
    }
    if name_servers.is_empty() {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(SystemResolverSnapshot {
        name_servers,
        timeout: snapshot.options.timeout,
        attempts: snapshot.options.attempts,
        concurrent_requests: snapshot.options.num_concurrent_reqs.max(1),
        try_tcp_on_error: snapshot.options.try_tcp_on_error,
        recursion_desired: snapshot.options.recursion_desired,
    })
}

async fn lookup_system_addresses(
    snapshot: &SystemResolverSnapshot,
    query_ids: &QueryIdSequence,
    name: &str,
) -> Result<Vec<IpAddr>, SystemResolverConfigurationUnavailable> {
    let name = DnsName::from_ascii(name).map_err(|_| SystemResolverConfigurationUnavailable)?;
    if !name.is_fqdn() {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let mut ipv4 = Box::pin(query_system_record(
        snapshot,
        query_ids,
        &name,
        RecordType::A,
    ));
    let mut ipv6 = Box::pin(query_system_record(
        snapshot,
        query_ids,
        &name,
        RecordType::AAAA,
    ));
    let mut ipv4_result = None;
    let mut ipv6_result = None;
    let (ipv4_result, ipv6_result) = poll_fn(|context| {
        if ipv4_result.is_none()
            && let Poll::Ready(result) = ipv4.as_mut().poll(context)
        {
            ipv4_result = Some(result);
        }
        if ipv6_result.is_none()
            && let Poll::Ready(result) = ipv6.as_mut().poll(context)
        {
            ipv6_result = Some(result);
        }
        match (ipv4_result.take(), ipv6_result.take()) {
            (Some(ipv4), Some(ipv6)) => Poll::Ready((ipv4, ipv6)),
            (ipv4, ipv6) => {
                ipv4_result = ipv4;
                ipv6_result = ipv6;
                Poll::Pending
            }
        }
    })
    .await;
    let mut addresses = Vec::new();
    let mut successful_family = false;
    for resolved in [ipv4_result, ipv6_result].into_iter().flatten() {
        successful_family = true;
        for address in resolved {
            if !addresses.contains(&address) {
                addresses.push(address);
                if addresses.len() > SYSTEM_RESOLVER_MAX_DNS_ADDRESSES {
                    return Err(SystemResolverConfigurationUnavailable);
                }
            }
        }
    }
    if !successful_family || addresses.is_empty() {
        Err(SystemResolverConfigurationUnavailable)
    } else {
        Ok(addresses)
    }
}

async fn query_system_record(
    snapshot: &SystemResolverSnapshot,
    query_ids: &QueryIdSequence,
    name: &DnsName,
    record_type: RecordType,
) -> Result<Vec<IpAddr>, SystemResolverConfigurationUnavailable> {
    let mut current = name.clone();
    let mut visited = vec![current.clone()];
    let mut cname_hops = 0usize;
    loop {
        let answer = query_system_record_once(snapshot, query_ids, &current, record_type).await?;
        let target =
            advance_system_cname_chain(&mut visited, &mut cname_hops, &answer.canonical_chain)?;
        if !answer.addresses.is_empty() {
            return Ok(answer.addresses);
        }
        let Some(target) = target else {
            return Ok(Vec::new());
        };
        current = target;
    }
}

fn advance_system_cname_chain(
    visited: &mut Vec<DnsName>,
    cname_hops: &mut usize,
    canonical_chain: &[DnsName],
) -> Result<Option<DnsName>, SystemResolverConfigurationUnavailable> {
    for target in canonical_chain {
        *cname_hops = cname_hops
            .checked_add(1)
            .ok_or(SystemResolverConfigurationUnavailable)?;
        if *cname_hops > SYSTEM_RESOLVER_MAX_DNS_CNAME_RECORDS || visited.contains(target) {
            return Err(SystemResolverConfigurationUnavailable);
        }
        visited.push(target.clone());
    }
    Ok(canonical_chain.last().cloned())
}

async fn query_system_record_once(
    snapshot: &SystemResolverSnapshot,
    query_ids: &QueryIdSequence,
    name: &DnsName,
    record_type: RecordType,
) -> Result<SystemDnsAnswer, SystemResolverConfigurationUnavailable> {
    for _ in 0..snapshot.attempts {
        for servers in snapshot.name_servers.chunks(snapshot.concurrent_requests) {
            let mut pending_queries = Vec::with_capacity(servers.len());
            for server in servers {
                let id = query_ids.next();
                let query = system_dns_query(id, name, record_type, snapshot.recursion_desired)?;
                let deadline = tokio::time::Instant::now() + snapshot.timeout;
                let future = Box::pin(async move {
                    tokio::time::timeout_at(
                        deadline,
                        query_system_name_server(
                            *server,
                            &query,
                            id,
                            name,
                            record_type,
                            snapshot.try_tcp_on_error,
                        ),
                    )
                    .await
                    .unwrap_or(Err(SystemDnsServerError::Retry))
                });
                pending_queries.push(Some(future));
            }
            let batch = poll_fn(|context| {
                let mut all_complete = true;
                for pending in &mut pending_queries {
                    let Some(future) = pending.as_mut() else {
                        continue;
                    };
                    match future.as_mut().poll(context) {
                        Poll::Ready(Ok(answer)) => return Poll::Ready(Ok(answer)),
                        Poll::Ready(Err(SystemDnsServerError::TrustedNegative)) => {
                            return Poll::Ready(Ok(SystemDnsAnswer::empty()));
                        }
                        Poll::Ready(Err(SystemDnsServerError::Retry)) => *pending = None,
                        Poll::Pending => all_complete = false,
                    }
                }
                if all_complete {
                    Poll::Ready(Err(SystemResolverConfigurationUnavailable))
                } else {
                    Poll::Pending
                }
            })
            .await;
            if let Ok(answer) = batch {
                return Ok(answer);
            }
        }
    }
    Err(SystemResolverConfigurationUnavailable)
}

fn system_dns_query(
    id: u16,
    name: &DnsName,
    record_type: RecordType,
    recursion_desired: bool,
) -> Result<Vec<u8>, SystemResolverConfigurationUnavailable> {
    let mut query = Message::new(id, MessageType::Query, OpCode::Query);
    query.metadata.recursion_desired = recursion_desired;
    query.add_query(Query::query(name.clone(), record_type));
    let wire = query
        .to_vec()
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    if wire.len() > SYSTEM_RESOLVER_MAX_DNS_MESSAGE_BYTES {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(wire)
}

async fn query_system_name_server(
    server: SystemNameServer,
    wire: &[u8],
    id: u16,
    name: &DnsName,
    record_type: RecordType,
    try_tcp_on_error: bool,
) -> Result<SystemDnsAnswer, SystemDnsServerError> {
    let response = if let Some(udp) = server.udp {
        match exchange_system_dns_udp(udp, wire)
            .await
            .and_then(|response| classify_system_dns_udp_response(&response, id, name, record_type))
        {
            Ok(SystemDnsUdpResponse::Complete(response)) => Ok(response),
            Ok(SystemDnsUdpResponse::Truncated) => {
                let Some(tcp) = server.tcp else {
                    return Err(SystemDnsServerError::Retry);
                };
                exchange_system_dns_tcp(tcp, wire).await
            }
            Err(_) if try_tcp_on_error => {
                let Some(tcp) = server.tcp else {
                    return Err(SystemDnsServerError::Retry);
                };
                exchange_system_dns_tcp(tcp, wire).await
            }
            Err(error) => Err(error),
        }
    } else {
        let Some(tcp) = server.tcp else {
            return Err(SystemDnsServerError::Retry);
        };
        exchange_system_dns_tcp(tcp, wire).await
    }
    .map_err(|_| SystemDnsServerError::Retry)?;
    match validate_system_dns_response(&response, id, name, record_type) {
        Ok(SystemDnsResponse::Answer(answer)) => Ok(answer),
        Ok(SystemDnsResponse::Negative) if server.trust_negative_responses => {
            Err(SystemDnsServerError::TrustedNegative)
        }
        Ok(SystemDnsResponse::Negative) | Err(_) => Err(SystemDnsServerError::Retry),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SystemDnsServerError {
    Retry,
    TrustedNegative,
}

struct SystemDnsAnswer {
    addresses: Vec<IpAddr>,
    canonical_chain: Vec<DnsName>,
}

impl SystemDnsAnswer {
    fn empty() -> Self {
        Self {
            addresses: Vec::new(),
            canonical_chain: Vec::new(),
        }
    }
}

async fn exchange_system_dns_udp(
    nameserver: SocketAddr,
    wire: &[u8],
) -> Result<Vec<u8>, SystemResolverConfigurationUnavailable> {
    let bind = match nameserver {
        SocketAddr::V4(_) => SocketAddr::from(([0, 0, 0, 0], 0)),
        SocketAddr::V6(_) => SocketAddr::from(([0_u16; 8], 0)),
    };
    let socket = tokio::net::UdpSocket::bind(bind)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    socket
        .connect(nameserver)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    let sent = socket
        .send(wire)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    if sent != wire.len() {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let mut response = [0_u8; SYSTEM_RESOLVER_MAX_DNS_MESSAGE_BYTES + 1];
    let received = socket
        .recv(&mut response)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    if received > SYSTEM_RESOLVER_MAX_DNS_MESSAGE_BYTES {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(response[..received].to_vec())
}

async fn exchange_system_dns_tcp(
    nameserver: SocketAddr,
    wire: &[u8],
) -> Result<Message, SystemResolverConfigurationUnavailable> {
    let wire_len = u16::try_from(wire.len()).map_err(|_| SystemResolverConfigurationUnavailable)?;
    let mut stream = tokio::net::TcpStream::connect(nameserver)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    stream
        .write_all(&wire_len.to_be_bytes())
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    stream
        .write_all(wire)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    let response_len = stream
        .read_u16()
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    let response_len = usize::from(response_len);
    if !(12..=SYSTEM_RESOLVER_MAX_DNS_MESSAGE_BYTES).contains(&response_len) {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let mut response = vec![0_u8; response_len];
    stream
        .read_exact(&mut response)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    decode_system_dns_response(&response)
}

enum SystemDnsUdpResponse {
    Complete(Message),
    Truncated,
}

fn classify_system_dns_udp_response(
    response: &[u8],
    id: u16,
    name: &DnsName,
    record_type: RecordType,
) -> Result<SystemDnsUdpResponse, SystemResolverConfigurationUnavailable> {
    validate_system_dns_header_count_caps(response)?;
    let mut decoder = BinDecoder::new(response);
    let header = Header::read(&mut decoder).map_err(|_| SystemResolverConfigurationUnavailable)?;
    if header.metadata.id != id
        || header.metadata.message_type != MessageType::Response
        || header.metadata.op_code != OpCode::Query
        || !matches!(
            header.metadata.response_code,
            ResponseCode::NoError | ResponseCode::NXDomain
        )
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let query = Query::read(&mut decoder).map_err(|_| SystemResolverConfigurationUnavailable)?;
    if !query.name().is_fqdn()
        || query.name() != name
        || query.query_type() != record_type
        || query.query_class() != DNSClass::IN
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    if header.metadata.truncation {
        Ok(SystemDnsUdpResponse::Truncated)
    } else {
        decode_system_dns_response(response).map(SystemDnsUdpResponse::Complete)
    }
}

fn decode_system_dns_response(
    response: &[u8],
) -> Result<Message, SystemResolverConfigurationUnavailable> {
    validate_system_dns_header_counts(response)?;
    let mut decoder = BinDecoder::new(response);
    let message =
        Message::read(&mut decoder).map_err(|_| SystemResolverConfigurationUnavailable)?;
    if !decoder.is_empty() {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(message)
}

#[derive(Clone, Copy)]
struct SystemDnsHeaderCounts {
    questions: usize,
    answers: usize,
    authorities: usize,
    additionals: usize,
}

impl SystemDnsHeaderCounts {
    fn resource_records(self) -> Option<usize> {
        self.answers
            .checked_add(self.authorities)
            .and_then(|total| total.checked_add(self.additionals))
    }
}

fn validate_system_dns_header_count_caps(
    response: &[u8],
) -> Result<SystemDnsHeaderCounts, SystemResolverConfigurationUnavailable> {
    let header = response
        .get(..12)
        .ok_or(SystemResolverConfigurationUnavailable)?;
    let counts = SystemDnsHeaderCounts {
        questions: usize::from(u16::from_be_bytes([header[4], header[5]])),
        answers: usize::from(u16::from_be_bytes([header[6], header[7]])),
        authorities: usize::from(u16::from_be_bytes([header[8], header[9]])),
        additionals: usize::from(u16::from_be_bytes([header[10], header[11]])),
    };
    let resource_records = counts
        .resource_records()
        .ok_or(SystemResolverConfigurationUnavailable)?;
    if counts.questions != 1
        || counts.answers > SYSTEM_RESOLVER_MAX_DNS_ANSWER_RECORDS
        || counts.authorities > SYSTEM_RESOLVER_MAX_DNS_RESOURCE_RECORDS
        || counts.additionals > SYSTEM_RESOLVER_MAX_DNS_RESOURCE_RECORDS
        || resource_records > SYSTEM_RESOLVER_MAX_DNS_RESOURCE_RECORDS
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(counts)
}

fn validate_system_dns_header_counts(
    response: &[u8],
) -> Result<(), SystemResolverConfigurationUnavailable> {
    let counts = validate_system_dns_header_count_caps(response)?;
    let resource_records = counts
        .resource_records()
        .ok_or(SystemResolverConfigurationUnavailable)?;
    let minimum_wire_len = 12_usize
        .checked_add(
            counts
                .questions
                .checked_mul(5)
                .ok_or(SystemResolverConfigurationUnavailable)?,
        )
        .and_then(|length| {
            resource_records
                .checked_mul(11)
                .and_then(|records| length.checked_add(records))
        })
        .ok_or(SystemResolverConfigurationUnavailable)?;
    if minimum_wire_len > response.len() {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(())
}

fn validate_system_dns_response(
    response: &Message,
    id: u16,
    name: &DnsName,
    record_type: RecordType,
) -> Result<SystemDnsResponse, SystemResolverConfigurationUnavailable> {
    if response.metadata.id != id
        || response.metadata.message_type != MessageType::Response
        || response.metadata.op_code != OpCode::Query
        || response.metadata.truncation
        || response.queries.len() != 1
        || response.queries[0].name() != name
        || response.queries[0].query_type() != record_type
        || response.queries[0].query_class() != DNSClass::IN
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    if response.metadata.response_code == ResponseCode::NXDomain {
        return if response.answers.is_empty() {
            Ok(SystemDnsResponse::Negative)
        } else {
            Err(SystemResolverConfigurationUnavailable)
        };
    }
    if response.metadata.response_code != ResponseCode::NoError {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let mut chain = vec![name.clone()];
    let terminal = loop {
        let current = chain
            .last()
            .expect("the DNS validation chain starts nonempty");
        let mut targets = response.answers.iter().filter_map(|record| {
            (&record.name == current)
                .then_some(record)
                .and_then(|record| match &record.data {
                    RData::CNAME(target) if record.dns_class == DNSClass::IN => {
                        Some(target.0.clone())
                    }
                    _ => None,
                })
        });
        let Some(target) = targets.next() else {
            break current.clone();
        };
        if targets.any(|candidate| candidate != target)
            || chain.contains(&target)
            || chain.len() >= 8
        {
            return Err(SystemResolverConfigurationUnavailable);
        }
        chain.push(target);
    };

    let mut addresses = Vec::new();
    for record in &response.answers {
        match &record.data {
            RData::A(address) => {
                if record_type != RecordType::A
                    || record.dns_class != DNSClass::IN
                    || record.name != terminal
                {
                    return Err(SystemResolverConfigurationUnavailable);
                }
                addresses.push(IpAddr::V4(address.0));
            }
            RData::AAAA(address) => {
                if record_type != RecordType::AAAA
                    || record.dns_class != DNSClass::IN
                    || record.name != terminal
                {
                    return Err(SystemResolverConfigurationUnavailable);
                }
                addresses.push(IpAddr::V6(address.0));
            }
            RData::CNAME(target) => {
                let Some(position) = chain.iter().position(|name| name == &record.name) else {
                    return Err(SystemResolverConfigurationUnavailable);
                };
                if record.dns_class != DNSClass::IN || chain.get(position + 1) != Some(&target.0) {
                    return Err(SystemResolverConfigurationUnavailable);
                }
            }
            _ => {}
        }
        if addresses.len() > SYSTEM_RESOLVER_MAX_DNS_ADDRESSES {
            return Err(SystemResolverConfigurationUnavailable);
        }
    }
    Ok(SystemDnsResponse::Answer(SystemDnsAnswer {
        addresses,
        canonical_chain: chain.into_iter().skip(1).collect(),
    }))
}

enum SystemDnsResponse {
    Answer(SystemDnsAnswer),
    Negative,
}

#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
fn load_system_resolver_snapshot()
-> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    let bytes = read_bounded_system_resolver_configuration(std::path::Path::new(
        SYSTEM_RESOLVER_CONFIG_PATH,
    ))?;
    let (config, options) = hickory_resolver::system_conf::parse_resolv_conf(bytes)
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    Ok(ParsedSystemResolverSnapshot { config, options })
}

#[cfg(any(target_os = "windows", target_vendor = "apple"))]
fn load_system_resolver_snapshot()
-> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    let (config, options) = hickory_resolver::system_conf::read_system_conf()
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    Ok(ParsedSystemResolverSnapshot { config, options })
}

#[cfg(target_os = "android")]
fn load_system_resolver_snapshot()
-> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    // Hickory's Android loader requires initialized NDK process context and
    // panics when that global context is absent. A release panic aborts this
    // process, so catalog DNS stays unavailable until a non-panicking native
    // configuration boundary is explicitly provided.
    Err(SystemResolverConfigurationUnavailable)
}

#[cfg(not(any(
    all(unix, not(any(target_os = "android", target_vendor = "apple"))),
    target_os = "android",
    target_os = "windows",
    target_vendor = "apple"
)))]
fn load_system_resolver_snapshot()
-> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    Err(SystemResolverConfigurationUnavailable)
}

#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
fn read_bounded_system_resolver_configuration(
    path: &std::path::Path,
) -> Result<Vec<u8>, SystemResolverConfigurationUnavailable> {
    use std::fs::OpenOptions;
    use std::io::Read as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let initial_metadata =
        std::fs::metadata(path).map_err(|_| SystemResolverConfigurationUnavailable)?;
    validate_system_resolver_file_metadata(&initial_metadata)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    let metadata = file
        .metadata()
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    validate_system_resolver_file_metadata(&metadata)?;

    let mut bytes = Vec::with_capacity(SYSTEM_RESOLVER_MAX_CONFIG_BYTES + 1);
    let mut buffer = [0_u8; 8 * 1024];
    let mut interrupted = 0usize;
    let mut read_calls = 0usize;
    loop {
        if read_calls >= SYSTEM_RESOLVER_MAX_READ_CALLS {
            return Err(SystemResolverConfigurationUnavailable);
        }
        read_calls += 1;
        let remaining = SYSTEM_RESOLVER_MAX_CONFIG_BYTES
            .saturating_sub(bytes.len())
            .saturating_add(1);
        let read_limit = remaining.min(buffer.len());
        match file.read(&mut buffer[..read_limit]) {
            Ok(0) => {
                let final_metadata = file
                    .metadata()
                    .map_err(|_| SystemResolverConfigurationUnavailable)?;
                validate_system_resolver_file_metadata(&final_metadata)?;
                return Ok(bytes);
            }
            Ok(read) => {
                bytes.extend_from_slice(&buffer[..read]);
                if bytes.len() > SYSTEM_RESOLVER_MAX_CONFIG_BYTES {
                    return Err(SystemResolverConfigurationUnavailable);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                interrupted += 1;
                if interrupted > SYSTEM_RESOLVER_MAX_INTERRUPTED_READS {
                    return Err(SystemResolverConfigurationUnavailable);
                }
            }
            Err(_) => return Err(SystemResolverConfigurationUnavailable),
        }
    }
}

#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
fn validate_system_resolver_file_metadata(
    metadata: &std::fs::Metadata,
) -> Result<(), SystemResolverConfigurationUnavailable> {
    if metadata.file_type().is_file() && metadata.len() <= SYSTEM_RESOLVER_MAX_CONFIG_BYTES as u64 {
        Ok(())
    } else {
        Err(SystemResolverConfigurationUnavailable)
    }
}

#[derive(Clone, Copy)]
struct SystemResolverConfigurationUnavailable;

impl fmt::Debug for SystemResolverConfigurationUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SystemResolverConfigurationUnavailable")
    }
}

impl fmt::Display for SystemResolverConfigurationUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("system resolver configuration unavailable")
    }
}

impl std::error::Error for SystemResolverConfigurationUnavailable {}

fn client_builder(
    certificates: Vec<Certificate>,
    limits: AiGatewayModelCatalogHttpLimits,
) -> reqwest::ClientBuilder {
    Client::builder()
        .no_hickory_dns()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .referer(false)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .http1_only()
        .connect_timeout(limits.connect_timeout)
        .pool_max_idle_per_host(limits.max_active_requests)
        .connection_verbose(false)
        .tls_backend_rustls()
        .tls_sslkeylogfile(false)
        .tls_certs_only(certificates)
}

impl fmt::Debug for AiGatewayModelCatalogHttpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayModelCatalogHttpTransport")
            .field("client", &"<redacted>")
            .field("endpoint", &"<redacted>")
            .field("authorization", &"<redacted>")
            .field("limits", &self.limits)
            .field("permits", &"<redacted>")
            .finish()
    }
}

impl AiGatewayModelCatalogTransport for AiGatewayModelCatalogHttpTransport {
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if tokio::runtime::Handle::try_current().is_err() {
                pending::<()>().await;
                return;
            }
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
        })
    }

    fn get(
        &self,
        access: AiGatewayModelCatalogRequestAccess,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<AiGatewayModelCatalogTransportResponse, AiGatewayModelCatalogTransportError>,
    > {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(cancelled_error());
            }
            if tokio::runtime::Handle::try_current().is_err() {
                return Err(runtime_required_error());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(resource_limit_error());
            }
            let attempt_deadline = now
                .checked_add(self.limits.request_timeout)
                .map_or(deadline, |attempt| attempt.min(deadline));
            let mut authority = RequestAuthority::new(cancellation, attempt_deadline);
            let permit = authority
                .wait(Arc::clone(&self.permits).acquire_owned())
                .await?
                .map_err(|_| transport_error())?;
            authority.check()?;
            let request = build_request(
                self.endpoint.url.clone(),
                self.authorization.as_ref(),
                access,
            )?;
            authority.check()?;
            let response = authority
                .wait(self.client.execute(request))
                .await?
                .map_err(|error| map_reqwest_error(&error))?;
            authority.check()?;
            let status = response.status().as_u16();
            if status != 200 {
                return Ok(AiGatewayModelCatalogTransportResponse::new(
                    status,
                    Vec::new(),
                ));
            }
            let body = read_body(response, &mut authority, permit).await?;
            authority.check()?;
            Ok(AiGatewayModelCatalogTransportResponse::new(status, body))
        })
    }
}

fn build_request(
    endpoint: Url,
    authorization: Option<&HeaderValue>,
    access: AiGatewayModelCatalogRequestAccess,
) -> Result<Request, AiGatewayModelCatalogTransportError> {
    let mut request = Request::new(Method::GET, endpoint);
    let headers = request.headers_mut();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    match access {
        AiGatewayModelCatalogRequestAccess::Authenticated => {
            let authorization = authorization.ok_or_else(malformed_transport_error)?;
            headers.insert(AUTHORIZATION, authorization.clone());
        }
        AiGatewayModelCatalogRequestAccess::Public => {}
    }
    Ok(request)
}

async fn read_body(
    mut response: reqwest::Response,
    authority: &mut RequestAuthority,
    _permit: OwnedSemaphorePermit,
) -> Result<Vec<u8>, AiGatewayModelCatalogTransportError> {
    if let Some(content_length) = response.headers().get(CONTENT_LENGTH) {
        let content_length = content_length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(malformed_transport_error)?;
        if content_length > AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES as u64 {
            return Err(resource_limit_error());
        }
    }
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES),
    );
    loop {
        authority.check()?;
        let chunk = authority
            .wait(response.chunk())
            .await?
            .map_err(|error| map_reqwest_error(&error))?;
        authority.check()?;
        let Some(chunk) = chunk else {
            return Ok(body);
        };
        if chunk.is_empty() {
            continue;
        }
        let total = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(resource_limit_error)?;
        if total > AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES {
            return Err(resource_limit_error());
        }
        for part in chunk.chunks(AI_GATEWAY_MODEL_CATALOG_HTTP_MAX_RESPONSE_CHUNK_BYTES) {
            body.extend_from_slice(part);
        }
    }
}

struct RequestAuthority {
    cancellation: CancellationToken,
    cancelled: Pin<Box<Cancelled>>,
    deadline: Instant,
    timeout: Pin<Box<Sleep>>,
}

impl RequestAuthority {
    fn new(cancellation: CancellationToken, deadline: Instant) -> Self {
        let cancelled = Box::pin(cancellation.cancelled());
        let timeout = Box::pin(tokio::time::sleep_until(tokio::time::Instant::from_std(
            deadline,
        )));
        Self {
            cancellation,
            cancelled,
            deadline,
            timeout,
        }
    }

    fn check(&self) -> Result<(), AiGatewayModelCatalogTransportError> {
        if self.cancellation.is_cancelled() {
            Err(cancelled_error())
        } else if Instant::now() >= self.deadline {
            Err(resource_limit_error())
        } else {
            Ok(())
        }
    }

    async fn wait<F>(&mut self, future: F) -> Result<F::Output, AiGatewayModelCatalogTransportError>
    where
        F: Future,
    {
        let mut future = Box::pin(future);
        poll_fn(|context| {
            if self.cancelled.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(cancelled_error()));
            }
            if self.timeout.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(resource_limit_error()));
            }
            match future.as_mut().poll(context) {
                Poll::Ready(output) => match self.check() {
                    Ok(()) => Poll::Ready(Ok(output)),
                    Err(error) => Poll::Ready(Err(error)),
                },
                Poll::Pending => Poll::Pending,
            }
        })
        .await
    }
}

fn map_reqwest_error(error: &reqwest::Error) -> AiGatewayModelCatalogTransportError {
    if error.is_timeout() {
        resource_limit_error()
    } else if error_chain_contains_tls(error)
        || error.is_connect()
        || error.is_body()
        || error_chain_contains_retryable_transport(error)
    {
        transport_error()
    } else if error.is_builder()
        || error.is_decode()
        || error.is_redirect()
        || error.is_status()
        || error.is_upgrade()
    {
        malformed_transport_error()
    } else {
        transport_error()
    }
}

fn error_chain_contains_retryable_transport(error: &reqwest::Error) -> bool {
    let mut current = error.source();
    while let Some(source) = current {
        if let Some(error) = source.downcast_ref::<std::io::Error>()
            && !matches!(
                error.kind(),
                std::io::ErrorKind::InvalidData | std::io::ErrorKind::InvalidInput
            )
        {
            return true;
        }
        if let Some(error) = source.downcast_ref::<hyper::Error>()
            && (error.is_incomplete_message()
                || error.is_closed()
                || error.is_canceled()
                || error.is_body_write_aborted())
        {
            return true;
        }
        current = source.source();
    }
    false
}

fn error_chain_contains_tls(error: &reqwest::Error) -> bool {
    let mut current = error.source();
    while let Some(source) = current {
        if source.downcast_ref::<rustls::Error>().is_some() {
            return true;
        }
        current = source.source();
    }
    false
}

const fn transport_error() -> AiGatewayModelCatalogTransportError {
    AiGatewayModelCatalogTransportError::new(AiGatewayModelCatalogTransportErrorKind::Transport)
}

const fn malformed_transport_error() -> AiGatewayModelCatalogTransportError {
    AiGatewayModelCatalogTransportError::new(
        AiGatewayModelCatalogTransportErrorKind::MalformedResponse,
    )
}

const fn resource_limit_error() -> AiGatewayModelCatalogTransportError {
    AiGatewayModelCatalogTransportError::new(AiGatewayModelCatalogTransportErrorKind::ResourceLimit)
}

const fn runtime_required_error() -> AiGatewayModelCatalogTransportError {
    AiGatewayModelCatalogTransportError::new(
        AiGatewayModelCatalogTransportErrorKind::RuntimeRequired,
    )
}

const fn cancelled_error() -> AiGatewayModelCatalogTransportError {
    AiGatewayModelCatalogTransportError::new(AiGatewayModelCatalogTransportErrorKind::Cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::{Record, rdata::CNAME};
    use hickory_resolver::config::{ConnectionConfig, NameServerConfig};
    use std::net::{Ipv4Addr, Ipv6Addr, TcpListener, UdpSocket};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll};

    #[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
    static TEMP_DIRECTORY_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    struct PendingResolver {
        started: Arc<AtomicUsize>,
        dropped: Arc<AtomicUsize>,
    }

    impl Resolve for PendingResolver {
        fn resolve(&self, _: Name) -> Resolving {
            self.started.fetch_add(1, Ordering::SeqCst);
            Box::pin(PendingResolution {
                dropped: Arc::clone(&self.dropped),
            })
        }
    }

    struct PendingResolution {
        dropped: Arc<AtomicUsize>,
    }

    impl Future for PendingResolution {
        type Output = Result<Addrs, Box<dyn std::error::Error + Send + Sync>>;

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for PendingResolution {
        fn drop(&mut self) {
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn transport_with_resolver(
        resolver: Arc<dyn Resolve>,
        limits: AiGatewayModelCatalogHttpLimits,
    ) -> AiGatewayModelCatalogHttpTransport {
        let certificates = root_certificates().expect("load fixed root certificates");
        let client = client_builder(certificates, limits)
            .dns_resolver(resolver)
            .build()
            .expect("build catalog client with deterministic resolver");
        AiGatewayModelCatalogHttpTransport {
            client,
            endpoint: AiGatewayModelCatalogHttpEndpoint::default(),
            authorization: None,
            limits,
            permits: Arc::new(Semaphore::new(limits.max_active_requests)),
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime")
    }

    fn unavailable_system_resolver()
    -> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
        Err(SystemResolverConfigurationUnavailable)
    }

    fn system_resolver_snapshot(
        nameservers: Vec<NameServerConfig>,
    ) -> ParsedSystemResolverSnapshot {
        let mut options = ResolverOpts::default();
        options.timeout = Duration::from_secs(1);
        options.attempts = 1;
        options.num_concurrent_reqs = 1;
        ParsedSystemResolverSnapshot {
            config: ResolverConfig::from_parts(None, Vec::new(), nameservers),
            options,
        }
    }

    fn local_system_resolver_snapshot(address: SocketAddr) -> ParsedSystemResolverSnapshot {
        let mut connection = ConnectionConfig::udp();
        connection.port = address.port();
        let nameserver = NameServerConfig::new(address.ip(), true, vec![connection]);
        let mut snapshot = system_resolver_snapshot(vec![nameserver]);
        snapshot.config.add_search(
            "search.invalid."
                .parse()
                .expect("parse local DNS search domain"),
        );
        snapshot
    }

    fn local_system_resolver_snapshot_with_tcp(
        address: SocketAddr,
    ) -> ParsedSystemResolverSnapshot {
        let mut udp = ConnectionConfig::udp();
        udp.port = address.port();
        let mut tcp = ConnectionConfig::tcp();
        tcp.port = address.port();
        let nameserver = NameServerConfig::new(address.ip(), true, vec![udp, tcp]);
        system_resolver_snapshot(vec![nameserver])
    }

    async fn answer_two_ip_queries(socket: tokio::net::UdpSocket) {
        let mut buffer = [0_u8; 512];
        for _ in 0..2 {
            let (read, peer) =
                tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
                    .await
                    .expect("receive DNS query before test timeout")
                    .expect("receive DNS query");
            let request = Message::from_vec(&buffer[..read]).expect("decode DNS query");
            let query = request
                .queries
                .first()
                .expect("DNS query contains one question")
                .clone();
            assert_eq!(query.name().to_string(), "catalog.");
            let answer = match query.query_type() {
                RecordType::A => RData::A(Ipv4Addr::LOCALHOST.into()),
                RecordType::AAAA => RData::AAAA(Ipv6Addr::LOCALHOST.into()),
                other => panic!("unexpected DNS query type: {other:?}"),
            };
            let mut response = Message::new(
                request.metadata.id,
                MessageType::Response,
                request.metadata.op_code,
            );
            response.metadata.recursion_desired = request.metadata.recursion_desired;
            response.metadata.recursion_available = true;
            response.queries.push(query.clone());
            response
                .answers
                .push(Record::from_rdata(query.name().clone(), 60, answer));
            let response = response.to_vec().expect("encode DNS response");
            socket
                .send_to(&response, peer)
                .await
                .expect("send DNS response");
        }
    }

    async fn answer_truncated_cross_response_queries(
        udp: tokio::net::UdpSocket,
        tcp: tokio::net::TcpListener,
    ) {
        let mut buffer = [0_u8; 512];
        for _ in 0..2 {
            let (read, peer) =
                tokio::time::timeout(Duration::from_secs(2), udp.recv_from(&mut buffer))
                    .await
                    .expect("receive initial UDP DNS query before timeout")
                    .expect("receive initial UDP DNS query");
            let request = Message::from_vec(&buffer[..read]).expect("decode initial DNS query");
            assert_eq!(request.queries[0].name().to_string(), "catalog.");
            let mut response = Message::new(
                request.metadata.id,
                MessageType::Response,
                request.metadata.op_code,
            );
            response.metadata.truncation = true;
            response.queries.push(request.queries[0].clone());
            let response = response.to_vec().expect("encode truncated DNS response");
            udp.send_to(&response, peer)
                .await
                .expect("send truncated DNS response");
        }

        for _ in 0..2 {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(2), tcp.accept())
                .await
                .expect("accept TCP DNS replay before timeout")
                .expect("accept TCP DNS replay");
            let query_len = usize::from(stream.read_u16().await.expect("read TCP DNS length"));
            let mut query_wire = vec![0_u8; query_len];
            stream
                .read_exact(&mut query_wire)
                .await
                .expect("read TCP DNS query");
            let request = Message::from_vec(&query_wire).expect("decode TCP DNS query");
            let query = request.queries[0].clone();
            assert_eq!(query.name().to_string(), "catalog.");
            let alias = DnsName::from_ascii("edge.catalog.").expect("parse DNS alias");
            let mut response = Message::new(
                request.metadata.id,
                MessageType::Response,
                request.metadata.op_code,
            );
            response.queries.push(query.clone());
            response.answers.push(Record::from_rdata(
                query.name().clone(),
                60,
                RData::CNAME(CNAME(alias)),
            ));
            let response = response.to_vec().expect("encode TCP CNAME response");
            let response_len = u16::try_from(response.len()).expect("bounded TCP DNS response");
            stream
                .write_all(&response_len.to_be_bytes())
                .await
                .expect("write TCP DNS response length");
            stream
                .write_all(&response)
                .await
                .expect("write TCP DNS response");
        }

        for _ in 0..2 {
            let (read, peer) =
                tokio::time::timeout(Duration::from_secs(2), udp.recv_from(&mut buffer))
                    .await
                    .expect("receive alias UDP DNS query before timeout")
                    .expect("receive alias UDP DNS query");
            let request = Message::from_vec(&buffer[..read]).expect("decode alias DNS query");
            let query = request.queries[0].clone();
            assert_eq!(query.name().to_string(), "edge.catalog.");
            let answer = match query.query_type() {
                RecordType::A => RData::A(Ipv4Addr::LOCALHOST.into()),
                RecordType::AAAA => RData::AAAA(Ipv6Addr::LOCALHOST.into()),
                other => panic!("unexpected DNS query type: {other:?}"),
            };
            let mut response = Message::new(
                request.metadata.id,
                MessageType::Response,
                request.metadata.op_code,
            );
            response.queries.push(query.clone());
            response
                .answers
                .push(Record::from_rdata(query.name().clone(), 60, answer));
            let response = response.to_vec().expect("encode alias DNS response");
            udp.send_to(&response, peer)
                .await
                .expect("send alias DNS response");
        }
    }

    async fn answer_two_tcp_ip_queries(listener: tokio::net::TcpListener) {
        for _ in 0..2 {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
                .await
                .expect("accept TCP-only DNS query before timeout")
                .expect("accept TCP-only DNS query");
            let query_len = usize::from(stream.read_u16().await.expect("read TCP DNS length"));
            let mut query_wire = vec![0_u8; query_len];
            stream
                .read_exact(&mut query_wire)
                .await
                .expect("read TCP DNS query");
            let request = Message::from_vec(&query_wire).expect("decode TCP DNS query");
            let query = request.queries[0].clone();
            let answer = match query.query_type() {
                RecordType::A => RData::A(Ipv4Addr::LOCALHOST.into()),
                RecordType::AAAA => RData::AAAA(Ipv6Addr::LOCALHOST.into()),
                other => panic!("unexpected DNS query type: {other:?}"),
            };
            let mut response = Message::new(
                request.metadata.id,
                MessageType::Response,
                request.metadata.op_code,
            );
            response.queries.push(query.clone());
            response
                .answers
                .push(Record::from_rdata(query.name().clone(), 60, answer));
            let response = response.to_vec().expect("encode TCP-only DNS response");
            let response_len = u16::try_from(response.len()).expect("bounded TCP DNS response");
            stream
                .write_all(&response_len.to_be_bytes())
                .await
                .expect("write TCP DNS response length");
            stream
                .write_all(&response)
                .await
                .expect("write TCP DNS response");
        }
    }

    async fn poll_pending_once<F: Future>(future: Pin<&mut F>) {
        let mut future = future;
        poll_fn(|context| {
            assert!(future.as_mut().poll(context).is_pending());
            Poll::Ready(())
        })
        .await;
    }

    #[test]
    fn system_dns_snapshot_loads_eagerly_once_and_not_during_resolution() {
        let loads = Arc::new(AtomicUsize::new(0));
        let observed_loads = Arc::clone(&loads);
        let resolver = SystemBoundedResolver::with_loader(move || {
            observed_loads.fetch_add(1, Ordering::SeqCst);
            Ok(system_resolver_snapshot(vec![NameServerConfig::udp(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
            )]))
        });
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        let snapshot = resolver
            .snapshot
            .as_ref()
            .as_ref()
            .expect("validated system DNS snapshot");
        assert_eq!(snapshot.name_servers.len(), 1);
        assert_eq!(snapshot.timeout, Duration::from_secs(1));
        assert_eq!(snapshot.attempts, 1);

        for _ in 0..2 {
            let name = "ai-gateway.vercel.sh"
                .parse::<Name>()
                .expect("parse resolver name");
            let lookup = resolver.resolve(name);
            assert_eq!(loads.load(Ordering::SeqCst), 1);
            drop(lookup);
        }
    }

    #[test]
    fn production_dns_lookup_name_is_one_absolute_fqdn() {
        let endpoint = Url::parse(AI_GATEWAY_MODEL_CATALOG_HTTP_DEFAULT_ENDPOINT)
            .expect("parse fixed production endpoint");
        let name = endpoint
            .host_str()
            .expect("production endpoint has a host")
            .parse::<Name>()
            .expect("parse production resolver name");
        assert_eq!(absolute_system_lookup_name(&name), "ai-gateway.vercel.sh.");

        let already_absolute = "ai-gateway.vercel.sh."
            .parse::<Name>()
            .expect("parse absolute resolver name");
        assert_eq!(
            absolute_system_lookup_name(&already_absolute),
            "ai-gateway.vercel.sh."
        );
    }

    #[test]
    fn system_dns_snapshot_builds_fresh_resolvers_across_current_thread_runtimes() {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind local DNS fixture");
        socket
            .set_nonblocking(true)
            .expect("make local DNS fixture nonblocking");
        let address = socket.local_addr().expect("read local DNS address");
        let loads = Arc::new(AtomicUsize::new(0));
        let observed_loads = Arc::clone(&loads);
        let resolver = SystemBoundedResolver::with_loader(move || {
            observed_loads.fetch_add(1, Ordering::SeqCst);
            Ok(local_system_resolver_snapshot(address))
        });
        assert_eq!(loads.load(Ordering::SeqCst), 1);

        for _ in 0..2 {
            let runtime_socket = socket.try_clone().expect("clone local DNS fixture");
            let runtime = runtime();
            runtime.block_on(async {
                let runtime_socket =
                    tokio::net::UdpSocket::from_std(runtime_socket).expect("adopt DNS fixture");
                let responder = tokio::spawn(answer_two_ip_queries(runtime_socket));
                let name = "catalog".parse::<Name>().expect("parse resolver name");
                let addresses = resolver
                    .resolve(name)
                    .await
                    .expect("resolve against local DNS fixture")
                    .map(|address| address.ip())
                    .collect::<Vec<_>>();
                assert_eq!(
                    addresses,
                    vec![
                        IpAddr::V4(Ipv4Addr::LOCALHOST),
                        IpAddr::V6(Ipv6Addr::LOCALHOST)
                    ]
                );
                responder.await.expect("join DNS fixture");
            });
            drop(runtime);
            assert_eq!(loads.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn system_dns_uses_configured_concurrent_nameserver_batch() {
        let blackhole =
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind blackhole DNS fixture");
        let blackhole_address = blackhole.local_addr().expect("read blackhole DNS address");
        let responder_socket =
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind responding DNS fixture");
        responder_socket
            .set_nonblocking(true)
            .expect("make responding DNS fixture nonblocking");
        let responder_address = responder_socket
            .local_addr()
            .expect("read responding DNS address");
        let nameservers = [blackhole_address, responder_address]
            .into_iter()
            .map(|address| {
                let mut connection = ConnectionConfig::udp();
                connection.port = address.port();
                NameServerConfig::new(address.ip(), true, vec![connection])
            })
            .collect();
        let mut snapshot = system_resolver_snapshot(nameservers);
        snapshot.options.num_concurrent_reqs = 2;
        let resolver = SystemBoundedResolver::with_sources(move || Ok(snapshot), || Ok([17; 32]));
        let runtime = runtime();
        runtime.block_on(async {
            let responder_socket = tokio::net::UdpSocket::from_std(responder_socket)
                .expect("adopt responding DNS fixture");
            let responder = tokio::spawn(answer_two_ip_queries(responder_socket));
            let name = "catalog".parse::<Name>().expect("parse resolver name");
            let addresses = resolver
                .resolve(name)
                .await
                .expect("resolve through second server in concurrent batch")
                .map(|address| address.ip())
                .collect::<Vec<_>>();
            assert_eq!(
                addresses,
                vec![
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    IpAddr::V6(Ipv6Addr::LOCALHOST)
                ]
            );
            responder.await.expect("join responding DNS fixture");
        });
        drop(runtime);
        drop(blackhole);
    }

    #[test]
    fn system_dns_query_ids_are_deterministic_and_wrap_after_u32_space() {
        let sequence = QueryIdSequence::new([7_u8; 32]);
        assert_eq!(sequence.next(), 0xd9e3);
        assert_eq!(sequence.next(), 0x0786);

        let wrapping = QueryIdSequence::with_counter([7_u8; 32], u32::MAX);
        assert_eq!(wrapping.next(), 0x715f);
        assert_eq!(wrapping.next(), 0xd9e3);
    }

    #[test]
    fn system_dns_truncation_replays_tcp_and_follows_bounded_cross_response_cname() {
        let tcp = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind local TCP DNS fixture");
        tcp.set_nonblocking(true)
            .expect("make TCP DNS fixture nonblocking");
        let address = tcp.local_addr().expect("read local TCP DNS address");
        let udp = UdpSocket::bind(address).expect("bind matching UDP DNS fixture");
        udp.set_nonblocking(true)
            .expect("make UDP DNS fixture nonblocking");
        let resolver = SystemBoundedResolver::with_sources(
            move || Ok(local_system_resolver_snapshot_with_tcp(address)),
            || Ok([11_u8; 32]),
        );
        let runtime = runtime();
        runtime.block_on(async {
            let udp = tokio::net::UdpSocket::from_std(udp).expect("adopt UDP DNS fixture");
            let tcp = tokio::net::TcpListener::from_std(tcp).expect("adopt TCP DNS fixture");
            let responder = tokio::spawn(answer_truncated_cross_response_queries(udp, tcp));
            let name = "catalog".parse::<Name>().expect("parse resolver name");
            let addresses = resolver
                .resolve(name)
                .await
                .expect("resolve through TCP replay and CNAME")
                .map(|address| address.ip())
                .collect::<Vec<_>>();
            assert_eq!(
                addresses,
                vec![
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    IpAddr::V6(Ipv6Addr::LOCALHOST)
                ]
            );
            responder.await.expect("join DNS fixture");
        });
        drop(runtime);
    }

    #[test]
    fn system_dns_supports_configured_tcp_only_nameserver() {
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind local TCP-only DNS fixture");
        listener
            .set_nonblocking(true)
            .expect("make TCP-only DNS fixture nonblocking");
        let address = listener
            .local_addr()
            .expect("read local TCP-only DNS address");
        let mut connection = ConnectionConfig::tcp();
        connection.port = address.port();
        let nameserver = NameServerConfig::new(address.ip(), true, vec![connection]);
        let resolver = SystemBoundedResolver::with_sources(
            move || Ok(system_resolver_snapshot(vec![nameserver])),
            || Ok([13_u8; 32]),
        );
        let runtime = runtime();
        runtime.block_on(async {
            let listener =
                tokio::net::TcpListener::from_std(listener).expect("adopt TCP-only DNS fixture");
            let responder = tokio::spawn(answer_two_tcp_ip_queries(listener));
            let name = "catalog".parse::<Name>().expect("parse resolver name");
            let addresses = resolver
                .resolve(name)
                .await
                .expect("resolve through TCP-only nameserver")
                .map(|address| address.ip())
                .collect::<Vec<_>>();
            assert_eq!(
                addresses,
                vec![
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    IpAddr::V6(Ipv6Addr::LOCALHOST)
                ]
            );
            responder.await.expect("join TCP-only DNS fixture");
        });
        drop(runtime);
    }

    #[test]
    fn system_dns_decoder_rejects_trailing_bytes_and_oversized_sections() {
        let name = DnsName::from_ascii("catalog.").expect("parse DNS name");
        let mut response = Message::new(7, MessageType::Response, OpCode::Query);
        response.add_query(Query::query(name, RecordType::A));
        let mut trailing = response.to_vec().expect("encode DNS response");
        trailing.push(0);
        assert!(decode_system_dns_response(&trailing).is_err());

        let mut oversized = vec![0_u8; 12];
        oversized[4..6].copy_from_slice(&1_u16.to_be_bytes());
        let too_many_answers = u16::try_from(SYSTEM_RESOLVER_MAX_DNS_ANSWER_RECORDS + 1)
            .expect("DNS answer cap fits u16");
        oversized[6..8].copy_from_slice(&too_many_answers.to_be_bytes());
        assert!(validate_system_dns_header_count_caps(&oversized).is_err());
    }

    #[test]
    fn system_dns_cross_response_cname_chain_rejects_cycles_and_eighth_hop() {
        let first = DnsName::from_ascii("first.example.").expect("parse first name");
        let second = DnsName::from_ascii("second.example.").expect("parse second name");
        let mut visited = vec![first.clone()];
        let mut hops = 0;
        assert_eq!(
            advance_system_cname_chain(&mut visited, &mut hops, std::slice::from_ref(&second),)
                .expect("accept first CNAME hop"),
            Some(second)
        );
        assert!(
            advance_system_cname_chain(&mut visited, &mut hops, std::slice::from_ref(&first),)
                .is_err()
        );

        let mut bounded = vec![first];
        let mut bounded_hops = 0;
        for index in 0..SYSTEM_RESOLVER_MAX_DNS_CNAME_RECORDS {
            let target = DnsName::from_ascii(format!("alias-{index}.example."))
                .expect("parse bounded CNAME target");
            advance_system_cname_chain(
                &mut bounded,
                &mut bounded_hops,
                std::slice::from_ref(&target),
            )
            .expect("accept bounded CNAME hop");
        }
        let eighth = DnsName::from_ascii("eighth.example.").expect("parse eighth name");
        assert!(
            advance_system_cname_chain(
                &mut bounded,
                &mut bounded_hops,
                std::slice::from_ref(&eighth),
            )
            .is_err()
        );
    }

    #[test]
    fn system_dns_snapshot_rejects_poll_time_randomization_configuration() {
        let nameserver = NameServerConfig::udp(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let mut case_randomized = system_resolver_snapshot(vec![nameserver.clone()]);
        case_randomized.options.case_randomization = true;
        assert!(validate_system_resolver_snapshot(&case_randomized).is_err());

        let mut avoided_port = system_resolver_snapshot(vec![nameserver]);
        avoided_port.options.avoid_local_udp_ports = Arc::new([53_u16].into_iter().collect());
        assert!(validate_system_resolver_snapshot(&avoided_port).is_err());
    }

    #[test]
    fn entropy_failure_is_snapshotted_redacted_and_performs_no_network() {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind no-network witness");
        socket
            .set_nonblocking(true)
            .expect("make no-network witness nonblocking");
        let address = socket.local_addr().expect("read no-network address");
        let resolver: Arc<dyn Resolve> = Arc::new(SystemBoundedResolver::with_sources(
            move || Ok(local_system_resolver_snapshot(address)),
            || Err(SystemResolverConfigurationUnavailable),
        ));
        let limits = AiGatewayModelCatalogHttpLimits::new(
            Duration::from_millis(50),
            Duration::from_millis(50),
            1,
        )
        .expect("construct short catalog limits");
        let transport = transport_with_resolver(resolver, limits);
        let runtime = runtime();
        let error = runtime
            .block_on(transport.get(
                AiGatewayModelCatalogRequestAccess::Public,
                Instant::now() + Duration::from_secs(1),
                CancellationToken::new(),
            ))
            .expect_err("construction-time entropy failure must fail closed");
        assert_eq!(
            error.kind(),
            AiGatewayModelCatalogTransportErrorKind::Transport
        );
        assert_eq!(
            error.to_string(),
            "AI Gateway model catalog transport failed"
        );
        assert_eq!(transport.permits.available_permits(), 1);
        let mut packet = [0_u8; 1];
        let receive_error = socket
            .recv_from(&mut packet)
            .expect_err("entropy failure must send no DNS packet");
        assert_eq!(receive_error.kind(), std::io::ErrorKind::WouldBlock);
        drop(runtime);
    }

    #[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
    #[test]
    fn bounded_unix_dns_config_accepts_regular_symlinks_and_rejects_other_shapes() {
        use std::os::unix::fs::symlink;

        let directory = loop {
            let suffix = TEMP_DIRECTORY_COUNTER.fetch_add(1, Ordering::SeqCst);
            let candidate = std::env::temp_dir().join(format!(
                "machine-god-catalog-dns-{}-{suffix}",
                std::process::id()
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create DNS test directory: {error}"),
            }
        };
        let configuration = directory.join("resolv.conf");
        let link = directory.join("resolv-link.conf");
        let oversized = directory.join("resolv-oversized.conf");
        let bytes = b"nameserver 192.0.2.1\nsearch example.test\n";
        std::fs::write(&configuration, bytes).expect("write DNS test configuration");
        symlink(&configuration, &link).expect("create DNS test symlink");
        assert_eq!(
            read_bounded_system_resolver_configuration(&link)
                .expect("read symlink to regular DNS configuration"),
            bytes
        );

        let oversized_file = std::fs::File::create(&oversized).expect("create oversized DNS file");
        oversized_file
            .set_len((SYSTEM_RESOLVER_MAX_CONFIG_BYTES + 1) as u64)
            .expect("size oversized DNS file");
        drop(oversized_file);
        assert!(read_bounded_system_resolver_configuration(&oversized).is_err());
        assert!(read_bounded_system_resolver_configuration(&directory).is_err());

        std::fs::remove_file(&link).expect("remove DNS test symlink");
        std::fs::remove_file(&configuration).expect("remove DNS test configuration");
        std::fs::remove_file(&oversized).expect("remove oversized DNS file");
        std::fs::remove_dir(&directory).expect("remove DNS test directory");
    }

    #[test]
    fn pending_async_dns_releases_request_and_permit_on_cancel_and_deadline() {
        let started = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        let limits = AiGatewayModelCatalogHttpLimits::new(
            Duration::from_secs(30),
            Duration::from_secs(30),
            1,
        )
        .unwrap();
        let resolver: Arc<dyn Resolve> = Arc::new(PendingResolver {
            started: Arc::clone(&started),
            dropped: Arc::clone(&dropped),
        });
        let transport = transport_with_resolver(Arc::clone(&resolver), limits);

        let cancellation = CancellationToken::new();
        let request_cancellation = cancellation.clone();
        let cancellation_runtime = runtime();
        cancellation_runtime.block_on(async {
            tokio::time::pause();
            let mut request = Box::pin(transport.get(
                AiGatewayModelCatalogRequestAccess::Public,
                Instant::now() + Duration::from_secs(60),
                request_cancellation,
            ));
            poll_pending_once(request.as_mut()).await;
            assert_eq!(started.load(Ordering::SeqCst), 1);
            assert_eq!(dropped.load(Ordering::SeqCst), 0);
            assert_eq!(transport.permits.available_permits(), 0);

            cancellation.cancel();
            assert_eq!(
                request.await.unwrap_err().kind(),
                AiGatewayModelCatalogTransportErrorKind::Cancelled
            );
            assert_eq!(dropped.load(Ordering::SeqCst), 1);
            assert_eq!(transport.permits.available_permits(), 1);
        });
        drop(cancellation_runtime);

        let deadline_runtime = runtime();
        deadline_runtime.block_on(async {
            tokio::time::pause();
            let mut request = Box::pin(transport.get(
                AiGatewayModelCatalogRequestAccess::Public,
                Instant::now() + Duration::from_secs(60),
                CancellationToken::new(),
            ));
            poll_pending_once(request.as_mut()).await;
            assert_eq!(started.load(Ordering::SeqCst), 2);
            assert_eq!(dropped.load(Ordering::SeqCst), 1);
            assert_eq!(transport.permits.available_permits(), 0);

            tokio::time::advance(Duration::from_secs(31)).await;
            assert_eq!(
                request.await.unwrap_err().kind(),
                AiGatewayModelCatalogTransportErrorKind::ResourceLimit
            );
            assert_eq!(dropped.load(Ordering::SeqCst), 2);
            assert_eq!(transport.permits.available_permits(), 1);
        });
        drop(deadline_runtime);
    }

    #[test]
    fn unavailable_system_dns_configuration_fails_closed_and_redacted() {
        let limits = AiGatewayModelCatalogHttpLimits::new(
            Duration::from_secs(30),
            Duration::from_secs(30),
            1,
        )
        .unwrap();
        let resolver: Arc<dyn Resolve> = Arc::new(SystemBoundedResolver::with_loader(
            unavailable_system_resolver,
        ));
        let transport = transport_with_resolver(resolver, limits);
        let runtime = runtime();
        let error = runtime
            .block_on(transport.get(
                AiGatewayModelCatalogRequestAccess::Public,
                Instant::now() + Duration::from_secs(60),
                CancellationToken::new(),
            ))
            .unwrap_err();
        assert_eq!(
            error.kind(),
            AiGatewayModelCatalogTransportErrorKind::Transport
        );
        assert_eq!(
            error.to_string(),
            "AI Gateway model catalog transport failed"
        );
        assert_eq!(transport.permits.available_permits(), 1);
        drop(runtime);
    }

    #[test]
    fn oversized_system_dns_configuration_fails_closed_and_redacted() {
        let nameservers = (0..=SYSTEM_RESOLVER_MAX_NAMESERVERS)
            .map(|index| {
                let octet = u8::try_from(index + 1).expect("bounded test index");
                NameServerConfig::udp(IpAddr::V4(Ipv4Addr::new(192, 0, 2, octet)))
            })
            .collect();
        let resolver: Arc<dyn Resolve> = Arc::new(SystemBoundedResolver::with_loader(|| {
            Ok(system_resolver_snapshot(nameservers))
        }));
        let limits = AiGatewayModelCatalogHttpLimits::new(
            Duration::from_secs(30),
            Duration::from_secs(30),
            1,
        )
        .unwrap();
        let transport = transport_with_resolver(resolver, limits);
        let runtime = runtime();
        let error = runtime
            .block_on(transport.get(
                AiGatewayModelCatalogRequestAccess::Public,
                Instant::now() + Duration::from_secs(60),
                CancellationToken::new(),
            ))
            .unwrap_err();
        assert_eq!(
            error.kind(),
            AiGatewayModelCatalogTransportErrorKind::Transport
        );
        assert_eq!(
            error.to_string(),
            "AI Gateway model catalog transport failed"
        );
        assert_eq!(transport.permits.available_permits(), 1);
        drop(runtime);
    }
}
