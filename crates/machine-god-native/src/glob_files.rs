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
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::BinaryHeap;

/// Maximum number of UTF-8 bytes accepted in a requested glob pattern.
pub const MAX_GLOB_FILES_PATTERN_BYTES: usize = 4 * 1024;

/// Maximum number of UTF-8 bytes accepted in a requested search-root path.
pub const MAX_GLOB_FILES_PATH_BYTES: usize = 4 * 1024;

/// Maximum number of UTF-8 bytes in one returned workspace-relative path.
pub const MAX_GLOB_FILES_RESULT_PATH_BYTES: usize = 4 * 1024;

/// Maximum number of paths returned by [`GlobFilesTool`].
pub const MAX_GLOB_FILES_MATCHES: usize = 100;

/// Maximum aggregate number of raw UTF-8 path bytes returned by [`GlobFilesTool`].
pub const MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES: usize = 16 * 1024;

/// Maximum number of non-dot directory entries visited by one search.
pub const MAX_GLOB_FILES_VISITED_ENTRIES: usize = 100_000;

/// Maximum aggregate number of raw non-dot entry-name bytes visited by one search.
pub const MAX_GLOB_FILES_TOTAL_ENTRY_NAME_BYTES: usize = 16 * 1024 * 1024;

/// Maximum recursive directory traversal depth below the selected search root.
pub const MAX_GLOB_FILES_DEPTH: usize = 256;

/// Registered name of [`GlobFilesTool`].
pub const GLOB_FILES_TOOL_NAME: &str = "glob_files";

/// Stable category for failure to acquire a read-only workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobFilesToolOpenErrorKind {
    /// Native recursive enumeration is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`GlobFilesTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct GlobFilesToolOpenError {
    kind: GlobFilesToolOpenErrorKind,
}

impl GlobFilesToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> GlobFilesToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: GlobFilesToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for GlobFilesToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GlobFilesToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for GlobFilesToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            GlobFilesToolOpenErrorKind::UnsupportedPlatform => {
                "native glob_files is unsupported on this platform"
            }
            GlobFilesToolOpenErrorKind::InvalidRoot => {
                "native glob_files workspace root is invalid"
            }
            GlobFilesToolOpenErrorKind::InvalidFileType => {
                "native glob_files workspace root is not a directory"
            }
            GlobFilesToolOpenErrorKind::Unavailable => {
                "native glob_files workspace root is unavailable"
            }
        })
    }
}

impl Error for GlobFilesToolOpenError {}

/// A bounded recursive native glob tool confined to one retained workspace root.
///
/// Construction acquires the only ambient filesystem authority used by this
/// tool. Supported Linux and macOS implementations retain the opened directory
/// descriptor; later calls never reopen the workspace root by path.
pub struct GlobFilesTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl GlobFilesTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) const fn from_root_descriptor(root: OwnedFd) -> Self {
        Self { root }
    }

    /// Opens and retains an absolute workspace root without following its final
    /// component when native descriptor-relative access is supported.
    ///
    /// # Errors
    ///
    /// Returns a redacted typed failure when the platform is unsupported, the
    /// path is relative, or the root cannot be opened as a real directory.
    pub fn open(root: &Path) -> Result<Self, GlobFilesToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(GlobFilesToolOpenError::new(
                GlobFilesToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(GlobFilesToolOpenError::new(
                    GlobFilesToolOpenErrorKind::InvalidRoot,
                ));
            }

            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor).map_err(|_| {
                GlobFilesToolOpenError::new(GlobFilesToolOpenErrorKind::Unavailable)
            })?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(GlobFilesToolOpenError::new(
                    GlobFilesToolOpenErrorKind::InvalidFileType,
                ));
            }

            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for GlobFilesTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GlobFilesTool")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GlobMode {
    Matches,
    Count,
}

impl GlobMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Matches => "matches",
            Self::Count => "count",
        }
    }

    fn parse(value: &str) -> Result<Self, ToolError> {
        match value {
            "matches" => Ok(Self::Matches),
            "count" => Ok(Self::Count),
            _ => Err(invalid_arguments()),
        }
    }
}

struct RequestedArguments {
    pattern: String,
    path: Option<String>,
    mode: Option<GlobMode>,
}

struct ExecutionArguments {
    pattern: String,
    path: String,
    mode: GlobMode,
}

