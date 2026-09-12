use super::{
    NativeMcpOwnedPeer, NativeMcpRuntime, NativeMcpRuntimeError as Error, Result,
    route::{ServerRoute, ToolRoute},
};
use crate::{
    McpToolCatalogSnapshot, McpToolMetadata,
    mcp::{
        catalog::{
            McpCatalogCandidate, McpCatalogServerInput, McpDescriptor, McpDescriptorCatalog,
            McpDescriptorLimits, McpToolEligibility, McpToolExposureDecision,
            McpToolExposurePolicy,
        },
        protocol::{ProtocolVersion, TransportKind},
        submission::{McpSubmissionRuntimeBinding, McpSubmissionRuntimeOwner, McpToolRequest},
    },
};
use machine_god_core::{CancellationToken, ToolName, ToolSpec};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

/// Complete caller-owned server candidate. Names and identity bytes are supplied
/// only by explicit native configuration/authentication admission, never a tool.
pub struct NativeMcpServerCandidate {
    pub server: Arc<str>,
    pub configuration: Arc<[u8]>,
    pub authentication: Arc<[u8]>,
    pub catalogs: Vec<McpDescriptorCatalog>,
    /// Explicit monotonic origin for every relative catalog timestamp.
    pub catalog_epoch: std::time::Instant,
    pub peer: NativeMcpOwnedPeer,
    /// Exact configured operation timeout, independent of other servers.
    pub operation_timeout: std::time::Duration,
    /// Explicit host/configuration/network/authentication generation observers.
    /// Shared unchanged by every executable binding from this server.
    pub authority_cancellations: Arc<[CancellationToken]>,
}
impl NativeMcpServerCandidate {
    fn validate_options(&self) -> Result<()> {
        if self.operation_timeout.is_zero()
            || self.operation_timeout > std::time::Duration::from_millis(u64::from(u32::MAX))
            || self.authority_cancellations.len()
                > crate::mcp::submission::MAX_MCP_RUNTIME_CANCELLATION_GUARDS
        {
            return Err(Error::Limit);
        }
        if self
            .authority_cancellations
            .iter()
            .any(CancellationToken::is_cancelled)
        {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
}
impl std::fmt::Debug for NativeMcpServerCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeMcpServerCandidate { <redacted> }")
    }
}
pub struct NativeMcpRuntimeCandidate {
    pub(super) publication: Arc<Publication>,
}
impl std::fmt::Debug for NativeMcpRuntimeCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeMcpRuntimeCandidate { <redacted> }")
    }
}
pub(super) struct Publication {
    pub identity: Arc<()>,
    pub servers: Vec<Arc<ServerRoute>>,
    pub tools: BTreeMap<ToolName, Arc<ToolRoute>>,
    pub snapshot: McpToolCatalogSnapshot,
    pub retired: Arc<AtomicBool>,
    pub descriptors: McpCatalogCandidate,
    pub retained_bytes: usize,
}
impl Publication {
    pub fn check(&self) -> Result<()> {
        if self.retired.load(Ordering::Acquire) {
            Err(Error::Unavailable)
        } else {
            Ok(())
        }
    }
}
impl NativeMcpRuntimeCandidate {
    #[must_use]
    pub fn descriptors(&self) -> &McpCatalogCandidate {
        &self.publication.descriptors
    }
    #[must_use]
    pub fn retained_byte_charge(&self) -> usize {
        self.publication.retained_bytes
    }

