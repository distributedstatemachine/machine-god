//! Pure native context preferences and deterministic advisory summarization.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{self, Write};

use machine_god_core::{
    ContentBlock, Message, Role, SessionContextProjection, SessionRecord, ToolCall, ToolOutput,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::session_store::{MAX_FILE_SESSION_BYTES, MAX_STORED_JSON_DEPTH, MAX_STORED_JSON_NODES};

/// Reserved, versioned context preferences; these values confer no authority.
pub const NATIVE_CONTEXT_PREFERENCES_KEY: &str = "machine_god.context_preferences";
const MAX_MESSAGES: usize = MAX_FILE_SESSION_BYTES / 16;
const MAX_SUMMARY_BYTES: usize = 1200;
const MAX_SUMMARY_LINES: usize = 24;
const TEXT_LINE_BYTES: usize = 156;
const FIELDS: [&str; 3] = [
    "schema_version",
    "first_retained_message",
    "max_history_turns",
];

/// Bounded native preferences. Zero max history disables automatic compaction.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct NativeContextPreferences {
    first_retained_message: usize,
    max_history_turns: usize,
}

impl fmt::Debug for NativeContextPreferences {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeContextPreferences")
            .finish_non_exhaustive()
    }
}

/// Fixed, redacted failures containing no history, metadata, or summary text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeContextError {
    MalformedPreferences,
    UnsupportedVersion,
    PreferenceLimit,
    InvalidCursor,
    InvalidHistory,
    RecordLimit,
}

impl fmt::Display for NativeContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MalformedPreferences => "native context preferences are malformed",
            Self::UnsupportedVersion => "native context preference version is unsupported",
            Self::PreferenceLimit => "native context preference exceeds its limit",
            Self::InvalidCursor => "native context cursor is invalid for this history",
            Self::InvalidHistory => "native context history is structurally invalid",
            Self::RecordLimit => "native context record exceeds its limits",
        })
    }
}

impl std::error::Error for NativeContextError {}

impl NativeContextPreferences {
    /// Reads only the reserved shallow scalar entry; unrelated values are not visited.
    /// # Errors
    /// Rejects unknown fields/versions, noninteger values, and out-of-bound scalars.
    pub fn from_metadata(metadata: &BTreeMap<String, Value>) -> Result<Self, NativeContextError> {
        let Some(value) = metadata.get(NATIVE_CONTEXT_PREFERENCES_KEY) else {
            return Ok(Self::default());
        };
        let object = value
            .as_object()
            .ok_or(NativeContextError::MalformedPreferences)?;
        if object.len() != FIELDS.len() || FIELDS.iter().any(|field| !object.contains_key(*field)) {
            return Err(NativeContextError::MalformedPreferences);
        }
        let version = object["schema_version"]
            .as_u64()
            .ok_or(NativeContextError::MalformedPreferences)?;
        if version != 1 {
            return Err(NativeContextError::UnsupportedVersion);
        }
        let scalar = |name| {
            let number = object[name]
                .as_u64()
                .ok_or(NativeContextError::MalformedPreferences)?;
            usize::try_from(number)
                .ok()
                .filter(|value| *value <= MAX_FILE_SESSION_BYTES)
                .ok_or(NativeContextError::PreferenceLimit)
        };
        Ok(Self {
            first_retained_message: scalar("first_retained_message")?,
            max_history_turns: scalar("max_history_turns")?,
        })
    }

