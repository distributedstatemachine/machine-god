use super::{
    McpHttpConnection, McpHttpControl, McpHttpDestination, McpHttpError, McpHttpObservation,
    McpHttpResponse, McpHttpTrust, McpSubmission, McpSubmissionRuntime, Result,
};
use crate::mcp::{lifetime::McpPeerLifetime, submission::McpSubmissionWriter};
use futures_util::future::poll_fn;
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Instant,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};

pub(super) const IO_BYTES: usize = 16 * 1024;
#[cfg(test)]
#[path = "io_feature_tests.rs"]
mod feature_tests;
#[derive(Default)]
pub(super) struct Observation {
    pub attempted: AtomicBool,
    pub acknowledged: AtomicUsize,
    pub complete: CancellationToken,
}
pub(super) struct Completion(pub McpHttpObservation);
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.0.complete.cancel();
    }
}

pub(super) struct Lifetime {
    cancellation: CancellationToken,
    deadline: Instant,
    tool: Option<BoxFuture<'static, ()>>,
    clock: Arc<dyn super::McpHttpClock>,
    feature: Option<crate::mcp::control::McpFeatureControlAuthority>,
    listener: bool,
    listener_policy: Option<McpPeerLifetime>,
}
impl Lifetime {
    pub fn new(
        cancellation: CancellationToken,
        deadline: Instant,
        clock: Arc<dyn super::McpHttpClock>,
    ) -> Self {
        Self {
            cancellation,
            deadline,
            tool: None,
            clock,
            feature: None,
            listener: false,
            listener_policy: None,
        }
    }
    fn check(&mut self, cx: &mut Context<'_>) -> Result<()> {
        if self.cancellation.is_cancelled()
            || self.feature.as_ref().is_some_and(|guard| !guard.is_live())
            || self
                .tool
                .as_mut()
                .is_some_and(|future| future.as_mut().poll(cx).is_ready())
        {
            return Err(McpHttpError::Cancelled);
        }
        if self
            .read_deadline()
            .is_some_and(|deadline| self.clock.now() >= deadline)
        {
            return Err(McpHttpError::Deadline);
        }
        Ok(())
    }
    fn read_deadline(&self) -> Option<Instant> {
        self.listener_policy
            .map_or(Some(self.deadline), McpPeerLifetime::deadline)
    }
    pub(super) fn guard_feature(&mut self, guard: crate::mcp::control::McpFeatureControlAuthority) {
        self.tool = Some(guard.cancelled());
        self.feature = Some(guard);
    }
    pub async fn wait<T>(&mut self, future: impl Future<Output = Result<T>>) -> Result<T> {
        let mut future = std::pin::pin!(future);
        let mut cancelled = std::pin::pin!(self.cancellation.cancelled());
        let clock = self.clock.clone();
        let mut timeout = self
            .read_deadline()
            .map(|deadline| clock.sleep_until(deadline));
        poll_fn(|cx| {
            self.check(cx)?;
            if cancelled.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(McpHttpError::Cancelled));
            }
            if timeout
                .as_mut()
                .is_some_and(|future| future.as_mut().poll(cx).is_ready())
            {
                return Poll::Ready(Err(McpHttpError::Deadline));
            }
            let result = future.as_mut().poll(cx);
            self.check(cx)?;
            result
        })
        .await
    }
}

enum Stream {
    Plain(tokio::net::TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>),
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Plain(value) => Pin::new(value).poll_read(cx, buf),
            Self::Tls(value) => Pin::new(&mut **value).poll_read(cx, buf),
        }
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut *self {
            Self::Plain(value) => Pin::new(value).poll_write(cx, buf),
            Self::Tls(value) => Pin::new(&mut **value).poll_write(cx, buf),
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Plain(value) => Pin::new(value).poll_flush(cx),
            Self::Tls(value) => Pin::new(&mut **value).poll_flush(cx),
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Plain(value) => Pin::new(value).poll_shutdown(cx),
            Self::Tls(value) => Pin::new(&mut **value).poll_shutdown(cx),
        }
    }
}

async fn connect(
    destination: &McpHttpDestination,
    trust: Option<McpHttpTrust>,
    lifetime: &mut Lifetime,
) -> Result<Stream> {
    let mut selected = None;
    for address in &destination.addresses {
        match lifetime
            .wait(async {
                tokio::net::TcpStream::connect(address)
                    .await
                    .map_err(|_| McpHttpError::Connect)
            })
            .await
        {
            Ok(socket) => {
                selected = Some(socket);
                break;
            }
            Err(McpHttpError::Connect) => {}
            Err(error) => return Err(error),
        }
    }
    let socket = selected.ok_or(McpHttpError::Connect)?;
    socket
        .set_nodelay(true)
        .map_err(|_| McpHttpError::Connect)?;
    let Some(trust) = trust else {
        return Ok(Stream::Plain(socket));
    };
    let name = match destination.endpoint().host() {
        url::Host::Domain(value) => rustls::pki_types::ServerName::try_from(value.to_owned())
            .map_err(|_| McpHttpError::Invalid)?,
        url::Host::Ipv4(value) => rustls::pki_types::ServerName::IpAddress(value.into()),
        url::Host::Ipv6(value) => rustls::pki_types::ServerName::IpAddress(value.into()),
    };
    let tls = lifetime
        .wait(async {
            tokio_rustls::TlsConnector::from(trust.0)
                .connect_with(name, socket, |connection| {
                    connection.set_buffer_limit(Some(32 * 1024));
                })
                .await
                .map_err(|_| McpHttpError::Tls)
        })
        .await?;
    if tls
        .get_ref()
        .1
        .alpn_protocol()
        .is_some_and(|value| value != b"http/1.1")
    {
        return Err(McpHttpError::Tls);
    }
    Ok(Stream::Tls(Box::new(tls)))
}

