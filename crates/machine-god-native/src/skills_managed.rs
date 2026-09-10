//! Human-admitted skill management, separate from the model install tool.

#[path = "skills_managed/filesystem.rs"]
mod filesystem;
#[path = "skills_managed/git.rs"]
mod git;
#[path = "skills_managed/planning.rs"]
mod planning;
#[path = "skills_managed/publication.rs"]
mod publication;
#[path = "skills_managed/source.rs"]
mod source;

pub use git::{NativeSkillGitLease, NativeSkillGitRequest, NativeSkillGitRunner};
pub use source::{
    NativeSkillInstallSource, NativeSkillSourceKind, parse_skill_create_command,
    parse_skill_install_command,
};

use machine_god_core::CancellationToken;
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const MAX_MANAGED_SKILL_ITEMS: usize = 64;
pub const MAX_MANAGED_SKILL_ENTRIES: usize = 4096;
pub const MAX_MANAGED_SKILL_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_MANAGED_SKILL_TOTAL_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_MANAGED_SKILL_OPERATIONS: usize = 16_384;

/// Fixed errors never include source, filesystem, Git output, or credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillManagedErrorKind {
    InvalidSource,
    InvalidName,
    ConflictingFilter,
    InvalidMetadata,
    InvalidEntry,
    ResourceLimit,
    Unavailable,
    NoMatches,
    Collision,
    ReplacementConsentRequired,
    Changed,
    Busy,
    Cancelled,
    TimedOut,
    GitUnavailable,
    GitFailed,
    Unsupported,
    Indeterminate,
}

#[derive(Clone, Eq, PartialEq)]
pub struct NativeSkillManagedError {
    pub kind: NativeSkillManagedErrorKind,
    pub recovery_id: Option<String>,
}
impl NativeSkillManagedError {
    #[must_use]
    pub const fn new(kind: NativeSkillManagedErrorKind) -> Self {
        Self {
            kind,
            recovery_id: None,
        }
    }
    #[must_use]
    pub fn with_recovery(kind: NativeSkillManagedErrorKind, recovery_id: String) -> Self {
        Self {
            kind,
            recovery_id: Some(recovery_id),
        }
    }
}
impl From<NativeSkillManagedErrorKind> for NativeSkillManagedError {
    fn from(kind: NativeSkillManagedErrorKind) -> Self {
        Self::new(kind)
    }
}
impl fmt::Debug for NativeSkillManagedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSkillManagedError")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for NativeSkillManagedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("managed skill operation failed")
    }
}
impl std::error::Error for NativeSkillManagedError {}

/// The exact destination state observed by a plan, not a reusable overwrite grant.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeSkillDestinationRevision {
    pub destination: String,
    fingerprint: [u8; 32],
}
impl fmt::Debug for NativeSkillDestinationRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSkillDestinationRevision")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default)]
pub enum NativeSkillReplacementConsent {
    #[default]
    NoReplace,
    ExactDestinations(Vec<NativeSkillDestinationRevision>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillItemOutcome {
    Installed,
    Replaced,
    Removed,
    Failed,
    RolledBack,
    Indeterminate,
    NotAttempted,
}

/// A per-item publication receipt; recovery identifiers are private transaction basenames.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSkillItemReceipt {
    pub destination: String,
    pub outcome: NativeSkillItemOutcome,
    pub error: Option<NativeSkillManagedErrorKind>,
    pub recovery_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSkillBatchReceipt {
    pub items: Vec<NativeSkillItemReceipt>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSkillInstallItem {
    pub name: String,
    pub destination: String,
    pub replaces: bool,
    pub entries: usize,
    pub bytes: usize,
}

/// An admitted immutable content snapshot. Drop performs no filesystem work.
pub struct NativeSkillInstallPlan {
    authority: Arc<File>,
    namespace: Option<filesystem::Identity>,
    items: Vec<publication::PlannedItem>,
    source: Option<(File, filesystem::Tree)>,
}
impl fmt::Debug for NativeSkillInstallPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSkillInstallPlan")
            .field("items", &self.items.len())
            .finish_non_exhaustive()
    }
}
impl NativeSkillInstallPlan {
    #[must_use]
    pub fn items(&self) -> Vec<NativeSkillInstallItem> {
        self.items
            .iter()
            .map(publication::PlannedItem::summary)
            .collect()
    }

