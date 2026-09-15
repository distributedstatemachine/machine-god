use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind};
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
