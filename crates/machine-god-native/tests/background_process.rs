#![cfg(any(target_os = "linux", target_os = "macos"))]

#[path = "../src/background_process.rs"]
mod background_process;

use background_process::BackgroundProcessHelper;
use background_process::{
    BACKGROUND_PROCESS_TERM_GRACE, BackgroundProcessErrorKind, BackgroundProcessExit,
    BackgroundProcessOutcome, BackgroundProcessRequest, BackgroundProcessSignal,
    BackgroundProcessSignalErrorKind, MAX_BACKGROUND_PROCESS_COMMAND_BYTES,
    MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES, OwnedBackgroundProcess,
    SystemBackgroundProcessAdapter, cancel_release_before_commit_for_test,
    cancellation_wakeup_latency_for_test, clear_release_before_commit_cancellation_for_test,
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
use std::process::Command;
#[cfg(target_os = "linux")]
use std::process::Stdio;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);
#[cfg(target_os = "linux")]
static TEST_SUBREAPER: OnceLock<()> = OnceLock::new();

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
    #[cfg(target_os = "linux")]
    TEST_SUBREAPER.get_or_init(|| {
        // Cargo can itself be PID 1 in a container and does not reap the test
        // binary's orphaned fixtures. Make the test binary their nearest
        // reaper so the production captured-member protocol is exercised.
        rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1)).unwrap();
    });
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
    let hostile_environment = vec![
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
        hostile_environment,
        directory.path(),
    )
    .unwrap();

    let prepared = adapter().prepare(request).unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(
        !marker.exists(),
        "requested environment caused a pre-release effect"
    );
    prepared.abort_and_reap().unwrap();

    let benign_environment = vec![
        (OsString::from("PAYLOAD"), OsString::from("after-release")),
        (
            OsString::from("FRAME_BYTES"),
            OsString::from_vec(b"\xa7MGBG-FRAME-\xff".to_vec()),
        ),
    ];
    let request = BackgroundProcessRequest::open(
        "[ \"$PAYLOAD\" = after-release ] && printf released > released".to_owned(),
        "workspace".to_owned(),
        benign_environment,
        directory.path(),
    )
    .unwrap();
    let prepared = adapter().prepare(request).unwrap();
    let owned = prepared.release().unwrap();
    assert_eq!(owned.wait().unwrap(), BackgroundProcessExit::Exited(0));
    assert_eq!(fs::read_to_string(marker).unwrap(), "released");
}

#[test]
fn cancellation_after_complete_payload_before_commit_runs_no_user_command() {
    let directory = FreshDirectory::new("cancel-before-commit");
    let marker = directory.path().join("user-ran");
    let mut prepared = adapter()
        .prepare(request(directory.path(), "printf user-ran > user-ran"))
        .expect("prepare inert helper");
    let controller = prepared.attach_signal_controller().unwrap();
    let pid = prepared.pid();
    let cancellation = CancellationToken::new();
    cancel_release_before_commit_for_test(pid);
    let error = prepared
        .release_cancellable(&cancellation)
        .expect_err("pre-commit cancellation aborts release");
    clear_release_before_commit_cancellation_for_test();

    assert_eq!(error.kind(), BackgroundProcessErrorKind::Cancelled);
    assert!(cancellation.is_cancelled());
    assert!(controller.is_closed_for_test());
    assert!(
        !marker.exists(),
        "complete payload without the final commit byte executed the user command"
    );
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
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

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn incompatible_sigchld_modes_fail_before_the_requested_child_is_spawned() {
    let executable = std::env::current_exe().unwrap();
    let launcher_directory = FreshDirectory::new("sigchld-preload");
    let source = launcher_directory.path().join("sigchld-preload.c");
    let preload = launcher_directory.path().join("sigchld-preload.so");
    fs::write(
        &source,
        b"#define _POSIX_C_SOURCE 200809L\n#include <signal.h>\n#include <stdlib.h>\n#include <string.h>\n#include <unistd.h>\n__attribute__((constructor)) static void install_sigchld_mode(void) { const char *mode = getenv(\"MACHINE_GOD_REAP_MODE\"); if (mode == NULL) return; struct sigaction action = {0}; if (sigemptyset(&action.sa_mask) != 0) _exit(90); if (strcmp(mode, \"ignored\") == 0) { action.sa_handler = SIG_IGN; } else if (strcmp(mode, \"no-cld-wait\") == 0) { action.sa_handler = SIG_DFL; action.sa_flags = SA_NOCLDWAIT; } else { _exit(88); } if (sigaction(SIGCHLD, &action, NULL) != 0) _exit(91); }\n",
    )
    .unwrap();
    let compiler = Command::new("cc")
        .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&preload)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        compiler.status.success(),
        "preload compilation failed: {}; stdout: {}; stderr: {}",
        compiler.status,
        String::from_utf8_lossy(&compiler.stdout),
        String::from_utf8_lossy(&compiler.stderr)
    );

    for mode in ["ignored", "no-cld-wait"] {
        let directory = FreshDirectory::new(mode);
        let output = Command::new(&executable)
            .args([
                "--exact",
                "child_reaping_mode_entry",
                "--test-threads=1",
                "--quiet",
            ])
            .env("LD_PRELOAD", &preload)
            .env("MACHINE_GOD_REAP_MODE", mode)
            .env("MACHINE_GOD_REAP_MODE_TEST_ROOT", directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode} subprocess failed: {}; stdout: {}; stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!directory.path().join("must-not-run").exists());
    }
}

