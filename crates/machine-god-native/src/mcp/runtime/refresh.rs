//! Conditional tool rebinding on the same serialized, already-owned peer.

use super::{
    NativeMcpPublicationCheckpoint, NativeMcpRuntime, NativeMcpRuntimeError as Error, Result,
    State,
    candidate::{Publication, add_charge},
    route::{PeerGuard, ServerRoute, ToolRoute},
};
use crate::{
    McpToolCatalogSnapshot,
    mcp::{
        catalog::{
            McpCatalogCandidate, McpCatalogServerInput, McpDescriptor, McpDescriptorCatalog,
            McpDescriptorLimits, McpToolEligibility, McpToolExposureDecision,
            McpToolExposurePolicy,
        },
        pagination::McpCatalogKind,
        protocol::TransportKind,
        submission::{McpSubmissionRuntime, McpToolRequest},
    },
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct NativeMcpToolRefresh {
    expected: NativeMcpPublicationCheckpoint,
    previous: Arc<Publication>,
    server: Arc<ServerRoute>,
    replacement: Option<Arc<Publication>>,
    runtimes: Vec<Arc<McpSubmissionRuntime>>,
    replaced: Vec<Arc<ToolRoute>>,
    retirement: RetiredCatalog,
}
impl NativeMcpToolRefresh {
    pub(super) fn is_changed(&self) -> bool {
        self.replacement.is_some()
    }
    pub(super) fn publication_checkpoint(&self) -> NativeMcpPublicationCheckpoint {
        NativeMcpPublicationCheckpoint::for_publication(
            self.replacement.as_ref().unwrap_or(&self.previous),
        )
    }
}

/// Weak accounting observes captured registrations and unsent submissions even
/// after their old publication itself has disappeared. It never retains peers.
pub(super) struct RetiredCatalog {
    publication: Weak<Publication>,
    previous: Option<Weak<Publication>>,
    tools: Vec<Weak<ToolRoute>>,
    bindings: Vec<Weak<McpSubmissionRuntime>>,
    charge: usize,
}
impl RetiredCatalog {
    fn live(&self) -> bool {
        self.publication.strong_count() != 0
            || self
                .previous
                .as_ref()
                .is_some_and(|view| view.strong_count() != 0)
            || self.tools.iter().any(|tool| tool.strong_count() != 0)
            || self
                .bindings
                .iter()
                .any(|binding| binding.strong_count() != 0)
    }
}
pub(super) fn retained_catalog_charge(state: &mut State) -> Result<usize> {
    state.retired_catalogs.retain(RetiredCatalog::live);
    state
        .retired_catalogs
        .iter()
        .try_fold(0usize, |total, record| {
            total.checked_add(record.charge).ok_or(Error::Limit)
        })
}

impl NativeMcpRuntime {
    /// Pure preparation against the exact active view. Caller keeps the selected
    /// peer lane from catalog fetching through cache prevalidation and commit.
    pub(super) fn prepare_tool_refresh(
        &self,
        expected: &NativeMcpPublicationCheckpoint,
        server: &Arc<ServerRoute>,
        catalog: McpDescriptorCatalog,
        reserved: &[&str],
    ) -> Result<NativeMcpToolRefresh> {
        let previous = {
            let state = self.state.lock().map_err(|_| Error::Unavailable)?;
            if state.closed {
                return Err(Error::Unavailable);
            }
            expected.check(self, &state)?;
            state.active.clone().ok_or(Error::Unavailable)?
        };
        previous.check()?;
        if !previous
            .servers
            .iter()
            .any(|route| Arc::ptr_eq(route, server))
            || catalog.kind() != McpCatalogKind::Tools
            || catalog.version() != server.protocol.version
        {
            return Err(Error::Invalid);
        }
        server.check_authority()?;
        let original = catalogs(&previous, &server.name)?;
        let names = reserved_for(&previous, server, reserved)?;
        let mut selected_catalogs = original.to_vec();
        selected_catalogs.retain(|item| item.kind() != McpCatalogKind::Tools);
        selected_catalogs.push(catalog.clone());
        let selected_segment = admit_segment(server, &selected_catalogs, &names)?;
        let unchanged = original
            .iter()
            .find(|item| item.kind() == McpCatalogKind::Tools)
            .is_some_and(|old| same_tools(old, &catalog))
            && selected_segment.tools().iter().all(|exposed| {
                machine_god_core::ToolName::new(exposed.name())
                    .ok()
                    .and_then(|name| previous.tools.get(&name))
                    .is_some_and(|tool| {
                        tool.server.ptr_eq(&Arc::downgrade(server))
                            && tool.descriptor.raw_json().get()
                                == exposed.descriptor().raw_json().get()
                    })
            });
        let mut prepared = NativeMcpToolRefresh {
            expected: expected.clone(),
            server: server.clone(),
            retirement: RetiredCatalog {
                publication: Arc::downgrade(&previous),
                previous: previous.previous.as_ref().map(Arc::downgrade),
                tools: Vec::new(),
                bindings: Vec::new(),
                charge: previous.retained_bytes,
            },
            previous,
            replacement: None,
            runtimes: Vec::new(),
            replaced: Vec::new(),
        };
        if unchanged {
            return Ok(prepared);
        }
        let previous = &prepared.previous;
        let mut segments = Vec::with_capacity(previous.servers.len());
        let mut tools = previous.tools.clone();
        tools.retain(|_, tool| !tool.server.ptr_eq(&Arc::downgrade(server)));
        let mut metadata: Vec<_> = previous
            .snapshot
            .tools()
            .iter()
            .filter(|entry| entry.server() != server.name.as_ref())
            .cloned()
            .collect();
        let mut charge = 4096;
        for route in &previous.servers {
            let selected = Arc::ptr_eq(route, server);
            let segment = if selected {
                selected_segment.clone()
            } else {
                let names = reserved_for(previous, route, reserved)?;
                admit_segment(route, catalogs(previous, &route.name)?, &names)?
            };
            add_charge(
                &mut charge,
                segment.retained_byte_charge(),
                self.limits.max_retained_bytes,
            )?;
            add_charge(
                &mut charge,
                route.configuration.len(),
                self.limits.max_retained_bytes,
            )?;
            add_charge(
                &mut charge,
                route.authentication.len(),
                self.limits.max_retained_bytes,
            )?;
            add_charge(
                &mut charge,
                route.name.len() + 4096,
                self.limits.max_retained_bytes,
            )?;
            if selected {
                for exposed in segment.tools() {
                    let (name, tool, entry) = self.prepare_tool(route, exposed)?;
                    if tools.insert(name, tool).is_some() {
                        return Err(Error::Invalid);
                    }
                    metadata.push(entry);
                }
            }
            segments.push(segment);
        }
        if tools.len() > self.limits.max_tools {
            return Err(Error::Limit);
        }
        for tool in tools.values() {
            add_charge(
                &mut charge,
                tool.retained_bytes,
                self.limits.max_retained_bytes,
            )?;
        }
        let snapshot = McpToolCatalogSnapshot::new(metadata).map_err(|_| Error::Limit)?;
        prepared.runtimes = tools
            .values()
            .filter(|tool| tool.server.ptr_eq(&Arc::downgrade(server)))
            .map(|tool| tool.binding.clone())
            .collect();
        prepared.replaced = previous
            .tools
            .values()
            .filter(|tool| tool.server.ptr_eq(&Arc::downgrade(server)))
            .cloned()
            .collect();
        prepared.retirement.tools = prepared.replaced.iter().map(Arc::downgrade).collect();
        prepared.retirement.bindings = prepared
            .replaced
            .iter()
            .map(|tool| Arc::downgrade(&tool.binding))
            .collect();
        prepared.replacement = Some(Arc::new(Publication {
            identity: self.identity.clone(),
            servers: previous.servers.clone(),
            tools,
            snapshot,
            retired: Arc::new(AtomicBool::new(false)),
            descriptors: segments.into_boxed_slice(),
            previous: None,
            deferred_sealed: previous.deferred_sealed,
            retained_bytes: charge,
        }));
        Ok(prepared)
    }

    /// No I/O and no suspension. Every fallible step precedes retirement. The
    /// transport guard supplies an infallible exact whitelist swap; callbacks
    /// and retired owner destruction occur only after the publication lock.
    pub(super) fn commit_tool_refresh(
        &self,
        mut refresh: NativeMcpToolRefresh,
        peer: &mut PeerGuard<'_>,
    ) -> Result<NativeMcpPublicationCheckpoint> {
        if !std::ptr::eq(peer.server, Arc::as_ptr(&refresh.server)) {
            return Err(Error::Invalid);
        }
        let prospective = refresh.publication_checkpoint();
        if refresh.replacement.is_none() {
            let state = self.state.lock().map_err(|_| Error::Unavailable)?;
            if state.closed {
                return Err(Error::Unavailable);
            }
            refresh.expected.check(self, &state)?;
            refresh.server.check_authority()?;
            return Ok(prospective);
        }
        let replacement = refresh.replacement.take().ok_or(Error::Invalid)?;
        let staged = peer
            .peer
            .prepare_runtime_set(std::mem::take(&mut refresh.runtimes))?;
        let mut deferred = Vec::with_capacity(refresh.replaced.len());
        let mut state = self.state.lock().map_err(|_| Error::Unavailable)?;
        if state.closed {
            return Err(Error::Unavailable);
        }
        refresh.expected.check(self, &state)?;
        for server in &replacement.servers {
            server.check_authority()?;
        }
        let retained = retained_catalog_charge(&mut state)?;
        if state.retired_catalogs.len() >= self.limits.max_retired_servers
            || retained
                .checked_add(refresh.retirement.charge)
                .and_then(|n| n.checked_add(state.retired_byte_charge))
                .and_then(|n| n.checked_add(replacement.retained_bytes))
                .is_none_or(|n| n > self.limits.max_retained_bytes)
        {
            return Err(Error::Limit);
        }
        state
            .retired_catalogs
            .try_reserve(1)
            .map_err(|_| Error::Limit)?;
        // From here onward every operation is infallible and allocation-free.
        for tool in &refresh.replaced {
            deferred.push(tool.owner.retire_deferred());
        }
        refresh.previous.retired.store(true, Ordering::Release);
        let old_whitelist = staged.commit();
        let old_publication = state.active.replace(replacement);
        state.retired_catalogs.push(refresh.retirement);
        drop(state);
        for retirement in deferred {
            retirement.complete();
        }
        old_whitelist.release();
        drop(old_publication);
        Ok(prospective)
    }
}

fn catalogs<'a>(publication: &'a Publication, server: &str) -> Result<&'a [McpDescriptorCatalog]> {
    publication
        .descriptors
        .iter()
        .flat_map(McpCatalogCandidate::servers)
        .find(|candidate| candidate.name() == server)
        .map(crate::mcp::catalog::McpCandidateServer::catalogs)
        .ok_or(Error::Invalid)
}
fn same_tools(left: &McpDescriptorCatalog, right: &McpDescriptorCatalog) -> bool {
    left.descriptors().len() == right.descriptors().len()
        && left
            .descriptors()
            .iter()
            .zip(right.descriptors())
            .all(|(left, right)| match (left, right) {
                (McpDescriptor::Tool(left), McpDescriptor::Tool(right)) => {
                    left.raw_json().get() == right.raw_json().get()
                }
                _ => false,
            })
}
fn reserved_for<'a>(
    previous: &'a Publication,
    server: &Arc<ServerRoute>,
    reserved: &'a [&str],
) -> Result<Vec<&'a str>> {
    let maximum = McpDescriptorLimits::default().max_reserved_names;
    if reserved.len() > maximum {
        return Err(Error::Limit);
    }
    let mut names: BTreeSet<_> = reserved.iter().copied().collect();
    names.extend(
        previous
            .tools
            .values()
            .filter(|tool| !tool.server.ptr_eq(&Arc::downgrade(server)))
            .map(|tool| tool.name.as_str()),
    );
    if names.len() > maximum {
        return Err(Error::Limit);
    }
    Ok(names.into_iter().collect())
}
fn admit_segment(
    route: &ServerRoute,
    catalogs: &[McpDescriptorCatalog],
    reserved: &[&str],
) -> Result<McpCatalogCandidate> {
    let decisions: Vec<_> = catalogs
        .iter()
        .flat_map(McpDescriptorCatalog::descriptors)
        .filter_map(|descriptor| match descriptor {
            McpDescriptor::Tool(tool) => Some(McpToolExposureDecision {
                remote_name: tool.name(),
                eligibility: if McpToolRequest::validate_modern_http_schema(tool.input_schema())
                    .is_ok()
                {
                    McpToolEligibility::Admit
                } else {
                    McpToolEligibility::ExcludeModernHttpHeaders
                },
            }),
            _ => None,
        })
        .collect();
    McpCatalogCandidate::build(
        &[McpCatalogServerInput {
            server_name: &route.name,
            catalogs,
            tool_policy: if route.protocol.transport == TransportKind::StreamableHttp {
                McpToolExposurePolicy::ModernHttp(&decisions)
            } else {
                McpToolExposurePolicy::Standard
            },
        }],
        reserved,
        McpDescriptorLimits::default(),
    )
    .map_err(|_| Error::Invalid)
}
