//! Explicit descriptor authority for immutable, generation-pinned workspace scopes.

use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock, Weak};

use rustix::fd::OwnedFd;
use rustix::fs::{FileType, Mode, OFlags, Stat};

const MAX_PATH_BYTES: usize = 4096;
const MAX_ADDITIONAL: usize = 16;
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

/// Fixed errors, without paths or operating-system diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeWorkspaceAuthorityError {
    InvalidPath,
    TooManyDirectories,
    DuplicateRoot,
    OverlappingState,
    Unavailable,
    StaleGeneration,
    WrongAuthority,
}

impl fmt::Display for NativeWorkspaceAuthorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "workspace authority: {self:?}")
    }
}

impl std::error::Error for NativeWorkspaceAuthorityError {}

type Result<T> = std::result::Result<T, NativeWorkspaceAuthorityError>;

/// A retained source spelling and its observed (or provisional) absolute identity.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeWorkspaceSource {
    source: PathBuf,
    identity: PathBuf,
    identity_canonical: bool,
}

impl NativeWorkspaceSource {
    /// Validates path bytes and lexical components without filesystem effects.
    ///
    /// # Errors
    /// Returns `InvalidPath` for invalid bytes, parent components, or relative identity.
    pub fn new(source: PathBuf, mut identity: PathBuf, identity_canonical: bool) -> Result<Self> {
        validate_path(&source, false)?;
        validate_path(&identity, true)?;
        let normalized = normalize(&identity);
        if normalized.as_os_str() != identity.as_os_str() {
            identity = normalized;
        }
        Ok(Self {
            source,
            identity,
            identity_canonical,
        })
    }

    #[must_use]
    pub fn source(&self) -> &Path {
        &self.source
    }
    #[must_use]
    pub fn identity(&self) -> &Path {
        &self.identity
    }
    #[must_use]
    pub const fn identity_canonical(&self) -> bool {
        self.identity_canonical
    }
}

impl fmt::Debug for NativeWorkspaceSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWorkspaceSource")
            .field("identity_canonical", &self.identity_canonical)
            .finish_non_exhaustive()
    }
}

/// One unique additional identity, possibly present in both saved and launch sources.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeWorkspaceEntrySpec {
    source: NativeWorkspaceSource,
    saved: bool,
    launch: bool,
    saved_record: Option<crate::NativeSavedWorkspaceDirectory>,
}

impl NativeWorkspaceEntrySpec {
    /// # Errors
    /// Returns `InvalidPath` if neither source provenance is present.
    pub fn new(source: NativeWorkspaceSource, saved: bool, launch: bool) -> Result<Self> {
        if !saved && !launch {
            return Err(NativeWorkspaceAuthorityError::InvalidPath);
        }
        Ok(Self {
            source,
            saved,
            launch,
            saved_record: None,
        })
    }
    #[must_use]
    pub const fn source(&self) -> &NativeWorkspaceSource {
        &self.source
    }
    #[must_use]
    pub const fn saved(&self) -> bool {
        self.saved
    }
    #[must_use]
    pub const fn launch(&self) -> bool {
        self.launch
    }

    pub(crate) fn saved_record(&self) -> Option<&crate::NativeSavedWorkspaceDirectory> {
        self.saved_record.as_ref()
    }

    pub(crate) fn with_saved_record(
        mut self,
        record: crate::NativeSavedWorkspaceDirectory,
    ) -> Result<Self> {
        if !self.saved
            || self.source.source.as_os_str().as_bytes() != record.source_bytes()
            || (record.identity_canonical()
                && self.source.identity.as_os_str().as_bytes() != record.identity_bytes())
        {
            return Err(NativeWorkspaceAuthorityError::InvalidPath);
        }
        self.saved_record = Some(record);
        Ok(self)
    }

    pub(crate) fn include_launch_source(&mut self) {
        self.launch = true;
    }
    pub(crate) fn include_saved_source(&mut self) {
        self.saved = true;
    }
}

struct Root {
    descriptor: OwnedFd,
    identity: PathBuf,
    metadata: Stat,
}

struct StateRoot {
    identity: PathBuf,
    ancestor: Root,
    exists: bool,
}

/// An entry remains visible when unavailable or suppressed.
#[derive(Clone)]
pub struct NativeWorkspaceEntry {
    spec: NativeWorkspaceEntrySpec,
    root: Option<Arc<Root>>,
    active: bool,
}

