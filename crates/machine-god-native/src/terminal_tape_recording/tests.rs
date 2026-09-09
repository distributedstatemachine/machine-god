use super::*;
use futures_executor::block_on;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mg-tape-recording-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn request(&self) -> TerminalTapeRecordingRequest {
        TerminalTapeRecordingRequest {
            destination: TerminalTapeRecordingDestination::Explicit(self.0.join("tape.fxtape")),
            options: TerminalTapeRecordingOptions::new(20, 3, 100, b"test-version".to_vec()),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn settle(scope: &NativeOwnedWorkerScope) {
    scope.close();
    scope.completion().wait_on_worker().unwrap();
    assert!(scope.completion().is_complete());
}
fn frames(bytes: &[u8]) -> Vec<(i32, u8, Vec<u8>)> {
    assert_eq!(&bytes[..5], b"FXTP\x01");
    let mut offset = 18 + usize::from(bytes[17]);
    let mut result = Vec::new();
    while offset < bytes.len() {
        let delta = i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let kind = bytes[offset + 4];
        let count = u32::from_le_bytes(bytes[offset + 5..offset + 9].try_into().unwrap()) as usize;
        offset += 9;
        result.push((delta, kind, bytes[offset..offset + count].to_vec()));
        offset += count;
    }
    result
}

#[test]
fn full_tape_roundtrips_through_real_replay_with_exact_header_and_event_kinds() {
    let fixture = Fixture::new();
    let scope = NativeOwnedWorkerScope::new();
    let mut request = fixture.request();
    request.options.record_stdin = true;
    let mut recorder = block_on(TerminalTapeRecorder::start(
        request,
        scope.clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    let path = recorder.path().to_owned();
    let events = [
        (TerminalTapeRecordingFrame::StdoutWritten(b"accepted"), 107),
        (TerminalTapeRecordingFrame::StdinAccepted(b"input"), 106),
        (
            TerminalTapeRecordingFrame::Resize { cols: 10, rows: 2 },
            i64::MAX,
        ),
        (TerminalTapeRecordingFrame::Sigint, i64::MIN),
        (TerminalTapeRecordingFrame::Marker(b"marker"), i64::MAX),
    ];
    for (frame, time) in events {
        block_on(recorder.record(frame, time, CancellationToken::new())).unwrap();
    }
    let receipt = block_on(recorder.finish()).unwrap();
    assert!(receipt.complete && receipt.closed && !receipt.active);
    assert_eq!(receipt.frames, 5);
    assert!(!recorder.completion().workers().is_complete());
    settle(&scope);
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, receipt.bytes_written);
    assert_eq!(&bytes[5..9], &[20, 0, 3, 0]);
    assert_eq!(&bytes[9..17], &100_i64.to_le_bytes());
    assert_eq!(bytes[17], 12);
    assert_eq!(&bytes[18..30], b"test-version");
    let frames = frames(&bytes);
    assert_eq!(
        frames.iter().map(|frame| frame.0).collect::<Vec<_>>(),
        [7, 0, i32::MAX, 0, i32::MAX]
    );
    assert_eq!(
        frames.iter().map(|frame| frame.1).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );
    assert_eq!(frames[0].2, b"accepted");
    assert_eq!(frames[2].2, [10, 0, 2, 0]);
    let replay = block_on(crate::replay_terminal_tape(
        crate::TerminalTapeReplayRequest::new(path, false, false, None, None),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(replay.stdout(), b"|accepted  |\n|          |\n");
    assert!(replay.stderr().is_empty());
}

#[test]
fn input_requires_opt_in_and_empty_output_does_not_create_frames() {
    let fixture = Fixture::new();
    let scope = NativeOwnedWorkerScope::new();
    let mut recorder = block_on(TerminalTapeRecorder::start(
        fixture.request(),
        scope.clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    for frame in [
        TerminalTapeRecordingFrame::StdinAccepted(b"PRIVATE INPUT"),
        TerminalTapeRecordingFrame::StdoutWritten(b""),
    ] {
        assert_eq!(
            block_on(recorder.record(frame, 101, CancellationToken::new()))
                .unwrap()
                .frames,
            0
        );
    }
    block_on(recorder.finish()).unwrap();
    settle(&scope);
    assert!(frames(&fs::read(recorder.path()).unwrap()).is_empty());
}

#[test]
fn unpolled_invalid_and_precancelled_start_have_no_destination_effects() {
    let fixture = Fixture::new();
    let scope = NativeOwnedWorkerScope::new();
    drop(TerminalTapeRecorder::start(
        fixture.request(),
        scope.clone(),
        CancellationToken::new(),
    ));
    let mut invalid = fixture.request();
    invalid.options.version = vec![b'v'; 256];
    assert_eq!(
        block_on(TerminalTapeRecorder::start(
            invalid,
            scope.clone(),
            CancellationToken::new()
        ))
        .unwrap_err(),
        TerminalTapeRecordingError::InvalidRequest
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        block_on(TerminalTapeRecorder::start(
            fixture.request(),
            scope.clone(),
            cancellation
        ))
        .unwrap_err(),
        TerminalTapeRecordingError::Cancelled
    );
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
    settle(&scope);
}

#[test]
fn explicit_paths_refuse_overwrites_final_symlinks_and_symlink_ancestors() {
    let fixture = Fixture::new();
    let existing = fixture.0.join("existing");
    fs::write(&existing, b"retained").unwrap();
    symlink(&existing, fixture.0.join("link")).unwrap();
    fs::create_dir(fixture.0.join("real")).unwrap();
    symlink(fixture.0.join("real"), fixture.0.join("alias")).unwrap();
    for path in [
        existing.clone(),
        fixture.0.join("link"),
        fixture.0.join("alias/new"),
    ] {
        let scope = NativeOwnedWorkerScope::new();
        let mut request = fixture.request();
        request.destination = TerminalTapeRecordingDestination::Explicit(path);
        assert!(
            block_on(TerminalTapeRecorder::start(
                request,
                scope.clone(),
                CancellationToken::new()
            ))
            .is_err()
        );
        settle(&scope);
    }
    assert_eq!(fs::read(existing).unwrap(), b"retained");
    assert!(!fixture.0.join("real/new").exists());
}

#[test]
fn automatic_recordings_use_retained_state_identity_and_private_exclusive_files() {
    let fixture = Fixture::new();
    let original = fixture.0.join("state");
    fs::create_dir(&original).unwrap();
    let store = Arc::new(FileSessionStore::open(&original).unwrap());
    let retained = fixture.0.join("retained");
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    let scope = NativeOwnedWorkerScope::new();
    let mut names = Vec::new();
    for _ in 0..2 {
        let mut request = fixture.request();
        request.destination = TerminalTapeRecordingDestination::Automatic {
            store: store.clone(),
            state_path: original.clone(),
        };
        let mut recorder = block_on(TerminalTapeRecorder::start(
            request,
            scope.clone(),
            CancellationToken::new(),
        ))
        .unwrap();
        names.push(recorder.path().file_name().unwrap().to_owned());
        block_on(recorder.finish()).unwrap();
    }
    settle(&scope);
    assert_ne!(names[0], names[1]);
    assert!(!original.join("recordings").exists());
    assert_eq!(
        fs::metadata(retained.join("recordings"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    for name in names {
        assert_eq!(
            fs::metadata(retained.join("recordings").join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn recording_limits_fail_explicitly_and_preserve_replayable_complete_prefix() {
    for byte_limit in [false, true] {
        let fixture = Fixture::new();
        let scope = NativeOwnedWorkerScope::new();
        let mut request = fixture.request();
        if byte_limit {
            request.options.max_bytes = 40;
        } else {
            request.options.max_frames = 1;
        }
        let mut recorder = block_on(TerminalTapeRecorder::start(
            request,
            scope.clone(),
            CancellationToken::new(),
        ))
        .unwrap();
        block_on(recorder.record(
            TerminalTapeRecordingFrame::StdoutWritten(b"a"),
            101,
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(
            block_on(recorder.record(
                TerminalTapeRecordingFrame::StdoutWritten(b"b"),
                102,
                CancellationToken::new()
            ))
            .unwrap_err(),
            TerminalTapeRecordingError::LimitExceeded
        );
        let final_status = block_on(recorder.finish()).unwrap();
        assert!(final_status.closed && !final_status.complete);
        assert_eq!(
            final_status.failure,
            Some(TerminalTapeRecordingError::LimitExceeded)
        );
        assert_eq!(final_status.frames, 1);
        settle(&scope);
        assert_eq!(frames(&fs::read(recorder.path()).unwrap()).len(), 1);
    }
}

#[test]
fn oversized_frame_is_not_allocated_or_admitted_and_final_status_stays_failed() {
    let fixture = Fixture::new();
    let scope = NativeOwnedWorkerScope::new();
    let mut recorder = block_on(TerminalTapeRecorder::start(
        fixture.request(),
        scope.clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    let observer = recorder.completion();
    let bytes = vec![b'x'; MAX_TERMINAL_TAPE_RECORDING_FRAME_BYTES + 1];
    assert_eq!(
        block_on(recorder.record(
            TerminalTapeRecordingFrame::StdoutWritten(&bytes),
            101,
            CancellationToken::new()
        ))
        .unwrap_err(),
        TerminalTapeRecordingError::LimitExceeded
    );
    drop(recorder);
    settle(&scope);
    assert_eq!(
        observer.status().failure,
        Some(TerminalTapeRecordingError::LimitExceeded)
    );
    assert!(observer.status().closed && !observer.status().complete);
}

#[derive(Default)]
struct Probe {
    bytes: Vec<u8>,
    calls: usize,
    max_chunk: usize,
    fail_at: Option<usize>,
    interrupted: bool,
    flush_error: bool,
    sync_error: bool,
    flushes: usize,
    syncs: usize,
    dropped: bool,
    cancel: Option<CancellationToken>,
}
struct Sink(Arc<Mutex<Probe>>);
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut probe = self.0.lock().unwrap();
        probe.calls += 1;
        if probe.interrupted {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        if probe.fail_at == Some(probe.calls) {
            return Err(io::Error::other("PRIVATE NATIVE FAILURE"));
        }
        let count = bytes.len().min(probe.max_chunk);
        probe.bytes.extend_from_slice(&bytes[..count]);
        if let Some(token) = &probe.cancel {
            token.cancel();
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        let mut probe = self.0.lock().unwrap();
        probe.flushes += 1;
        if probe.flush_error {
            Err(io::Error::other("flush"))
        } else {
            Ok(())
        }
    }
}
impl TapeSink for Sink {
    fn sync(&mut self) -> io::Result<()> {
        let mut probe = self.0.lock().unwrap();
        probe.syncs += 1;
        if probe.sync_error {
            Err(io::Error::other("sync"))
        } else {
            Ok(())
        }
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        self.0.lock().unwrap().dropped = true;
    }
}
fn writer(probe: Arc<Mutex<Probe>>) -> TapeWriter {
    TapeWriter {
        sink: Some(Box::new(Sink(probe))),
        status: Arc::new(Mutex::new(TerminalTapeRecordingStatus {
            active: true,
            ..TerminalTapeRecordingStatus::pending()
        })),
        options: TerminalTapeRecordingOptions::new(20, 3, 100, vec![]),
        last_ms: 100,
    }
}

#[test]
fn partial_write_zero_progress_interrupted_exhaustion_and_flush_failures_are_typed() {
    for (max_chunk, fail_at, interrupted, expected, bytes) in [
        (
            4,
            Some(3),
            false,
            TerminalTapeRecordingError::WriteFailed,
            8,
        ),
        (0, None, false, TerminalTapeRecordingError::WriteFailed, 0),
        (4, None, true, TerminalTapeRecordingError::LimitExceeded, 0),
    ] {
        let probe = Arc::new(Mutex::new(Probe {
            max_chunk,
            fail_at,
            interrupted,
            ..Probe::default()
        }));
        let mut writer = writer(probe.clone());
        let error = writer
            .frame(
                &FramePayload {
                    kind: 1,
                    bytes: b"abcdef".to_vec(),
                },
                101,
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(error, expected);
        let status = writer.close(Some(error));
        assert_eq!(status.bytes_written, bytes);
        assert_eq!(status.frames, 0);
        assert_eq!(status.failure, Some(expected));
        assert!(probe.lock().unwrap().dropped);
        if interrupted {
            assert_eq!(probe.lock().unwrap().calls, MAX_WRITE_ATTEMPTS);
        }
    }
    for flush_error in [true, false] {
        let probe = Arc::new(Mutex::new(Probe {
            max_chunk: 64,
            flush_error,
            sync_error: !flush_error,
            ..Probe::default()
        }));
        let mut writer = writer(probe.clone());
        let status = writer.close(None);
        assert_eq!(
            status.failure,
            Some(TerminalTapeRecordingError::FlushFailed)
        );
        assert!(status.closed && !status.complete && probe.lock().unwrap().dropped);
    }
}

#[test]
fn fully_written_zero_payload_frame_keeps_its_receipt_after_post_write_cancellation() {
    let cancellation = CancellationToken::new();
    let probe = Arc::new(Mutex::new(Probe {
        max_chunk: 64,
        cancel: Some(cancellation.clone()),
        ..Probe::default()
    }));
    let mut writer = writer(probe);
    assert_eq!(
        writer
            .frame(
                &FramePayload {
                    kind: 4,
                    bytes: vec![]
                },
                101,
                &cancellation
            )
            .unwrap_err(),
        TerminalTapeRecordingError::Cancelled
    );
    let status = writer.close(Some(TerminalTapeRecordingError::Cancelled));
    assert_eq!(status.frames, 1);
    assert_eq!(status.bytes_written, 9);
}

struct Noop;
impl Wake for Noop {
    fn wake(self: Arc<Self>) {}
}

#[test]
fn dropped_receipt_and_recorder_keep_started_writes_scoped_until_file_close() {
    struct Blocked {
        inner: Sink,
        entered: Option<mpsc::SyncSender<()>>,
        release: mpsc::Receiver<()>,
    }
    impl Write for Blocked {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                entered.send(()).unwrap();
                self.release.recv().unwrap();
            }
            self.inner.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }
    impl TapeSink for Blocked {
        fn sync(&mut self) -> io::Result<()> {
            self.inner.sync()
        }
    }
    let scope = NativeOwnedWorkerScope::new();
    let probe = Arc::new(Mutex::new(Probe {
        max_chunk: 64,
        ..Probe::default()
    }));
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let mut writer = writer(probe.clone());
    writer.sink = Some(Box::new(Blocked {
        inner: Sink(probe.clone()),
        entered: Some(entered),
        release: gate,
    }));
    probe.lock().unwrap().dropped = false;
    let completion = TerminalTapeRecordingCompletion {
        status: writer.status.clone(),
        workers: scope.completion(),
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    scope.spawn(move || serve(writer, receiver)).unwrap();
    let mut recorder = TerminalTapeRecorder {
        sender: Some(sender),
        path: PathBuf::new(),
        record_stdin: false,
        completion: completion.clone(),
    };
    let mut future = recorder.record(
        TerminalTapeRecordingFrame::StdoutWritten(b"kept"),
        101,
        CancellationToken::new(),
    );
    let waker = Waker::from(Arc::new(Noop));
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    ready
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    drop(future);
    let mut queued = recorder.record(
        TerminalTapeRecordingFrame::StdoutWritten(b"queued"),
        102,
        CancellationToken::new(),
    );
    assert!(matches!(
        queued.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    drop(queued);
    assert_eq!(
        block_on(recorder.record(
            TerminalTapeRecordingFrame::StdoutWritten(b"not admitted"),
            103,
            CancellationToken::new(),
        ))
        .unwrap_err(),
        TerminalTapeRecordingError::Busy
    );
    drop(recorder);
    scope.close();
    assert!(!completion.workers().is_complete());
    assert!(!probe.lock().unwrap().dropped);
    release.send(()).unwrap();
    completion.workers().wait_on_worker().unwrap();
    assert!(probe.lock().unwrap().dropped);
    assert_eq!(completion.status().frames, 2);
    assert_eq!(
        completion.status().failure,
        Some(TerminalTapeRecordingError::Abandoned)
    );
    assert!(completion.status().closed);
}

#[test]
fn reply_publisher_survives_panicking_waker_and_abandoned_receipt() {
    struct PanicWake(AtomicBool);
    impl Wake for PanicWake {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
            panic!("injected wake failure");
        }
    }
    let (sender, mut receiver) = reply::channel::<usize>();
    let wake = Arc::new(PanicWake(AtomicBool::new(false)));
    let waker = Waker::from(wake.clone());
    assert!(matches!(
        std::pin::Pin::new(&mut receiver).poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    sender.send(Ok(7));
    assert!(wake.0.load(Ordering::SeqCst));
    assert_eq!(block_on(receiver).unwrap(), 7);
    let (sender, receiver) = reply::channel::<usize>();
    drop(receiver);
    sender.send(Ok(9));
}
