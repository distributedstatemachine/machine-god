use std::error::Error;
use std::fmt;
use std::path::{Component, Path};

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, FilesystemAccess, PreparedToolCall, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Value, json};

#[cfg(unix)]
use rustix::fd::{AsFd, OwnedFd};
#[cfg(unix)]
use rustix::fs::{FileType, Mode, OFlags};

/// Maximum number of file bytes returned by [`ReadFileTool`].
pub const MAX_READ_FILE_BYTES: usize = 8 * 1024;

/// Maximum number of UTF-8 bytes accepted in a requested path.
pub const MAX_READ_FILE_PATH_BYTES: usize = 4 * 1024;

/// Registered name of [`ReadFileTool`].
pub const READ_FILE_TOOL_NAME: &str = "read_file";

/// Stable category for failure to acquire a read-only workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadFileToolOpenErrorKind {
    /// Native file execution is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`ReadFileTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ReadFileToolOpenError {
    kind: ReadFileToolOpenErrorKind,
}

impl ReadFileToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> ReadFileToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: ReadFileToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for ReadFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadFileToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for ReadFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ReadFileToolOpenErrorKind::UnsupportedPlatform => {
                "native read_file is unsupported on this platform"
            }
            ReadFileToolOpenErrorKind::InvalidRoot => "native read_file workspace root is invalid",
            ReadFileToolOpenErrorKind::InvalidFileType => {
                "native read_file workspace root is not a directory"
            }
            ReadFileToolOpenErrorKind::Unavailable => {
                "native read_file workspace root is unavailable"
            }
        })
    }
}

impl Error for ReadFileToolOpenError {}

/// A read-only native tool confined to one explicitly opened workspace root.
///
/// Construction acquires the only ambient filesystem authority used by this
/// tool. Supported Unix implementations retain the opened directory descriptor;
/// later calls never reopen the workspace root by path.
pub struct ReadFileTool {
    #[cfg(unix)]
    root: OwnedFd,
    #[cfg(not(unix))]
    _unsupported: std::convert::Infallible,
}

