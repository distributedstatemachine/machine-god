use super::routing::{bounded, exchange, request};
use super::{
    Arc, CancellationToken, Completion, Instant, McpHttpControl, McpHttpPeer,
    McpHttpPeerCompletion, McpHttpPeerError, McpHttpPeerOptions, McpPeerCapabilities,
    McpSubmissionHttpHead, NegotiatedProtocol, ProtocolVersion, Result, TransportKind, VecDeque,
    head, stream,
};
use crate::mcp::{
    protocol::{HttpDiscoveryStatus, Negotiation, NegotiationAction},
    sse::{SseLimits, SseMode},
};

fn inert(options: McpHttpPeerOptions, cancellation: CancellationToken) -> Result<McpHttpPeer> {
    if options.transport == TransportKind::Stdio {
        return Err(McpHttpPeerError::Invalid);
    }
    let transport = options.transport;
    Ok(McpHttpPeer {
        destination: options.destination.clone(),
        options,
        protocol: NegotiatedProtocol {
            transport,
            version: if transport == TransportKind::LegacySse {
                ProtocolVersion::Legacy20241105
            } else {
                ProtocolVersion::Modern
            },
        },
        capabilities: McpPeerCapabilities::default(),
        session: None,
        cancellation,
        completion: McpHttpPeerCompletion(Arc::new(Completion::default())),
        listener: None,
        listener_resume: stream::Resume::default(),
        listener_reconnects: 0,
        next_id: Some(1),
        reserved: super::McpPendingToolReservation::default(),
        runtimes: Vec::new(),
        notifications: VecDeque::new(),
        notification_bytes: 0,
        operation_events: 0,
        closed: false,
    })
}

pub(super) async fn connect(
    options: McpHttpPeerOptions,
    cancellation: CancellationToken,
    deadline: Instant,
) -> Result<McpHttpPeer> {
    let mut peer = inert(options, cancellation)?;
    let transport = peer.options.transport;
    peer.check(deadline)?;
    if transport == TransportKind::LegacySse {
        legacy_endpoint(&mut peer, deadline).await?;
    }
    let (mut negotiation, mut action) = Negotiation::new(transport);
    let mut initialized_capabilities = McpPeerCapabilities::default();
    loop {
        peer.check(deadline)?;
        let version = match action {
            NegotiationAction::SendDiscover => ProtocolVersion::Modern,
            NegotiationAction::Initialize(version) => version,
            NegotiationAction::Ready(protocol) => {
                peer.protocol = protocol;
                peer.capabilities = initialized_capabilities;
                if protocol.needs_initialized_notification() {
                    super::routing::notification(
                        &mut peer,
                        McpHttpControl::notification(
                            br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                        )?,
                        deadline,
                    )
                    .await?;
                }
                peer.check(deadline)?;
                return Ok(peer);
            }
            NegotiationAction::Failed(error) => return Err(McpHttpPeerError::Negotiation(error)),
            NegotiationAction::RestartInitialize(_) => return Err(McpHttpPeerError::Protocol),
        };
        peer.protocol.version = version;
        let modern = version == ProtocolVersion::Modern;
        let id = peer.allocate()?;
        let method = if modern {
            "server/discover"
        } else {
            "initialize"
        };
        let params = if modern {
            serde_json::json!({})
        } else {
            serde_json::json!({"protocolVersion":version.as_str(),"capabilities":{},"clientInfo":{"name":"machine-god","version":env!("CARGO_PKG_VERSION")}})
        };
        let control = request(&id, method, params, version)?;
        // Pinned legacy initialization has no protocol/session headers before selection.
        let head = if modern {
            peer.make_head(Some(method), None)?
        } else {
            McpSubmissionHttpHead::new(
                peer.destination.endpoint(),
                &peer.options.headers.iter().collect::<Vec<_>>(),
            )
            .map_err(|_| McpHttpPeerError::Invalid)?
        };
        let writer = peer.connection(Arc::new(head), deadline)?.control(control);
        let received = exchange(&mut peer, writer, &id, modern, !modern, deadline).await?;
        if matches!(received.status, 404 | 405) {
            action = negotiation.http_discovery_mismatch(received.status);
            continue;
        }
        let frame = received.frame.ok_or(McpHttpPeerError::Protocol)?;
        let status = if received.status == 400 {
            HttpDiscoveryStatus::VersionError
        } else {
            HttpDiscoveryStatus::Ordinary
        };
        action = negotiation.response(frame.envelope(), &id, status);
        if let NegotiationAction::Ready(protocol) = action {
            initialized_capabilities =
                McpPeerCapabilities::admit(frame.envelope(), protocol.version)
                    .map_err(|_| McpHttpPeerError::Protocol)?;
            if protocol.version != ProtocolVersion::Modern
                && transport == TransportKind::StreamableHttp
            {
                peer.session = received.session;
            }
        }
    }
}

async fn legacy_endpoint(peer: &mut McpHttpPeer, deadline: Instant) -> Result<()> {
    let head = Arc::new(peer.make_head(None, None)?);
    let connection = peer.connection(head, peer.options.lifetime_deadline)?;
    let response = bounded(
        connection.control(McpHttpControl::listen()),
        &*peer.options.clock,
        &peer.cancellation,
        deadline,
    )
    .await??;
    head::status(&response, false)?;
    if response.status != 200 || head::media(&response.headers)? != head::Media::Sse {
        return Err(McpHttpPeerError::Protocol);
    }
    let mut reader =
        stream::Reader::new(response.body, SseMode::Legacy, SseLimits::default())?.next();
    for _ in 0..256 {
        let (next, event) = bounded(
            &mut reader,
            &*peer.options.clock,
            &peer.cancellation,
            deadline,
        )
        .await?;
        let event = event?.ok_or(McpHttpPeerError::Protocol)?;
        if event.event() == Some("endpoint") {
            let destination = peer.options.destination.message_endpoint(event.data())?;
            peer.destination = destination;
            peer.listener = Some(next.next());
            return Ok(());
        }
        if event.event().unwrap_or("message") == "message" && !event.data().is_empty() {
            return Err(McpHttpPeerError::Protocol);
        }
        reader = next.next();
    }
    Err(McpHttpPeerError::Limit)
}
