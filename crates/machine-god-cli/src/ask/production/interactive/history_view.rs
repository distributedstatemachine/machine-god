//! Lazy projection of a captured canonical record, never an execution replay.

use machine_god_core::{ContentBlock, Role, SessionRecord, ToolOutput};
use std::fmt;

const CHUNK_BYTES: usize = 4096;
const SCAN_STEPS: usize = 256;

pub(super) enum HistoryViewStep {
    Chunk(Vec<u8>),
    /// The bounded scan advanced without visible output. Keep driving native
    /// work and schedule another presentation poll; this is not end-of-history.
    Progress,
    Done,
}

impl fmt::Debug for HistoryViewStep {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Chunk(_) => "HistoryViewStep::Chunk(..)",
            Self::Progress => "HistoryViewStep::Progress",
            Self::Done => "HistoryViewStep::Done",
        })
    }
}

pub(super) struct HistoryView {
    record: SessionRecord,
    message: usize,
    block: usize,
    part: u8,
    offset: usize,
    header: bool,
}

impl fmt::Debug for HistoryView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HistoryView { .. }")
    }
}

impl HistoryView {
    /// Takes the one already-captured presentation snapshot. Construction does
    /// not scan or clone history, inspect metadata, or acquire any authority.
    pub(super) fn new(record: SessionRecord) -> Self {
        Self {
            record,
            message: 0,
            block: 0,
            part: 0,
            offset: 0,
            header: true,
        }
    }

    /// Complete text is streamed with whole UTF-8 scalars and whole escapes.
    /// Raw JSON, tool arguments/results and system instructions stay collapsed;
    /// status describes the saved record, not whether any effect happened.
    /// Typed command/diff/background cards require their native presentation
    /// adapters and are deliberately not inferred from arbitrary output JSON.
    pub(super) fn next_chunk(&mut self) -> HistoryViewStep {
        if self.message == self.record.messages.len() {
            return HistoryViewStep::Done;
        }
        let mut output = Vec::with_capacity(CHUNK_BYTES);
        for _ in 0..SCAN_STEPS {
            let Some(message) = self.record.messages.get(self.message) else {
                return if output.is_empty() {
                    HistoryViewStep::Done
                } else {
                    HistoryViewStep::Chunk(output)
                };
            };
            if message.role == Role::System {
                self.next_message();
                continue;
            }
            let piece = if self.header {
                Some(Piece::lines(match message.role {
                    Role::User => "\n[user]\n",
                    Role::Assistant => "\n[assistant]\n",
                    Role::Tool => "\n[recorded tool evidence]\n",
                    _ => "\n[unrecognized history role]\n",
                }))
            } else if let Some(block) = message.content.get(self.block) {
                block_piece(block, message.role, self.part)
            } else {
                self.next_message();
                continue;
            };
            let Some(piece) = piece else {
                self.block += 1;
                self.part = 0;
                self.offset = 0;
                continue;
            };
            let consumed = append_piece(piece, self.offset, &mut output);
            self.offset += consumed;
            if self.offset != piece.text.len() {
                break;
            }
            self.offset = 0;
            if self.header {
                self.header = false;
            } else {
                self.part += 1;
            }
            if output.len() == CHUNK_BYTES {
                break;
            }
        }
        if output.is_empty() {
            HistoryViewStep::Progress
        } else {
            HistoryViewStep::Chunk(output)
        }
    }

    fn next_message(&mut self) {
        self.message += 1;
        self.block = 0;
        self.part = 0;
        self.offset = 0;
        self.header = true;
    }
}

#[derive(Clone, Copy)]
struct Piece<'a> {
    text: &'a str,
    multiline: bool,
}

impl<'a> Piece<'a> {
    fn lines(text: &'a str) -> Self {
        Self {
            text,
            multiline: true,
        }
    }
    fn identity(text: &'a str) -> Self {
        Self {
            text,
            multiline: false,
        }
    }
}

fn block_piece(block: &ContentBlock, role: Role, part: u8) -> Option<Piece<'_>> {
    match block {
        ContentBlock::Text { text } if matches!(role, Role::User | Role::Assistant) => match part {
            0 => Some(Piece::lines(text)),
            1 => Some(Piece::lines("\n")),
            _ => None,
        },
        ContentBlock::Text { .. } => {
            (part == 0).then_some(Piece::lines("[text detail not displayed]\n"))
        }
        ContentBlock::Json { .. } => {
            (part == 0).then_some(Piece::lines("[structured detail not displayed]\n"))
        }
        ContentBlock::ToolCall { call } => match part {
            0 => Some(Piece::lines("[recorded tool call: ")),
            1 => Some(Piece::identity(call.name.as_str())),
            2 => Some(Piece::lines("; id=")),
            3 => Some(Piece::identity(call.id.as_str())),
            4 => Some(Piece::lines("; completion not implied]\n")),
            _ => None,
        },
        ContentBlock::ToolResult { call_id, output } => match part {
            0 => Some(Piece::lines("[recorded tool result: id=")),
            1 => Some(Piece::identity(call_id.as_str())),
            2 => Some(Piece::lines(result_label(output))),
            _ => None,
        },
        _ => (part == 0).then_some(Piece::lines("[unrecognized content not displayed]\n")),
    }
}

fn result_label(output: &ToolOutput) -> &'static str {
    // This exact two-field marker is owned by core's unknown_tool_result(),
    // not a heuristic interpretation of arbitrary tool-specific JSON.
    let unknown = output.is_error
        && output.content.as_object().is_some_and(|object| {
            object.len() == 2
                && object.get("code").and_then(serde_json::Value::as_str)
                    == Some("tool_result_unknown")
                && object.get("message").and_then(serde_json::Value::as_str)
                    == Some("tool result status is unknown")
        });
    if unknown {
        "; status=unknown; effects not inferred]\n"
    } else if output.is_error {
        "; status=error; effects may be partial; detail not displayed]\n"
    } else {
        "; status=success; detail not displayed]\n"
    }
}

fn append_piece(piece: Piece<'_>, offset: usize, output: &mut Vec<u8>) -> usize {
    let mut consumed = 0;
    for character in piece.text[offset..].chars() {
        let formatting = matches!(character, '\u{061c}' | '\u{200e}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
        let mut encoded = [0; 6];
        let bytes =
            if (character != '\n' || !piece.multiline) && (character.is_control() || formatting) {
                escape(character, &mut encoded)
            } else {
                character.encode_utf8(&mut encoded).as_bytes()
            };
        if output.len() + bytes.len() > CHUNK_BYTES {
            break;
        }
        output.extend_from_slice(bytes);
        consumed += character.len_utf8();
    }
    consumed
}

fn escape(character: char, buffer: &mut [u8; 6]) -> &[u8] {
    let short = match character {
        '\n' => Some(b'n'),
        '\r' => Some(b'r'),
        '\t' => Some(b't'),
        '\u{8}' => Some(b'b'),
        '\u{c}' => Some(b'f'),
        _ => None,
    };
    buffer[0] = b'\\';
    if let Some(short) = short {
        buffer[1] = short;
        &buffer[..2]
    } else {
        // All controls and the explicitly matched formatting characters are
        // within the BMP. Match the existing interactive output JSON escapes.
        buffer[1] = b'u';
        let value = u32::from(character);
        for (index, byte) in buffer[2..].iter_mut().enumerate() {
            let nibble = usize::try_from((value >> ((3 - index) * 4)) & 15).expect("hex digit");
            *byte = b"0123456789abcdef"[nibble];
        }
        buffer
    }
}

#[cfg(test)]
#[path = "history_view/tests.rs"]
mod tests;
