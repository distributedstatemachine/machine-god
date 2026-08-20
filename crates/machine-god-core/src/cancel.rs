use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll, Waker};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

type WaiterId = u64;

/// A clonable cooperative-cancellation signal with no executor dependency.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    inner: Arc<CancellationInner>,
}

#[derive(Debug, Default)]
struct CancellationInner {
    cancelled: AtomicBool,
    waiters: Mutex<WaiterRegistry>,
}

#[derive(Debug, Default)]
struct WaiterRegistry {
    next_id: WaiterId,
    // The registry has at most one entry per live waiting future/stream. A
    // repeated poll updates that keyed entry rather than appending another.
    waiters: BTreeMap<WaiterId, Waker>,
}

impl WaiterRegistry {
    fn vacant_id(&mut self) -> WaiterId {
        let start = self.next_id;
        loop {
            let candidate = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if !self.waiters.contains_key(&candidate) {
                return candidate;
            }
            assert!(
                self.next_id != start,
                "cancellation waiter ID space exhausted"
            );
        }
    }
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
            let mut registry = self
                .inner
                .waiters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            core::mem::take(&mut registry.waiters)
        };
        for waiter in waiters.into_values() {
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
            waiter_id: None,
        }
    }

    pub(crate) fn register(&self, waiter_id: &mut Option<WaiterId>, waker: &Waker) {
        if self.is_cancelled() {
            self.deregister(waiter_id);
            waker.wake_by_ref();
            return;
        }
        let mut registry = self
            .inner
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = if let Some(id) = *waiter_id {
            id
        } else {
            let id = registry.vacant_id();
            *waiter_id = Some(id);
            id
        };
        match registry.waiters.get_mut(&id) {
            Some(existing) if existing.will_wake(waker) => {}
            Some(existing) => existing.clone_from(waker),
            None => {
                registry.waiters.insert(id, waker.clone());
            }
        }
        drop(registry);
        if self.is_cancelled() {
            self.deregister(waiter_id);
            waker.wake_by_ref();
        }
    }

    pub(crate) fn deregister(&self, waiter_id: &mut Option<WaiterId>) {
        let Some(id) = waiter_id.take() else {
            return;
        };
        self.inner
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .waiters
            .remove(&id);
    }

    #[cfg(test)]
    fn waiter_count(&self) -> usize {
        self.inner
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .waiters
            .len()
    }
}

/// Future returned by [`CancellationToken::cancelled`].
#[derive(Debug)]
pub struct Cancelled {
    token: CancellationToken,
    waiter_id: Option<WaiterId>,
}

impl Future for Cancelled {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.token.is_cancelled() {
            let mut waiter_id = self.waiter_id.take();
            self.token.deregister(&mut waiter_id);
            Poll::Ready(())
        } else {
            let token = self.token.clone();
            token.register(&mut self.waiter_id, context.waker());
            if self.token.is_cancelled() {
                let mut waiter_id = self.waiter_id.take();
                self.token.deregister(&mut waiter_id);
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }
}

impl Drop for Cancelled {
    fn drop(&mut self) {
        self.token.deregister(&mut self.waiter_id);
    }
}

#[cfg(test)]
mod tests {
    use super::CancellationToken;
    use core::future::Future;
    use core::pin::Pin;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::task::{Context, Poll, Waker};
    use futures_executor::block_on;
    use std::sync::Arc;
    use std::task::Wake;

    #[test]
    fn cancellation_is_idempotent_and_awaitable() {
        let token = CancellationToken::new();
        let observer = token.clone();
        assert!(token.cancel());
        assert!(!token.cancel());
        block_on(observer.cancelled());
        assert!(observer.is_cancelled());
    }

    #[derive(Debug, Default)]
    struct WakeCounter(AtomicUsize);

    impl Wake for WakeCounter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn dropped_and_repolled_waiters_do_not_accumulate_or_wake() {
        let token = CancellationToken::new();
        let dropped_counter = Arc::new(WakeCounter::default());
        let retained_counter = Arc::new(WakeCounter::default());
        let dropped_waker = Waker::from(Arc::clone(&dropped_counter));
        let retained_waker = Waker::from(Arc::clone(&retained_counter));
        let mut dropped = Box::pin(token.cancelled());
        let mut retained = Box::pin(token.cancelled());

        for _ in 0..100 {
            assert!(matches!(
                Pin::as_mut(&mut dropped).poll(&mut Context::from_waker(&dropped_waker)),
                Poll::Pending
            ));
        }
        assert_eq!(token.waiter_count(), 1);
        assert!(matches!(
            Pin::as_mut(&mut retained).poll(&mut Context::from_waker(&retained_waker)),
            Poll::Pending
        ));
        assert_eq!(token.waiter_count(), 2);

        drop(dropped);
        assert_eq!(token.waiter_count(), 1);
        assert!(token.cancel());
        assert_eq!(token.waiter_count(), 0);
        assert_eq!(dropped_counter.0.load(Ordering::Relaxed), 0);
        assert_eq!(retained_counter.0.load(Ordering::Relaxed), 1);
        assert!(matches!(
            Pin::as_mut(&mut retained).poll(&mut Context::from_waker(&retained_waker)),
            Poll::Ready(())
        ));
    }
}
