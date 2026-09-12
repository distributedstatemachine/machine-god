use super::*;
use crate::mcp::browser_launcher::{
    NativeMcpBrowserLaunchError as Error, NativeMcpBrowserLaunchOutcome as Outcome,
    NativeMcpBrowserUrl,
};
use futures_executor::block_on;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn opener(scope: &NativeOwnedWorkerScope) -> NativeBackgroundUrlOpener {
    // Deliberately not a launcher installation. Conversion must not inspect it,
    // and every test rejects before an actual launcher worker could start.
    NativeBackgroundUrlOpener::new(
        "/unavailable-mcp-browser-composition-fixture".into(),
        File::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap(),
        vec![("MG_BROWSER_CAPTURED".into(), "selected".into())],
        scope.clone(),
    )
    .unwrap()
}

fn url() -> NativeMcpBrowserUrl {
    NativeMcpBrowserUrl::new("https://example.test/observed?state=private").unwrap()
}

fn deadline() -> Instant {
    Instant::now().checked_add(Duration::from_secs(30)).unwrap()
}

#[test]
fn conversion_retains_the_exact_allocation_and_unpolled_requests_stay_inert() {
    let scope = NativeOwnedWorkerScope::new();
    let background = opener(&scope);
    assert_eq!(Arc::strong_count(&background.launcher.inner), 1);
    let mcp = background.mcp_launcher();
    let second = background.mcp_launcher();
    assert_eq!(Arc::strong_count(&background.launcher.inner), 3);
    let original = CancellationToken::new();
    drop(mcp.launch(
        url(),
        original.clone(),
        CancellationToken::new(),
        deadline(),
    ));
    assert_eq!(Arc::strong_count(&background.launcher.inner), 3);
    assert!(!background.launcher.inner.active.load(Ordering::Acquire));
    assert!(!original.is_cancelled());
    assert_eq!(format!("{mcp:?}"), "NativeMcpBrowserLauncher { .. }");
    drop((mcp, second));
    assert_eq!(Arc::strong_count(&background.launcher.inner), 1);
    scope.close();
    assert!(scope.completion().is_complete());
}

#[test]
fn converted_launcher_shares_admission_and_preserves_original_cancellation() {
    struct HeldAdmission(Arc<AtomicBool>);
    impl Drop for HeldAdmission {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let scope = NativeOwnedWorkerScope::new();
    let background = opener(&scope);
    let mcp = background.mcp_launcher();
    background
        .launcher
        .inner
        .active
        .store(true, Ordering::Release);
    let held = HeldAdmission(Arc::clone(&background.launcher.inner.active));
    assert_eq!(
        block_on(mcp.launch(
            url(),
            CancellationToken::new(),
            CancellationToken::new(),
            deadline()
        )),
        Err(Error::Busy)
    );
    for revoke in [false, true] {
        let caller = CancellationToken::new();
        let owner = CancellationToken::new();
        if revoke {
            owner.cancel();
        } else {
            caller.cancel();
        }
        assert_eq!(
            block_on(mcp.launch(url(), caller, owner, deadline())),
            Err(Error::Cancelled)
        );
    }
    drop(held);
    scope.close();
    assert!(scope.completion().is_complete());
}

#[test]
fn converted_launcher_keeps_the_original_closed_worker_scope() {
    let scope = NativeOwnedWorkerScope::new();
    let background = opener(&scope);
    let mcp = background.mcp_launcher();
    scope.close();
    assert!(scope.completion().is_complete());
    assert_eq!(
        block_on(mcp.launch(
            url(),
            CancellationToken::new(),
            CancellationToken::new(),
            deadline()
        )),
        Ok(Outcome::Indeterminate)
    );
    assert!(!background.launcher.inner.active.load(Ordering::Acquire));
    assert!(scope.completion().is_complete());
}
