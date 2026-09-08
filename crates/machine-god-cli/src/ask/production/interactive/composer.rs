//! Pure bounded raw-terminal editing. Cursor offsets are UTF-8 byte offsets;
//! character movement uses the pinned display unit plus zero-width continuation
//! rule, not Unicode scalar or generic grapheme boundaries.
//!
//! Bracketed paste is an atomic insertion. Invalid/oversized paste is discarded
//! through its closing marker, retaining the previous draft. Invalid/oversized
//! ordinary text rejects that attempted insertion, retains the previous draft,
//! and drains through Enter without submitting a truncated prefix. That recovery
//! boundary emits Changed; reset and idle Ctrl-C are explicit draft discards.
//! Physical EOF belongs to the input owner and is never inferred from byte 4.

use machine_god_native::native_terminal_display_unit_at;
use std::fmt;

pub(super) const MAX_COMPOSER_BYTES: usize = 256 * 1024;
pub(super) const MAX_COMPOSER_STEP_BYTES: usize = 4096;
const MAX_ESCAPE_BYTES: usize = 32;
const PASTE_END: &[u8] = b"\x1b[201~";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ComposerContext {
    pub active_response: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComposerInputError {
    TooLong,
    InvalidUtf8,
    ContainsNul,
    InvalidEscape,
}
impl fmt::Display for ComposerInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLong => "interactive draft exceeds byte limit",
            Self::InvalidUtf8 => "interactive input is not valid UTF-8",
            Self::ContainsNul => "interactive input contains a NUL byte",
            Self::InvalidEscape => "interactive input escape sequence is unsupported",
        })
    }
}
impl std::error::Error for ComposerInputError {}

pub(super) enum ComposerEvent {
    Submit(String),
    CancelRequested,
    ExitRequested,
    Changed,
    InputError(ComposerInputError),
}
impl fmt::Debug for ComposerEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Submit(_) => f.write_str("Submit(..)"),
            Self::CancelRequested => f.write_str("CancelRequested"),
            Self::ExitRequested => f.write_str("ExitRequested"),
            Self::Changed => f.write_str("Changed"),
            Self::InputError(error) => f.debug_tuple("InputError").field(error).finish(),
        }
    }
}

#[derive(Default)]
pub(super) struct Composer {
    text: String,
    cursor: usize,
    decoder: Decoder,
    skip_lf: bool,
}
impl fmt::Debug for Composer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Composer")
            .field("bytes", &self.text.len())
            .field("cursor", &self.cursor)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
enum Decoder {
    #[default]
    Ready,
    Utf8(Utf8),
    Escape {
        bytes: [u8; MAX_ESCAPE_BYTES],
        len: usize,
    },
    Osc {
        escape: bool,
    },
    Paste(Paste),
    RejectLine,
}

#[derive(Default)]
struct Utf8 {
    bytes: [u8; 4],
    len: usize,
}
impl Utf8 {
    fn push(&mut self, byte: u8) -> Result<Option<&str>, ComposerInputError> {
        if self.len == self.bytes.len() {
            return Err(ComposerInputError::InvalidUtf8);
        }
        self.bytes[self.len] = byte;
        self.len += 1;
        match std::str::from_utf8(&self.bytes[..self.len]) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.error_len().is_none() => Ok(None),
            Err(_) => Err(ComposerInputError::InvalidUtf8),
        }
    }
}

#[derive(Default)]
struct Paste {
    text: String,
    utf8: Utf8,
    marker: usize,
    failed: bool,
    skip_lf: bool,
}