struct Writer<'a> {
    stream: &'a mut Stream,
    observation: &'a McpHttpObservation,
    cancellation: CancellationToken,
    deadline: Instant,
    clock: Arc<dyn super::McpHttpClock>,
    feature: Option<crate::mcp::control::McpFeatureControlAuthority>,
}
impl McpSubmissionWriter for Writer<'_> {
    fn poll_write(&mut self, cx: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        if self.cancellation.is_cancelled()
            || self.clock.now() >= self.deadline
            || self.feature.as_ref().is_some_and(|guard| !guard.is_live())
        {
            return Poll::Ready(Err(io::Error::other("MCP HTTP writer lifetime expired")));
        }
        self.observation.0.attempted.store(true, Ordering::Release);
        let result = Pin::new(&mut *self.stream).poll_write(cx, bytes);
        if let Poll::Ready(Ok(count)) = result {
            self.observation
                .0
                .acknowledged
                .fetch_add(count, Ordering::AcqRel);
        }
        result
    }
    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.cancellation.is_cancelled()
            || self.clock.now() >= self.deadline
            || self.feature.as_ref().is_some_and(|guard| !guard.is_live())
        {
            return Poll::Ready(Err(io::Error::other("MCP HTTP writer lifetime expired")));
        }
        self.observation.0.attempted.store(true, Ordering::Release);
        Pin::new(&mut *self.stream).poll_flush(cx)
    }
}

pub(super) async fn submit(
    connection: McpHttpConnection,
    submission: McpSubmission,
    runtime: Arc<McpSubmissionRuntime>,
) -> Result<McpHttpResponse> {
    if !submission.belongs_to_runtime(&runtime) {
        return Err(McpHttpError::Submission);
    }
    let bytes = submission
        .http_request_bytes()
        .map_err(|_| McpHttpError::Submission)?;
    let split = memchr::memmem::find(bytes, b"\r\n\r\n").ok_or(McpHttpError::Submission)? + 4;
    if connection
        .head
        .encode(&bytes[split..])
        .map_err(|_| McpHttpError::Submission)?
        .as_ref()
        != bytes
    {
        return Err(McpHttpError::Submission);
    }
    let bytes = bytes.to_vec();
    let McpHttpConnection {
        destination,
        trust,
        limits,
        mut lifetime,
        observation,
        completion,
        ..
    } = connection;
    lifetime.tool = Some(submission.cancelled_owned());
    let mut stream = connect(&destination, trust, &mut lifetime).await?;
    {
        let writer = Writer {
            feature: None,
            stream: &mut stream,
            observation: &observation,
            cancellation: lifetime.cancellation.clone(),
            deadline: lifetime.deadline,
            clock: lifetime.clock.clone(),
        };
        let mut driver = submission
            .into_http_driver(writer)
            .map_err(|_| McpHttpError::Submission)?;
        let mut offset = 0;
        while offset < bytes.len() {
            let end = (offset + IO_BYTES).min(bytes.len());
            offset += lifetime
                .wait(poll_fn(|cx| {
                    driver
                        .poll_write(cx, &bytes[offset..end])
                        .map_err(|_| McpHttpError::Submission)
                }))
                .await?;
            tokio::task::yield_now().await;
        }
        lifetime
            .wait(poll_fn(|cx| {
                driver.poll_flush(cx).map_err(|_| McpHttpError::Submission)
            }))
            .await?;
    }
    drop(bytes);
    super::response::receive(
        Buffered::new(stream, lifetime, completion, limits.wire_bytes),
        limits,
    )
    .await
}

