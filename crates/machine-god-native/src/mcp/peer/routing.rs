use std::future::{Future, poll_fn};
use std::task::Poll;

use futures_util::stream::{FuturesUnordered, StreamExt};
use serde_json::{Value, json};

use super::{
    BoxFuture, CancellationToken, Instant, McpCatalogKind, McpCatalogLimits, McpPeerError,
    McpPeerTimer, McpRawCatalog, McpStdioConnection, McpStdioError, McpStdioPeer, McpSubmission,
    Result, RpcEnvelope, RpcId, VecDeque,
};
use crate::mcp::pagination::McpCatalogBuilder;
use crate::mcp::protocol::{ProtocolVersion, RpcKind};
use crate::mcp::stdio::{McpStdioControl, McpStdioFrame, McpStdioWriteReceipt};

pub(super) struct CloseOnDrop<'a>(pub Option<&'a McpStdioConnection>);
impl Drop for CloseOnDrop<'_> {
    fn drop(&mut self) {
        if let Some(connection) = self.0 {
            connection.close();
        }
    }
}

pub(super) fn check(cancellation: &CancellationToken, deadline: Instant) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(McpPeerError::Cancelled)
    } else if Instant::now() >= deadline {
        Err(McpPeerError::Deadline)
    } else {
        Ok(())
    }
}

pub(super) async fn bounded<T>(
    future: impl Future<Output = T>,
    timer: &dyn McpPeerTimer,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<T> {
    let mut future = std::pin::pin!(future);
    let mut elapsed = timer.sleep_until(deadline);
    let mut cancelled = std::pin::pin!(cancellation.cancelled());
    poll_fn(|cx| {
        if let Err(error) = check(cancellation, deadline) {
            return Poll::Ready(Err(error));
        }
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(McpPeerError::Cancelled));
        }
        if elapsed.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(McpPeerError::Deadline));
        }
        future.as_mut().poll(cx).map(Ok)
    })
    .await
}

pub(super) fn request(
    id: &RpcId,
    method: &str,
    mut params: Value,
    version: ProtocolVersion,
) -> Result<McpStdioControl> {
    let RpcId::Integer(id) = id else {
        return Err(McpPeerError::Correlation);
    };
    if version == ProtocolVersion::Modern {
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": version.as_str(),
            "io.modelcontextprotocol/clientInfo": {"name":"machine-god", "version":env!("CARGO_PKG_VERSION")},
            "io.modelcontextprotocol/clientCapabilities": {}
        });
    }
    let bytes =
        serde_json::to_vec(&json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
            .map_err(|_| McpPeerError::InvalidResult)?;
    McpStdioControl::discovery(&bytes).map_err(Into::into)
}

type Write = BoxFuture<'static, std::result::Result<McpStdioWriteReceipt, McpStdioError>>;
enum Event {
    Written(std::result::Result<McpStdioWriteReceipt, McpStdioError>),
    Replied(std::result::Result<McpStdioWriteReceipt, McpStdioError>),
    Frame(std::result::Result<McpStdioFrame, McpStdioError>),
}

/// Drains stdout even while a partially written request is waiting on stdin.
/// Unsupported replies have an independent bounded queue, polled concurrently.
pub(super) struct Exchange<'a> {
    pub connection: &'a McpStdioConnection,
    pub notifications: &'a mut VecDeque<RpcEnvelope>,
    pub notification_bytes: &'a mut usize,
    pub timer: &'a dyn McpPeerTimer,
    pub cancellation: &'a CancellationToken,
}
pub(super) async fn exchange(
    context: Exchange<'_>,
    writer: Write,
    expected: &RpcId,
    deadline: Instant,
    discovery_timeout: bool,
) -> Result<McpStdioFrame> {
    let Exchange {
        connection,
        notifications,
        notification_bytes,
        timer,
        cancellation,
    } = context;
    let mut guard = CloseOnDrop(Some(connection));
    let mut writer = Some(writer);
    let mut replies = FuturesUnordered::<Write>::new();
    let mut receive = connection.receive_frame();
    let mut response = None;
    let mut observations = 0_usize;
    loop {
        if writer.is_none() && response.is_some() && replies.is_empty() {
            guard.0 = None;
            return response.ok_or(McpPeerError::Correlation);
        }
        let event = bounded(
            poll_fn(|cx| {
                if let Some(write) = &mut writer
                    && let Poll::Ready(value) = write.as_mut().poll(cx)
                {
                    return Poll::Ready(Event::Written(value));
                }
                if let Poll::Ready(Some(value)) = replies.poll_next_unpin(cx) {
                    return Poll::Ready(Event::Replied(value));
                }
                receive.as_mut().poll(cx).map(Event::Frame)
            }),
            timer,
            cancellation,
            deadline,
        )
        .await;
        let event = match event {
            Err(McpPeerError::Deadline)
                if discovery_timeout
                    && writer.is_none()
                    && response.is_none()
                    && replies.is_empty() =>
            {
                connection.close_after_discovery_timeout();
                guard.0 = None;
                return Err(McpPeerError::Deadline);
            }
            other => other?,
        };
        match event {
            Event::Written(receipt) => {
                receipt?.outcome?;
                writer = None;
            }
            Event::Replied(receipt) => {
                receipt?.outcome?;
            }
            Event::Frame(frame) => {
                let frame = frame?;
                observations += 1;
                if observations > 256 {
                    return Err(McpPeerError::Capacity);
                }
                match frame.envelope().kind() {
                    RpcKind::Success | RpcKind::Error => {
                        frame
                            .envelope()
                            .correlate(expected, false)
                            .map_err(|_| McpPeerError::Correlation)?;
                        if response.replace(frame).is_some() {
                            return Err(McpPeerError::Correlation);
                        }
                    }
                    RpcKind::Notification => {
                        retain_notification(frame, notifications, notification_bytes)?;
                    }
                    RpcKind::Request => {
                        if replies.len() >= 7 {
                            return Err(McpPeerError::Capacity);
                        }
                        let control = McpStdioControl::unsupported(
                            frame.envelope().id().ok_or(McpPeerError::Correlation)?,
                        )?;
                        replies.push(connection.control(control, deadline));
                    }
                }
                receive = connection.receive_frame();
            }
        }
    }
}

