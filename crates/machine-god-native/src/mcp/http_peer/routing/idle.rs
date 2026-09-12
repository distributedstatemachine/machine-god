use super::{
    Instant, McpHttpPeer, McpHttpPeerError, McpHttpPeerFrame, Result, TransportKind,
    observe_listener, prepare_listener, route_listener, stream,
};

// This borrows observation authority only. The pending read (including a GET
// reconnect) stays in the peer, so losing a UI race cannot abandon its parser.
struct Observation<'a>(&'a mut McpHttpPeer);
impl Drop for Observation<'_> {
    fn drop(&mut self) {
        if self.0.check_owner().is_err() {
            self.0.close();
        }
    }
}

pub(in crate::mcp::http_peer) async fn next_notification(
    peer: &mut McpHttpPeer,
    deadline: Instant,
) -> Result<McpHttpPeerFrame> {
    if let Err(error) = peer.check_owner() {
        peer.close();
        return Err(error);
    }
    peer.available()?;
    peer.check(deadline)?;
    if let Some(frame) = peer.take_notification() {
        return Ok(frame);
    }
    if peer.listener.is_none() {
        return Err(McpHttpPeerError::ListenerUnsupported);
    }
    peer.operation_events = 0;
    peer.listener_reconnects = 0;
    let observation = Observation(peer);
    match read(observation.0, deadline).await {
        Ok(Some(frame)) => Ok(frame),
        // Only the outer observation timer is nonfatal. An expired retained
        // acquisition returns Err and is never renewed by a later observer.
        Ok(None) => Err(McpHttpPeerError::Deadline),
        Err(error) => {
            observation.0.close();
            Err(error)
        }
    }
}

async fn read(peer: &mut McpHttpPeer, deadline: Instant) -> Result<Option<McpHttpPeerFrame>> {
    loop {
        let pending = peer.listener.as_mut().ok_or(McpHttpPeerError::Closed)?;
        let result = observe_listener(
            pending,
            &*peer.options.clock,
            &peer.cancellation,
            peer.options.lifetime.constrain(deadline),
        )
        .await;
        if matches!(result, Err(McpHttpPeerError::Deadline)) && peer.check_owner().is_ok() {
            return Ok(None);
        }
        let (reader, event) = result?;
        peer.listener = None;
        let event = event?;
        if let Some(event) = &event {
            route_listener(peer, event, None)?;
        }
        // Retain a Ready event before checking the clock; never repoll its
        // completed future or publish a successful observation after its bound.
        if let Err(error) = peer.check(deadline) {
            peer.listener = Some(reader.next());
            peer.check_owner()?;
            debug_assert!(matches!(error, McpHttpPeerError::Deadline));
            return Ok(None);
        }
        if event.is_some() {
            peer.listener = Some(reader.next());
            if let Some(frame) = peer.take_notification() {
                return Ok(Some(frame));
            }
        } else {
            reconnect(peer, reader, deadline)?;
        }
    }
}

fn reconnect(peer: &mut McpHttpPeer, ended: Box<stream::Reader>, deadline: Instant) -> Result<()> {
    if peer.protocol.transport != TransportKind::StreamableHttp || peer.listener_reconnects >= 32 {
        return Err(McpHttpPeerError::Closed);
    }
    peer.listener_reconnects += 1;
    let acquire = prepare_listener(peer, deadline, peer.listener_resume.retry_ms)?;
    peer.listener_resume = stream::Resume::default();
    // Clean EOF already released `ended`'s socket. Keep that reader only as
    // the existing Read error carrier; acquisition and its original deadline
    // are owned here before the next await, not by the temporary observer.
    peer.listener = Some(Box::pin(async move {
        match acquire.await {
            Ok(reader) => reader.next().await,
            Err(error) => (ended, Err(error)),
        }
    }));
    Ok(())
}
