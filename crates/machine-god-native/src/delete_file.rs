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
use std::ffi::OsStr;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{AtFlags, FileType, Mode, OFlags};

/// Maximum UTF-8 bytes accepted in a requested or normalized target path.
pub const MAX_DELETE_FILE_PATH_BYTES: usize = 4 * 1024;
/// Maximum components accepted in a normalized target path.
pub const MAX_DELETE_FILE_PATH_COMPONENTS: usize = 256;
/// Maximum serialized byte size of accepted arguments.
pub const MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES: usize = 64 * 1024;
/// Maximum serialized [`ToolOutput`] bytes produced by this tool.
pub const MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES: usize = 16 * 1024;

/// Registered name of [`DeleteFileTool`].
pub const DELETE_FILE_TOOL_NAME: &str = "delete_file";

const DELETE_FILE_DESCRIPTION: &str =
    "Delete one regular file or empty directory within the configured workspace";
const PATH_DESCRIPTION: &str = "Workspace-relative file or empty-directory path";

#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_PARENT_SYNC_CALLS: usize = 16;

/// Stable category for failure to acquire a deletable workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteFileToolOpenErrorKind {
    /// Native delete execution is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`DeleteFileTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct DeleteFileToolOpenError {
    kind: DeleteFileToolOpenErrorKind,
}

impl DeleteFileToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> DeleteFileToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: DeleteFileToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for DeleteFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeleteFileToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for DeleteFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            DeleteFileToolOpenErrorKind::UnsupportedPlatform => {
                "native delete_file is unsupported on this platform"
            }
            DeleteFileToolOpenErrorKind::InvalidRoot => {
                "native delete_file workspace root is invalid"
            }
            DeleteFileToolOpenErrorKind::InvalidFileType => {
                "native delete_file workspace root is not a directory"
            }
            DeleteFileToolOpenErrorKind::Unavailable => {
                "native delete_file workspace root is unavailable"
            }
        })
    }
}

impl Error for DeleteFileToolOpenError {}

/// A one-entry native deleter confined to an explicitly opened workspace root.
///
/// Supported Linux and macOS implementations retain the opened root descriptor
/// and never reopen its injected pathname. Each call validates one regular file
/// or directory twice before issuing exactly one `unlinkat`. This is not a
/// pathname compare-and-swap: after final validation, a file-class delete can
/// remove any replacement non-directory entry accepted by empty `unlinkat`
/// flags, including a regular file, symlink, FIFO, socket, or special entry,
/// without following a symlink referent. A directory-class delete can remove a
/// different empty directory. File/directory flag mismatches fail.
pub struct DeleteFileTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl DeleteFileTool {
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
    pub fn open(root: &Path) -> Result<Self, DeleteFileToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(DeleteFileToolOpenError::new(
                DeleteFileToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(DeleteFileToolOpenError::new(
                    DeleteFileToolOpenErrorKind::InvalidRoot,
                ));
            }
            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor).map_err(|_| {
                DeleteFileToolOpenError::new(DeleteFileToolOpenErrorKind::Unavailable)
            })?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(DeleteFileToolOpenError::new(
                    DeleteFileToolOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for DeleteFileTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeleteFileTool")
            .finish_non_exhaustive()
    }
}

struct ValidatedArguments<'a> {
    requested_path: &'a str,
    path: String,
}

impl Tool for DeleteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: delete_file_name(),
            description: DELETE_FILE_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": PATH_DESCRIPTION }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != delete_file_name() {
            return Err(invalid_arguments());
        }
        let arguments = validate_arguments(&call.arguments)?;
        let prepared_arguments = json!({ "path": arguments.path });
        if !serialized_value_fits(
            &prepared_arguments,
            MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES,
        ) {
            return Err(invalid_arguments());
        }
        let path = prepared_arguments["path"]
            .as_str()
            .expect("prepared delete_file path is a string")
            .to_owned();
        Ok(PreparedToolCall::new(
            Capability::Filesystem {
                access: FilesystemAccess::Delete,
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
                self.execute_supported(&arguments.path, &cancellation)
            }
        })
    }
}