    /// Encodes exactly the three schema-1 scalar fields.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({"schema_version": 1, "first_retained_message": self.first_retained_message, "max_history_turns": self.max_history_turns})
    }

    #[must_use]
    pub const fn first_retained_message(&self) -> usize {
        self.first_retained_message
    }

    #[must_use]
    pub const fn max_history_turns(&self) -> usize {
        self.max_history_turns
    }

    /// Changes only this value, never the record or store.
    /// # Errors
    /// Rejects values above the native record byte ceiling without changing the
    /// previous preference; that ceiling exceeds every possible stored group count.
    pub fn set_max_history_turns(&mut self, maximum: usize) -> Result<(), NativeContextError> {
        if maximum > MAX_FILE_SESSION_BYTES {
            return Err(NativeContextError::PreferenceLimit);
        }
        self.max_history_turns = maximum;
        Ok(())
    }

    pub(crate) fn validate_selection(
        &self,
        record: &SessionRecord,
    ) -> Result<(), NativeContextError> {
        History::new(record)?.cursor(self.first_retained_message)?;
        Ok(())
    }

    /// Retains the final existing logical user group. Zero/one group is a no-op.
    /// # Errors
    /// Rejects malformed/over-bound records and stale or system-dropping cursors.
    pub fn force_compact(&mut self, record: &SessionRecord) -> Result<bool, NativeContextError> {
        let history = History::new(record)?;
        history.cursor(self.first_retained_message)?;
        if history.starts.len() <= 1 {
            return Ok(false);
        }
        let next = history
            .starts
            .last()
            .copied()
            .ok_or(NativeContextError::InvalidHistory)?;
        history.cursor(next)?;
        let changed = self.first_retained_message != next;
        self.first_retained_message = next;
        Ok(changed)
    }

    /// Derives one turn-local projection without changing canonical history.
    /// # Errors
    /// Rejects malformed/over-bound records and stale or system-dropping cursors.
    pub fn projection(
        &self,
        record: &SessionRecord,
    ) -> Result<Option<SessionContextProjection>, NativeContextError> {
        let history = History::new(record)?;
        let start = history.cursor(self.first_retained_message)?;
        let mut first = self.first_retained_message;
        let mut summary = if start == 0 {
            None
        } else {
            Some(history.summary(None, 0, start)?)
        };
        let effective_len = history.starts.len() - start + usize::from(summary.is_some());
        if self.max_history_turns != 0 && effective_len > self.max_history_turns {
            if self.max_history_turns == 1 {
                first = *history
                    .starts
                    .last()
                    .ok_or(NativeContextError::InvalidHistory)?;
                summary = None;
            } else {
                let keep = (self.max_history_turns - 1).min(4);
                let keep_from = history.starts.len() - keep;
                if keep_from > start {
                    summary = Some(history.summary(summary.as_deref(), start, keep_from)?);
                    first = history.starts[keep_from];
                }
            }
        }
        history.cursor(first)?;
        if first == 0 && summary.is_none() {
            return Ok(None);
        }
        Ok(Some(SessionContextProjection {
            first_retained_message: first,
            prefix_summary: summary,
        }))
    }
}

struct History<'a> {
    record: &'a SessionRecord,
    starts: Vec<usize>,
    leading_systems: usize,
}

impl<'a> History<'a> {
    fn new(record: &'a SessionRecord) -> Result<Self, NativeContextError> {
        validate_record(record)?;
        let mut starts = Vec::new();
        let mut pending = BTreeSet::new();
        let leading_systems = record
            .messages
            .iter()
            .take_while(|message| message.role == Role::System)
            .count();
        for (index, message) in record.messages.iter().enumerate() {
            if (!pending.is_empty() && message.role != Role::Tool)
                || (index >= leading_systems && starts.is_empty() && message.role != Role::User)
                || (message.role == Role::Tool && message.content.is_empty())
            {
                return Err(NativeContextError::InvalidHistory);
            }
            if message.role == Role::User {
                starts.push(index);
            }
            for block in &message.content {
                match block {
                    ContentBlock::ToolCall { call }
                        if message.role == Role::Assistant && pending.insert(&call.id) => {}
                    ContentBlock::ToolResult { call_id, .. }
                        if message.role == Role::Tool && pending.remove(call_id) => {}
                    ContentBlock::Text { .. } | ContentBlock::Json { .. }
                        if message.role != Role::Tool => {}
                    _ => return Err(NativeContextError::InvalidHistory),
                }
            }
        }
        if !pending.is_empty() {
            return Err(NativeContextError::InvalidHistory);
        }
        Ok(Self {
            record,
            starts,
            leading_systems,
        })
    }

