use std::error::Error;
use std::fmt;
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::cmp::Ordering;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::BinaryHeap;

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, FilesystemAccess, PreparedToolCall, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Value, json};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};

/// Maximum UTF-8 bytes accepted in the raw lexical query.
pub const MAX_SEMANTIC_SEARCH_QUERY_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 bytes accepted in the selected workspace-relative path.
pub const MAX_SEMANTIC_SEARCH_PATH_BYTES: usize = 4 * 1024;
/// Maximum UTF-8 bytes retained in one workspace-relative result path.
pub const MAX_SEMANTIC_SEARCH_RESULT_PATH_BYTES: usize = 4 * 1024;
/// Maximum recursive traversal depth below the selected root.
pub const MAX_SEMANTIC_SEARCH_DEPTH: usize = 256;
/// Maximum charged non-dot directory-entry visits; one overflow witness may be observed.
pub const MAX_SEMANTIC_SEARCH_VISITED_ENTRIES: usize = 2_000;
/// Maximum aggregate raw directory-entry name bytes observed by one search.
pub const MAX_SEMANTIC_SEARCH_TOTAL_ENTRY_NAME_BYTES: usize = 8 * 1024 * 1024;
/// Maximum eligible file size.
pub const MAX_SEMANTIC_SEARCH_FILE_BYTES: usize = 100 * 1024;
/// Maximum aggregate file bytes accepted by one search.
pub const MAX_SEMANTIC_SEARCH_TOTAL_CONTENT_BYTES: usize = 64 * 1024 * 1024;
/// Maximum file-read attempts, including successful, EOF, witness, and interrupted reads.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub const MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS: usize = 12 * 1024;
/// Maximum UTF-8 bytes displayed from the best line in one result.
pub const MAX_SEMANTIC_SEARCH_RESULT_LINE_BYTES: usize = 2_000;
/// Maximum checked matcher work steps.
pub const MAX_SEMANTIC_SEARCH_MATCH_STEPS: usize = 64 * 1024 * 1024;
/// Maximum scored results retained before final ordering.
pub const MAX_SEMANTIC_SEARCH_RETAINED_RESULTS: usize = 200;
/// Maximum ordered results displayed to the model.
pub const MAX_SEMANTIC_SEARCH_SHOWN_RESULTS: usize = 100;
/// Maximum aggregate retained result-path bytes.
pub const MAX_SEMANTIC_SEARCH_TOTAL_RESULT_PATH_BYTES: usize = 819_200;
/// Maximum aggregate retained best-line bytes.
pub const MAX_SEMANTIC_SEARCH_TOTAL_RESULT_LINE_BYTES: usize = 400_000;
/// Maximum serialized [`ToolOutput`] bytes.
pub const MAX_SEMANTIC_SEARCH_SERIALIZED_RESULT_BYTES: usize = 48 * 1024;
/// Maximum non-stopword keywords retained from the query.
pub const MAX_SEMANTIC_SEARCH_KEYWORDS: usize = 16;

#[cfg(any(target_os = "linux", target_os = "macos"))]
const CONTENT_READ_CHUNK_BYTES: usize = 8 * 1024;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const CANCELLATION_CHECK_INTERVAL: usize = 1_024;

/// Registered name of [`SemanticSearchTool`].
pub const SEMANTIC_SEARCH_TOOL_NAME: &str = "semantic_search";

const SEMANTIC_SEARCH_DESCRIPTION: &str = "Lexically search workspace text files for concept keywords when exact symbols are unknown, ranking likely files for follow-up reads";
const QUERY_DESCRIPTION: &str = "Natural-language lexical query describing the concept to find";
const PATH_DESCRIPTION: &str =
    "Workspace-relative regular file or directory search root; defaults to the workspace root";

#[cfg(any(target_os = "linux", target_os = "macos"))]
const IGNORED_DIRECTORY_NAMES: &[&str] = &[
    ".git",
    ".zig-cache",
    "zig-out",
    "node_modules",
    ".next",
    "dist",
    "build",
    "coverage",
    "target",
];

#[cfg(any(target_os = "linux", target_os = "macos"))]
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "in", "on", "at", "to", "for", "of", "and", "or",
    "not", "it", "this", "that", "with", "from", "by", "as", "do", "does", "how", "what", "where",
    "when", "why", "which",
];

/// Stable category for failure to acquire a read-only workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticSearchToolOpenErrorKind {
    /// Native lexical search is unsupported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`SemanticSearchTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SemanticSearchToolOpenError {
    kind: SemanticSearchToolOpenErrorKind,
}

impl SemanticSearchToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> SemanticSearchToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: SemanticSearchToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for SemanticSearchToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SemanticSearchToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for SemanticSearchToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            SemanticSearchToolOpenErrorKind::UnsupportedPlatform => {
                "native semantic_search is unsupported on this platform"
            }
            SemanticSearchToolOpenErrorKind::InvalidRoot => {
                "native semantic_search workspace root is invalid"
            }
            SemanticSearchToolOpenErrorKind::InvalidFileType => {
                "native semantic_search workspace root is not a directory"
            }
            SemanticSearchToolOpenErrorKind::Unavailable => {
                "native semantic_search workspace root is unavailable"
            }
        })
    }
}

impl Error for SemanticSearchToolOpenError {}

/// A bounded lexical concept search confined to one retained workspace root.
pub struct SemanticSearchTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl SemanticSearchTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) const fn from_root_descriptor(root: OwnedFd) -> Self {
        Self { root }
    }

    /// Opens and retains an absolute workspace root without following its final
    /// component on supported platforms.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted failure when the platform, root spelling, root
    /// type, or root availability is unsuitable.
    pub fn open(root: &Path) -> Result<Self, SemanticSearchToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(SemanticSearchToolOpenError::new(
                SemanticSearchToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(SemanticSearchToolOpenError::new(
                    SemanticSearchToolOpenErrorKind::InvalidRoot,
                ));
            }
            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor).map_err(|_| {
                SemanticSearchToolOpenError::new(SemanticSearchToolOpenErrorKind::Unavailable)
            })?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(SemanticSearchToolOpenError::new(
                    SemanticSearchToolOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for SemanticSearchTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SemanticSearchTool")
            .finish_non_exhaustive()
    }
}

struct RequestedArguments {
    query: String,
    path: Option<String>,
}

#[derive(Clone)]
struct ExecutionArguments {
    query: String,
    path: String,
}

impl ExecutionArguments {
    fn as_json(&self) -> Value {
        json!({
            "query": self.query,
            "path": self.path,
        })
    }
}

impl Tool for SemanticSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: semantic_search_name(),
            description: SEMANTIC_SEARCH_DESCRIPTION.to_owned(),
            input_schema: semantic_search_input_schema(),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != semantic_search_name() {
            return Err(invalid_arguments());
        }
        let requested = decode_requested_arguments(call.arguments)?;
        let arguments = ExecutionArguments {
            query: normalize_query(&requested.query)?,
            path: normalize_relative_path(requested.path.as_deref().unwrap_or("."))?,
        };
        let capability_path = arguments.path.clone();
        Ok(PreparedToolCall::new(
            Capability::Filesystem {
                access: FilesystemAccess::SearchContent,
                path: capability_path,
            },
            arguments.as_json(),
        ))
    }

    fn execute(
        &self,
        _context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            let arguments = decode_execution_arguments(arguments)?;
            validate_canonical_arguments(&arguments)?;

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = (arguments, cancellation);
                Err(unsupported_platform())
            }

            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.execute_unix(&arguments, &cancellation)
            }
        })
    }
}