impl NativeWorkspaceEntry {
    #[must_use]
    pub const fn spec(&self) -> &NativeWorkspaceEntrySpec {
        &self.spec
    }
    #[must_use]
    pub const fn source(&self) -> &NativeWorkspaceSource {
        &self.spec.source
    }
    #[must_use]
    pub const fn saved(&self) -> bool {
        self.spec.saved
    }
    #[must_use]
    pub const fn launch(&self) -> bool {
        self.spec.launch
    }
    #[must_use]
    pub const fn available(&self) -> bool {
        self.root.is_some()
    }
    #[must_use]
    pub const fn active(&self) -> bool {
        self.active
    }
}

impl fmt::Debug for NativeWorkspaceEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWorkspaceEntry")
            .field("spec", &self.spec)
            .field("available", &self.available())
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

struct Scope {
    generation: u64,
    primary: Arc<Root>,
    state: Arc<StateRoot>,
    entries: Vec<NativeWorkspaceEntry>,
    saved_suppressed: bool,
}

/// An immutable scope. Cloning it retains the exact open root descriptors.
#[derive(Clone)]
pub struct NativeWorkspaceScopeSnapshot(Arc<Scope>);

impl NativeWorkspaceScopeSnapshot {
    #[cfg(feature = "ai-gateway-http")]
    pub(crate) fn validate_host_binding(
        &self,
        primary: &OwnedFd,
        primary_identity: &Path,
        state: &OwnedFd,
    ) -> Result<()> {
        if self.primary_identity() != primary_identity || !self.0.state.exists {
            return Err(NativeWorkspaceAuthorityError::WrongAuthority);
        }
        // Prepared state paths can contain aliases (for example /var on macOS).
        // Compare the actually retained objects, not a second pathname lookup.
        for (supplied, retained) in [
            (primary, self.0.primary.as_ref()),
            (state, &self.0.state.ancestor),
        ] {
            let metadata = rustix::fs::fstat(supplied)
                .map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
            let current = rustix::fs::fstat(&retained.descriptor)
                .map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir()
                || metadata.st_nlink == 0
                || !same_identity(&metadata, &current)
                || !same_identity(&metadata, &retained.metadata)
            {
                return Err(NativeWorkspaceAuthorityError::WrongAuthority);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.0.generation
    }
    #[must_use]
    pub fn primary_identity(&self) -> &Path {
        &self.0.primary.identity
    }
    #[must_use]
    pub fn entries(&self) -> &[NativeWorkspaceEntry] {
        &self.0.entries
    }
    #[must_use]
    pub fn saved_suppressed(&self) -> bool {
        self.0.saved_suppressed
    }

    /// Selects a retained root lexically; performs no filesystem operation.
    /// Descendant symlinks must still be validated by the consuming tool/preparer.
    ///
    /// # Errors
    /// Returns `InvalidPath` for traversal, state paths, or paths outside active roots.
    pub fn route(&self, path: &Path) -> Result<NativeWorkspaceRoute> {
        validate_path(path, false)?;
        let path = normalize(path);
        let absolute = if path.is_absolute() {
            path
        } else {
            self.primary_identity().join(path)
        };
        validate_path(&absolute, true)?;
        if absolute.starts_with(&self.0.state.identity) {
            return Err(NativeWorkspaceAuthorityError::InvalidPath);
        }
        let root = std::iter::once(&self.0.primary)
            .chain(
                self.0
                    .entries
                    .iter()
                    .filter(|entry| entry.active)
                    .filter_map(|entry| entry.root.as_ref()),
            )
            .find(|root| absolute.starts_with(&root.identity))
            .ok_or(NativeWorkspaceAuthorityError::InvalidPath)?;
        let relative = absolute
            .strip_prefix(&root.identity)
            .map_err(|_| NativeWorkspaceAuthorityError::InvalidPath)?;
        Ok(NativeWorkspaceRoute {
            scope: self.clone(),
            root: Arc::clone(root),
            relative: if relative.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative.to_path_buf()
            },
        })
    }
}

impl fmt::Debug for NativeWorkspaceScopeSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWorkspaceScopeSnapshot")
            .field("generation", &self.generation())
            .field("entries", &self.entries())
            .field("saved_suppressed", &self.saved_suppressed())
            .finish_non_exhaustive()
    }
}

