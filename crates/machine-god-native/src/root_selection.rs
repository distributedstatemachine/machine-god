use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags};

use crate::workspace::WorkspaceRoot;
#[cfg(feature = "ai-gateway-http")]
use crate::workspace::WorkspaceRootError;
use crate::{FileSessionStore, NativeEnvironment, STATE_NAMESPACE};

const PRIVATE_DIRECTORY_MODE: u64 = 0o700;
const GROUP_OR_OTHER_WRITE: u64 = 0o022;
const GROUP_OR_OTHER_PERMISSIONS: u64 = 0o077;

/// Stable category for native-root selection failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeRootSelectionErrorKind {
    /// The explicitly supplied workspace path is not a safe absolute lexical path.
    InvalidWorkspaceRoot,
    /// Neither state environment input selected a usable value.
    StateRootUnavailable,
    /// The selected state environment value is not absolute Unicode.
    InvalidStateEnvironment,
}

impl NativeRootSelectionErrorKind {
    /// Returns the stable, machine-readable name of this category.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidWorkspaceRoot => "invalid_workspace_root",
            Self::StateRootUnavailable => "state_root_unavailable",
            Self::InvalidStateEnvironment => "invalid_state_environment",
        }
    }
}

/// Fixed, redacted failure to select native workspace and state roots.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeRootSelectionError {
    kind: NativeRootSelectionErrorKind,
}

impl NativeRootSelectionError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> NativeRootSelectionErrorKind {
        self.kind
    }

    const fn new(kind: NativeRootSelectionErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for NativeRootSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRootSelectionError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeRootSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeRootSelectionErrorKind::InvalidWorkspaceRoot => {
                "native workspace root selection is invalid"
            }
            NativeRootSelectionErrorKind::StateRootUnavailable => {
                "native state root selection is unavailable"
            }
            NativeRootSelectionErrorKind::InvalidStateEnvironment => {
                "native state environment selection is invalid"
            }
        })
    }
}

impl Error for NativeRootSelectionError {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StateSelection {
    Xdg { base: PathBuf },
    Home { base: PathBuf },
}

impl StateSelection {
    fn base(&self) -> &Path {
        match self {
            Self::Xdg { base } | Self::Home { base } => base,
        }
    }

    fn suffix(&self) -> &'static [&'static str] {
        match self {
            Self::Xdg { .. } => &[STATE_NAMESPACE],
            Self::Home { .. } => &[".local", "state", STATE_NAMESPACE],
        }
    }
}

/// Pure selection of the workspace and state roots a native host will prepare.
///
/// Selection performs no filesystem operation. A nonempty `XDG_STATE_HOME`
/// takes precedence over `HOME`; an invalid selected value fails without
/// falling back to the lower-precedence input.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeRootSelection {
    workspace_root: PathBuf,
    state_root: PathBuf,
    state_selection: StateSelection,
}

impl NativeRootSelection {
    /// Selects roots from an injected environment snapshot and workspace path.
    ///
    /// # Errors
    ///
    /// Returns a fixed, redacted error if the workspace is not absolute or has
    /// a lexical parent component, or if no valid state environment input is
    /// available.
    pub fn from_environment(
        environment: &NativeEnvironment,
        workspace_root: &Path,
    ) -> Result<Self, NativeRootSelectionError> {
        if !workspace_root.is_absolute()
            || workspace_root
                .components()
                .any(|component| component == Component::ParentDir)
        {
            return Err(NativeRootSelectionError::new(
                NativeRootSelectionErrorKind::InvalidWorkspaceRoot,
            ));
        }
        let workspace_root = workspace_root.components().collect::<PathBuf>();

        let state_selection = select_state(environment)?;

        let state_root = state_selection
            .suffix()
            .iter()
            .fold(state_selection.base().to_path_buf(), |path, part| {
                path.join(part)
            });
        Ok(Self {
            workspace_root,
            state_root,
            state_selection,
        })
    }

    /// Returns the selected absolute workspace path.
    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Returns the selected absolute `machine-god` state path.
    #[must_use]
    pub fn state_root(&self) -> &Path {
        &self.state_root
    }
}