fn semantic_search_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_SEMANTIC_SEARCH_QUERY_BYTES,
                "description": QUERY_DESCRIPTION
            },
            "path": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_SEMANTIC_SEARCH_PATH_BYTES,
                "description": PATH_DESCRIPTION
            }
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

fn decode_requested_arguments(arguments: Value) -> Result<RequestedArguments, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.is_empty() || object.len() > 2 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(query)) = object.remove("query") else {
        return Err(invalid_arguments());
    };
    let path = match object.remove("path") {
        Some(Value::String(path)) => Some(path),
        Some(_) => return Err(invalid_arguments()),
        None => None,
    };
    if !object.is_empty() {
        return Err(invalid_arguments());
    }
    Ok(RequestedArguments { query, path })
}

fn decode_execution_arguments(arguments: Value) -> Result<ExecutionArguments, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 2 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(query)) = object.remove("query") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(path)) = object.remove("path") else {
        return Err(invalid_arguments());
    };
    if !object.is_empty() {
        return Err(invalid_arguments());
    }
    Ok(ExecutionArguments { query, path })
}

fn validate_canonical_arguments(arguments: &ExecutionArguments) -> Result<(), ToolError> {
    if normalize_query(&arguments.query)? != arguments.query
        || normalize_relative_path(&arguments.path)? != arguments.path
    {
        return Err(invalid_arguments());
    }
    Ok(())
}

fn normalize_query(query: &str) -> Result<String, ToolError> {
    if query.len() > MAX_SEMANTIC_SEARCH_QUERY_BYTES
        || query.trim_matches([' ', '\t', '\r', '\n']).is_empty()
        || query.chars().any(is_forbidden_query_character)
    {
        return Err(invalid_query());
    }
    Ok(query.to_owned())
}

fn is_forbidden_query_character(character: char) -> bool {
    (character.is_control() && character != '\t')
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_SEMANTIC_SEARCH_PATH_BYTES
        || path.starts_with('/')
        || path.chars().any(is_forbidden_path_character)
    {
        return Err(invalid_path());
    }
    let mut normalized = String::with_capacity(path.len());
    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return Err(invalid_path());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() {
        normalized.push('.');
    }
    if normalized.len() > MAX_SEMANTIC_SEARCH_PATH_BYTES {
        return Err(invalid_path());
    }
    Ok(normalized)
}

fn is_forbidden_path_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

fn semantic_search_name() -> ToolName {
    ToolName::new(SEMANTIC_SEARCH_TOOL_NAME).expect("semantic_search is a valid tool name")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Eq, PartialEq)]
