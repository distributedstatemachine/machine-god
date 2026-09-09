//! Explicit-authority, owned-worker session maintenance. No operation replays tools.

#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::session_store::{FileSessionScanControl, FileSessionScanError};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::{FileSessionStore, NativeOwnedWorkerScope};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_core::SessionIncarnationId;
use machine_god_core::{BoxFuture, CancellationToken, SessionId, SessionRecord};
use std::fmt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::Arc;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod process;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use process::execute_process_session_maintenance;

#[derive(Debug)]
pub enum NativeSessionMaintenanceRequest {
    Migrate { session_id: SessionId },
    Recover { session_id: SessionId },
    Cleanup { mode: NativeSessionCleanupMode },
}
#[derive(Debug)]
pub enum NativeSessionMaintenanceReceipt {
    Migration(NativeSessionMigration),
    Recovery(NativeSessionRecovery),
    /// Copy identity is known, but durability or worker-join observation is not.
    RecoveryIndeterminate {
        session_id: SessionId,
    },
    Cleanup(NativeSessionCleanupReport),
}
/// Unsupported targets fail before environment, filesystem or worker effects.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[must_use]
pub fn execute_process_session_maintenance(
    _request: NativeSessionMaintenanceRequest,
    _cancellation: CancellationToken,
) -> BoxFuture<'static, Result<NativeSessionMaintenanceReceipt, NativeSessionMaintenanceError>> {
    Box::pin(async { Err(NativeSessionMaintenanceError::UnsupportedPlatform) })
}

/// Fixed, content-free failures. Indeterminate means publication crossed its
/// effect boundary; reload the selected ID before retrying.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSessionMaintenanceError {
    UnsupportedPlatform,
    Missing,
    Busy,
    Cancelled,
    Oversized,
    Corrupt,
    UnsupportedVersion,
    DestinationExists,
    InvalidDestination,
    Unavailable,
    Indeterminate,
}
impl fmt::Display for NativeSessionMaintenanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session maintenance: {self:?}")
    }
}
impl std::error::Error for NativeSessionMaintenanceError {}
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl From<FileSessionScanError> for NativeSessionMaintenanceError {
    fn from(error: FileSessionScanError) -> Self {
        match error {
            FileSessionScanError::Busy => Self::Busy,
            FileSessionScanError::Cancelled => Self::Cancelled,
            FileSessionScanError::Store(_) => Self::Unavailable,
        }
    }
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl From<rustix::io::Errno> for NativeSessionMaintenanceError {
    fn from(_: rustix::io::Errno) -> Self {
        Self::Unavailable
    }
}

/// Envelope version remains one; migration upgrades native metadata only.
#[derive(Debug)]
pub enum NativeSessionMigration {
    AlreadyCurrent(SessionRecord),
    Migrated(SessionRecord),
}
/// A separately published resumable record, never a replacement of its source.
#[derive(Debug)]
pub struct NativeSessionRecovery {
    pub record: SessionRecord,
    pub truncated_source: bool,
    pub unknown_tool_results: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSessionCleanupMode {
    ReportOnly,
    Apply,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSessionCleanupStatus {
    ActiveWriter,
    Untrusted,
    ReportOnly,
    Completed,
    Indeterminate,
}
/// Counts, not arbitrary filesystem paths. At most 1,024 entries and 64 MiB are
/// examined; incomplete reports never authorize cleanup of unseen entries.
#[derive(Debug, Default)]
pub struct NativeSessionCleanupReport {
    pub outcomes: Vec<NativeSessionCleanupStatus>,
    pub scan_complete: bool,
}

/// Inert construction over the exact supplied store and completion-owned workers.
#[derive(Clone)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub struct NativeSessionMaintenance {
    store: Arc<FileSessionStore>,
    workers: NativeOwnedWorkerScope,
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl NativeSessionMaintenance {
    #[must_use]
    pub const fn new(store: Arc<FileSessionStore>, workers: NativeOwnedWorkerScope) -> Self {
        Self { store, workers }
    }
    /// Upgrades supported historical metadata atomically. No historical fact is inferred.
    #[must_use]
    pub fn migrate(
        &self,
        id: SessionId,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionMigration, NativeSessionMaintenanceError>> {
        self.run(cancel, move |store, control| {
            store.maintenance_migrate(&id, control)
        })
    }
    /// Salvages complete messages into a new identity. Clears continuation,
    /// preferences and metadata authority; unpaired tools receive unknown results.
    #[must_use]
    pub fn recover(
        &self,
        source: SessionId,
        destination: SessionId,
        incarnation: SessionIncarnationId,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionRecovery, NativeSessionMaintenanceError>> {
        self.run(cancel, move |store, control| {
            store.maintenance_recover(&source, &destination, &incarnation, control)
        })
    }
    /// Only redundant or exactly reproducible native migration staging files can
    /// be removed. Unknown, partial and active artifacts remain untouched.
    #[must_use]
    pub fn cleanup(
        &self,
        mode: NativeSessionCleanupMode,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCleanupReport, NativeSessionMaintenanceError>> {
        self.run(cancel, move |store, control| {
            store.maintenance_cleanup(mode, control)
        })
    }
    fn run<T: Send + 'static>(
        &self,
        cancel: CancellationToken,
        operation: impl FnOnce(
            &FileSessionStore,
            &FileSessionScanControl,
        ) -> Result<T, NativeSessionMaintenanceError>
        + Send
        + 'static,
    ) -> BoxFuture<'static, Result<T, NativeSessionMaintenanceError>> {
        let store = Arc::clone(&self.store);
        let workers = self.workers.clone();
        Box::pin(async move {
            let abandoned = CancellationToken::new();
            let guard = CancelOnDrop(abandoned.clone());
            let control = FileSessionScanControl {
                cancel,
                abandoned,
                #[cfg(test)]
                after_read: None,
            };
            control.check()?;
            let result = workers
                .run(move || operation(&store, &control))
                .await
                .map_err(|_| NativeSessionMaintenanceError::Unavailable)?;
            drop(guard);
            result
        })
    }
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl fmt::Debug for NativeSessionMaintenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSessionMaintenance")
            .finish_non_exhaustive()
    }
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
struct CancelOnDrop(CancellationToken);
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
