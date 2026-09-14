//! Completion observation never needs another collector admission.

use super::*;
use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::task::Wake;
use std::time::{Duration, Instant};

const OBSERVATION_BOUND: Duration = Duration::from_secs(10);

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

fn counter() -> (Waker, Arc<WakeCount>) {
    let count = Arc::new(WakeCount::default());
    (Waker::from(count.clone()), count)
}

fn poll<F: Future + ?Sized>(future: Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(waker))
}

#[test]
fn completion_wait_construction_and_unpolled_drop_are_inert() {
    let scope = NativeOwnedWorkerScope::new();
    let completion = scope.completion();
    let wait = completion.wait();
    assert!(!completion.is_complete());
    assert_eq!(scope.state.status.lock().unwrap().tickets, 0);
    drop(wait);
    assert!(!completion.is_complete());
    assert_eq!(scope.state.status.lock().unwrap().tickets, 0);
    scope.close();
    assert!(completion.is_complete());
}

#[test]
fn open_empty_scope_waits_and_close_completes_without_worker_admission() {
    let scope = NativeOwnedWorkerScope::new();
    let completion = scope.completion();
    let mut wait = Box::pin(completion.wait());
    let (waker, wakes) = counter();
    assert!(with_rejected_unscoped_workers(|| poll(wait.as_mut(), &waker)).is_pending());
    assert_eq!(scope.state.status.lock().unwrap().tickets, 0);
    scope.close();
    assert!(wakes.0.load(Ordering::Acquire) > 0);
    assert_eq!(
        with_rejected_unscoped_workers(|| poll(wait.as_mut(), &waker)),
        Poll::Ready(())
    );
    assert_eq!(scope.state.status.lock().unwrap().tickets, 0);
    // An observer created after closure must not need a retained notification.
    assert_eq!(
        poll(Box::pin(completion.wait()).as_mut(), Waker::noop()),
        Poll::Ready(())
    );
}

#[test]
fn separate_waiters_all_wake_and_abandonment_cannot_steal_completion() {
    for abandoned_first in [false, true] {
        let scope = NativeOwnedWorkerScope::new();
        let ticket = scope.admit().unwrap();
        let completion = scope.completion();
        let mut abandoned = Box::pin(completion.wait());
        let mut first = Box::pin(completion.wait());
        let mut second = Box::pin(completion.wait());
        let (first_waker, first_wakes) = counter();
        let (second_waker, second_wakes) = counter();
        if abandoned_first {
            assert!(poll(abandoned.as_mut(), Waker::noop()).is_pending());
        }
        assert!(poll(first.as_mut(), &first_waker).is_pending());
        assert!(poll(second.as_mut(), &second_waker).is_pending());
        if !abandoned_first {
            assert!(poll(abandoned.as_mut(), Waker::noop()).is_pending());
        }
        drop(abandoned);
        scope.close();
        assert!(!completion.is_complete());
        assert!(poll(first.as_mut(), &first_waker).is_pending());
        assert!(poll(second.as_mut(), &second_waker).is_pending());
        first_wakes.0.store(0, Ordering::Release);
        second_wakes.0.store(0, Ordering::Release);
        drop(ticket);
        assert!(first_wakes.0.load(Ordering::Acquire) > 0);
        assert!(second_wakes.0.load(Ordering::Acquire) > 0);
        assert_eq!(poll(first.as_mut(), &first_waker), Poll::Ready(()));
        assert_eq!(poll(second.as_mut(), &second_waker), Poll::Ready(()));
    }
}

