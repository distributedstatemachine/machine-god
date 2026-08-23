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
use rustix::fs::{AtFlags, FileType, Mode, OFlags};

/// Maximum UTF-8 bytes accepted in a requested or normalized target path.
pub const MAX_EDIT_FILE_PATH_BYTES: usize = 4 * 1024;
/// Maximum components accepted in a normalized target path.
pub const MAX_EDIT_FILE_PATH_COMPONENTS: usize = 256;
/// Maximum UTF-8 bytes accepted in `old_string`.
pub const MAX_EDIT_FILE_OLD_STRING_BYTES: usize = 48 * 1024;
/// Maximum UTF-8 bytes accepted in `new_string`.
pub const MAX_EDIT_FILE_NEW_STRING_BYTES: usize = 48 * 1024;
/// Maximum serialized byte size of accepted arguments.
pub const MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES: usize = 64 * 1024;
/// Maximum bytes accepted in an existing target file.
pub const MAX_EDIT_FILE_EXISTING_BYTES: usize = 48 * 1024;
/// Maximum bytes accepted in an edited result.
pub const MAX_EDIT_FILE_RESULTING_BYTES: usize = 48 * 1024;
/// Maximum bytes processed by one native read, write, or copy batch.
pub const MAX_EDIT_FILE_CHUNK_BYTES: usize = 8 * 1024;
/// Maximum charged KMP work steps accepted by one edit.
pub const MAX_EDIT_FILE_MATCH_WORK_STEPS: usize = 393_216;
/// Maximum exclusive temporary-name attempts made by one execution.
pub const MAX_EDIT_FILE_TEMP_ATTEMPTS: usize = 8;
/// Maximum serialized [`ToolOutput`] bytes produced by this tool.
pub const MAX_EDIT_FILE_SERIALIZED_RESULT_BYTES: usize = 16 * 1024;

/// Registered name of [`EditFileTool`].
pub const EDIT_FILE_TOOL_NAME: &str = "edit_file";

const EDIT_FILE_DESCRIPTION: &str =
    "Replace one exact text occurrence in an existing workspace file";
const PATH_DESCRIPTION: &str = "Workspace-relative file path";
const OLD_STRING_DESCRIPTION: &str = "Exact UTF-8 text to replace";
const NEW_STRING_DESCRIPTION: &str = "UTF-8 replacement text";

#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEMP_NAME_PREFIX: &str = ".machine-god-edit-";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEMP_RANDOM_BYTES: usize = 16;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_INTERRUPTED_SYSCALL_ATTEMPTS: usize = 16;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_ENTROPY_SYSCALL_ATTEMPTS: usize =
    TEMP_RANDOM_BYTES + MAX_INTERRUPTED_SYSCALL_ATTEMPTS - 1;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MATCH_CANCELLATION_BATCH_STEPS: usize = 1_024;

/// Stable category for failure to acquire an editable workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditFileToolOpenErrorKind {
    /// Native edit execution is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire an [`EditFileTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct EditFileToolOpenError {
    kind: EditFileToolOpenErrorKind,
}

impl EditFileToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> EditFileToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: EditFileToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for EditFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EditFileToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for EditFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            EditFileToolOpenErrorKind::UnsupportedPlatform => {
                "native edit_file is unsupported on this platform"
            }
            EditFileToolOpenErrorKind::InvalidRoot => "native edit_file workspace root is invalid",
            EditFileToolOpenErrorKind::InvalidFileType => {
                "native edit_file workspace root is not a directory"
            }
            EditFileToolOpenErrorKind::Unavailable => {
                "native edit_file workspace root is unavailable"
            }
        })
    }
}

impl Error for EditFileToolOpenError {}

