use super::{
    Error, McpFeatureCodecLimits, McpTemplateMatchBudget, Result, charge, matches_resource_template,
};
use crate::mcp::{
    catalog::{
        McpDescriptor, McpDescriptorCatalog, McpPromptDescriptor, McpResourceDescriptor,
        McpResourceTemplateDescriptor,
    },
    pagination::McpCatalogKind,
    peer::McpPeerCapabilities,
    protocol::{McpClientMetadata, NegotiatedProtocol, ProtocolVersion, RpcId},
};
use crate::{McpFeatureAction as Action, McpFeatureRequest};
use serde::Serialize;
use serde_json::value::RawValue;
use std::fmt;

#[derive(Clone, Copy, Debug)]
pub struct McpFeatureExchangeOptions {
    pub(super) protocol: NegotiatedProtocol,
    pub(super) id: i64,
    capabilities: McpPeerCapabilities,
    progress: Option<u64>,
    form: bool,
    url: bool,
}
impl McpFeatureExchangeOptions {
    /// Uses peer-selected protocol data and a separately reserved request ID.
    /// # Errors
    /// Rejects negative IDs and unsupported protocol/transport pairs.
    pub fn new(
        protocol: NegotiatedProtocol,
        id: i64,
        capabilities: McpPeerCapabilities,
    ) -> Result<Self> {
        if id < 0
            || ProtocolVersion::parse_for(protocol.transport, protocol.version.as_str())
                != Some(protocol.version)
        {
            return Err(Error::InvalidRequest);
        }
        Ok(Self {
            protocol,
            id,
            capabilities,
            progress: None,
            form: false,
            url: false,
        })
    }
    #[must_use]
    pub const fn with_progress_token(mut self, token: u64) -> Self {
        self.progress = Some(token);
        self
    }
    /// Advertisements do not provide responders, permission or input consent.
    #[must_use]
    pub const fn with_elicitation(mut self, form: bool, url: bool) -> Self {
        self.form = form;
        self.url = url;
        self
    }
}

