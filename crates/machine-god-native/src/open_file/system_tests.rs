use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};

use rustix::fd::AsRawFd;
use rustix::fs::{Mode, OFlags};

use super::{
    ACTIVE_SYSTEM_LAUNCHES, BeforeSpawnHook, MAX_CONCURRENT_OPEN_FILE_LAUNCHES,
    OPEN_FILE_LAUNCH_TIMEOUT, OpenFileLaunch, OpenFileLaunchOutcome, OpenFileLaunchRequest,
    OpenFileLauncher, SystemLaunchConfig, SystemLaunchPermit, SystemOpenFileLauncher, WorkerHandle,
};
use machine_god_core::{CancellationToken, ToolErrorKind};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-open-file-system-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create temporary directory: {error}"),
            }
        }
        panic!("could not allocate a unique temporary directory");
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("remove temporary directory");
    }
}

struct ThreadWake(thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

fn waker() -> Waker {
    Waker::from(Arc::new(ThreadWake(thread::current())))
}

fn poll_once(future: &mut super::OpenFileLaunch, waker: &Waker) -> Poll<OpenFileLaunchOutcome> {
    Future::poll(future.as_mut(), &mut Context::from_waker(waker))
}

fn drive_to_completion(
    future: &mut super::OpenFileLaunch,
    deadline: Instant,
) -> OpenFileLaunchOutcome {
    let waker = waker();
    loop {
        if let Poll::Ready(outcome) = poll_once(future, &waker) {
            return outcome;
        }
        assert!(Instant::now() < deadline, "launcher future timed out");
        thread::park_timeout(Duration::from_millis(10));
    }
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() {
        assert!(Instant::now() < deadline, "helper marker was not created");
        thread::sleep(Duration::from_millis(5));
    }
}

fn write_script(directory: &Path, body: &str) -> PathBuf {
    let path = directory.join("controlled-helper");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write controlled helper");
    let mut permissions = fs::metadata(&path)
        .expect("inspect controlled helper")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).expect("make controlled helper executable");
    path
}

fn request(directory: &Path) -> OpenFileLaunchRequest {
    let target_path = directory.join("target.txt");
    fs::write(&target_path, b"descriptor target").expect("write descriptor target");
    let target = rustix::fs::open(
        &target_path,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .expect("retain descriptor target");
    let proc_path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        target.as_raw_fd()
    ));
    OpenFileLaunchRequest {
        path: "target.txt".to_owned(),
        proc_path,
        target,
    }
}

fn launcher(program: PathBuf, timeout: Duration) -> SystemOpenFileLauncher {
    launcher_with_test_controls(program, timeout, LauncherTestControls::default())
}

#[derive(Default)]
struct LauncherTestControls {
    before_spawn: Option<Arc<BeforeSpawnHook>>,
    before_first_wait: Option<Arc<BeforeSpawnHook>>,
    after_wait_probe: Option<Arc<BeforeSpawnHook>>,
    before_forced_wait_failure: Option<Arc<BeforeSpawnHook>>,
    after_publish: Option<Arc<BeforeSpawnHook>>,
    force_wait_failure: bool,
}

fn launcher_with_test_controls(
    program: PathBuf,
    timeout: Duration,
    controls: LauncherTestControls,
) -> SystemOpenFileLauncher {
    SystemOpenFileLauncher {
        config: Arc::new(SystemLaunchConfig {
            program,
            current_dir: PathBuf::from("/"),
            timeout,
            before_spawn: controls.before_spawn,
            before_first_wait: controls.before_first_wait,
            after_wait_probe: controls.after_wait_probe,
            before_forced_wait_failure: controls.before_forced_wait_failure,
            after_publish: controls.after_publish,
            force_wait_failure: controls.force_wait_failure,
        }),
    }
}

fn read_pid(path: &Path) -> u32 {
    fs::read_to_string(path)
        .expect("read helper pid")
        .trim()
        .parse()
        .expect("parse helper pid")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcIdentity {
    device: u64,
    inode: u64,
}

fn proc_identity(path: &Path) -> ProcIdentity {
    let stat = rustix::fs::stat(path).expect("inspect proc identity");
    ProcIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
    }
}