#[test]
fn external_reap_loses_authority_without_any_group_signal() {
    let directory = FreshDirectory::new("external-reap");
    let mut prepared = adapter()
        .prepare(request(directory.path(), "exit 0"))
        .unwrap();
    let controller = prepared.attach_signal_controller().unwrap();
    let mut owned = prepared.release().unwrap();
    owned.activate_signal_controller().unwrap();
    let leader = owned.pid();
    let pid = rustix::process::Pid::from_raw(i32::try_from(leader.get()).unwrap()).unwrap();
    rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::empty())
        .unwrap()
        .expect("external reaper retained the child");
    reset_group_signal_attempts_for_test(leader);

    let error = owned.wait().unwrap_err();

    assert_eq!(error.kind(), BackgroundProcessErrorKind::Wait);
    assert_eq!(group_signal_attempts_for_test(), 0);
    assert!(controller.is_closed_for_test());
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

#[cfg(target_os = "linux")]
struct EscapedProcessGuard {
    pid: rustix::process::Pid,
}

#[cfg(target_os = "linux")]
impl EscapedProcessGuard {
    fn new(pid: u32) -> Self {
        Self {
            pid: rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap(),
        }
    }

    fn kill(&self) {
        let _ = rustix::process::kill_process(self.pid, rustix::process::Signal::KILL);
    }