/// A pure route retaining its generation and exact root descriptor.
#[derive(Clone)]
pub struct NativeWorkspaceRoute {
    scope: NativeWorkspaceScopeSnapshot,
    root: Arc<Root>,
    relative: PathBuf,
}

impl NativeWorkspaceRoute {
    #[must_use]
    pub fn root_descriptor(&self) -> &OwnedFd {
        &self.root.descriptor
    }
    #[must_use]
    pub fn root_identity(&self) -> &Path {
        &self.root.identity
    }
    #[must_use]
    pub fn relative_path(&self) -> &Path {
        &self.relative
    }
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.scope.generation()
    }
}

impl fmt::Debug for NativeWorkspaceRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWorkspaceRoute")
            .field("generation", &self.generation())
            .finish_non_exhaustive()
    }
}

struct Manager {
    current: RwLock<NativeWorkspaceScopeSnapshot>,
}

/// Shared scope manager. Blocking preparation must run on the host's owned worker.
#[derive(Clone)]
pub struct NativeWorkspaceAuthority(Arc<Manager>);

/// A replacement bound to one manager and one observed generation.
pub struct NativeWorkspacePreparedInstall {
    manager: Weak<Manager>,
    expected_generation: u64,
    replacement: NativeWorkspaceScopeSnapshot,
}

impl NativeWorkspacePreparedInstall {
    #[must_use]
    pub const fn snapshot(&self) -> &NativeWorkspaceScopeSnapshot {
        &self.replacement
    }
}

impl fmt::Debug for NativeWorkspacePreparedInstall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWorkspacePreparedInstall")
            .field("expected_generation", &self.expected_generation)
            .field("replacement", &self.replacement)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for NativeWorkspaceAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWorkspaceAuthority")
            .finish_non_exhaustive()
    }
}

impl NativeWorkspaceAuthority {
    /// Validates already-owned primary/state descriptors and opens additional roots.
    /// An absent state target uses `None` plus its explicit path, retaining its
    /// nearest existing directory as exclusion proof without creating anything.
    /// `None` is rejected for an existing target. This is explicitly blocking;
    /// it never discovers paths or creates directories.
    ///
    /// # Errors
    /// Returns a fixed error for invalid, duplicate, unavailable, or overlapping roots.
    pub fn open_blocking(
        primary: OwnedFd,
        primary_identity: PathBuf,
        state: Option<OwnedFd>,
        state_identity: PathBuf,
        entries: Vec<NativeWorkspaceEntrySpec>,
        saved_suppressed: bool,
    ) -> Result<Self> {
        validate_specs(&entries)?;
        let primary = Arc::new(validate_root(primary, primary_identity)?);
        let state = Arc::new(open_state(state, state_identity)?);
        reject_state_overlap(&primary, &state)?;
        let entries = open_entries(&primary, &state, entries, saved_suppressed)?;
        Ok(Self(Arc::new(Manager {
            current: RwLock::new(NativeWorkspaceScopeSnapshot(Arc::new(Scope {
                generation: 0,
                primary,
                state,
                entries,
                saved_suppressed,
            }))),
        })))
    }

    /// Returns the current immutable scope without filesystem effects.
    ///
    /// # Errors
    /// Returns `Unavailable` if the publication lock was poisoned.
    pub fn snapshot(&self) -> Result<NativeWorkspaceScopeSnapshot> {
        self.0
            .current
            .read()
            .map(|scope| scope.clone())
            .map_err(|_| NativeWorkspaceAuthorityError::Unavailable)
    }

    /// Prepares a replacement without changing the current scope.
    ///
    /// # Errors
    /// Returns a fixed validation/opening error, or `StaleGeneration` at generation exhaustion.
    pub fn prepare_blocking(
        &self,
        entries: Vec<NativeWorkspaceEntrySpec>,
        saved_suppressed: bool,
    ) -> Result<NativeWorkspacePreparedInstall> {
        validate_specs(&entries)?;
        let current = self.snapshot()?;
        let generation = current
            .generation()
            .checked_add(1)
            .ok_or(NativeWorkspaceAuthorityError::StaleGeneration)?;
        let entries = open_entries(
            &current.0.primary,
            &current.0.state,
            entries,
            saved_suppressed,
        )?;
        Ok(NativeWorkspacePreparedInstall {
            manager: Arc::downgrade(&self.0),
            expected_generation: current.generation(),
            replacement: NativeWorkspaceScopeSnapshot(Arc::new(Scope {
                generation,
                primary: Arc::clone(&current.0.primary),
                state: Arc::clone(&current.0.state),
                entries,
                saved_suppressed,
            })),
        })
    }