#[test]
fn closure_and_ticket_release_in_either_order_cannot_lose_completion() {
    for release_first in [false, true] {
        for register_first in [false, true] {
            let scope = NativeOwnedWorkerScope::new();
            let ticket = scope.admit().unwrap();
            let completion = scope.completion();
            let mut wait = Box::pin(completion.wait());
            let (waker, wakes) = counter();
            if register_first {
                assert!(poll(wait.as_mut(), &waker).is_pending());
            }
            if release_first {
                drop(ticket);
                assert!(!completion.is_complete());
                assert!(poll(wait.as_mut(), &waker).is_pending());
                wakes.0.store(0, Ordering::Release);
                scope.close();
            } else {
                scope.close();
                assert!(!completion.is_complete());
                assert!(poll(wait.as_mut(), &waker).is_pending());
                wakes.0.store(0, Ordering::Release);
                drop(ticket);
            }
            assert!(wakes.0.load(Ordering::Acquire) > 0);
            assert_eq!(poll(wait.as_mut(), &waker), Poll::Ready(()));
        }
    }
}

#[test]
fn registration_racing_scope_closure_rechecks_the_completed_state() {
    let scope = NativeOwnedWorkerScope::new();
    let completion = scope.completion();
    let reentrant_scope = scope.clone();
    let (waker, calls) = reentrant_waker(Callback::Clone, move || {
        reentrant_scope.close();
    });
    let mut wait = Box::pin(completion.wait());
    let first = poll(wait.as_mut(), &waker);
    assert!(calls.calls() > 0);
    assert!(completion.is_complete());
    if first.is_pending() {
        assert_eq!(poll(wait.as_mut(), Waker::noop()), Poll::Ready(()));
    } else {
        assert_eq!(first, Poll::Ready(()));
    }
}

#[test]
fn completion_waker_callbacks_can_reenter_scope_observation() {
    for callback in [Callback::Clone, Callback::Drop, Callback::Wake] {
        let scope = NativeOwnedWorkerScope::new();
        let ticket = scope.admit().unwrap();
        let completion = scope.completion();
        let state = scope.state.clone();
        let observed = completion.clone();
        let (waker, calls) = reentrant_waker(callback, move || {
            assert!(
                state.status.try_lock().is_ok(),
                "scope lock held during callback"
            );
            let _ = observed.is_complete();
        });
        let mut wait = Box::pin(completion.wait());
        assert!(poll(wait.as_mut(), &waker).is_pending());
        if callback == Callback::Drop {
            assert!(poll(wait.as_mut(), Waker::noop()).is_pending());
        }
        scope.close();
        drop(ticket);
        assert_eq!(poll(wait.as_mut(), Waker::noop()), Poll::Ready(()));
        drop(wait);
        drop(waker);
        assert!(calls.calls() > 0);
    }
}

#[test]
fn replacement_clone_and_drop_can_close_the_scope_reentrantly() {
    for callback in [Callback::Clone, Callback::Drop] {
        let scope = NativeOwnedWorkerScope::new();
        let completion = scope.completion();
        let mut wait = Box::pin(completion.wait());
        let closing = scope.clone();
        let (waker, calls) = reentrant_waker(callback, move || closing.close());
        let replacement = if callback == Callback::Clone {
            assert!(poll(wait.as_mut(), Waker::noop()).is_pending());
            poll(wait.as_mut(), &waker)
        } else {
            assert!(poll(wait.as_mut(), &waker).is_pending());
            poll(wait.as_mut(), Waker::noop())
        };
        assert!(calls.calls() > 0);
        assert!(completion.is_complete());
        if replacement.is_pending() {
            assert_eq!(poll(wait.as_mut(), Waker::noop()), Poll::Ready(()));
        }
    }
}

#[test]
fn panicking_clone_and_drop_do_not_poison_scope_or_other_waiters() {
    for callback in [Callback::Clone, Callback::Drop] {
        let scope = NativeOwnedWorkerScope::new();
        let ticket = scope.admit().unwrap();
        let completion = scope.completion();
        let mut survivor = Box::pin(completion.wait());
        let (survivor_waker, wakes) = counter();
        assert!(poll(survivor.as_mut(), &survivor_waker).is_pending());
        let mut hostile = Box::pin(completion.wait());
        let (waker, _) = reentrant_waker(callback, || panic!("completion waker callback"));
        assert!(contain(|| poll(hostile.as_mut(), &waker)).is_ok());
        if callback == Callback::Drop {
            assert!(contain(|| poll(hostile.as_mut(), Waker::noop())).is_ok());
        }
        assert!(contain(|| drop(hostile)).is_ok());
        let _ = contain(|| drop(waker));
        scope.close();
        assert!(poll(survivor.as_mut(), &survivor_waker).is_pending());
        wakes.0.store(0, Ordering::Release);
        drop(ticket);
        assert!(wakes.0.load(Ordering::Acquire) > 0);
        assert_eq!(poll(survivor.as_mut(), &survivor_waker), Poll::Ready(()));
        assert!(!scope.state.status.is_poisoned());
    }
}

