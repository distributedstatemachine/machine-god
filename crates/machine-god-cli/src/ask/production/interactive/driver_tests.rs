use super::super::support;
use super::super::*;
use machine_god_core::{
    CancellationToken, Capability, PermissionRequest, PermissionRequestId, PermissionRisk, TurnId,
};
use machine_god_native as native;
use native::{
    NativeInteractiveControl, NativeModelPreferences, NativeReasoningEffort, PermissionPrompter,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Wake, Waker};
use std::time::Duration;

struct Harness {
    driver: Driver,
    _input_writer: std::io::PipeWriter,
    work: tokio::sync::mpsc::Receiver<OutputWork>,
    ack: tokio::sync::mpsc::Sender<OutputAcknowledgement>,
    signal: tokio::sync::mpsc::Sender<AskSignal>,
    signals: AskSignals,
    bridge: Arc<NativeInteractivePromptBridge>,
}

async fn harness(fixture: &support::Fixture) -> Harness {
    assert!(Arc::ptr_eq(
        &fixture.undo,
        &fixture.host.undo_tracker().unwrap()
    ));
    assert!(Arc::ptr_eq(
        &fixture.routes,
        &fixture.host.model_routes().unwrap()
    ));
    assert!(Arc::ptr_eq(
        &fixture.observations,
        &fixture.host.observations().unwrap()
    ));
    let options = NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        NativeModelPreferences::new("fixture/default", NativeReasoningEffort::default(), false)
            .unwrap(),
    )
    .unwrap();
    let owner = NativeInteractiveSession::open(
        fixture.host.clone(),
        options,
        NativeInteractiveInitialSession::Fresh,
        100,
    )
    .await
    .unwrap();
    let (read, write) = std::io::pipe().unwrap();
    let input = NativeInteractiveInput::new(
        NativeInteractiveInputSource::AdoptNonblockingStatus(
            std::os::fd::OwnedFd::from(read).into(),
        ),
        CancellationToken::new(),
    );
    let (bridge, inbox) =
        NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
    let (send, work) = tokio::sync::mpsc::channel(1);
    let (ack, acknowledgements) = tokio::sync::mpsc::channel(1);
    let (signal, received) = tokio::sync::mpsc::channel(1);
    Harness {
        driver: Driver::new(
            owner,
            input,
            inbox,
            OutputBridge {
                work: send,
                acknowledgements,
            },
        )
        .unwrap(),
        _input_writer: write,
        work,
        ack,
        signal,
        signals: AskSignals::new(received),
        bridge,
    }
}

#[test]
fn real_tool_rounds_and_failed_save_receipt_are_observed_independently_of_output() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let result = runtime.block_on(async {
        fixture.transport.push(support::call(
            "write_file",
            &serde_json::json!({"path":"driver.txt","content":"observed"}),
        ));
        fixture.transport.push(support::answer());
        harness
            .driver
            .owner
            .enqueue("perform the write".into())
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                assert!(harness.driver.poll(cx, &mut harness.signals).is_pending());
                if matches!(
                    harness.driver.outcome,
                    Some(NativeInteractiveOutcome::Turn(_))
                ) {
                    return Poll::Ready(());
                }
                if harness.work.try_recv().is_ok() {
                    harness
                        .ack
                        .try_send(OutputAcknowledgement::Succeeded)
                        .unwrap();
                    cx.waker().wake_by_ref();
                }
                Poll::Pending
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.workspace.join("driver.txt")).unwrap(),
            "observed"
        );
        assert_eq!(fixture.transport.requests().len(), 2);
        let blocked = fixture.block_publication(&harness.driver.owner.runtime().id());
        harness
            .driver
            .owner
            .request_control(
                NativeInteractiveControl::Rename {
                    title: "must fail to save".into(),
                },
                103,
            )
            .unwrap();
        until(&mut harness, |driver| driver.control_outcome.is_some()).await;
        assert!(
            harness
                .driver
                .control_outcome
                .as_ref()
                .unwrap()
                .result
                .is_err()
        );
        assert!(harness.driver.native_failed);
        drop(blocked);
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    let _ = runtime.block_on(finish_tail(&mut tail));
}

fn executor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn until(harness: &mut Harness, condition: impl Fn(&Driver) -> bool) {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let result = harness.driver.poll(cx, &mut harness.signals);
            if condition(&harness.driver) {
                Poll::Ready(())
            } else {
                assert!(result.is_pending());
                Poll::Pending
            }
        }),
    )
    .await
    .unwrap();
}

async fn finish_signal(harness: &mut Harness) -> TurnDriveResult {
    harness.signal.send(AskSignal::Interrupt).await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)),
    )
    .await
    .unwrap()
}

