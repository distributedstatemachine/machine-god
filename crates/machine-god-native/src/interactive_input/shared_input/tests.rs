use super::*;
use crate::{NativeInteractiveInput, NativeInteractiveInputSource};
use machine_god_core::CancellationToken;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

mod stream;

pub(super) struct ChildObservations {
    pub(super) pid: AtomicU32,
    pub(super) deferred: Option<Arc<AtomicBool>>,
}

fn pipe() -> (File, File) {
    let (read, write) = std::io::pipe().unwrap();
    (OwnedFd::from(read).into(), OwnedFd::from(write).into())
}
fn helper(
    child: &str,
    deferred: Option<Arc<AtomicBool>>,
) -> (NativeInteractiveInputHelper, Arc<ChildObservations>) {
    let path = std::env::current_exe().unwrap();
    let mut helper = NativeInteractiveInputHelper::new(&path, File::open(&path).unwrap()).unwrap();
    helper.arguments = Some(Arc::new(vec![
        "--exact".into(),
        format!("interactive_input::shared_input::tests::{child}").into(),
        "--ignored".into(),
        "--nocapture".into(),
    ]));
    let observations = Arc::new(ChildObservations {
        pid: AtomicU32::new(0),
        deferred,
    });
    helper.observations = Some(Arc::clone(&observations));
    (helper, observations)
}
fn input(file: File, helper: NativeInteractiveInputHelper) -> NativeInteractiveInput {
    NativeInteractiveInput::new(
        NativeInteractiveInputSource::PreserveShared {
            input: file,
            helper,
        },
        CancellationToken::new(),
    )
}
fn poll(
    input: &mut NativeInteractiveInput,
) -> Poll<Result<Option<NativeInteractiveInputChunk>, Error>> {
    input.poll_chunk(&mut Context::from_waker(Waker::noop()))
}
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "input helper observation expired"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn next(input: &mut NativeInteractiveInput) -> Result<Option<NativeInteractiveInputChunk>, Error> {
    let mut result = None;
    until(|| {
        if let Poll::Ready(value) = poll(input) {
            result = Some(value);
        }
        result.is_some()
    });
    result.unwrap()
}
fn joined(input: &NativeInteractiveInput) {
    until(|| input.completion().is_complete());
    input.completion().wait_on_worker().unwrap();
}
fn assert_reaped(observations: &ChildObservations) {
    let raw = i32::try_from(observations.pid.load(Ordering::Acquire)).unwrap();
    assert!(raw > 0);
    let pid = rustix::process::Pid::from_raw(raw).unwrap();
    assert_eq!(
        rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG).unwrap_err(),
        rustix::io::Errno::CHILD
    );
}
fn flags(file: &File) -> OFlags {
    rustix::fs::fcntl_getfl(file).unwrap()
}

#[test]
#[ignore = "private subprocess helper entrypoint"]
fn pipe_helper_child() {
    std::process::exit(if run_interactive_input_helper().is_ok() {
        0
    } else {
        125
    });
}

#[test]
#[ignore = "private malformed helper entrypoint"]
fn malformed_helper_child() {
    let channel = std::io::stderr();
    let mut hello = [0; 8];
    let mut read = 0;
    while read < hello.len() {
        read += rustix::io::read(&channel, &mut hello[read..]).unwrap();
    }
    rustix::io::write(&channel, b"BADREADY").unwrap();
    std::process::exit(0);
}

#[test]
#[ignore = "private stalled helper entrypoint"]
fn stalled_helper_child() {
    // Intentionally does not acknowledge the private handshake. Parent
    // cancellation must kill/reap it without changing inherited stdin flags.
    let mut byte = [0];
    let _ = rustix::io::read(std::io::stdin(), &mut byte[..]);
    std::process::exit(0);
}

