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
use std::ffi::OsStr;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};

/// Maximum UTF-8 bytes accepted in either requested or normalized endpoint.
pub const MAX_RENAME_FILE_PATH_BYTES: usize = 4 * 1024;
/// Maximum components accepted in either normalized endpoint.
pub const MAX_RENAME_FILE_PATH_COMPONENTS: usize = 256;
/// Maximum serialized byte size of accepted arguments.
pub const MAX_RENAME_FILE_SERIALIZED_ARGUMENT_BYTES: usize = 64 * 1024;
/// Maximum serialized [`ToolOutput`] bytes produced by this tool.
pub const MAX_RENAME_FILE_SERIALIZED_RESULT_BYTES: usize = 16 * 1024;

/// Registered name of [`RenameFileTool`].
pub const RENAME_FILE_TOOL_NAME: &str = "rename_file";

const RENAME_FILE_DESCRIPTION: &str =
    "Rename one existing regular file to an absent path within the configured workspace";
const OLD_PATH_DESCRIPTION: &str = "Current workspace-relative regular-file path";
const NEW_PATH_DESCRIPTION: &str = "New workspace-relative file path";

#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_PARENT_SYNC_CALLS: usize = 16;

/// Stable category for failure to acquire a rename-capable workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenameFileToolOpenErrorKind {
    /// Native rename execution is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`RenameFileTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct RenameFileToolOpenError {
    kind: RenameFileToolOpenErrorKind,
}

impl RenameFileToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> RenameFileToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: RenameFileToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for RenameFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenameFileToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for RenameFileToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            RenameFileToolOpenErrorKind::UnsupportedPlatform => {
                "native rename_file is unsupported on this platform"
            }
            RenameFileToolOpenErrorKind::InvalidRoot => {
                "native rename_file workspace root is invalid"
            }
            RenameFileToolOpenErrorKind::InvalidFileType => {
                "native rename_file workspace root is not a directory"
            }
            RenameFileToolOpenErrorKind::Unavailable => {
                "native rename_file workspace root is unavailable"
            }
        })
    }
}

impl Error for RenameFileToolOpenError {}

/// Native no-replace file renamer confined to one retained workspace root.
///
/// Linux and macOS execution accepts only an existing regular-file source and
/// an absent destination whose parents already exist. It uses one no-replace
/// rename syscall. Portable rename has no source-inode compare-and-swap, so a
/// final-window replacement can be moved; postcommit identity verification
/// prevents that outcome from being reported as success.
pub struct RenameFileTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl RenameFileTool {
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
    pub fn open(root: &Path) -> Result<Self, RenameFileToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(RenameFileToolOpenError::new(
                RenameFileToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(RenameFileToolOpenError::new(
                    RenameFileToolOpenErrorKind::InvalidRoot,
                ));
            }
            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor).map_err(|_| {
                RenameFileToolOpenError::new(RenameFileToolOpenErrorKind::Unavailable)
            })?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(RenameFileToolOpenError::new(
                    RenameFileToolOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for RenameFileTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenameFileTool")
            .finish_non_exhaustive()
    }
}

struct ValidatedArguments<'a> {
    requested_old: &'a str,
    requested_new: &'a str,
    old: String,
    new: String,
}

