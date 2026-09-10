//! Explicit, bounded, read-only authority for the human-invoked skills catalog.
//!
//! Constructors validate values only. Discovery and materialization are
//! synchronous effects intended for an admitted, coordinator-owned worker.
//! Cancellation brackets native calls but cannot preempt a blocking kernel call.

use std::{fmt, fs::File, path::PathBuf, sync::Arc};

use machine_god_core::CancellationToken;

use crate::skills_metadata::{NativeSkillMetadata, NativeSkillMetadataError};

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "skills_catalog/discovery.rs"]
mod discovery;
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "skills_catalog/io.rs"]
mod io;
#[path = "skills_catalog/snapshot.rs"]
mod snapshot;
#[cfg(test)]
#[path = "skills_catalog/tests.rs"]
mod tests;

pub const MAX_NATIVE_SKILL_ROOTS: usize = 128;
pub const MAX_NATIVE_SKILL_VISITED_ENTRIES: usize = 16_384;
pub const MAX_NATIVE_SKILL_CANDIDATES: usize = 1_024;
pub const MAX_NATIVE_SKILL_DIAGNOSTICS: usize = 256;
pub const MAX_NATIVE_SKILL_DISCOVERY_BYTES: usize = 8 * 1_024 * 1_024;
pub const MAX_NATIVE_SKILL_SNAPSHOT_BYTES: usize = 4 * 1_024 * 1_024;
/// Independently bounded names retained while sorting one directory listing.
pub const MAX_NATIVE_SKILL_DIRECTORY_BYTES: usize = 4 * 1_024 * 1_024;
pub const MAX_NATIVE_SKILL_IO_ATTEMPTS: usize = 65_536;
pub const MAX_NATIVE_SKILL_PATH_BYTES: usize = 4_096;
pub const MAX_NATIVE_SKILL_PATH_COMPONENTS: usize = 32;
pub const MAX_NATIVE_SKILL_LINK_HOPS: usize = 32;
pub const MAX_NATIVE_SKILL_MATERIALIZED_BYTES: usize = 1_024 * 1_024;
pub const MAX_NATIVE_SKILL_QUERY_BYTES: usize = 1_024;
pub const MAX_NATIVE_SKILL_QUERY_ROWS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillSource {
    WorkspaceShared,
    WorkspaceOpencode,
    WorkspaceCodex,
    WorkspaceClaude,
    WorkspaceAgents,
    WorkspaceClaw,
    Managed,
    GlobalFx,
    GlobalOpencode,
    GlobalCodex,
    GlobalClaude,
    GlobalAgents,
    GlobalClaw,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillLinkPolicy {
    Reject,
    /// Links may resolve only beneath the explicit retained base directory.
    Contained,
}

/// A retained base and one relative discovery root, with no ambient lookup.
#[derive(Clone)]
pub struct NativeSkillRoot {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    directory: Arc<File>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    relative: PathBuf,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    authority_path: PathBuf,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    display_path: PathBuf,
    source: NativeSkillSource,
    links: NativeSkillLinkPolicy,
}

impl NativeSkillRoot {
    /// Validates spelling without inspecting the descriptor or filesystem.
    /// Display spelling must equal the captured authority identity plus relative
    /// components. Absolute compatibility links are mapped beneath that captured
    /// identity, never reopened through the host filesystem namespace.
    ///
    /// # Errors
    /// Returns `InvalidRoot` for invalid paths, inconsistent spelling, or a
    /// managed root which permits links; unsupported platforms fail inertly.
    pub fn from_directory(
        directory: Arc<File>,
        relative: PathBuf,
        authority_path: PathBuf,
        display_path: PathBuf,
        source: NativeSkillSource,
        links: NativeSkillLinkPolicy,
    ) -> Result<Self> {
        supported()?;
        validate_path(&relative, false)?;
        validate_path(&authority_path, true)?;
        validate_path(&display_path, true)?;
        if authority_path.join(&relative) != display_path
            || (source == NativeSkillSource::Managed && links != NativeSkillLinkPolicy::Reject)
        {
            return Err(NativeSkillCatalogError::InvalidRoot);
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            Ok(Self {
                directory,
                relative,
                authority_path,
                display_path,
                source,
                links,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            drop((directory, relative, authority_path, display_path));
            Err(NativeSkillCatalogError::UnsupportedPlatform)
        }
    }
}

impl fmt::Debug for NativeSkillRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillRoot")
            .field("source", &self.source)
            .field("links", &self.links)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillCatalogError {
    UnsupportedPlatform,
    InvalidRoot,
    InvalidQuery,
    Cancelled,
    ResourceLimit,
    Unavailable,
    PathRejected,
    InvalidUtf8,
    InvalidMetadata(NativeSkillMetadataError),
    NotFound,
    AmbiguousName,
    IncompleteDiscovery,
    NameLocationMismatch,
    WrongAuthority,
    StaleSelection,
}

impl fmt::Display for NativeSkillCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "skill catalog: {self:?}")
    }
}
impl std::error::Error for NativeSkillCatalogError {}
type Result<T> = std::result::Result<T, NativeSkillCatalogError>;

#[derive(Clone, Debug)]
pub struct NativeSkillCatalog {
    roots: Arc<[Arc<NativeSkillRoot>]>,
}

impl NativeSkillCatalog {
    /// Captures explicit, ordered roots without performing I/O or spawning work.
    ///
    /// # Errors
    /// Returns `ResourceLimit` for too many roots or unsupported-platform error.
    pub fn new(roots: Vec<NativeSkillRoot>) -> Result<Self> {
        supported()?;
        if roots.len() > MAX_NATIVE_SKILL_ROOTS {
            return Err(NativeSkillCatalogError::ResourceLimit);
        }
        Ok(Self {
            roots: roots.into_iter().map(Arc::new).collect(),
        })
    }