fn select_state(
    environment: &NativeEnvironment,
) -> Result<StateSelection, NativeRootSelectionError> {
    if let Some(value) = nonempty(environment.xdg_state_home.as_deref()) {
        Ok(StateSelection::Xdg {
            base: validated_state_base(value)?,
        })
    } else if let Some(value) = nonempty(environment.home.as_deref()) {
        Ok(StateSelection::Home {
            base: validated_state_base(value)?,
        })
    } else {
        Err(NativeRootSelectionError::new(
            NativeRootSelectionErrorKind::StateRootUnavailable,
        ))
    }
}

impl fmt::Debug for NativeRootSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRootSelection")
            .finish_non_exhaustive()
    }
}

fn nonempty(value: Option<&OsStr>) -> Option<&OsStr> {
    value.filter(|value| !value.is_empty())
}

fn validated_state_base(value: &OsStr) -> Result<PathBuf, NativeRootSelectionError> {
    let path = Path::new(value);
    if value.to_str().is_none() || !path.is_absolute() {
        return Err(NativeRootSelectionError::new(
            NativeRootSelectionErrorKind::InvalidStateEnvironment,
        ));
    }
    Ok(path.components().collect())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExistingSessionStoreError {
    InvalidEnvironment,
    UnsafeStateRoot,
    Unavailable,
}

pub(crate) fn open_existing_session_store(
    environment: &NativeEnvironment,
) -> Result<Option<FileSessionStore>, ExistingSessionStoreError> {
    let selection =
        select_state(environment).map_err(|_| ExistingSessionStoreError::InvalidEnvironment)?;
    let effective_uid = rustix::process::geteuid().as_raw();
    let Some(mut state_root) = open_existing_state_base(selection.base())? else {
        return Ok(None);
    };
    validate_listing_directory(&state_root, effective_uid, false)?;

    for (index, component) in selection.suffix().iter().enumerate() {
        let is_final = index + 1 == selection.suffix().len();
        let Some(next) = open_existing_suffix_directory(state_root.as_fd(), component)? else {
            return Ok(None);
        };
        validate_listing_directory(&next, effective_uid, is_final)?;
        state_root = next;
    }

    Ok(Some(FileSessionStore::from_root_descriptor(state_root)))
}

/// Stable category for native-root preparation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PreparedNativeRootsErrorKind {
    /// The selected workspace could not be retained as a directory.
    WorkspaceRoot,
    /// The selected, already-existing state base could not be retained.
    StateBase,
    /// A fixed state-root suffix could not be safely opened or created.
    StateRoot,
    /// A state directory failed ownership, mode, or macOS ACL safety checks.
    UnsafeStateDirectory,
    /// The retained workspace and final state roots overlap.
    OverlappingRoots,
}

impl PreparedNativeRootsErrorKind {
    /// Returns the stable, machine-readable name of this category.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceRoot => "workspace_root",
            Self::StateBase => "state_base",
            Self::StateRoot => "state_root",
            Self::UnsafeStateDirectory => "unsafe_state_directory",
            Self::OverlappingRoots => "overlapping_roots",
        }
    }
}

/// Fixed, redacted failure to prepare selected native roots.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct PreparedNativeRootsError {
    kind: PreparedNativeRootsErrorKind,
}

impl PreparedNativeRootsError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> PreparedNativeRootsErrorKind {
        self.kind
    }

    const fn new(kind: PreparedNativeRootsErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for PreparedNativeRootsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedNativeRootsError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for PreparedNativeRootsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            PreparedNativeRootsErrorKind::WorkspaceRoot => {
                "native workspace root preparation failed"
            }
            PreparedNativeRootsErrorKind::StateBase => "native state base preparation failed",
            PreparedNativeRootsErrorKind::StateRoot => "native state root preparation failed",
            PreparedNativeRootsErrorKind::UnsafeStateDirectory => {
                "native state directory is unsafe"
            }
            PreparedNativeRootsErrorKind::OverlappingRoots => {
                "native workspace and state roots overlap"
            }
        })
    }
}

