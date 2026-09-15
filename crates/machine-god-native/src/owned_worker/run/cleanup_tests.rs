use super::*;
use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

fn cleanup(host: &NativeOwnedWorkerScope) -> NativeOwnedWorkerRun {
    host.begin_cleanup_run_with_keepalive(Arc::new(())).unwrap()
}

fn saturated(host: &NativeOwnedWorkerScope) -> Vec<NativeOwnedWorkerRun> {
    (0..MAX_CLEANUP_RUNS).map(|_| cleanup(host)).collect()
}

fn poll<F: Future + ?Sized>(future: Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(waker))
}

#[derive(Default)]
struct Count(AtomicUsize);
impl Wake for Count {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

#[test]
fn ordinary_and_cleanup_reserves_are_separately_bounded_and_reusable() {
    let host = NativeOwnedWorkerScope::new();
    let ordinary: Vec<_> = (0..MAX_RUNS).map(|_| host.begin_run().unwrap()).collect();
    assert!(host.begin_run().is_err());
    let mut cleanups = saturated(&host);
    assert!(host.begin_cleanup_run_with_keepalive(Arc::new(())).is_err());
    assert!(host.begin_run().is_err());
    let completed = cleanups.pop().unwrap();
    let observer = completed.completion();
    completed.close();
    assert!(observer.is_complete());
    cleanups.push(cleanup(&host));
    assert!(
        host.begin_run().is_err(),
        "ordinary work cannot borrow settlement capacity"
    );
    drop(ordinary);
    assert!(host.begin_run().is_ok());
    assert!(host.begin_cleanup_run_with_keepalive(Arc::new(())).is_err());
    drop(cleanups);
    assert!(observer.is_complete());
    assert!(host.begin_cleanup_run_with_keepalive(Arc::new(())).is_ok());
    host.close();
}

#[test]
fn cleanup_capacity_observers_are_inert_and_observation_is_not_a_reservation() {
    let host = NativeOwnedWorkerScope::new();
    drop(host.wait_for_cleanup_capacity());
    assert_eq!(host.state.status.lock().unwrap().cleanup_runs, 0);
    let mut held = saturated(&host);
    let count = Arc::new(Count::default());
    let waker = Waker::from(count.clone());
    let mut first = host.wait_for_cleanup_capacity();
    let mut second = host.wait_for_cleanup_capacity();
    assert!(poll(first.as_mut(), &waker).is_pending());
    assert!(poll(second.as_mut(), &waker).is_pending());
    drop(held.pop());
    assert!(count.0.load(Ordering::Acquire) >= 2);
    assert_eq!(poll(first.as_mut(), &waker), Poll::Ready(Ok(())));
    held.push(cleanup(&host));
    assert!(poll(second.as_mut(), &waker).is_pending());
    drop(held.pop());
    assert_eq!(poll(second.as_mut(), &waker), Poll::Ready(Ok(())));
    drop(held);
    host.close();
}

#[test]
fn cleanup_capacity_close_wakes_even_with_unsettled_host_tickets() {
    let host = NativeOwnedWorkerScope::new();
    let held = saturated(&host);
    let ticket = host.admit().unwrap();
    let count = Arc::new(Count::default());
    let waker = Waker::from(count.clone());
    let mut wait = host.wait_for_cleanup_capacity();
    assert!(poll(wait.as_mut(), &waker).is_pending());
    host.close();
    assert!(count.0.load(Ordering::Acquire) > 0);
    assert_eq!(
        poll(wait.as_mut(), &waker),
        Poll::Ready(Err(NativeOwnedWorkerSpawnError))
    );
    assert!(!host.completion().is_complete());
    assert!(host.begin_cleanup_run_with_keepalive(Arc::new(())).is_err());
    assert_eq!(
        futures_executor::block_on(host.wait_for_cleanup_capacity()),
        Err(NativeOwnedWorkerSpawnError)
    );
    drop(ticket);
    drop(held);
    host.completion().wait_on_worker().unwrap();
}

#[test]
fn cleanup_capacity_refund_during_waker_registration_cannot_be_lost() {
    let host = NativeOwnedWorkerScope::new();
    let mut held = saturated(&host);
    let victim = held.pop().unwrap();
    let (waker, calls) = reentrant_waker(Callback::Clone, move || victim.close());
    let mut wait = host.wait_for_cleanup_capacity();
    assert_eq!(poll(wait.as_mut(), &waker), Poll::Ready(Ok(())));
    assert!(calls.calls() > 0);
    drop(held);
    host.close();
}

#[test]
fn cleanup_refund_and_keepalive_wait_for_actual_collector_tls() {
    struct Owner(Arc<AtomicBool>);
    impl Drop for Owner {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    struct Gate(
        std::sync::mpsc::SyncSender<()>,
        std::sync::mpsc::Receiver<()>,
    );
    impl Drop for Gate {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
            self.1.recv_timeout(Duration::from_secs(10)).unwrap();
        }
    }
    thread_local! { static GATE: RefCell<Option<Gate>> = const { RefCell::new(None) }; }
    let host = NativeOwnedWorkerScope::new();
    let held: Vec<_> = (1..MAX_CLEANUP_RUNS).map(|_| cleanup(&host)).collect();
    let dropped = Arc::new(AtomicBool::new(false));
    let owner = Arc::new(Owner(dropped.clone()));
    let weak = Arc::downgrade(&owner);
    let run = host.begin_cleanup_run_with_keepalive(owner).unwrap();
    let observer = run.completion();
    let (entered, entered_rx) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    run.with_poll(|| {
        host.spawn(move || {
            GATE.with(|slot| *slot.borrow_mut() = Some(Gate(entered, release_rx)));
        })
    })
    .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    run.close();
    assert!(!observer.is_complete());
    assert!(weak.upgrade().is_some());
    let mut capacity = host.wait_for_cleanup_capacity();
    assert!(poll(capacity.as_mut(), Waker::noop()).is_pending());
    release.send(()).unwrap();
    futures_executor::block_on(capacity).unwrap();
    assert!(observer.is_complete());
    assert!(dropped.load(Ordering::Acquire));
    assert!(weak.upgrade().is_none());
    drop(held);
    host.close();
    host.completion().wait_on_worker().unwrap();
}
