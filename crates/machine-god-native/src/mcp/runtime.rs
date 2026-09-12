//! Atomic native publication of explicitly admitted, owned MCP server peers.

mod addition;
mod call;
mod candidate;
mod checkpoint;
mod deferred;
mod executor;
mod features;
mod peer;
mod readiness;
mod route;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::script::ScriptPeer;
mod tool;

pub use addition::NativeMcpRuntimeAddition;
pub use call::{NativeMcpRuntimeToolCall, NativeMcpRuntimeToolResponse};
pub use candidate::{NativeMcpRuntimeCandidate, NativeMcpServerCandidate};
pub use checkpoint::NativeMcpPublicationCheckpoint;
pub use executor::{
    NativeMcpToolCompletionPolicy, NativeMcpToolExecutionPolicy, NativeMcpToolExecutor,
};
pub use features::{NativeMcpFeatureError, NativeMcpFeatureResult, NativeMcpHumanCommand};
pub use peer::{NativeMcpOwnedPeer, NativeMcpPeerCompletion};

use super::{context::NativeMcpContexts, submission::McpSubmissionRegistry};
use crate::{McpToolCatalog, McpToolCatalogError, McpToolCatalogErrorKind, McpToolCatalogSnapshot};
use machine_god_core::{BoxFuture, CancellationToken, ToolContext};
use std::{
    fmt,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Instant,
};

/// Explicit monotonic observation; construction never reads an ambient clock.
pub trait NativeMcpRuntimeClock: Send + Sync + 'static {
    fn now(&self) -> Instant;
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()>;
}

