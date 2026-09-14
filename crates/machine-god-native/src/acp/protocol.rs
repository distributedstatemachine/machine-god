//! Effect-free, bounded ACP v1 JSON-RPC wire boundary.
//!
//! Correlation labels are not native session or tool authority. The driver owns
//! the actual permission/continuation custody and output backpressure.

use std::fmt;

use serde::Serialize;
use serde::ser::SerializeMap;
use serde_json::Value;

mod bounds;
mod framing;
mod pending;

pub use framing::AcpFrameDecoder;
pub use pending::{AcpPendingRequests, AcpScope};

/// The only supported ACP protocol version.
pub const ACP_PROTOCOL_VERSION: u32 = 1;
/// Maximum JSON frame bytes, excluding its newline delimiter.
pub const ACP_MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
/// Maximum simultaneous outbound interaction requests.
pub const ACP_MAX_PENDING_REQUESTS: usize = 32;
/// Maximum nesting, including the JSON-RPC envelope.
pub const ACP_MAX_JSON_DEPTH: usize = 64;
/// Maximum value and object-key tokens in one frame.
pub const ACP_MAX_JSON_NODES: usize = 65_536;
/// Conservative retained JSON allocation charge per frame.
pub const ACP_MAX_RETAINED_BYTES: usize = 32 * 1024 * 1024;
/// Maximum UTF-8 bytes in a request identifier.
pub const ACP_MAX_ID_BYTES: usize = 1024;
/// Maximum UTF-8 bytes in a method name.
pub const ACP_MAX_METHOD_BYTES: usize = 256;

/// JSON-RPC request identifier. Fractional/exponent numeric IDs are rejected.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize)]
#[serde(untagged)]
pub enum AcpId {
    Integer(i64),
    String(String),
}

impl fmt::Debug for AcpId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AcpId(<redacted>)")
    }
}

/// Remote error data remains exact JSON and is never included in diagnostics.
#[derive(Clone, PartialEq, Serialize)]
pub struct AcpRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl fmt::Debug for AcpRpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcpRpcError")
            .field("code", &self.code)
            .finish_non_exhaustive()
    }
}

/// A single non-batch JSON-RPC message. Only error responses may have null IDs.
#[derive(Clone, PartialEq)]
pub enum AcpMessage {
    Request {
        id: AcpId,
        method: String,
        params: Option<Value>,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
    Response {
        id: Option<AcpId>,
        outcome: Result<Value, AcpRpcError>,
    },
}

impl fmt::Debug for AcpMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Request { .. } => "AcpMessage::Request(<redacted>)",
            Self::Notification { .. } => "AcpMessage::Notification(<redacted>)",
            Self::Response { .. } => "AcpMessage::Response(<redacted>)",
        })
    }
}

/// Stable, payload-free boundary errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpProtocolError {
    FrameTooLarge,
    JsonBudgetExceeded,
    ParseError,
    InvalidRequest,
    UnsupportedVersion,
    TruncatedFrame,
    PendingLimit,
    IdentifierExhausted,
    UnknownResponse,
    StaleResponse,
}

impl fmt::Display for AcpProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::FrameTooLarge => "ACP frame exceeds its byte limit",
            Self::JsonBudgetExceeded => "ACP JSON exceeds its resource budget",
            Self::ParseError => "invalid ACP JSON",
            Self::InvalidRequest => "invalid ACP JSON-RPC envelope",
            Self::UnsupportedVersion => "unsupported ACP protocol version",
            Self::TruncatedFrame => "ACP input ended before a newline delimiter",
            Self::PendingLimit => "ACP outbound request capacity reached",
            Self::IdentifierExhausted => "ACP outbound identifier space exhausted",
            Self::UnknownResponse => "unknown or already settled ACP response",
            Self::StaleResponse => "ACP response scope does not match",
        })
    }
}

impl std::error::Error for AcpProtocolError {}

/// Validates the initialize protocol version without accepting older versions.
///
/// # Errors
/// Rejects missing, non-integer, and unsupported protocol versions.
pub fn validate_protocol_version(params: &Value) -> Result<(), AcpProtocolError> {
    if params
        .get("protocolVersion")
        .and_then(Value::as_number)
        .is_some_and(|n| n.as_str() == "1")
    {
        Ok(())
    } else {
        Err(AcpProtocolError::UnsupportedVersion)
    }
}

