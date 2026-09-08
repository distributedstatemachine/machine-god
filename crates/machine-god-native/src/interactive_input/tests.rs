use super::*;
use std::collections::VecDeque;
use std::io::Write;
use std::time::Instant;

fn pipe() -> (File, File) {
    let (read, write) = std::io::pipe().unwrap();
    (
        std::os::fd::OwnedFd::from(read).into(),
        std::os::fd::OwnedFd::from(write).into(),
    )
}
fn flags(file: &File) -> OFlags {
    rustix::fs::fcntl_getfl(file).unwrap()
}
fn input(file: File) -> NativeInteractiveInput {
    NativeInteractiveInput::new(
        NativeInteractiveInputSource::AdoptNonblockingStatus(file),
        CancellationToken::new(),
    )
}
fn poll(
    input: &mut NativeInteractiveInput,
) -> Poll<Result<Option<NativeInteractiveInputChunk>, NativeInteractiveInputError>> {
    input.poll_chunk(&mut Context::from_waker(Waker::noop()))
}
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "bounded input observation expired"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn next(
    input: &mut NativeInteractiveInput,
) -> Result<Option<NativeInteractiveInputChunk>, NativeInteractiveInputError> {
    let mut result = None;
    until(|| {
        if let Poll::Ready(value) = poll(input) {
            result = Some(value);
        }
        result.is_some()
    });
    result.unwrap()
}
fn joined(completion: &NativeOwnedWorkerCompletion) {
    until(|| completion.is_complete());
    completion.wait_on_worker().unwrap();
}
fn slot_full(input: &NativeInteractiveInput) -> bool {
    input.shared.state.lock().unwrap().chunk.is_some()
}

