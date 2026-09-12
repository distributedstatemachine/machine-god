use super::{McpPeerError as Error, McpStdioPeer, Result, RpcId, routing};
use crate::{
    McpFeatureRequest,
    mcp::{
        catalog::McpDescriptorCatalog,
        control::{self, McpFeatureControlAuthority, McpFeatureOperationOptions, McpFeatureReply},
        feature::{McpFeatureCatalogLoad, McpFeatureExchangeOptions},
        stdio::McpStdioControl,
    },
};
use std::time::Instant;

pub(super) async fn execute(
    peer: &mut McpStdioPeer,
    request: &McpFeatureRequest,
    server: &str,
    catalogs: &[McpDescriptorCatalog],
    authority: McpFeatureControlAuthority,
    options: McpFeatureOperationOptions,
    deadline: Instant,
) -> Result<McpFeatureReply> {
    peer.check_available()?;
    let deadline = peer.lifetime.constrain(deadline);
    routing::check(&peer.cancellation, deadline)?;
    options.codec.validate().map_err(Error::Feature)?;
    let wire = peer.connection.wire_limits();
    if wire.max_frame_bytes < options.codec.max_response_bytes
        || wire.max_nodes < options.codec.max_nodes
        || wire.max_depth < 33
        || options.epoch > peer.timer.now()
    {
        return Err(Error::Capacity);
    }
    if !authority.is_live() {
        return Err(Error::Cancelled);
    }
    let mut load: Option<McpFeatureCatalogLoad> = None;
    loop {
        if !authority.is_live() {
            return Err(Error::Cancelled);
        }
        let RpcId::Integer(id) = peer.allocate()? else {
            return Err(Error::Correlation);
        };
        let exchange = control::prepare(
            request,
            server,
            catalogs,
            McpFeatureExchangeOptions::new(peer.protocol, id, peer.capabilities)
                .map_err(Error::Feature)?,
            load.as_ref().and_then(McpFeatureCatalogLoad::next_cursor),
            options,
        )
        .map_err(Error::Feature)?;
        if control::is_list(request) && load.is_none() {
            load = Some(
                McpFeatureCatalogLoad::new(&exchange, options.pagination, options.descriptors)
                    .map_err(Error::Feature)?,
            );
        }
        let outgoing = McpStdioControl::feature(&exchange, authority.clone())?;
        peer.closed = true;
        let mut close = routing::CloseOnDrop(Some(&peer.connection));
        let frame = control::guarded(
            &authority,
            routing::exchange(
                routing::Exchange {
                    connection: &peer.connection,
                    notifications: &mut peer.notifications,
                    notification_bytes: &mut peer.notification_bytes,
                    pending_replies: &mut peer.pending_replies,
                    timer: &peer.timer,
                    cancellation: &peer.cancellation,
                },
                peer.connection.control(outgoing, deadline),
                &exchange.request_id(),
                deadline,
            ),
        )
        .await
        .map_err(|()| Error::Cancelled)??;
        let reply = control::admit(
            &mut load,
            &exchange,
            frame.bytes(),
            peer.timer.now(),
            options.epoch,
        )
        .map_err(Error::Feature)?;
        if !authority.is_live() {
            return Err(Error::Cancelled);
        }
        routing::check(&peer.cancellation, deadline)?;
        close.0 = None;
        peer.closed = false;
        if let Some(reply) = reply {
            return Ok(reply);
        }
    }
}
