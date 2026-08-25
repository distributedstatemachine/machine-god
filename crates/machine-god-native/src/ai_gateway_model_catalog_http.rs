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
use hickory_resolver::{
    TokioResolver,
    config::{LookupIpStrategy, ProtocolConfig, ResolveHosts, ResolverConfig, ResolverOpts},
    net::runtime::TokioRuntimeProvider,
};
use machine_god_core::{BoxFuture, CancellationToken, Cancelled};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_LENGTH, HeaderValue, USER_AGENT,
};
use reqwest::{Certificate, Client, Method, Request, Url};
use std::error::Error as _;
use std::fmt;
use std::future::{Future, pending, poll_fn};
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};
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
        EndpointKind::Production => SystemHickoryResolver::new(),
        EndpointKind::LoopbackTest => SystemHickoryResolver::unavailable(),
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

struct SystemResolverSnapshot {
    config: ResolverConfig,
    options: ResolverOpts,
}

struct SystemHickoryResolver {
    snapshot: Arc<Result<SystemResolverSnapshot, SystemResolverConfigurationUnavailable>>,
}

impl SystemHickoryResolver {
    fn new() -> Self {
        Self::with_loader(load_system_resolver_snapshot)
    }

    fn unavailable() -> Self {
        Self {
            snapshot: Arc::new(Err(SystemResolverConfigurationUnavailable)),
        }
    }

    fn with_loader<F>(loader: F) -> Self
    where
        F: FnOnce() -> Result<SystemResolverSnapshot, SystemResolverConfigurationUnavailable>,
    {
        Self {
            snapshot: Arc::new(loader().and_then(validate_system_resolver_snapshot)),
        }
    }
}

impl Resolve for SystemHickoryResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let snapshot = Arc::clone(&self.snapshot);
        let name = absolute_hickory_lookup_name(&name);
        Box::pin(async move {
            let snapshot = match Arc::as_ref(&snapshot) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return Err(Box::new(*error) as Box<dyn std::error::Error + Send + Sync>);
                }
            };
            let resolver = build_system_hickory_resolver(snapshot)
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?;
            let lookup = resolver.lookup_ip(name).await?;
            let addrs: Addrs = Box::new(
                lookup
                    .iter()
                    .map(|address| SocketAddr::new(address, 0))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Ok(addrs)
        })
    }
}

fn absolute_hickory_lookup_name(name: &Name) -> String {
    let host = name.as_str().trim_end_matches('.');
    let mut absolute = String::with_capacity(host.len() + 1);
    absolute.push_str(host);
    absolute.push('.');
    absolute
}

fn build_system_hickory_resolver(
    snapshot: &SystemResolverSnapshot,
) -> Result<TokioResolver, SystemResolverConfigurationUnavailable> {
    TokioResolver::builder_with_config(snapshot.config.clone(), TokioRuntimeProvider::default())
        .with_options(snapshot.options.clone())
        .build()
        .map_err(|_| SystemResolverConfigurationUnavailable)
}

fn validate_system_resolver_snapshot(
    mut snapshot: SystemResolverSnapshot,
) -> Result<SystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    let nameservers = snapshot.config.name_servers();
    if nameservers.is_empty() || nameservers.len() > SYSTEM_RESOLVER_MAX_NAMESERVERS {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let connection_count = nameservers.iter().try_fold(0usize, |count, server| {
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
        || snapshot.options.trust_anchor.is_some()
        || !snapshot.options.allow_answers.is_empty()
        || !snapshot.options.deny_answers.is_empty()
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    snapshot.options.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
    snapshot.options.use_hosts_file = ResolveHosts::Never;
    snapshot.options.cache_size = 0;
    Ok(snapshot)
}

#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
fn load_system_resolver_snapshot()
-> Result<SystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    let bytes = read_bounded_system_resolver_configuration(std::path::Path::new(
        SYSTEM_RESOLVER_CONFIG_PATH,
    ))?;
    let (config, options) = hickory_resolver::system_conf::parse_resolv_conf(bytes)
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    Ok(SystemResolverSnapshot { config, options })
}

#[cfg(any(target_os = "android", target_os = "windows", target_vendor = "apple"))]
fn load_system_resolver_snapshot()
-> Result<SystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    let (config, options) = hickory_resolver::system_conf::read_system_conf()
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    Ok(SystemResolverSnapshot { config, options })
}

#[cfg(not(any(
    all(unix, not(any(target_os = "android", target_vendor = "apple"))),
    target_os = "android",
    target_os = "windows",
    target_vendor = "apple"
)))]
fn load_system_resolver_snapshot()
-> Result<SystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
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
    use hickory_resolver::config::{ConnectionConfig, NameServerConfig};
    use hickory_resolver::proto::{
        op::{Message, MessageType},
        rr::{RData, Record, RecordType},
    };
    use std::net::{Ipv4Addr, Ipv6Addr, UdpSocket};
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
    -> Result<SystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
        Err(SystemResolverConfigurationUnavailable)
    }

    fn system_resolver_snapshot(nameservers: Vec<NameServerConfig>) -> SystemResolverSnapshot {
        let mut options = ResolverOpts::default();
        options.timeout = Duration::from_secs(1);
        options.attempts = 1;
        options.num_concurrent_reqs = 1;
        SystemResolverSnapshot {
            config: ResolverConfig::from_parts(None, Vec::new(), nameservers),
            options,
        }
    }

    fn local_system_resolver_snapshot(address: SocketAddr) -> SystemResolverSnapshot {
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
        let resolver = SystemHickoryResolver::with_loader(move || {
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
        assert_eq!(snapshot.options.cache_size, 0);
        assert!(matches!(
            snapshot.options.use_hosts_file,
            ResolveHosts::Never
        ));

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
        assert_eq!(absolute_hickory_lookup_name(&name), "ai-gateway.vercel.sh.");

        let already_absolute = "ai-gateway.vercel.sh."
            .parse::<Name>()
            .expect("parse absolute resolver name");
        assert_eq!(
            absolute_hickory_lookup_name(&already_absolute),
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
        let resolver = SystemHickoryResolver::with_loader(move || {
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
        let resolver: Arc<dyn Resolve> = Arc::new(SystemHickoryResolver::with_loader(
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
        let resolver: Arc<dyn Resolve> = Arc::new(SystemHickoryResolver::with_loader(|| {
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