impl Tool for GlobFilesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: glob_files_name(),
            description: "Find file paths matching a glob pattern within the configured workspace"
                .to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern using / as the path separator"
                    },
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative directory path; defaults to the workspace root"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["matches", "count"],
                        "description": "Return matching paths or only their count; defaults to matches"
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != glob_files_name() {
            return Err(invalid_arguments());
        }
        let arguments = decode_requested_arguments(call.arguments)?;
        let pattern = normalize_pattern(&arguments.pattern)?;
        let path = match arguments.path {
            Some(path) => normalize_relative_path(&path)?,
            None => ".".to_owned(),
        };
        let mode = arguments.mode.unwrap_or(GlobMode::Matches);
        let capability_path = path.clone();
        Ok(PreparedToolCall::new(
            Capability::Filesystem {
                access: FilesystemAccess::EnumerateRecursive,
                path: capability_path,
            },
            json!({
                "pattern": pattern,
                "path": path,
                "mode": mode.as_str(),
            }),
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
            let pattern = normalize_pattern(&arguments.pattern)?;
            let path = normalize_relative_path(&arguments.path)?;
            if pattern != arguments.pattern || path != arguments.path {
                return Err(invalid_arguments());
            }

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = (cancellation, arguments.mode);
                Err(unsupported_platform())
            }

            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.execute_unix(&pattern, &path, arguments.mode, &cancellation)
            }
        })
    }
}

fn decode_requested_arguments(arguments: Value) -> Result<RequestedArguments, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.is_empty() || object.len() > 3 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(pattern)) = object.remove("pattern") else {
        return Err(invalid_arguments());
    };
    let path = match object.remove("path") {
        Some(Value::String(path)) => Some(path),
        Some(_) => return Err(invalid_arguments()),
        None => None,
    };
    let mode = match object.remove("mode") {
        Some(Value::String(mode)) => Some(GlobMode::parse(&mode)?),
        Some(_) => return Err(invalid_arguments()),
        None => None,
    };
    if !object.is_empty() {
        return Err(invalid_arguments());
    }
    Ok(RequestedArguments {
        pattern,
        path,
        mode,
    })
}

