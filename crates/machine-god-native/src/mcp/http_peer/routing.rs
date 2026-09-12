use super::{
    Arc, BoxFuture, CancellationToken, Future, Instant, McpCatalogKind, McpCatalogLimits,
    McpHttpClock, McpHttpControl, McpHttpPeer, McpHttpPeerError, McpHttpPeerFrame, McpRawCatalog,
    McpSubmission, McpSubmissionHttpHead, ProtocolVersion, Result, RpcId, RpcKind, head, stream,
};
use crate::mcp::{
    http::{McpHttpError, McpHttpResponse},
    pagination::McpCatalogBuilder,
};
use std::task::Poll;
mod exchange;
pub(super) use exchange::exchange;

/// A polled operation abandoned at any await retires its peer.
pub(super) struct Operation<'a> {
    pub peer: &'a mut McpHttpPeer,
    pub settled: bool,
}
impl<'a> Operation<'a> {
    pub fn begin(peer: &'a mut McpHttpPeer) -> Self {
        // Limits bound one caller-driven operation, not the useful lifetime of
        // a server that has already released earlier event/reconnect state.
        peer.operation_events = 0;
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

pub(super) fn request(
    id: &RpcId,
    method: &str,
    mut params: serde_json::Value,
    version: ProtocolVersion,
) -> Result<McpHttpControl> {
    let RpcId::Integer(id) = id else {
        return Err(McpHttpPeerError::Correlation);
    };
    params["_meta"] = serde_json::json!({"io.modelcontextprotocol/protocolVersion":version.as_str(),"io.modelcontextprotocol/clientInfo":{"name":"machine-god","version":env!("CARGO_PKG_VERSION")},"io.modelcontextprotocol/clientCapabilities":{}});
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
    let writer = connection.submit(submission, runtime);
    let response = {
        let mut response = std::pin::pin!(exchange(operation.peer, writer, &id, false, deadline));
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
    let response = response?;
    let frame = response.frame;
    operation.settled = true;
    Ok(frame)
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
        if !(name == "mcp-method" && value == b"tools/call"
            || name == "mcp-name"
            || name.starts_with("mcp-param-"))
        {
            return Err(McpHttpPeerError::Invalid);
        }
    }
    if !supplied.contains(&("mcp-method", b"tools/call".as_slice())) {
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
        let head = Arc::new(peer.make_head(Some(kind.method()))?);
        let writer = peer.connection(head, deadline)?.control(control);
        let frame = exchange(peer, writer, &id, false, deadline).await?.frame;
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
            peer.check(deadline)?;
            operation.settled = true;
            return Ok(catalog);
        }
    }
}

pub(super) fn charge_event(peer: &mut McpHttpPeer) -> Result<()> {
    peer.operation_events += 1;
    if peer.operation_events > 4096 {
        return Err(McpHttpPeerError::Limit);
    }
    Ok(())
}
