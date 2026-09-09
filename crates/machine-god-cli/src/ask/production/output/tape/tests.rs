use super::super::tests::{Fixture, runtime};
use super::*;
use machine_god_native::NativeOwnedWorkerScope;
use std::future::poll_fn;

#[test]
fn bounded_queue_exhaustion_is_explicit_incomplete_not_eviction_or_detached_work() {
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
        let mut lane = TapeLane::new(recorder, true);
        let (_release, until) = tokio::sync::oneshot::channel();
        lane.hold_for_test(until);
        for _ in 0..MAX_QUEUED_EVENTS {
            lane.stdin(b"one chunk");
        }
        assert_eq!(lane.queue.len(), MAX_QUEUED_EVENTS);
        lane.sigint();
        assert!(lane.failed);
        assert!(lane.queue.is_empty());
        assert!(poll_fn(|cx| lane.poll_finish(cx)).await.is_err());
        drop(lane);
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        assert!(completion.status().closed);
        assert!(!completion.status().complete);
        assert!(completion.status().failure.is_some());
    });
}

#[test]
fn blocked_recording_retains_stdout_and_wakes_after_receipt_without_a_thread_per_frame() {
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
        let mut lane = TapeLane::new(recorder, false);
        let (release, until) = tokio::sync::oneshot::channel();
        lane.hold_for_test(until);
        let bytes = vec![b'x'; MAX_TERMINAL_TAPE_RECORDING_FRAME_BYTES + 1];
        lane.stdout(bytes.clone(), false);
        poll_fn(|cx| {
            assert!(matches!(lane.poll_stdout(cx), Some(Poll::Pending)));
            Poll::Ready(())
        })
        .await;
        assert_eq!(lane.stdout.as_ref().unwrap().bytes, bytes);
        assert!(!completion.status().complete);
        release.send(()).unwrap();
        assert_eq!(
            poll_fn(|cx| lane.poll_stdout(cx).unwrap()).await,
            OutputAcknowledgement::Succeeded
        );
        assert!(lane.stdout.is_none());
        poll_fn(|cx| lane.poll_finish(cx)).await.unwrap();
        assert_eq!(completion.status().frames, 2);
        drop(lane);
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        assert!(completion.status().complete);
    });
}

#[test]
fn clock_failure_on_an_accepted_stdout_receipt_reports_failure_without_panicking() {
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
        let mut lane = TapeLane::new(recorder, false);
        lane.clock = || Err(());
        lane.stdout(b"accepted".to_vec(), false);
        assert_eq!(
            poll_fn(|cx| lane.poll_stdout(cx).unwrap()).await,
            OutputAcknowledgement::Failed
        );
        drop(lane);
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        assert!(!completion.status().complete);
        assert!(completion.status().failure.is_some());
    });
}