fn assert_proc_identity_released(path: &Path, original: ProcIdentity) {
    match rustix::fs::stat(path) {
        Err(error) if error == rustix::io::Errno::NOENT => {}
        Ok(stat) => assert_ne!(
            ProcIdentity {
                device: stat.st_dev,
                inode: stat.st_ino,
            },
            original,
            "original proc identity remains at {}",
            path.display()
        ),
        Err(error) => panic!("inspect released proc identity {}: {error}", path.display()),
    }
}

fn process_identity(pid: u32) -> (PathBuf, ProcIdentity) {
    let path = PathBuf::from(format!("/proc/{pid}"));
    let identity = proc_identity(&path);
    (path, identity)
}

fn wait_for_zombie(path: &Path) {
    let stat_path = path.join("stat");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let stat = fs::read_to_string(&stat_path).expect("read helper process state");
        let (_, tail) = stat
            .rsplit_once(") ")
            .expect("helper process stat has a state field");
        if tail.starts_with("Z ") {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "helper did not become a zombie before the test deadline"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_active_launches(expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while ACTIVE_SYSTEM_LAUNCHES.load(Ordering::Acquire) != expected {
        assert!(
            Instant::now() < deadline,
            "active system launch count did not become {expected}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

struct ReentrantWake {
    future: Arc<Mutex<Option<OpenFileLaunch>>>,
    outcome: Arc<Mutex<Option<OpenFileLaunchOutcome>>>,
    waiting_thread: thread::Thread,
}

impl Wake for ReentrantWake {
    fn wake(self: Arc<Self>) {
        let waker = Waker::from(Arc::clone(&self));
        let mut future = self
            .future
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let outcome = match poll_once(future.as_mut().expect("launch future exists"), &waker) {
            Poll::Ready(outcome) => outcome,
            Poll::Pending => panic!("published worker outcome must complete inline repoll"),
        };
        let completed = future.take().expect("completed launch future exists");
        drop(completed);
        *self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
        self.waiting_thread.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        Arc::clone(self).wake();
    }
}

struct BlockingWake(Arc<BeforeSpawnHook>);

impl Wake for BlockingWake {
    fn wake(self: Arc<Self>) {
        self.0.pause_worker();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.pause_worker();
    }
}

fn process_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
fn production_configuration_is_the_exact_fixed_launcher_contract() {
    let launcher = SystemOpenFileLauncher::default();
    assert_eq!(MAX_CONCURRENT_OPEN_FILE_LAUNCHES, 32);
    assert_eq!(launcher.config.program, Path::new("/usr/bin/xdg-open"));
    assert_eq!(launcher.config.current_dir, Path::new("/"));
    assert_eq!(launcher.config.timeout, OPEN_FILE_LAUNCH_TIMEOUT);
    assert!(launcher.config.before_spawn.is_none());
    assert!(launcher.config.before_first_wait.is_none());
    assert!(launcher.config.after_wait_probe.is_none());
    assert!(launcher.config.before_forced_wait_failure.is_none());
    assert!(launcher.config.after_publish.is_none());
    assert!(!launcher.config.force_wait_failure);
}

#[test]
fn saturated_global_limit_starts_no_extra_worker_or_helper_and_releases_cleanly() {
    let _lock = process_test_lock();
    wait_for_active_launches(0);
    let permits = (0..MAX_CONCURRENT_OPEN_FILE_LAUNCHES)
        .map(|_| SystemLaunchPermit::acquire().expect("launch permit below fixed limit"))
        .collect::<Vec<_>>();
    assert_eq!(
        ACTIVE_SYSTEM_LAUNCHES.load(Ordering::Acquire),
        MAX_CONCURRENT_OPEN_FILE_LAUNCHES
    );
    assert!(SystemLaunchPermit::acquire().is_none());

    let temporary = TemporaryDirectory::new();
    let marker = temporary.path().join("started");
    let script = write_script(
        temporary.path(),
        &format!("printf started > '{}'", marker.display()),
    );
    let launch_request = request(temporary.path());
    let target_proc_path = launch_request.proc_path.clone();
    let target_identity = proc_identity(&target_proc_path);
    let mut saturated = launcher(script.clone(), Duration::from_secs(2))
        .launch(launch_request, CancellationToken::new());

    assert_eq!(
        drive_to_completion(&mut saturated, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::Unavailable
    );
    assert!(!marker.exists());
    assert_proc_identity_released(&target_proc_path, target_identity);
    assert_eq!(
        ACTIVE_SYSTEM_LAUNCHES.load(Ordering::Acquire),
        MAX_CONCURRENT_OPEN_FILE_LAUNCHES
    );

    drop(permits);
    wait_for_active_launches(0);
    let mut admitted = launcher(script, Duration::from_secs(2))
        .launch(request(temporary.path()), CancellationToken::new());
    assert_eq!(
        drive_to_completion(&mut admitted, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::Accepted
    );
    assert_eq!(
        fs::read_to_string(marker).expect("read launch marker"),
        "started"
    );
    wait_for_active_launches(0);
}

#[test]
fn cancelled_failed_precommit_operation_reports_cancellation_without_mapping_failure() {
    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    let mapped = AtomicBool::new(false);

    let error = super::finish_precommit_operation::<(), ()>(Err(()), &cancellation, |()| {
        mapped.store(true, Ordering::Relaxed);
        super::unavailable()
    })
    .expect_err("cancellation must win over the failed operation");

    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "open_file_cancelled");
    assert!(!mapped.load(Ordering::Relaxed));
}

#[test]
fn controlled_helper_receives_exact_target_cwd_environment_and_null_streams() {
    let _lock = process_test_lock();
    let temporary = TemporaryDirectory::new();
    let record = temporary.path().join("record");
    let script = write_script(
        temporary.path(),
        &format!(
            r#"stdin_target=$(readlink /proc/$$/fd/0)
stdout_target=$(readlink /proc/$$/fd/1)
stderr_target=$(readlink /proc/$$/fd/2)
{{
  printf 'argument_count=%s\n' "$#"
  printf 'argument=%s\n' "$1"
  printf 'cwd=%s\n' "$PWD"
  printf 'stdin=%s\n' "$stdin_target"
  printf 'stdout=%s\n' "$stdout_target"
  printf 'stderr=%s\n' "$stderr_target"
  printf 'path=%s\n' "$PATH"
}} > '{}'
exit 0"#,
            record.display()
        ),
    );
    let request = request(temporary.path());
    let expected_proc_path = request.proc_path.clone();
    let mut future =
        launcher(script, Duration::from_secs(2)).launch(request, CancellationToken::new());

    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(3)),
        OpenFileLaunchOutcome::Accepted
    );

    let contents = fs::read_to_string(record).expect("read helper record");
    let lines = contents.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "argument_count=1");
    assert_eq!(
        lines[1],
        format!("argument={}", expected_proc_path.display())
    );
    assert_eq!(lines[2], "cwd=/");
    assert_eq!(lines[3], "stdin=/dev/null");
    assert_eq!(lines[4], "stdout=/dev/null");
    assert_eq!(lines[5], "stderr=/dev/null");
    assert_eq!(
        lines[6],
        format!("path={}", std::env::var("PATH").expect("test PATH exists"))
    );
}

#[test]
fn launch_future_is_inert_and_precancellation_never_starts_the_helper() {
    let _lock = process_test_lock();
    let temporary = TemporaryDirectory::new();
    let marker = temporary.path().join("started");
    let script = write_script(
        temporary.path(),
        &format!("printf started > '{}'", marker.display()),
    );
    let launcher = launcher(script, Duration::from_secs(2));

    let future = launcher.launch(request(temporary.path()), CancellationToken::new());
    drop(future);
    assert!(!marker.exists());

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    let mut future = launcher.launch(request(temporary.path()), cancellation);
    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::Cancelled
    );
    assert!(!marker.exists());
}

#[test]
fn cancellation_before_the_serialized_spawn_gate_never_starts_the_helper() {
    let _lock = process_test_lock();
    let temporary = TemporaryDirectory::new();
    let marker = temporary.path().join("started");
    let script = write_script(
        temporary.path(),
        &format!("printf started > '{}'", marker.display()),
    );
    let hook = Arc::new(BeforeSpawnHook::new());
    let launcher = launcher_with_test_controls(
        script,
        Duration::from_secs(2),
        LauncherTestControls {
            before_spawn: Some(Arc::clone(&hook)),
            ..LauncherTestControls::default()
        },
    );
    let cancellation = CancellationToken::new();
    let mut future = launcher.launch(request(temporary.path()), cancellation.clone());
    let waker = waker();

    assert!(poll_once(&mut future, &waker).is_pending());
    hook.reached.wait();
    assert!(cancellation.cancel());
    hook.release.wait();

    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::Cancelled
    );
    assert!(!marker.exists());
}

#[test]
fn spawn_failure_and_nonzero_exit_preserve_the_commit_boundary() {
    let _lock = process_test_lock();
    let temporary = TemporaryDirectory::new();
    let missing = temporary.path().join("missing-helper");
    let mut future = launcher(missing, Duration::from_secs(2))
        .launch(request(temporary.path()), CancellationToken::new());
    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::Unavailable
    );

    let script = write_script(temporary.path(), "exit 7");
    let mut future = launcher(script, Duration::from_secs(2))
        .launch(request(temporary.path()), CancellationToken::new());
    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::ResultUnknown
    );

    let script = write_script(temporary.path(), "kill -TERM \"$$\"");
    let mut future = launcher(script, Duration::from_secs(2))
        .launch(request(temporary.path()), CancellationToken::new());
    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::ResultUnknown
    );
}

