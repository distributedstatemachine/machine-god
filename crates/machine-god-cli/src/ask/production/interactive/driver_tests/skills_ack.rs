//! Real input chunks and output acknowledgements, with no timing assumptions.
use super::*;
use std::io::Write as _;

async fn prepared(fixture: &support::Fixture, columns: u16) -> Harness {
    let mut harness = harness(fixture).await;
    harness.driver.command("/skills create review", 100);
    until(&mut harness, |driver| driver.control_outcome.is_some()).await;
    let snapshot = harness
        .driver
        .owner
        .skills_catalog()
        .unwrap()
        .discover(&CancellationToken::new())
        .unwrap();
    harness.driver = harness
        .driver
        .with_raw_input(columns, None)
        .with_skills_snapshot(Some(Arc::new(snapshot)));
    pump_until(&mut harness, |driver| {
        presentation_idle(driver) && driver.control_outcome.is_none()
    })
    .await;
    harness.input_writer.write_all(b"Help $re").unwrap();
    input_until(&mut harness, Driver::skills_open).await;
    harness
}

/// Accept every real write, but retain the corresponding selectable-frame
/// flush acknowledgement. A visible footer alone must not grant authority.
async fn hold_frame_flush(harness: &mut Harness) -> InputBinding {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            assert!(harness.driver.poll(cx, &mut harness.signals).is_pending());
            if let Ok(work) = harness.work.try_recv() {
                if matches!(work, OutputWork::Flush)
                    && let Some(InFlight::Flush {
                        confirm: Some(binding @ InputBinding::Skills { .. }),
                        ..
                    }) = &harness.driver.in_flight
                {
                    return Poll::Ready(binding.clone());
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
    .unwrap()
}

async fn select_before_ack(harness: &mut Harness, keys: &[u8]) {
    harness.input_writer.write_all(keys).unwrap();
    input_until(harness, Driver::has_pending_skills_selection).await;
    assert_eq!(harness.driver.input.raw_draft(), Some(("Help $re", 8)));
    assert!(matches!(
        harness.driver.skills_binding(),
        Some(InputBinding::Skills { frame: None, .. })
    ));
    assert_eq!(harness.driver.owner.runtime().status().queued_jobs, 0);
}

fn release_flush(harness: &Harness) {
    harness
        .ack
        .try_send(OutputAcknowledgement::Succeeded)
        .unwrap();
}

async fn assert_not_submitted(harness: &mut Harness, fixture: &support::Fixture) {
    pump_until(harness, presentation_idle).await;
    assert_eq!(harness.driver.owner.runtime().status().queued_jobs, 0);
    assert!(fixture.transport.requests().is_empty());
}

#[test]
fn exact_pending_skill_frame_selects_once_after_flush_without_coalesced_submission() {
    for keys in [b"\t".as_slice(), b"\r", b"\t\r", b"\t\t\r"] {
        let runtime = executor();
        let fixture = support::Fixture::new_with_skills();
        let mut harness = runtime.block_on(prepared(&fixture, 100));
        let result = runtime.block_on(async {
            let binding = hold_frame_flush(&mut harness).await;
            assert!(matches!(
                binding,
                InputBinding::Skills { frame: Some(_), .. }
            ));
            select_before_ack(&mut harness, keys).await;
            assert!(fixture.transport.requests().is_empty());
            release_flush(&harness);
            assert_not_submitted(&mut harness, &fixture).await;
            assert_eq!(
                harness.driver.input.raw_draft(),
                Some(("Help $review ", 13))
            );
            assert!(!harness.driver.skills_open());
            assert!(!harness.driver.has_pending_skills_selection());
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn queued_skill_flush_ack_does_not_lose_a_new_exact_frame_tab() {
    let runtime = executor();
    let fixture = support::Fixture::new_with_skills();
    let mut harness = runtime.block_on(prepared(&fixture, 100));
    let result = runtime.block_on(async {
        hold_frame_flush(&mut harness).await;
        release_flush(&harness);
        // Poll only input: the real successful flush ACK is already queued,
        // but the driver has not consumed it. This is the second race schedule.
        select_before_ack(&mut harness, b"\t").await;
        assert_not_submitted(&mut harness, &fixture).await;
        assert_eq!(
            harness.driver.input.raw_draft(),
            Some(("Help $review ", 13))
        );
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn pending_skill_intent_is_revoked_by_new_edit_navigation_or_partial_input() {
    for changed in [b"v".as_slice(), b"\x1b[B", b"\x1b", b"\xc3", b"\x1b["] {
        let runtime = executor();
        let fixture = support::Fixture::new_with_skills();
        let mut harness = runtime.block_on(prepared(&fixture, 100));
        let result = runtime.block_on(async {
            hold_frame_flush(&mut harness).await;
            select_before_ack(&mut harness, b"\t").await;
            harness.input_writer.write_all(changed).unwrap();
            input_until(&mut harness, |driver| {
                !driver.has_pending_skills_selection()
            })
            .await;
            release_flush(&harness);
            assert_not_submitted(&mut harness, &fixture).await;
            assert!(
                !harness
                    .driver
                    .input
                    .raw_draft()
                    .unwrap()
                    .0
                    .contains("$review ")
            );
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn pending_skill_intent_cannot_transfer_across_presentation_or_editor_changes() {
    for change in 0..5 {
        let runtime = executor();
        let fixture = support::Fixture::new_with_skills();
        let mut harness = runtime.block_on(prepared(&fixture, 100));
        let result = runtime.block_on(async {
            hold_frame_flush(&mut harness).await;
            select_before_ack(&mut harness, b"\t").await;
            match change {
                0 | 1 => {
                    harness.driver.frontend.as_mut().unwrap().columns =
                        if change == 0 { 72 } else { 8 };
                    harness.driver.invalidate_skills_frame();
                }
                2 => {
                    harness.driver.input.reset_raw_draft();
                    harness.driver.reset_skills();
                }
                3 => harness
                    .driver
                    .sync_skills_input_owner(&InputBinding::AwaitingPrompt),
                4 => harness.driver.close_skills(),
                _ => unreachable!(),
            }
            assert!(!harness.driver.has_pending_skills_selection());
            release_flush(&harness);
            assert_not_submitted(&mut harness, &fixture).await;
            assert!(
                !harness
                    .driver
                    .input
                    .raw_draft()
                    .unwrap()
                    .0
                    .contains("$review ")
            );
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn coalesced_arrow_tab_cannot_select_a_later_skill_frame() {
    let runtime = executor();
    let fixture = support::Fixture::new_with_skills();
    let mut harness = runtime.block_on(prepared(&fixture, 100));
    let result = runtime.block_on(async {
        hold_frame_flush(&mut harness).await;
        harness.input_writer.write_all(b"\x1b[B\t").unwrap();
        // Consume navigation and the original chunk's Tab without submitting
        // output ACKs. A later frame must never supply that Tab's identity.
        input_until(&mut harness, |driver| driver.notice.is_some()).await;
        assert!(!harness.driver.has_pending_skills_selection());
        release_flush(&harness);
        assert_not_submitted(&mut harness, &fixture).await;
        assert_eq!(harness.driver.input.raw_draft(), Some(("Help $re", 8)));
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn hidden_skill_frame_never_retains_selection_authority() {
    let runtime = executor();
    let fixture = support::Fixture::new_with_skills();
    let mut harness = runtime.block_on(prepared(&fixture, 8));
    let result = runtime.block_on(async {
        let binding = hold_frame_flush(&mut harness).await;
        assert!(matches!(binding, InputBinding::Skills { frame: None, .. }));
        harness.input_writer.write_all(b"\t").unwrap();
        input_until(&mut harness, |driver| driver.notice.is_some()).await;
        assert!(!harness.driver.has_pending_skills_selection());
        release_flush(&harness);
        assert_not_submitted(&mut harness, &fixture).await;
        assert_eq!(harness.driver.input.raw_draft(), Some(("Help $re", 8)));
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn pending_skill_selection_cannot_hold_shutdown_or_survive_failed_output() {
    for stop in 0..3 {
        let runtime = executor();
        let fixture = support::Fixture::new_with_skills();
        let mut harness = runtime.block_on(prepared(&fixture, 100));
        let result = runtime.block_on(async {
            hold_frame_flush(&mut harness).await;
            select_before_ack(&mut harness, b"\t").await;
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
            assert!(!harness.driver.has_pending_skills_selection());
            assert_eq!(harness.driver.owner.runtime().status().queued_jobs, 0);
            assert!(fixture.transport.requests().is_empty());
            result
        });
        assert_eq!(
            result.outcome,
            match stop {
                0 => AskCommandOutcome::Interrupted,
                1 => AskCommandOutcome::Completed,
                2 => AskCommandOutcome::OutputFailure,
                _ => unreachable!(),
            }
        );
        // Native cleanup completed even though the removed Flush work never
        // received its ACK. A late presentation ACK cannot resurrect selection.
        if stop != 2 {
            release_flush(&harness);
        }
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}
