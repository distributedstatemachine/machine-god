use std::error::Error;
use std::fmt;
use std::path::Path;

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, FilesystemAccess, PreparedToolCall, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Value, json};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};

/// Maximum number of UTF-8 bytes accepted in a literal search pattern.
pub const MAX_GREP_FILES_PATTERN_BYTES: usize = 4 * 1024;
/// Maximum number of UTF-8 bytes accepted in a selected search-root path.
pub const MAX_GREP_FILES_PATH_BYTES: usize = 4 * 1024;
/// Maximum number of UTF-8 bytes accepted in an include pattern.
pub const MAX_GREP_FILES_INCLUDE_BYTES: usize = 4 * 1024;
/// Maximum number of UTF-8 bytes in one returned workspace-relative path.
pub const MAX_GREP_FILES_RESULT_PATH_BYTES: usize = 4 * 1024;
/// Maximum requested result count for paginated modes.
pub const MAX_GREP_FILES_HEAD_LIMIT: usize = 100;
/// Maximum accepted zero-based result offset.
pub const MAX_GREP_FILES_OFFSET: usize = 100_000;
/// Maximum requested context lines before and after one match.
pub const MAX_GREP_FILES_CONTEXT_LINES: usize = 5;
/// Maximum eligible file size in bytes.
pub const MAX_GREP_FILES_FILE_BYTES: usize = 200 * 1024;
/// Maximum number of source bytes in one returned line or context value.
pub const MAX_GREP_FILES_RESULT_LINE_BYTES: usize = 4 * 1024;
/// Maximum number of non-dot directory entries visited by one search.
pub const MAX_GREP_FILES_VISITED_ENTRIES: usize = 100_000;
/// Maximum aggregate number of raw directory-entry name bytes visited.
pub const MAX_GREP_FILES_TOTAL_ENTRY_NAME_BYTES: usize = 16 * 1024 * 1024;
/// Maximum number of include-selected regular files attempted.
pub const MAX_GREP_FILES_CANDIDATE_FILES: usize = 10_000;
/// Maximum aggregate file bytes actually read, including overflow witnesses.
pub const MAX_GREP_FILES_TOTAL_CONTENT_BYTES: usize = 64 * 1024 * 1024;
/// Maximum include-glob matcher work steps.
pub const MAX_GREP_FILES_INCLUDE_MATCH_STEPS: usize = 8 * 1024 * 1024;
/// Maximum literal matcher work steps.
pub const MAX_GREP_FILES_CONTENT_MATCH_STEPS: usize = 256 * 1024 * 1024;
/// Maximum recursive directory traversal depth below the selected root.
pub const MAX_GREP_FILES_DEPTH: usize = 256;
/// Maximum aggregate raw path bytes retained in a paginated result.
pub const MAX_GREP_FILES_TOTAL_RESULT_PATH_BYTES: usize = 8 * 1024;
/// Maximum aggregate raw excerpt and context bytes retained in a result.
pub const MAX_GREP_FILES_TOTAL_RESULT_TEXT_BYTES: usize = 8 * 1024;
/// Maximum serialized [`ToolOutput`] bytes produced by this tool.
pub const MAX_GREP_FILES_SERIALIZED_RESULT_BYTES: usize = 48 * 1024;

/// Registered name of [`GrepFilesTool`].
pub const GREP_FILES_TOOL_NAME: &str = "grep_files";

const GREP_FILES_DESCRIPTION: &str =
    "Search UTF-8 text files for a literal substring within the configured workspace";
const PATTERN_DESCRIPTION: &str = "Literal plain-text pattern to search for";
const PATH_DESCRIPTION: &str =
    "Workspace-relative regular file or directory search root; defaults to the workspace root";
const INCLUDE_DESCRIPTION: &str =
    "Optional glob pattern applied to candidate paths before file contents are read";
const CASE_INSENSITIVE_DESCRIPTION: &str =
    "Search case-insensitively using ASCII case folding when true";
const MODE_DESCRIPTION: &str = "Return matching lines, unique files with matches, or exact matching-line and matching-file counts";
const HEAD_LIMIT_DESCRIPTION: &str =
    "Maximum results to return for matches or files_with_matches; defaults to 100";
const OFFSET_DESCRIPTION: &str =
    "Zero-based result offset for matches or files_with_matches; defaults to 0";
const CONTEXT_LINES_DESCRIPTION: &str =
    "Lines before and after each emitted match in matches mode; defaults to 0";

/// Stable category for failure to acquire a read-only workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrepFilesToolOpenErrorKind {
    /// Native content search is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`GrepFilesTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct GrepFilesToolOpenError {
    kind: GrepFilesToolOpenErrorKind,
}

impl GrepFilesToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> GrepFilesToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: GrepFilesToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for GrepFilesToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GrepFilesToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for GrepFilesToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            GrepFilesToolOpenErrorKind::UnsupportedPlatform => {
                "native grep_files is unsupported on this platform"
            }
            GrepFilesToolOpenErrorKind::InvalidRoot => {
                "native grep_files workspace root is invalid"
            }
            GrepFilesToolOpenErrorKind::InvalidFileType => {
                "native grep_files workspace root is not a directory"
            }
            GrepFilesToolOpenErrorKind::Unavailable => {
                "native grep_files workspace root is unavailable"
            }
        })
    }
}

impl Error for GrepFilesToolOpenError {}

/// A bounded literal content search confined to one retained workspace root.
pub struct GrepFilesTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl GrepFilesTool {
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
    pub fn open(root: &Path) -> Result<Self, GrepFilesToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(GrepFilesToolOpenError::new(
                GrepFilesToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(GrepFilesToolOpenError::new(
                    GrepFilesToolOpenErrorKind::InvalidRoot,
                ));
            }
            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor).map_err(|_| {
                GrepFilesToolOpenError::new(GrepFilesToolOpenErrorKind::Unavailable)
            })?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(GrepFilesToolOpenError::new(
                    GrepFilesToolOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for GrepFilesTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GrepFilesTool")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GrepMode {
    Matches,
    FilesWithMatches,
    Count,
}

impl GrepMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Matches => "matches",
            Self::FilesWithMatches => "files_with_matches",
            Self::Count => "count",
        }
    }

    fn parse(value: &str) -> Result<Self, ToolError> {
        match value {
            "matches" => Ok(Self::Matches),
            "files_with_matches" => Ok(Self::FilesWithMatches),
            "count" => Ok(Self::Count),
            _ => Err(invalid_arguments()),
        }
    }
}

struct RequestedArguments {
    pattern: String,
    path: Option<String>,
    include: Option<String>,
    case_insensitive: Option<bool>,
    mode: Option<GrepMode>,
    head_limit: Option<usize>,
    offset: Option<usize>,
    context_lines: Option<usize>,
}

#[derive(Clone)]
struct ExecutionArguments {
    pattern: String,
    path: String,
    include: Option<String>,
    case_insensitive: bool,
    mode: GrepMode,
    head_limit: usize,
    offset: usize,
    context_lines: usize,
}

