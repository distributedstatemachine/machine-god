//! Shared ownership wrapper for bounded synchronous profile control operations.

use super::{
    ControlFuture, NativeInteractiveControlError as Error,
    NativeInteractiveControlReceipt as Receipt,
};
use crate::{NativeConversationRuntime, NativeOwnedWorkerScope};
use machine_god_core::CancellationToken;
use std::sync::Arc;

pub(super) fn run(
    runtime: Arc<NativeConversationRuntime>,
    workers: NativeOwnedWorkerScope,
    cancellation: CancellationToken,
    cancelled: Error,
    operation: impl FnOnce(&CancellationToken) -> Result<Receipt, Error> + Send + 'static,
) -> ControlFuture {
    let cancellation_on_drop = CancelOnDrop(cancellation.clone());
    Box::pin(async move {
        // An unpolled response owns only cancellation, never a runtime fence.
        let _cancellation_on_drop = cancellation_on_drop;
        if cancellation.is_cancelled() {
            return Err(cancelled);
        }
        let permit = runtime.acquire_file_control().map_err(Error::Runtime)?;
        workers
            .run(move || {
                // Contain the operation before releasing the fence: neither an
                // operation panic nor a panicking waiter may erase a save receipt.
                let result = contain(|| operation(&cancellation));
                let _release = contain(|| drop(permit));
                result?
            })
            .await
            .map_err(|_| Error::Unavailable)?
    })
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn contain<T>(operation: impl FnOnce() -> T) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|payload| {
        std::mem::forget(payload);
        Error::Unavailable
    })
}
