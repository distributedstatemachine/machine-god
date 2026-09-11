//! Typed schema-bound tool envelopes, frozen before native permission admission.

use super::{
    Arc, BoxFuture, CancellationToken, CopiedInvocation, CopiedRequest, Framing,
    MAX_MCP_SUBMISSION_REQUEST_BYTES, McpSubmissionError, McpSubmissionRegistry,
    McpSubmissionRuntime, PermissionInvocation, PermissionRequest, PreparedMcpSubmission, Result,
    RpcId, ToolCallId, ToolName, Value, bounded_json, canonical_arguments,
};
use crate::mcp::{
    protocol::{McpClientMetadata, NegotiatedProtocol, ProtocolVersion, TransportKind},
    schema::{McpSchema, McpSchemaValidation},
};
use serde::Serialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod headers;

/// Protocol data selected by an admitted peer. Capability advertisements do not
/// provide a responder, consent, permission, or continuation authority.
#[derive(Clone, Copy, Debug)]
pub struct McpToolCallOptions {
    protocol: NegotiatedProtocol,
    request_id: i64,
    progress_token: Option<u64>,
    form: bool,
    url: bool,
}
impl McpToolCallOptions {
    /// Uses a peer-reserved ID and an admitted protocol/transport combination.
    ///
    /// # Errors
    /// Rejects unsupported version/transport pairs and negative application IDs.
    pub fn new(protocol: NegotiatedProtocol, request_id: i64) -> Result<Self> {
        if request_id < 0
            || ProtocolVersion::parse_for(protocol.transport, protocol.version.as_str())
                != Some(protocol.version)
        {
            return Err(McpSubmissionError::Invalid);
        }
        Ok(Self {
            protocol,
            request_id,
            progress_token: None,
            form: false,
            url: false,
        })
    }

    #[must_use]
    pub const fn with_progress_token(mut self, token: u64) -> Self {
        self.progress_token = Some(token);
        self
    }

    /// Advertise only modes backed by the host's actual input responder.
    #[must_use]
    pub const fn with_elicitation(mut self, form: bool, url: bool) -> Self {
        self.form = form;
        self.url = url;
        self
    }
}

/// Immutable request data, not permission or a transport write handle. There is
/// no raw payload constructor, mutable metadata, or continuation-byte escape.
pub struct McpToolRequest {
    runtime: Arc<McpSubmissionRuntime>,
    schema: McpSchema,
    tool: ToolName,
    call: ToolCallId,
    arguments: Box<[u8]>,
    payload: Box<[u8]>,
    options: McpToolCallOptions,
    tool_reservation: Option<super::McpToolReservation>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    head: Option<Arc<super::McpSubmissionHttpHead>>,
}
impl std::fmt::Debug for McpToolRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("McpToolRequest { <redacted> }")
    }
}

impl McpToolRequest {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    /// Checks pinned modern HTTP header eligibility before catalog publication.
    /// This inspects an already admitted schema and grants no tool authority.
    ///
    /// # Errors
    /// Rejects non-object roots, misplaced/ambiguous header annotations and
    /// unsupported annotated property types.
    pub fn validate_modern_http_schema(schema: &McpSchema) -> Result<()> {
        schema
            .require_object_root()
            .map_err(|_| McpSubmissionError::Invalid)?;
        headers::validate_schema(schema)
    }

