//! Audited test-only `RawWaker` fixture for lock-reentrancy regressions.

use core::mem::ManuallyDrop;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{RawWaker, RawWakerVTable, Waker};
use std::sync::Arc;

/// Raw-waker callback that reenters the caller-supplied operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Callback {
    Clone,
    Drop,
    Wake,
}

struct State {
    callback: Callback,
    reenter: Box<dyn Fn() + Send + Sync>,
    calls: AtomicUsize,
}

/// Observable handle for a reentrant waker fixture.
#[derive(Clone)]
pub struct Handle(Arc<State>);

impl core::fmt::Debug for Handle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Handle")
            .field("calls", &self.calls())
            .finish_non_exhaustive()
    }
}

impl Handle {
    #[must_use]
    pub fn calls(&self) -> usize {
        self.0.calls.load(Ordering::Relaxed)
    }
}

impl State {
    fn invoke(&self, callback: Callback) {
        if self.callback == callback {
            (self.reenter)();
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
    }
}

unsafe fn clone(data: *const ()) -> RawWaker {
    let state = ManuallyDrop::new(unsafe { Arc::<State>::from_raw(data.cast()) });
    state.invoke(Callback::Clone);
    raw(Arc::clone(&state))
}

unsafe fn wake(data: *const ()) {
    let state = unsafe { Arc::<State>::from_raw(data.cast()) };
    state.invoke(Callback::Wake);
}

unsafe fn wake_by_ref(data: *const ()) {
    let state = ManuallyDrop::new(unsafe { Arc::<State>::from_raw(data.cast()) });
    state.invoke(Callback::Wake);
}

unsafe fn drop(data: *const ()) {
    let state = unsafe { Arc::<State>::from_raw(data.cast()) };
    state.invoke(Callback::Drop);
}

const VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

fn raw(state: Arc<State>) -> RawWaker {
    RawWaker::new(Arc::into_raw(state).cast(), &VTABLE)
}

/// Creates a waker whose selected raw callback runs `reenter` synchronously.
#[must_use]
pub fn new(callback: Callback, reenter: impl Fn() + Send + Sync + 'static) -> (Waker, Handle) {
    let state = Arc::new(State {
        callback,
        reenter: Box::new(reenter),
        calls: AtomicUsize::new(0),
    });
    let waker = unsafe { Waker::from_raw(raw(Arc::clone(&state))) };
    (waker, Handle(state))
}