    /// Scans one directory level beneath each explicit root in order.
    /// Missing roots are empty; malformed, inaccessible or bounded-away roots
    /// and candidates produce diagnostics and an incomplete snapshot.
    ///
    /// # Errors
    /// Cancellation discards observations; unsupported platforms perform no I/O.
    pub fn discover(&self, cancellation: &CancellationToken) -> Result<NativeSkillSnapshot> {
        check(cancellation)?;
        supported()?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            discovery::discover(self, cancellation)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err(NativeSkillCatalogError::UnsupportedPlatform)
        }
    }

    /// Reopens only the exact selected location through retained authority.
    /// Revision means stat identity/timestamps plus the observed metadata-prefix
    /// digest, not a prior full-body digest or an atomic filesystem snapshot.
    ///
    /// # Errors
    /// Rejects cancellation, foreign/stale selection, invalid content, native
    /// failures and independently bounded path/read/I/O exhaustion.
    pub fn materialize(
        &self,
        selection: &NativeSkillSelection,
        cancellation: &CancellationToken,
    ) -> Result<NativeSkillMaterialized> {
        check(cancellation)?;
        supported()?;
        if !self
            .roots
            .iter()
            .any(|root| Arc::ptr_eq(root, &selection.root))
        {
            return Err(NativeSkillCatalogError::WrongAuthority);
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            discovery::materialize(selection, cancellation)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err(NativeSkillCatalogError::UnsupportedPlatform)
        }
    }
}

/// Opaque exact location/revision binding retaining its original root authority.
#[derive(Clone)]
pub struct NativeSkillSelection {
    root: Arc<NativeSkillRoot>,
    relative: PathBuf,
    location: PathBuf,
    metadata: NativeSkillMetadata,
    directory_identity: [i128; 2],
    revision: [i128; 8],
    prefix_digest: [u8; 32],
    prefix_bytes: usize,
}

impl NativeSkillSelection {
    #[must_use]
    pub fn location(&self) -> &std::path::Path {
        &self.location
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.metadata.name
    }
    #[must_use]
    pub fn source(&self) -> NativeSkillSource {
        self.root.source
    }
    /// Owned variable-sized data retained by a queued clone of this selection.
    /// The shared root descriptor/spec is not duplicated by cloning.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.relative.as_os_str().len()
            + self.location.as_os_str().len()
            + self.metadata.name.len()
            + self.metadata.description.len()
    }
}

impl PartialEq for NativeSkillSelection {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.root, &other.root)
            && self.relative == other.relative
            && self.directory_identity == other.directory_identity
            && self.revision == other.revision
            && self.prefix_digest == other.prefix_digest
            && self.prefix_bytes == other.prefix_bytes
            && self.metadata == other.metadata
    }
}
impl Eq for NativeSkillSelection {}

impl fmt::Debug for NativeSkillSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillSelection")
            .field("source", &self.source())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct NativeSkillEntry {
    pub metadata: NativeSkillMetadata,
    selection: NativeSkillSelection,
}
impl NativeSkillEntry {
    /// Borrows the exact binding for comparison and admission without copying.
    #[must_use]
    pub fn selection_ref(&self) -> &NativeSkillSelection {
        &self.selection
    }
    #[must_use]
    pub fn selection(&self) -> NativeSkillSelection {
        self.selection.clone()
    }
    #[must_use]
    pub fn location(&self) -> &std::path::Path {
        self.selection.location()
    }
    #[must_use]
    pub fn source(&self) -> NativeSkillSource {
        self.selection.source()
    }
}

#[derive(Clone)]
pub struct NativeSkillDiagnostic {
    pub location: PathBuf,
    pub source: NativeSkillSource,
    pub root: bool,
    pub cause: NativeSkillCatalogError,
}
impl fmt::Debug for NativeSkillDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillDiagnostic")
            .field("source", &self.source)
            .field("root", &self.root)
            .field("cause", &self.cause)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct NativeSkillSnapshot {
    entries: Vec<NativeSkillEntry>,
    diagnostics: Vec<NativeSkillDiagnostic>,
    complete: bool,
    generation: [u8; 32],
}

#[derive(Clone)]
pub struct NativeSkillMaterialized {
    pub text: String,
    pub metadata: NativeSkillMetadata,
    pub selection: NativeSkillSelection,
    /// Digest of the admitted complete bytes, computed only during this read.
    pub digest: [u8; 32],
}
impl fmt::Debug for NativeSkillMaterialized {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillMaterialized")
            .field("bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

fn supported() -> Result<()> {
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        Ok(())
    } else {
        Err(NativeSkillCatalogError::UnsupportedPlatform)
    }
}

fn check(cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(NativeSkillCatalogError::Cancelled)
    } else {
        Ok(())
    }
}

fn validate_path(path: &std::path::Path, absolute: bool) -> Result<()> {
    use std::path::Component;
    let text = path.to_str().ok_or(NativeSkillCatalogError::InvalidRoot)?;
    if text.len() > MAX_NATIVE_SKILL_PATH_BYTES
        || path.is_absolute() != absolute
        || path.components().count() > MAX_NATIVE_SKILL_PATH_COMPONENTS
        || text.contains(['\0', '\\'])
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(NativeSkillCatalogError::InvalidRoot);
    }
    Ok(())
}