#[test]
fn panicking_completion_wake_does_not_escape_close_or_ticket_destruction() {
    for release_first in [false, true] {
        let scope = NativeOwnedWorkerScope::new();
        let ticket = scope.admit().unwrap();
        let completion = scope.completion();
        let mut hostile = Box::pin(completion.wait());
        let mut survivor = Box::pin(completion.wait());
        let (waker, _) = reentrant_waker(Callback::Wake, || panic!("completion wake"));
        let (survivor_waker, wakes) = counter();
        assert!(poll(hostile.as_mut(), &waker).is_pending());
        assert!(poll(survivor.as_mut(), &survivor_waker).is_pending());
        if release_first {
            drop(ticket);
            assert!(contain(|| scope.close()).is_ok());
        } else {
            assert!(contain(|| scope.close()).is_ok());
            // A close wake may have consumed a registration; re-arm both.
            assert!(poll(hostile.as_mut(), &waker).is_pending());
            assert!(poll(survivor.as_mut(), &survivor_waker).is_pending());
            wakes.0.store(0, Ordering::Release);
            assert!(contain(|| drop(ticket)).is_ok());
        }
        assert!(wakes.0.load(Ordering::Acquire) > 0);
        assert_eq!(poll(survivor.as_mut(), &survivor_waker), Poll::Ready(()));
        assert_eq!(poll(hostile.as_mut(), Waker::noop()), Poll::Ready(()));
        assert!(!scope.state.status.is_poisoned());
    }
}

struct ReleaseOnDrop(Option<Sender<()>>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

struct ThreadCleanup {
    entered: Sender<()>,
    release: Receiver<()>,
}

impl Drop for ThreadCleanup {
    fn drop(&mut self) {
        let _ = self.entered.send(());
        let _ = self.release.recv_timeout(OBSERVATION_BOUND);
    }
}

thread_local! {
    static CLEANUP: RefCell<Option<ThreadCleanup>> = const { RefCell::new(None) };
}

struct WakeNotice(Sender<()>);

impl Wake for WakeNotice {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }
}

fn wait_bounded(
    mut wait: Pin<&mut impl Future<Output = ()>>,
    waker: &Waker,
    notices: &Receiver<()>,
) {
    let until = Instant::now() + OBSERVATION_BOUND;
    loop {
        match poll(wait.as_mut(), waker) {
            Poll::Ready(()) => return,
            Poll::Pending => notices
                .recv_timeout(until.saturating_duration_since(Instant::now()))
                .unwrap(),
        }
    }
}

#[test]
fn response_ready_does_not_complete_wait_until_collector_joins_tls_cleanup() {
    let scope = NativeOwnedWorkerScope::new();
    let completion = scope.completion();
    let (entered, observed) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let release = ReleaseOnDrop(Some(release));
    let mut response = scope.run(move || {
        CLEANUP.with(|slot| {
            *slot.borrow_mut() = Some(ThreadCleanup {
                entered,
                release: released,
            });
        });
        42
    });
    let mut wait = Box::pin(completion.wait());
    let (wake, noticed) = mpsc::channel();
    let waker = Waker::from(Arc::new(WakeNotice(wake)));
    assert!(poll(wait.as_mut(), &waker).is_pending());
    assert!(poll(response.as_mut(), Waker::noop()).is_pending());
    observed.recv_timeout(OBSERVATION_BOUND).unwrap();
    assert_eq!(poll(response.as_mut(), Waker::noop()), Poll::Ready(Ok(42)));
    scope.close();
    assert!(!completion.is_complete());
    assert!(poll(wait.as_mut(), &waker).is_pending());
    while noticed.try_recv().is_ok() {}
    drop(release);
    noticed.recv_timeout(OBSERVATION_BOUND).unwrap();
    wait_bounded(wait.as_mut(), &waker, &noticed);
    completion.wait_on_worker().unwrap();
}