impl Error for PreparedNativeRootsError {}

/// Authoritative retained descriptors for a selected workspace and state root.
///
/// Preparation opens the workspace first. It never creates the selected state
/// base, and creates only the fixed state suffix when components are absent.
/// Existing directories are never chmodded or otherwise repaired.
pub struct PreparedNativeRoots {
    selection: NativeRootSelection,
    workspace: WorkspaceRoot,
    session_store: FileSessionStore,
}

impl PreparedNativeRoots {
    /// Opens, validates, and when necessary creates the fixed state-root suffix.
    ///
    /// # Errors
    ///
    /// Returns a fixed, redacted error if a root cannot be retained, a state
    /// directory fails ownership, mode, or macOS ACL safety validation, or
    /// descriptor identity proves that the workspace and final state root
    /// overlap.
    pub fn prepare(selection: NativeRootSelection) -> Result<Self, PreparedNativeRootsError> {
        let workspace = WorkspaceRoot::open(selection.workspace_root()).map_err(|_| {
            PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::WorkspaceRoot)
        })?;
        let effective_uid = rustix::process::geteuid().as_raw();
        let state_base = open_state_base(selection.state_selection.base())?;
        validate_existing_directory(&state_base, effective_uid, false)?;

        if descriptor_is_ancestor(workspace.descriptor(), &state_base)
            .map_err(|()| PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateBase))?
        {
            return Err(PreparedNativeRootsError::new(
                PreparedNativeRootsErrorKind::OverlappingRoots,
            ));
        }

        let mut state_root = state_base;
        for (index, component) in selection.state_selection.suffix().iter().enumerate() {
            let is_final = index + 1 == selection.state_selection.suffix().len();
            state_root =
                prepare_suffix_directory(state_root.as_fd(), component, effective_uid, is_final)?;
        }

        let overlaps = descriptor_is_ancestor(workspace.descriptor(), &state_root)
            .and_then(|workspace_contains_state| {
                if workspace_contains_state {
                    Ok(true)
                } else {
                    descriptor_is_ancestor(&state_root, workspace.descriptor())
                }
            })
            .map_err(|()| PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateRoot))?;
        if overlaps {
            return Err(PreparedNativeRootsError::new(
                PreparedNativeRootsErrorKind::OverlappingRoots,
            ));
        }

        Ok(Self {
            selection,
            workspace,
            session_store: FileSessionStore::from_root_descriptor(state_root),
        })
    }

    /// Returns the pure selection used to prepare these retained roots.
    #[must_use]
    pub const fn selection(&self) -> &NativeRootSelection {
        &self.selection
    }

    /// Returns the selected workspace path without reopening it.
    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        self.selection.workspace_root()
    }

    /// Returns the selected state path without reopening it.
    #[must_use]
    pub fn state_root(&self) -> &Path {
        self.selection.state_root()
    }

    #[cfg(feature = "ai-gateway-http")]
    pub(crate) fn into_parts(
        self,
    ) -> Result<(crate::workspace::WorkspaceTools, FileSessionStore), WorkspaceRootError> {
        let tools = self.workspace.into_tools()?;
        Ok((tools, self.session_store))
    }
}

impl fmt::Debug for PreparedNativeRoots {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = (&self.workspace, &self.session_store);
        formatter
            .debug_struct("PreparedNativeRoots")
            .finish_non_exhaustive()
    }
}

fn open_state_base(path: &Path) -> Result<OwnedFd, PreparedNativeRootsError> {
    let descriptor = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateBase))?;
    ensure_directory(&descriptor)
        .map_err(|()| PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateBase))?;
    Ok(descriptor)
}

