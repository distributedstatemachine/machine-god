//! Selection intent uses real input and held output acknowledgements, not sleeps.
use super::*;
use machine_god_core::{
    Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionStore,
};
use std::io::Write as _;

async fn prepared(fixture: &support::Fixture) -> Harness {
    let mut record = SessionRecord::empty(
        SessionId::new("pending-picker-target").unwrap(),
        SessionIncarnationId::new("pending-picker-life").unwrap(),
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
    let mut harness = harness(fixture).await;
    harness.driver = harness.driver.with_raw_input(80, None);
    harness.driver.picker = Some(picker::Picker::new(
        fixture.host.session_catalog_reader().unwrap(),
        Some(harness.driver.owner.runtime().id()),
        24,
    ));
    pump_until(&mut harness, presentation_idle).await;
    harness.driver.command("/resume", 100);
    harness
}

fn pending(driver: &Driver) -> bool {
    driver
        .picker
        .as_ref()
        .is_some_and(picker::Picker::has_pending_selection)
}

async fn hold_frame_flush(harness: &mut Harness) {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            assert!(harness.driver.poll(cx, &mut harness.signals).is_pending());
            if let Ok(work) = harness.work.try_recv() {
                if matches!(work, OutputWork::Flush)
                    && matches!(
                        &harness.driver.in_flight,
                        Some(InFlight::Flush {
                            confirm: Some(InputBinding::Picker { .. }),
                            ..
                        })
                    )
                    && matches!(
                        harness.driver.picker_binding(),
                        Some(InputBinding::AwaitingPicker {
                            pending_revision: Some(_),
                            ..
                        })
                    )
                {
                    return Poll::Ready(());
                }
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
}

async fn select_before_ack(harness: &mut Harness, keys: &[u8]) {
    harness.input_writer.write_all(keys).unwrap();
    input_until(harness, pending).await;
    assert!(harness.driver.picker_request.is_none());
    assert!(harness.driver.scope_active);
    assert_eq!(harness.driver.owner.runtime().status().queued_jobs, 0);
}

fn release_flush(harness: &Harness) {
    harness
        .ack
        .try_send(OutputAcknowledgement::Succeeded)
        .unwrap();
}

#[test]
fn rendered_picker_enter_selects_exactly_once_after_real_flush_ack() {
    for (queued_ack, keys) in [(false, b"\r".as_slice()), (false, b"\r\r\r"), (true, b"\r")] {
        let runtime = executor();
        let fixture = support::Fixture::new();
        let mut harness = runtime.block_on(prepared(&fixture));
        let result = runtime.block_on(async {
            hold_frame_flush(&mut harness).await;
            let original = harness.driver.owner.runtime().id();
            if queued_ack {
                release_flush(&harness);
            }
            // This polls input only, including when the ACK is already queued.
            select_before_ack(&mut harness, keys).await;
            assert_eq!(harness.driver.owner.runtime().id(), original);
            if !queued_ack {
                release_flush(&harness);
            }
            pump_until(&mut harness, |driver| {
                driver.owner.runtime().id().as_str() == "pending-picker-target"
                    && driver.picker_request.is_none()
                    && presentation_idle(driver)
            })
            .await;
            assert!(!harness.driver.picker_open());
            assert!(!pending(&harness.driver));
            assert_eq!(harness.driver.owner.runtime().record().messages.len(), 1);
            assert!(fixture.transport.requests().is_empty());
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn pending_picker_enter_is_revoked_by_edit_navigation_and_partial_chunks() {
    for keys in [
        b"x".as_slice(),
        b"\x1b[B",
        b"\t",
        b"\xc3",
        b"\x1b[",
        b"\x03",
    ] {
        let runtime = executor();
        let fixture = support::Fixture::new();
        let mut harness = runtime.block_on(prepared(&fixture));
        let original = harness.driver.owner.runtime().id();
        let result = runtime.block_on(async {
            hold_frame_flush(&mut harness).await;
            select_before_ack(&mut harness, b"\r").await;
            harness.input_writer.write_all(keys).unwrap();
            input_until(&mut harness, |driver| !pending(driver)).await;
            release_flush(&harness);
            pump_until(&mut harness, presentation_idle).await;
            assert!(harness.driver.picker_request.is_none());
            assert_eq!(harness.driver.owner.runtime().id(), original);
            assert!(fixture.transport.requests().is_empty());
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn mixed_picker_chunk_cannot_rearm_enter_before_flush_or_change_sessions() {
    for keys in [b"\rx".as_slice(), b"\r\rx", b"\r\x1b", b"\r\x03"] {
        let runtime = executor();
        let fixture = support::Fixture::new();
        let mut harness = runtime.block_on(prepared(&fixture));
        let original = harness.driver.owner.runtime().id();
        let result = runtime.block_on(async {
            hold_frame_flush(&mut harness).await;
            release_flush(&harness);
            harness.input_writer.write_all(keys).unwrap();
            input_until(&mut harness, |driver| {
                driver.input.received_chunk_remainder() == Some(&keys[1..])
            })
            .await;
            assert!(!pending(&harness.driver));
            // Match the actual driver's input-then-output phase ordering,
            // consuming the queued frame ACK before decoding the retained tail.
            poll_fn(|cx| {
                harness.driver.poll_output(cx);
                Poll::Ready(())
            })
            .await;
            assert!(harness.driver.picker_request.is_none());
            pump_until(&mut harness, |driver| {
                let applied = if keys.ends_with(b"x") {
                    driver.input.raw_draft() == Some(("x", 1))
                } else if keys.ends_with(b"\x03") {
                    driver.frontend.as_ref().unwrap().cancel_armed.is_some()
                } else {
                    !driver.picker_open()
                };
                applied && presentation_idle(driver)
            })
            .await;
            assert!(!pending(&harness.driver));
            assert!(harness.driver.picker_request.is_none());
            assert_eq!(harness.driver.owner.runtime().id(), original);
            assert!(fixture.transport.requests().is_empty());
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn pending_picker_enter_cannot_transfer_to_new_view_or_input_owner() {
    for change in 0..6 {
        let runtime = executor();
        let fixture = support::Fixture::new();
        let mut harness = runtime.block_on(prepared(&fixture));
        let original = harness.driver.owner.runtime().id();
        let result = runtime.block_on(async {
            hold_frame_flush(&mut harness).await;
            select_before_ack(&mut harness, b"\r").await;
            match change {
                0 => harness.driver.picker.as_mut().unwrap().resize(12),
                1 => harness.driver.picker.as_mut().unwrap().toggle_scope(),
                2 => harness.driver.close_picker(),
                3 => {
                    harness.driver.close_picker();
                    harness.driver.command("/resume", 101);
                }
                4 => harness
                    .driver
                    .picker
                    .as_mut()
                    .unwrap()
                    .set_current(original.clone()),
                5 => {
                    poll_fn(|cx| {
                        harness
                            .driver
                            .poll_raw_input(cx, InputBinding::AwaitingPrompt, 101);
                        Poll::Ready(())
                    })
                    .await;
                }
                _ => unreachable!(),
            }
            assert!(!pending(&harness.driver));
            release_flush(&harness);
            pump_until(&mut harness, presentation_idle).await;
            assert!(harness.driver.picker_request.is_none());
            assert_eq!(harness.driver.owner.runtime().id(), original);
            assert!(fixture.transport.requests().is_empty());
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn coalesced_navigation_enter_cannot_borrow_a_new_picker_revision() {
    let runtime = executor();
    let fixture = support::Fixture::new();
    let mut harness = runtime.block_on(prepared(&fixture));
    let original = harness.driver.owner.runtime().id();
    let result = runtime.block_on(async {
        hold_frame_flush(&mut harness).await;
        let before = harness.driver.picker.as_ref().unwrap().identity();
        harness.input_writer.write_all(b"\x1b[B\r").unwrap();
        input_until(&mut harness, |driver| {
            driver.picker.as_ref().unwrap().identity() != before
        })
        .await;
        release_flush(&harness);
        pump_until(&mut harness, presentation_idle).await;
        assert!(!pending(&harness.driver));
        assert!(harness.driver.picker_request.is_none());
        assert_eq!(harness.driver.owner.runtime().id(), original);
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn pending_picker_enter_is_revoked_by_actual_prompt_modal_and_native_reset() {
    for reset in [false, true] {
        let runtime = executor();
        let fixture = support::Fixture::new();
        let mut harness = runtime.block_on(prepared(&fixture));
        let original = harness.driver.owner.runtime().id();
        let result = runtime.block_on(async {
            hold_frame_flush(&mut harness).await;
            select_before_ack(&mut harness, b"\r").await;
            if reset {
                harness.driver.command("/new", 101);
            } else {
                let bridge = harness.bridge.clone();
                let mut prompt = bridge.prompt(permission(&harness, "picker-modal"));
                let mut cx = Context::from_waker(Waker::noop());
                assert!(prompt.as_mut().poll(&mut cx).is_pending());
                harness.driver.poll_modal(&mut cx);
                assert!(harness.driver.modal.is_some());
                assert!(!pending(&harness.driver));
                drop(prompt);
                harness.driver.poll_modal(&mut cx);
                assert!(harness.driver.modal.is_none());
            }
            assert!(!pending(&harness.driver));
            release_flush(&harness);
            pump_until(&mut harness, |driver| {
                presentation_idle(driver) && (!reset || driver.owner.runtime().id() != original)
            })
            .await;
            assert_ne!(
                harness.driver.owner.runtime().id().as_str(),
                "pending-picker-target"
            );
            assert!(harness.driver.picker_request.is_none());
            assert!(fixture.transport.requests().is_empty());
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn pending_picker_enter_cannot_delay_shutdown_or_survive_failed_output() {
    for stop in 0..3 {
        let runtime = executor();
        let fixture = support::Fixture::new();
        let mut harness = runtime.block_on(prepared(&fixture));
        let original = harness.driver.owner.runtime().id();
        let result = runtime.block_on(async {
            hold_frame_flush(&mut harness).await;
            select_before_ack(&mut harness, b"\r").await;
            match stop {
                0 => harness.signal.try_send(AskSignal::Interrupt).unwrap(),
                1 => {
                    let (_read, replacement) = std::io::pipe().unwrap();
                    drop(std::mem::replace(&mut harness.input_writer, replacement));
                }
                2 => harness.ack.try_send(OutputAcknowledgement::Failed).unwrap(),
                _ => unreachable!(),
            }
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| harness.driver.poll(cx, &mut harness.signals)),
            )
            .await
            .unwrap();
            assert!(harness.driver.owner.is_closed());
            assert!(!pending(&harness.driver));
            assert!(harness.driver.picker_request.is_none());
            assert_eq!(harness.driver.owner.runtime().id(), original);
            assert!(fixture.transport.requests().is_empty());
            result
        });
        assert_eq!(
            result.outcome,
            match stop {
                0 => AskCommandOutcome::Interrupted,
                1 => AskCommandOutcome::OperationalFailure,
                2 => AskCommandOutcome::OutputFailure,
                _ => unreachable!(),
            }
        );
        if stop != 2 {
            release_flush(&harness);
        }
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}
