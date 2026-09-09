//! One bounded recording receipt; no executor thread or queue per frame.

use super::TerminalTapeRecordingError;
use futures_util::task::AtomicWaker;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

struct State<T> {
    value: Option<Result<T, TerminalTapeRecordingError>>,
    abandoned: bool,
}
struct Shared<T> {
    state: Mutex<State<T>>,
    wake: AtomicWaker,
}
pub(super) struct Sender<T>(Option<Arc<Shared<T>>>);
pub(super) struct Receiver<T>(Arc<Shared<T>>);

pub(super) fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            value: None,
            abandoned: false,
        }),
        wake: AtomicWaker::new(),
    });
    (Sender(Some(Arc::clone(&shared))), Receiver(shared))
}

fn contain(operation: impl FnOnce()) -> bool {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(operation)) {
        std::mem::forget(payload);
        false
    } else {
        true
    }
}

impl<T> Sender<T> {
    pub(super) fn send(mut self, value: Result<T, TerminalTapeRecordingError>) {
        if let Some(shared) = self.0.take() {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.abandoned {
                drop(state);
                drop(value);
                return;
            }
            state.value = Some(value);
            drop(state);
            contain(|| shared.wake.wake());
        }
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if let Some(shared) = self.0.take() {
            Sender(Some(shared)).send(Err(TerminalTapeRecordingError::WorkerUnavailable));
        }
    }
}
impl<T> Future for Receiver<T> {
    type Output = Result<T, TerminalTapeRecordingError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let take = || {
            self.0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .value
                .take()
        };
        if let Some(value) = take() {
            return Poll::Ready(value);
        }
        if !contain(|| self.0.wake.register(cx.waker())) {
            return Poll::Ready(Err(TerminalTapeRecordingError::WorkerUnavailable));
        }
        take().map_or(Poll::Pending, Poll::Ready)
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let value = {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.abandoned = true;
            state.value.take()
        };
        drop(value);
        contain(|| drop(self.0.wake.take()));
    }
}