#[test]
fn helper_known_to_exit_after_deadline_is_rejected_before_the_first_wait_decision() {
    let _lock = process_test_lock();
    let temporary = TemporaryDirectory::new();
    let pid_record = temporary.path().join("pid");
    let exited = temporary.path().join("exited");
    let script = write_script(
        temporary.path(),
        &format!(
            "printf '%s\\n' \"$$\" > '{}'\nsleep 0.10\nprintf exited > '{}'",
            pid_record.display(),
            exited.display()
        ),
    );
    let before_first_wait = Arc::new(BeforeSpawnHook::new());
    let mut future = launcher_with_test_controls(
        script,
        Duration::from_millis(25),
        LauncherTestControls {
            before_first_wait: Some(Arc::clone(&before_first_wait)),
            ..LauncherTestControls::default()
        },
    )
    .launch(request(temporary.path()), CancellationToken::new());
    let waker = waker();

    assert!(poll_once(&mut future, &waker).is_pending());
    before_first_wait.reached.wait();
    wait_for_file(&exited);
    let pid = read_pid(&pid_record);
    let (process_path, original_identity) = process_identity(pid);
    before_first_wait.release.wait();

    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::ResultUnknown
    );
    assert_proc_identity_released(&process_path, original_identity);
}

#[test]
fn successful_wait_probe_crossing_deadline_before_observation_is_rejected_and_reaped() {
    let _lock = process_test_lock();
    let temporary = TemporaryDirectory::new();
    let pid_record = temporary.path().join("pid");
    let script = write_script(
        temporary.path(),
        &format!("printf '%s\\n' \"$$\" > '{}'\nexit 0", pid_record.display()),
    );
    let timeout = Duration::from_secs(1);
    let before_first_wait = Arc::new(BeforeSpawnHook::new());
    let after_wait_probe = Arc::new(BeforeSpawnHook::new());
    let mut future = launcher_with_test_controls(
        script,
        timeout,
        LauncherTestControls {
            before_first_wait: Some(Arc::clone(&before_first_wait)),
            after_wait_probe: Some(Arc::clone(&after_wait_probe)),
            ..LauncherTestControls::default()
        },
    )
    .launch(request(temporary.path()), CancellationToken::new());
    let waker = waker();

    assert!(poll_once(&mut future, &waker).is_pending());
    before_first_wait.reached.wait();
    wait_for_file(&pid_record);
    let pid = read_pid(&pid_record);
    let (process_path, original_identity) = process_identity(pid);
    wait_for_zombie(&process_path);
    before_first_wait.release.wait();

    // Reaching this seam proves that the pre-probe deadline guard passed and
    // that the real `try_wait` already observed the exit-zero child.
    after_wait_probe.reached.wait();
    thread::sleep(timeout + Duration::from_millis(50));
    after_wait_probe.release.wait();

    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::ResultUnknown
    );
    assert_proc_identity_released(&process_path, original_identity);
}

