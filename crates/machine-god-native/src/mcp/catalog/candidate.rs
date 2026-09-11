use super::{
    McpCatalogError as Error, McpCatalogKind, McpDescriptor, McpDescriptorCatalog,
    McpDescriptorLimits, McpToolDescriptor, ProtocolVersion, Result, charge, names,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

/// Caller supplies the shared modern-HTTP header validator's data-only outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpToolEligibility {
    Admit,
    ExcludeModernHttpHeaders,
}
pub struct McpToolExposureDecision<'a> {
    pub remote_name: &'a str,
    pub eligibility: McpToolEligibility,
}
pub enum McpToolExposurePolicy<'a> {
    Standard,
    /// Exactly one decision for every tool; no unknown or duplicate names.
    ModernHttp(&'a [McpToolExposureDecision<'a>]),
}
pub struct McpCatalogServerInput<'a> {
    pub server_name: &'a str,
    pub catalogs: &'a [McpDescriptorCatalog],
    pub tool_policy: McpToolExposurePolicy<'a>,
}
pub struct McpCandidateServer {
    name: Box<str>,
    catalogs: Box<[McpDescriptorCatalog]>,
}
pub struct McpExposedTool {
    server_index: usize,
    name: Box<str>,
    descriptor: McpToolDescriptor,
    tags: Box<[Box<str>]>,
    search_text: Box<str>,
}
pub struct McpExcludedTool {
    server_index: usize,
    descriptor: McpToolDescriptor,
}
/// An explicit full-fidelity model-tool view; not the legacy bounded `ToolSpec`.
pub struct McpToolModelProjection<'a> {
    server: &'a str,
    tool: &'a McpExposedTool,
}
#[derive(Clone)]
pub struct McpCatalogCandidate(Arc<Candidate>);
struct Candidate {
    servers: Box<[McpCandidateServer]>,
    tools: Box<[McpExposedTool]>,
    exclusions: Box<[McpExcludedTool]>,
    retained_bytes: usize,
}
macro_rules! redacted {
    ($($name:ident $(<$lifetime:lifetime>)?),+ $(,)?) => { $(
        impl fmt::Debug for $name $(<$lifetime>)? {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), " { .. }"))
            }
        }
    )+ };
}
redacted!(
    McpToolExposureDecision<'_>,
    McpToolExposurePolicy<'_>,
    McpCatalogServerInput<'_>,
    McpCandidateServer,
    McpExposedTool,
    McpExcludedTool,
    McpToolModelProjection<'_>,
    McpCatalogCandidate
);