impl ExecutionArguments {
    fn as_json(&self) -> Value {
        json!({
            "path": self.path,
            "pattern": self.pattern,
            "include": self.include,
            "case_insensitive": self.case_insensitive,
            "mode": self.mode.as_str(),
            "head_limit": self.head_limit,
            "offset": self.offset,
            "context_lines": self.context_lines,
        })
    }
}

impl Tool for GrepFilesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: grep_files_name(),
            description: GREP_FILES_DESCRIPTION.to_owned(),
            input_schema: grep_files_input_schema(),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != grep_files_name() {
            return Err(invalid_arguments());
        }
        let requested = decode_requested_arguments(call.arguments)?;
        let arguments = ExecutionArguments {
            pattern: normalize_literal_pattern(&requested.pattern)?,
            path: normalize_relative_path(requested.path.as_deref().unwrap_or("."))?,
            include: requested
                .include
                .as_deref()
                .map(normalize_include_pattern)
                .transpose()?,
            case_insensitive: requested.case_insensitive.unwrap_or(false),
            mode: requested.mode.unwrap_or(GrepMode::Matches),
            head_limit: requested.head_limit.unwrap_or(MAX_GREP_FILES_HEAD_LIMIT),
            offset: requested.offset.unwrap_or(0),
            context_lines: requested.context_lines.unwrap_or(0),
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

fn grep_files_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": { "type": "string", "description": PATTERN_DESCRIPTION },
            "path": { "type": "string", "description": PATH_DESCRIPTION },
            "include": { "type": "string", "description": INCLUDE_DESCRIPTION },
            "case_insensitive": {
                "type": "boolean",
                "description": CASE_INSENSITIVE_DESCRIPTION
            },
            "mode": {
                "type": "string",
                "enum": ["matches", "files_with_matches", "count"],
                "description": MODE_DESCRIPTION
            },
            "head_limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_GREP_FILES_HEAD_LIMIT,
                "description": HEAD_LIMIT_DESCRIPTION
            },
            "offset": {
                "type": "integer",
                "minimum": 0,
                "maximum": MAX_GREP_FILES_OFFSET,
                "description": OFFSET_DESCRIPTION
            },
            "context_lines": {
                "type": "integer",
                "minimum": 0,
                "maximum": MAX_GREP_FILES_CONTEXT_LINES,
                "description": CONTEXT_LINES_DESCRIPTION
            }
        },
        "required": ["pattern"],
        "additionalProperties": false
    })
}

fn decode_requested_arguments(arguments: Value) -> Result<RequestedArguments, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.is_empty() || object.len() > 8 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(pattern)) = object.remove("pattern") else {
        return Err(invalid_arguments());
    };
    let path = take_optional_string(&mut object, "path")?;
    let include = take_optional_string(&mut object, "include")?;
    let case_insensitive = take_optional_bool(&mut object, "case_insensitive")?;
    let mode = take_optional_string(&mut object, "mode")?
        .as_deref()
        .map(GrepMode::parse)
        .transpose()?;
    let head_limit =
        take_optional_bounded_integer(&mut object, "head_limit", 1, MAX_GREP_FILES_HEAD_LIMIT)?;
    let offset = take_optional_bounded_integer(&mut object, "offset", 0, MAX_GREP_FILES_OFFSET)?;
    let context_lines = take_optional_bounded_integer(
        &mut object,
        "context_lines",
        0,
        MAX_GREP_FILES_CONTEXT_LINES,
    )?;
    if !object.is_empty() {
        return Err(invalid_arguments());
    }
    Ok(RequestedArguments {
        pattern,
        path,
        include,
        case_insensitive,
        mode,
        head_limit,
        offset,
        context_lines,
    })
}

fn decode_execution_arguments(arguments: Value) -> Result<ExecutionArguments, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 8 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(pattern)) = object.remove("pattern") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(path)) = object.remove("path") else {
        return Err(invalid_arguments());
    };
    let include = match object.remove("include") {
        Some(Value::String(include)) => Some(include),
        Some(Value::Null) => None,
        _ => return Err(invalid_arguments()),
    };
    let Some(Value::Bool(case_insensitive)) = object.remove("case_insensitive") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(mode)) = object.remove("mode") else {
        return Err(invalid_arguments());
    };
    let head_limit =
        take_required_bounded_integer(&mut object, "head_limit", 1, MAX_GREP_FILES_HEAD_LIMIT)?;
    let offset = take_required_bounded_integer(&mut object, "offset", 0, MAX_GREP_FILES_OFFSET)?;
    let context_lines = take_required_bounded_integer(
        &mut object,
        "context_lines",
        0,
        MAX_GREP_FILES_CONTEXT_LINES,
    )?;
    if !object.is_empty() {
        return Err(invalid_arguments());
    }
    Ok(ExecutionArguments {
        pattern,
        path,
        include,
        case_insensitive,
        mode: GrepMode::parse(&mode)?,
        head_limit,
        offset,
        context_lines,
    })
}

