use super::{McpHttpControl, McpHttpPeer, McpHttpPeerError as Error, Result, RpcId, routing};
use crate::{
    McpFeatureRequest,
    mcp::{
        catalog::McpDescriptorCatalog,
        control::{self, McpFeatureControlAuthority, McpFeatureOperationOptions, McpFeatureReply},
        feature::{McpFeatureCatalogLoad, McpFeatureExchangeOptions},
    },
};
use std::{sync::Arc, time::Instant};

pub(super) async fn execute(
    peer: &mut McpHttpPeer,
    request: &McpFeatureRequest,
    server: &str,
    catalogs: &[McpDescriptorCatalog],
    authority: McpFeatureControlAuthority,
    options: McpFeatureOperationOptions,
    deadline: Instant,
) -> Result<McpFeatureReply> {
    peer.available()?;
    peer.check(deadline)?;
    options.codec.validate().map_err(Error::Feature)?;
    if options.epoch > peer.options.clock.now() {
        return Err(Error::Invalid);
    }
    if !authority.is_live() {
        return Err(Error::Cancelled);
    }
    let mut operation = routing::Operation::begin(peer);
    operation.peer.feature_authority = Some(authority.clone());
    operation.peer.response_limits = super::WireLimits {
        max_frame_bytes: options.codec.max_response_bytes,
        max_depth: 33,
        max_nodes: options.codec.max_nodes,
    };
    let mut load: Option<McpFeatureCatalogLoad> = None;
    loop {
        let peer = &mut *operation.peer;
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
        let outgoing = McpHttpControl::feature(&exchange, authority.clone())?;
        let head = Arc::new(peer.make_head(Some(exchange.method()), None)?);
        let writer = peer.connection(head, deadline)?.control(outgoing);
        let frame = control::guarded(
            &authority,
            routing::exchange(peer, writer, &exchange.request_id(), false, false, deadline),
        )
        .await
        .map_err(|()| Error::Cancelled)??
        .frame
        .ok_or(Error::Protocol)?;
        let reply = control::admit(
            &mut load,
            &exchange,
            frame.bytes(),
            peer.options.clock.now(),
            options.epoch,
        )
        .map_err(Error::Feature)?;
        if !authority.is_live() {
            return Err(Error::Cancelled);
        }
        peer.check(deadline)?;
        if let Some(reply) = reply {
            operation.settled = true;
            return Ok(reply);
        }
    }
}