fn validate_arguments(arguments: &Value) -> Result<ValidatedArguments<'_>, ToolError> {
    let Value::Object(object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() != 1 {
        return Err(invalid_arguments());
    }
    let Some(Value::String(path)) = object.get("path") else {
        return Err(invalid_arguments());
    };
    let normalized = normalize_relative_path(path)?;
    if !serialized_value_fits(arguments, MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES) {
        return Err(invalid_arguments());
    }
    Ok(ValidatedArguments {
        requested_path: path,
        path: normalized,
    })
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_DELETE_FILE_PATH_BYTES
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
        if components > MAX_DELETE_FILE_PATH_COMPONENTS {
            return Err(invalid_path());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() || normalized.len() > MAX_DELETE_FILE_PATH_BYTES {
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

fn delete_file_name() -> ToolName {
    ToolName::new(DELETE_FILE_TOOL_NAME).expect("delete_file is a valid tool name")
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeletePhase {
    Initial,
    Revalidate,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OpenSite {
    Root,
    #[cfg(target_os = "macos")]
    RootParent,
    Intermediate(usize),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FstatSite {
    Root,
    Intermediate(usize),
    FinalParent,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatatSite {
    #[cfg(target_os = "macos")]
    LinkedRoot,
    Target,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeleteCheckpoint {
    BeforeOpen(DeletePhase, OpenSite, usize),
    AfterOpen(DeletePhase, OpenSite, usize),
    BeforeFstat(DeletePhase, FstatSite, usize),
    AfterFstat(DeletePhase, FstatSite, usize),
    BeforeStatat(DeletePhase, StatatSite, usize),
    AfterStatat(DeletePhase, StatatSite, usize),
    AfterRootValidation(DeletePhase),
    AfterValidation(DeletePhase),
    FinalPreUnlink,
    AfterDelete,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TargetKind {
    RegularFile,
    Directory,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) trait DeleteFileEvidence {
    fn checkpoint(&mut self, _checkpoint: DeleteCheckpoint, _cancellation: &CancellationToken) {}

    fn open_walk(
        &mut self,
        _phase: DeletePhase,
        _site: OpenSite,
        _ordinal: usize,
        parent: BorrowedFd<'_>,
        component: &OsStr,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty())
    }

    fn fstat(
        &mut self,
        _phase: DeletePhase,
        _site: FstatSite,
        _ordinal: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::fstat(descriptor)
    }

    fn statat(
        &mut self,
        _phase: DeletePhase,
        _site: StatatSite,
        _ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn after_validation(
        &mut self,
        _phase: DeletePhase,
        _parent: BorrowedFd<'_>,
        _basename: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        Ok(())
    }

    fn final_pre_unlink(
        &mut self,
        _parent: BorrowedFd<'_>,
        _basename: &str,
        _kind: TargetKind,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        Ok(())
    }

    fn unlink(
        &mut self,
        parent: BorrowedFd<'_>,
        basename: &str,
        flags: AtFlags,
    ) -> Result<(), rustix::io::Errno> {
        rustix::fs::unlinkat(parent, basename, flags)
    }

    fn after_unlink(
        &mut self,
        _parent: BorrowedFd<'_>,
        _basename: &str,
        _kind: TargetKind,
        _flags: AtFlags,
        _outcome: Result<(), rustix::io::Errno>,
        _cancellation: &CancellationToken,
    ) -> Result<(), rustix::io::Errno> {
        Ok(())
    }

    fn sync_parent(
        &mut self,
        _attempt: usize,
        parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        rustix::fs::fsync(parent)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NativeDeleteFileEvidence;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl DeleteFileEvidence for NativeDeleteFileEvidence {}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct OperationOrdinals {
    open: usize,
    fstat: usize,
    statat: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
enum EvidenceOperationError {
    Cancelled,
    Os(rustix::io::Errno),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl OperationOrdinals {
    fn next_open(&mut self) -> usize {
        let ordinal = self.open;
        self.open = self.open.saturating_add(1);
        ordinal
    }

    fn next_fstat(&mut self) -> usize {
        let ordinal = self.fstat;
        self.fstat = self.fstat.saturating_add(1);
        ordinal
    }

    fn next_statat(&mut self) -> usize {
        let ordinal = self.statat;
        self.statat = self.statat.saturating_add(1);
        ordinal
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DirectoryIdentity {
    device: i128,
    inode: i128,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl DirectoryIdentity {
    fn from_stat(metadata: &rustix::fs::Stat) -> Result<Self, ()> {
        if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
            return Err(());
        }
        Ok(Self {
            device: i128::from(metadata.st_dev),
            inode: i128::from(metadata.st_ino),
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TargetIdentity {
    device: i128,
    inode: i128,
    kind: TargetKind,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl TargetIdentity {
    fn from_stat(metadata: &rustix::fs::Stat) -> Result<Self, ()> {
        let file_type = FileType::from_raw_mode(metadata.st_mode);
        let kind = if file_type.is_file() {
            TargetKind::RegularFile
        } else if file_type.is_dir() {
            TargetKind::Directory
        } else {
            return Err(());
        };
        Ok(Self {
            device: i128::from(metadata.st_dev),
            inode: i128::from(metadata.st_ino),
            kind,
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ParentWalk<'path> {
    parent: OwnedFd,
    parent_identity: DirectoryIdentity,
    basename: &'path str,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl DeleteFileTool {
    fn execute_supported(
        &self,
        normalized: &str,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let mut evidence = NativeDeleteFileEvidence;
        self.execute_supported_with_evidence(normalized, cancellation, &mut evidence)
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn execute_supported_with_evidence<Evidence: DeleteFileEvidence>(
        &self,
        normalized: &str,
        cancellation: &CancellationToken,
        evidence: &mut Evidence,
    ) -> Result<ToolOutput, ToolError> {
        let success_output =
            build_success_output_with_limit(normalized, MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES)?;
        let mut ordinals = OperationOrdinals::default();

        let initial = self.walk_parent_with_evidence(
            normalized,
            cancellation,
            DeletePhase::Initial,
            evidence,
            &mut ordinals,
        )?;
        let initial_target = inspect_target_with_evidence(
            initial.parent.as_fd(),
            initial.basename,
            cancellation,
            DeletePhase::Initial,
            evidence,
            &mut ordinals,
        )?;
        evidence.after_validation(
            DeletePhase::Initial,
            initial.parent.as_fd(),
            initial.basename,
            cancellation,
        )?;
        evidence.checkpoint(
            DeleteCheckpoint::AfterValidation(DeletePhase::Initial),
            cancellation,
        );
        check_cancellation(cancellation)?;

        let revalidated = self.walk_parent_with_evidence(
            normalized,
            cancellation,
            DeletePhase::Revalidate,
            evidence,
            &mut ordinals,
        )?;
        if revalidated.parent_identity != initial.parent_identity {
            return Err(target_changed());
        }
        let final_target = inspect_target_with_evidence(
            revalidated.parent.as_fd(),
            revalidated.basename,
            cancellation,
            DeletePhase::Revalidate,
            evidence,
            &mut ordinals,
        )?;
        if final_target != initial_target {
            return Err(target_changed());
        }
        evidence.after_validation(
            DeletePhase::Revalidate,
            revalidated.parent.as_fd(),
            revalidated.basename,
            cancellation,
        )?;
        evidence.checkpoint(
            DeleteCheckpoint::AfterValidation(DeletePhase::Revalidate),
            cancellation,
        );
        check_cancellation(cancellation)?;

        evidence.final_pre_unlink(
            revalidated.parent.as_fd(),
            revalidated.basename,
            final_target.kind,
            cancellation,
        )?;
        evidence.checkpoint(DeleteCheckpoint::FinalPreUnlink, cancellation);
        check_cancellation(cancellation)?;

        let flags = match final_target.kind {
            TargetKind::RegularFile => AtFlags::empty(),
            TargetKind::Directory => AtFlags::REMOVEDIR,
        };
        let unlink_outcome =
            evidence.unlink(revalidated.parent.as_fd(), revalidated.basename, flags);
        let after_unlink = evidence.after_unlink(
            revalidated.parent.as_fd(),
            revalidated.basename,
            final_target.kind,
            flags,
            unlink_outcome,
            cancellation,
        );

        match unlink_outcome {
            Ok(()) => {
                evidence.checkpoint(DeleteCheckpoint::AfterDelete, cancellation);
                let sync = sync_parent_bounded(revalidated.parent.as_fd(), evidence);
                if after_unlink.is_err() || sync.is_err() {
                    return Err(commit_ambiguous());
                }
                debug_assert!(serialized_value_fits(
                    &success_output,
                    MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES
                ));
                Ok(success_output)
            }
            Err(error) if error == rustix::io::Errno::INTR => {
                evidence.checkpoint(DeleteCheckpoint::AfterDelete, cancellation);
                let _ = sync_parent_bounded(revalidated.parent.as_fd(), evidence);
                Err(commit_ambiguous())
            }
            Err(error) => Err(map_unlink_error_with_evidence(
                error,
                final_target,
                revalidated.parent.as_fd(),
                revalidated.basename,
                cancellation,
                evidence,
                &mut ordinals,
            )),
        }
    }

    fn walk_parent_with_evidence<'path, Evidence: DeleteFileEvidence>(
        &self,
        normalized: &'path str,
        cancellation: &CancellationToken,
        phase: DeletePhase,
        evidence: &mut Evidence,
        ordinals: &mut OperationOrdinals,
    ) -> Result<ParentWalk<'path>, ToolError> {
        let mut parent = evidence_open_walk(
            self.root.as_fd(),
            OsStr::new("."),
            phase,
            OpenSite::Root,
            cancellation,
            evidence,
            ordinals,
        )
        .map_err(|error| map_evidence_walk_error(error, phase))?;
        validate_linked_root(parent.as_fd(), phase, cancellation, evidence, ordinals)?;
        precommit_checkpoint(
            evidence,
            DeleteCheckpoint::AfterRootValidation(phase),
            cancellation,
        )?;

        let mut components = normalized.split('/').peekable();
        let mut depth = 0_usize;
        loop {
            check_cancellation(cancellation)?;
            let component = components.next().ok_or_else(invalid_arguments)?;
            if components.peek().is_none() {
                let parent_metadata = evidence_fstat(
                    parent.as_fd(),
                    phase,
                    FstatSite::FinalParent,
                    cancellation,
                    evidence,
                    ordinals,
                )
                .map_err(|error| map_evidence_metadata_error(error, phase))?;
                let parent_identity = DirectoryIdentity::from_stat(&parent_metadata)
                    .map_err(|()| map_rejected_phase(phase))?;
                return Ok(ParentWalk {
                    parent,
                    parent_identity,
                    basename: component,
                });
            }

            let site = OpenSite::Intermediate(depth);
            parent = evidence_open_walk(
                parent.as_fd(),
                OsStr::new(component),
                phase,
                site,
                cancellation,
                evidence,
                ordinals,
            )
            .map_err(|error| map_evidence_parent_open_error(error, phase))?;
            let metadata = evidence_fstat(
                parent.as_fd(),
                phase,
                FstatSite::Intermediate(depth),
                cancellation,
                evidence,
                ordinals,
            )
            .map_err(|error| map_evidence_metadata_error(error, phase))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(map_rejected_phase(phase));
            }
            depth = depth
                .checked_add(1)
                .ok_or_else(|| map_operational_phase(phase))?;
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn inspect_target_with_evidence<Evidence: DeleteFileEvidence>(
    parent: BorrowedFd<'_>,
    basename: &str,
    cancellation: &CancellationToken,
    phase: DeletePhase,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<TargetIdentity, ToolError> {
    let metadata = evidence_statat(
        parent,
        OsStr::new(basename),
        phase,
        StatatSite::Target,
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|error| map_evidence_target_error(error, phase))?;
    TargetIdentity::from_stat(&metadata).map_err(|()| map_rejected_phase(phase))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_linked_root<Evidence: DeleteFileEvidence>(
    root: BorrowedFd<'_>,
    phase: DeletePhase,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<(), ToolError> {
    let root_metadata = evidence_fstat(
        root,
        phase,
        FstatSite::Root,
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|error| map_linked_root_metadata_error(error, phase))?;
    if !FileType::from_raw_mode(root_metadata.st_mode).is_dir() {
        return Err(map_rejected_phase(phase));
    }

    #[cfg(target_os = "linux")]
    if root_metadata.st_nlink == 0 {
        return Err(map_operational_phase(phase));
    }

    #[cfg(target_os = "macos")]
    validate_linked_macos_root(
        root,
        &root_metadata,
        phase,
        cancellation,
        evidence,
        ordinals,
    )?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_linked_macos_root<Evidence: DeleteFileEvidence>(
    root: BorrowedFd<'_>,
    root_metadata: &rustix::fs::Stat,
    phase: DeletePhase,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<(), ToolError> {
    check_cancellation(cancellation)?;
    let root_path = rustix::fs::getpath(root);
    check_cancellation(cancellation)?;
    let root_path = root_path.map_err(|error| map_walk_error(error, phase))?;
    let root_path = root_path.as_bytes();
    if root_path == b"/" {
        return Ok(());
    }
    let name = root_path
        .rsplit(|byte| *byte == b'/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| map_operational_phase(phase))?;
    let site = OpenSite::RootParent;
    let root_parent = evidence_open_walk(
        root,
        OsStr::new(".."),
        phase,
        site,
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|error| map_evidence_walk_error(error, phase))?;
    let linked = evidence_statat(
        root_parent.as_fd(),
        OsStr::from_bytes(name),
        phase,
        StatatSite::LinkedRoot,
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|error| map_linked_root_metadata_error(error, phase))?;
    if linked.st_dev != root_metadata.st_dev
        || linked.st_ino != root_metadata.st_ino
        || !FileType::from_raw_mode(linked.st_mode).is_dir()
    {
        return Err(map_operational_phase(phase));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn evidence_fstat<Evidence: DeleteFileEvidence>(
    descriptor: BorrowedFd<'_>,
    phase: DeletePhase,
    site: FstatSite,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<rustix::fs::Stat, EvidenceOperationError> {
    let ordinal = ordinals.next_fstat();
    evidence.checkpoint(
        DeleteCheckpoint::BeforeFstat(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    let metadata = evidence.fstat(phase, site, ordinal, descriptor);
    evidence.checkpoint(
        DeleteCheckpoint::AfterFstat(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    metadata.map_err(EvidenceOperationError::Os)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn evidence_statat<Evidence: DeleteFileEvidence>(
    parent: BorrowedFd<'_>,
    name: &OsStr,
    phase: DeletePhase,
    site: StatatSite,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<rustix::fs::Stat, EvidenceOperationError> {
    let ordinal = ordinals.next_statat();
    evidence.checkpoint(
        DeleteCheckpoint::BeforeStatat(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    let metadata = evidence.statat(phase, site, ordinal, parent, name);
    evidence.checkpoint(
        DeleteCheckpoint::AfterStatat(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    metadata.map_err(EvidenceOperationError::Os)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn evidence_open_walk<Evidence: DeleteFileEvidence>(
    parent: BorrowedFd<'_>,
    component: &OsStr,
    phase: DeletePhase,
    site: OpenSite,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<OwnedFd, EvidenceOperationError> {
    let ordinal = ordinals.next_open();
    evidence.checkpoint(
        DeleteCheckpoint::BeforeOpen(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    let descriptor = evidence.open_walk(phase, site, ordinal, parent, component);
    evidence.checkpoint(
        DeleteCheckpoint::AfterOpen(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    descriptor.map_err(EvidenceOperationError::Os)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn precommit_checkpoint<Evidence: DeleteFileEvidence>(
    evidence: &mut Evidence,
    checkpoint: DeleteCheckpoint,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    evidence.checkpoint(checkpoint, cancellation);
    check_cancellation(cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_parent_bounded<Evidence: DeleteFileEvidence>(
    parent: BorrowedFd<'_>,
    evidence: &mut Evidence,
) -> Result<(), ()> {
    for attempt in 0..MAX_PARENT_SYNC_CALLS {
        match evidence.sync_parent(attempt, parent) {
            Ok(()) => return Ok(()),
            Err(error)
                if error == rustix::io::Errno::INTR && attempt + 1 < MAX_PARENT_SYNC_CALLS => {}
            Err(_) => return Err(()),
        }
    }
    Err(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn build_success_output_with_limit(
    normalized: &str,
    limit: usize,
) -> Result<ToolOutput, ToolError> {
    let output = ToolOutput::success(json!({ "path": normalized }));
    if !serialized_value_fits(&output, limit) {
        return Err(delete_failed());
    }
    Ok(output)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> DeleteFileToolOpenError {
    let kind = if is_rejected_type_error(error) {
        DeleteFileToolOpenErrorKind::InvalidFileType
    } else {
        DeleteFileToolOpenErrorKind::Unavailable
    };
    DeleteFileToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_walk_error(error: rustix::io::Errno, phase: DeletePhase) -> ToolError {
    match phase {
        _ if is_permission_error(error) => permission_denied(),
        DeletePhase::Revalidate => target_changed(),
        DeletePhase::Initial => unavailable(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_parent_open_error(error: rustix::io::Errno, phase: DeletePhase) -> ToolError {
    match phase {
        _ if is_permission_error(error) => permission_denied(),
        DeletePhase::Revalidate => target_changed(),
        DeletePhase::Initial if error == rustix::io::Errno::NOENT => not_found(),
        DeletePhase::Initial if is_rejected_type_error(error) => path_rejected(),
        DeletePhase::Initial => unavailable(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_evidence_walk_error(error: EvidenceOperationError, phase: DeletePhase) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error) => map_walk_error(error, phase),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_evidence_parent_open_error(error: EvidenceOperationError, phase: DeletePhase) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error) => map_parent_open_error(error, phase),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_evidence_metadata_error(error: EvidenceOperationError, phase: DeletePhase) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error) if is_permission_error(error) => permission_denied(),
        EvidenceOperationError::Os(_) => map_operational_phase(phase),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_linked_root_metadata_error(error: EvidenceOperationError, phase: DeletePhase) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error) if is_permission_error(error) => permission_denied(),
        EvidenceOperationError::Os(_) => map_operational_phase(phase),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_evidence_target_error(error: EvidenceOperationError, phase: DeletePhase) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error) => map_target_metadata_error(error, phase),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_target_metadata_error(error: rustix::io::Errno, phase: DeletePhase) -> ToolError {
    match phase {
        _ if is_permission_error(error) => permission_denied(),
        DeletePhase::Revalidate => target_changed(),
        DeletePhase::Initial if error == rustix::io::Errno::NOENT => not_found(),
        DeletePhase::Initial if is_rejected_type_error(error) => path_rejected(),
        DeletePhase::Initial => unavailable(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_operational_phase(phase: DeletePhase) -> ToolError {
    match phase {
        DeletePhase::Initial => unavailable(),
        DeletePhase::Revalidate => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_rejected_phase(phase: DeletePhase) -> ToolError {
    match phase {
        DeletePhase::Initial => path_rejected(),
        DeletePhase::Revalidate => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_unlink_error(error: rustix::io::Errno, kind: TargetKind) -> ToolError {
    if error == rustix::io::Errno::NOENT
        || error == rustix::io::Errno::NOTDIR
        || error == rustix::io::Errno::ISDIR
        || error == rustix::io::Errno::LOOP
    {
        target_changed()
    } else if kind == TargetKind::Directory
        && (error == rustix::io::Errno::NOTEMPTY || error == rustix::io::Errno::EXIST)
    {
        directory_not_empty()
    } else if is_permission_error(error) || error == rustix::io::Errno::ROFS {
        permission_denied()
    } else {
        delete_failed()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_unlink_error_with_evidence<Evidence: DeleteFileEvidence>(
    error: rustix::io::Errno,
    target: TargetIdentity,
    parent: BorrowedFd<'_>,
    basename: &str,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> ToolError {
    #[cfg(target_os = "macos")]
    if error == rustix::io::Errno::PERM && target.kind == TargetKind::RegularFile {
        match evidence_statat(
            parent,
            OsStr::new(basename),
            DeletePhase::Revalidate,
            StatatSite::Target,
            cancellation,
            evidence,
            ordinals,
        ) {
            Ok(metadata) => {
                return match TargetIdentity::from_stat(&metadata) {
                    Ok(observed) if observed == target => permission_denied(),
                    Ok(_) | Err(()) => target_changed(),
                };
            }
            Err(EvidenceOperationError::Cancelled) => return cancelled(),
            Err(EvidenceOperationError::Os(error)) if is_target_change_error(error) => {
                return target_changed();
            }
            Err(EvidenceOperationError::Os(_)) => return permission_denied(),
        }
    }

    #[cfg(target_os = "linux")]
    let _ = (parent, basename, cancellation, evidence, ordinals);

    map_unlink_error(error, target.kind)
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

#[cfg(target_os = "macos")]
fn is_target_change_error(error: rustix::io::Errno) -> bool {
    error == rustix::io::Errno::NOENT || is_rejected_type_error(error)
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
        "delete_file_invalid_arguments",
        "delete_file arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "delete_file_invalid_path",
        "delete_file path is invalid",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "delete_file_unsupported_platform",
        "native delete_file is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "delete_file_not_found",
        "requested path is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "delete_file_permission_denied",
        "requested path cannot be deleted",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn path_rejected() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "delete_file_path_rejected",
        "requested path is not a confined regular file or empty directory",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn directory_not_empty() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "delete_file_directory_not_empty",
        "requested directory is not empty",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "delete_file_unavailable",
        "requested path is unavailable",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn target_changed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "delete_file_target_changed",
        "requested path changed before deletion",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn delete_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "delete_file_delete_failed",
        "requested path could not be deleted",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn commit_ambiguous() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "delete_file_commit_ambiguous",
        "requested path deletion status is uncertain",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "delete_file_cancelled",
        "delete_file execution was cancelled",
        false,
    )
}

#[cfg(test)]
mod race_tests;

#[cfg(test)]
mod tests;
