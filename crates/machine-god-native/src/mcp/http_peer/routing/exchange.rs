use super::{
    BoxFuture, Instant, McpHttpError, McpHttpPeer, McpHttpPeerError, McpHttpPeerFrame,
    McpHttpResponse, Result, RpcId, RpcKind, bounded, charge_event, head, stream,
};
type Write = BoxFuture<'static, std::result::Result<McpHttpResponse, McpHttpError>>;
pub(in crate::mcp::http_peer) struct Received {
    pub frame: McpHttpPeerFrame,
    pub status: u16,
}
pub(in crate::mcp::http_peer) async fn exchange(
    peer: &mut McpHttpPeer,
    writer: Write,
    expected: &RpcId,
    discovery: bool,
    deadline: Instant,
) -> Result<Received> {
    let response = bounded(writer, &*peer.options.clock, &peer.cancellation, deadline).await??;
    peer.check(deadline)?;
    head::status(&response)?;
    if response.status != 200 && !(discovery && response.status == 400) {
        return Err(McpHttpPeerError::Protocol);
    }
    // Modern HTTP is stateless; a legacy session cannot become request authority.
    if head::singleton(&response.headers, "mcp-session-id")?.is_some() {
        return Err(McpHttpPeerError::Protocol);
    }
    let status = response.status;
    let media = head::media(&response.headers)?;
    if status == 400 && media != head::Media::Json {
        return Err(McpHttpPeerError::Protocol);
    }
    let frame = match media {
        head::Media::Json => stream::json(response.body, peer.response_limits).await?,
        head::Media::Sse => {
            response_stream(peer, response.body, expected, discovery, deadline).await?
        }
    };
    frame
        .envelope
        .correlate(expected, discovery)
        .map_err(|_| McpHttpPeerError::Correlation)?;
    peer.check(deadline)?;
    Ok(Received { frame, status })
}

async fn response_stream(
    peer: &mut McpHttpPeer,
    body: crate::mcp::http::McpHttpBody,
    expected: &RpcId,
    discovery: bool,
    deadline: Instant,
) -> Result<McpHttpPeerFrame> {
    let limits = stream::response_limits(peer.response_limits);
    let mut read = stream::Reader::new(body, limits)?.next();
    for _ in 0..1024 {
        let (reader, event) = bounded(
            &mut read,
            &*peer.options.clock,
            &peer.cancellation,
            deadline,
        )
        .await?;
        let event = event?.ok_or(McpHttpPeerError::Protocol)?;
        charge_event(peer)?;
        let frame = McpHttpPeerFrame::parse_with_limits(
            event.data().as_bytes().into(),
            peer.response_limits,
        )?;
        peer.check(deadline)?;
        match frame.envelope.kind() {
            RpcKind::Success | RpcKind::Error => {
                frame
                    .envelope
                    .correlate(expected, discovery)
                    .map_err(|_| McpHttpPeerError::Correlation)?;
                return Ok(frame);
            }
            RpcKind::Notification => peer.retain(frame)?,
            RpcKind::Request => return Err(McpHttpPeerError::Protocol),
        }
        read = reader.next();
    }
    Err(McpHttpPeerError::Limit)
}