impl Composer {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub const fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Consumes at most 4,096 bytes and emits at most one event. Empty input is
    /// inert. A returned prefix must be removed before feeding the remainder.
    pub fn feed(
        &mut self,
        bytes: &[u8],
        context: ComposerContext,
    ) -> (usize, Option<ComposerEvent>) {
        let bytes = &bytes[..bytes.len().min(MAX_COMPOSER_STEP_BYTES)];
        let mut consumed = 0;
        while consumed < bytes.len() {
            if bytes[consumed] == 3 && !matches!(self.decoder, Decoder::Paste(_)) {
                self.decoder = Decoder::Ready;
                self.skip_lf = false;
                return (consumed + 1, self.key(3, context));
            }
            if matches!(self.decoder, Decoder::Ready) {
                if self.skip_lf {
                    self.skip_lf = false;
                    if bytes[consumed] == b'\n' {
                        consumed += 1;
                        continue;
                    }
                }
                if printable(bytes[consumed]) {
                    let available = &bytes[consumed..];
                    let end = available
                        .iter()
                        .position(|byte| !printable(*byte))
                        .unwrap_or(available.len());
                    let candidate = &available[..end];
                    let valid = match std::str::from_utf8(candidate) {
                        Ok(text) => text.len(),
                        Err(error) => error.valid_up_to(),
                    };
                    if valid > 0 {
                        let text =
                            std::str::from_utf8(&candidate[..valid]).expect("validated prefix");
                        let event = self.insert(text);
                        consumed += valid;
                        if matches!(event, ComposerEvent::InputError(_)) {
                            self.decoder = Decoder::RejectLine;
                        }
                        return (consumed, Some(event));
                    }
                    self.decoder = Decoder::Utf8(Utf8::default());
                }
            }
            let byte = bytes[consumed];
            consumed += 1;
            let decoder = std::mem::take(&mut self.decoder);
            let event = match decoder {
                Decoder::Ready => self.key(byte, context),
                Decoder::Utf8(mut utf8) => match utf8.push(byte) {
                    Ok(Some(text)) => {
                        let event = self.insert(text);
                        if matches!(event, ComposerEvent::InputError(_)) {
                            self.decoder = Decoder::RejectLine;
                        }
                        Some(event)
                    }
                    Ok(None) => {
                        self.decoder = Decoder::Utf8(utf8);
                        None
                    }
                    Err(error) => {
                        // A delimiter proving incomplete UTF-8 is already the
                        // rejection boundary; never also turn it into Submit.
                        self.reject_at(byte);
                        Some(ComposerEvent::InputError(error))
                    }
                },
                Decoder::Escape { bytes, len } => self.escape(byte, bytes, len),
                Decoder::Osc { escape } => {
                    if matches!(byte, b'\r' | b'\n') {
                        self.skip_lf = byte == b'\r';
                        Some(ComposerEvent::Changed)
                    } else if byte != 7 && !(escape && byte == b'\\') {
                        self.decoder = Decoder::Osc { escape: byte == 27 };
                        None
                    } else {
                        None
                    }
                }
                Decoder::Paste(paste) => self.paste_byte(paste, byte),
                Decoder::RejectLine => match byte {
                    b'\r' | b'\n' => {
                        self.skip_lf = byte == b'\r';
                        Some(ComposerEvent::Changed)
                    }
                    3 => self.key(byte, context),
                    _ => {
                        self.decoder = Decoder::RejectLine;
                        None
                    }
                },
            };
            if event.is_some() {
                return (consumed, event);
            }
        }
        (consumed, None)
    }

    fn insert(&mut self, text: &str) -> ComposerEvent {
        if text.len() > MAX_COMPOSER_BYTES - self.text.len() {
            return ComposerEvent::InputError(ComposerInputError::TooLong);
        }
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        ComposerEvent::Changed
    }

    fn key(&mut self, byte: u8, context: ComposerContext) -> Option<ComposerEvent> {
        match byte {
            b'\r' | b'\n' => {
                self.cursor = 0;
                self.skip_lf = byte == b'\r';
                Some(ComposerEvent::Submit(std::mem::take(&mut self.text)))
            }
            3 => {
                if !context.active_response {
                    self.reset();
                }
                Some(ComposerEvent::CancelRequested)
            }
            4 if self.is_empty() => {
                (!context.active_response).then_some(ComposerEvent::ExitRequested)
            }
            4 => self.delete_forward(),
            8 | 127 => {
                let previous = previous_start(&self.text, self.cursor);
                if previous == self.cursor {
                    return None;
                }
                self.text.replace_range(previous..self.cursor, "");
                self.cursor = previous;
                Some(ComposerEvent::Changed)
            }
            1 => self.move_to(0),
            5 => self.move_to(self.text.len()),
            2 => self.move_to(previous_start(&self.text, self.cursor)),
            6 => self.move_to(next_end(&self.text, self.cursor)),
            12 => Some(ComposerEvent::Changed),
            27 => {
                let mut bytes = [0; MAX_ESCAPE_BYTES];
                bytes[0] = 27;
                self.decoder = Decoder::Escape { bytes, len: 1 };
                None
            }
            0 => {
                self.decoder = Decoder::RejectLine;
                Some(ComposerEvent::InputError(ComposerInputError::ContainsNul))
            }
            _ => None,
        }
    }

    fn move_to(&mut self, cursor: usize) -> Option<ComposerEvent> {
        if self.cursor == cursor {
            None
        } else {
            self.cursor = cursor;
            Some(ComposerEvent::Changed)
        }
    }
    fn delete_forward(&mut self) -> Option<ComposerEvent> {
        let end = next_end(&self.text, self.cursor);
        if end == self.cursor {
            return None;
        }
        self.text.replace_range(self.cursor..end, "");
        Some(ComposerEvent::Changed)
    }

    fn reject_at(&mut self, byte: u8) {
        self.skip_lf = byte == b'\r';
        if !matches!(byte, b'\r' | b'\n') {
            self.decoder = Decoder::RejectLine;
        }
    }

