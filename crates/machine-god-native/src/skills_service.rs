//! Synchronous human-invoked skills commands under explicit native authority.
//!
//! The host owns admission, worker completion, cancellation, and presentation.
//! Construction is inert. Call `execute` only from an admitted owned worker.
//! Mutation receipts are never replaced by a subsequent catalog-refresh error.

use crate::skills_catalog::{
    NativeSkillCatalog, NativeSkillCatalogError, NativeSkillSelection, NativeSkillSnapshot,
};
use crate::skills_commands::{
    MAX_NATIVE_SKILLS_COMMAND_BYTES, MAX_NATIVE_SKILLS_SELECTOR_BYTES, NativeSkillsCommand,
};
use crate::skills_managed::{
    NativeManagedSkills, NativeSkillBatchReceipt, NativeSkillInstallPlan, NativeSkillInstallSource,
    NativeSkillItemOutcome, NativeSkillManagedError, NativeSkillReplacementConsent,
    parse_skill_create_command, parse_skill_install_command,
};
use machine_god_core::CancellationToken;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

#[path = "skills_service/selection.rs"]
mod selection;
#[cfg(test)]
#[path = "skills_service/tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillsNotice {
    NotFound,
    Ambiguous,
    IncompleteDiscovery,
}

/// Menu data, never an eagerly materialized skill body.
#[derive(Clone)]
pub struct NativeSkillsCatalogView {
    pub snapshot: NativeSkillSnapshot,
    pub query: String,
    pub focus: Option<NativeSkillSelection>,
    pub notice: Option<NativeSkillsNotice>,
}
impl fmt::Debug for NativeSkillsCatalogView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillsCatalogView")
            .field("notice", &self.notice)
            .field("focused", &self.focus.is_some())
            .finish_non_exhaustive()
    }
}

pub enum NativeSkillsServiceResult {
    Catalog(Box<NativeSkillsCatalogView>),
    Path(PathBuf),
    Managed(NativeSkillBatchReceipt),
}
impl NativeSkillsServiceResult {
    /// Any partial, rolled-back, unattempted or uncertain mutation is not success.
    #[must_use]
    pub fn failed(&self) -> bool {
        match self {
            Self::Managed(receipt) => {
                receipt.items.is_empty()
                    || receipt.items.iter().any(|item| {
                        !matches!(
                            item.outcome,
                            NativeSkillItemOutcome::Installed
                                | NativeSkillItemOutcome::Replaced
                                | NativeSkillItemOutcome::Removed
                        ) || item.error.is_some()
                            || item.recovery_id.is_some()
                    })
            }
            Self::Catalog(view) => matches!(view.notice, Some(NativeSkillsNotice::NotFound)),
            Self::Path(_) => false,
        }
    }
}
impl fmt::Debug for NativeSkillsServiceResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Catalog(_) => "NativeSkillsServiceResult::Catalog(..)",
            Self::Path(_) => "NativeSkillsServiceResult::Path(..)",
            Self::Managed(_) => "NativeSkillsServiceResult::Managed(..)",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeSkillsServiceError {
    InvalidCommand,
    MissingManagedAuthority,
    Cancelled,
    Catalog(NativeSkillCatalogError),
    Managed(NativeSkillManagedError),
}
impl fmt::Display for NativeSkillsServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native skills command failed")
    }
}
impl std::error::Error for NativeSkillsServiceError {}
impl From<NativeSkillCatalogError> for NativeSkillsServiceError {
    fn from(error: NativeSkillCatalogError) -> Self {
        if error == NativeSkillCatalogError::Cancelled {
            Self::Cancelled
        } else {
            Self::Catalog(error)
        }
    }
}
impl From<NativeSkillManagedError> for NativeSkillsServiceError {
    fn from(error: NativeSkillManagedError) -> Self {
        Self::Managed(error)
    }
}

/// Explicit catalog and optional managed-write authority; no ambient root lookup.
#[derive(Clone, Debug)]
pub struct NativeSkillsService {
    catalog: Arc<NativeSkillCatalog>,
    managed: Option<Arc<NativeManagedSkills>>,
}
impl NativeSkillsService {
    /// Borrows the exact retained discovery authority without observations.
    #[must_use]
    pub fn catalog(&self) -> &Arc<NativeSkillCatalog> {
        &self.catalog
    }

    #[must_use]
    pub const fn new(
        catalog: Arc<NativeSkillCatalog>,
        managed: Option<Arc<NativeManagedSkills>>,
    ) -> Self {
        Self { catalog, managed }
    }

    /// Validates even directly constructed command variants before effects.
    /// Does not spawn work, materialize show bodies, or refresh after mutation.
    /// # Errors
    /// Returns fixed validation/authority/catalog errors or preserves a managed
    /// preparation error, including its opaque recovery identifier.
    pub fn execute(
        &self,
        command: NativeSkillsCommand,
        cwd: &Path,
        cancellation: &CancellationToken,
    ) -> Result<NativeSkillsServiceResult, NativeSkillsServiceError> {
        let prepared = PreparedCommand::new(command, cwd)?;
        if cancellation.is_cancelled() {
            return Err(NativeSkillsServiceError::Cancelled);
        }
        match prepared {
            PreparedCommand::List => Ok(NativeSkillsServiceResult::Catalog(Box::new(
                NativeSkillsCatalogView {
                    snapshot: self.catalog.discover(cancellation)?,
                    query: String::new(),
                    focus: None,
                    notice: None,
                },
            ))),
            PreparedCommand::Show(selector) => Ok(NativeSkillsServiceResult::Catalog(Box::new(
                selection::show(self.catalog.discover(cancellation)?, &selector),
            ))),
            PreparedCommand::Path => Ok(NativeSkillsServiceResult::Path(
                self.managed()?.managed_path(),
            )),
            PreparedCommand::Create(name, replace) => {
                let managed = self.managed()?;
                let plan = managed.prepare_create(&name, cancellation)?;
                Self::commit(managed, plan, replace, cancellation)
            }
            PreparedCommand::Install(source, replace) => {
                let managed = self.managed()?;
                let plan = managed.prepare_install(&source, cwd, cancellation)?;
                Self::commit(managed, plan, replace, cancellation)
            }
            PreparedCommand::Remove(selector) => self.remove(&selector, cancellation),
        }
    }

