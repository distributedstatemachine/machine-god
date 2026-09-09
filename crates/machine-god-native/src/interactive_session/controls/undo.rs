//! File effects retain both exact runtime admission and full-host worker ownership.

use super::{NativeInteractiveControlError as Error, NativeInteractiveControlReceipt as Receipt};
use crate::{
    FileUndoError, FileUndoOutcome, FileUndoTracker, NativeConversationRuntime,
    NativeOwnedWorkerScope,
};
use machine_god_core::{BoxFuture, CancellationToken};
use std::sync::Arc;

pub(super) fn execute(
    runtime: Arc<NativeConversationRuntime>,
    tracker: Arc<FileUndoTracker>,
    workers: NativeOwnedWorkerScope,
) -> BoxFuture<'static, Result<Receipt, Error>> {
    run(runtime, workers, move || {
        tracker.undo_last(&CancellationToken::new())
    })
}

pub(in crate::interactive_session) fn run(
    runtime: Arc<NativeConversationRuntime>,
    workers: NativeOwnedWorkerScope,
    inverse: impl FnOnce() -> Result<FileUndoOutcome, FileUndoError> + Send + 'static,
) -> BoxFuture<'static, Result<Receipt, Error>> {
    Box::pin(async move {
        let permit = runtime
            .acquire_file_control()
            .map_err(|_| Error::Undo(FileUndoError::Unavailable))?;
        workers
            .run(move || {
                // The worker, not its response future, owns this fence. Dropping
                // the owner cannot retire an inverse still executing syscalls.
                // Accepted controls finish before deferred cancel/shutdown. The
                // tracker itself owns the inverse's irreversible boundary.
                let result = contain(inverse).and_then(std::convert::identity);
                // Admission release may wake an injected waiter. Contain it
                // separately, never while unwinding the inverse callback.
                contain(|| drop(permit))?;
                result
            })
            .await
            .map_err(|_| Error::Undo(FileUndoError::Unavailable))?
            .map(Receipt::Undone)
            .map_err(Error::Undo)
    })
}

fn contain<T>(operation: impl FnOnce() -> T) -> Result<T, FileUndoError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|payload| {
        // Match the owned-worker boundary: payload destructors may also panic.
        std::mem::forget(payload);
        FileUndoError::Ambiguous
    })
}