    fn cursor(&self, first: usize) -> Result<usize, NativeContextError> {
        if first == 0 {
            return Ok(0);
        }
        let group = self
            .starts
            .binary_search(&first)
            .map_err(|_| NativeContextError::InvalidCursor)?;
        if self.record.messages[self.leading_systems..first]
            .iter()
            .any(|message| message.role == Role::System)
        {
            return Err(NativeContextError::InvalidCursor);
        }
        Ok(group)
    }

    fn group(&self, index: usize) -> &'a [Message] {
        &self.record.messages[self.starts[index]
            ..self
                .starts
                .get(index + 1)
                .copied()
                .unwrap_or(self.record.messages.len())]
    }

    fn summary(
        &self,
        existing: Option<&str>,
        from: usize,
        to: usize,
    ) -> Result<String, NativeContextError> {
        let mut lines = vec![
            "Conversation summary:".to_owned(),
            format!("- Earlier turns compacted: {}", to - from),
        ];
        if let Some(existing) = existing {
            lines.push("- Previously compacted context:".to_owned());
            lines.extend(
                existing
                    .split('\n')
                    .map(trim)
                    .filter(|line| !line.is_empty())
                    .map(|line| format!("  {line}")),
            );
        }
        let users = (from..to).map(|index| compact_message(&self.group(index)[0], TEXT_LINE_BYTES));
        append_text_lines(&mut lines, users, "- Recent user requests:", 4);
        let assistants = (from..to).filter_map(|index| {
            self.group(index)
                .iter()
                .rev()
                .filter(|message| message.role == Role::Assistant)
                .map(|message| compact_message(message, TEXT_LINE_BYTES))
                .find(|text| !text.is_empty())
        });
        append_text_lines(&mut lines, assistants, "- Assistant outcomes:", 3);
        let mut evidence = 0;
        for index in from..to {
            let mut calls = BTreeMap::<_, &ToolCall>::new();
            for message in self.group(index) {
                for block in &message.content {
                    match block {
                        ContentBlock::ToolCall { call } => {
                            calls.insert(&call.id, call);
                        }
                        ContentBlock::ToolResult { call_id, output } => {
                            let call = calls
                                .remove(call_id)
                                .ok_or(NativeContextError::InvalidHistory)?;
                            if evidence == 0 {
                                lines.push("- Tool execution evidence:".to_owned());
                            }
                            lines.push(evidence_line(self.record, call, output)?);
                            evidence += 1;
                            if evidence == 4 {
                                return Ok(compress_lines(&lines));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // SessionRecord has no authoritative upstream background/interrupted or
        // FileEvidence variants. Missing text is not evidence of those facts.
        if lines.len() <= 2 {
            lines.push("- Earlier conversation context compacted.".to_owned());
        }
        Ok(compress_lines(&lines))
    }
}

fn append_text_lines(
    lines: &mut Vec<String>,
    texts: impl Iterator<Item = String>,
    header: &str,
    count: usize,
) {
    for (index, text) in texts
        .filter(|text| !text.is_empty())
        .take(count)
        .enumerate()
    {
        if index == 0 {
            lines.push(header.to_owned());
        }
        lines.push(format!("  - {text}"));
    }
}

fn trim(text: &str) -> &str {
    text.trim_matches([' ', '\t', '\r', '\n'])
}

fn compact_message(message: &Message, maximum: usize) -> String {
    compact_text(
        message.content.iter().filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        }),
        maximum,
    )
}

fn compact_text<'a>(parts: impl Iterator<Item = &'a str>, maximum: usize) -> String {
    let mut output = String::with_capacity(maximum);
    let mut space = false;
    for part in parts {
        for character in part.chars() {
            if matches!(character, ' ' | '\t' | '\r' | '\n') {
                if !output.is_empty() {
                    space = true;
                }
                continue;
            }
            if space && output.len() < maximum {
                output.push(' ');
                space = false;
            }
            if character.len_utf8() > maximum - output.len() {
                return trim(&output).to_owned();
            }
            output.push(character);
        }
        if !output.is_empty() {
            space = true;
        }
    }
    trim(&output).to_owned()
}

fn compress_lines(lines: &[String]) -> String {
    let mut output = String::with_capacity(MAX_SUMMARY_BYTES);
    let mut seen = BTreeSet::new();
    let mut line_count = 0;
    let mut omitted = 0;
    for line in lines {
        let line = trim(line);
        if line.is_empty() {
            continue;
        }
        if !seen.insert(line)
            || line_count == MAX_SUMMARY_LINES
            || output.len() + line.len() + usize::from(line_count > 0) > MAX_SUMMARY_BYTES
        {
            omitted += 1;
            continue;
        }
        if line_count > 0 {
            output.push('\n');
        }
        output.push_str(line);
        line_count += 1;
    }
    if omitted > 0 && line_count < MAX_SUMMARY_LINES {
        let notice = format!("- ... {omitted} additional line(s) omitted.");
        if output.len() + notice.len() + usize::from(line_count > 0) <= MAX_SUMMARY_BYTES {
            if line_count > 0 {
                output.push('\n');
            }
            output.push_str(&notice);
        }
    }
    output
}

fn evidence_line(
    record: &SessionRecord,
    call: &ToolCall,
    output: &ToolOutput,
) -> Result<String, NativeContextError> {
    let status = if output.is_error
        && output.content.get("code").and_then(Value::as_str) == Some("tool_result_unknown")
    {
        "unknown"
    } else if output.is_error {
        "failure"
    } else {
        "success"
    };
    if let Some((bytes, handle, preview)) = archive_evidence(record, call, &output.content) {
        let preview = compact_text(std::iter::once(preview), 96);
        return Ok(format!(
            "- {} {status} ({bytes} stored bytes, handle={handle}, preview={preview})",
            call.name
        ));
    }
    let bytes = measure(output, MAX_FILE_SESSION_BYTES)?;
    Ok(format!("- {} {status} ({bytes} stored bytes)", call.name))
}

/// Only bounded, structurally valid native receipts correlated to this session
/// and call advertise an archive. This performs no existence/authenticity check.
fn archive_evidence<'a>(
    record: &SessionRecord,
    call: &ToolCall,
    value: &'a Value,
) -> Option<(u64, &'a str, &'a str)> {
    let object = value.as_object()?;
    if object.len() != 3 || object.get("type")?.as_str()? != "tool_result_archive" {
        return None;
    }
    let preview = object.get("preview")?.as_str()?;
    if preview.len() > 1024 {
        return None;
    }
    let archive = object.get("archive")?.as_object()?;
    if archive.len() != 3 {
        return None;
    }
    let bytes = archive.get("source_total_bytes")?.as_u64()?;
    if !(65_537..=210_763_776).contains(&bytes) {
        return None;
    }
    let handle = archive.get("handle")?.as_str()?;
    let suffix = handle.strip_prefix("tool-archive-v1-")?;
    if suffix.len() != 129
        || !suffix.is_ascii()
        || suffix.as_bytes()[64] != b'-'
        || !suffix[..64]
            .bytes()
            .chain(suffix[65..].bytes())
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let context = archive.get("source_context")?.as_object()?;
    if context.len() != 4
        || context.get("session_id")?.as_str()? != record.id.as_str()
        || context.get("session_incarnation_id")?.as_str()? != record.incarnation_id.as_str()
        || context.get("call_id")?.as_str()? != call.id.as_str()
    {
        return None;
    }
    let turn = context.get("turn_id")?.as_str()?;
    let sequence = turn.strip_prefix("turn-")?.parse::<u64>().ok()?;
    if sequence == 0 || sequence >= record.next_turn_sequence || turn != format!("turn-{sequence}")
    {
        return None;
    }
    Some((bytes, handle, preview))
}

fn measure(value: &impl Serialize, maximum: usize) -> Result<usize, NativeContextError> {
    let mut counter = ByteCounter { bytes: 0, maximum };
    serde_json::to_writer(&mut counter, value).map_err(|_| NativeContextError::RecordLimit)?;
    Ok(counter.bytes)
}

struct ByteCounter {
    bytes: usize,
    maximum: usize,
}

impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|bytes| *bytes <= self.maximum)
            .ok_or_else(|| io::Error::other("context byte limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn validate_record(record: &SessionRecord) -> Result<(), NativeContextError> {
    if record.messages.len() > MAX_MESSAGES || record.metadata.len() > MAX_STORED_JSON_NODES {
        return Err(NativeContextError::RecordLimit);
    }
    if record.next_turn_sequence == 0 {
        return Err(NativeContextError::InvalidHistory);
    }
    let mut budget = JsonBudget {
        nodes: 0,
        raw_bytes: 0,
    };
    for (key, value) in &record.metadata {
        budget.bytes(key.len())?;
        budget.value(value)?;
    }
    let mut blocks = 0_usize;
    for message in &record.messages {
        blocks = blocks
            .checked_add(message.content.len())
            .ok_or(NativeContextError::RecordLimit)?;
        // Every serialized ContentBlock consumes more than 16 bytes. This cap
        // bounds the pre-serialization walk without excluding an in-bound wire.
        if blocks > MAX_FILE_SESSION_BYTES / 16 {
            return Err(NativeContextError::RecordLimit);
        }
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => budget.bytes(text.len())?,
                ContentBlock::Json { value } => budget.value(value)?,
                ContentBlock::ToolCall { call } => budget.value(&call.arguments)?,
                ContentBlock::ToolResult { output, .. } => budget.value(&output.content)?,
                _ => return Err(NativeContextError::InvalidHistory),
            }
        }
    }
    measure(record, MAX_FILE_SESSION_BYTES)?;
    Ok(())
}

