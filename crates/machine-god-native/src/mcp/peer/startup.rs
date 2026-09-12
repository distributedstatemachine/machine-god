use serde_json::json;

use super::McpStdioCompletionObserver;
use super::routing::{CloseOnDrop, Exchange, bounded, check, exchange, request};
use super::{
    Arc, CancellationToken, Duration, Instant, McpPeerCapabilities, McpPeerError, McpPeerTimer,
    McpStdioError, McpStdioLaunchFactory, McpStdioPeer, NativeOwnedWorkerScope, NegotiatedProtocol,
    Result, VecDeque,
};
use crate::mcp::protocol::{
    HttpDiscoveryStatus, Negotiation, NegotiationAction, ProtocolVersion, RpcKind, TransportKind,
};
use crate::mcp::stdio::{McpStdioControl, McpStdioReadEnd};

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
    fn cleanup_deadline(&self, now: Instant) -> Result<Instant> {
        self.deadline.map_or_else(
            || {
                now.checked_add(Duration::from_secs(30))
                    .ok_or(McpPeerError::Capacity)
            },
            Ok,
        )
    }
    fn exchange_deadline(&self, now: Instant, selected: Instant, modern: bool) -> Result<Instant> {
        if modern && self.observer.is_none() {
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
    let (mut peer, mut selected_deadline) = startup
        .initial_peer(factory, host.clone(), timer, cancellation)
        .await?;
    let (mut negotiation, mut action) = Negotiation::new(TransportKind::Stdio);
    let mut was_modern = true;
    loop {
        startup.live(&peer.cancellation, &*peer.timer)?;
        let version = match action {
            NegotiationAction::SendDiscover => ProtocolVersion::Modern,
            NegotiationAction::RestartInitialize(version) => {
                if was_modern {
                    selected_deadline = startup.attempt_deadline(peer.timer.now())?;
                }
                // Pinned legacy connection control starts before disconnecting
                // the previous child; cleanup consumes this attempt's budget.
                settle(
                    &peer,
                    &host,
                    startup
                        .cleanup_deadline(peer.timer.now())?
                        .min(selected_deadline),
                    true,
                )
                .await?;
                live(&peer.cancellation, &*peer.timer, selected_deadline)?;
                peer.connection = startup
                    .launch(
                        factory,
                        host.clone(),
                        peer.cancellation.clone(),
                        selected_deadline,
                    )
                    .await?;
                // Notifications cannot acquire a new connection generation.
                peer.notifications.clear();
                peer.notification_bytes = 0;
                version
            }
            NegotiationAction::Ready(protocol) => {
                peer.protocol = protocol;
                if protocol.needs_initialized_notification() {
                    initialized(&peer, selected_deadline).await?;
                }
                live(&peer.cancellation, &*peer.timer, selected_deadline)?;
                return Ok((peer, selected_deadline));
            }
            NegotiationAction::Failed(error) => return Err(McpPeerError::Negotiation(error)),
            NegotiationAction::Initialize(_) => return Err(McpPeerError::InvalidResult),
        };
        let modern = version == ProtocolVersion::Modern;
        was_modern = modern;
        let attempt_deadline =
            startup.exchange_deadline(peer.timer.now(), selected_deadline, modern)?;
        let id = peer.allocate()?;
        let (method, params) = startup_params(version);
        let control = request(&id, method, params, version)?;
        let response = exchange(
            Exchange {
                connection: &peer.connection,
                notifications: &mut peer.notifications,
                notification_bytes: &mut peer.notification_bytes,
                timer: &*peer.timer,
                cancellation: &peer.cancellation,
            },
            peer.connection.control(control, attempt_deadline),
            &id,
            attempt_deadline,
            modern
                && startup
                    .deadline
                    .is_none_or(|deadline| attempt_deadline < deadline),
        )
        .await;
        match response {
            Ok(frame) => {
                if frame.envelope().kind() == RpcKind::Success {
                    peer.capabilities = McpPeerCapabilities::admit(frame.envelope(), version)?;
                }
                action = negotiation.response(frame.envelope(), &id, HttpDiscoveryStatus::Ordinary);
            }
            Err(error) => {
                action = unavailable(
                    &peer,
                    &host,
                    &mut negotiation,
                    modern,
                    error,
                    startup.cleanup_deadline(peer.timer.now())?,
                )
                .await?;
            }
        }
    }
}

fn unnegotiated(
    connection: crate::mcp::stdio::McpStdioConnection,
    timer: Arc<dyn McpPeerTimer>,
    cancellation: CancellationToken,
) -> McpStdioPeer {
    McpStdioPeer {
        connection,
        protocol: NegotiatedProtocol {
            transport: TransportKind::Stdio,
            version: ProtocolVersion::Modern,
        },
        capabilities: McpPeerCapabilities::default(),
        timer,
        cancellation,
        next_id: Some(1),
        reserved: super::McpPendingToolReservation::default(),
        notifications: VecDeque::new(),
        notification_bytes: 0,
        closed: false,
    }
}

fn startup_params(version: ProtocolVersion) -> (&'static str, serde_json::Value) {
    if version == ProtocolVersion::Modern {
        ("server/discover", json!({}))
    } else {
        (
            "initialize",
            json!({"protocolVersion":version.as_str(), "capabilities":{},
            "clientInfo":{"name":"machine-god", "version":env!("CARGO_PKG_VERSION")}}),
        )
    }
}

async fn initialized(peer: &McpStdioPeer, deadline: Instant) -> Result<()> {
    let mut guard = CloseOnDrop(Some(&peer.connection));
    let notification = McpStdioControl::notification(
        br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )?;
    // This fixed notification fits one pipe write; no peer response is required.
    bounded(
        peer.connection.control(notification, deadline),
        &*peer.timer,
        &peer.cancellation,
        deadline,
    )
    .await??
    .outcome?;
    guard.0 = None;
    Ok(())
}

async fn settle(
    peer: &McpStdioPeer,
    host: &NativeOwnedWorkerScope,
    deadline: Instant,
    close: bool,
) -> Result<()> {
    if close {
        peer.connection.close();
    }
    let completion = peer.connection.completion();
    bounded(
        host.run(move || completion.wait_on_worker()),
        &*peer.timer,
        &peer.cancellation,
        deadline,
    )
    .await?
    .map_err(|_| McpPeerError::Closed)?
    .map_err(|_| McpPeerError::Closed)?;
    live(&peer.cancellation, &*peer.timer, deadline)
}

async fn unavailable(
    peer: &McpStdioPeer,
    host: &NativeOwnedWorkerScope,
    negotiation: &mut Negotiation,
    modern: bool,
    error: McpPeerError,
    deadline: Instant,
) -> Result<NegotiationAction> {
    live(&peer.cancellation, &*peer.timer, deadline)?;
    if !matches!(
        error,
        McpPeerError::Deadline | McpPeerError::Transport(McpStdioError::Closed)
    ) {
        return Err(error);
    }
    // Timeout exchange has requested the worker's freeze; cancelling here would
    // erase that observation. EOF is already closed. Both retain cleanup owners.
    settle(peer, host, deadline, false).await?;
    let observation = peer
        .connection
        .close_observation()
        .ok_or(McpPeerError::Closed)?;
    if observation.buffered_partial_frame || observation.unconsumed_complete_frames != 0 {
        return Err(McpPeerError::InvalidResult);
    }
    match (observation.read_end, observation.reason, modern) {
        (McpStdioReadEnd::CleanEof, McpStdioError::Closed, true)
        | (McpStdioReadEnd::DiscoveryTimeoutQuiescent, McpStdioError::Deadline, true) => {
            Ok(negotiation.stdio_discovery_unavailable())
        }
        (McpStdioReadEnd::CleanEof, McpStdioError::Closed, false) => {
            Ok(negotiation.stdio_initialize_closed())
        }
        _ => Err(error),
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