impl Tool for RenameFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: rename_file_name(),
            description: RENAME_FILE_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "old_path": { "type": "string", "description": OLD_PATH_DESCRIPTION },
                    "new_path": { "type": "string", "description": NEW_PATH_DESCRIPTION }
                },
                "required": ["old_path", "new_path"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != rename_file_name() {
            return Err(invalid_arguments());
        }
        let arguments = validate_arguments(&call.arguments)?;
        let prepared_arguments = json!({
            "old_path": arguments.old,
            "new_path": arguments.new,
        });
        if !serialized_value_fits(
            &prepared_arguments,
            MAX_RENAME_FILE_SERIALIZED_ARGUMENT_BYTES,
        ) {
            return Err(invalid_arguments());
        }
        let old_path = prepared_arguments["old_path"]
            .as_str()
            .expect("prepared rename_file old_path is a string")
            .to_owned();
        let new_path = prepared_arguments["new_path"]
            .as_str()
            .expect("prepared rename_file new_path is a string")
            .to_owned();
        Ok(PreparedToolCall::new(
            Capability::FilesystemRename { old_path, new_path },
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
            if arguments.old != arguments.requested_old || arguments.new != arguments.requested_new
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
                self.execute_supported(&arguments.old, &arguments.new, &cancellation)
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
    let Some(Value::String(old_path)) = object.get("old_path") else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(new_path)) = object.get("new_path") else {
        return Err(invalid_arguments());
    };
    let normalized_old = normalize_relative_path(old_path)?;
    let normalized_new = normalize_relative_path(new_path)?;
    if normalized_old == normalized_new {
        return Err(invalid_arguments());
    }
    if !serialized_value_fits(arguments, MAX_RENAME_FILE_SERIALIZED_ARGUMENT_BYTES) {
        return Err(invalid_arguments());
    }
    Ok(ValidatedArguments {
        requested_old: old_path,
        requested_new: new_path,
        old: normalized_old,
        new: normalized_new,
    })
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_RENAME_FILE_PATH_BYTES
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
        if components > MAX_RENAME_FILE_PATH_COMPONENTS {
            return Err(invalid_path());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() || normalized.len() > MAX_RENAME_FILE_PATH_BYTES {
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

fn rename_file_name() -> ToolName {
    ToolName::new(RENAME_FILE_TOOL_NAME).expect("rename_file is a valid tool name")
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
pub(super) enum RenamePhase {
    Initial,
    Revalidate,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RenameEndpoint {
    Source,
    Destination,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RenameOpenSite {
    Root(RenameEndpoint),
    #[cfg(target_os = "macos")]
    RootParent(RenameEndpoint),
    Intermediate(RenameEndpoint, usize),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RenameFstatSite {
    Root(RenameEndpoint),
    Intermediate(RenameEndpoint, usize),
    FinalParent(RenameEndpoint),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RenameStatatSite {
    #[cfg(target_os = "macos")]
    LinkedRoot(RenameEndpoint),
    Source,
    Destination,
    PublishedDestination,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RenameSyncSide {
    Source,
    Destination,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RenameCheckpoint {
    BeforeOpen(RenamePhase, RenameOpenSite, usize),
    AfterOpen(RenamePhase, RenameOpenSite, usize),
    BeforeFstat(RenamePhase, RenameFstatSite, usize),
    AfterFstat(RenamePhase, RenameFstatSite, usize),
    BeforeStatat(RenamePhase, RenameStatatSite, usize),
    AfterStatat(RenamePhase, RenameStatatSite, usize),
    AfterRootValidation(RenamePhase, RenameEndpoint),
    AfterValidation(RenamePhase),
    FinalPreRename,
    AfterRename,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) trait RenameFileEvidence {
    fn checkpoint(&mut self, _checkpoint: RenameCheckpoint, _cancellation: &CancellationToken) {}

    fn open_walk(
        &mut self,
        _phase: RenamePhase,
        _site: RenameOpenSite,
        _ordinal: usize,
        parent: BorrowedFd<'_>,
        component: &OsStr,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty())
    }

    fn fstat(
        &mut self,
        _phase: RenamePhase,
        _site: RenameFstatSite,
        _ordinal: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::fstat(descriptor)
    }

    fn statat(
        &mut self,
        _phase: RenamePhase,
        _site: RenameStatatSite,
        _ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    #[allow(clippy::too_many_arguments)]
    fn after_validation(
        &mut self,
        _phase: RenamePhase,
        _source_parent: BorrowedFd<'_>,
        _source_name: &str,
        _destination_parent: BorrowedFd<'_>,
        _destination_name: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn final_pre_rename(
        &mut self,
        _source_parent: BorrowedFd<'_>,
        _source_name: &str,
        _destination_parent: BorrowedFd<'_>,
        _destination_name: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        Ok(())
    }

    fn rename(
        &mut self,
        source_parent: BorrowedFd<'_>,
        source_name: &str,
        destination_parent: BorrowedFd<'_>,
        destination_name: &str,
    ) -> Result<(), rustix::io::Errno> {
        rustix::fs::renameat_with(
            source_parent,
            source_name,
            destination_parent,
            destination_name,
            RenameFlags::NOREPLACE,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn after_rename(
        &mut self,
        _source_parent: BorrowedFd<'_>,
        _source_name: &str,
        _destination_parent: BorrowedFd<'_>,
        _destination_name: &str,
        _outcome: Result<(), rustix::io::Errno>,
        _cancellation: &CancellationToken,
    ) -> Result<(), rustix::io::Errno> {
        Ok(())
    }

    fn sync_parent(
        &mut self,
        _side: RenameSyncSide,
        _attempt: usize,
        parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        rustix::fs::fsync(parent)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NativeRenameFileEvidence;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl RenameFileEvidence for NativeRenameFileEvidence {}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct OperationOrdinals {
    open: usize,
    fstat: usize,
    statat: usize,
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
#[derive(Clone, Copy)]
enum EvidenceOperationError {
    Cancelled,
    Os(rustix::io::Errno),
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
struct SourceIdentity {
    device: i128,
    inode: i128,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl SourceIdentity {
    fn from_stat(metadata: &rustix::fs::Stat) -> Result<Self, ()> {
        if !FileType::from_raw_mode(metadata.st_mode).is_file() {
            return Err(());
        }
        Ok(Self {
            device: i128::from(metadata.st_dev),
            inode: i128::from(metadata.st_ino),
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ParentWalk<'path> {
    parent: OwnedFd,
    identity: DirectoryIdentity,
    basename: &'path str,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl RenameFileTool {
    fn execute_supported(
        &self,
        old_path: &str,
        new_path: &str,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let mut evidence = NativeRenameFileEvidence;
        self.execute_supported_with_evidence(old_path, new_path, cancellation, &mut evidence)
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn execute_supported_with_evidence<Evidence: RenameFileEvidence>(
        &self,
        old_path: &str,
        new_path: &str,
        cancellation: &CancellationToken,
        evidence: &mut Evidence,
    ) -> Result<ToolOutput, ToolError> {
        let success = build_success_output(old_path, new_path)?;
        let mut ordinals = OperationOrdinals::default();

        let initial_source = self.walk_parent(
            old_path,
            RenameEndpoint::Source,
            RenamePhase::Initial,
            cancellation,
            evidence,
            &mut ordinals,
        )?;
        let initial_source_identity = inspect_source(
            initial_source.parent.as_fd(),
            initial_source.basename,
            RenamePhase::Initial,
            cancellation,
            evidence,
            &mut ordinals,
        )?;
        let initial_destination = self.walk_parent(
            new_path,
            RenameEndpoint::Destination,
            RenamePhase::Initial,
            cancellation,
            evidence,
            &mut ordinals,
        )?;
        require_destination_absent(
            initial_destination.parent.as_fd(),
            initial_destination.basename,
            RenamePhase::Initial,
            cancellation,
            evidence,
            &mut ordinals,
        )?;
        evidence.after_validation(
            RenamePhase::Initial,
            initial_source.parent.as_fd(),
            initial_source.basename,
            initial_destination.parent.as_fd(),
            initial_destination.basename,
            cancellation,
        )?;
        precommit_checkpoint(
            evidence,
            RenameCheckpoint::AfterValidation(RenamePhase::Initial),
            cancellation,
        )?;

        let final_source = self.walk_parent(
            old_path,
            RenameEndpoint::Source,
            RenamePhase::Revalidate,
            cancellation,
            evidence,
            &mut ordinals,
        )?;
        if final_source.identity != initial_source.identity {
            return Err(target_changed());
        }
        let final_source_identity = inspect_source(
            final_source.parent.as_fd(),
            final_source.basename,
            RenamePhase::Revalidate,
            cancellation,
            evidence,
            &mut ordinals,
        )?;
        if final_source_identity != initial_source_identity {
            return Err(target_changed());
        }
        let final_destination = self.walk_parent(
            new_path,
            RenameEndpoint::Destination,
            RenamePhase::Revalidate,
            cancellation,
            evidence,
            &mut ordinals,
        )?;
        if final_destination.identity != initial_destination.identity {
            return Err(target_changed());
        }
        require_destination_absent(
            final_destination.parent.as_fd(),
            final_destination.basename,
            RenamePhase::Revalidate,
            cancellation,
            evidence,
            &mut ordinals,
        )?;
        evidence.after_validation(
            RenamePhase::Revalidate,
            final_source.parent.as_fd(),
            final_source.basename,
            final_destination.parent.as_fd(),
            final_destination.basename,
            cancellation,
        )?;
        precommit_checkpoint(
            evidence,
            RenameCheckpoint::AfterValidation(RenamePhase::Revalidate),
            cancellation,
        )?;

        evidence.final_pre_rename(
            final_source.parent.as_fd(),
            final_source.basename,
            final_destination.parent.as_fd(),
            final_destination.basename,
            cancellation,
        )?;
        precommit_checkpoint(evidence, RenameCheckpoint::FinalPreRename, cancellation)?;

        let outcome = evidence.rename(
            final_source.parent.as_fd(),
            final_source.basename,
            final_destination.parent.as_fd(),
            final_destination.basename,
        );
        let after_rename = evidence.after_rename(
            final_source.parent.as_fd(),
            final_source.basename,
            final_destination.parent.as_fd(),
            final_destination.basename,
            outcome,
            cancellation,
        );

        match outcome {
            Ok(()) => {
                evidence.checkpoint(RenameCheckpoint::AfterRename, cancellation);
                let published = postcommit_destination_identity(
                    final_destination.parent.as_fd(),
                    final_destination.basename,
                    cancellation,
                    evidence,
                    &mut ordinals,
                );
                let source_sync = sync_parent_bounded(
                    final_source.parent.as_fd(),
                    RenameSyncSide::Source,
                    evidence,
                );
                let destination_sync = if final_source.identity == final_destination.identity {
                    Ok(())
                } else {
                    sync_parent_bounded(
                        final_destination.parent.as_fd(),
                        RenameSyncSide::Destination,
                        evidence,
                    )
                };
                if after_rename.is_err()
                    || published != Ok(initial_source_identity)
                    || source_sync.is_err()
                    || destination_sync.is_err()
                {
                    return Err(commit_ambiguous());
                }
                Ok(success)
            }
            Err(error) if error == rustix::io::Errno::INTR => {
                evidence.checkpoint(RenameCheckpoint::AfterRename, cancellation);
                let _ = sync_parent_bounded(
                    final_source.parent.as_fd(),
                    RenameSyncSide::Source,
                    evidence,
                );
                if final_source.identity != final_destination.identity {
                    let _ = sync_parent_bounded(
                        final_destination.parent.as_fd(),
                        RenameSyncSide::Destination,
                        evidence,
                    );
                }
                Err(commit_ambiguous())
            }
            Err(error) => {
                let _ = after_rename;
                check_cancellation(cancellation)?;
                Err(map_rename_error(error))
            }
        }
    }

    fn walk_parent<'path, Evidence: RenameFileEvidence>(
        &self,
        path: &'path str,
        endpoint: RenameEndpoint,
        phase: RenamePhase,
        cancellation: &CancellationToken,
        evidence: &mut Evidence,
        ordinals: &mut OperationOrdinals,
    ) -> Result<ParentWalk<'path>, ToolError> {
        let mut parent = evidence_open_walk(
            self.root.as_fd(),
            OsStr::new("."),
            phase,
            RenameOpenSite::Root(endpoint),
            cancellation,
            evidence,
            ordinals,
        )
        .map_err(|error| map_walk_evidence_error(error, phase, endpoint, true))?;
        validate_linked_root(
            parent.as_fd(),
            endpoint,
            phase,
            cancellation,
            evidence,
            ordinals,
        )?;
        precommit_checkpoint(
            evidence,
            RenameCheckpoint::AfterRootValidation(phase, endpoint),
            cancellation,
        )?;

        let mut components = path.split('/').peekable();
        let mut depth = 0_usize;
        loop {
            check_cancellation(cancellation)?;
            let component = components.next().ok_or_else(invalid_arguments)?;
            if components.peek().is_none() {
                let metadata = evidence_fstat(
                    parent.as_fd(),
                    phase,
                    RenameFstatSite::FinalParent(endpoint),
                    cancellation,
                    evidence,
                    ordinals,
                )
                .map_err(|error| map_metadata_evidence_error(error, phase))?;
                let identity = DirectoryIdentity::from_stat(&metadata)
                    .map_err(|()| map_rejected_phase(phase))?;
                return Ok(ParentWalk {
                    parent,
                    identity,
                    basename: component,
                });
            }

            let site = RenameOpenSite::Intermediate(endpoint, depth);
            parent = evidence_open_walk(
                parent.as_fd(),
                OsStr::new(component),
                phase,
                site,
                cancellation,
                evidence,
                ordinals,
            )
            .map_err(|error| map_walk_evidence_error(error, phase, endpoint, false))?;
            let metadata = evidence_fstat(
                parent.as_fd(),
                phase,
                RenameFstatSite::Intermediate(endpoint, depth),
                cancellation,
                evidence,
                ordinals,
            )
            .map_err(|error| map_metadata_evidence_error(error, phase))?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(map_rejected_phase(phase));
            }
            depth = depth.checked_add(1).ok_or_else(unavailable)?;
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn inspect_source<Evidence: RenameFileEvidence>(
    parent: BorrowedFd<'_>,
    name: &str,
    phase: RenamePhase,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<SourceIdentity, ToolError> {
    let metadata = evidence_statat(
        parent,
        OsStr::new(name),
        phase,
        RenameStatatSite::Source,
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|error| map_source_evidence_error(error, phase))?;
    SourceIdentity::from_stat(&metadata).map_err(|()| map_rejected_phase(phase))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn require_destination_absent<Evidence: RenameFileEvidence>(
    parent: BorrowedFd<'_>,
    name: &str,
    phase: RenamePhase,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<(), ToolError> {
    match evidence_statat(
        parent,
        OsStr::new(name),
        phase,
        RenameStatatSite::Destination,
        cancellation,
        evidence,
        ordinals,
    ) {
        Ok(_) => match phase {
            RenamePhase::Initial => Err(destination_exists()),
            RenamePhase::Revalidate => Err(target_changed()),
        },
        Err(EvidenceOperationError::Cancelled) => Err(cancelled()),
        Err(EvidenceOperationError::Os(error)) if error == rustix::io::Errno::NOENT => Ok(()),
        Err(EvidenceOperationError::Os(error)) if is_permission_error(error) => {
            Err(permission_denied())
        }
        Err(EvidenceOperationError::Os(error)) if is_rejected_type_error(error) => {
            Err(map_rejected_phase(phase))
        }
        Err(EvidenceOperationError::Os(_)) => Err(map_operational_phase(phase)),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn postcommit_destination_identity<Evidence: RenameFileEvidence>(
    parent: BorrowedFd<'_>,
    name: &str,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<SourceIdentity, ()> {
    let ordinal = ordinals.next_statat();
    let phase = RenamePhase::Revalidate;
    let site = RenameStatatSite::PublishedDestination;
    evidence.checkpoint(
        RenameCheckpoint::BeforeStatat(phase, site, ordinal),
        cancellation,
    );
    let metadata = evidence.statat(phase, site, ordinal, parent, OsStr::new(name));
    evidence.checkpoint(
        RenameCheckpoint::AfterStatat(phase, site, ordinal),
        cancellation,
    );
    metadata
        .map_err(|_| ())
        .and_then(|metadata| SourceIdentity::from_stat(&metadata))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_linked_root<Evidence: RenameFileEvidence>(
    root: BorrowedFd<'_>,
    endpoint: RenameEndpoint,
    phase: RenamePhase,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<(), ToolError> {
    let metadata = evidence_fstat(
        root,
        phase,
        RenameFstatSite::Root(endpoint),
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|error| map_metadata_evidence_error(error, phase))?;
    if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
        return Err(map_rejected_phase(phase));
    }

    #[cfg(target_os = "linux")]
    if metadata.st_nlink == 0 {
        return Err(map_operational_phase(phase));
    }

    #[cfg(target_os = "macos")]
    validate_linked_macos_root(
        root,
        &metadata,
        endpoint,
        phase,
        cancellation,
        evidence,
        ordinals,
    )?;

    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn validate_linked_macos_root<Evidence: RenameFileEvidence>(
    root: BorrowedFd<'_>,
    root_metadata: &rustix::fs::Stat,
    endpoint: RenameEndpoint,
    phase: RenamePhase,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<(), ToolError> {
    check_cancellation(cancellation)?;
    let root_path = rustix::fs::getpath(root);
    check_cancellation(cancellation)?;
    let root_path = root_path.map_err(|error| map_walk_error(error, phase, endpoint, true))?;
    let root_path = root_path.as_bytes();
    if root_path == b"/" {
        return Ok(());
    }
    let name = root_path
        .rsplit(|byte| *byte == b'/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| map_operational_phase(phase))?;
    let parent = evidence_open_walk(
        root,
        OsStr::new(".."),
        phase,
        RenameOpenSite::RootParent(endpoint),
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|error| map_walk_evidence_error(error, phase, endpoint, true))?;
    let linked = evidence_statat(
        parent.as_fd(),
        OsStr::from_bytes(name),
        phase,
        RenameStatatSite::LinkedRoot(endpoint),
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|error| map_metadata_evidence_error(error, phase))?;
    if linked.st_dev != root_metadata.st_dev
        || linked.st_ino != root_metadata.st_ino
        || !FileType::from_raw_mode(linked.st_mode).is_dir()
    {
        return Err(map_operational_phase(phase));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn evidence_open_walk<Evidence: RenameFileEvidence>(
    parent: BorrowedFd<'_>,
    component: &OsStr,
    phase: RenamePhase,
    site: RenameOpenSite,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<OwnedFd, EvidenceOperationError> {
    let ordinal = ordinals.next_open();
    evidence.checkpoint(
        RenameCheckpoint::BeforeOpen(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    let result = evidence.open_walk(phase, site, ordinal, parent, component);
    evidence.checkpoint(
        RenameCheckpoint::AfterOpen(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    result.map_err(EvidenceOperationError::Os)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn evidence_fstat<Evidence: RenameFileEvidence>(
    descriptor: BorrowedFd<'_>,
    phase: RenamePhase,
    site: RenameFstatSite,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<rustix::fs::Stat, EvidenceOperationError> {
    let ordinal = ordinals.next_fstat();
    evidence.checkpoint(
        RenameCheckpoint::BeforeFstat(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    let result = evidence.fstat(phase, site, ordinal, descriptor);
    evidence.checkpoint(
        RenameCheckpoint::AfterFstat(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    result.map_err(EvidenceOperationError::Os)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn evidence_statat<Evidence: RenameFileEvidence>(
    parent: BorrowedFd<'_>,
    name: &OsStr,
    phase: RenamePhase,
    site: RenameStatatSite,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<rustix::fs::Stat, EvidenceOperationError> {
    let ordinal = ordinals.next_statat();
    evidence.checkpoint(
        RenameCheckpoint::BeforeStatat(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    let result = evidence.statat(phase, site, ordinal, parent, name);
    evidence.checkpoint(
        RenameCheckpoint::AfterStatat(phase, site, ordinal),
        cancellation,
    );
    if cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    result.map_err(EvidenceOperationError::Os)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn precommit_checkpoint<Evidence: RenameFileEvidence>(
    evidence: &mut Evidence,
    checkpoint: RenameCheckpoint,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    evidence.checkpoint(checkpoint, cancellation);
    check_cancellation(cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_parent_bounded<Evidence: RenameFileEvidence>(
    parent: BorrowedFd<'_>,
    side: RenameSyncSide,
    evidence: &mut Evidence,
) -> Result<(), ()> {
    for attempt in 0..MAX_PARENT_SYNC_CALLS {
        match evidence.sync_parent(side, attempt, parent) {
            Ok(()) => return Ok(()),
            Err(error)
                if error == rustix::io::Errno::INTR && attempt + 1 < MAX_PARENT_SYNC_CALLS => {}
            Err(_) => return Err(()),
        }
    }
    Err(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn build_success_output(old_path: &str, new_path: &str) -> Result<ToolOutput, ToolError> {
    let output = ToolOutput::success(json!({ "old_path": old_path, "new_path": new_path }));
    if !serialized_value_fits(&output, MAX_RENAME_FILE_SERIALIZED_RESULT_BYTES) {
        return Err(rename_failed());
    }
    Ok(output)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> RenameFileToolOpenError {
    let kind = if is_rejected_type_error(error) {
        RenameFileToolOpenErrorKind::InvalidFileType
    } else {
        RenameFileToolOpenErrorKind::Unavailable
    };
    RenameFileToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_walk_evidence_error(
    error: EvidenceOperationError,
    phase: RenamePhase,
    endpoint: RenameEndpoint,
    root: bool,
) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error) => map_walk_error(error, phase, endpoint, root),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_walk_error(
    error: rustix::io::Errno,
    phase: RenamePhase,
    endpoint: RenameEndpoint,
    root: bool,
) -> ToolError {
    if is_permission_error(error) {
        return permission_denied();
    }
    if phase == RenamePhase::Revalidate {
        return target_changed();
    }
    if !root && endpoint == RenameEndpoint::Source && error == rustix::io::Errno::NOENT {
        return not_found();
    }
    if is_rejected_type_error(error) {
        return path_rejected();
    }
    unavailable()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_metadata_evidence_error(error: EvidenceOperationError, phase: RenamePhase) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error) if is_permission_error(error) => permission_denied(),
        EvidenceOperationError::Os(_) => map_operational_phase(phase),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_source_evidence_error(error: EvidenceOperationError, phase: RenamePhase) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error) if is_permission_error(error) => permission_denied(),
        EvidenceOperationError::Os(error)
            if phase == RenamePhase::Initial && error == rustix::io::Errno::NOENT =>
        {
            not_found()
        }
        EvidenceOperationError::Os(error) if is_rejected_type_error(error) => {
            map_rejected_phase(phase)
        }
        EvidenceOperationError::Os(_) => map_operational_phase(phase),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_operational_phase(phase: RenamePhase) -> ToolError {
    match phase {
        RenamePhase::Initial => unavailable(),
        RenamePhase::Revalidate => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_rejected_phase(phase: RenamePhase) -> ToolError {
    match phase {
        RenamePhase::Initial => path_rejected(),
        RenamePhase::Revalidate => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_rename_error(error: rustix::io::Errno) -> ToolError {
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
        rename_failed()
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
        "rename_file_invalid_arguments",
        "rename_file arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "rename_file_invalid_path",
        "rename_file path is invalid",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "rename_file_unsupported_platform",
        "native rename_file is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "rename_file_not_found",
        "rename source is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "rename_file_permission_denied",
        "requested rename is not permitted",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn path_rejected() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "rename_file_path_rejected",
        "requested rename path is not confined",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn destination_exists() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "rename_file_destination_exists",
        "rename destination already exists",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "rename_file_unavailable",
        "requested rename is unavailable",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn target_changed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "rename_file_target_changed",
        "rename paths changed before execution",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unsupported_filesystem() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "rename_file_unsupported_filesystem",
        "atomic no-replace rename is unavailable",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rename_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "rename_file_rename_failed",
        "requested file could not be renamed",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn commit_ambiguous() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "rename_file_commit_ambiguous",
        "requested file rename status is uncertain",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "rename_file_cancelled",
        "rename_file execution was cancelled",
        false,
    )
}
