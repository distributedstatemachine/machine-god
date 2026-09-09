use super::*;
use crate::background_process::TmuxChild;
use futures_executor::block_on;
use std::io::Read as _;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::task::{Context, Poll, Waker};

pub(super) struct Probe {
    pid: AtomicU32,
    deferred: Option<Arc<AtomicBool>>,
    worker_destroyed: Arc<AtomicBool>,
}
thread_local! {
    static WORKER_EXIT: std::cell::RefCell<Option<ExitMarker>> = const { std::cell::RefCell::new(None) };
}
struct ExitMarker(Arc<AtomicBool>);
impl Drop for ExitMarker {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
impl Probe {
    pub(super) fn observe(&self, child: &mut TmuxChild) {
        self.pid.store(child.id(), Ordering::Release);
        if let Some(deferred) = &self.deferred {
            child.defer_reap_for_test(Arc::clone(deferred));
        }
        WORKER_EXIT.with(|slot| {
            *slot.borrow_mut() = Some(ExitMarker(Arc::clone(&self.worker_destroyed)));
        });
    }
}

struct Fixture {
    directory: PathBuf,
    clipboard: NativeClipboard,
    probe: Arc<Probe>,
    scope: NativeOwnedWorkerScope,
}
impl Fixture {
    fn new(mode: &str, deferred: Option<Arc<AtomicBool>>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "machine-god-clipboard-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut authority =
            NativeClipboardExecutable::new(&executable, File::open(&executable).unwrap()).unwrap();
        authority.arguments = Some(Arc::new(vec![
            "--exact".into(),
            "clipboard::tests::fixture_child".into(),
            "--ignored".into(),
            "--nocapture".into(),
        ]));
        let scope = NativeOwnedWorkerScope::new();
        let probe = Arc::new(Probe {
            pid: AtomicU32::new(0),
            deferred,
            worker_destroyed: Arc::new(AtomicBool::new(false)),
        });
        let mut clipboard = NativeClipboard::new(
            authority,
            directory.clone(),
            vec![
                ("MG_CLIPBOARD_MODE".into(), mode.into()),
                (
                    "MG_CLIPBOARD_DIRECTORY".into(),
                    directory.as_os_str().to_owned(),
                ),
                ("MG_CLIPBOARD_FROZEN".into(), "é\nvalue".into()),
            ],
            scope.clone(),
        )
        .unwrap();
        Arc::get_mut(&mut clipboard.inner).unwrap().probe = Some(Arc::clone(&probe));
        Self {
            directory,
            clipboard,
            probe,
            scope,
        }
    }
    fn run(&self, text: &str) -> Result<(), NativeClipboardError> {
        block_on(
            self.clipboard
                .copy(Arc::from(text), CancellationToken::new()),
        )
    }
    fn joined(&self) {
        self.scope.close();
        self.scope.completion().wait_on_worker().unwrap();
        assert!(self.scope.completion().is_complete());
    }
    fn assert_reaped(&self) {
        let raw = i32::try_from(self.probe.pid.load(Ordering::Acquire)).unwrap();
        let pid = rustix::process::Pid::from_raw(raw).unwrap();
        assert_eq!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG).unwrap_err(),
            rustix::io::Errno::CHILD
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(deferred) = &self.probe.deferred {
            deferred.store(false, Ordering::Release);
        }
        self.joined();
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
fn until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "clipboard fixture observation expired"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn poll<T>(future: &mut BoxFuture<'static, T>) -> Poll<T> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
#[ignore = "private clipboard fixture subprocess, never the user's clipboard"]
fn fixture_child() {
    let directory = PathBuf::from(std::env::var_os("MG_CLIPBOARD_DIRECTORY").unwrap());
    let mode = std::env::var("MG_CLIPBOARD_MODE").unwrap();
    assert_eq!(std::env::var("MG_CLIPBOARD_FROZEN").unwrap(), "é\nvalue");
    assert_eq!(
        std::env::current_dir().unwrap(),
        std::fs::canonicalize(&directory).unwrap()
    );
    assert!(std::env::var_os("PATH").is_none());
    std::fs::write(directory.join("ready"), b"ready").unwrap();
    if mode == "nonzero" {
        std::process::exit(7);
    }
    if mode == "signal" {
        rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::KILL)
            .unwrap();
        unreachable!();
    }
    if mode == "blocked-write" {
        std::thread::sleep(Duration::from_secs(60));
        std::process::exit(9);
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(u64::try_from(crate::MAX_FILE_SESSION_BYTES + 1).unwrap())
        .read_to_end(&mut bytes)
        .unwrap();
    std::fs::write(directory.join("bytes"), bytes).unwrap();
    if mode == "blocked-exit" {
        std::thread::sleep(Duration::from_secs(60));
    }
    std::process::exit(0);
}

#[test]
fn exact_utf8_control_bytes_and_eof_without_added_newline() {
    let fixture = Fixture::new("copy", None);
    let text = "é🙂\0\u{1b}[31m\r\n raw tail";
    assert_eq!(fixture.run(text), Ok(()));
    assert_eq!(
        std::fs::read(fixture.directory.join("bytes")).unwrap(),
        text.as_bytes()
    );
    fixture.joined();
    fixture.assert_reaped();
    assert!(fixture.probe.worker_destroyed.load(Ordering::Acquire));
}

#[test]
fn empty_input_closes_stdin_and_success_releases_shared_admission() {
    let fixture = Fixture::new("copy", None);
    assert_eq!(fixture.run(""), Ok(()));
    assert_eq!(std::fs::read(fixture.directory.join("bytes")).unwrap(), b"");
    assert_eq!(
        block_on(
            fixture
                .clipboard
                .clone()
                .copy(Arc::from("next"), CancellationToken::new())
        ),
        Ok(())
    );
    assert_eq!(
        std::fs::read(fixture.directory.join("bytes")).unwrap(),
        b"next"
    );
}

#[test]
fn maximum_store_valid_text_is_complete_and_overflow_is_inert() {
    let fixture = Fixture::new("copy", None);
    let text = "x".repeat(crate::MAX_FILE_SESSION_BYTES);
    assert_eq!(fixture.run(&text), Ok(()));
    assert_eq!(
        std::fs::read(fixture.directory.join("bytes")).unwrap(),
        text.as_bytes()
    );
    assert_eq!(
        fixture.run(&"x".repeat(crate::MAX_FILE_SESSION_BYTES + 1)),
        Err(NativeClipboardError::ResourceLimit)
    );
}

#[test]
fn unpolled_and_precancelled_operations_have_no_child_or_admission_effect() {
    let fixture = Fixture::new("copy", None);
    drop(
        fixture
            .clipboard
            .copy(Arc::from("never"), CancellationToken::new()),
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        block_on(fixture.clipboard.copy(Arc::from("never"), cancel)),
        Err(NativeClipboardError::Cancelled)
    );
    assert_eq!(fixture.probe.pid.load(Ordering::Acquire), 0);
    assert!(!fixture.clipboard.inner.active.load(Ordering::Acquire));
    assert!(!fixture.directory.join("ready").exists());
    assert_eq!(fixture.run("next"), Ok(()));
}

#[test]
fn nonzero_and_signal_exit_are_not_success() {
    for mode in ["nonzero", "signal"] {
        let fixture = Fixture::new(mode, None);
        assert_eq!(fixture.run(""), Err(NativeClipboardError::ExitFailed));
        fixture.joined();
        fixture.assert_reaped();
    }
}

#[test]
fn early_closed_pipe_is_a_write_failure() {
    let fixture = Fixture::new("nonzero", None);
    assert_eq!(
        fixture.run(&"x".repeat(crate::MAX_FILE_SESSION_BYTES)),
        Err(NativeClipboardError::WriteFailed)
    );
    fixture.joined();
    fixture.assert_reaped();
}

#[test]
fn blocked_write_and_exit_cancel_without_waiting_for_operation_deadline() {
    for mode in ["blocked-write", "blocked-exit"] {
        let fixture = Fixture::new(mode, None);
        let text: Arc<str> = if mode == "blocked-write" {
            "x".repeat(crate::MAX_FILE_SESSION_BYTES).into()
        } else {
            Arc::from("text")
        };
        let cancel = CancellationToken::new();
        let mut future = fixture.clipboard.copy(text, cancel.clone());
        assert!(poll(&mut future).is_pending());
        let marker = fixture.directory.join(if mode == "blocked-exit" {
            "bytes"
        } else {
            "ready"
        });
        until(|| marker.exists());
        assert_eq!(
            fixture.run("must not overlap"),
            Err(NativeClipboardError::Busy)
        );
        cancel.cancel();
        assert_eq!(block_on(future), Err(NativeClipboardError::Cancelled));
        fixture.joined();
        fixture.assert_reaped();
    }
}

#[test]
fn abandoned_response_cancels_private_job_and_actual_scope_join_settles_child() {
    let fixture = Fixture::new("blocked-exit", None);
    let cancel = CancellationToken::new();
    let mut future = fixture.clipboard.copy(Arc::from("text"), cancel.clone());
    assert!(poll(&mut future).is_pending());
    until(|| fixture.directory.join("bytes").exists());
    drop(future);
    fixture.joined();
    fixture.assert_reaped();
    assert!(!cancel.is_cancelled());
    assert!(fixture.probe.worker_destroyed.load(Ordering::Acquire));
}

#[test]
fn cancelled_unreaped_child_keeps_admission_and_scope_completion_pending() {
    let deferred = Arc::new(AtomicBool::new(true));
    let fixture = Fixture::new("blocked-exit", Some(Arc::clone(&deferred)));
    let cancel = CancellationToken::new();
    let mut future = fixture.clipboard.copy(Arc::from("text"), cancel.clone());
    assert!(poll(&mut future).is_pending());
    until(|| fixture.directory.join("bytes").exists());
    cancel.cancel();
    assert_eq!(block_on(future), Err(NativeClipboardError::Cancelled));
    assert_eq!(
        fixture.run("must not overlap"),
        Err(NativeClipboardError::Busy)
    );
    fixture.scope.close();
    assert!(!fixture.scope.completion().is_complete());
    deferred.store(false, Ordering::Release);
    fixture.joined();
    fixture.assert_reaped();
    assert!(!fixture.clipboard.inner.active.load(Ordering::Acquire));
}

#[test]
fn deadline_failure_keeps_real_cleanup_and_does_not_become_success() {
    let fixture = Fixture::new("blocked-exit", None);
    assert_eq!(fixture.run("text"), Err(NativeClipboardError::TimedOut));
    fixture.joined();
    fixture.assert_reaped();
}

#[test]
fn spawn_and_identity_failure_are_fixed_and_release_admission() {
    let mut fixture = Fixture::new("copy", None);
    let invalid = fixture.directory.join("not-an-executable-image");
    std::fs::write(&invalid, b"#!/machine-god-missing-clipboard-interpreter\n").unwrap();
    std::fs::set_permissions(&invalid, std::fs::Permissions::from_mode(0o700)).unwrap();
    Arc::get_mut(&mut fixture.clipboard.inner)
        .unwrap()
        .executable =
        NativeClipboardExecutable::new(&invalid, File::open(&invalid).unwrap()).unwrap();
    assert_eq!(
        fixture.run("secret"),
        Err(NativeClipboardError::Unavailable)
    );
    assert!(!fixture.clipboard.inner.active.load(Ordering::Acquire));
    let replacement = fixture.directory.join("replacement");
    std::fs::write(&replacement, b"other bytes").unwrap();
    std::fs::rename(&replacement, &invalid).unwrap();
    assert_eq!(
        fixture.run("secret"),
        Err(NativeClipboardError::Unavailable)
    );
    assert_eq!(fixture.probe.pid.load(Ordering::Acquire), 0);
    assert!(!format!("{:?}", fixture.clipboard).contains("not-an-executable"));
    assert!(
        !NativeClipboardError::Unavailable
            .to_string()
            .contains("secret")
    );
}

#[test]
fn unavailable_cwd_and_closed_scope_do_not_launch() {
    let mut fixture = Fixture::new("copy", None);
    Arc::get_mut(&mut fixture.clipboard.inner)
        .unwrap()
        .working_directory = fixture.directory.join("missing");
    assert_eq!(fixture.run("text"), Err(NativeClipboardError::Unavailable));
    assert!(!fixture.directory.join("ready").exists());
    fixture.scope.close();
    assert_eq!(fixture.run("text"), Err(NativeClipboardError::Unavailable));
    assert!(!fixture.clipboard.inner.active.load(Ordering::Acquire));
}

#[test]
fn worker_requires_matching_regular_executable_not_symlink_or_directory() {
    let mut fixture = Fixture::new("copy", None);
    let executable = std::env::current_exe().unwrap();
    let alias = fixture.directory.join("program-alias");
    std::os::unix::fs::symlink(&executable, &alias).unwrap();
    Arc::get_mut(&mut fixture.clipboard.inner)
        .unwrap()
        .executable =
        NativeClipboardExecutable::new(&alias, File::open(&executable).unwrap()).unwrap();
    assert_eq!(fixture.run("text"), Err(NativeClipboardError::Unavailable));

    let not_executable = fixture.directory.join("plain-file");
    std::fs::write(&not_executable, b"plain").unwrap();
    std::fs::set_permissions(&not_executable, std::fs::Permissions::from_mode(0o600)).unwrap();
    Arc::get_mut(&mut fixture.clipboard.inner)
        .unwrap()
        .executable =
        NativeClipboardExecutable::new(&not_executable, File::open(&not_executable).unwrap())
            .unwrap();
    assert_eq!(fixture.run("text"), Err(NativeClipboardError::Unavailable));

    Arc::get_mut(&mut fixture.clipboard.inner)
        .unwrap()
        .executable =
        NativeClipboardExecutable::new(&fixture.directory, File::open(&fixture.directory).unwrap())
            .unwrap();
    assert_eq!(fixture.run("text"), Err(NativeClipboardError::Unavailable));
    assert_eq!(fixture.probe.pid.load(Ordering::Acquire), 0);
}

#[test]
fn inert_authority_and_environment_validation_are_bounded() {
    for path in ["relative", "/tmp/../bad", "/nul\0path"] {
        assert_eq!(
            validate_path(Path::new(path)),
            Err(NativeClipboardError::InvalidAuthority)
        );
    }
    assert_eq!(
        validate_path(Path::new(&format!("/{}", "x".repeat(4096)))),
        Err(NativeClipboardError::InvalidAuthority)
    );
    for mut entries in [
        vec![("A".into(), "1".into()), ("A".into(), "2".into())],
        vec![("A=B".into(), "v".into())],
        vec![("A".into(), "\0".into())],
    ] {
        assert_eq!(
            validate_environment(&mut entries),
            Err(NativeClipboardError::InvalidAuthority)
        );
    }
    assert_eq!(
        validate_environment(&mut vec![
            ("A".into(), "v".into());
            MAX_TERMINAL_ENVIRONMENT_ENTRIES + 1
        ]),
        Err(NativeClipboardError::ResourceLimit)
    );
    assert_eq!(
        validate_environment(&mut [(
            "A".into(),
            "x".repeat(MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES + 1).into()
        )]),
        Err(NativeClipboardError::ResourceLimit)
    );
    let mut maximum = vec![(
        "A".into(),
        "x".repeat(MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES).into(),
    )];
    assert_eq!(validate_environment(&mut maximum), Ok(()));
    let mut aggregate: Vec<(OsString, OsString)> = (0..16)
        .map(|index| {
            (
                format!("K{index:02}").into(),
                "x".repeat(MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES).into(),
            )
        })
        .collect();
    aggregate[0].1 = "x".repeat(MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES - 48).into();
    assert_eq!(validate_environment(&mut aggregate), Ok(()));
    aggregate[0].1.push("x");
    assert_eq!(
        validate_environment(&mut aggregate),
        Err(NativeClipboardError::ResourceLimit)
    );
}

#[test]
fn constructor_does_not_inspect_program_or_cwd() {
    let fixture = Fixture::new("copy", None);
    let missing = fixture.directory.join("missing-program");
    let authority = NativeClipboardExecutable::new(
        &missing,
        File::open(std::env::current_exe().unwrap()).unwrap(),
    )
    .unwrap();
    let clipboard = NativeClipboard::new(
        authority,
        fixture.directory.join("missing-cwd"),
        vec![],
        fixture.scope.clone(),
    )
    .unwrap();
    drop(clipboard.copy(Arc::from("unpolled"), CancellationToken::new()));
    assert_eq!(fixture.probe.pid.load(Ordering::Acquire), 0);
    assert!(!clipboard.inner.active.load(Ordering::Acquire));
    assert_eq!(
        block_on(clipboard.copy(Arc::from("text"), CancellationToken::new())),
        Err(NativeClipboardError::Unavailable)
    );
}
