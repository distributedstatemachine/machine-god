use std::error::Error;
use std::fmt;
use std::io;
use std::path::Path;

use machine_god_core::{
    BoxFuture, CancellationToken, Capability, PreparedToolCall, Tool, ToolCall, ToolContext,
    ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Value, json};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags, SeekFrom};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use sha2::{Digest, Sha256};

/// Maximum UTF-8 bytes accepted in either requested or normalized endpoint.
pub const MAX_COPY_FILE_PATH_BYTES: usize = 4 * 1024;
/// Maximum components accepted in either normalized endpoint.
pub const MAX_COPY_FILE_PATH_COMPONENTS: usize = 256;
/// Maximum source bytes accepted by one copy.
pub const MAX_COPY_FILE_SOURCE_BYTES: usize = 16 * 1024 * 1024;
/// Maximum bytes transferred by one read or write call.
pub const MAX_COPY_FILE_CHUNK_BYTES: usize = 64 * 1024;
/// Maximum native calls accepted by one logical streaming phase.
pub const MAX_COPY_FILE_IO_CALLS: usize = 4 * 1024;
/// Maximum exclusive temporary-name attempts made by one execution.
pub const MAX_COPY_FILE_TEMP_ATTEMPTS: usize = 8;
/// Maximum serialized byte size of accepted arguments.
pub const MAX_COPY_FILE_SERIALIZED_ARGUMENT_BYTES: usize = 64 * 1024;
/// Maximum serialized [`ToolOutput`] bytes produced by this tool.
pub const MAX_COPY_FILE_SERIALIZED_RESULT_BYTES: usize = 16 * 1024;

/// Registered name of [`CopyFileTool`].
pub const COPY_FILE_TOOL_NAME: &str = "copy_file";

const COPY_FILE_DESCRIPTION: &str =
    "Copy one existing regular file to an absent path within the configured workspace";
const SOURCE_DESCRIPTION: &str = "Source workspace-relative regular-file path";
const DESTINATION_DESCRIPTION: &str = "Destination workspace-relative file path";

#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_INTERRUPTED_SYSCALL_ATTEMPTS: usize = 16;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEMP_NAME_PREFIX: &str = ".machine-god-copy-";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEMP_RANDOM_BYTES: usize = 16;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_ENTROPY_SYSCALL_ATTEMPTS: usize =
    TEMP_RANDOM_BYTES + MAX_INTERRUPTED_SYSCALL_ATTEMPTS - 1;

/// Stable category for failure to acquire a copy-capable workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyFileToolOpenErrorKind {
    /// Native copy execution is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`CopyFileTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CopyFileToolOpenError {
    kind: CopyFileToolOpenErrorKind,
}

impl CopyFileToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> CopyFileToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: CopyFileToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for CopyFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CopyFileToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for CopyFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            CopyFileToolOpenErrorKind::UnsupportedPlatform => {
                "native copy_file is unsupported on this platform"
            }
            CopyFileToolOpenErrorKind::InvalidRoot => "native copy_file workspace root is invalid",
            CopyFileToolOpenErrorKind::InvalidFileType => {
                "native copy_file workspace root is not a directory"
            }
            CopyFileToolOpenErrorKind::Unavailable => {
                "native copy_file workspace root is unavailable"
            }
        })
    }
}

impl Error for CopyFileToolOpenError {}

