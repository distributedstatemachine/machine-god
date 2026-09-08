//! Explicit native historical observations; never filesystem or execution authority.

use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, Write};

use machine_god_core::{ContentBlock, Role, SessionRecord, ToolCallId, ToolName};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::background_inspection::{MAX_BACKGROUND_PATH_BYTES, MAX_BACKGROUND_SERVER_URL_BYTES};
use crate::session_store::{MAX_FILE_SESSION_BYTES, MAX_STORED_JSON_NODES};

/// Reserved schema-1 native observations. Missing entries mean no known facts.
pub const NATIVE_CONVERSATION_HISTORY_KEY: &str = "machine_god.conversation_history";

/// Exact known lifecycle outcome, independent of optional observations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeHistoryState {
    Running,
    Completed,
    Cancelled,
    Failed,
    /// A known running checkpoint was abandoned without an exact saved outcome.
    Interrupted,
}

/// Descriptive file action, matching the pinned native evidence vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeHistoryFileAction {
    Read,
    Write,
    Edit,
    Delete,
    Rename,
    Copy,
    Search,
    List,
    Unknown,
}

impl NativeHistoryFileAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Rename => "rename",
            Self::Copy => "copy",
            Self::Search => "search",
            Self::List => "list",
            Self::Unknown => "unknown",
        }
    }
}

/// Explicit result status; missing historical success is never invented.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeHistoryFileStatus {
    Unknown,
    Success,
    Failure,
}

/// Exact canonical call location, including identity checks against that location.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NativeHistoryFileSource {
    assistant_message: usize,
    content_block: usize,
    call_id: ToolCallId,
    tool_name: ToolName,
}

impl NativeHistoryFileSource {
    /// # Errors
    /// Rejects positions exceeding the native record envelope.
    pub fn new(
        assistant_message: usize,
        content_block: usize,
        call_id: ToolCallId,
        tool_name: ToolName,
    ) -> Result<Self, Error> {
        if assistant_message > MAX_FILE_SESSION_BYTES || content_block > MAX_FILE_SESSION_BYTES {
            return Err(Error::Limit);
        }
        Ok(Self {
            assistant_message,
            content_block,
            call_id,
            tool_name,
        })
    }
    #[must_use]
    pub const fn assistant_message(&self) -> usize {
        self.assistant_message
    }
    #[must_use]
    pub const fn content_block(&self) -> usize {
        self.content_block
    }
    #[must_use]
    pub const fn call_id(&self) -> &ToolCallId {
        &self.call_id
    }
    #[must_use]
    pub const fn tool_name(&self) -> &ToolName {
        &self.tool_name
    }
}

/// A historical observation, not proof of present file contents or existence.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NativeHistoryFileEvidence {
    path: String,
    action: NativeHistoryFileAction,
    stale: bool,
    source: Option<NativeHistoryFileSource>,
    new_path: Option<String>,
    status: NativeHistoryFileStatus,
    model_view_covers_full_file: bool,
}

impl NativeHistoryFileEvidence {
    /// # Errors
    /// Rejects empty, NUL-bearing or oversized paths.
    pub fn new(
        path: &str,
        action: NativeHistoryFileAction,
        stale: bool,
    ) -> Result<Self, NativeConversationHistoryError> {
        text(path, MAX_BACKGROUND_PATH_BYTES)?;
        Ok(Self {
            path: path.to_owned(),
            action,
            stale,
            source: None,
            new_path: None,
            status: NativeHistoryFileStatus::Unknown,
            model_view_covers_full_file: false,
        })
    }
    /// Attaches explicit execution evidence, never inferred from tool text.
    /// # Errors
    /// Rejects invalid destination paths and full-file claims outside successful reads.
    pub fn with_execution(
        mut self,
        source: NativeHistoryFileSource,
        status: NativeHistoryFileStatus,
        new_path: Option<&str>,
        full_file: bool,
    ) -> Result<Self, Error> {
        if let Some(path) = new_path {
            text(path, MAX_BACKGROUND_PATH_BYTES)?;
        }
        validate_file_controls(self.action, status, true, new_path.is_some(), full_file)?;
        self.source = Some(source);
        self.new_path = new_path.map(str::to_owned);
        self.status = status;
        self.model_view_covers_full_file = full_file;
        Ok(self)
    }
    #[must_use]
    pub const fn source(&self) -> Option<&NativeHistoryFileSource> {
        self.source.as_ref()
    }
    #[must_use]
    pub fn new_path(&self) -> Option<&str> {
        self.new_path.as_deref()
    }
    #[must_use]
    pub const fn status(&self) -> NativeHistoryFileStatus {
        self.status
    }
    #[must_use]
    pub const fn model_view_covers_full_file(&self) -> bool {
        self.model_view_covers_full_file
    }
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
    #[must_use]
    pub const fn action(&self) -> NativeHistoryFileAction {
        self.action
    }
    #[must_use]
    pub const fn stale(&self) -> bool {
        self.stale
    }
}

