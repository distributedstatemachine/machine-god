//! Demand-driven refresh under the original peer lane and caller authority.
use super::{
    NativeMcpOwnedPeer, NativeMcpPublicationCheckpoint, NativeMcpRuntime,
    NativeMcpRuntimeError as Error, Result,
    refresh::BorrowedRefreshCaller,
    route::{PeerGuard, ServerRoute},
};
use crate::mcp::{
    catalog::{McpDescriptorCatalog, McpDescriptorLimits},
    catalog_refresh::McpRefreshDecision,
    context::NativeMcpTurnContext,
    control::McpFeatureControlAuthority,
    pagination::{McpCatalogKind, McpCatalogLimits},
};
use futures_util::future::{Either, select};
use machine_god_core::CancellationToken;
use std::sync::Arc;

impl NativeMcpRuntime {
    pub(super) async fn refresh_for_turn(
        &self,
        context: &NativeMcpTurnContext,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        let registry = context.registry().map_err(|_| Error::Unavailable)?;
        let caller = BorrowedRefreshCaller::Turn(&registry, cancellation);
        let servers = self
            .catalog_publication()?
            .map_or_else(Vec::new, |view| view.servers.clone());
        for server in servers {
            let mut lane = server.acquire(context, cancellation).await?;
            match select(
                Box::pin(async {
                    select(context.cancelled(), cancellation.cancelled()).await;
                }),
                Box::pin(self.refresh_server(&server, &mut lane, &caller)),
            )
            .await
            {
                Either::Left(_) => return Err(Error::Cancelled),
                Either::Right((result, _)) => {
                    result?;
                }
            }
            context.revalidate().map_err(|_| Error::Unavailable)?;
            if cancellation.is_cancelled() {
                return Err(Error::Cancelled);
            }
        }
        Ok(())
    }

    pub(super) async fn refresh_for_human(
        &self,
        name: &str,
        command: &CancellationToken,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        let publication = self.catalog_publication()?.ok_or(Error::Unavailable)?;
        publication.check()?;
        let server = publication
            .servers
            .iter()
            .find(|server| server.name.as_ref() == name)
            .cloned()
            .ok_or(Error::Unavailable)?;
        // Queueing and fetching remain bound to this view. Only this operation's
        // own successful catalog cutover may retire it before feature selection.
        let authority = McpFeatureControlAuthority::for_human(
            command.clone(),
            cancellation.clone(),
            server.cancellation.clone(),
            publication.retired.clone(),
            server.authority_cancellations.clone(),
        )
        .map_err(|_| Error::Unavailable)?;
        let mut lane = server.acquire_feature(&authority).await?;
        let deadline = lane.deadline;
        let caller = BorrowedRefreshCaller::Human(command, cancellation);
        match select(
            authority.cancelled(),
            Box::pin(self.refresh_server(&server, &mut lane, &caller)),
        )
        .await
        {
            Either::Left(_) => Err(Error::Cancelled),
            Either::Right((result, _)) => {
                let changed = result?;
                caller.check()?;
                server.check_authority()?;
                if (!changed && !authority.is_live()) || server.clock.now() >= deadline {
                    return Err(Error::Cancelled);
                }
                Ok(())
            }
        }
    }

    fn catalog_publication(&self) -> Result<Option<Arc<super::candidate::Publication>>> {
        let state = self.state.lock().map_err(|_| Error::Unavailable)?;
        if state.closed {
            return Err(Error::Unavailable);
        }
        Ok(state.active.clone())
    }

