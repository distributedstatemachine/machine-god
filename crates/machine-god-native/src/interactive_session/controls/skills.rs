//! Skills effects share the file-control fence and the actual host worker scope.

use super::{NativeInteractiveControlError as Error, NativeInteractiveControlReceipt as Receipt};
use crate::{
    NativeConversationRuntime, NativeOwnedWorkerScope, NativeSkillsCommand, NativeSkillsService,
    NativeSkillsServiceError, NativeSkillsServiceResult,
};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{path::PathBuf, sync::Arc};

/// Bound direct enum construction before retaining an accepted control. Domain
/// validation remains in the service before observation or managed effects.
pub(super) fn validate_size(
    command: &NativeSkillsCommand,
) -> Result<(), crate::NativeInteractiveError> {
    let (text, maximum) = match command {
        NativeSkillsCommand::List | NativeSkillsCommand::Path => return Ok(()),
        NativeSkillsCommand::Show { selector } | NativeSkillsCommand::Remove { selector } => {
            (selector, crate::MAX_NATIVE_SKILLS_SELECTOR_BYTES)
        }
        NativeSkillsCommand::Create { arguments } | NativeSkillsCommand::Install { arguments } => {
            (arguments, crate::MAX_NATIVE_SKILLS_COMMAND_BYTES)
        }
    };
    if text.is_empty() || text.len() > maximum || text.chars().any(|c| c.is_control() && c != '\t')
    {
        return Err(crate::NativeInteractiveError::Configuration);
    }
    Ok(())
}

pub(super) fn execute(
    runtime: Arc<NativeConversationRuntime>,
    service: Arc<NativeSkillsService>,
    workers: NativeOwnedWorkerScope,
    cwd: PathBuf,
    command: NativeSkillsCommand,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<Receipt, Error>> {
    run(runtime, workers, cancellation, move |token| {
        service.execute(command, &cwd, token)
    })
}

fn run(
    runtime: Arc<NativeConversationRuntime>,
    workers: NativeOwnedWorkerScope,
    cancellation: CancellationToken,
    operation: impl FnOnce(
        &CancellationToken,
    ) -> Result<NativeSkillsServiceResult, NativeSkillsServiceError>
    + Send
    + 'static,
) -> BoxFuture<'static, Result<Receipt, Error>> {
    let cancellation_on_drop = CancelOnDrop(cancellation.clone());
    Box::pin(async move {
        // Constructed outside the async body so dropping an unpolled response
        // also requests cancellation, but never starts work or acquires a fence.
        let _cancellation_on_drop = cancellation_on_drop;
        if cancellation.is_cancelled() {
            return Err(Error::Skills(NativeSkillsServiceError::Cancelled));
        }
        let permit = runtime.acquire_file_control().map_err(Error::Runtime)?;
        workers
            .run(move || {
                // Contain callback unwinding before dropping the admission permit:
                // waking an injected lifecycle waiter must not double-panic.
                let result = contain(|| operation(&cancellation));
                // A panicking lifecycle notification must not discard publication
                // receipts or managed errors carrying retained recovery identifiers.
                let _release = contain(|| drop(permit));
                result?.map_err(Error::Skills)
            })
            .await
            .map_err(|_| Error::Unavailable)?
            .map(Receipt::Skills)
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

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