    fn managed(&self) -> Result<&NativeManagedSkills, NativeSkillsServiceError> {
        self.managed
            .as_deref()
            .ok_or(NativeSkillsServiceError::MissingManagedAuthority)
    }
    fn commit(
        managed: &NativeManagedSkills,
        plan: NativeSkillInstallPlan,
        replace: bool,
        cancellation: &CancellationToken,
    ) -> Result<NativeSkillsServiceResult, NativeSkillsServiceError> {
        let consent = if replace {
            NativeSkillReplacementConsent::ExactDestinations(plan.replacements())
        } else {
            NativeSkillReplacementConsent::NoReplace
        };
        Ok(NativeSkillsServiceResult::Managed(managed.commit(
            plan,
            &consent,
            cancellation,
        )?))
    }
    fn remove(
        &self,
        selector: &str,
        cancellation: &CancellationToken,
    ) -> Result<NativeSkillsServiceResult, NativeSkillsServiceError> {
        let managed = self.managed()?;
        let snapshot = self.catalog.discover(cancellation)?;
        let selected = selection::managed(&snapshot, selector, &managed.managed_path())?;
        if !managed.owns_selection(&selected) {
            return Err(NativeSkillsServiceError::Catalog(
                NativeSkillCatalogError::WrongAuthority,
            ));
        }
        let name = selected
            .location()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(NativeSkillsServiceError::InvalidCommand)?;
        #[cfg(test)]
        selection::before_remove_preparation();
        let plan = managed.prepare_remove(name, cancellation)?;
        #[cfg(test)]
        selection::before_remove_revalidation();
        // The original catalog revision must still name the selected skill after
        // preparation. The managed plan checks later destination-content changes.
        drop(self.catalog.materialize(&selected, cancellation)?);
        Self::commit(managed, plan, false, cancellation)
    }
}

enum PreparedCommand {
    List,
    Show(String),
    Path,
    Create(String, bool),
    Install(NativeSkillInstallSource, bool),
    Remove(String),
}
impl PreparedCommand {
    fn new(command: NativeSkillsCommand, cwd: &Path) -> Result<Self, NativeSkillsServiceError> {
        match command {
            NativeSkillsCommand::List => Ok(Self::List),
            NativeSkillsCommand::Path => Ok(Self::Path),
            NativeSkillsCommand::Show { selector } => {
                validate_selector(&selector)?;
                Ok(Self::Show(selector))
            }
            NativeSkillsCommand::Remove { selector } => {
                validate_selector(&selector)?;
                Ok(Self::Remove(selector))
            }
            NativeSkillsCommand::Create { arguments } => {
                validate_arguments(&arguments)?;
                let (name, replace) = parse_skill_create_command(&arguments)
                    .map_err(|kind| NativeSkillsServiceError::Managed(kind.into()))?;
                Ok(Self::Create(name, replace))
            }
            NativeSkillsCommand::Install { arguments } => {
                validate_arguments(&arguments)?;
                let cwd_text = cwd
                    .to_str()
                    .ok_or(NativeSkillsServiceError::InvalidCommand)?;
                if !cwd.is_absolute()
                    || cwd_text.len() > MAX_NATIVE_SKILLS_SELECTOR_BYTES
                    || cwd_text.chars().any(char::is_control)
                {
                    return Err(NativeSkillsServiceError::InvalidCommand);
                }
                let (source, replace) = parse_skill_install_command(&arguments)
                    .map_err(|kind| NativeSkillsServiceError::Managed(kind.into()))?;
                Ok(Self::Install(source, replace))
            }
        }
    }
}
fn validate_arguments(arguments: &str) -> Result<(), NativeSkillsServiceError> {
    if arguments.is_empty()
        || arguments.len() > MAX_NATIVE_SKILLS_COMMAND_BYTES
        || arguments.chars().any(|c| c.is_control() && c != '\t')
    {
        return Err(NativeSkillsServiceError::InvalidCommand);
    }
    Ok(())
}
fn validate_selector(selector: &str) -> Result<(), NativeSkillsServiceError> {
    if selector.is_empty()
        || selector.len() > MAX_NATIVE_SKILLS_SELECTOR_BYTES
        || selector.chars().any(char::is_control)
        || selector.trim_matches([' ', '\t']) != selector
    {
        return Err(NativeSkillsServiceError::InvalidCommand);
    }
    if !Path::new(selector).is_absolute()
        && (selector.len() > 256
            || selector.contains(['/', '\\'])
            || matches!(selector, "." | ".."))
    {
        return Err(NativeSkillsServiceError::InvalidCommand);
    }
    if Path::new(selector)
        .components()
        .any(|part| part == std::path::Component::ParentDir)
    {
        return Err(NativeSkillsServiceError::InvalidCommand);
    }
    Ok(())
}