/// Native bounded file copier confined to one retained workspace root.
pub struct CopyFileTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl CopyFileTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) const fn from_root_descriptor(root: OwnedFd) -> Self {
        Self { root }
    }

    /// Opens and retains an absolute workspace directory without following its
    /// final component.
    ///
    /// # Errors
    ///
    /// Returns a fixed redacted failure when the platform is unsupported, the
    /// path is relative, or the root cannot be retained as a real directory.
    pub fn open(root: &Path) -> Result<Self, CopyFileToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(CopyFileToolOpenError::new(
                CopyFileToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(CopyFileToolOpenError::new(
                    CopyFileToolOpenErrorKind::InvalidRoot,
                ));
            }
            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor)
                .map_err(|_| CopyFileToolOpenError::new(CopyFileToolOpenErrorKind::Unavailable))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(CopyFileToolOpenError::new(
                    CopyFileToolOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests;

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CopyCheckpoint {
    AfterInitialSourceParent,
    AfterSourceRetained,
    AfterInitialDestinationParent,
    AfterDestinationAbsent,
    AfterStageCreated,
    AfterCopy,
    AfterInitialStageVerification,
    AfterFinalStageVerification,
    AfterFinalSourceParent,
    AfterFinalSourceValidation,
    AfterFinalDestinationParent,
    AfterFinalDestinationValidation,
    AfterFinalStageValidation,
    FinalPrePublish,
    AfterPublish,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CopySyncSite {
    Staged,
    DestinationParent,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CopyBufferSite {
    Copy,
    InitialStageHash,
    FinalStageHash,
    PublishedHash,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
trait CopyFileEvidence {
    fn checkpoint(&mut self, _checkpoint: CopyCheckpoint, _cancellation: &CancellationToken) {}

    fn observe_buffer(&mut self, _site: CopyBufferSite, _buffer: &[u8]) {}

    fn next_temp_name(
        &mut self,
        _attempt: usize,
        cancellation: &CancellationToken,
    ) -> Result<String, ToolError> {
        random_temp_name(cancellation)
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

    fn initial_stage_metadata(
        &mut self,
        file: BorrowedFd<'_>,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::fstat(file)
    }

    fn publish(
        &mut self,
        staged_parent: BorrowedFd<'_>,
        staged_name: &str,
        destination_parent: BorrowedFd<'_>,
        destination_name: &str,
    ) -> Result<(), rustix::io::Errno> {
        rustix::fs::renameat_with(
            staged_parent,
            staged_name,
            destination_parent,
            destination_name,
            RenameFlags::NOREPLACE,
        )
    }

    fn after_publish(
        &mut self,
        _outcome: Result<(), rustix::io::Errno>,
        _cancellation: &CancellationToken,
    ) {
    }

    fn sync(
        &mut self,
        _site: CopySyncSite,
        _attempt: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        rustix::fs::fsync(descriptor)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NativeCopyFileEvidence;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CopyFileEvidence for NativeCopyFileEvidence {}

#[cfg(any(target_os = "linux", target_os = "macos"))]
trait CopyFileCleanupEvidence {
    fn set_mode(&self, file: BorrowedFd<'_>, mode: Mode) -> Result<(), rustix::io::Errno> {
        rustix::fs::fchmod(file, mode)
    }

    fn fstat(&self, file: BorrowedFd<'_>) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::fstat(file)
    }

    fn statat(
        &self,
        parent: BorrowedFd<'_>,
        name: &str,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn unlink(&self, parent: BorrowedFd<'_>, name: &str) -> Result<(), rustix::io::Errno> {
        rustix::fs::unlinkat(parent, name, AtFlags::empty())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NativeCopyFileCleanupEvidence;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CopyFileCleanupEvidence for NativeCopyFileCleanupEvidence {}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn precommit_checkpoint<Evidence: CopyFileEvidence>(
    evidence: &mut Evidence,
    checkpoint: CopyCheckpoint,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    evidence.checkpoint(checkpoint, cancellation);
    check_cancellation(cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn precommit_call<ResultValue>(
    cancellation: &CancellationToken,
    call: impl FnOnce() -> ResultValue,
) -> Result<ResultValue, ToolError> {
    check_cancellation(cancellation)?;
    let result = call();
    check_cancellation(cancellation)?;
    Ok(result)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn retain_source(
    parent: BorrowedFd<'_>,
    basename: &str,
    cancellation: &CancellationToken,
) -> Result<RetainedSource, ToolError> {
    let path_result = precommit_call(cancellation, || {
        rustix::fs::statat(parent, basename, AtFlags::SYMLINK_NOFOLLOW)
    })?;
    let path_metadata = match path_result {
        Ok(metadata) => metadata,
        Err(error) if error == rustix::io::Errno::NOENT => return Err(not_found()),
        Err(error) if is_permission_error(error) => return Err(permission_denied()),
        Err(error) if is_rejected_type_error(error) => return Err(path_rejected()),
        Err(_) => return Err(unavailable()),
    };
    if !FileType::from_raw_mode(path_metadata.st_mode).is_file() {
        return Err(path_rejected());
    }
    let path_fingerprint = SourceFingerprint::from_stat(&path_metadata)?;

    let file = precommit_call(cancellation, || {
        rustix::fs::openat(
            parent,
            basename,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
    })?
    .map_err(map_source_open_error)?;
    let descriptor_metadata =
        precommit_call(cancellation, || rustix::fs::fstat(&file))?.map_err(|_| unavailable())?;
    let descriptor_fingerprint = SourceFingerprint::from_stat(&descriptor_metadata)?;
    if descriptor_fingerprint != path_fingerprint {
        return Err(target_changed());
    }
    Ok(RetainedSource {
        file,
        fingerprint: descriptor_fingerprint,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn revalidate_source_path(
    parent: BorrowedFd<'_>,
    basename: &str,
    source: &RetainedSource,
    cancellation: Option<&CancellationToken>,
) -> Result<(), ToolError> {
    check_optional_cancellation(cancellation)?;
    let descriptor_result = rustix::fs::fstat(&source.file);
    check_optional_cancellation(cancellation)?;
    let descriptor = descriptor_result.map_err(|_| target_changed())?;
    let path_result = rustix::fs::statat(parent, basename, AtFlags::SYMLINK_NOFOLLOW);
    check_optional_cancellation(cancellation)?;
    let path = path_result.map_err(|_| target_changed())?;
    if source.fingerprint.matches_stat(&descriptor) && source.fingerprint.matches_stat(&path) {
        Ok(())
    } else {
        Err(target_changed())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn require_destination_absent(
    parent: BorrowedFd<'_>,
    basename: &str,
    phase: WalkPhase,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    check_cancellation(cancellation)?;
    let result = rustix::fs::statat(parent, basename, AtFlags::SYMLINK_NOFOLLOW);
    check_cancellation(cancellation)?;
    match result {
        Err(error) if error == rustix::io::Errno::NOENT => Ok(()),
        Ok(_) if phase == WalkPhase::Initial => Err(destination_exists()),
        Err(error) if phase == WalkPhase::Initial && is_permission_error(error) => {
            Err(permission_denied())
        }
        Err(error) if phase == WalkPhase::Initial && is_rejected_type_error(error) => {
            Err(path_rejected())
        }
        Err(_) if phase == WalkPhase::Initial => Err(unavailable()),
        Ok(_) | Err(_) => Err(target_changed()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_staged_file<
    'parent,
    Evidence: CopyFileEvidence,
    CleanupEvidence: CopyFileCleanupEvidence,
>(
    parent: BorrowedFd<'parent>,
    basename: &str,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    cleanup_evidence: &'parent CleanupEvidence,
) -> Result<StagedFile<'parent, CleanupEvidence>, ToolError> {
    for attempt in 0..MAX_COPY_FILE_TEMP_ATTEMPTS {
        let name = precommit_call(cancellation, || {
            evidence.next_temp_name(attempt, cancellation)
        })??;
        if name == basename {
            continue;
        }
        check_cancellation(cancellation)?;
        let result = evidence.open_stage(parent, &name);
        match result {
            Ok(file) => {
                let mut staged = StagedFile::new(parent, cleanup_evidence, file, name);
                check_cancellation(cancellation)?;
                staged.initialize(cancellation, evidence)?;
                return Ok(staged);
            }
            Err(error) => {
                check_cancellation(cancellation)?;
                if error == rustix::io::Errno::EXIST {
                    continue;
                }
                if is_permission_error(error) {
                    return Err(permission_denied());
                }
                return Err(unavailable());
            }
        }
    }
    Err(unavailable())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn random_temp_name(cancellation: &CancellationToken) -> Result<String, ToolError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut random = [0_u8; TEMP_RANDOM_BYTES];
    fill_random_with(&mut random, cancellation, entropy_read)?;
    let mut name = String::with_capacity(TEMP_NAME_PREFIX.len() + TEMP_RANDOM_BYTES * 2);
    name.push_str(TEMP_NAME_PREFIX);
    for byte in random {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(name)
}

#[cfg(target_os = "linux")]
fn entropy_read(buffer: &mut [u8]) -> Result<usize, rustix::io::Errno> {
    rustix::rand::getrandom(buffer, rustix::rand::GetRandomFlags::NONBLOCK)
}

#[cfg(target_os = "macos")]
fn entropy_read(buffer: &mut [u8]) -> Result<usize, rustix::io::Errno> {
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
fn fill_random_with(
    buffer: &mut [u8],
    cancellation: &CancellationToken,
    mut read_entropy: impl FnMut(&mut [u8]) -> Result<usize, rustix::io::Errno>,
) -> Result<(), ToolError> {
    let mut offset = 0_usize;
    let mut interruptions = 0_usize;
    let mut calls = 0_usize;
    while offset < buffer.len() {
        check_cancellation(cancellation)?;
        if calls >= MAX_ENTROPY_SYSCALL_ATTEMPTS {
            return Err(unavailable());
        }
        calls = calls.checked_add(1).ok_or_else(unavailable)?;
        let remaining = &mut buffer[offset..];
        let result = read_entropy(remaining);
        check_cancellation(cancellation)?;
        match result {
            Ok(0) => return Err(unavailable()),
            Ok(read) if read <= remaining.len() => {
                offset = offset.checked_add(read).ok_or_else(unavailable)?;
            }
            Err(error) if error == rustix::io::Errno::INTR => {
                interruptions = interruptions.checked_add(1).ok_or_else(unavailable)?;
                if interruptions >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    return Err(unavailable());
                }
            }
            Ok(_) | Err(_) => return Err(unavailable()),
        }
    }
    check_cancellation(cancellation)
}

#[cfg(target_os = "macos")]
fn clear_staged_acl(file: BorrowedFd<'_>) -> Result<(), ToolError> {
    calcifer_macos_acl::clear_acl(file).map_err(|_| copy_failed())
}

#[cfg(target_os = "linux")]
fn clear_staged_acl(_file: BorrowedFd<'_>) -> Result<(), ToolError> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn staged_acl_is_empty(file: BorrowedFd<'_>) -> Result<bool, ToolError> {
    calcifer_macos_acl::read_acl(file)
        .map(|acl| acl.is_empty())
        .map_err(|_| copy_failed())
}

#[cfg(target_os = "linux")]
fn staged_acl_is_empty(_file: BorrowedFd<'_>) -> Result<bool, ToolError> {
    Ok(true)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn stream_source_to_stage(
    source: BorrowedFd<'_>,
    staged: BorrowedFd<'_>,
    expected_size: usize,
    cancellation: &CancellationToken,
    buffer: &mut [u8],
) -> Result<[u8; 32], ToolError> {
    precommit_call(cancellation, || {
        rustix::fs::seek(source, SeekFrom::Start(0))
    })?
    .map_err(map_precommit_io_error)?;
    precommit_call(cancellation, || {
        rustix::fs::seek(staged, SeekFrom::Start(0))
    })?
    .map_err(map_precommit_io_error)?;
    stream_source_to_stage_with(
        expected_size,
        cancellation,
        buffer,
        |chunk| rustix::io::read(source, chunk),
        |chunk| rustix::io::write(staged, chunk),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn stream_source_to_stage_with(
    expected_size: usize,
    cancellation: &CancellationToken,
    buffer: &mut [u8],
    mut read_source: impl FnMut(&mut [u8]) -> Result<usize, rustix::io::Errno>,
    mut write_staged: impl FnMut(&[u8]) -> Result<usize, rustix::io::Errno>,
) -> Result<[u8; 32], ToolError> {
    if buffer.is_empty() || buffer.len() > MAX_COPY_FILE_CHUNK_BYTES {
        return Err(copy_failed());
    }
    let mut hasher = Sha256::new();
    let mut copied = 0_usize;
    let mut read_calls = 0_usize;
    let mut read_interruptions = 0_usize;
    let mut write_calls = 0_usize;
    let mut write_interruptions = 0_usize;

    while copied < expected_size {
        check_cancellation(cancellation)?;
        let wanted = (expected_size - copied).min(buffer.len());
        let read = loop {
            if read_calls >= MAX_COPY_FILE_IO_CALLS {
                return Err(copy_failed());
            }
            read_calls = read_calls.checked_add(1).ok_or_else(copy_failed)?;
            let result = read_source(&mut buffer[..wanted]);
            check_cancellation(cancellation)?;
            match result {
                Ok(0) => return Err(target_changed()),
                Ok(read) if read <= wanted => break read,
                Ok(_) => return Err(copy_failed()),
                Err(error) if error == rustix::io::Errno::INTR => {
                    read_interruptions =
                        read_interruptions.checked_add(1).ok_or_else(copy_failed)?;
                    if read_interruptions >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                        return Err(copy_failed());
                    }
                }
                Err(error) => return Err(map_precommit_io_error(error)),
            }
        };
        hasher.update(&buffer[..read]);
        let mut written = 0_usize;
        while written < read {
            check_cancellation(cancellation)?;
            if write_calls >= MAX_COPY_FILE_IO_CALLS {
                return Err(copy_failed());
            }
            write_calls = write_calls.checked_add(1).ok_or_else(copy_failed)?;
            let result = write_staged(&buffer[written..read]);
            check_cancellation(cancellation)?;
            match result {
                Ok(0) => return Err(copy_failed()),
                Ok(progress) if progress <= read - written => {
                    written = written.checked_add(progress).ok_or_else(copy_failed)?;
                }
                Ok(_) => return Err(copy_failed()),
                Err(error) if error == rustix::io::Errno::INTR => {
                    write_interruptions =
                        write_interruptions.checked_add(1).ok_or_else(copy_failed)?;
                    if write_interruptions >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                        return Err(copy_failed());
                    }
                }
                Err(error) => return Err(map_precommit_io_error(error)),
            }
        }
        copied = copied.checked_add(read).ok_or_else(copy_failed)?;
    }

    check_cancellation(cancellation)?;
    let mut overflow = [0_u8; 1];
    loop {
        if read_calls >= MAX_COPY_FILE_IO_CALLS {
            return Err(copy_failed());
        }
        read_calls = read_calls.checked_add(1).ok_or_else(copy_failed)?;
        let result = read_source(&mut overflow);
        check_cancellation(cancellation)?;
        match result {
            Ok(0) => break,
            Ok(1) if expected_size == MAX_COPY_FILE_SOURCE_BYTES => {
                return Err(source_too_large());
            }
            Ok(1) => return Err(target_changed()),
            Ok(_) => return Err(copy_failed()),
            Err(error) if error == rustix::io::Errno::INTR => {
                read_interruptions = read_interruptions.checked_add(1).ok_or_else(copy_failed)?;
                if read_interruptions >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    return Err(copy_failed());
                }
            }
            Err(error) => return Err(map_precommit_io_error(error)),
        }
    }
    Ok(hasher.finalize().into())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn hash_file(
    file: BorrowedFd<'_>,
    expected_size: usize,
    cancellation: Option<&CancellationToken>,
    buffer: &mut [u8],
) -> Result<[u8; 32], ToolError> {
    check_optional_cancellation(cancellation)?;
    let seek = rustix::fs::seek(file, SeekFrom::Start(0));
    check_optional_cancellation(cancellation)?;
    seek.map_err(map_precommit_io_error)?;
    hash_file_with(expected_size, cancellation, buffer, |chunk| {
        rustix::io::read(file, chunk)
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn hash_file_with(
    expected_size: usize,
    cancellation: Option<&CancellationToken>,
    buffer: &mut [u8],
    mut read_file: impl FnMut(&mut [u8]) -> Result<usize, rustix::io::Errno>,
) -> Result<[u8; 32], ToolError> {
    if buffer.is_empty() || buffer.len() > MAX_COPY_FILE_CHUNK_BYTES {
        return Err(copy_failed());
    }
    let mut hasher = Sha256::new();
    let mut read_bytes = 0_usize;
    let mut calls = 0_usize;
    let mut interruptions = 0_usize;
    while read_bytes < expected_size {
        check_optional_cancellation(cancellation)?;
        if calls >= MAX_COPY_FILE_IO_CALLS {
            return Err(copy_failed());
        }
        calls = calls.checked_add(1).ok_or_else(copy_failed)?;
        let wanted = (expected_size - read_bytes).min(buffer.len());
        let result = read_file(&mut buffer[..wanted]);
        check_optional_cancellation(cancellation)?;
        match result {
            Ok(0) => return Err(copy_failed()),
            Ok(read) if read <= wanted => {
                hasher.update(&buffer[..read]);
                read_bytes = read_bytes.checked_add(read).ok_or_else(copy_failed)?;
            }
            Ok(_) => return Err(copy_failed()),
            Err(error) if error == rustix::io::Errno::INTR => {
                interruptions = interruptions.checked_add(1).ok_or_else(copy_failed)?;
                if interruptions >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    return Err(copy_failed());
                }
            }
            Err(error) => return Err(map_precommit_io_error(error)),
        }
    }
    check_optional_cancellation(cancellation)?;
    let mut overflow = [0_u8; 1];
    loop {
        if calls >= MAX_COPY_FILE_IO_CALLS {
            return Err(copy_failed());
        }
        calls = calls.checked_add(1).ok_or_else(copy_failed)?;
        let result = read_file(&mut overflow);
        check_optional_cancellation(cancellation)?;
        match result {
            Ok(0) => break,
            Ok(_) => return Err(copy_failed()),
            Err(error) if error == rustix::io::Errno::INTR => {
                interruptions = interruptions.checked_add(1).ok_or_else(copy_failed)?;
                if interruptions >= MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    return Err(copy_failed());
                }
            }
            Err(error) => return Err(map_precommit_io_error(error)),
        }
    }
    Ok(hasher.finalize().into())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_staged<CleanupEvidence: CopyFileCleanupEvidence>(
    staged: &StagedFile<'_, CleanupEvidence>,
    expected_size: usize,
    expected_mode: Option<Mode>,
    expected_digest: [u8; 32],
    cancellation: &CancellationToken,
    buffer: &mut [u8],
) -> Result<(), ToolError> {
    let metadata = precommit_call(cancellation, || rustix::fs::fstat(&staged.file))?
        .map_err(|_| copy_failed())?;
    let acl_is_empty = precommit_call(cancellation, || staged_acl_is_empty(staged.file.as_fd()))??;
    if !FileType::from_raw_mode(metadata.st_mode).is_file()
        || FileIdentity::from_stat(&metadata) != staged.identity()
        || usize::try_from(metadata.st_size).ok() != Some(expected_size)
        || expected_mode.is_some_and(|mode| metadata.st_mode & 0o777 != mode.as_raw_mode())
        || !acl_is_empty
    {
        return Err(copy_failed());
    }
    let digest = hash_file(
        staged.file.as_fd(),
        expected_size,
        Some(cancellation),
        buffer,
    )?;
    check_cancellation(cancellation)?;
    if digest != expected_digest {
        return Err(copy_failed());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn revalidate_staged_path<CleanupEvidence: CopyFileCleanupEvidence>(
    parent: BorrowedFd<'_>,
    staged: &StagedFile<'_, CleanupEvidence>,
    expected_size: usize,
    expected_mode: Mode,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let path = precommit_call(cancellation, || {
        rustix::fs::statat(parent, &staged.name, AtFlags::SYMLINK_NOFOLLOW)
    })?
    .map_err(|_| target_changed())?;
    let descriptor = precommit_call(cancellation, || rustix::fs::fstat(&staged.file))?
        .map_err(|_| target_changed())?;
    if FileType::from_raw_mode(path.st_mode).is_file()
        && FileType::from_raw_mode(descriptor.st_mode).is_file()
        && FileIdentity::from_stat(&path) == staged.identity()
        && FileIdentity::from_stat(&descriptor) == staged.identity()
        && usize::try_from(path.st_size).ok() == Some(expected_size)
        && usize::try_from(descriptor.st_size).ok() == Some(expected_size)
        && path.st_mode & 0o777 == expected_mode.as_raw_mode()
        && descriptor.st_mode & 0o777 == expected_mode.as_raw_mode()
    {
        Ok(())
    } else {
        Err(target_changed())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_published<CleanupEvidence: CopyFileCleanupEvidence>(
    parent: BorrowedFd<'_>,
    basename: &str,
    staged: &StagedFile<'_, CleanupEvidence>,
    expected_size: usize,
    expected_mode: Mode,
    expected_digest: [u8; 32],
    buffer: &mut [u8],
) -> Result<(), ToolError> {
    let path = rustix::fs::statat(parent, basename, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| commit_ambiguous())?;
    let descriptor = rustix::fs::fstat(&staged.file).map_err(|_| commit_ambiguous())?;
    if !FileType::from_raw_mode(path.st_mode).is_file()
        || !FileType::from_raw_mode(descriptor.st_mode).is_file()
        || FileIdentity::from_stat(&path) != staged.identity()
        || FileIdentity::from_stat(&descriptor) != staged.identity()
        || usize::try_from(path.st_size).ok() != Some(expected_size)
        || usize::try_from(descriptor.st_size).ok() != Some(expected_size)
        || path.st_mode & 0o777 != expected_mode.as_raw_mode()
        || descriptor.st_mode & 0o777 != expected_mode.as_raw_mode()
        || !staged_acl_is_empty(staged.file.as_fd()).unwrap_or(false)
    {
        return Err(commit_ambiguous());
    }
    let digest = hash_file(staged.file.as_fd(), expected_size, None, buffer)
        .map_err(|_| commit_ambiguous())?;
    if digest != expected_digest {
        return Err(commit_ambiguous());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_precommit<Evidence: CopyFileEvidence>(
    file: BorrowedFd<'_>,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
) -> Result<(), ToolError> {
    sync_precommit_with(cancellation, |attempt| {
        evidence.sync(CopySyncSite::Staged, attempt, file)
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_precommit_with(
    cancellation: &CancellationToken,
    mut sync: impl FnMut(usize) -> Result<(), rustix::io::Errno>,
) -> Result<(), ToolError> {
    for attempt in 0..MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
        check_cancellation(cancellation)?;
        let result = sync(attempt);
        check_cancellation(cancellation)?;
        match result {
            Ok(()) => return Ok(()),
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(error) => return Err(map_precommit_io_error(error)),
        }
    }
    Err(copy_failed())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_postcommit<Evidence: CopyFileEvidence>(
    parent: BorrowedFd<'_>,
    evidence: &mut Evidence,
) -> Result<(), rustix::io::Errno> {
    sync_postcommit_with(|attempt| evidence.sync(CopySyncSite::DestinationParent, attempt, parent))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_postcommit_with(
    mut sync: impl FnMut(usize) -> Result<(), rustix::io::Errno>,
) -> Result<(), rustix::io::Errno> {
    for attempt in 0..MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
        match sync(attempt) {
            Ok(()) => return Ok(()),
            Err(error) if error == rustix::io::Errno::INTR => {}
            Err(error) => return Err(error),
        }
    }
    Err(rustix::io::Errno::INTR)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_root_is_linked(
    root: BorrowedFd<'_>,
    phase: WalkPhase,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    #[cfg(target_os = "linux")]
    {
        if precommit_call(cancellation, || rustix::fs::fstat(root))?
            .map_err(|_| map_parent_revalidation_failure(phase))?
            .st_nlink
            == 0
        {
            return Err(map_parent_revalidation_failure(phase));
        }
    }
    #[cfg(target_os = "macos")]
    ensure_macos_root_is_linked(root, phase, cancellation)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_macos_root_is_linked(
    root: BorrowedFd<'_>,
    phase: WalkPhase,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let root_metadata = precommit_call(cancellation, || rustix::fs::fstat(root))?
        .map_err(|_| map_parent_revalidation_failure(phase))?;
    let root_path = precommit_call(cancellation, || rustix::fs::getpath(root))?
        .map_err(|_| map_parent_revalidation_failure(phase))?;
    let root_path = root_path.as_bytes();
    if root_path == b"/" {
        return Ok(());
    }
    let name = root_path
        .rsplit(|byte| *byte == b'/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| map_parent_revalidation_failure(phase))?;
    let name = std::ffi::CString::new(name).map_err(|_| map_parent_revalidation_failure(phase))?;
    let parent = precommit_call(cancellation, || {
        rustix::fs::openat(root, "..", directory_open_flags(), Mode::empty())
    })?
    .map_err(|_| map_parent_revalidation_failure(phase))?;
    let linked = precommit_call(cancellation, || {
        rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW)
    })?
    .map_err(|_| map_parent_revalidation_failure(phase))?;
    if FileType::from_raw_mode(linked.st_mode).is_dir()
        && FileIdentity::from_stat(&root_metadata) == FileIdentity::from_stat(&linked)
    {
        Ok(())
    } else {
        Err(map_parent_revalidation_failure(phase))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> CopyFileToolOpenError {
    let kind = if is_rejected_type_error(error) {
        CopyFileToolOpenErrorKind::InvalidFileType
    } else {
        CopyFileToolOpenErrorKind::Unavailable
    };
    CopyFileToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_parent_open_error(error: rustix::io::Errno, phase: WalkPhase) -> ToolError {
    match phase {
        WalkPhase::Revalidate => target_changed(),
        WalkPhase::Initial if error == rustix::io::Errno::NOENT => not_found(),
        WalkPhase::Initial if is_permission_error(error) => permission_denied(),
        WalkPhase::Initial if is_rejected_type_error(error) => path_rejected(),
        WalkPhase::Initial => unavailable(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_parent_revalidation_failure(phase: WalkPhase) -> ToolError {
    match phase {
        WalkPhase::Initial => unavailable(),
        WalkPhase::Revalidate => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_parent_rejected(phase: WalkPhase) -> ToolError {
    match phase {
        WalkPhase::Initial => path_rejected(),
        WalkPhase::Revalidate => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_source_open_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::NOENT {
        target_changed()
    } else if is_permission_error(error) {
        permission_denied()
    } else if is_rejected_type_error(error) {
        path_rejected()
    } else {
        unavailable()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_precommit_io_error(error: rustix::io::Errno) -> ToolError {
    if is_permission_error(error) {
        permission_denied()
    } else {
        copy_failed()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_publish_error(error: rustix::io::Errno) -> ToolError {
    if error == rustix::io::Errno::EXIST {
        destination_exists()
    } else if is_permission_error(error) {
        permission_denied()
    } else if error == rustix::io::Errno::XDEV
        || error == rustix::io::Errno::NOSYS
        || error == rustix::io::Errno::NOTSUP
        || error == rustix::io::Errno::INVAL
    {
        unsupported_filesystem()
    } else if error == rustix::io::Errno::NOENT || is_rejected_type_error(error) {
        target_changed()
    } else {
        copy_failed()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_permission_error(error: rustix::io::Errno) -> bool {
    error == rustix::io::Errno::ACCESS
        || error == rustix::io::Errno::PERM
        || error == rustix::io::Errno::ROFS
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_rejected_type_error(error: rustix::io::Errno) -> bool {
    error == rustix::io::Errno::LOOP
        || error == rustix::io::Errno::NOTDIR
        || error == rustix::io::Errno::ISDIR
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn check_optional_cancellation(cancellation: Option<&CancellationToken>) -> Result<(), ToolError> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        Err(cancelled())
    } else {
        Ok(())
    }
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
        "copy_file_invalid_arguments",
        "copy_file arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "copy_file_invalid_path",
        "copy_file path is invalid",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "copy_file_unsupported_platform",
        "native copy_file is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "copy_file_not_found",
        "copy source or destination parent is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn source_too_large() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "copy_file_source_too_large",
        "copy source exceeds the supported size limit",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "copy_file_permission_denied",
        "requested copy is not permitted",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn path_rejected() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "copy_file_path_rejected",
        "requested copy path is not confined",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn destination_exists() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "copy_file_destination_exists",
        "copy destination already exists",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "copy_file_unavailable",
        "requested copy is unavailable",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn target_changed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "copy_file_target_changed",
        "copy paths changed before commit",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unsupported_filesystem() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "copy_file_unsupported_filesystem",
        "atomic no-replace copy publication is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn copy_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "copy_file_copy_failed",
        "requested file could not be copied",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn commit_ambiguous() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "copy_file_commit_ambiguous",
        "requested file copy status is uncertain",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "copy_file_cancelled",
        "copy_file execution was cancelled",
        false,
    )
}

impl fmt::Debug for CopyFileTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CopyFileTool")
            .finish_non_exhaustive()
    }
}

struct ValidatedArguments<'a> {
    requested_source: &'a str,
    requested_destination: &'a str,
    source: String,
    destination: String,
}

impl Tool for CopyFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: copy_file_name(),
            description: COPY_FILE_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": SOURCE_DESCRIPTION },
                    "destination": { "type": "string", "description": DESTINATION_DESCRIPTION }
                },
                "required": ["source", "destination"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != copy_file_name() {
            return Err(invalid_arguments());
        }
        let arguments = validate_arguments(&call.arguments)?;
        let prepared_arguments = json!({
            "source": arguments.source,
            "destination": arguments.destination,
        });
        if !serialized_value_fits(&prepared_arguments, MAX_COPY_FILE_SERIALIZED_ARGUMENT_BYTES) {
            return Err(invalid_arguments());
        }
        let source = prepared_arguments["source"]
            .as_str()
            .expect("prepared copy_file source is a string")
            .to_owned();
        let destination = prepared_arguments["destination"]
            .as_str()
            .expect("prepared copy_file destination is a string")
            .to_owned();
        Ok(PreparedToolCall::new(
            Capability::FilesystemCopy {
                source,
                destination,
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
            if arguments.source != arguments.requested_source
                || arguments.destination != arguments.requested_destination
            {
                return Err(invalid_arguments());
            }

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = cancellation;
                Err(unsupported_platform())
            }

            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.execute_supported(&arguments.source, &arguments.destination, &cancellation)
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
    let Some(Value::String(source)) = object.get("source") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(destination)) = object.get("destination") else {
        return Err(invalid_arguments());
    };
    let normalized_source = normalize_relative_path(source)?;
    let normalized_destination = normalize_relative_path(destination)?;
    if normalized_source == normalized_destination {
        return Err(invalid_arguments());
    }
    if !serialized_value_fits(arguments, MAX_COPY_FILE_SERIALIZED_ARGUMENT_BYTES) {
        return Err(invalid_arguments());
    }
    Ok(ValidatedArguments {
        requested_source: source,
        requested_destination: destination,
        source: normalized_source,
        destination: normalized_destination,
    })
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_COPY_FILE_PATH_BYTES
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
        if components > MAX_COPY_FILE_PATH_COMPONENTS {
            return Err(invalid_path());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() || normalized.len() > MAX_COPY_FILE_PATH_BYTES {
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

fn copy_file_name() -> ToolName {
    ToolName::new(COPY_FILE_TOOL_NAME).expect("copy_file is a valid tool name")
}

fn serialized_value_fits(value: &(impl serde::Serialize + ?Sized), limit: usize) -> bool {
    let mut writer = JsonByteCounter { written: 0, limit };
    serde_json::to_writer(&mut writer, value).is_ok()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn build_success_output(
    source: &str,
    destination: &str,
    bytes_copied: usize,
) -> Result<ToolOutput, ToolError> {
    let output = ToolOutput::success(json!({
        "source": source,
        "destination": destination,
        "bytes_copied": bytes_copied,
    }));
    if serialized_value_fits(&output, MAX_COPY_FILE_SERIALIZED_RESULT_BYTES) {
        Ok(output)
    } else {
        Err(copy_failed())
    }
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WalkPhase {
    Initial,
    Revalidate,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: i128,
    inode: i128,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FileIdentity {
    fn from_stat(metadata: &rustix::fs::Stat) -> Self {
        Self {
            device: i128::from(metadata.st_dev),
            inode: i128::from(metadata.st_ino),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourceFingerprint {
    identity: FileIdentity,
    size: usize,
    mode: Mode,
    modified_seconds: i128,
    modified_nanoseconds: i128,
    changed_seconds: i128,
    changed_nanoseconds: i128,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl SourceFingerprint {
    fn from_stat(metadata: &rustix::fs::Stat) -> Result<Self, ToolError> {
        if !FileType::from_raw_mode(metadata.st_mode).is_file() {
            return Err(path_rejected());
        }
        let size = usize::try_from(metadata.st_size).map_err(|_| copy_failed())?;
        if size > MAX_COPY_FILE_SOURCE_BYTES {
            return Err(source_too_large());
        }
        Ok(Self {
            identity: FileIdentity::from_stat(metadata),
            size,
            mode: Mode::from_raw_mode(metadata.st_mode & 0o777),
            modified_seconds: i128::from(metadata.st_mtime),
            modified_nanoseconds: i128::from(metadata.st_mtime_nsec),
            changed_seconds: i128::from(metadata.st_ctime),
            changed_nanoseconds: i128::from(metadata.st_ctime_nsec),
        })
    }

    fn matches_stat(self, metadata: &rustix::fs::Stat) -> bool {
        FileType::from_raw_mode(metadata.st_mode).is_file()
            && self.identity == FileIdentity::from_stat(metadata)
            && usize::try_from(metadata.st_size).ok() == Some(self.size)
            && Mode::from_raw_mode(metadata.st_mode & 0o777) == self.mode
            && i128::from(metadata.st_mtime) == self.modified_seconds
            && i128::from(metadata.st_mtime_nsec) == self.modified_nanoseconds
            && i128::from(metadata.st_ctime) == self.changed_seconds
            && i128::from(metadata.st_ctime_nsec) == self.changed_nanoseconds
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ParentWalk<'path> {
    parent: OwnedFd,
    identity: FileIdentity,
    basename: &'path str,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct RetainedSource {
    file: OwnedFd,
    fingerprint: SourceFingerprint,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct StagedFile<'parent, CleanupEvidence: CopyFileCleanupEvidence> {
    cleanup_parent: BorrowedFd<'parent>,
    cleanup_evidence: &'parent CleanupEvidence,
    file: OwnedFd,
    name: String,
    identity: Option<FileIdentity>,
    published: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'parent, CleanupEvidence: CopyFileCleanupEvidence> StagedFile<'parent, CleanupEvidence> {
    fn new(
        cleanup_parent: BorrowedFd<'parent>,
        cleanup_evidence: &'parent CleanupEvidence,
        file: OwnedFd,
        name: String,
    ) -> Self {
        Self {
            cleanup_parent,
            cleanup_evidence,
            file,
            name,
            identity: None,
            published: false,
        }
    }

    fn initialize<Evidence: CopyFileEvidence>(
        &mut self,
        cancellation: &CancellationToken,
        evidence: &mut Evidence,
    ) -> Result<(), ToolError> {
        let metadata = precommit_call(cancellation, || {
            evidence.initial_stage_metadata(self.file.as_fd())
        })?
        .map_err(|_| copy_failed())?;
        if !FileType::from_raw_mode(metadata.st_mode).is_file()
            || usize::try_from(metadata.st_size).ok() != Some(0)
        {
            return Err(copy_failed());
        }
        self.identity = Some(FileIdentity::from_stat(&metadata));
        Ok(())
    }

    fn identity(&self) -> FileIdentity {
        self.identity
            .expect("staged file identity is initialized before use")
    }

    fn mark_published(&mut self) {
        self.published = true;
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<CleanupEvidence: CopyFileCleanupEvidence> Drop for StagedFile<'_, CleanupEvidence> {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        cleanup_unpublished_stage(
            self.cleanup_parent,
            self.file.as_fd(),
            &self.name,
            self.identity,
            self.cleanup_evidence,
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_unpublished_stage<CleanupEvidence: CopyFileCleanupEvidence>(
    parent: BorrowedFd<'_>,
    file: BorrowedFd<'_>,
    name: &str,
    identity: Option<FileIdentity>,
    evidence: &CleanupEvidence,
) {
    let _ = evidence.set_mode(file, Mode::from_raw_mode(0o600));
    let Ok(descriptor) = evidence.fstat(file) else {
        return;
    };
    let Ok(path) = evidence.statat(parent, name) else {
        return;
    };
    let descriptor_identity = FileIdentity::from_stat(&descriptor);
    if FileType::from_raw_mode(descriptor.st_mode).is_file()
        && FileType::from_raw_mode(path.st_mode).is_file()
        && FileIdentity::from_stat(&path) == descriptor_identity
        && identity.is_none_or(|expected| descriptor_identity == expected)
    {
        let _ = evidence.unlink(parent, name);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CopyFileTool {
    #[allow(clippy::too_many_lines)]
    fn execute_supported(
        &self,
        source_path: &str,
        destination_path: &str,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let mut evidence = NativeCopyFileEvidence;
        let cleanup_evidence = NativeCopyFileCleanupEvidence;
        self.execute_supported_with_evidence(
            source_path,
            destination_path,
            cancellation,
            &mut evidence,
            &cleanup_evidence,
        )
    }

    #[allow(clippy::too_many_lines)]
    fn execute_supported_with_evidence<
        Evidence: CopyFileEvidence,
        CleanupEvidence: CopyFileCleanupEvidence,
    >(
        &self,
        source_path: &str,
        destination_path: &str,
        cancellation: &CancellationToken,
        evidence: &mut Evidence,
        cleanup_evidence: &CleanupEvidence,
    ) -> Result<ToolOutput, ToolError> {
        check_cancellation(cancellation)?;
        let mut buffer = vec![0_u8; MAX_COPY_FILE_CHUNK_BYTES].into_boxed_slice();

        let initial_source_parent =
            self.walk_parent(source_path, WalkPhase::Initial, cancellation)?;
        precommit_checkpoint(
            evidence,
            CopyCheckpoint::AfterInitialSourceParent,
            cancellation,
        )?;
        let source = retain_source(
            initial_source_parent.parent.as_fd(),
            initial_source_parent.basename,
            cancellation,
        )?;
        precommit_checkpoint(evidence, CopyCheckpoint::AfterSourceRetained, cancellation)?;
        let success = build_success_output(source_path, destination_path, source.fingerprint.size)?;
        let initial_destination_parent =
            self.walk_parent(destination_path, WalkPhase::Initial, cancellation)?;
        precommit_checkpoint(
            evidence,
            CopyCheckpoint::AfterInitialDestinationParent,
            cancellation,
        )?;
        require_destination_absent(
            initial_destination_parent.parent.as_fd(),
            initial_destination_parent.basename,
            WalkPhase::Initial,
            cancellation,
        )?;
        precommit_checkpoint(
            evidence,
            CopyCheckpoint::AfterDestinationAbsent,
            cancellation,
        )?;

        let mut staged = create_staged_file(
            initial_destination_parent.parent.as_fd(),
            initial_destination_parent.basename,
            cancellation,
            evidence,
            cleanup_evidence,
        )?;
        precommit_checkpoint(evidence, CopyCheckpoint::AfterStageCreated, cancellation)?;
        precommit_call(cancellation, || {
            rustix::fs::fchmod(&staged.file, Mode::from_raw_mode(0o600))
        })?
        .map_err(map_precommit_io_error)?;
        precommit_call(cancellation, || clear_staged_acl(staged.file.as_fd()))??;
        if !precommit_call(cancellation, || staged_acl_is_empty(staged.file.as_fd()))?? {
            return Err(copy_failed());
        }

        evidence.observe_buffer(CopyBufferSite::Copy, &buffer);
        let source_digest = stream_source_to_stage(
            source.file.as_fd(),
            staged.file.as_fd(),
            source.fingerprint.size,
            cancellation,
            &mut buffer,
        )?;
        precommit_checkpoint(evidence, CopyCheckpoint::AfterCopy, cancellation)?;
        let source_after_copy = precommit_call(cancellation, || rustix::fs::fstat(&source.file))?
            .map_err(|_| target_changed())?;
        if !source.fingerprint.matches_stat(&source_after_copy) {
            return Err(target_changed());
        }
        evidence.observe_buffer(CopyBufferSite::InitialStageHash, &buffer);
        verify_staged(
            &staged,
            source.fingerprint.size,
            None,
            source_digest,
            cancellation,
            &mut buffer,
        )?;
        precommit_checkpoint(
            evidence,
            CopyCheckpoint::AfterInitialStageVerification,
            cancellation,
        )?;

        let final_mode = source.fingerprint.mode;
        precommit_call(cancellation, || {
            rustix::fs::fchmod(&staged.file, final_mode)
        })?
        .map_err(map_precommit_io_error)?;
        sync_precommit(staged.file.as_fd(), cancellation, evidence)?;
        evidence.observe_buffer(CopyBufferSite::FinalStageHash, &buffer);
        verify_staged(
            &staged,
            source.fingerprint.size,
            Some(final_mode),
            source_digest,
            cancellation,
            &mut buffer,
        )?;
        precommit_checkpoint(
            evidence,
            CopyCheckpoint::AfterFinalStageVerification,
            cancellation,
        )?;

        let final_source_parent =
            self.walk_parent(source_path, WalkPhase::Revalidate, cancellation)?;
        precommit_checkpoint(
            evidence,
            CopyCheckpoint::AfterFinalSourceParent,
            cancellation,
        )?;
        if final_source_parent.identity != initial_source_parent.identity {
            return Err(target_changed());
        }
        revalidate_source_path(
            final_source_parent.parent.as_fd(),
            final_source_parent.basename,
            &source,
            Some(cancellation),
        )?;
        precommit_checkpoint(
            evidence,
            CopyCheckpoint::AfterFinalSourceValidation,
            cancellation,
        )?;
        let final_destination_parent =
            self.walk_parent(destination_path, WalkPhase::Revalidate, cancellation)?;
        precommit_checkpoint(
            evidence,
            CopyCheckpoint::AfterFinalDestinationParent,
            cancellation,
        )?;
        if final_destination_parent.identity != initial_destination_parent.identity {
            return Err(target_changed());
        }
        require_destination_absent(
            final_destination_parent.parent.as_fd(),
            final_destination_parent.basename,
            WalkPhase::Revalidate,
            cancellation,
        )?;
        precommit_checkpoint(
            evidence,
            CopyCheckpoint::AfterFinalDestinationValidation,
            cancellation,
        )?;
        revalidate_staged_path(
            initial_destination_parent.parent.as_fd(),
            &staged,
            source.fingerprint.size,
            final_mode,
            cancellation,
        )?;
        precommit_checkpoint(
            evidence,
            CopyCheckpoint::AfterFinalStageValidation,
            cancellation,
        )?;

        precommit_checkpoint(evidence, CopyCheckpoint::FinalPrePublish, cancellation)?;
        let publish = evidence.publish(
            initial_destination_parent.parent.as_fd(),
            &staged.name,
            final_destination_parent.parent.as_fd(),
            final_destination_parent.basename,
        );
        evidence.after_publish(publish, cancellation);
        evidence.checkpoint(CopyCheckpoint::AfterPublish, cancellation);
        match publish {
            Ok(()) => {
                staged.mark_published();
                evidence.observe_buffer(CopyBufferSite::PublishedHash, &buffer);
                let published = verify_published(
                    final_destination_parent.parent.as_fd(),
                    final_destination_parent.basename,
                    &staged,
                    source.fingerprint.size,
                    final_mode,
                    source_digest,
                    &mut buffer,
                );
                let postcommit_cancellation = CancellationToken::new();
                let source_stable = self
                    .walk_parent(source_path, WalkPhase::Revalidate, &postcommit_cancellation)
                    .and_then(|postcommit_source_parent| {
                        if postcommit_source_parent.identity != initial_source_parent.identity {
                            return Err(target_changed());
                        }
                        revalidate_source_path(
                            postcommit_source_parent.parent.as_fd(),
                            postcommit_source_parent.basename,
                            &source,
                            None,
                        )
                    });
                let synced = sync_postcommit(final_destination_parent.parent.as_fd(), evidence);
                if published.is_err() || source_stable.is_err() || synced.is_err() {
                    return Err(commit_ambiguous());
                }
            }
            Err(error) if error == rustix::io::Errno::INTR => {
                let _ = sync_postcommit(final_destination_parent.parent.as_fd(), evidence);
                return Err(commit_ambiguous());
            }
            Err(error) => {
                check_cancellation(cancellation)?;
                return Err(map_publish_error(error));
            }
        }

        Ok(success)
    }

    fn walk_parent<'path>(
        &self,
        path: &'path str,
        phase: WalkPhase,
        cancellation: &CancellationToken,
    ) -> Result<ParentWalk<'path>, ToolError> {
        let mut parent = precommit_call(cancellation, || {
            rustix::fs::openat(
                self.root.as_fd(),
                ".",
                directory_open_flags(),
                Mode::empty(),
            )
        })?
        .map_err(|error| map_parent_open_error(error, phase))?;
        ensure_root_is_linked(parent.as_fd(), phase, cancellation)?;

        let mut components = path.split('/').peekable();
        loop {
            check_cancellation(cancellation)?;
            let component = components.next().ok_or_else(invalid_arguments)?;
            if components.peek().is_none() {
                let metadata = precommit_call(cancellation, || rustix::fs::fstat(&parent))?
                    .map_err(|_| map_parent_revalidation_failure(phase))?;
                if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                    return Err(map_parent_rejected(phase));
                }
                return Ok(ParentWalk {
                    parent,
                    identity: FileIdentity::from_stat(&metadata),
                    basename: component,
                });
            }
            parent = precommit_call(cancellation, || {
                rustix::fs::openat(
                    parent.as_fd(),
                    component,
                    directory_open_flags(),
                    Mode::empty(),
                )
            })?
            .map_err(|error| map_parent_open_error(error, phase))?;
            let metadata = precommit_call(cancellation, || rustix::fs::fstat(&parent))?
                .map_err(|_| map_parent_revalidation_failure(phase))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(map_parent_rejected(phase));
            }
        }
    }
}
