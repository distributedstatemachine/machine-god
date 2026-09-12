use super::{
    Event, Instant, MAX_OPERATION_FRAMES, MAX_UNSUPPORTED_REPLIES, McpPeerError, McpStdioPeer,
    Poll, Result, RpcEnvelope, RpcKind, StreamExt, bounded, check, retain_notification,
    unsupported_reply,
};
use std::future::poll_fn;

struct Observation<'a>(&'a mut McpStdioPeer);
impl Drop for Observation<'_> {
    fn drop(&mut self) {
        if self.0.check_owner().is_err() {
            self.0.close();
        }
    }
}

pub(in crate::mcp::peer) async fn next_notification(
    peer: &mut McpStdioPeer,
    deadline: Instant,
) -> Result<RpcEnvelope> {
    if let Err(error) = peer.check_owner() {
        peer.close();
        return Err(error);
    }
    peer.check_available()?;
    let observation = Observation(peer);
    match read(observation.0, deadline).await {
        Ok(Some(notification)) => Ok(notification),
        Ok(None) => Err(McpPeerError::Deadline),
        Err(error) => {
            observation.0.close();
            Err(error)
        }
    }
}

async fn read(peer: &mut McpStdioPeer, deadline: Instant) -> Result<Option<RpcEnvelope>> {
    let deadline = peer.lifetime.constrain(deadline);
    // Dropping this receiver releases only the receiving lane/waker. Complete
    // frames and partial NDJSON stay with the existing connection worker.
    let mut receive = peer.connection.receive_frame();
    let mut observations = 0;
    loop {
        peer.check_owner()?;
        if !observing(peer, deadline)? {
            return Ok(None);
        }
        if peer.pending_replies.is_empty()
            && let Some(notification) = peer.take_notification()
        {
            return Ok(Some(notification));
        }
        let event = bounded(
            poll_fn(|cx| {
                if let Poll::Ready(Some(reply)) = peer.pending_replies.poll_next_unpin(cx) {
                    return Poll::Ready(Event::Replied(reply));
                }
                receive.as_mut().poll(cx).map(Event::Frame)
            }),
            &*peer.timer,
            &peer.cancellation,
            deadline,
        )
        .await;
        let event = match event {
            Err(McpPeerError::Deadline) if peer.check_owner().is_ok() => return Ok(None),
            other => other?,
        };
        match event {
            Event::Replied(receipt) => receipt?,
            Event::Frame(frame) => {
                let frame = frame?;
                observations += 1;
                if observations > MAX_OPERATION_FRAMES {
                    return Err(McpPeerError::Capacity);
                }
                match frame.envelope().kind() {
                    RpcKind::Notification => {
                        retain_notification(
                            frame,
                            &mut peer.notifications,
                            &mut peer.notification_bytes,
                        )?;
                    }
                    RpcKind::Request => {
                        if peer.pending_replies.len() >= MAX_UNSUPPORTED_REPLIES {
                            return Err(McpPeerError::Capacity);
                        }
                        peer.pending_replies.push(unsupported_reply(
                            &peer.connection,
                            frame.envelope().id().ok_or(McpPeerError::Correlation)?,
                            peer.timer.clone(),
                            peer.cancellation.clone(),
                            deadline,
                        )?);
                    }
                    RpcKind::Success | RpcKind::Error => return Err(McpPeerError::Correlation),
                }
                // The next loop checks the clock only after retaining the complete
                // notification/reply. No completed receive future is polled twice.
                receive = peer.connection.receive_frame();
            }
            Event::Written(_) => unreachable!("idle observations have no primary writer"),
        }
    }
}

fn observing(peer: &McpStdioPeer, deadline: Instant) -> Result<bool> {
    match check(&peer.cancellation, deadline) {
        Err(McpPeerError::Deadline) => return Ok(false),
        other => other?,
    }
    Ok(peer.timer.now() < deadline)
}
