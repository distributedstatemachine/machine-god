use std::fmt;

use serde_json::Value;

/// Finite admission limits, checked before framing or JSON allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireLimits {
    /// Maximum bytes in one frame, including CR but excluding LF.
    pub max_frame_bytes: usize,
    /// Maximum JSON value depth; the root value has depth one.
    pub max_depth: usize,
    /// Maximum JSON values and object keys in one frame.
    pub max_nodes: usize,
}

impl Default for WireLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 8 * 1024 * 1024,
            max_depth: 64,
            max_nodes: 65_536,
        }
    }
}

impl WireLimits {
    /// Reject zero or excessive limits instead of accepting an unbounded parser.
    ///
    /// # Errors
    /// Returns [`WireError::InvalidLimits`] above 16 MiB, depth 64 or 262,144 nodes.
    pub fn validate(self) -> Result<Self, WireError> {
        if self.max_frame_bytes == 0
            || self.max_frame_bytes > 16 * 1024 * 1024
            || self.max_depth == 0
            || self.max_depth > 64
            || self.max_nodes == 0
            || self.max_nodes > 262_144
        {
            return Err(WireError::InvalidLimits);
        }
        Ok(self)
    }
}

/// Stable redacted failures. No server bytes or serde diagnostics are retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    /// Invalid caller-selected bounds.
    InvalidLimits,
    /// The byte budget was exhausted before copying more input.
    FrameTooLarge,
    /// Invalid JSON, duplicate keys, trailing data or depth/node exhaustion.
    InvalidJson,
    /// Invalid or ambiguous JSON-RPC envelope.
    InvalidEnvelope,
    /// Response belongs to another request or is not a response.
    MismatchedId,
    /// EOF arrived with an unterminated frame.
    IncompleteFrame,
    /// A decoder was already finished or rejected input.
    ClosedDecoder,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidLimits => "invalid MCP wire limits",
            Self::FrameTooLarge => "MCP frame exceeds byte limit",
            Self::InvalidJson => "invalid or excessive MCP JSON",
            Self::InvalidEnvelope => "invalid MCP envelope",
            Self::MismatchedId => "MCP response ID mismatch",
            Self::IncompleteFrame => "incomplete MCP frame",
            Self::ClosedDecoder => "MCP decoder is closed",
        })
    }
}

impl std::error::Error for WireError {}

/// A single incremental framing step, never an unbounded collection of frames.
#[derive(Eq, PartialEq)]
pub struct FrameProgress {
    /// Bytes consumed from the provided chunk. Re-submit only its remaining tail.
    pub consumed: usize,
    /// One complete frame, or none for a partial/empty line.
    pub frame: Option<Vec<u8>>,
}

impl fmt::Debug for FrameProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameProgress")
            .field("consumed", &self.consumed)
            .field("frame_bytes", &self.frame.as_ref().map(Vec::len))
            .finish()
    }
}

/// Incremental LF framing; trailing CR and empty lines match pinned stdio.
///
/// Retains at most one bounded partial frame. A complete frame transfers its
/// storage to the caller; callers must bound their own queued frames and bytes.
pub struct NdjsonDecoder {
    limits: WireLimits,
    buffer: Vec<u8>,
    closed: bool,
}

impl fmt::Debug for NdjsonDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NdjsonDecoder")
            .field("limits", &self.limits)
            .field("buffered_bytes", &self.buffer.len())
            .field("closed", &self.closed)
            .finish()
    }
}

impl NdjsonDecoder {
    /// Construct an inert decoder.
    ///
    /// # Errors
    /// Rejects invalid limits.
    pub fn new(limits: WireLimits) -> Result<Self, WireError> {
        Ok(Self {
            limits: limits.validate()?,
            buffer: Vec::new(),
            closed: false,
        })
    }

