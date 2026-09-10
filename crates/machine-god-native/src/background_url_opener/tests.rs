use super::{
    BackgroundServerUrl, BoxFuture, CancellationToken, Duration, File, Instant,
    NativeBackgroundOpenError as Error, NativeBackgroundOpenOutcome as Outcome,
    NativeBackgroundUrlOpener, NativeOwnedWorkerScope, Ordering, PathBuf,
};
use crate::background_commands::url::detect_server_url;
use futures_executor::block_on;
use std::sync::{Arc, atomic::AtomicU64};
use std::task::{Context, Poll, Waker};

struct Fixture {
    opener: NativeBackgroundUrlOpener,
    directory: PathBuf,
    scope: NativeOwnedWorkerScope,
}

impl Fixture {
    fn new(script: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "machine-god-background-open-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let program = std::fs::canonicalize("/bin/sh").unwrap();
        let scope = NativeOwnedWorkerScope::new();
        let mut opener = NativeBackgroundUrlOpener::new(
            program.clone(),
            File::open(&program).unwrap(),
            vec![(
                "MG_BACKGROUND_OPEN_OUTPUT".into(),
                directory.join("output").into_os_string(),
            )],
            scope.clone(),
        )
        .unwrap();
        Arc::get_mut(&mut opener.inner).unwrap().arguments =
            vec!["-c".into(), script.into(), "--".into()];
        Self {
            opener,
            directory,
            scope,
        }
    }

    fn open(&self) -> BoxFuture<'static, Result<Outcome, Error>> {
        self.opener
            .open(url(), CancellationToken::new(), CancellationToken::new())
    }

    fn settle(&self) {
        self.scope.close();
        self.scope.completion().wait_on_worker().unwrap();
        assert!(self.scope.completion().is_complete());
        assert!(!self.opener.inner.active.load(Ordering::Acquire));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.settle();
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn url() -> BackgroundServerUrl {
    detect_server_url(b"http://localhost:3000/path?q=one&other=two")
        .unwrap()
        .unwrap()
}

fn poll<T>(future: &mut BoxFuture<'static, T>) -> Poll<T> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}

fn until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !predicate() {
        assert!(Instant::now() < deadline, "URL launcher fixture timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn unpolled_construction_and_clone_do_not_launch_or_admit() {
    let fixture = Fixture::new("exit 9");
    let future = fixture.open();
    let clone = fixture.opener.clone();
    assert!(!fixture.opener.inner.active.load(Ordering::Acquire));
    assert!(!fixture.directory.join("output").exists());
    drop((future, clone));
    fixture.settle();
}

#[test]
fn exact_single_url_argument_without_shell_interpolation_and_explicit_environment() {
    let fixture = Fixture::new(
        "test \"$#\" -eq 1 || exit 7; test -z \"$HOME\" || exit 8; \
         printf '%s' \"$1\" > \"$MG_BACKGROUND_OPEN_OUTPUT\"",
    );
    assert_eq!(block_on(fixture.open()), Ok(Outcome::Opened));
    assert_eq!(
        std::fs::read(fixture.directory.join("output")).unwrap(),
        url().as_str().as_bytes()
    );
    assert!(!fixture.opener.inner.active.load(Ordering::Acquire));
}

#[test]
fn launcher_exit_failure_is_not_reported_as_no_effect() {
    let fixture = Fixture::new("exit 7");
    assert_eq!(block_on(fixture.open()), Ok(Outcome::LauncherFailed));
}

#[test]
fn cancellation_or_revocation_before_poll_never_admits() {
    let fixture = Fixture::new("exit 9");
    for revoke in [false, true] {
        let cancellation = CancellationToken::new();
        let revoked = CancellationToken::new();
        if revoke {
            revoked.cancel();
        } else {
            cancellation.cancel();
        }
        assert_eq!(
            block_on(fixture.opener.open(url(), cancellation, revoked)),
            Err(Error::Cancelled)
        );
        assert!(!fixture.opener.inner.active.load(Ordering::Acquire));
    }
}

#[test]
fn committed_cancel_is_indeterminate_and_shared_admission_survives_until_cleanup() {
    let fixture = Fixture::new("printf ready > \"$MG_BACKGROUND_OPEN_OUTPUT\"; exec /bin/sleep 30");
    let cancelled = CancellationToken::new();
    let mut first = fixture
        .opener
        .open(url(), cancelled.clone(), CancellationToken::new());
    assert!(poll(&mut first).is_pending());
    until(|| fixture.directory.join("output").exists());
    assert_eq!(block_on(fixture.open()), Err(Error::Busy));
    cancelled.cancel();
    assert_eq!(block_on(first), Ok(Outcome::Indeterminate));
    fixture.settle();
}

#[test]
fn abandoned_request_does_not_cancel_the_callers_token_or_abandon_cleanup() {
    let fixture = Fixture::new("printf ready > \"$MG_BACKGROUND_OPEN_OUTPUT\"; exec /bin/sleep 30");
    let cancellation = CancellationToken::new();
    let mut first = fixture
        .opener
        .open(url(), cancellation.clone(), CancellationToken::new());
    assert!(poll(&mut first).is_pending());
    until(|| fixture.directory.join("output").exists());
    drop(first);
    fixture.settle();
    assert!(!cancellation.is_cancelled());
}

#[test]
fn mismatched_retained_executable_fails_before_launch() {
    let mut fixture = Fixture::new("exit 9");
    Arc::get_mut(&mut fixture.opener.inner).unwrap().executable =
        File::open(&fixture.directory).unwrap();
    assert_eq!(block_on(fixture.open()), Err(Error::Unavailable));
    assert!(!fixture.directory.join("output").exists());
}

#[test]
fn authority_validation_and_debug_are_inert_and_redacted() {
    let fixture = Fixture::new("exit 9");
    assert_eq!(
        format!("{:?}", fixture.opener),
        "NativeBackgroundUrlOpener { .. }"
    );
    let invalid = NativeBackgroundUrlOpener::new(
        "relative-browser".into(),
        File::open(&fixture.directory).unwrap(),
        Vec::new(),
        fixture.scope.clone(),
    );
    assert_eq!(invalid.unwrap_err(), Error::InvalidAuthority);
    assert_eq!(
        Error::Unavailable.to_string(),
        "background URL opener unavailable"
    );
}
