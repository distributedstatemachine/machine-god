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
    input_writer: std::io::PipeWriter,
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
                tape: None,
                work: send,
                acknowledgements,
            },
        )
        .unwrap(),
        input_writer: write,
        work,
        ack,
        signal,
        signals: AskSignals::new(received),
        bridge,
    }
}

#[test]
fn pending_tape_ack_never_owns_native_save_progress_or_host_input_cleanup() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let tape_scope = native::NativeOwnedWorkerScope::new();
    let recorder = runtime
        .block_on(native::TerminalTapeRecorder::start(
            native::TerminalTapeRecordingRequest {
                destination: native::TerminalTapeRecordingDestination::Explicit(
                    fixture.workspace.join("driver-tape.fxtape"),
                ),
                options: native::TerminalTapeRecordingOptions::new(20, 3, 100, b"test".to_vec()),
            },
            tape_scope.clone(),
            CancellationToken::new(),
        ))
        .unwrap();
    let tape_completion = recorder.completion();
    let mut lane = super::super::super::output::tape::TapeLane::new(recorder, false);
    let (_release, held) = tokio::sync::oneshot::channel();
    lane.hold_for_test(held);
    harness.driver.output.tape = Some(lane);
    let result = runtime.block_on(async {
        harness
            .driver
            .owner
            .request_control(
                NativeInteractiveControl::Rename {
                    title: "save while recording waits".into(),
                },
                101,
            )
            .unwrap();
        poll_fn(|cx| {
            assert!(harness.driver.poll(cx, &mut harness.signals).is_pending());
            if let Ok(OutputWork::Write(bytes)) = harness.work.try_recv() {
                harness
                    .ack
                    .try_send(OutputAcknowledgement::Written {
                        timestamp_ms: Ok(101),
                        bytes,
                        failed: false,
                    })
                    .unwrap();
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
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
        assert!(harness.driver.in_flight.is_some());
        assert!(!tape_completion.status().closed);
        finish_signal(&mut harness).await
    });
    // dispose joins the complete native host and input while the separate tape
    // still owns an unacknowledged accepted stdout prefix.
    let mut tail = dispose(harness, fixture, result);
    assert!(!tape_completion.status().closed);
    let result = runtime.block_on(finish_tail(&mut tail));
    assert_eq!(result.outcome, AskCommandOutcome::Interrupted);
    assert!(result.stalled_output_after_signal);
    drop(tail);
    tape_scope.close();
    tape_scope.completion().wait_on_worker().unwrap();
    assert!(tape_completion.status().closed);
    assert!(!tape_completion.status().complete);
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
fn final_presentation_records_shutdown_output_after_native_host_has_joined() {
    use super::super::super::output;
    let runtime = executor();
    let fixture = support::Fixture::new();
    let tape_fixture = output::tests::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let tape_scope = native::NativeOwnedWorkerScope::new();
    let recorder = runtime
        .block_on(native::TerminalTapeRecorder::start(
            tape_fixture.request(false),
            tape_scope.clone(),
            CancellationToken::new(),
        ))
        .unwrap();
    let path = recorder.path().to_owned();
    let completion = recorder.completion();
    harness.driver.output.tape = Some(output::tape::TapeLane::new(recorder, false));
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
    assert!(!completion.status().closed);
    let mut accepted = Vec::new();
    let result = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let result = tail.presentation.poll(cx, &mut tail.signals);
                if let Ok(work) = tail.work.try_recv() {
                    let receipt = match work {
                        OutputWork::Write(bytes) => {
                            accepted.extend_from_slice(&bytes);
                            OutputAcknowledgement::Written {
                                timestamp_ms: Ok(101),
                                bytes,
                                failed: false,
                            }
                        }
                        OutputWork::Flush => OutputAcknowledgement::Succeeded,
                    };
                    tail.ack.try_send(receipt).unwrap();
                    cx.waker().wake_by_ref();
                }
                result
            }),
        )
        .await
        .unwrap()
    });
    assert_eq!(result.outcome, AskCommandOutcome::Completed);
    assert!(completion.status().closed && completion.status().complete);
    assert!(!completion.workers().is_complete());
    drop(tail);
    tape_scope.close();
    tape_scope.completion().wait_on_worker().unwrap();
    let replay = runtime
        .block_on(native::replay_terminal_tape(
            native::TerminalTapeReplayRequest::new(path, false, true, None, None),
            CancellationToken::new(),
        ))
        .unwrap();
    let summary: serde_json::Value = serde_json::from_slice(replay.stdout()).unwrap();
    assert_eq!(summary["stdout_bytes"], accepted.len());
    assert!(
        String::from_utf8(accepted)
            .unwrap()
            .contains("session closed")
    );
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
fn allowlist_save_receipt_survives_blocked_output_and_native_free_shutdown() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let store = Arc::new(native::NativeUserConfigStore::new(
        fixture.workspace.join("allowlist-user"),
    ));
    let mut harness = runtime.block_on(harness(&fixture));
    harness.driver = harness.driver.with_resources(None, Some(store.clone()));
    let result = runtime.block_on(async {
        until(&mut harness, |driver| driver.in_flight.is_some()).await;
        harness.driver.command("/allowlist add tool read_file", 101);
        until(&mut harness, |driver| driver.control_outcome.is_some()).await;
        assert!(matches!(
            &harness.driver.control_outcome.as_ref().unwrap().result,
            Ok(native::NativeInteractiveControlReceipt::Allowlist(
                native::NativeAllowlistReceipt::Mutation {
                    outcome: native::NativeConfiguredPermissionMutationOutcome::Changed { .. },
                    reload: Some(Ok(())),
                    ..
                }
            ))
        ));
        let snapshot = store.load().unwrap();
        assert_eq!(
            snapshot
                .loaded()
                .config()
                .permission_sources(&fixture.workspace)
                .unwrap()
                .effective()
                .rules()[0]
                .permission(),
            "read"
        );
        assert!(fixture.transport.requests().is_empty());
        harness.driver.shutdown();
        until(&mut harness, |driver| driver.owner.is_closed()).await;
        assert!(harness.driver.control_outcome.is_some());
        poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)).await
    });
    assert_eq!(result.outcome, AskCommandOutcome::Completed);
    let mut tail = dispose(harness, fixture, result);
    assert_eq!(tail.presentation.controls.len(), 1);
    let (result, output) = runtime.block_on(finish_raw_tail(&mut tail));
    assert_eq!(result.outcome, AskCommandOutcome::Completed);
    assert!(String::from_utf8_lossy(&output).contains("settings saved"));
    assert!(tail.presentation.controls.is_empty());
}