    /// Validates the exact immutable runtime schema and canonical invocation,
    /// then encodes pinned modern/legacy metadata without acquiring a slot.
    /// Optional top-level header nulls are omitted only for validation fallback;
    /// the transmitted and permission-reviewed arguments remain unchanged.
    ///
    /// # Errors
    /// Rejects stale/mismatched runtime/schema/tool identity, invalid arguments,
    /// and finite argument/schema/envelope bounds. Server-authoritative schema
    /// assessment remains server-authoritative, never a local-validation claim.
    pub fn new(
        runtime: Arc<McpSubmissionRuntime>,
        schema: &McpSchema,
        invocation: PermissionInvocation<'_>,
        options: McpToolCallOptions,
    ) -> Result<Self> {
        runtime.live()?;
        if invocation.tool_name != runtime.binding.tool_name()
            || schema.raw_json().as_bytes() != runtime.binding.schema_bytes()
        {
            return Err(McpSubmissionError::Denied);
        }
        schema
            .require_object_root()
            .map_err(|_| McpSubmissionError::Invalid)?;
        let arguments = canonical_arguments(invocation.arguments)?;
        validate_arguments(schema, invocation.arguments, &arguments)?;
        let raw: &RawValue =
            serde_json::from_slice(&arguments).map_err(|_| McpSubmissionError::Invalid)?;
        let payload = bounded_json(
            &Envelope {
                jsonrpc: "2.0",
                id: options.request_id,
                method: "tools/call",
                params: Params {
                    name: runtime.binding.remote_tool(),
                    arguments: raw,
                    metadata: McpClientMetadata::for_protocol(
                        options.protocol.version,
                        options.progress_token,
                        options.form,
                        options.url,
                    ),
                },
            },
            MAX_MCP_SUBMISSION_REQUEST_BYTES,
        )?;
        runtime.live()?;
        Ok(Self {
            runtime,
            schema: schema.clone(),
            tool: invocation.tool_name.clone(),
            call: invocation.call_id.clone(),
            arguments,
            payload,
            options,
            tool_reservation: None,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            head: None,
        })
    }

    /// Retains the admitted schema for the mandatory native automatic review.
    /// Reading it grants nothing; server-authoritative assessment does not make
    /// the schema optional or waive the review's required-validation flag.
    #[must_use]
    pub fn schema(&self) -> &McpSchema {
        &self.schema
    }

