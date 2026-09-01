#![cfg(any(target_os = "linux", target_os = "macos"))]

#[path = "../src/background_process.rs"]
mod background_process;

use background_process::BackgroundProcessHelper;
use background_process::{
    BACKGROUND_PROCESS_TERM_GRACE, BackgroundProcessErrorKind, BackgroundProcessExit,
    BackgroundProcessOutcome, BackgroundProcessRequest, MAX_BACKGROUND_PROCESS_COMMAND_BYTES,
    MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES, OwnedBackgroundProcess,
    SystemBackgroundProcessAdapter, cancellation_wakeup_latency_for_test,
    group_signal_attempts_for_test, group_snapshot_is_quiescent_for_test,
    inject_group_snapshot_failures_for_test, inject_group_snapshot_spawn_failures_for_test,
    inject_waitid_failures_for_test, leader_observations_for_test,
    permission_denied_group_signal_is_failure_for_test, reset_group_signal_attempts_for_test,
    reset_leader_observations_for_test, run_background_process_helper,
};
use machine_god_core::CancellationToken;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
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
    let environment = vec![(OsString::from("LANG"), OsString::from("C"))];
    BackgroundProcessRequest::open(
        command.into(),
        "workspace".to_owned(),
        environment,
        directory,
    )
    .unwrap()
}

fn adapter() -> SystemBackgroundProcessAdapter {
    let helper = BackgroundProcessHelper::new(
        std::env::current_exe().unwrap(),
        vec![
            OsString::from("--exact"),
            OsString::from("helper_process_entry"),
            OsString::from("--test-threads=1"),
            OsString::from("--quiet"),
        ],
    )
    .unwrap();
    SystemBackgroundProcessAdapter::with_helper(helper)
}

#[test]
fn helper_process_entry() {
    if std::env::var_os("MACHINE_GOD_BACKGROUND_HELPER_MODE").is_some() {
        for key in [
            "LD_PRELOAD",
            "LD_AUDIT",
            "DYLD_INSERT_LIBRARIES",
            "DYLD_LIBRARY_PATH",
        ] {
            assert!(
                std::env::var_os(key).is_none(),
                "requested loader environment reached the pre-release helper"
            );
        }
        run_background_process_helper().unwrap();
        unreachable!("successful helper execution replaces this process");
    }
}

#[test]
fn requested_environment_is_inert_until_release_and_frame_bytes_cannot_fake_readiness() {
    let directory = FreshDirectory::new("inert-environment");
    let marker = directory.path().join("released");
    let environment = vec![
        (OsString::from("PAYLOAD"), OsString::from("after-release")),
        (
            OsString::from("LD_PRELOAD"),
            OsString::from("/definitely/missing"),
        ),
        (
            OsString::from("LD_AUDIT"),
            OsString::from("/definitely/missing"),
        ),
        (
            OsString::from("DYLD_INSERT_LIBRARIES"),
            OsString::from("/definitely/missing"),
        ),
        (
            OsString::from("DYLD_LIBRARY_PATH"),
            OsString::from("/definitely/missing"),
        ),
        (
            OsString::from("FRAME_BYTES"),
            OsString::from_vec(b"\xa7MGBG-FRAME-\xff".to_vec()),
        ),
    ];
    let request = BackgroundProcessRequest::open(
        "[ \"$PAYLOAD\" = after-release ] && printf released > released".to_owned(),
        "workspace".to_owned(),
        environment,
        directory.path(),
    )
    .unwrap();

    let prepared = adapter().prepare(request).unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(
        !marker.exists(),
        "requested environment caused a pre-release effect"
    );
    let owned = prepared.release().unwrap();
    assert_eq!(owned.wait().unwrap(), BackgroundProcessExit::Exited(0));
    assert_eq!(fs::read_to_string(marker).unwrap(), "released");
}

