use super::{McpHttpControl, McpHttpPeer, McpHttpPeerError as Error, Result, RpcId, routing};
use crate::{
    McpFeatureRequest,
    mcp::{
        catalog::McpDescriptorCatalog,
        control::{self, McpFeatureControlAuthority, McpFeatureOperationOptions, McpFeatureRound},
        feature::{McpFeatureCatalogLoad, McpFeatureExchange, McpFeatureExchangeOptions},
        mrtr::McpValidatedResponses,
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
) -> Result<McpFeatureRound> {
    let mut operation = begin(peer, &authority, options, deadline)?;
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
        if let Some(round) = send(peer, exchange, &mut load, &authority, options, deadline).await? {
            operation.settled = true;
            return Ok(round);
        }
    }
}

pub(super) async fn resume(
    peer: &mut McpHttpPeer,
    round: McpFeatureRound,
    responses: McpValidatedResponses,
    deadline: Instant,
) -> Result<McpFeatureRound> {
    peer.available()?;
    peer.check(deadline)?;
    round
        .check_peer(&peer.feature_identity)
        .map_err(Error::Feature)?;
    let RpcId::Integer(id) = peer.allocate()? else {
        return Err(Error::Correlation);
    };
    let (exchange, authority, options) = round
        .resume(&peer.feature_identity, id, responses)
        .map_err(Error::Feature)?;
    let mut operation = begin(peer, &authority, options, deadline)?;
    let result = send(
        operation.peer,
        exchange,
        &mut None,
        &authority,
        options,
        deadline,
    )
    .await?
    .ok_or(Error::Protocol)?;
    operation.settled = true;
    Ok(result)
}

fn begin<'a>(
    peer: &'a mut McpHttpPeer,
    authority: &McpFeatureControlAuthority,
    options: McpFeatureOperationOptions,
    deadline: Instant,
) -> Result<routing::Operation<'a>> {
    peer.available()?;
    peer.check(deadline)?;
    options.codec.validate().map_err(Error::Feature)?;
    if options.epoch > peer.options.clock.now() {
        return Err(Error::Invalid);
    }
    if !authority.is_live() {
        return Err(Error::Cancelled);
    }
    let operation = routing::Operation::begin(peer);
    operation.peer.feature_authority = Some(authority.clone());
    operation.peer.response_limits = super::WireLimits {
        max_frame_bytes: options.codec.max_response_bytes,
        max_depth: 33,
        max_nodes: options.codec.max_nodes,
    };
    Ok(operation)
}

async fn send(
    peer: &mut McpHttpPeer,
    exchange: McpFeatureExchange,
    load: &mut Option<McpFeatureCatalogLoad>,
    authority: &McpFeatureControlAuthority,
    options: McpFeatureOperationOptions,
    deadline: Instant,
) -> Result<Option<McpFeatureRound>> {
    let outgoing = McpHttpControl::feature(&exchange, authority.clone())?;
    let head = Arc::new(peer.make_head(Some(exchange.method()))?);
    let writer = peer.connection(head, deadline)?.control(outgoing);
    let frame = control::guarded(
        authority,
        routing::exchange(peer, writer, &exchange.request_id(), false, deadline),
    )
    .await
    .map_err(|()| Error::Cancelled)??
    .frame;
    let reply = control::admit(
        load,
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
    let round = reply
        .map(|reply| {
            McpFeatureRound::new(
                reply,
                exchange,
                authority.clone(),
                options,
                peer.feature_identity.clone(),
            )
        })
        .transpose()
        .map_err(Error::Feature)?;
    if !authority.is_live() {
        return Err(Error::Cancelled);
    }
    peer.check(deadline)?;
    Ok(round)
}
