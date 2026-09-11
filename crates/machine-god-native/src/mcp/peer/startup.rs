use serde_json::json;

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

pub(super) async fn connect(
    factory: &mut dyn McpStdioLaunchFactory,
    host: NativeOwnedWorkerScope,
    timer: Arc<dyn McpPeerTimer>,
    cancellation: CancellationToken,
    deadline: Instant,
    discovery_timeout: Duration,
) -> Result<McpStdioPeer> {
    check(&cancellation, deadline)?;
    if discovery_timeout.is_zero()
        || discovery_timeout > Duration::from_secs(300)
        || deadline.saturating_duration_since(Instant::now()) > Duration::from_secs(300)
    {
        return Err(McpPeerError::Capacity);
    }
    let connection = factory
        .launch()?
        .connect(host.clone(), deadline, cancellation.clone(), Box::new(()))
        .await?;
    let mut peer = McpStdioPeer {
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
    };
    let (mut negotiation, mut action) = Negotiation::new(TransportKind::Stdio);
    loop {
        check(&peer.cancellation, deadline)?;
        let version = match action {
            NegotiationAction::SendDiscover => ProtocolVersion::Modern,
            NegotiationAction::RestartInitialize(version) => {
                settle(&peer, &host, deadline, true).await?;
                check(&peer.cancellation, deadline)?;
                peer.connection = factory
                    .launch()?
                    .connect(
                        host.clone(),
                        deadline,
                        peer.cancellation.clone(),
                        Box::new(()),
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
                    initialized(&peer, deadline).await?;
                }
                check(&peer.cancellation, deadline)?;
                return Ok(peer);
            }
            NegotiationAction::Failed(error) => return Err(McpPeerError::Negotiation(error)),
            NegotiationAction::Initialize(_) => return Err(McpPeerError::InvalidResult),
        };
        let modern = version == ProtocolVersion::Modern;
        let attempt_deadline = if modern {
            Instant::now()
                .checked_add(discovery_timeout)
                .ok_or(McpPeerError::Capacity)?
                .min(deadline)
        } else {
            deadline
        };
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
            modern && attempt_deadline < deadline,
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
                action =
                    unavailable(&peer, &host, &mut negotiation, modern, error, deadline).await?;
            }
        }
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
    check(&peer.cancellation, deadline)
}

async fn unavailable(
    peer: &McpStdioPeer,
    host: &NativeOwnedWorkerScope,
    negotiation: &mut Negotiation,
    modern: bool,
    error: McpPeerError,
    deadline: Instant,
) -> Result<NegotiationAction> {
    check(&peer.cancellation, deadline)?;
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
