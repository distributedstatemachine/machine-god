use super::{
    Arc, BoxFuture, CancellationToken, Duration, Future, Instant, McpCatalogKind, McpCatalogLimits,
    McpHttpClock, McpHttpConnection, McpHttpControl, McpHttpError, McpHttpLimits, McpHttpPeer,
    McpHttpPeerError, McpHttpPeerFrame, McpHttpSessionTeardown, McpRawCatalog, McpSubmission,
    McpSubmissionHttpHead, ProtocolVersion, Result, RpcId, RpcKind, TransportKind, head, stream,
};
use crate::mcp::{
    http::McpHttpResponse,
    pagination::McpCatalogBuilder,
    sse::{SseEvent, SseMode},
};
use std::task::Poll;
mod exchange;
mod idle;
pub(super) use exchange::{exchange, notification};
pub(super) use idle::next_notification;

/// A polled operation abandoned at any await retires its peer and listener.
pub(super) struct Operation<'a> {
    pub peer: &'a mut McpHttpPeer,
    pub settled: bool,
}
impl<'a> Operation<'a> {
    pub fn begin(peer: &'a mut McpHttpPeer) -> Self {
        // Limits bound one caller-driven operation, not the useful lifetime of
        // a server that has already released earlier event/reconnect state.
        peer.operation_events = 0;
        peer.listener_reconnects = 0;
        Self {
            peer,
            settled: false,
        }
    }
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.peer.response_limits = super::WireLimits::default();
        self.peer.feature_authority = None;
        if !self.settled {
            self.peer.close();
        }
    }
}

pub(super) async fn bounded<T>(
    future: impl Future<Output = T>,
    clock: &dyn McpHttpClock,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<T> {
    let mut future = std::pin::pin!(future);
    let mut timeout = clock.sleep_until(deadline);
    let mut cancelled = std::pin::pin!(cancellation.cancelled());
    std::future::poll_fn(|cx| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(McpHttpPeerError::Cancelled));
        }
        if clock.now() >= deadline || timeout.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(McpHttpPeerError::Deadline));
        }
        let result = future.as_mut().poll(cx);
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(McpHttpPeerError::Cancelled));
        }
        if clock.now() >= deadline {
            return Poll::Ready(Err(McpHttpPeerError::Deadline));
        }
        result.map(Ok)
    })
    .await
}

// Unlike an application exchange, an idle listener read remains owned after
// its observer times out. Never discard a Ready reader at the post-poll time
// check: the caller first retains its exact framing/event state, then checks.
async fn observe_listener(
    read: &mut stream::Read,
    clock: &dyn McpHttpClock,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(Box<stream::Reader>, Result<Option<SseEvent>>)> {
    let mut timeout = clock.sleep_until(deadline);
    let mut cancelled = std::pin::pin!(cancellation.cancelled());
    std::future::poll_fn(|cx| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(McpHttpPeerError::Cancelled));
        }
        if clock.now() >= deadline || timeout.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(McpHttpPeerError::Deadline));
        }
        read.as_mut().poll(cx).map(Ok)
    })
    .await
}

pub(super) fn request(
    id: &RpcId,
    method: &str,
    mut params: serde_json::Value,
    version: ProtocolVersion,
) -> Result<McpHttpControl> {
    let RpcId::Integer(id) = id else {
        return Err(McpHttpPeerError::Correlation);
    };
    if version == ProtocolVersion::Modern {
        params["_meta"] = serde_json::json!({"io.modelcontextprotocol/protocolVersion":version.as_str(),"io.modelcontextprotocol/clientInfo":{"name":"machine-god","version":env!("CARGO_PKG_VERSION")},"io.modelcontextprotocol/clientCapabilities":{}});
    }
    let bytes = serde_json::to_vec(
        &serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
    )
    .map_err(|_| McpHttpPeerError::Invalid)?;
    Ok(McpHttpControl::discovery(&bytes)?)
}

