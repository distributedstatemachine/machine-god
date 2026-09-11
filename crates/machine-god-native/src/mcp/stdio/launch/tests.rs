use super::*;
use std::sync::Mutex;

#[test]
fn parent_completion_waits_for_retained_nested_connection_cleanup() {
    let parent = NativeOwnedWorkerScope::new();
    let child = NativeOwnedWorkerScope::new();
    let completion = child.completion();
    let hold = Arc::new(Mutex::new(None));
    let worker_hold = hold.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    child
        .spawn(move || {
            *worker_hold.lock().unwrap() = NativeOwnedWorkerScope::retain_current_cleanup();
            ready_tx.send(()).unwrap();
        })
        .unwrap();
    let shared = Arc::new(Shared::new(WireLimits::default(), CancellationToken::new()));
    let startup = Arc::new(Response::new());
    parent
        .spawn(move || {
            let _owner = OwnerWait {
                scope: child,
                shared,
                startup,
            };
        })
        .unwrap();
    parent.close();
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!completion.is_complete());
    assert!(!parent.completion().is_complete());
    // This simulates the existing reaper retaining a nested inventory cleanup
    // ticket after the original child worker/direct child have both finished.
    drop(hold.lock().unwrap().take());
    parent.completion().wait_on_worker().unwrap();
    assert!(completion.is_complete());
}
