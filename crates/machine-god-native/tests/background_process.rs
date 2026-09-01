#![cfg(any(target_os = "linux", target_os = "macos"))]

#[path = "../src/background_process.rs"]
mod background_process;

#[cfg(target_os = "macos")]
use background_process::BackgroundProcessHelper;
use background_process::{
    BACKGROUND_PROCESS_TERM_GRACE, BackgroundProcessErrorKind, BackgroundProcessExit,
    BackgroundProcessOutcome, BackgroundProcessRequest, MAX_BACKGROUND_PROCESS_COMMAND_BYTES,
    MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES, OwnedBackgroundProcess,
    SystemBackgroundProcessAdapter, cancellation_wakeup_latency_for_test,
    group_snapshot_is_quiescent_for_test, leader_observations_for_test,
    permission_denied_group_signal_is_failure_for_test, reset_leader_observations_for_test,
    run_background_process_helper,
};
use machine_god_core::CancellationToken;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct FreshDirectory {
    path: PathBuf,
}

impl FreshDirectory {
    fn new(label: &str) -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "machine-god-background-process-{}-{sequence}-{label}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FreshDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn request(directory: &Path, command: impl Into<String>) -> BackgroundProcessRequest {
    #[cfg(target_os = "linux")]
    let environment = vec![(OsString::from("LANG"), OsString::from("C"))];
    #[cfg(target_os = "macos")]
    let environment = vec![
        (OsString::from("LANG"), OsString::from("C")),
        (
            OsString::from("MACHINE_GOD_BACKGROUND_HELPER_TEST"),
            OsString::from("1"),
        ),
    ];
    BackgroundProcessRequest::open(
        command.into(),
        "workspace".to_owned(),
        environment,
        directory,
    )
    .unwrap()
}

#[cfg(target_os = "linux")]
fn adapter() -> SystemBackgroundProcessAdapter {
    SystemBackgroundProcessAdapter::default()
}

#[cfg(target_os = "macos")]
fn adapter() -> SystemBackgroundProcessAdapter {
    let helper = BackgroundProcessHelper::new(
        std::env::current_exe().unwrap(),
        vec![
            OsString::from("--exact"),
            OsString::from("macos_helper_process_entry"),
            OsString::from("--test-threads=1"),
            OsString::from("--quiet"),
        ],
    )
    .unwrap();
    SystemBackgroundProcessAdapter::with_helper(helper)
}

#[cfg(target_os = "macos")]
#[test]
fn macos_helper_process_entry() {
    if std::env::var_os("MACHINE_GOD_BACKGROUND_HELPER_TEST").is_some() {
        run_background_process_helper().unwrap();
        unreachable!("successful helper execution replaces this process");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_prepare_requires_helper_ready_handshake() {
    let directory = FreshDirectory::new("helper-ready");
    let request = BackgroundProcessRequest::open(
        "printf bad > marker".to_owned(),
        "workspace".to_owned(),
        vec![(OsString::from("LANG"), OsString::from("C"))],
        directory.path(),
    )
    .unwrap();
    let error = adapter().prepare(request).unwrap_err();
    assert_eq!(error.kind(), BackgroundProcessErrorKind::Spawn);
    assert!(!directory.path().join("marker").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn non_macos_helper_entry_is_an_active_unsupported_error() {
    assert_eq!(
        run_background_process_helper().unwrap_err().kind(),
        BackgroundProcessErrorKind::Unsupported
    );
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for marker");
        thread::sleep(Duration::from_millis(2));
    }
}

fn wait_for_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(raw) = fs::read_to_string(path)
            && let Ok(pid) = raw.parse()
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "timed out waiting for child PID");
        thread::sleep(Duration::from_millis(2));
    }
}

fn process_exists(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    rustix::process::test_kill_process(pid).is_ok()
}

