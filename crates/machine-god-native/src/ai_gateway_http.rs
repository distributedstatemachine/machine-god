//! Bounded native HTTP transport for the pinned Vercel AI Gateway endpoint.

use crate::ai_gateway_http_shared::{
    AiGatewayBearerToken, AiGatewayHttpConfigError, AiGatewayHttpConfigErrorKind,
    authorization_value, root_certificates,
};
use crate::{AiGatewayByteStream, AiGatewayTransport, AiGatewayTransportRequest};
use bytes::Bytes;
use futures_core::Stream;
use machine_god_core::{BoxFuture, CancellationToken, Cancelled, ProviderError, ProviderErrorKind};
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, HeaderName, HeaderValue};
use reqwest::{Client, Method, Request, Url};
use std::error::Error as _;
use std::fmt;
use std::future::{Future, poll_fn};
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, Sleep};

/// Pinned production language-model endpoint.
pub const AI_GATEWAY_HTTP_DEFAULT_ENDPOINT: &str =
    "https://ai-gateway.vercel.sh/v3/ai/language-model";
/// Maximum accepted endpoint size.
pub const AI_GATEWAY_HTTP_MAX_ENDPOINT_BYTES: usize = 2 * 1024;
/// Default connection-establishment timeout.
pub const AI_GATEWAY_HTTP_DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default total request and response-stream timeout.
pub const AI_GATEWAY_HTTP_DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Default maximum simultaneous active requests per transport.
pub const AI_GATEWAY_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS: usize = 16;
/// Hard maximum connection-establishment timeout.
pub const AI_GATEWAY_HTTP_MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Hard maximum total request and response-stream timeout.
pub const AI_GATEWAY_HTTP_MAX_REQUEST_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// Hard maximum simultaneous active requests per transport.
pub const AI_GATEWAY_HTTP_MAX_ACTIVE_REQUESTS: usize = 64;
/// Default size of each response chunk exposed to the codec.
pub const AI_GATEWAY_HTTP_DEFAULT_RESPONSE_CHUNK_BYTES: usize = 64 * 1024;
/// Hard maximum size of each response chunk exposed to the codec.
pub const AI_GATEWAY_HTTP_MAX_RESPONSE_CHUNK_BYTES: usize = 1024 * 1024;

const CODEC_HEADER_NAMES: [&str; 7] = [
    "content-type",
    "ai-gateway-protocol-version",
    "ai-language-model-specification-version",
    "ai-language-model-id",
    "ai-language-model-streaming",
    "x-session-id",
    "x-session-affinity",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EndpointKind {
    Production,
    LoopbackTest,
}

/// An approved AI Gateway HTTP endpoint.
#[derive(Clone, Eq, PartialEq)]
pub struct AiGatewayHttpEndpoint {
    url: Url,
    kind: EndpointKind,
}

impl AiGatewayHttpEndpoint {
    /// Constructs a strict plaintext numeric-loopback endpoint for tests only.
    ///
    /// The URL must use `http`, include an explicit nonzero port and absolute
    /// path, and contain no userinfo, query, or fragment.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when the endpoint is not a canonical 127/8 or
    /// `::1` loopback URL.
    pub fn loopback_http(endpoint: &str) -> Result<Self, AiGatewayHttpConfigError> {
        if endpoint.is_empty()
            || endpoint.len() > AI_GATEWAY_HTTP_MAX_ENDPOINT_BYTES
            || !endpoint.is_ascii()
        {
            return Err(invalid_endpoint());
        }
        let Some(after_scheme) = endpoint.strip_prefix("http://") else {
            return Err(invalid_endpoint());
        };
        let Some((original_authority, _)) = after_scheme.split_once('/') else {
            return Err(invalid_endpoint());
        };
        let socket = original_authority
            .parse::<SocketAddr>()
            .map_err(|_| invalid_endpoint())?;
        if socket.port() == 0 || original_authority != socket.to_string() {
            return Err(invalid_endpoint());
        }
        let url = endpoint.parse::<Url>().map_err(|_| invalid_endpoint())?;
        if url.scheme() != "http"
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
        if address != socket.ip() || url.port_or_known_default() != Some(socket.port()) {
            return Err(invalid_endpoint());
        }
        let is_loopback = match address {
            IpAddr::V4(address) => address.octets()[0] == 127,
            IpAddr::V6(address) => address.is_loopback(),
        };
        if !is_loopback {
            return Err(invalid_endpoint());
        }
        Ok(Self {
            url,
            kind: EndpointKind::LoopbackTest,
        })
    }
}

fn invalid_endpoint() -> AiGatewayHttpConfigError {
    AiGatewayHttpConfigError::new(AiGatewayHttpConfigErrorKind::InvalidEndpoint)
}

impl Default for AiGatewayHttpEndpoint {
    fn default() -> Self {
        Self {
            url: AI_GATEWAY_HTTP_DEFAULT_ENDPOINT
                .parse()
                .expect("the pinned AI Gateway endpoint is valid"),
            kind: EndpointKind::Production,
        }
    }
}

impl fmt::Debug for AiGatewayHttpEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayHttpEndpoint")
            .field("kind", &self.kind)
            .field("url", &"<redacted>")
            .finish()
    }
}

