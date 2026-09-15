use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::task::{Context, Waker};
use std::time::Duration;

#[test]
fn overlapping_runs_close_independently_and_reuse_resident_capacity() {
    let scope = NativeOwnedWorkerScope::new();
    let left = scope.begin_run().unwrap();
    let right = scope.begin_run().unwrap();
    let (started, started_rx) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    left.with_poll(|| {
        scope.spawn(move || {
            started.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        })
    })
    .unwrap();
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    right.close();
    assert!(right.completion().is_complete());
    left.close();
    assert!(!left.completion().is_complete());
    release.send(()).unwrap();
    left.completion().wait_on_worker().unwrap();
    let runs = (0..MAX_RUNS)
        .map(|_| scope.begin_run().unwrap())
        .collect::<Vec<_>>();
    assert!(scope.begin_run().is_err());
    let observers = runs
        .iter()
        .map(NativeOwnedWorkerRun::completion)
        .collect::<Vec<_>>();
    drop(runs);
    assert!(
        observers
            .iter()
            .all(NativeOwnedWorkerCompletion::is_complete)
    );
    assert!(scope.begin_run().is_ok());
    scope.close();
    scope.completion().wait_on_worker().unwrap();
}

#[test]
fn attribution_restores_after_panic_and_unpolled_future_never_retargets() {
    let scope = NativeOwnedWorkerScope::new();
    let left = scope.begin_run().unwrap();
    let right = scope.begin_run().unwrap();
    let future = left.with_poll(|| scope.run(|| 7));
    left.close();
    let result = right.with_poll(|| futures_executor::block_on(future));
    assert_eq!(result, Err(NativeOwnedWorkerSpawnError));
    let other_scope = NativeOwnedWorkerScope::new();
    assert!(right.with_poll(|| other_scope.spawn(|| {})).is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| right.with_poll(|| panic!("poll")))).is_err());
    assert!(RunAttribution::current().is_none());
    right.close();
    assert!(right.completion().is_complete());
    scope.close();
    scope.completion().wait_on_worker().unwrap();
}

#[test]
fn run_waits_for_actual_join_including_tls_and_rejects_self_wait() {
    struct Gate(
        std::sync::mpsc::SyncSender<()>,
        std::sync::mpsc::Receiver<()>,
        NativeOwnedWorkerCompletion,
    );
    impl Drop for Gate {
        fn drop(&mut self) {
            assert_eq!(self.2.wait_on_worker(), Err(NativeOwnedWorkerSpawnError));
            self.0.send(()).unwrap();
            self.1.recv_timeout(Duration::from_secs(10)).unwrap();
        }
    }
    thread_local! { static GATE: RefCell<Option<Gate>> = const { RefCell::new(None) }; }
    let scope = NativeOwnedWorkerScope::new();
    let run = scope.begin_run().unwrap();
    let completion = run.completion();
    let (entered, entered_rx) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    run.with_poll(|| {
        scope.spawn(move || {
            GATE.with(|slot| *slot.borrow_mut() = Some(Gate(entered, release_rx, completion)));
        })
    })
    .unwrap();
    run.close();
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(!run.completion().is_complete());
    release.send(()).unwrap();
    run.completion().wait_on_worker().unwrap();
    scope.close();
    scope.completion().wait_on_worker().unwrap();
}

#[test]
fn service_promotion_keeps_host_and_independent_failed_cleanup_snapshot() {
    let scope = NativeOwnedWorkerScope::new();
    let run = scope.begin_run().unwrap();
    let (retained, retained_rx) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    run.with_poll(|| {
        scope.spawn(move || {
            let failed = NativeOwnedWorkerScope::retain_current_cleanup().unwrap();
            let mut service = failed.clone();
            service.promote_to_service();
            promote_current_worker_to_service();
            assert!(RunAttribution::current().is_none());
            retained.send((failed, service)).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        })
    })
    .unwrap();
    let (failed, service) = retained_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    run.close();
    scope.close();
    assert!(!run.completion().is_complete());
    drop(failed);
    run.completion().wait_on_worker().unwrap();
    assert!(!scope.completion().is_complete());
    drop(service);
    release.send(()).unwrap();
    scope.completion().wait_on_worker().unwrap();
}

#[test]
fn cleanup_promotion_never_releases_ordinary_worker_before_join() {
    let scope = NativeOwnedWorkerScope::new();
    let run = scope.begin_run().unwrap();
    let (started, started_rx) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    run.with_poll(|| {
        scope.spawn(move || {
            let mut cleanup = NativeOwnedWorkerScope::retain_current_cleanup().unwrap();
            cleanup.promote_to_service();
            started.send(cleanup).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        })
    })
    .unwrap();
    let cleanup = started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    run.close();
    assert!(!run.completion().is_complete());
    release.send(()).unwrap();
    run.completion().wait_on_worker().unwrap();
    scope.close();
    assert!(!scope.completion().is_complete());
    drop(cleanup);
    scope.completion().wait_on_worker().unwrap();
}

