use super::*;
use std::sync::Mutex;

#[test]
fn launch_future_rejects_closed_original_run_before_native_effects() {
    let host = NativeOwnedWorkerScope::new();
    let original = host.begin_run().unwrap();
    let later = host.begin_run().unwrap();
    let future = original.with_poll(|| {
        observed_launch().connect(
            host.clone(),
            Instant::now() + Duration::from_secs(5),
            CancellationToken::new(),
            Box::new(()),
        )
    });
    original.close();
    assert!(matches!(
        later.with_poll(|| futures_executor::block_on(future)),
        Err(McpStdioError::Capacity)
    ));
    later.close();
    host.close();
    assert!(host.completion().is_complete());
}

fn observed_launch() -> McpStdioLaunch {
    McpStdioLaunch::new(
        PathBuf::from("/unavailable-explicit-helper"),
        vec![],
        "/unavailable-explicit-server".into(),
        vec![],
        vec![],
        None,
        Arc::new(File::open(".").unwrap()),
        WireLimits::default(),
    )
    .unwrap()
}

#[test]
fn observed_launch_rejection_host_failure_and_unwind_settle_without_workers() {
    for scenario in 0..3 {
        let host = NativeOwnedWorkerScope::new();
        if scenario == 1 {
            host.close();
        }
        let observed = Arc::new(Mutex::new(None));
        let capture = observed.clone();
        let admit = Arc::new(move |completion: crate::NativeOwnedWorkerCompletion| {
            assert!(!completion.is_complete());
            *capture.lock().unwrap() = Some(completion);
            assert_ne!(scenario, 2, "deliberate observer unwind");
            scenario != 0
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            futures_executor::block_on(observed_launch().connect_observed(
                host.clone(),
                Instant::now() + Duration::from_secs(5),
                CancellationToken::new(),
                Box::new(()),
                admit,
            ))
        }));
        match result {
            Ok(result) => assert!(matches!(result, Err(McpStdioError::Capacity))),
            Err(_) => assert_eq!(scenario, 2),
        }
        assert!(observed.lock().unwrap().as_ref().unwrap().is_complete());
        host.close();
        assert!(host.completion().is_complete());
    }
}

#[test]
fn observed_launch_is_inert_and_long_deadline_admission_is_explicit() {
    let host = NativeOwnedWorkerScope::new();
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let capture = hits.clone();
    let observer: Arc<dyn Fn(crate::NativeOwnedWorkerCompletion) -> bool + Send + Sync> =
        Arc::new(move |_| {
            capture.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            false
        });
    let deadline = Instant::now() + Duration::from_millis(u64::from(u32::MAX));
    drop(observed_launch().connect_observed(
        host.clone(),
        deadline,
        CancellationToken::new(),
        Box::new(()),
        observer.clone(),
    ));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(matches!(
        futures_executor::block_on(observed_launch().connect(
            host.clone(),
            deadline,
            CancellationToken::new(),
            Box::new(())
        )),
        Err(McpStdioError::Invalid)
    ));
    assert!(matches!(
        futures_executor::block_on(observed_launch().connect_observed(
            host.clone(),
            deadline,
            CancellationToken::new(),
            Box::new(()),
            observer.clone()
        )),
        Err(McpStdioError::Capacity)
    ));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(matches!(
        futures_executor::block_on(observed_launch().connect_observed(
            host.clone(),
            deadline + Duration::from_secs(1),
            CancellationToken::new(),
            Box::new(()),
            observer.clone()
        )),
        Err(McpStdioError::Invalid)
    ));
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        futures_executor::block_on(observed_launch().connect_observed(
            host.clone(),
            deadline,
            cancel,
            Box::new(()),
            observer
        )),
        Err(McpStdioError::Cancelled)
    ));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    host.close();
    assert!(host.completion().is_complete());
}

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
