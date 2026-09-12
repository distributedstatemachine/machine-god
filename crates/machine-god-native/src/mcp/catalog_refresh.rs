//! Caller-driven catalog freshness and modern subscription admission.
//!
//! Decisions are data, never permission to connect, publish or replay a call.
use super::{catalog::McpDescriptorCatalog, pagination::McpCatalogKind};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

mod notification;
mod state;
mod subscription;
#[cfg(test)]
mod tests;

pub use notification::{McpRefreshNotification, McpSubscriptionFilters};
pub use subscription::validate_subscription_response;

/// Opaque policy partition identity, not a runtime publication or effect grant.
#[derive(Clone, Default)]
pub struct McpRefreshGeneration(Arc<()>);
impl McpRefreshGeneration {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(()))
    }
    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl fmt::Debug for McpRefreshGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpRefreshGeneration { <opaque> }")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpRefreshError {
    Invalid,
    Foreign,
    ClockRegression,
    Exhausted,
    Closed,
}
impl fmt::Display for McpRefreshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP catalog refresh policy failed")
    }
}
impl std::error::Error for McpRefreshError {}
type Result<T> = std::result::Result<T, McpRefreshError>;

#[derive(Debug)]
pub enum McpRefreshDecision {
    Hit,
    RetryLater { may_serve_snapshot: bool },
    AlreadyRefreshing { may_serve_snapshot: bool },
    Refresh(McpRefreshTicket),
}

/// One in-flight decision. Drop releases only its inert coalescing marker.
pub struct McpRefreshTicket {
    generation: McpRefreshGeneration,
    kind: McpCatalogKind,
    active: Arc<AtomicBool>,
    invalidation: u64,
    may_serve_snapshot: bool,
}
impl McpRefreshTicket {
    /// Reports retained same-partition data, not continuing runtime authority.
    #[must_use]
    pub const fn may_serve_snapshot(&self) -> bool {
        self.may_serve_snapshot
    }
}
impl fmt::Debug for McpRefreshTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpRefreshTicket")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl Drop for McpRefreshTicket {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
struct CacheTimes {
    fetched: u64,
    expires: u64,
}
#[derive(Default)]
struct Family {
    times: Option<CacheTimes>,
    invalidation: u64,
    handled: u64,
    attempt: u8,
    retry_at: Option<u64>,
    active: Option<Arc<AtomicBool>>,
}
struct Subscription {
    id: i64,
    acknowledged: bool,
    filters: McpSubscriptionFilters,
}

pub struct McpCatalogRefresh {
    generation: McpRefreshGeneration,
    families: [Family; 4],
    last_now: u64,
    closed: bool,
    subscription: Option<Subscription>,
    last_subscription_id: Option<i64>,
    resource_reads: u64,
    handled_resource_reads: u64,
}
impl fmt::Debug for McpCatalogRefresh {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpCatalogRefresh { <redacted> }")
    }
}
impl McpCatalogRefresh {
    /// Retains only admitted timestamps, not descriptors or catalog storage.
    /// Construction reads no clock and starts no transport or refresh work.
    /// # Errors
    /// Rejects duplicate catalog families or more than four catalogs.
    pub fn new(
        generation: McpRefreshGeneration,
        catalogs: &[McpDescriptorCatalog],
    ) -> Result<Self> {
        if catalogs.len() > 4 {
            return Err(McpRefreshError::Invalid);
        }
        let mut policy = Self {
            generation,
            families: std::array::from_fn(|_| Family::default()),
            last_now: 0,
            closed: false,
            subscription: None,
            last_subscription_id: None,
            resource_reads: 0,
            handled_resource_reads: 0,
        };
        for catalog in catalogs {
            let family = &mut policy.families[index(catalog.kind())];
            if family.times.is_some() {
                return Err(McpRefreshError::Invalid);
            }
            family.times = Some(times(catalog));
            policy.last_now = policy.last_now.max(catalog.fetched_at_ms());
        }
        Ok(policy)
    }

    fn check(&self, generation: &McpRefreshGeneration) -> Result<()> {
        if !self.generation.matches(generation) {
            return Err(McpRefreshError::Foreign);
        }
        if self.closed {
            return Err(McpRefreshError::Closed);
        }
        Ok(())
    }
    fn observe_time(&mut self, now: u64) -> Result<()> {
        if now < self.last_now {
            self.closed = true;
            return Err(McpRefreshError::ClockRegression);
        }
        self.last_now = now;
        Ok(())
    }

    /// A read-cache owner clears all read entries before acknowledging this
    /// generation. No per-URI result storage is retained here.
    /// # Errors
    /// Rejects foreign generations and closed policy state.
    pub fn resource_read_invalidation(
        &self,
        generation: &McpRefreshGeneration,
    ) -> Result<Option<u64>> {
        self.check(generation)?;
        Ok((self.resource_reads != self.handled_resource_reads).then_some(self.resource_reads))
    }
    /// Acknowledges only observed invalidations after the owner's cache clear.
    /// Later invalidations remain pending.
    /// # Errors
    /// Rejects foreign generations, closed state and unobserved counters.
    pub fn clear_resource_reads(
        &mut self,
        generation: &McpRefreshGeneration,
        through: u64,
    ) -> Result<()> {
        self.check(generation)?;
        if through > self.resource_reads {
            return Err(McpRefreshError::Invalid);
        }
        self.handled_resource_reads = self.handled_resource_reads.max(through);
        Ok(())
    }
}

fn index(kind: McpCatalogKind) -> usize {
    match kind {
        McpCatalogKind::Tools => 0,
        McpCatalogKind::Resources => 1,
        McpCatalogKind::ResourceTemplates => 2,
        McpCatalogKind::Prompts => 3,
    }
}
fn times(catalog: &McpDescriptorCatalog) -> CacheTimes {
    CacheTimes {
        fetched: catalog.fetched_at_ms(),
        expires: catalog.expires_at_ms(),
    }
}