pub(super) async fn call(
    peer: &mut McpHttpPeer,
    submission: McpSubmission,
    projected: Arc<McpSubmissionHttpHead>,
    deadline: Instant,
) -> Result<McpHttpPeerFrame> {
    peer.check(deadline)?;
    if submission
        .http_head()
        .is_some_and(|retained| !Arc::ptr_eq(&retained, &projected))
    {
        return Err(McpHttpPeerError::Invalid);
    }
    if !peer
        .reserved
        .matches(submission.rpc_id(), submission.tool_reservation())
    {
        return Err(McpHttpPeerError::Correlation);
    }
    let runtime = peer
        .runtimes
        .iter()
        .find(|runtime| submission.belongs_to_runtime(runtime))
        .cloned()
        .ok_or(McpHttpPeerError::Invalid)?;
    validate_projected_head(peer, &projected)?;
    let id = peer
        .reserved
        .take(submission.rpc_id(), submission.tool_reservation())
        .ok_or(McpHttpPeerError::Correlation)?;
    let mut operation = Operation::begin(peer);
    let mut cancelled = submission.cancelled_owned();
    let connection = operation.peer.connection(projected, deadline)?;
    let observation = connection.observation();
    let writer = connection.submit(submission, runtime);
    let response = {
        let mut response = std::pin::pin!(exchange(
            operation.peer,
            writer,
            &id,
            false,
            false,
            deadline
        ));
        std::future::poll_fn(|cx| {
            if cancelled.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(McpHttpPeerError::Cancelled));
            }
            let result = response.as_mut().poll(cx);
            if cancelled.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(McpHttpPeerError::Cancelled));
            }
            result
        })
        .await
    };
    if observation.was_attempted()
        && matches!(
            response,
            Err(McpHttpPeerError::Cancelled
                | McpHttpPeerError::Deadline
                | McpHttpPeerError::Transport(McpHttpError::Cancelled | McpHttpError::Deadline))
        )
    {
        cancel_request(operation.peer, &id).await;
    }
    let response = response?;
    let frame = response.frame.ok_or(McpHttpPeerError::Protocol)?;
    operation.settled = true;
    Ok(frame)
}

async fn cancel_request(peer: &mut McpHttpPeer, id: &RpcId) {
    if peer.protocol.version == ProtocolVersion::Modern {
        return;
    }
    let Some(deadline) = peer
        .options
        .clock
        .now()
        .checked_add(Duration::from_millis(100))
    else {
        return;
    };
    let deadline = peer.options.lifetime.constrain(deadline);
    let id = match id {
        RpcId::Integer(id) => serde_json::Value::from(*id),
        RpcId::String(id) => serde_json::Value::String(id.clone()),
        RpcId::Null => return,
    };
    let Ok(bytes) = serde_json::to_vec(
        &serde_json::json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"local operation ended"}}),
    ) else {
        return;
    };
    let Ok(control) = McpHttpControl::notification(&bytes) else {
        return;
    };
    let _ = notification(peer, control, deadline).await;
}

fn validate_projected_head(peer: &McpHttpPeer, projected: &McpSubmissionHttpHead) -> Result<()> {
    let base = peer.request_head()?;
    if projected.endpoint() != base.endpoint() {
        return Err(McpHttpPeerError::Invalid);
    }
    let supplied: Vec<_> = projected.headers().collect();
    let expected: Vec<_> = base.headers().collect();
    for &(name, value) in &expected {
        if !supplied.contains(&(name, value)) {
            return Err(McpHttpPeerError::Invalid);
        }
    }
    for &(name, value) in &supplied {
        if expected.iter().any(|candidate| candidate.0 == name) {
            continue;
        }
        if peer.protocol.version != ProtocolVersion::Modern
            || !(name == "mcp-method" && value == b"tools/call"
                || name == "mcp-name"
                || name.starts_with("mcp-param-"))
        {
            return Err(McpHttpPeerError::Invalid);
        }
    }
    if peer.protocol.version == ProtocolVersion::Modern
        && !supplied.contains(&("mcp-method", b"tools/call".as_slice()))
    {
        return Err(McpHttpPeerError::Invalid);
    }
    Ok(())
}

