//! Bounded, effect-free selection from one immutable canonical observation.

use crate::MAX_FILE_SESSION_BYTES;
use machine_god_core::{ContentBlock, Role, SessionRecord};
use std::{fmt, sync::Arc};

const SCAN_STEPS: usize = 256;
const COPY_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeClipboardReplyError {
    ResourceLimit,
}
impl fmt::Display for NativeClipboardReplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("clipboard reply exceeds resource limits")
    }
}
impl std::error::Error for NativeClipboardReplyError {}

#[derive(Clone)]
pub enum NativeClipboardReplyStep {
    Progress,
    Selected(Arc<str>),
    Empty,
}
impl fmt::Debug for NativeClipboardReplyStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeClipboardReplyStep { .. }")
    }
}

enum Phase {
    Search,
    Inspect,
    Copy,
    Done(Result<NativeClipboardReplyStep, NativeClipboardReplyError>),
}

/// Selects the latest nonempty, tool-call-free assistant message, preserving
/// original text bytes and block order. Metadata does not define reply kind.
/// Construction pins an already owned snapshot without traversing its contents.
pub struct NativeClipboardReplySelection {
    record: Arc<SessionRecord>,
    message: usize,
    block: usize,
    offset: usize,
    length: usize,
    buffer: String,
    phase: Phase,
}
impl fmt::Debug for NativeClipboardReplySelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeClipboardReplySelection { .. }")
    }
}

impl NativeClipboardReplySelection {
    #[must_use]
    pub fn new(record: Arc<SessionRecord>) -> Self {
        Self {
            message: record.messages.len(),
            record,
            block: 0,
            offset: 0,
            length: 0,
            buffer: String::new(),
            phase: Phase::Search,
        }
    }

    /// Visits at most 256 message/block positions and copies at most 4,096 text
    /// bytes per advance. Eligibility is fully checked before payload allocation.
    ///
    /// The final safe `String` to `Arc<str>` conversion separately copies at most
    /// `MAX_FILE_SESSION_BYTES` once. Peak owned payload is twice that byte cap
    /// during conversion, excluding allocator overhead and the existing snapshot.
    /// Terminal steps are fused; repeated selection only clones the result Arc.
    ///
    /// # Errors
    /// Returns `ResourceLimit` for an oversized latest eligible reply or rejected
    /// buffer reservation. Never falls back to an older reply in that case.
    pub fn next_step(&mut self) -> Result<NativeClipboardReplyStep, NativeClipboardReplyError> {
        let mut copied = 0;
        for _ in 0..SCAN_STEPS {
            match &self.phase {
                Phase::Done(result) => return result.clone(),
                Phase::Search => {
                    if self.message == 0 {
                        return self.finish(Ok(NativeClipboardReplyStep::Empty));
                    }
                    self.message -= 1;
                    if self.record.messages[self.message].role == Role::Assistant {
                        self.block = 0;
                        self.length = 0;
                        self.phase = Phase::Inspect;
                    }
                }
                Phase::Inspect => {
                    let content = &self.record.messages[self.message].content;
                    if self.block == content.len() {
                        if self.length == 0 {
                            self.phase = Phase::Search;
                        } else {
                            return self.begin_copy();
                        }
                    } else {
                        match &content[self.block] {
                            ContentBlock::ToolCall { .. } => self.phase = Phase::Search,
                            ContentBlock::Text { text } => {
                                self.length = self
                                    .length
                                    .saturating_add(text.len())
                                    .min(MAX_FILE_SESSION_BYTES + 1);
                            }
                            _ => {}
                        }
                        self.block += 1;
                    }
                }
                Phase::Copy => {
                    let content = &self.record.messages[self.message].content;
                    if self.block == content.len() {
                        let selected = Arc::<str>::from(self.buffer.as_str());
                        self.buffer = String::new();
                        return self.finish(Ok(NativeClipboardReplyStep::Selected(selected)));
                    }
                    if copied == COPY_BYTES {
                        break;
                    }
                    if let ContentBlock::Text { text } = &content[self.block] {
                        let end = text
                            .floor_char_boundary(text.len().min(self.offset + COPY_BYTES - copied));
                        if end == self.offset && end != text.len() {
                            break;
                        }
                        self.buffer.push_str(&text[self.offset..end]);
                        copied += end - self.offset;
                        self.offset = end;
                        if end != text.len() {
                            continue;
                        }
                    }
                    self.block += 1;
                    self.offset = 0;
                }
            }
        }
        Ok(NativeClipboardReplyStep::Progress)
    }

    fn begin_copy(&mut self) -> Result<NativeClipboardReplyStep, NativeClipboardReplyError> {
        if self.length > MAX_FILE_SESSION_BYTES
            || self.buffer.try_reserve_exact(self.length).is_err()
        {
            return self.finish(Err(NativeClipboardReplyError::ResourceLimit));
        }
        self.block = 0;
        self.offset = 0;
        self.phase = Phase::Copy;
        Ok(NativeClipboardReplyStep::Progress)
    }

    fn finish(
        &mut self,
        result: Result<NativeClipboardReplyStep, NativeClipboardReplyError>,
    ) -> Result<NativeClipboardReplyStep, NativeClipboardReplyError> {
        self.phase = Phase::Done(result.clone());
        result
    }
}

#[cfg(test)]
mod tests;