fn take_optional_string(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, ToolError> {
    match object.remove(key) {
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(invalid_arguments()),
        None => Ok(None),
    }
}

fn take_optional_bool(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<bool>, ToolError> {
    match object.remove(key) {
        Some(Value::Bool(value)) => Ok(Some(value)),
        Some(_) => Err(invalid_arguments()),
        None => Ok(None),
    }
}

fn take_optional_bounded_integer(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    minimum: usize,
    maximum: usize,
) -> Result<Option<usize>, ToolError> {
    match object.remove(key) {
        Some(Value::Number(value)) => {
            let value = value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| (*value >= minimum) && (*value <= maximum))
                .ok_or_else(invalid_arguments)?;
            Ok(Some(value))
        }
        Some(_) => Err(invalid_arguments()),
        None => Ok(None),
    }
}

fn take_required_bounded_integer(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    minimum: usize,
    maximum: usize,
) -> Result<usize, ToolError> {
    take_optional_bounded_integer(object, key, minimum, maximum)?.ok_or_else(invalid_arguments)
}

fn validate_canonical_arguments(arguments: &ExecutionArguments) -> Result<(), ToolError> {
    if normalize_literal_pattern(&arguments.pattern)? != arguments.pattern
        || normalize_relative_path(&arguments.path)? != arguments.path
        || arguments
            .include
            .as_deref()
            .map(normalize_include_pattern)
            .transpose()?
            != arguments.include
    {
        return Err(invalid_arguments());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum SearchRoot {
    Directory(OwnedFd),
    File(OwnedFd),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Eq, PartialEq)]
enum EntryKind {
    Directory,
    RegularFile,
    Other,
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

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct ScanBudget {
    visited_entries: usize,
    total_entry_name_bytes: usize,
    candidate_files: usize,
    total_content_bytes: usize,
    include_match_steps: usize,
    content_match_steps: usize,
}

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
impl ScanBudget {
    fn observe_entry(&mut self, name_bytes: usize) -> Result<(), ToolError> {
        self.visited_entries = self.visited_entries.checked_add(1).ok_or_else(scan_limit)?;
        self.total_entry_name_bytes = self
            .total_entry_name_bytes
            .checked_add(name_bytes)
            .ok_or_else(scan_limit)?;
        if self.visited_entries > MAX_GREP_FILES_VISITED_ENTRIES
            || self.total_entry_name_bytes > MAX_GREP_FILES_TOTAL_ENTRY_NAME_BYTES
        {
            return Err(scan_limit());
        }
        Ok(())
    }

    fn observe_candidate(&mut self) -> Result<(), ToolError> {
        self.candidate_files = self.candidate_files.checked_add(1).ok_or_else(scan_limit)?;
        if self.candidate_files > MAX_GREP_FILES_CANDIDATE_FILES {
            return Err(scan_limit());
        }
        Ok(())
    }

    fn observe_content_bytes(&mut self, bytes: usize) -> Result<(), ToolError> {
        self.total_content_bytes = self
            .total_content_bytes
            .checked_add(bytes)
            .ok_or_else(scan_limit)?;
        if self.total_content_bytes > MAX_GREP_FILES_TOTAL_CONTENT_BYTES {
            return Err(scan_limit());
        }
        Ok(())
    }

    fn observe_include_steps(&mut self, steps: usize) -> Result<(), ToolError> {
        self.include_match_steps = self
            .include_match_steps
            .checked_add(steps)
            .ok_or_else(scan_limit)?;
        if self.include_match_steps > MAX_GREP_FILES_INCLUDE_MATCH_STEPS {
            return Err(scan_limit());
        }
        Ok(())
    }

    fn observe_content_steps(&mut self, steps: usize) -> Result<(), ToolError> {
        self.content_match_steps = self
            .content_match_steps
            .checked_add(steps)
            .ok_or_else(scan_limit)?;
        if self.content_match_steps > MAX_GREP_FILES_CONTENT_MATCH_STEPS {
            return Err(scan_limit());
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct ScanStats {
    searched_files: usize,
    skipped_oversized_files: usize,
    skipped_non_text_files: usize,
    matching_lines: u64,
    matching_files: u64,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ContextRecord {
    line_number: u64,
    line: String,
    line_truncated: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct MatchRecord {
    path: String,
    line_number: u64,
    match_start_byte: usize,
    excerpt_start_byte: usize,
    line: String,
    line_truncated: bool,
    context_before: Vec<ContextRecord>,
    context_after: Vec<ContextRecord>,
    context_truncated: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct RetainedResults {
    matches: Vec<MatchRecord>,
    files: Vec<String>,
    total_path_bytes: usize,
    total_text_bytes: usize,
    stopped: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ScanOutcome {
    budget: ScanBudget,
    stats: ScanStats,
    retained: RetainedResults,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
struct LineRange {
    start: usize,
    end: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct LiteralMatcher {
    needle: Vec<u8>,
    prefix: Vec<usize>,
    case_insensitive: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl LiteralMatcher {
    fn compile(
        pattern: &str,
        case_insensitive: bool,
        budget: &mut ScanBudget,
    ) -> Result<Self, ToolError> {
        let needle = pattern
            .as_bytes()
            .iter()
            .map(|byte| fold_ascii(*byte, case_insensitive))
            .collect::<Vec<_>>();
        let mut prefix = vec![0_usize; needle.len()];
        let mut matched = 0_usize;
        for index in 1..needle.len() {
            budget.observe_content_steps(1)?;
            while matched > 0 && needle[index] != needle[matched] {
                budget.observe_content_steps(1)?;
                matched = prefix[matched - 1];
            }
            budget.observe_content_steps(1)?;
            if needle[index] == needle[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        Ok(Self {
            needle,
            prefix,
            case_insensitive,
        })
    }

    fn find(
        &self,
        haystack: &[u8],
        budget: &mut ScanBudget,
        cancellation: &CancellationToken,
    ) -> Result<Option<usize>, ToolError> {
        let mut matched = 0_usize;
        for (index, byte) in haystack.iter().copied().enumerate() {
            if index.is_multiple_of(1024) {
                check_cancellation(cancellation)?;
            }
            let byte = fold_ascii(byte, self.case_insensitive);
            budget.observe_content_steps(1)?;
            while matched > 0 && byte != self.needle[matched] {
                budget.observe_content_steps(1)?;
                matched = self.prefix[matched - 1];
            }
            budget.observe_content_steps(1)?;
            if byte == self.needle[matched] {
                matched += 1;
                if matched == self.needle.len() {
                    return Ok(Some(index + 1 - self.needle.len()));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn fold_ascii(byte: u8, case_insensitive: bool) -> u8 {
    if case_insensitive && byte.is_ascii_uppercase() {
        byte + (b'a' - b'A')
    } else {
        byte
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl GrepFilesTool {
    fn execute_unix(
        &self,
        arguments: &ExecutionArguments,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        check_cancellation(cancellation)?;
        let root = self.open_search_root(&arguments.path, cancellation)?;
        let outcome = scan_root(root, arguments, cancellation)?;
        render_output(arguments, outcome, cancellation)
    }

    fn open_search_root(
        &self,
        search_path: &str,
        cancellation: &CancellationToken,
    ) -> Result<SearchRoot, ToolError> {
        check_cancellation(cancellation)?;
        let mut current = rustix::fs::openat(
            self.root.as_fd(),
            ".",
            directory_open_flags(),
            Mode::empty(),
        )
        .map_err(|_| unavailable())?;
        check_cancellation(cancellation)?;
        ensure_root_is_linked(current.as_fd(), cancellation)?;
        if search_path == "." {
            return Ok(SearchRoot::Directory(current));
        }

        let mut components = search_path.split('/').peekable();
        loop {
            let component = components.next().ok_or_else(invalid_arguments)?;
            check_cancellation(cancellation)?;
            if components.peek().is_some() {
                current = rustix::fs::openat(
                    current.as_fd(),
                    component,
                    directory_open_flags(),
                    Mode::empty(),
                )
                .map_err(map_search_root_open_error)?;
                check_cancellation(cancellation)?;
                continue;
            }
            let metadata =
                rustix::fs::statat(current.as_fd(), component, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(map_search_root_open_error)?;
            check_cancellation(cancellation)?;
            let initial_type = FileType::from_raw_mode(metadata.st_mode);
            let flags = if initial_type.is_dir() {
                directory_open_flags()
            } else if initial_type.is_file() {
                content_open_flags()
            } else {
                return Err(rejected_path());
            };
            let selected = rustix::fs::openat(current.as_fd(), component, flags, Mode::empty())
                .map_err(map_search_root_open_error)?;
            check_cancellation(cancellation)?;
            let selected_metadata = rustix::fs::fstat(&selected).map_err(|_| unavailable())?;
            check_cancellation(cancellation)?;
            let file_type = FileType::from_raw_mode(selected_metadata.st_mode);
            return if file_type.is_dir() {
                Ok(SearchRoot::Directory(selected))
            } else if file_type.is_file() {
                Ok(SearchRoot::File(selected))
            } else {
                Err(rejected_path())
            };
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn scan_root(
    root: SearchRoot,
    arguments: &ExecutionArguments,
    cancellation: &CancellationToken,
) -> Result<ScanOutcome, ToolError> {
    let mut outcome = ScanOutcome {
        budget: ScanBudget::default(),
        stats: ScanStats::default(),
        retained: RetainedResults::default(),
    };
    let matcher = LiteralMatcher::compile(
        &arguments.pattern,
        arguments.case_insensitive,
        &mut outcome.budget,
    )?;
    match root {
        SearchRoot::File(file) => {
            let include_selected = match &arguments.include {
                Some(include) if include.contains('/') => false,
                Some(include) => segment_matches(
                    include.as_bytes(),
                    basename(&arguments.path).as_bytes(),
                    &mut outcome.budget,
                    cancellation,
                )?,
                None => true,
            };
            if include_selected {
                outcome.budget.observe_candidate()?;
                scan_open_file(
                    &file,
                    &arguments.path,
                    arguments,
                    &matcher,
                    &mut outcome,
                    cancellation,
                )?;
            }
        }
        SearchRoot::Directory(directory) => {
            scan_directory_tree(directory, arguments, &matcher, &mut outcome, cancellation)?;
        }
    }
    check_cancellation(cancellation)?;
    Ok(outcome)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn scan_directory_tree(
    directory: OwnedFd,
    arguments: &ExecutionArguments,
    matcher: &LiteralMatcher,
    outcome: &mut ScanOutcome,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let mut stack = vec![make_directory_frame(
        directory,
        String::new(),
        0,
        &mut outcome.budget,
        cancellation,
    )?];
    while !stack.is_empty() {
        check_cancellation(cancellation)?;
        let next = stack
            .last_mut()
            .expect("nonempty grep traversal stack")
            .entries
            .next();
        let Some(entry) = next else {
            stack.pop();
            continue;
        };
        let frame = stack.last().expect("nonempty grep traversal stack");
        let relative_path = join_relative(&frame.relative_path, &entry.name);
        match entry.kind {
            EntryKind::Directory => {
                let depth = frame.depth.checked_add(1).ok_or_else(scan_limit)?;
                if depth > MAX_GREP_FILES_DEPTH {
                    return Err(scan_limit());
                }
                check_cancellation(cancellation)?;
                let child = match rustix::fs::openat(
                    frame.directory.as_fd(),
                    entry.name.as_str(),
                    directory_open_flags(),
                    Mode::empty(),
                ) {
                    Ok(child) => child,
                    Err(error) if error == rustix::io::Errno::NOENT => continue,
                    Err(error) => return Err(map_descendant_open_error(error)),
                };
                check_cancellation(cancellation)?;
                stack.push(make_directory_frame(
                    child,
                    relative_path,
                    depth,
                    &mut outcome.budget,
                    cancellation,
                )?);
            }
            EntryKind::RegularFile => {
                if !include_matches(
                    arguments.include.as_deref(),
                    &relative_path,
                    &entry.name,
                    &mut outcome.budget,
                    cancellation,
                )? {
                    continue;
                }
                let workspace_path = join_workspace_path(&arguments.path, &relative_path)?;
                outcome.budget.observe_candidate()?;
                check_cancellation(cancellation)?;
                let file = match rustix::fs::openat(
                    frame.directory.as_fd(),
                    entry.name.as_str(),
                    content_open_flags(),
                    Mode::empty(),
                ) {
                    Ok(file) => file,
                    Err(error) if error == rustix::io::Errno::NOENT => continue,
                    Err(error) => return Err(map_content_open_error(error)),
                };
                check_cancellation(cancellation)?;
                scan_open_file(
                    &file,
                    &workspace_path,
                    arguments,
                    matcher,
                    outcome,
                    cancellation,
                )?;
            }
            EntryKind::Other => {}
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn make_directory_frame(
    directory: OwnedFd,
    relative_path: String,
    depth: usize,
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
) -> Result<DirectoryFrame, ToolError> {
    let entries = read_directory_entries(directory.as_fd(), budget, cancellation)?;
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
    cancellation: &CancellationToken,
) -> Result<Vec<DirectoryEntry>, ToolError> {
    check_cancellation(cancellation)?;
    let mut stream = Dir::read_from(directory).map_err(map_directory_stream_error)?;
    check_cancellation(cancellation)?;
    let mut entries = Vec::new();
    loop {
        check_cancellation(cancellation)?;
        let next = stream.next();
        check_cancellation(cancellation)?;
        let Some(entry) = next else {
            break;
        };
        let entry = entry.map_err(map_directory_stream_error)?;
        let name_bytes = entry.file_name().to_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        budget.observe_entry(name_bytes.len())?;
        let name = std::str::from_utf8(name_bytes).map_err(|_| invalid_entry_name())?;
        if name.chars().any(is_forbidden_character) {
            return Err(invalid_entry_name());
        }
        check_cancellation(cancellation)?;
        let metadata = match rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(metadata) => metadata,
            Err(error) if error == rustix::io::Errno::NOENT => continue,
            Err(error) => return Err(map_scan_metadata_error(error)),
        };
        check_cancellation(cancellation)?;
        let file_type = FileType::from_raw_mode(metadata.st_mode);
        let kind = if file_type.is_dir() {
            EntryKind::Directory
        } else if file_type.is_file() {
            EntryKind::RegularFile
        } else {
            EntryKind::Other
        };
        let mut sort_key = name_bytes.to_vec();
        if kind == EntryKind::Directory {
            sort_key.push(b'/');
        }
        entries.push(DirectoryEntry {
            name: name.to_owned(),
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
fn include_matches(
    include: Option<&str>,
    relative_path: &str,
    basename: &str,
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
) -> Result<bool, ToolError> {
    let Some(include) = include else {
        return Ok(true);
    };
    if include.contains('/') {
        let segments = include.split('/').collect::<Vec<_>>();
        let non_recursive = segments.iter().filter(|segment| **segment != "**").count();
        path_matches(
            &segments,
            non_recursive,
            relative_path,
            budget,
            cancellation,
        )
    } else {
        segment_matches(
            include.as_bytes(),
            basename.as_bytes(),
            budget,
            cancellation,
        )
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn scan_open_file(
    file: &OwnedFd,
    workspace_path: &str,
    arguments: &ExecutionArguments,
    matcher: &LiteralMatcher,
    outcome: &mut ScanOutcome,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    check_cancellation(cancellation)?;
    let metadata = rustix::fs::fstat(file).map_err(|_| read_failed())?;
    check_cancellation(cancellation)?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file() {
        return Err(rejected_path());
    }
    let size = u64::try_from(metadata.st_size).map_err(|_| read_failed())?;
    if size > MAX_GREP_FILES_FILE_BYTES as u64 {
        outcome.stats.skipped_oversized_files = outcome
            .stats
            .skipped_oversized_files
            .checked_add(1)
            .ok_or_else(scan_limit)?;
        return Ok(());
    }

    let bytes = read_bounded_content(file, &mut outcome.budget, cancellation)?;
    if bytes.len() > MAX_GREP_FILES_FILE_BYTES {
        outcome.stats.skipped_oversized_files = outcome
            .stats
            .skipped_oversized_files
            .checked_add(1)
            .ok_or_else(scan_limit)?;
        return Ok(());
    }
    if bytes.contains(&0) {
        outcome.stats.skipped_non_text_files = outcome
            .stats
            .skipped_non_text_files
            .checked_add(1)
            .ok_or_else(scan_limit)?;
        return Ok(());
    }
    let Ok(content) = std::str::from_utf8(&bytes) else {
        outcome.stats.skipped_non_text_files = outcome
            .stats
            .skipped_non_text_files
            .checked_add(1)
            .ok_or_else(scan_limit)?;
        return Ok(());
    };
    outcome.stats.searched_files = outcome
        .stats
        .searched_files
        .checked_add(1)
        .ok_or_else(scan_limit)?;
    search_content(
        content,
        workspace_path,
        arguments,
        matcher,
        &mut outcome.budget,
        &mut outcome.stats,
        &mut outcome.retained,
        cancellation,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_bounded_content(
    file: &OwnedFd,
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, ToolError> {
    let mut bytes = vec![0_u8; MAX_GREP_FILES_FILE_BYTES + 1];
    let mut length = 0_usize;
    loop {
        check_cancellation(cancellation)?;
        if length == bytes.len() {
            break;
        }
        let aggregate_remaining = MAX_GREP_FILES_TOTAL_CONTENT_BYTES
            .checked_sub(budget.total_content_bytes)
            .ok_or_else(scan_limit)?;
        let requested = (bytes.len() - length).min(aggregate_remaining.saturating_add(1));
        if requested == 0 {
            return Err(scan_limit());
        }
        match rustix::io::read(file, &mut bytes[length..length + requested]) {
            Ok(0) => break,
            Ok(read) => {
                budget.observe_content_bytes(read)?;
                length = length.checked_add(read).ok_or_else(scan_limit)?;
                check_cancellation(cancellation)?;
            }
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(_) => return Err(read_failed()),
        }
    }
    bytes.truncate(length);
    Ok(bytes)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn search_content(
    content: &str,
    workspace_path: &str,
    arguments: &ExecutionArguments,
    matcher: &LiteralMatcher,
    budget: &mut ScanBudget,
    stats: &mut ScanStats,
    retained: &mut RetainedResults,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let lines = line_ranges(content);
    let mut file_matched = false;
    for (line_index, range) in lines.iter().copied().enumerate() {
        check_cancellation(cancellation)?;
        let line = &content[range.start..range.end];
        let Some(match_start_byte) = matcher.find(line.as_bytes(), budget, cancellation)? else {
            continue;
        };
        stats.matching_lines = stats.matching_lines.checked_add(1).ok_or_else(scan_limit)?;
        file_matched = true;
        if arguments.mode == GrepMode::Matches {
            retain_match(
                content,
                &lines,
                line_index,
                workspace_path,
                match_start_byte,
                arguments,
                stats.matching_lines,
                retained,
            )?;
        }
    }
    if file_matched {
        stats.matching_files = stats.matching_files.checked_add(1).ok_or_else(scan_limit)?;
        if arguments.mode == GrepMode::FilesWithMatches {
            retain_file(workspace_path, arguments, stats.matching_files, retained)?;
        }
    }
    check_cancellation(cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn line_ranges(content: &str) -> Vec<LineRange> {
    let mut ranges = Vec::new();
    let mut start = 0_usize;
    while start < content.len() {
        let end = content.as_bytes()[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(content.len(), |offset| start + offset);
        ranges.push(LineRange { start, end });
        if end == content.len() {
            break;
        }
        start = end + 1;
    }
    ranges
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn retain_match(
    content: &str,
    lines: &[LineRange],
    line_index: usize,
    workspace_path: &str,
    match_start_byte: usize,
    arguments: &ExecutionArguments,
    one_based_match_ordinal: u64,
    retained: &mut RetainedResults,
) -> Result<(), ToolError> {
    let zero_based_ordinal = one_based_match_ordinal
        .checked_sub(1)
        .ok_or_else(scan_limit)?;
    if zero_based_ordinal < arguments.offset as u64
        || retained.stopped
        || retained.matches.len() >= arguments.head_limit
    {
        return Ok(());
    }
    let range = lines[line_index];
    let source_line = &content[range.start..range.end];
    let (excerpt_start_byte, excerpt) =
        excerpt_containing_match(source_line, match_start_byte, arguments.pattern.len());
    let next_path_bytes = retained
        .total_path_bytes
        .checked_add(workspace_path.len())
        .ok_or_else(scan_limit)?;
    let next_text_bytes = retained
        .total_text_bytes
        .checked_add(excerpt.len())
        .ok_or_else(scan_limit)?;
    if next_path_bytes > MAX_GREP_FILES_TOTAL_RESULT_PATH_BYTES
        || next_text_bytes > MAX_GREP_FILES_TOTAL_RESULT_TEXT_BYTES
    {
        retained.stopped = true;
        return Ok(());
    }
    retained.total_path_bytes = next_path_bytes;
    retained.total_text_bytes = next_text_bytes;
    let line_number = u64::try_from(line_index)
        .ok()
        .and_then(|line| line.checked_add(1))
        .ok_or_else(scan_limit)?;
    let mut record = MatchRecord {
        path: workspace_path.to_owned(),
        line_number,
        match_start_byte,
        excerpt_start_byte,
        line: excerpt.to_owned(),
        line_truncated: excerpt_start_byte > 0 || excerpt.len() < source_line.len(),
        context_before: Vec::new(),
        context_after: Vec::new(),
        context_truncated: false,
    };

    let before_start = line_index.saturating_sub(arguments.context_lines);
    let mut context_accepting = true;
    for (context_index, range) in lines
        .iter()
        .copied()
        .enumerate()
        .take(line_index)
        .skip(before_start)
    {
        if context_accepting
            && !retain_context_line(
                content,
                range,
                context_index,
                &mut record.context_before,
                retained,
            )?
        {
            record.context_truncated = true;
            context_accepting = false;
        }
    }
    let after_end = lines.len().min(
        line_index
            .saturating_add(arguments.context_lines)
            .saturating_add(1),
    );
    for (context_index, range) in lines
        .iter()
        .copied()
        .enumerate()
        .take(after_end)
        .skip(line_index.saturating_add(1))
    {
        if context_accepting
            && !retain_context_line(
                content,
                range,
                context_index,
                &mut record.context_after,
                retained,
            )?
        {
            record.context_truncated = true;
            context_accepting = false;
        } else if !context_accepting {
            record.context_truncated = true;
        }
    }
    retained.matches.push(record);
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn retain_context_line(
    content: &str,
    range: LineRange,
    line_index: usize,
    destination: &mut Vec<ContextRecord>,
    retained: &mut RetainedResults,
) -> Result<bool, ToolError> {
    let source_line = &content[range.start..range.end];
    let line = utf8_prefix(source_line, MAX_GREP_FILES_RESULT_LINE_BYTES);
    let next_text_bytes = retained
        .total_text_bytes
        .checked_add(line.len())
        .ok_or_else(scan_limit)?;
    if next_text_bytes > MAX_GREP_FILES_TOTAL_RESULT_TEXT_BYTES {
        return Ok(false);
    }
    retained.total_text_bytes = next_text_bytes;
    let line_number = u64::try_from(line_index)
        .ok()
        .and_then(|line| line.checked_add(1))
        .ok_or_else(scan_limit)?;
    destination.push(ContextRecord {
        line_number,
        line: line.to_owned(),
        line_truncated: line.len() < source_line.len(),
    });
    Ok(true)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn retain_file(
    workspace_path: &str,
    arguments: &ExecutionArguments,
    one_based_file_ordinal: u64,
    retained: &mut RetainedResults,
) -> Result<(), ToolError> {
    let zero_based_ordinal = one_based_file_ordinal
        .checked_sub(1)
        .ok_or_else(scan_limit)?;
    if zero_based_ordinal < arguments.offset as u64
        || retained.stopped
        || retained.files.len() >= arguments.head_limit
    {
        return Ok(());
    }
    let next_path_bytes = retained
        .total_path_bytes
        .checked_add(workspace_path.len())
        .ok_or_else(scan_limit)?;
    if next_path_bytes > MAX_GREP_FILES_TOTAL_RESULT_PATH_BYTES {
        retained.stopped = true;
        return Ok(());
    }
    retained.total_path_bytes = next_path_bytes;
    retained.files.push(workspace_path.to_owned());
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn excerpt_containing_match(line: &str, match_start: usize, pattern_bytes: usize) -> (usize, &str) {
    let match_end = match_start + pattern_bytes;
    if line.len() <= MAX_GREP_FILES_RESULT_LINE_BYTES {
        return (0, line);
    }
    let spare = MAX_GREP_FILES_RESULT_LINE_BYTES - pattern_bytes;
    let mut start = match_start.saturating_sub(spare / 2);
    while start < match_start && !line.is_char_boundary(start) {
        start += 1;
    }
    let mut end = (start + MAX_GREP_FILES_RESULT_LINE_BYTES).min(line.len());
    while end > match_end && !line.is_char_boundary(end) {
        end -= 1;
    }
    if end < match_end {
        start = match_end - MAX_GREP_FILES_RESULT_LINE_BYTES;
        while !line.is_char_boundary(start) {
            start += 1;
        }
        end = match_end;
    }
    (start, &line[start..end])
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn utf8_prefix(text: &str, maximum_bytes: usize) -> &str {
    let mut end = text.len().min(maximum_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn render_output(
    arguments: &ExecutionArguments,
    mut outcome: ScanOutcome,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    check_cancellation(cancellation)?;
    let content = match arguments.mode {
        GrepMode::Matches => loop {
            let value = matches_output_value(arguments, &outcome);
            if serialized_tool_output_size(&value)? <= MAX_GREP_FILES_SERIALIZED_RESULT_BYTES {
                break value;
            }
            if remove_last_context(&mut outcome.retained.matches) {
                continue;
            }
            if outcome.retained.matches.pop().is_none() {
                return Err(scan_limit());
            }
            outcome.retained.stopped = true;
        },
        GrepMode::FilesWithMatches => loop {
            let value = files_output_value(arguments, &outcome);
            if serialized_tool_output_size(&value)? <= MAX_GREP_FILES_SERIALIZED_RESULT_BYTES {
                break value;
            }
            if outcome.retained.files.pop().is_none() {
                return Err(scan_limit());
            }
            outcome.retained.stopped = true;
        },
        GrepMode::Count => {
            let value = count_output_value(arguments, &outcome);
            if serialized_tool_output_size(&value)? > MAX_GREP_FILES_SERIALIZED_RESULT_BYTES {
                return Err(scan_limit());
            }
            value
        }
    };
    check_cancellation(cancellation)?;
    Ok(ToolOutput::success(content))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn common_output_value(arguments: &ExecutionArguments, outcome: &ScanOutcome) -> Value {
    json!({
        "path": arguments.path,
        "pattern": arguments.pattern,
        "include": arguments.include,
        "case_insensitive": arguments.case_insensitive,
        "mode": arguments.mode.as_str(),
        "head_limit": arguments.head_limit,
        "offset": arguments.offset,
        "context_lines": arguments.context_lines,
        "candidate_files": outcome.budget.candidate_files,
        "searched_files": outcome.stats.searched_files,
        "skipped_oversized_files": outcome.stats.skipped_oversized_files,
        "skipped_non_text_files": outcome.stats.skipped_non_text_files,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn matches_output_value(arguments: &ExecutionArguments, outcome: &ScanOutcome) -> Value {
    let emitted = outcome.retained.matches.len();
    let total = outcome.stats.matching_lines;
    let (next_offset, truncated) = pagination_fields(arguments.offset, emitted, total);
    let records = outcome
        .retained
        .matches
        .iter()
        .map(match_record_value)
        .collect::<Vec<_>>();
    let mut value = common_output_value(arguments, outcome);
    let object = value
        .as_object_mut()
        .expect("common grep output is an object");
    object.insert("matches".to_owned(), Value::Array(records));
    object.insert("total_matches".to_owned(), json!(total));
    object.insert(
        "matching_files".to_owned(),
        json!(outcome.stats.matching_files),
    );
    object.insert("next_offset".to_owned(), json!(next_offset));
    object.insert("truncated".to_owned(), json!(truncated));
    value
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn files_output_value(arguments: &ExecutionArguments, outcome: &ScanOutcome) -> Value {
    let emitted = outcome.retained.files.len();
    let total = outcome.stats.matching_files;
    let (next_offset, truncated) = pagination_fields(arguments.offset, emitted, total);
    let mut value = common_output_value(arguments, outcome);
    let object = value
        .as_object_mut()
        .expect("common grep output is an object");
    object.insert("files".to_owned(), json!(outcome.retained.files));
    object.insert("total_files".to_owned(), json!(total));
    object.insert(
        "matching_lines".to_owned(),
        json!(outcome.stats.matching_lines),
    );
    object.insert("next_offset".to_owned(), json!(next_offset));
    object.insert("truncated".to_owned(), json!(truncated));
    value
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn count_output_value(arguments: &ExecutionArguments, outcome: &ScanOutcome) -> Value {
    let mut value = common_output_value(arguments, outcome);
    let object = value
        .as_object_mut()
        .expect("common grep output is an object");
    object.insert(
        "matching_lines".to_owned(),
        json!(outcome.stats.matching_lines),
    );
    object.insert(
        "matching_files".to_owned(),
        json!(outcome.stats.matching_files),
    );
    value
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn match_record_value(record: &MatchRecord) -> Value {
    json!({
        "path": record.path,
        "line_number": record.line_number,
        "match_start_byte": record.match_start_byte,
        "excerpt_start_byte": record.excerpt_start_byte,
        "line": record.line,
        "line_truncated": record.line_truncated,
        "context_before": record.context_before.iter().map(context_record_value).collect::<Vec<_>>(),
        "context_after": record.context_after.iter().map(context_record_value).collect::<Vec<_>>(),
        "context_truncated": record.context_truncated,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn context_record_value(record: &ContextRecord) -> Value {
    json!({
        "line_number": record.line_number,
        "line": record.line,
        "line_truncated": record.line_truncated,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn pagination_fields(offset: usize, emitted: usize, total: u64) -> (Option<usize>, bool) {
    let end = offset.checked_add(emitted);
    let has_later = end.is_some_and(|end| (end as u64) < total);
    let next_offset = has_later.then_some(end.expect("later results require a representable end"));
    let truncated = offset > 0 || emitted as u64 != total;
    (next_offset, truncated)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn remove_last_context(records: &mut [MatchRecord]) -> bool {
    for record in records.iter_mut().rev() {
        if record.context_after.pop().is_some() || record.context_before.pop().is_some() {
            record.context_truncated = true;
            return true;
        }
    }
    false
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn serialized_tool_output_size(content: &Value) -> Result<usize, ToolError> {
    serde_json::to_vec(&ToolOutput::success(content.clone()))
        .map(|bytes| bytes.len())
        .map_err(|_| scan_limit())
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_GREP_FILES_PATH_BYTES
        || path.starts_with('/')
        || path.chars().any(is_forbidden_character)
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
    if normalized.len() > MAX_GREP_FILES_PATH_BYTES {
        return Err(invalid_path());
    }
    Ok(normalized)
}

fn normalize_literal_pattern(pattern: &str) -> Result<String, ToolError> {
    if pattern.is_empty()
        || pattern.len() > MAX_GREP_FILES_PATTERN_BYTES
        || pattern.chars().any(is_forbidden_character)
    {
        return Err(invalid_pattern());
    }
    Ok(pattern.to_owned())
}

fn normalize_include_pattern(pattern: &str) -> Result<String, ToolError> {
    if pattern.is_empty()
        || pattern.len() > MAX_GREP_FILES_INCLUDE_BYTES
        || pattern.starts_with('/')
        || pattern.chars().any(is_forbidden_character)
    {
        return Err(invalid_include());
    }
    let mut normalized = String::with_capacity(pattern.len());
    for component in pattern.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return Err(invalid_include());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() || normalized.len() > MAX_GREP_FILES_INCLUDE_BYTES {
        return Err(invalid_include());
    }
    Ok(normalized)
}

fn is_forbidden_character(character: char) -> bool {
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

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
fn segment_matches(
    pattern: &[u8],
    candidate: &[u8],
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
) -> Result<bool, ToolError> {
    let mut pattern_index = 0_usize;
    let mut candidate_index = 0_usize;
    let mut latest_star = None;
    let mut star_candidate_index = 0_usize;
    while candidate_index < candidate.len() {
        if candidate_index.is_multiple_of(1024) {
            check_cancellation(cancellation)?;
        }
        budget.observe_include_steps(1)?;
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?'
                || pattern[pattern_index] == candidate[candidate_index])
        {
            pattern_index += 1;
            candidate_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            latest_star = Some(pattern_index);
            pattern_index += 1;
            star_candidate_index = candidate_index;
        } else if let Some(star_index) = latest_star {
            star_candidate_index += 1;
            candidate_index = star_candidate_index;
            pattern_index = star_index + 1;
        } else {
            return Ok(false);
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        budget.observe_include_steps(1)?;
        pattern_index += 1;
    }
    Ok(pattern_index == pattern.len())
}

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
fn path_matches(
    pattern_segments: &[&str],
    non_recursive_pattern_segments: usize,
    candidate: &str,
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
) -> Result<bool, ToolError> {
    budget.observe_include_steps(candidate.len())?;
    let candidate_segments = candidate.split('/').collect::<Vec<_>>();
    if non_recursive_pattern_segments > candidate_segments.len() {
        return Ok(false);
    }
    let columns = candidate_segments
        .len()
        .checked_add(1)
        .ok_or_else(scan_limit)?;
    let mut previous = vec![false; columns];
    let mut current = vec![false; columns];
    previous[0] = true;
    let mut previous_was_recursive = false;
    for pattern_segment in pattern_segments {
        check_cancellation(cancellation)?;
        budget.observe_include_steps(1)?;
        if *pattern_segment == "**" && previous_was_recursive {
            continue;
        }
        budget.observe_include_steps(columns)?;
        if *pattern_segment == "**" {
            current[0] = previous[0];
            for candidate_index in 1..=candidate_segments.len() {
                if candidate_index.is_multiple_of(1024) {
                    check_cancellation(cancellation)?;
                }
                current[candidate_index] =
                    previous[candidate_index] || current[candidate_index - 1];
            }
        } else {
            current[0] = false;
            for candidate_index in 1..=candidate_segments.len() {
                current[candidate_index] = if previous[candidate_index - 1] {
                    segment_matches(
                        pattern_segment.as_bytes(),
                        candidate_segments[candidate_index - 1].as_bytes(),
                        budget,
                        cancellation,
                    )?
                } else {
                    false
                };
            }
        }
        std::mem::swap(&mut previous, &mut current);
        previous_was_recursive = *pattern_segment == "**";
    }
    Ok(previous[candidate_segments.len()])
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn join_relative(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn join_workspace_path(search_path: &str, relative_path: &str) -> Result<String, ToolError> {
    let path = if search_path == "." {
        relative_path.to_owned()
    } else {
        let capacity = search_path
            .len()
            .checked_add(1)
            .and_then(|length| length.checked_add(relative_path.len()))
            .ok_or_else(scan_limit)?;
        let mut path = String::with_capacity(capacity);
        path.push_str(search_path);
        path.push('/');
        path.push_str(relative_path);
        path
    };
    if path.len() > MAX_GREP_FILES_RESULT_PATH_BYTES {
        return Err(scan_limit());
    }
    Ok(path)
}

fn grep_files_name() -> ToolName {
    ToolName::new(GREP_FILES_TOOL_NAME).expect("grep_files is a valid tool name")
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
        check_cancellation(cancellation)?;
        let metadata = rustix::fs::fstat(root).map_err(|_| unavailable())?;
        check_cancellation(cancellation)?;
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
    check_cancellation(cancellation)?;
    let root_metadata = rustix::fs::fstat(root).map_err(|_| unavailable())?;
    check_cancellation(cancellation)?;
    let root_path = rustix::fs::getpath(root).map_err(|_| unavailable())?;
    check_cancellation(cancellation)?;
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
    check_cancellation(cancellation)?;
    let parent = rustix::fs::openat(root, "..", directory_open_flags(), Mode::empty())
        .map_err(|_| unavailable())?;
    check_cancellation(cancellation)?;
    let linked =
        rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| unavailable())?;
    check_cancellation(cancellation)?;
    if linked.st_dev != root_metadata.st_dev
        || linked.st_ino != root_metadata.st_ino
        || !FileType::from_raw_mode(linked.st_mode).is_dir()
    {
        return Err(unavailable());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> GrepFilesToolOpenError {
    let kind = if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        GrepFilesToolOpenErrorKind::InvalidFileType
    } else {
        GrepFilesToolOpenErrorKind::Unavailable
    };
    GrepFilesToolOpenError::new(kind)
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
            "grep_files_cancelled",
            "grep_files execution was cancelled",
            false,
        ))
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "grep_files_invalid_arguments",
        "grep_files arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "grep_files_invalid_path",
        "grep_files path is invalid",
        false,
    )
}

fn invalid_pattern() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "grep_files_invalid_pattern",
        "grep_files pattern is invalid",
        false,
    )
}

fn invalid_include() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "grep_files_invalid_include",
        "grep_files include pattern is invalid",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "grep_files_unsupported_platform",
        "native grep_files is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "grep_files_not_found",
        "requested search root is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "grep_files_permission_denied",
        "requested search root cannot be searched",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rejected_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "grep_files_path_rejected",
        "requested path is not a confined regular file or directory",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "grep_files_unavailable",
        "requested content search is unavailable",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "grep_files_read_failed",
        "requested content search could not be completed",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn invalid_entry_name() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "grep_files_invalid_entry_name",
        "requested content search contains an unsupported entry name",
        false,
    )
}

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
fn scan_limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "grep_files_scan_limit",
        "requested content search exceeds the scan limit",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CancellationToken, GrepMode, LiteralMatcher, MAX_GREP_FILES_CONTENT_MATCH_STEPS,
        MAX_GREP_FILES_INCLUDE_MATCH_STEPS, ScanBudget, excerpt_containing_match,
        grep_files_input_schema, normalize_include_pattern, normalize_literal_pattern,
        normalize_relative_path, path_matches, segment_matches,
    };
    use serde_json::json;

    #[test]
    fn schema_is_strict_and_complete() {
        let schema = grep_files_input_schema();
        assert_eq!(schema["required"], json!(["pattern"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["properties"].as_object().unwrap().len(), 8);
        assert_eq!(
            schema["properties"]["mode"]["enum"],
            json!(["matches", "files_with_matches", "count"])
        );
    }

    #[test]
    fn normalizers_freeze_literal_path_and_include_roles() {
        assert_eq!(normalize_literal_pattern("../*.rs").unwrap(), "../*.rs");
        assert_eq!(normalize_relative_path("./src//.").unwrap(), "src");
        assert_eq!(
            normalize_include_pattern("./src//**/*.rs").unwrap(),
            "src/**/*.rs"
        );
        assert!(normalize_literal_pattern("").is_err());
        assert!(normalize_relative_path("../outside").is_err());
        assert!(normalize_include_pattern("../*.rs").is_err());
    }

    #[test]
    fn include_matcher_preserves_delivered_glob_grammar() {
        let mut budget = ScanBudget::default();
        let cancellation = CancellationToken::new();
        assert!(segment_matches(b"*.rs", b"lib.rs", &mut budget, &cancellation).unwrap());
        let segments = ["**", "*.rs"];
        assert!(path_matches(&segments, 1, "lib.rs", &mut budget, &cancellation).unwrap());
        assert!(path_matches(&segments, 1, "src/lib.rs", &mut budget, &cancellation).unwrap());
    }

    #[test]
    fn literal_matcher_is_linear_and_ascii_case_optional() {
        let mut budget = ScanBudget::default();
        let cancellation = CancellationToken::new();
        let matcher = LiteralMatcher::compile("Needle", true, &mut budget).unwrap();
        assert_eq!(
            matcher
                .find(b"a needle here", &mut budget, &cancellation)
                .unwrap(),
            Some(2)
        );
        let sensitive = LiteralMatcher::compile("Needle", false, &mut budget).unwrap();
        assert_eq!(
            sensitive
                .find(b"a needle here", &mut budget, &cancellation)
                .unwrap(),
            None
        );
    }

    #[test]
    fn excerpt_contains_complete_match_on_utf8_boundaries() {
        let prefix = "x".repeat(5000);
        let line = format!("{prefix}needle{}", "y".repeat(5000));
        let (start, excerpt) = excerpt_containing_match(&line, 5000, 6);
        assert!(line.is_char_boundary(start));
        assert!(excerpt.contains("needle"));
        assert!(excerpt.len() <= super::MAX_GREP_FILES_RESULT_LINE_BYTES);
    }

    #[test]
    fn exact_matcher_caps_allow_the_limit_and_reject_the_next_step() {
        let mut include = ScanBudget {
            include_match_steps: MAX_GREP_FILES_INCLUDE_MATCH_STEPS - 1,
            ..ScanBudget::default()
        };
        include.observe_include_steps(1).unwrap();
        assert!(include.observe_include_steps(1).is_err());

        let mut content = ScanBudget {
            content_match_steps: MAX_GREP_FILES_CONTENT_MATCH_STEPS - 1,
            ..ScanBudget::default()
        };
        content.observe_content_steps(1).unwrap();
        assert!(content.observe_content_steps(1).is_err());
    }

    #[test]
    fn every_scan_meter_allows_its_exact_cap_and_rejects_the_next_unit() {
        let mut entries = ScanBudget {
            visited_entries: super::MAX_GREP_FILES_VISITED_ENTRIES - 1,
            ..ScanBudget::default()
        };
        entries.observe_entry(0).unwrap();
        assert!(entries.observe_entry(0).is_err());

        let mut names = ScanBudget {
            total_entry_name_bytes: super::MAX_GREP_FILES_TOTAL_ENTRY_NAME_BYTES - 1,
            ..ScanBudget::default()
        };
        names.observe_entry(1).unwrap();
        assert!(names.observe_entry(1).is_err());

        let mut candidates = ScanBudget {
            candidate_files: super::MAX_GREP_FILES_CANDIDATE_FILES - 1,
            ..ScanBudget::default()
        };
        candidates.observe_candidate().unwrap();
        assert!(candidates.observe_candidate().is_err());

        let mut content = ScanBudget {
            total_content_bytes: super::MAX_GREP_FILES_TOTAL_CONTENT_BYTES - 1,
            ..ScanBudget::default()
        };
        content.observe_content_bytes(1).unwrap();
        assert!(content.observe_content_bytes(1).is_err());

        let mut include = ScanBudget {
            include_match_steps: MAX_GREP_FILES_INCLUDE_MATCH_STEPS - 1,
            ..ScanBudget::default()
        };
        include.observe_include_steps(1).unwrap();
        assert!(include.observe_include_steps(1).is_err());

        let mut literal = ScanBudget {
            content_match_steps: MAX_GREP_FILES_CONTENT_MATCH_STEPS - 1,
            ..ScanBudget::default()
        };
        literal.observe_content_steps(1).unwrap();
        assert!(literal.observe_content_steps(1).is_err());
    }

    #[test]
    fn mode_names_are_frozen() {
        assert_eq!(GrepMode::Matches.as_str(), "matches");
        assert_eq!(GrepMode::FilesWithMatches.as_str(), "files_with_matches");
        assert_eq!(GrepMode::Count.as_str(), "count");
    }
}