pub(super) async fn catalog(
    peer: &mut McpHttpPeer,
    kind: McpCatalogKind,
    limits: McpCatalogLimits,
    epoch: Instant,
    deadline: Instant,
) -> Result<McpRawCatalog> {
    peer.available()?;
    peer.check(deadline)?;
    if epoch > peer.options.clock.now()
        || matches!(
            kind,
            McpCatalogKind::Resources | McpCatalogKind::ResourceTemplates
        ) && !peer.capabilities.resources()
        || kind == McpCatalogKind::Prompts && !peer.capabilities.prompts()
    {
        return Err(McpHttpPeerError::Invalid);
    }
    let mut operation = Operation::begin(peer);
    let peer = &mut *operation.peer;
    let mut builder = McpCatalogBuilder::new(kind, peer.protocol.version, limits)
        .map_err(|_| McpHttpPeerError::Limit)?;
    loop {
        let cursor = builder.next_cursor().map(str::to_owned);
        let id = peer.allocate()?;
        let params = cursor.as_ref().map_or_else(
            || serde_json::json!({}),
            |cursor| serde_json::json!({"cursor":cursor}),
        );
        let control = request(&id, kind.method(), params, peer.protocol.version)?;
        let head = Arc::new(peer.make_head(Some(kind.method()), None)?);
        let writer = peer.connection(head, deadline)?.control(control);
        let frame = exchange(peer, writer, &id, false, false, deadline)
            .await?
            .frame
            .ok_or(McpHttpPeerError::Protocol)?;
        let received_at = u64::try_from(
            peer.options
                .clock
                .now()
                .saturating_duration_since(epoch)
                .as_millis(),
        )
        .map_err(|_| McpHttpPeerError::Limit)?;
        if !builder
            .append_response(frame.bytes(), &id, cursor.as_deref(), received_at)
            .map_err(|_| McpHttpPeerError::Protocol)?
        {
            let catalog = builder.finish().map_err(|_| McpHttpPeerError::Protocol)?;
            operation.settled = true;
            return Ok(catalog);
        }
    }
}

pub(super) async fn open_listener(peer: &mut McpHttpPeer, deadline: Instant) -> Result<()> {
    peer.listener = Some(prepare_listener(peer, deadline, 0)?.await?.next());
    // Hints are observations of this new stream, not inherited event fields.
    peer.listener_resume = stream::Resume::default();
    Ok(())
}

fn prepare_listener(
    peer: &McpHttpPeer,
    deadline: Instant,
    retry_ms: u32,
) -> Result<BoxFuture<'static, Result<Box<stream::Reader>>>> {
    let head = Arc::new(peer.make_head(None, peer.listener_resume.id.as_deref())?);
    let connection = peer.connection(head, deadline)?;
    let clock = peer.options.clock.clone();
    let cancellation = peer.cancellation.clone();
    let session = peer.session.clone();
    let lifetime = peer.options.lifetime;
    let deadline = lifetime.constrain(deadline);
    let wake = clock
        .now()
        .checked_add(Duration::from_millis(u64::from(retry_ms)))
        .ok_or(McpHttpPeerError::Limit)?
        .min(deadline);
    Ok(Box::pin(async move {
        if retry_ms != 0 {
            bounded(clock.sleep_until(wake), &*clock, &cancellation, deadline).await?;
        }
        let mut response = bounded(
            connection.control(McpHttpControl::listen()),
            &*clock,
            &cancellation,
            deadline,
        )
        .await??;
        head::status(&response, session.is_some())?;
        if response.status == 405 {
            return Err(McpHttpPeerError::ListenerUnsupported);
        }
        head::stable_session(&response.headers, session.as_deref())?;
        if response.status != 200 || head::media(&response.headers)? != head::Media::Sse {
            return Err(McpHttpPeerError::Protocol);
        }
        response.body.promote_listener(lifetime)?;
        stream::Reader::new(response.body, SseMode::Legacy, stream::listener_limits())
    }))
}

pub(super) async fn start_listener(peer: &mut McpHttpPeer, deadline: Instant) -> Result<()> {
    peer.available()?;
    peer.check(deadline)?;
    if peer.listener.is_some() {
        return Ok(());
    }
    if peer.protocol.version == ProtocolVersion::Modern {
        return Err(McpHttpPeerError::ListenerUnsupported);
    }
    if peer.protocol.transport == TransportKind::LegacySse {
        return Err(McpHttpPeerError::Closed);
    }
    let mut operation = Operation::begin(peer);
    let result = open_listener(operation.peer, deadline).await;
    operation.settled =
        result.is_ok() || matches!(result, Err(McpHttpPeerError::ListenerUnsupported));
    result
}

