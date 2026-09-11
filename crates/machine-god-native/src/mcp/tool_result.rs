//! Lossless, method-specific tool result admission; no consent or replay authority.

use super::{
    catalog::McpToolDescriptor,
    feature::McpFeatureCodecLimits,
    mrtr::McpInputRequired,
    protocol::{NegotiatedProtocol, ProtocolVersion, RpcId},
    submission::McpSubmissionRuntime,
};
use machine_god_core::{ToolContext, ToolName, ToolOutput};
use serde_json::value::RawValue;
use std::{fmt, sync::Arc};

mod decode;
#[cfg(test)]
mod tests;

/// Independently bounded complete data, before archive/model projection.
#[derive(Clone, Copy, Debug)]
pub struct McpToolResultLimits {
    pub max_response_bytes: usize,
    pub max_nodes: usize,
    pub max_content_field_bytes: usize,
    pub max_content_items: usize,
    pub max_retained_bytes: usize,
    /// Compact complete result bytes, including all unknown fields.
    pub max_result_bytes: usize,
}
impl Default for McpToolResultLimits {
    fn default() -> Self {
        let content = McpFeatureCodecLimits::default();
        Self {
            max_response_bytes: content.max_response_bytes,
            max_nodes: content.max_nodes,
            max_content_field_bytes: content.max_content_field_bytes,
            max_content_items: content.max_content_items,
            max_retained_bytes: content.max_retained_bytes,
            max_result_bytes: 4 * 1024 * 1024 + 16 * 1024,
        }
    }
}
impl McpToolResultLimits {
    fn content_limits(self) -> McpFeatureCodecLimits {
        McpFeatureCodecLimits {
            max_response_bytes: self.max_response_bytes,
            max_nodes: self.max_nodes,
            max_content_field_bytes: self.max_content_field_bytes,
            max_content_items: self.max_content_items,
            max_retained_bytes: self.max_retained_bytes,
            ..McpFeatureCodecLimits::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpToolResultError {
    InvalidLimits,
    InvalidContext,
    InvalidResponse,
    Correlation,
    UnsupportedResultType,
    InvalidContent,
    InvalidStructuredContent,
    InvalidInputRequired,
    Limit,
}
impl fmt::Display for McpToolResultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP tool response rejected")
    }
}
impl std::error::Error for McpToolResultError {}
type Error = McpToolResultError;
type Result<T> = std::result::Result<T, Error>;

/// Exact response provenance supplied by native execution, not an authority token.
/// The runtime still owns and revalidates its live turn, route and cancellation.
/// This value is deliberately neither cloneable nor serializable.
pub struct McpToolResponseContext {
    context: ToolContext,
    tool: ToolName,
    server: Arc<str>,
    descriptor: McpToolDescriptor,
    runtime: Arc<McpSubmissionRuntime>,
    protocol: NegotiatedProtocol,
    request_id: RpcId,
}
impl McpToolResponseContext {
    /// Captures already selected identity without acquiring any execution grant.
    /// # Errors
    /// Rejects empty/oversized server identities, IDs and protocol/transport combinations.
    pub fn new(
        context: ToolContext,
        tool: ToolName,
        server: Arc<str>,
        descriptor: McpToolDescriptor,
        runtime: Arc<McpSubmissionRuntime>,
        protocol: NegotiatedProtocol,
        request_id: RpcId,
    ) -> Result<Self> {
        if server.is_empty()
            || server.len() > 128
            || !matches!(request_id, RpcId::Integer(id) if id >= 0)
            || ProtocolVersion::parse_for(protocol.transport, protocol.version.as_str())
                != Some(protocol.version)
        {
            return Err(Error::InvalidContext);
        }
        Ok(Self {
            context,
            tool,
            server,
            descriptor,
            runtime,
            protocol,
            request_id,
        })
    }
    #[must_use]
    pub fn tool_context(&self) -> &ToolContext {
        &self.context
    }
    #[must_use]
    pub fn tool_name(&self) -> &ToolName {
        &self.tool
    }
    #[must_use]
    pub fn server(&self) -> &str {
        &self.server
    }
    #[must_use]
    pub fn descriptor(&self) -> &McpToolDescriptor {
        &self.descriptor
    }
    #[must_use]
    pub fn runtime(&self) -> &Arc<McpSubmissionRuntime> {
        &self.runtime
    }
    #[must_use]
    pub fn protocol(&self) -> NegotiatedProtocol {
        self.protocol
    }
    #[must_use]
    pub fn request_id(&self) -> &RpcId {
        &self.request_id
    }
}

/// Inert, non-clone custody of validated input requests and exact call provenance.
/// Reading server state or obtaining browser consent never grants resubmission.
pub struct McpToolInputRequired {
    context: McpToolResponseContext,
    required: McpInputRequired,
}
impl McpToolInputRequired {
    #[must_use]
    pub fn context(&self) -> &McpToolResponseContext {
        &self.context
    }
    #[must_use]
    pub fn required(&self) -> &McpInputRequired {
        &self.required
    }
    /// Consumes data custody only; native continuation must obtain a fresh grant.
    #[must_use]
    pub fn into_parts(self) -> (McpToolResponseContext, McpInputRequired) {
        (self.context, self.required)
    }
}

/// Validated protocol failure data, distinct from a completed tool's `isError`.
pub struct McpToolProtocolFailure {
    code: i64,
    message: Box<str>,
    raw: Box<RawValue>,
}
impl McpToolProtocolFailure {
    #[must_use]
    pub fn code(&self) -> i64 {
        self.code
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
    /// Complete original error object. Never interpolate this directly into a terminal.
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
}

pub enum McpToolResponseDisposition {
    /// Complete original result, including structured content and unknown metadata.
    /// This is not an already archived, sanitized or model-sized projection.
    Complete(ToolOutput),
    ProtocolFailure(McpToolProtocolFailure),
    InputRequired(Box<McpToolInputRequired>),
}

/// Immutable, effect-free admission policy; construction acquires no authority.
pub struct NativeMcpToolResultAdmission {
    limits: McpToolResultLimits,
}
impl NativeMcpToolResultAdmission {
    /// # Errors
    /// Rejects zero or enlarged limits rather than accepting unbounded input.
    pub fn new(limits: McpToolResultLimits) -> Result<Self> {
        limits
            .content_limits()
            .validate()
            .map_err(|_| Error::InvalidLimits)?;
        if limits.max_result_bytes == 0
            || limits.max_result_bytes > McpToolResultLimits::default().max_result_bytes
        {
            return Err(Error::InvalidLimits);
        }
        Ok(Self { limits })
    }
}

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($ty), " { <redacted> }"))
            }
        }
    )+};
}
redacted_debug!(
    McpToolResponseContext,
    McpToolInputRequired,
    McpToolProtocolFailure,
    McpToolResponseDisposition,
    NativeMcpToolResultAdmission
);
