//! Success-only transfer from the original run to the retained MCP service.

use crate::owned_worker::{NativeOwnedWorkerServiceHandoff, current_service_handoff};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(super) struct ServiceHandoff {
    published: AtomicBool,
    workers: Mutex<[Option<NativeOwnedWorkerServiceHandoff>; 2]>,
}

impl ServiceHandoff {
    // Both original workers register before the startup receipt becomes visible.
    pub(super) fn register_owner(&self) {
        self.register(0);
    }

    pub(super) fn register_child(&self) {
        self.register(1);
    }

    fn register(&self, index: usize) {
        let previous = self
            .workers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[index]
            .replace(current_service_handoff().expect("MCP owned worker"));
        debug_assert!(previous.is_none());
        drop(previous);
    }

    // Only native executable catalog publication calls this. Raw connection,
    // initialize, private candidate construction and Drop do not publish.
    pub(super) fn publish(&self) {
        self.published.store(true, Ordering::Release);
    }

    // Called by the original child worker before handing its process cleanup
    // to service lifetime. Completion/waker callbacks never run under this lock.
    pub(super) fn promote(&self) -> bool {
        if !self.published.load(Ordering::Acquire) {
            return false;
        }
        let workers = std::mem::take(
            &mut *self
                .workers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for worker in workers.into_iter().flatten() {
            worker.promote();
        }
        true
    }
}