#[test]
fn checked_wait_error_returns_unknown_and_terminates_and_reaps_the_helper() {
    let _lock = process_test_lock();
    let temporary = TemporaryDirectory::new();
    let pid_record = temporary.path().join("pid");
    let script = write_script(
        temporary.path(),
        &format!(
            "printf '%s\\n' \"$$\" > '{}'\nwhile :; do :; done",
            pid_record.display()
        ),
    );
    let before_forced_wait_failure = Arc::new(BeforeSpawnHook::new());
    let mut future = launcher_with_test_controls(
        script,
        Duration::from_secs(2),
        LauncherTestControls {
            before_forced_wait_failure: Some(Arc::clone(&before_forced_wait_failure)),
            force_wait_failure: true,
            ..LauncherTestControls::default()
        },
    )
    .launch(request(temporary.path()), CancellationToken::new());
    let waker = waker();

    assert!(poll_once(&mut future, &waker).is_pending());
    before_forced_wait_failure.reached.wait();
    wait_for_file(&pid_record);
    let pid = read_pid(&pid_record);
    let (process_path, original_identity) = process_identity(pid);
    before_forced_wait_failure.release.wait();
    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::ResultUnknown
    );
    assert_proc_identity_released(&process_path, original_identity);
}

#[test]
fn no_waker_published_outcome_joins_the_blocked_worker_before_returning() {
    let _lock = process_test_lock();
    wait_for_active_launches(0);
    let temporary = TemporaryDirectory::new();
    let script = write_script(temporary.path(), "exit 0");
    let after_publish = Arc::new(BeforeSpawnHook::new());
    let launcher = launcher_with_test_controls(
        script,
        Duration::from_secs(2),
        LauncherTestControls {
            after_publish: Some(Arc::clone(&after_publish)),
            ..LauncherTestControls::default()
        },
    );
    let mut worker = WorkerHandle::spawn(
        request(temporary.path()),
        CancellationToken::new(),
        Arc::clone(&launcher.config),
    )
    .expect("worker starts below the fixed launch limit");

    after_publish.reached.wait();
    assert_eq!(ACTIVE_SYSTEM_LAUNCHES.load(Ordering::Acquire), 1);
    let waker = waker();
    let outcome = worker
        .poll_outcome(&Context::from_waker(&waker))
        .expect("published worker outcome is ready without registering a waker");
    assert_eq!(outcome, OpenFileLaunchOutcome::Accepted);

    let (completed, observed_completion) = std::sync::mpsc::channel();
    let join_thread = thread::spawn(move || {
        completed
            .send(worker.join_finished(outcome))
            .expect("report joined worker outcome");
    });
    assert!(
        observed_completion
            .recv_timeout(Duration::from_millis(250))
            .is_err(),
        "join_finished returned before the published worker completed"
    );
    assert_eq!(
        ACTIVE_SYSTEM_LAUNCHES.load(Ordering::Acquire),
        1,
        "published worker released its permit before returning"
    );

    after_publish.release.wait();
    assert_eq!(
        observed_completion
            .recv_timeout(Duration::from_secs(2))
            .expect("join_finished returns after the worker is released"),
        OpenFileLaunchOutcome::Accepted
    );
    join_thread.join().expect("join_finished thread succeeds");
    wait_for_active_launches(0);
}

