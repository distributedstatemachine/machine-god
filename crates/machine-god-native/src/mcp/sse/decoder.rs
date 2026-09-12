use std::fmt;

use super::{SseError, SseEvent, SseLimits, SseProgress};

/// Inert incremental decoder with finite memory, work and lifetime budgets.
///
/// UTF-8 validation occurs once per completed line, permitting code points to
/// span arbitrary input chunks. A leading BOM is not stripped at the pin.
pub struct SseDecoder {
    limits: SseLimits,
    line: Vec<u8>,
    data: String,
    pending_cr: bool,
    fields: usize,
    events: usize,
    total_bytes: usize,
    closed: bool,
}

impl fmt::Debug for SseDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SseDecoder")
            .field("limits", &self.limits)
            .field("line_bytes", &self.line.len())
            .field("data_bytes", &self.data.len())
            .field("events", &self.events)
            .field("total_bytes", &self.total_bytes)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl SseDecoder {
    /// Construct without reading input or acquiring any external authority.
    ///
    /// # Errors
    /// Rejects invalid limits before allocating input buffers.
    pub fn new(limits: SseLimits) -> Result<Self, SseError> {
        Ok(Self {
            limits: limits.validate()?,
            line: Vec::new(),
            data: String::new(),
            pending_cr: false,
            fields: 0,
            events: 0,
            total_bytes: 0,
            closed: false,
        })
    }

    /// Process at most `max_push_bytes` and stop at the first emitted event.
    ///
    /// Empty input is a no-op, not EOF. Ignored fields/comments still consume
    /// raw-byte and line budgets. Callers own queue limits and must yield between
    /// bounded pushes instead of draining an unbounded stream in one task poll.
    ///
    /// # Errors
    /// Every error permanently closes the decoder and releases retained payloads.
    pub fn push(&mut self, chunk: &[u8]) -> Result<SseProgress, SseError> {
        if self.closed {
            return Err(SseError::Closed);
        }
        match self.push_inner(chunk) {
            Ok(progress) => Ok(progress),
            Err(error) => {
                self.close();
                Err(error)
            }
        }
    }

    fn push_inner(&mut self, chunk: &[u8]) -> Result<SseProgress, SseError> {
        let count = chunk.len().min(self.limits.max_push_bytes);
        for (index, byte) in chunk[..count].iter().copied().enumerate() {
            if self.total_bytes == self.limits.max_total_bytes {
                return Err(SseError::TotalLimit);
            }
            self.total_bytes += 1;
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if matches!(byte, b'\r' | b'\n') {
                self.pending_cr = byte == b'\r';
                let mut line = std::mem::take(&mut self.line);
                let text = std::str::from_utf8(&line).map_err(|_| SseError::InvalidUtf8)?;
                let result = self.process_line(text);
                line.clear();
                self.line = line;
                if let Some(event) = result? {
                    return Ok(SseProgress {
                        consumed: index + 1,
                        event: Some(event),
                    });
                }
            } else {
                if self.line.len() == self.limits.max_line_bytes {
                    return Err(SseError::LineLimit);
                }
                grow_bytes(&mut self.line, 1, self.limits.max_line_bytes);
                self.line.push(byte);
            }
        }
        Ok(SseProgress {
            consumed: count,
            event: None,
        })
    }

    /// Mark EOF without dispatching a partial event.
    ///
    /// A trailing CR is already a complete line terminator. EOF succeeds after
    /// complete ignored/comment lines and ignored empty data.
    ///
    /// # Errors
    /// Rejects invalid UTF-8, an unterminated line or pending event; always closes.
    pub fn finish(&mut self) -> Result<(), SseError> {
        if self.closed {
            return Err(SseError::Closed);
        }
        let result = if std::str::from_utf8(&self.line).is_err() {
            Err(SseError::InvalidUtf8)
        } else if !self.line.is_empty() || self.has_pending() {
            Err(SseError::IncompleteStream)
        } else {
            Ok(())
        };
        self.close();
        result
    }

    fn has_pending(&self) -> bool {
        !self.data.is_empty()
    }

    fn process_line(&mut self, line: &str) -> Result<Option<SseEvent>, SseError> {
        if line.is_empty() {
            self.fields = 0;
            if !self.has_pending() {
                return Ok(None);
            }
            if self.events == self.limits.max_events {
                return Err(SseError::EventLimit);
            }
            self.events += 1;
            let event = SseEvent {
                data: std::mem::take(&mut self.data),
            };
            return Ok(Some(event));
        }
        if self.fields == self.limits.max_fields {
            return Err(SseError::FieldCountLimit);
        }
        self.fields += 1;
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        if field == "data" {
            let separator = !self.data.is_empty();
            let additional = value.len() + usize::from(separator);
            if additional > self.limits.max_data_bytes - self.data.len() {
                return Err(SseError::DataLimit);
            }
            grow_string(&mut self.data, additional, self.limits.max_data_bytes);
            if separator {
                self.data.push('\n');
            }
            self.data.push_str(value);
        }
        Ok(None)
    }

    fn close(&mut self) {
        self.closed = true;
        self.line = Vec::new();
        self.data = String::new();
    }
}

fn grow_bytes(buffer: &mut Vec<u8>, additional: usize, cap: usize) {
    let needed = buffer.len() + additional;
    if needed > buffer.capacity() {
        let capacity = needed.max(buffer.capacity().saturating_mul(2)).min(cap);
        buffer.reserve_exact(capacity - buffer.len());
    }
}

fn grow_string(buffer: &mut String, additional: usize, cap: usize) {
    let needed = buffer.len() + additional;
    if needed > buffer.capacity() {
        let capacity = needed.max(buffer.capacity().saturating_mul(2)).min(cap);
        buffer.reserve_exact(capacity - buffer.len());
    }
}