    fn kill_and_reap_if_adopted(&self) {
        self.kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match rustix::process::waitid(
                rustix::process::WaitId::Pid(self.pid),
                rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOHANG,
            ) {
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(2)),
                Ok(Some(_) | None) | Err(_) => return,
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for EscapedProcessGuard {
    fn drop(&mut self) {
        self.kill_and_reap_if_adopted();
    }
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn released_stdout_and_stderr_share_one_stream_without_the_readiness_marker() {
    let directory = FreshDirectory::new("merged-output");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "printf 'stdout-one'; printf 'stderr-two' >&2; printf 'stdout-three'",
        ))
        .unwrap()
        .release_with_output()
        .unwrap();
    let mut output = Vec::new();

    let outcome = owned
        .wait_with_stop_and_output(&CancellationToken::new(), |bytes| {
            output.extend_from_slice(bytes);
        })
        .unwrap();

    assert_eq!(
        outcome,
        BackgroundProcessOutcome::Completed(BackgroundProcessExit::Exited(0))
    );
    assert_eq!(output, b"stdout-onestderr-twostdout-three");
    assert!(
        !output.contains(&0xa7),
        "the helper readiness marker must remain private"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn ordinary_release_preserves_null_output_for_no_capture_callers() {
    let directory = FreshDirectory::new("null-output");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "i=0; while [ $i -lt 200000 ]; do printf '0123456789abcdef'; printf error >&2; i=$((i+1)); done",
        ))
        .unwrap()
        .release()
        .unwrap();
    let mut bytes = 0_usize;

    let outcome = owned
        .wait_with_stop_and_output(&CancellationToken::new(), |chunk| {
            bytes = bytes.saturating_add(chunk.len());
        })
        .unwrap();

    assert_eq!(
        outcome,
        BackgroundProcessOutcome::Completed(BackgroundProcessExit::Exited(0))
    );
    assert_eq!(bytes, 0);
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
    assert!(stop.cancel());
    assert_eq!(
        worker.join().unwrap().unwrap(),
        BackgroundProcessOutcome::Stopped
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
    let mut prepared = adapter()
        .prepare(request(
            directory.path(),
            "trap '' TERM; sleep 30 & printf '%s' \"$!\" > descendant.pid; while :; do :; done",
        ))
        .unwrap();
    let controller = prepared.attach_signal_controller().unwrap();
    let mut owned = prepared.release().unwrap();
    owned.activate_signal_controller().unwrap();
    let leader = owned.pid();
    let descendant = wait_for_pid(&pid_file);
    inject_waitid_failures_for_test(leader, 1);
    inject_group_snapshot_spawn_failures_for_test(leader, 1);

    let error = owned.wait().unwrap_err();

    assert_eq!(error.kind(), BackgroundProcessErrorKind::Wait);
    assert_processes_absent(&[leader.get(), descendant]);
    assert!(controller.is_closed_for_test());
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
fn merged_output_drains_an_unbounded_flood_without_backpressure() {
    let directory = FreshDirectory::new("flood");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "i=0; while [ $i -lt 200000 ]; do printf '0123456789abcdef'; i=$((i+1)); done",
        ))
        .unwrap()
        .release_with_output()
        .unwrap();
    let mut bytes = 0_u64;
    let outcome = owned
        .wait_with_stop_and_output(&CancellationToken::new(), |chunk| {
            bytes = bytes.saturating_add(chunk.len() as u64);
        })
        .unwrap();
    assert_eq!(
        outcome,
        BackgroundProcessOutcome::Completed(BackgroundProcessExit::Exited(0))
    );
    assert_eq!(bytes, 3_200_000);
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

#[cfg(target_os = "linux")]
#[test]
fn partial_term_delivery_cannot_hide_a_descendant_that_escapes_the_group() {
    let directory = FreshDirectory::new("escaped-descendant");
    let source = directory.path().join("escape.c");
    let executable = directory.path().join("escape");
    fs::write(
        &source,
        br#"#include <signal.h>
#include <stdio.h>
#include <unistd.h>
static volatile sig_atomic_t leave_group = 0;
static void on_term(int signal) { (void)signal; leave_group = 1; }
int main(void) {
  pid_t child = fork();
  if (child < 0) return 80;
  if (child == 0) {
    struct sigaction action = {0};
    action.sa_handler = on_term;
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGTERM, &action, 0) != 0) return 81;
    FILE *marker = fopen("escaped.pid.tmp", "w");
    if (!marker || fprintf(marker, "%d", (int)getpid()) < 1 || fclose(marker) != 0) return 82;
    if (rename("escaped.pid.tmp", "escaped.pid") != 0) return 83;
    while (!leave_group) pause();
    if (setpgid(0, 0) != 0) return 84;
    for (;;) pause();
  }
  signal(SIGTERM, SIG_IGN);
  for (;;) pause();
}
"#,
    )
    .unwrap();
    assert!(
        Command::new("cc")
            .args(["-Wall", "-Wextra", "-Werror", "-o"])
            .arg(&executable)
            .arg(&source)
            .status()
            .unwrap()
            .success()
    );
    let owned = adapter()
        .prepare(request(directory.path(), "exec ./escape"))
        .unwrap()
        .release()
        .unwrap();
    let escaped = wait_for_pid(&directory.path().join("escaped.pid"));
    let escaped_guard = EscapedProcessGuard::new(escaped);

    let error = owned.stop().unwrap_err();

    assert_eq!(error.kind(), BackgroundProcessErrorKind::Cleanup);
    assert!(
        process_exists(escaped),
        "escaped identity was mistaken for gone"
    );
    escaped_guard.kill_and_reap_if_adopted();
    assert_processes_absent(&[escaped]);
}

