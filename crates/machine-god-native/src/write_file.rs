use std::error::Error;
use std::fmt;
use std::io;
use std::path::Path;

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, FilesystemAccess, PreparedToolCall, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Value, json};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};

/// Maximum number of UTF-8 bytes accepted in a normalized target path.
pub const MAX_WRITE_FILE_PATH_BYTES: usize = 4 * 1024;
/// Maximum number of components accepted in a normalized target path.
pub const MAX_WRITE_FILE_PATH_COMPONENTS: usize = 256;
/// Maximum number of raw UTF-8 content bytes accepted.
pub const MAX_WRITE_FILE_CONTENT_BYTES: usize = 48 * 1024;
/// Maximum serialized byte size of accepted arguments.
pub const MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES: usize = 64 * 1024;
/// Maximum bytes supplied to one native write operation.
pub const MAX_WRITE_FILE_CHUNK_BYTES: usize = 8 * 1024;
/// Maximum exclusive temporary-name attempts made by one execution.
pub const MAX_WRITE_FILE_TEMP_ATTEMPTS: usize = 8;
/// Maximum serialized [`ToolOutput`] bytes produced by this tool.
pub const MAX_WRITE_FILE_SERIALIZED_RESULT_BYTES: usize = 16 * 1024;

// A native syscall may be interrupted repeatedly without making progress. Cap
// the total interrupted attempts in each write or durability phase so every
// operation remains bounded even when interrupts are interleaved with progress.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS: usize = 16;

/// Registered name of [`WriteFileTool`].
pub const WRITE_FILE_TOOL_NAME: &str = "write_file";

const WRITE_FILE_DESCRIPTION: &str = "Write one file within the configured workspace";
const PATH_DESCRIPTION: &str = "Workspace-relative file path";
const CONTENT_DESCRIPTION: &str = "UTF-8 content to write";

#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEMP_NAME_PREFIX: &str = ".machine-god-write-";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEMP_RANDOM_BYTES: usize = 16;

/// Stable category for failure to acquire a writable workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteFileToolOpenErrorKind {
    /// Native file mutation is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`WriteFileTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct WriteFileToolOpenError {
    kind: WriteFileToolOpenErrorKind,
}

impl WriteFileToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> WriteFileToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: WriteFileToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for WriteFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WriteFileToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for WriteFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            WriteFileToolOpenErrorKind::UnsupportedPlatform => {
                "native write_file is unsupported on this platform"
            }
            WriteFileToolOpenErrorKind::InvalidRoot => {
                "native write_file workspace root is invalid"
            }
            WriteFileToolOpenErrorKind::InvalidFileType => {
                "native write_file workspace root is not a directory"
            }
            WriteFileToolOpenErrorKind::Unavailable => {
                "native write_file workspace root is unavailable"
            }
        })
    }
}

impl Error for WriteFileToolOpenError {}

/// A native file writer confined to one explicitly opened workspace root.
///
/// Construction acquires the only ambient filesystem authority used by this
/// tool. Supported Linux and macOS implementations retain the opened directory
/// descriptor; later calls never reopen the workspace root by its injected path.
pub struct WriteFileTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl WriteFileTool {
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
    pub fn open(root: &Path) -> Result<Self, WriteFileToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(WriteFileToolOpenError::new(
                WriteFileToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(WriteFileToolOpenError::new(
                    WriteFileToolOpenErrorKind::InvalidRoot,
                ));
            }

            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor).map_err(|_| {
                WriteFileToolOpenError::new(WriteFileToolOpenErrorKind::Unavailable)
            })?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(WriteFileToolOpenError::new(
                    WriteFileToolOpenErrorKind::InvalidFileType,
                ));
            }

            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for WriteFileTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WriteFileTool")
            .finish_non_exhaustive()
    }
}

struct ValidatedArguments<'a> {
    requested_path: &'a str,
    path: String,
    content: &'a str,
}

impl Tool for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: write_file_name(),
            description: WRITE_FILE_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": PATH_DESCRIPTION
                    },
                    "content": {
                        "type": "string",
                        "description": CONTENT_DESCRIPTION
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != write_file_name() {
            return Err(invalid_arguments());
        }
        let arguments = validate_arguments(&call.arguments)?;
        let prepared_arguments = json!({
            "path": arguments.path,
            "content": arguments.content,
        });
        let path = prepared_arguments["path"]
            .as_str()
            .expect("prepared write_file path is a string")
            .to_owned();
        Ok(PreparedToolCall::new(
            Capability::Filesystem {
                access: FilesystemAccess::Write,
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
            let arguments = validate_arguments(&arguments)?;
            let normalized = arguments.path;
            if normalized != arguments.requested_path {
                return Err(invalid_arguments());
            }

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = cancellation;
                Err(unsupported_platform())
            }

            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.execute_supported(&normalized, arguments.content.as_bytes(), &cancellation)
            }
        })
    }
}

fn validate_arguments(arguments: &Value) -> Result<ValidatedArguments<'_>, ToolError> {
    let Value::Object(object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 2 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(path)) = object.get("path") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(content)) = object.get("content") else {
        return Err(invalid_arguments());
    };
    let normalized = normalize_relative_path(path)?;
    if content.len() > MAX_WRITE_FILE_CONTENT_BYTES {
        return Err(content_too_large());
    }
    if !serialized_value_fits(arguments, MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES) {
        return Err(invalid_arguments());
    }
    Ok(ValidatedArguments {
        requested_path: path,
        path: normalized,
        content,
    })
}

fn serialized_value_fits(value: &(impl serde::Serialize + ?Sized), limit: usize) -> bool {
    let mut writer = JsonByteCounter { written: 0, limit };
    serde_json::to_writer(&mut writer, value).is_ok()
}

struct JsonByteCounter {
    written: usize,
    limit: usize,
}