/// Exact immutable identity chosen from admitted descriptor data.
pub enum McpFeatureIdentity {
    Catalog(McpCatalogKind),
    Resource(McpResourceDescriptor),
    ResourceTemplate(McpResourceTemplateDescriptor),
    Prompt(McpPromptDescriptor),
}
impl fmt::Debug for McpFeatureIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpFeatureIdentity { .. }")
    }
}
pub struct McpFeatureExchange {
    pub(super) request: McpFeatureRequest,
    pub(super) options: McpFeatureExchangeOptions,
    pub(super) limits: McpFeatureCodecLimits,
    pub(super) cursor: Option<Box<str>>,
    identity: McpFeatureIdentity,
    wire: Box<RawValue>,
}
impl fmt::Debug for McpFeatureExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpFeatureExchange")
            .field("action", &self.request.action())
            .finish_non_exhaustive()
    }
}
impl McpFeatureExchange {
    /// Derives one fixed feature method from exact request and catalog data.
    /// `cursor` is legal only for list actions; it is never a generic parameter.
    /// # Errors
    /// Rejects identity/capability/version mismatches, unknown or missing required
    /// prompt arguments, absent catalogs and bounded template work exhaustion.
    pub fn prepare(
        request: &McpFeatureRequest,
        server: &str,
        catalogs: &[McpDescriptorCatalog],
        options: McpFeatureExchangeOptions,
        cursor: Option<&str>,
        limits: McpFeatureCodecLimits,
    ) -> Result<Self> {
        let limits = limits.validate()?;
        if request.server() != server
            || catalogs.len() > 4
            || cursor.is_some_and(|cursor| cursor.len() > 4096)
        {
            return Err(Error::InvalidRequest);
        }
        let action = request.action();
        if cursor.is_some() && list_kind(action).is_none() {
            return Err(Error::InvalidRequest);
        }
        let caps = options.capabilities;
        let available = match action {
            Action::ResourceList | Action::ResourceTemplates | Action::ResourceRead => {
                caps.resources()
            }
            Action::PromptList | Action::PromptGet => caps.prompts(),
            Action::PromptComplete => caps.prompts() && caps.completions(),
            Action::ResourceComplete => caps.resources() && caps.completions(),
        };
        if !available {
            return Err(Error::Unsupported);
        }
        let mut bytes = 0;
        for (index, catalog) in catalogs.iter().enumerate() {
            if catalog.version() != options.protocol.version
                || catalogs[..index]
                    .iter()
                    .any(|other| other.kind() == catalog.kind())
            {
                return Err(Error::InvalidRequest);
            }
            charge(
                &mut bytes,
                catalog.retained_byte_charge(),
                limits.max_retained_bytes,
            )?;
        }
        let identity = admit_identity(request, catalogs)?;
        let params = Params {
            request,
            cursor,
            metadata: McpClientMetadata::for_protocol(
                options.protocol.version,
                options.progress,
                options.form,
                options.url,
            ),
        };
        let wire = serde_json::to_string(&Envelope {
            jsonrpc: "2.0",
            id: options.id,
            method: method(action),
            params,
        })
        .map_err(|_| Error::InvalidRequest)?;
        // The request is already bounded to 64 KiB; typed metadata and the
        // cursor are finite additional fields, with no arbitrary-JSON input.
        if wire.len() > 128 * 1024 {
            return Err(Error::Limit);
        }
        charge(
            &mut bytes,
            wire.len().checked_mul(2).ok_or(Error::Limit)?,
            limits.max_retained_bytes,
        )?;
        charge(&mut bytes, 1024, limits.max_retained_bytes)?;
        // The cloned typed request retains sparse argument/context B-trees;
        // charge container slots separately from the serialized text budget.
        let entries = request
            .arguments()
            .len()
            .checked_add(request.context().len())
            .ok_or(Error::Limit)?;
        charge(
            &mut bytes,
            entries.checked_mul(1024).ok_or(Error::Limit)?,
            limits.max_retained_bytes,
        )?;
        let wire = RawValue::from_string(wire).map_err(|_| Error::InvalidRequest)?;
        Ok(Self {
            request: request.clone(),
            options,
            limits,
            cursor: cursor.map(Into::into),
            identity,
            wire,
        })
    }
    #[must_use]
    pub fn request(&self) -> &McpFeatureRequest {
        &self.request
    }
    #[must_use]
    pub fn identity(&self) -> &McpFeatureIdentity {
        &self.identity
    }
    #[must_use]
    pub fn request_id(&self) -> RpcId {
        RpcId::Integer(self.options.id)
    }
    #[must_use]
    pub fn protocol(&self) -> NegotiatedProtocol {
        self.options.protocol
    }
    #[must_use]
    pub fn method(&self) -> &'static str {
        method(self.request.action())
    }
    /// JSON request data only; a native peer requires separate effect authority.
    #[must_use]
    pub fn wire_json(&self) -> &RawValue {
        &self.wire
    }
}
pub(super) fn list_kind(action: Action) -> Option<McpCatalogKind> {
    match action {
        Action::ResourceList => Some(McpCatalogKind::Resources),
        Action::ResourceTemplates => Some(McpCatalogKind::ResourceTemplates),
        Action::PromptList => Some(McpCatalogKind::Prompts),
        _ => None,
    }
}
fn method(action: Action) -> &'static str {
    match action {
        Action::ResourceList => "resources/list",
        Action::ResourceTemplates => "resources/templates/list",
        Action::ResourceRead => "resources/read",
        Action::PromptList => "prompts/list",
        Action::PromptGet => "prompts/get",
        Action::PromptComplete | Action::ResourceComplete => "completion/complete",
    }
}
fn family(
    catalogs: &[McpDescriptorCatalog],
    kind: McpCatalogKind,
) -> Result<&McpDescriptorCatalog> {
    catalogs
        .iter()
        .find(|catalog| catalog.kind() == kind)
        .ok_or(Error::MissingCatalog)
}
fn admit_identity(
    request: &McpFeatureRequest,
    catalogs: &[McpDescriptorCatalog],
) -> Result<McpFeatureIdentity> {
    if let Some(kind) = list_kind(request.action()) {
        return Ok(McpFeatureIdentity::Catalog(kind));
    }
    let identity = request.identity().ok_or(Error::InvalidRequest)?;
    match request.action() {
        Action::ResourceRead => {
            for descriptor in family(catalogs, McpCatalogKind::Resources)?.descriptors() {
                if let McpDescriptor::Resource(resource) = descriptor
                    && resource.uri() == identity
                {
                    return Ok(McpFeatureIdentity::Resource(resource.clone()));
                }
            }
            let mut budget = McpTemplateMatchBudget::new(1024 * 1024)?;
            for descriptor in family(catalogs, McpCatalogKind::ResourceTemplates)?.descriptors() {
                if let McpDescriptor::ResourceTemplate(template) = descriptor
                    && matches_resource_template(template, identity, &mut budget)?
                {
                    return Ok(McpFeatureIdentity::ResourceTemplate(template.clone()));
                }
            }
            Err(Error::NotFound)
        }
        Action::ResourceComplete => family(catalogs, McpCatalogKind::ResourceTemplates)?
            .descriptors()
            .iter()
            .find_map(|descriptor| match descriptor {
                McpDescriptor::ResourceTemplate(template)
                    if template.uri_template() == identity =>
                {
                    Some(McpFeatureIdentity::ResourceTemplate(template.clone()))
                }
                _ => None,
            })
            .ok_or(Error::NotFound),
        Action::PromptGet | Action::PromptComplete => {
            let prompt = family(catalogs, McpCatalogKind::Prompts)?
                .descriptors()
                .iter()
                .find_map(|descriptor| match descriptor {
                    McpDescriptor::Prompt(prompt) if prompt.name() == identity => Some(prompt),
                    _ => None,
                })
                .ok_or(Error::NotFound)?;
            if request.action() == Action::PromptGet
                && (request
                    .arguments()
                    .keys()
                    .any(|name| prompt.argument_named(name).is_none())
                    || prompt.arguments().iter().any(|argument| {
                        argument.required() && !request.arguments().contains_key(argument.name())
                    }))
            {
                return Err(Error::InvalidRequest);
            }
            Ok(McpFeatureIdentity::Prompt(prompt.clone()))
        }
        _ => Err(Error::InvalidRequest),
    }
}
#[derive(Serialize)]
struct Envelope<'a> {
    jsonrpc: &'static str,
    id: i64,
    method: &'static str,
    params: Params<'a>,
}
struct Params<'a> {
    request: &'a McpFeatureRequest,
    cursor: Option<&'a str>,
    metadata: McpClientMetadata,
}
impl Serialize for Params<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("_meta", &self.metadata)?;
        let request = self.request;
        match request.action() {
            Action::ResourceList | Action::ResourceTemplates | Action::PromptList => {
                if let Some(cursor) = self.cursor {
                    map.serialize_entry("cursor", cursor)?;
                }
            }
            Action::ResourceRead => map.serialize_entry("uri", &request.identity())?,
            Action::PromptGet => {
                map.serialize_entry("name", &request.identity())?;
                map.serialize_entry("arguments", request.arguments())?;
            }
            Action::PromptComplete | Action::ResourceComplete => {
                #[derive(Serialize)]
                struct Reference<'a> {
                    #[serde(rename = "type")]
                    kind: &'static str,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    name: Option<&'a str>,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    uri: Option<&'a str>,
                }
                #[derive(Serialize)]
                struct Argument<'a> {
                    name: Option<&'a str>,
                    value: &'a str,
                }
                let prompt = request.action() == Action::PromptComplete;
                map.serialize_entry(
                    "ref",
                    &Reference {
                        kind: if prompt { "ref/prompt" } else { "ref/resource" },
                        name: prompt.then(|| request.identity()).flatten(),
                        uri: (!prompt).then(|| request.identity()).flatten(),
                    },
                )?;
                map.serialize_entry(
                    "argument",
                    &Argument {
                        name: request.argument(),
                        value: request.value(),
                    },
                )?;
                if !request.context().is_empty() {
                    #[derive(Serialize)]
                    struct Context<'a> {
                        arguments: &'a std::collections::BTreeMap<Box<str>, Box<str>>,
                    }
                    map.serialize_entry(
                        "context",
                        &Context {
                            arguments: request.context(),
                        },
                    )?;
                }
            }
        }
        map.end()
    }
}
