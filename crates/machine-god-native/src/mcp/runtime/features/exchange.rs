use super::Result;
use crate::{
    McpFeatureAction as Action, McpFeatureRequest,
    mcp::{
        catalog::{McpDescriptor, McpDescriptorCatalog},
        commands::McpFeatureCommand,
        control::{
            McpFeatureControlAuthority, McpFeatureOperationOptions, McpFeatureReply,
            McpFeatureRound,
        },
        feature::McpFeatureCodecError,
        runtime::{
            NativeMcpRuntimeError,
            route::{PeerGuard, ServerRoute},
        },
    },
};

pub(super) async fn run(
    lane: &mut PeerGuard<'_>,
    server: &ServerRoute,
    request: &McpFeatureRequest,
    authority: &McpFeatureControlAuthority,
    options: McpFeatureOperationOptions,
) -> Result<McpFeatureRound> {
    if server.clock.now() < server.catalog_epoch {
        return Err(NativeMcpRuntimeError::Invalid.into());
    }
    super::super::subscriptions::drain(lane, server).await?;
    let mut catalogs = Vec::new();
    // Only this exact peer's admitted partition supplies identity evidence.
    // A retained snapshot never extends the command/turn's authority.
    let list = match request.action() {
        Action::ResourceRead => Some(McpFeatureCommand::ResourceList {
            server: server.name.to_string(),
        }),
        Action::ResourceComplete => Some(McpFeatureCommand::ResourceTemplates {
            server: server.name.to_string(),
        }),
        Action::PromptGet | Action::PromptComplete => Some(McpFeatureCommand::PromptList {
            server: server.name.to_string(),
        }),
        _ => None,
    };
    if let Some(list) = list {
        load(lane, server, list, authority, options, &mut catalogs).await?;
    }
    if request.action() == Action::ResourceRead
        && !catalogs.iter().any(|catalog| {
            catalog.descriptors().iter().any(|descriptor| {
                matches!(descriptor,
            McpDescriptor::Resource(resource) if Some(resource.uri()) == request.identity())
            })
        })
    {
        load(
            lane,
            server,
            McpFeatureCommand::ResourceTemplates {
                server: server.name.to_string(),
            },
            authority,
            options,
            &mut catalogs,
        )
        .await?;
    }
    let response = lane.peer.feature_round(
        request,
        &server.name,
        &catalogs,
        authority.clone(),
        options,
        lane.deadline,
    );
    timed(server, authority, lane.deadline, response).await
}

async fn load(
    lane: &mut PeerGuard<'_>,
    server: &ServerRoute,
    command: McpFeatureCommand,
    authority: &McpFeatureControlAuthority,
    options: McpFeatureOperationOptions,
    catalogs: &mut Vec<McpDescriptorCatalog>,
) -> Result<()> {
    let request =
        McpFeatureRequest::try_from(command).map_err(|_| McpFeatureCodecError::InvalidRequest)?;
    let catalog = catalog(lane, server, &request, authority, options).await?;
    let retained = catalogs
        .iter()
        .try_fold(catalog.retained_byte_charge(), |sum, item| {
            sum.checked_add(item.retained_byte_charge())
                .ok_or(McpFeatureCodecError::Limit)
        })?;
    if retained > options.codec.max_retained_bytes {
        return Err(McpFeatureCodecError::Limit.into());
    }
    catalogs.push(catalog);
    Ok(())
}

async fn catalog(
    lane: &mut PeerGuard<'_>,
    server: &ServerRoute,
    request: &McpFeatureRequest,
    authority: &McpFeatureControlAuthority,
    options: McpFeatureOperationOptions,
) -> Result<McpDescriptorCatalog> {
    use crate::mcp::{catalog_refresh::McpRefreshDecision, pagination::McpCatalogKind};
    let kind = match request.action() {
        Action::ResourceList => McpCatalogKind::Resources,
        Action::ResourceTemplates => McpCatalogKind::ResourceTemplates,
        Action::PromptList => McpCatalogKind::Prompts,
        _ => return Err(McpFeatureCodecError::InvalidRequest.into()),
    };
    let now = super::super::catalog_driver::elapsed(server)?;
    let (decision, cached) = {
        let mut state = server
            .catalogs
            .lock()
            .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
        (state.begin(kind, now)?, state.cached(kind))
    };
    let ticket = match decision {
        McpRefreshDecision::Hit
        | McpRefreshDecision::RetryLater {
            may_serve_snapshot: true,
        }
        | McpRefreshDecision::AlreadyRefreshing {
            may_serve_snapshot: true,
        } => {
            return cached.ok_or_else(|| NativeMcpRuntimeError::Unavailable.into());
        }
        McpRefreshDecision::Refresh(ticket) => ticket,
        _ => return Err(NativeMcpRuntimeError::Unavailable.into()),
    };
    let response = lane.peer.feature(
        request,
        &server.name,
        &[],
        authority.clone(),
        options,
        lane.deadline,
    );
    let reply = timed(server, authority, lane.deadline, response).await;
    let now = super::super::catalog_driver::elapsed(server)?;
    let mut state = server
        .catalogs
        .lock()
        .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
    let catalog = match reply {
        Ok(McpFeatureReply::Catalog(catalog)) => catalog,
        failure => {
            state.fail(ticket, now)?;
            if authority.is_live()
                && server.clock.now() < lane.deadline
                && let Some(cached) = cached
            {
                return Ok(cached);
            }
            return Err(failure
                .err()
                .unwrap_or_else(|| McpFeatureCodecError::InvalidResponse.into()));
        }
    };
    state.finish(ticket, &catalog, now)?;
    Ok(catalog)
}

pub(super) async fn timed<T>(
    server: &ServerRoute,
    authority: &McpFeatureControlAuthority,
    deadline: std::time::Instant,
    operation: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    use futures_util::future::{Either, select};
    let stopped = async {
        select(authority.cancelled(), server.clock.sleep_until(deadline)).await;
    };
    if !authority.is_live() || server.clock.now() >= deadline {
        return Err(NativeMcpRuntimeError::Cancelled.into());
    }
    let result = match select(Box::pin(operation), Box::pin(stopped)).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => return Err(NativeMcpRuntimeError::Cancelled.into()),
    };
    if !authority.is_live() || server.clock.now() >= deadline {
        return Err(NativeMcpRuntimeError::Cancelled.into());
    }
    result
}
