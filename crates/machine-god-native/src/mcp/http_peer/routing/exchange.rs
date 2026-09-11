use super::{
    Arc, BoxFuture, Instant, McpHttpControl, McpHttpError, McpHttpPeer, McpHttpPeerError,
    McpHttpPeerFrame, McpHttpResponse, Poll, ProtocolVersion, Result, RpcId, RpcKind, SseEvent,
    SseLimits, SseMode, TransportKind, bounded, charge_event, delay, head, reconnect_listener,
    route_listener, stream,
};
type Write = BoxFuture<'static, std::result::Result<McpHttpResponse, McpHttpError>>;
pub(in crate::mcp::http_peer) struct Received {
    pub frame: Option<McpHttpPeerFrame>,
    pub status: u16,
    pub session: Option<Box<str>>,
}
pub(in crate::mcp::http_peer) async fn notification(
    peer: &mut McpHttpPeer,
    control: McpHttpControl,
    deadline: Instant,
) -> Result<()> {
    let response = peer
        .connection(Arc::new(peer.make_head(None, None)?), deadline)?
        .control(control)
        .await?;
    head::status(&response, peer.session.is_some())?;
    head::stable_session(&response.headers, peer.session.as_deref())?;
    if response.status != 202 {
        return Err(McpHttpPeerError::Protocol);
    }
    Ok(())
}
pub(in crate::mcp::http_peer) async fn exchange(
    peer: &mut McpHttpPeer,
    writer: Write,
    expected: &RpcId,
    discovery: bool,
    capture_session: bool,
    deadline: Instant,
) -> Result<Received> {
    if peer.protocol.transport == TransportKind::LegacySse {
        return old_sse(peer, writer, expected, deadline).await;
    }
    let response = drive_writer(peer, writer, deadline).await?;
    head::status(&response, peer.session.is_some())?;
    if discovery && matches!(response.status, 404 | 405) {
        return Ok(Received {
            frame: None,
            status: response.status,
            session: None,
        });
    }
    if response.status != 200 && !(discovery && response.status == 400) {
        return Err(McpHttpPeerError::Protocol);
    }
    let session = if capture_session {
        head::session(&response.headers)?
    } else {
        head::stable_session(&response.headers, peer.session.as_deref())?;
        None
    };
    let status = response.status;
    let media = head::media(&response.headers)?;
    if status == 400 && media != head::Media::Json {
        return Err(McpHttpPeerError::Protocol);
    }
    let frame = match media {
        head::Media::Json => stream::json(response.body, 8 * 1024 * 1024).await?,
        head::Media::Sse => {
            response_stream(peer, response.body, expected, deadline, !capture_session).await?
        }
    };
    frame
        .envelope
        .correlate(expected, discovery)
        .map_err(|_| McpHttpPeerError::Correlation)?;
    Ok(Received {
        frame: Some(frame),
        status,
        session,
    })
}

enum Event {
    Written(std::result::Result<McpHttpResponse, McpHttpError>),
    Read(Box<stream::Reader>, Result<Option<SseEvent>>),
}
/// Polls the owned listener without abandoning its in-progress body read.
async fn drive_writer(
    peer: &mut McpHttpPeer,
    mut writer: Write,
    deadline: Instant,
) -> Result<McpHttpResponse> {
    loop {
        let event = bounded(
            std::future::poll_fn(|cx| {
                if let Poll::Ready(response) = writer.as_mut().poll(cx) {
                    return Poll::Ready(Event::Written(response));
                }
                if let Some(listener) = &mut peer.listener {
                    return listener
                        .as_mut()
                        .poll(cx)
                        .map(|(reader, event)| Event::Read(reader, event));
                }
                Poll::Pending
            }),
            &*peer.options.clock,
            &peer.cancellation,
            deadline,
        )
        .await?;
        match event {
            Event::Written(response) => return response.map_err(Into::into),
            Event::Read(reader, event) => {
                peer.listener = None;
                if let Some(event) = event? {
                    route_listener(peer, &event, None)?;
                    peer.listener = Some(reader.next());
                } else {
                    drop(reader);
                    reconnect_listener(peer, deadline).await?;
                }
            }
        }
    }
}