#[test]
fn allowlist_renderer_keeps_publication_uncertainty_and_runtime_reload_failure_distinct() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let store = Arc::new(native::NativeUserConfigStore::new(
        fixture.workspace.join("allowlist-user"),
    ));
    let mut harness = runtime.block_on(harness(&fixture));
    harness.driver = harness.driver.with_resources(None, Some(store));
    let result = runtime.block_on(async {
        harness.driver.command("/allowlist add tool read_file", 101);
        until(&mut harness, |driver| driver.control_outcome.is_some()).await;
        let mut outcome = harness.driver.control_outcome.take().unwrap();
        let Ok(native::NativeInteractiveControlReceipt::Allowlist(
            native::NativeAllowlistReceipt::Mutation {
                reload, sources, ..
            },
        )) = &mut outcome.result
        else {
            panic!("saved allowlist mutation");
        };
        // Exercise presentation of the native failure variant with a real
        // accepted control identity; native fault-injection tests own causality.
        *reload = Some(Err(native::NativeAllowlistReloadError::Unavailable));
        *sources = None;
        let text = String::from_utf8(super::render_control(&outcome).unwrap()).unwrap();
        assert!(text.contains("settings saved; effective source unknown; runtime reload failed"));
        assert!(!text.contains("outcome uncertain"));
        assert!(super::control_failed(&outcome));
        for error in [
            native::NativeAllowlistError::Ambiguous,
            native::NativeAllowlistError::Config(native::NativeUserConfigError::CommitAmbiguous),
        ] {
            outcome.result = Err(native::NativeInteractiveControlError::Allowlist(error));
            let text = String::from_utf8(super::render_control(&outcome).unwrap()).unwrap();
            assert!(text.contains("outcome uncertain"));
            assert!(text.contains("no automatic retry"));
            assert!(!text.contains("settings saved"));
            assert!(super::control_failed(&outcome));
        }
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    let _ = runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn undo_renderer_keeps_outcomes_reasons_paths_and_bounds_distinct() {
    use native::{
        FileUndoError as Error, FileUndoOutcome as Outcome, FileUndoUnavailableReason as Reason,
        NativeInteractiveControlError as ControlError, NativeInteractiveControlReceipt as Receipt,
    };
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let result = runtime.block_on(async {
        harness.driver.command("/undo", 101);
        until(&mut harness, |driver| driver.control_outcome.is_some()).await;
        let mut receipt = harness.driver.control_outcome.take().unwrap();
        assert!(matches!(receipt.result, Ok(Receipt::Undone(Outcome::Empty))));
        assert!(String::from_utf8(super::render_control(&receipt).unwrap()).unwrap().contains("Nothing to undo."));
        for (outcome, verb) in [
            (Outcome::Restored("世界/\u{1b}[31m\n\u{202e}.txt".into()), "Restored"),
            (Outcome::Removed("世界/\u{1b}[31m\n\u{202e}.txt".into()), "Removed"),
        ] {
            receipt.result = Ok(Receipt::Undone(outcome));
            let text = String::from_utf8(super::render_control(&receipt).unwrap()).unwrap();
            assert!(text.contains(verb));
            assert!(text.contains("世界/\\u001b[31m\\n\\u202e.txt"));
            assert!(!text.contains('\u{1b}') && !text.contains('\u{202e}'));
            assert_eq!(text.matches('\n').count(), 2);
        }
        let mut rendered = std::collections::BTreeSet::new();
        for error in [Error::Busy, Error::Rejected, Error::Changed, Error::ResourceLimit,
            Error::Unavailable, Error::Cancelled, Error::Ambiguous,
            Error::NotUndoable(Reason::PreimageTooLarge), Error::NotUndoable(Reason::SnapshotUnavailable)] {
            receipt.result = Err(ControlError::Undo(error));
            let text = String::from_utf8(super::render_control(&receipt).unwrap()).unwrap();
            assert!(!text.contains("Nothing to undo") && !text.contains("authoritative reload"));
            if error == Error::Ambiguous {
                assert!(text.contains("effects may be partial"));
                assert!(text.contains("recovery artifacts retained"));
                assert!(text.contains("no automatic retry"));
            } else {
                assert!(text.contains(&error.to_string()));
            }
            assert!(rendered.insert(text), "every native reason stays distinguishable");
        }
        receipt.result = Ok(Receipt::Undone(Outcome::Restored("\u{1b}".repeat(4096))));
        let bounded = super::render_control(&receipt).unwrap();
        assert!(bounded.len() > 4096 && bounded.len() <= crate::ask::production::interactive::MAX_PRESENTATION_OUTPUT_BYTES);
        receipt.result = Ok(Receipt::Undone(Outcome::Removed("\0".repeat(crate::ask::production::interactive::MAX_PRESENTATION_OUTPUT_BYTES))));
        assert!(super::render_control(&receipt).is_err(), "never report a truncated successful inverse");
        assert!(matches!(&receipt.result, Ok(Receipt::Undone(Outcome::Removed(path))) if path.len() == crate::ask::production::interactive::MAX_PRESENTATION_OUTPUT_BYTES));
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    let _ = runtime.block_on(finish_tail(&mut tail));
}

#[test]
fn undo_receipt_survives_blocked_output_shutdown_and_final_flush_acknowledgement() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let (result, receipt_id) = runtime.block_on(async {
        fixture.transport.push(support::call("write_file", &serde_json::json!({
            "path": "undo-尾.txt", "content": "tracked output barrier"
        })));
        fixture.transport.push(support::answer());
        harness.driver.owner.enqueue("write before undo".into()).unwrap();
        let _ = pump_until(&mut harness, |driver| presentation_idle(driver)
            && driver.owner.runtime().status().queued_jobs == 0).await;
        assert!(fixture.workspace.join("undo-尾.txt").exists());
        let record = harness.driver.owner.runtime().record();
        harness.driver.note(b"\n[blocked output]\n");
        until(&mut harness, |driver| driver.in_flight.is_some()).await;
        std::io::Write::write_all(&mut harness.input_writer, b"/undo\n").unwrap();
        until(&mut harness, |driver| driver.control_outcome.is_some()).await;
        let receipt = harness.driver.control_outcome.as_ref().unwrap();
        assert!(matches!(&receipt.result, Ok(native::NativeInteractiveControlReceipt::Undone(native::FileUndoOutcome::Removed(path))) if path == "undo-尾.txt"));
        let receipt_id = receipt.id;
        assert!(!fixture.workspace.join("undo-尾.txt").exists());
        assert_eq!(harness.driver.owner.runtime().record(), record);
        harness.driver.shutdown();
        until(&mut harness, |driver| driver.owner.is_closed()).await;
        assert_eq!(harness.driver.control_outcome.as_ref().unwrap().id, receipt_id);
        let result = poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)).await;
        assert_eq!(result.outcome, AskCommandOutcome::Completed);
        (result, receipt_id)
    });
    let mut tail = dispose(harness, fixture, result);
    assert_eq!(tail.presentation.controls.front().unwrap().id, receipt_id);
    let mut saw_undo = false;
    let mut saw_flush = false;
    let result = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let result = tail.presentation.poll(cx, &mut tail.signals);
                if let Ok(work) = tail.work.try_recv() {
                    match work {
                        OutputWork::Write(bytes) => {
                            assert!(bytes.len() <= 4096);
                            if String::from_utf8_lossy(&bytes).contains("Removed undo-尾.txt") {
                                saw_undo = true;
                                assert_eq!(
                                    tail.presentation.controls.front().unwrap().id,
                                    receipt_id
                                );
                            }
                        }
                        OutputWork::Flush if saw_undo && !saw_flush => {
                            saw_flush = true;
                            assert_eq!(tail.presentation.controls.front().unwrap().id, receipt_id);
                        }
                        OutputWork::Flush => {}
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
    assert!(saw_undo && saw_flush);
    assert!(tail.presentation.controls.is_empty());
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

#[test]
fn copy_renderer_preserves_fixed_outcomes_without_echoing_reply_or_failure_details() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let result = runtime.block_on(async {
        let before = harness.driver.owner.runtime().record();
        harness.driver.command("/copy extra", 101);
        assert!(!harness.driver.owner.has_pending_copy());
        harness.driver.command("/copy", 102);
        until(&mut harness, |driver| driver.copy_outcome.is_some()).await;
        let mut receipt = harness.driver.copy_outcome.take().unwrap();
        assert!(matches!(
            receipt.result,
            Ok(native::NativeInteractiveCopyReceipt::Empty)
        ));
        assert!(
            String::from_utf8(super::super::clipboard::render(&receipt).unwrap())
                .unwrap()
                .contains("No assistant reply to copy.")
        );
        receipt.result = Ok(native::NativeInteractiveCopyReceipt::Copied);
        assert!(
            String::from_utf8(super::super::clipboard::render(&receipt).unwrap())
                .unwrap()
                .contains("Copied to clipboard.")
        );
        for error in [
            native::NativeClipboardError::InvalidAuthority,
            native::NativeClipboardError::ResourceLimit,
            native::NativeClipboardError::Busy,
            native::NativeClipboardError::Unavailable,
            native::NativeClipboardError::Cancelled,
            native::NativeClipboardError::TimedOut,
            native::NativeClipboardError::WriteFailed,
            native::NativeClipboardError::ExitFailed,
        ] {
            receipt.result = Err(native::NativeInteractiveCopyError::Clipboard(error));
            let bytes = super::super::clipboard::render(&receipt).unwrap();
            assert!(bytes.len() < 128);
            assert!(
                String::from_utf8(bytes)
                    .unwrap()
                    .contains("Failed to copy to clipboard.")
            );
        }
        for error in [
            native::NativeInteractiveCopyError::ResourceLimit,
            native::NativeInteractiveCopyError::Cancelled,
        ] {
            receipt.result = Err(error);
            assert!(
                String::from_utf8(super::super::clipboard::render(&receipt).unwrap())
                    .unwrap()
                    .contains("Failed to copy to clipboard.")
            );
        }
        assert_eq!(harness.driver.owner.runtime().record(), before);
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_tail(&mut tail));
}

#[test]
fn copy_failure_survives_blocked_output_and_final_flush_without_failing_the_session() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let (result, receipt_id) = runtime.block_on(async {
        fixture.transport.push(support::answer());
        harness
            .driver
            .owner
            .enqueue("produce a saved reply".into())
            .unwrap();
        let _ = pump_until(&mut harness, |driver| {
            presentation_idle(driver) && driver.owner.runtime().status().queued_jobs == 0
        })
        .await;
        let before = harness.driver.owner.runtime().record();
        harness.driver.note(b"\n[blocked output]\n");
        until(&mut harness, |driver| driver.in_flight.is_some()).await;
        std::io::Write::write_all(&mut harness.input_writer, b"/copy\n").unwrap();
        until(&mut harness, |driver| driver.copy_outcome.is_some()).await;
        let receipt = harness.driver.copy_outcome.as_ref().unwrap();
        assert!(matches!(
            receipt.result,
            Err(native::NativeInteractiveCopyError::Clipboard(
                native::NativeClipboardError::Unavailable
            ))
        ));
        let receipt_id = receipt.id;
        harness.driver.command("/copy", 201);
        assert!(
            !harness.driver.owner.has_pending_copy(),
            "unacknowledged receipt bounds further copy admission"
        );
        assert_eq!(harness.driver.owner.runtime().record(), before);
        assert_eq!(fixture.transport.requests().len(), 1);
        assert!(
            !harness.driver.native_failed,
            "optional clipboard failure is not a failed conversation"
        );
        harness.driver.shutdown();
        let result = poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)).await;
        assert_eq!(result.outcome, AskCommandOutcome::Completed);
        (result, receipt_id)
    });
    let mut tail = dispose(harness, fixture, result);
    assert_eq!(tail.presentation.copies.front().unwrap().id, receipt_id);
    let mut saw_message = false;
    let mut saw_flush = false;
    let result = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let result = tail.presentation.poll(cx, &mut tail.signals);
                if let Ok(work) = tail.work.try_recv() {
                    match work {
                        OutputWork::Write(bytes) => {
                            assert!(bytes.len() <= 4096);
                            if String::from_utf8_lossy(&bytes)
                                .contains("Failed to copy to clipboard.")
                            {
                                saw_message = true;
                                assert_eq!(
                                    tail.presentation.copies.front().unwrap().id,
                                    receipt_id
                                );
                            }
                        }
                        OutputWork::Flush if saw_message && !saw_flush => {
                            saw_flush = true;
                            assert_eq!(tail.presentation.copies.front().unwrap().id, receipt_id);
                        }
                        OutputWork::Flush => {}
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
    assert!(saw_message && saw_flush);
    assert!(tail.presentation.copies.is_empty());
    assert_eq!(result.outcome, AskCommandOutcome::Completed);
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
            history: false,
            clear_row: false,
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

#[test]
fn raw_empty_ctrl_d_and_physical_eof_have_distinct_owned_outcomes() {
    use std::io::Write as _;
    for physical_eof in [false, true] {
        let runtime = executor();
        let fixture = support::Fixture::new();
        let mut harness = runtime.block_on(harness(&fixture));
        harness.driver = harness.driver.with_raw_input(80, None);
        if physical_eof {
            let (_, replacement) = std::io::pipe().unwrap();
            drop(std::mem::replace(&mut harness.input_writer, replacement));
        } else {
            harness.input_writer.write_all(&[4]).unwrap();
        }
        let result = runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| {
                    let result = harness.driver.poll(cx, &mut harness.signals);
                    if harness.work.try_recv().is_ok() {
                        harness
                            .ack
                            .try_send(OutputAcknowledgement::Succeeded)
                            .unwrap();
                        cx.waker().wake_by_ref();
                    }
                    result
                }),
            )
            .await
            .unwrap()
        });
        assert_eq!(
            result.outcome,
            if physical_eof {
                AskCommandOutcome::OperationalFailure
            } else {
                AskCommandOutcome::Completed
            }
        );
        assert!(harness.driver.owner.is_closed());
        let mut tail = dispose(harness, fixture, result);
        let (settled, output) = runtime.block_on(finish_raw_tail(&mut tail));
        assert_eq!(settled.outcome, result.outcome);
        assert!(output.windows(8).any(|bytes| bytes == b"\x1b[?2004l"));
    }
}