/// Native HTTP request, timeout, and response-chunk bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AiGatewayHttpLimits {
    connect_timeout: Duration,
    request_timeout: Duration,
    max_active_requests: usize,
    max_response_chunk_bytes: usize,
}

impl AiGatewayHttpLimits {
    /// Constructs explicit nonzero limits.
    ///
    /// # Errors
    ///
    /// Returns an error when a value is zero, the connect timeout exceeds five
    /// minutes or the total timeout, the total timeout exceeds one hour, the
    /// active-request count exceeds 64, or the chunk bound exceeds 1 MiB.
    pub fn new(
        connect_timeout: Duration,
        request_timeout: Duration,
        max_active_requests: usize,
        max_response_chunk_bytes: usize,
    ) -> Result<Self, AiGatewayHttpConfigError> {
        if connect_timeout.is_zero()
            || request_timeout.is_zero()
            || connect_timeout > request_timeout
            || connect_timeout > AI_GATEWAY_HTTP_MAX_CONNECT_TIMEOUT
            || request_timeout > AI_GATEWAY_HTTP_MAX_REQUEST_TIMEOUT
            || !(1..=AI_GATEWAY_HTTP_MAX_ACTIVE_REQUESTS).contains(&max_active_requests)
            || !(1..=AI_GATEWAY_HTTP_MAX_RESPONSE_CHUNK_BYTES).contains(&max_response_chunk_bytes)
        {
            return Err(AiGatewayHttpConfigError::new(
                AiGatewayHttpConfigErrorKind::InvalidLimits,
            ));
        }
        Ok(Self {
            connect_timeout,
            request_timeout,
            max_active_requests,
            max_response_chunk_bytes,
        })
    }

    /// Returns the connection-establishment timeout.
    #[must_use]
    pub const fn connect_timeout(self) -> Duration {
        self.connect_timeout
    }

    /// Returns the total request and response-stream timeout.
    #[must_use]
    pub const fn request_timeout(self) -> Duration {
        self.request_timeout
    }

    /// Returns the simultaneous active-request bound.
    #[must_use]
    pub const fn max_active_requests(self) -> usize {
        self.max_active_requests
    }

    /// Returns the response chunk bound.
    #[must_use]
    pub const fn max_response_chunk_bytes(self) -> usize {
        self.max_response_chunk_bytes
    }
}