    /// Consume at most one LF-delimited line from `chunk`.
    ///
    /// Empty lines consume input but return no frame. Empty input is a no-op,
    /// not EOF. UTF-8 and JSON are checked separately by [`parse_envelope`].
    ///
    /// # Errors
    /// Oversize input poisons the decoder; subsequent calls reject input.
    pub fn push(&mut self, chunk: &[u8]) -> Result<FrameProgress, WireError> {
        if self.closed {
            return Err(WireError::ClosedDecoder);
        }
        let remaining = self.limits.max_frame_bytes - self.buffer.len();
        let newline = memchr::memchr(b'\n', &chunk[..chunk.len().min(remaining + 1)]);
        let count = newline.unwrap_or(chunk.len());
        if count > remaining {
            self.closed = true;
            self.buffer = Vec::new();
            return Err(WireError::FrameTooLarge);
        }
        // Amortize tiny chunks without allowing geometric growth beyond the
        // fixed frame cap or sizing from the unconsumed tail of this chunk.
        let needed = self.buffer.len() + count;
        if needed > self.buffer.capacity() {
            let capacity = needed
                .max(self.buffer.capacity().saturating_mul(2))
                .min(self.limits.max_frame_bytes);
            self.buffer.reserve_exact(capacity - self.buffer.len());
        }
        self.buffer.extend_from_slice(&chunk[..count]);
        let Some(_) = newline else {
            return Ok(FrameProgress {
                consumed: count,
                frame: None,
            });
        };
        while self.buffer.last() == Some(&b'\r') {
            self.buffer.pop();
        }
        let frame = if self.buffer.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.buffer))
        };
        Ok(FrameProgress {
            consumed: count + 1,
            frame,
        })
    }

    /// Mark EOF, accepting only an empty partial buffer.
    ///
    /// # Errors
    /// An unterminated JSON object is not a complete NDJSON frame, even if valid.
    pub fn finish(&mut self) -> Result<(), WireError> {
        if self.closed {
            return Err(WireError::ClosedDecoder);
        }
        self.closed = true;
        let incomplete = !self.buffer.is_empty();
        self.buffer = Vec::new();
        if incomplete {
            Err(WireError::IncompleteFrame)
        } else {
            Ok(())
        }
    }
}

/// Exact JSON-RPC identity. String and integer IDs never compare equal.
#[derive(Clone, Eq, PartialEq)]
pub enum RpcId {
    /// Signed 64-bit integer, matching pinned JSON integer representation.
    Integer(i64),
    /// An opaque string of at most 1,024 UTF-8 bytes.
    String(String),
    /// Allowed only on error responses, never requests or successful replies.
    Null,
}

impl fmt::Debug for RpcId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Integer(_) => "RpcId::Integer([redacted])",
            Self::String(_) => "RpcId::String([redacted])",
            Self::Null => "RpcId::Null",
        })
    }
}

impl RpcId {
    fn parse(value: &Value) -> Result<Self, WireError> {
        match value {
            Value::Null => Ok(Self::Null),
            Value::String(text) if text.len() <= 1024 => Ok(Self::String(text.clone())),
            Value::Number(number) => number
                .as_i64()
                .map(Self::Integer)
                .ok_or(WireError::InvalidEnvelope),
            _ => Err(WireError::InvalidEnvelope),
        }
    }
}

/// Disjoint validated JSON-RPC envelope variants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpcKind {
    /// Method invocation with a non-null ID.
    Request,
    /// Method invocation without an ID.
    Notification,
    /// Response containing only a result discriminant.
    Success,
    /// Response containing only a validated error discriminant.
    Error,
}

/// Borrowed, validated protocol error; messages/data remain untrusted.
pub struct RpcProtocolError<'a> {
    /// Integer JSON-RPC error code.
    pub code: i64,
    /// Original message, for explicit policy-controlled use only.
    pub message: &'a str,
    /// Optional protocol evidence, never authority or executable input.
    pub data: Option<&'a Value>,
}

impl fmt::Debug for RpcProtocolError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RpcProtocolError")
            .field("code", &self.code)
            .finish_non_exhaustive()
    }
}

/// Bounded, duplicate-free and validated envelope. Debug omits all wire data.
pub struct RpcEnvelope {
    value: Value,
    kind: RpcKind,
    id: Option<RpcId>,
}