#[test]
fn raw_cursor_delete_and_double_ctrl_c_use_the_composed_driver() {
    use std::io::Write as _;
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    harness.driver = harness.driver.with_raw_input(8, None);
    harness.input_writer.write_all(b"abc\x01\x04").unwrap();
    let result = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                harness.driver.poll_input(cx, 101);
                if harness.driver.input.raw_draft() == Some(("bc", 0)) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        assert!(!harness.driver.shutting_down);
        harness.input_writer.write_all(&[3]).unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                harness.driver.poll_input(cx, 102);
                if harness.driver.input.raw_draft() == Some(("", 0)) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        assert!(!harness.driver.shutting_down);
        harness.input_writer.write_all(&[3]).unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let result = harness.driver.poll(cx, &mut harness.signals);
                if harness.work.try_recv().is_ok() {
                    harness
                        .ack
                        .try_send(OutputAcknowledgement::Succeeded)
                        .unwrap();
                    cx.waker().wake_by_ref();
                }
                result
            }),
        )
        .await
        .unwrap()
    });
    assert_eq!(result.outcome, AskCommandOutcome::Completed);
    let mut tail = dispose(harness, fixture, result);
    let (settled, _) = runtime.block_on(finish_raw_tail(&mut tail));
    assert_eq!(settled.outcome, AskCommandOutcome::Completed);
}