impl Default for AiGatewayHttpLimits {
    fn default() -> Self {
        Self {
            connect_timeout: AI_GATEWAY_HTTP_DEFAULT_CONNECT_TIMEOUT,
            request_timeout: AI_GATEWAY_HTTP_DEFAULT_REQUEST_TIMEOUT,
            max_active_requests: AI_GATEWAY_HTTP_DEFAULT_MAX_ACTIVE_REQUESTS,
            max_response_chunk_bytes: AI_GATEWAY_HTTP_DEFAULT_RESPONSE_CHUNK_BYTES,
        }
    }
}

/// Tokio-hosted native HTTP implementation of [`AiGatewayTransport`].
///
/// Construction is effect-free and does not require a runtime. The returned
/// startup future and response stream must be polled inside a live Tokio
/// runtime with I/O and time enabled, and that runtime must remain driven
/// through connection teardown. Polling without an active runtime handle
/// returns a fixed transport error.
///
/// # Panics
///
/// Tokio may panic if an active runtime handle lacks its I/O or time driver.
/// With the workspace release profile, such a violated host precondition aborts
/// the process.
pub struct AiGatewayHttpTransport {
    client: Client,
    endpoint: AiGatewayHttpEndpoint,
    authorization: HeaderValue,
    limits: AiGatewayHttpLimits,
    permits: Arc<Semaphore>,
}

impl AiGatewayHttpTransport {
    /// Constructs a production-endpoint transport with default limits.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted error if the backend cannot be initialized.
    pub fn new(token: AiGatewayBearerToken) -> Result<Self, AiGatewayHttpConfigError> {
        Self::with_endpoint_and_limits(
            token,
            AiGatewayHttpEndpoint::default(),
            AiGatewayHttpLimits::default(),
        )
    }

    /// Constructs a transport with an approved endpoint and explicit limits.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted error if the backend cannot be initialized.
    pub fn with_endpoint_and_limits(
        token: AiGatewayBearerToken,
        endpoint: AiGatewayHttpEndpoint,
        limits: AiGatewayHttpLimits,
    ) -> Result<Self, AiGatewayHttpConfigError> {
        let authorization = authorization_value(&token)?;
        drop(token);
        let certificates = root_certificates()?;
        let client = Client::builder()
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
            .timeout(limits.request_timeout)
            .pool_max_idle_per_host(limits.max_active_requests)
            .connection_verbose(false)
            .tls_backend_rustls()
            .tls_sslkeylogfile(false)
            .tls_certs_only(certificates)
            .build()
            .map_err(|_| {
                AiGatewayHttpConfigError::new(AiGatewayHttpConfigErrorKind::ClientInitialization)
            })?;
        Ok(Self {
            client,
            endpoint,
            authorization,
            limits,
            permits: Arc::new(Semaphore::new(limits.max_active_requests)),
        })
    }
}

impl fmt::Debug for AiGatewayHttpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayHttpTransport")
            .field("client", &"<redacted>")
            .field("endpoint", &"<redacted>")
            .field("authorization", &"<redacted>")
            .field("limits", &self.limits)
            .field("permits", &"<redacted>")
            .finish()
    }
}

impl AiGatewayTransport for AiGatewayHttpTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(cancelled_error());
            }
            if tokio::runtime::Handle::try_current().is_err() {
                return Err(runtime_required_error());
            }
            let deadline = Instant::now() + self.limits.request_timeout;
            let permit = acquire_permit(Arc::clone(&self.permits), &cancellation, deadline).await?;
            if cancellation.is_cancelled() {
                return Err(cancelled_error());
            }
            let request = build_http_request(
                self.endpoint.url.clone(),
                self.authorization.clone(),
                request,
            )?;
            if cancellation.is_cancelled() {
                return Err(cancelled_error());
            }
            let send = self.client.execute(request);
            let response = await_startup(send, &cancellation, deadline).await?;
            if cancellation.is_cancelled() {
                return Err(cancelled_error());
            }
            if response.status().as_u16() != 200 {
                return Err(status_error(response.status().as_u16()));
            }
            let stream = HttpResponseStream::new(
                response,
                cancellation,
                self.limits.max_response_chunk_bytes,
                deadline,
                permit,
            );
            Ok(Box::pin(stream) as AiGatewayByteStream)
        })
    }
}