impl fmt::Debug for RpcEnvelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RpcEnvelope")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl RpcEnvelope {
    /// Move the already admitted exact JSON value without copying or granting
    /// any operation, transport or continuation authority.
    #[must_use]
    pub fn into_value(self) -> Value {
        self.value
    }
    /// Validated envelope kind.
    #[must_use]
    pub const fn kind(&self) -> RpcKind {
        self.kind
    }
    /// Exact wire identity, absent only on notifications.
    #[must_use]
    pub fn id(&self) -> Option<&RpcId> {
        self.id.as_ref()
    }
    /// Request or notification method, at most 256 UTF-8 bytes.
    #[must_use]
    pub fn method(&self) -> Option<&str> {
        self.value.get("method").and_then(Value::as_str)
    }
    /// Structured request/notification parameters, when present.
    #[must_use]
    pub fn params(&self) -> Option<&Value> {
        self.value.get("params")
    }
    /// Successful response payload, including explicit JSON null.
    #[must_use]
    pub fn result(&self) -> Option<&Value> {
        self.value.get("result")
    }
    /// Protocol error payload.
    #[must_use]
    pub fn protocol_error(&self) -> Option<RpcProtocolError<'_>> {
        let error = self.value.get("error")?;
        Some(RpcProtocolError {
            code: error.get("code")?.as_i64()?,
            message: error.get("message")?.as_str()?,
            data: error.get("data"),
        })
    }
    /// Access original validated data without cloning it. It is still untrusted.
    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }
    /// Correlate a final response to an outstanding request.
    ///
    /// The null-ID exception is only for an HTTP startup discovery error; normal
    /// calls, stdio, and successful replies must match the exact non-null ID.
    ///
    /// # Errors
    /// Rejects unsolicited, stale, wrong-kind or mismatched responses.
    pub fn correlate(
        &self,
        expected: &RpcId,
        allow_http_discovery_null_error: bool,
    ) -> Result<(), WireError> {
        if *expected == RpcId::Null || !matches!(self.kind, RpcKind::Success | RpcKind::Error) {
            return Err(WireError::MismatchedId);
        }
        if self.id.as_ref() == Some(expected)
            || (allow_http_discovery_null_error
                && self.kind == RpcKind::Error
                && self.id == Some(RpcId::Null))
        {
            Ok(())
        } else {
            Err(WireError::MismatchedId)
        }
    }
}

/// Parse exactly one bounded JSON-RPC envelope without duplicate object keys.
///
/// JSON-RPC batches are not supported. Unknown extension members are retained,
/// but request/response discriminants cannot be mixed. JSON result shapes are
/// method-specific; negotiation performs its own additional shape admission.
///
/// # Errors
/// Returns only redacted byte/JSON/envelope errors; never echoes remote input.
pub fn parse_envelope(bytes: &[u8], limits: WireLimits) -> Result<RpcEnvelope, WireError> {
    let value = super::parse_json(bytes, limits)?;
    let object = value.as_object().ok_or(WireError::InvalidEnvelope)?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(WireError::InvalidEnvelope);
    }
    let id = object.get("id").map(RpcId::parse).transpose()?;
    let has_result = object.contains_key("result");
    let has_error = object.contains_key("error");
    let kind = if let Some(method) = object.get("method") {
        let method = method.as_str().ok_or(WireError::InvalidEnvelope)?;
        if method.is_empty()
            || method.len() > 256
            || has_result
            || has_error
            || id == Some(RpcId::Null)
            || object
                .get("params")
                .is_some_and(|v| !v.is_object() && !v.is_array())
        {
            return Err(WireError::InvalidEnvelope);
        }
        if id.is_some() {
            RpcKind::Request
        } else {
            RpcKind::Notification
        }
    } else {
        if id.is_none() || has_result == has_error || object.contains_key("params") {
            return Err(WireError::InvalidEnvelope);
        }
        if has_error {
            let error = object["error"]
                .as_object()
                .ok_or(WireError::InvalidEnvelope)?;
            if error.get("code").and_then(Value::as_i64).is_none()
                || error.get("message").and_then(Value::as_str).is_none()
            {
                return Err(WireError::InvalidEnvelope);
            }
            RpcKind::Error
        } else {
            if id == Some(RpcId::Null) {
                return Err(WireError::InvalidEnvelope);
            }
            RpcKind::Success
        }
    };
    Ok(RpcEnvelope { value, kind, id })
}