/// A native exact-text editor confined to one explicitly opened workspace root.
///
/// Supported Linux and macOS implementations retain the opened root descriptor;
/// later calls never reopen the workspace root by its injected path.
pub struct EditFileTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl EditFileTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) const fn from_root_descriptor(root: OwnedFd) -> Self {
        Self { root }
    }

    /// Opens and retains an absolute workspace directory without following its
    /// final component.
    ///
    /// # Errors
    ///
    /// Returns a redacted typed failure when the platform is unsupported, the
    /// path is relative, or the root cannot be retained as a real directory.
    pub fn open(root: &Path) -> Result<Self, EditFileToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(EditFileToolOpenError::new(
                EditFileToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(EditFileToolOpenError::new(
                    EditFileToolOpenErrorKind::InvalidRoot,
                ));
            }
            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor)
                .map_err(|_| EditFileToolOpenError::new(EditFileToolOpenErrorKind::Unavailable))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(EditFileToolOpenError::new(
                    EditFileToolOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self::from_root_descriptor(descriptor))
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
) -> Result<(), rustix::io::Errno> {
    rustix::fs::renameat(parent, staged_name, parent, basename)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn native_sync_parent(parent: BorrowedFd<'_>) -> Result<(), rustix::io::Errno> {
    rustix::fs::fsync(parent)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WalkStep {
    Root,
    Intermediate(usize),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) trait EditFileEvidence {
    fn open_walk(
        &mut self,
        _phase: WalkPhase,
        _step: WalkStep,
        parent: BorrowedFd<'_>,
        component: &str,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty())
    }

    fn open_target(
        &mut self,
        _phase: ReadPhase,
        parent: BorrowedFd<'_>,
        basename: &str,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        rustix::fs::openat(
            parent,
            basename,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
    }

    fn open_stage(
        &mut self,
        parent: BorrowedFd<'_>,
        name: &str,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        rustix::fs::openat(
            parent,
            name,
            OFlags::RDWR
                | OFlags::CREATE
                | OFlags::EXCL
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::from_raw_mode(0o600),
        )
    }

    fn pread(
        &mut self,
        _phase: ReadPhase,
        file: BorrowedFd<'_>,
        buffer: &mut [u8],
        offset: u64,
    ) -> Result<usize, rustix::io::Errno> {
        rustix::io::pread(file, buffer, offset)
    }

    fn fstat(
        &mut self,
        _phase: ReadPhase,
        file: BorrowedFd<'_>,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::fstat(file)
    }

    fn statat(
        &mut self,
        _phase: ReadPhase,
        parent: BorrowedFd<'_>,
        name: &str,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn after_stage_created(
        &mut self,
        _parent: BorrowedFd<'_>,
        _file: BorrowedFd<'_>,
        _name: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        Ok(())
    }

    fn after_final_stage_sync(
        &mut self,
        _parent: BorrowedFd<'_>,
        _file: BorrowedFd<'_>,
        _name: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        Ok(())
    }

    fn after_rename(
        &mut self,
        _parent: BorrowedFd<'_>,
        _file: BorrowedFd<'_>,
        _basename: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NativeEditFileEvidence;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl EditFileEvidence for NativeEditFileEvidence {}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_target_with_evidence<Evidence: EditFileEvidence>(
    parent: BorrowedFd<'_>,
    basename: &str,
    phase: ReadPhase,
    evidence: &mut Evidence,
) -> Result<OwnedFd, ToolError> {
    match evidence.statat(phase, parent, basename) {
        Ok(metadata) if FileType::from_raw_mode(metadata.st_mode).is_file() => {}
        Ok(_) => {
            return Err(match phase {
                ReadPhase::Initial => rejected_path(),
                ReadPhase::Staged => write_failed(),
                ReadPhase::Revalidate => target_changed(),
                ReadPhase::Published => commit_ambiguous(),
            });
        }
        Err(error) => return Err(map_target_open_error(error, phase)),
    }
    evidence
        .open_target(phase, parent, basename)
        .map_err(|error| map_target_open_error(error, phase))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fingerprint_and_validate_path_with_evidence<Evidence: EditFileEvidence>(
    parent: BorrowedFd<'_>,
    basename: &str,
    file: BorrowedFd<'_>,
    phase: ReadPhase,
    evidence: &mut Evidence,
) -> Result<FileFingerprint, ToolError> {
    let descriptor_metadata = evidence.fstat(phase, file).map_err(|_| match phase {
        ReadPhase::Initial => unavailable(true),
        ReadPhase::Staged => write_failed(),
        ReadPhase::Revalidate => target_changed(),
        ReadPhase::Published => commit_ambiguous(),
    })?;
    if !FileType::from_raw_mode(descriptor_metadata.st_mode).is_file() {
        return Err(match phase {
            ReadPhase::Initial => rejected_path(),
            ReadPhase::Staged => write_failed(),
            ReadPhase::Revalidate => target_changed(),
            ReadPhase::Published => commit_ambiguous(),
        });
    }
    let path_metadata = evidence
        .statat(phase, parent, basename)
        .map_err(|_| match phase {
            ReadPhase::Staged => write_failed(),
            ReadPhase::Initial | ReadPhase::Revalidate => target_changed(),
            ReadPhase::Published => commit_ambiguous(),
        })?;
    if !FileType::from_raw_mode(path_metadata.st_mode).is_file()
        || FileFingerprint::from_stat(&path_metadata)
            != FileFingerprint::from_stat(&descriptor_metadata)
    {
        return Err(match phase {
            ReadPhase::Initial | ReadPhase::Revalidate => target_changed(),
            ReadPhase::Staged => write_failed(),
            ReadPhase::Published => commit_ambiguous(),
        });
    }
    Ok(FileFingerprint::from_stat(&descriptor_metadata))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn create_staged_file_with(
    parent: BorrowedFd<'_>,
    basename: &str,
    cancellation: &CancellationToken,
    mut next_name: impl FnMut(usize) -> Result<String, ToolError>,
) -> Result<(String, OwnedFd), ToolError> {
    let mut evidence = NativeEditFileEvidence;
    create_staged_file_with_evidence(
        parent,
        basename,
        cancellation,
        &mut next_name,
        &mut evidence,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_staged_file_with_evidence<Evidence: EditFileEvidence>(
    parent: BorrowedFd<'_>,
    basename: &str,
    cancellation: &CancellationToken,
    mut next_name: impl FnMut(usize) -> Result<String, ToolError>,
    evidence: &mut Evidence,
) -> Result<(String, OwnedFd), ToolError> {
    for attempt in 0..MAX_EDIT_FILE_TEMP_ATTEMPTS {
        check_cancellation(cancellation)?;
        let name = next_name(attempt)?;
        check_cancellation(cancellation)?;
        if name == basename {
            continue;
        }
        let result = evidence.open_stage(parent, &name);
        match result {
            Ok(file) => {
                let acl_result = clear_and_verify_staged_acl(file.as_fd());
                if cancellation.is_cancelled() {
                    cleanup_unpublished_file(parent, file.as_fd(), &name);
                    return Err(cancelled());
                }
                if let Err(error) = acl_result {
                    cleanup_unpublished_file(parent, file.as_fd(), &name);
                    return Err(error);
                }
                return Ok((name, file));
            }
            Err(error) => {
                check_cancellation(cancellation)?;
                if error == rustix::io::Errno::EXIST {
                    continue;
                }
                if is_permission_error(error) {
                    return Err(permission_denied());
                }
                return Err(unavailable(true));
            }
        }
    }
    Err(unavailable(true))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_unpublished_file(parent: BorrowedFd<'_>, file: BorrowedFd<'_>, name: &str) {
    cleanup_unpublished_file_with(parent, file, name, rustix::fs::fchmod, |parent, name| {
        rustix::fs::unlinkat(parent, name, AtFlags::empty())
    });
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn cleanup_unpublished_file_with<'fd>(
    parent: BorrowedFd<'fd>,
    file: BorrowedFd<'fd>,
    name: &str,
    mut set_mode: impl FnMut(BorrowedFd<'fd>, Mode) -> Result<(), rustix::io::Errno>,
    mut unlink: impl FnMut(BorrowedFd<'fd>, &str) -> Result<(), rustix::io::Errno>,
) {
    let _ = set_mode(file, Mode::from_raw_mode(0o600));
    let Ok(descriptor_metadata) = rustix::fs::fstat(file) else {
        return;
    };
    let Ok(path_metadata) = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) else {
        return;
    };
    if same_identity(&descriptor_metadata, &path_metadata)
        && FileType::from_raw_mode(path_metadata.st_mode).is_file()
    {
        let _ = unlink(parent, name);
    }
}

#[cfg(target_os = "macos")]
fn clear_and_verify_staged_acl(file: BorrowedFd<'_>) -> Result<(), ToolError> {
    calcifer_macos_acl::clear_acl(file).map_err(|_| write_failed())?;
    let acl = calcifer_macos_acl::read_acl(file).map_err(|_| write_failed())?;
    if acl.is_empty() {
        Ok(())
    } else {
        Err(write_failed())
    }
}

#[cfg(target_os = "linux")]
fn clear_and_verify_staged_acl(_file: BorrowedFd<'_>) -> Result<(), ToolError> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn verify_staged_acl(file: BorrowedFd<'_>, phase: ReadPhase) -> Result<(), ToolError> {
    let acl = calcifer_macos_acl::read_acl(file).map_err(|_| map_read_phase_failure(phase))?;
    if acl.is_empty() {
        Ok(())
    } else {
        Err(map_read_phase_failure(phase))
    }
}

#[cfg(target_os = "linux")]
fn verify_staged_acl(_file: BorrowedFd<'_>, _phase: ReadPhase) -> Result<(), ToolError> {
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn random_temp_name(cancellation: &CancellationToken) -> Result<String, ToolError> {
    random_temp_name_with(cancellation, native_entropy_read)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn random_temp_name_with(
    cancellation: &CancellationToken,
    read: impl FnMut(&mut [u8]) -> Result<usize, rustix::io::Errno>,
) -> Result<String, ToolError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut random = [0_u8; TEMP_RANDOM_BYTES];
    fill_random_with(&mut random, cancellation, read)?;
    let mut name = String::with_capacity(TEMP_NAME_PREFIX.len() + TEMP_RANDOM_BYTES * 2);
    name.push_str(TEMP_NAME_PREFIX);
    for byte in random {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(name)
}

#[cfg(target_os = "linux")]
fn native_entropy_read(buffer: &mut [u8]) -> Result<usize, rustix::io::Errno> {
    rustix::rand::getrandom(buffer, rustix::rand::GetRandomFlags::NONBLOCK)
}

#[cfg(target_os = "macos")]
fn native_entropy_read(buffer: &mut [u8]) -> Result<usize, rustix::io::Errno> {
    let requested = buffer.len();
    getrandom::fill(buffer).map_or_else(
        |error| {
            Err(error.raw_os_error().map_or(rustix::io::Errno::IO, |raw| {
                rustix::io::Errno::from_raw_os_error(raw)
            }))
        },
        |()| Ok(requested),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn fill_random_with(
    buffer: &mut [u8],
    cancellation: &CancellationToken,
    mut read: impl FnMut(&mut [u8]) -> Result<usize, rustix::io::Errno>,
) -> Result<(), ToolError> {
    let mut offset = 0_usize;
    let mut interrupted = 0_usize;
    let mut calls = 0_usize;
    while offset < buffer.len() {
        check_cancellation(cancellation)?;
        if calls >= MAX_ENTROPY_SYSCALL_ATTEMPTS {
            return Err(unavailable(true));
        }
        calls = calls.checked_add(1).ok_or_else(|| unavailable(true))?;
        let result = read(&mut buffer[offset..]);
        check_cancellation(cancellation)?;
        match result {
            Ok(count) if count > 0 && count <= buffer.len() - offset => {
                offset = offset.checked_add(count).ok_or_else(|| unavailable(true))?;
            }
            Err(error) if error == rustix::io::Errno::INTR => {
                interrupted = interrupted
                    .checked_add(1)
                    .ok_or_else(|| unavailable(true))?;
                if interrupted >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    return Err(unavailable(true));
                }
            }
            Ok(_) | Err(_) => return Err(unavailable(true)),
        }
    }
    check_cancellation(cancellation)
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
pub(super) fn write_content_with(
    content: &[u8],
    cancellation: &CancellationToken,
    mut write: impl FnMut(&[u8]) -> Result<usize, rustix::io::Errno>,
) -> Result<(), ToolError> {
    let mut offset = 0_usize;
    let mut interrupted = 0_usize;
    while offset < content.len() {
        check_cancellation(cancellation)?;
        let end = offset
            .saturating_add(MAX_EDIT_FILE_CHUNK_BYTES)
            .min(content.len());
        let chunk = &content[offset..end];
        let result = write(chunk);
        check_cancellation(cancellation)?;
        match result {
            Ok(0) => return Err(write_failed()),
            Ok(written) if written <= chunk.len() => {
                offset = offset.checked_add(written).ok_or_else(write_failed)?;
            }
            Err(error) if error == rustix::io::Errno::INTR => {
                interrupted = interrupted.checked_add(1).ok_or_else(write_failed)?;
                if interrupted >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    return Err(write_failed());
                }
            }
            Ok(_) | Err(_) => return Err(write_failed()),
        }
    }
    check_cancellation(cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_before_commit(
    file: BorrowedFd<'_>,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    sync_before_commit_with(cancellation, || rustix::fs::fsync(file))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn sync_before_commit_with(
    cancellation: &CancellationToken,
    mut sync: impl FnMut() -> Result<(), rustix::io::Errno>,
) -> Result<(), ToolError> {
    let mut interrupted = 0_usize;
    loop {
        check_cancellation(cancellation)?;
        let result = sync();
        check_cancellation(cancellation)?;
        match result {
            Ok(()) => return Ok(()),
            Err(error) if error == rustix::io::Errno::INTR => {
                interrupted = interrupted.checked_add(1).ok_or_else(write_failed)?;
                if interrupted >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    return Err(write_failed());
                }
            }
            Err(_) => return Err(write_failed()),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_after_commit_with(
    mut sync: impl FnMut() -> Result<(), rustix::io::Errno>,
) -> Result<(), rustix::io::Errno> {
    let mut interrupted = 0_usize;
    loop {
        match sync() {
            Ok(()) => return Ok(()),
            Err(error) if error == rustix::io::Errno::INTR => {
                interrupted += 1;
                if interrupted >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_staged_descriptor<Evidence: EditFileEvidence>(
    staged: &StagedFile<'_>,
    content_length: usize,
    expected_mode: Option<Mode>,
    evidence: &mut Evidence,
) -> Result<(), ToolError> {
    let metadata = evidence
        .fstat(ReadPhase::Staged, staged.file.as_fd())
        .map_err(|_| write_failed())?;
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
#[allow(clippy::too_many_arguments)]
fn verify_staged_content_at_path<Evidence: EditFileEvidence>(
    parent: BorrowedFd<'_>,
    name: &str,
    staged: &StagedFile<'_>,
    expected: &[u8],
    expected_mode: Mode,
    cancellation: &CancellationToken,
    phase: ReadPhase,
    evidence: &mut Evidence,
) -> Result<FileFingerprint, ToolError> {
    verify_staged_acl(staged.file.as_fd(), phase)?;
    let before = fingerprint_and_validate_path_with_evidence(
        parent,
        name,
        staged.file.as_fd(),
        phase,
        evidence,
    )?;
    let (actual, fingerprint) = read_bounded_stable_for_phase_with_evidence(
        staged.file.as_fd(),
        cancellation,
        MAX_EDIT_FILE_RESULTING_BYTES,
        phase,
        evidence,
    )?;
    if actual != expected
        || fingerprint != before
        || fingerprint.device != i128::from(staged.identity.st_dev)
        || fingerprint.inode != i128::from(staged.identity.st_ino)
        || fingerprint.mode != expected_mode.as_raw_mode()
        || usize::try_from(fingerprint.size).ok() != Some(expected.len())
    {
        return Err(map_read_phase_failure(phase));
    }
    let after = fingerprint_and_validate_path_with_evidence(
        parent,
        name,
        staged.file.as_fd(),
        phase,
        evidence,
    )?;
    if after != fingerprint {
        return Err(map_read_phase_failure(phase));
    }
    verify_staged_acl(staged.file.as_fd(), phase)?;
    Ok(fingerprint)
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
fn same_identity(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> bool {
    left.st_dev == right.st_dev && left.st_ino == right.st_ino
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_root_is_linked(root: BorrowedFd<'_>, phase: WalkPhase) -> Result<(), ToolError> {
    #[cfg(target_os = "linux")]
    {
        if rustix::fs::fstat(root)
            .map_err(|_| map_walk_failure(phase))?
            .st_nlink
            == 0
        {
            return Err(map_walk_failure(phase));
        }
    }
    #[cfg(target_os = "macos")]
    ensure_macos_root_is_linked(root, phase)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_macos_root_is_linked(root: BorrowedFd<'_>, phase: WalkPhase) -> Result<(), ToolError> {
    let root_metadata = rustix::fs::fstat(root).map_err(|_| map_walk_failure(phase))?;
    let root_path = rustix::fs::getpath(root).map_err(|_| map_walk_failure(phase))?;
    let root_path = root_path.as_bytes();
    if root_path == b"/" {
        return Ok(());
    }
    let name = root_path
        .rsplit(|byte| *byte == b'/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| map_walk_failure(phase))?;
    let name = std::ffi::CString::new(name).map_err(|_| map_walk_failure(phase))?;
    let parent = rustix::fs::openat(root, "..", directory_open_flags(), Mode::empty())
        .map_err(|_| map_walk_failure(phase))?;
    let linked = rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| map_walk_failure(phase))?;
    if !same_identity(&root_metadata, &linked) || !FileType::from_raw_mode(linked.st_mode).is_dir()
    {
        return Err(map_walk_failure(phase));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> EditFileToolOpenError {
    let kind = if is_rejected_type_error(error) {
        EditFileToolOpenErrorKind::InvalidFileType
    } else {
        EditFileToolOpenErrorKind::Unavailable
    };
    EditFileToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_walk_failure(phase: WalkPhase) -> ToolError {
    match phase {
        WalkPhase::Initial => unavailable(true),
        WalkPhase::Revalidate => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_walk_rejected(phase: WalkPhase) -> ToolError {
    match phase {
        WalkPhase::Initial => rejected_path(),
        WalkPhase::Revalidate => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_parent_open_error(error: rustix::io::Errno, phase: WalkPhase) -> ToolError {
    match phase {
        WalkPhase::Revalidate => target_changed(),
        WalkPhase::Initial if error == rustix::io::Errno::NOENT => not_found(),
        WalkPhase::Initial if is_permission_error(error) => permission_denied(),
        WalkPhase::Initial if is_rejected_type_error(error) => rejected_path(),
        WalkPhase::Initial => unavailable(true),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_target_open_error(error: rustix::io::Errno, phase: ReadPhase) -> ToolError {
    match phase {
        ReadPhase::Staged => write_failed(),
        ReadPhase::Revalidate => target_changed(),
        ReadPhase::Published => commit_ambiguous(),
        ReadPhase::Initial if error == rustix::io::Errno::NOENT => not_found(),
        ReadPhase::Initial if is_permission_error(error) => permission_denied(),
        ReadPhase::Initial if is_rejected_type_error(error) => rejected_path(),
        ReadPhase::Initial => unavailable(true),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_publish_error(error: rustix::io::Errno) -> ToolError {
    if is_permission_error(error) {
        permission_denied()
    } else {
        write_failed()
    }
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
        "edit_file_invalid_arguments",
        "edit_file arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "edit_file_invalid_path",
        "edit_file path is invalid",
        false,
    )
}

fn text_too_large() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "edit_file_text_too_large",
        "edit_file text exceeds the supported size limit",
        false,
    )
}

fn old_string_empty() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "edit_file_old_string_empty",
        "edit_file old_string must not be empty",
        false,
    )
}

fn strings_identical() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "edit_file_strings_identical",
        "edit_file old_string and new_string must differ",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "edit_file_unsupported_platform",
        "native edit_file is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "edit_file_not_found",
        "requested file is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "edit_file_permission_denied",
        "requested file cannot be edited",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rejected_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "edit_file_path_rejected",
        "requested path is not a confined regular file",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn existing_too_large() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "edit_file_existing_too_large",
        "requested file exceeds the supported size limit",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn invalid_utf8() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "edit_file_invalid_utf8",
        "requested file is not valid UTF-8",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn match_not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "edit_file_match_not_found",
        "old_string was not found",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn match_ambiguous() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "edit_file_match_ambiguous",
        "old_string occurs more than once",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn match_work_exceeded() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "edit_file_match_work_exceeded",
        "edit_file match work exceeds the supported limit",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn result_too_large() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "edit_file_result_too_large",
        "edited file exceeds the supported size limit",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable(retryable: bool) -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "edit_file_unavailable",
        "requested file is unavailable",
        retryable,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn target_changed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "edit_file_target_changed",
        "requested file changed before commit",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "edit_file_write_failed",
        "requested file could not be edited",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn commit_ambiguous() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "edit_file_commit_ambiguous",
        "requested file commit status is uncertain",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "edit_file_cancelled",
        "edit_file execution was cancelled",
        false,
    )
}

impl fmt::Debug for EditFileTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EditFileTool")
            .finish_non_exhaustive()
    }
}

struct ValidatedArguments<'a> {
    requested_path: &'a str,
    path: String,
    old_string: &'a str,
    new_string: &'a str,
}

impl Tool for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: edit_file_name(),
            description: EDIT_FILE_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": PATH_DESCRIPTION },
                    "old_string": { "type": "string", "description": OLD_STRING_DESCRIPTION },
                    "new_string": { "type": "string", "description": NEW_STRING_DESCRIPTION }
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != edit_file_name() {
            return Err(invalid_arguments());
        }
        let arguments = validate_arguments(&call.arguments)?;
        let prepared_arguments = json!({
            "path": arguments.path,
            "old_string": arguments.old_string,
            "new_string": arguments.new_string,
        });
        if !serialized_value_fits(&prepared_arguments, MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES) {
            return Err(invalid_arguments());
        }
        let path = prepared_arguments["path"]
            .as_str()
            .expect("prepared edit_file path is a string")
            .to_owned();
        Ok(PreparedToolCall::new(
            Capability::Filesystem {
                access: FilesystemAccess::Edit,
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
            if arguments.path != arguments.requested_path {
                return Err(invalid_arguments());
            }

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = cancellation;
                Err(unsupported_platform())
            }

            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.execute_supported(
                    &arguments.path,
                    arguments.old_string.as_bytes(),
                    arguments.new_string.as_bytes(),
                    &cancellation,
                )
            }
        })
    }
}

fn validate_arguments(arguments: &Value) -> Result<ValidatedArguments<'_>, ToolError> {
    let Value::Object(object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 3 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(path)) = object.get("path") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(old_string)) = object.get("old_string") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(new_string)) = object.get("new_string") else {
        return Err(invalid_arguments());
    };
    let normalized = normalize_relative_path(path)?;
    if old_string.len() > MAX_EDIT_FILE_OLD_STRING_BYTES
        || new_string.len() > MAX_EDIT_FILE_NEW_STRING_BYTES
    {
        return Err(text_too_large());
    }
    if old_string.is_empty() {
        return Err(old_string_empty());
    }
    if old_string == new_string {
        return Err(strings_identical());
    }
    if !serialized_value_fits(arguments, MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES) {
        return Err(invalid_arguments());
    }
    Ok(ValidatedArguments {
        requested_path: path,
        path: normalized,
        old_string,
        new_string,
    })
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_EDIT_FILE_PATH_BYTES
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
        if components > MAX_EDIT_FILE_PATH_COMPONENTS {
            return Err(invalid_path());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() || normalized.len() > MAX_EDIT_FILE_PATH_BYTES {
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

#[cfg(test)]
mod tests;

fn edit_file_name() -> ToolName {
    ToolName::new(EDIT_FILE_TOOL_NAME).expect("edit_file is a valid tool name")
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct MatchWorkMeter<'a, Notify> {
    cancellation: &'a CancellationToken,
    limit: usize,
    total: usize,
    batch: usize,
    notify: Notify,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<Notify: FnMut(usize)> MatchWorkMeter<'_, Notify> {
    fn charge(&mut self) -> Result<(), ToolError> {
        self.total = self.total.checked_add(1).ok_or_else(match_work_exceeded)?;
        if self.total > self.limit {
            return Err(match_work_exceeded());
        }
        self.batch += 1;
        if self.batch == MATCH_CANCELLATION_BATCH_STEPS {
            (self.notify)(self.batch);
            self.batch = 0;
            check_cancellation(self.cancellation)?;
        }
        Ok(())
    }

    fn checkpoint(&mut self) -> Result<(), ToolError> {
        if self.batch != 0 {
            (self.notify)(self.batch);
            self.batch = 0;
        }
        check_cancellation(self.cancellation)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn find_unique_match_with_budget(
    preimage: &[u8],
    pattern: &[u8],
    cancellation: &CancellationToken,
    max_steps: usize,
    on_batch: impl FnMut(usize),
) -> Result<usize, ToolError> {
    check_cancellation(cancellation)?;
    if pattern.is_empty() {
        return Err(old_string_empty());
    }

    let mut meter = MatchWorkMeter {
        cancellation,
        limit: max_steps,
        total: 0,
        batch: 0,
        notify: on_batch,
    };
    let mut prefix = vec![0_usize; pattern.len()];
    let mut matched = 0_usize;
    for index in 1..pattern.len() {
        loop {
            meter.charge()?;
            if pattern[index] == pattern[matched] {
                matched += 1;
                prefix[index] = matched;
                break;
            }
            if matched == 0 {
                break;
            }
            meter.charge()?;
            matched = prefix[matched - 1];
        }
    }
    meter.checkpoint()?;

    matched = 0;
    let mut unique_offset = None;
    for (index, byte) in preimage.iter().copied().enumerate() {
        loop {
            meter.charge()?;
            if byte == pattern[matched] {
                matched += 1;
                break;
            }
            if matched == 0 {
                break;
            }
            meter.charge()?;
            matched = prefix[matched - 1];
        }

        if matched == pattern.len() {
            let offset = index + 1 - pattern.len();
            meter.checkpoint()?;
            if unique_offset.replace(offset).is_some() {
                check_cancellation(cancellation)?;
                return Err(match_ambiguous());
            }
            meter.charge()?;
            matched = prefix[matched - 1];
            meter.checkpoint()?;
        }
    }
    meter.checkpoint()?;
    unique_offset.ok_or_else(match_not_found)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn find_unique_match(
    preimage: &[u8],
    pattern: &[u8],
    cancellation: &CancellationToken,
) -> Result<usize, ToolError> {
    find_unique_match_with_budget(
        preimage,
        pattern,
        cancellation,
        MAX_EDIT_FILE_MATCH_WORK_STEPS,
        |_| {},
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn build_postimage_with_budget(
    preimage: &[u8],
    match_offset: usize,
    old_len: usize,
    replacement: &[u8],
    cancellation: &CancellationToken,
    max_bytes: usize,
    mut on_batch: impl FnMut(usize),
) -> Result<Vec<u8>, ToolError> {
    check_cancellation(cancellation)?;
    let suffix_offset = match_offset
        .checked_add(old_len)
        .filter(|end| *end <= preimage.len())
        .ok_or_else(write_failed)?;
    let result_len = match_offset
        .checked_add(replacement.len())
        .and_then(|length| length.checked_add(preimage.len() - suffix_offset))
        .ok_or_else(result_too_large)?;
    if result_len > max_bytes {
        return Err(result_too_large());
    }
    let mut result = Vec::with_capacity(result_len);
    for part in [
        &preimage[..match_offset],
        replacement,
        &preimage[suffix_offset..],
    ] {
        for chunk in part.chunks(MAX_EDIT_FILE_CHUNK_BYTES) {
            check_cancellation(cancellation)?;
            result.extend_from_slice(chunk);
            on_batch(chunk.len());
            check_cancellation(cancellation)?;
        }
    }
    debug_assert_eq!(result.len(), result_len);
    Ok(result)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn build_postimage(
    preimage: &[u8],
    match_offset: usize,
    old_len: usize,
    replacement: &[u8],
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, ToolError> {
    build_postimage_with_budget(
        preimage,
        match_offset,
        old_len,
        replacement,
        cancellation,
        MAX_EDIT_FILE_RESULTING_BYTES,
        |_| {},
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn build_success_output_with_limit(
    normalized: &str,
    bytes_written: usize,
    limit: usize,
) -> Result<ToolOutput, ToolError> {
    let output = ToolOutput::success(json!({
        "path": normalized,
        "bytes_written": bytes_written,
    }));
    if !serialized_value_fits(&output, limit) {
        return Err(write_failed());
    }
    Ok(output)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FileFingerprint {
    device: i128,
    inode: i128,
    mode: rustix::fs::RawMode,
    size: i128,
    modified_seconds: i128,
    modified_nanoseconds: i128,
    changed_seconds: i128,
    changed_nanoseconds: i128,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FileFingerprint {
    fn from_stat(metadata: &rustix::fs::Stat) -> Self {
        Self {
            device: i128::from(metadata.st_dev),
            inode: i128::from(metadata.st_ino),
            mode: metadata.st_mode & 0o777,
            size: i128::from(metadata.st_size),
            modified_seconds: i128::from(metadata.st_mtime),
            modified_nanoseconds: i128::from(metadata.st_mtime_nsec),
            changed_seconds: i128::from(metadata.st_ctime),
            changed_nanoseconds: i128::from(metadata.st_ctime_nsec),
        }
    }

    fn ordinary_mode(self) -> Mode {
        Mode::from_raw_mode(self.mode)
    }

    fn stable_for_phase(self, other: Self, _phase: ReadPhase) -> bool {
        self == other
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReadPhase {
    Initial,
    Staged,
    Revalidate,
    Published,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_read_phase_failure(phase: ReadPhase) -> ToolError {
    match phase {
        ReadPhase::Initial => unavailable(true),
        ReadPhase::Staged => write_failed(),
        ReadPhase::Revalidate => target_changed(),
        ReadPhase::Published => commit_ambiguous(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_read_phase_too_large(phase: ReadPhase) -> ToolError {
    match phase {
        ReadPhase::Initial => existing_too_large(),
        ReadPhase::Staged => write_failed(),
        ReadPhase::Revalidate => target_changed(),
        ReadPhase::Published => commit_ambiguous(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn check_read_phase_cancellation(
    phase: ReadPhase,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    match phase {
        ReadPhase::Published => Ok(()),
        ReadPhase::Initial | ReadPhase::Staged | ReadPhase::Revalidate => {
            check_cancellation(cancellation)
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn read_bounded_stable_with(
    _file: BorrowedFd<'_>,
    cancellation: &CancellationToken,
    read: impl FnMut(&mut [u8], u64) -> Result<usize, rustix::io::Errno>,
    stat: impl FnMut() -> Result<rustix::fs::Stat, rustix::io::Errno>,
) -> Result<(Vec<u8>, FileFingerprint), ToolError> {
    read_bounded_stable_for_phase_with(
        cancellation,
        MAX_EDIT_FILE_EXISTING_BYTES,
        ReadPhase::Initial,
        read,
        stat,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[cfg_attr(not(test), allow(dead_code))]
fn read_bounded_stable_for_phase_with(
    cancellation: &CancellationToken,
    max_bytes: usize,
    phase: ReadPhase,
    read: impl FnMut(&mut [u8], u64) -> Result<usize, rustix::io::Errno>,
    stat: impl FnMut() -> Result<rustix::fs::Stat, rustix::io::Errno>,
) -> Result<(Vec<u8>, FileFingerprint), ToolError> {
    let mut source = ClosureStableReadSource { read, stat };
    read_bounded_stable_for_phase_from(cancellation, max_bytes, phase, &mut source)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
trait StableReadSource {
    fn pread(&mut self, buffer: &mut [u8], offset: u64) -> Result<usize, rustix::io::Errno>;

    fn fstat(&mut self) -> Result<rustix::fs::Stat, rustix::io::Errno>;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[cfg_attr(not(test), allow(dead_code))]
struct ClosureStableReadSource<Read, Stat> {
    read: Read,
    stat: Stat,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<Read, Stat> StableReadSource for ClosureStableReadSource<Read, Stat>
where
    Read: FnMut(&mut [u8], u64) -> Result<usize, rustix::io::Errno>,
    Stat: FnMut() -> Result<rustix::fs::Stat, rustix::io::Errno>,
{
    fn pread(&mut self, buffer: &mut [u8], offset: u64) -> Result<usize, rustix::io::Errno> {
        (self.read)(buffer, offset)
    }

    fn fstat(&mut self) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        (self.stat)()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct EvidenceStableReadSource<'evidence, 'fd, Evidence> {
    evidence: &'evidence mut Evidence,
    file: BorrowedFd<'fd>,
    phase: ReadPhase,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<Evidence: EditFileEvidence> StableReadSource for EvidenceStableReadSource<'_, '_, Evidence> {
    fn pread(&mut self, buffer: &mut [u8], offset: u64) -> Result<usize, rustix::io::Errno> {
        self.evidence.pread(self.phase, self.file, buffer, offset)
    }

    fn fstat(&mut self) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        self.evidence.fstat(self.phase, self.file)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_bounded_stable_for_phase_with_evidence<Evidence: EditFileEvidence>(
    file: BorrowedFd<'_>,
    cancellation: &CancellationToken,
    max_bytes: usize,
    phase: ReadPhase,
    evidence: &mut Evidence,
) -> Result<(Vec<u8>, FileFingerprint), ToolError> {
    let mut source = EvidenceStableReadSource {
        evidence,
        file,
        phase,
    };
    read_bounded_stable_for_phase_from(cancellation, max_bytes, phase, &mut source)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_bounded_stable_for_phase_from(
    cancellation: &CancellationToken,
    max_bytes: usize,
    phase: ReadPhase,
    source: &mut impl StableReadSource,
) -> Result<(Vec<u8>, FileFingerprint), ToolError> {
    check_read_phase_cancellation(phase, cancellation)?;
    let initial = source.fstat().map_err(|_| map_read_phase_failure(phase))?;
    check_read_phase_cancellation(phase, cancellation)?;
    if !FileType::from_raw_mode(initial.st_mode).is_file() {
        return Err(match phase {
            ReadPhase::Initial => rejected_path(),
            ReadPhase::Staged => write_failed(),
            ReadPhase::Revalidate => target_changed(),
            ReadPhase::Published => commit_ambiguous(),
        });
    }

    let capacity = max_bytes
        .checked_add(1)
        .ok_or_else(|| map_read_phase_failure(phase))?;
    let mut bytes = vec![0_u8; capacity];
    let mut length = 0_usize;
    let mut interrupted = 0_usize;
    while length < capacity {
        check_read_phase_cancellation(phase, cancellation)?;
        let end = length
            .saturating_add(MAX_EDIT_FILE_CHUNK_BYTES)
            .min(capacity);
        let offset = u64::try_from(length).map_err(|_| map_read_phase_failure(phase))?;
        let result = source.pread(&mut bytes[length..end], offset);
        check_read_phase_cancellation(phase, cancellation)?;
        match result {
            Ok(0) => break,
            Ok(count) if count <= end - length => {
                length = length
                    .checked_add(count)
                    .ok_or_else(|| map_read_phase_failure(phase))?;
            }
            Err(error) if error == rustix::io::Errno::INTR => {
                interrupted = interrupted
                    .checked_add(1)
                    .ok_or_else(|| map_read_phase_failure(phase))?;
                if interrupted >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    return Err(map_read_phase_failure(phase));
                }
            }
            Ok(_) | Err(_) => return Err(map_read_phase_failure(phase)),
        }
    }

    check_read_phase_cancellation(phase, cancellation)?;
    let final_metadata = source.fstat().map_err(|_| map_read_phase_failure(phase))?;
    check_read_phase_cancellation(phase, cancellation)?;
    if !FileType::from_raw_mode(final_metadata.st_mode).is_file() {
        return Err(map_read_phase_failure(phase));
    }
    let initial_fingerprint = FileFingerprint::from_stat(&initial);
    let final_fingerprint = FileFingerprint::from_stat(&final_metadata);
    if !initial_fingerprint.stable_for_phase(final_fingerprint, phase) {
        return Err(match phase {
            ReadPhase::Initial | ReadPhase::Revalidate => target_changed(),
            ReadPhase::Staged => write_failed(),
            ReadPhase::Published => commit_ambiguous(),
        });
    }
    let stable_size =
        usize::try_from(final_fingerprint.size).map_err(|_| map_read_phase_failure(phase))?;
    if length > max_bytes || stable_size > max_bytes {
        return Err(map_read_phase_too_large(phase));
    }
    if length != stable_size {
        return Err(map_read_phase_failure(phase));
    }
    bytes.truncate(length);
    Ok((bytes, final_fingerprint))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WalkPhase {
    Initial,
    Revalidate,
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
    fn new_with_evidence<Evidence: EditFileEvidence>(
        cleanup_parent: BorrowedFd<'a>,
        file: OwnedFd,
        name: String,
        evidence: &mut Evidence,
    ) -> Result<Self, ToolError> {
        let Ok(identity) = evidence.fstat(ReadPhase::Staged, file.as_fd()) else {
            cleanup_unpublished_file(cleanup_parent, file.as_fd(), &name);
            return Err(write_failed());
        };
        if !FileType::from_raw_mode(identity.st_mode).is_file() {
            cleanup_unpublished_file(cleanup_parent, file.as_fd(), &name);
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
        let _ = rustix::fs::fchmod(&self.file, Mode::from_raw_mode(0o600));
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
impl EditFileTool {
    fn execute_supported(
        &self,
        normalized: &str,
        old_string: &[u8],
        new_string: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        self.execute_supported_with(
            normalized,
            old_string,
            new_string,
            cancellation,
            native_set_mode,
            write_content,
            sync_before_commit,
            |_, _| {},
            || {},
            || {},
            native_publish_staged,
            native_sync_parent,
        )
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(super) fn execute_supported_with<
        SetMode,
        WriteContent,
        SyncStaged,
        BeforeStagedRevalidation,
        BeforeFinalVerification,
        BeforeRename,
        Publish,
        SyncParent,
    >(
        &self,
        normalized: &str,
        old_string: &[u8],
        new_string: &[u8],
        cancellation: &CancellationToken,
        mut set_mode: SetMode,
        mut write: WriteContent,
        mut sync_staged: SyncStaged,
        mut before_staged_revalidation: BeforeStagedRevalidation,
        mut before_final_verification: BeforeFinalVerification,
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
        BeforeFinalVerification: FnMut(),
        BeforeRename: FnMut(),
        Publish: for<'fd> FnMut(BorrowedFd<'fd>, &str, &str) -> Result<(), rustix::io::Errno>,
        SyncParent: for<'fd> FnMut(BorrowedFd<'fd>) -> Result<(), rustix::io::Errno>,
    {
        let mut evidence = NativeEditFileEvidence;
        self.execute_supported_with_evidence(
            normalized,
            old_string,
            new_string,
            cancellation,
            &mut evidence,
            &mut set_mode,
            &mut write,
            &mut sync_staged,
            &mut before_staged_revalidation,
            &mut before_final_verification,
            &mut before_rename,
            &mut publish,
            &mut sync_parent,
        )
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(super) fn execute_supported_with_evidence<
        Evidence,
        SetMode,
        WriteContent,
        SyncStaged,
        BeforeStagedRevalidation,
        BeforeFinalVerification,
        BeforeRename,
        Publish,
        SyncParent,
    >(
        &self,
        normalized: &str,
        old_string: &[u8],
        new_string: &[u8],
        cancellation: &CancellationToken,
        evidence: &mut Evidence,
        mut set_mode: SetMode,
        mut write: WriteContent,
        mut sync_staged: SyncStaged,
        mut before_staged_revalidation: BeforeStagedRevalidation,
        mut before_final_verification: BeforeFinalVerification,
        mut before_rename: BeforeRename,
        mut publish: Publish,
        mut sync_parent: SyncParent,
    ) -> Result<ToolOutput, ToolError>
    where
        Evidence: EditFileEvidence,
        SetMode: for<'fd> FnMut(BorrowedFd<'fd>, Mode) -> Result<(), rustix::io::Errno>,
        WriteContent:
            for<'fd> FnMut(BorrowedFd<'fd>, &[u8], &CancellationToken) -> Result<(), ToolError>,
        SyncStaged: for<'fd> FnMut(BorrowedFd<'fd>, &CancellationToken) -> Result<(), ToolError>,
        BeforeStagedRevalidation: for<'fd> FnMut(BorrowedFd<'fd>, &str),
        BeforeFinalVerification: FnMut(),
        BeforeRename: FnMut(),
        Publish: for<'fd> FnMut(BorrowedFd<'fd>, &str, &str) -> Result<(), rustix::io::Errno>,
        SyncParent: for<'fd> FnMut(BorrowedFd<'fd>) -> Result<(), rustix::io::Errno>,
    {
        check_cancellation(cancellation)?;
        let initial_walk =
            self.walk_parent_with_evidence(normalized, cancellation, WalkPhase::Initial, evidence)?;
        let initial_parent_metadata = evidence
            .fstat(ReadPhase::Initial, initial_walk.parent.as_fd())
            .map_err(|_| unavailable(true))?;
        check_cancellation(cancellation)?;
        if !FileType::from_raw_mode(initial_parent_metadata.st_mode).is_dir() {
            return Err(rejected_path());
        }

        let initial_target = open_target_with_evidence(
            initial_walk.parent.as_fd(),
            initial_walk.basename,
            ReadPhase::Initial,
            evidence,
        )?;
        let initial_open_fingerprint = fingerprint_and_validate_path_with_evidence(
            initial_walk.parent.as_fd(),
            initial_walk.basename,
            initial_target.as_fd(),
            ReadPhase::Initial,
            evidence,
        )?;
        let (preimage, initial_fingerprint) = read_bounded_stable_for_phase_with_evidence(
            initial_target.as_fd(),
            cancellation,
            MAX_EDIT_FILE_EXISTING_BYTES,
            ReadPhase::Initial,
            evidence,
        )?;
        if initial_fingerprint != initial_open_fingerprint {
            return Err(target_changed());
        }
        std::str::from_utf8(&preimage).map_err(|_| invalid_utf8())?;
        check_cancellation(cancellation)?;
        let match_offset = find_unique_match(&preimage, old_string, cancellation)?;
        check_cancellation(cancellation)?;
        let postimage = build_postimage(
            &preimage,
            match_offset,
            old_string.len(),
            new_string,
            cancellation,
        )?;
        let success_output = build_success_output_with_limit(
            normalized,
            postimage.len(),
            MAX_EDIT_FILE_SERIALIZED_RESULT_BYTES,
        )?;

        let (temp_name, temp_file) = create_staged_file_with_evidence(
            initial_walk.parent.as_fd(),
            initial_walk.basename,
            cancellation,
            |_| random_temp_name(cancellation),
            evidence,
        )?;
        let mut staged = StagedFile::new_with_evidence(
            initial_walk.parent.as_fd(),
            temp_file,
            temp_name,
            evidence,
        )?;
        evidence.after_stage_created(
            initial_walk.parent.as_fd(),
            staged.file.as_fd(),
            &staged.name,
            cancellation,
        )?;
        check_cancellation(cancellation)?;

        let private_mode = Mode::from_raw_mode(0o600);
        set_mode(staged.file.as_fd(), private_mode).map_err(|_| write_failed())?;
        check_cancellation(cancellation)?;
        verify_staged_descriptor(&staged, 0, Some(private_mode), evidence)?;
        write(staged.file.as_fd(), &postimage, cancellation)?;
        verify_staged_descriptor(&staged, postimage.len(), Some(private_mode), evidence)?;
        verify_staged_content_at_path(
            initial_walk.parent.as_fd(),
            &staged.name,
            &staged,
            &postimage,
            private_mode,
            cancellation,
            ReadPhase::Staged,
            evidence,
        )?;

        let final_walk = self.walk_parent_with_evidence(
            normalized,
            cancellation,
            WalkPhase::Revalidate,
            evidence,
        )?;
        let final_parent_metadata = evidence
            .fstat(ReadPhase::Revalidate, final_walk.parent.as_fd())
            .map_err(|_| target_changed())?;
        check_cancellation(cancellation)?;
        revalidate_parent_identity(&initial_parent_metadata, &final_parent_metadata)?;
        before_final_verification();
        check_cancellation(cancellation)?;

        let verification_target = open_target_with_evidence(
            final_walk.parent.as_fd(),
            final_walk.basename,
            ReadPhase::Revalidate,
            evidence,
        )?;
        let verification_open_fingerprint = fingerprint_and_validate_path_with_evidence(
            final_walk.parent.as_fd(),
            final_walk.basename,
            verification_target.as_fd(),
            ReadPhase::Revalidate,
            evidence,
        )?;
        let (verification_bytes, verification_fingerprint) =
            read_bounded_stable_for_phase_with_evidence(
                verification_target.as_fd(),
                cancellation,
                MAX_EDIT_FILE_EXISTING_BYTES,
                ReadPhase::Revalidate,
                evidence,
            )?;
        if verification_open_fingerprint != initial_fingerprint
            || verification_fingerprint != initial_fingerprint
            || verification_bytes != preimage
        {
            return Err(target_changed());
        }
        // Keep both target descriptors alive through the irreversible boundary.
        let _retained_initial_target = &initial_target;
        let _retained_verification_target = &verification_target;

        check_cancellation(cancellation)?;
        before_staged_revalidation(final_walk.parent.as_fd(), &staged.name);
        check_cancellation(cancellation)?;
        verify_staged_content_at_path(
            final_walk.parent.as_fd(),
            &staged.name,
            &staged,
            &postimage,
            private_mode,
            cancellation,
            ReadPhase::Staged,
            evidence,
        )?;

        before_rename();
        check_cancellation(cancellation)?;
        verify_staged_content_at_path(
            final_walk.parent.as_fd(),
            &staged.name,
            &staged,
            &postimage,
            private_mode,
            cancellation,
            ReadPhase::Staged,
            evidence,
        )?;

        let final_mode = initial_fingerprint.ordinary_mode();
        check_cancellation(cancellation)?;
        set_mode(staged.file.as_fd(), final_mode).map_err(|_| write_failed())?;
        check_cancellation(cancellation)?;
        sync_staged(staged.file.as_fd(), cancellation)?;
        evidence.after_final_stage_sync(
            final_walk.parent.as_fd(),
            staged.file.as_fd(),
            &staged.name,
            cancellation,
        )?;
        verify_staged_content_at_path(
            final_walk.parent.as_fd(),
            &staged.name,
            &staged,
            &postimage,
            final_mode,
            cancellation,
            ReadPhase::Staged,
            evidence,
        )?;
        check_cancellation(cancellation)?;
        publish(final_walk.parent.as_fd(), &staged.name, final_walk.basename)
            .map_err(map_publish_error)?;
        staged.mark_published();

        let after_rename = evidence.after_rename(
            final_walk.parent.as_fd(),
            staged.file.as_fd(),
            final_walk.basename,
            cancellation,
        );
        let published_verification = verify_staged_content_at_path(
            final_walk.parent.as_fd(),
            final_walk.basename,
            &staged,
            &postimage,
            final_mode,
            cancellation,
            ReadPhase::Published,
            evidence,
        );
        let directory_sync = sync_after_commit_with(|| sync_parent(final_walk.parent.as_fd()));
        if after_rename.is_err() || published_verification.is_err() || directory_sync.is_err() {
            return Err(commit_ambiguous());
        }

        debug_assert!(serialized_value_fits(
            &success_output,
            MAX_EDIT_FILE_SERIALIZED_RESULT_BYTES
        ));
        Ok(success_output)
    }

    fn walk_parent_with_evidence<'a, Evidence: EditFileEvidence>(
        &self,
        normalized: &'a str,
        cancellation: &CancellationToken,
        phase: WalkPhase,
        evidence: &mut Evidence,
    ) -> Result<ParentWalk<'a>, ToolError> {
        check_cancellation(cancellation)?;
        let mut parent = evidence
            .open_walk(phase, WalkStep::Root, self.root.as_fd(), ".")
            .map_err(|_| map_walk_failure(phase))?;
        check_cancellation(cancellation)?;
        ensure_root_is_linked(parent.as_fd(), phase)?;

        let mut components = normalized.split('/').peekable();
        let mut depth = 0_usize;
        loop {
            check_cancellation(cancellation)?;
            let component = components.next().ok_or_else(invalid_arguments)?;
            if components.peek().is_none() {
                return Ok(ParentWalk {
                    parent,
                    basename: component,
                });
            }
            parent = evidence
                .open_walk(
                    phase,
                    WalkStep::Intermediate(depth),
                    parent.as_fd(),
                    component,
                )
                .map_err(|error| map_parent_open_error(error, phase))?;
            check_cancellation(cancellation)?;
            let read_phase = match phase {
                WalkPhase::Initial => ReadPhase::Initial,
                WalkPhase::Revalidate => ReadPhase::Revalidate,
            };
            let metadata = evidence
                .fstat(read_phase, parent.as_fd())
                .map_err(|_| map_walk_failure(phase))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(map_walk_rejected(phase));
            }
            depth = depth
                .checked_add(1)
                .ok_or_else(|| map_walk_failure(phase))?;
        }
    }
}