pub(super) async fn control(
    connection: McpHttpConnection,
    control: McpHttpControl,
) -> Result<McpHttpResponse> {
    let feature = control.feature_guard();
    if feature.as_ref().is_some_and(|guard| !guard.is_live()) {
        return Err(McpHttpError::Cancelled);
    }
    let bytes = control.encode(&connection.head)?;
    let McpHttpConnection {
        destination,
        trust,
        limits,
        mut lifetime,
        observation,
        completion,
        ..
    } = connection;
    lifetime.listener = control.is_listener();
    if let Some(feature) = feature {
        lifetime.guard_feature(feature);
    }
    let mut stream = connect(&destination, trust, &mut lifetime).await?;
    let mut writer = Writer {
        feature: lifetime.feature.clone(),
        stream: &mut stream,
        observation: &observation,
        cancellation: lifetime.cancellation.clone(),
        deadline: lifetime.deadline,
        clock: lifetime.clock.clone(),
    };
    let mut offset = 0;
    while offset < bytes.len() {
        let end = (offset + IO_BYTES).min(bytes.len());
        let count = lifetime
            .wait(poll_fn(|cx| {
                writer
                    .poll_write(cx, &bytes[offset..end])
                    .map_err(|_| McpHttpError::Io)
            }))
            .await?;
        if count == 0 {
            return Err(McpHttpError::Io);
        }
        offset += count;
        tokio::task::yield_now().await;
    }
    lifetime
        .wait(poll_fn(|cx| {
            writer.poll_flush(cx).map_err(|_| McpHttpError::Io)
        }))
        .await?;
    drop(bytes);
    super::response::receive(
        Buffered::new(stream, lifetime, completion, limits.wire_bytes),
        limits,
    )
    .await
}

/// Field order releases the socket before publishing completion.
pub(super) struct Buffered {
    stream: Stream,
    _completion: Completion,
    lifetime: Lifetime,
    bytes: Box<[u8; IO_BYTES]>,
    start: usize,
    end: usize,
    remaining_wire: u64,
}
impl Buffered {
    pub(super) fn is_listener(&self) -> bool {
        self.lifetime.listener
    }
    pub(super) fn promote_listener(&mut self, lifetime: McpPeerLifetime) -> Result<()> {
        if !self.lifetime.listener
            || self.lifetime.tool.is_some() && self.lifetime.feature.is_none()
            || self.lifetime.listener_policy.is_some()
        {
            return Err(McpHttpError::Invalid);
        }
        if self.lifetime.cancellation.is_cancelled()
            || self
                .lifetime
                .feature
                .as_ref()
                .is_some_and(|guard| !guard.is_live())
        {
            return Err(McpHttpError::Cancelled);
        }
        let now = self.lifetime.clock.now();
        if now >= self.lifetime.deadline || lifetime.is_expired(now) {
            return Err(McpHttpError::Deadline);
        }
        self.lifetime.listener_policy = Some(lifetime);
        // Only the typed GET listener is transferred, never the application
        // response it may help carry. Its acquisition guard remains live through
        // this point; later reads belong to the peer rather than that old turn.
        self.lifetime.feature = None;
        self.lifetime.tool = None;
        Ok(())
    }
    fn new(
        stream: Stream,
        lifetime: Lifetime,
        completion: Completion,
        remaining_wire: u64,
    ) -> Self {
        Self {
            stream,
            _completion: completion,
            lifetime,
            bytes: Box::new([0; IO_BYTES]),
            start: 0,
            end: 0,
            remaining_wire,
        }
    }
    pub async fn take(&mut self, maximum: usize) -> Result<Box<[u8]>> {
        self.ready().await?;
        let count = maximum.min(self.end - self.start);
        let bytes = self.bytes[self.start..self.start + count].into();
        self.start += count;
        Ok(bytes)
    }
    async fn ready(&mut self) -> Result<()> {
        self.lifetime.wait(async { Ok(()) }).await?;
        if self.start < self.end {
            return Ok(());
        }
        let capacity = usize::try_from(self.remaining_wire.min(IO_BYTES as u64 - 1) + 1)
            .map_err(|_| McpHttpError::Limit)?;
        let count = self
            .lifetime
            .wait(async {
                self.stream
                    .read(&mut self.bytes[..capacity])
                    .await
                    .map_err(|_| McpHttpError::Io)
            })
            .await?;
        self.remaining_wire = self
            .remaining_wire
            .checked_sub(count as u64)
            .ok_or(McpHttpError::Limit)?;
        self.start = 0;
        self.end = count;
        Ok(())
    }
    pub async fn line(&mut self, maximum: usize) -> Result<Vec<u8>> {
        let mut line = Vec::new();
        loop {
            self.ready().await?;
            if self.start == self.end {
                return Err(McpHttpError::Protocol);
            }
            let remaining = &self.bytes[self.start..self.end];
            let count = memchr::memchr(b'\n', remaining).map_or(remaining.len(), |at| at + 1);
            if count > maximum.saturating_sub(line.len()) {
                return Err(McpHttpError::Limit);
            }
            line.extend_from_slice(&remaining[..count]);
            self.start += count;
            if line.last() == Some(&b'\n') {
                if !line.ends_with(b"\r\n")
                    || line[..line.len() - 2].iter().any(|byte| {
                        *byte == b'\r' || (*byte < 0x20 && *byte != b'\t') || *byte == 0x7f
                    })
                {
                    return Err(McpHttpError::Protocol);
                }
                return Ok(line);
            }
            tokio::task::yield_now().await;
        }
    }
}
