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
    super::owned_operation::run(
        runtime,
        workers,
        cancellation,
        Error::Skills(NativeSkillsServiceError::Cancelled),
        move |token| operation(token).map(Receipt::Skills).map_err(Error::Skills),
    )
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