async fn finish_raw_tail(harness: &mut TailHarness) -> (TurnDriveResult, Vec<u8>) {
    let mut output = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let result = harness.presentation.poll(cx, &mut harness.signals);
            if let Ok(work) = harness.work.try_recv() {
                if let OutputWork::Write(bytes) = work {
                    output.extend(bytes);
                }
                harness
                    .ack
                    .try_send(OutputAcknowledgement::Succeeded)
                    .unwrap();
                cx.waker().wake_by_ref();
            }
            result
        }),
    )
    .await
    .unwrap();
    (result, output)
}

#[test]
fn session_picker_refuses_drafts_and_queued_work_without_cancelling_them() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    harness.driver = harness.driver.with_raw_input(80, None);
    harness.driver.picker = Some(picker::Picker::new(
        fixture.host.session_catalog_reader().unwrap(),
        Some(harness.driver.owner.runtime().id()),
        24,
    ));
    let result = runtime.block_on(async {
        use std::io::Write;
        harness.input_writer.write_all(b"unfinished draft").unwrap();
        until(&mut harness, |driver| {
            driver
                .input
                .raw_draft()
                .is_some_and(|(text, _)| text == "unfinished draft")
        })
        .await;
        harness
            .driver
            .open_picker(native::NativeSessionCatalogScope::All);
        assert!(!harness.driver.picker_open());
        assert_eq!(
            harness.driver.input.raw_draft().unwrap().0,
            "unfinished draft"
        );
        harness.driver.input.reset_raw_draft();
        harness
            .driver
            .owner
            .enqueue("queued prompt".into())
            .unwrap();
        harness
            .driver
            .open_picker(native::NativeSessionCatalogScope::All);
        assert!(!harness.driver.picker_open());
        assert_eq!(harness.driver.owner.runtime().status().queued_jobs, 1);
        harness.driver.shutdown();
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    let _ = runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn stale_command_chunk_cannot_select_picker_and_escape_preserves_identity() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    harness.driver = harness.driver.with_raw_input(80, None);
    let original = harness.driver.owner.runtime().id();
    harness.driver.picker = Some(picker::Picker::new(
        fixture.host.session_catalog_reader().unwrap(),
        Some(original.clone()),
        24,
    ));
    let result = runtime.block_on(async {
        harness.driver.command("/resume", 100);
        assert!(harness.driver.picker_open());
        assert!(harness.driver.picker_event(
            &composer::ComposerEvent::Submit(String::new()),
            &InputBinding::Command,
            101
        ));
        assert!(harness.driver.picker_request.is_none());
        let binding = harness.driver.picker_binding().unwrap();
        assert!(harness.driver.picker_event(
            &composer::ComposerEvent::EscapeRequested,
            &binding,
            102
        ));
        assert!(!harness.driver.picker_open());
        assert_eq!(harness.driver.owner.runtime().id(), original);
        assert!(harness.driver.scope_active);
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    let _ = runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn picker_enter_received_before_flush_cannot_select_after_acknowledgement() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    harness.driver = harness.driver.with_raw_input(80, None);
    let original = harness.driver.owner.runtime().id();
    harness.driver.picker = Some(picker::Picker::new(
        fixture.host.session_catalog_reader().unwrap(),
        Some(original.clone()),
        24,
    ));
    let result = runtime.block_on(async {
        use machine_god_core::{
            Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionStore,
        };
        let mut record = SessionRecord::empty(
            SessionId::new("selectable").unwrap(),
            SessionIncarnationId::new("selectable-life").unwrap(),
        );
        record
            .messages
            .push(Message::text(Role::User, "saved request"));
        record.metadata.insert(
            native::NATIVE_SESSION_METADATA_KEY.into(),
            native::NativeSessionMetadata::new(
                &fixture.workspace,
                10,
                native::NativeSessionOrigin::Cli,
            )
            .unwrap()
            .to_value(),
        );
        fixture
            .host
            .session_lifecycle()
            .session_store()
            .save(record, None)
            .await
            .unwrap();
        harness.driver.command("/resume", 100);
        let old = harness.driver.picker_binding().unwrap();
        assert!(matches!(old, InputBinding::AwaitingPicker { .. }));
        let _ = pump_until(&mut harness, |driver| {
            matches!(driver.picker_binding(), Some(InputBinding::Picker { revision, .. }) if revision > 0)
        })
        .await;
        assert!(harness.driver.picker_event(
            &composer::ComposerEvent::Submit(String::new()),
            &old,
            101
        ));
        assert!(harness.driver.picker_request.is_none());
        assert_eq!(harness.driver.owner.runtime().id(), original);
        let current = harness.driver.picker_binding().unwrap();
        assert!(harness.driver.picker_event(
            &composer::ComposerEvent::Submit(String::new()),
            &current,
            102
        ));
        assert!(
            harness.driver.picker_request.is_some(),
            "fresh acknowledged selection is admitted"
        );
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    let _ = runtime.block_on(finish_raw_tail(&mut tail));
}

async fn stale_picker_selection(fixture: &support::Fixture, harness: &mut Harness) {
    use machine_god_core::{
        Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionStore,
    };
    let id = SessionId::new("stale-selection").unwrap();
    let mut record = SessionRecord::empty(
        id.clone(),
        SessionIncarnationId::new("stale-selection-life").unwrap(),
    );
    record
        .messages
        .push(Message::text(Role::User, "saved request"));
    record.metadata.insert(
        native::NATIVE_SESSION_METADATA_KEY.into(),
        native::NativeSessionMetadata::new(
            &fixture.workspace,
            10,
            native::NativeSessionOrigin::Cli,
        )
        .unwrap()
        .to_value(),
    );
    fixture
        .host
        .session_lifecycle()
        .session_store()
        .save(record, None)
        .await
        .unwrap();
    let original = harness.driver.owner.runtime().id();
    harness.driver.picker = Some(picker::Picker::new(
        fixture.host.session_catalog_reader().unwrap(),
        Some(original),
        24,
    ));
    harness.driver.command("/resume", 100);
    pump_until(harness, |driver| {
        matches!(driver.picker_binding(), Some(InputBinding::Picker { revision, .. }) if revision > 0)
    }).await;
    let session = fixture.host.session_lifecycle().resume(id).await.unwrap();
    native::rename_native_session(&session, "changed since display", 101)
        .await
        .unwrap();
    let binding = harness.driver.picker_binding().unwrap();
    assert!(harness.driver.picker_event(
        &composer::ComposerEvent::Submit(String::new()),
        &binding,
        102,
    ));
    assert!(harness.driver.picker_request.is_some());
}

#[test]
fn stale_picker_rejection_is_retryable_and_survives_output_and_shutdown() {
    for (acknowledge_receipt, retry) in [(false, false), (true, false), (true, true)] {
        let runtime = executor();
        let fixture = support::Fixture::new();
        let mut harness = runtime.block_on(harness(&fixture));
        harness.driver = harness.driver.with_raw_input(80, None);
        let original = harness.driver.owner.runtime().id();
        let result = runtime.block_on(async {
            stale_picker_selection(&fixture, &mut harness).await;
            until(&mut harness, |driver| {
                matches!(
                    driver.outcome,
                    Some(NativeInteractiveOutcome::Rejected { .. })
                )
            })
            .await;
            let outcome = harness.driver.outcome.as_ref().unwrap();
            assert!(matches!(outcome, NativeInteractiveOutcome::Rejected {
                error: native::NativeInteractiveError::Resume(error), ..
            } if error.kind() == native::NativeSessionResumeErrorKind::Conflict));
            assert!(
                super::outcome_failed(outcome),
                "non-picker rejections remain failures"
            );
            assert!(!harness.driver.native_failed);
            assert!(harness.driver.picker_open());
            assert!(harness.driver.scope_active);
            assert_eq!(harness.driver.owner.runtime().id(), original);
            assert!(fixture.transport.requests().is_empty());
            if acknowledge_receipt {
                let output = pump_until(&mut harness, |driver| driver.outcome.is_none()).await;
                assert!(String::from_utf8_lossy(&output).contains("rejected"));
                assert!(harness.driver.picker_rejection.is_none());
            }
            if retry {
                // Reopen to get a new observed tuple, then explicitly select it.
                // No failed attempt may silently follow a changed revision.
                harness.driver.close_picker();
                harness.driver.command("/resume", 103);
                pump_until(&mut harness, |driver| {
                    matches!(driver.picker_binding(), Some(InputBinding::Picker { revision, .. }) if revision > 0)
                }).await;
                let binding = harness.driver.picker_binding().unwrap();
                assert!(harness.driver.picker_event(
                    &composer::ComposerEvent::Submit(String::new()), &binding, 104,
                ));
                pump_until(&mut harness, |driver| {
                    driver.owner.runtime().id() != original && driver.picker_request.is_none()
                }).await;
                assert_eq!(harness.driver.owner.runtime().id().as_str(), "stale-selection");
                assert!(!harness.driver.picker_open());
                assert!(!harness.driver.native_failed);
                assert!(fixture.transport.requests().is_empty());
            }
            harness.driver.shutdown();
            tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)),
            )
            .await
            .unwrap()
        });
        assert_eq!(result.outcome, AskCommandOutcome::Completed);
        let mut tail = dispose(harness, fixture, result);
        assert!(!tail.presentation.native_failed);
        if !acknowledge_receipt {
            assert!(
                tail.presentation
                    .outcomes
                    .iter()
                    .any(|outcome| matches!(outcome, NativeInteractiveOutcome::Rejected { .. }))
            );
        }
        let (result, output) = runtime.block_on(finish_raw_tail(&mut tail));
        assert_eq!(result.outcome, AskCommandOutcome::Completed);
        if !acknowledge_receipt {
            assert!(String::from_utf8_lossy(&output).contains("rejected"));
        }
        assert!(tail.presentation.outcomes.is_empty());
    }
}

async fn pump_until(harness: &mut Harness, condition: impl Fn(&Driver) -> bool) -> Vec<u8> {
    let mut output = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            assert!(harness.driver.poll(cx, &mut harness.signals).is_pending());
            if let Ok(work) = harness.work.try_recv() {
                if let OutputWork::Write(bytes) = work {
                    assert!(bytes.len() <= 4096);
                    output.extend(bytes);
                }
                harness
                    .ack
                    .try_send(OutputAcknowledgement::Succeeded)
                    .unwrap();
                cx.waker().wake_by_ref();
            }
            if condition(&harness.driver) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }),
    )
    .await
    .unwrap();
    output
}