    /// Refreshes availability using retained identities, never retargeting canonical sources.
    ///
    /// # Errors
    /// Returns the same validation/opening errors as `prepare_blocking`.
    pub fn refresh_blocking(&self) -> Result<NativeWorkspacePreparedInstall> {
        let current = self.snapshot()?;
        self.prepare_from_snapshot(&current)
    }

    fn prepare_from_snapshot(
        &self,
        current: &NativeWorkspaceScopeSnapshot,
    ) -> Result<NativeWorkspacePreparedInstall> {
        let generation = current
            .generation()
            .checked_add(1)
            .ok_or(NativeWorkspaceAuthorityError::StaleGeneration)?;
        let entries = open_entries(
            &current.0.primary,
            &current.0.state,
            current
                .entries()
                .iter()
                .map(|entry| entry.spec.clone())
                .collect(),
            current.saved_suppressed(),
        )?;
        Ok(NativeWorkspacePreparedInstall {
            manager: Arc::downgrade(&self.0),
            expected_generation: current.generation(),
            replacement: NativeWorkspaceScopeSnapshot(Arc::new(Scope {
                generation,
                primary: Arc::clone(&current.0.primary),
                state: Arc::clone(&current.0.state),
                entries,
                saved_suppressed: current.saved_suppressed(),
            })),
        })
    }

    // Only an admitted native owner may publish a prepared replacement.
    pub(crate) fn install(
        &self,
        prepared: NativeWorkspacePreparedInstall,
    ) -> Result<NativeWorkspaceScopeSnapshot> {
        if !Weak::ptr_eq(&prepared.manager, &Arc::downgrade(&self.0)) {
            return Err(NativeWorkspaceAuthorityError::WrongAuthority);
        }
        let mut current = self
            .0
            .current
            .write()
            .map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
        if current.generation() != prepared.expected_generation {
            return Err(NativeWorkspaceAuthorityError::StaleGeneration);
        }
        *current = prepared.replacement;
        Ok(current.clone())
    }
}

fn validate_path(path: &Path, absolute: bool) -> Result<()> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_PATH_BYTES
        || bytes.contains(&0)
        || (absolute && !path.is_absolute())
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(NativeWorkspaceAuthorityError::InvalidPath);
    }
    Ok(())
}

fn normalize(path: &Path) -> PathBuf {
    path.components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}

fn validate_specs(entries: &[NativeWorkspaceEntrySpec]) -> Result<()> {
    if entries.len() > MAX_ADDITIONAL {
        return Err(NativeWorkspaceAuthorityError::TooManyDirectories);
    }
    for (index, entry) in entries.iter().enumerate() {
        if entries[..index]
            .iter()
            .any(|other| other.source.identity == entry.source.identity)
        {
            return Err(NativeWorkspaceAuthorityError::DuplicateRoot);
        }
    }
    Ok(())
}

fn validate_root(descriptor: OwnedFd, mut identity: PathBuf) -> Result<Root> {
    validate_path(&identity, true)?;
    let normalized = normalize(&identity);
    if normalized.as_os_str() != identity.as_os_str() {
        identity = normalized;
    }
    let check = rustix::fs::open(&identity, DIRECTORY_FLAGS, Mode::empty())
        .map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    let metadata =
        rustix::fs::fstat(&descriptor).map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    let check_metadata =
        rustix::fs::fstat(&check).map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    let canonical =
        std::fs::canonicalize(&identity).map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    let canonical_metadata =
        rustix::fs::stat(&canonical).map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    if !FileType::from_raw_mode(metadata.st_mode).is_dir()
        || canonical != identity
        || !same_identity(&metadata, &check_metadata)
        || !same_identity(&metadata, &canonical_metadata)
    {
        return Err(NativeWorkspaceAuthorityError::Unavailable);
    }
    Ok(Root {
        descriptor,
        identity,
        metadata,
    })
}