#[cfg(target_os = "linux")]
#[test]
fn child_reaping_mode_entry() {
    let Some(root) = std::env::var_os("MACHINE_GOD_REAP_MODE_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let error = adapter()
        .prepare(request(&root, "printf bad > must-not-run"))
        .unwrap_err();
    assert_eq!(error.kind(), BackgroundProcessErrorKind::Spawn);
    assert!(!root.join("must-not-run").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn incompatible_sigchld_modes_fail_before_the_requested_child_is_spawned() {
    let executable = std::env::current_exe().unwrap();
    for mode in ["ignored", "no-cld-wait"] {
        let directory = FreshDirectory::new(mode);
        let status = if mode == "ignored" {
            Command::new("/bin/sh")
                .args([
                    "-c",
                    "trap '' CHLD; exec \"$1\" --exact child_reaping_mode_entry --test-threads=1 --quiet",
                    "machine-god-reap-launcher",
                ])
                .arg(&executable)
                .env("MACHINE_GOD_REAP_MODE_TEST_ROOT", directory.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
        } else {
            let source = directory.path().join("no-cld-wait.c");
            let launcher = directory.path().join("no-cld-wait");
            fs::write(
                &source,
                b"#include <signal.h>\n#include <unistd.h>\nint main(int argc, char **argv) { (void)argc; struct sigaction sa = {0}; sa.sa_handler = SIG_DFL; sigemptyset(&sa.sa_mask); sa.sa_flags = SA_NOCLDWAIT; if (sigaction(SIGCHLD, &sa, 0) != 0) return 90; execv(argv[1], &argv[1]); return 91; }\n",
            )
            .unwrap();
            assert!(
                Command::new("cc")
                    .args(["-Wall", "-Wextra", "-Werror", "-o"])
                    .arg(&launcher)
                    .arg(&source)
                    .status()
                    .unwrap()
                    .success()
            );
            Command::new(&launcher)
                .arg(&executable)
                .args([
                    "--exact",
                    "child_reaping_mode_entry",
                    "--test-threads=1",
                    "--quiet",
                ])
                .env("MACHINE_GOD_REAP_MODE_TEST_ROOT", directory.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
        };
        assert!(status.success(), "{mode} subprocess failed: {status}");
        assert!(!directory.path().join("must-not-run").exists());
    }
}

#[test]
fn external_reap_loses_authority_without_any_group_signal() {
    let directory = FreshDirectory::new("external-reap");
    let owned = adapter()
        .prepare(request(directory.path(), "exit 0"))
        .unwrap()
        .release()
        .unwrap();
    let leader = owned.pid();
    let pid = rustix::process::Pid::from_raw(i32::try_from(leader.get()).unwrap()).unwrap();
    rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::empty())
        .unwrap()
        .expect("external reaper retained the child");
    reset_group_signal_attempts_for_test(leader);

    let error = owned.wait().unwrap_err();

    assert_eq!(error.kind(), BackgroundProcessErrorKind::Wait);
    assert_eq!(group_signal_attempts_for_test(), 0);
}

#[test]
fn prepare_requires_helper_ready_handshake() {
    let directory = FreshDirectory::new("helper-ready");
    let request = BackgroundProcessRequest::open(
        "printf bad > marker".to_owned(),
        "workspace".to_owned(),
        vec![(OsString::from("LANG"), OsString::from("C"))],
        directory.path(),
    )
    .unwrap();
    let helper = BackgroundProcessHelper::new(
        PathBuf::from("/bin/sh"),
        vec![OsString::from("-c"), OsString::from("exit 0")],
    )
    .unwrap();
    let error = SystemBackgroundProcessAdapter::with_helper(helper)
        .prepare(request)
        .unwrap_err();
    assert_eq!(error.kind(), BackgroundProcessErrorKind::Spawn);
    assert!(!directory.path().join("marker").exists());
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

fn assert_processes_absent(pids: &[u32]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let surviving = pids
            .iter()
            .copied()
            .filter(|pid| process_exists(*pid))
            .collect::<Vec<_>>();
        if surviving.is_empty() {
            return;
        }
        if Instant::now() >= deadline {
            for pid in &surviving {
                if let Some(pid) = i32::try_from(*pid)
                    .ok()
                    .and_then(rustix::process::Pid::from_raw)
                {
                    let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
                }
            }
            panic!("cleanup returned while processes still existed: {surviving:?}");
        }
        thread::sleep(Duration::from_millis(5));
    }
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
    reset_leader_observations_for_test(owned.pid());
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

#[test]
fn waitid_and_snapshot_spawn_failures_keep_wait_precedence_and_cleanup() {
    let directory = FreshDirectory::new("waitid-cleanup-failure");
    let pid_file = directory.path().join("descendant.pid");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "trap '' TERM; sleep 30 & printf '%s' \"$!\" > descendant.pid; while :; do :; done",
        ))
        .unwrap()
        .release()
        .unwrap();
    let leader = owned.pid();
    let descendant = wait_for_pid(&pid_file);
    inject_waitid_failures_for_test(leader, 1);
    inject_group_snapshot_spawn_failures_for_test(leader, 1);

    let error = owned.wait().unwrap_err();

    assert_eq!(error.kind(), BackgroundProcessErrorKind::Wait);
    assert_processes_absent(&[leader.get(), descendant]);
}

#[test]
fn group_snapshot_failure_after_observation_still_cleans_and_reaps() {
    let directory = FreshDirectory::new("snapshot-cleanup-failure");
    let pid_file = directory.path().join("descendant.pid");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "sleep 30 & printf '%s' \"$!\" > descendant.pid; exit 7",
        ))
        .unwrap()
        .release()
        .unwrap();
    let leader = owned.pid();
    let descendant = wait_for_pid(&pid_file);
    inject_group_snapshot_failures_for_test(leader, 1);

    let error = owned.wait().unwrap_err();

    assert_eq!(error.kind(), BackgroundProcessErrorKind::Cleanup);
    assert_processes_absent(&[leader.get(), descendant]);
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
            let environment = vec![(OsString::from("PAYLOAD"), OsString::from("a ' b $ c"))];
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