async fn old_sse(
    peer: &mut McpHttpPeer,
    writer: Write,
    expected: &RpcId,
    deadline: Instant,
) -> Result<Received> {
    let mut writer = Some(writer);
    let mut final_frame = None;
    for _ in 0..1024 {
        if writer.is_none() && final_frame.is_some() {
            return Ok(Received {
                frame: final_frame,
                status: 200,
                session: None,
            });
        }
        let event = bounded(
            std::future::poll_fn(|cx| {
                if let Some(writer) = &mut writer
                    && let Poll::Ready(response) = writer.as_mut().poll(cx)
                {
                    return Poll::Ready(Event::Written(response));
                }
                let Some(listener) = &mut peer.listener else {
                    return Poll::Ready(Event::Written(Err(McpHttpError::Closed)));
                };
                listener
                    .as_mut()
                    .poll(cx)
                    .map(|(reader, event)| Event::Read(reader, event))
            }),
            &*peer.options.clock,
            &peer.cancellation,
            deadline,
        )
        .await?;
        match event {
            Event::Written(response) => {
                let response = response?;
                head::status(&response, false)?;
                if response.status != 202 {
                    return Err(McpHttpPeerError::Protocol);
                }
                writer = None;
            }
            Event::Read(reader, event) => {
                peer.listener = None;
                let event = event?.ok_or(McpHttpPeerError::Closed)?;
                if let Some(frame) = route_listener(peer, &event, Some(expected))?
                    && final_frame.replace(frame).is_some()
                {
                    return Err(McpHttpPeerError::Correlation);
                }
                peer.listener = Some(reader.next());
            }
        }
    }
    Err(McpHttpPeerError::Limit)
}

async fn response_stream(
    peer: &mut McpHttpPeer,
    body: crate::mcp::http::McpHttpBody,
    expected: &RpcId,
    deadline: Instant,
    allow_resume: bool,
) -> Result<McpHttpPeerFrame> {
    let modern = peer.protocol.version == ProtocolVersion::Modern;
    let mode = if modern {
        SseMode::Modern
    } else {
        SseMode::Legacy
    };
    let mut read = stream::Reader::new(body, mode, SseLimits::default())?.next();
    let mut resume = stream::Resume::default();
    let mut reconnects = 0;
    for _ in 0..1024 {
        let event = bounded(
            std::future::poll_fn(|cx| {
                if let Poll::Ready((reader, event)) = read.as_mut().poll(cx) {
                    return Poll::Ready((true, reader, event));
                }
                if let Some(listener) = &mut peer.listener {
                    return listener
                        .as_mut()
                        .poll(cx)
                        .map(|(reader, event)| (false, reader, event));
                }
                Poll::Pending
            }),
            &*peer.options.clock,
            &peer.cancellation,
            deadline,
        )
        .await?;
        let (primary, reader, event) = event;
        if !primary {
            peer.listener = None;
            if let Some(event) = event? {
                route_listener(peer, &event, None)?;
                peer.listener = Some(reader.next());
            } else {
                drop(reader);
                reconnect_listener(peer, deadline).await?;
            }
            continue;
        }
        let Some(event) = event? else {
            drop(reader);
            if modern || !allow_resume || !resume.resumable(peer.protocol) || reconnects == 8 {
                return Err(McpHttpPeerError::Protocol);
            }
            reconnects += 1;
            delay(peer, resume.retry_ms, deadline).await?;
            let head = Arc::new(peer.make_head(None, resume.id.as_deref())?);
            let response = drive_writer(
                peer,
                peer.connection(head, deadline)?
                    .control(McpHttpControl::listen()),
                deadline,
            )
            .await?;
            head::status(&response, peer.session.is_some())?;
            head::stable_session(&response.headers, peer.session.as_deref())?;
            if response.status != 200 || head::media(&response.headers)? != head::Media::Sse {
                return Err(McpHttpPeerError::Protocol);
            }
            read = stream::Reader::new(response.body, mode, SseLimits::default())?.next();
            resume = stream::Resume::default();
            continue;
        };
        charge_event(peer)?;
        if !event.data().is_empty() {
            let frame = McpHttpPeerFrame::parse(event.data().as_bytes().into())?;
            match frame.envelope.kind() {
                RpcKind::Success | RpcKind::Error => {
                    frame
                        .envelope
                        .correlate(expected, false)
                        .map_err(|_| McpHttpPeerError::Correlation)?;
                    return Ok(frame);
                }
                RpcKind::Notification => peer.retain(frame)?,
                RpcKind::Request if !modern => {
                    let control = McpHttpControl::unsupported(
                        frame.envelope.id().ok_or(McpHttpPeerError::Correlation)?,
                    )?;
                    notification(peer, control, deadline).await?;
                }
                RpcKind::Request => return Err(McpHttpPeerError::Protocol),
            }
        }
        if !modern {
            resume.commit(&event)?;
        }
        read = reader.next();
    }
    Err(McpHttpPeerError::Limit)
}
