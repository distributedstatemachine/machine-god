//! One explicit operation owner for administrative and interactive workspace edits.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use machine_god_core::BoxFuture;

use crate::{
    NativeConversationRuntime, NativeOwnedWorkerScope, NativeUserConfigError,
    NativeUserConfigStore, NativeWorkspaceAuthority, NativeWorkspaceAuthorityError,
    NativeWorkspaceScopeSnapshot,
};

mod operation;
mod sources;
pub(crate) use sources::merge_workspace_sources_blocking;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeWorkspaceAction {
    List,
    Add(PathBuf),
    Remove(PathBuf),
    Clear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeWorkspaceServiceError {
    Busy,
    InvalidPath,
    UnknownDirectory,
    Authority(NativeWorkspaceAuthorityError),
    Config(NativeUserConfigError),
    Unavailable,
    Ambiguous,
}
impl fmt::Display for NativeWorkspaceServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("workspace operation failed")
    }
}
impl std::error::Error for NativeWorkspaceServiceError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeWorkspaceReconciliation {
    CachedBusy,
    Refreshed,
    Confirmed,
    AmbiguousIntended,
    AmbiguousBefore,
    Indeterminate,
    ReloadFailed(NativeWorkspaceServiceError),
}

/// Saved and runtime observations are independent. `None` means unknown, never
/// rollback. An ambiguous reconciliation is not a confirmed durability receipt.
#[derive(Clone, Debug)]
pub struct NativeWorkspaceReceipt {
    pub action: NativeWorkspaceAction,
    pub snapshot: NativeWorkspaceScopeSnapshot,
    pub saved_changed: Option<bool>,
    pub runtime_changed: Option<bool>,
    pub launch_flag_can_restore: bool,
    pub reconciliation: NativeWorkspaceReconciliation,
}

/// Construction receives every effect capability explicitly and performs no I/O.
pub struct NativeWorkspaceService {
    authority: NativeWorkspaceAuthority,
    store: Arc<NativeUserConfigStore>,
    workers: NativeOwnedWorkerScope,
    active: AtomicBool,
}
impl fmt::Debug for NativeWorkspaceService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeWorkspaceService { .. }")
    }
}
impl NativeWorkspaceService {
    #[must_use]
    pub const fn new(
        authority: NativeWorkspaceAuthority,
        store: Arc<NativeUserConfigStore>,
        workers: NativeOwnedWorkerScope,
    ) -> Self {
        Self {
            authority,
            store,
            workers,
            active: AtomicBool::new(false),
        }
    }

    /// Administrative operation using the same owner as interactive commands.
    /// The returned future is inert until polled.
    #[must_use]
    pub fn execute(
        self: &Arc<Self>,
        action: NativeWorkspaceAction,
    ) -> BoxFuture<'static, Result<NativeWorkspaceReceipt, NativeWorkspaceServiceError>> {
        self.execute_inner(
            None,
            action,
            #[cfg(test)]
            None,
        )
    }

    /// Captures the exact runtime and rejects active or queued work before I/O.
    /// A busy list is an explicitly cached observation, not a refresh.
    #[must_use]
    pub fn execute_for_runtime(
        self: &Arc<Self>,
        runtime: Arc<NativeConversationRuntime>,
        action: NativeWorkspaceAction,
    ) -> BoxFuture<'static, Result<NativeWorkspaceReceipt, NativeWorkspaceServiceError>> {
        self.execute_inner(
            Some(runtime),
            action,
            #[cfg(test)]
            None,
        )
    }

    fn execute_inner(
        self: &Arc<Self>,
        runtime: Option<Arc<NativeConversationRuntime>>,
        action: NativeWorkspaceAction,
        #[cfg(test)] hook: Option<Hook>,
    ) -> BoxFuture<'static, Result<NativeWorkspaceReceipt, NativeWorkspaceServiceError>> {
        let service = Arc::clone(self);
        Box::pin(async move {
            let lane = match service.active.compare_exchange(
                false,
                true,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => Lane(Arc::clone(&service)),
                Err(_) => return service.busy(action),
            };
            let Ok(lease) = runtime
                .as_ref()
                .map(|runtime| runtime.acquire_workspace_control())
                .transpose()
            else {
                return service.busy(action);
            };
            let mutation = !matches!(action, NativeWorkspaceAction::List);
            let started = Arc::new(AtomicBool::new(false));
            let observed_start = Arc::clone(&started);
            service
                .workers
                .clone()
                .run(move || {
                    started.store(true, Ordering::Release);
                    let result = operation::run(
                        &service,
                        action,
                        #[cfg(test)]
                        hook.as_ref(),
                    );
                    let cleanup = contain(|| {
                        #[cfg(test)]
                        if let Some(hook) = hook {
                            hook(Stage::BeforeCleanup);
                        }
                        drop(lease);
                        drop(runtime);
                        drop(lane);
                    });
                    match (result, cleanup) {
                        (Ok(mut receipt), Err(())) => {
                            receipt.reconciliation = NativeWorkspaceReconciliation::ReloadFailed(
                                NativeWorkspaceServiceError::Unavailable,
                            );
                            Ok(receipt)
                        }
                        (result, _) => result,
                    }
                })
                .await
                .map_err(|_| {
                    if mutation && observed_start.load(Ordering::Acquire) {
                        NativeWorkspaceServiceError::Ambiguous
                    } else {
                        NativeWorkspaceServiceError::Unavailable
                    }
                })?
        })
    }

    fn busy(
        &self,
        action: NativeWorkspaceAction,
    ) -> Result<NativeWorkspaceReceipt, NativeWorkspaceServiceError> {
        if action != NativeWorkspaceAction::List {
            return Err(NativeWorkspaceServiceError::Busy);
        }
        Ok(NativeWorkspaceReceipt {
            action,
            snapshot: self
                .authority
                .snapshot()
                .map_err(NativeWorkspaceServiceError::Authority)?,
            saved_changed: Some(false),
            runtime_changed: Some(false),
            launch_flag_can_restore: false,
            reconciliation: NativeWorkspaceReconciliation::CachedBusy,
        })
    }
}

struct Lane(Arc<NativeWorkspaceService>);
impl Drop for Lane {
    fn drop(&mut self) {
        self.0.active.store(false, Ordering::Release);
    }
}

fn contain<T>(operation: impl FnOnce() -> T) -> Result<T, ()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|payload| {
        std::mem::forget(payload);
    })
}

#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum Stage {
    BeforeLoad,
    BeforeCommit,
    AfterCommit,
    BeforeInstall,
    BeforeCleanup,
}
#[cfg(test)]
type Hook = Arc<dyn Fn(Stage) + Send + Sync>;
#[cfg(test)]
mod tests;
