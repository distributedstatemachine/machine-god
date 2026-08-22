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
use rustix::fs::{AtFlags, FileType, Mode, OFlags};

/// Maximum number of UTF-8 bytes accepted in a requested path.
pub const MAX_FILE_INFO_PATH_BYTES: usize = 4 * 1024;

/// Registered name of [`FileInfoTool`].
pub const FILE_INFO_TOOL_NAME: &str = "file_info";

/// Stable category for failure to acquire a read-only workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileInfoToolOpenErrorKind {
    /// Native metadata inspection is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`FileInfoTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct FileInfoToolOpenError {
    kind: FileInfoToolOpenErrorKind,
}

impl FileInfoToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> FileInfoToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: FileInfoToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for FileInfoToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileInfoToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for FileInfoToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            FileInfoToolOpenErrorKind::UnsupportedPlatform => {
                "native file_info is unsupported on this platform"
            }
            FileInfoToolOpenErrorKind::InvalidRoot => "native file_info workspace root is invalid",
            FileInfoToolOpenErrorKind::InvalidFileType => {
                "native file_info workspace root is not a directory"
            }
            FileInfoToolOpenErrorKind::Unavailable => {
                "native file_info workspace root is unavailable"
            }
        })
    }
}

impl Error for FileInfoToolOpenError {}