/// Explicit background observations; these strings confer no process authority.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NativeHistoryBackground {
    log_path: String,
    url: Option<String>,
    expect_url: bool,
}

impl NativeHistoryBackground {
    /// # Errors
    /// Rejects empty, NUL-bearing or oversized strings.
    pub fn new(
        log_path: &str,
        url: Option<&str>,
        expect_url: bool,
    ) -> Result<Self, NativeConversationHistoryError> {
        text(log_path, MAX_BACKGROUND_PATH_BYTES)?;
        if let Some(url) = url {
            text(url, MAX_BACKGROUND_SERVER_URL_BYTES)?;
        }
        Ok(Self {
            log_path: log_path.to_owned(),
            url: url.map(str::to_owned),
            expect_url,
        })
    }
    #[must_use]
    pub fn log_path(&self) -> &str {
        &self.log_path
    }
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
    #[must_use]
    pub const fn expect_url(&self) -> bool {
        self.expect_url
    }
}

/// Sparse facts for one canonical user boundary, retained through continuation.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NativeHistoryGroup {
    first_user_message: usize,
    turn_sequence: u64,
    state: NativeHistoryState,
    files: Vec<NativeHistoryFileEvidence>,
    background: Option<NativeHistoryBackground>,
}

impl NativeHistoryGroup {
    #[must_use]
    pub const fn first_user_message(&self) -> usize {
        self.first_user_message
    }
    #[must_use]
    pub const fn turn_sequence(&self) -> u64 {
        self.turn_sequence
    }
    #[must_use]
    pub const fn state(&self) -> NativeHistoryState {
        self.state
    }
    #[must_use]
    pub fn files(&self) -> &[NativeHistoryFileEvidence] {
        &self.files
    }
    #[must_use]
    pub const fn background(&self) -> Option<&NativeHistoryBackground> {
        self.background.as_ref()
    }
}

/// Pure bounded ledger. Validation only reads the reserved entry and user roles.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct NativeConversationHistory {
    groups: Vec<NativeHistoryGroup>,
}

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => { $(impl fmt::Debug for $ty {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+ };
}
redacted_debug!(
    NativeConversationHistory,
    NativeHistoryGroup,
    NativeHistoryFileEvidence,
    NativeHistoryBackground
);
redacted_debug!(NativeHistoryFileSource);

/// Fixed failures never include history text, paths, or URLs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeConversationHistoryError {
    Malformed,
    UnsupportedVersion,
    Limit,
    InvalidReference,
    InvalidTransition,
}

impl fmt::Display for NativeConversationHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "native conversation history is malformed",
            Self::UnsupportedVersion => "native conversation history version is unsupported",
            Self::Limit => "native conversation history exceeds its limits",
            Self::InvalidReference => "native conversation history reference is invalid",
            Self::InvalidTransition => "native conversation history transition is invalid",
        })
    }
}
impl std::error::Error for NativeConversationHistoryError {}
type Error = NativeConversationHistoryError;

