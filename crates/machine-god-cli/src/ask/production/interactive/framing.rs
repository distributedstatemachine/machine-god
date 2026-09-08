//! Pure bounded byte framing; no input acquisition, slash routing or submission.

use std::fmt;

pub(super) const MAX_INTERACTIVE_INPUT_LINE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InteractiveInputFrameError {
    TooLong,
    InvalidUtf8,
    ContainsNul,
}

impl fmt::Display for InteractiveInputFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLong => "interactive input exceeds byte limit",
            Self::InvalidUtf8 => "interactive input is not valid UTF-8",
            Self::ContainsNul => "interactive input contains a NUL byte",
        })
    }
}
impl std::error::Error for InteractiveInputFrameError {}

/// Retains one raw line and at most one deferred CR, never a queue of frames.
#[derive(Default)]
pub(super) struct InteractiveInputFramer {
    line: Vec<u8>,
    pending_cr: bool,
    discarding: bool,
    finished: bool,
}

impl fmt::Debug for InteractiveInputFramer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InteractiveInputFramer")
            .field("buffered_bytes", &self.line.len())
            .field("pending_cr", &self.pending_cr)
            .field("discarding", &self.discarding)
            .field("finished", &self.finished)
            .finish()
    }
}

impl InteractiveInputFramer {
    /// Consumes through at most one frame/error. The caller retains all bytes
    /// after the returned count. LF ends a line; only a CR directly before LF
    /// is removed. A bare CR is content, including at EOF.
    ///
    /// The first proven oversized byte emits one error; later calls discard
    /// through LF before resuming framing, without retaining discarded bytes.
    /// UTF-8 and NUL validation wait until a complete frame (NUL takes priority).
    /// Empty input and input after `finish` consume zero and return no event.
    pub(super) fn feed(
        &mut self,
        bytes: &[u8],
    ) -> (usize, Option<Result<String, InteractiveInputFrameError>>) {
        if self.finished {
            return (0, None);
        }
        for (index, &byte) in bytes.iter().enumerate() {
            let consumed = index + 1;
            if self.discarding {
                if byte == b'\n' {
                    self.discarding = false;
                }
                continue;
            }
            if byte == b'\n' {
                self.pending_cr = false;
                return (consumed, Some(self.complete_line()));
            }
            if self.pending_cr {
                self.pending_cr = false;
                if !self.append(b'\r') {
                    return (consumed, Some(Err(self.reject_oversize())));
                }
            }
            if byte == b'\r' {
                self.pending_cr = true;
            } else if !self.append(byte) {
                return (consumed, Some(Err(self.reject_oversize())));
            }
        }
        (bytes.len(), None)
    }

    /// Flushes one nonempty partial line once. EOF after a delimiter does not
    /// invent a blank line. An oversized line already reported by `feed` is
    /// silently retired; a deferred CR can prove oversize for the first time.
    pub(super) fn finish(&mut self) -> Option<Result<String, InteractiveInputFrameError>> {
        if self.finished {
            return None;
        }
        self.finished = true;
        if self.discarding {
            return None;
        }
        if self.pending_cr {
            self.pending_cr = false;
            if !self.append(b'\r') {
                return Some(Err(self.reject_oversize()));
            }
        }
        if self.line.is_empty() {
            None
        } else {
            Some(self.complete_line())
        }
    }

    fn append(&mut self, byte: u8) -> bool {
        if self.line.len() == MAX_INTERACTIVE_INPUT_LINE_BYTES {
            return false;
        }
        if self.line.len() == self.line.capacity() {
            let capacity = (self.line.capacity() * 2).clamp(64, MAX_INTERACTIVE_INPUT_LINE_BYTES);
            self.line.reserve_exact(capacity - self.line.len());
        }
        self.line.push(byte);
        true
    }

    fn reject_oversize(&mut self) -> InteractiveInputFrameError {
        self.line.clear();
        self.pending_cr = false;
        self.discarding = true;
        InteractiveInputFrameError::TooLong
    }

