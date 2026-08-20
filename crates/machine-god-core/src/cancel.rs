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
        let mut incoming = Some(waker.clone());
        let superseded = {
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
                Some(existing) if existing.will_wake(waker) => None,
                Some(existing) => Some(core::mem::replace(
                    existing,
                    incoming.take().expect("incoming waker is available"),
                )),
                None => {
                    registry
                        .waiters
                        .insert(id, incoming.take().expect("incoming waker is available"));
                    None
                }
            }
        };
        drop(superseded);
        drop(incoming);
        if self.is_cancelled() {
            self.deregister(waiter_id);
            waker.wake_by_ref();
        }
    }

    pub(crate) fn deregister(&self, waiter_id: &mut Option<WaiterId>) {
        let Some(id) = waiter_id.take() else {
            return;
        };
        let removed = {
            let mut registry = self
                .inner
                .waiters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registry.waiters.remove(&id)
        };
        drop(removed);
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
    use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
    use std::sync::{Arc, mpsc};
    use std::task::Wake;
    use std::time::Duration;

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

    fn assert_completes(operation: impl FnOnce() + Send + 'static) {
        let (completed_tx, completed_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            operation();
            completed_tx.send(()).unwrap();
        });
        completed_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reentrant waker operation deadlocked");
        worker.join().unwrap();
    }

    #[test]
    fn waker_clone_reentrancy_occurs_before_registry_lock() {
        let token = CancellationToken::new();
        let reentrant_token = token.clone();
        let (waker, state) = reentrant_waker(Callback::Clone, move || {
            let _ = reentrant_token.waiter_count();
        });

        assert_completes(move || {
            let mut waiter_id = None;
            token.register(&mut waiter_id, &waker);
            token.deregister(&mut waiter_id);
        });
        assert_eq!(state.calls(), 1);
    }

    #[test]
    fn replacement_drops_superseded_waker_after_registry_unlock() {
        let token = CancellationToken::new();
        let reentrant_token = token.clone();
        let (waker, state) = reentrant_waker(Callback::Drop, move || {
            let _ = reentrant_token.waiter_count();
        });
        let mut waiter_id = None;
        token.register(&mut waiter_id, &waker);
        drop(waker);
        let calls_before_replacement = state.calls();

        assert_completes(move || {
            let replacement = futures_util::task::noop_waker();
            token.register(&mut waiter_id, &replacement);
            token.deregister(&mut waiter_id);
        });
        assert_eq!(state.calls(), calls_before_replacement + 1);
    }

    #[test]
    fn deregistration_drops_removed_waker_after_registry_unlock() {
        let token = CancellationToken::new();
        let reentrant_token = token.clone();
        let (waker, state) = reentrant_waker(Callback::Drop, move || {
            let _ = reentrant_token.waiter_count();
        });
        let mut waiter_id = None;
        token.register(&mut waiter_id, &waker);
        drop(waker);
        let calls_before_deregister = state.calls();

        assert_completes(move || token.deregister(&mut waiter_id));
        assert_eq!(state.calls(), calls_before_deregister + 1);
    }

    #[test]
    fn cancellation_wakes_reentrant_waker_after_registry_unlock() {
        let token = CancellationToken::new();
        let reentrant_token = token.clone();
        let (waker, state) = reentrant_waker(Callback::Wake, move || {
            let _ = reentrant_token.waiter_count();
        });
        let mut waiter_id = None;
        token.register(&mut waiter_id, &waker);
        drop(waker);

        assert_completes(move || assert!(token.cancel()));
        assert_eq!(state.calls(), 1);
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