/// A native metadata tool confined to one explicitly opened workspace root.
///
/// Construction acquires the only ambient filesystem authority used by this
/// tool. Supported Linux and macOS implementations retain the opened directory
/// descriptor; later calls never reopen the workspace root by its injected path.
pub struct FileInfoTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl FileInfoTool {
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
    pub fn open(root: &Path) -> Result<Self, FileInfoToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(FileInfoToolOpenError::new(
                FileInfoToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(FileInfoToolOpenError::new(
                    FileInfoToolOpenErrorKind::InvalidRoot,
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
            let metadata = rustix::fs::fstat(&descriptor)
                .map_err(|_| FileInfoToolOpenError::new(FileInfoToolOpenErrorKind::Unavailable))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(FileInfoToolOpenError::new(
                    FileInfoToolOpenErrorKind::InvalidFileType,
                ));
            }

            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for FileInfoTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileInfoTool")
            .finish_non_exhaustive()
    }
}

struct FileInfoArguments {
    path: String,
}

impl Tool for FileInfoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: file_info_name(),
            description: "Inspect metadata for one path within the configured workspace".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative path to inspect"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != file_info_name() {
            return Err(invalid_arguments());
        }
        let arguments = decode_arguments(call.arguments)?;
        let normalized = normalize_relative_path(&arguments.path)?;
        let prepared_arguments = json!({ "path": normalized });
        let path = prepared_arguments["path"]
            .as_str()
            .expect("prepared file_info path is a string")
            .to_owned();
        Ok(PreparedToolCall::new(
            Capability::Filesystem {
                access: FilesystemAccess::Metadata,
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
            let arguments = decode_arguments(arguments)?;
            let normalized = normalize_relative_path(&arguments.path)?;
            if normalized != arguments.path {
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

fn decode_arguments(arguments: Value) -> Result<FileInfoArguments, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 1 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(path)) = object.remove("path") else {
        return Err(invalid_arguments());
    };
    Ok(FileInfoArguments { path })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FileInfoTool {
    fn execute_unix(
        &self,
        normalized: &str,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        check_cancellation(cancellation)?;
        let root = rustix::fs::openat(
            self.root.as_fd(),
            ".",
            OFlags::RDONLY
                | OFlags::DIRECTORY
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| unavailable())?;
        check_cancellation(cancellation)?;
        ensure_root_is_linked(root.as_fd())?;

        let (metadata, basename) = if normalized == "." {
            check_cancellation(cancellation)?;
            let metadata = rustix::fs::fstat(&root).map_err(|_| metadata_failed())?;
            check_cancellation(cancellation)?;
            (metadata, None)
        } else {
            let mut directory = root;
            let mut components = normalized.split('/').peekable();
            loop {
                check_cancellation(cancellation)?;
                let component = components.next().ok_or_else(invalid_arguments)?;
                if components.peek().is_some() {
                    directory = rustix::fs::openat(
                        directory.as_fd(),
                        component,
                        OFlags::RDONLY
                            | OFlags::DIRECTORY
                            | OFlags::NOFOLLOW
                            | OFlags::CLOEXEC
                            | OFlags::NONBLOCK,
                        Mode::empty(),
                    )
                    .map_err(map_ancestor_open_error)?;
                    check_cancellation(cancellation)?;
                } else {
                    check_cancellation(cancellation)?;
                    let metadata =
                        rustix::fs::statat(directory.as_fd(), component, AtFlags::SYMLINK_NOFOLLOW)
                            .map_err(map_metadata_error)?;
                    check_cancellation(cancellation)?;
                    break (metadata, Some(component));
                }
            }
        };

        let file_type = FileType::from_raw_mode(metadata.st_mode);
        let kind = classify_file_type(file_type);
        let size_bytes = u64::try_from(metadata.st_size).map_err(|_| invalid_metadata())?;
        let unix_seconds: i64 = metadata.st_mtime;
        let nanoseconds = u32::try_from(metadata.st_mtime_nsec)
            .ok()
            .filter(|value| *value < 1_000_000_000)
            .ok_or_else(invalid_metadata)?;
        let extension = if file_type.is_file() {
            basename.and_then(file_extension)
        } else {
            None
        };

        check_cancellation(cancellation)?;
        Ok(ToolOutput::success(json!({
            "path": normalized,
            "kind": kind,
            "size_bytes": size_bytes,
            "modified": {
                "unix_seconds": unix_seconds,
                "nanoseconds": nanoseconds,
            },
            "extension": extension,
        })))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_root_is_linked(root: rustix::fd::BorrowedFd<'_>) -> Result<(), ToolError> {
    #[cfg(target_os = "linux")]
    {
        if rustix::fs::fstat(root).map_err(|_| unavailable())?.st_nlink == 0 {
            return Err(unavailable());
        }
    }
    #[cfg(target_os = "macos")]
    ensure_macos_root_is_linked(root)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_macos_root_is_linked(root: rustix::fd::BorrowedFd<'_>) -> Result<(), ToolError> {
    let root_metadata = rustix::fs::fstat(root).map_err(|_| unavailable())?;
    let root_path = rustix::fs::getpath(root).map_err(|_| unavailable())?;
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
    let parent = rustix::fs::openat(
        root,
        "..",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| unavailable())?;
    let linked_metadata =
        rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| unavailable())?;
    if linked_metadata.st_dev != root_metadata.st_dev
        || linked_metadata.st_ino != root_metadata.st_ino
        || !FileType::from_raw_mode(linked_metadata.st_mode).is_dir()
    {
        return Err(unavailable());
    }
    Ok(())
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn file_extension(basename: &str) -> Option<&str> {
    let dot = basename.rfind('.')?;
    if dot == 0 || dot + 1 == basename.len() {
        None
    } else {
        Some(&basename[dot + 1..])
    }
}

fn file_info_name() -> ToolName {
    ToolName::new(FILE_INFO_TOOL_NAME).expect("file_info is a valid tool name")
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_FILE_INFO_PATH_BYTES
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
fn map_root_open_error(error: rustix::io::Errno) -> FileInfoToolOpenError {
    let kind = if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        FileInfoToolOpenErrorKind::InvalidFileType
    } else {
        FileInfoToolOpenErrorKind::Unavailable
    };
    FileInfoToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_ancestor_open_error(error: rustix::io::Errno) -> ToolError {
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
fn map_metadata_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::NOENT {
        not_found()
    } else if error == rustix::io::Errno::NOTDIR {
        rejected_path()
    } else if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else if error == rustix::io::Errno::NAMETOOLONG {
        unavailable()
    } else {
        metadata_failed()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(ToolError::new(
            ToolErrorKind::Cancelled,
            "file_info_cancelled",
            "file_info execution was cancelled",
            false,
        ))
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "file_info_invalid_arguments",
        "file_info arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "file_info_invalid_path",
        "file_info path is invalid",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "file_info_unsupported_platform",
        "native file_info is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "file_info_not_found",
        "requested path is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "file_info_permission_denied",
        "requested path metadata cannot be inspected",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rejected_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "file_info_path_rejected",
        "requested path is not confined to the workspace",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "file_info_unavailable",
        "requested path metadata is unavailable",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn metadata_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "file_info_metadata_failed",
        "requested path metadata could not be inspected",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn invalid_metadata() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "file_info_invalid_metadata",
        "requested path metadata is invalid",
        false,
    )
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use super::file_extension;
    use super::{MAX_FILE_INFO_PATH_BYTES, normalize_relative_path};

    #[test]
    fn lexical_normalization_is_workspace_relative_and_accepts_root() {
        assert_eq!(
            normalize_relative_path("./src//./lib.rs").unwrap(),
            "src/lib.rs"
        );
        assert_eq!(normalize_relative_path("./").unwrap(), ".");
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
        assert!(normalize_relative_path(&"x".repeat(MAX_FILE_INFO_PATH_BYTES + 1)).is_err());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn extension_uses_a_nonleading_final_dot_with_a_nonempty_suffix() {
        assert_eq!(file_extension("a.tar.gz"), Some("gz"));
        assert_eq!(file_extension(".config.json"), Some("json"));
        assert_eq!(file_extension(".bashrc"), None);
        assert_eq!(file_extension("foo."), None);
        assert_eq!(file_extension("plain"), None);
    }
}
