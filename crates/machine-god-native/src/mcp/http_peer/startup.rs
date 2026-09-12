use super::routing::{exchange, request};
use super::{
    Arc, CancellationToken, Completion, Instant, McpHttpPeer, McpHttpPeerCompletion,
    McpHttpPeerError, McpHttpPeerOptions, McpPeerCapabilities, NegotiatedProtocol, ProtocolVersion,
    Result, TransportKind, VecDeque,
};
use super::{Duration, McpHttpCompletionObserver};
use crate::mcp::protocol::{HttpDiscoveryStatus, Negotiation, NegotiationAction};

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
            version: ProtocolVersion::Modern,
        },
        capabilities: McpPeerCapabilities::default(),
        cancellation,
        completion: McpHttpPeerCompletion(Arc::new(Completion::default())),
        feature_identity: Arc::new(()),
        next_id: Some(1),
        reserved: super::McpPendingToolReservation::default(),
        runtimes: Vec::new(),
        notifications: VecDeque::new(),
        notification_bytes: 0,
        operation_events: 0,
        closed: false,
        configured_timeouts: false,
        response_limits: super::WireLimits::default(),
        feature_authority: None,
        authentication: None,
    })
}

pub(super) async fn connect(
    options: McpHttpPeerOptions,
    cancellation: CancellationToken,
    deadline: Instant,
) -> Result<McpHttpPeer> {
    connect_inner(options, cancellation, Some(deadline), None)
        .await
        .map(|(peer, _)| peer)
}

pub(super) async fn connect_observed(
    options: McpHttpPeerOptions,
    cancellation: CancellationToken,
    deadline: Option<Instant>,
    startup_timeout: Duration,
    first_attempt_deadline: Option<Instant>,
    observer: McpHttpCompletionObserver,
    authentication: Option<Arc<crate::mcp::auth::McpAuthLease>>,
) -> Result<(McpHttpPeer, Instant)> {
    if startup_timeout.is_zero() || startup_timeout > Duration::from_millis(u64::from(u32::MAX)) {
        return Err(McpHttpPeerError::Limit);
    }
    connect_inner(
        options,
        cancellation,
        deadline,
        Some(ConfiguredStartup {
            timeout: startup_timeout,
            first_deadline: first_attempt_deadline,
            observer,
            authentication,
        }),
    )
    .await
}

struct ConfiguredStartup {
    timeout: Duration,
    first_deadline: Option<Instant>,
    observer: McpHttpCompletionObserver,
    authentication: Option<Arc<crate::mcp::auth::McpAuthLease>>,
}

fn observed_owner(
    options: McpHttpPeerOptions,
    cancellation: CancellationToken,
    outer_deadline: Option<Instant>,
    configured: Option<&ConfiguredStartup>,
) -> Result<(McpHttpPeer, Instant)> {
    let mut peer = inert(options, cancellation)?;
    peer.authentication = configured.and_then(|policy| policy.authentication.clone());
    peer.configured_timeouts = configured.is_some();
    check_outer(&peer, outer_deadline)?;
    let mut deadline = attempt_deadline(
        &peer,
        outer_deadline,
        configured.map(|policy| policy.timeout),
    )?;
    if let Some(first) = configured.and_then(|policy| policy.first_deadline) {
        deadline = deadline.min(first);
    }
    peer.check(deadline)?;
    if let Some(policy) = configured
        && !(policy.observer)(peer.completion())
    {
        return Err(McpHttpPeerError::Limit);
    }
    Ok((peer, deadline))
}

async fn connect_inner(
    options: McpHttpPeerOptions,
    cancellation: CancellationToken,
    outer_deadline: Option<Instant>,
    configured: Option<ConfiguredStartup>,
) -> Result<(McpHttpPeer, Instant)> {
    let (mut peer, deadline) =
        observed_owner(options, cancellation, outer_deadline, configured.as_ref())?;
    let (mut negotiation, mut action) = Negotiation::new(peer.options.transport);
    loop {
        check_outer(&peer, outer_deadline)?;
        match action {
            NegotiationAction::SendDiscover => {}
            NegotiationAction::Ready(protocol) => {
                peer.protocol = protocol;
                peer.check(deadline)?;
                return Ok((peer, deadline));
            }
            NegotiationAction::Failed(error) => return Err(McpHttpPeerError::Negotiation(error)),
        }
        peer.check(deadline)?;
        let id = peer.allocate()?;
        let control = request(
            &id,
            "server/discover",
            serde_json::json!({}),
            ProtocolVersion::Modern,
        )?;
        let head = peer.make_head(Some("server/discover"))?;
        let writer = peer.connection(Arc::new(head), deadline)?.control(control);
        let received = exchange(&mut peer, writer, &id, true, deadline).await?;
        let status = if received.status == 400 {
            HttpDiscoveryStatus::VersionError
        } else {
            HttpDiscoveryStatus::Ordinary
        };
        action = negotiation.response(received.frame.envelope(), &id, status);
        if let NegotiationAction::Ready(protocol) = action {
            peer.capabilities =
                McpPeerCapabilities::admit(received.frame.envelope(), protocol.version)
                    .map_err(|_| McpHttpPeerError::Protocol)?;
        }
    }
}

fn check_outer(peer: &McpHttpPeer, outer: Option<Instant>) -> Result<()> {
    match outer {
        Some(deadline) => peer.check(deadline),
        None => peer.check_owner(),
    }
}

fn attempt_deadline(
    peer: &McpHttpPeer,
    outer: Option<Instant>,
    timeout: Option<Duration>,
) -> Result<Instant> {
    let selected = match timeout {
        Some(timeout) => peer
            .options
            .clock
            .now()
            .checked_add(timeout)
            .ok_or(McpHttpPeerError::Limit)?,
        None => outer.ok_or(McpHttpPeerError::Invalid)?,
    };
    let selected = outer.map_or(selected, |outer| outer.min(selected));
    Ok(peer.options.lifetime.constrain(selected))
}
