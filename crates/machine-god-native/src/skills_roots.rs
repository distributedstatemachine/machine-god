//! Explicit, read-only composition of the pinned discovery root order.
//!
//! The host captures directory capabilities and their absolute authority labels
//! once. This module never reads the environment/current directory, reopens an
//! absolute label, creates missing skill roots, or manufactures managed-write
//! authority. Composition is synchronous admitted-worker work, not a detached
//! task; cooperative cancellation cannot preempt one blocking kernel call.

use crate::skills_catalog::{
    NativeSkillCatalog, NativeSkillCatalogError, NativeSkillLinkPolicy, NativeSkillRoot,
    NativeSkillSource,
};
use machine_god_core::CancellationToken;
use std::{
    fmt,
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "skills_roots/compose.rs"]
mod compose;
#[cfg(test)]
#[path = "skills_roots/tests.rs"]
mod tests;

/// At six roots per workspace level, managed plus six home roots fit the
/// catalog's 128-root limit. Deep ancestry fails rather than truncating it.
pub const MAX_NATIVE_SKILL_WORKSPACE_LEVELS: usize = 20;
/// Charges each attempted stat/open, with cancellation before and after it.
pub const MAX_NATIVE_SKILL_ROOT_IO_ATTEMPTS: usize = 128;

type Result<T> = std::result::Result<T, NativeSkillRootsError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillRootsError {
    UnsupportedPlatform,
    InvalidAuthority,
    Cancelled,
    ResourceLimit,
    Unavailable,
    ChangedAncestry,
    Catalog(NativeSkillCatalogError),
}

impl fmt::Display for NativeSkillRootsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "skill roots: {self:?}")
    }
}
impl std::error::Error for NativeSkillRootsError {}

/// Explicit retained read authority; the path is a captured identity/display
/// label, never authority to reopen it. Ancestor composition requires labels
/// consistent with descriptor ancestry (normally a captured canonical path).
#[derive(Clone)]
pub struct NativeSkillDirectoryAuthority {
    directory: Arc<File>,
    path: PathBuf,
}

impl fmt::Debug for NativeSkillDirectoryAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeSkillDirectoryAuthority(<retained>)")
    }
}

impl NativeSkillDirectoryAuthority {
    /// Validates catalog-compatible UTF-8/absolute/no-parent labels without I/O.
    /// Descriptor directory/type inspection occurs only during composition.
    /// # Errors
    /// Rejects unsupported platforms and invalid/bounded-out catalog labels.
    pub fn from_directory(directory: Arc<File>, captured_absolute_path: PathBuf) -> Result<Self> {
        supported()?;
        let path: PathBuf = captured_absolute_path.components().collect();
        // Reuse the catalog's lexical policy, including its independent byte
        // and component limits; do not duplicate or widen its path grammar.
        NativeSkillRoot::from_directory(
            directory.clone(),
            PathBuf::new(),
            path.clone(),
            captured_absolute_path,
            NativeSkillSource::WorkspaceShared,
            NativeSkillLinkPolicy::Contained,
        )
        .map_err(|_| NativeSkillRootsError::InvalidAuthority)?;
        Ok(Self { directory, path })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Composes nearest workspace/ancestor roots, then native managed, then home
/// compatibility roots. Workspace levels stop before the explicit home by
/// identity or label, otherwise include filesystem root. Every compatibility
/// base is retained, and its contained-link scope is that base, not its child.
///
/// Supply `NativeManagedSkills::catalog_root()` unchanged: only that manager's
/// retained directory identity establishes its write ownership. This function
/// validates Managed/Reject provenance, never infers ownership from labels.
/// Missing ordinary skill children are not inspected or created here.
///
/// # Errors
/// Rejects invalid managed provenance before effects, unavailable/non-directory
/// supplied bases, changed ancestry, more than 20 workspace levels, 128 native
/// attempts, cancellation and catalog path/root bounds. Failure returns no
/// partial catalog. Unsupported targets fail without filesystem effects.
pub fn compose_native_skill_catalog(
    workspace: Option<&NativeSkillDirectoryAuthority>,
    home: Option<&NativeSkillDirectoryAuthority>,
    managed: Option<NativeSkillRoot>,
    cancellation: &CancellationToken,
) -> Result<NativeSkillCatalog> {
    check(cancellation)?;
    supported()?;
    let managed = managed
        .map(|root| {
            if root.is_managed_root() {
                Ok(root)
            } else {
                Err(NativeSkillRootsError::InvalidAuthority)
            }
        })
        .transpose()?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        compose::compose(workspace, home, managed, cancellation)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // Fields remain explicit inert inputs on unsupported platforms.
        let _ = workspace.or(home).map(|authority| &authority.directory);
        let _ = managed;
        Err(NativeSkillRootsError::UnsupportedPlatform)
    }
}

fn supported() -> Result<()> {
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        Ok(())
    } else {
        Err(NativeSkillRootsError::UnsupportedPlatform)
    }
}

fn check(cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(NativeSkillRootsError::Cancelled)
    } else {
        Ok(())
    }
}