struct TailHarness {
    presentation: super::FinalPresentation,
    signals: AskSignals,
    signal: tokio::sync::mpsc::Sender<AskSignal>,
    work: tokio::sync::mpsc::Receiver<OutputWork>,
    ack: tokio::sync::mpsc::Sender<OutputAcknowledgement>,
}

fn dispose(harness: Harness, fixture: support::Fixture, result: TurnDriveResult) -> TailHarness {
    let completion = harness.driver.input.input.completion();
    let presentation = harness.driver.into_presentation(result);
    completion.wait_on_worker().unwrap();
    // This asserts the complete host has exactly one strong owner before its
    // final drop/join, despite an unacknowledged output write still outstanding.
    fixture.finish();
    TailHarness {
        presentation,
        signals: harness.signals,
        signal: harness.signal,
        work: harness.work,
        ack: harness.ack,
    }
}

async fn finish_tail(harness: &mut TailHarness) -> TurnDriveResult {
    if harness.presentation.signal.is_none() {
        harness.signal.send(AskSignal::Interrupt).await.unwrap();
    }
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| harness.presentation.poll(cx, &mut harness.signals)),
    )
    .await
    .unwrap()
}

#[test]
fn ordinary_quit_joins_host_then_finishes_all_output_without_a_signal() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let result = runtime.block_on(async {
        harness.driver.shutdown();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)),
        )
        .await
        .unwrap()
    });
    let mut tail = dispose(harness, fixture, result);
    let mut writes = Vec::new();
    let result = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let result = tail.presentation.poll(cx, &mut tail.signals);
                if let Ok(work) = tail.work.try_recv() {
                    if let OutputWork::Write(bytes) = work {
                        writes.extend(bytes);
                    }
                    tail.ack.try_send(OutputAcknowledgement::Succeeded).unwrap();
                    cx.waker().wake_by_ref();
                }
                result
            }),
        )
        .await
        .unwrap()
    });
    assert_eq!(result.outcome, AskCommandOutcome::Completed);
    assert!(!result.stalled_output_after_signal);
    assert!(
        String::from_utf8(writes)
            .unwrap()
            .contains("session closed")
    );
    assert!(tail.presentation.outcomes.is_empty());
    assert!(tail.presentation.controls.is_empty());
}

#[test]
fn blocked_stdout_does_not_block_save_receipt_or_native_shutdown() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let result = runtime.block_on(async {
        // Occupy stdout before publishing a native control.
        until(&mut harness, |driver| driver.in_flight.is_some()).await;
        harness
            .driver
            .owner
            .request_control(
                NativeInteractiveControl::Rename {
                    title: "saved despite blocked output".into(),
                },
                101,
            )
            .unwrap();
        until(&mut harness, |driver| driver.control_outcome.is_some()).await;
        assert!(
            harness
                .driver
                .control_outcome
                .as_ref()
                .unwrap()
                .result
                .is_ok()
        );
        harness.driver.shutdown();
        until(&mut harness, |driver| driver.owner.is_closed()).await;
        assert!(
            harness.driver.control_outcome.is_some(),
            "typed receipt remains owned before acknowledgement"
        );
        // No signal and no stdout receipt is needed to release native ownership.
        let result = poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)).await;
        assert_eq!(result.outcome, AskCommandOutcome::Completed);
        assert!(!result.stalled_output_after_signal);
        result
    });
    let mut tail = dispose(harness, fixture, result);
    assert!(!tail.presentation.controls.is_empty());
    let result = runtime.block_on(finish_tail(&mut tail));
    assert_eq!(result.outcome, AskCommandOutcome::Interrupted);
    assert!(result.stalled_output_after_signal);
}

#[test]
fn output_failure_still_settles_native_owner_before_return() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let result = runtime.block_on(async {
        until(&mut harness, |driver| driver.in_flight.is_some()).await;
        harness
            .ack
            .send(OutputAcknowledgement::Failed)
            .await
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)),
        )
        .await
        .unwrap();
        assert_eq!(result.outcome, AskCommandOutcome::OutputFailure);
        assert!(harness.driver.owner.is_closed());
        result
    });
    let mut tail = dispose(harness, fixture, result);
    let result = runtime.block_on(poll_fn(|cx| tail.presentation.poll(cx, &mut tail.signals)));
    assert_eq!(result.outcome, AskCommandOutcome::OutputFailure);
}

fn permission(harness: &Harness, id: &str) -> PermissionRequest {
    PermissionRequest {
        id: PermissionRequestId::new(id).unwrap(),
        session_id: harness.driver.owner.runtime().id(),
        session_incarnation_id: harness.driver.owner.runtime().incarnation_id(),
        turn_id: TurnId::new("prompt-turn").unwrap(),
        capability: Capability::Custom {
            name: "display-test".into(),
            details: serde_json::json!({}),
        },
        risk: PermissionRisk::High,
        reason: "permission before display acknowledgement".into(),
    }
}