/// Finite runtime ownership bounds, additional to schema/catalog/peer bounds.
#[derive(Clone, Copy, Debug)]
pub struct NativeMcpRuntimeLimits {
    pub max_servers: usize,
    pub max_tools: usize,
    pub max_retired_servers: usize,
    pub max_pending_operations: usize,
    pub max_retained_bytes: usize,
}
impl Default for NativeMcpRuntimeLimits {
    fn default() -> Self {
        Self {
            max_servers: 64,
            max_tools: 131_072,
            max_retired_servers: 64,
            max_pending_operations: 64,
            max_retained_bytes: 256 * 1024 * 1024,
        }
    }
}
impl NativeMcpRuntimeLimits {
    fn validate(self) -> Result<Self> {
        let cap = Self::default();
        for (value, maximum) in [
            (self.max_servers, cap.max_servers),
            (self.max_tools, cap.max_tools),
            (self.max_retired_servers, cap.max_retired_servers),
            (self.max_pending_operations, cap.max_pending_operations),
            (self.max_retained_bytes, cap.max_retained_bytes),
        ] {
            if value == 0 || value > maximum {
                return Err(NativeMcpRuntimeError::Limit);
            }
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpRuntimeError {
    Invalid,
    Limit,
    Unavailable,
    Cancelled,
}
impl fmt::Display for NativeMcpRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native MCP runtime unavailable")
    }
}
impl std::error::Error for NativeMcpRuntimeError {}
type Result<T> = std::result::Result<T, NativeMcpRuntimeError>;

struct TurnPin {
    registry: Weak<McpSubmissionRegistry>,
    publication: Weak<candidate::Publication>,
}
#[derive(Default)]
struct State {
    active: Option<Arc<candidate::Publication>>,
    retired: Vec<Arc<route::ServerRoute>>,
    retired_byte_charge: usize,
    completions: Vec<NativeMcpPeerCompletion>,
    turns: Vec<TurnPin>,
    closed: bool,
}

/// One host-owned publication lineage. Reverse engine/turn references are weak;
/// dynamic registrations never retain a peer generation on their own.
pub struct NativeMcpRuntime {
    contexts: Arc<NativeMcpContexts>,
    clock: Arc<dyn NativeMcpRuntimeClock>,
    limits: NativeMcpRuntimeLimits,
    state: Mutex<State>,
    identity: Arc<()>,
    controller: OnceLock<Weak<super::controller::NativeMcpController>>,
    executor: Arc<dyn NativeMcpToolExecutor>,
    policy: NativeMcpToolExecutionPolicy,
    feature_operations: Arc<std::sync::atomic::AtomicUsize>,
}
impl fmt::Debug for NativeMcpRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpRuntime { <redacted> }")
    }
}
impl NativeMcpRuntime {
    /// Retains explicit authorities without discovering, launching or polling.
    /// # Errors
    /// Rejects limits larger than the finite native runtime caps.
    pub fn new(
        contexts: Arc<NativeMcpContexts>,
        clock: Arc<dyn NativeMcpRuntimeClock>,
        executor: Arc<dyn NativeMcpToolExecutor>,
        policy: NativeMcpToolExecutionPolicy,
        limits: NativeMcpRuntimeLimits,
    ) -> Result<Self> {
        Ok(Self {
            contexts,
            clock,
            limits: limits.validate()?,
            state: Mutex::default(),
            identity: Arc::new(()),
            controller: OnceLock::new(),
            executor,
            policy: policy.validate()?,
            feature_operations: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    /// Publishes every prepared server together. On failure the old executable
    /// generation remains usable and the rejected candidate is dropped outside
    /// the publication lock. Drain bounded retirement before another reload.
    /// # Errors
    /// Rejects a foreign candidate, closed runtime or exhausted cleanup capacity.
    pub fn publish(&self, candidate: NativeMcpRuntimeCandidate) -> Result<()> {
        self.publish_selected(candidate, None)
    }

    /// Publishes only if the observed publication is still exact. Obsolete
    /// asynchronous startup/reload results cannot replace a newer generation.
    /// # Errors
    /// Rejects foreign/stale checkpoints and the ordinary publication failures,
    /// without retiring or cancelling the current usable generation.
    pub fn publish_if(
        &self,
        candidate: NativeMcpRuntimeCandidate,
        expected: &NativeMcpPublicationCheckpoint,
    ) -> Result<()> {
        self.publish_selected(candidate, Some(expected))
    }

    fn publish_selected(
        &self,
        candidate: NativeMcpRuntimeCandidate,
        expected: Option<&NativeMcpPublicationCheckpoint>,
    ) -> Result<()> {
        let candidate = candidate.publication;
        if !Arc::ptr_eq(&self.identity, &candidate.identity) {
            return Err(NativeMcpRuntimeError::Invalid);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
        if state.closed {
            return Err(NativeMcpRuntimeError::Unavailable);
        }
        if let Some(expected) = expected {
            expected.check(self, &state)?;
        }
        // Preparation is not activation: selected authority may have been
        // revoked while the private candidate was waiting for publication.
        candidate.check()?;
        for server in &candidate.servers {
            server.check_authority()?;
        }
        let previous_count = state.active.as_ref().map_or(0, |value| value.servers.len());
        if state.retired.len() + state.completions.len() + previous_count
            > self.limits.max_retired_servers
        {
            return Err(NativeMcpRuntimeError::Limit);
        }
        let previous_charge = state
            .active
            .as_ref()
            .map_or(0, |value| value.retained_bytes);
        let retired_charge = state
            .retired_byte_charge
            .checked_add(previous_charge)
            .ok_or(NativeMcpRuntimeError::Limit)?;
        if retired_charge
            .checked_add(candidate.retained_bytes)
            .is_none_or(|total| total > self.limits.max_retained_bytes)
        {
            return Err(NativeMcpRuntimeError::Limit);
        }
        state.retired_byte_charge = retired_charge;
        let previous = state.active.take();
        let deferred: Vec<_> = previous
            .as_ref()
            .into_iter()
            .flat_map(|value| value.tools.values())
            .map(|tool| tool.owner.retire_deferred())
            .collect();
        if let Some(previous) = &previous {
            previous
                .retired
                .store(true, std::sync::atomic::Ordering::Release);
            state.retired.extend(previous.servers.iter().cloned());
        }
        state.active = Some(candidate);
        drop(state);
        for retirement in deferred {
            retirement.complete();
        }
        if let Some(previous) = &previous {
            for server in &previous.servers {
                server.cancellation.cancel();
            }
        }
        drop(previous);
        Ok(())
    }

    /// Irreversibly prevents new publication and invalidates every executable
    /// binding before waking waiters. Cleanup remains explicitly host-driven.
    pub fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        let previous = state.active.take();
        state.retired_byte_charge += previous.as_ref().map_or(0, |value| value.retained_bytes);
        let deferred: Vec<_> = previous
            .as_ref()
            .into_iter()
            .flat_map(|value| value.tools.values())
            .map(|tool| tool.owner.retire_deferred())
            .collect();
        if let Some(previous) = &previous {
            previous
                .retired
                .store(true, std::sync::atomic::Ordering::Release);
            state.retired.extend(previous.servers.iter().cloned());
        }
        let retired = state.retired.clone();
        drop(state);
        for retirement in deferred {
            retirement.complete();
        }
        for server in retired {
            server.cancellation.cancel();
        }
        drop(previous);
    }

    /// Closes retired peers after their cancelled operation releases ownership.
    /// Returned receipts distinguish closure from actual worker/reap completion.
    /// No work occurs before polling; cancellation preserves undrained ownership.
    /// # Errors
    /// Rejects expiration/cancellation or unavailable publication ownership.
    pub async fn drain_retired(
        &self,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<Vec<NativeMcpPeerCompletion>> {
        use futures_util::future::{Either, select};
        loop {
            if cancellation.is_cancelled() || self.clock.now() >= deadline {
                return Err(NativeMcpRuntimeError::Cancelled);
            }
            let server = {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
                if state.retired.is_empty() {
                    state.retired_byte_charge = 0;
                    return Ok(std::mem::take(&mut state.completions));
                }
                state.retired.first().cloned()
            };
            let Some(server) = server else {
                continue;
            };
            let stopped = async {
                select(cancellation.cancelled(), self.clock.sleep_until(deadline)).await;
            };
            let mut peer = match select(Box::pin(server.peer.lock()), Box::pin(stopped)).await {
                Either::Left((peer, _)) => peer,
                Either::Right(_) => return Err(NativeMcpRuntimeError::Cancelled),
            };
            peer.close();
            let completion = peer.completion();
            drop(peer);
            let removed = {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
                if let Some(index) = state
                    .retired
                    .iter()
                    .position(|entry| Arc::ptr_eq(entry, &server))
                {
                    state.completions.push(completion);
                    Some(state.retired.remove(index))
                } else {
                    None
                }
            };
            drop(removed);
        }
    }

    fn for_turn(
        &self,
        registry: &Arc<McpSubmissionRegistry>,
    ) -> Result<Option<Arc<candidate::Publication>>> {
        registry
            .revalidate()
            .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| NativeMcpRuntimeError::Unavailable)?;
        if state.closed {
            return Err(NativeMcpRuntimeError::Unavailable);
        }
        // Retain upgraded registry owners until after unlocking: dropping them
        // may retire proof-bearing slots and invoke a reentrant waker.
        let owners: Vec<_> = state
            .turns
            .iter()
            .map(|pin| pin.registry.upgrade())
            .collect();
        let mut index = 0;
        state.turns.retain(|_| {
            let live = owners[index]
                .as_ref()
                .is_some_and(|owner| owner.revalidate().is_ok());
            index += 1;
            live
        });
        let weak = Arc::downgrade(registry);
        let result = if let Some(pin) = state.turns.iter().find(|pin| pin.registry.ptr_eq(&weak)) {
            pin.publication
                .upgrade()
                .ok_or(NativeMcpRuntimeError::Unavailable)
                .map(Some)
        } else if let Some(publication) = state.active.clone() {
            if state.turns.len() == 64 {
                Err(NativeMcpRuntimeError::Limit)
            } else {
                state.turns.push(TurnPin {
                    registry: weak,
                    publication: Arc::downgrade(&publication),
                });
                Ok(Some(publication))
            }
        } else {
            Ok(None)
        };
        drop(state);
        drop(owners);
        let publication = result?;
        if let Some(publication) = &publication {
            publication.check()?;
        }
        Ok(publication)
    }
}
impl Drop for NativeMcpRuntime {
    fn drop(&mut self) {
        self.close();
    }
}
impl McpToolCatalog for NativeMcpRuntime {
    fn snapshot(
        &self,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        Box::pin(async {
            Err(McpToolCatalogError::new(
                McpToolCatalogErrorKind::Unavailable,
            ))
        })
    }
    fn snapshot_for_turn(
        &self,
        context: ToolContext,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(McpToolCatalogError::new(McpToolCatalogErrorKind::Cancelled));
            }
            let context = self
                .contexts
                .snapshot_for_tool(&context)
                .map_err(|_| catalog_error())?;
            let registry = context.registry().map_err(|_| catalog_error())?;
            self.activate_for_turn(&context, &registry, &cancellation)
                .await
                .map_err(|error| {
                    McpToolCatalogError::new(if error == NativeMcpRuntimeError::Cancelled {
                        McpToolCatalogErrorKind::Cancelled
                    } else {
                        McpToolCatalogErrorKind::Unavailable
                    })
                })?;
            let publication = self.for_turn(&registry).map_err(|_| catalog_error())?;
            Ok(
                publication.map_or_else(McpToolCatalogSnapshot::discovering, |value| {
                    value.snapshot.clone()
                }),
            )
        })
    }
}
fn catalog_error() -> McpToolCatalogError {
    McpToolCatalogError::new(McpToolCatalogErrorKind::Unavailable)
}
