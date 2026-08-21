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
use rustix::fs::{Dir, FileType, Mode, OFlags};

/// Maximum number of UTF-8 bytes accepted in a requested path.
pub const MAX_LIST_FILES_PATH_BYTES: usize = 4 * 1024;

/// Maximum number of directory entries returned by [`ListFilesTool`].
pub const MAX_LIST_FILES_ENTRIES: usize = 100;

/// Maximum aggregate number of UTF-8 name bytes returned by [`ListFilesTool`].
pub const MAX_LIST_FILES_TOTAL_NAME_BYTES: usize = 16 * 1024;

/// Registered name of [`ListFilesTool`].
pub const LIST_FILES_TOOL_NAME: &str = "list_files";

/// Stable category for failure to acquire a read-only workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListFilesToolOpenErrorKind {
    /// Native directory enumeration is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`ListFilesTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ListFilesToolOpenError {
    kind: ListFilesToolOpenErrorKind,
}

impl ListFilesToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> ListFilesToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: ListFilesToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for ListFilesToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ListFilesToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for ListFilesToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ListFilesToolOpenErrorKind::UnsupportedPlatform => {
                "native list_files is unsupported on this platform"
            }
            ListFilesToolOpenErrorKind::InvalidRoot => {
                "native list_files workspace root is invalid"
            }
            ListFilesToolOpenErrorKind::InvalidFileType => {
                "native list_files workspace root is not a directory"
            }
            ListFilesToolOpenErrorKind::Unavailable => {
                "native list_files workspace root is unavailable"
            }
        })
    }
}

impl Error for ListFilesToolOpenError {}

/// A read-only native tool confined to one explicitly opened workspace root.
///
/// Construction acquires the only ambient filesystem authority used by this
/// tool. Supported Linux and macOS implementations retain the opened directory
/// descriptor; later calls never reopen the workspace root by path.
pub struct ListFilesTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl ListFilesTool {
    /// Opens and retains an absolute workspace root without following its final
    /// component when native descriptor-relative access is supported.
    ///
    /// # Errors
    ///
    /// Returns a redacted typed failure when the platform is unsupported, the
    /// path is relative, or the root cannot be opened as a real directory.
    pub fn open(root: &Path) -> Result<Self, ListFilesToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(ListFilesToolOpenError::new(
                ListFilesToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(ListFilesToolOpenError::new(
                    ListFilesToolOpenErrorKind::InvalidRoot,
                ));
            }

            let descriptor = rustix::fs::open(
                &lexical_root,
                OFlags::RDONLY
                    | OFlags::DIRECTORY
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC
                    | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor).map_err(|_| {
                ListFilesToolOpenError::new(ListFilesToolOpenErrorKind::Unavailable)
            })?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(ListFilesToolOpenError::new(
                    ListFilesToolOpenErrorKind::InvalidFileType,
                ));
            }

            Ok(Self { root: descriptor })
        }
    }
}

impl fmt::Debug for ListFilesTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ListFilesTool")
            .finish_non_exhaustive()
    }
}

impl Tool for ListFilesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: list_files_name(),
            description: "List one directory within the configured workspace".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative directory path; defaults to the workspace root"
                    }
                },
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != list_files_name() {
            return Err(invalid_arguments());
        }
        let requested_path = decode_requested_arguments(call.arguments)?;
        let normalized = match requested_path {
            Some(path) => normalize_relative_path(&path)?,
            None => ".".to_owned(),
        };
        let prepared_arguments = json!({ "path": normalized });
        let path = prepared_arguments["path"]
            .as_str()
            .expect("prepared list_files path is a string")
            .to_owned();
        Ok(PreparedToolCall::new(
            Capability::Filesystem {
                access: FilesystemAccess::Enumerate,
                path,
            },
            prepared_arguments,
        ))
    }

    fn execute(
        &self,
        _context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            let path = decode_execution_arguments(arguments)?;
            let normalized = normalize_relative_path(&path)?;
            if normalized != path {
                return Err(invalid_arguments());
            }

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = cancellation;
                Err(unsupported_platform())
            }

            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.execute_unix(&normalized, &cancellation)
            }
        })
    }
}

fn decode_requested_arguments(arguments: Value) -> Result<Option<String>, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    match object.len() {
        0 => Ok(None),
        1 => match object.remove("path") {
            Some(Value::String(path)) => Ok(Some(path)),
            _ => Err(invalid_arguments()),
        },
        _ => Err(invalid_arguments()),
    }
}