    #[must_use]
    pub fn replacements(&self) -> Vec<NativeSkillDestinationRevision> {
        self.items
            .iter()
            .filter_map(|item| item.expected.clone())
            .collect()
    }
}

/// Retains only the explicitly selected native state directory.
pub struct NativeManagedSkills {
    root: Arc<File>,
    root_path: PathBuf,
    git: Option<Arc<dyn NativeSkillGitRunner>>,
}
impl fmt::Debug for NativeManagedSkills {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeManagedSkills")
            .finish_non_exhaustive()
    }
}
impl NativeManagedSkills {
    /// Reports the configured native managed location without inspecting or creating it.
    #[must_use]
    pub fn managed_path(&self) -> PathBuf {
        self.root_path.join("skills")
    }

    /// Binds discovery to this exact retained state-root capability without I/O.
    /// # Errors
    /// Rejects labels which exceed the catalog's independent path bounds.
    pub fn catalog_root(
        &self,
    ) -> Result<
        crate::skills_catalog::NativeSkillRoot,
        crate::skills_catalog::NativeSkillCatalogError,
    > {
        crate::skills_catalog::NativeSkillRoot::from_directory(
            Arc::clone(&self.root),
            PathBuf::from("skills"),
            self.root_path.clone(),
            self.managed_path(),
            crate::skills_catalog::NativeSkillSource::Managed,
            crate::skills_catalog::NativeSkillLinkPolicy::Reject,
        )
    }

    pub(crate) fn owns_selection(
        &self,
        selection: &crate::skills_catalog::NativeSkillSelection,
    ) -> bool {
        selection.belongs_to_managed_directory(&self.root)
    }

    /// Opens the selected existing state root without creating the managed namespace.
    /// # Errors
    /// Rejects nonabsolute, unavailable, or non-directory roots.
    pub fn open(
        root: &Path,
        git: Option<Arc<dyn NativeSkillGitRunner>>,
    ) -> Result<Self, NativeSkillManagedError> {
        let descriptor = filesystem::open_absolute_directory(root)?;
        Ok(Self {
            root: Arc::new(descriptor),
            root_path: root.to_owned(),
            git,
        })
    }

    /// Reads and snapshots a classified source; call only inside an admitted owned worker.
    /// # Errors
    /// Rejects invalid, incomplete, ambiguous, changed, cancelled, or oversized sources.
    pub fn prepare_install(
        &self,
        source: &NativeSkillInstallSource,
        cwd: &Path,
        cancellation: &CancellationToken,
    ) -> Result<NativeSkillInstallPlan, NativeSkillManagedError> {
        publication::prepare_install(self, source, cwd, cancellation)
    }

    /// Builds the native template, preserving existing sibling resources on replacement.
    /// # Errors
    /// Rejects invalid names or unreadable, changed, or oversized existing destinations.
    pub fn prepare_create(
        &self,
        name: &str,
        cancellation: &CancellationToken,
    ) -> Result<NativeSkillInstallPlan, NativeSkillManagedError> {
        publication::prepare_create(self, name, cancellation).map_err(Into::into)
    }

    /// Captures one exact managed destination for removal; never searches compatibility roots.
    /// # Errors
    /// Rejects missing, malformed, or oversized destinations.
    pub fn prepare_remove(
        &self,
        name: &str,
        cancellation: &CancellationToken,
    ) -> Result<NativeSkillInstallPlan, NativeSkillManagedError> {
        publication::prepare_remove(self, name, cancellation).map_err(Into::into)
    }

    /// Publishes a plan with exact replacement consent and per-item recovery evidence.
    /// # Errors
    /// Rejects a foreign plan or invalid consent before any publication.
    pub fn commit(
        &self,
        plan: NativeSkillInstallPlan,
        consent: &NativeSkillReplacementConsent,
        cancellation: &CancellationToken,
    ) -> Result<NativeSkillBatchReceipt, NativeSkillManagedError> {
        publication::commit(self, plan, consent, cancellation)
    }
}

#[cfg(test)]
#[path = "skills_managed/tests.rs"]
mod tests;
