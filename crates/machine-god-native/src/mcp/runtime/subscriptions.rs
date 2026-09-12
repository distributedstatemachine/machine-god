//! Exact peer subscription state, shared by startup and caller-driven refresh.
use super::{
    NativeMcpCatalogState, NativeMcpOwnedPeer, NativeMcpRuntimeError as Error, Result,
    route::{PeerGuard, ServerRoute},
};
use crate::mcp::{
    catalog_refresh::{McpRefreshNotification, McpSubscriptionFilters},
    protocol::{RpcEnvelope, RpcId},
};
use futures_util::FutureExt;
use std::{sync::MutexGuard, time::Instant};

#[cfg(test)]
mod tests;

fn state(server: &ServerRoute) -> Result<MutexGuard<'_, NativeMcpCatalogState>> {
    server.catalogs.lock().map_err(|_| Error::Unavailable)
}

fn check(lane: &PeerGuard<'_>, server: &ServerRoute) -> Result<()> {
    server.check_authority()?;
    if server.clock.now() >= lane.deadline {
        return Err(Error::Cancelled);
    }
    Ok(())
}

/// Caller selects this whole future against its original authority and deadline.
/// An abandoned ACK wait keeps the exact selected ID and peer-owned partial read;
/// subsequent demand resumes admission, never sends a replacement request.
pub(super) async fn ensure(lane: &mut PeerGuard<'_>, server: &ServerRoute) -> Result<()> {
    check(lane, server)?;
    drain(lane, server).await?;
    let filters = {
        let selected = state(server)?;
        if selected.subscription_stopped || selected.acknowledged {
            return Ok(());
        }
        if selected.subscription.is_some() {
            None
        } else {
            Some(selected.filters(&lane.peer)?)
        }
    };
    if let Some(filters) = filters {
        if filters.is_empty() {
            return Ok(());
        }
        check(lane, server)?;
        let id = lane
            .peer
            .start_subscription(&filters, lane.deadline)
            .await?;
        state(server)?.install(id, filters)?;
    }
    // Per-observation cap: unrelated ready traffic cannot starve this caller.
    for _ in 0..64 {
        check(lane, server)?;
        if state(server)?.acknowledged {
            if lane.peer.active_subscription() != state(server)?.subscription {
                state(server)?.ended()?;
                return Err(Error::Unavailable);
            }
            return Ok(());
        }
        let result = lane.peer.poll_subscription(lane.deadline).await;
        let Some(envelope) = observe_poll(lane, server, result)? else {
            return Err(Error::Unavailable);
        };
        if state(server)?.observe(&envelope)? {
            lane.peer.close_subscription(lane.deadline).await?;
            return Err(Error::Unavailable);
        }
    }
    Err(Error::Limit)
}

/// Only unrestricted, admitted resource reads call this before cache lookup.
/// Validate the complete replacement before disturbing the current listener.
pub(super) async fn ensure_resource(
    lane: &mut PeerGuard<'_>,
    server: &ServerRoute,
    uri: &str,
) -> Result<()> {
    check(lane, server)?;
    if !lane.peer.capabilities().resources_subscribe() {
        return ensure(lane, server).await;
    }
    let replacement = {
        let selected = state(server)?;
        if selected.subscription_stopped || selected.uris.iter().any(|value| value.as_ref() == uri)
        {
            None
        } else {
            let mut uris: Vec<&str> = selected.uris.iter().map(AsRef::as_ref).collect();
            uris.push(uri);
            McpSubscriptionFilters::new(lane.peer.capabilities(), &uris)
                .map_err(|_| Error::Limit)?;
            Some(uris.into_iter().map(Box::<str>::from).collect::<Vec<_>>())
        }
    };
    let Some(replacement) = replacement else {
        return ensure(lane, server).await;
    };
    // Expire before cancellation/replacement I/O. An abandoned close cannot
    // leave positive-TTL results trusted through a notification gap.
    {
        let mut selected = state(server)?;
        if selected.subscription.is_none() {
            let generation = selected.generation.clone();
            selected
                .policy
                .invalidate_subscription_handoff(&generation)
                .map_err(|_| Error::Unavailable)?;
        }
        selected.ended()?;
    }
    lane.peer.close_subscription(lane.deadline).await?;
    state(server)?.uris = replacement;
    ensure(lane, server).await
}