impl io::Write for JsonByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(written) = self.written.checked_add(bytes.len()) else {
            return Err(io::Error::other("serialized JSON byte count overflowed"));
        };
        if written > self.limit {
            return Err(io::Error::other("serialized JSON exceeded its byte limit"));
        }
        self.written = written;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_WRITE_FILE_PATH_BYTES
        || path.starts_with('/')
        || path.chars().any(is_forbidden_path_character)
    {
        return Err(invalid_path());
    }

    let mut normalized = String::with_capacity(path.len());
    let mut components = 0_usize;
    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return Err(invalid_path());
        }
        components = components.checked_add(1).ok_or_else(invalid_path)?;
        if components > MAX_WRITE_FILE_PATH_COMPONENTS {
            return Err(invalid_path());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() || normalized.len() > MAX_WRITE_FILE_PATH_BYTES {
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

fn write_file_name() -> ToolName {
    ToolName::new(WRITE_FILE_TOOL_NAME).expect("write_file is a valid tool name")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
enum WalkPhase {
    Initial,
    Revalidate,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum TargetSnapshot {
    Missing,
    Existing(rustix::fs::Stat),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ParentWalk<'a> {
    parent: OwnedFd,
    basename: &'a str,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct StagedFile<'a> {
    cleanup_parent: BorrowedFd<'a>,
    file: OwnedFd,
    name: String,
    identity: rustix::fs::Stat,
    published: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'a> StagedFile<'a> {
    fn new(cleanup_parent: BorrowedFd<'a>, file: OwnedFd, name: String) -> Result<Self, ToolError> {
        let identity = rustix::fs::fstat(&file).map_err(|_| write_failed())?;
        if !FileType::from_raw_mode(identity.st_mode).is_file() {
            return Err(write_failed());
        }
        Ok(Self {
            cleanup_parent,
            file,
            name,
            identity,
            published: false,
        })
    }

    fn mark_published(&mut self) {
        self.published = true;
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for StagedFile<'_> {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        let Ok(descriptor_metadata) = rustix::fs::fstat(&self.file) else {
            return;
        };
        let Ok(path_metadata) =
            rustix::fs::statat(self.cleanup_parent, &self.name, AtFlags::SYMLINK_NOFOLLOW)
        else {
            return;
        };
        if same_identity(&descriptor_metadata, &self.identity)
            && same_identity(&path_metadata, &self.identity)
            && FileType::from_raw_mode(path_metadata.st_mode).is_file()
        {
            let _ = rustix::fs::unlinkat(self.cleanup_parent, &self.name, AtFlags::empty());
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl WriteFileTool {
    fn execute_supported(
        &self,
        normalized: &str,
        content: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        self.execute_supported_with(
            normalized,
            content,
            cancellation,
            native_set_mode,
            write_content,
            sync_before_commit,
            |_, _| {},
            || {},
            native_publish_staged,
            native_sync_parent,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_supported_with<
        SetMode,
        WriteContent,
        SyncStaged,
        BeforeStagedRevalidation,
        BeforeRename,
        Publish,
        SyncParent,
    >(
        &self,
        normalized: &str,
        content: &[u8],
        cancellation: &CancellationToken,
        mut set_mode: SetMode,
        mut write: WriteContent,
        mut sync_staged: SyncStaged,
        mut before_staged_revalidation: BeforeStagedRevalidation,
        mut before_rename: BeforeRename,
        mut publish: Publish,
        mut sync_parent: SyncParent,
    ) -> Result<ToolOutput, ToolError>
    where
        SetMode: for<'fd> FnMut(BorrowedFd<'fd>, Mode) -> Result<(), rustix::io::Errno>,
        WriteContent:
            for<'fd> FnMut(BorrowedFd<'fd>, &[u8], &CancellationToken) -> Result<(), ToolError>,
        SyncStaged: for<'fd> FnMut(BorrowedFd<'fd>, &CancellationToken) -> Result<(), ToolError>,
        BeforeStagedRevalidation: for<'fd> FnMut(BorrowedFd<'fd>, &str),
        BeforeRename: FnMut(),
        Publish: for<'fd> FnMut(BorrowedFd<'fd>, &str, &str, bool) -> Result<(), rustix::io::Errno>,
        SyncParent: for<'fd> FnMut(BorrowedFd<'fd>) -> Result<(), rustix::io::Errno>,
    {
        check_cancellation(cancellation)?;
        let initial = self.walk_parent(normalized, cancellation, WalkPhase::Initial)?;
        let initial_parent_metadata =
            rustix::fs::fstat(&initial.parent).map_err(|_| unavailable(true))?;
        if !FileType::from_raw_mode(initial_parent_metadata.st_mode).is_dir() {
            return Err(rejected_path());
        }
        check_cancellation(cancellation)?;
        let initial_target = inspect_initial_target(initial.parent.as_fd(), initial.basename)?;
        let final_mode = match &initial_target {
            TargetSnapshot::Missing => Mode::from_raw_mode(0o644),
            TargetSnapshot::Existing(metadata) => Mode::from_raw_mode(metadata.st_mode & 0o777),
        };

        let (temp_name, temp_file) =
            create_staged_file(initial.parent.as_fd(), initial.basename, cancellation)?;
        let mut staged = StagedFile::new(initial.parent.as_fd(), temp_file, temp_name)?;

        check_cancellation(cancellation)?;
        let private_mode = Mode::from_raw_mode(0o600);
        set_mode(staged.file.as_fd(), private_mode).map_err(map_precommit_io_error)?;
        verify_staged_descriptor(&staged, 0, Some(private_mode))?;
        write(staged.file.as_fd(), content, cancellation)?;
        verify_staged_descriptor(&staged, content.len(), None)?;
        check_cancellation(cancellation)?;
        set_mode(staged.file.as_fd(), final_mode).map_err(map_precommit_io_error)?;
        check_cancellation(cancellation)?;
        sync_staged(staged.file.as_fd(), cancellation)?;
        verify_staged_descriptor(&staged, content.len(), Some(final_mode))?;
        check_cancellation(cancellation)?;

        let final_walk = self.walk_parent(normalized, cancellation, WalkPhase::Revalidate)?;
        let final_parent_metadata =
            rustix::fs::fstat(&final_walk.parent).map_err(|_| target_changed())?;
        revalidate_parent_identity(&initial_parent_metadata, &final_parent_metadata)?;
        revalidate_target(
            final_walk.parent.as_fd(),
            final_walk.basename,
            &initial_target,
        )?;
        check_cancellation(cancellation)?;
        before_staged_revalidation(final_walk.parent.as_fd(), &staged.name);
        revalidate_staged_path(
            final_walk.parent.as_fd(),
            &staged,
            content.len(),
            final_mode,
        )?;

        before_rename();
        check_cancellation(cancellation)?;
        let creating = matches!(initial_target, TargetSnapshot::Missing);
        publish(
            final_walk.parent.as_fd(),
            &staged.name,
            final_walk.basename,
            creating,
        )
        .map_err(|error| {
            if creating {
                map_create_rename_error(error)
            } else {
                map_replace_rename_error(error)
            }
        })?;
        staged.mark_published();

        let published_identity_matches = published_target_matches(
            final_walk.parent.as_fd(),
            final_walk.basename,
            &staged,
            content.len(),
            final_mode,
        );
        let directory_sync = sync_after_commit_with(|| sync_parent(final_walk.parent.as_fd()));
        finish_after_commit(published_identity_matches, directory_sync)?;
        let output = ToolOutput::success(json!({
            "path": normalized,
            "bytes_written": content.len(),
        }));
        debug_assert!(serialized_value_fits(
            &output,
            MAX_WRITE_FILE_SERIALIZED_RESULT_BYTES
        ));
        Ok(output)
    }

    fn walk_parent<'a>(
        &self,
        normalized: &'a str,
        cancellation: &CancellationToken,
        phase: WalkPhase,
    ) -> Result<ParentWalk<'a>, ToolError> {
        self.walk_parent_with(normalized, cancellation, phase, || {})
    }

    fn walk_parent_with<'a>(
        &self,
        normalized: &'a str,
        cancellation: &CancellationToken,
        phase: WalkPhase,
        mut after_parent_component_opened: impl FnMut(),
    ) -> Result<ParentWalk<'a>, ToolError> {
        check_cancellation(cancellation)?;
        let mut parent = rustix::fs::openat(
            self.root.as_fd(),
            ".",
            directory_open_flags(),
            Mode::empty(),
        )
        .map_err(|_| unavailable(true))?;
        check_cancellation(cancellation)?;
        ensure_root_is_linked(parent.as_fd())?;

        let mut components = normalized.split('/').peekable();
        loop {
            check_cancellation(cancellation)?;
            let component = components.next().ok_or_else(invalid_arguments)?;
            if components.peek().is_none() {
                return Ok(ParentWalk {
                    parent,
                    basename: component,
                });
            }
            parent = rustix::fs::openat(
                parent.as_fd(),
                component,
                directory_open_flags(),
                Mode::empty(),
            )
            .map_err(|error| map_parent_open_error(error, phase))?;
            check_cancellation(cancellation)?;
            let metadata =
                rustix::fs::fstat(&parent).map_err(|_| map_parent_revalidation_failure(phase))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(map_parent_rejected(phase));
            }
            after_parent_component_opened();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn native_set_mode(file: BorrowedFd<'_>, mode: Mode) -> Result<(), rustix::io::Errno> {
    rustix::fs::fchmod(file, mode)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn native_publish_staged(
    parent: BorrowedFd<'_>,
    staged_name: &str,
    basename: &str,
    creating: bool,
) -> Result<(), rustix::io::Errno> {
    if creating {
        rustix::fs::renameat_with(
            parent,
            staged_name,
            parent,
            basename,
            RenameFlags::NOREPLACE,
        )
    } else {
        rustix::fs::renameat(parent, staged_name, parent, basename)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn native_sync_parent(parent: BorrowedFd<'_>) -> Result<(), rustix::io::Errno> {
    rustix::fs::fsync(parent)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn inspect_initial_target(
    parent: BorrowedFd<'_>,
    basename: &str,
) -> Result<TargetSnapshot, ToolError> {
    match rustix::fs::statat(parent, basename, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) if FileType::from_raw_mode(metadata.st_mode).is_file() => {
            Ok(TargetSnapshot::Existing(metadata))
        }
        Ok(_) => Err(rejected_path()),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(TargetSnapshot::Missing),
        Err(error) if is_permission_error(error) => Err(permission_denied()),
        Err(error) if is_rejected_type_error(error) => Err(rejected_path()),
        Err(_) => Err(unavailable(true)),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_staged_file(
    parent: BorrowedFd<'_>,
    basename: &str,
    cancellation: &CancellationToken,
) -> Result<(String, OwnedFd), ToolError> {
    create_staged_file_with(parent, basename, cancellation, |_| random_temp_name())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_staged_file_with(
    parent: BorrowedFd<'_>,
    basename: &str,
    cancellation: &CancellationToken,
    mut next_name: impl FnMut(usize) -> Result<String, ToolError>,
) -> Result<(String, OwnedFd), ToolError> {
    for attempt in 0..MAX_WRITE_FILE_TEMP_ATTEMPTS {
        check_cancellation(cancellation)?;
        let name = next_name(attempt)?;
        if name == basename {
            continue;
        }
        match rustix::fs::openat(
            parent,
            &name,
            OFlags::WRONLY
                | OFlags::CREATE
                | OFlags::EXCL
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::from_raw_mode(0o600),
        ) {
            Ok(file) => return Ok((name, file)),
            Err(error) if error == rustix::io::Errno::EXIST => {}
            Err(error) if is_permission_error(error) => return Err(permission_denied()),
            Err(_) => return Err(unavailable(true)),
        }
    }
    Err(unavailable(true))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn random_temp_name() -> Result<String, ToolError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut random = [0_u8; TEMP_RANDOM_BYTES];
    getrandom::fill(&mut random).map_err(|_| unavailable(true))?;
    let mut name = String::with_capacity(TEMP_NAME_PREFIX.len() + TEMP_RANDOM_BYTES * 2);
    name.push_str(TEMP_NAME_PREFIX);
    for byte in random {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(name)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_content(
    file: BorrowedFd<'_>,
    content: &[u8],
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    write_content_with(content, cancellation, |chunk| {
        rustix::io::write(file, chunk)
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_content_with(
    content: &[u8],
    cancellation: &CancellationToken,
    mut write: impl FnMut(&[u8]) -> Result<usize, rustix::io::Errno>,
) -> Result<(), ToolError> {
    let mut offset = 0_usize;
    let mut interrupted_attempts = 0_usize;
    while offset < content.len() {
        check_cancellation(cancellation)?;
        let end = offset
            .saturating_add(MAX_WRITE_FILE_CHUNK_BYTES)
            .min(content.len());
        let chunk = &content[offset..end];
        match write(chunk) {
            Ok(0) => return Err(write_failed()),
            Ok(written) if written <= chunk.len() => {
                offset = offset.checked_add(written).ok_or_else(write_failed)?;
            }
            Ok(_) => return Err(write_failed()),
            Err(error) if error == rustix::io::Errno::INTR => {
                interrupted_attempts = interrupted_attempts
                    .checked_add(1)
                    .ok_or_else(write_failed)?;
                check_cancellation(cancellation)?;
                if interrupted_attempts >= MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS {
                    return Err(map_precommit_io_error(error));
                }
            }
            Err(error) => return Err(map_precommit_io_error(error)),
        }
    }
    check_cancellation(cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_staged_descriptor(
    staged: &StagedFile<'_>,
    content_length: usize,
    expected_mode: Option<Mode>,
) -> Result<(), ToolError> {
    let metadata = rustix::fs::fstat(&staged.file).map_err(|_| write_failed())?;
    if !same_identity(&metadata, &staged.identity)
        || !FileType::from_raw_mode(metadata.st_mode).is_file()
        || usize::try_from(metadata.st_size).ok() != Some(content_length)
        || expected_mode.is_some_and(|mode| metadata.st_mode & 0o777 != mode.as_raw_mode())
    {
        return Err(write_failed());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn revalidate_staged_path(
    parent: BorrowedFd<'_>,
    staged: &StagedFile<'_>,
    content_length: usize,
    expected_mode: Mode,
) -> Result<(), ToolError> {
    let path_metadata = rustix::fs::statat(parent, &staged.name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| write_failed())?;
    if !same_identity(&path_metadata, &staged.identity)
        || !FileType::from_raw_mode(path_metadata.st_mode).is_file()
        || usize::try_from(path_metadata.st_size).ok() != Some(content_length)
        || path_metadata.st_mode & 0o777 != expected_mode.as_raw_mode()
    {
        return Err(write_failed());
    }
    verify_staged_descriptor(staged, content_length, Some(expected_mode))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn revalidate_target(
    parent: BorrowedFd<'_>,
    basename: &str,
    initial: &TargetSnapshot,
) -> Result<(), ToolError> {
    let current = match rustix::fs::statat(parent, basename, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => Some(metadata),
        Err(error) if error == rustix::io::Errno::NOENT => None,
        Err(_) => return Err(target_changed()),
    };
    match (initial, current) {
        (TargetSnapshot::Missing, None) => Ok(()),
        (TargetSnapshot::Existing(expected), Some(actual))
            if FileType::from_raw_mode(actual.st_mode).is_file()
                && same_identity(expected, &actual)
                && expected.st_mode & 0o777 == actual.st_mode & 0o777 =>
        {
            Ok(())
        }
        _ => Err(target_changed()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn revalidate_parent_identity(
    initial: &rustix::fs::Stat,
    current: &rustix::fs::Stat,
) -> Result<(), ToolError> {
    if same_identity(initial, current) && FileType::from_raw_mode(current.st_mode).is_dir() {
        Ok(())
    } else {
        Err(target_changed())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn published_target_matches(
    parent: BorrowedFd<'_>,
    basename: &str,
    staged: &StagedFile<'_>,
    content_length: usize,
    expected_mode: Mode,
) -> bool {
    rustix::fs::statat(parent, basename, AtFlags::SYMLINK_NOFOLLOW).is_ok_and(|metadata| {
        same_identity(&metadata, &staged.identity)
            && FileType::from_raw_mode(metadata.st_mode).is_file()
            && usize::try_from(metadata.st_size).ok() == Some(content_length)
            && metadata.st_mode & 0o777 == expected_mode.as_raw_mode()
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_before_commit(
    file: BorrowedFd<'_>,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    sync_before_commit_with(cancellation, || rustix::fs::fsync(file))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_before_commit_with(
    cancellation: &CancellationToken,
    mut sync: impl FnMut() -> Result<(), rustix::io::Errno>,
) -> Result<(), ToolError> {
    for _ in 0..MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS {
        check_cancellation(cancellation)?;
        match sync() {
            Err(error) if error == rustix::io::Errno::INTR => {
                check_cancellation(cancellation)?;
            }
            Ok(()) => return Ok(()),
            Err(error) => return Err(map_precommit_io_error(error)),
        }
    }
    Err(map_precommit_io_error(rustix::io::Errno::INTR))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_after_commit_with(
    mut sync: impl FnMut() -> Result<(), rustix::io::Errno>,
) -> Result<(), rustix::io::Errno> {
    for _ in 0..MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS {
        match sync() {
            Err(error) if error == rustix::io::Errno::INTR => {}
            Ok(()) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Err(rustix::io::Errno::INTR)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn finish_after_commit(
    published_identity_matches: bool,
    directory_sync: Result<(), rustix::io::Errno>,
) -> Result<(), ToolError> {
    if published_identity_matches && directory_sync.is_ok() {
        Ok(())
    } else {
        Err(commit_ambiguous())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn same_identity(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> bool {
    left.st_dev == right.st_dev && left.st_ino == right.st_ino
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_root_is_linked(root: BorrowedFd<'_>) -> Result<(), ToolError> {
    #[cfg(target_os = "linux")]
    {
        if rustix::fs::fstat(root)
            .map_err(|_| unavailable(true))?
            .st_nlink
            == 0
        {
            return Err(unavailable(true));
        }
    }
    #[cfg(target_os = "macos")]
    ensure_macos_root_is_linked(root)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_macos_root_is_linked(root: BorrowedFd<'_>) -> Result<(), ToolError> {
    let root_metadata = rustix::fs::fstat(root).map_err(|_| unavailable(true))?;
    let root_path = rustix::fs::getpath(root).map_err(|_| unavailable(true))?;
    let root_path = root_path.as_bytes();
    if root_path == b"/" {
        return Ok(());
    }
    let name = root_path
        .rsplit(|byte| *byte == b'/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| unavailable(true))?;
    let name = std::ffi::CString::new(name).map_err(|_| unavailable(true))?;
    let parent = rustix::fs::openat(root, "..", directory_open_flags(), Mode::empty())
        .map_err(|_| unavailable(true))?;
    let linked = rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| unavailable(true))?;
    if !same_identity(&root_metadata, &linked) || !FileType::from_raw_mode(linked.st_mode).is_dir()
    {
        return Err(unavailable(true));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> WriteFileToolOpenError {
    let kind = if is_rejected_type_error(error) {
        WriteFileToolOpenErrorKind::InvalidFileType
    } else {
        WriteFileToolOpenErrorKind::Unavailable
    };
    WriteFileToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_parent_open_error(error: rustix::io::Errno, phase: WalkPhase) -> ToolError {
    match phase {
        WalkPhase::Revalidate => target_changed(),
        WalkPhase::Initial if error == rustix::io::Errno::NOENT => not_found(),
        WalkPhase::Initial if is_rejected_type_error(error) => rejected_path(),
        WalkPhase::Initial if is_permission_error(error) => permission_denied(),
        WalkPhase::Initial => unavailable(true),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_parent_revalidation_failure(phase: WalkPhase) -> ToolError {
    match phase {
        WalkPhase::Initial => unavailable(true),
        WalkPhase::Revalidate => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_parent_rejected(phase: WalkPhase) -> ToolError {
    match phase {
        WalkPhase::Initial => rejected_path(),
        WalkPhase::Revalidate => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_create_rename_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::EXIST {
        target_changed()
    } else if is_permission_error(error) {
        permission_denied()
    } else {
        write_failed()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_replace_rename_error(error: rustix::io::Errno) -> ToolError {
    if is_permission_error(error) {
        permission_denied()
    } else {
        write_failed()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_precommit_io_error(_error: rustix::io::Errno) -> ToolError {
    write_failed()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_permission_error(error: rustix::io::Errno) -> bool {
    error == rustix::io::Errno::ACCESS || error == rustix::io::Errno::PERM
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_rejected_type_error(error: rustix::io::Errno) -> bool {
    error == rustix::io::Errno::LOOP
        || error == rustix::io::Errno::NOTDIR
        || error == rustix::io::Errno::ISDIR
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "write_file_invalid_arguments",
        "write_file arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "write_file_invalid_path",
        "write_file path is invalid",
        false,
    )
}

fn content_too_large() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "write_file_content_too_large",
        "write_file content exceeds the supported size limit",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "write_file_not_found",
        "requested parent directory is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "write_file_permission_denied",
        "requested file cannot be written",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rejected_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "write_file_path_rejected",
        "requested path is not a confined regular file target",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable(retryable: bool) -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "write_file_unavailable",
        "requested file is unavailable",
        retryable,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn target_changed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "write_file_target_changed",
        "requested file changed before commit",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "write_file_write_failed",
        "requested file could not be written",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn commit_ambiguous() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "write_file_commit_ambiguous",
        "requested file commit status is uncertain",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "write_file_cancelled",
        "write_file execution was cancelled",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "write_file_unsupported_platform",
        "native write_file is unsupported on this platform",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_normalization_is_bounded_and_confined() {
        assert_eq!(
            normalize_relative_path("./src//nested/./file.txt").unwrap(),
            "src/nested/file.txt"
        );
        let exact_components = std::iter::repeat_n("a", MAX_WRITE_FILE_PATH_COMPONENTS)
            .collect::<Vec<_>>()
            .join("/");
        assert_eq!(
            normalize_relative_path(&exact_components).unwrap(),
            exact_components
        );
        let too_many_components = format!("{exact_components}/a");
        for path in [
            "",
            ".",
            "..",
            "src/../secret",
            "/absolute",
            "nul\0byte",
            "line\u{2028}separator",
            &too_many_components,
        ] {
            assert_eq!(
                normalize_relative_path(path)
                    .expect_err("path must be rejected")
                    .code,
                "write_file_invalid_path"
            );
        }
    }

    #[test]
    fn argument_limit_precedence_and_bounded_serialization_are_exact() {
        let malformed = json!({ "path": "ok", "content": "ok", "extra": false });
        assert_eq!(
            validate_arguments(&malformed)
                .err()
                .expect("shape must be rejected")
                .code,
            "write_file_invalid_arguments"
        );

        let invalid_path_with_large_content = json!({
            "path": "../outside",
            "content": "x".repeat(MAX_WRITE_FILE_CONTENT_BYTES + 1),
        });
        assert_eq!(
            validate_arguments(&invalid_path_with_large_content)
                .err()
                .expect("path must be rejected")
                .code,
            "write_file_invalid_path"
        );

        let oversized_content = json!({
            "path": "file.txt",
            "content": "x".repeat(MAX_WRITE_FILE_CONTENT_BYTES + 1),
        });
        assert_eq!(
            validate_arguments(&oversized_content)
                .err()
                .expect("content must be rejected")
                .code,
            "write_file_content_too_large"
        );

        let escape_heavy = json!({
            "path": "file.txt",
            "content": "\0".repeat(MAX_WRITE_FILE_CONTENT_BYTES),
        });
        assert_eq!(
            validate_arguments(&escape_heavy)
                .err()
                .expect("serialized input must be rejected")
                .code,
            "write_file_invalid_arguments"
        );

        let three_bytes = json!("x");
        assert!(serialized_value_fits(&three_bytes, 3));
        assert!(!serialized_value_fits(&three_bytes, 2));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    mod supported {
        use std::cell::{Cell, RefCell};
        use std::fmt::Write as _;
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;
        use std::path::{Path, PathBuf};

        use super::*;

        struct TempDirectory(PathBuf);

        impl TempDirectory {
            fn new(label: &str) -> Self {
                for _ in 0..MAX_WRITE_FILE_TEMP_ATTEMPTS {
                    let mut random = [0_u8; 16];
                    getrandom::fill(&mut random).expect("test temporary-name randomness");
                    let mut suffix = String::with_capacity(random.len() * 2);
                    for byte in random {
                        write!(&mut suffix, "{byte:02x}").expect("write temporary suffix");
                    }
                    let path = std::env::temp_dir().join(format!(
                        "machine-god-write-file-{label}-{}-{suffix}",
                        std::process::id()
                    ));
                    match fs::create_dir(&path) {
                        Ok(()) => return Self(path),
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(error) => panic!("create temporary directory: {error}"),
                    }
                }
                panic!("allocate temporary directory");
            }

            fn path(&self) -> &Path {
                &self.0
            }

            fn descriptor(&self) -> OwnedFd {
                rustix::fs::open(self.path(), directory_open_flags(), Mode::empty())
                    .expect("open temporary directory")
            }
        }

        impl Drop for TempDirectory {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn assert_no_staged_files(root: &Path) {
            let staged = fs::read_dir(root)
                .expect("read temporary workspace")
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .filter(|name| name.to_string_lossy().starts_with(TEMP_NAME_PREFIX))
                .collect::<Vec<_>>();
            assert!(staged.is_empty(), "staged files were retained: {staged:?}");
        }

        #[test]
        fn device_target_is_rejected_before_staging() {
            let device_directory = rustix::fs::open("/dev", directory_open_flags(), Mode::empty())
                .expect("open the standard Unix device directory");
            let error = inspect_initial_target(device_directory.as_fd(), "null")
                .err()
                .expect("the null device must not be accepted as a regular-file target");
            assert_eq!(error.code, "write_file_path_rejected");
            assert!(!error.retryable);
        }

        #[test]
        fn pipeline_fchmod_failures_clean_each_precommit_stage() {
            for failed_call in 0..=1 {
                let label = format!("chmod-failure-{failed_call}");
                let temporary = TempDirectory::new(&label);
                let tool = WriteFileTool::open(temporary.path()).unwrap();
                let mode_calls = Cell::new(0_usize);
                let error = tool
                    .execute_supported_with(
                        "target.txt",
                        b"new content",
                        &CancellationToken::new(),
                        |file, mode| {
                            let call = mode_calls.get();
                            mode_calls.set(call + 1);
                            if call == failed_call {
                                Err(rustix::io::Errno::IO)
                            } else {
                                native_set_mode(file, mode)
                            }
                        },
                        write_content,
                        sync_before_commit,
                        |_, _| {},
                        || {},
                        native_publish_staged,
                        native_sync_parent,
                    )
                    .unwrap_err();
                assert_eq!(mode_calls.get(), failed_call + 1);
                assert_eq!(error.code, "write_file_write_failed");
                assert!(error.retryable);
                assert!(!temporary.path().join("target.txt").exists());
                assert_no_staged_files(temporary.path());
            }
        }

        #[test]
        fn pipeline_write_interrupt_exhaustion_preserves_target_and_cleans_stage() {
            let temporary = TempDirectory::new("write-failure");
            let target_path = temporary.path().join("target.txt");
            fs::write(&target_path, b"old content").unwrap();
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let write_calls = Cell::new(0_usize);
            let error = tool
                .execute_supported_with(
                    "target.txt",
                    b"replacement",
                    &CancellationToken::new(),
                    native_set_mode,
                    |_, content, cancellation| {
                        write_content_with(content, cancellation, |_| {
                            write_calls.set(write_calls.get() + 1);
                            Err(rustix::io::Errno::INTR)
                        })
                    },
                    sync_before_commit,
                    |_, _| {},
                    || {},
                    native_publish_staged,
                    native_sync_parent,
                )
                .unwrap_err();
            assert_eq!(
                write_calls.get(),
                MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS
            );
            assert_eq!(error.code, "write_file_write_failed");
            assert!(error.retryable);
            assert_eq!(fs::read(&target_path).unwrap(), b"old content");
            assert_no_staged_files(temporary.path());
        }

        #[test]
        fn pipeline_staged_sync_interrupt_exhaustion_preserves_target_and_cleans_stage() {
            let temporary = TempDirectory::new("staged-sync-failure");
            let target_path = temporary.path().join("target.txt");
            fs::write(&target_path, b"old content").unwrap();
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let sync_calls = Cell::new(0_usize);
            let error = tool
                .execute_supported_with(
                    "target.txt",
                    b"replacement",
                    &CancellationToken::new(),
                    native_set_mode,
                    write_content,
                    |_, cancellation| {
                        sync_before_commit_with(cancellation, || {
                            sync_calls.set(sync_calls.get() + 1);
                            Err(rustix::io::Errno::INTR)
                        })
                    },
                    |_, _| {},
                    || {},
                    native_publish_staged,
                    native_sync_parent,
                )
                .unwrap_err();
            assert_eq!(
                sync_calls.get(),
                MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS
            );
            assert_eq!(error.code, "write_file_write_failed");
            assert!(error.retryable);
            assert_eq!(fs::read(&target_path).unwrap(), b"old content");
            assert_no_staged_files(temporary.path());
        }

        #[test]
        fn pipeline_create_and_replace_rename_failures_preserve_targets() {
            let temporary = TempDirectory::new("rename-failures");
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let create_calls = Cell::new(0_usize);
            let create_error = tool
                .execute_supported_with(
                    "new.txt",
                    b"new content",
                    &CancellationToken::new(),
                    native_set_mode,
                    write_content,
                    sync_before_commit,
                    |_, _| {},
                    || {},
                    |_, _, _, creating| {
                        assert!(creating);
                        create_calls.set(create_calls.get() + 1);
                        Err(rustix::io::Errno::EXIST)
                    },
                    native_sync_parent,
                )
                .unwrap_err();
            assert_eq!(create_calls.get(), 1);
            assert_eq!(create_error.code, "write_file_target_changed");
            assert!(create_error.retryable);
            assert!(!temporary.path().join("new.txt").exists());
            assert_no_staged_files(temporary.path());

            let existing_path = temporary.path().join("existing.txt");
            fs::write(&existing_path, b"old content").unwrap();
            let replace_calls = Cell::new(0_usize);
            let replace_error = tool
                .execute_supported_with(
                    "existing.txt",
                    b"replacement",
                    &CancellationToken::new(),
                    native_set_mode,
                    write_content,
                    sync_before_commit,
                    |_, _| {},
                    || {},
                    |_, _, _, creating| {
                        assert!(!creating);
                        replace_calls.set(replace_calls.get() + 1);
                        Err(rustix::io::Errno::ACCESS)
                    },
                    native_sync_parent,
                )
                .unwrap_err();
            assert_eq!(replace_calls.get(), 1);
            assert_eq!(replace_error.code, "write_file_permission_denied");
            assert!(!replace_error.retryable);
            assert_eq!(fs::read(&existing_path).unwrap(), b"old content");
            assert_no_staged_files(temporary.path());
        }

        #[test]
        fn pipeline_rejects_a_swapped_staged_name_without_deleting_the_intruder() {
            let temporary = TempDirectory::new("staged-name-swap");
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let displaced_name = ".displaced-machine-god-stage";
            let root = temporary.path().to_owned();
            let error = tool
                .execute_supported_with(
                    "target.txt",
                    b"owned content",
                    &CancellationToken::new(),
                    native_set_mode,
                    write_content,
                    sync_before_commit,
                    |parent, staged_name| {
                        rustix::fs::renameat(parent, staged_name, parent, displaced_name)
                            .expect("displace the owned staged name");
                        fs::write(root.join(staged_name), b"intruder")
                            .expect("replace the staged name");
                    },
                    || {},
                    native_publish_staged,
                    native_sync_parent,
                )
                .unwrap_err();
            assert_eq!(error.code, "write_file_write_failed");
            assert!(error.retryable);
            assert!(!temporary.path().join("target.txt").exists());
            assert_eq!(
                fs::read(temporary.path().join(displaced_name)).unwrap(),
                b"owned content"
            );
            let intruder = fs::read_dir(temporary.path())
                .unwrap()
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(TEMP_NAME_PREFIX)
                })
                .expect("the identity-checked cleanup must preserve the intruder");
            assert_eq!(fs::read(intruder.path()).unwrap(), b"intruder");
        }

        #[test]
        fn pipeline_cancellation_immediately_before_rename_preserves_the_target() {
            let temporary = TempDirectory::new("pre-rename-cancel");
            let target_path = temporary.path().join("target.txt");
            fs::write(&target_path, b"old content").unwrap();
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let cancellation = CancellationToken::new();
            let cancellation_before_rename = cancellation.clone();
            let publish_calls = Cell::new(0_usize);
            let error = tool
                .execute_supported_with(
                    "target.txt",
                    b"replacement",
                    &cancellation,
                    native_set_mode,
                    write_content,
                    sync_before_commit,
                    |_, _| {},
                    || {
                        cancellation_before_rename.cancel();
                    },
                    |parent, staged_name, basename, creating| {
                        publish_calls.set(publish_calls.get() + 1);
                        native_publish_staged(parent, staged_name, basename, creating)
                    },
                    native_sync_parent,
                )
                .unwrap_err();
            assert_eq!(error.code, "write_file_cancelled");
            assert_eq!(publish_calls.get(), 0);
            assert_eq!(fs::read(&target_path).unwrap(), b"old content");
            assert_no_staged_files(temporary.path());
        }

        #[test]
        fn pipeline_cancellation_after_staged_verification_cleans_without_publication() {
            let temporary = TempDirectory::new("post-sync-cancel");
            let target_path = temporary.path().join("target.txt");
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let cancellation = CancellationToken::new();
            let cancellation_after_verification = cancellation.clone();
            let staged_name = RefCell::new(None);
            let publish_calls = Cell::new(0_usize);
            let error = tool
                .execute_supported_with(
                    "target.txt",
                    b"fully staged content",
                    &cancellation,
                    native_set_mode,
                    write_content,
                    sync_before_commit,
                    |parent, name| {
                        let metadata = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
                            .expect("the synced staged file remains linked before verification");
                        assert!(FileType::from_raw_mode(metadata.st_mode).is_file());
                        assert_eq!(
                            usize::try_from(metadata.st_size).unwrap(),
                            b"fully staged content".len()
                        );
                        assert_eq!(metadata.st_mode & 0o777, 0o644);
                        *staged_name.borrow_mut() = Some(name.to_owned());
                    },
                    || {
                        cancellation_after_verification.cancel();
                    },
                    |parent, staged_name, basename, creating| {
                        publish_calls.set(publish_calls.get() + 1);
                        native_publish_staged(parent, staged_name, basename, creating)
                    },
                    native_sync_parent,
                )
                .unwrap_err();
            assert_eq!(error.code, "write_file_cancelled");
            assert!(!error.retryable);
            assert_eq!(publish_calls.get(), 0);
            assert!(!target_path.exists());
            let staged_name = staged_name
                .into_inner()
                .expect("the post-sync verification seam must run");
            assert!(!temporary.path().join(staged_name).exists());
            assert_no_staged_files(temporary.path());
        }

        #[test]
        fn pipeline_native_create_preserves_a_target_raced_in_before_rename() {
            let temporary = TempDirectory::new("create-race");
            let target_path = temporary.path().join("target.txt");
            let raced_target = target_path.clone();
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let before_rename_calls = Cell::new(0_usize);
            let error = tool
                .execute_supported_with(
                    "target.txt",
                    b"owned content",
                    &CancellationToken::new(),
                    native_set_mode,
                    write_content,
                    sync_before_commit,
                    |_, _| {},
                    || {
                        before_rename_calls.set(before_rename_calls.get() + 1);
                        fs::write(&raced_target, b"raced content")
                            .expect("race in a target after final validation");
                    },
                    native_publish_staged,
                    native_sync_parent,
                )
                .unwrap_err();
            assert_eq!(before_rename_calls.get(), 1);
            assert_eq!(error.code, "write_file_target_changed");
            assert!(error.retryable);
            assert_eq!(fs::read(&target_path).unwrap(), b"raced content");
            assert_no_staged_files(temporary.path());
        }

        #[test]
        fn pipeline_native_replace_can_replace_a_target_raced_in_before_rename() {
            let temporary = TempDirectory::new("replace-race");
            let target_path = temporary.path().join("target.txt");
            let displaced_path = temporary.path().join("displaced.txt");
            fs::write(&target_path, b"initial content").unwrap();
            let raced_target = target_path.clone();
            let raced_displaced = displaced_path.clone();
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let before_rename_calls = Cell::new(0_usize);
            let output = tool
                .execute_supported_with(
                    "target.txt",
                    b"owned replacement",
                    &CancellationToken::new(),
                    native_set_mode,
                    write_content,
                    sync_before_commit,
                    |_, _| {},
                    || {
                        before_rename_calls.set(before_rename_calls.get() + 1);
                        fs::rename(&raced_target, &raced_displaced)
                            .expect("displace the validated target");
                        fs::write(&raced_target, b"raced replacement")
                            .expect("race in a replacement after final validation");
                    },
                    native_publish_staged,
                    native_sync_parent,
                )
                .expect("ordinary replacement rename may replace the raced target");
            assert_eq!(before_rename_calls.get(), 1);
            assert!(!output.is_error);
            assert_eq!(fs::read(&target_path).unwrap(), b"owned replacement");
            assert_eq!(fs::read(&displaced_path).unwrap(), b"initial content");
            assert_no_staged_files(temporary.path());
        }

        #[test]
        fn pipeline_native_publish_can_land_in_a_retained_moved_parent() {
            let temporary = TempDirectory::new("moved-parent-race");
            let workspace = temporary.path().join("workspace");
            fs::create_dir(&workspace).unwrap();
            let original_parent = workspace.join("nested");
            let moved_parent = temporary.path().join("outside-workspace");
            fs::create_dir(&original_parent).unwrap();
            let raced_original_parent = original_parent.clone();
            let raced_moved_parent = moved_parent.clone();
            let tool = WriteFileTool::open(&workspace).unwrap();
            let before_rename_calls = Cell::new(0_usize);
            let output = tool
                .execute_supported_with(
                    "nested/target.txt",
                    b"retained-parent content",
                    &CancellationToken::new(),
                    native_set_mode,
                    write_content,
                    sync_before_commit,
                    |_, _| {},
                    || {
                        before_rename_calls.set(before_rename_calls.get() + 1);
                        fs::rename(&raced_original_parent, &raced_moved_parent)
                            .expect("move the validated parent");
                        fs::create_dir(&raced_original_parent)
                            .expect("replace the workspace-visible parent path");
                    },
                    native_publish_staged,
                    native_sync_parent,
                )
                .expect("descriptor-relative publication retains the parent moved outside root");
            assert_eq!(before_rename_calls.get(), 1);
            assert!(!output.is_error);
            assert!(!original_parent.join("target.txt").exists());
            assert_eq!(
                fs::read(moved_parent.join("target.txt")).unwrap(),
                b"retained-parent content"
            );
            assert_no_staged_files(&original_parent);
            assert_no_staged_files(&moved_parent);
        }

        #[test]
        fn pipeline_parent_sync_failure_is_ambiguous_after_a_real_rename() {
            let temporary = TempDirectory::new("post-rename-sync-failure");
            let target_path = temporary.path().join("target.txt");
            fs::write(&target_path, b"old content").unwrap();
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let sync_calls = Cell::new(0_usize);
            let error = tool
                .execute_supported_with(
                    "target.txt",
                    b"committed content",
                    &CancellationToken::new(),
                    native_set_mode,
                    write_content,
                    sync_before_commit,
                    |_, _| {},
                    || {},
                    native_publish_staged,
                    |_| {
                        sync_calls.set(sync_calls.get() + 1);
                        Err(rustix::io::Errno::INTR)
                    },
                )
                .unwrap_err();
            assert_eq!(
                sync_calls.get(),
                MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS
            );
            assert_eq!(error.code, "write_file_commit_ambiguous");
            assert!(!error.retryable);
            assert_eq!(fs::read(&target_path).unwrap(), b"committed content");
            assert_no_staged_files(temporary.path());
        }

        #[test]
        fn eight_temp_collisions_are_preserved_and_fail_closed() {
            let temporary = TempDirectory::new("collisions");
            let collision_name = ".machine-god-write-collision";
            let collision_path = temporary.path().join(collision_name);
            fs::write(&collision_path, b"foreign").unwrap();
            let parent = temporary.descriptor();
            let attempts = Cell::new(0_usize);
            let error = create_staged_file_with(
                parent.as_fd(),
                "target.txt",
                &CancellationToken::new(),
                |attempt| {
                    assert_eq!(attempt, attempts.get());
                    attempts.set(attempts.get() + 1);
                    Ok(collision_name.to_owned())
                },
            )
            .unwrap_err();
            assert_eq!(error.code, "write_file_unavailable");
            assert_eq!(attempts.get(), MAX_WRITE_FILE_TEMP_ATTEMPTS);
            assert_eq!(fs::read(&collision_path).unwrap(), b"foreign");
        }

        #[test]
        fn cancellation_between_temp_attempts_preserves_collision() {
            let temporary = TempDirectory::new("collision-cancel");
            let collision_name = ".machine-god-write-collision";
            let collision_path = temporary.path().join(collision_name);
            fs::write(&collision_path, b"foreign").unwrap();
            let parent = temporary.descriptor();
            let cancellation = CancellationToken::new();
            let cancellation_for_name = cancellation.clone();
            let error =
                create_staged_file_with(parent.as_fd(), "target.txt", &cancellation, move |_| {
                    cancellation_for_name.cancel();
                    Ok(collision_name.to_owned())
                })
                .unwrap_err();
            assert_eq!(error.code, "write_file_cancelled");
            assert_eq!(fs::read(&collision_path).unwrap(), b"foreign");
        }

        #[test]
        fn cleanup_does_not_unlink_a_swapped_staged_name() {
            let temporary = TempDirectory::new("cleanup-swap");
            let parent = temporary.descriptor();
            let owned_name = ".machine-god-write-owned";
            let moved_name = ".machine-god-write-moved";
            let file = rustix::fs::openat(
                parent.as_fd(),
                owned_name,
                OFlags::WRONLY
                    | OFlags::CREATE
                    | OFlags::EXCL
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC
                    | OFlags::NONBLOCK,
                Mode::from_raw_mode(0o600),
            )
            .unwrap();
            let staged = StagedFile::new(parent.as_fd(), file, owned_name.to_owned()).unwrap();
            rustix::fs::renameat(parent.as_fd(), owned_name, parent.as_fd(), moved_name).unwrap();
            fs::write(temporary.path().join(owned_name), b"sentinel").unwrap();
            drop(staged);
            assert_eq!(
                fs::read(temporary.path().join(owned_name)).unwrap(),
                b"sentinel"
            );
        }

        #[test]
        fn target_identity_replacement_with_the_same_mode_fails_revalidation() {
            let temporary = TempDirectory::new("target-identity-change");
            let target_path = temporary.path().join("target.txt");
            fs::write(&target_path, b"old").unwrap();
            fs::set_permissions(&target_path, fs::Permissions::from_mode(0o640)).unwrap();
            let parent = temporary.descriptor();
            let initial = inspect_initial_target(parent.as_fd(), "target.txt").unwrap();
            revalidate_target(parent.as_fd(), "target.txt", &initial).unwrap();

            let displaced_path = temporary.path().join("displaced.txt");
            fs::rename(&target_path, &displaced_path).unwrap();
            fs::write(&target_path, b"replacement").unwrap();
            fs::set_permissions(&target_path, fs::Permissions::from_mode(0o640)).unwrap();
            let replacement =
                rustix::fs::statat(parent.as_fd(), "target.txt", AtFlags::SYMLINK_NOFOLLOW)
                    .unwrap();
            let TargetSnapshot::Existing(initial_metadata) = &initial else {
                panic!("target must initially exist");
            };
            assert!(!same_identity(initial_metadata, &replacement));
            assert_eq!(
                initial_metadata.st_mode & 0o777,
                replacement.st_mode & 0o777
            );
            assert_eq!(
                revalidate_target(parent.as_fd(), "target.txt", &initial)
                    .unwrap_err()
                    .code,
                "write_file_target_changed"
            );
            assert_eq!(fs::read(&target_path).unwrap(), b"replacement");
            assert_eq!(fs::read(&displaced_path).unwrap(), b"old");
        }

        #[test]
        fn final_parent_replacement_after_move_is_target_changed() {
            let temporary = TempDirectory::new("parent-identity-change");
            let original_path = temporary.path().join("nested");
            fs::create_dir(&original_path).unwrap();
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let cancellation = CancellationToken::new();
            let initial = tool
                .walk_parent("nested/target.txt", &cancellation, WalkPhase::Initial)
                .unwrap();
            let initial_metadata = rustix::fs::fstat(&initial.parent).unwrap();
            revalidate_parent_identity(&initial_metadata, &initial_metadata).unwrap();

            let moved_path = temporary.path().join("moved");
            fs::rename(&original_path, &moved_path).unwrap();
            fs::create_dir(&original_path).unwrap();
            let current = tool
                .walk_parent("nested/target.txt", &cancellation, WalkPhase::Revalidate)
                .unwrap();
            let current_metadata = rustix::fs::fstat(&current.parent).unwrap();
            assert!(!same_identity(&initial_metadata, &current_metadata));
            let error = revalidate_parent_identity(&initial_metadata, &current_metadata)
                .expect_err("moved/replaced final parent must fail revalidation");
            assert_eq!(error.code, "write_file_target_changed");
            assert!(error.retryable);

            for classified in [
                map_parent_open_error(rustix::io::Errno::NOENT, WalkPhase::Revalidate),
                map_parent_revalidation_failure(WalkPhase::Revalidate),
                map_parent_rejected(WalkPhase::Revalidate),
            ] {
                assert_eq!(classified.code, "write_file_target_changed");
            }
        }

        #[test]
        fn traversal_cancellation_after_an_opened_parent_component_stops_the_walk() {
            let temporary = TempDirectory::new("walk-cancel");
            fs::create_dir(temporary.path().join("nested")).unwrap();
            fs::create_dir(temporary.path().join("nested/deeper")).unwrap();
            let tool = WriteFileTool::open(temporary.path()).unwrap();
            let cancellation = CancellationToken::new();
            let cancellation_after_open = cancellation.clone();
            let opened_components = Cell::new(0_usize);
            let error = tool
                .walk_parent_with(
                    "nested/deeper/target.txt",
                    &cancellation,
                    WalkPhase::Initial,
                    || {
                        opened_components.set(opened_components.get() + 1);
                        cancellation_after_open.cancel();
                    },
                )
                .err()
                .expect("cancellation after opening a parent must stop traversal");
            assert_eq!(opened_components.get(), 1);
            assert_eq!(error.code, "write_file_cancelled");
            assert!(!error.retryable);
        }

        #[test]
        fn partial_interrupted_writes_are_bounded_and_complete() {
            let content = vec![b'x'; MAX_WRITE_FILE_CHUNK_BYTES * 2 + 37];
            let mut observed = Vec::new();
            let mut requested = Vec::new();
            let mut calls = 0_usize;
            write_content_with(&content, &CancellationToken::new(), |chunk| {
                calls += 1;
                requested.push(chunk.len());
                if calls == 1 {
                    return Err(rustix::io::Errno::INTR);
                }
                let written = chunk.len().min(997);
                observed.extend_from_slice(&chunk[..written]);
                Ok(written)
            })
            .unwrap();
            assert_eq!(observed, content);
            assert!(
                requested
                    .iter()
                    .all(|requested| *requested <= MAX_WRITE_FILE_CHUNK_BYTES)
            );
            assert_eq!(requested[0], MAX_WRITE_FILE_CHUNK_BYTES);
            assert_eq!(requested[1], MAX_WRITE_FILE_CHUNK_BYTES);
        }

        #[test]
        fn zero_and_failed_writes_fail_without_an_unbounded_retry() {
            let zero_calls = Cell::new(0_usize);
            let zero_error = write_content_with(b"content", &CancellationToken::new(), |_| {
                zero_calls.set(zero_calls.get() + 1);
                Ok(0)
            })
            .unwrap_err();
            assert_eq!(zero_calls.get(), 1);
            assert_eq!(zero_error.code, "write_file_write_failed");

            let error_calls = Cell::new(0_usize);
            let io_error = write_content_with(b"content", &CancellationToken::new(), |_| {
                error_calls.set(error_calls.get() + 1);
                Err(rustix::io::Errno::IO)
            })
            .unwrap_err();
            assert_eq!(error_calls.get(), 1);
            assert_eq!(io_error.code, "write_file_write_failed");
            assert!(io_error.retryable);

            let overreported = write_content_with(b"content", &CancellationToken::new(), |chunk| {
                Ok(chunk.len() + 1)
            })
            .unwrap_err();
            assert_eq!(overreported.code, "write_file_write_failed");

            let interrupted_calls = Cell::new(0_usize);
            let interrupted = write_content_with(b"content", &CancellationToken::new(), |_| {
                interrupted_calls.set(interrupted_calls.get() + 1);
                Err(rustix::io::Errno::INTR)
            })
            .unwrap_err();
            assert_eq!(
                interrupted_calls.get(),
                MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS
            );
            assert_eq!(interrupted.code, "write_file_write_failed");
            assert!(interrupted.retryable);

            let interleaved_calls = Cell::new(0_usize);
            let interleaved_content = vec![b'x'; MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS + 1];
            let interleaved =
                write_content_with(&interleaved_content, &CancellationToken::new(), |_| {
                    let call = interleaved_calls.get();
                    interleaved_calls.set(call + 1);
                    if call.is_multiple_of(2) {
                        Ok(1)
                    } else {
                        Err(rustix::io::Errno::INTR)
                    }
                })
                .unwrap_err();
            assert_eq!(
                interleaved_calls.get(),
                MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS * 2
            );
            assert_eq!(interleaved.code, "write_file_write_failed");
            assert!(interleaved.retryable);
        }

        #[test]
        fn write_cancellation_is_checked_between_partial_syscalls() {
            let cancellation = CancellationToken::new();
            let cancellation_after_write = cancellation.clone();
            let calls = Cell::new(0_usize);
            let error = write_content_with(b"content", &cancellation, |chunk| {
                calls.set(calls.get() + 1);
                cancellation_after_write.cancel();
                Ok(chunk.len().min(1))
            })
            .unwrap_err();
            assert_eq!(calls.get(), 1);
            assert_eq!(error.code, "write_file_cancelled");

            let final_interrupt_cancellation = CancellationToken::new();
            let cancel_on_final_interrupt = final_interrupt_cancellation.clone();
            let interrupted_calls = Cell::new(0_usize);
            let error = write_content_with(b"content", &final_interrupt_cancellation, |_| {
                let call = interrupted_calls.get() + 1;
                interrupted_calls.set(call);
                if call == MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS {
                    cancel_on_final_interrupt.cancel();
                }
                Err(rustix::io::Errno::INTR)
            })
            .unwrap_err();
            assert_eq!(
                interrupted_calls.get(),
                MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS
            );
            assert_eq!(error.code, "write_file_cancelled");
        }

        #[test]
        fn precommit_file_sync_retries_interrupts_and_checks_cancellation() {
            let attempts = Cell::new(0_usize);
            sync_before_commit_with(&CancellationToken::new(), || {
                let attempt = attempts.get();
                attempts.set(attempt + 1);
                if attempt == 0 {
                    Err(rustix::io::Errno::INTR)
                } else {
                    Ok(())
                }
            })
            .unwrap();
            assert_eq!(attempts.get(), 2);

            let error_attempts = Cell::new(0_usize);
            let error = sync_before_commit_with(&CancellationToken::new(), || {
                error_attempts.set(error_attempts.get() + 1);
                Err(rustix::io::Errno::IO)
            })
            .unwrap_err();
            assert_eq!(error_attempts.get(), 1);
            assert_eq!(error.code, "write_file_write_failed");

            let cancellation = CancellationToken::new();
            let cancellation_after_interrupt = cancellation.clone();
            let cancelled_attempts = Cell::new(0_usize);
            let error = sync_before_commit_with(&cancellation, || {
                cancelled_attempts.set(cancelled_attempts.get() + 1);
                cancellation_after_interrupt.cancel();
                Err(rustix::io::Errno::INTR)
            })
            .unwrap_err();
            assert_eq!(cancelled_attempts.get(), 1);
            assert_eq!(error.code, "write_file_cancelled");

            let exhausted_attempts = Cell::new(0_usize);
            let exhausted = sync_before_commit_with(&CancellationToken::new(), || {
                exhausted_attempts.set(exhausted_attempts.get() + 1);
                Err(rustix::io::Errno::INTR)
            })
            .unwrap_err();
            assert_eq!(
                exhausted_attempts.get(),
                MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS
            );
            assert_eq!(exhausted.code, "write_file_write_failed");
            assert!(exhausted.retryable);

            let final_interrupt_cancellation = CancellationToken::new();
            let cancel_on_final_interrupt = final_interrupt_cancellation.clone();
            let final_interrupt_attempts = Cell::new(0_usize);
            let cancelled = sync_before_commit_with(&final_interrupt_cancellation, || {
                let attempt = final_interrupt_attempts.get() + 1;
                final_interrupt_attempts.set(attempt);
                if attempt == MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS {
                    cancel_on_final_interrupt.cancel();
                }
                Err(rustix::io::Errno::INTR)
            })
            .unwrap_err();
            assert_eq!(
                final_interrupt_attempts.get(),
                MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS
            );
            assert_eq!(cancelled.code, "write_file_cancelled");
        }

        #[test]
        fn create_and_replace_rename_error_classes_are_fixed() {
            let cases = [
                (
                    map_create_rename_error(rustix::io::Errno::EXIST),
                    "write_file_target_changed",
                    true,
                ),
                (
                    map_create_rename_error(rustix::io::Errno::ACCESS),
                    "write_file_permission_denied",
                    false,
                ),
                (
                    map_create_rename_error(rustix::io::Errno::PERM),
                    "write_file_permission_denied",
                    false,
                ),
                (
                    map_create_rename_error(rustix::io::Errno::IO),
                    "write_file_write_failed",
                    true,
                ),
                (
                    map_replace_rename_error(rustix::io::Errno::ACCESS),
                    "write_file_permission_denied",
                    false,
                ),
                (
                    map_replace_rename_error(rustix::io::Errno::PERM),
                    "write_file_permission_denied",
                    false,
                ),
                (
                    map_replace_rename_error(rustix::io::Errno::EXIST),
                    "write_file_write_failed",
                    true,
                ),
                (
                    map_replace_rename_error(rustix::io::Errno::IO),
                    "write_file_write_failed",
                    true,
                ),
            ];
            for (error, expected_code, expected_retryable) in cases {
                assert_eq!(error.code, expected_code);
                assert_eq!(error.retryable, expected_retryable);
            }
        }

        #[test]
        fn postrename_directory_sync_interrupt_and_error_are_ambiguous() {
            let attempts = Cell::new(0_usize);
            let directory_sync = sync_after_commit_with(|| {
                let attempt = attempts.get();
                attempts.set(attempt + 1);
                if attempt == 0 {
                    Err(rustix::io::Errno::INTR)
                } else {
                    Err(rustix::io::Errno::IO)
                }
            });
            assert_eq!(attempts.get(), 2);
            let error = finish_after_commit(true, directory_sync).unwrap_err();
            assert_eq!(error.code, "write_file_commit_ambiguous");
            assert!(!error.retryable);

            let retry_attempts = Cell::new(0_usize);
            let successful_sync = sync_after_commit_with(|| {
                let attempt = retry_attempts.get();
                retry_attempts.set(attempt + 1);
                if attempt == 0 {
                    Err(rustix::io::Errno::INTR)
                } else {
                    Ok(())
                }
            });
            assert_eq!(retry_attempts.get(), 2);
            assert!(finish_after_commit(true, successful_sync).is_ok());

            let exhausted_attempts = Cell::new(0_usize);
            let exhausted_sync = sync_after_commit_with(|| {
                exhausted_attempts.set(exhausted_attempts.get() + 1);
                Err(rustix::io::Errno::INTR)
            });
            assert_eq!(
                exhausted_attempts.get(),
                MAX_WRITE_FILE_INTERRUPTED_SYSCALL_ATTEMPTS
            );
            let exhausted = finish_after_commit(true, exhausted_sync).unwrap_err();
            assert_eq!(exhausted.code, "write_file_commit_ambiguous");
            assert!(!exhausted.retryable);

            assert!(finish_after_commit(true, Ok(())).is_ok());
            for result in [
                finish_after_commit(false, Ok(())),
                finish_after_commit(true, Err(rustix::io::Errno::IO)),
            ] {
                let error = result.unwrap_err();
                assert_eq!(error.code, "write_file_commit_ambiguous");
                assert!(!error.retryable);
            }
        }

        #[test]
        fn temp_names_are_fixed_length_hex_and_private_prefix() {
            let name = random_temp_name().unwrap();
            assert_eq!(name.len(), TEMP_NAME_PREFIX.len() + TEMP_RANDOM_BYTES * 2);
            let suffix = name.strip_prefix(TEMP_NAME_PREFIX).unwrap();
            assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }
}