fn open_entries(
    primary: &Arc<Root>,
    state: &Arc<StateRoot>,
    entries: Vec<NativeWorkspaceEntrySpec>,
    suppressed: bool,
) -> Result<Vec<NativeWorkspaceEntry>> {
    revalidate_root(primary)?;
    revalidate_root(&state.ancestor)?;
    if !state.exists {
        match std::fs::symlink_metadata(&state.identity) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(NativeWorkspaceAuthorityError::Unavailable),
        }
        let (_, projected) = nearest_directory(&state.identity)?;
        if projected != state.identity {
            return Err(NativeWorkspaceAuthorityError::Unavailable);
        }
    }
    let mut opened: Vec<NativeWorkspaceEntry> = Vec::with_capacity(entries.len());
    for mut spec in entries {
        let observed = if spec.source.identity_canonical {
            spec.source.identity.clone()
        } else if spec.source.source.is_absolute() {
            normalize(&spec.source.source)
        } else {
            primary.identity.join(normalize(&spec.source.source))
        };
        validate_path(&observed, true)?;
        reject_lexical_state(&spec.source.identity, &state.identity)?;
        reject_lexical_state(&observed, &state.identity)?;
        if spec.source.identity == primary.identity || observed == primary.identity {
            return Err(NativeWorkspaceAuthorityError::DuplicateRoot);
        }
        let root = open_additional(&observed, spec.source.identity_canonical)?;
        if let Some(root) = &root {
            reject_state_overlap(root, state)?;
            if same_identity(&root.metadata, &primary.metadata) {
                return Err(NativeWorkspaceAuthorityError::DuplicateRoot);
            }
            spec.source.identity.clone_from(&root.identity);
            spec.source.identity_canonical = true;
        } else if !spec.source.identity_canonical {
            spec.source.identity = provisional_identity(&observed, state)?;
            if spec.source.identity == primary.identity {
                return Err(NativeWorkspaceAuthorityError::DuplicateRoot);
            }
        }
        for other in &opened {
            if other.source().identity == spec.source.identity
                || root
                    .as_ref()
                    .zip(other.root.as_ref())
                    .is_some_and(|(left, right)| same_identity(&left.metadata, &right.metadata))
            {
                return Err(NativeWorkspaceAuthorityError::DuplicateRoot);
            }
        }
        let active = root.is_some() && (spec.launch || (spec.saved && !suppressed));
        opened.push(NativeWorkspaceEntry {
            spec,
            root: root.map(Arc::new),
            active,
        });
    }
    Ok(opened)
}

fn revalidate_root(root: &Root) -> Result<()> {
    let descriptor = root
        .descriptor
        .try_clone()
        .map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    validate_root(descriptor, root.identity.clone()).map(|_| ())
}

fn provisional_identity(observed: &Path, state: &StateRoot) -> Result<PathBuf> {
    // Resolve only an existing directory prefix. Missing tails remain visible,
    // and an alias into private state cannot hide behind an unavailable leaf.
    let (ancestor, identity) = nearest_directory(observed)?;
    if state.exists && descriptor_ancestor(&state.ancestor.descriptor, &ancestor.descriptor)? {
        return Err(NativeWorkspaceAuthorityError::OverlappingState);
    }
    reject_lexical_state(&identity, &state.identity)?;
    Ok(identity)
}

fn open_state(descriptor: Option<OwnedFd>, identity: PathBuf) -> Result<StateRoot> {
    validate_path(&identity, true)?;
    if let Some(descriptor) = descriptor {
        let ancestor = validate_root(descriptor, identity)?;
        return Ok(StateRoot {
            identity: ancestor.identity.clone(),
            ancestor,
            exists: true,
        });
    }
    match std::fs::symlink_metadata(&identity) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(NativeWorkspaceAuthorityError::Unavailable),
    }
    let (ancestor, projected) = nearest_directory(&normalize(&identity))?;
    if projected == ancestor.identity {
        return Err(NativeWorkspaceAuthorityError::Unavailable);
    }
    Ok(StateRoot {
        identity: projected,
        ancestor,
        exists: false,
    })
}

