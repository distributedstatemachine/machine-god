#![cfg(any(target_os = "linux", target_os = "macos"))]

use machine_god_core::CancellationToken;
use machine_god_native::{
    NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES, NativeInteractiveInput, NativeInteractiveInputChunk,
    NativeInteractiveInputError, NativeInteractiveInputHelper, NativeInteractiveInputSource,
    NativeOwnedWorkerCompletion,
};
use rustix::fs::{Mode, OFlags};
use std::fs::File;
use std::io::Write as _;
use std::io::{Seek, SeekFrom};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

fn helper() -> NativeInteractiveInputHelper {
    let selected = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY");
    #[cfg(target_os = "macos")]
    assert!(
        selected.is_some(),
        "set MACHINE_GOD_TERMINAL_RELEASE_BINARY to the freshly built production CLI"
    );
    let path = selected.map_or_else(
        || {
            // Cargo builds the real CLI executable for workspace/CLI
            // integration tests before running this native test target.
            std::env::current_exe()
                .expect("locate the native integration test executable")
                .parent()
                .and_then(Path::parent)
                .expect("native integration test executable has a profile directory")
                .join("machine-god")
        },
        PathBuf::from,
    );
    let path = path
        .canonicalize()
        .expect("build/provide the production CLI with interactive-input helper dispatch");
    NativeInteractiveInputHelper::new(&path, File::open(&path).unwrap()).unwrap()
}

fn input(file: File, stop: CancellationToken) -> NativeInteractiveInput {
    NativeInteractiveInput::new(
        NativeInteractiveInputSource::PreserveShared {
            input: file,
            helper: helper(),
        },
        stop,
    )
}

fn stream_input(file: File, stop: CancellationToken) -> NativeInteractiveInput {
    NativeInteractiveInput::new(
        NativeInteractiveInputSource::PreserveSharedStream {
            input: file,
            helper: helper(),
            null_device: Some(File::open("/dev/null").unwrap()),
        },
        stop,
    )
}

