use std::fmt;

use super::{SseError, SseEvent, SseLimits, SseMode, SseProgress};

/// Inert incremental decoder with finite memory, work and lifetime budgets.
///
/// UTF-8 validation occurs once per completed line, permitting code points to
/// span arbitrary input chunks. A leading BOM is not stripped at the pin.
pub struct SseDecoder {
    mode: SseMode,
    limits: SseLimits,
    line: Vec<u8>,
    data: String,
    event: Option<String>,
    id: Option<String>,
    retry_ms: Option<u32>,
    saw_data: bool,
    pending_cr: bool,
    fields: usize,
    events: usize,
    total_bytes: usize,
    closed: bool,
}

impl fmt::Debug for SseDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SseDecoder")
            .field("mode", &self.mode)
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
    pub fn new(mode: SseMode, limits: SseLimits) -> Result<Self, SseError> {
        Ok(Self {
            mode,
            limits: limits.validate()?,
            line: Vec::new(),
            data: String::new(),
            event: None,
            id: None,
            retry_ms: None,
            saw_data: false,
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
    /// complete ignored/comment lines and, in modern mode, ignored empty data.
    /// Legacy recognized fields require a terminating blank line.
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
        match self.mode {
            SseMode::Modern => !self.data.is_empty(),
            SseMode::Legacy => {
                self.saw_data
                    || self.event.is_some()
                    || self.id.is_some()
                    || self.retry_ms.is_some()
            }
        }
    }

    fn process_line(&mut self, line: &str) -> Result<Option<SseEvent>, SseError> {
        if line.is_empty() {
            self.fields = 0;
            if !self.has_pending() {
                self.saw_data = false;
                return Ok(None);
            }
            if self.events == self.limits.max_events {
                return Err(SseError::EventLimit);
            }
            self.events += 1;
            let event = SseEvent {
                data: std::mem::take(&mut self.data),
                event: self.event.take(),
                id: self.id.take(),
                retry_ms: self.retry_ms.take(),
                had_data_field: self.saw_data,
            };
            self.saw_data = false;
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
            let separator = match self.mode {
                SseMode::Modern => !self.data.is_empty(),
                SseMode::Legacy => self.saw_data,
            };
            let additional = value.len() + usize::from(separator);
            if additional > self.limits.max_data_bytes - self.data.len() {
                return Err(SseError::DataLimit);
            }
            grow_string(&mut self.data, additional, self.limits.max_data_bytes);
            if separator {
                self.data.push('\n');
            }
            self.data.push_str(value);
            self.saw_data = true;
        } else if self.mode == SseMode::Legacy {
            match field {
                "event" => replace(&mut self.event, value, self.limits.max_field_bytes)?,
                "id" if !value.contains('\0') => {
                    replace(&mut self.id, value, self.limits.max_field_bytes)?;
                }
                "retry" => {
                    if let Some(value) = parse_retry(value) {
                        self.retry_ms = Some(value);
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn close(&mut self) {
        self.closed = true;
        self.line = Vec::new();
        self.data = String::new();
        self.event = None;
        self.id = None;
        self.retry_ms = None;
        self.saw_data = false;
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

fn replace(target: &mut Option<String>, value: &str, cap: usize) -> Result<(), SseError> {
    if value.len() > cap {
        return Err(SseError::FieldLimit);
    }
    let target = target.get_or_insert_with(String::new);
    target.clear();
    grow_string(target, value.len(), cap);
    target.push_str(value);
    Ok(())
}

// The pin uses std.fmt.parseInt(u32, value, 10), not browser digits-only
// parsing: optional '+', internal underscores and negative zero are accepted.
fn parse_retry(value: &str) -> Option<u32> {
    let (negative, digits) = if let Some(rest) = value.strip_prefix('-') {
        (true, rest)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') {
        return None;
    }
    let mut parsed = 0_u32;
    for byte in digits.bytes() {
        if byte == b'_' {
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        parsed = parsed
            .checked_mul(10)?
            .checked_add(u32::from(byte - b'0'))?;
        if negative && parsed != 0 {
            return None;
        }
    }
    Some(parsed.min(60_000))
}
