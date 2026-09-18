//! Host policy for retained operations, independent of presentation or shutdown.
use crate::{managed::manager::ManagedManager, mcp::runtime::NativeMcpRuntimeClock};
use machine_god_core::BoxFuture;
use std::{sync::Arc, task::Context, time::Duration};

const RETRY_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct Recovery {
    clock: Arc<dyn NativeMcpRuntimeClock>,
    sleep: Option<BoxFuture<'static, ()>>,
}

impl Recovery {
    pub(crate) fn new(clock: Arc<dyn NativeMcpRuntimeClock>) -> Self {
        Self { clock, sleep: None }
    }

    pub(crate) fn poll(&mut self, manager: &ManagedManager, cx: &mut Context<'_>) {
        if manager.progress().recovery_required.is_none() {
            self.sleep = None;
            return;
        }
        if self.sleep.is_none() {
            let Some(deadline) = self.clock.now().checked_add(RETRY_INTERVAL) else {
                // Clock exhaustion cannot authorize an unbounded immediate retry.
                return;
            };
            let clock = self.clock.clone();
            self.sleep = Some(Box::pin(async move { clock.sleep_until(deadline).await }));
        }
        if self.sleep.as_mut().unwrap().as_mut().poll(cx).is_ready() {
            self.sleep = None;
            // The manager owns every original future/receipt and its capacity.
            // No new command, creation, provider call or global fence reset.
            manager.retry_automatic_reconciliation();
            cx.waker().wake_by_ref();
        }
    }
}