impl NativeConversationHistory {
    /// Validates shallow shapes and total bounds before cloning any supplied text.
    /// Unrelated metadata and transcript payloads are never cloned or traversed.
    /// # Errors
    /// Rejects malformed, over-limit, duplicate, unordered or dangling facts.
    pub fn from_record(record: &SessionRecord) -> Result<Self, Error> {
        let Some(value) = record.metadata.get(NATIVE_CONVERSATION_HISTORY_KEY) else {
            return Ok(Self::default());
        };
        let root = object(value, &["schema_version", "groups"])?;
        if uint(&root["schema_version"])? != 1 {
            return Err(Error::UnsupportedVersion);
        }
        let groups = root["groups"].as_array().ok_or(Error::Malformed)?;
        let mut nodes = 3usize;
        let mut previous = None;
        for value in groups {
            nodes = nodes.checked_add(6).ok_or(Error::Limit)?;
            if nodes > MAX_STORED_JSON_NODES {
                return Err(Error::Limit);
            }
            let group = group_object(value)?;
            let index = index(&group["first_user_message"])?;
            let sequence = uint(&group["turn_sequence"])?;
            let state = state(&group["state"])?;
            if sequence == 0
                || sequence >= record.next_turn_sequence
                || record
                    .messages
                    .get(index)
                    .is_none_or(|message| message.role != Role::User)
                || previous.is_some_and(|(old_index, old_sequence, old_state)| {
                    index <= old_index
                        || sequence <= old_sequence
                        || old_state == NativeHistoryState::Running
                })
            {
                return Err(Error::InvalidReference);
            }
            previous = Some((index, sequence, state));
            let group_end = record.messages[index + 1..]
                .iter()
                .position(|message| message.role == Role::User)
                .map_or(record.messages.len(), |offset| index + 1 + offset);
            let files = group["files"].as_array().ok_or(Error::Malformed)?;
            let mut seen_files = BTreeSet::new();
            let mut seen_sources = BTreeSet::new();
            let mut last_source = None;
            for value in files {
                nodes = nodes.checked_add(8).ok_or(Error::Limit)?;
                if nodes > MAX_STORED_JSON_NODES {
                    return Err(Error::Limit);
                }
                let file = file_object(value)?;
                let path = string(&file["path"], MAX_BACKGROUND_PATH_BYTES)?;
                let action = action(&file["action"])?;
                boolean(&file["stale"])?;
                let status = file_status(&file["status"])?;
                let full_file = boolean(&file["model_view_covers_full_file"])?;
                if !file["new_path"].is_null() {
                    string(&file["new_path"], MAX_BACKGROUND_PATH_BYTES)?;
                }
                let has_source = !file["source"].is_null();
                validate_file_controls(
                    action,
                    status,
                    has_source,
                    !file["new_path"].is_null(),
                    full_file,
                )?;
                if has_source {
                    nodes = nodes.checked_add(4).ok_or(Error::Limit)?;
                    if nodes > MAX_STORED_JSON_NODES {
                        return Err(Error::Limit);
                    }
                    let (message, block) =
                        validate_source(&file["source"], record, index, group_end)?;
                    if !seen_sources.insert((message, block)) {
                        return Err(Error::Malformed);
                    }
                    if last_source.is_some_and(|previous| previous >= (message, block)) {
                        return Err(Error::InvalidReference);
                    }
                    last_source = Some((message, block));
                } else if !seen_files.insert((path, action.as_str())) {
                    return Err(Error::Malformed);
                }
            }
            if !group["background"].is_null() {
                nodes = nodes.checked_add(3).ok_or(Error::Limit)?;
                if nodes > MAX_STORED_JSON_NODES {
                    return Err(Error::Limit);
                }
                let background = background_object(&group["background"])?;
                string(&background["log_path"], MAX_BACKGROUND_PATH_BYTES)?;
                if !background["url"].is_null() {
                    string(&background["url"], MAX_BACKGROUND_SERVER_URL_BYTES)?;
                }
                boolean(&background["expect_url"])?;
            }
        }
        measure(value)?;
        let groups = groups.iter().map(decode_group).collect::<Result<_, _>>()?;
        Ok(Self { groups })
    }