    async fn refresh_server(
        &self,
        server: &Arc<ServerRoute>,
        lane: &mut PeerGuard<'_>,
        caller: &BorrowedRefreshCaller<'_>,
    ) -> Result<bool> {
        caller.check()?;
        server.check_authority()?;
        super::subscriptions::ensure(lane, server).await?;
        if !lane.peer.supports_tools() {
            return Ok(false);
        }
        let now = elapsed(server)?;
        let decision = server
            .catalogs
            .lock()
            .map_err(|_| Error::Unavailable)?
            .begin(McpCatalogKind::Tools, now)?;
        let ticket = match decision {
            McpRefreshDecision::Refresh(ticket) => ticket,
            McpRefreshDecision::Hit
            | McpRefreshDecision::RetryLater {
                may_serve_snapshot: true,
            }
            | McpRefreshDecision::AlreadyRefreshing {
                may_serve_snapshot: true,
            } => return Ok(false),
            _ => return Err(Error::Unavailable),
        };
        let expected = self.publication_checkpoint()?;
        let fetched = self.fetch_tools(server, lane).await;
        let now = elapsed(server)?;
        let attempted = fetched.and_then(|catalog| {
            server.check_authority()?;
            if server.clock.now() >= lane.deadline {
                return Err(Error::Cancelled);
            }
            server
                .catalogs
                .lock()
                .map_err(|_| Error::Unavailable)?
                .validate(&ticket, &catalog, now)?;
            let candidate = self.prepare_tool_refresh(&expected, server, catalog.clone(), &[])?;
            let changed = candidate.is_changed();
            let prospective = candidate.publication_checkpoint();
            self.commit_catalog_refresh(&expected, &prospective, candidate, lane, caller)?;
            Ok((catalog, changed))
        });
        // The peer lane serializes all policy mutation. Do not retain the cache
        // mutex across publication callbacks or controller cleanup observation.
        let mut state = server.catalogs.lock().map_err(|_| Error::Unavailable)?;
        match attempted {
            Ok((catalog, changed)) => {
                state.finish_tools(ticket, &catalog, now, changed)?;
                Ok(changed)
            }
            Err(error) => {
                let may_serve = ticket.may_serve_snapshot();
                state.fail(ticket, now)?;
                server.check_authority()?;
                caller.check()?;
                if may_serve && server.clock.now() < lane.deadline {
                    Ok(false)
                } else {
                    Err(error)
                }
            }
        }
    }

    fn commit_catalog_refresh(
        &self,
        expected: &NativeMcpPublicationCheckpoint,
        prospective: &NativeMcpPublicationCheckpoint,
        candidate: super::refresh::NativeMcpToolRefresh,
        lane: &mut PeerGuard<'_>,
        caller: &BorrowedRefreshCaller<'_>,
    ) -> Result<NativeMcpPublicationCheckpoint> {
        if let Some(controller) = self.controller.get() {
            controller
                .upgrade()
                .ok_or(Error::Unavailable)?
                .sync_catalog_publication(expected, prospective, || {
                    self.commit_tool_refresh_guarded(candidate, lane, caller)
                })
        } else if let Some(ephemeral) = self.ephemeral.get() {
            ephemeral
                .upgrade()
                .ok_or(Error::Unavailable)?
                .sync_catalog_publication(expected, prospective, || {
                    self.commit_tool_refresh_guarded(candidate, lane, caller)
                })
        } else {
            self.commit_tool_refresh_guarded(candidate, lane, caller)
        }
    }

    async fn fetch_tools(
        &self,
        server: &ServerRoute,
        lane: &mut PeerGuard<'_>,
    ) -> Result<McpDescriptorCatalog> {
        let limits = McpCatalogLimits {
            max_item_bytes: McpCatalogLimits::default()
                .max_item_bytes
                .min(self.limits.max_retained_bytes),
            ..McpCatalogLimits::default()
        };
        let fetch = async {
            match &mut *lane.peer {
                #[cfg(test)]
                NativeMcpOwnedPeer::Script(peer) => {
                    peer.tools_catalog(limits, server.catalog_epoch, server.clock.as_ref())
                        .await
                }
                NativeMcpOwnedPeer::Stdio(peer) => peer
                    .catalog(
                        McpCatalogKind::Tools,
                        limits,
                        server.catalog_epoch,
                        lane.deadline,
                    )
                    .await
                    .map_err(|_| Error::Unavailable),
                #[cfg(feature = "mcp-http")]
                NativeMcpOwnedPeer::Http(peer) => peer
                    .catalog(
                        McpCatalogKind::Tools,
                        limits,
                        server.catalog_epoch,
                        lane.deadline,
                    )
                    .await
                    .map_err(|_| Error::Unavailable),
            }
        };
        let raw = match select(Box::pin(fetch), server.clock.sleep_until(lane.deadline)).await {
            Either::Left((result, _)) => result?,
            Either::Right(_) => return Err(Error::Cancelled),
        };
        McpDescriptorCatalog::admit(
            raw,
            McpDescriptorLimits {
                max_catalog_bytes: McpDescriptorLimits::default()
                    .max_catalog_bytes
                    .min(self.limits.max_retained_bytes),
                ..McpDescriptorLimits::default()
            },
        )
        .map_err(|_| Error::Invalid)
    }
}

pub(super) fn elapsed(server: &ServerRoute) -> Result<u64> {
    server
        .clock
        .now()
        .checked_duration_since(server.catalog_epoch)
        .ok_or(Error::Invalid)?
        .as_millis()
        .try_into()
        .map_err(|_| Error::Limit)
}