#[test]
fn prompt_only_accepts_its_exact_flushed_page_and_old_flush_cannot_activate_replacement() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let result = runtime.block_on(async {
        harness.driver.notice = None;
        let bridge = harness.bridge.clone();
        let mut first = bridge.prompt(permission(&harness, "first"));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(first.as_mut().poll(&mut cx).is_pending());
        harness.driver.poll_modal(&mut cx);
        let old = harness
            .driver
            .modal
            .as_ref()
            .unwrap()
            .presentation_binding();
        assert!(matches!(
            harness.driver.modal.as_ref().unwrap().binding(),
            InputBinding::AwaitingPrompt
        ));
        harness.driver.poll_output(&mut cx);
        assert!(matches!(
            harness.work.try_recv().unwrap(),
            OutputWork::Write(_)
        ));
        harness
            .ack
            .try_send(OutputAcknowledgement::Succeeded)
            .unwrap();
        harness.driver.poll_output(&mut cx);
        assert!(matches!(
            harness.work.try_recv().unwrap(),
            OutputWork::Flush
        ));
        assert!(!harness.driver.modal.as_ref().unwrap().displayed);
        drop(first);
        let mut second = bridge.prompt(permission(&harness, "second"));
        assert!(second.as_mut().poll(&mut cx).is_pending());
        harness.driver.poll_modal(&mut cx);
        harness
            .ack
            .try_send(OutputAcknowledgement::Succeeded)
            .unwrap();
        harness.driver.poll_output(&mut cx);
        assert!(
            !harness.driver.modal.as_ref().unwrap().displayed,
            "old successful flush cannot activate replacement"
        );
        harness.driver.line("y", &old, 101);
        assert!(second.as_mut().poll(&mut cx).is_pending());
        drop(second);
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    let _ = runtime.block_on(finish_tail(&mut tail));
}

#[derive(Default)]
struct CountWake(AtomicUsize);
impl Wake for CountWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn bounded_output_chunks_and_idle_poll_do_not_spin() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let result = runtime.block_on(async {
        harness.driver.notice = None;
        harness.driver.render = Some(Render {
            bytes: vec![b'x'; 9000],
            model_text: false,
            offset: 0,
            confirm: None,
            receipt: None,
        });
        let callbacks = Arc::new(CountWake::default());
        let waker = Waker::from(callbacks.clone());
        let mut cx = Context::from_waker(&waker);
        for expected in [4096, 4096, 808] {
            harness.driver.poll_output(&mut cx);
            let OutputWork::Write(bytes) = harness.work.try_recv().unwrap() else {
                panic!("bounded bytes");
            };
            assert_eq!(bytes.len(), expected);
            assert!(harness.work.try_recv().is_err());
            let before = callbacks.0.load(Ordering::Relaxed);
            harness.driver.poll_output(&mut cx);
            assert_eq!(
                before,
                callbacks.0.load(Ordering::Relaxed),
                "blocked ack does not self-wake"
            );
            harness
                .ack
                .try_send(OutputAcknowledgement::Succeeded)
                .unwrap();
        }
        harness.driver.poll_output(&mut cx);
        assert!(matches!(
            harness.work.try_recv().unwrap(),
            OutputWork::Flush
        ));
        harness
            .ack
            .try_send(OutputAcknowledgement::Succeeded)
            .unwrap();
        harness.driver.poll_output(&mut cx);
        let before = callbacks.0.load(Ordering::Relaxed);
        harness.driver.poll_output(&mut cx);
        assert_eq!(before, callbacks.0.load(Ordering::Relaxed));
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    let _ = runtime.block_on(finish_tail(&mut tail));
}

#[test]
fn model_chunks_escape_controls_preserve_lines_and_never_truncate_large_deltas() {
    assert!(super::output_chunk(&vec![0x80; 4096], true).is_err());
    let source = format!(
        "{}\n\u{1b}[2J\u{202e}end \"quote\" \\path",
        "a🦀".repeat(20_000)
    );
    let mut remaining = source.as_bytes();
    let mut rendered = Vec::new();
    while !remaining.is_empty() {
        let (chunk, consumed) = super::output_chunk(remaining, true).unwrap();
        assert!(consumed > 0);
        assert!(chunk.len() <= super::OUTPUT_CHUNK_BYTES);
        assert!(std::str::from_utf8(&chunk).is_ok());
        rendered.extend(chunk);
        remaining = &remaining[consumed..];
    }
    assert_eq!(
        String::from_utf8(rendered).unwrap(),
        format!(
            "{}\n\\u001b[2J\\u202eend \"quote\" \\path",
            "a🦀".repeat(20_000)
        )
    );
}
