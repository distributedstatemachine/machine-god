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
#[cfg(target_os = "macos")]
use rustix::fs::AtFlags;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{FileType, Mode, OFlags};

/// Maximum UTF-8 bytes accepted in a requested or canonical directory path.
pub const MAX_CREATE_FOLDER_PATH_BYTES: usize = 4 * 1024;
/// Maximum components accepted in a canonical directory path.
pub const MAX_CREATE_FOLDER_PATH_COMPONENTS: usize = 256;
/// Maximum `mkdirat` calls made by one execution.
pub const MAX_CREATE_FOLDER_MKDIR_CALLS: usize = 256;
/// Maximum `fsync` calls made by one execution.
pub const MAX_CREATE_FOLDER_SYNC_CALLS: usize = 4_112;
/// Maximum serialized byte size of accepted arguments.
pub const MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES: usize = 64 * 1024;
/// Maximum serialized [`ToolOutput`] bytes produced by this tool.
pub const MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES: usize = 16 * 1024;

/// Registered name of [`CreateFolderTool`].
pub const CREATE_FOLDER_TOOL_NAME: &str = "create_folder";

const CREATE_FOLDER_DESCRIPTION: &str =
    "Create one directory path and missing parents within the configured workspace";
const PATH_DESCRIPTION: &str = "Workspace-relative directory path";

#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_SYNC_CALLS_PER_SITE: usize = 16;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const REQUESTED_DIRECTORY_MODE: Mode = Mode::from_raw_mode(0o755);

/// Stable category for failure to acquire a create-capable workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateFolderToolOpenErrorKind {
    /// Native folder creation is not supported on this platform.
    UnsupportedPlatform,
    /// The injected root path was not absolute.
    InvalidRoot,
    /// The injected root did not resolve to a real directory.
    InvalidFileType,
    /// The injected root could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to acquire a [`CreateFolderTool`] workspace root.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CreateFolderToolOpenError {
    kind: CreateFolderToolOpenErrorKind,
}

impl CreateFolderToolOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> CreateFolderToolOpenErrorKind {
        self.kind
    }

    const fn new(kind: CreateFolderToolOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for CreateFolderToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateFolderToolOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for CreateFolderToolOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            CreateFolderToolOpenErrorKind::UnsupportedPlatform => {
                "native create_folder is unsupported on this platform"
            }
            CreateFolderToolOpenErrorKind::InvalidRoot => {
                "native create_folder workspace root is invalid"
            }
            CreateFolderToolOpenErrorKind::InvalidFileType => {
                "native create_folder workspace root is not a directory"
            }
            CreateFolderToolOpenErrorKind::Unavailable => {
                "native create_folder workspace root is unavailable"
            }
        })
    }
}

impl Error for CreateFolderToolOpenError {}

/// Recursive native directory creator confined to one retained workspace root.
///
/// Linux and macOS execution walks without following symlinks, creates each
/// missing component descriptor-relatively with requested mode `0755`, and
/// honors the process umask and inherited ACLs. Once a creation succeeds or an
/// interrupted creation becomes uncertain, no created prefix is rolled back.
pub struct CreateFolderTool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

impl CreateFolderTool {
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
    pub fn open(root: &Path) -> Result<Self, CreateFolderToolOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(CreateFolderToolOpenError::new(
                CreateFolderToolOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(CreateFolderToolOpenError::new(
                    CreateFolderToolOpenErrorKind::InvalidRoot,
                ));
            }
            let descriptor = rustix::fs::open(&lexical_root, directory_open_flags(), Mode::empty())
                .map_err(map_root_open_error)?;
            let metadata = rustix::fs::fstat(&descriptor).map_err(|_| {
                CreateFolderToolOpenError::new(CreateFolderToolOpenErrorKind::Unavailable)
            })?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(CreateFolderToolOpenError::new(
                    CreateFolderToolOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self::from_root_descriptor(descriptor))
        }
    }
}

impl fmt::Debug for CreateFolderTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateFolderTool")
            .finish_non_exhaustive()
    }
}

struct ValidatedArguments<'a> {
    requested_path: &'a str,
    path: String,
}