fn open_existing_state_base(path: &Path) -> Result<Option<OwnedFd>, ExistingSessionStoreError> {
    let path_metadata = match rustix::fs::statat(CWD, path, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => metadata,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(_) => return Err(ExistingSessionStoreError::Unavailable),
    };
    if !FileType::from_raw_mode(path_metadata.st_mode).is_dir() {
        return Err(ExistingSessionStoreError::UnsafeStateRoot);
    }

    let descriptor = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| ExistingSessionStoreError::Unavailable)?;
    let descriptor_metadata =
        rustix::fs::fstat(&descriptor).map_err(|_| ExistingSessionStoreError::Unavailable)?;
    if !same_identity(&path_metadata, &descriptor_metadata)
        || !FileType::from_raw_mode(descriptor_metadata.st_mode).is_dir()
    {
        return Err(ExistingSessionStoreError::Unavailable);
    }
    Ok(Some(descriptor))
}

fn open_existing_suffix_directory(
    parent: BorrowedFd<'_>,
    name: &str,
) -> Result<Option<OwnedFd>, ExistingSessionStoreError> {
    let path_metadata = match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => metadata,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(_) => return Err(ExistingSessionStoreError::Unavailable),
    };
    if !FileType::from_raw_mode(path_metadata.st_mode).is_dir() {
        return Err(ExistingSessionStoreError::UnsafeStateRoot);
    }

    let Ok(descriptor) = rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) else {
        return Err(ExistingSessionStoreError::Unavailable);
    };
    let descriptor_metadata =
        rustix::fs::fstat(&descriptor).map_err(|_| ExistingSessionStoreError::Unavailable)?;
    if !same_identity(&path_metadata, &descriptor_metadata)
        || !FileType::from_raw_mode(descriptor_metadata.st_mode).is_dir()
    {
        return Err(ExistingSessionStoreError::Unavailable);
    }
    Ok(Some(descriptor))
}

fn validate_listing_directory(
    descriptor: &OwnedFd,
    effective_uid: u32,
    is_final: bool,
) -> Result<(), ExistingSessionStoreError> {
    validate_existing_directory(descriptor, effective_uid, is_final).map_err(|error| {
        match error.kind() {
            PreparedNativeRootsErrorKind::UnsafeStateDirectory => {
                ExistingSessionStoreError::UnsafeStateRoot
            }
            PreparedNativeRootsErrorKind::WorkspaceRoot
            | PreparedNativeRootsErrorKind::StateBase
            | PreparedNativeRootsErrorKind::StateRoot
            | PreparedNativeRootsErrorKind::OverlappingRoots => {
                ExistingSessionStoreError::Unavailable
            }
        }
    })
}

fn prepare_suffix_directory(
    parent: BorrowedFd<'_>,
    name: &str,
    effective_uid: u32,
    is_final: bool,
) -> Result<OwnedFd, PreparedNativeRootsError> {
    let existed_before = match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => true,
        Err(error) if error == rustix::io::Errno::NOENT => false,
        Err(_) => {
            return Err(PreparedNativeRootsError::new(
                PreparedNativeRootsErrorKind::StateRoot,
            ));
        }
    };

    let created = if existed_before {
        false
    } else {
        match rustix::fs::mkdirat(parent, name, private_directory_mode()) {
            Ok(()) => {
                // A process umask may remove owner bits and make the new
                // directory impossible to open. The parent descriptor has
                // already been validated as effective-UID-owned and not
                // group/other-writable, so same-UID mutation is the remaining
                // trust boundary while this fixed name is normalized. Empty
                // flags are required because Linux does not implement
                // `AT_SYMLINK_NOFOLLOW` for `fchmodat`.
                rustix::fs::chmodat(parent, name, private_directory_mode(), AtFlags::empty())
                    .map_err(|_| {
                        PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateRoot)
                    })?;
                true
            }
            Err(error) if error == rustix::io::Errno::EXIST => false,
            Err(_) => {
                return Err(PreparedNativeRootsError::new(
                    PreparedNativeRootsErrorKind::StateRoot,
                ));
            }
        }
    };

    let path_metadata = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateRoot))?;
    if !FileType::from_raw_mode(path_metadata.st_mode).is_dir() {
        return Err(PreparedNativeRootsError::new(
            PreparedNativeRootsErrorKind::StateRoot,
        ));
    }
    let descriptor = rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateRoot))?;
    let descriptor_metadata = rustix::fs::fstat(&descriptor)
        .map_err(|_| PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateRoot))?;
    if !same_identity(&path_metadata, &descriptor_metadata)
        || !FileType::from_raw_mode(descriptor_metadata.st_mode).is_dir()
    {
        return Err(PreparedNativeRootsError::new(
            PreparedNativeRootsErrorKind::StateRoot,
        ));
    }

    if created {
        rustix::fs::fchmod(&descriptor, private_directory_mode())
            .map_err(|_| PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateRoot))?;
        let metadata = rustix::fs::fstat(&descriptor)
            .map_err(|_| PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateRoot))?;
        if metadata.st_uid != effective_uid
            || u64::from(metadata.st_mode) & 0o777 != PRIVATE_DIRECTORY_MODE
        {
            return Err(PreparedNativeRootsError::new(
                PreparedNativeRootsErrorKind::UnsafeStateDirectory,
            ));
        }
        #[cfg(target_os = "macos")]
        validate_extended_acl(&descriptor)?;
    } else {
        validate_existing_directory(&descriptor, effective_uid, is_final)?;
    }
    Ok(descriptor)
}

