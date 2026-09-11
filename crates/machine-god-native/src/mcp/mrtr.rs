//! Inert, bounded multi-round-trip input and elicitation codecs.
//!
//! Validated data is neither user consent nor a continuation/execution grant.

use serde_json::value::RawValue;
use std::fmt;

mod bounds;
mod elicitation;
mod form;
mod input;
mod sampling;
mod strings;
#[cfg(test)]
mod tests;

pub use elicitation::{
    McpElicitationAction, McpElicitationMode, McpElicitationRequest, McpHostClassification,
};
pub use form::{McpFormChoice, McpFormField, McpFormFieldKind, McpFormSchema};
pub use input::{
    McpInputRequest, McpInputRequestPayload, McpInputRequired, McpInputResponse,
    McpInputResponseKind, McpValidatedResponses,
};

/// Positive limits may only be lowered. Defaults follow pinned MRTR bounds;
/// nodes and conservative retained bytes independently bound Rust containers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpMrtrLimits {
    pub max_requests: usize,
    pub max_name_bytes: usize,
    pub max_json_bytes: usize,
    pub max_string_bytes: usize,
    pub max_collection_items: usize,
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_retained_bytes: usize,
    pub max_pattern_bytes: usize,
    pub max_pattern_depth: usize,
    pub max_pattern_states: usize,
    pub max_pattern_repeat: usize,
    pub max_pattern_steps: usize,
    pub max_number_bytes: usize,
    pub max_number_exponent_abs: i64,
    pub max_number_expanded_digits: usize,
}
impl Default for McpMrtrLimits {
    fn default() -> Self {
        Self {
            max_requests: 32,
            max_name_bytes: 256,
            max_json_bytes: 128 * 1024,
            max_string_bytes: 64 * 1024,
            max_collection_items: 256,
            max_depth: 32,
            max_nodes: 65_536,
            max_retained_bytes: 16 * 1024 * 1024,
            max_pattern_bytes: 512,
            max_pattern_depth: 64,
            max_pattern_states: 2048,
            max_pattern_repeat: 1024,
            max_pattern_steps: 100_000,
            max_number_bytes: 4096,
            max_number_exponent_abs: 1_000_000,
            max_number_expanded_digits: 8192,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpMrtrError {
    InvalidLimits,
    InvalidJson,
    InvalidRequest,
    InvalidResponse,
    InvalidSchema,
    UnsupportedMode,
    UnsupportedSchema,
    SecretField,
    InvalidUrl,
    InsecureUrl,
    Limit,
}
impl fmt::Display for McpMrtrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP input codec admission failed")
    }
}
impl std::error::Error for McpMrtrError {}
type Error = McpMrtrError;
type Result<T> = std::result::Result<T, Error>;

fn raw(value: &RawValue) -> Box<RawValue> {
    value.to_owned()
}

// Only known small typed structures are serialized here; RawValue fields retain
// exact source numbers and literal private-looking object keys.
fn encode(value: &impl serde::Serialize, limits: McpMrtrLimits) -> Result<Box<RawValue>> {
    let json = serde_json::to_string(value).map_err(|_| Error::InvalidJson)?;
    if json.len() > limits.max_json_bytes {
        return Err(Error::Limit);
    }
    RawValue::from_string(json).map_err(|_| Error::InvalidJson)
}
