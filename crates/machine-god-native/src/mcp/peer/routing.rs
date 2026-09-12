use std::future::{Future, poll_fn};
use std::task::Poll;

use futures_util::stream::{FuturesUnordered, StreamExt};
use serde_json::{Value, json};

use super::{
    Arc, BoxFuture, CancellationToken, Instant, McpCatalogKind, McpCatalogLimits, McpPeerError,
    McpPeerTimer, McpRawCatalog, McpStdioConnection, McpStdioError, McpStdioPeer, McpSubmission,
    Result, RpcEnvelope, RpcId, VecDeque,
};
use crate::mcp::pagination::McpCatalogBuilder;
use crate::mcp::protocol::{ProtocolVersion, RpcKind};
use crate::mcp::stdio::{McpStdioControl, McpStdioFrame, McpStdioWriteReceipt};

mod idle;
pub(super) use idle::next_notification;

const MAX_UNSUPPORTED_REPLIES: usize = crate::mcp::stdio::MAX_MCP_STDIO_WRITES - 1;
const MAX_OPERATION_FRAMES: usize = 256;
pub(super) type Replies = FuturesUnordered<BoxFuture<'static, Result<()>>>;

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
        if timer.now() >= deadline {
            return Poll::Ready(Err(McpPeerError::Deadline));
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
pub(super) fn validate_receipt(
    receipt: std::result::Result<McpStdioWriteReceipt, McpStdioError>,
) -> Result<()> {
    receipt?.outcome?;
    Ok(())
}

pub(super) fn unsupported_reply(
    connection: &McpStdioConnection,
    id: &RpcId,
    timer: Arc<dyn McpPeerTimer>,
    cancellation: CancellationToken,
    deadline: Instant,
) -> Result<BoxFuture<'static, Result<()>>> {
    let control = McpStdioControl::unsupported(id)?;
    let writer = connection.control(control, deadline);
    Ok(Box::pin(async move {
        validate_receipt(bounded(writer, &*timer, &cancellation, deadline).await?)?;
        check(&cancellation, deadline)?;
        if timer.now() >= deadline {
            return Err(McpPeerError::Deadline);
        }
        Ok(())
    }))
}

enum Event {
    Written(std::result::Result<McpStdioWriteReceipt, McpStdioError>),
    Replied(Result<()>),
    Frame(std::result::Result<McpStdioFrame, McpStdioError>),
}

/// Drains stdout even while a partially written request is waiting on stdin.
/// Unsupported replies have an independent bounded queue, polled concurrently.
pub(super) struct Exchange<'a> {
    pub connection: &'a McpStdioConnection,
    pub notifications: &'a mut VecDeque<RpcEnvelope>,
    pub notification_bytes: &'a mut usize,
    pub pending_replies: &'a mut Replies,
    pub timer: &'a Arc<dyn McpPeerTimer>,
    pub cancellation: &'a CancellationToken,
}
pub(super) async fn exchange(
    mut context: Exchange<'_>,
    writer: Write,
    expected: &RpcId,
    deadline: Instant,
    discovery_timeout: bool,
) -> Result<McpStdioFrame> {
    let connection = context.connection;
    let timer = context.timer;
    let cancellation = context.cancellation;
    let mut guard = CloseOnDrop(Some(connection));
    let mut writer = Some(writer);
    // Transfer only on first poll. Consequential abandonment still closes the
    // connection and drops these exact writers, never restoring replay authority.
    let mut replies = std::mem::take(context.pending_replies);
    let mut draining = !replies.is_empty();
    let mut receive = connection.receive_frame();
    let mut response = None;
    let mut observations = 0_usize;
    loop {
        check(cancellation, deadline)?;
        if timer.now() >= deadline {
            return Err(McpPeerError::Deadline);
        }
        draining &= !replies.is_empty();
        if writer.is_none() && response.is_some() && replies.is_empty() {
            guard.0 = None;
            return response.ok_or(McpPeerError::Correlation);
        }
        let event = bounded(
            poll_fn(|cx| {
                if !draining
                    && let Some(write) = &mut writer
                    && let Poll::Ready(value) = write.as_mut().poll(cx)
                {
                    return Poll::Ready(Event::Written(value));
                }
                if let Poll::Ready(Some(value)) = replies.poll_next_unpin(cx) {
                    return Poll::Ready(Event::Replied(value));
                }
                receive.as_mut().poll(cx).map(Event::Frame)
            }),
            timer.as_ref(),
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
                validate_receipt(receipt)?;
                writer = None;
            }
            Event::Replied(receipt) => {
                receipt?;
            }
            Event::Frame(frame) => {
                let frame = frame?;
                observations += 1;
                if observations > MAX_OPERATION_FRAMES {
                    return Err(McpPeerError::Capacity);
                }
                route_exchange_frame(
                    &mut context,
                    frame,
                    expected,
                    &mut replies,
                    &mut response,
                    draining,
                    deadline,
                )?;
                receive = connection.receive_frame();
            }
        }
    }
}