pub(super) fn charge_event(peer: &mut McpHttpPeer) -> Result<()> {
    peer.operation_events += 1;
    if peer.operation_events > 4096 {
        return Err(McpHttpPeerError::Limit);
    }
    Ok(())
}
pub(super) fn route_listener(
    peer: &mut McpHttpPeer,
    event: &SseEvent,
    expected: Option<&RpcId>,
) -> Result<Option<McpHttpPeerFrame>> {
    charge_event(peer)?;
    if peer.protocol.transport == TransportKind::LegacySse {
        match event.event().unwrap_or("message") {
            "endpoint" => return Err(McpHttpPeerError::Protocol),
            "message" => {}
            _ => return Ok(None),
        }
    }
    if event.data().is_empty() {
        peer.listener_resume.commit(event)?;
        return Ok(None);
    }
    let frame =
        McpHttpPeerFrame::parse_with_limits(event.data().as_bytes().into(), peer.response_limits)?;
    let result = match frame.envelope.kind() {
        RpcKind::Notification => {
            peer.retain(frame)?;
            None
        }
        RpcKind::Success | RpcKind::Error if expected.is_some() => {
            frame
                .envelope
                .correlate(expected.unwrap(), false)
                .map_err(|_| McpHttpPeerError::Correlation)?;
            Some(frame)
        }
        // A notification-only Streamable listener does not route foreign replies.
        RpcKind::Success | RpcKind::Error
            if peer.protocol.transport == TransportKind::StreamableHttp =>
        {
            None
        }
        _ => return Err(McpHttpPeerError::Protocol),
    };
    peer.listener_resume.commit(event)?;
    Ok(result)
}
pub(super) async fn reconnect_listener(peer: &mut McpHttpPeer, deadline: Instant) -> Result<()> {
    if peer.protocol.transport != TransportKind::StreamableHttp || peer.listener_reconnects >= 32 {
        return Err(McpHttpPeerError::Closed);
    }
    peer.listener_reconnects += 1;
    delay(peer, peer.listener_resume.retry_ms, deadline).await?;
    open_listener(peer, deadline).await
}
pub(super) async fn delay(peer: &mut McpHttpPeer, millis: u32, deadline: Instant) -> Result<()> {
    let deadline = peer.options.lifetime.constrain(deadline);
    peer.check(deadline)?;
    let wake = peer
        .options
        .clock
        .now()
        .checked_add(Duration::from_millis(u64::from(millis)))
        .ok_or(McpHttpPeerError::Limit)?
        .min(deadline);
    bounded(
        peer.options.clock.sleep_until(wake),
        &*peer.options.clock,
        &peer.cancellation,
        deadline,
    )
    .await?;
    peer.check(deadline)
}

pub(super) async fn shutdown(peer: &mut McpHttpPeer, deadline: Instant) -> McpHttpSessionTeardown {
    if peer.closed {
        return if peer.session.take().is_some() {
            McpHttpSessionTeardown::NotAttempted
        } else {
            McpHttpSessionTeardown::NotNeeded
        };
    }
    if peer.session.is_none() {
        peer.close();
        return McpHttpSessionTeardown::NotNeeded;
    }
    let result = (|| {
        let head = Arc::new(peer.make_head(None, None)?);
        let connection = McpHttpConnection::from_prepared_head(
            peer.destination.clone(),
            head,
            peer.options.trust.clone(),
            McpHttpLimits::default(),
            CancellationToken::new(),
            deadline,
            peer.options.clock.clone(),
        )?;
        peer.completion
            .0
            .exchanges
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(connection.observation());
        Ok::<_, McpHttpPeerError>(connection)
    })();
    peer.session = None;
    peer.close();
    let Ok(connection) = result else {
        return McpHttpSessionTeardown::NotAttempted;
    };
    let observation = connection.observation();
    match connection
        .control(McpHttpControl::terminate_session())
        .await
    {
        Ok(response) if matches!(response.status, 200 | 204) => McpHttpSessionTeardown::Confirmed,
        Ok(response) if response.status == 405 => McpHttpSessionTeardown::Unsupported,
        _ if !observation.was_attempted() => McpHttpSessionTeardown::NotAttempted,
        _ => McpHttpSessionTeardown::Ambiguous,
    }
}