fn nearest_directory(observed: &Path) -> Result<(Root, PathBuf)> {
    for ancestor in observed.ancestors().take(MAX_PATH_BYTES) {
        let canonical = match std::fs::canonicalize(ancestor) {
            Ok(canonical) => canonical,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                continue;
            }
            Err(_) => return Err(NativeWorkspaceAuthorityError::Unavailable),
        };
        let descriptor = match rustix::fs::open(&canonical, DIRECTORY_FLAGS, Mode::empty()) {
            Ok(descriptor) => descriptor,
            Err(rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR) => continue,
            Err(_) => return Err(NativeWorkspaceAuthorityError::Unavailable),
        };
        let retained = validate_root(descriptor, canonical.clone())?;
        let tail = observed
            .strip_prefix(ancestor)
            .map_err(|_| NativeWorkspaceAuthorityError::InvalidPath)?;
        let identity = canonical.join(tail);
        validate_path(&identity, true)?;
        return Ok((retained, identity));
    }
    Err(NativeWorkspaceAuthorityError::Unavailable)
}

fn open_additional(observed: &Path, canonical_identity: bool) -> Result<Option<Root>> {
    // A provisional source has not acquired a stable identity yet. Resolve it
    // once, then open the resolved path without following a replaced leaf.
    // Canonical identities never resolve through their historical source again.
    let opened_identity = if canonical_identity {
        observed.to_path_buf()
    } else {
        match std::fs::canonicalize(observed) {
            Ok(identity) => identity,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(None);
            }
            Err(_) => return Err(NativeWorkspaceAuthorityError::Unavailable),
        }
    };
    let descriptor = match rustix::fs::open(&opened_identity, DIRECTORY_FLAGS, Mode::empty()) {
        Ok(descriptor) => descriptor,
        Err(rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR | rustix::io::Errno::LOOP) => {
            return Ok(None);
        }
        Err(_) => return Err(NativeWorkspaceAuthorityError::Unavailable),
    };
    let metadata =
        rustix::fs::fstat(&descriptor).map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    let identity = std::fs::canonicalize(&opened_identity)
        .map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    validate_path(&identity, true)?;
    if canonical_identity && identity != observed {
        return Ok(None);
    }
    let checked =
        rustix::fs::stat(&identity).map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    if !FileType::from_raw_mode(metadata.st_mode).is_dir() || !same_identity(&metadata, &checked) {
        return Err(NativeWorkspaceAuthorityError::Unavailable);
    }
    Ok(Some(Root {
        descriptor,
        identity,
        metadata,
    }))
}

fn reject_lexical_state(root: &Path, state: &Path) -> Result<()> {
    if root.starts_with(state) || state.starts_with(root) {
        return Err(NativeWorkspaceAuthorityError::OverlappingState);
    }
    Ok(())
}

fn reject_state_overlap(root: &Root, state: &StateRoot) -> Result<()> {
    reject_lexical_state(&root.identity, &state.identity)?;
    if descriptor_ancestor(&root.descriptor, &state.ancestor.descriptor)?
        || (state.exists && descriptor_ancestor(&state.ancestor.descriptor, &root.descriptor)?)
    {
        return Err(NativeWorkspaceAuthorityError::OverlappingState);
    }
    Ok(())
}

fn descriptor_ancestor(ancestor: &OwnedFd, descendant: &OwnedFd) -> Result<bool> {
    let ancestor =
        rustix::fs::fstat(ancestor).map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    let mut current = descendant
        .try_clone()
        .map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
    // Explicitly bounded even if a concurrently renamed ancestry never reaches its root.
    for _ in 0..MAX_PATH_BYTES {
        let metadata =
            rustix::fs::fstat(&current).map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
        if same_identity(&ancestor, &metadata) {
            return Ok(true);
        }
        let parent = rustix::fs::openat(&current, "..", DIRECTORY_FLAGS, Mode::empty())
            .map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
        let parent_metadata =
            rustix::fs::fstat(&parent).map_err(|_| NativeWorkspaceAuthorityError::Unavailable)?;
        if same_identity(&metadata, &parent_metadata) {
            return Ok(false);
        }
        current = parent;
    }
    Err(NativeWorkspaceAuthorityError::Unavailable)
}

fn same_identity(left: &Stat, right: &Stat) -> bool {
    left.st_dev == right.st_dev && left.st_ino == right.st_ino
}

#[cfg(test)]
mod tests;