impl Tool for CreateFolderTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: create_folder_name(),
            description: CREATE_FOLDER_DESCRIPTION.to_owned(),
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
        if call.name != create_folder_name() {
            return Err(invalid_arguments());
        }
        let arguments = validate_arguments(&call.arguments)?;
        let prepared_arguments = json!({ "path": arguments.path });
        if !serialized_value_fits(
            &prepared_arguments,
            MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES,
        ) {
            return Err(invalid_arguments());
        }
        let path = prepared_arguments["path"]
            .as_str()
            .expect("prepared create_folder path is a string")
            .to_owned();
        Ok(PreparedToolCall::new(
            Capability::Filesystem {
                access: FilesystemAccess::Create,
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
    if !serialized_value_fits(arguments, MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES) {
        return Err(invalid_arguments());
    }
    Ok(ValidatedArguments {
        requested_path: path,
        path: normalized,
    })
}

fn normalize_relative_path(path: &str) -> Result<String, ToolError> {
    if path.is_empty()
        || path.len() > MAX_CREATE_FOLDER_PATH_BYTES
        || path.starts_with('/')
        || path.starts_with('~')
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
        if components > MAX_CREATE_FOLDER_PATH_COMPONENTS {
            return Err(invalid_path());
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    if normalized.is_empty() || normalized.len() > MAX_CREATE_FOLDER_PATH_BYTES {
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

fn create_folder_name() -> ToolName {
    ToolName::new(CREATE_FOLDER_TOOL_NAME).expect("create_folder is a valid tool name")
}

pub(super) fn serialized_value_fits(
    value: &(impl serde::Serialize + ?Sized),
    limit: usize,
) -> bool {
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
pub(super) enum CreateFolderPhase {
    Initial,
    Revalidate,
    Create,
    Postcommit,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CreateFolderOpenSite {
    Root,
    #[cfg(target_os = "macos")]
    RootParent,
    Component(usize),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CreateFolderFstatSite {
    Root,
    Component(usize),
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CreateFolderStatatSite {
    LinkedRoot,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CreateFolderSyncSite {
    CreatedDirectory(usize),
    FirstCreatedParent(usize),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CreateFolderCheckpoint {
    BeforeOpen(CreateFolderPhase, CreateFolderOpenSite, usize),
    AfterOpen(CreateFolderPhase, CreateFolderOpenSite, usize),
    BeforeFstat(CreateFolderPhase, CreateFolderFstatSite, usize),
    AfterFstat(CreateFolderPhase, CreateFolderFstatSite, usize),
    #[cfg(target_os = "macos")]
    BeforeStatat(CreateFolderPhase, CreateFolderStatatSite, usize),
    #[cfg(target_os = "macos")]
    AfterStatat(CreateFolderPhase, CreateFolderStatatSite, usize),
    #[cfg(target_os = "macos")]
    BeforeRootPath(CreateFolderPhase),
    #[cfg(target_os = "macos")]
    AfterRootPath(CreateFolderPhase),
    AfterRootValidation(CreateFolderPhase),
    AfterWalk(CreateFolderPhase),
    FinalPreCreate,
    BeforeMkdir(usize, usize),
    AfterMkdir(usize, usize),
    AfterCommit,
    AfterPostcommitVerification,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) trait CreateFolderEvidence {
    fn checkpoint(
        &mut self,
        _checkpoint: CreateFolderCheckpoint,
        _cancellation: &CancellationToken,
    ) {
    }

    fn open_walk(
        &mut self,
        _phase: CreateFolderPhase,
        _site: CreateFolderOpenSite,
        _ordinal: usize,
        parent: BorrowedFd<'_>,
        component: &OsStr,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty())
    }

    fn fstat(
        &mut self,
        _phase: CreateFolderPhase,
        _site: CreateFolderFstatSite,
        _ordinal: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::fstat(descriptor)
    }

    #[cfg(target_os = "macos")]
    fn statat(
        &mut self,
        _phase: CreateFolderPhase,
        _site: CreateFolderStatatSite,
        _ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn mkdir(
        &mut self,
        _ordinal: usize,
        _component_index: usize,
        parent: BorrowedFd<'_>,
        component: &OsStr,
        mode: Mode,
    ) -> Result<(), rustix::io::Errno> {
        rustix::fs::mkdirat(parent, component, mode)
    }

    fn sync_directory(
        &mut self,
        _site: CreateFolderSyncSite,
        _attempt: usize,
        directory: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        rustix::fs::fsync(directory)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NativeCreateFolderEvidence;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CreateFolderEvidence for NativeCreateFolderEvidence {}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct OperationOrdinals {
    open: usize,
    fstat: usize,
    #[cfg(target_os = "macos")]
    statat: usize,
    mkdir: usize,
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

    #[cfg(target_os = "macos")]
    fn next_statat(&mut self) -> usize {
        let ordinal = self.statat;
        self.statat = self.statat.saturating_add(1);
        ordinal
    }

    fn next_mkdir(&mut self) -> usize {
        let ordinal = self.mkdir;
        self.mkdir = self.mkdir.saturating_add(1);
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
#[derive(Clone, Copy)]
enum CancellationMode {
    Observe,
    Ignore,
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
struct InitialWalk {
    identities: Vec<DirectoryIdentity>,
    deepest: OwnedFd,
    existing_components: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct SyncEntry {
    descriptor: OwnedFd,
    site: CreateFolderSyncSite,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CreateFolderTool {
    fn execute_supported(
        &self,
        normalized: &str,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let mut evidence = NativeCreateFolderEvidence;
        self.execute_supported_with_evidence(normalized, cancellation, &mut evidence)
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn execute_supported_with_evidence<Evidence: CreateFolderEvidence>(
        &self,
        normalized: &str,
        cancellation: &CancellationToken,
        evidence: &mut Evidence,
    ) -> Result<ToolOutput, ToolError> {
        let success = build_success_output(normalized)?;
        let components = normalized.split('/').collect::<Vec<_>>();
        debug_assert!(!components.is_empty());
        debug_assert!(components.len() <= MAX_CREATE_FOLDER_PATH_COMPONENTS);
        let mut ordinals = OperationOrdinals::default();
        let InitialWalk {
            identities: initial_identities,
            deepest: initial_deepest,
            existing_components,
        } = self.initial_walk(&components, cancellation, evidence, &mut ordinals)?;

        let revalidated = self.rewalk_exact_prefix(
            &components,
            &initial_identities,
            existing_components,
            CreateFolderPhase::Revalidate,
            CancellationMode::Observe,
            cancellation,
            evidence,
            &mut ordinals,
        )?;
        precommit_checkpoint(
            evidence,
            if existing_components == components.len() {
                CreateFolderCheckpoint::AfterWalk(CreateFolderPhase::Revalidate)
            } else {
                CreateFolderCheckpoint::FinalPreCreate
            },
            cancellation,
        )?;

        drop(initial_deepest);

        if existing_components == components.len() {
            drop(revalidated);
            debug_assert!(serialized_value_fits(
                &success,
                MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES
            ));
            return Ok(success);
        }

        let mut identities = initial_identities;
        let mut current = Some(revalidated);
        let mut sync_entries: Vec<SyncEntry> = Vec::with_capacity(
            components
                .len()
                .saturating_sub(existing_components)
                .saturating_add(1),
        );
        let mut committed = false;
        let mut uncertain = false;
        let mut postcommit_failure = false;

        for (component_index, component) in components.iter().enumerate().skip(existing_components)
        {
            debug_assert!(ordinals.mkdir < MAX_CREATE_FOLDER_MKDIR_CALLS);
            let mkdir_ordinal = ordinals.next_mkdir();
            let parent = if committed {
                sync_entries
                    .last()
                    .expect("committed create_folder retains its current directory")
                    .descriptor
                    .as_fd()
            } else {
                current
                    .as_ref()
                    .expect("precommit create_folder retains its current directory")
                    .as_fd()
            };
            let mkdir_outcome = evidence_mkdir(
                parent,
                OsStr::new(component),
                component_index,
                mkdir_ordinal,
                if committed {
                    CancellationMode::Ignore
                } else {
                    CancellationMode::Observe
                },
                cancellation,
                evidence,
            )?;

            match mkdir_outcome {
                Ok(()) => {
                    if !committed {
                        committed = true;
                        sync_entries.push(SyncEntry {
                            descriptor: current
                                .take()
                                .expect("first create retains its parent descriptor"),
                            site: CreateFolderSyncSite::FirstCreatedParent(component_index),
                        });
                        evidence.checkpoint(CreateFolderCheckpoint::AfterCommit, cancellation);
                    }
                    if let Ok((descriptor, identity)) = open_created_component(
                        sync_entries
                            .last()
                            .expect("create parent is retained")
                            .descriptor
                            .as_fd(),
                        component,
                        component_index,
                        cancellation,
                        evidence,
                        &mut ordinals,
                    ) {
                        identities.push(identity);
                        sync_entries.push(SyncEntry {
                            descriptor,
                            site: CreateFolderSyncSite::CreatedDirectory(component_index),
                        });
                    } else {
                        postcommit_failure = true;
                        break;
                    }
                }
                Err(error) if error == rustix::io::Errno::INTR => {
                    uncertain = true;
                    if !committed {
                        committed = true;
                        sync_entries.push(SyncEntry {
                            descriptor: current
                                .take()
                                .expect("uncertain create retains its parent descriptor"),
                            site: CreateFolderSyncSite::FirstCreatedParent(component_index),
                        });
                        evidence.checkpoint(CreateFolderCheckpoint::AfterCommit, cancellation);
                    }
                    if let Ok((descriptor, identity)) = open_created_component(
                        sync_entries
                            .last()
                            .expect("uncertain create parent is retained")
                            .descriptor
                            .as_fd(),
                        component,
                        component_index,
                        cancellation,
                        evidence,
                        &mut ordinals,
                    ) {
                        identities.push(identity);
                        sync_entries.push(SyncEntry {
                            descriptor,
                            site: CreateFolderSyncSite::CreatedDirectory(component_index),
                        });
                    }
                    break;
                }
                Err(error) if error == rustix::io::Errno::EXIST => {
                    let mode = if committed {
                        CancellationMode::Ignore
                    } else {
                        CancellationMode::Observe
                    };
                    match open_and_identify_component(
                        parent,
                        component,
                        component_index,
                        CreateFolderPhase::Create,
                        mode,
                        cancellation,
                        evidence,
                        &mut ordinals,
                    ) {
                        Ok((descriptor, identity)) => {
                            identities.push(identity);
                            if committed {
                                sync_entries.push(SyncEntry {
                                    descriptor,
                                    site: CreateFolderSyncSite::CreatedDirectory(component_index),
                                });
                            } else {
                                current = Some(descriptor);
                            }
                        }
                        Err(error) if committed => {
                            let _ = error;
                            postcommit_failure = true;
                            break;
                        }
                        Err(error) => {
                            return Err(map_create_open_error(
                                error,
                                component_index + 1 == components.len(),
                            ));
                        }
                    }
                }
                Err(error) if committed => {
                    let _ = error;
                    postcommit_failure = true;
                    break;
                }
                Err(error) => return Err(map_mkdir_error(error)),
            }
        }

        if !committed {
            let final_directory = self.rewalk_exact_prefix(
                &components,
                &identities,
                components.len(),
                CreateFolderPhase::Revalidate,
                CancellationMode::Observe,
                cancellation,
                evidence,
                &mut ordinals,
            )?;
            drop(final_directory);
            precommit_checkpoint(
                evidence,
                CreateFolderCheckpoint::AfterWalk(CreateFolderPhase::Revalidate),
                cancellation,
            )?;
            return Ok(success);
        }

        let verified = self
            .rewalk_exact_prefix(
                &components,
                &identities,
                components.len(),
                CreateFolderPhase::Postcommit,
                CancellationMode::Ignore,
                cancellation,
                evidence,
                &mut ordinals,
            )
            .is_ok();
        evidence.checkpoint(
            CreateFolderCheckpoint::AfterPostcommitVerification,
            cancellation,
        );
        let durable = sync_all_bottom_up(&sync_entries, evidence);
        if uncertain || postcommit_failure || !verified || !durable {
            return Err(commit_ambiguous());
        }
        Ok(success)
    }

    fn initial_walk<Evidence: CreateFolderEvidence>(
        &self,
        components: &[&str],
        cancellation: &CancellationToken,
        evidence: &mut Evidence,
        ordinals: &mut OperationOrdinals,
    ) -> Result<InitialWalk, ToolError> {
        let phase = CreateFolderPhase::Initial;
        let (mut directory, root_identity) = self.acquire_root(
            phase,
            CancellationMode::Observe,
            cancellation,
            evidence,
            ordinals,
        )?;
        let mut identities = Vec::with_capacity(components.len().saturating_add(1));
        identities.push(root_identity);

        for (component_index, component) in components.iter().enumerate() {
            match open_and_identify_component(
                directory.as_fd(),
                component,
                component_index,
                phase,
                CancellationMode::Observe,
                cancellation,
                evidence,
                ordinals,
            ) {
                Ok((next, identity)) => {
                    identities.push(identity);
                    directory = next;
                }
                Err(EvidenceOperationError::Cancelled) => return Err(cancelled()),
                Err(EvidenceOperationError::Os(error)) if error == rustix::io::Errno::NOENT => {
                    evidence.checkpoint(
                        CreateFolderCheckpoint::AfterWalk(CreateFolderPhase::Initial),
                        cancellation,
                    );
                    check_cancellation(cancellation)?;
                    return Ok(InitialWalk {
                        identities,
                        deepest: directory,
                        existing_components: component_index,
                    });
                }
                Err(error) => {
                    return Err(map_initial_open_error(
                        error,
                        component_index + 1 == components.len(),
                    ));
                }
            }
        }
        evidence.checkpoint(
            CreateFolderCheckpoint::AfterWalk(CreateFolderPhase::Initial),
            cancellation,
        );
        check_cancellation(cancellation)?;
        Ok(InitialWalk {
            identities,
            deepest: directory,
            existing_components: components.len(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn rewalk_exact_prefix<Evidence: CreateFolderEvidence>(
        &self,
        components: &[&str],
        expected: &[DirectoryIdentity],
        component_count: usize,
        phase: CreateFolderPhase,
        cancellation_mode: CancellationMode,
        cancellation: &CancellationToken,
        evidence: &mut Evidence,
        ordinals: &mut OperationOrdinals,
    ) -> Result<OwnedFd, ToolError> {
        let mut identities_match = expected.len() == component_count.saturating_add(1);
        let (mut directory, root_identity) =
            self.acquire_root(phase, cancellation_mode, cancellation, evidence, ordinals)?;
        identities_match &= expected.first() == Some(&root_identity);
        for (component_index, component) in components.iter().take(component_count).enumerate() {
            let (next, identity) = open_and_identify_component(
                directory.as_fd(),
                component,
                component_index,
                phase,
                cancellation_mode,
                cancellation,
                evidence,
                ordinals,
            )
            .map_err(|error| map_phase_operation_error(error, phase))?;
            identities_match &= expected.get(component_index + 1) == Some(&identity);
            directory = next;
        }
        if identities_match {
            Ok(directory)
        } else {
            Err(map_phase_failure(phase))
        }
    }

    fn acquire_root<Evidence: CreateFolderEvidence>(
        &self,
        phase: CreateFolderPhase,
        cancellation_mode: CancellationMode,
        cancellation: &CancellationToken,
        evidence: &mut Evidence,
        ordinals: &mut OperationOrdinals,
    ) -> Result<(OwnedFd, DirectoryIdentity), ToolError> {
        let root = evidence_open(
            self.root.as_fd(),
            OsStr::new("."),
            phase,
            CreateFolderOpenSite::Root,
            cancellation_mode,
            cancellation,
            evidence,
            ordinals,
        )
        .map_err(|error| map_phase_operation_error(error, phase))?;
        let metadata = evidence_fstat(
            root.as_fd(),
            phase,
            CreateFolderFstatSite::Root,
            cancellation_mode,
            cancellation,
            evidence,
            ordinals,
        )
        .map_err(|error| map_phase_operation_error(error, phase))?;
        let identity =
            DirectoryIdentity::from_stat(&metadata).map_err(|()| map_phase_failure(phase))?;
        validate_linked_root(
            root.as_fd(),
            &metadata,
            phase,
            cancellation_mode,
            cancellation,
            evidence,
            ordinals,
        )?;
        evidence.checkpoint(
            CreateFolderCheckpoint::AfterRootValidation(phase),
            cancellation,
        );
        if matches!(cancellation_mode, CancellationMode::Observe) {
            check_cancellation(cancellation)?;
        }
        Ok((root, identity))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn open_and_identify_component<Evidence: CreateFolderEvidence>(
    parent: BorrowedFd<'_>,
    component: &str,
    component_index: usize,
    phase: CreateFolderPhase,
    cancellation_mode: CancellationMode,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<(OwnedFd, DirectoryIdentity), EvidenceOperationError> {
    let descriptor = evidence_open(
        parent,
        OsStr::new(component),
        phase,
        CreateFolderOpenSite::Component(component_index),
        cancellation_mode,
        cancellation,
        evidence,
        ordinals,
    )?;
    let metadata = evidence_fstat(
        descriptor.as_fd(),
        phase,
        CreateFolderFstatSite::Component(component_index),
        cancellation_mode,
        cancellation,
        evidence,
        ordinals,
    )?;
    let identity = DirectoryIdentity::from_stat(&metadata)
        .map_err(|()| EvidenceOperationError::Os(rustix::io::Errno::NOTDIR))?;
    Ok((descriptor, identity))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_created_component<Evidence: CreateFolderEvidence>(
    parent: BorrowedFd<'_>,
    component: &str,
    component_index: usize,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<(OwnedFd, DirectoryIdentity), ()> {
    open_and_identify_component(
        parent,
        component,
        component_index,
        CreateFolderPhase::Postcommit,
        CancellationMode::Ignore,
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|_| ())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn evidence_open<Evidence: CreateFolderEvidence>(
    parent: BorrowedFd<'_>,
    component: &OsStr,
    phase: CreateFolderPhase,
    site: CreateFolderOpenSite,
    cancellation_mode: CancellationMode,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<OwnedFd, EvidenceOperationError> {
    let ordinal = ordinals.next_open();
    evidence.checkpoint(
        CreateFolderCheckpoint::BeforeOpen(phase, site, ordinal),
        cancellation,
    );
    if matches!(cancellation_mode, CancellationMode::Observe) && cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    let result = evidence.open_walk(phase, site, ordinal, parent, component);
    evidence.checkpoint(
        CreateFolderCheckpoint::AfterOpen(phase, site, ordinal),
        cancellation,
    );
    if matches!(cancellation_mode, CancellationMode::Observe) && cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    result.map_err(EvidenceOperationError::Os)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn evidence_fstat<Evidence: CreateFolderEvidence>(
    descriptor: BorrowedFd<'_>,
    phase: CreateFolderPhase,
    site: CreateFolderFstatSite,
    cancellation_mode: CancellationMode,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<rustix::fs::Stat, EvidenceOperationError> {
    let ordinal = ordinals.next_fstat();
    evidence.checkpoint(
        CreateFolderCheckpoint::BeforeFstat(phase, site, ordinal),
        cancellation,
    );
    if matches!(cancellation_mode, CancellationMode::Observe) && cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    let result = evidence.fstat(phase, site, ordinal, descriptor);
    evidence.checkpoint(
        CreateFolderCheckpoint::AfterFstat(phase, site, ordinal),
        cancellation,
    );
    if matches!(cancellation_mode, CancellationMode::Observe) && cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    result.map_err(EvidenceOperationError::Os)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn evidence_mkdir<Evidence: CreateFolderEvidence>(
    parent: BorrowedFd<'_>,
    component: &OsStr,
    component_index: usize,
    ordinal: usize,
    cancellation_mode: CancellationMode,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
) -> Result<Result<(), rustix::io::Errno>, ToolError> {
    evidence.checkpoint(
        CreateFolderCheckpoint::BeforeMkdir(component_index, ordinal),
        cancellation,
    );
    if matches!(cancellation_mode, CancellationMode::Observe) {
        check_cancellation(cancellation)?;
    }
    let outcome = evidence.mkdir(
        ordinal,
        component_index,
        parent,
        component,
        REQUESTED_DIRECTORY_MODE,
    );
    evidence.checkpoint(
        CreateFolderCheckpoint::AfterMkdir(component_index, ordinal),
        cancellation,
    );
    if matches!(cancellation_mode, CancellationMode::Observe)
        && !matches!(outcome, Ok(()) | Err(rustix::io::Errno::INTR))
    {
        check_cancellation(cancellation)?;
    }
    Ok(outcome)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn validate_linked_root<Evidence: CreateFolderEvidence>(
    root: BorrowedFd<'_>,
    metadata: &rustix::fs::Stat,
    phase: CreateFolderPhase,
    cancellation_mode: CancellationMode,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<(), ToolError> {
    #[cfg(target_os = "linux")]
    {
        let _ = (root, cancellation_mode, cancellation, evidence, ordinals);
        if metadata.st_nlink == 0 {
            return Err(map_phase_failure(phase));
        }
    }

    #[cfg(target_os = "macos")]
    validate_linked_macos_root(
        root,
        metadata,
        phase,
        cancellation_mode,
        cancellation,
        evidence,
        ordinals,
    )?;

    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn validate_linked_macos_root<Evidence: CreateFolderEvidence>(
    root: BorrowedFd<'_>,
    root_metadata: &rustix::fs::Stat,
    phase: CreateFolderPhase,
    cancellation_mode: CancellationMode,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<(), ToolError> {
    evidence.checkpoint(CreateFolderCheckpoint::BeforeRootPath(phase), cancellation);
    if matches!(cancellation_mode, CancellationMode::Observe) {
        check_cancellation(cancellation)?;
    }
    let root_path = rustix::fs::getpath(root);
    evidence.checkpoint(CreateFolderCheckpoint::AfterRootPath(phase), cancellation);
    if matches!(cancellation_mode, CancellationMode::Observe) {
        check_cancellation(cancellation)?;
    }
    let root_path = root_path.map_err(|_| map_phase_failure(phase))?;
    let root_path = root_path.as_bytes();
    if root_path == b"/" {
        return Ok(());
    }
    let name = root_path
        .rsplit(|byte| *byte == b'/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| map_phase_failure(phase))?;
    let parent = evidence_open(
        root,
        OsStr::new(".."),
        phase,
        CreateFolderOpenSite::RootParent,
        cancellation_mode,
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|error| map_phase_operation_error(error, phase))?;
    let linked = evidence_statat(
        parent.as_fd(),
        OsStr::from_bytes(name),
        phase,
        CreateFolderStatatSite::LinkedRoot,
        cancellation_mode,
        cancellation,
        evidence,
        ordinals,
    )
    .map_err(|error| map_phase_operation_error(error, phase))?;
    if linked.st_dev != root_metadata.st_dev
        || linked.st_ino != root_metadata.st_ino
        || !FileType::from_raw_mode(linked.st_mode).is_dir()
    {
        return Err(map_phase_failure(phase));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn evidence_statat<Evidence: CreateFolderEvidence>(
    parent: BorrowedFd<'_>,
    name: &OsStr,
    phase: CreateFolderPhase,
    site: CreateFolderStatatSite,
    cancellation_mode: CancellationMode,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
    ordinals: &mut OperationOrdinals,
) -> Result<rustix::fs::Stat, EvidenceOperationError> {
    let ordinal = ordinals.next_statat();
    evidence.checkpoint(
        CreateFolderCheckpoint::BeforeStatat(phase, site, ordinal),
        cancellation,
    );
    if matches!(cancellation_mode, CancellationMode::Observe) && cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    let result = evidence.statat(phase, site, ordinal, parent, name);
    evidence.checkpoint(
        CreateFolderCheckpoint::AfterStatat(phase, site, ordinal),
        cancellation,
    );
    if matches!(cancellation_mode, CancellationMode::Observe) && cancellation.is_cancelled() {
        return Err(EvidenceOperationError::Cancelled);
    }
    result.map_err(EvidenceOperationError::Os)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_all_bottom_up<Evidence: CreateFolderEvidence>(
    entries: &[SyncEntry],
    evidence: &mut Evidence,
) -> bool {
    debug_assert!(entries.len() <= MAX_CREATE_FOLDER_PATH_COMPONENTS + 1);
    let mut all_succeeded = true;
    let mut total_calls = 0_usize;
    for entry in entries.iter().rev() {
        let mut site_succeeded = false;
        for attempt in 0..MAX_SYNC_CALLS_PER_SITE {
            debug_assert!(total_calls < MAX_CREATE_FOLDER_SYNC_CALLS);
            total_calls = total_calls.saturating_add(1);
            match evidence.sync_directory(entry.site, attempt, entry.descriptor.as_fd()) {
                Ok(()) => {
                    site_succeeded = true;
                    break;
                }
                Err(error)
                    if error == rustix::io::Errno::INTR
                        && attempt + 1 < MAX_SYNC_CALLS_PER_SITE => {}
                Err(_) => break,
            }
        }
        if !site_succeeded {
            all_succeeded = false;
        }
    }
    all_succeeded
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn precommit_checkpoint<Evidence: CreateFolderEvidence>(
    evidence: &mut Evidence,
    checkpoint: CreateFolderCheckpoint,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    evidence.checkpoint(checkpoint, cancellation);
    check_cancellation(cancellation)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn build_success_output(path: &str) -> Result<ToolOutput, ToolError> {
    let output = ToolOutput::success(json!({ "path": path }));
    if !serialized_value_fits(&output, MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES) {
        return Err(create_failed());
    }
    Ok(output)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(error: rustix::io::Errno) -> CreateFolderToolOpenError {
    let kind = if is_rejected_type_error(error) {
        CreateFolderToolOpenErrorKind::InvalidFileType
    } else {
        CreateFolderToolOpenErrorKind::Unavailable
    };
    CreateFolderToolOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_initial_open_error(error: EvidenceOperationError, final_component: bool) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error) if is_permission_error(error) => permission_denied(),
        EvidenceOperationError::Os(error) if is_rejected_type_error(error) && final_component => {
            target_exists()
        }
        EvidenceOperationError::Os(error) if is_rejected_type_error(error) => path_rejected(),
        EvidenceOperationError::Os(_) => unavailable(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_create_open_error(error: EvidenceOperationError, final_component: bool) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error) if is_permission_error(error) => permission_denied(),
        EvidenceOperationError::Os(error) if is_rejected_type_error(error) && final_component => {
            target_exists()
        }
        EvidenceOperationError::Os(error) if is_rejected_type_error(error) => path_rejected(),
        EvidenceOperationError::Os(_) => target_changed(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_phase_operation_error(error: EvidenceOperationError, phase: CreateFolderPhase) -> ToolError {
    match error {
        EvidenceOperationError::Cancelled => cancelled(),
        EvidenceOperationError::Os(error)
            if phase != CreateFolderPhase::Postcommit && is_permission_error(error) =>
        {
            permission_denied()
        }
        EvidenceOperationError::Os(_) => map_phase_failure(phase),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_phase_failure(phase: CreateFolderPhase) -> ToolError {
    match phase {
        CreateFolderPhase::Initial => unavailable(),
        CreateFolderPhase::Revalidate | CreateFolderPhase::Create => target_changed(),
        CreateFolderPhase::Postcommit => commit_ambiguous(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_mkdir_error(error: rustix::io::Errno) -> ToolError {
    if is_permission_error(error) {
        permission_denied()
    } else if is_rejected_type_error(error) || error == rustix::io::Errno::NOENT {
        target_changed()
    } else {
        create_failed()
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
        "create_folder_invalid_arguments",
        "create_folder arguments are invalid",
        false,
    )
}

fn invalid_path() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "create_folder_invalid_path",
        "create_folder path is invalid",
        false,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported_platform() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "create_folder_unsupported_platform",
        "native create_folder is unsupported on this platform",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn permission_denied() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "create_folder_permission_denied",
        "requested folder cannot be created",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn path_rejected() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "create_folder_path_rejected",
        "requested folder path is not confined",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn target_exists() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "create_folder_target_exists",
        "requested folder path already exists as a non-directory",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "create_folder_unavailable",
        "requested folder is unavailable",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn target_changed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "create_folder_target_changed",
        "requested folder path changed during creation",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_failed() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "create_folder_create_failed",
        "requested folder could not be created",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn commit_ambiguous() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "create_folder_commit_ambiguous",
        "requested folder creation status is uncertain",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "create_folder_cancelled",
        "create_folder execution was cancelled",
        false,
    )
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests;