fn route_exchange_frame(
    context: &mut Exchange<'_>,
    frame: McpStdioFrame,
    expected: &RpcId,
    replies: &mut Replies,
    response: &mut Option<McpStdioFrame>,
    draining: bool,
    deadline: Instant,
) -> Result<()> {
    match frame.envelope().kind() {
        RpcKind::Success | RpcKind::Error => {
            if draining {
                return Err(McpPeerError::Correlation);
            }
            frame
                .envelope()
                .correlate(expected, false)
                .map_err(|_| McpPeerError::Correlation)?;
            if response.replace(frame).is_some() {
                return Err(McpPeerError::Correlation);
            }
        }
        RpcKind::Notification => {
            retain_notification(frame, context.notifications, context.notification_bytes)?;
        }
        RpcKind::Request => {
            if replies.len() >= MAX_UNSUPPORTED_REPLIES {
                return Err(McpPeerError::Capacity);
            }
            replies.push(unsupported_reply(
                context.connection,
                frame.envelope().id().ok_or(McpPeerError::Correlation)?,
                context.timer.clone(),
                context.cancellation.clone(),
                deadline,
            )?);
        }
    }
    Ok(())
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
) -> Result<McpStdioFrame> {
    peer.check_lifetime()?;
    let deadline = peer.lifetime.constrain(deadline);
    check(&peer.cancellation, deadline)?;
    if peer.closed {
        return Err(McpPeerError::Correlation);
    }
    let id = peer
        .reserved
        .take(submission.rpc_id(), submission.tool_reservation())
        .ok_or(McpPeerError::Correlation)?;
    peer.closed = true;
    let frame = exchange(
        Exchange {
            connection: &peer.connection,
            notifications: &mut peer.notifications,
            notification_bytes: &mut peer.notification_bytes,
            pending_replies: &mut peer.pending_replies,
            timer: &peer.timer,
            cancellation: &peer.cancellation,
        },
        peer.connection.submit(submission, deadline),
        &id,
        deadline,
        false,
    )
    .await?;
    peer.closed = false;
    Ok(frame)
}

pub(super) async fn catalog(
    peer: &mut McpStdioPeer,
    kind: McpCatalogKind,
    limits: McpCatalogLimits,
    epoch: Instant,
    deadline: Instant,
) -> Result<McpRawCatalog> {
    peer.check_available()?;
    let deadline = peer.lifetime.constrain(deadline);
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
    if epoch > peer.timer.now() {
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
                pending_replies: &mut peer.pending_replies,
                timer: &peer.timer,
                cancellation: &peer.cancellation,
            },
            peer.connection.control(control, deadline),
            &id,
            deadline,
            false,
        )
        .await?;
        peer.closed = false;
        let received_at_ms = u64::try_from(
            peer.timer
                .now()
                .checked_duration_since(epoch)
                .ok_or(McpPeerError::InvalidResult)?
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        if !builder
            .append_response(frame.bytes(), &id, cursor.as_deref(), received_at_ms)
            .map_err(|_| McpPeerError::InvalidResult)?
        {
            return builder.finish().map_err(|_| McpPeerError::InvalidResult);
        }
    }
}