    /// Returns the selected server's catalog timestamp origin, not a freshness
    /// decision or an observation of the current clock.
    #[must_use]
    pub fn catalog_epoch(&self, server: &str) -> Option<std::time::Instant> {
        self.publication
            .servers
            .iter()
            .find(|route| route.name.as_ref() == server)
            .map(|route| route.catalog_epoch)
    }
}
impl NativeMcpRuntime {
    /// Builds all descriptors, bindings and executable registrations privately.
    /// No peer I/O or visibility change occurs; any rejection drops the complete
    /// candidate and preserves the active publication.
    /// # Errors
    /// Rejects incomplete/mismatched catalogs, name/schema/header admission and
    /// aggregate ownership budgets. No server is silently skipped.
    pub fn prepare_candidate(
        &self,
        mut servers: Vec<NativeMcpServerCandidate>,
        reserved: &[&str],
    ) -> Result<NativeMcpRuntimeCandidate> {
        let (descriptors, mut charge) = self.admit_descriptors(&servers, reserved)?;
        let mut owners = Vec::with_capacity(servers.len());
        let mut tools = BTreeMap::new();
        let mut metadata = Vec::with_capacity(descriptors.tools().len());
        for (index, server) in servers.drain(..).enumerate() {
            let selected: Vec<_> = descriptors
                .tools()
                .iter()
                .filter(|tool| tool.server_index() == index)
                .collect();
            let mut bindings = Vec::with_capacity(selected.len());
            for exposed in &selected {
                bindings.push(shared_binding(&server, exposed)?);
            }
            let mut peer = server.peer;
            peer.admit_runtimes(
                bindings
                    .iter()
                    .map(|(_, binding)| binding.clone())
                    .collect(),
            )?;
            let route = Arc::new(ServerRoute {
                name: server.server,
                catalog_epoch: server.catalog_epoch,
                protocol: peer.protocol(),
                peer: futures_util::lock::Mutex::new(peer),
                cancellation: CancellationToken::new(),
                pending: AtomicUsize::new(0),
                max_pending: self.limits.max_pending_operations,
                clock: self.clock.clone(),
                timeout: server.operation_timeout,
                authority_cancellations: server.authority_cancellations,
            });
            for (exposed, (owner, binding)) in selected.into_iter().zip(bindings) {
                let name = ToolName::new(exposed.name()).map_err(|_| Error::Invalid)?;
                let input_schema = machine_god_core::json::from_str(
                    exposed.descriptor().input_schema().raw_json(),
                )
                .map_err(|_| Error::Invalid)?;
                let parsed_charge = model_value_charge(&input_schema)?;
                let spec = ToolSpec {
                    name: name.clone(),
                    description: exposed.descriptor().effective_description().into(),
                    input_schema,
                };
                let tool = Arc::new(ToolRoute {
                    name: name.clone(),
                    spec,
                    descriptor: exposed.descriptor().clone(),
                    server: Arc::downgrade(&route),
                    binding,
                    owner,
                    contexts: self.contexts.clone(),
                    executor: self.executor.clone(),
                    policy: self.policy,
                });
                // Charge conservative per-tool native/map/registration overhead,
                // the parsed model schema, and duplicated searchable metadata.
                let spec_bytes = exposed.descriptor().input_schema().raw_json().len();
                let estimate = spec_bytes
                    .checked_mul(4)
                    .and_then(|n| n.checked_add(parsed_charge))
                    .and_then(|n| n.checked_add(exposed.search_text().len() * 2))
                    .and_then(|n| {
                        n.checked_add(exposed.descriptor().effective_description().len() * 2 + 2048)
                    })
                    .ok_or(Error::Limit)?;
                add_charge(&mut charge, estimate, self.limits.max_retained_bytes)?;
                let entry = McpToolMetadata::new(
                    exposed.name(),
                    route.name.as_ref(),
                    exposed.descriptor().effective_description(),
                    exposed.search_text(),
                    exposed.tags().iter().map(ToString::to_string).collect(),
                )
                .map_err(|_| Error::Limit)?
                .with_shared_tool(Arc::new(super::tool::RuntimeTool(tool.clone())))
                .map_err(|_| Error::Limit)?;
                tools.insert(name, tool);
                metadata.push(entry);
            }
            owners.push(route);
        }
        let snapshot = McpToolCatalogSnapshot::new(metadata).map_err(|_| Error::Limit)?;
        Ok(NativeMcpRuntimeCandidate {
            publication: Arc::new(Publication {
                identity: self.identity.clone(),
                servers: owners,
                tools,
                snapshot,
                retired: Arc::new(AtomicBool::new(false)),
                descriptors,
                retained_bytes: charge,
            }),
        })
    }
    fn admit_descriptors(
        &self,
        servers: &[NativeMcpServerCandidate],
        reserved: &[&str],
    ) -> Result<(McpCatalogCandidate, usize)> {
        if servers.len() > self.limits.max_servers {
            return Err(Error::Limit);
        }
        for server in servers {
            server.validate_options()?;
        }
        let policies: Vec<_> = servers
            .iter()
            .map(|server| {
                server
                    .catalogs
                    .iter()
                    .flat_map(McpDescriptorCatalog::descriptors)
                    .filter_map(|descriptor| {
                        if let McpDescriptor::Tool(tool) = descriptor {
                            Some(McpToolExposureDecision {
                                remote_name: tool.name(),
                                eligibility: if McpToolRequest::validate_modern_http_schema(
                                    tool.input_schema(),
                                )
                                .is_ok()
                                {
                                    McpToolEligibility::Admit
                                } else {
                                    McpToolEligibility::ExcludeModernHttpHeaders
                                },
                            })
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let inputs: Vec<_> = servers
            .iter()
            .zip(&policies)
            .map(|(server, decisions)| {
                let protocol = server.peer.protocol();
                McpCatalogServerInput {
                    server_name: &server.server,
                    catalogs: &server.catalogs,
                    tool_policy: if protocol.version == ProtocolVersion::Modern
                        && protocol.transport != TransportKind::Stdio
                    {
                        McpToolExposurePolicy::ModernHttp(decisions)
                    } else {
                        McpToolExposurePolicy::Standard
                    },
                }
            })
            .collect();
        for server in servers {
            if server
                .catalogs
                .iter()
                .any(|catalog| catalog.version() != server.peer.protocol().version)
                || server.peer.supports_tools()
                    && !server.catalogs.iter().any(|catalog| {
                        catalog.kind() == crate::mcp::pagination::McpCatalogKind::Tools
                    })
            {
                return Err(Error::Invalid);
            }
        }
        let descriptors =
            McpCatalogCandidate::build(&inputs, reserved, McpDescriptorLimits::default())
                .map_err(|_| Error::Invalid)?;
        if descriptors.tools().len() > self.limits.max_tools {
            return Err(Error::Limit);
        }
        let mut charge = descriptors.retained_byte_charge();
        for server in servers {
            add_charge(
                &mut charge,
                server.configuration.len(),
                self.limits.max_retained_bytes,
            )?;
            add_charge(
                &mut charge,
                server.authentication.len(),
                self.limits.max_retained_bytes,
            )?;
            add_charge(
                &mut charge,
                server.server.len() + 4096,
                self.limits.max_retained_bytes,
            )?;
        }
        Ok((descriptors, charge))
    }
}
fn shared_binding(
    server: &NativeMcpServerCandidate,
    exposed: &crate::mcp::catalog::McpExposedTool,
) -> Result<(
    McpSubmissionRuntimeOwner,
    Arc<crate::mcp::submission::McpSubmissionRuntime>,
)> {
    let name = ToolName::new(exposed.name()).map_err(|_| Error::Invalid)?;
    let binding = McpSubmissionRuntimeBinding::shared(
        server.server.clone(),
        name,
        Arc::from(exposed.descriptor().name()),
        server.configuration.clone(),
        exposed.descriptor().input_schema().clone(),
        server.authentication.clone(),
    )
    .map_err(|_| Error::Invalid)?;
    let owner = McpSubmissionRuntimeOwner::new();
    let binding = owner
        .install_guarded(binding, server.authority_cancellations.clone())
        .map_err(|_| Error::Unavailable)?;
    Ok((owner, binding))
}
fn add_charge(total: &mut usize, amount: usize, maximum: usize) -> Result<()> {
    *total = total.checked_add(amount).ok_or(Error::Limit)?;
    if *total > maximum {
        Err(Error::Limit)
    } else {
        Ok(())
    }
}
fn model_value_charge(value: &serde_json::Value) -> Result<usize> {
    let mut stack = vec![value];
    let mut count = 0usize;
    while let Some(value) = stack.pop() {
        count = count.checked_add(1).ok_or(Error::Limit)?;
        if count > 4096 {
            return Err(Error::Limit);
        }
        match value {
            serde_json::Value::Object(fields) => stack.extend(fields.values()),
            serde_json::Value::Array(values) => stack.extend(values),
            _ => {}
        }
    }
    // Two retained Value trees (Tool implementation and captured registration),
    // conservatively charging container/map/Value allocation overhead per node.
    count.checked_mul(256).ok_or(Error::Limit)
}
