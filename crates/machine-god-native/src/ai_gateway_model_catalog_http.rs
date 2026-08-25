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
use machine_god_core::{BoxFuture, CancellationToken, Cancelled};
use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_LENGTH, HeaderValue, USER_AGENT,
};
use reqwest::{Certificate, Client, Method, Request, Url};
use std::error::Error as _;
use std::fmt;
use std::future::{Future, poll_fn};
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
/// Construction performs no network request and requires no runtime. Polling
/// the returned request future requires a current Tokio runtime with I/O and
/// time enabled. Drop owns cancellation: request, response, permit, and waiters
/// remain inside that future and are released together.
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
        let client = build_client(certificates, limits)?;
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
    limits: AiGatewayModelCatalogHttpLimits,
) -> Result<Client, AiGatewayModelCatalogHttpConfigError> {
    Client::builder()
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
        .build()
        .map_err(|_| {
            AiGatewayModelCatalogHttpConfigError::new(
                AiGatewayModelCatalogHttpConfigErrorKind::ClientInitialization,
            )
        })
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
