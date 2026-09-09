use super::*;
use machine_god_core::CancellationToken;
use machine_god_native::{
    NativeOwnedWorkerScope, TerminalTapeRecorder, TerminalTapeRecordingDestination,
    TerminalTapeRecordingOptions, TerminalTapeRecordingRequest, TerminalTapeReplayRequest,
    replay_terminal_tape,
};
use std::{
    fs,
    future::poll_fn,
    io::{self, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(in crate::ask::production) struct Fixture(PathBuf);
impl Fixture {
    pub(in crate::ask::production) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mg-cli-tape-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    pub(in crate::ask::production) fn request(&self, stdin: bool) -> TerminalTapeRecordingRequest {
        let mut options = TerminalTapeRecordingOptions::new(12, 2, 100, b"test".to_vec());
        options.record_stdin = stdin;
        TerminalTapeRecordingRequest {
            destination: TerminalTapeRecordingDestination::Explicit(self.0.join("tape.fxtape")),
            options,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

pub(super) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

struct ShortWriter {
    bytes: Vec<u8>,
    stop: usize,
    calls: usize,
}
impl Write for ShortWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.bytes.len() == self.stop {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let count = bytes.len().min(2).min(self.stop - self.bytes.len());
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn actual_stdout_prefix_survives_short_writes_interruptions_and_partial_errors() {
    for stop in [0, 3, 6] {
        let mut output = ShortWriter {
            bytes: Vec::new(),
            stop,
            calls: 0,
        };
        let (work, received) = tokio::sync::mpsc::channel(1);
        let (ack, mut received_ack) = tokio::sync::mpsc::channel(1);
        work.try_send(OutputWork::Write(b"abcdef".to_vec()))
            .unwrap();
        drop(work);
        serve_output_with_clock(received, &ack, &mut output, || Ok(123));
        assert_eq!(
            received_ack.try_recv().unwrap(),
            OutputAcknowledgement::Written {
                timestamp_ms: Ok(123),
                bytes: b"abcdef"[..stop].to_vec(),
                failed: stop < 6,
            }
        );
        assert_eq!(output.bytes, b"abcdef"[..stop]);
    }
}

#[test]
fn zero_progress_and_repeated_interruptions_are_bounded_failures() {
    struct NoProgress {
        interrupted: bool,
        calls: usize,
    }
    impl Write for NoProgress {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if self.interrupted {
                Err(io::ErrorKind::Interrupted.into())
            } else {
                Ok(0)
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }
    for interrupted in [false, true] {
        let mut output = NoProgress {
            interrupted,
            calls: 0,
        };
        assert_eq!(write_prefix(&mut output, b"x"), (0, true));
        assert_eq!(output.calls, if interrupted { 4097 } else { 1 });
    }
}

#[test]
fn output_bridge_roundtrips_exact_stdout_and_opted_in_events_through_real_replay() {
    let fixture = Fixture::new();
    let scope = NativeOwnedWorkerScope::new();
    runtime().block_on(async {
        let recorder = TerminalTapeRecorder::start(
            fixture.request(true),
            scope.clone(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let completion = recorder.completion();
        let path = recorder.path().to_owned();
        let (work, _received) = tokio::sync::mpsc::channel(1);
        let (ack, acknowledgements) = tokio::sync::mpsc::channel(1);
        let mut output = OutputBridge {
            work,
            acknowledgements,
            tape: Some(tape::TapeLane::new(recorder, true)),
        };
        let lane = output.tape.as_mut().unwrap();
        lane.stdin(b"raw\x03\xff");
        lane.resize(10, 2);
        lane.sigint();
        lane.marker(b"cli-test");
        ack.send(OutputAcknowledgement::Written {
            timestamp_ms: Ok(107),
            bytes: b"accepted".to_vec(),
            failed: false,
        })
        .await
        .unwrap();
        assert_eq!(
            output.acknowledgement().await,
            Some(OutputAcknowledgement::Succeeded)
        );
        poll_fn(|cx| output.poll_finish_tape(cx)).await.unwrap();
        assert!(completion.status().complete);
        assert!(!completion.workers().is_complete());
        drop(output);
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        assert!(scope.completion().is_complete());
        let replay = replay_terminal_tape(
            TerminalTapeReplayRequest::new(path, false, true, None, None),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let summary: serde_json::Value = serde_json::from_slice(replay.stdout()).unwrap();
        assert_eq!(summary["stdout_bytes"], 8);
        assert_eq!(summary["frame_count"], 5);
        assert_eq!(summary["resize_count"], 1);
    });
}

#[test]
fn partial_output_failure_records_only_accepted_prefix_then_closes_incomplete() {
    let fixture = Fixture::new();
    let scope = NativeOwnedWorkerScope::new();
    runtime().block_on(async {
        let recorder = TerminalTapeRecorder::start(
            fixture.request(false),
            scope.clone(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let completion = recorder.completion();
        let path = recorder.path().to_owned();
        let (work, received) = tokio::sync::mpsc::channel(1);
        let (ack, acknowledgements) = tokio::sync::mpsc::channel(1);
        let mut output = OutputBridge {
            work,
            acknowledgements,
            tape: Some(tape::TapeLane::new(recorder, false)),
        };
        output.tape.as_mut().unwrap().stdin(b"must-not-record");
        let thread = std::thread::spawn(move || {
            let mut writer = ShortWriter {
                bytes: Vec::new(),
                stop: 3,
                calls: 0,
            };
            serve_output(received, &ack, &mut writer);
            writer.bytes
        });
        output
            .work
            .try_send(OutputWork::Write(b"accepted-suffix".to_vec()))
            .unwrap();
        assert_eq!(
            output.acknowledgement().await,
            Some(OutputAcknowledgement::Failed)
        );
        drop(output);
        assert_eq!(thread.join().unwrap(), b"acc");
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        let status = completion.status();
        assert!(status.closed && !status.complete && status.failure.is_some());
        assert_eq!(status.frames, 1);
        let replay = replay_terminal_tape(
            TerminalTapeReplayRequest::new(path, false, false, None, None),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(replay.stdout(), b"|acc         |\n|            |\n");
    });
}

#[test]
fn native_recording_limit_failure_is_an_explicit_output_failure() {
    let fixture = Fixture::new();
    let scope = NativeOwnedWorkerScope::new();
    runtime().block_on(async {
        let mut request = fixture.request(false);
        request.options.max_frames = 1;
        let recorder =
            TerminalTapeRecorder::start(request, scope.clone(), CancellationToken::new())
                .await
                .unwrap();
        let completion = recorder.completion();
        let (work, _received) = tokio::sync::mpsc::channel(1);
        let (ack, acknowledgements) = tokio::sync::mpsc::channel(1);
        let mut output = OutputBridge {
            work,
            acknowledgements,
            tape: Some(tape::TapeLane::new(recorder, false)),
        };
        output.tape.as_mut().unwrap().marker(b"one frame allowed");
        ack.send(OutputAcknowledgement::Written {
            timestamp_ms: Ok(107),
            bytes: b"accepted".to_vec(),
            failed: false,
        })
        .await
        .unwrap();
        assert_eq!(
            output.acknowledgement().await,
            Some(OutputAcknowledgement::Failed)
        );
        drop(output);
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        assert_eq!(
            completion.status().failure,
            Some(machine_god_native::TerminalTapeRecordingError::LimitExceeded)
        );
        assert!(!completion.status().complete);
    });
}