fn decode_execution_arguments(arguments: Value) -> Result<String, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 1 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(path)) = object.remove("path") else {
        return Err(invalid_arguments());
    };
    Ok(path)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl ListFilesTool {
    fn execute_unix(
        &self,
        normalized: &str,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        check_cancellation(cancellation)?;
        let mut directory = None;
        if normalized != "." {
            for component in normalized.split('/') {
                check_cancellation(cancellation)?;
                let directory_fd = directory
                    .as_ref()
                    .map_or_else(|| self.root.as_fd(), AsFd::as_fd);
                directory = Some(
                    rustix::fs::openat(
                        directory_fd,
                        component,
                        OFlags::RDONLY
                            | OFlags::DIRECTORY
                            | OFlags::NOFOLLOW
                            | OFlags::CLOEXEC
                            | OFlags::NONBLOCK,
                        Mode::empty(),
                    )
                    .map_err(map_path_open_error)?,
                );
                check_cancellation(cancellation)?;
            }
        }

        let directory_fd = directory
            .as_ref()
            .map_or_else(|| self.root.as_fd(), AsFd::as_fd);
        let mut stream = Dir::read_from(directory_fd).map_err(map_directory_stream_error)?;
        let mut entries = Vec::new();
        let mut total_name_bytes = 0_usize;
        let mut truncated = false;

        loop {
            check_cancellation(cancellation)?;
            let next = stream.next();
            check_cancellation(cancellation)?;
            let Some(entry) = next else {
                break;
            };
            let entry = entry.map_err(|_| read_failed())?;
            let name_bytes = entry.file_name().to_bytes();
            if name_bytes == b"." || name_bytes == b".." {
                continue;
            }
            let name = std::str::from_utf8(name_bytes).map_err(|_| invalid_entry_name())?;
            if name.chars().any(is_forbidden_path_character) {
                return Err(invalid_entry_name());
            }
            if entries.len() >= MAX_LIST_FILES_ENTRIES
                || total_name_bytes
                    .checked_add(name_bytes.len())
                    .is_none_or(|total| total > MAX_LIST_FILES_TOTAL_NAME_BYTES)
            {
                truncated = true;
                break;
            }

            total_name_bytes += name_bytes.len();
            entries.push(json!({
                "name": name,
                "kind": classify_file_type(entry.file_type()),
            }));
        }

        check_cancellation(cancellation)?;
        entries.sort_unstable_by(|left, right| {
            left["name"]
                .as_str()
                .expect("list_files entry name is a string")
                .cmp(
                    right["name"]
                        .as_str()
                        .expect("list_files entry name is a string"),
                )
        });
        check_cancellation(cancellation)?;
        Ok(ToolOutput::success(json!({
            "path": normalized,
            "entries": entries,
            "truncated": truncated,
        })))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn classify_file_type(file_type: FileType) -> &'static str {
    if file_type.is_file() {
        "file"
    } else if file_type.is_dir() {
        "directory"
    } else if file_type.is_symlink() {
        "symlink"
    } else {
        "other"
    }
}

fn list_files_name() -> ToolName {
    ToolName::new(LIST_FILES_TOOL_NAME).expect("list_files is a valid tool name")
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_LIST_FILES_PATH_BYTES
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> ListFilesToolOpenError {
    let kind = if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        ListFilesToolOpenErrorKind::InvalidFileType
    } else {
        ListFilesToolOpenErrorKind::Unavailable
    };
    ListFilesToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_path_open_error(error: rustix::io::Errno) -> ToolError {
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

fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(ToolError::new(
            ToolErrorKind::Cancelled,
            "list_files_cancelled",
            "list_files execution was cancelled",
            false,
        ))
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "list_files_invalid_arguments",
        "list_files arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "list_files_invalid_path",
        "list_files path is invalid",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "list_files_unsupported_platform",
        "native list_files is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "list_files_not_found",
        "requested directory is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "list_files_permission_denied",
        "requested directory cannot be listed",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rejected_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "list_files_path_rejected",
        "requested path is not a confined directory",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "list_files_unavailable",
        "requested directory is unavailable",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "list_files_read_failed",
        "requested directory could not be listed",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn invalid_entry_name() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "list_files_invalid_entry_name",
        "requested directory contains an unsupported entry name",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{MAX_LIST_FILES_PATH_BYTES, normalize_relative_path};

    #[test]
    fn lexical_normalization_is_workspace_relative() {
        assert_eq!(
            normalize_relative_path("./src//./lib.rs").unwrap(),
            "src/lib.rs"
        );
        assert_eq!(normalize_relative_path("./").unwrap(), ".");
        assert_eq!(
            normalize_relative_path("name\\with\\slashes").unwrap(),
            "name\\with\\slashes"
        );
        assert_eq!(
            normalize_relative_path(" surrounding spaces ").unwrap(),
            " surrounding spaces "
        );
    }

    #[test]
    fn lexical_normalization_rejects_unsafe_or_ambiguous_paths() {
        for path in [
            "",
            "..",
            "src/../secret",
            "/absolute",
            "nul\0byte",
            "line\u{2028}separator",
            "right-to-left\u{202e}override",
            "isolate\u{2066}text",
        ] {
            assert!(normalize_relative_path(path).is_err(), "accepted {path:?}");
        }
        assert!(normalize_relative_path(&"x".repeat(MAX_LIST_FILES_PATH_BYTES + 1)).is_err());
    }
}