#[test]
fn shared_construction_and_precancellation_do_not_spawn_or_mutate_flags() {
    let (read, _write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    let (authority, observations) = helper("pipe_helper_child", None);
    let input = input(read, authority);
    let completion = input.completion();
    drop(input);
    until(|| completion.is_complete());
    assert_eq!(observations.pid.load(Ordering::Acquire), 0);
    assert_eq!(flags(&alias), original);
    let stop = CancellationToken::new();
    stop.cancel();
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = NativeInteractiveInput::new(
        NativeInteractiveInputSource::PreserveShared {
            input: alias.try_clone().unwrap(),
            helper,
        },
        stop,
    );
    assert_eq!(next(&mut input).unwrap_err(), Error::Cancelled);
    joined(&input);
    assert_eq!(observations.pid.load(Ordering::Acquire), 0);
    assert_eq!(flags(&alias), original);
}

#[test]
fn helper_preserves_shared_pipe_flags_raw_bytes_and_eof_then_reaps() {
    let (read, mut write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = input(read, helper);
    write.write_all(&[0xf0, 0x9f]).unwrap();
    assert_eq!(next(&mut input).unwrap().unwrap().as_bytes(), [0xf0, 0x9f]);
    write.write_all(&[0x98, 0x80, b'\n', b'\r', b'x']).unwrap();
    drop(write);
    assert_eq!(
        next(&mut input).unwrap().unwrap().as_bytes(),
        [0x98, 0x80, b'\n', b'\r', b'x']
    );
    assert!(next(&mut input).unwrap().is_none());
    joined(&input);
    assert_eq!(flags(&alias), original);
    assert_reaped(&observations);
}

#[test]
fn helper_does_not_prefetch_beyond_one_credit_or_wait_for_full_slot_consumption() {
    let (read, mut write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = input(read, helper);
    write
        .write_all(&[b'a'; super::super::NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES])
        .unwrap();
    assert!(poll(&mut input).is_pending());
    until(|| input.shared.state.lock().unwrap().chunk.is_some());
    write.write_all(b"unread").unwrap();
    std::thread::sleep(super::super::WAIT_INTERVAL * 3);
    assert_eq!(rustix::io::ioctl_fionread(&alias).unwrap(), 6);
    input.request_stop();
    joined(&input);
    assert_eq!(next(&mut input).unwrap_err(), Error::Cancelled);
    assert_eq!(flags(&alias), original);
    assert_eq!(rustix::io::ioctl_fionread(&alias).unwrap(), 6);
    assert_reaped(&observations);
}

#[test]
fn cancellation_of_idle_and_unacknowledged_helpers_reaps_without_a_consumer() {
    for child in ["pipe_helper_child", "stalled_helper_child"] {
        let (read, _write) = pipe();
        let alias = read.try_clone().unwrap();
        let original = flags(&alias);
        let (helper, observations) = helper(child, None);
        let mut input = input(read, helper);
        assert!(poll(&mut input).is_pending());
        until(|| observations.pid.load(Ordering::Acquire) > 0);
        let completion = input.completion();
        drop(input);
        until(|| completion.is_complete());
        completion.wait_on_worker().unwrap();
        assert_eq!(flags(&alias), original);
        assert_reaped(&observations);
    }
}

#[test]
fn malformed_helper_is_reaped_without_consuming_source_input() {
    let (read, mut write) = pipe();
    let alias = read.try_clone().unwrap();
    write.write_all(b"reserved").unwrap();
    let (helper, observations) = helper("malformed_helper_child", None);
    let mut input = input(read, helper);
    assert_eq!(next(&mut input).unwrap_err(), Error::Read);
    joined(&input);
    assert_eq!(rustix::io::ioctl_fionread(&alias).unwrap(), 8);
    assert_reaped(&observations);
}

struct ReleaseDeferred(Arc<AtomicBool>);
impl Drop for ReleaseDeferred {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[test]
fn quarantined_helper_keeps_input_completion_pending_until_actual_reap() {
    let (read, _write) = pipe();
    let deferred = Arc::new(AtomicBool::new(true));
    let release = ReleaseDeferred(Arc::clone(&deferred));
    let (helper, observations) = helper("pipe_helper_child", Some(deferred));
    let mut input = input(read, helper);
    assert!(poll(&mut input).is_pending());
    until(|| observations.pid.load(Ordering::Acquire) > 0);
    input.request_stop();
    std::thread::sleep(Duration::from_millis(700));
    assert!(!input.completion().is_complete());
    drop(release);
    joined(&input);
    assert_reaped(&observations);
}

#[test]
fn invalid_program_authority_fails_before_spawn_without_shared_mutation() {
    let path = std::env::current_exe().unwrap();
    let unrelated =
        File::open(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let mut helper = NativeInteractiveInputHelper::new(&path, unrelated).unwrap();
    let observations = Arc::new(ChildObservations {
        pid: AtomicU32::new(0),
        deferred: None,
    });
    helper.observations = Some(Arc::clone(&observations));
    let (read, _write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    let mut input = input(read, helper);
    assert_eq!(next(&mut input).unwrap_err(), Error::Unavailable);
    joined(&input);
    assert_eq!(flags(&alias), original);
    assert_eq!(observations.pid.load(Ordering::Acquire), 0);
}

#[test]
fn shared_regular_input_is_rejected_without_helper_admission() {
    let file = File::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = input(file, helper);
    assert_eq!(next(&mut input).unwrap_err(), Error::InvalidDescriptor);
    joined(&input);
    assert_eq!(observations.pid.load(Ordering::Acquire), 0);
}

#[test]
fn failed_exec_preserves_pipe_and_settles_child_admission() {
    use std::os::unix::fs::PermissionsExt as _;
    static NEXT: AtomicU32 = AtomicU32::new(0);
    struct RemoveFile(PathBuf);
    impl Drop for RemoveFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let (path, mut file) = loop {
        let path = std::env::temp_dir().join(format!(
            "machine-god-invalid-input-helper-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => break (path, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("test helper setup failed: {error}"),
        }
    };
    let _cleanup = RemoveFile(path.clone());
    file.write_all(b"#!/machine-god-missing-input-interpreter\n")
        .unwrap();
    file.set_permissions(std::fs::Permissions::from_mode(0o700))
        .unwrap();
    drop(file);
    let helper = NativeInteractiveInputHelper::new(&path, File::open(&path).unwrap()).unwrap();
    let (read, mut write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    write.write_all(b"reserved").unwrap();
    let mut input = input(read, helper);
    assert_eq!(next(&mut input).unwrap_err(), Error::Unavailable);
    joined(&input);
    assert_eq!(flags(&alias), original);
    assert_eq!(rustix::io::ioctl_fionread(&alias).unwrap(), 8);
}

#[test]
fn competing_pipe_reader_cannot_strand_adapter_shutdown() {
    let (read, mut write) = pipe();
    let mut competing = read.try_clone().unwrap();
    let original = flags(&competing);
    write.write_all(b"taken").unwrap();
    let mut taken = [0; 5];
    std::io::Read::read_exact(&mut competing, &mut taken).unwrap();
    assert_eq!(&taken, b"taken");
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = input(read, helper);
    assert!(poll(&mut input).is_pending());
    until(|| observations.pid.load(Ordering::Acquire) > 0);
    input.request_stop();
    joined(&input);
    assert_eq!(flags(&competing), original);
    assert_reaped(&observations);
}

#[test]
fn helper_binding_checks_spelling_inertly_and_redacts_debug() {
    let (read, _write) = pipe();
    for path in [
        PathBuf::from("relative"),
        PathBuf::from("/a/../b"),
        PathBuf::from(format!("/{}", "a".repeat(MAX_PROGRAM_BYTES))),
    ] {
        assert_eq!(
            NativeInteractiveInputHelper::new(&path, read.try_clone().unwrap()).unwrap_err(),
            Error::InvalidDescriptor
        );
    }
    let bound = NativeInteractiveInputHelper::new(Path::new("/PRIVATE_HELPER"), read).unwrap();
    assert!(!format!("{bound:?}").contains("PRIVATE"));
}

fn pty() -> (File, File) {
    let master = rustix::fs::open(
        "/dev/ptmx",
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .unwrap();
    rustix::pty::grantpt(&master).unwrap();
    rustix::pty::unlockpt(&master).unwrap();
    #[cfg(target_os = "linux")]
    let slave = rustix::pty::ioctl_tiocgptpeer(
        &master,
        rustix::pty::OpenptFlags::RDWR
            | rustix::pty::OpenptFlags::NOCTTY
            | rustix::pty::OpenptFlags::CLOEXEC,
    )
    .unwrap();
    #[cfg(target_os = "macos")]
    let slave = {
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap()
    };
    (master.into(), slave.into())
}

#[test]
fn shared_tty_reopens_independently_and_preserves_original_flags_and_termios() {
    let (mut master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let original = flags(&alias);
    let settings = rustix::termios::tcgetattr(&alias).unwrap();
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = input(slave, helper);
    master.write_all(b"line\n").unwrap();
    assert_eq!(next(&mut input).unwrap().unwrap().as_bytes(), b"line\n");
    input.request_stop();
    joined(&input);
    assert_eq!(flags(&alias), original);
    assert!(same_settings(
        &settings,
        &rustix::termios::tcgetattr(&alias).unwrap()
    ));
    assert_eq!(observations.pid.load(Ordering::Acquire), 0);
}

#[test]
fn shared_tty_master_cannot_reopen_as_an_unrelated_clone() {
    let (master, _slave) = pty();
    let alias = master.try_clone().unwrap();
    let original = flags(&alias);
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = input(master, helper);
    assert_eq!(next(&mut input).unwrap_err(), Error::InvalidDescriptor);
    joined(&input);
    assert_eq!(flags(&alias), original);
    assert_eq!(observations.pid.load(Ordering::Acquire), 0);
}

#[test]
fn shared_zero_minimum_tty_waits_for_bytes_and_stops_with_full_slot() {
    let (mut master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let mut settings = rustix::termios::tcgetattr(&slave).unwrap();
    settings.make_raw();
    settings.special_codes[rustix::termios::SpecialCodeIndex::VMIN] = 0;
    settings.special_codes[rustix::termios::SpecialCodeIndex::VTIME] = 0;
    rustix::termios::tcsetattr(&slave, rustix::termios::OptionalActions::Now, &settings).unwrap();
    let original = flags(&alias);
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = input(slave, helper);
    assert!(poll(&mut input).is_pending());
    std::thread::sleep(super::super::WAIT_INTERVAL * 3);
    assert!(poll(&mut input).is_pending());
    master.write_all(b"after-zero").unwrap();
    assert_eq!(next(&mut input).unwrap().unwrap().as_bytes(), b"after-zero");
    assert!(poll(&mut input).is_pending());
    master.write_all(b"full-slot").unwrap();
    until(|| input.shared.state.lock().unwrap().chunk.is_some());
    input.request_stop();
    joined(&input);
    assert_eq!(flags(&alias), original);
    assert!(same_settings(
        &settings,
        &rustix::termios::tcgetattr(&alias).unwrap()
    ));
    assert_eq!(observations.pid.load(Ordering::Acquire), 0);
}

#[test]
fn tty_reopen_rejects_wrong_identity_and_descriptor_magic_paths() {
    let (_master, source) = pty();
    let (_other_master, other) = pty();
    let original = rustix::fs::fstat(&source).unwrap();
    let settings = rustix::termios::tcgetattr(&source).unwrap();
    let original_flags = flags(&source);
    let other_name = rustix::termios::ttyname(&other, Vec::new()).unwrap();
    let wrong = Path::new(std::ffi::OsStr::from_bytes(other_name.to_bytes()));
    for path in [wrong, Path::new("/dev/fd/0"), Path::new("/dev/stdin")] {
        assert_eq!(
            reopen_terminal(&source, path, &original, &settings, original_flags).unwrap_err(),
            Error::Unavailable
        );
    }
    assert_eq!(flags(&source), original_flags);
    assert!(same_settings(
        &settings,
        &rustix::termios::tcgetattr(&source).unwrap()
    ));
}
