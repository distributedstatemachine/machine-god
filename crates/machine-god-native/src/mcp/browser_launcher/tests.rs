use super::*;
mod guard;
use futures_executor::block_on;
use std::{
    fs::File,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
    time::Duration,
};

struct Fixture {
    launcher: NativeMcpBrowserLauncher,
    directory: PathBuf,
    scope: NativeOwnedWorkerScope,
}
impl Fixture {
    fn new(script: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "machine-god-mcp-browser-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let program = std::fs::canonicalize("/bin/sh").unwrap();
        let executable =
            NativeBackgroundUrlExecutable::new(program.clone(), File::open(program).unwrap())
                .unwrap();
        let scope = NativeOwnedWorkerScope::new();
        let mut launcher = NativeMcpBrowserLauncher::new(
            executable,
            vec![(
                "MG_BROWSER_OUTPUT".into(),
                directory.join("output").into_os_string(),
            )],
            scope.clone(),
        )
        .unwrap();
        launcher
            .launcher
            .set_test_arguments(vec!["-c".into(), script.into(), "--".into()]);
        Self {
            launcher,
            directory,
            scope,
        }
    }
    fn launch(
        &self,
        cancel: CancellationToken,
        owner: CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'static, Result<NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLaunchError>>
    {
        self.launcher.launch(url(), cancel, owner, deadline)
    }
    fn settle(&self) {
        self.scope.close();
        self.scope.completion().wait_on_worker().unwrap();
        assert!(self.scope.completion().is_complete());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.settle();
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
fn deadline() -> Instant {
    Instant::now().checked_add(Duration::from_secs(15)).unwrap()
}
fn url() -> NativeMcpBrowserUrl {
    NativeMcpBrowserUrl::new("https://example.test/oauth?q=$HOME&state=secret;value").unwrap()
}
fn poll<T>(future: &mut BoxFuture<'static, T>) -> Poll<T> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}
fn until(mut predicate: impl FnMut() -> bool) {
    let deadline = deadline();
    while !predicate() {
        assert!(Instant::now() < deadline, "browser fixture timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn url_admission_is_bounded_exact_and_redacted() {
    let prefix = "HTTPS://Example.test/";
    let text = format!("{prefix}{}", "x".repeat(MAX_URL_BYTES - prefix.len()));
    let admitted = NativeMcpBrowserUrl::new(&text).unwrap();
    assert_eq!(admitted.as_str(), text);
    assert_eq!(format!("{admitted:?}"), "NativeMcpBrowserUrl { .. }");
    assert_eq!(
        NativeMcpBrowserUrl::new(&(text + "x")).unwrap_err(),
        NativeMcpBrowserLaunchError::InvalidUrl
    );
    for invalid in [
        "",
        "file:///tmp/x",
        "javascript:alert(1)",
        "http:example.test",
        "http:///example.test",
        "https://user:secret@example.test",
        "https://@example.test",
        "https://example.test/a\\b",
        "https://example.test/a b",
        "https://example.test/\n",
        "https://example.test/\u{2003}",
    ] {
        assert_eq!(
            NativeMcpBrowserUrl::new(invalid).unwrap_err(),
            NativeMcpBrowserLaunchError::InvalidUrl
        );
    }
    assert_eq!(
        NativeMcpBrowserLaunchError::Unavailable.to_string(),
        "MCP browser launcher unavailable"
    );
}

#[test]
fn unpolled_and_pre_cancelled_expired_requests_never_launch() {
    let fixture = Fixture::new("printf effect > \"$MG_BROWSER_OUTPUT\"");
    drop(fixture.launch(
        CancellationToken::new(),
        CancellationToken::new(),
        deadline(),
    ));
    for owner_cancel in [false, true] {
        let cancel = CancellationToken::new();
        let owner = CancellationToken::new();
        if owner_cancel {
            owner.cancel();
        } else {
            cancel.cancel();
        }
        assert_eq!(
            block_on(fixture.launch(cancel, owner, deadline())),
            Err(NativeMcpBrowserLaunchError::Cancelled)
        );
    }
    assert_eq!(
        block_on(fixture.launch(
            CancellationToken::new(),
            CancellationToken::new(),
            Instant::now()
        )),
        Err(NativeMcpBrowserLaunchError::TimedOut)
    );
    assert!(!fixture.directory.join("output").exists());
    assert_eq!(
        format!("{:?}", fixture.launcher),
        "NativeMcpBrowserLauncher { .. }"
    );
}

#[test]
fn exact_large_url_is_one_argument_with_only_captured_environment() {
    let fixture = Fixture::new(
        "test \"$#\" -eq 1 || exit 7; test -z \"$HOME\" || exit 8; printf '%s' \"$1\" > \"$MG_BROWSER_OUTPUT\"",
    );
    let text = format!(
        "https://example.test/oauth?state={}&command=$HOME;$(no-command)",
        "s".repeat(8192)
    );
    let url = NativeMcpBrowserUrl::new(&text).unwrap();
    assert_eq!(
        block_on(fixture.launcher.launch(
            url,
            CancellationToken::new(),
            CancellationToken::new(),
            deadline()
        )),
        Ok(NativeMcpBrowserLaunchOutcome::Opened)
    );
    assert_eq!(
        std::fs::read(fixture.directory.join("output")).unwrap(),
        text.as_bytes()
    );
}

#[test]
fn failed_launcher_is_not_no_effect_or_oauth_success() {
    let fixture = Fixture::new("exit 7");
    assert_eq!(
        block_on(fixture.launch(
            CancellationToken::new(),
            CancellationToken::new(),
            deadline()
        )),
        Ok(NativeMcpBrowserLaunchOutcome::LauncherFailed)
    );
}

#[test]
fn original_owner_and_caller_cancellation_survive_spawn_and_shared_admission() {
    for revoke in [false, true] {
        let fixture = Fixture::new("printf ready > \"$MG_BROWSER_OUTPUT\"; exec /bin/sleep 30");
        let cancellation = CancellationToken::new();
        let owner = CancellationToken::new();
        let mut operation = fixture.launch(cancellation.clone(), owner.clone(), deadline());
        assert!(poll(&mut operation).is_pending());
        until(|| fixture.directory.join("output").exists());
        assert_eq!(
            block_on(fixture.launcher.clone().launch(
                url(),
                CancellationToken::new(),
                CancellationToken::new(),
                deadline()
            )),
            Err(NativeMcpBrowserLaunchError::Busy)
        );
        if revoke {
            owner.cancel();
        } else {
            cancellation.cancel();
        }
        assert_eq!(
            block_on(operation),
            Ok(NativeMcpBrowserLaunchOutcome::Indeterminate)
        );
        fixture.settle();
    }
}

#[test]
fn supplied_deadline_and_abandonment_retain_owned_cleanup() {
    let fixture = Fixture::new("printf ready > \"$MG_BROWSER_OUTPUT\"; exec /bin/sleep 30");
    let caller = CancellationToken::new();
    let expires = Instant::now().checked_add(Duration::from_secs(2)).unwrap();
    let mut operation = fixture.launch(caller.clone(), CancellationToken::new(), expires);
    assert!(poll(&mut operation).is_pending());
    until(|| fixture.directory.join("output").exists());
    assert_eq!(
        block_on(operation),
        Ok(NativeMcpBrowserLaunchOutcome::Indeterminate)
    );
    fixture.settle();
    assert!(!caller.is_cancelled());

    let fixture = Fixture::new("printf ready > \"$MG_BROWSER_OUTPUT\"; exec /bin/sleep 30");
    let mut operation = fixture.launch(caller.clone(), CancellationToken::new(), deadline());
    assert!(poll(&mut operation).is_pending());
    until(|| fixture.directory.join("output").exists());
    drop(operation);
    fixture.settle();
    assert!(!caller.is_cancelled());
}