    /// Exact canonical invocation evidence, not the HTTP header projection.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn arguments_json(&self) -> &str {
        std::str::from_utf8(&self.arguments).expect("JSON serialization is UTF-8")
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn binding(&self) -> &super::McpSubmissionRuntimeBinding {
        &self.runtime.binding
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn revalidate(&self) -> Result<()> {
        self.runtime.live()
    }

    /// Retains the owning peer's exact unsent-ID allocation through permission
    /// and writer submission. Dropping it permits that peer to reclaim only
    /// the abandoned allocation; it does not authorize execution.
    /// # Errors
    /// Rejects foreign IDs or a repeated attachment, without replacing a lease.
    pub fn with_reservation(mut self, reservation: super::McpToolReservation) -> Result<Self> {
        if self.tool_reservation.is_some()
            || reservation.rpc_id() != &RpcId::Integer(self.options.request_id)
        {
            return Err(McpSubmissionError::Invalid);
        }
        self.tool_reservation = Some(reservation);
        Ok(self)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    /// Fixes the peer-selected HTTP head and schema-derived modern headers.
    /// Authentication/session fields remain exact; there is no postapproval edit.
    ///
    /// # Errors
    /// Rejects stdio, repeated attachment, reserved overrides, invalid header
    /// annotations/values, mismatched protocol headers and transport bounds.
    pub fn with_http_head(mut self, base: &super::McpSubmissionHttpHead) -> Result<Self> {
        if self.options.protocol.transport == TransportKind::Stdio || self.head.is_some() {
            return Err(McpSubmissionError::Invalid);
        }
        self.head = Some(Arc::new(headers::project(&self, base)?));
        Ok(self)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    /// Exact preapproval connection-selection data, not a proof or write grant.
    #[must_use]
    pub fn http_head(&self) -> Option<Arc<super::McpSubmissionHttpHead>> {
        self.head.clone()
    }

    fn into_copied(self, invocation: CopiedInvocation) -> Result<CopiedRequest> {
        if self.tool != invocation.tool
            || self.call != invocation.call
            || self.arguments != invocation.arguments
        {
            return Err(McpSubmissionError::Denied);
        }
        let id = RpcId::Integer(self.options.request_id);
        if self.options.protocol.transport == TransportKind::Stdio {
            let mut wire = self.payload.into_vec();
            wire.push(b'\n');
            let mut copied = invocation.with_wire(wire.into_boxed_slice(), Framing::Ndjson, id);
            copied.tool_reservation = self.tool_reservation;
            return Ok(copied);
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let head = self.head.ok_or(McpSubmissionError::Invalid)?;
            let mut copied = invocation.with_wire(head.encode(&self.payload)?, Framing::Http, id);
            copied.tool_reservation = self.tool_reservation;
            Ok(copied)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        Err(McpSubmissionError::Invalid)
    }
}

impl McpSubmissionRegistry {
    /// Prepares a typed, schema-bound request under the same exact invocation,
    /// one-shot reservation and concrete-proof rules as raw preparation.
    /// Validation/copying is immediate; reservation still waits for first poll.
    ///
    /// # Errors
    /// Rejects foreign/changed invocation data, missing HTTP head, closed scopes
    /// and the existing finite submission limits. No arbitrary wire is accepted.
    pub fn prepare_tool(
        self: &Arc<Self>,
        request: &PermissionRequest,
        invocation: PermissionInvocation<'_>,
        projection: McpToolRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<PreparedMcpSubmission>> {
        let runtime = projection.runtime.clone();
        let copied = self
            .copy_invocation(request, invocation, &runtime, &cancellation)
            .and_then(|invocation| projection.into_copied(invocation));
        self.prepare_copied(runtime, copied, cancellation)
    }
}

#[derive(Serialize)]
struct Envelope<'a> {
    jsonrpc: &'static str,
    id: i64,
    method: &'static str,
    params: Params<'a>,
}
#[derive(Serialize)]
struct Params<'a> {
    name: &'a str,
    arguments: &'a RawValue,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    metadata: Option<McpClientMetadata>,
}

fn validate_arguments(schema: &McpSchema, value: &Value, bytes: &[u8]) -> Result<()> {
    let result = schema
        .validate_json(bytes)
        .map_err(|_| McpSubmissionError::Invalid)?;
    if !matches!(result, McpSchemaValidation::Invalid(_)) {
        return Ok(());
    }
    let Some(arguments) = value.as_object() else {
        return Err(McpSubmissionError::Invalid);
    };
    let fields: BTreeMap<String, &RawValue> =
        serde_json::from_str(schema.raw_json()).map_err(|_| McpSubmissionError::Invalid)?;
    let Some(properties) = fields.get("properties") else {
        return Err(McpSubmissionError::Invalid);
    };
    let properties: BTreeMap<String, &RawValue> =
        serde_json::from_str(properties.get()).map_err(|_| McpSubmissionError::Invalid)?;
    let required: Vec<String> = fields
        .get("required")
        .map(|raw| serde_json::from_str(raw.get()))
        .transpose()
        .map_err(|_| McpSubmissionError::Invalid)?
        .unwrap_or_default();
    let mut retained = BTreeMap::new();
    for (name, value) in arguments {
        let omit = if value.is_null() && !required.contains(name) {
            properties
                .get(name)
                .filter(|raw| raw.get().starts_with('{'))
                .map(|raw| serde_json::from_str::<BTreeMap<String, &RawValue>>(raw.get()))
                .transpose()
                .map_err(|_| McpSubmissionError::Invalid)?
                .is_some_and(|fields| fields.contains_key("x-mcp-header"))
        } else {
            false
        };
        if !omit {
            retained.insert(name, value);
        }
    }
    if retained.len() == arguments.len() {
        return Err(McpSubmissionError::Invalid);
    }
    let normalized = bounded_json(&retained, super::MAX_MCP_SUBMISSION_ARGUMENT_BYTES)?;
    match schema
        .validate_json(&normalized)
        .map_err(|_| McpSubmissionError::Invalid)?
    {
        McpSchemaValidation::Valid | McpSchemaValidation::ServerAuthoritative => Ok(()),
        McpSchemaValidation::Invalid(_) => Err(McpSubmissionError::Invalid),
    }
}
