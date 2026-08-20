use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex};

/// A clonable cooperative-cancellation signal with no executor dependency.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    inner: Arc<CancellationInner>,
}

#[derive(Debug, Default)]
struct CancellationInner {
    cancelled: AtomicBool,
    waiters: Mutex<Vec<Waker>>,
}

impl CancellationToken {
    /// Creates a token in the non-cancelled state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels the token and wakes registered tasks.
    ///
    /// Returns `true` only for the call that changed the state. Later calls are
    /// harmless and return `false`.
    pub fn cancel(&self) -> bool {
        if self.inner.cancelled.swap(true, Ordering::AcqRel) {
            return false;
        }
        let waiters = {
            let mut waiters = self
                .inner
                .waiters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            core::mem::take(&mut *waiters)
        };
        for waiter in waiters {
            waiter.wake();
        }
        true
    }

    /// Returns whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Resolves once cancellation is requested.
    #[must_use]
    pub fn cancelled(&self) -> Cancelled {
        Cancelled {
            token: self.clone(),
        }
    }

    pub(crate) fn register(&self, waker: &Waker) {
        if self.is_cancelled() {
            waker.wake_by_ref();
            return;
        }
        let mut waiters = self
            .inner
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !waiters.iter().any(|existing| existing.will_wake(waker)) {
            waiters.push(waker.clone());
        }
        drop(waiters);
        if self.is_cancelled() {
            self.cancel();
            waker.wake_by_ref();
        }
    }
}

/// Future returned by [`CancellationToken::cancelled`].
#[derive(Debug)]
pub struct Cancelled {
    token: CancellationToken,
}

impl Future for Cancelled {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.token.is_cancelled() {
            Poll::Ready(())
        } else {
            self.token.register(context.waker());
            if self.token.is_cancelled() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CancellationToken;
    use futures_executor::block_on;

    #[test]
    fn cancellation_is_idempotent_and_awaitable() {
        let token = CancellationToken::new();
        let observer = token.clone();
        assert!(token.cancel());
        assert!(!token.cancel());
        block_on(observer.cancelled());
        assert!(observer.is_cancelled());
    }
}