#[test]
fn request_is_bounded_and_errors_are_redacted() {
    let directory = FreshDirectory::new("bounds");
    let oversized = "x".repeat(MAX_BACKGROUND_PROCESS_COMMAND_BYTES + 1);
    let error = BackgroundProcessRequest::open(
        oversized,
        "workspace".to_owned(),
        Vec::new(),
        directory.path(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), BackgroundProcessErrorKind::InvalidRequest);
    assert_eq!(error.to_string(), "background process operation failed");
    assert!(!format!("{error:?}").contains(directory.path().to_str().unwrap()));

    let environment = (0..=MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES)
        .map(|index| (OsString::from(format!("K{index}")), OsString::from("v")))
        .collect();
    assert_eq!(
        BackgroundProcessRequest::open(
            "true".to_owned(),
            "workspace".to_owned(),
            environment,
            directory.path(),
        )
        .unwrap_err()
        .kind(),
        BackgroundProcessErrorKind::InvalidRequest
    );
    let valid = request(directory.path(), "true");
    assert_eq!(valid.cwd(), "workspace");
    assert!(!valid.environment().is_empty());
    assert!(rustix::fs::fstat(valid.directory_fd()).is_ok());
    #[cfg(target_os = "linux")]
    assert!(valid.descriptor_path().starts_with("/proc/"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn command_is_blocked_before_release_then_receives_null_stdin() {
    let directory = FreshDirectory::new("gate");
    let marker = directory.path().join("released");
    let command = "if IFS= read -r value; then exit 91; fi; [ -c /dev/stdin ] || exit 92; printf released > released";
    let prepared = adapter()
        .prepare(request(directory.path(), command))
        .unwrap();
    assert_ne!(prepared.pid().get(), 0);
    thread::sleep(Duration::from_millis(150));
    assert!(!marker.exists(), "user command ran before release");
    let owned = prepared.release().unwrap();
    assert_eq!(owned.wait().unwrap(), BackgroundProcessExit::Exited(0));
    assert_eq!(fs::read_to_string(marker).unwrap(), "released");
}

#[test]
fn idle_wait_observation_uses_bounded_backoff_and_stays_shutdown_responsive() {
    let directory = FreshDirectory::new("backoff-idle");
    let owned = adapter()
        .prepare(request(directory.path(), "sleep 1"))
        .unwrap()
        .release()
        .unwrap();
    reset_leader_observations_for_test();
    assert_eq!(owned.wait().unwrap(), BackgroundProcessExit::Exited(0));
    let observations = leader_observations_for_test();
    assert!(
        observations < 40,
        "one idle process used {observations} observations per second"
    );
    assert!(
        cancellation_wakeup_latency_for_test(Duration::from_secs(1)) < Duration::from_millis(250),
        "cancellation must wake a parked observer"
    );

    let directory = FreshDirectory::new("backoff-cancel");
    let ready = directory.path().join("ready");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "printf ready > ready; while :; do /bin/sleep 1; done",
        ))
        .unwrap()
        .release()
        .unwrap();
    wait_for(&ready);
    let stop = CancellationToken::new();
    let worker_stop = stop.clone();
    let worker = thread::spawn(move || owned.wait_with_stop(&worker_stop));
    thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    assert!(stop.cancel());
    assert_eq!(
        worker.join().unwrap().unwrap(),
        BackgroundProcessOutcome::Stopped
    );
    assert!(
        started.elapsed() < BACKGROUND_PROCESS_TERM_GRACE + Duration::from_millis(250),
        "bounded observation backoff delayed cooperative shutdown"
    );
}

#[test]
fn group_cleanup_fails_closed_for_permission_denial_and_surviving_members() {
    assert!(permission_denied_group_signal_is_failure_for_test());
    assert!(group_snapshot_is_quiescent_for_test(b"41\n", 41).unwrap());
    assert!(!group_snapshot_is_quiescent_for_test(b"", 41).unwrap());
    assert!(!group_snapshot_is_quiescent_for_test(b"41\n73\n", 41).unwrap());
    assert!(group_snapshot_is_quiescent_for_test(b"not-a-pid\n", 41).is_err());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn command_is_an_argv_value_not_wrapper_interpolation() {
    let directory = FreshDirectory::new("argv");
    let command = "printf '%s' \"$PAYLOAD\" > 'literal $(touch injected)'";
    let request = BackgroundProcessRequest::open(
        command.to_owned(),
        "workspace".to_owned(),
        {
            #[cfg(target_os = "linux")]
            let environment = vec![(OsString::from("PAYLOAD"), OsString::from("a ' b $ c"))];
            #[cfg(target_os = "macos")]
            let environment = vec![
                (OsString::from("PAYLOAD"), OsString::from("a ' b $ c")),
                (
                    OsString::from("MACHINE_GOD_BACKGROUND_HELPER_TEST"),
                    OsString::from("1"),
                ),
            ];
            environment
        },
        directory.path(),
    )
    .unwrap();
    assert_eq!(request.command(), command);
    assert_eq!(request.program(), "/bin/sh");
    let owned = adapter().prepare(request).unwrap().release().unwrap();
    assert_eq!(owned.wait().unwrap(), BackgroundProcessExit::Exited(0));
    assert_eq!(
        fs::read_to_string(directory.path().join("literal $(touch injected)")).unwrap(),
        "a ' b $ c"
    );
    assert!(!directory.path().join("injected").exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn retained_directory_survives_rename_and_symlink_replacement() {
    let parent = FreshDirectory::new("retained-parent");
    let original = parent.path().join("cwd");
    let moved = parent.path().join("moved");
    let replacement = parent.path().join("replacement");
    fs::create_dir(&original).unwrap();
    fs::create_dir(&replacement).unwrap();
    let request = request(&original, "printf retained > marker");
    fs::rename(&original, &moved).unwrap();
    std::os::unix::fs::symlink(&replacement, &original).unwrap();

    let owned = adapter().prepare(request).unwrap().release().unwrap();
    assert_eq!(owned.wait().unwrap(), BackgroundProcessExit::Exited(0));
    assert_eq!(
        fs::read_to_string(moved.join("marker")).unwrap(),
        "retained"
    );
    assert!(!replacement.join("marker").exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn null_output_handles_an_unbounded_flood_without_backpressure() {
    let directory = FreshDirectory::new("flood");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "i=0; while [ $i -lt 200000 ]; do printf '0123456789abcdef'; i=$((i+1)); done",
        ))
        .unwrap()
        .release()
        .unwrap();
    assert_eq!(owned.wait().unwrap(), BackgroundProcessExit::Exited(0));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn normal_leader_exit_cleans_lingering_group_descendant() {
    let directory = FreshDirectory::new("descendant");
    let pid_file = directory.path().join("descendant.pid");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "sleep 30 & printf '%s' \"$!\" > descendant.pid; exit 7",
        ))
        .unwrap()
        .release()
        .unwrap();
    let original_group = owned.pid().get();
    let descendant = wait_for_pid(&pid_file);
    assert_eq!(owned.wait().unwrap(), BackgroundProcessExit::Exited(7));
    assert!(!process_is_in_group(descendant, original_group));
}

fn process_is_in_group(pid: u32, group: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    let Ok(group) = i32::try_from(group) else {
        return false;
    };
    rustix::process::getpgid(Some(pid))
        .is_ok_and(|observed| observed.as_raw_nonzero().get() == group)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn stop_escalates_past_ignored_term_and_reaps() {
    let directory = FreshDirectory::new("term-ignore");
    let ready = directory.path().join("ready");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "trap '' TERM; printf ready > ready; while :; do :; done",
        ))
        .unwrap()
        .release()
        .unwrap();
    let pid = owned.pid().get();
    wait_for(&ready);
    let started = Instant::now();
    owned.stop().unwrap();
    assert!(started.elapsed() >= BACKGROUND_PROCESS_TERM_GRACE);
    assert!(!process_exists(pid));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn cancellable_wait_stops_and_reaps_while_wait_is_active() {
    let directory = FreshDirectory::new("cancel-wait");
    let ready = directory.path().join("ready");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "trap '' TERM; printf ready > ready; while :; do :; done",
        ))
        .unwrap()
        .release()
        .unwrap();
    let pid = owned.pid().get();
    wait_for(&ready);
    let stop = CancellationToken::new();
    let worker_stop = stop.clone();
    let worker = thread::spawn(move || owned.wait_with_stop(&worker_stop));
    thread::sleep(Duration::from_millis(20));
    assert!(stop.cancel());
    assert_eq!(
        worker.join().unwrap().unwrap(),
        BackgroundProcessOutcome::Stopped
    );
    assert!(!process_exists(pid));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn prepared_abort_and_both_handle_drops_clean_processes() {
    let directory = FreshDirectory::new("drop");
    let marker = directory.path().join("must-not-run");
    let prepared = adapter()
        .prepare(request(
            directory.path(),
            "printf bad > must-not-run; sleep 30",
        ))
        .unwrap();
    let prepared_pid = prepared.pid().get();
    prepared.abort_and_reap().unwrap();
    assert!(!marker.exists());
    assert!(!process_exists(prepared_pid));

    let prepared = adapter()
        .prepare(request(directory.path(), "sleep 30"))
        .unwrap();
    let dropped_prepared_pid = prepared.pid().get();
    drop(prepared);
    assert!(!process_exists(dropped_prepared_pid));

    let owned: OwnedBackgroundProcess = adapter()
        .prepare(request(directory.path(), "sleep 30"))
        .unwrap()
        .release()
        .unwrap();
    let owned_pid = owned.pid().get();
    drop(owned);
    assert!(!process_exists(owned_pid));
}
