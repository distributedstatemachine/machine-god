//! Explicit startup selection, on one admitted worker and without discovery.

mod selection;
#[cfg(test)]
mod tests;

use crate::skills_managed::{NativeManagedSkills, NativeSkillGitRunner};
use crate::skills_roots::{
    NativeSkillDirectoryAuthority, NativeSkillRootsError, compose_native_skill_catalog,
};
use crate::skills_service::NativeSkillsService;
use crate::{
    NativeEnvironment, NativeOwnedWorkerScope, NativeReferenceHostTerminalOptions,
    PreparedNativeRoots,
};
use futures_util::future::{Either, select};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{fmt, fs::File, sync::Arc};

/// Startup selection has its own bounds; root expansion retains its independent
/// 128-call and 20-workspace-level bounds. No individual kernel call is preempted.
pub const MAX_NATIVE_SKILLS_STARTUP_IO_ATTEMPTS: usize = 128;
pub const MAX_NATIVE_SKILLS_STARTUP_PATH_BYTES: usize = 16 * 1024;
pub const MAX_NATIVE_SKILLS_STARTUP_PATH_ENTRIES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillsStartupError {
    Cancelled,
    Admission,
    InvalidEnvironment,
    InvalidHome,
    InvalidPath,
    ResourceLimit,
    Unavailable,
    Roots(NativeSkillRootsError),
}
impl fmt::Display for NativeSkillsStartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native skills startup failed")
    }
}
impl std::error::Error for NativeSkillsStartupError {}

impl From<NativeSkillRootsError> for NativeSkillsStartupError {
    fn from(error: NativeSkillRootsError) -> Self {
        if error == NativeSkillRootsError::Cancelled {
            Self::Cancelled
        } else {
            Self::Roots(error)
        }
    }
}

type StartupResult =
    Result<(PreparedNativeRoots, Arc<NativeSkillsService>), NativeSkillsStartupError>;

/// Returns an inert future consuming explicit selections, never ambient state.
/// First poll admits one worker. Dropping a polled future cancels its private
/// operation token, never the caller's token; scope completion retains the
/// actual worker through native returns and cleanup. Caller cancellation wakes
/// the future and cancels that same private operation without awaiting kernel I/O.
///
/// Returns the original prepared roots for later reference-host composition.
/// No skill discovery, managed namespace creation, process or network occurs.
/// Missing HOME means no home roots; invalid supplied HOME never becomes absence.
/// Missing Git leaves local management usable and remote operations unavailable.
///
/// # Errors
/// Returns fixed validation, admission, cancellation or root-composition errors.
/// Paths, environment contents and operating-system diagnostics are not exposed.
#[must_use]
pub fn prepare_native_skills(
    roots: PreparedNativeRoots,
    environment: NativeEnvironment,
    terminal: NativeReferenceHostTerminalOptions,
    scope: NativeOwnedWorkerScope,
    cancellation: CancellationToken,
) -> BoxFuture<'static, StartupResult> {
    on_worker(scope, cancellation, move |stop| {
        compose(roots, &environment, &terminal, &stop)
    })
}

fn on_worker<T: Send + 'static>(
    scope: NativeOwnedWorkerScope,
    cancellation: CancellationToken,
    operation: impl FnOnce(CancellationToken) -> Result<T, NativeSkillsStartupError> + Send + 'static,
) -> BoxFuture<'static, Result<T, NativeSkillsStartupError>> {
    Box::pin(async move {
        if cancellation.is_cancelled() {
            return Err(NativeSkillsStartupError::Cancelled);
        }
        let stop = CancellationToken::new();
        let _stop_on_drop = StopOnDrop(stop.clone());
        let response = scope.run(move || operation(stop));
        let result = match select(response, cancellation.cancelled()).await {
            Either::Left((result, _)) => result.map_err(|_| NativeSkillsStartupError::Admission)?,
            Either::Right(_) => return Err(NativeSkillsStartupError::Cancelled),
        };
        if cancellation.is_cancelled() {
            return Err(NativeSkillsStartupError::Cancelled);
        }
        result
    })
}

struct StopOnDrop(CancellationToken);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn compose(
    roots: PreparedNativeRoots,
    environment: &NativeEnvironment,
    terminal: &NativeReferenceHostTerminalOptions,
    cancellation: &CancellationToken,
) -> StartupResult {
    let mut budget = selection::Budget::new(cancellation);
    selection::validate_environment(environment, terminal)?;
    let workspace = budget.call(|| roots.try_clone_skills_workspace())?;
    let state = budget.call(|| roots.try_clone_skills_state())?;
    let workspace = NativeSkillDirectoryAuthority::from_directory(
        Arc::new(File::from(workspace)),
        roots.canonical_workspace_root().to_owned(),
    )?;
    let home = selection::home(environment, &mut budget)?;
    let git: Option<Arc<dyn NativeSkillGitRunner>> = selection::git(terminal, &mut budget)?
        .map(|runner| Arc::new(runner) as Arc<dyn NativeSkillGitRunner>);
    let managed = Arc::new(NativeManagedSkills::from_retained_root(
        Arc::new(File::from(state)),
        roots.state_root().to_owned(),
        git,
    ));
    let managed_root = managed
        .catalog_root()
        .map_err(NativeSkillRootsError::Catalog)?;
    let catalog = compose_native_skill_catalog(
        Some(&workspace),
        home.as_ref(),
        Some(managed_root),
        cancellation,
    )?;
    budget.check()?;
    Ok((
        roots,
        Arc::new(NativeSkillsService::new(Arc::new(catalog), Some(managed))),
    ))
}