#[cfg(target_os = "linux")]
#[test]
fn unobserved_session_escape_is_outside_bounded_cleanup_ownership() {
    let directory = FreshDirectory::new("unobserved-session-escape");
    let source = directory.path().join("escape.c");
    let executable = directory.path().join("escape");
    fs::write(
        &source,
        br#"#include <signal.h>
#include <stdio.h>
#include <unistd.h>
int main(void) {
  pid_t child = fork();
  if (child < 0) return 80;
  if (child == 0) {
    if (setsid() < 0) return 81;
    FILE *marker = fopen("escaped.pid.tmp", "w");
    if (!marker || fprintf(marker, "%d", (int)getpid()) < 1 || fclose(marker) != 0) return 82;
    if (rename("escaped.pid.tmp", "escaped.pid") != 0) return 83;
    for (;;) pause();
  }
  signal(SIGTERM, SIG_IGN);
  for (;;) pause();
}
"#,
    )
    .unwrap();
    assert!(
        Command::new("cc")
            .args(["-Wall", "-Wextra", "-Werror", "-o"])
            .arg(&executable)
            .arg(&source)
            .status()
            .unwrap()
            .success()
    );
    let owned = adapter()
        .prepare(request(directory.path(), "exec ./escape"))
        .unwrap()
        .release()
        .unwrap();
    let leader = owned.pid().get();
    let escaped = wait_for_pid(&directory.path().join("escaped.pid"));
    let escaped_guard = EscapedProcessGuard::new(escaped);

    owned.stop().unwrap();

    assert_processes_absent(&[leader]);
    assert!(
        process_exists(escaped),
        "an unobserved session escape is outside the bounded ownership set"
    );
    escaped_guard.kill_and_reap_if_adopted();
    assert_processes_absent(&[escaped]);
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
fn cancellable_output_wait_drains_before_stop_and_reaps() {
    let directory = FreshDirectory::new("cancel-output-wait");
    let ready = directory.path().join("ready");
    let owned = adapter()
        .prepare(request(
            directory.path(),
            "printf captured; printf ready > ready; trap '' TERM; while :; do /bin/sleep 1; done",
        ))
        .unwrap()
        .release_cancellable_with_output(&CancellationToken::new())
        .unwrap();
    let pid = owned.pid().get();
    let stop = CancellationToken::new();
    let worker_stop = stop.clone();
    let worker = thread::spawn(move || {
        let mut output = Vec::new();
        let outcome = owned.wait_with_stop_and_output(&worker_stop, |bytes| {
            output.extend_from_slice(bytes);
        });
        (outcome, output)
    });
    wait_for(&ready);
    assert!(stop.cancel());

    let (outcome, output) = worker.join().unwrap();
    assert_eq!(outcome.unwrap(), BackgroundProcessOutcome::Stopped);
    assert_eq!(output, b"captured");
    assert!(!process_exists(pid));
}

#[test]
fn attached_signal_controller_is_hidden_until_owned_activation_and_closes_after_wait() {
    let directory = FreshDirectory::new("signal-controller");
    let ready = directory.path().join("ready");
    let terminated = directory.path().join("terminated");
    let mut prepared = adapter()
        .prepare(request(
            directory.path(),
            "trap 'printf terminated > terminated; exit 23' TERM; printf ready > ready; while :; do /bin/sleep 1; done",
        ))
        .unwrap();
    let controller = prepared.attach_signal_controller().unwrap();
    assert_eq!(
        prepared
            .attach_signal_controller()
            .err()
            .expect("a second attachment fails")
            .kind(),
        BackgroundProcessSignalErrorKind::AlreadyAttached
    );
    assert_eq!(
        controller
            .signal(BackgroundProcessSignal::Terminate)
            .unwrap_err()
            .kind(),
        BackgroundProcessSignalErrorKind::NotFound
    );

    let mut owned = prepared.release().unwrap();
    let pid = owned.pid().get();
    assert_eq!(
        controller
            .signal(BackgroundProcessSignal::Terminate)
            .unwrap_err()
            .kind(),
        BackgroundProcessSignalErrorKind::NotFound,
        "release does not expose guessed-ID signal authority"
    );
    owned.activate_signal_controller().unwrap();
    wait_for(&ready);
    controller
        .signal(BackgroundProcessSignal::Terminate)
        .unwrap();

    assert_eq!(owned.wait().unwrap(), BackgroundProcessExit::Exited(23));
    assert_eq!(fs::read_to_string(terminated).unwrap(), "terminated");
    assert!(!process_exists(pid), "normal wait must reap the leader");
    assert_eq!(
        controller
            .signal(BackgroundProcessSignal::Kill)
            .unwrap_err()
            .kind(),
        BackgroundProcessSignalErrorKind::NotFound,
        "normal exit synchronously closes retained signal clones"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn signal_controller_delivers_to_a_descendant_that_escaped_with_setsid() {
    let directory = FreshDirectory::new("signal-escaped-descendant");
    let source = directory.path().join("signal-tree.c");
    let executable = directory.path().join("signal-tree");
    fs::write(
        &source,
        br#"#include <signal.h>
#include <stdio.h>
#include <unistd.h>
static volatile sig_atomic_t terminated = 0;
static void on_term(int signal) { (void)signal; terminated = 1; }
static int write_marker(const char *path, const char *value) {
  FILE *marker = fopen(path, "w");
  if (!marker || fputs(value, marker) < 0 || fclose(marker) != 0) return 1;
  return 0;
}
int main(void) {
  pid_t child = fork();
  if (child < 0) return 80;
  struct sigaction action = {0};
  action.sa_handler = on_term;
  sigemptyset(&action.sa_mask);
  if (sigaction(SIGTERM, &action, 0) != 0) return 81;
  if (child == 0) {
    if (setsid() < 0) return 82;
    char pid[32];
    if (snprintf(pid, sizeof(pid), "%d", (int)getpid()) < 1) return 83;
    if (write_marker("escaped.pid", pid) != 0) return 84;
    while (!terminated) pause();
    return write_marker("escaped.signaled", "yes") == 0 ? 24 : 85;
  }
  while (!terminated) pause();
  return write_marker("leader.signaled", "yes") == 0 ? 23 : 86;
}
"#,
    )
    .unwrap();
    assert!(
        Command::new("cc")
            .args(["-Wall", "-Wextra", "-Werror", "-o"])
            .arg(&executable)
            .arg(&source)
            .status()
            .unwrap()
            .success()
    );

    let mut prepared = adapter()
        .prepare(request(directory.path(), "exec ./signal-tree"))
        .unwrap();
    let controller = prepared.attach_signal_controller().unwrap();
    let mut owned = prepared.release().unwrap();
    owned.activate_signal_controller().unwrap();
    let escaped = wait_for_pid(&directory.path().join("escaped.pid"));
    #[cfg(target_os = "linux")]
    let escaped_guard = EscapedProcessGuard::new(escaped);

    controller
        .signal(BackgroundProcessSignal::Terminate)
        .expect("identity-safe tree signal is delivered");
    wait_for(&directory.path().join("escaped.signaled"));
    wait_for(&directory.path().join("leader.signaled"));
    assert_eq!(owned.wait().unwrap(), BackgroundProcessExit::Exited(23));
    #[cfg(target_os = "linux")]
    escaped_guard.kill_and_reap_if_adopted();
    assert_processes_absent(&[escaped]);
}

#[test]
fn owned_wait_closes_signal_gate_before_reaping_the_identity_pin() {
    let directory = FreshDirectory::new("signal-close-before-reap");
    let ready = directory.path().join("ready");
    let mut prepared = adapter()
        .prepare(request(directory.path(), "printf ready > ready; exit 0"))
        .unwrap();
    let controller = prepared.attach_signal_controller().unwrap();
    let mut owned = prepared.release().unwrap();
    let pid = owned.pid().get();
    owned.activate_signal_controller().unwrap();
    wait_for(&ready);
    let gate = controller.hold_gate_for_test();

    let waiter = thread::spawn(move || owned.wait());
    thread::sleep(Duration::from_millis(100));
    assert!(
        process_exists(pid),
        "the retained leader was reaped while lifecycle close was blocked"
    );
    drop(gate);

    assert_eq!(
        waiter.join().unwrap().unwrap(),
        BackgroundProcessExit::Exited(0)
    );
    assert!(!process_exists(pid));
    assert_eq!(
        controller
            .signal(BackgroundProcessSignal::Interrupt)
            .unwrap_err()
            .kind(),
        BackgroundProcessSignalErrorKind::NotFound
    );
}

#[test]
fn abort_stop_and_drop_close_all_attached_signal_clones() {
    let directory = FreshDirectory::new("signal-lifecycle-close");

    let mut prepared = adapter()
        .prepare(request(directory.path(), "sleep 30"))
        .unwrap();
    let aborted = prepared.attach_signal_controller().unwrap();
    prepared.abort_and_reap().unwrap();
    assert!(aborted.is_closed_for_test());

    let mut prepared = adapter()
        .prepare(request(directory.path(), "sleep 30"))
        .unwrap();
    let stopped = prepared.attach_signal_controller().unwrap();
    let mut owned = prepared.release().unwrap();
    owned.activate_signal_controller().unwrap();
    owned.stop().unwrap();
    assert!(stopped.is_closed_for_test());

    let mut prepared = adapter()
        .prepare(request(directory.path(), "sleep 30"))
        .unwrap();
    let dropped = prepared.attach_signal_controller().unwrap();
    let mut owned = prepared.release().unwrap();
    owned.activate_signal_controller().unwrap();
    drop(owned);
    assert!(dropped.is_closed_for_test());
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

    let output_ready = directory.path().join("output-ready");
    let owned: OwnedBackgroundProcess = adapter()
        .prepare(request(
            directory.path(),
            "printf ready > output-ready; while :; do printf '0123456789abcdef'; done",
        ))
        .unwrap()
        .release()
        .unwrap();
    let owned_pid = owned.pid().get();
    wait_for(&output_ready);
    thread::sleep(Duration::from_millis(20));
    drop(owned);
    assert!(!process_exists(owned_pid));
}
