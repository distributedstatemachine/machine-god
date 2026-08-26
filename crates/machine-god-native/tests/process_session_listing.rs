use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_native::{
    NativeEnvironment, NativeSessionListingErrorKind, list_native_sessions, list_process_sessions,
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_core::{SessionId, SessionIncarnationId, SessionRecord, SessionStore};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_native::FileSessionStore;

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let base = std::env::temp_dir().join("machine-god-process-session-listing-tests");
        std::fs::create_dir_all(&base).unwrap();
        loop {
            let identifier = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("{}-{label}-{identifier}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create {}: {error}", path.display()),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("failed to remove {}: {error}", self.0.display());
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(Pin::as_mut(&mut future), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("native process session listing unexpectedly remained pending"),
    }
}

fn environment(xdg_state_home: Option<&Path>, home: Option<&Path>) -> NativeEnvironment {
    NativeEnvironment::new(
        Some(OsString::from("relative-config-must-not-be-read")),
        xdg_state_home.map(Path::as_os_str).map(OsString::from),
        home.map(Path::as_os_str).map(OsString::from),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn private_directory(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn save_session(state_base: &Path, value: &str) {
    let root = state_base.join("machine-god");
    private_directory(&root);
    let store = FileSessionStore::open(&root).unwrap();
    let record = SessionRecord::empty(
        SessionId::new(value).unwrap(),
        SessionIncarnationId::new(format!("incarnation-{value}")).unwrap(),
    );
    ready(store.save(record, None)).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn values(list: &machine_god_native::NativeSessionList) -> Vec<&str> {
    list.session_ids().iter().map(SessionId::as_str).collect()
}

#[test]
fn process_entrypoint_is_future_owned_and_can_be_dropped_unpolled() {
    let future = list_process_sessions();
    drop(future);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn unpolled_and_missing_root_listing_is_effect_free_and_empty() {
    let temporary = TestDirectory::new("missing-root");
    let state_base = temporary.path().join("missing-state");
    let home = temporary.path().join("missing-home");

    let future = list_native_sessions(environment(Some(&state_base), Some(&home)));
    assert!(!state_base.exists());
    assert!(!home.exists());
    drop(future);
    assert!(!state_base.exists());
    assert!(!home.exists());

    let listed = ready(list_native_sessions(environment(
        Some(&state_base),
        Some(&home),
    )))
    .unwrap();
    assert!(listed.session_ids().is_empty());
    assert!(!listed.truncated());
    assert!(!state_base.exists());
    assert!(!home.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn xdg_state_precedes_home_and_listing_is_sorted() {
    let temporary = TestDirectory::new("xdg-precedence");
    let xdg_state = temporary.path().join("xdg-state");
    let home = temporary.path().join("home");
    save_session(&xdg_state, "zeta-session");
    save_session(&xdg_state, "alpha-session");
    save_session(&home.join(".local/state"), "home-must-not-appear");

    let listed = ready(list_native_sessions(environment(
        Some(&xdg_state),
        Some(&home),
    )))
    .unwrap();
    assert_eq!(values(&listed), ["alpha-session", "zeta-session"]);
    assert!(!listed.truncated());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn home_fallback_uses_dot_local_state() {
    let temporary = TestDirectory::new("home-fallback");
    let home = temporary.path().join("home");
    save_session(&home.join(".local/state"), "fallback-session");

    let listed = ready(list_native_sessions(environment(None, Some(&home)))).unwrap();
    assert_eq!(values(&listed), ["fallback-session"]);
    assert!(!listed.truncated());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn successful_listing_only_creates_a_private_lock_sidecar() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = TestDirectory::new("lock-sidecar");
    let state_base = temporary.path().join("state");
    let state_root = state_base.join("machine-god");
    save_session(&state_base, "lock-session");
    let record_path = std::fs::read_dir(&state_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let lock_path = std::fs::read_dir(&state_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "lock")
        })
        .unwrap();
    std::fs::remove_file(lock_path).unwrap();
    let before = std::fs::read(&record_path).unwrap();

    let listed = ready(list_native_sessions(environment(Some(&state_base), None))).unwrap();
    assert_eq!(values(&listed), ["lock-session"]);
    assert_eq!(std::fs::read(&record_path).unwrap(), before);
    let mut entries = std::fs::read_dir(&state_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(entries.len(), 2);
    let lock = entries
        .iter()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "lock")
        })
        .unwrap();
    assert_eq!(
        std::fs::metadata(lock).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn result_bound_sets_truncated_without_partial_error() {
    let temporary = TestDirectory::new("result-bound");
    let state_base = temporary.path().join("state");
    for index in 0..101 {
        save_session(&state_base, &format!("bounded-session-{index:03}"));
    }

    let listed = ready(list_native_sessions(environment(Some(&state_base), None))).unwrap();
    assert_eq!(listed.session_ids().len(), 100);
    assert!(listed.truncated());
    assert!(
        listed
            .session_ids()
            .windows(2)
            .all(|pair| pair[0].as_str() < pair[1].as_str())
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn canonical_corruption_is_a_fixed_redacted_error() {
    let temporary = TestDirectory::new("corrupt");
    let state_base = temporary.path().join("state-PATH_SECRET");
    let state_root = state_base.join("machine-god");
    save_session(&state_base, "corrupt-session-secret");
    let record_path = std::fs::read_dir(&state_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    std::fs::write(&record_path, b"CORRUPT_RECORD_SECRET:not-json").unwrap();

    let error = ready(list_native_sessions(environment(Some(&state_base), None))).unwrap_err();
    assert_eq!(error.kind(), NativeSessionListingErrorKind::Corrupt);
    assert_eq!(error.kind().as_str(), "corrupt");
    let presentation = format!("{error:?} {error}");
    for forbidden in [
        "PATH_SECRET",
        "CORRUPT_RECORD_SECRET",
        "corrupt-session-secret",
    ] {
        assert!(!presentation.contains(forbidden));
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn invalid_unsafe_symlink_and_wrong_kind_roots_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let temporary = TestDirectory::new("invalid-roots");
    let target = temporary.path().join("target");
    private_directory(&target);

    let wrong_kind_base = temporary.path().join("wrong-kind-PATH_SECRET");
    std::fs::create_dir(&wrong_kind_base).unwrap();
    std::fs::write(
        wrong_kind_base.join("machine-god"),
        b"WRONG_KIND_CONTENT_SECRET",
    )
    .unwrap();

    let symlink_base = temporary.path().join("symlink-PATH_SECRET");
    std::fs::create_dir(&symlink_base).unwrap();
    symlink(&target, symlink_base.join("machine-god")).unwrap();

    let unsafe_base = temporary.path().join("unsafe-PATH_SECRET");
    let unsafe_root = unsafe_base.join("machine-god");
    std::fs::create_dir_all(&unsafe_root).unwrap();
    std::fs::set_permissions(&unsafe_root, std::fs::Permissions::from_mode(0o755)).unwrap();

    let relative = NativeEnvironment::new(
        None,
        Some(OsString::from("relative-state-PATH_SECRET")),
        Some(temporary.path().as_os_str().to_owned()),
    );
    for (environment, expected_kind) in [
        (
            environment(Some(&wrong_kind_base), None),
            NativeSessionListingErrorKind::UnsafeStateRoot,
        ),
        (
            environment(Some(&symlink_base), None),
            NativeSessionListingErrorKind::UnsafeStateRoot,
        ),
        (
            environment(Some(&unsafe_base), None),
            NativeSessionListingErrorKind::UnsafeStateRoot,
        ),
        (relative, NativeSessionListingErrorKind::InvalidEnvironment),
    ] {
        let error = ready(list_native_sessions(environment)).unwrap_err();
        assert_eq!(error.kind(), expected_kind);
        let presentation = format!("{error:?} {error}");
        assert!(!presentation.contains("PATH_SECRET"));
        assert!(!presentation.contains("WRONG_KIND_CONTENT_SECRET"));
    }

    assert_eq!(
        std::fs::read(wrong_kind_base.join("machine-god")).unwrap(),
        b"WRONG_KIND_CONTENT_SECRET"
    );
    assert!(symlink_base.join("machine-god").is_symlink());
    assert_eq!(std::fs::read_dir(target).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(unsafe_root).unwrap().count(), 0);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn non_unicode_selected_environment_is_invalid_and_redacted() {
    use std::os::unix::ffi::OsStringExt;

    let environment = NativeEnvironment::new(
        None,
        Some(OsString::from_vec(
            b"NON_UNICODE_STATE_SECRET-\xff".to_vec(),
        )),
        Some(OsString::from("HOME_FALLBACK_MUST_NOT_BE_USED")),
    );
    let error = ready(list_native_sessions(environment)).unwrap_err();
    assert_eq!(
        error.kind(),
        NativeSessionListingErrorKind::InvalidEnvironment
    );
    let presentation = format!("{error:?} {error}");
    assert!(!presentation.contains("NON_UNICODE_STATE_SECRET"));
    assert!(!presentation.contains("HOME_FALLBACK_MUST_NOT_BE_USED"));
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn unsupported_platform_is_fixed_and_redacted() {
    let temporary = TestDirectory::new("unsupported");
    let error = ready(list_native_sessions(environment(
        Some(temporary.path()),
        None,
    )))
    .unwrap_err();
    assert_eq!(
        error.kind(),
        NativeSessionListingErrorKind::UnsupportedPlatform
    );
    assert_eq!(error.kind().as_str(), "unsupported_platform");
    assert_eq!(
        format!("{error:?}"),
        "NativeSessionListingError { kind: UnsupportedPlatform }"
    );
}