pub(super) async fn drain(lane: &mut PeerGuard<'_>, server: &ServerRoute) -> Result<()> {
    check(lane, server)?;
    for _ in 0..64 {
        check(lane, server)?;
        let Some(result) = lane.peer.poll_subscription(lane.deadline).now_or_never() else {
            return close_unselected(lane, server).await;
        };
        let Some(envelope) = observe_poll(lane, server, result)? else {
            return close_unselected(lane, server).await;
        };
        if state(server)?.observe(&envelope)? {
            lane.peer.close_subscription(lane.deadline).await?;
            return Ok(());
        }
    }
    // Do not serve a purported hit with more ready invalidations left unseen.
    Err(Error::Limit)
}

async fn close_unselected(lane: &mut PeerGuard<'_>, server: &ServerRoute) -> Result<()> {
    // Drain first: late stdio finals can release retired-ID capacity. A failed
    // prior explicit close never grants permission to install a second listener.
    let unselected = state(server)?.subscription.is_none();
    if unselected && lane.peer.active_subscription().is_some() {
        lane.peer.close_subscription(lane.deadline).await?;
    }
    Ok(())
}

fn observe_poll(
    lane: &PeerGuard<'_>,
    server: &ServerRoute,
    result: Result<Option<RpcEnvelope>>,
) -> Result<Option<RpcEnvelope>> {
    // Drain admitted queued frames first, even after the native listener ended.
    // Invalidate its remaining snapshots once that queue has been observed.
    if !matches!(result, Ok(Some(_))) && lane.peer.active_subscription().is_none() {
        state(server)?.ended()?;
    }
    result
}

impl NativeMcpCatalogState {
    fn filters(&self, peer: &NativeMcpOwnedPeer) -> Result<McpSubscriptionFilters> {
        let uris = self.uris.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        McpSubscriptionFilters::new(peer.capabilities(), &uris).map_err(|_| Error::Invalid)
    }

    fn install(&mut self, id: RpcId, filters: McpSubscriptionFilters) -> Result<()> {
        let RpcId::Integer(value) = id else {
            return Err(Error::Invalid);
        };
        self.policy
            .install_subscription(&self.generation, value, filters)
            .map_err(|_| Error::Invalid)?;
        self.subscription = Some(id);
        self.acknowledged = false;
        Ok(())
    }

    fn ended(&mut self) -> Result<()> {
        if let Some(RpcId::Integer(id)) = self.subscription.as_ref() {
            self.policy
                .end_subscription(&self.generation, *id)
                .map_err(|_| Error::Unavailable)?;
        }
        self.subscription = None;
        self.acknowledged = false;
        Ok(())
    }

    /// Startup owns the peer and applies its original attempt deadline and
    /// cancellation guards around this future. An ACK is not tool authority.
    pub(crate) async fn start_subscription(
        &mut self,
        peer: &mut NativeMcpOwnedPeer,
        deadline: Instant,
    ) -> Result<()> {
        let filters = self.filters(peer)?;
        if filters.is_empty() {
            return Ok(());
        }
        let id = peer.start_subscription(&filters, deadline).await?;
        self.install(id, filters)?;
        while !self.acknowledged {
            let Some(envelope) = peer.poll_subscription(deadline).await? else {
                self.ended()?;
                return Err(Error::Unavailable);
            };
            if self.observe(&envelope)? {
                peer.close_subscription(deadline).await?;
                return Err(Error::Unavailable);
            }
        }
        if peer.active_subscription() != self.subscription {
            self.ended()?;
            return Err(Error::Unavailable);
        }
        Ok(())
    }

    /// Returns whether the exact listener must close; payloads are not retained.
    pub(super) fn observe(&mut self, envelope: &RpcEnvelope) -> Result<bool> {
        match self
            .policy
            .observe(&self.generation, envelope)
            .map_err(|_| Error::Unavailable)?
        {
            McpRefreshNotification::Acknowledged => self.acknowledged = true,
            McpRefreshNotification::CloseUnsupported | McpRefreshNotification::CloseCancelled => {
                self.ended()?;
                self.subscription_stopped = true;
                return Ok(true);
            }
            McpRefreshNotification::Ignored
            | McpRefreshNotification::Invalidated
            | McpRefreshNotification::AllResourceReads => {}
        }
        Ok(false)
    }
}
