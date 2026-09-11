//! Effect-free, bounded server-sent event framing for the pinned MCP consumers.
//!
//! Modern and legacy modes intentionally differ; neither is a browser `EventSource`
//! implementation. Events carry untrusted observations, not validated JSON-RPC,
//! endpoint authority, cursor changes or permission to reconnect. See
//! `docs/mcp-sse.md` for the exact compatibility and resource boundary.

mod decoder;

pub use decoder::SseDecoder;

use std::fmt;

/// Pinned consumer semantics, not automatic protocol negotiation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SseMode {
    /// Modern HTTP reads only nonempty data; other fields have no effect.
    Modern,
    /// Legacy HTTP/SSE retains event, ID, retry and empty priming observations.
    Legacy,
}

/// Fixed-ceiling decoder budgets. Zero never means unlimited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SseLimits {
    /// Bytes per line, excluding its CR/LF terminator.
    pub max_line_bytes: usize,
    /// Aggregate decoded data bytes, including inserted newlines.
    pub max_data_bytes: usize,
    /// Bytes in each retained event-name or ID field.
    pub max_field_bytes: usize,
    /// Nonempty lines per block, including ignored fields and comments.
    pub max_fields: usize,
    /// Emitted events over this decoder's lifetime.
    pub max_events: usize,
    /// Consumed raw bytes, including comments and every CR/LF byte.
    pub max_total_bytes: usize,
    /// Maximum raw bytes consumed in one push, even when all are ignored.
    pub max_push_bytes: usize,
}

impl Default for SseLimits {
    fn default() -> Self {
        Self {
            max_line_bytes: 8 * 1024 * 1024,
            max_data_bytes: 8 * 1024 * 1024,
            max_field_bytes: 4096,
            max_fields: 1024,
            max_events: 4096,
            max_total_bytes: 64 * 1024 * 1024,
            max_push_bytes: 16 * 1024,
        }
    }
}

impl SseLimits {
    /// Admit only finite positive limits no greater than the documented ceilings.
    ///
    /// # Errors
    /// Returns [`SseError::InvalidLimits`] for a zero or excessive limit.
    pub fn validate(self) -> Result<Self, SseError> {
        for (value, ceiling) in [
            (self.max_line_bytes, 16 * 1024 * 1024),
            (self.max_data_bytes, 16 * 1024 * 1024),
            (self.max_field_bytes, 64 * 1024),
            (self.max_fields, 16_384),
            (self.max_events, 65_536),
            (self.max_total_bytes, 1024 * 1024 * 1024),
            (self.max_push_bytes, 64 * 1024),
        ] {
            if value == 0 || value > ceiling {
                return Err(SseError::InvalidLimits);
            }
        }
        Ok(self)
    }
}

/// Stable failures with no retained or formatted server-controlled text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SseError {
    /// A configured budget is zero or above its fixed ceiling.
    InvalidLimits,
    /// Raw line bytes exceed their budget.
    LineLimit,
    /// Joined data bytes exceed their budget.
    DataLimit,
    /// Retained event/ID bytes exceed their budget.
    FieldLimit,
    /// Too many nonempty lines occurred without a blank separator.
    FieldCountLimit,
    /// Too many events were dispatched.
    EventLimit,
    /// The decoder's consumed raw-byte budget is exhausted.
    TotalLimit,
    /// A complete line, or the final partial line at EOF, is not UTF-8.
    InvalidUtf8,
    /// EOF leaves a partial line or a mode-specific pending event.
    IncompleteStream,
    /// A previous error or finish closed this decoder.
    Closed,
}

impl fmt::Display for SseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidLimits => "invalid MCP SSE limits",
            Self::LineLimit => "MCP SSE line limit exceeded",
            Self::DataLimit => "MCP SSE data limit exceeded",
            Self::FieldLimit => "MCP SSE field limit exceeded",
            Self::FieldCountLimit => "MCP SSE field count exceeded",
            Self::EventLimit => "MCP SSE event count exceeded",
            Self::TotalLimit => "MCP SSE raw byte limit exceeded",
            Self::InvalidUtf8 => "invalid MCP SSE UTF-8",
            Self::IncompleteStream => "incomplete MCP SSE stream",
            Self::Closed => "MCP SSE decoder is closed",
        })
    }
}
impl std::error::Error for SseError {}

/// Framing distinction: control-only observations are not JSON-RPC messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SseEventClass {
    /// Nonempty data requiring method/transport-specific interpretation.
    Data,
    /// Legacy empty-data or metadata-only priming/control observation.
    Control,
}

/// One complete, untrusted observation, emitted only at a blank line.
///
/// Fields do not inherit from earlier events. In particular `id: None` is not
/// `Some("")`: consumers must validate before committing any cursor change.
#[derive(Eq, PartialEq)]
pub struct SseEvent {
    pub(super) data: String,
    pub(super) event: Option<String>,
    pub(super) id: Option<String>,
    pub(super) retry_ms: Option<u32>,
    pub(super) had_data_field: bool,
}

impl SseEvent {
    /// Distinguish nonempty data from legacy control/priming observations.
    #[must_use]
    pub fn class(&self) -> SseEventClass {
        if self.data.is_empty() {
            SseEventClass::Control
        } else {
            SseEventClass::Data
        }
    }
    /// Joined data, without an extra trailing newline.
    #[must_use]
    pub fn data(&self) -> &str {
        &self.data
    }
    /// Explicit event name; absent and explicitly empty remain distinct.
    #[must_use]
    pub fn event(&self) -> Option<&str> {
        self.event.as_deref()
    }
    /// Explicit non-NUL ID, not a committed or inherited resume cursor.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
    /// Parsed delay capped at 60,000 ms; this does not authorize reconnection.
    #[must_use]
    pub const fn retry_ms(&self) -> Option<u32> {
        self.retry_ms
    }
    /// Whether a data field occurred, including an explicitly empty one.
    #[must_use]
    pub const fn had_data_field(&self) -> bool {
        self.had_data_field
    }
}

impl fmt::Debug for SseEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SseEvent")
            .field("class", &self.class())
            .field("data_bytes", &self.data.len())
            .field("has_event", &self.event.is_some())
            .field("has_id", &self.id.is_some())
            .field("has_retry", &self.retry_ms.is_some())
            .finish_non_exhaustive()
    }
}

/// One bounded decoder step. No internal or returned aggregate event queue.
#[derive(Debug, Eq, PartialEq)]
pub struct SseProgress {
    /// Only these input bytes were consumed; re-submit the remaining tail.
    pub consumed: usize,
    /// At most one complete event; none is normal for comments/partial lines.
    pub event: Option<SseEvent>,
}

#[cfg(test)]
mod tests;
