//! Descriptive cards from native contracts, with no execution or filesystem access.

use super::{CHUNK_BYTES, HistoryViewStep, Piece, SCAN_STEPS, append_piece};
use machine_god_core::{ContentBlock, Role, SessionRecord, TerminalActionRequest};
use machine_god_native::{
    NativeConversationHistory, NativeHistoryFileEvidence, NativeHistoryFileStatus,
    decode_terminal_action,
};

pub(super) struct CommandCard {
    command: String,
    requested_cwd: Option<String>,
}

impl CommandCard {
    pub(super) fn from_block(block: &ContentBlock, role: Role) -> Option<Self> {
        let ContentBlock::ToolCall { call } = block else {
            return None;
        };
        if role != Role::Assistant || call.name.as_str() != "terminal" {
            return None;
        }
        // Parsing needs a structurally absolute resolved directory. This inert
        // sentinel is never displayed, resolved, opened or used as authority.
        let request = decode_terminal_action(&call.arguments, "/__historical_unresolved__").ok()?;
        let command = match request {
            TerminalActionRequest::Exec { request } => request.command,
            TerminalActionRequest::Start { request } => request.command?,
            _ => return None,
        };
        Some(Self {
            command,
            requested_cwd: call
                .arguments
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        })
    }

    pub(super) fn piece(&self, part: u8) -> Option<Piece<'_>> {
        Some(match part {
            0 => Piece::lines("[recorded command request; execution not implied]\ncommand: "),
            1 => Piece::identity(&self.command),
            2 => Piece::lines("\nrequested cwd: "),
            3 => Piece::identity(
                self.requested_cwd
                    .as_deref()
                    .unwrap_or("[default; resolved cwd not recorded]"),
            ),
            4 => Piece::lines("\n"),
            _ => return None,
        })
    }
}

pub(super) struct Observations {
    history: Option<NativeConversationHistory>,
    group: usize,
    file: usize,
    part: u8,
    offset: usize,
    invalid: bool,
}

impl Observations {
    pub(super) fn new(record: &SessionRecord) -> Self {
        let history = NativeConversationHistory::from_record(record);
        Self {
            invalid: history.is_err(),
            history: history.ok(),
            group: 0,
            file: 0,
            part: 0,
            offset: 0,
        }
    }

    pub(super) fn next_chunk(&mut self, record: &SessionRecord) -> HistoryViewStep {
        if std::mem::take(&mut self.invalid) {
            return HistoryViewStep::Chunk(
                b"[historical observations invalid; details not displayed]\n".to_vec(),
            );
        }
        let mut output = Vec::with_capacity(CHUNK_BYTES);
        for _ in 0..SCAN_STEPS {
            let Some(group) = self
                .history
                .as_ref()
                .and_then(|history| history.groups().get(self.group))
            else {
                return if output.is_empty() {
                    HistoryViewStep::Done
                } else {
                    HistoryViewStep::Chunk(output)
                };
            };
            let piece = if let Some(file) = group.files().get(self.file) {
                file_piece(file, record, self.part)
            } else if self.file == group.files().len() {
                group.background().and_then(|background| {
                    Some(match self.part {
                        0 => Piece::lines(
                            "[recorded background observation; current liveness unknown]\nlog: ",
                        ),
                        1 => Piece::identity(background.log_path()),
                        2 => Piece::lines("\nurl: "),
                        3 => Piece::identity(background.url().unwrap_or("[not recorded]")),
                        4 => Piece::lines("\n"),
                        _ => return None,
                    })
                })
            } else {
                self.group += 1;
                self.file = 0;
                continue;
            };
            let Some(piece) = piece else {
                self.file += 1;
                self.part = 0;
                self.offset = 0;
                continue;
            };
            self.offset += append_piece(piece, self.offset, &mut output);
            if self.offset != piece.text.len() {
                break;
            }
            self.offset = 0;
            self.part += 1;
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
}

fn file_piece<'a>(
    file: &'a NativeHistoryFileEvidence,
    record: &'a SessionRecord,
    part: u8,
) -> Option<Piece<'a>> {
    Some(match part {
        0 => Piece::lines("[recorded file observation; current contents not checked]\naction: "),
        1 => Piece::identity(file.action().as_str()),
        2 => Piece::lines("\npath: "),
        3 => Piece::identity(file.path()),
        4 => Piece::lines("\nstatus: "),
        5 => Piece::identity(match file.status() {
            NativeHistoryFileStatus::Unknown => "unknown; effects not inferred",
            NativeHistoryFileStatus::Success => "recorded success",
            NativeHistoryFileStatus::Failure => "recorded failure; effects may be partial",
        }),
        6 => Piece::lines(if file.stale() {
            "\nstale: yes\ndestination: "
        } else {
            "\nstale: no (historical flag only)\ndestination: "
        }),
        7 => Piece::identity(file.new_path().unwrap_or("[not recorded]")),
        8 => Piece::lines("\n"),
        9..=13 => return diff_piece(file, record, part - 9),
        _ => return None,
    })
}

fn diff_piece<'a>(
    file: &NativeHistoryFileEvidence,
    record: &'a SessionRecord,
    part: u8,
) -> Option<Piece<'a>> {
    let source = file.source()?;
    // Native validation bound this source to its exact assistant block, not a
    // global call-id guess. Display requested fragments, never an invented diff
    // against current contents or proof that a mutation succeeded.
    if source.tool_name().as_str() != "edit_file" {
        return None;
    }
    let ContentBlock::ToolCall { call } = record
        .messages
        .get(source.assistant_message())?
        .content
        .get(source.content_block())?
    else {
        return None;
    };
    let arguments = call.arguments.as_object()?;
    if arguments.len() != 3 || !arguments.get("path")?.is_string() {
        return None;
    }
    let old = arguments.get("old_string")?.as_str()?;
    let new = arguments.get("new_string")?.as_str()?;
    Some(match part {
        0 => Piece::lines("[recorded requested replacement; not a full-file diff]\n- "),
        1 => Piece::identity(old),
        2 => Piece::lines("\n+ "),
        3 => Piece::identity(new),
        4 => Piece::lines("\n"),
        _ => return None,
    })
}