#[test]
fn inline_reentrant_wake_completes_on_the_worker_without_self_joining() {
    let _lock = process_test_lock();
    let temporary = TemporaryDirectory::new();
    let script = write_script(temporary.path(), "exit 0");
    let launcher = launcher(script, Duration::from_secs(2));
    let future = Arc::new(Mutex::new(Some(
        launcher.launch(request(temporary.path()), CancellationToken::new()),
    )));
    let outcome = Arc::new(Mutex::new(None));
    let waker = Waker::from(Arc::new(ReentrantWake {
        future: Arc::clone(&future),
        outcome: Arc::clone(&outcome),
        waiting_thread: thread::current(),
    }));

    assert!(
        poll_once(
            future
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_mut()
                .expect("launch future exists"),
            &waker,
        )
        .is_pending()
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(completed) = outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            assert_eq!(completed, OpenFileLaunchOutcome::Accepted);
            break;
        }
        assert!(Instant::now() < deadline, "inline wake timed out");
        thread::park_timeout(Duration::from_millis(10));
    }
}

#[test]
fn blocked_wake_releases_request_before_publication_and_holds_permit_until_worker_return() {
    let _lock = process_test_lock();
    wait_for_active_launches(0);
    let temporary = TemporaryDirectory::new();
    let script = write_script(temporary.path(), "exit 0");
    let before_first_wait = Arc::new(BeforeSpawnHook::new());
    let launcher = launcher_with_test_controls(
        script,
        Duration::from_secs(2),
        LauncherTestControls {
            before_first_wait: Some(Arc::clone(&before_first_wait)),
            ..LauncherTestControls::default()
        },
    );
    let wake_hook = Arc::new(BeforeSpawnHook::new());
    let waker = Waker::from(Arc::new(BlockingWake(Arc::clone(&wake_hook))));
    let launch_request = request(temporary.path());
    let target_proc_path = launch_request.proc_path.clone();
    let target_identity = proc_identity(&target_proc_path);
    let mut future = launcher.launch(launch_request, CancellationToken::new());

    assert!(poll_once(&mut future, &waker).is_pending());
    before_first_wait.reached.wait();
    before_first_wait.release.wait();
    wake_hook.reached.wait();
    assert_proc_identity_released(&target_proc_path, target_identity);
    assert_eq!(ACTIVE_SYSTEM_LAUNCHES.load(Ordering::Acquire), 1);
    let tail_saturation_permits = (1..MAX_CONCURRENT_OPEN_FILE_LAUNCHES)
        .map(|_| SystemLaunchPermit::acquire().expect("permit below blocked-tail limit"))
        .collect::<Vec<_>>();
    assert_eq!(
        ACTIVE_SYSTEM_LAUNCHES.load(Ordering::Acquire),
        MAX_CONCURRENT_OPEN_FILE_LAUNCHES
    );
    assert!(SystemLaunchPermit::acquire().is_none());

    let (dropped, observed_drop) = std::sync::mpsc::channel();
    let drop_thread = thread::spawn(move || {
        drop(future);
        dropped.send(()).expect("report completed future drop");
    });
    let drop_result = observed_drop.recv_timeout(Duration::from_millis(250));
    assert!(
        drop_result.is_ok(),
        "future drop joined a worker blocked in its wake callback"
    );
    assert_eq!(
        ACTIVE_SYSTEM_LAUNCHES.load(Ordering::Acquire),
        MAX_CONCURRENT_OPEN_FILE_LAUNCHES,
        "blocked wake tail released its active launch permit too early"
    );

    wake_hook.release.wait();
    drop_thread.join().expect("future drop thread succeeds");
    wait_for_active_launches(MAX_CONCURRENT_OPEN_FILE_LAUNCHES - 1);
    drop(tail_saturation_permits);
    wait_for_active_launches(0);
}