    fn complete_line(&mut self) -> Result<String, InteractiveInputFrameError> {
        if self.line.contains(&0) {
            self.line.clear();
            return Err(InteractiveInputFrameError::ContainsNul);
        }
        // A tiny queued String must not carry the previous oversized line's
        // allocation. Box conversion explicitly removes spare Vec capacity.
        let bytes = std::mem::take(&mut self.line).into_boxed_slice().into_vec();
        String::from_utf8(bytes).map_err(|_| InteractiveInputFrameError::InvalidUtf8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(bytes: &[u8], chunk_size: usize) -> Vec<Result<String, InteractiveInputFrameError>> {
        let mut framer = InteractiveInputFramer::default();
        let mut results = Vec::new();
        for chunk in bytes.chunks(chunk_size) {
            let mut remainder = chunk;
            while !remainder.is_empty() {
                let (consumed, frame) = framer.feed(remainder);
                assert!((1..=remainder.len()).contains(&consumed));
                remainder = &remainder[consumed..];
                if let Some(frame) = frame {
                    results.push(frame);
                }
                assert!(framer.line.len() <= MAX_INTERACTIVE_INPUT_LINE_BYTES);
                assert!(framer.line.capacity() <= MAX_INTERACTIVE_INPUT_LINE_BYTES);
            }
        }
        results.extend(framer.finish());
        assert_eq!(framer.finish(), None);
        assert_eq!(framer.feed(b"after EOF\n"), (0, None));
        results
    }

    #[test]
    fn one_frame_stops_before_remainder_and_blank_lines_are_valid() {
        let mut framer = InteractiveInputFramer::default();
        assert_eq!(framer.feed(b"one\nsecond\n"), (4, Some(Ok("one".into()))));
        assert_eq!(framer.feed(b"\n\r\n"), (1, Some(Ok(String::new()))));
        assert_eq!(framer.feed(b"\r\n"), (2, Some(Ok(String::new()))));
        assert_eq!(framer.finish(), None);
    }

    #[test]
    fn chunk_boundaries_preserve_utf8_crlf_and_bare_carriage_returns() {
        let bytes = "😀\r\n\rcontent\r\rx\nlast\r".as_bytes();
        for chunk_size in 1..=bytes.len() {
            assert_eq!(
                collect(bytes, chunk_size),
                vec![
                    Ok("😀".into()),
                    Ok("\rcontent\r\rx".into()),
                    Ok("last\r".into()),
                ]
            );
        }
    }

    #[test]
    fn incomplete_utf8_waits_for_delimiter_or_eof() {
        let mut framer = InteractiveInputFramer::default();
        assert_eq!(framer.feed(&[0xf0, 0x9f]), (2, None));
        assert_eq!(framer.feed(&[0x98, 0x80]), (2, None));
        assert_eq!(framer.finish(), Some(Ok("😀".into())));
        assert_eq!(
            collect(&[0xf0, 0x9f], 1),
            vec![Err(InteractiveInputFrameError::InvalidUtf8)]
        );
    }

    #[test]
    fn invalid_utf8_and_nul_report_once_then_resume_at_next_line() {
        for chunk_size in 1..=12 {
            assert_eq!(
                collect(b"\xff\n\0\r\nok\n", chunk_size),
                vec![
                    Err(InteractiveInputFrameError::InvalidUtf8),
                    Err(InteractiveInputFrameError::ContainsNul),
                    Ok("ok".into()),
                ]
            );
        }
        assert_eq!(
            collect(b"\xff\0", 1),
            vec![Err(InteractiveInputFrameError::ContainsNul)]
        );
    }

    #[test]
    fn eof_is_idempotent_for_empty_partial_blank_and_bare_cr() {
        for (bytes, expected) in [
            (b"".as_slice(), vec![]),
            (b"partial", vec![Ok("partial".into())]),
            (b"\n", vec![Ok(String::new())]),
            (b"\r", vec![Ok("\r".into())]),
        ] {
            assert_eq!(collect(bytes, 1), expected);
        }
        let mut framer = InteractiveInputFramer::default();
        assert_eq!(framer.feed(b""), (0, None));
        assert_eq!(framer.feed(b"x"), (1, None));
        assert_eq!(framer.feed(b""), (0, None));
        assert_eq!(framer.finish(), Some(Ok("x".into())));
    }

    #[test]
    fn exact_limit_accepts_lf_crlf_and_eof_without_counting_delimiters() {
        let line = vec![b'a'; MAX_INTERACTIVE_INPUT_LINE_BYTES];
        for ending in [b"\n".as_slice(), b"\r\n", b""] {
            let mut framer = InteractiveInputFramer::default();
            assert_eq!(framer.feed(&line), (line.len(), None));
            assert!(framer.line.capacity() <= MAX_INTERACTIVE_INPUT_LINE_BYTES);
            let (_, frame) = framer.feed(ending);
            let output = frame.or_else(|| framer.finish()).unwrap().unwrap();
            assert_eq!(output.as_bytes(), line);
            assert_eq!(output.capacity(), output.len());
        }
    }

    #[test]
    fn first_oversize_stops_consumption_then_discards_without_growth() {
        let mut framer = InteractiveInputFramer::default();
        let line = vec![b'a'; MAX_INTERACTIVE_INPUT_LINE_BYTES];
        assert_eq!(framer.feed(&line), (line.len(), None));
        assert_eq!(
            framer.feed(b"xignored\nvalid\n"),
            (1, Some(Err(InteractiveInputFrameError::TooLong)))
        );
        let capacity = framer.line.capacity();
        for _ in 0..10 {
            assert_eq!(framer.feed(&line), (line.len(), None));
            assert!(framer.line.is_empty());
            assert_eq!(framer.line.capacity(), capacity);
        }
        let (consumed, frame) = framer.feed(b"ignored\nvalid\nnext");
        assert_eq!(consumed, 14);
        let output = frame.unwrap().unwrap();
        assert_eq!(output, "valid");
        assert_eq!(output.capacity(), output.len());
        assert_eq!(framer.feed(b"next"), (4, None));
        assert_eq!(framer.finish(), Some(Ok("next".into())));
    }

    #[test]
    fn deferred_cr_only_exceeds_limit_when_proven_content() {
        let line = vec![b'a'; MAX_INTERACTIVE_INPUT_LINE_BYTES];
        for tail in [b"x".as_slice(), b"\r", b""] {
            let mut framer = InteractiveInputFramer::default();
            assert_eq!(framer.feed(&line), (line.len(), None));
            assert_eq!(framer.feed(b"\r"), (1, None));
            assert!(framer.pending_cr);
            let (_, frame) = framer.feed(tail);
            assert_eq!(
                frame.or_else(|| framer.finish()),
                Some(Err(InteractiveInputFrameError::TooLong))
            );
            assert_eq!(framer.finish(), None);
        }
        let mut framer = InteractiveInputFramer::default();
        assert_eq!(framer.feed(&line[..line.len() - 1]), (line.len() - 1, None));
        assert_eq!(
            framer.feed(b"\r\r\n").1.unwrap().unwrap().len(),
            MAX_INTERACTIVE_INPUT_LINE_BYTES
        );
    }

    #[test]
    fn eof_after_already_reported_oversize_does_not_report_again() {
        let line = vec![b'a'; MAX_INTERACTIVE_INPUT_LINE_BYTES + 1];
        let mut framer = InteractiveInputFramer::default();
        assert_eq!(
            framer.feed(&line),
            (line.len(), Some(Err(InteractiveInputFrameError::TooLong)))
        );
        assert_eq!(framer.finish(), None);
        assert_eq!(framer.finish(), None);
    }

    #[test]
    fn invalid_maximum_frame_does_not_lend_large_capacity_to_tiny_valid_frame() {
        let mut line = vec![b'a'; MAX_INTERACTIVE_INPUT_LINE_BYTES];
        line[0] = 0;
        let mut framer = InteractiveInputFramer::default();
        assert_eq!(framer.feed(&line), (line.len(), None));
        assert_eq!(
            framer.feed(b"\n"),
            (1, Some(Err(InteractiveInputFrameError::ContainsNul)))
        );
        let frame = framer.feed(b"x\n").1.unwrap().unwrap();
        assert_eq!(frame, "x");
        assert_eq!(frame.capacity(), 1);
    }

    #[test]
    fn error_and_framer_debug_do_not_disclose_input() {
        let mut framer = InteractiveInputFramer::default();
        assert_eq!(framer.feed(b"PRIVATE_INPUT"), (13, None));
        assert!(!format!("{framer:?}").contains("PRIVATE"));
        for error in [
            InteractiveInputFrameError::TooLong,
            InteractiveInputFrameError::InvalidUtf8,
            InteractiveInputFrameError::ContainsNul,
        ] {
            assert!(!error.to_string().contains("PRIVATE"));
        }
    }
}
