//! Bounded queued invocation data and post-FIFO owned materialization.

use crate::{
    NativeOwnedWorkerScope, NativeQueuedJobId, NativeSkillCatalog, NativeSkillCatalogError,
    NativeSkillInvocationError, NativeSkillInvocationPlan, NativeSkillPromptContext,
    NativeSkillSelection, NativeSkillSnapshot,
};
use machine_god_core::{BoxFuture, CancellationToken, MAX_SESSION_USER_CONTEXT_BYTES};
use std::{fmt, sync::Arc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeQueuedSkillsReceipt {
    pub queued_id: NativeQueuedJobId,
    /// The host must visibly report that automatic invocation was suppressed.
    pub automatic_matching_incomplete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillsQueueError {
    Invocation(NativeSkillInvocationError),
    Catalog {
        selection_index: usize,
        error: NativeSkillCatalogError,
    },
    ContextLimit,
    Cancelled,
    WorkerUnavailable,
}
impl fmt::Display for NativeSkillsQueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("queued skill invocation failed")
    }
}
impl std::error::Error for NativeSkillsQueueError {}

type ContextResult = Result<Option<NativeSkillPromptContext>, NativeSkillsQueueError>;
type OwnedContextResult<L> = Result<(L, ContextResult), NativeSkillsQueueError>;
#[cfg(test)]
type BeforeRead = Box<dyn FnOnce(&CancellationToken) + Send>;

pub(crate) struct QueuedSkills {
    catalog: Arc<NativeSkillCatalog>,
    plan: NativeSkillInvocationPlan,
    workers: NativeOwnedWorkerScope,
    #[cfg(test)]
    before_read: Option<BeforeRead>,
}
impl QueuedSkills {
    pub(crate) fn resolve(
        prompt: &str,
        catalog: Arc<NativeSkillCatalog>,
        snapshot: &NativeSkillSnapshot,
        explicit: &[NativeSkillSelection],
        workers: NativeOwnedWorkerScope,
    ) -> Result<Self, NativeSkillsQueueError> {
        let plan = NativeSkillInvocationPlan::resolve(prompt, snapshot, explicit)
            .map_err(NativeSkillsQueueError::Invocation)?;
        for (selection_index, selection) in plan.selections().iter().enumerate() {
            if !catalog.owns_selection(selection) {
                return Err(NativeSkillsQueueError::Catalog {
                    selection_index,
                    error: NativeSkillCatalogError::WrongAuthority,
                });
            }
        }
        Ok(Self {
            catalog,
            plan,
            workers,
            #[cfg(test)]
            before_read: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn set_test_hook(&mut self, hook: impl FnOnce(&CancellationToken) + Send + 'static) {
        self.before_read = Some(Box::new(hook));
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.plan.retained_bytes()
    }
    pub(crate) fn incomplete(&self) -> bool {
        self.plan.automatic_matching_incomplete()
    }

    pub(crate) fn materialize<L: Send + 'static>(
        self,
        lease: L,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, OwnedContextResult<L>> {
        let workers = self.workers.clone();
        let on_drop = CancelOnDrop(Some(cancellation.clone()));
        Box::pin(async move {
            let mut on_drop = on_drop;
            let result = workers
                .run(move || {
                    // The exact runtime lease always travels with the response;
                    // callback panic is contained before that lease can be dropped.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.read_context(&cancellation)
                    }))
                    .unwrap_or_else(|payload| {
                        std::mem::forget(payload);
                        Err(NativeSkillsQueueError::WorkerUnavailable)
                    });
                    (lease, result)
                })
                .await
                .map_err(|_| NativeSkillsQueueError::WorkerUnavailable);
            if result.is_ok() {
                on_drop.0.take();
            }
            result
        })
    }

    fn read_context(
        self,
        cancellation: &CancellationToken,
    ) -> Result<Option<NativeSkillPromptContext>, NativeSkillsQueueError> {
        #[cfg(test)]
        if let Some(hook) = self.before_read {
            hook(cancellation);
        }
        let mut text = String::new();
        for (selection_index, selection) in self.plan.selections().iter().enumerate() {
            check_cancel(cancellation)?;
            let materialized =
                self.catalog
                    .materialize(selection, cancellation)
                    .map_err(|error| NativeSkillsQueueError::Catalog {
                        selection_index,
                        error,
                    })?;
            check_cancel(cancellation)?;
            let location = selection
                .location()
                .to_str()
                .ok_or(NativeSkillsQueueError::ContextLimit)?;
            append(&mut text, "External skill advisory context\nName: ")?;
            append(&mut text, selection.name())?;
            append(&mut text, "\nLocation: ")?;
            append(&mut text, location)?;
            append(&mut text, "\nFull external skill text:\n")?;
            append(&mut text, &materialized.text)?;
            append(&mut text, "\nEnd external skill advisory context\n\n")?;
        }
        check_cancel(cancellation)?;
        if self.plan.selections().is_empty() {
            return Ok(None);
        }
        NativeSkillPromptContext::new(text)
            .map(Some)
            .map_err(|_| NativeSkillsQueueError::ContextLimit)
    }
}

fn append(destination: &mut String, text: &str) -> Result<(), NativeSkillsQueueError> {
    if text.len() > MAX_SESSION_USER_CONTEXT_BYTES - destination.len() {
        return Err(NativeSkillsQueueError::ContextLimit);
    }
    destination.push_str(text);
    Ok(())
}
fn check_cancel(cancellation: &CancellationToken) -> Result<(), NativeSkillsQueueError> {
    if cancellation.is_cancelled() {
        Err(NativeSkillsQueueError::Cancelled)
    } else {
        Ok(())
    }
}
struct CancelOnDrop(Option<CancellationToken>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(token) = &self.0 {
            token.cancel();
        }
    }
}