fn retain_notification(
    frame: McpStdioFrame,
    notifications: &mut VecDeque<RpcEnvelope>,
    bytes: &mut usize,
) -> Result<()> {
    let total = bytes
        .checked_add(frame.bytes().len())
        .ok_or(McpPeerError::Capacity)?;
    if notifications.len() >= 64 || total > 1024 * 1024 {
        return Err(McpPeerError::Capacity);
    }
    *bytes = total;
    notifications.push_back(frame.into_envelope());
    Ok(())
}

pub(super) async fn call(
    peer: &mut McpStdioPeer,
    submission: McpSubmission,
    deadline: Instant,
) -> Result<RpcEnvelope> {
    check(&peer.cancellation, deadline)?;
    if peer.closed || peer.reserved.as_ref() != Some(submission.rpc_id()) {
        return Err(McpPeerError::Correlation);
    }
    let id = peer.reserved.take().ok_or(McpPeerError::Correlation)?;
    peer.closed = true;
    let frame = exchange(
        Exchange {
            connection: &peer.connection,
            notifications: &mut peer.notifications,
            notification_bytes: &mut peer.notification_bytes,
            timer: &*peer.timer,
            cancellation: &peer.cancellation,
        },
        peer.connection.submit(submission, deadline),
        &id,
        deadline,
        false,
    )
    .await?;
    peer.closed = false;
    Ok(frame.into_envelope())
}

pub(super) async fn catalog(
    peer: &mut McpStdioPeer,
    kind: McpCatalogKind,
    limits: McpCatalogLimits,
    epoch: Instant,
    deadline: Instant,
) -> Result<McpRawCatalog> {
    peer.check_available()?;
    check(&peer.cancellation, deadline)?;
    if matches!(
        kind,
        McpCatalogKind::Resources | McpCatalogKind::ResourceTemplates
    ) && !peer.capabilities.resources()
        || kind == McpCatalogKind::Prompts && !peer.capabilities.prompts()
    {
        return Err(McpPeerError::InvalidResult);
    }
    let mut builder = McpCatalogBuilder::new(kind, peer.protocol.version, limits)
        .map_err(|_| McpPeerError::Capacity)?;
    if epoch > Instant::now() {
        return Err(McpPeerError::InvalidResult);
    }
    loop {
        let cursor = builder.next_cursor().map(str::to_owned);
        let id = peer.allocate()?;
        let params = cursor
            .as_ref()
            .map_or_else(|| json!({}), |cursor| json!({"cursor":cursor}));
        let control = request(&id, kind.method(), params, peer.protocol.version)?;
        peer.closed = true;
        let frame = exchange(
            Exchange {
                connection: &peer.connection,
                notifications: &mut peer.notifications,
                notification_bytes: &mut peer.notification_bytes,
                timer: &*peer.timer,
                cancellation: &peer.cancellation,
            },
            peer.connection.control(control, deadline),
            &id,
            deadline,
            false,
        )
        .await?;
        peer.closed = false;
        let received_at_ms = u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX);
        if !builder
            .append_response(frame.bytes(), &id, cursor.as_deref(), received_at_ms)
            .map_err(|_| McpPeerError::InvalidResult)?
        {
            return builder.finish().map_err(|_| McpPeerError::InvalidResult);
        }
    }
}
