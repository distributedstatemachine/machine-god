use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};

use rustix::fd::AsRawFd;
use rustix::fs::{Mode, OFlags};

use super::{
    OPEN_FILE_LAUNCH_TIMEOUT, OpenFileLaunchOutcome, OpenFileLaunchRequest, OpenFileLauncher,
    SystemLaunchConfig, SystemOpenFileLauncher,
};
use machine_god_core::CancellationToken;

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
    SystemOpenFileLauncher {
        config: Arc::new(SystemLaunchConfig {
            program,
            current_dir: PathBuf::from("/"),
            timeout,
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

fn assert_process_reaped(pid: u32) {
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "direct helper process {pid} remains"
    );
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
    assert_eq!(launcher.config.program, Path::new("/usr/bin/xdg-open"));
    assert_eq!(launcher.config.current_dir, Path::new("/"));
    assert_eq!(launcher.config.timeout, OPEN_FILE_LAUNCH_TIMEOUT);
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

    let mut future = launcher(script.clone(), Duration::from_millis(40))
        .launch(request(temporary.path()), CancellationToken::new());
    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::ResultUnknown
    );
    assert_process_reaped(read_pid(&pid_record));

    fs::remove_file(&pid_record).expect("remove timeout pid record");
    let cancellation = CancellationToken::new();
    let mut future = launcher(script.clone(), Duration::from_secs(2))
        .launch(request(temporary.path()), cancellation.clone());
    let waker = waker();
    assert!(poll_once(&mut future, &waker).is_pending());
    wait_for_file(&pid_record);
    let pid = read_pid(&pid_record);
    assert!(cancellation.cancel());
    assert_eq!(
        drive_to_completion(&mut future, Instant::now() + Duration::from_secs(2)),
        OpenFileLaunchOutcome::ResultUnknown
    );
    assert_process_reaped(pid);

    fs::remove_file(&pid_record).expect("remove cancellation pid record");
    let mut future = launcher(script, Duration::from_secs(2))
        .launch(request(temporary.path()), CancellationToken::new());
    assert!(poll_once(&mut future, &waker).is_pending());
    wait_for_file(&pid_record);
    let pid = read_pid(&pid_record);
    drop(future);
    assert_process_reaped(pid);
}
