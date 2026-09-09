use super::{
    Availability, Entry, ProductionWorkspaceCommandHost, Provenance, Reconciliation,
    WorkspaceAction, WorkspaceCommandHost, WorkspaceOperationalFailure, WorkspaceSnapshot,
};
use machine_god_native::{
    ConfigFileState, NativeEnvironment, NativeOwnedWorkerScope, NativeRootSelection,
    NativeUserConfigError, NativeUserConfigStore, NativeWorkspaceAction,
    NativeWorkspaceAuthorityError, NativeWorkspaceReceipt, NativeWorkspaceReconciliation,
    NativeWorkspaceService, NativeWorkspaceServiceError, inspect_native_status,
    prepare_native_workspace, prepare_native_workspace_without_settings,
};
use std::path::Path;
use std::sync::Arc;

impl WorkspaceCommandHost for ProductionWorkspaceCommandHost {
    fn execute_workspace(
        &self,
        action: &WorkspaceAction,
    ) -> Result<WorkspaceSnapshot, WorkspaceOperationalFailure> {
        let environment = NativeEnvironment::from_process();
        let status = inspect_native_status(&environment);
        let store = match status.config_file_path().and_then(Path::parent) {
            Some(directory) => Some(Arc::new(NativeUserConfigStore::new(directory.to_owned()))),
            None if status.config_file_state() == ConfigFileState::Unavailable => None,
            None => return Err(WorkspaceOperationalFailure::Unavailable),
        };
        if store.is_none() && !matches!(action, WorkspaceAction::List) {
            return Err(WorkspaceOperationalFailure::Unavailable);
        }
        let selection = NativeRootSelection::from_current_process(&environment)
            .map_err(|_| WorkspaceOperationalFailure::Unavailable)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| WorkspaceOperationalFailure::Unavailable)?;
        let workers = NativeOwnedWorkerScope::new();
        let completion = workers.completion();
        let result = runtime.block_on(async {
            let preparation = match store.as_ref() {
                Some(store) => prepare_native_workspace(
                    selection,
                    store.clone(),
                    Vec::new(),
                    false,
                    workers.clone(),
                ),
                None => prepare_native_workspace_without_settings(
                    selection,
                    Vec::new(),
                    false,
                    workers.clone(),
                ),
            };
            let authority = preparation.await?;
            let service = Arc::new(match store {
                Some(store) => NativeWorkspaceService::new(authority, store, workers.clone()),
                None => NativeWorkspaceService::without_settings(authority, workers.clone()),
            });
            service
                .execute(match action {
                    WorkspaceAction::List => NativeWorkspaceAction::List,
                    WorkspaceAction::Add(path) => NativeWorkspaceAction::Add(path.clone()),
                    WorkspaceAction::Remove(path) => NativeWorkspaceAction::Remove(path.clone()),
                    WorkspaceAction::Clear => NativeWorkspaceAction::Clear,
                })
                .await
        });
        workers.close();
        drop(runtime);
        completion
            .wait_on_worker()
            .map_err(|_| WorkspaceOperationalFailure::Unavailable)?;
        // Publication responses and actual worker/TLS completion both precede
        // projection/output; no collector or native worker is detached.
        result
            .map(|receipt| from_native(&receipt))
            .map_err(classify)
    }
}

pub(super) fn from_native(receipt: &NativeWorkspaceReceipt) -> WorkspaceSnapshot {
    let reconciliation = match receipt.reconciliation {
        NativeWorkspaceReconciliation::CachedBusy => Reconciliation::CachedBusy,
        NativeWorkspaceReconciliation::Refreshed => Reconciliation::Refreshed,
        NativeWorkspaceReconciliation::Confirmed => Reconciliation::Confirmed,
        NativeWorkspaceReconciliation::AmbiguousIntended => Reconciliation::AmbiguousIntended,
        NativeWorkspaceReconciliation::AmbiguousBefore => Reconciliation::AmbiguousBefore,
        NativeWorkspaceReconciliation::Indeterminate => Reconciliation::Indeterminate,
        NativeWorkspaceReconciliation::ReloadFailed(error) => {
            Reconciliation::ReloadFailed(classify(error))
        }
    };
    WorkspaceSnapshot {
        primary: receipt.snapshot.primary_identity().to_owned(),
        generation: receipt.snapshot.generation(),
        saved_suppressed: receipt.snapshot.saved_suppressed(),
        entries: receipt
            .snapshot
            .entries()
            .iter()
            .map(|entry| Entry {
                source: entry.source().source().to_owned(),
                identity: entry.source().identity().to_owned(),
                identity_canonical: entry.source().identity_canonical(),
                provenance: Provenance {
                    saved: entry.saved(),
                    launch: entry.launch(),
                },
                availability: Availability {
                    available: entry.available(),
                    active: entry.active(),
                },
            })
            .collect(),
        saved_changed: receipt.saved_changed,
        runtime_changed: receipt.runtime_changed,
        reconciliation,
        launch_flag_can_restore: receipt.launch_flag_can_restore,
    }
}

pub(super) fn classify(error: NativeWorkspaceServiceError) -> WorkspaceOperationalFailure {
    match error {
        NativeWorkspaceServiceError::Busy => WorkspaceOperationalFailure::Busy,
        NativeWorkspaceServiceError::InvalidPath => WorkspaceOperationalFailure::InvalidPath,
        NativeWorkspaceServiceError::UnknownDirectory => {
            WorkspaceOperationalFailure::UnknownDirectory
        }
        NativeWorkspaceServiceError::Ambiguous => WorkspaceOperationalFailure::Ambiguous,
        NativeWorkspaceServiceError::Unavailable => WorkspaceOperationalFailure::Unavailable,
        NativeWorkspaceServiceError::Authority(error) => match error {
            NativeWorkspaceAuthorityError::InvalidPath => WorkspaceOperationalFailure::InvalidPath,
            NativeWorkspaceAuthorityError::TooManyDirectories => {
                WorkspaceOperationalFailure::ResourceLimit
            }
            NativeWorkspaceAuthorityError::DuplicateRoot => {
                WorkspaceOperationalFailure::DuplicateRoot
            }
            NativeWorkspaceAuthorityError::OverlappingState => {
                WorkspaceOperationalFailure::OverlappingState
            }
            _ => WorkspaceOperationalFailure::Unavailable,
        },
        NativeWorkspaceServiceError::Config(error) => match error {
            NativeUserConfigError::Busy => WorkspaceOperationalFailure::Busy,
            NativeUserConfigError::Conflict => WorkspaceOperationalFailure::Conflict,
            NativeUserConfigError::InvalidConfig(_) => {
                WorkspaceOperationalFailure::InvalidConfiguration
            }
            NativeUserConfigError::CommitAmbiguous => WorkspaceOperationalFailure::Ambiguous,
            NativeUserConfigError::UnsafePath => WorkspaceOperationalFailure::UnsafePath,
            NativeUserConfigError::Persistence => WorkspaceOperationalFailure::Persistence,
            _ => WorkspaceOperationalFailure::Unavailable,
        },
    }
}
