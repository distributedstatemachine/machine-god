use super::{
    Arc, Duration, Instant, McpHttpControl, McpHttpPeer, McpHttpPeerError, McpHttpPeerFrame,
    Result, RpcId, RpcKind, WireLimits, head, routing::bounded, stream,
};
use crate::mcp::catalog_refresh::McpSubscriptionFilters;

pub(super) struct Subscription {
    pub id: RpcId,
    read: stream::Read,
    deadline: Instant,
    pending: Option<McpHttpPeerFrame>,
}

const LIMITS: WireLimits = WireLimits {
    max_frame_bytes: 64 * 1024,
    max_depth: 64,
    max_nodes: 8192,
};

pub(super) async fn start(
    peer: &mut McpHttpPeer,
    filters: &McpSubscriptionFilters,
    deadline: Instant,
) -> Result<RpcId> {
    peer.available()?;
    peer.check(deadline)?;
    if peer.subscription.is_some() {
        return Err(McpHttpPeerError::Invalid);
    }
    let lifetime = peer.options.lifetime.constrain(
        peer.options
            .clock
            .now()
            .checked_add(Duration::from_millis(u64::from(u32::MAX)))
            .ok_or(McpHttpPeerError::Limit)?,
    );
    let id = peer.allocate()?;
    let control = McpHttpControl::subscription(&id, filters, peer.protocol.version)?;
    let head = Arc::new(peer.make_head(Some("subscriptions/listen"))?);
    let writer = peer.connection(head, deadline)?.control(control);
    let mut response =
        bounded(writer, &*peer.options.clock, &peer.cancellation, deadline).await??;
    peer.check(deadline)?;
    head::status(&response)?;
    if response.status != 200
        || head::singleton(&response.headers, "mcp-session-id")?.is_some()
        || head::media(&response.headers)? != head::Media::Sse
    {
        return Err(McpHttpPeerError::Protocol);
    }
    response.body.subscription_deadline(lifetime);
    let mut limits = stream::response_limits(LIMITS);
    limits.max_events = 1024;
    let reader = stream::Reader::new(response.body, limits)?;
    peer.check(deadline)?;
    peer.subscription = Some(Subscription {
        id: id.clone(),
        read: reader.next(),
        deadline: lifetime,
        pending: None,
    });
    Ok(id)
}

pub(super) async fn poll(
    peer: &mut McpHttpPeer,
    deadline: Instant,
) -> Result<Option<McpHttpPeerFrame>> {
    if let Err(error) = peer.check_owner() {
        peer.close();
        return Err(error);
    }
    if peer
        .subscription
        .as_ref()
        .is_some_and(|subscription| peer.options.clock.now() >= subscription.deadline)
    {
        peer.close_subscription();
        return Err(McpHttpPeerError::Deadline);
    }
    if peer.options.clock.now() >= deadline {
        return Ok(None);
    }
    // Ordinary response streams can carry this subscription's notifications.
    // Drain that already-admitted queue before waiting on the listener socket,
    // including after its final response. Dequeue preserves existing accounting;
    // it neither touches the owned partial read nor resets its stream quotas.
    if let Some(frame) = peer.take_notification() {
        return Ok(Some(frame));
    }
    let Some(subscription) = &mut peer.subscription else {
        return Ok(None);
    };
    let lifetime = subscription.deadline;
    if subscription.pending.is_some() {
        return Ok(subscription.pending.take());
    }
    // Borrow the owned read, never take it across an await. Its HTTP chunk,
    // SSE line and buffered tail remain exactly where an abandoned poll left
    // them, including when no event has yet been completed.
    let selected = deadline.min(lifetime);
    let clock = peer.options.clock.clone();
    let cancellation = peer.cancellation.clone();
    let mut timeout = clock.sleep_until(selected);
    let mut cancelled = std::pin::pin!(cancellation.cancelled());
    // A completed owned read must always be recovered, even if the selected
    // clock advances during its final poll. The ordinary exchange wrapper's
    // post-poll deadline rejection intentionally drops its socket instead.
    let read = std::future::poll_fn(|cx| {
        use std::{future::Future, task::Poll};
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(McpHttpPeerError::Cancelled));
        }
        if peer.options.clock.now() >= selected || timeout.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(McpHttpPeerError::Deadline));
        }
        subscription.read.as_mut().poll(cx).map(Ok)
    })
    .await;
    if let Err(error) = peer.check_owner() {
        peer.close();
        return Err(error);
    }
    let result = match read {
        Err(McpHttpPeerError::Deadline) if peer.options.clock.now() < lifetime => {
            return Ok(None);
        }
        Err(error) => Err(error),
        Ok((reader, event)) => match event {
            Ok(Some(event)) => {
                let frame =
                    McpHttpPeerFrame::parse_with_limits(event.data().as_bytes().into(), LIMITS);
                if let Some(subscription) = &mut peer.subscription {
                    subscription.read = reader.next();
                }
                frame.and_then(|frame| receive(peer, frame))
            }
            Ok(None) => Err(McpHttpPeerError::Protocol),
            Err(error) => Err(error),
        },
    };
    if result.is_err() {
        peer.close_subscription();
    }
    if let Err(error) = peer.check_owner() {
        peer.close();
        return Err(error);
    }
    if peer.options.clock.now() >= lifetime && peer.subscription.is_some() {
        peer.close_subscription();
        return Err(McpHttpPeerError::Deadline);
    }
    if let Ok(Some(frame)) = result {
        if peer.options.clock.now() >= deadline {
            if let Some(subscription) = &mut peer.subscription {
                subscription.pending = Some(frame);
            }
            return Ok(None);
        }
        return Ok(Some(frame));
    }
    result
}

fn receive(peer: &mut McpHttpPeer, frame: McpHttpPeerFrame) -> Result<Option<McpHttpPeerFrame>> {
    peer.check_owner()?;
    match frame.envelope().kind() {
        RpcKind::Notification => Ok(Some(frame)),
        RpcKind::Request => Err(McpHttpPeerError::Protocol),
        RpcKind::Success | RpcKind::Error => {
            let subscription = peer.subscription.as_ref().ok_or(McpHttpPeerError::Closed)?;
            frame
                .envelope()
                .correlate(&subscription.id, false)
                .map_err(|_| McpHttpPeerError::Correlation)?;
            crate::mcp::catalog_refresh::validate_subscription_response(
                frame.envelope(),
                &subscription.id,
            )
            .map_err(|_| McpHttpPeerError::Protocol)?;
            peer.close_subscription();
            Ok(None)
        }
    }
}
