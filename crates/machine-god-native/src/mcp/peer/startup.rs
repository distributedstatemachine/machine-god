use serde_json::json;

use super::McpStdioCompletionObserver;
use super::routing::{Exchange, check, exchange, request};
use super::{
    Arc, CancellationToken, Duration, Instant, McpPeerCapabilities, McpPeerError, McpPeerTimer,
    McpStdioLaunchFactory, McpStdioPeer, NativeOwnedWorkerScope, NegotiatedProtocol, Result,
    VecDeque,
};
use crate::mcp::protocol::{
    HttpDiscoveryStatus, Negotiation, NegotiationAction, ProtocolVersion, TransportKind,
};

#[cfg(test)]
mod tests;

pub(super) async fn connect(
    factory: &mut dyn McpStdioLaunchFactory,
    host: NativeOwnedWorkerScope,
    timer: Arc<dyn McpPeerTimer>,
    cancellation: CancellationToken,
    deadline: Instant,
    discovery_timeout: Duration,
) -> Result<McpStdioPeer> {
    connect_inner(
        factory,
        host,
        timer,
        cancellation,
        Startup {
            deadline: Some(deadline),
            timeout: discovery_timeout,
            observer: None,
        },
    )
    .await
    .map(|(peer, _)| peer)
}

pub(super) async fn connect_observed(
    factory: &mut dyn McpStdioLaunchFactory,
    host: NativeOwnedWorkerScope,
    timer: Arc<dyn McpPeerTimer>,
    cancellation: CancellationToken,
    deadline: Option<Instant>,
    startup_timeout: Duration,
    observer: McpStdioCompletionObserver,
) -> Result<(McpStdioPeer, Instant)> {
    connect_inner(
        factory,
        host,
        timer,
        cancellation,
        Startup {
            deadline,
            timeout: startup_timeout,
            observer: Some(observer),
        },
    )
    .await
}

