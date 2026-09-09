//! CLI-facing native environment selection, identity generation and worker join.
use super::{
    CancelOnDrop, NativeSessionMaintenanceError, NativeSessionMaintenanceReceipt,
    NativeSessionMaintenanceRequest,
};
use crate::session_store::FileSessionScanControl;
use crate::{FileSessionStore, NativeOwnedWorkerScope};
use machine_god_core::{BoxFuture, CancellationToken, SessionId, SessionIncarnationId};

/// Captures state-only environment on a completion-owned worker, opens only the
/// existing native hierarchy, and waits for its actual worker cleanup before
/// returning a confirmed receipt. A recovery uncertainty receipt retains the copy
/// ID without claiming successful join observation. Construction is inert.
/// Dropping a polled response requests
/// cancellation and closes admission; the native collector retains cleanup.
#[must_use]
pub fn execute_process_session_maintenance(
    request: NativeSessionMaintenanceRequest,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<NativeSessionMaintenanceReceipt, NativeSessionMaintenanceError>> {
    execute_with_environment(request, cancellation, || {
        crate::state_environment::capture_state_environment(
            &mut crate::state_environment::ProcessStateEnvironmentReader,
        )
    })
}

fn execute_with_environment(
    request: NativeSessionMaintenanceRequest,
    cancel: CancellationToken,
    environment: impl FnOnce() -> crate::NativeEnvironment + Send + 'static,
) -> BoxFuture<'static, Result<NativeSessionMaintenanceReceipt, NativeSessionMaintenanceError>> {
    Box::pin(async move {
        let scope = NativeOwnedWorkerScope::new();
        let close = CloseScope(scope.clone());
        let abandoned = CancellationToken::new();
        let guard = CancelOnDrop(abandoned.clone());
        let control = FileSessionScanControl {
            cancel,
            abandoned,
            #[cfg(test)]
            after_read: None,
        };
        control.check()?;
        let result = scope
            .run(move || {
                control.check()?;
                let environment = environment();
                control.check()?;
                let store = crate::root_selection::open_existing_session_store(&environment)
                    .map_err(|_| NativeSessionMaintenanceError::Unavailable)?
                    .ok_or(NativeSessionMaintenanceError::Missing)?;
                dispatch(&store, request, &control)
            })
            .await
            .map_err(|_| NativeSessionMaintenanceError::Unavailable);
        drop(close);
        let completion = scope.completion();
        let joined = crate::NativeOwnedWorkerSpawner::new()
            .run(move || completion.wait_on_worker())
            .await;
        if !matches!(joined, Ok(Ok(()))) {
            return join_failure(result);
        }
        drop(guard);
        result?
    })
}

fn join_failure(
    result: Result<
        Result<NativeSessionMaintenanceReceipt, NativeSessionMaintenanceError>,
        NativeSessionMaintenanceError,
    >,
) -> Result<NativeSessionMaintenanceReceipt, NativeSessionMaintenanceError> {
    use super::{NativeSessionCleanupStatus, NativeSessionMigration};
    let may_have_committed = match result {
        Ok(Ok(NativeSessionMaintenanceReceipt::Recovery(recovery))) => {
            return Ok(NativeSessionMaintenanceReceipt::RecoveryIndeterminate {
                session_id: recovery.record.id,
            });
        }
        Ok(Ok(receipt @ NativeSessionMaintenanceReceipt::RecoveryIndeterminate { .. })) => {
            return Ok(receipt);
        }
        Ok(
            Ok(NativeSessionMaintenanceReceipt::Migration(NativeSessionMigration::Migrated(_)))
            | Err(NativeSessionMaintenanceError::Indeterminate),
        ) => true,
        Ok(Ok(NativeSessionMaintenanceReceipt::Cleanup(report))) => {
            report.outcomes.iter().any(|status| {
                matches!(
                    status,
                    NativeSessionCleanupStatus::Completed
                        | NativeSessionCleanupStatus::Indeterminate
                )
            })
        }
        _ => false,
    };
    if may_have_committed {
        Err(NativeSessionMaintenanceError::Indeterminate)
    } else {
        Err(NativeSessionMaintenanceError::Unavailable)
    }
}
fn dispatch(
    store: &FileSessionStore,
    request: NativeSessionMaintenanceRequest,
    control: &FileSessionScanControl,
) -> Result<NativeSessionMaintenanceReceipt, NativeSessionMaintenanceError> {
    match request {
        NativeSessionMaintenanceRequest::Migrate { session_id } => store
            .maintenance_migrate(&session_id, control)
            .map(NativeSessionMaintenanceReceipt::Migration),
        NativeSessionMaintenanceRequest::Recover { session_id } => {
            let destination = SessionId::new(random_identity("s_")?)
                .map_err(|_| NativeSessionMaintenanceError::Unavailable)?;
            let incarnation = SessionIncarnationId::new(random_identity("inc_")?)
                .map_err(|_| NativeSessionMaintenanceError::Unavailable)?;
            match store.maintenance_recover(&session_id, &destination, &incarnation, control) {
                Ok(recovery) => Ok(NativeSessionMaintenanceReceipt::Recovery(recovery)),
                Err(NativeSessionMaintenanceError::Indeterminate) => {
                    Ok(NativeSessionMaintenanceReceipt::RecoveryIndeterminate {
                        session_id: destination,
                    })
                }
                Err(error) => Err(error),
            }
        }
        NativeSessionMaintenanceRequest::Cleanup { mode } => store
            .maintenance_cleanup(mode, control)
            .map(NativeSessionMaintenanceReceipt::Cleanup),
    }
}
fn random_identity(prefix: &str) -> Result<String, NativeSessionMaintenanceError> {
    use std::fmt::Write;
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|_| NativeSessionMaintenanceError::Unavailable)?;
    let mut result = String::with_capacity(prefix.len() + 64);
    result.push_str(prefix);
    for byte in random {
        write!(result, "{byte:02x}").expect("String formatting is infallible");
    }
    Ok(result)
}
struct CloseScope(NativeOwnedWorkerScope);
impl Drop for CloseScope {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[cfg(test)]
mod tests;