#[test]
fn construction_disabled_drop_and_precancellation_are_inert() {
    let mut disabled = NativeInteractiveInput::default();
    assert!(next(&mut disabled).unwrap().is_none());
    joined(&disabled.completion());
    assert!(next(&mut disabled).unwrap().is_none());
    let (read, _write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    let unpolled = input(read);
    let completion = unpolled.completion();
    assert!(!flags(&alias).contains(OFlags::NONBLOCK));
    drop(unpolled);
    joined(&completion);
    assert_eq!(flags(&alias), original);

    let stop = CancellationToken::new();
    stop.cancel();
    let mut cancelled = NativeInteractiveInput::new(
        NativeInteractiveInputSource::AdoptNonblockingStatus(alias.try_clone().unwrap()),
        stop,
    );
    assert_eq!(
        next(&mut cancelled).unwrap_err(),
        NativeInteractiveInputError::Cancelled
    );
    assert_eq!(flags(&alias), original);
    joined(&cancelled.completion());
}

#[test]
fn preserve_rejects_blocking_without_mutation_and_accepts_nonblocking() {
    let (read, _write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    let mut rejected = NativeInteractiveInput::new(
        NativeInteractiveInputSource::PreserveNonblocking(read),
        CancellationToken::new(),
    );
    assert_eq!(
        next(&mut rejected).unwrap_err(),
        NativeInteractiveInputError::NonblockingRequired
    );
    joined(&rejected.completion());
    assert_eq!(flags(&alias), original);
    rustix::fs::fcntl_setfl(&alias, original | OFlags::NONBLOCK).unwrap();
    let mut accepted = NativeInteractiveInput::new(
        NativeInteractiveInputSource::PreserveNonblocking(alias.try_clone().unwrap()),
        CancellationToken::new(),
    );
    assert!(poll(&mut accepted).is_pending());
    accepted.request_stop();
    joined(&accepted.completion());
    assert_eq!(flags(&alias), original | OFlags::NONBLOCK);
}

#[test]
fn explicit_adoption_changes_aliases_and_never_restores_them() {
    let (read, _write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    let mut reader = input(read);
    assert!(poll(&mut reader).is_pending());
    until(|| flags(&alias).contains(OFlags::NONBLOCK));
    assert_eq!(flags(&alias), original | OFlags::NONBLOCK);
    let completion = reader.completion();
    drop(reader);
    joined(&completion);
    assert_eq!(flags(&alias), original | OFlags::NONBLOCK);
}

#[test]
fn chunks_preserve_partial_bytes_delimiters_and_eof() {
    let (read, mut write) = pipe();
    let mut reader = input(read);
    write.write_all(&[0xf0, 0x9f]).unwrap();
    assert_eq!(next(&mut reader).unwrap().unwrap().as_bytes(), [0xf0, 0x9f]);
    write.write_all(&[0x98, 0x80, b'\n', b'\n', b'x']).unwrap();
    drop(write);
    assert_eq!(
        next(&mut reader).unwrap().unwrap().as_bytes(),
        [0x98, 0x80, b'\n', b'\n', b'x']
    );
    assert!(next(&mut reader).unwrap().is_none());
    assert!(next(&mut reader).unwrap().is_none());
    joined(&reader.completion());
}

#[test]
fn one_exact_chunk_and_demand_gate_prevent_read_ahead() {
    let (read, mut write) = pipe();
    let alias = read.try_clone().unwrap();
    let mut reader = input(read);
    write
        .write_all(&[b'a'; NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES])
        .unwrap();
    let chunk = next(&mut reader).unwrap().unwrap();
    assert_eq!(
        chunk.as_bytes(),
        &[b'a'; NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES]
    );
    assert!(!reader.shared.state.lock().unwrap().demand);
    write.write_all(b"not admitted").unwrap();
    // Test-only inspection of the original pipe verifies that no follow-up
    // read is admitted. Production callers must not compete for input.
    std::thread::sleep(WAIT_INTERVAL * 2);
    let mut bytes = [0; 12];
    assert_eq!(rustix::io::read(&alias, &mut bytes).unwrap(), 12);
    assert_eq!(&bytes, b"not admitted");
    reader.request_stop();
    joined(&reader.completion());
}

#[test]
fn idle_and_full_slot_pipe_stop_join_without_output_consumption() {
    for full in [false, true] {
        let (read, mut write) = pipe();
        let stop = CancellationToken::new();
        let mut reader = NativeInteractiveInput::new(
            NativeInteractiveInputSource::AdoptNonblockingStatus(read),
            stop.clone(),
        );
        assert!(poll(&mut reader).is_pending());
        if full {
            write.write_all(b"unconsumed").unwrap();
            until(|| slot_full(&reader));
        }
        stop.cancel();
        joined(&reader.completion());
        assert_eq!(
            next(&mut reader).unwrap_err(),
            NativeInteractiveInputError::Cancelled
        );
        assert!(next(&mut reader).unwrap().is_none());
    }
}

fn pty() -> (File, File) {
    use rustix::fs::Mode;
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
fn real_pty_idle_full_slot_and_drop_join_without_output_consumer() {
    for full in [false, true] {
        let (mut master, slave) = pty();
        let mut reader = input(slave);
        assert!(poll(&mut reader).is_pending());
        if full {
            master.write_all(b"line\n").unwrap();
            until(|| slot_full(&reader));
        }
        let completion = reader.completion();
        drop(reader);
        joined(&completion);
    }
}

#[test]
fn real_pty_canonical_partial_line_is_delivered_without_line_interpretation() {
    let (mut master, slave) = pty();
    let mut reader = input(slave);
    assert!(poll(&mut reader).is_pending());
    master.write_all(b"abc").unwrap();
    std::thread::sleep(WAIT_INTERVAL * 2);
    assert!(!slot_full(&reader));
    master.write_all(b"\n").unwrap();
    assert_eq!(next(&mut reader).unwrap().unwrap().as_bytes(), b"abc\n");
    reader.request_stop();
    joined(&reader.completion());
}

fn zero_minimum_pty() -> (File, File) {
    let (master, slave) = pty();
    let mut settings = rustix::termios::tcgetattr(&slave).unwrap();
    settings.make_raw();
    settings.special_codes[rustix::termios::SpecialCodeIndex::VMIN] = 0;
    settings.special_codes[rustix::termios::SpecialCodeIndex::VTIME] = 0;
    rustix::termios::tcsetattr(&slave, rustix::termios::OptionalActions::Now, &settings).unwrap();
    // The real terminal, not a fake decoder, demonstrates zero as no-data.
    assert_eq!(rustix::io::read(&slave, &mut [0; 4]).unwrap(), 0);
    (master, slave)
}

#[test]
fn zero_minimum_pty_waits_for_later_input_and_stops_idle_or_with_full_slot() {
    for full in [false, true] {
        let (mut master, slave) = zero_minimum_pty();
        let alias = slave.try_clone().unwrap();
        let mut reader = input(slave);
        assert!(poll(&mut reader).is_pending());
        until(|| flags(&alias).contains(OFlags::NONBLOCK));
        std::thread::sleep(WAIT_INTERVAL * 2);
        assert!(!slot_full(&reader));
        assert!(reader.shared.state.lock().unwrap().terminal.is_none());
        master.write_all(b"later").unwrap();
        assert_eq!(next(&mut reader).unwrap().unwrap().as_bytes(), b"later");
        assert!(poll(&mut reader).is_pending());
        if full {
            master.write_all(b"pending").unwrap();
            until(|| slot_full(&reader));
        }
        reader.request_stop();
        joined(&reader.completion());
        assert_eq!(
            next(&mut reader).unwrap_err(),
            NativeInteractiveInputError::Cancelled
        );
    }
}

#[test]
fn zero_minimum_pty_peer_close_never_manufactures_eof() {
    let (master, slave) = zero_minimum_pty();
    let alias = slave.try_clone().unwrap();
    let mut reader = input(slave);
    assert!(poll(&mut reader).is_pending());
    until(|| flags(&alias).contains(OFlags::NONBLOCK));
    drop(master);
    let deadline = Instant::now() + WAIT_INTERVAL * 4;
    let observed = loop {
        match poll(&mut reader) {
            Poll::Ready(value) => break Some(value),
            Poll::Pending if Instant::now() >= deadline => break None,
            Poll::Pending => std::thread::sleep(Duration::from_millis(1)),
        }
    };
    match observed {
        Some(Err(NativeInteractiveInputError::Read)) => {
            eprintln!("PTY peer closure supplies a read/poll failure, not clean EOF");
        }
        None => eprintln!(
            "PTY peer closure supplies no positive end evidence; input remains cancellable"
        ),
        other => panic!("unexpected peer-close outcome: {other:?}"),
    }
    reader.request_stop();
    joined(&reader.completion());
}

#[test]
fn unsupported_descriptors_are_rejected_before_status_mutation() {
    for file in [
        File::open(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap(),
        File::open(".").unwrap(),
        File::open("/dev/null").unwrap(),
    ] {
        let alias = file.try_clone().unwrap();
        let original = flags(&alias);
        let mut reader = input(file);
        assert_eq!(
            next(&mut reader).unwrap_err(),
            NativeInteractiveInputError::InvalidDescriptor
        );
        joined(&reader.completion());
        assert_eq!(flags(&alias), original);
    }
    let (_, write) = pipe();
    let mut reader = input(write);
    assert_eq!(
        next(&mut reader).unwrap_err(),
        NativeInteractiveInputError::InvalidDescriptor
    );
    joined(&reader.completion());

    let (socket, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
    peer.write_all(b"unread").unwrap();
    let file = File::from(std::os::fd::OwnedFd::from(socket));
    let alias = file.try_clone().unwrap();
    let original = flags(&alias);
    let mut reader = input(file);
    assert_eq!(
        next(&mut reader).unwrap_err(),
        NativeInteractiveInputError::InvalidDescriptor
    );
    joined(&reader.completion());
    assert_eq!(flags(&alias), original);
    let mut bytes = [0; 6];
    assert_eq!(rustix::io::read(&alias, &mut bytes).unwrap(), 6);
    assert_eq!(&bytes, b"unread");
}

#[test]
fn rejected_worker_admission_does_not_touch_descriptor() {
    let (read, _write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    let mut reader = input(read);
    reader.scope.close();
    assert_eq!(
        next(&mut reader).unwrap_err(),
        NativeInteractiveInputError::Unavailable
    );
    joined(&reader.completion());
    assert_eq!(flags(&alias), original);
}

struct ScriptIo {
    polls: Mutex<VecDeque<rustix::io::Result<bool>>>,
    reads: Mutex<VecDeque<rustix::io::Result<usize>>>,
}
impl InputIo for ScriptIo {
    fn readable(&self, _: &File) -> rustix::io::Result<bool> {
        self.polls.lock().unwrap().pop_front().unwrap_or(Ok(true))
    }
    fn read(&self, _: &File, _: &mut [u8]) -> rustix::io::Result<usize> {
        self.reads
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected read")
    }
}

#[test]
fn native_poll_read_retries_and_errors_use_fixed_outcomes() {
    use rustix::io::Errno;
    for (polls, reads, expected) in [
        (
            vec![Err(Errno::BADF)],
            vec![],
            Err(NativeInteractiveInputError::Read),
        ),
        (
            vec![Ok(true)],
            vec![Err(Errno::IO)],
            Err(NativeInteractiveInputError::Read),
        ),
        (
            vec![Err(Errno::INTR), Ok(false), Ok(true)],
            vec![Err(Errno::INTR), Err(Errno::AGAIN), Ok(0)],
            Ok(()),
        ),
    ] {
        let (read, _write) = pipe();
        let mut reader = input(read);
        let source = reader.source.take().unwrap();
        reader.shared.state.lock().unwrap().demand = true;
        let io = ScriptIo {
            polls: Mutex::new(polls.into()),
            reads: Mutex::new(reads.into()),
        };
        assert_eq!(run_worker(source, &reader.shared, &io), expected);
    }
}

#[test]
fn terminal_error_is_independent_of_occupied_presentation_slot_and_debug_is_redacted() {
    let mut reader = NativeInteractiveInput::default();
    reader.shared.state.lock().unwrap().chunk = Some(NativeInteractiveInputChunk {
        bytes: [b'x'; NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES],
        len: 23,
    });
    reader.shared.finish(Err(NativeInteractiveInputError::Read));
    reader.scope.close();
    assert_eq!(
        next(&mut reader).unwrap_err(),
        NativeInteractiveInputError::Read
    );
    assert!(next(&mut reader).unwrap().is_none());
    assert!(!format!("{reader:?}").contains("xxx"));
}

struct ExitGate {
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}
impl Drop for ExitGate {
    fn drop(&mut self) {
        self.entered.store(true, Ordering::Release);
        until(|| self.release.load(Ordering::Acquire));
    }
}
thread_local! {
    static EXIT_GATE: std::cell::RefCell<Option<ExitGate>> = const { std::cell::RefCell::new(None) };
}
struct InstallExitGate {
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}
impl std::task::Wake for InstallExitGate {
    fn wake(self: Arc<Self>) {
        EXIT_GATE.with(|slot| {
            *slot.borrow_mut() = Some(ExitGate {
                entered: Arc::clone(&self.entered),
                release: Arc::clone(&self.release),
            });
        });
    }
}
struct ReleaseOnDrop(Arc<AtomicBool>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[test]
fn completion_waits_for_actual_thread_local_destruction_not_only_eof() {
    let (read, write) = pipe();
    drop(write);
    let mut reader = input(read);
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let release_on_drop = ReleaseOnDrop(Arc::clone(&release));
    let waker = Waker::from(Arc::new(InstallExitGate {
        entered: Arc::clone(&entered),
        release,
    }));
    assert!(
        reader
            .poll_chunk(&mut Context::from_waker(&waker))
            .is_pending()
    );
    until(|| entered.load(Ordering::Acquire));
    assert!(next(&mut reader).unwrap().is_none());
    assert!(!reader.completion().is_complete());
    drop(release_on_drop);
    joined(&reader.completion());
}