impl ReadFileTool {
    #[cfg(unix)]
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
    pub fn open(root: &Path) -> Result<Self, ReadFileToolOpenError> {
        #[cfg(not(unix))]
        {
            let _ = root;
            Err(ReadFileToolOpenError::new(
                ReadFileToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(unix)]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(ReadFileToolOpenError::new(
                    ReadFileToolOpenErrorKind::InvalidRoot,
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
                .map_err(|_| ReadFileToolOpenError::new(ReadFileToolOpenErrorKind::Unavailable))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(ReadFileToolOpenError::new(
                    ReadFileToolOpenErrorKind::InvalidFileType,
                ));
            }

            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for ReadFileTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadFileTool")
            .finish_non_exhaustive()
    }
}

struct ReadFileArguments {
    path: String,
}

impl Tool for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: read_file_name(),
            description: "Read one UTF-8 file within the configured workspace".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative file path"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != read_file_name() {
            return Err(invalid_arguments());
        }
        let arguments = decode_arguments(call.arguments)?;
        let normalized = normalize_relative_path(&arguments.path)?;
        let prepared_arguments = json!({ "path": normalized });
        let path = prepared_arguments["path"]
            .as_str()
            .expect("prepared read_file path is a string")
            .to_owned();
        Ok(PreparedToolCall::new(
            Capability::Filesystem {
                access: FilesystemAccess::Read,
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

            #[cfg(not(unix))]
            {
                let _ = cancellation;
                Err(unsupported_platform())
            }

            #[cfg(unix)]
            {
                self.execute_unix(&normalized, &cancellation)
            }
        })
    }
}

fn decode_arguments(arguments: Value) -> Result<ReadFileArguments, ToolError> {
    let Value::Object(mut object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 1 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(path)) = object.remove("path") else {
        return Err(invalid_arguments());
    };
    Ok(ReadFileArguments { path })
}

#[cfg(unix)]
impl ReadFileTool {
    fn execute_unix(
        &self,
        normalized: &str,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        check_cancellation(cancellation)?;
        let mut directory: Option<OwnedFd> = None;
        let mut components = normalized.split('/').peekable();
        let file = loop {
            check_cancellation(cancellation)?;
            let component = components.next().ok_or_else(invalid_arguments)?;
            let directory_fd = directory
                .as_ref()
                .map_or_else(|| self.root.as_fd(), AsFd::as_fd);
            if components.peek().is_some() {
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
            } else {
                break rustix::fs::openat(
                    directory_fd,
                    component,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(map_path_open_error)?;
            }
        };

        let metadata = rustix::fs::fstat(&file).map_err(|_| unavailable(true))?;
        if !FileType::from_raw_mode(metadata.st_mode).is_file() {
            return Err(rejected_file_type());
        }
        if u64::try_from(metadata.st_size).is_ok_and(|size| size > MAX_READ_FILE_BYTES as u64) {
            return Err(too_large());
        }

        let mut bytes = vec![0_u8; MAX_READ_FILE_BYTES + 1];
        let mut length = 0;
        loop {
            check_cancellation(cancellation)?;
            match rustix::io::read(&file, &mut bytes[length..]) {
                Ok(0) => break,
                Ok(read) => {
                    check_cancellation(cancellation)?;
                    length += read;
                    if length > MAX_READ_FILE_BYTES {
                        return Err(too_large());
                    }
                }
                Err(error) if error == rustix::io::Errno::INTR => {}
                Err(_) => return Err(read_failed()),
            }
        }
        check_cancellation(cancellation)?;
        bytes.truncate(length);
        let content = String::from_utf8(bytes).map_err(|_| not_utf8())?;
        check_cancellation(cancellation)?;
        Ok(ToolOutput::success(json!({ "content": content })))
    }
}

fn read_file_name() -> ToolName {
    ToolName::new(READ_FILE_TOOL_NAME).expect("read_file is a valid tool name")
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.len() > MAX_READ_FILE_PATH_BYTES || path.chars().any(is_forbidden_path_character) {
        return Err(invalid_path());
    }

    let mut normalized = String::with_capacity(path.len());
    for component in Path::new(path).components() {
        let component = match component {
            Component::CurDir => continue,
            Component::Normal(component) => component
                .to_str()
                .expect("components of a UTF-8 path remain UTF-8"),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(invalid_path());
            }
        };
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() {
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

#[cfg(unix)]
fn map_root_open_error(error: rustix::io::Errno) -> ReadFileToolOpenError {
    let kind = if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        ReadFileToolOpenErrorKind::InvalidFileType
    } else {
        ReadFileToolOpenErrorKind::Unavailable
    };
    ReadFileToolOpenError::new(kind)
}

#[cfg(unix)]
fn map_path_open_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::NOENT {
        not_found()
    } else if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        rejected_file_type()
    } else if error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM {
        permission_denied()
    } else {
        unavailable(true)
    }
}

#[cfg(unix)]
fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(ToolError::new(
            ToolErrorKind::Cancelled,
            "read_file_cancelled",
            "read_file execution was cancelled",
            false,
        ))
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "read_file_invalid_arguments",
        "read_file arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "read_file_invalid_path",
        "read_file path is invalid",
        false,
    )
}

#[cfg(not(unix))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "read_file_unsupported_platform",
        "native read_file is unsupported on this platform",
        false,
    )
}

#[cfg(unix)]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "read_file_not_found",
        "requested file is unavailable",
        false,
    )
}

#[cfg(unix)]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "read_file_permission_denied",
        "requested file cannot be read",
        false,
    )
}

#[cfg(unix)]
fn rejected_file_type() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "read_file_path_rejected",
        "requested path is not a confined regular file",
        false,
    )
}

#[cfg(unix)]
fn unavailable(retryable: bool) -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "read_file_unavailable",
        "requested file is unavailable",
        retryable,
    )
}

#[cfg(unix)]
fn read_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "read_file_read_failed",
        "requested file could not be read",
        true,
    )
}

#[cfg(unix)]
fn too_large() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "read_file_too_large",
        "requested file exceeds the read limit",
        false,
    )
}

#[cfg(unix)]
fn not_utf8() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "read_file_not_utf8",
        "requested file is not valid UTF-8",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{MAX_READ_FILE_PATH_BYTES, normalize_relative_path};

    #[test]
    fn lexical_normalization_is_workspace_relative() {
        assert_eq!(
            normalize_relative_path("./src//./lib.rs").unwrap(),
            "src/lib.rs"
        );
    }

    #[test]
    fn lexical_normalization_rejects_unsafe_or_ambiguous_paths() {
        for path in [
            "",
            ".",
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
        assert!(normalize_relative_path(&"x".repeat(MAX_READ_FILE_PATH_BYTES + 1)).is_err());
    }
}