fn build_http_request(
    endpoint: Url,
    authorization: HeaderValue,
    request: AiGatewayTransportRequest,
) -> Result<Request, ProviderError> {
    let (headers, body) = request.into_parts();
    let mut request = Request::new(Method::POST, endpoint);
    {
        let target = request.headers_mut();
        target.insert(AUTHORIZATION, authorization);
        target.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
        target.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let mut seen = [false; CODEC_HEADER_NAMES.len()];
        for header in headers {
            let (name, value) = header.into_parts();
            let name =
                HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid_request_error())?;
            let Some(index) = CODEC_HEADER_NAMES
                .iter()
                .position(|expected| *expected == name.as_str())
            else {
                return Err(invalid_request_error());
            };
            if seen[index] {
                return Err(invalid_request_error());
            }
            seen[index] = true;
            let value = HeaderValue::from_str(&value).map_err(|_| invalid_request_error())?;
            target.insert(name, value);
        }
        if seen.contains(&false) {
            return Err(invalid_request_error());
        }
    }
    *request.body_mut() = Some(body.into());
    Ok(request)
}

async fn acquire_permit(
    permits: Arc<Semaphore>,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<OwnedSemaphorePermit, ProviderError> {
    let mut acquire = Box::pin(permits.acquire_owned());
    let mut cancelled = cancellation.cancelled();
    let mut timeout = deadline_sleep(deadline);
    poll_fn(|context| {
        if Pin::new(&mut cancelled).poll(context).is_ready() {
            return Poll::Ready(Err(cancelled_error()));
        }
        if timeout.as_mut().poll(context).is_ready() {
            return Poll::Ready(Err(transport_error()));
        }
        match acquire.as_mut().poll(context) {
            Poll::Ready(result) => {
                if cancellation.is_cancelled() {
                    Poll::Ready(Err(cancelled_error()))
                } else if deadline <= Instant::now() {
                    Poll::Ready(Err(transport_error()))
                } else {
                    Poll::Ready(result.map_err(|_| protocol_error()))
                }
            }
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

async fn await_startup<F>(
    send: F,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<reqwest::Response, ProviderError>
where
    F: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    let mut send = Box::pin(send);
    let mut cancelled = cancellation.cancelled();
    let mut timeout = deadline_sleep(deadline);
    poll_fn(|context| {
        if Pin::new(&mut cancelled).poll(context).is_ready() {
            return Poll::Ready(Err(cancelled_error()));
        }
        if timeout.as_mut().poll(context).is_ready() {
            return Poll::Ready(Err(transport_error()));
        }
        match send.as_mut().poll(context) {
            Poll::Ready(result) => {
                if cancellation.is_cancelled() {
                    Poll::Ready(Err(cancelled_error()))
                } else if deadline <= Instant::now() {
                    Poll::Ready(Err(transport_error()))
                } else {
                    Poll::Ready(result.map_err(|error| map_reqwest_error(&error)))
                }
            }
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

fn deadline_sleep(deadline: Instant) -> Pin<Box<Sleep>> {
    Box::pin(tokio::time::sleep_until(deadline))
}

type ReqwestChunkFuture = Pin<
    Box<
        dyn Future<Output = (reqwest::Response, Result<Option<Bytes>, reqwest::Error>)>
            + Send
            + 'static,
    >,
>;

struct HttpResponseStream {
    response: Option<reqwest::Response>,
    pending_chunk: Option<ReqwestChunkFuture>,
    remainder: Option<Bytes>,
    cancellation: CancellationToken,
    cancelled: Option<Cancelled>,
    timeout: Option<Pin<Box<Sleep>>>,
    deadline: Instant,
    permit: Option<OwnedSemaphorePermit>,
    chunk_bytes: usize,
    finished: bool,
}

impl HttpResponseStream {
    fn new(
        response: reqwest::Response,
        cancellation: CancellationToken,
        chunk_bytes: usize,
        deadline: Instant,
        permit: OwnedSemaphorePermit,
    ) -> Self {
        Self {
            response: Some(response),
            pending_chunk: None,
            remainder: None,
            cancellation,
            cancelled: None,
            timeout: None,
            deadline,
            permit: Some(permit),
            chunk_bytes,
            finished: false,
        }
    }

    fn clear_waiters(&mut self) {
        self.cancelled = None;
        self.timeout = None;
    }

    fn finish(&mut self) {
        self.finished = true;
        self.clear_waiters();
        self.remainder = None;
        self.response = None;
        self.pending_chunk = None;
        self.permit = None;
    }

    fn take_chunk(&mut self, mut bytes: Bytes) -> Vec<u8> {
        if bytes.len() <= self.chunk_bytes {
            return bytes.to_vec();
        }
        let chunk = bytes.split_to(self.chunk_bytes).to_vec();
        self.remainder = Some(bytes);
        chunk
    }

    fn start_chunk_read(&mut self) {
        let mut response = self
            .response
            .take()
            .expect("an idle response stream owns its response");
        self.pending_chunk = Some(Box::pin(async move {
            let result = response.chunk().await;
            (response, result)
        }));
    }

    fn poll_waiters(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Vec<u8>, ProviderError>>> {
        if self.cancelled.is_none() {
            self.cancelled = Some(self.cancellation.cancelled());
        }
        if Pin::new(self.cancelled.as_mut().expect("waiter exists"))
            .poll(context)
            .is_ready()
        {
            self.finish();
            return Poll::Ready(Some(Err(cancelled_error())));
        }
        if self.timeout.is_none() {
            self.timeout = Some(deadline_sleep(self.deadline));
        }
        if self
            .timeout
            .as_mut()
            .expect("timeout exists")
            .as_mut()
            .poll(context)
            .is_ready()
        {
            self.finish();
            return Poll::Ready(Some(Err(transport_error())));
        }
        if self.cancellation.is_cancelled() {
            self.finish();
            Poll::Ready(Some(Err(cancelled_error())))
        } else if self.deadline <= Instant::now() {
            self.finish();
            Poll::Ready(Some(Err(transport_error())))
        } else {
            Poll::Pending
        }
    }
}

impl fmt::Debug for HttpResponseStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpResponseStream")
            .field("response", &"<redacted>")
            .field("pending_chunk", &"<redacted>")
            .field("has_remainder", &self.remainder.is_some())
            .field("cancellation", &"<redacted>")
            .field("cancelled", &self.cancelled.is_some())
            .field("timeout", &"<redacted>")
            .field("deadline", &"<redacted>")
            .field("permit", &"<redacted>")
            .field("chunk_bytes", &self.chunk_bytes)
            .field("finished", &self.finished)
            .finish()
    }
}

impl Stream for HttpResponseStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }
        if tokio::runtime::Handle::try_current().is_err() {
            self.finish();
            return Poll::Ready(Some(Err(runtime_required_error())));
        }
        if self.cancellation.is_cancelled() {
            self.finish();
            return Poll::Ready(Some(Err(cancelled_error())));
        }
        if self.deadline <= Instant::now() {
            self.finish();
            return Poll::Ready(Some(Err(transport_error())));
        }
        if let Some(bytes) = self.remainder.take() {
            let chunk = self.take_chunk(bytes);
            if self.cancellation.is_cancelled() {
                self.finish();
                return Poll::Ready(Some(Err(cancelled_error())));
            }
            if self.deadline <= Instant::now() {
                self.finish();
                return Poll::Ready(Some(Err(transport_error())));
            }
            self.clear_waiters();
            return Poll::Ready(Some(Ok(chunk)));
        }

        if self.pending_chunk.is_none() {
            self.start_chunk_read();
        }
        let result = self
            .pending_chunk
            .as_mut()
            .expect("unfinished stream has a pending chunk read")
            .as_mut()
            .poll(context);
        if self.cancellation.is_cancelled() {
            self.finish();
            return Poll::Ready(Some(Err(cancelled_error())));
        }
        if self.deadline <= Instant::now() {
            self.finish();
            return Poll::Ready(Some(Err(transport_error())));
        }
        match result {
            Poll::Pending => self.poll_waiters(context),
            Poll::Ready((response, result)) => {
                self.pending_chunk = None;
                self.response = Some(response);
                match result {
                    Ok(None) => {
                        self.finish();
                        Poll::Ready(None)
                    }
                    Ok(Some(bytes)) if bytes.is_empty() => {
                        self.clear_waiters();
                        context.waker().wake_by_ref();
                        Poll::Pending
                    }
                    Ok(Some(bytes)) => {
                        let chunk = self.take_chunk(bytes);
                        self.clear_waiters();
                        Poll::Ready(Some(Ok(chunk)))
                    }
                    Err(error) => {
                        let error = map_reqwest_error(&error);
                        self.finish();
                        Poll::Ready(Some(Err(error)))
                    }
                }
            }
        }
    }
}

fn status_error(status: u16) -> ProviderError {
    match status {
        401 | 403 => ProviderError::new(
            ProviderErrorKind::Authentication,
            "ai_gateway_http_authentication",
            "AI Gateway authentication failed",
            false,
        ),
        429 => ProviderError::new(
            ProviderErrorKind::RateLimited,
            "ai_gateway_http_rate_limited",
            "AI Gateway rate limit reached",
            true,
        ),
        408 | 425 | 500..=599 => unavailable_error(),
        400..=499 => ProviderError::new(
            ProviderErrorKind::InvalidRequest,
            "ai_gateway_http_invalid_request",
            "AI Gateway rejected the request",
            false,
        ),
        300..=399 => ProviderError::new(
            ProviderErrorKind::Protocol,
            "ai_gateway_http_redirect",
            "AI Gateway returned a redirect",
            false,
        ),
        _ => ProviderError::new(
            ProviderErrorKind::Protocol,
            "ai_gateway_http_unexpected_status",
            "AI Gateway returned an unexpected status",
            false,
        ),
    }
}

fn map_reqwest_error(error: &reqwest::Error) -> ProviderError {
    if error_chain_contains_tls(error) {
        return ProviderError::new(
            ProviderErrorKind::Transport,
            "ai_gateway_http_tls",
            "AI Gateway TLS transport failed",
            false,
        );
    }
    if error.is_timeout()
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
        protocol_error()
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

fn protocol_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Protocol,
        "ai_gateway_http_protocol",
        "AI Gateway HTTP protocol failed",
        false,
    )
}

fn invalid_request_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::InvalidRequest,
        "ai_gateway_http_invalid_request",
        "AI Gateway HTTP request is invalid",
        false,
    )
}

fn unavailable_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Unavailable,
        "ai_gateway_http_unavailable",
        "AI Gateway is unavailable",
        true,
    )
}

fn transport_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Transport,
        "ai_gateway_http_transport",
        "AI Gateway transport failed",
        true,
    )
}

fn runtime_required_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Transport,
        "ai_gateway_http_runtime_required",
        "AI Gateway HTTP transport requires an active Tokio runtime",
        false,
    )
}

fn cancelled_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Cancelled,
        "ai_gateway_http_cancelled",
        "AI Gateway HTTP request cancelled",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{AiGatewayBearerToken, AiGatewayHttpTransport, root_certificates};

    #[test]
    fn client_construction_is_effect_free_and_needs_no_runtime() {
        assert!(!root_certificates().unwrap().is_empty());
        AiGatewayHttpTransport::new(AiGatewayBearerToken::new("test-token").unwrap()).unwrap();
    }
}
