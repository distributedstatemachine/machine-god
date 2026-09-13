use std::fmt;

use super::{ACP_MAX_FRAME_BYTES, AcpMessage, AcpProtocolError, decode_frame};

/// Incremental newline framing with one retained frame and no message queue.
/// Oversize input is reported once, then drained to the next newline. A caller
/// controls backpressure by choosing when to request the next message.
#[derive(Default)]
pub struct AcpFrameDecoder {
    partial: Vec<u8>,
    draining: bool,
}

impl fmt::Debug for AcpFrameDecoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcpFrameDecoder")
            .field("buffered_bytes", &self.partial.len())
            .field("draining", &self.draining)
            .finish()
    }
}

impl AcpFrameDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Consumes input up to the next complete frame or framing error. `None`
    /// means all supplied input was consumed and more bytes are needed.
    /// Empty/whitespace frames are malformed JSON, not silent keep-alives.
    pub fn next(&mut self, input: &mut &[u8]) -> Option<Result<AcpMessage, AcpProtocolError>> {
        loop {
            if input.is_empty() {
                return None;
            }
            let newline = memchr::memchr(b'\n', input);
            let end = newline.unwrap_or(input.len());
            if self.draining {
                *input = &input[end + usize::from(newline.is_some())..];
                if newline.is_some() {
                    self.draining = false;
                    continue;
                }
                return None;
            }
            if end > ACP_MAX_FRAME_BYTES - self.partial.len() {
                self.partial = Vec::new();
                self.draining = newline.is_none();
                *input = &input[end + usize::from(newline.is_some())..];
                return Some(Err(AcpProtocolError::FrameTooLarge));
            }
            // A complete unfragmented frame can be decoded without retaining a
            // second copy of the bytes. CRLF works via JSON whitespace.
            if self.partial.is_empty() && newline.is_some() {
                let result = decode_frame(&input[..end]);
                *input = &input[end + 1..];
                return Some(result);
            }
            if self.partial.capacity() - self.partial.len() < end {
                let target = (self.partial.len() + end)
                    .max(self.partial.capacity().saturating_mul(2))
                    .min(ACP_MAX_FRAME_BYTES);
                self.partial.reserve_exact(target - self.partial.len());
            }
            self.partial.extend_from_slice(&input[..end]);
            *input = &input[end + usize::from(newline.is_some())..];
            if newline.is_some() {
                let result = decode_frame(&self.partial);
                // Do not retain an 8 MiB high-water allocation between frames.
                self.partial = Vec::new();
                return Some(result);
            }
            return None;
        }
    }

    /// Ends this input stream, reclaiming partial bytes. An oversized frame was
    /// already reported by `next`; an ordinary unterminated frame is rejected.
    pub fn finish(&mut self) -> Option<AcpProtocolError> {
        let truncated = !self.partial.is_empty();
        self.partial = Vec::new();
        self.draining = false;
        truncated.then_some(AcpProtocolError::TruncatedFrame)
    }
}
