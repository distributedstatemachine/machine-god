//! Effect-free, bounded server-sent event framing for the pinned MCP consumers.
//!
//! Modern data-only framing is not a browser `EventSource` implementation.
//! Events are untrusted data, never endpoint or execution authority.

mod decoder;

pub use decoder::SseDecoder;

use std::fmt;

/// Fixed-ceiling decoder budgets. Zero never means unlimited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SseLimits {
    /// Bytes per line, excluding its CR/LF terminator.
    pub max_line_bytes: usize,
    /// Aggregate decoded data bytes, including inserted newlines.
    pub max_data_bytes: usize,
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
    /// Too many nonempty lines occurred without a blank separator.
    FieldCountLimit,
    /// Too many events were dispatched.
    EventLimit,
    /// The decoder's consumed raw-byte budget is exhausted.
    TotalLimit,
    /// A complete line, or the final partial line at EOF, is not UTF-8.
    InvalidUtf8,
    /// EOF leaves a partial line or a pending event.
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

/// One complete, untrusted nonempty data event, emitted only at a blank line.
#[derive(Eq, PartialEq)]
pub struct SseEvent {
    pub(super) data: String,
}
impl SseEvent {
    /// Joined data, without an extra trailing newline.
    #[must_use]
    pub fn data(&self) -> &str {
        &self.data
    }
}
impl fmt::Debug for SseEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SseEvent")
            .field("data_bytes", &self.data.len())
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
