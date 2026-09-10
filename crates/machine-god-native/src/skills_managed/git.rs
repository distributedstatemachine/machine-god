use super::{
    NativeManagedSkills, NativeSkillManagedError, NativeSkillManagedErrorKind as Kind, filesystem,
};
use crate::{NativeOwnedWorkerCleanup, NativeOwnedWorkerScope};
use machine_god_core::CancellationToken;
use std::fmt;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Explicit Git execution authority. Implementations own the launched original
/// group and positively captured members. Unresolved ownership retains the
/// request lease through cleanup and exact direct-child reap.
pub trait NativeSkillGitRunner: Send + Sync + fmt::Debug {
    /// Clones into the already created directory. No shell/package-manager use.
    /// # Errors
    /// Failure must retain any unresolved process and request-lease ownership.
    fn clone_repository(
        &self,
        request: NativeSkillGitRequest,
        cancellation: &CancellationToken,
    ) -> Result<(), NativeSkillManagedError>;
}

pub struct NativeSkillGitRequest {
    pub url: String,
    pub directory: Arc<File>,
    pub directory_path: PathBuf,
    pub deadline: Instant,
    pub max_output_bytes: usize,
    /// A rejection/watchdog threshold, not a hard disk quota between checks.
    pub clone_rejection_bytes: u64,
    pub lease: NativeSkillGitLease,
}
impl fmt::Debug for NativeSkillGitRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillGitRequest")
            .finish_non_exhaustive()
    }
}

/// Keeps a private clone transaction and its original worker-cleanup attribution.
/// Last-drop cleanup is identity checked and operation bounded. A failed cleanup
/// leaves its random directory intact; it never follows replacement paths.
#[derive(Clone)]
pub struct NativeSkillGitLease(Arc<LeaseInner>);
struct LeaseInner {
    parent: Arc<File>,
    directory: Arc<File>,
    name: String,
    cleanup: Option<NativeOwnedWorkerCleanup>,
    cleaned: std::sync::atomic::AtomicBool,
}
impl fmt::Debug for NativeSkillGitLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillGitLease")
            .finish_non_exhaustive()
    }
}
impl NativeSkillGitLease {
    #[must_use]
    pub fn recovery_id(&self) -> &str {
        &self.0.name
    }
    pub(super) fn cleanup(&self) -> Result<(), NativeSkillManagedError> {
        if Arc::strong_count(&self.0) != 1 {
            return Err(NativeSkillManagedError::with_recovery(
                Kind::Indeterminate,
                self.0.name.clone(),
            ));
        }
        // An explicit failed cleanup returns its recovery ID and preserves the
        // residue; Drop must not silently retry and destroy recovery evidence.
        self.0
            .cleaned
            .store(true, std::sync::atomic::Ordering::Release);
        filesystem::cleanup(&self.0.parent, &self.0.name, &self.0.directory)
            .map_err(|kind| NativeSkillManagedError::with_recovery(kind, self.0.name.clone()))?;
        Ok(())
    }
}
impl Drop for LeaseInner {
    fn drop(&mut self) {
        if self.cleaned.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let cleanup = || {
            let _ = filesystem::cleanup(&self.parent, &self.name, &self.directory);
        };
        if let Some(ticket) = &self.cleanup {
            ticket.run_on_cleanup_worker(cleanup);
        } else {
            cleanup();
        }
    }
}

pub(super) fn clone_source(
    owner: &NativeManagedSkills,
    url: &str,
    cancellation: &CancellationToken,
) -> Result<(Arc<File>, NativeSkillGitLease), NativeSkillManagedError> {
    if cancellation.is_cancelled() {
        return Err(Kind::Cancelled.into());
    }
    let runner = owner.git.as_ref().ok_or(Kind::GitUnavailable)?;
    // The path is reporting/exec configuration only; storage remains descriptor relative.
    let named = filesystem::open_absolute_directory(&owner.root_path)?;
    if filesystem::identity(&named)? != filesystem::identity(&owner.root)? {
        return Err(Kind::Changed.into());
    }
    let name = filesystem::random_name()?;
    let directory = Arc::new(filesystem::create_directory(&owner.root, &name)?);
    let lease = NativeSkillGitLease(Arc::new(LeaseInner {
        parent: Arc::clone(&owner.root),
        directory: Arc::clone(&directory),
        name: name.clone(),
        cleanup: NativeOwnedWorkerScope::retain_current_cleanup(),
        cleaned: std::sync::atomic::AtomicBool::new(false),
    }));
    let request = NativeSkillGitRequest {
        url: url.to_owned(),
        directory: Arc::clone(&directory),
        directory_path: owner.root_path.join(&name),
        deadline: Instant::now() + Duration::from_secs(120),
        max_output_bytes: 64 * 1024,
        clone_rejection_bytes: 256 * 1024 * 1024,
        lease: lease.clone(),
    };
    if let Err(mut error) = runner.clone_repository(request, cancellation) {
        match lease.cleanup() {
            Ok(()) => {}
            Err(cleanup) => error.recovery_id = cleanup.recovery_id,
        }
        return Err(error);
    }
    filesystem::verify_named(&owner.root, &name, &directory)
        .map_err(|kind| NativeSkillManagedError::with_recovery(kind, name))?;
    Ok((directory, lease))
}