enum EntryKind {
    Directory,
    RegularFile,
    Symlink,
    Other,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum SearchRoot {
    Directory(OwnedFd),
    File(OwnedFd),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct DirectoryEntry {
    name: String,
    sort_key: Vec<u8>,
    kind: EntryKind,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct DirectoryFrame {
    directory: OwnedFd,
    relative_path: String,
    depth: usize,
    entries: std::vec::IntoIter<DirectoryEntry>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct IncompleteReasons {
    traversal_cap: bool,
    result_cap: bool,
    output_cap: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl IncompleteReasons {
    const fn any(&self) -> bool {
        self.traversal_cap || self.result_cap || self.output_cap
    }

    fn as_strings(&self) -> Vec<&'static str> {
        let mut reasons = Vec::with_capacity(3);
        if self.traversal_cap {
            reasons.push("traversal_cap");
        }
        if self.result_cap {
            reasons.push("result_cap");
        }
        if self.output_cap {
            reasons.push("output_cap");
        }
        reasons
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct ScanBudget {
    visited_entries: usize,
    total_entry_name_bytes: usize,
    candidate_files: usize,
    total_content_bytes: usize,
    content_read_attempts: usize,
    match_steps: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl ScanBudget {
    fn remaining_entries(&self) -> Result<usize, ToolError> {
        MAX_SEMANTIC_SEARCH_VISITED_ENTRIES
            .checked_sub(self.visited_entries)
            .ok_or_else(scan_limit)
    }

    fn observe_entry_name(&mut self, name_bytes: usize) -> Result<(), ToolError> {
        self.total_entry_name_bytes = self
            .total_entry_name_bytes
            .checked_add(name_bytes)
            .ok_or_else(scan_limit)?;
        if self.total_entry_name_bytes > MAX_SEMANTIC_SEARCH_TOTAL_ENTRY_NAME_BYTES {
            return Err(scan_limit());
        }
        Ok(())
    }

    fn charge_entries(&mut self, entries: usize) -> Result<(), ToolError> {
        if entries > self.remaining_entries()? {
            return Err(scan_limit());
        }
        self.visited_entries = self
            .visited_entries
            .checked_add(entries)
            .ok_or_else(scan_limit)?;
        Ok(())
    }

    fn observe_candidate(&mut self) -> Result<(), ToolError> {
        self.candidate_files = self.candidate_files.checked_add(1).ok_or_else(scan_limit)?;
        if self.candidate_files > MAX_SEMANTIC_SEARCH_VISITED_ENTRIES {
            return Err(scan_limit());
        }
        Ok(())
    }

    fn observe_content_bytes(&mut self, bytes: usize) -> Result<(), ToolError> {
        self.total_content_bytes = self
            .total_content_bytes
            .checked_add(bytes)
            .ok_or_else(scan_limit)?;
        if self.total_content_bytes > MAX_SEMANTIC_SEARCH_TOTAL_CONTENT_BYTES {
            return Err(scan_limit());
        }
        Ok(())
    }

    fn charge_content_read_attempt(&mut self) -> Result<(), ToolError> {
        if self.content_read_attempts >= MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS {
            return Err(scan_limit());
        }
        self.content_read_attempts = self
            .content_read_attempts
            .checked_add(1)
            .ok_or_else(scan_limit)?;
        Ok(())
    }

    fn observe_match_step(&mut self, cancellation: &CancellationToken) -> Result<(), ToolError> {
        if self.match_steps.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
            check_cancellation(cancellation)?;
        }
        if self.match_steps >= MAX_SEMANTIC_SEARCH_MATCH_STEPS {
            return Err(scan_limit());
        }
        self.match_steps = self.match_steps.checked_add(1).ok_or_else(scan_limit)?;
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct ScanStats {
    searched_files: usize,
    skipped_oversized_files: usize,
    skipped_non_text_files: usize,
    skipped_symlink_entries: usize,
    matching_files: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct SearchResult {
    path: String,
    score: u64,
    line_number: u64,
    line: String,
    line_truncated: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct WorstFirstResult(SearchResult);

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl PartialEq for WorstFirstResult {
    fn eq(&self, other: &Self) -> bool {
        self.0.score == other.0.score && self.0.path.as_bytes() == other.0.path.as_bytes()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Eq for WorstFirstResult {}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl PartialOrd for WorstFirstResult {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Ord for WorstFirstResult {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .0
            .score
            .cmp(&self.0.score)
            .then_with(|| self.0.path.as_bytes().cmp(other.0.path.as_bytes()))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct RetainedResults {
    records: BinaryHeap<WorstFirstResult>,
    total_path_bytes: usize,
    total_line_bytes: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl RetainedResults {
    fn replacement_totals(
        &self,
        removed: Option<&SearchResult>,
        added: &SearchResult,
    ) -> Result<(usize, usize), ToolError> {
        let removed_path_bytes = removed.map_or(0, |result| result.path.len());
        let removed_line_bytes = removed.map_or(0, |result| result.line.len());
        let next_path_bytes = self
            .total_path_bytes
            .checked_sub(removed_path_bytes)
            .and_then(|bytes| bytes.checked_add(added.path.len()))
            .ok_or_else(scan_limit)?;
        let next_line_bytes = self
            .total_line_bytes
            .checked_sub(removed_line_bytes)
            .and_then(|bytes| bytes.checked_add(added.line.len()))
            .ok_or_else(scan_limit)?;
        if next_path_bytes > MAX_SEMANTIC_SEARCH_TOTAL_RESULT_PATH_BYTES
            || next_line_bytes > MAX_SEMANTIC_SEARCH_TOTAL_RESULT_LINE_BYTES
        {
            return Err(scan_limit());
        }
        Ok((next_path_bytes, next_line_bytes))
    }

    fn retain_global_best(&mut self, result: SearchResult) -> Result<bool, ToolError> {
        if self.records.len() < MAX_SEMANTIC_SEARCH_RETAINED_RESULTS {
            let (path_bytes, line_bytes) = self.replacement_totals(None, &result)?;
            self.records.push(WorstFirstResult(result));
            self.total_path_bytes = path_bytes;
            self.total_line_bytes = line_bytes;
            return Ok(false);
        }
        let worst = self.records.peek().ok_or_else(scan_limit)?;
        if !result_is_better(&result, &worst.0) {
            return Ok(true);
        }
        let (path_bytes, line_bytes) = self.replacement_totals(Some(&worst.0), &result)?;
        self.records.pop().ok_or_else(scan_limit)?;
        self.records.push(WorstFirstResult(result));
        self.total_path_bytes = path_bytes;
        self.total_line_bytes = line_bytes;
        Ok(true)
    }

    fn into_records(self) -> Vec<SearchResult> {
        self.records
            .into_vec()
            .into_iter()
            .map(|result| result.0)
            .collect()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn result_is_better(left: &SearchResult, right: &SearchResult) -> bool {
    left.score > right.score
        || (left.score == right.score && left.path.as_bytes() < right.path.as_bytes())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct ScanOutcome {
    budget: ScanBudget,
    stats: ScanStats,
    retained: RetainedResults,
    incomplete: IncompleteReasons,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct ContentBuffer {
    storage: Vec<u8>,
    length: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl ContentBuffer {
    fn reset(&mut self) {
        self.length = 0;
    }

    fn read_window(&mut self, requested: usize) -> Result<&mut [u8], ToolError> {
        let end = self.length.checked_add(requested).ok_or_else(scan_limit)?;
        if requested == 0 || end > MAX_SEMANTIC_SEARCH_FILE_BYTES + 1 {
            return Err(scan_limit());
        }
        if self.storage.len() < end {
            let additional = end - self.storage.len();
            self.storage
                .try_reserve_exact(additional)
                .map_err(|_| read_failed())?;
            self.storage.resize(end, 0);
        }
        Ok(&mut self.storage[self.length..end])
    }

    fn commit_read(&mut self, bytes: usize) -> Result<(), ToolError> {
        self.length = self.length.checked_add(bytes).ok_or_else(scan_limit)?;
        if self.length > MAX_SEMANTIC_SEARCH_FILE_BYTES + 1 {
            return Err(scan_limit());
        }
        Ok(())
    }

    fn as_slice(&self) -> &[u8] {
        &self.storage[..self.length]
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct Keyword {
    raw: String,
    folded: Vec<u8>,
    prefix: Vec<usize>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Keyword {
    fn compile(
        raw: &str,
        budget: &mut ScanBudget,
        cancellation: &CancellationToken,
    ) -> Result<Self, ToolError> {
        let mut folded = Vec::with_capacity(raw.len());
        for (index, byte) in raw.as_bytes().iter().copied().enumerate() {
            if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                check_cancellation(cancellation)?;
            }
            folded.push(fold_ascii(byte));
        }
        check_cancellation(cancellation)?;
        let mut prefix = vec![0_usize; folded.len()];
        let mut matched = 0_usize;
        for index in 1..folded.len() {
            while matched > 0
                && !charged_byte_equality(folded[index], folded[matched], budget, cancellation)?
            {
                matched = prefix[matched - 1];
            }
            if charged_byte_equality(folded[index], folded[matched], budget, cancellation)? {
                matched = matched.checked_add(1).ok_or_else(scan_limit)?;
            }
            prefix[index] = matched;
        }
        Ok(Self {
            raw: raw.to_owned(),
            folded,
            prefix,
        })
    }

    fn is_present(
        &self,
        haystack: &[u8],
        budget: &mut ScanBudget,
        cancellation: &CancellationToken,
    ) -> Result<bool, ToolError> {
        let mut matched = 0_usize;
        for byte in haystack.iter().copied().map(fold_ascii) {
            while matched > 0
                && !charged_byte_equality(byte, self.folded[matched], budget, cancellation)?
            {
                matched = self.prefix[matched - 1];
            }
            if charged_byte_equality(byte, self.folded[matched], budget, cancellation)? {
                matched = matched.checked_add(1).ok_or_else(scan_limit)?;
                if matched == self.folded.len() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn charged_byte_equality(
    left: u8,
    right: u8,
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
) -> Result<bool, ToolError> {
    budget.observe_match_step(cancellation)?;
    Ok(left == right)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn probe_keyword_presence(
    keyword: &Keyword,
    haystack: &[u8],
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
) -> Result<bool, ToolError> {
    budget.observe_match_step(cancellation)?;
    keyword.is_present(haystack, budget, cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl SemanticSearchTool {
    fn execute_unix(
        &self,
        arguments: &ExecutionArguments,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        check_cancellation(cancellation)?;
        let raw_keywords = split_search_keywords(&arguments.query, cancellation)?;
        if raw_keywords.is_empty() {
            return render_empty_output(arguments, cancellation);
        }

        let mut outcome = ScanOutcome::default();
        let mut keywords = Vec::with_capacity(raw_keywords.len());
        for raw in raw_keywords {
            keywords.push(Keyword::compile(raw, &mut outcome.budget, cancellation)?);
        }
        let root = self.open_search_root(&arguments.path, cancellation)?;
        let mut content_buffer = ContentBuffer::default();
        scan_root(
            root,
            arguments,
            &keywords,
            &mut outcome,
            &mut content_buffer,
            cancellation,
        )?;
        render_output(arguments, &keywords, outcome, cancellation)
    }

    fn open_search_root(
        &self,
        search_path: &str,
        cancellation: &CancellationToken,
    ) -> Result<SearchRoot, ToolError> {
        let mut current = execution_filesystem_call(cancellation, || {
            rustix::fs::openat(
                self.root.as_fd(),
                ".",
                directory_open_flags(),
                Mode::empty(),
            )
        })?
        .map_err(map_retained_root_reacquisition_error)?;
        ensure_root_is_linked(current.as_fd(), cancellation)?;
        if search_path == "." {
            return Ok(SearchRoot::Directory(current));
        }

        let mut components = search_path.split('/').peekable();
        loop {
            let component = components.next().ok_or_else(invalid_arguments)?;
            if components.peek().is_some() {
                current = execution_filesystem_call(cancellation, || {
                    rustix::fs::openat(
                        current.as_fd(),
                        component,
                        directory_open_flags(),
                        Mode::empty(),
                    )
                })?
                .map_err(map_search_root_open_error)?;
                continue;
            }
            let metadata = execution_filesystem_call(cancellation, || {
                rustix::fs::statat(current.as_fd(), component, AtFlags::SYMLINK_NOFOLLOW)
            })?
            .map_err(map_search_root_open_error)?;
            let initial = classify_file_type(FileType::from_raw_mode(metadata.st_mode));
            let flags = match initial {
                EntryKind::Directory => directory_open_flags(),
                EntryKind::RegularFile => content_open_flags(),
                EntryKind::Symlink | EntryKind::Other => return Err(rejected_path()),
            };
            let selected = execution_filesystem_call(cancellation, || {
                rustix::fs::openat(current.as_fd(), component, flags, Mode::empty())
            })?
            .map_err(map_search_root_open_error)?;
            let opened = execution_filesystem_call(cancellation, || rustix::fs::fstat(&selected))?
                .map_err(|_| unavailable())?;
            let opened = classify_file_type(FileType::from_raw_mode(opened.st_mode));
            if opened != initial {
                return Err(rejected_path());
            }
            return match opened {
                EntryKind::Directory => Ok(SearchRoot::Directory(selected)),
                EntryKind::RegularFile => Ok(SearchRoot::File(selected)),
                EntryKind::Symlink | EntryKind::Other => Err(rejected_path()),
            };
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn split_search_keywords<'a>(
    query: &'a str,
    cancellation: &CancellationToken,
) -> Result<Vec<&'a str>, ToolError> {
    let mut keywords = Vec::with_capacity(MAX_SEMANTIC_SEARCH_KEYWORDS);
    let mut start = None;
    for (index, byte) in query.bytes().enumerate() {
        if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
            check_cancellation(cancellation)?;
        }
        if is_search_splitter(byte) {
            if let Some(token_start) = start.take() {
                retain_query_token(&query[token_start..index], &mut keywords);
                if keywords.len() == MAX_SEMANTIC_SEARCH_KEYWORDS {
                    return Ok(keywords);
                }
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(token_start) = start {
        retain_query_token(&query[token_start..], &mut keywords);
    }
    keywords.truncate(MAX_SEMANTIC_SEARCH_KEYWORDS);
    check_cancellation(cancellation)?;
    Ok(keywords)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn retain_query_token<'a>(token: &'a str, keywords: &mut Vec<&'a str>) {
    if token.len() >= 2
        && !STOP_WORDS
            .iter()
            .any(|stop_word| token.eq_ignore_ascii_case(stop_word))
    {
        keywords.push(token);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn is_search_splitter(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b',' | b'.' | b';' | b':' | b'?' | b'!')
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn fold_ascii(byte: u8) -> u8 {
    if byte.is_ascii_uppercase() {
        byte + (b'a' - b'A')
    } else {
        byte
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn scan_root(
    root: SearchRoot,
    arguments: &ExecutionArguments,
    keywords: &[Keyword],
    outcome: &mut ScanOutcome,
    content_buffer: &mut ContentBuffer,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    match root {
        SearchRoot::File(file) => {
            outcome.budget.observe_candidate()?;
            if let Some(result) = score_open_file(
                &file,
                &arguments.path,
                keywords,
                &mut outcome.budget,
                &mut outcome.stats,
                content_buffer,
                cancellation,
            )? {
                retain_result(result, outcome)?;
            }
        }
        SearchRoot::Directory(directory) => scan_directory_tree(
            directory,
            arguments,
            keywords,
            outcome,
            content_buffer,
            cancellation,
        )?,
    }
    check_cancellation(cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn scan_directory_tree(
    directory: OwnedFd,
    arguments: &ExecutionArguments,
    keywords: &[Keyword],
    outcome: &mut ScanOutcome,
    content_buffer: &mut ContentBuffer,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let mut stack = vec![make_directory_frame(
        directory,
        String::new(),
        0,
        outcome,
        cancellation,
    )?];
    while !stack.is_empty() {
        check_cancellation(cancellation)?;
        if outcome.incomplete.traversal_cap {
            break;
        }
        let next = stack
            .last_mut()
            .expect("nonempty semantic search traversal stack")
            .entries
            .next();
        let Some(entry) = next else {
            stack.pop();
            continue;
        };
        let frame = stack
            .last()
            .expect("nonempty semantic search traversal stack");
        if let Some(child) = process_directory_entry(
            &entry,
            frame,
            arguments,
            keywords,
            outcome,
            content_buffer,
            cancellation,
        )? {
            stack.push(child);
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn process_directory_entry(
    entry: &DirectoryEntry,
    frame: &DirectoryFrame,
    arguments: &ExecutionArguments,
    keywords: &[Keyword],
    outcome: &mut ScanOutcome,
    content_buffer: &mut ContentBuffer,
    cancellation: &CancellationToken,
) -> Result<Option<DirectoryFrame>, ToolError> {
    match entry.kind {
        EntryKind::Directory => {
            if is_ignored_directory(&entry.name) {
                return Ok(None);
            }
            let relative_length =
                checked_descendant_path_length(&arguments.path, &frame.relative_path, &entry.name)?;
            let relative_path = join_relative(&frame.relative_path, &entry.name, relative_length);
            let depth = frame.depth.checked_add(1).ok_or_else(scan_limit)?;
            if depth > MAX_SEMANTIC_SEARCH_DEPTH {
                return Err(scan_limit());
            }
            let child = classify_post_observation_result(
                execution_filesystem_call(cancellation, || {
                    rustix::fs::openat(
                        frame.directory.as_fd(),
                        entry.name.as_str(),
                        directory_open_flags(),
                        Mode::empty(),
                    )
                })?,
                map_descendant_open_error,
            )?;
            let Some(child) = child else {
                return Ok(None);
            };
            make_directory_frame(child, relative_path, depth, outcome, cancellation).map(Some)
        }
        EntryKind::RegularFile => {
            let relative_length =
                checked_descendant_path_length(&arguments.path, &frame.relative_path, &entry.name)?;
            let relative_path = join_relative(&frame.relative_path, &entry.name, relative_length);
            let workspace_path = join_workspace_path(&arguments.path, &relative_path)?;
            outcome.budget.observe_candidate()?;
            let file = classify_post_observation_result(
                execution_filesystem_call(cancellation, || {
                    rustix::fs::openat(
                        frame.directory.as_fd(),
                        entry.name.as_str(),
                        content_open_flags(),
                        Mode::empty(),
                    )
                })?,
                map_content_open_error,
            )?;
            let Some(file) = file else {
                return Ok(None);
            };
            if let Some(result) = score_open_file(
                &file,
                &workspace_path,
                keywords,
                &mut outcome.budget,
                &mut outcome.stats,
                content_buffer,
                cancellation,
            )? {
                retain_result(result, outcome)?;
            }
            Ok(None)
        }
        EntryKind::Symlink => {
            outcome.stats.skipped_symlink_entries = outcome
                .stats
                .skipped_symlink_entries
                .checked_add(1)
                .ok_or_else(scan_limit)?;
            Ok(None)
        }
        EntryKind::Other => Ok(None),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn retain_result(result: SearchResult, outcome: &mut ScanOutcome) -> Result<(), ToolError> {
    outcome.stats.matching_files = outcome
        .stats
        .matching_files
        .checked_add(1)
        .ok_or_else(scan_limit)?;
    if outcome.retained.retain_global_best(result)? {
        outcome.incomplete.result_cap = true;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn make_directory_frame(
    directory: OwnedFd,
    relative_path: String,
    depth: usize,
    outcome: &mut ScanOutcome,
    cancellation: &CancellationToken,
) -> Result<DirectoryFrame, ToolError> {
    let entries = read_directory_entries(
        directory.as_fd(),
        &mut outcome.budget,
        &mut outcome.incomplete,
        cancellation,
    )?;
    Ok(DirectoryFrame {
        directory,
        relative_path,
        depth,
        entries: entries.into_iter(),
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_directory_entries(
    directory: rustix::fd::BorrowedFd<'_>,
    budget: &mut ScanBudget,
    incomplete: &mut IncompleteReasons,
    cancellation: &CancellationToken,
) -> Result<Vec<DirectoryEntry>, ToolError> {
    let mut stream = execution_filesystem_call(cancellation, || Dir::read_from(directory))?
        .map_err(map_directory_stream_error)?;
    let remaining = budget.remaining_entries()?;
    let mut raw_entries = Vec::new();
    loop {
        let next = execution_filesystem_call(cancellation, || stream.next())?;
        let Some(entry) = next else {
            break;
        };
        let entry = entry.map_err(map_directory_stream_error)?;
        let name_bytes = entry.file_name().to_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        if name_bytes.len() > MAX_SEMANTIC_SEARCH_RESULT_PATH_BYTES {
            return Err(scan_limit());
        }
        budget.observe_entry_name(name_bytes.len())?;
        raw_entries.push(name_bytes.to_vec());
        if raw_entries.len() > remaining {
            budget.charge_entries(remaining)?;
            incomplete.traversal_cap = true;
            return Ok(Vec::new());
        }
    }
    budget.charge_entries(raw_entries.len())?;
    let mut entries = Vec::with_capacity(raw_entries.len());
    for name_bytes in raw_entries {
        let name = std::str::from_utf8(&name_bytes).map_err(|_| invalid_entry_name())?;
        if name.chars().any(is_forbidden_path_character) {
            return Err(invalid_entry_name());
        }
        let name = name.to_owned();
        let metadata = classify_post_observation_result(
            execution_filesystem_call(cancellation, || {
                rustix::fs::statat(directory, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            })?,
            map_scan_metadata_error,
        )?;
        let Some(metadata) = metadata else {
            continue;
        };
        let kind = classify_file_type(FileType::from_raw_mode(metadata.st_mode));
        let mut sort_key = name_bytes;
        if kind == EntryKind::Directory {
            sort_key.push(b'/');
        }
        entries.push(DirectoryEntry {
            name,
            sort_key,
            kind,
        });
    }
    check_cancellation(cancellation)?;
    entries.sort_unstable_by(|left, right| left.sort_key.cmp(&right.sort_key));
    check_cancellation(cancellation)?;
    Ok(entries)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn score_open_file(
    file: &OwnedFd,
    workspace_path: &str,
    keywords: &[Keyword],
    budget: &mut ScanBudget,
    stats: &mut ScanStats,
    content_buffer: &mut ContentBuffer,
    cancellation: &CancellationToken,
) -> Result<Option<SearchResult>, ToolError> {
    let metadata = execution_filesystem_call(cancellation, || rustix::fs::fstat(file))?
        .map_err(|_| read_failed())?;
    if classify_file_type(FileType::from_raw_mode(metadata.st_mode)) != EntryKind::RegularFile {
        return Err(rejected_path());
    }
    let size = u64::try_from(metadata.st_size).map_err(|_| read_failed())?;
    if size > MAX_SEMANTIC_SEARCH_FILE_BYTES as u64 {
        stats.skipped_oversized_files = stats
            .skipped_oversized_files
            .checked_add(1)
            .ok_or_else(scan_limit)?;
        return Ok(None);
    }

    let bytes = read_bounded_content(file, content_buffer, budget, cancellation)?;
    if bytes.len() > MAX_SEMANTIC_SEARCH_FILE_BYTES {
        stats.skipped_oversized_files = stats
            .skipped_oversized_files
            .checked_add(1)
            .ok_or_else(scan_limit)?;
        return Ok(None);
    }
    if bytes.contains(&0) {
        stats.skipped_non_text_files = stats
            .skipped_non_text_files
            .checked_add(1)
            .ok_or_else(scan_limit)?;
        return Ok(None);
    }
    let Ok(content) = std::str::from_utf8(bytes) else {
        stats.skipped_non_text_files = stats
            .skipped_non_text_files
            .checked_add(1)
            .ok_or_else(scan_limit)?;
        return Ok(None);
    };
    stats.searched_files = stats.searched_files.checked_add(1).ok_or_else(scan_limit)?;
    score_text_file(content, workspace_path, keywords, budget, cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_bounded_content<'a>(
    file: &OwnedFd,
    content_buffer: &'a mut ContentBuffer,
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
) -> Result<&'a [u8], ToolError> {
    read_bounded_content_with(content_buffer, budget, cancellation, |window| {
        rustix::io::read(file, window)
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_bounded_content_with<'a>(
    content_buffer: &'a mut ContentBuffer,
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
    mut read: impl FnMut(&mut [u8]) -> Result<usize, rustix::io::Errno>,
) -> Result<&'a [u8], ToolError> {
    content_buffer.reset();
    loop {
        check_cancellation(cancellation)?;
        if content_buffer.length == MAX_SEMANTIC_SEARCH_FILE_BYTES + 1 {
            break;
        }
        let aggregate_remaining = MAX_SEMANTIC_SEARCH_TOTAL_CONTENT_BYTES
            .checked_sub(budget.total_content_bytes)
            .ok_or_else(scan_limit)?;
        if aggregate_remaining == 0 {
            let mut witness = [0_u8; 1];
            budget.charge_content_read_attempt()?;
            let observed = execution_filesystem_call(cancellation, || read(&mut witness))?;
            match observed {
                Ok(0) => break,
                Ok(_) => return Err(scan_limit()),
                Err(error) if error == rustix::io::Errno::INTR => continue,
                Err(_) => return Err(read_failed()),
            }
        }
        let requested = (MAX_SEMANTIC_SEARCH_FILE_BYTES + 1 - content_buffer.length)
            .min(aggregate_remaining)
            .min(CONTENT_READ_CHUNK_BYTES);
        let window = content_buffer.read_window(requested)?;
        budget.charge_content_read_attempt()?;
        let observed = execution_filesystem_call(cancellation, || read(window))?;
        match observed {
            Ok(0) => break,
            Ok(bytes_read) => {
                if bytes_read > requested {
                    return Err(read_failed());
                }
                budget.observe_content_bytes(bytes_read)?;
                content_buffer.commit_read(bytes_read)?;
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(read_failed()),
        }
    }
    Ok(content_buffer.as_slice())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn score_text_file(
    content: &str,
    workspace_path: &str,
    keywords: &[Keyword],
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
) -> Result<Option<SearchResult>, ToolError> {
    let mut total_score = 0_u64;
    let mut best_line_score = 0_u64;
    let mut best_line_number = 0_u64;
    let mut best_line = String::new();
    let mut best_line_truncated = false;
    let mut line_start = 0_usize;
    let mut line_number = 1_u64;

    for (index, byte) in content.as_bytes().iter().copied().enumerate() {
        if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
            check_cancellation(cancellation)?;
        }
        if byte != b'\n' {
            continue;
        }
        score_line(
            &content[line_start..index],
            line_number,
            keywords,
            budget,
            cancellation,
            &mut total_score,
            &mut best_line_score,
            &mut best_line_number,
            &mut best_line,
            &mut best_line_truncated,
        )?;
        line_start = index.checked_add(1).ok_or_else(scan_limit)?;
        line_number = line_number.checked_add(1).ok_or_else(scan_limit)?;
    }
    if line_start < content.len() {
        score_line(
            &content[line_start..],
            line_number,
            keywords,
            budget,
            cancellation,
            &mut total_score,
            &mut best_line_score,
            &mut best_line_number,
            &mut best_line,
            &mut best_line_truncated,
        )?;
    }

    let basename = workspace_path.rsplit('/').next().unwrap_or(workspace_path);
    for keyword in keywords {
        if probe_keyword_presence(keyword, basename.as_bytes(), budget, cancellation)? {
            total_score = total_score.checked_add(3).ok_or_else(scan_limit)?;
        }
    }
    check_cancellation(cancellation)?;
    if total_score == 0 {
        return Ok(None);
    }
    Ok(Some(SearchResult {
        path: workspace_path.to_owned(),
        score: total_score,
        line_number: best_line_number,
        line: best_line,
        line_truncated: best_line_truncated,
    }))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn score_line(
    line: &str,
    line_number: u64,
    keywords: &[Keyword],
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
    total_score: &mut u64,
    best_line_score: &mut u64,
    best_line_number: &mut u64,
    best_line: &mut String,
    best_line_truncated: &mut bool,
) -> Result<(), ToolError> {
    let mut line_score = 0_u64;
    for keyword in keywords {
        if probe_keyword_presence(keyword, line.as_bytes(), budget, cancellation)? {
            line_score = line_score.checked_add(1).ok_or_else(scan_limit)?;
        }
    }
    if line_score == 0 {
        return Ok(());
    }
    *total_score = total_score.checked_add(line_score).ok_or_else(scan_limit)?;
    if line_score > *best_line_score {
        let clipped = clip_utf8(line, MAX_SEMANTIC_SEARCH_RESULT_LINE_BYTES);
        *best_line_score = line_score;
        *best_line_number = line_number;
        best_line.clear();
        best_line.push_str(clipped);
        *best_line_truncated = clipped.len() < line.len();
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn clip_utf8(text: &str, maximum_bytes: usize) -> &str {
    if text.len() <= maximum_bytes {
        return text;
    }
    let mut end = maximum_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn render_empty_output(
    arguments: &ExecutionArguments,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    check_cancellation(cancellation)?;
    let value = output_value(arguments, &[], &[], &ScanOutcome::default());
    if serialized_tool_output_size(&value)? > MAX_SEMANTIC_SEARCH_SERIALIZED_RESULT_BYTES {
        return Err(scan_limit());
    }
    check_cancellation(cancellation)?;
    Ok(ToolOutput::success(value))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn render_output(
    arguments: &ExecutionArguments,
    keywords: &[Keyword],
    mut outcome: ScanOutcome,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    check_cancellation(cancellation)?;
    let mut ranked = std::mem::take(&mut outcome.retained).into_records();
    ranked.sort_unstable_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.path.as_bytes().cmp(right.path.as_bytes()))
    });
    let displayed = ranked
        .into_iter()
        .take(MAX_SEMANTIC_SEARCH_SHOWN_RESULTS)
        .collect::<Vec<_>>();
    if outcome.stats.matching_files > displayed.len() {
        outcome.incomplete.output_cap = true;
    }
    let raw_keywords = keywords
        .iter()
        .map(|keyword| keyword.raw.as_str())
        .collect::<Vec<_>>();
    let full = output_value(arguments, &raw_keywords, &displayed, &outcome);
    if serialized_tool_output_size(&full)? <= MAX_SEMANTIC_SEARCH_SERIALIZED_RESULT_BYTES {
        check_cancellation(cancellation)?;
        return Ok(ToolOutput::success(full));
    }
    outcome.incomplete.output_cap = true;

    let mut fitting = 0_usize;
    let mut excluded = displayed.len();
    while fitting < excluded {
        check_cancellation(cancellation)?;
        let candidate = fitting
            .checked_add(excluded)
            .and_then(|sum| sum.checked_add(1))
            .ok_or_else(scan_limit)?
            / 2;
        let value = output_value(arguments, &raw_keywords, &displayed[..candidate], &outcome);
        if serialized_tool_output_size(&value)? <= MAX_SEMANTIC_SEARCH_SERIALIZED_RESULT_BYTES {
            fitting = candidate;
        } else {
            excluded = candidate.checked_sub(1).ok_or_else(scan_limit)?;
        }
    }
    let value = output_value(arguments, &raw_keywords, &displayed[..fitting], &outcome);
    if serialized_tool_output_size(&value)? > MAX_SEMANTIC_SEARCH_SERIALIZED_RESULT_BYTES {
        return Err(scan_limit());
    }
    check_cancellation(cancellation)?;
    Ok(ToolOutput::success(value))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn output_value(
    arguments: &ExecutionArguments,
    keywords: &[&str],
    displayed: &[SearchResult],
    outcome: &ScanOutcome,
) -> Value {
    json!({
        "query": arguments.query,
        "path": arguments.path,
        "keywords": keywords,
        "results": displayed.iter().map(result_value).collect::<Vec<_>>(),
        "visited_entries": outcome.budget.visited_entries,
        "candidate_files": outcome.budget.candidate_files,
        "searched_files": outcome.stats.searched_files,
        "skipped_oversized_files": outcome.stats.skipped_oversized_files,
        "skipped_non_text_files": outcome.stats.skipped_non_text_files,
        "skipped_symlink_entries": outcome.stats.skipped_symlink_entries,
        "matching_files": outcome.stats.matching_files,
        "incomplete": outcome.incomplete.any(),
        "incomplete_reasons": outcome.incomplete.as_strings(),
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn result_value(result: &SearchResult) -> Value {
    json!({
        "path": result.path,
        "score": result.score,
        "line_number": result.line_number,
        "line": result.line,
        "line_truncated": result.line_truncated,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn serialized_tool_output_size(content: &Value) -> Result<usize, ToolError> {
    serde_json::to_vec(&ToolOutput::success(content.clone()))
        .map(|bytes| bytes.len())
        .map_err(|_| scan_limit())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn checked_descendant_path_length(
    search_path: &str,
    relative_parent: &str,
    name: &str,
) -> Result<usize, ToolError> {
    let relative_length = if relative_parent.is_empty() {
        name.len()
    } else {
        relative_parent
            .len()
            .checked_add(1)
            .and_then(|length| length.checked_add(name.len()))
            .ok_or_else(scan_limit)?
    };
    if checked_workspace_path_length(search_path, relative_length)?
        > MAX_SEMANTIC_SEARCH_RESULT_PATH_BYTES
    {
        return Err(scan_limit());
    }
    Ok(relative_length)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn checked_workspace_path_length(
    search_path: &str,
    relative_length: usize,
) -> Result<usize, ToolError> {
    if search_path == "." {
        Ok(relative_length)
    } else {
        search_path
            .len()
            .checked_add(1)
            .and_then(|length| length.checked_add(relative_length))
            .ok_or_else(scan_limit)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn join_relative(parent: &str, name: &str, capacity: usize) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        let mut path = String::with_capacity(capacity);
        path.push_str(parent);
        path.push('/');
        path.push_str(name);
        path
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn join_workspace_path(search_path: &str, relative_path: &str) -> Result<String, ToolError> {
    let capacity = checked_workspace_path_length(search_path, relative_path.len())?;
    if capacity > MAX_SEMANTIC_SEARCH_RESULT_PATH_BYTES {
        return Err(scan_limit());
    }
    if search_path == "." {
        Ok(relative_path.to_owned())
    } else {
        let mut path = String::with_capacity(capacity);
        path.push_str(search_path);
        path.push('/');
        path.push_str(relative_path);
        Ok(path)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_ignored_directory(name: &str) -> bool {
    IGNORED_DIRECTORY_NAMES.contains(&name)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn classify_file_type(file_type: FileType) -> EntryKind {
    if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::RegularFile
    } else if file_type.is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn classify_post_observation_result<T>(
    result: Result<T, rustix::io::Errno>,
    map_error: impl FnOnce(rustix::io::Errno) -> ToolError,
) -> Result<Option<T>, ToolError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(None),
        Err(error) => Err(map_error(error)),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn execution_filesystem_call<ResultValue>(
    cancellation: &CancellationToken,
    call: impl FnOnce() -> ResultValue,
) -> Result<ResultValue, ToolError> {
    check_cancellation(cancellation)?;
    let raw_result = call();
    check_cancellation(cancellation)?;
    Ok(raw_result)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn directory_open_flags() -> OFlags {
    OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NONBLOCK)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn content_open_flags() -> OFlags {
    OFlags::RDONLY
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NONBLOCK)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_root_is_linked(
    root: rustix::fd::BorrowedFd<'_>,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    #[cfg(target_os = "linux")]
    {
        let metadata = execution_filesystem_call(cancellation, || rustix::fs::fstat(root))?
            .map_err(|_| unavailable())?;
        if metadata.st_nlink == 0 {
            return Err(unavailable());
        }
    }
    #[cfg(target_os = "macos")]
    ensure_macos_root_is_linked(root, cancellation)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_macos_root_is_linked(
    root: rustix::fd::BorrowedFd<'_>,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let root_metadata = execution_filesystem_call(cancellation, || rustix::fs::fstat(root))?
        .map_err(|_| unavailable())?;
    let root_path = execution_filesystem_call(cancellation, || rustix::fs::getpath(root))?
        .map_err(|_| unavailable())?;
    let root_path = root_path.as_bytes();
    if root_path == b"/" {
        return Ok(());
    }
    let name = root_path
        .rsplit(|byte| *byte == b'/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(unavailable)?;
    let name = std::ffi::CString::new(name).map_err(|_| unavailable())?;
    let parent = execution_filesystem_call(cancellation, || {
        rustix::fs::openat(root, "..", directory_open_flags(), Mode::empty())
    })?
    .map_err(|_| unavailable())?;
    let linked = execution_filesystem_call(cancellation, || {
        rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW)
    })?
    .map_err(|_| unavailable())?;
    if linked.st_dev != root_metadata.st_dev
        || linked.st_ino != root_metadata.st_ino
        || !FileType::from_raw_mode(linked.st_mode).is_dir()
    {
        return Err(unavailable());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> SemanticSearchToolOpenError {
    let kind = if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        SemanticSearchToolOpenErrorKind::InvalidFileType
    } else {
        SemanticSearchToolOpenErrorKind::Unavailable
    };
    SemanticSearchToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_search_root_open_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::NOENT {
        not_found()
    } else if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        rejected_path()
    } else if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        unavailable()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_retained_root_reacquisition_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        unavailable()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_directory_stream_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        read_failed()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_scan_metadata_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        read_failed()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_descendant_open_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        rejected_path()
    } else if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        read_failed()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_content_open_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        rejected_path()
    } else if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        read_failed()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(ToolError::new(
            ToolErrorKind::Cancelled,
            "semantic_search_cancelled",
            "semantic_search execution was cancelled",
            false,
        ))
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "semantic_search_invalid_arguments",
        "semantic_search arguments are invalid",
        false,
    )
}

fn invalid_query() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "semantic_search_invalid_query",
        "semantic_search query is invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "semantic_search_invalid_path",
        "semantic_search path is invalid",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "semantic_search_unsupported_platform",
        "native semantic_search is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "semantic_search_not_found",
        "requested search root is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "semantic_search_permission_denied",
        "requested search root cannot be searched",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rejected_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "semantic_search_path_rejected",
        "requested path is not a confined regular file or directory",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "semantic_search_unavailable",
        "requested semantic search is unavailable",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "semantic_search_read_failed",
        "requested semantic search could not be completed",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn invalid_entry_name() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "semantic_search_invalid_entry_name",
        "requested semantic search contains an unsupported entry name",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn scan_limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "semantic_search_scan_limit",
        "requested semantic search exceeds the scan limit",
        false,
    )
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod cancellation_checkpoint_tests {
    use std::cell::Cell;

    use super::*;

    fn assert_cancelled<T>(result: Result<T, ToolError>) {
        let Err(error) = result else {
            panic!("filesystem observation bypassed post-call cancellation");
        };
        assert_eq!(error.kind, ToolErrorKind::Cancelled);
        assert_eq!(error.code, "semantic_search_cancelled");
    }

    fn assert_scan_limit<T>(result: Result<T, ToolError>) {
        let Err(error) = result else {
            panic!("semantic search work exceeded its hard budget");
        };
        assert_eq!(error.kind, ToolErrorKind::Execution);
        assert_eq!(error.code, "semantic_search_scan_limit");
        assert!(!error.retryable);
    }

    #[test]
    fn execution_filesystem_call_checks_before_invocation() {
        let cancellation = CancellationToken::new();
        assert!(cancellation.cancel());
        let called = Cell::new(false);

        assert_cancelled(execution_filesystem_call(&cancellation, || {
            called.set(true);
            Ok::<usize, rustix::io::Errno>(1)
        }));
        assert!(!called.get());
    }

    #[test]
    fn successful_raw_result_cannot_bypass_post_call_cancellation() {
        let cancellation = CancellationToken::new();

        assert_cancelled(execution_filesystem_call(&cancellation, || {
            assert!(cancellation.cancel());
            Ok::<usize, rustix::io::Errno>(8)
        }));
    }

    #[test]
    fn read_eof_cannot_bypass_post_call_cancellation() {
        let cancellation = CancellationToken::new();

        assert_cancelled(execution_filesystem_call(&cancellation, || {
            assert!(cancellation.cancel());
            Ok::<usize, rustix::io::Errno>(0)
        }));
    }

    #[test]
    fn absent_directory_entry_cannot_bypass_post_call_cancellation() {
        let cancellation = CancellationToken::new();

        assert_cancelled(execution_filesystem_call(&cancellation, || {
            assert!(cancellation.cancel());
            None::<Result<(), rustix::io::Errno>>
        }));
    }

    #[test]
    fn noent_interrupted_and_io_errors_cannot_bypass_post_call_cancellation() {
        for raw_error in [
            rustix::io::Errno::NOENT,
            rustix::io::Errno::INTR,
            rustix::io::Errno::IO,
        ] {
            let cancellation = CancellationToken::new();
            assert_cancelled(execution_filesystem_call(&cancellation, || {
                assert!(cancellation.cancel());
                Err::<(), _>(raw_error)
            }));
        }
    }

    #[test]
    fn aggregate_overflow_witness_cannot_bypass_post_call_cancellation() {
        let cancellation = CancellationToken::new();

        assert_cancelled(execution_filesystem_call(&cancellation, || {
            assert!(cancellation.cancel());
            Ok::<usize, rustix::io::Errno>(1)
        }));
    }

    #[test]
    fn repeated_interrupted_reads_stop_before_an_unmetered_attempt() {
        assert_eq!(MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS, 12_288);
        let cancellation = CancellationToken::new();
        let mut budget = ScanBudget {
            content_read_attempts: MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS - 2,
            ..ScanBudget::default()
        };
        let mut content = ContentBuffer::default();
        let calls = Cell::new(0_usize);

        assert_scan_limit(read_bounded_content_with(
            &mut content,
            &mut budget,
            &cancellation,
            |_| {
                calls.set(calls.get() + 1);
                Err(rustix::io::Errno::INTR)
            },
        ));
        assert_eq!(calls.get(), 2);
        assert_eq!(
            budget.content_read_attempts,
            MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS
        );
    }

    #[test]
    fn repeated_one_byte_reads_stop_before_an_unmetered_attempt() {
        let cancellation = CancellationToken::new();
        let mut budget = ScanBudget {
            content_read_attempts: MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS - 2,
            ..ScanBudget::default()
        };
        let mut content = ContentBuffer::default();
        let calls = Cell::new(0_usize);

        assert_scan_limit(read_bounded_content_with(
            &mut content,
            &mut budget,
            &cancellation,
            |window| {
                calls.set(calls.get() + 1);
                window[0] = b'x';
                Ok(1)
            },
        ));
        assert_eq!(calls.get(), 2);
        assert_eq!(budget.total_content_bytes, 2);
        assert_eq!(content.as_slice(), b"xx");
    }

    #[test]
    fn final_inclusive_read_and_eof_attempts_succeed_at_the_exact_boundary() {
        let cancellation = CancellationToken::new();
        let mut budget = ScanBudget {
            content_read_attempts: MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS - 2,
            ..ScanBudget::default()
        };
        let mut content = ContentBuffer::default();
        let calls = Cell::new(0_usize);

        let bytes = read_bounded_content_with(&mut content, &mut budget, &cancellation, |window| {
            let call = calls.get();
            calls.set(call + 1);
            if call == 0 {
                window[0] = b'x';
                Ok(1)
            } else {
                Ok(0)
            }
        })
        .expect("the inclusive attempt ceiling admits EOF")
        .to_vec();
        assert_eq!(bytes, b"x");
        assert_eq!(calls.get(), 2);
        assert_eq!(
            budget.content_read_attempts,
            MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS
        );
    }

    #[test]
    fn aggregate_eof_witness_is_charged_and_admitted_at_the_exact_boundary() {
        let cancellation = CancellationToken::new();
        let mut budget = ScanBudget {
            total_content_bytes: MAX_SEMANTIC_SEARCH_TOTAL_CONTENT_BYTES,
            content_read_attempts: MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS - 1,
            ..ScanBudget::default()
        };
        let mut content = ContentBuffer::default();
        let calls = Cell::new(0_usize);

        let bytes = read_bounded_content_with(&mut content, &mut budget, &cancellation, |window| {
            assert_eq!(window.len(), 1);
            calls.set(calls.get() + 1);
            Ok(0)
        })
        .expect("the inclusive attempt ceiling admits an aggregate EOF witness");
        assert!(bytes.is_empty());
        assert_eq!(calls.get(), 1);
        assert_eq!(
            budget.content_read_attempts,
            MAX_SEMANTIC_SEARCH_CONTENT_READ_ATTEMPTS
        );
    }

    #[test]
    fn empty_haystack_dispatch_and_byte_comparison_use_the_exact_match_boundary() {
        let cancellation = CancellationToken::new();
        let keyword = Keyword {
            raw: "x".to_owned(),
            folded: vec![b'x'],
            prefix: vec![0],
        };
        let mut empty_budget = ScanBudget {
            match_steps: MAX_SEMANTIC_SEARCH_MATCH_STEPS - 1,
            ..ScanBudget::default()
        };
        assert!(
            !probe_keyword_presence(&keyword, b"", &mut empty_budget, &cancellation)
                .expect("one dispatch is admitted at the inclusive boundary")
        );
        assert_eq!(empty_budget.match_steps, MAX_SEMANTIC_SEARCH_MATCH_STEPS);
        assert_scan_limit(probe_keyword_presence(
            &keyword,
            b"",
            &mut empty_budget,
            &cancellation,
        ));

        let mut comparison_budget = ScanBudget {
            match_steps: MAX_SEMANTIC_SEARCH_MATCH_STEPS - 2,
            ..ScanBudget::default()
        };
        assert!(
            probe_keyword_presence(&keyword, b"x", &mut comparison_budget, &cancellation,)
                .expect("one dispatch and one comparison reach the inclusive boundary")
        );
        assert_eq!(
            comparison_budget.match_steps,
            MAX_SEMANTIC_SEARCH_MATCH_STEPS
        );
    }

    #[test]
    fn newline_heavy_empty_lines_charge_every_keyword_dispatch() {
        let cancellation = CancellationToken::new();
        let keywords = (0..MAX_SEMANTIC_SEARCH_KEYWORDS)
            .map(|index| {
                let raw = format!("kw{index:02}");
                Keyword {
                    folded: raw.as_bytes().to_vec(),
                    prefix: vec![0; raw.len()],
                    raw,
                }
            })
            .collect::<Vec<_>>();
        let newline_count = 4_096_usize;
        let content = "\n".repeat(newline_count);
        let mut budget = ScanBudget::default();

        let result = score_text_file(&content, "", &keywords, &mut budget, &cancellation)
            .expect("newline-heavy bounded input remains searchable");
        assert!(result.is_none());
        assert_eq!(
            budget.match_steps,
            (newline_count + 1) * MAX_SEMANTIC_SEARCH_KEYWORDS
        );
    }

    #[test]
    fn retained_root_reacquisition_maps_only_access_denials_to_permission() {
        for raw_error in [rustix::io::Errno::ACCESS, rustix::io::Errno::PERM] {
            let error = map_retained_root_reacquisition_error(raw_error);
            assert_eq!(error.kind, ToolErrorKind::PermissionDenied);
            assert_eq!(error.code, "semantic_search_permission_denied");
            assert!(!error.retryable);
        }
        let error = map_retained_root_reacquisition_error(rustix::io::Errno::IO);
        assert_eq!(error.kind, ToolErrorKind::Unavailable);
        assert_eq!(error.code, "semantic_search_unavailable");
        assert!(error.retryable);
    }
}
