//! ACP resource data held until the native FIFO selects exact turn authority.

use crate::acp::{
    prompt::NativeAcpPrompt,
    resources::{NativeAcpOwnedResourceContextResult, NativeAcpResourceContextReader},
};
use crate::{NativeOwnedWorkerScope, NativeWorkspaceScopeSnapshot};
use machine_god_core::{BoxFuture, CancellationToken, Prompt};

pub(super) struct QueuedAcpResources {
    input: NativeAcpPrompt,
    workers: NativeOwnedWorkerScope,
    #[cfg(test)]
    pub(super) before_read: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

impl QueuedAcpResources {
    pub(super) fn split(
        mut input: NativeAcpPrompt,
        workers: NativeOwnedWorkerScope,
    ) -> (Prompt, Self) {
        let prompt = std::mem::replace(&mut input.prompt, Prompt::from(""));
        (
            prompt,
            Self {
                input,
                workers,
                #[cfg(test)]
                before_read: None,
            },
        )
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.input.retained_bytes()
    }

    pub(super) fn materialize<L: Send + 'static>(
        mut self,
        prompt: Prompt,
        scope: NativeWorkspaceScopeSnapshot,
        lease: L,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, NativeAcpOwnedResourceContextResult<L>> {
        self.input.prompt = prompt;
        let reader = NativeAcpResourceContextReader::new(scope, self.workers);
        #[cfg(test)]
        let reader = match self.before_read {
            Some(hook) => reader.with_test_before_read(hook),
            None => reader,
        };
        reader.materialize_with_lease(self.input, lease, cancellation)
    }
}
