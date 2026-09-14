//! Blocking lifecycle preparation on the exact host-owned worker scope.

use crate::{
    NativeConversation, NativeConversationError, NativeInteractiveError,
    NativeInteractiveSessionOptions, NativeInteractiveTransition, NativeOwnedWorkerScope,
    NativeSessionLifecycle, NativeSessionMetadata, prepare_native_session_resume,
};
use machine_god_core::BoxFuture;

pub(super) struct Preparation {
    lifecycle: NativeSessionLifecycle,
    options: NativeInteractiveSessionOptions,
    kind: NativeInteractiveTransition,
    now_ms: i64,
    #[cfg(test)]
    before_io: Option<Box<dyn FnOnce() + Send>>,
}
impl Preparation {
    pub(super) fn new(
        lifecycle: NativeSessionLifecycle,
        options: NativeInteractiveSessionOptions,
        kind: NativeInteractiveTransition,
        now_ms: i64,
    ) -> Self {
        Self {
            lifecycle,
            options,
            kind,
            now_ms,
            #[cfg(test)]
            before_io: None,
        }
    }
    /// Inert until polled. An admitted operation owns the lifecycle/engine lease
    /// on its worker through the complete prepare/adopt result, even if the
    /// response future is dropped. Composition/terminal effects stay async.
    pub(super) fn run(
        self,
        workers: NativeOwnedWorkerScope,
    ) -> BoxFuture<'static, Result<NativeConversation, NativeInteractiveError>> {
        Box::pin(async move {
            workers
                .run(move || self.run_blocking())
                .await
                .map_err(|_| NativeInteractiveError::Unavailable)?
        })
    }
    fn run_blocking(self) -> Result<NativeConversation, NativeInteractiveError> {
        #[cfg(test)]
        if let Some(hook) = self.before_io {
            hook();
        }
        // These lifecycle futures perform only native identity/store work.
        // Never move candidate composition or terminal Tokio I/O into this executor.
        futures_executor::block_on(async move {
            match self.kind {
                NativeInteractiveTransition::Resume(target) => prepare_native_session_resume(
                    &self.lifecycle,
                    target,
                    &self.options.workspace,
                    self.now_ms,
                )
                .await
                .map_err(NativeInteractiveError::Resume)?
                .adopt()
                .await
                .map_err(NativeInteractiveError::Resume),
                NativeInteractiveTransition::Clear
                | NativeInteractiveTransition::New
                | NativeInteractiveTransition::Reset => {
                    let metadata = NativeSessionMetadata::new(
                        &self.options.workspace,
                        self.now_ms,
                        self.options.origin,
                    )
                    .map_err(|_| NativeInteractiveError::Configuration)?;
                    let session = self
                        .lifecycle
                        .create_generated_with_metadata(metadata)
                        .await
                        .map_err(|error| {
                            NativeInteractiveError::Conversation(
                                NativeConversationError::Lifecycle(error),
                            )
                        })?;
                    NativeConversation::from_session(session).map_err(Into::into)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests;
