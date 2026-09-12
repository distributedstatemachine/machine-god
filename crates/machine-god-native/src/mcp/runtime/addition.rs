//! One conditional deferred batch, without replacing existing executable routes.

use super::{
    NativeMcpPublicationCheckpoint, NativeMcpRuntime, NativeMcpRuntimeError as Error,
    NativeMcpServerCandidate, Result, candidate::Publication,
};
use crate::{McpToolCatalogSnapshot, mcp::catalog::McpDescriptorLimits};
use std::{collections::BTreeSet, fmt, sync::Arc};

/// Opaque unpublished addition bound to the exact observed active publication.
/// It cannot be submitted through ordinary replacement publication APIs.
pub struct NativeMcpRuntimeAddition {
    publication: Arc<Publication>,
    expected: NativeMcpPublicationCheckpoint,
}
impl fmt::Debug for NativeMcpRuntimeAddition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpRuntimeAddition { <redacted> }")
    }
}
impl NativeMcpRuntimeAddition {
    /// Observes the exact prospective merged view without publishing or retaining
    /// it. This avoids a racy current-publication lookup after successful append.
    #[must_use]
    pub fn publication_checkpoint(&self) -> NativeMcpPublicationCheckpoint {
        NativeMcpPublicationCheckpoint::for_publication(&self.publication)
    }

    /// Conservative retained charge of the complete two-view lineage.
    #[must_use]
    pub fn retained_byte_charge(&self) -> usize {
        self.publication.retained_bytes
    }
}

impl NativeMcpRuntime {
    /// Prepares one nonempty deferred batch against an exact active publication.
    /// Existing peers, bindings, names and captured registrations are shared, not
    /// reopened or rebound. No peer I/O, cancellation or publication occurs.
    ///
    /// # Errors
    /// Rejects empty batches, missing/foreign/stale publications, duplicate
    /// servers, a second addition in this replacement lineage, revoked authority,
    /// and exhausted merged name/tool/server/storage bounds. Controllers treat
    /// an empty deferred startup batch as a no-op instead of calling this method.
    pub fn prepare_addition(
        &self,
        servers: Vec<NativeMcpServerCandidate>,
        reserved: &[&str],
        expected: &NativeMcpPublicationCheckpoint,
    ) -> Result<NativeMcpRuntimeAddition> {
        if servers.is_empty() {
            return Err(Error::Invalid);
        }
        let previous = self.addition_base(expected)?;
        validate_servers(&previous, &servers, self.limits.max_servers)?;
        let reserved = reserved_names(&previous, reserved)?;
        let prepared = self.prepare_candidate(servers, &reserved)?;
        let next = prepared.publication;
        if previous.tools.len() + next.tools.len() > self.limits.max_tools {
            return Err(Error::Limit);
        }
        // Both original views and the merged metadata/map/vector copies are
        // conservatively covered before any merged allocation is made. Existing
        // per-tool charges include owned names/search/tags and map overhead.
        let retained_bytes = previous
            .retained_bytes
            .checked_add(next.retained_bytes)
            .and_then(|bytes| bytes.checked_mul(2))
            .and_then(|bytes| bytes.checked_add(4096))
            .filter(|bytes| *bytes <= self.limits.max_retained_bytes)
            .ok_or(Error::Limit)?;
        let publication = merge(previous, &next, retained_bytes)?;
        Ok(NativeMcpRuntimeAddition {
            publication: Arc::new(publication),
            expected: expected.clone(),
        })
    }

    fn addition_base(&self, expected: &NativeMcpPublicationCheckpoint) -> Result<Arc<Publication>> {
        let state = self.state.lock().map_err(|_| Error::Unavailable)?;
        if state.closed {
            return Err(Error::Unavailable);
        }
        expected.check(self, &state)?;
        let previous = state.active.clone().ok_or(Error::Unavailable)?;
        drop(state);
        if previous.previous.is_some() {
            return Err(Error::Limit);
        }
        previous.check()?;
        for server in &previous.servers {
            server.check_authority()?;
        }
        Ok(previous)
    }

    /// Atomically exposes a prepared addition only while its checkpoint is exact.
    /// Old turns retain their original catalog; new turns see the merged catalog.
    /// No existing binding is retired, peer cancelled, or name reassigned.
    ///
    /// # Errors
    /// Rejects closed/foreign/stale candidates, revoked existing or new server
    /// guards, and active-plus-retired storage overflow before changing state.
    pub fn publish_addition(&self, addition: NativeMcpRuntimeAddition) -> Result<()> {
        let candidate = addition.publication;
        let mut state = self.state.lock().map_err(|_| Error::Unavailable)?;
        if state.closed {
            return Err(Error::Unavailable);
        }
        addition.expected.check(self, &state)?;
        candidate.check()?;
        for server in &candidate.servers {
            server.check_authority()?;
        }
        if state
            .retired_byte_charge
            .checked_add(candidate.retained_bytes)
            .is_none_or(|bytes| bytes > self.limits.max_retained_bytes)
        {
            return Err(Error::Limit);
        }
        let previous = state.active.replace(candidate);
        drop(state);
        drop(previous);
        Ok(())
    }
}

fn validate_servers(
    previous: &Publication,
    additions: &[NativeMcpServerCandidate],
    maximum: usize,
) -> Result<()> {
    if previous.servers.len() + additions.len() > maximum {
        return Err(Error::Limit);
    }
    let mut names: BTreeSet<_> = previous
        .servers
        .iter()
        .map(|server| server.name.as_ref())
        .collect();
    for server in additions {
        if !names.insert(server.server.as_ref()) {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}

fn reserved_names<'a>(previous: &'a Publication, reserved: &'a [&str]) -> Result<Vec<&'a str>> {
    let maximum = McpDescriptorLimits::default().max_reserved_names;
    if reserved.len() > maximum || previous.tools.len() > maximum {
        return Err(Error::Limit);
    }
    let mut names: BTreeSet<_> = reserved.iter().copied().collect();
    names.extend(
        previous
            .tools
            .keys()
            .map(machine_god_core::ToolName::as_str),
    );
    if names.len() > maximum {
        return Err(Error::Limit);
    }
    Ok(names.into_iter().collect())
}

fn merge(
    previous: Arc<Publication>,
    next: &Publication,
    retained_bytes: usize,
) -> Result<Publication> {
    let snapshot = McpToolCatalogSnapshot::new(
        previous
            .snapshot
            .tools()
            .iter()
            .chain(next.snapshot.tools())
            .cloned()
            .collect(),
    )
    .map_err(|_| Error::Limit)?;
    Ok(Publication {
        identity: previous.identity.clone(),
        servers: previous
            .servers
            .iter()
            .chain(&next.servers)
            .cloned()
            .collect(),
        tools: previous
            .tools
            .iter()
            .chain(&next.tools)
            .map(|(name, tool)| (name.clone(), tool.clone()))
            .collect(),
        snapshot,
        retired: previous.retired.clone(),
        descriptors: previous
            .descriptors
            .iter()
            .chain(next.descriptors.iter())
            .cloned()
            .collect(),
        previous: Some(previous),
        retained_bytes,
    })
}