fn ensure_directory(descriptor: &OwnedFd) -> Result<(), ()> {
    let metadata = rustix::fs::fstat(descriptor).map_err(|_| ())?;
    if FileType::from_raw_mode(metadata.st_mode).is_dir() {
        Ok(())
    } else {
        Err(())
    }
}

fn validate_existing_directory(
    descriptor: &OwnedFd,
    effective_uid: u32,
    is_final: bool,
) -> Result<(), PreparedNativeRootsError> {
    let metadata = rustix::fs::fstat(descriptor)
        .map_err(|_| PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::StateRoot))?;
    let permissions = u64::from(metadata.st_mode);
    if !FileType::from_raw_mode(metadata.st_mode).is_dir()
        || metadata.st_uid != effective_uid
        || permissions & GROUP_OR_OTHER_WRITE != 0
        || (is_final && permissions & GROUP_OR_OTHER_PERMISSIONS != 0)
    {
        return Err(PreparedNativeRootsError::new(
            PreparedNativeRootsErrorKind::UnsafeStateDirectory,
        ));
    }
    #[cfg(target_os = "macos")]
    validate_extended_acl(descriptor)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_extended_acl(descriptor: &OwnedFd) -> Result<(), PreparedNativeRootsError> {
    let acl = calcifer_macos_acl::read_acl(descriptor.as_fd()).map_err(|_| {
        PreparedNativeRootsError::new(PreparedNativeRootsErrorKind::UnsafeStateDirectory)
    })?;
    if acl.flags != 0
        || acl.entries.iter().any(|entry| {
            entry.tag != calcifer_macos_acl::TAG_DENY
                || entry.flags != 0
                || entry.permissions != calcifer_macos_acl::PERMISSION_DELETE
        })
    {
        return Err(PreparedNativeRootsError::new(
            PreparedNativeRootsErrorKind::UnsafeStateDirectory,
        ));
    }
    Ok(())
}

fn descriptor_is_ancestor(ancestor: &OwnedFd, descendant: &OwnedFd) -> Result<bool, ()> {
    let ancestor_metadata = rustix::fs::fstat(ancestor).map_err(|_| ())?;
    let mut current = descendant.try_clone().map_err(|_| ())?;
    loop {
        let current_metadata = rustix::fs::fstat(&current).map_err(|_| ())?;
        if same_identity(&ancestor_metadata, &current_metadata) {
            return Ok(true);
        }
        let parent = rustix::fs::openat(
            &current,
            "..",
            OFlags::RDONLY
                | OFlags::DIRECTORY
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| ())?;
        let parent_metadata = rustix::fs::fstat(&parent).map_err(|_| ())?;
        if same_identity(&current_metadata, &parent_metadata) {
            return Ok(false);
        }
        current = parent;
    }
}

fn same_identity(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> bool {
    left.st_dev == right.st_dev && left.st_ino == right.st_ino
}

fn private_directory_mode() -> Mode {
    Mode::RUSR | Mode::WUSR | Mode::XUSR
}