    fn escape(
        &mut self,
        byte: u8,
        mut bytes: [u8; MAX_ESCAPE_BYTES],
        len: usize,
    ) -> Option<ComposerEvent> {
        if len == MAX_ESCAPE_BYTES || matches!(byte, b'\r' | b'\n' | 0) {
            self.reject_at(byte);
            return Some(ComposerEvent::InputError(ComposerInputError::InvalidEscape));
        }
        bytes[len] = byte;
        let len = len + 1;
        if len == 2 {
            match byte {
                b'[' | b'O' => {
                    self.decoder = Decoder::Escape { bytes, len };
                    return None;
                }
                b']' => {
                    self.decoder = Decoder::Osc { escape: false };
                    return Some(ComposerEvent::InputError(ComposerInputError::InvalidEscape));
                }
                27 => return self.key(27, ComposerContext::default()),
                _ => return Some(ComposerEvent::InputError(ComposerInputError::InvalidEscape)),
            }
        }
        if (0x20..=0x3f).contains(&byte) && bytes[1] == b'[' {
            self.decoder = Decoder::Escape { bytes, len };
            return None;
        }
        match &bytes[..len] {
            b"\x1b[D" | b"\x1bOD" => self.move_to(previous_start(&self.text, self.cursor)),
            b"\x1b[C" | b"\x1bOC" => self.move_to(next_end(&self.text, self.cursor)),
            b"\x1b[H" | b"\x1bOH" | b"\x1b[1~" | b"\x1b[7~" => self.move_to(0),
            b"\x1b[F" | b"\x1bOF" | b"\x1b[4~" | b"\x1b[8~" => self.move_to(self.text.len()),
            b"\x1b[3~" => self.delete_forward(),
            b"\x1b[200~" => {
                self.decoder = Decoder::Paste(Paste::default());
                None
            }
            _ => Some(ComposerEvent::InputError(ComposerInputError::InvalidEscape)),
        }
    }

    fn paste_byte(&mut self, mut paste: Paste, byte: u8) -> Option<ComposerEvent> {
        if byte == PASTE_END[paste.marker] {
            paste.marker += 1;
            if paste.marker == PASTE_END.len() {
                if paste.failed {
                    return Some(ComposerEvent::Changed);
                }
                if paste.utf8.len != 0 {
                    return Some(ComposerEvent::InputError(ComposerInputError::InvalidUtf8));
                }
                return Some(self.insert(&paste.text));
            }
            self.decoder = Decoder::Paste(paste);
            return None;
        }
        let mut error = None;
        for &pending in &PASTE_END[..paste.marker] {
            if let Err(failure) = paste.push(pending, MAX_COMPOSER_BYTES - self.text.len()) {
                error = Some(failure);
                break;
            }
        }
        paste.marker = 0;
        if byte == PASTE_END[0] {
            paste.marker = 1;
        } else if let Err(failure) = paste.push(byte, MAX_COMPOSER_BYTES - self.text.len()) {
            error = Some(failure);
        }
        self.decoder = Decoder::Paste(paste);
        error.map(ComposerEvent::InputError)
    }
}

impl Paste {
    fn push(&mut self, byte: u8, limit: usize) -> Result<(), ComposerInputError> {
        if self.failed {
            return Ok(());
        }
        let result = self.push_validated(byte, limit);
        if result.is_err() {
            self.failed = true;
            self.text = String::new();
            self.utf8 = Utf8::default();
        }
        result
    }
    fn push_validated(&mut self, byte: u8, limit: usize) -> Result<(), ComposerInputError> {
        if byte == 0 {
            return Err(ComposerInputError::ContainsNul);
        }
        if self.skip_lf {
            self.skip_lf = false;
            if byte == b'\n' {
                return Ok(());
            }
        }
        let byte = if byte == b'\r' {
            self.skip_lf = true;
            b'\n'
        } else {
            byte
        };
        if let Some(text) = self.utf8.push(byte)? {
            if text.len() > limit - self.text.len() {
                return Err(ComposerInputError::TooLong);
            }
            self.text.push_str(text);
            self.utf8 = Utf8::default();
        }
        Ok(())
    }
}

fn printable(byte: u8) -> bool {
    byte == b'\t' || byte >= 32 && byte != 127
}

fn next_end(text: &str, start: usize) -> usize {
    let Some(first) = native_terminal_display_unit_at(text, start) else {
        return text.len();
    };
    let mut end = start + first.byte_len;
    if control_boundary(text, start) {
        return end;
    }
    while let Some(next) = native_terminal_display_unit_at(text, end) {
        if next.cell_width != 0 || control_boundary(text, end) {
            break;
        }
        end += next.byte_len;
    }
    end
}
fn previous_start(text: &str, cursor: usize) -> usize {
    let mut previous = 0;
    let mut next = 0;
    while next < cursor {
        previous = next;
        next = next_end(text, next);
    }
    previous
}
fn control_boundary(text: &str, index: usize) -> bool {
    text.as_bytes()
        .get(index)
        .is_some_and(|byte| *byte < 32 || *byte == 127)
}

#[cfg(test)]
#[path = "composer/tests.rs"]
mod tests;