    /// Emits the exact schema; all owned values have a bounded, shallow shape.
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({"schema_version": 1, "groups": self.groups})
    }
    #[must_use]
    pub fn groups(&self) -> &[NativeHistoryGroup] {
        &self.groups
    }
    #[must_use]
    pub fn group(&self, first_user_message: usize) -> Option<&NativeHistoryGroup> {
        self.groups
            .binary_search_by_key(&first_user_message, |group| group.first_user_message)
            .ok()
            .map(|index| &self.groups[index])
    }

    /// Starts a new boundary or continues the last one, preserving its facts.
    /// Caller must subsequently reserve this boundary in the canonical record.
    /// # Errors
    /// Rejects non-increasing identities and bound overflow without mutation.
    pub fn begin(&mut self, first_user_message: usize, turn_sequence: u64) -> Result<(), Error> {
        if first_user_message > MAX_FILE_SESSION_BYTES
            || turn_sequence == 0
            || turn_sequence == u64::MAX
        {
            return Err(Error::InvalidReference);
        }
        let mut next = self.clone();
        if let Some(last) = next.groups.last_mut() {
            if first_user_message < last.first_user_message || turn_sequence <= last.turn_sequence {
                return Err(Error::InvalidTransition);
            }
            if first_user_message == last.first_user_message {
                last.turn_sequence = turn_sequence;
                last.state = NativeHistoryState::Running;
                return self.replace(next);
            }
            if last.state == NativeHistoryState::Running {
                last.state = NativeHistoryState::Interrupted;
            }
        }
        next.groups.push(NativeHistoryGroup {
            first_user_message,
            turn_sequence,
            state: NativeHistoryState::Running,
            files: Vec::new(),
            background: None,
        });
        self.replace(next)
    }

    /// Settles only the exact latest running admission.
    /// # Errors
    /// Rejects stale identities or a nonterminal/duplicate outcome.
    pub fn finish(
        &mut self,
        first_user_message: usize,
        turn_sequence: u64,
        state: NativeHistoryState,
    ) -> Result<(), Error> {
        let last = self.last_mut(first_user_message, turn_sequence)?;
        if last.state != NativeHistoryState::Running || state == NativeHistoryState::Running {
            return Err(Error::InvalidTransition);
        }
        // All terminal labels are no longer than "interrupted". Check encoded
        // budget on a candidate so even this small growth is failure-atomic.
        let mut next = self.clone();
        next.last_mut(first_user_message, turn_sequence)?.state = state;
        self.replace(next)
    }

    /// Replaces matching call-source evidence in place or appends an observation.
    /// Explicit source-less facts retain their path/action identity.
    /// Retains observation order for bounded historical presentation.
    /// # Errors
    /// Rejects stale identities and total bound overflow without mutation.
    pub fn upsert_file(
        &mut self,
        first_user_message: usize,
        turn_sequence: u64,
        file: NativeHistoryFileEvidence,
    ) -> Result<(), Error> {
        self.exact_mut(first_user_message, turn_sequence)?;
        let mut next = self.clone();
        let files = &mut next.exact_mut(first_user_message, turn_sequence)?.files;
        if let Some(index) = files
            .iter()
            .position(|item| match (&item.source, &file.source) {
                (Some(left), Some(right)) => {
                    left.assistant_message == right.assistant_message
                        && left.content_block == right.content_block
                }
                (None, None) => item.path == file.path && item.action == file.action,
                _ => false,
            })
        {
            if files[index].source != file.source {
                return Err(Error::InvalidReference);
            }
            files[index] = file;
        } else {
            if let Some(source) = &file.source {
                let key = (source.assistant_message, source.content_block);
                if files
                    .iter()
                    .rev()
                    .find_map(|file| file.source.as_ref())
                    .is_some_and(|previous| {
                        (previous.assistant_message, previous.content_block) >= key
                    })
                {
                    return Err(Error::InvalidReference);
                }
            }
            files.push(file);
        }
        mark_stale(files);
        self.replace(next)
    }

    /// Updates explicit background observations independently of the outcome.
    /// # Errors
    /// Rejects stale identities and total bound overflow without mutation.
    pub fn set_background(
        &mut self,
        first_user_message: usize,
        turn_sequence: u64,
        background: Option<NativeHistoryBackground>,
    ) -> Result<(), Error> {
        self.exact_mut(first_user_message, turn_sequence)?;
        let mut next = self.clone();
        next.exact_mut(first_user_message, turn_sequence)?
            .background = background;
        self.replace(next)
    }

    fn last_mut(&mut self, index: usize, sequence: u64) -> Result<&mut NativeHistoryGroup, Error> {
        self.groups
            .last_mut()
            .filter(|group| group.first_user_message == index && group.turn_sequence == sequence)
            .ok_or(Error::InvalidTransition)
    }
    fn exact_mut(&mut self, index: usize, sequence: u64) -> Result<&mut NativeHistoryGroup, Error> {
        let position = self
            .groups
            .binary_search_by_key(&index, |group| group.first_user_message)
            .map_err(|_| Error::InvalidTransition)?;
        self.groups
            .get_mut(position)
            .filter(|group| group.turn_sequence == sequence)
            .ok_or(Error::InvalidTransition)
    }
    fn replace(&mut self, next: Self) -> Result<(), Error> {
        #[derive(Serialize)]
        struct Envelope<'a> {
            schema_version: u8,
            groups: &'a [NativeHistoryGroup],
        }
        let nodes = next
            .groups
            .iter()
            .try_fold(3usize, |nodes, group| {
                nodes
                    .checked_add(6)?
                    .checked_add(group.files.iter().try_fold(0usize, |sum, file| {
                        sum.checked_add(if file.source.is_some() { 12 } else { 8 })
                    })?)?
                    .checked_add(usize::from(group.background.is_some()) * 3)
            })
            .ok_or(Error::Limit)?;
        if nodes > MAX_STORED_JSON_NODES {
            return Err(Error::Limit);
        }
        measure(&Envelope {
            schema_version: 1,
            groups: &next.groups,
        })?;
        *self = next;
        Ok(())
    }
}