impl McpCatalogCandidate {
    /// Builds a whole replacement candidate without mutating supplied snapshots
    /// or reserved names. Input order is configuration order, never map order.
    ///
    /// # Errors
    /// Invalid families, eligibility, names or exhausted aggregate budgets reject
    /// the complete candidate. Callers retain the previous usable runtime.
    pub fn build(
        inputs: &[McpCatalogServerInput<'_>],
        reserved: &[&str],
        limits: McpDescriptorLimits,
    ) -> Result<Self> {
        let limits = limits.validate()?;
        let mut retained_bytes = preflight(inputs, reserved, limits)?;
        let mut allocator = names::Names::new(reserved, limits.max_name_attempts);
        let mut tools = Vec::new();
        let mut exclusions = Vec::new();
        let mut servers = Vec::with_capacity(inputs.len());
        for (server_index, input) in inputs.iter().enumerate() {
            let decisions = eligibility(input)?;
            for catalog in input.catalogs {
                for descriptor in catalog.descriptors() {
                    let McpDescriptor::Tool(descriptor) = descriptor else {
                        continue;
                    };
                    if decisions.get(descriptor.name())
                        == Some(&McpToolEligibility::ExcludeModernHttpHeaders)
                    {
                        exclusions.push(McpExcludedTool {
                            server_index,
                            descriptor: descriptor.clone(),
                        });
                        continue;
                    }
                    let tags = names::tags(input.server_name, descriptor.name());
                    let search_bytes = input.server_name.len()
                        + descriptor.name().len()
                        + descriptor.effective_description().len()
                        + descriptor.input_schema().raw_json().len()
                        + 3
                        + tags.iter().map(|tag| tag.len() + 1).sum::<usize>();
                    charge(
                        &mut retained_bytes,
                        search_bytes + 64 + tags.iter().map(|tag| tag.len()).sum::<usize>(),
                        limits.max_candidate_bytes,
                    )?;
                    let canonical_schema = canonical_schema(descriptor.input_schema().raw_json())?;
                    let name = allocator.allocate(input.server_name, descriptor.name())?;
                    let mut search = String::with_capacity(search_bytes);
                    for (index, value) in [
                        input.server_name,
                        descriptor.name(),
                        descriptor.effective_description(),
                        &canonical_schema,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if index != 0 {
                            search.push(' ');
                        }
                        search.push_str(value);
                    }
                    for tag in &tags {
                        search.push(' ');
                        search.push_str(tag);
                    }
                    search.make_ascii_lowercase();
                    tools.push(McpExposedTool {
                        server_index,
                        name,
                        descriptor: descriptor.clone(),
                        tags,
                        search_text: search.into_boxed_str(),
                    });
                }
            }
            servers.push(McpCandidateServer {
                name: input.server_name.into(),
                catalogs: input.catalogs.to_vec().into_boxed_slice(),
            });
        }
        Ok(Self(Arc::new(Candidate {
            servers: servers.into_boxed_slice(),
            tools: tools.into_boxed_slice(),
            exclusions: exclusions.into_boxed_slice(),
            retained_bytes,
        })))
    }
    #[must_use]
    pub fn servers(&self) -> &[McpCandidateServer] {
        &self.0.servers
    }
    #[must_use]
    pub fn tools(&self) -> &[McpExposedTool] {
        &self.0.tools
    }
    #[must_use]
    pub fn exclusions(&self) -> &[McpExcludedTool] {
        &self.0.exclusions
    }
    #[must_use]
    pub fn retained_byte_charge(&self) -> usize {
        self.0.retained_bytes
    }
    /// Exact lookup never treats a foreign candidate's tool as this one's data.
    #[must_use]
    pub fn model_projection(&self, exposed_name: &str) -> Option<McpToolModelProjection<'_>> {
        let tool = self
            .0
            .tools
            .iter()
            .find(|tool| tool.name() == exposed_name)?;
        Some(McpToolModelProjection {
            server: self.0.servers[tool.server_index].name(),
            tool,
        })
    }
}
fn canonical_schema(raw: &str) -> Result<String> {
    // Schema admission already bounded bytes/depth/nodes. The shared decoder
    // preserves literal private-looking keys and exact numeric token spellings.
    let value = machine_god_core::json::from_str(raw).map_err(|_| Error::InvalidDescriptor)?;
    let compact = serde_json::to_string(&value).map_err(|_| Error::InvalidDescriptor)?;
    // Compact key/string encoding cannot exceed the original valid JSON spelling;
    // retain the conservative preflight charge and verify it before retention.
    if compact.len() > raw.len() {
        return Err(Error::Limit);
    }
    Ok(compact)
}
fn preflight(
    inputs: &[McpCatalogServerInput<'_>],
    reserved: &[&str],
    limits: McpDescriptorLimits,
) -> Result<usize> {
    if inputs.len() > limits.max_servers || reserved.len() > limits.max_reserved_names {
        return Err(Error::Limit);
    }
    let mut retained = 0;
    for name in reserved {
        if !names::valid_reserved(name) {
            return Err(Error::InvalidReservedName);
        }
        charge(&mut retained, name.len(), limits.max_candidate_bytes)?;
    }
    let mut servers = BTreeSet::new();
    for input in inputs {
        if input.server_name.is_empty()
            || input.server_name.len() > 128
            || !input
                .server_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        {
            return Err(Error::InvalidServer);
        }
        if !servers.insert(input.server_name) {
            return Err(Error::DuplicateServer);
        }
        charge(
            &mut retained,
            input.server_name.len(),
            limits.max_candidate_bytes,
        )?;
        if input.catalogs.len() > 4 {
            return Err(Error::DuplicateFamily);
        }
        let mut families = Vec::new();
        for catalog in input.catalogs {
            let maximum = if catalog.kind() == McpCatalogKind::Tools {
                limits.max_tools
            } else {
                limits.max_features
            };
            if catalog.descriptors().len() > maximum {
                return Err(Error::Limit);
            }
            if families.contains(&catalog.kind()) {
                return Err(Error::DuplicateFamily);
            }
            families.push(catalog.kind());
            if input.catalogs[0].version() != catalog.version() {
                return Err(Error::ProtocolMismatch);
            }
            charge(
                &mut retained,
                catalog.retained_byte_charge(),
                limits.max_candidate_bytes,
            )?;
        }
        if let McpToolExposurePolicy::ModernHttp(decisions) = &input.tool_policy {
            if decisions.len() > limits.max_tools
                || input
                    .catalogs
                    .iter()
                    .any(|catalog| catalog.version() != ProtocolVersion::Modern)
            {
                return Err(Error::InvalidEligibility);
            }
            for decision in *decisions {
                if decision.remote_name.is_empty() || decision.remote_name.len() > 256 {
                    return Err(Error::InvalidEligibility);
                }
                charge(
                    &mut retained,
                    decision.remote_name.len(),
                    limits.max_candidate_bytes,
                )?;
            }
        }
    }
    Ok(retained)
}
fn eligibility<'a>(
    input: &'a McpCatalogServerInput<'_>,
) -> Result<BTreeMap<&'a str, McpToolEligibility>> {
    let McpToolExposurePolicy::ModernHttp(decisions) = &input.tool_policy else {
        return Ok(BTreeMap::new());
    };
    let mut result = BTreeMap::new();
    for decision in *decisions {
        if result
            .insert(decision.remote_name, decision.eligibility)
            .is_some()
        {
            return Err(Error::InvalidEligibility);
        }
    }
    let tools = input
        .catalogs
        .iter()
        .filter(|catalog| catalog.kind() == McpCatalogKind::Tools)
        .flat_map(McpDescriptorCatalog::descriptors)
        .filter_map(|descriptor| {
            if let McpDescriptor::Tool(tool) = descriptor {
                Some(tool)
            } else {
                None
            }
        });
    let mut count = 0;
    for tool in tools {
        if !result.contains_key(tool.name()) {
            return Err(Error::InvalidEligibility);
        }
        count += 1;
    }
    if count != result.len() {
        return Err(Error::InvalidEligibility);
    }
    Ok(result)
}
impl McpCandidateServer {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn catalogs(&self) -> &[McpDescriptorCatalog] {
        &self.catalogs
    }
}
impl McpExposedTool {
    #[must_use]
    pub fn server_index(&self) -> usize {
        self.server_index
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn descriptor(&self) -> &McpToolDescriptor {
        &self.descriptor
    }
    #[must_use]
    pub fn tags(&self) -> &[Box<str>] {
        &self.tags
    }
    #[must_use]
    pub fn search_text(&self) -> &str {
        &self.search_text
    }
}
impl McpExcludedTool {
    #[must_use]
    pub fn server_index(&self) -> usize {
        self.server_index
    }
    #[must_use]
    pub fn descriptor(&self) -> &McpToolDescriptor {
        &self.descriptor
    }
    #[must_use]
    pub fn reason(&self) -> McpToolEligibility {
        McpToolEligibility::ExcludeModernHttpHeaders
    }
}
impl McpToolModelProjection<'_> {
    #[must_use]
    pub fn name(&self) -> &str {
        self.tool.name()
    }
    #[must_use]
    pub fn server(&self) -> &str {
        self.server
    }
    #[must_use]
    pub fn description(&self) -> &str {
        self.tool.descriptor.effective_description()
    }
    #[must_use]
    pub fn input_schema(&self) -> &crate::mcp::schema::McpSchema {
        self.tool.descriptor.input_schema()
    }
    #[must_use]
    pub fn tags(&self) -> &[Box<str>] {
        self.tool.tags()
    }
}