#[test]
fn explicit_nested_scope_preserves_original_cohort_and_external_handoff_identity() {
    let host = NativeOwnedWorkerScope::new();
    let child = NativeOwnedWorkerScope::new();
    let run = host.begin_run().unwrap();
    let child_completion = child.completion();
    let (send, receive) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    let source = host.clone();
    run.with_poll(|| {
        host.spawn(move || {
            assert!(child.spawn(|| {}).is_err());
            child
                .with_inherited_run_from(&source, || {
                    child.spawn(move || {
                        let handoff = current_service_handoff().unwrap();
                        let failed_cleanup =
                            NativeOwnedWorkerScope::retain_current_cleanup().unwrap();
                        send.send((handoff, failed_cleanup)).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    })
                })
                .unwrap()
                .unwrap();
            child.close();
        })
    })
    .unwrap();
    let (handoff, failed_cleanup) = receive.recv_timeout(Duration::from_secs(10)).unwrap();
    run.close();
    handoff.promote();
    assert!(!run.completion().is_complete());
    drop(failed_cleanup);
    run.completion().wait_on_worker().unwrap();
    assert!(!child_completion.is_complete());
    release.send(()).unwrap();
    child_completion.wait_on_worker().unwrap();
    host.close();
    host.completion().wait_on_worker().unwrap();
}

#[test]
fn abandoned_handoff_and_cancelled_run_keep_actual_worker_obligation() {
    let scope = NativeOwnedWorkerScope::new();
    let run = scope.begin_run().unwrap();
    let (send, receive) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    let mut future = run.with_poll(|| {
        scope.run(move || {
            send.send(current_service_handoff().unwrap()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        })
    });
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(receive.recv_timeout(Duration::from_secs(10)).unwrap());
    drop(future);
    run.close();
    assert!(!run.completion().is_complete());
    release.send(()).unwrap();
    run.completion().wait_on_worker().unwrap();
    scope.close();
    scope.completion().wait_on_worker().unwrap();
}

#[test]
fn explicit_polled_run_overrides_unrelated_ambient_worker_for_queued_effects() {
    let scope = NativeOwnedWorkerScope::new();
    let caller = scope.begin_run().unwrap();
    let ambient = scope.begin_run().unwrap();
    let caller_completion = caller.completion();
    let (send, receive) = sync_channel(1);
    ambient
        .with_poll(|| {
            scope.spawn(move || {
                let attribution = caller.with_poll(NativeOwnedWorkerAttribution::current);
                let cleanup = attribution.admit().unwrap().unwrap();
                assert!(cleanup.ticket.owns(&caller.scope.state));
                caller.close();
                send.send(cleanup).unwrap();
            })
        })
        .unwrap();
    let cleanup = receive.recv_timeout(Duration::from_secs(10)).unwrap();
    ambient.close();
    ambient.completion().wait_on_worker().unwrap();
    assert!(!caller_completion.is_complete());
    drop(cleanup);
    caller_completion.wait_on_worker().unwrap();
    scope.close();
    scope.completion().wait_on_worker().unwrap();
}

struct JournalOwnerDrop(Arc<AtomicBool>);
impl Drop for JournalOwnerDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[test]
fn journal_keepalive_ends_after_empty_closed_cohort_not_observer_drop() {
    let scope = NativeOwnedWorkerScope::new();
    let dropped = Arc::new(AtomicBool::new(false));
    let owner = Arc::new(JournalOwnerDrop(Arc::clone(&dropped)));
    let weak = Arc::downgrade(&owner);
    let run = scope.begin_run_with_keepalive(owner.clone()).unwrap();
    let completion = run.completion();
    drop(owner);
    assert!(weak.upgrade().is_some());
    run.close();
    assert!(completion.is_complete());
    assert!(weak.upgrade().is_none());
    assert!(dropped.load(Ordering::Acquire));
    drop(run);
    assert!(completion.is_complete());
}

#[test]
fn journal_keepalive_survives_promotion_failed_cleanup_and_actual_tls_join() {
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
    let scope = NativeOwnedWorkerScope::new();
    let dropped = Arc::new(AtomicBool::new(false));
    let owner = Arc::new(JournalOwnerDrop(Arc::clone(&dropped)));
    let weak = Arc::downgrade(&owner);
    let run = scope.begin_run_with_keepalive(owner.clone()).unwrap();
    let (send, receive) = sync_channel(1);
    let (entered, entered_rx) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    run.with_poll(|| {
        scope.spawn(move || {
            let failed = NativeOwnedWorkerScope::retain_current_cleanup().unwrap();
            let handoff = current_service_handoff().unwrap();
            GATE.with(|slot| *slot.borrow_mut() = Some(Gate(entered, release_rx)));
            send.send((failed, handoff)).unwrap();
        })
    })
    .unwrap();
    drop(owner);
    let (failed, handoff) = receive.recv_timeout(Duration::from_secs(10)).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    run.close();
    handoff.promote();
    assert!(!run.completion().is_complete());
    drop(failed);
    run.completion().wait_on_worker().unwrap();
    assert!(weak.upgrade().is_some());
    assert!(!dropped.load(Ordering::Acquire));
    release.send(()).unwrap();
    scope.close();
    scope.completion().wait_on_worker().unwrap();
    assert!(weak.upgrade().is_none());
    assert!(dropped.load(Ordering::Acquire));
}