fn decode_execution_arguments(arguments: Value) -> Result<ExecutionArguments, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 3 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(pattern)) = object.remove("pattern") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(path)) = object.remove("path") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(mode)) = object.remove("mode") else {
        return Err(invalid_arguments());
    };
    if !object.is_empty() {
        return Err(invalid_arguments());
    }
    Ok(ExecutionArguments {
        pattern,
        path,
        mode: GlobMode::parse(&mode)?,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct DirectoryFrame {
    directory: OwnedFd,
    relative_path: String,
    depth: usize,
    entries: std::vec::IntoIter<String>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct ScanBudget {
    visited_entries: usize,
    total_entry_name_bytes: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl ScanBudget {
    fn observe_entry(&mut self, name_bytes: usize) -> Result<(), ToolError> {
        self.visited_entries = self.visited_entries.checked_add(1).ok_or_else(scan_limit)?;
        self.total_entry_name_bytes = self
            .total_entry_name_bytes
            .checked_add(name_bytes)
            .ok_or_else(scan_limit)?;
        if self.visited_entries > MAX_GLOB_FILES_VISITED_ENTRIES
            || self.total_entry_name_bytes > MAX_GLOB_FILES_TOTAL_ENTRY_NAME_BYTES
        {
            return Err(scan_limit());
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ScanResults {
    checked_matches: u64,
    retained_matches: BinaryHeap<String>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl GlobFilesTool {
    fn execute_unix(
        &self,
        pattern: &str,
        search_path: &str,
        mode: GlobMode,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let search_root = self.open_search_root(search_path, cancellation)?;
        let results = scan_tree(pattern, search_path, search_root, cancellation)?;
        render_results(pattern, search_path, mode, results, cancellation)
    }

    fn open_search_root(
        &self,
        search_path: &str,
        cancellation: &CancellationToken,
    ) -> Result<OwnedFd, ToolError> {
        check_cancellation(cancellation)?;
        let mut search_root = rustix::fs::openat(
            self.root.as_fd(),
            ".",
            directory_open_flags(),
            Mode::empty(),
        )
        .map_err(|_| unavailable())?;
        check_cancellation(cancellation)?;
        ensure_root_is_linked(search_root.as_fd(), cancellation)?;

        if search_path != "." {
            for component in search_path.split('/') {
                check_cancellation(cancellation)?;
                search_root = rustix::fs::openat(
                    search_root.as_fd(),
                    component,
                    directory_open_flags(),
                    Mode::empty(),
                )
                .map_err(map_search_root_open_error)?;
                check_cancellation(cancellation)?;
            }
        }
        Ok(search_root)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn scan_tree(
    pattern: &str,
    search_path: &str,
    search_root: OwnedFd,
    cancellation: &CancellationToken,
) -> Result<ScanResults, ToolError> {
    let slashful_pattern = pattern.contains('/');
    let pattern_segments = pattern.split('/').collect::<Vec<_>>();
    let mut budget = ScanBudget::default();
    let mut stack = vec![make_directory_frame(
        search_root,
        String::new(),
        0,
        &mut budget,
        cancellation,
    )?];
    let mut results = ScanResults {
        checked_matches: 0,
        retained_matches: BinaryHeap::with_capacity(MAX_GLOB_FILES_MATCHES),
    };

    while !stack.is_empty() {
        check_cancellation(cancellation)?;
        let next = stack
            .last_mut()
            .expect("nonempty glob traversal stack")
            .entries
            .next();
        let Some(name) = next else {
            stack.pop();
            continue;
        };

        let (relative_path, depth, metadata) = {
            let frame = stack.last().expect("nonempty glob traversal stack");
            let relative_path = join_relative(&frame.relative_path, &name);
            check_cancellation(cancellation)?;
            let metadata = match rustix::fs::statat(
                frame.directory.as_fd(),
                name.as_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(metadata) => metadata,
                Err(error) if error == rustix::io::Errno::NOENT => {
                    check_cancellation(cancellation)?;
                    continue;
                }
                Err(error) => return Err(map_scan_metadata_error(error)),
            };
            check_cancellation(cancellation)?;
            (relative_path, frame.depth, metadata)
        };

        let file_type = FileType::from_raw_mode(metadata.st_mode);
        if file_type.is_dir() {
            let child_depth = depth.checked_add(1).ok_or_else(scan_limit)?;
            if child_depth > MAX_GLOB_FILES_DEPTH {
                return Err(scan_limit());
            }
            check_cancellation(cancellation)?;
            let child = match rustix::fs::openat(
                stack
                    .last()
                    .expect("nonempty glob traversal stack")
                    .directory
                    .as_fd(),
                name.as_str(),
                directory_open_flags(),
                Mode::empty(),
            ) {
                Ok(child) => child,
                Err(error) if error == rustix::io::Errno::NOENT => {
                    check_cancellation(cancellation)?;
                    continue;
                }
                Err(error) => return Err(map_descendant_open_error(error)),
            };
            check_cancellation(cancellation)?;
            stack.push(make_directory_frame(
                child,
                relative_path,
                child_depth,
                &mut budget,
                cancellation,
            )?);
        } else if file_type.is_file() || file_type.is_symlink() {
            observe_candidate(
                pattern,
                &pattern_segments,
                slashful_pattern,
                search_path,
                &relative_path,
                &name,
                &mut results,
                cancellation,
            )?;
        }
    }
    check_cancellation(cancellation)?;
    Ok(results)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn observe_candidate(
    pattern: &str,
    pattern_segments: &[&str],
    slashful_pattern: bool,
    search_path: &str,
    relative_path: &str,
    name: &str,
    results: &mut ScanResults,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let workspace_path = join_workspace_path(search_path, relative_path)?;
    let candidate = if slashful_pattern {
        relative_path
    } else {
        name
    };
    check_cancellation(cancellation)?;
    let matched = if slashful_pattern {
        path_matches(pattern_segments, candidate)
    } else {
        segment_matches(pattern.as_bytes(), candidate.as_bytes())
    };
    check_cancellation(cancellation)?;
    if matched {
        results.checked_matches = results
            .checked_matches
            .checked_add(1)
            .ok_or_else(scan_limit)?;
        retain_smallest(&mut results.retained_matches, workspace_path);
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
fn render_results(
    pattern: &str,
    search_path: &str,
    mode: GlobMode,
    results: ScanResults,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    check_cancellation(cancellation)?;
    if mode == GlobMode::Count {
        return Ok(ToolOutput::success(json!({
            "path": search_path,
            "pattern": pattern,
            "mode": mode.as_str(),
            "count": results.checked_matches,
        })));
    }

    let mut matches = results.retained_matches.into_vec();
    check_cancellation(cancellation)?;
    matches.sort_unstable();
    check_cancellation(cancellation)?;
    let mut total_path_bytes = 0_usize;
    let prefix_length = matches
        .iter()
        .position(|path| {
            let Some(next_total) = total_path_bytes.checked_add(path.len()) else {
                return true;
            };
            if next_total > MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES {
                true
            } else {
                total_path_bytes = next_total;
                false
            }
        })
        .unwrap_or(matches.len());
    matches.truncate(prefix_length);
    let truncated = results.checked_matches > matches.len() as u64;
    check_cancellation(cancellation)?;
    Ok(ToolOutput::success(json!({
        "path": search_path,
        "pattern": pattern,
        "mode": mode.as_str(),
        "matches": matches,
        "truncated": truncated,
    })))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_directory_entries(
    directory: rustix::fd::BorrowedFd<'_>,
    budget: &mut ScanBudget,
    cancellation: &CancellationToken,
) -> Result<Vec<String>, ToolError> {
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
        if name.chars().any(is_forbidden_path_character) {
            return Err(invalid_entry_name());
        }
        entries.push(name.to_owned());
    }
    check_cancellation(cancellation)?;
    entries.sort_unstable();
    check_cancellation(cancellation)?;
    Ok(entries)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn retain_smallest(heap: &mut BinaryHeap<String>, path: String) {
    if heap.len() < MAX_GLOB_FILES_MATCHES {
        heap.push(path);
    } else if heap.peek().is_some_and(|largest| path < *largest) {
        let mut largest = heap.peek_mut().expect("full glob result heap is nonempty");
        *largest = path;
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn join_relative(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        let mut path = String::with_capacity(parent.len() + 1 + name.len());
        path.push_str(parent);
        path.push('/');
        path.push_str(name);
        path
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn join_workspace_path(search_path: &str, relative_path: &str) -> Result<String, ToolError> {
    let path = if search_path == "." {
        relative_path.to_owned()
    } else {
        let Some(capacity) = search_path
            .len()
            .checked_add(1)
            .and_then(|length| length.checked_add(relative_path.len()))
        else {
            return Err(scan_limit());
        };
        let mut path = String::with_capacity(capacity);
        path.push_str(search_path);
        path.push('/');
        path.push_str(relative_path);
        path
    };
    if path.len() > MAX_GLOB_FILES_RESULT_PATH_BYTES {
        return Err(scan_limit());
    }
    Ok(path)
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_GLOB_FILES_PATH_BYTES
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
    if normalized.len() > MAX_GLOB_FILES_PATH_BYTES {
        return Err(invalid_path());
    }
    Ok(normalized)
}

fn normalize_pattern(pattern: &str) -> Result<String, ToolError> {
    if pattern.is_empty()
        || pattern.len() > MAX_GLOB_FILES_PATTERN_BYTES
        || pattern.starts_with('/')
        || pattern.chars().any(is_forbidden_path_character)
    {
        return Err(invalid_pattern());
    }

    let mut normalized = String::with_capacity(pattern.len());
    for component in pattern.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return Err(invalid_pattern());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() || normalized.len() > MAX_GLOB_FILES_PATTERN_BYTES {
        return Err(invalid_pattern());
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

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
fn segment_matches(pattern: &[u8], candidate: &[u8]) -> bool {
    let mut pattern_index = 0_usize;
    let mut candidate_index = 0_usize;
    let mut latest_star = None;
    let mut star_candidate_index = 0_usize;

    while candidate_index < candidate.len() {
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
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
fn path_matches(pattern_segments: &[&str], candidate: &str) -> bool {
    let candidate_segments = candidate.split('/').collect::<Vec<_>>();
    if pattern_segments
        .iter()
        .filter(|segment| **segment != "**")
        .count()
        > candidate_segments.len()
    {
        return false;
    }
    let mut previous = vec![false; candidate_segments.len() + 1];
    let mut current = vec![false; candidate_segments.len() + 1];
    previous[0] = true;
    let mut previous_was_recursive = false;

    for pattern_segment in pattern_segments {
        if *pattern_segment == "**" && previous_was_recursive {
            continue;
        }
        current.fill(false);
        if *pattern_segment == "**" {
            current[0] = previous[0];
            for candidate_index in 1..=candidate_segments.len() {
                current[candidate_index] =
                    previous[candidate_index] || current[candidate_index - 1];
            }
        } else {
            for candidate_index in 1..=candidate_segments.len() {
                current[candidate_index] = previous[candidate_index - 1]
                    && segment_matches(
                        pattern_segment.as_bytes(),
                        candidate_segments[candidate_index - 1].as_bytes(),
                    );
            }
        }
        std::mem::swap(&mut previous, &mut current);
        previous_was_recursive = *pattern_segment == "**";
    }
    previous[candidate_segments.len()]
}

fn glob_files_name() -> ToolName {
    ToolName::new(GLOB_FILES_TOOL_NAME).expect("glob_files is a valid tool name")
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
    let linked_metadata =
        rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| unavailable())?;
    check_cancellation(cancellation)?;
    if linked_metadata.st_dev != root_metadata.st_dev
        || linked_metadata.st_ino != root_metadata.st_ino
        || !FileType::from_raw_mode(linked_metadata.st_mode).is_dir()
    {
        return Err(unavailable());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> GlobFilesToolOpenError {
    let kind = if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        GlobFilesToolOpenErrorKind::InvalidFileType
    } else {
        GlobFilesToolOpenErrorKind::Unavailable
    };
    GlobFilesToolOpenError::new(kind)
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
fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(ToolError::new(
            ToolErrorKind::Cancelled,
            "glob_files_cancelled",
            "glob_files execution was cancelled",
            false,
        ))
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "glob_files_invalid_arguments",
        "glob_files arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "glob_files_invalid_path",
        "glob_files path is invalid",
        false,
    )
}

fn invalid_pattern() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "glob_files_invalid_pattern",
        "glob_files pattern is invalid",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "glob_files_unsupported_platform",
        "native glob_files is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "glob_files_not_found",
        "requested search root is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "glob_files_permission_denied",
        "requested search root cannot be enumerated",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rejected_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "glob_files_path_rejected",
        "requested path is not a confined directory",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "glob_files_unavailable",
        "requested glob search is unavailable",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "glob_files_read_failed",
        "requested glob search could not be completed",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn invalid_entry_name() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "glob_files_invalid_entry_name",
        "requested glob search contains an unsupported entry name",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn scan_limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "glob_files_scan_limit",
        "requested glob search exceeds the scan limit",
        false,
    )
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use super::{
        MAX_GLOB_FILES_TOTAL_ENTRY_NAME_BYTES, MAX_GLOB_FILES_VISITED_ENTRIES, ScanBudget,
    };
    use super::{normalize_pattern, normalize_relative_path, path_matches, segment_matches};

    #[test]
    fn normalization_is_confined_and_canonical() {
        assert_eq!(
            normalize_relative_path("./src//./nested").unwrap(),
            "src/nested"
        );
        assert_eq!(normalize_relative_path("./").unwrap(), ".");
        assert_eq!(
            normalize_pattern("./src//**/./*.rs").unwrap(),
            "src/**/*.rs"
        );
        assert_eq!(
            normalize_pattern(r"name\with\slashes").unwrap(),
            r"name\with\slashes"
        );
        for pattern in ["", ".", "./", "..", "src/../*.rs", "/absolute"] {
            assert!(normalize_pattern(pattern).is_err(), "accepted {pattern:?}");
        }
    }

    #[test]
    fn segment_matcher_is_byte_oriented_and_treats_syntax_literally() {
        assert!(segment_matches(b"a*t", b"alphabet"));
        assert!(segment_matches(b"??", "é".as_bytes()));
        assert!(!segment_matches(b"?", "é".as_bytes()));
        assert!(segment_matches(br"[x]{y}\z", br"[x]{y}\z"));
        assert!(segment_matches(b"a**b", b"axxxb"));
        assert!(segment_matches(b"***", b"anything"));
    }

    #[test]
    fn path_matcher_supports_only_exact_recursive_double_star_segments() {
        assert!(path_matches(&["**", "*.rs"], "lib.rs"));
        assert!(path_matches(&["**", "*.rs"], "src/nested/lib.rs"));
        assert!(path_matches(&["src", "**", "lib.rs"], "src/lib.rs"));
        assert!(path_matches(&["src", "**", "lib.rs"], "src/nested/lib.rs"));
        assert!(!path_matches(&["***", "lib.rs"], "src/nested/lib.rs"));
        assert!(path_matches(&["a**b", "file"], "axxb/file"));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn scan_budget_accepts_exact_boundaries_and_rejects_the_next_observation() {
        let mut entry_limit = ScanBudget {
            visited_entries: MAX_GLOB_FILES_VISITED_ENTRIES - 1,
            total_entry_name_bytes: 0,
        };
        entry_limit.observe_entry(0).unwrap();
        assert!(entry_limit.observe_entry(0).is_err());

        let mut byte_limit = ScanBudget {
            visited_entries: 0,
            total_entry_name_bytes: MAX_GLOB_FILES_TOTAL_ENTRY_NAME_BYTES - 1,
        };
        byte_limit.observe_entry(1).unwrap();
        assert!(byte_limit.observe_entry(1).is_err());
    }
}