struct Startup {
    deadline: Option<Instant>,
    timeout: Duration,
    observer: Option<McpStdioCompletionObserver>,
}
impl Startup {
    fn validate(&self, now: Instant) -> Result<()> {
        let maximum = if self.observer.is_some() {
            Duration::from_millis(u64::from(u32::MAX))
        } else {
            Duration::from_secs(300)
        };
        if self.timeout.is_zero()
            || self.timeout > maximum
            || (self.observer.is_none()
                && self
                    .deadline
                    .is_none_or(|deadline| deadline.saturating_duration_since(now) > maximum))
        {
            return Err(McpPeerError::Capacity);
        }
        Ok(())
    }
    fn attempt_deadline(&self, now: Instant) -> Result<Instant> {
        if self.observer.is_none() {
            return self.deadline.ok_or(McpPeerError::Capacity);
        }
        let selected = now
            .checked_add(self.timeout)
            .ok_or(McpPeerError::Capacity)?;
        Ok(self
            .deadline
            .map_or(selected, |deadline| selected.min(deadline)))
    }
    fn live(&self, cancellation: &CancellationToken, timer: &dyn McpPeerTimer) -> Result<()> {
        match self.deadline {
            Some(deadline) => live(cancellation, timer, deadline),
            None if cancellation.is_cancelled() => Err(McpPeerError::Cancelled),
            None => Ok(()),
        }
    }
    fn exchange_deadline(&self, now: Instant, selected: Instant) -> Result<Instant> {
        if self.observer.is_none() {
            let discovery = now
                .checked_add(self.timeout)
                .ok_or(McpPeerError::Capacity)?;
            Ok(discovery.min(self.deadline.ok_or(McpPeerError::Capacity)?))
        } else {
            Ok(selected)
        }
    }
    async fn initial_peer(
        &self,
        factory: &mut dyn McpStdioLaunchFactory,
        host: NativeOwnedWorkerScope,
        timer: Arc<dyn McpPeerTimer>,
        cancellation: CancellationToken,
    ) -> Result<(McpStdioPeer, Instant)> {
        self.live(&cancellation, &*timer)?;
        self.validate(timer.now())?;
        let deadline = self.attempt_deadline(timer.now())?;
        live(&cancellation, &*timer, deadline)?;
        let connection = self
            .launch(factory, host, cancellation.clone(), deadline)
            .await?;
        Ok((unnegotiated(connection, timer, cancellation), deadline))
    }
    async fn launch(
        &self,
        factory: &mut dyn McpStdioLaunchFactory,
        host: NativeOwnedWorkerScope,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> Result<crate::mcp::stdio::McpStdioConnection> {
        let launch = factory.launch()?;
        match &self.observer {
            Some(observer) => {
                launch
                    .connect_observed(host, deadline, cancellation, Box::new(()), observer.clone())
                    .await
            }
            None => {
                launch
                    .connect(host, deadline, cancellation, Box::new(()))
                    .await
            }
        }
        .map_err(Into::into)
    }
}

async fn connect_inner(
    factory: &mut dyn McpStdioLaunchFactory,
    host: NativeOwnedWorkerScope,
    timer: Arc<dyn McpPeerTimer>,
    cancellation: CancellationToken,
    startup: Startup,
) -> Result<(McpStdioPeer, Instant)> {
    let (mut peer, selected_deadline) = startup
        .initial_peer(factory, host, timer, cancellation)
        .await?;
    let (mut negotiation, _) = Negotiation::new(TransportKind::Stdio);
    let deadline = startup.exchange_deadline(peer.timer.now(), selected_deadline)?;
    let id = peer.allocate()?;
    let control = request(&id, "server/discover", json!({}), ProtocolVersion::Modern)?;
    let frame = exchange(
        Exchange {
            connection: &peer.connection,
            notifications: &mut peer.notifications,
            notification_bytes: &mut peer.notification_bytes,
            pending_replies: &mut peer.pending_replies,
            subscription: &mut peer.subscription,
            timer: &peer.timer,
            cancellation: &peer.cancellation,
        },
        peer.connection.control(control, deadline),
        &id,
        deadline,
    )
    .await?;
    match negotiation.response(frame.envelope(), &id, HttpDiscoveryStatus::Ordinary) {
        NegotiationAction::Ready(protocol) => {
            peer.capabilities = McpPeerCapabilities::admit(frame.envelope(), protocol.version)?;
            peer.protocol = protocol;
            live(&peer.cancellation, &*peer.timer, selected_deadline)?;
            Ok((peer, selected_deadline))
        }
        NegotiationAction::Failed(error) => Err(McpPeerError::Negotiation(error)),
        NegotiationAction::SendDiscover => Err(McpPeerError::InvalidResult),
    }
}

pub(super) fn unnegotiated(
    connection: crate::mcp::stdio::McpStdioConnection,
    timer: Arc<dyn McpPeerTimer>,
    cancellation: CancellationToken,
) -> McpStdioPeer {
    McpStdioPeer {
        lifetime: super::McpPeerLifetime::OwnerControlled,
        connection,
        protocol: NegotiatedProtocol {
            transport: TransportKind::Stdio,
            version: ProtocolVersion::Modern,
        },
        capabilities: McpPeerCapabilities::default(),
        timer,
        cancellation,
        feature_identity: Arc::new(()),
        next_id: Some(1),
        reserved: super::McpPendingToolReservation::default(),
        notifications: VecDeque::new(),
        notification_bytes: 0,
        pending_replies: super::routing::Replies::new(),
        subscription: Box::default(),
        closed: false,
    }
}

fn live(
    cancellation: &CancellationToken,
    timer: &dyn McpPeerTimer,
    deadline: Instant,
) -> Result<()> {
    check(cancellation, deadline)?;
    if timer.now() >= deadline {
        Err(McpPeerError::Deadline)
    } else {
        Ok(())
    }
}