struct JsonBudget {
    nodes: usize,
    raw_bytes: usize,
}

enum Children<'a> {
    Array(std::slice::Iter<'a, Value>),
    Object(serde_json::map::Iter<'a>),
}

impl JsonBudget {
    fn bytes(&mut self, count: usize) -> Result<(), NativeContextError> {
        self.raw_bytes = self
            .raw_bytes
            .checked_add(count)
            .filter(|bytes| *bytes <= MAX_FILE_SESSION_BYTES)
            .ok_or(NativeContextError::RecordLimit)?;
        Ok(())
    }

    fn value(&mut self, root: &Value) -> Result<(), NativeContextError> {
        let mut frames = Vec::<Children<'_>>::new();
        let mut current = Some(root);
        loop {
            if let Some(value) = current.take() {
                self.nodes += 1;
                if self.nodes > MAX_STORED_JSON_NODES {
                    return Err(NativeContextError::RecordLimit);
                }
                let children = match value {
                    Value::Array(values) => Some(Children::Array(values.iter())),
                    Value::Object(values) => Some(Children::Object(values.iter())),
                    Value::String(text) => {
                        self.bytes(text.len())?;
                        None
                    }
                    _ => None,
                };
                if let Some(children) = children {
                    if frames.len() == MAX_STORED_JSON_DEPTH {
                        return Err(NativeContextError::RecordLimit);
                    }
                    frames.push(children);
                }
            }
            loop {
                let Some(frame) = frames.last_mut() else {
                    return Ok(());
                };
                current = match frame {
                    Children::Array(values) => values.next(),
                    Children::Object(values) => match values.next() {
                        Some((key, value)) => {
                            self.bytes(key.len())?;
                            Some(value)
                        }
                        None => None,
                    },
                };
                if current.is_some() {
                    break;
                }
                frames.pop();
            }
        }
    }
}
