//! Per-server admitted catalogs and separately bounded lazy feature retention.

use super::{NativeMcpRuntimeError as Error, Result};
use crate::mcp::{
    catalog::McpDescriptorCatalog,
    catalog_refresh::{
        McpCatalogRefresh, McpRefreshDecision, McpRefreshGeneration, McpRefreshTicket,
    },
    pagination::McpCatalogKind,
    protocol::RpcId,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(super) struct FeatureCacheBudget {
    bytes: AtomicUsize,
    maximum: usize,
}
impl FeatureCacheBudget {
    pub(super) fn new(maximum: usize) -> Self {
        Self {
            bytes: AtomicUsize::new(0),
            maximum,
        }
    }
    fn reserve(self: &Arc<Self>, bytes: usize) -> Option<Charge> {
        self.bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(bytes)
                    .filter(|total| *total <= self.maximum)
            })
            .ok()?;
        Some(Charge {
            budget: self.clone(),
            bytes,
        })
    }
}
struct Charge {
    budget: Arc<FeatureCacheBudget>,
    bytes: usize,
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
struct Cached {
    catalog: McpDescriptorCatalog,
    _charge: Option<Charge>,
}

/// Admitted metadata and cache state, not permission or a peer lifetime grant.
/// Startup transfers this state with its exact owned peer after subscription ACK.
pub struct NativeMcpCatalogState {
    pub(super) generation: McpRefreshGeneration,
    pub(super) policy: McpCatalogRefresh,
    catalogs: [Option<Cached>; 4],
    budget: Option<Arc<FeatureCacheBudget>>,
    pub(super) subscription: Option<RpcId>,
    pub(super) acknowledged: bool,
    pub(super) uris: Vec<Box<str>>,
}
impl std::fmt::Debug for NativeMcpCatalogState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeMcpCatalogState { <redacted> }")
    }
}
impl NativeMcpCatalogState {
    /// Constructs inert state from the exact initial admitted catalog allocations.
    /// # Errors
    /// Rejects duplicate families and more than four catalogs.
    pub fn new(catalogs: &[McpDescriptorCatalog]) -> Result<Self> {
        let generation = McpRefreshGeneration::new();
        let policy =
            McpCatalogRefresh::new(generation.clone(), catalogs).map_err(|_| Error::Invalid)?;
        let mut selected = std::array::from_fn(|_| None);
        for catalog in catalogs {
            selected[index(catalog.kind())] = Some(Cached {
                catalog: catalog.clone(),
                _charge: None,
            });
        }
        Ok(Self {
            generation,
            policy,
            catalogs: selected,
            budget: None,
            subscription: None,
            acknowledged: false,
            uris: Vec::new(),
        })
    }
    pub(super) fn initial_matches(&self, catalogs: &[McpDescriptorCatalog]) -> bool {
        self.catalogs.iter().flatten().count() == catalogs.len()
            && catalogs.iter().all(|catalog| {
                self.catalogs[index(catalog.kind())]
                    .as_ref()
                    .is_some_and(|entry| entry.catalog.same_allocation(catalog))
            })
    }
    pub(super) fn bind_budget(&mut self, budget: Arc<FeatureCacheBudget>) {
        self.budget = Some(budget);
    }
    pub(super) fn cached(&self, kind: McpCatalogKind) -> Option<McpDescriptorCatalog> {
        self.catalogs[index(kind)]
            .as_ref()
            .map(|entry| entry.catalog.clone())
    }
    pub(super) fn begin(&mut self, kind: McpCatalogKind, now: u64) -> Result<McpRefreshDecision> {
        self.policy
            .begin(&self.generation, kind, now)
            .map_err(|_| Error::Unavailable)
    }
    pub(super) fn fail(&mut self, ticket: McpRefreshTicket, now: u64) -> Result<()> {
        self.policy
            .fail(ticket, now)
            .map_err(|_| Error::Unavailable)
    }
    pub(super) fn validate(
        &self,
        ticket: &McpRefreshTicket,
        catalog: &McpDescriptorCatalog,
        now: u64,
    ) -> Result<()> {
        self.policy
            .validate_replacement(ticket, catalog, now)
            .map_err(|_| Error::Unavailable)
    }
    /// Tool payloads are charged by executable publication. Lazy features have
    /// an additional shared budget; pressure preserves the old snapshot/backoff
    /// while the caller may still use its fresh independently bounded result.
    pub(super) fn finish(
        &mut self,
        ticket: McpRefreshTicket,
        catalog: &McpDescriptorCatalog,
        now: u64,
    ) -> Result<bool> {
        self.validate(&ticket, catalog, now)?;
        let charge = if catalog.kind() == McpCatalogKind::Tools {
            None
        } else {
            let Some(charge) = self
                .budget
                .as_ref()
                .and_then(|budget| budget.reserve(catalog.retained_byte_charge()))
            else {
                self.fail(ticket, now)?;
                return Ok(false);
            };
            Some(charge)
        };
        self.policy
            .finish(ticket, catalog, now)
            .map_err(|_| Error::Unavailable)?;
        self.catalogs[index(catalog.kind())] = Some(Cached {
            catalog: catalog.clone(),
            _charge: charge,
        });
        Ok(true)
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