fn object<'a>(value: &'a Value, fields: &[&str]) -> Result<&'a Map<String, Value>, Error> {
    let object = value.as_object().ok_or(Error::Malformed)?;
    if object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field)) {
        return Err(Error::Malformed);
    }
    Ok(object)
}
fn group_object(value: &Value) -> Result<&Map<String, Value>, Error> {
    object(
        value,
        &[
            "first_user_message",
            "turn_sequence",
            "state",
            "files",
            "background",
        ],
    )
}
fn background_object(value: &Value) -> Result<&Map<String, Value>, Error> {
    object(value, &["log_path", "url", "expect_url"])
}
fn file_object(value: &Value) -> Result<&Map<String, Value>, Error> {
    object(
        value,
        &[
            "path",
            "action",
            "stale",
            "source",
            "new_path",
            "status",
            "model_view_covers_full_file",
        ],
    )
}
fn source_object(value: &Value) -> Result<&Map<String, Value>, Error> {
    object(
        value,
        &["assistant_message", "content_block", "call_id", "tool_name"],
    )
}
fn validate_source(
    value: &Value,
    record: &SessionRecord,
    first_user: usize,
    group_end: usize,
) -> Result<(usize, usize), Error> {
    let source = source_object(value)?;
    let message = index(&source["assistant_message"])?;
    let block = index(&source["content_block"])?;
    if message <= first_user || message >= group_end {
        return Err(Error::InvalidReference);
    }
    let canonical = record
        .messages
        .get(message)
        .filter(|message| message.role == Role::Assistant)
        .and_then(|message| message.content.get(block))
        .ok_or(Error::InvalidReference)?;
    let ContentBlock::ToolCall { call } = canonical else {
        return Err(Error::InvalidReference);
    };
    if source["call_id"].as_str() != Some(call.id.as_str())
        || source["tool_name"].as_str() != Some(call.name.as_str())
    {
        return Err(Error::InvalidReference);
    }
    Ok((message, block))
}
fn file_status(value: &Value) -> Result<NativeHistoryFileStatus, Error> {
    match value.as_str() {
        Some("unknown") => Ok(NativeHistoryFileStatus::Unknown),
        Some("success") => Ok(NativeHistoryFileStatus::Success),
        Some("failure") => Ok(NativeHistoryFileStatus::Failure),
        _ => Err(Error::Malformed),
    }
}
fn validate_file_controls(
    action: NativeHistoryFileAction,
    status: NativeHistoryFileStatus,
    has_source: bool,
    has_destination: bool,
    full_file: bool,
) -> Result<(), Error> {
    if (!has_source && (status != NativeHistoryFileStatus::Unknown || has_destination || full_file))
        || (has_destination
            && !matches!(
                action,
                NativeHistoryFileAction::Rename | NativeHistoryFileAction::Copy
            ))
        || (full_file
            && (status != NativeHistoryFileStatus::Success
                || action != NativeHistoryFileAction::Read))
    {
        return Err(Error::Malformed);
    }
    Ok(())
}
fn mark_stale(files: &mut [NativeHistoryFileEvidence]) {
    let mut mutated = BTreeSet::new();
    for file in files.iter_mut().rev() {
        if file.action == NativeHistoryFileAction::Read && mutated.contains(file.path.as_str()) {
            file.stale = true;
        }
        if file.status == NativeHistoryFileStatus::Success
            && matches!(
                file.action,
                NativeHistoryFileAction::Write
                    | NativeHistoryFileAction::Edit
                    | NativeHistoryFileAction::Delete
                    | NativeHistoryFileAction::Rename
                    | NativeHistoryFileAction::Copy
            )
        {
            if file.action != NativeHistoryFileAction::Copy {
                mutated.insert(file.path.as_str());
            }
            if let Some(path) = &file.new_path {
                mutated.insert(path.as_str());
            }
        }
    }
}
fn uint(value: &Value) -> Result<u64, Error> {
    value.as_u64().ok_or(Error::Malformed)
}
fn index(value: &Value) -> Result<usize, Error> {
    usize::try_from(uint(value)?)
        .ok()
        .filter(|index| *index <= MAX_FILE_SESSION_BYTES)
        .ok_or(Error::Limit)
}
fn boolean(value: &Value) -> Result<bool, Error> {
    value.as_bool().ok_or(Error::Malformed)
}
fn text(value: &str, maximum: usize) -> Result<(), Error> {
    if value.len() > maximum {
        return Err(Error::Limit);
    }
    if value.is_empty() || value.as_bytes().contains(&0) {
        return Err(Error::Malformed);
    }
    Ok(())
}
fn string(value: &Value, maximum: usize) -> Result<&str, Error> {
    let value = value.as_str().ok_or(Error::Malformed)?;
    text(value, maximum)?;
    Ok(value)
}
fn state(value: &Value) -> Result<NativeHistoryState, Error> {
    match value.as_str() {
        Some("running") => Ok(NativeHistoryState::Running),
        Some("completed") => Ok(NativeHistoryState::Completed),
        Some("cancelled") => Ok(NativeHistoryState::Cancelled),
        Some("failed") => Ok(NativeHistoryState::Failed),
        Some("interrupted") => Ok(NativeHistoryState::Interrupted),
        _ => Err(Error::Malformed),
    }
}
fn action(value: &Value) -> Result<NativeHistoryFileAction, Error> {
    match value.as_str() {
        Some("read") => Ok(NativeHistoryFileAction::Read),
        Some("write") => Ok(NativeHistoryFileAction::Write),
        Some("edit") => Ok(NativeHistoryFileAction::Edit),
        Some("delete") => Ok(NativeHistoryFileAction::Delete),
        Some("rename") => Ok(NativeHistoryFileAction::Rename),
        Some("copy") => Ok(NativeHistoryFileAction::Copy),
        Some("search") => Ok(NativeHistoryFileAction::Search),
        Some("list") => Ok(NativeHistoryFileAction::List),
        Some("unknown") => Ok(NativeHistoryFileAction::Unknown),
        _ => Err(Error::Malformed),
    }
}
fn decode_group(value: &Value) -> Result<NativeHistoryGroup, Error> {
    let group = group_object(value)?;
    let files = group["files"]
        .as_array()
        .ok_or(Error::Malformed)?
        .iter()
        .map(|value| {
            let file = file_object(value)?;
            let evidence = NativeHistoryFileEvidence::new(
                string(&file["path"], MAX_BACKGROUND_PATH_BYTES)?,
                action(&file["action"])?,
                boolean(&file["stale"])?,
            )?;
            if file["source"].is_null() {
                return Ok(evidence);
            }
            let source = source_object(&file["source"])?;
            evidence.with_execution(
                NativeHistoryFileSource::new(
                    index(&source["assistant_message"])?,
                    index(&source["content_block"])?,
                    ToolCallId::new(source["call_id"].as_str().ok_or(Error::Malformed)?)
                        .map_err(|_| Error::Malformed)?,
                    ToolName::new(source["tool_name"].as_str().ok_or(Error::Malformed)?)
                        .map_err(|_| Error::Malformed)?,
                )?,
                file_status(&file["status"])?,
                if file["new_path"].is_null() {
                    None
                } else {
                    Some(string(&file["new_path"], MAX_BACKGROUND_PATH_BYTES)?)
                },
                boolean(&file["model_view_covers_full_file"])?,
            )
        })
        .collect::<Result<_, _>>()?;
    let background = if group["background"].is_null() {
        None
    } else {
        let background = background_object(&group["background"])?;
        Some(NativeHistoryBackground::new(
            string(&background["log_path"], MAX_BACKGROUND_PATH_BYTES)?,
            if background["url"].is_null() {
                None
            } else {
                Some(string(&background["url"], MAX_BACKGROUND_SERVER_URL_BYTES)?)
            },
            boolean(&background["expect_url"])?,
        )?)
    };
    Ok(NativeHistoryGroup {
        first_user_message: index(&group["first_user_message"])?,
        turn_sequence: uint(&group["turn_sequence"])?,
        state: state(&group["state"])?,
        files,
        background,
    })
}
fn measure(value: &impl Serialize) -> Result<(), Error> {
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|size| *size <= MAX_FILE_SESSION_BYTES)
                .ok_or_else(|| io::Error::other("history limit"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(0), value).map_err(|_| Error::Limit)
}
