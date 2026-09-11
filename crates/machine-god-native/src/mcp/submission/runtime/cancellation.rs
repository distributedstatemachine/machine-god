use super::McpSubmissionRuntime;
use machine_god_core::{CancellationToken, Cancelled};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Only cancellation observations are retained, never their executable owner.
pub(crate) struct McpRuntimeCancellation {
    retired: Cancelled,
    guards: Box<[Cancelled]>,
}

impl McpRuntimeCancellation {
    pub(super) fn new(runtime: &McpSubmissionRuntime) -> Self {
        Self {
            retired: runtime.cancellation.cancelled(),
            // Empty ordinary bindings keep an allocation-free empty slice.
            guards: runtime
                .authority_cancellations
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(CancellationToken::cancelled)
                .collect(),
        }
    }
}

impl Future for McpRuntimeCancellation {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if Pin::new(&mut this.retired).poll(cx).is_ready()
            || this
                .guards
                .iter_mut()
                .any(|guard| Pin::new(guard).poll(cx).is_ready())
        {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