#[test]
fn timeout_cancellation_and_drop_terminate_reap_and_join_the_direct_helper() {
    let _lock = process_test_lock();
    let temporary = TemporaryDirectory::new();
    let pid_record = temporary.path().join("pid");
    let script = write_script(
        temporary.path(),
        &format!(
            "printf '%s\\n' \"$$\" > '{}'
while :; do :; done",
            pid_record.display()
        ),
    );

    let waker = waker();
    let mut future = launcher(script.clone(), Duration::from_millis(200))
        .launch(request(temporary.path()), CancellationToken::new());
    assert!(poll_once(&mut future, &waker).is_pending());
    wait_for_file(&pid_record);
    let timeout_pid = read_pid(&pid_record);
    let (timeout_process_path, timeout_identity) = process_identity(timeout_pid);
    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::ResultUnknown
    );
    assert_proc_identity_released(&timeout_process_path, timeout_identity);

    fs::remove_file(&pid_record).expect("remove timeout pid record");
    let cancellation = CancellationToken::new();
    let mut future = launcher(script.clone(), Duration::from_secs(2))
        .launch(request(temporary.path()), cancellation.clone());
    assert!(poll_once(&mut future, &waker).is_pending());
    wait_for_file(&pid_record);
    let pid = read_pid(&pid_record);
    let (process_path, original_identity) = process_identity(pid);
    assert!(cancellation.cancel());
    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::ResultUnknown
    );
    assert_proc_identity_released(&process_path, original_identity);

    fs::remove_file(&pid_record).expect("remove cancellation pid record");
    let mut future = launcher(script, Duration::from_secs(2))
        .launch(request(temporary.path()), CancellationToken::new());
    assert!(poll_once(&mut future, &waker).is_pending());
    wait_for_file(&pid_record);
    let pid = read_pid(&pid_record);
    let (process_path, original_identity) = process_identity(pid);
    drop(future);
    assert_proc_identity_released(&process_path, original_identity);
}
