use super::*;
use std::io::{Seek, SeekFrom};

fn regular(bytes: &[u8]) -> File {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let (path, mut file) = loop {
        let path = std::env::temp_dir().join(format!(
            "machine-god-stream-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
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
    // The retained file works even after unlink: no source-path reopen exists.
    std::fs::remove_file(path).unwrap();
    file.write_all(bytes).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file
}

fn stream(
    file: File,
    authority: NativeInteractiveInputHelper,
    null_device: Option<File>,
) -> NativeInteractiveInput {
    NativeInteractiveInput::new(
        NativeInteractiveInputSource::PreserveSharedStream {
            input: file,
            helper: authority,
            null_device,
        },
        CancellationToken::new(),
    )
}

#[test]
fn regular_stream_is_inert_then_uses_helper_from_retained_offset_with_bounded_credits() {
    for nonblocking in [false, true] {
        let mut file = regular(&vec![b'x'; 8200]);
        file.seek(SeekFrom::Start(3)).unwrap();
        if nonblocking {
            rustix::fs::fcntl_setfl(&file, flags(&file) | OFlags::NONBLOCK).unwrap();
        }
        let mut alias = file.try_clone().unwrap();
        let original = flags(&alias);
        let (helper, observations) = helper("pipe_helper_child", None);
        let mut input = stream(file, helper, None);
        assert_eq!(observations.pid.load(Ordering::Acquire), 0);
        assert_eq!(alias.stream_position().unwrap(), 3);
        assert!(poll(&mut input).is_pending());
        until(|| input.shared.state.lock().unwrap().chunk.is_some());
        assert_eq!(alias.stream_position().unwrap(), 4099);
        std::thread::sleep(super::super::super::WAIT_INTERVAL * 3);
        assert_eq!(alias.stream_position().unwrap(), 4099);
        assert_eq!(next(&mut input).unwrap().unwrap().as_bytes(), &[b'x'; 4096]);
        assert_eq!(next(&mut input).unwrap().unwrap().as_bytes(), &[b'x'; 4096]);
        assert_eq!(next(&mut input).unwrap().unwrap().as_bytes(), &[b'x'; 5]);
        assert!(next(&mut input).unwrap().is_none());
        joined(&input);
        assert_eq!(flags(&alias), original);
        assert_eq!(alias.stream_position().unwrap(), 8200);
        assert_reaped(&observations);
    }
}

#[test]
fn unpolled_and_precancelled_regular_stream_never_consumes_or_spawns() {
    for cancelled in [false, true] {
        let file = regular(b"reserved");
        let mut alias = file.try_clone().unwrap();
        let (helper, observations) = helper("pipe_helper_child", None);
        let stop = CancellationToken::new();
        if cancelled {
            stop.cancel();
        }
        let mut input = NativeInteractiveInput::new(
            NativeInteractiveInputSource::PreserveSharedStream {
                input: file,
                helper,
                null_device: None,
            },
            stop,
        );
        let completion = input.completion();
        if cancelled {
            assert_eq!(next(&mut input).unwrap_err(), Error::Cancelled);
        }
        drop(input);
        until(|| completion.is_complete());
        completion.wait_on_worker().unwrap();
        assert_eq!(alias.stream_position().unwrap(), 0);
        assert_eq!(observations.pid.load(Ordering::Acquire), 0);
    }
}

#[test]
fn full_regular_slot_drop_retains_actual_deferred_reap_without_consuming_more() {
    let file = regular(&[b'x'; 8192]);
    let mut alias = file.try_clone().unwrap();
    let deferred = Arc::new(AtomicBool::new(true));
    let release = ReleaseDeferred(deferred.clone());
    let (helper, observations) = helper("pipe_helper_child", Some(deferred));
    let mut input = stream(file, helper, None);
    assert!(poll(&mut input).is_pending());
    until(|| input.shared.state.lock().unwrap().chunk.is_some());
    let completion = input.completion();
    drop(input);
    std::thread::sleep(Duration::from_millis(700));
    assert!(!completion.is_complete());
    assert_eq!(alias.stream_position().unwrap(), 4096);
    drop(release);
    until(|| completion.is_complete());
    completion.wait_on_worker().unwrap();
    assert_reaped(&observations);
}

#[test]
fn stream_pipe_keeps_idle_read_cancellable_and_flags_unchanged() {
    let (read, mut write) = pipe();
    let alias = read.try_clone().unwrap();
    let original = flags(&alias);
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = stream(read, helper, None);
    write.write_all(b"ready").unwrap();
    assert_eq!(next(&mut input).unwrap().unwrap().as_bytes(), b"ready");
    assert!(poll(&mut input).is_pending());
    input.request_stop();
    joined(&input);
    assert_eq!(next(&mut input).unwrap_err(), Error::Cancelled);
    assert_eq!(flags(&alias), original);
    assert_reaped(&observations);
}

#[test]
fn explicit_matching_null_proof_is_empty_without_read_or_helper() {
    let file = File::open("/dev/null").unwrap();
    let alias = file.try_clone().unwrap();
    let original = flags(&alias);
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = stream(file, helper, Some(File::open("/dev/null").unwrap()));
    assert!(next(&mut input).unwrap().is_none());
    joined(&input);
    assert_eq!(flags(&alias), original);
    assert_eq!(observations.pid.load(Ordering::Acquire), 0);
}

#[test]
fn null_requires_supplied_matching_character_device_proof() {
    let proofs = [
        None,
        Some(regular(b"")),
        Some(File::open("/dev/zero").unwrap()),
        Some(File::options().write(true).open("/dev/null").unwrap()),
    ];
    for proof in proofs {
        let (helper, observations) = helper("pipe_helper_child", None);
        let mut input = stream(File::open("/dev/null").unwrap(), helper, proof);
        assert_eq!(next(&mut input).unwrap_err(), Error::InvalidDescriptor);
        joined(&input);
        assert_eq!(observations.pid.load(Ordering::Acquire), 0);
    }
}

#[test]
fn unreadable_null_input_is_not_converted_to_empty_success() {
    let (helper, observations) = helper("pipe_helper_child", None);
    let mut input = stream(
        File::options().write(true).open("/dev/null").unwrap(),
        helper,
        Some(File::open("/dev/null").unwrap()),
    );
    assert_eq!(next(&mut input).unwrap_err(), Error::InvalidDescriptor);
    joined(&input);
    assert_eq!(observations.pid.load(Ordering::Acquire), 0);
}

#[test]
fn stream_rejects_other_devices_ttys_directories_and_sockets_before_spawn() {
    let (_master, slave) = pty();
    let (socket, _peer) = UnixStream::pair().unwrap();
    for file in [
        File::open("/dev/zero").unwrap(),
        slave,
        File::open("/").unwrap(),
        OwnedFd::from(socket).into(),
    ] {
        let (helper, observations) = helper("pipe_helper_child", None);
        let mut input = stream(file, helper, Some(File::open("/dev/null").unwrap()));
        assert_eq!(next(&mut input).unwrap_err(), Error::InvalidDescriptor);
        joined(&input);
        assert_eq!(observations.pid.load(Ordering::Acquire), 0);
    }
}