/// Decodes a complete JSON frame (without requiring its newline).
///
/// # Errors
/// Rejects over-budget data before tree allocation, malformed/duplicate JSON,
/// invalid envelopes and unsupported identifier representations.
pub fn decode_frame(frame: &[u8]) -> Result<AcpMessage, AcpProtocolError> {
    let value = decode_value(frame)?;
    let Value::Object(mut object) = value else {
        return Err(AcpProtocolError::InvalidRequest);
    };
    if object.remove("jsonrpc") != Some(Value::String("2.0".into())) {
        return Err(AcpProtocolError::InvalidRequest);
    }
    let id = object.remove("id");
    let method = object.remove("method");
    let params = object.remove("params");
    let result = object.remove("result");
    let error = object.remove("error");
    // Unknown members are extensions, not authority. They have already paid the
    // same byte/node/depth budgets and duplicate-key policy as known members.
    if let Some(method) = method {
        if result.is_some() || error.is_some() {
            return Err(AcpProtocolError::InvalidRequest);
        }
        let Value::String(method) = method else {
            return Err(AcpProtocolError::InvalidRequest);
        };
        validate_method(&method)?;
        validate_params(params.as_ref())?;
        return match id {
            Some(value) => Ok(AcpMessage::Request {
                id: decode_id(value)?,
                method,
                params,
            }),
            None => Ok(AcpMessage::Notification { method, params }),
        };
    }
    if params.is_some() || result.is_some() == error.is_some() {
        return Err(AcpProtocolError::InvalidRequest);
    }
    let id = match id {
        Some(Value::Null) if error.is_some() => None,
        Some(value) => Some(decode_id(value)?),
        None => return Err(AcpProtocolError::InvalidRequest),
    };
    let outcome = match (result, error) {
        (Some(result), None) => Ok(result),
        (None, Some(Value::Object(mut error))) => {
            let code = match error.remove("code") {
                Some(Value::Number(n)) => parse_integer(n.as_str())?,
                _ => return Err(AcpProtocolError::InvalidRequest),
            };
            let Some(Value::String(message)) = error.remove("message") else {
                return Err(AcpProtocolError::InvalidRequest);
            };
            Err(AcpRpcError {
                code,
                message,
                data: error.remove("data"),
            })
        }
        _ => return Err(AcpProtocolError::InvalidRequest),
    };
    Ok(AcpMessage::Response { id, outcome })
}

/// Shared admission for native projection of raw protocol payloads. This uses
/// the same pre-allocation limits and exact JSON codec as complete frames.
pub(crate) fn decode_value(bytes: &[u8]) -> Result<Value, AcpProtocolError> {
    bounds::preflight(bytes)?;
    machine_god_core::json::from_slice(bytes).map_err(|_| AcpProtocolError::ParseError)
}

/// Validate a borrowed tree before projection clones it into an envelope.
pub(crate) fn validate_value(value: &Value, initial_depth: usize) -> Result<(), AcpProtocolError> {
    bounds::validate_tree(value, initial_depth)
}

fn parse_integer(text: &str) -> Result<i64, AcpProtocolError> {
    if text.contains(['.', 'e', 'E']) {
        return Err(AcpProtocolError::InvalidRequest);
    }
    text.parse().map_err(|_| AcpProtocolError::InvalidRequest)
}

fn decode_id(value: Value) -> Result<AcpId, AcpProtocolError> {
    match value {
        Value::String(id) if id.len() <= ACP_MAX_ID_BYTES => Ok(AcpId::String(id)),
        Value::Number(id) => parse_integer(id.as_str()).map(AcpId::Integer),
        _ => Err(AcpProtocolError::InvalidRequest),
    }
}

fn validate_id(id: &AcpId) -> Result<(), AcpProtocolError> {
    if matches!(id, AcpId::String(text) if text.len() > ACP_MAX_ID_BYTES) {
        Err(AcpProtocolError::InvalidRequest)
    } else {
        Ok(())
    }
}

fn validate_method(method: &str) -> Result<(), AcpProtocolError> {
    if method.is_empty()
        || method.len() > ACP_MAX_METHOD_BYTES
        || method.chars().any(char::is_control)
    {
        Err(AcpProtocolError::InvalidRequest)
    } else {
        Ok(())
    }
}

fn validate_params(params: Option<&Value>) -> Result<(), AcpProtocolError> {
    if params.is_some_and(|params| !params.is_object() && !params.is_array()) {
        Err(AcpProtocolError::InvalidRequest)
    } else {
        Ok(())
    }
}

/// Encodes a validated frame including one final newline, without cloning its
/// JSON tree. The caller owns bounded output queuing and write finalization.
///
/// # Errors
/// Rejects invalid constructed envelopes and depth/node/retention/output limits.
pub fn encode_frame(message: &AcpMessage) -> Result<Vec<u8>, AcpProtocolError> {
    validate_message_shape(message)?;
    bounds::encode(message)
}

/// Shared shallow admission for explicitly constructed native envelopes.
/// Payload projection still enforces its own borrowed-tree and byte budgets.
pub(crate) fn validate_message_shape(message: &AcpMessage) -> Result<(), AcpProtocolError> {
    match message {
        AcpMessage::Request { id, method, params } => {
            validate_id(id)?;
            validate_method(method)?;
            validate_params(params.as_ref())?;
        }
        AcpMessage::Notification { method, params } => {
            validate_method(method)?;
            validate_params(params.as_ref())?;
        }
        AcpMessage::Response { id, outcome } => {
            if let Some(id) = id {
                validate_id(id)?;
            }
            if id.is_none() && outcome.is_ok() {
                return Err(AcpProtocolError::InvalidRequest);
            }
        }
    }
    Ok(())
}

impl Serialize for AcpMessage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("jsonrpc", "2.0")?;
        match self {
            Self::Request { id, method, params } => {
                map.serialize_entry("id", id)?;
                map.serialize_entry("method", method)?;
                if let Some(params) = params {
                    map.serialize_entry("params", params)?;
                }
            }
            Self::Notification { method, params } => {
                map.serialize_entry("method", method)?;
                if let Some(params) = params {
                    map.serialize_entry("params", params)?;
                }
            }
            Self::Response { id, outcome } => {
                map.serialize_entry("id", id)?;
                match outcome {
                    Ok(result) => map.serialize_entry("result", result)?,
                    Err(error) => map.serialize_entry("error", error)?,
                }
            }
        }
        map.end()
    }
}

#[cfg(test)]
mod tests;