fn presentation_idle(driver: &Driver) -> bool {
    driver.outcome.is_none()
        && driver.render.is_none()
        && driver.in_flight.is_none()
        && driver.history.is_none()
        && !driver.owner.runtime().status().active
}

#[test]
fn confirmed_resume_replays_canonical_text_without_repeating_recorded_file_effects() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    let result = runtime.block_on(async {
        fixture.transport.push(support::call(
            "write_file",
            &serde_json::json!({"path":"history.txt","content":"original"}),
        ));
        fixture.transport.push(support::answer());
        let prompt = "historical request \u{1b}[2J";
        harness.driver.owner.enqueue(prompt.into()).unwrap();
        pump_until(&mut harness, |driver| {
            presentation_idle(driver) && driver.owner.runtime().status().queued_jobs == 0
        })
        .await;
        let saved = harness.driver.owner.runtime().record();
        let requests = fixture.transport.requests().len();
        assert_eq!(requests, 2);
        std::fs::write(
            fixture.workspace.join("history.txt"),
            "changed after original turn",
        )
        .unwrap();

        harness
            .driver
            .owner
            .request_transition(native::NativeInteractiveTransition::New, 200)
            .unwrap();
        pump_until(&mut harness, |driver| {
            presentation_idle(driver) && driver.owner.runtime().id() != saved.id
        })
        .await;
        harness
            .driver
            .owner
            .request_transition(
                native::NativeInteractiveTransition::Resume(native::NativeResumeTarget::Exact(
                    saved.id.clone(),
                )),
                300,
            )
            .unwrap();
        let output = pump_until(&mut harness, |driver| {
            presentation_idle(driver) && driver.owner.runtime().id() == saved.id
        })
        .await;
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("historical request"));
        assert!(output.contains("complete"));
        assert!(output.contains("write_file"));
        assert!(!output.contains('\u{1b}'));
        assert_eq!(fixture.transport.requests().len(), requests);
        assert_eq!(
            std::fs::read_to_string(fixture.workspace.join("history.txt")).unwrap(),
            "changed after original turn"
        );
        assert_eq!(
            harness.driver.owner.runtime().record().messages,
            saved.messages
        );
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    let _ = runtime.block_on(finish_tail(&mut tail));
}

#[test]
fn blocked_historical_output_cannot_keep_its_snapshot_or_native_host_alive_on_shutdown() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(harness(&fixture));
    harness.driver.history = Some(history_view::HistoryView::new(
        harness.driver.owner.runtime().record(),
    ));
    harness.driver.notice = None;
    harness.driver.render = Some(Render {
        history: true,
        clear_row: false,
        bytes: vec![b'x'; 4096],
        offset: 0,
        confirm: None,
        receipt: None,
        model_text: false,
    });
    let result = runtime.block_on(async {
        until(&mut harness, |driver| driver.in_flight.is_some()).await;
        let result = finish_signal(&mut harness).await;
        assert!(harness.driver.history.is_none());
        assert!(
            harness
                .driver
                .render
                .as_ref()
                .is_none_or(|render| !render.history)
        );
        result
    });
    let mut tail = dispose(harness, fixture, result);
    let _ = runtime.block_on(finish_tail(&mut tail));
}