#[test]
fn panicking_waiter_cannot_interrupt_real_collector_or_another_waiter() {
    let scope = NativeOwnedWorkerScope::new();
    let completion = scope.completion();
    let (entered, observed) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let release = ReleaseOnDrop(Some(release));
    let mut response = scope.run(move || {
        CLEANUP.with(|slot| {
            *slot.borrow_mut() = Some(ThreadCleanup {
                entered,
                release: released,
            });
        });
    });
    assert!(poll(response.as_mut(), Waker::noop()).is_pending());
    observed.recv_timeout(OBSERVATION_BOUND).unwrap();
    scope.close();
    let mut hostile = Box::pin(completion.wait());
    let mut survivor = Box::pin(completion.wait());
    let (hostile_waker, _) = reentrant_waker(Callback::Wake, || panic!("collector wake"));
    let (wake, noticed) = mpsc::channel();
    let survivor_waker = Waker::from(Arc::new(WakeNotice(wake)));
    assert!(poll(hostile.as_mut(), &hostile_waker).is_pending());
    assert!(poll(survivor.as_mut(), &survivor_waker).is_pending());
    drop(release);
    noticed.recv_timeout(OBSERVATION_BOUND).unwrap();
    wait_bounded(survivor.as_mut(), &survivor_waker, &noticed);
    assert_eq!(poll(hostile.as_mut(), Waker::noop()), Poll::Ready(()));
    completion.wait_on_worker().unwrap();
    assert_eq!(poll(response.as_mut(), Waker::noop()), Poll::Ready(Ok(())));
    // A subsequent actual join also needs the same process-wide collector.
    let next = NativeOwnedWorkerScope::new();
    let next_completion = next.completion();
    let mut next_wait = Box::pin(next_completion.wait());
    let (next_wake, next_noticed) = mpsc::channel();
    let next_waker = Waker::from(Arc::new(WakeNotice(next_wake)));
    assert_eq!(futures_executor::block_on(next.run(|| 7)), Ok(7));
    assert!(poll(next_wait.as_mut(), &next_waker).is_pending());
    next.close();
    wait_bounded(next_wait.as_mut(), &next_waker, &next_noticed);
    next_completion.wait_on_worker().unwrap();
}

#[test]
fn first_polled_inside_worker_can_transfer_wait_to_external_driver() {
    let scope = NativeOwnedWorkerScope::new();
    let completion = scope.completion();
    let worker_completion = completion.clone();
    let (entered, observed) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let release = ReleaseOnDrop(Some(release));
    let mut response = scope.run(move || {
        CLEANUP.with(|slot| {
            *slot.borrow_mut() = Some(ThreadCleanup {
                entered,
                release: released,
            });
        });
        let mut wait = Box::pin(async move { worker_completion.wait().await });
        assert!(poll(wait.as_mut(), Waker::noop()).is_pending());
        wait
    });
    assert!(poll(response.as_mut(), Waker::noop()).is_pending());
    observed.recv_timeout(OBSERVATION_BOUND).unwrap();
    let Poll::Ready(Ok(mut wait)) = poll(response.as_mut(), Waker::noop()) else {
        panic!("worker must publish its pending observation before TLS cleanup");
    };
    let (wake, noticed) = mpsc::channel();
    let waker = Waker::from(Arc::new(WakeNotice(wake)));
    scope.close();
    assert!(!completion.is_complete());
    assert!(poll(wait.as_mut(), &waker).is_pending());
    drop(release);
    noticed.recv_timeout(OBSERVATION_BOUND).unwrap();
    wait_bounded(wait.as_mut(), &waker, &noticed);
    completion.wait_on_worker().unwrap();
}