#[test]
fn production_stream_helper_reads_retained_regular_file_from_offset_then_eof() {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let (path, mut file) = loop {
        let path = std::env::temp_dir().join(format!(
            "machine-god-cli-stream-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        match std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => break (path, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("stream fixture unavailable: {error}"),
        }
    };
    std::fs::remove_file(path).unwrap();
    file.write_all(&[b'x'; 8200]).unwrap();
    file.seek(SeekFrom::Start(3)).unwrap();
    let mut alias = file.try_clone().unwrap();
    let original = flags(&alias);
    let mut input = stream_input(file, CancellationToken::new());
    assert_eq!(alias.stream_position().unwrap(), 3);
    for len in [4096, 4096, 5] {
        assert_eq!(next(&mut input).unwrap().as_bytes(), vec![b'x'; len]);
    }
    assert!(next(&mut input).is_none());
    joined(&input.completion());
    assert_eq!(alias.stream_position().unwrap(), 8200);
    assert_eq!(flags(&alias), original);
}

#[test]
fn production_stream_pipe_cancels_owned_idle_helper_and_preserves_flags() {
    let (file, mut writer) = pipe();
    let alias = file.try_clone().unwrap();
    let original = flags(&alias);
    let mut input = stream_input(file, CancellationToken::new());
    writer.write_all(b"stream").unwrap();
    assert_eq!(next(&mut input).unwrap().as_bytes(), b"stream");
    assert!(poll(&mut input).is_pending());
    let completion = input.completion();
    drop(input);
    joined(&completion);
    assert_eq!(flags(&alias), original);
    drop(alias);
    assert_eq!(
        writer.write(b"x").unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
}

#[test]
fn production_stream_explicit_null_authority_is_empty() {
    let mut input = stream_input(File::open("/dev/null").unwrap(), CancellationToken::new());
    assert!(next(&mut input).is_none());
    joined(&input.completion());
}

fn pipe() -> (File, File) {
    let (read, write) = std::io::pipe().unwrap();
    (OwnedFd::from(read).into(), OwnedFd::from(write).into())
}

fn flags(file: &File) -> OFlags {
    rustix::fs::fcntl_getfl(file).unwrap()
}

fn poll(
    input: &mut NativeInteractiveInput,
) -> Poll<Result<Option<NativeInteractiveInputChunk>, NativeInteractiveInputError>> {
    input.poll_chunk(&mut Context::from_waker(Waker::noop()))
}

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "production input observation expired"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn next(input: &mut NativeInteractiveInput) -> Option<NativeInteractiveInputChunk> {
    let mut result = None;
    until(|| {
        if let Poll::Ready(value) = poll(input) {
            result = Some(value.unwrap());
        }
        result.is_some()
    });
    result.unwrap()
}

fn joined(completion: &NativeOwnedWorkerCompletion) {
    until(|| completion.is_complete());
    completion.wait_on_worker().unwrap();
}

#[test]
fn production_helper_preserves_raw_chunks_eof_and_shared_pipe_flags() {
    let (read, mut write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    assert!(!original.contains(OFlags::NONBLOCK));
    let mut input = input(read, CancellationToken::new());
    assert_eq!(flags(&alias), original);
    write.write_all(&[0xf0, 0x9f]).unwrap();
    assert_eq!(next(&mut input).unwrap().as_bytes(), [0xf0, 0x9f]);
    assert_eq!(flags(&alias), original);
    write
        .write_all(&[0x98, 0x80, 0, b'\r', b'\n', b'x'])
        .unwrap();
    drop(write);
    assert_eq!(
        next(&mut input).unwrap().as_bytes(),
        [0x98, 0x80, 0, b'\r', b'\n', b'x']
    );
    assert!(next(&mut input).is_none());
    joined(&input.completion());
    assert!(next(&mut input).is_none());
    assert_eq!(flags(&alias), original);
}

#[test]
fn production_helper_reads_only_one_credited_chunk_and_stops_without_slot_consumption() {
    let (read, mut write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    let mut input = input(read, CancellationToken::new());
    write
        .write_all(&[b'a'; NATIVE_INTERACTIVE_INPUT_CHUNK_BYTES])
        .unwrap();
    assert!(poll(&mut input).is_pending());
    // Readiness alone is insufficient: wait until the actual helper consumes
    // the single credited read, without polling its queued result again.
    until(|| rustix::io::ioctl_fionread(&alias).unwrap() == 0);
    write.write_all(b"reserved").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(rustix::io::ioctl_fionread(&alias).unwrap(), 8);
    input.request_stop();
    joined(&input.completion());
    assert!(matches!(
        poll(&mut input),
        Poll::Ready(Err(NativeInteractiveInputError::Cancelled))
    ));
    assert_eq!(rustix::io::ioctl_fionread(&alias).unwrap(), 8);
    assert_eq!(flags(&alias), original);
    drop(input);
    drop(alias);
    assert_eq!(
        write.write(b"x").unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
}

#[test]
fn production_helper_idle_drop_and_host_cancellation_join_without_further_polls() {
    for cancel_host in [false, true] {
        let (read, mut write) = pipe();
        let alias = read.try_clone().unwrap();
        let original = flags(&alias);
        let host_stop = CancellationToken::new();
        let mut input = input(read, host_stop.clone());
        // A successful real round trip proves private dispatch and child
        // admission before asking the helper to wait on the still-open pipe.
        write.write_all(b"started").unwrap();
        assert_eq!(next(&mut input).unwrap().as_bytes(), b"started");
        assert!(poll(&mut input).is_pending());
        let completion = input.completion();
        if cancel_host {
            host_stop.cancel();
            joined(&completion);
            drop(input);
        } else {
            drop(input);
            joined(&completion);
            assert!(!host_stop.is_cancelled());
        }
        assert_eq!(flags(&alias), original);
        drop(alias);
        // Completion must not leave a helper retaining the input read end.
        assert_eq!(
            write.write(b"x").unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
    }
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
fn production_input_selection_preserves_shared_tty_flags_and_settings() {
    let (mut master, slave) = pty();
    let alias = slave.try_clone().unwrap();
    let original = flags(&alias);
    let settings = format!("{:?}", rustix::termios::tcgetattr(&alias).unwrap());
    let mut input = input(slave, CancellationToken::new());
    master.write_all(b"terminal input\n").unwrap();
    assert_eq!(next(&mut input).unwrap().as_bytes(), b"terminal input\n");
    assert_eq!(flags(&alias), original);
    assert!(poll(&mut input).is_pending());
    let completion = input.completion();
    drop(input);
    joined(&completion);
    assert_eq!(flags(&alias), original);
    assert_eq!(
        format!("{:?}", rustix::termios::tcgetattr(&alias).unwrap()),
        settings
    );
}
