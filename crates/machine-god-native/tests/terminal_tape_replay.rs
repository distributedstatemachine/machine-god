use std::fs::{self, File};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::CancellationToken;
use machine_god_native::{
    MAX_TERMINAL_TAPE_BYTES, TerminalTapeReplayErrorKind, TerminalTapeReplayOutput,
    TerminalTapeReplayRequest, replay_terminal_tape,
};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-terminal-tape-replay-{label}-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create temporary directory: {error}"),
            }
        }
        panic!("failed to allocate temporary directory")
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        match fs::remove_dir_all(&self.0) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if std::thread::panicking() => {
                eprintln!("failed to clean temporary directory: {error}");
            }
            Err(error) => panic!("failed to clean temporary directory: {error}"),
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(Pin::as_mut(&mut future), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("terminal tape replay unexpectedly remained pending"),
    }
}

struct Frame<'a> {
    delta_ms: i32,
    kind: u8,
    payload: &'a [u8],
}

fn tape(cols: u16, rows: u16, version: &[u8], frames: &[Frame<'_>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"FXTP\x01");
    bytes.extend_from_slice(&cols.to_le_bytes());
    bytes.extend_from_slice(&rows.to_le_bytes());
    bytes.extend_from_slice(&123_456_789_i64.to_le_bytes());
    bytes.push(u8::try_from(version.len()).expect("test version fits"));
    bytes.extend_from_slice(version);
    for frame in frames {
        bytes.extend_from_slice(&frame.delta_ms.to_le_bytes());
        bytes.push(frame.kind);
        bytes.extend_from_slice(
            &u32::try_from(frame.payload.len())
                .expect("test payload fits")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(frame.payload);
    }
    bytes
}

fn run(request: TerminalTapeReplayRequest) -> TerminalTapeReplayOutput {
    ready(replay_terminal_tape(request, CancellationToken::new())).expect("replay succeeds")
}

#[test]
fn plain_json_unknown_and_incomplete_outputs_match_fx_contract() {
    let directory = TemporaryDirectory::new("outputs");
    let plain_path = directory.path().join("plain.fxtape");
    fs::write(
        &plain_path,
        tape(
            5,
            2,
            b"vtest",
            &[Frame {
                delta_ms: 0,
                kind: 1,
                payload: b"hi",
            }],
        ),
    )
    .expect("write tape");
    let plain = run(TerminalTapeReplayRequest::new(
        plain_path, false, false, None, None,
    ));
    assert_eq!(plain.stdout(), b"|hi   |\n|     |\n");
    assert_eq!(plain.stderr(), b"");

    let json_path = directory.path().join("json.fxtape");
    let mut json_tape = tape(
        3,
        1,
        b"v\"\\\n\t\x01",
        &[
            Frame {
                delta_ms: 5,
                kind: 200,
                payload: b"ignored",
            },
            Frame {
                delta_ms: -2,
                kind: 3,
                payload: b"x",
            },
        ],
    );
    json_tape.extend_from_slice(&[1, 2, 3]);
    fs::write(&json_path, json_tape).expect("write tape");
    let json = run(TerminalTapeReplayRequest::new(
        json_path, false, true, None, None,
    ));
    assert_eq!(
        json.stdout(),
        b"{\"cols\":3,\"rows\":1,\"epoch_ms\":123456789,\"version\":\"v\\\"\\\\\\n\\t\\u0001\",\"frames\":[{\"delta_ms\":5,\"kind\":\"unknown\",\"len\":7},{\"delta_ms\":-2,\"kind\":\"resize\",\"len\":1}],\"frame_count\":2,\"resize_count\":0,\"stdout_bytes\":0}\n"
    );
    assert_eq!(
        json.stderr(),
        b"machine-god replay: ignored incomplete final tape frame\n"
    );
}

#[test]
fn frames_golden_and_frames_dir_preserve_fx_order_names_and_stale_files() {
    let directory = TemporaryDirectory::new("artifacts");
    let tape_path = directory.path().join("frames.fxtape");
    let golden_path = directory.path().join("golden.txt");
    let frames_dir = directory.path().join("rendered");
    let stale = frames_dir.join("frames").join("stale.txt");
    fs::create_dir_all(stale.parent().expect("parent")).expect("create frames directory");
    fs::write(&stale, b"preserved").expect("write stale file");
    let resize = [6, 0, 2, 0];
    fs::write(
        &tape_path,
        tape(
            5,
            2,
            b"vtest",
            &[
                Frame {
                    delta_ms: 1,
                    kind: 1,
                    payload: b"hello",
                },
                Frame {
                    delta_ms: 2,
                    kind: 5,
                    payload: b"hello",
                },
                Frame {
                    delta_ms: 3,
                    kind: 3,
                    payload: &resize,
                },
            ],
        ),
    )
    .expect("write tape");

    let output = run(TerminalTapeReplayRequest::new(
        tape_path,
        true,
        true,
        Some(golden_path.clone()),
        Some(frames_dir.clone()),
    ));
    let stdout = String::from_utf8(output.stdout().to_vec()).expect("UTF-8 output");
    assert!(stdout.starts_with("\n--- frame 1 (stdout, +1ms) ---\n|hello|\n|     |\n"));
    assert!(!stdout.contains("--- frame 2"));
    assert!(stdout.contains("--- frame 3 (resize, +3ms) ---"));
    assert!(stdout.ends_with("\"frame_count\":3,\"resize_count\":1,\"stdout_bytes\":5}\n"));
    assert_eq!(output.stderr(), b"");
    assert_eq!(
        fs::read(&golden_path).expect("read golden"),
        b"|hello |\n|      |\n"
    );
    assert_eq!(fs::read(&stale).expect("read stale file"), b"preserved");
    assert!(frames_dir.join("frames/0001.json").is_file());
    assert!(frames_dir.join("frames/0003.grid.txt").is_file());
    assert!(frames_dir.join("manifest.json").is_file());
    let metadata =
        fs::read_to_string(frames_dir.join("frames/0002.json")).expect("read marker metadata");
    assert!(metadata.contains("\"visible_markers\":[\"hello\"]"));
}

#[test]
fn future_is_inert_and_pre_cancelled_request_has_no_filesystem_effects() {
    let directory = TemporaryDirectory::new("inert");
    let missing = directory.path().join("missing.fxtape");
    let frames_dir = directory.path().join("frames-output");
    let future = replay_terminal_tape(
        TerminalTapeReplayRequest::new(
            missing.clone(),
            false,
            false,
            None,
            Some(frames_dir.clone()),
        ),
        CancellationToken::new(),
    );
    assert!(!frames_dir.exists());
    drop(future);
    assert!(!frames_dir.exists());

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    let error = ready(replay_terminal_tape(
        TerminalTapeReplayRequest::new(missing, false, false, None, Some(frames_dir.clone())),
        cancellation,
    ))
    .expect_err("cancelled replay fails");
    assert_eq!(error.kind(), TerminalTapeReplayErrorKind::Cancelled);
    assert!(!frames_dir.exists());
}

#[test]
fn request_and_output_debug_are_redacted_and_missing_code_is_stable() {
    let secret = "secret-replay-path";
    let request = TerminalTapeReplayRequest::new(
        PathBuf::from(secret),
        true,
        true,
        Some(PathBuf::from("secret-golden")),
        Some(PathBuf::from("secret-frames")),
    );
    let debug = format!("{request:?}");
    assert!(!debug.contains(secret));
    assert!(!debug.contains("secret-golden"));
    assert!(request.frames());
    assert!(request.json());
    assert_eq!(request.tape(), Path::new(secret));
    assert_eq!(request.golden(), Some(Path::new("secret-golden")));
    assert_eq!(request.frames_dir(), Some(Path::new("secret-frames")));

    let output = TerminalTapeReplayOutput::from_parts(b"hello".to_vec(), b"warning".to_vec())
        .expect("small fake output");
    let output_debug = format!("{output:?}");
    assert!(!output_debug.contains("hello"));
    assert!(!output_debug.contains("warning"));
    assert_eq!(
        output.into_parts(),
        (b"hello".to_vec(), b"warning".to_vec())
    );

    let error = ready(replay_terminal_tape(
        TerminalTapeReplayRequest::new(
            PathBuf::from("/definitely/missing/machine-god-replay.fxtape"),
            false,
            false,
            None,
            None,
        ),
        CancellationToken::new(),
    ))
    .expect_err("missing tape fails");
    assert_eq!(error.kind(), TerminalTapeReplayErrorKind::FileNotFound);
    assert_eq!(error.code(), "FileNotFound");
    assert_eq!(error.message(), "cannot open tape: FileNotFound");
    assert!(!format!("{error:?}").contains("machine-god-replay"));
}

#[test]
fn exclusive_input_cap_is_checked() {
    let directory = TemporaryDirectory::new("input-cap");
    let path = directory.path().join("too-large.fxtape");
    let file = File::create(&path).expect("create sparse tape");
    file.set_len(u64::try_from(MAX_TERMINAL_TAPE_BYTES).expect("cap fits u64"))
        .expect("size sparse tape");
    drop(file);

    let error = ready(replay_terminal_tape(
        TerminalTapeReplayRequest::new(path, false, false, None, None),
        CancellationToken::new(),
    ))
    .expect_err("exclusive cap rejects exact bound");
    assert_eq!(error.kind(), TerminalTapeReplayErrorKind::ResourceLimit);
}
