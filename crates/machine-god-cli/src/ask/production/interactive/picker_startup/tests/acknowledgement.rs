use super::super::super::super::OutputWork;
use super::*;

async fn hold_selectable_flush(harness: &mut Harness) {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            assert!(harness.startup.poll(cx, &mut harness.signals).is_pending());
            if let Ok(work) = harness.work.try_recv() {
                if matches!(work, OutputWork::Flush)
                    && matches!(
                        &harness.startup.in_flight,
                        Some(InFlight::Flush {
                            confirm: Some(InputBinding::Picker { .. }),
                            ..
                        })
                    )
                    && matches!(
                        harness.startup.picker.input_binding(),
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

async fn enter_before_ack(harness: &mut Harness) {
    harness.input_writer.write_all(b"\r\r").unwrap();
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            harness.startup.poll_input(cx);
            if harness.startup.picker.has_pending_selection() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }),
    )
    .await
    .unwrap();
    assert!(harness.startup.pending.is_none());
    assert!(harness.startup.owner.is_none());
}

#[test]
fn startup_rendered_enter_waits_for_exact_flush_then_opens_only_selected_session() {
    for queued_ack in [false, true] {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fixture = support::Fixture::new();
        runtime.block_on(async {
            let saved = saved_tool_turn(&fixture).await;
            let requests = fixture.transport.requests().len();
            let mut harness = harness(&fixture).await;
            hold_selectable_flush(&mut harness).await;
            if queued_ack {
                harness
                    .ack
                    .try_send(OutputAcknowledgement::Succeeded)
                    .unwrap();
            }
            enter_before_ack(&mut harness).await;
            assert_eq!(count(&fixture).await, 1);
            if !queued_ack {
                harness
                    .ack
                    .try_send(OutputAcknowledgement::Succeeded)
                    .unwrap();
            }
            tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| {
                    let result = harness.startup.poll(cx, &mut harness.signals);
                    acknowledge(&mut harness, cx);
                    result
                }),
            )
            .await
            .unwrap();
            let Ok(mut driver) = harness.startup.into_result(harness.inbox).unwrap() else {
                panic!("selected session")
            };
            assert_eq!(driver.owner.runtime().id(), saved.id);
            assert_eq!(driver.owner.runtime().record().messages, saved.messages);
            assert_eq!(fixture.transport.requests().len(), requests);
            assert_eq!(count(&fixture).await, 1);
            assert!(driver.history.is_some());
            driver.shutdown();
            tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| {
                    let result = driver.poll(cx, &mut harness.signals);
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
            .unwrap();
        });
        fixture.finish();
    }
}

async fn revoke_startup(harness: &mut Harness, change: usize) {
    // Consume the second Enter under its original chunk identity.
    poll_fn(|cx| {
        harness.startup.poll_input(cx);
        Poll::Ready(())
    })
    .await;
    match change {
        0 => {
            harness.input_writer.write_all(b"\xc3").unwrap();
            tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| {
                    harness.startup.poll_input(cx);
                    if harness.startup.picker.has_pending_selection() {
                        Poll::Pending
                    } else {
                        Poll::Ready(())
                    }
                }),
            )
            .await
            .unwrap();
        }
        1 => harness.startup.picker.resize(12),
        2 => harness.signal.try_send(AskSignal::Interrupt).unwrap(),
        3 => {
            let (_read, replacement) = std::io::pipe().unwrap();
            drop(std::mem::replace(&mut harness.input_writer, replacement));
        }
        4 => harness.ack.try_send(OutputAcknowledgement::Failed).unwrap(),
        _ => unreachable!(),
    }
    if change < 2 {
        harness
            .ack
            .try_send(OutputAcknowledgement::Succeeded)
            .unwrap();
        poll_fn(|cx| {
            harness.startup.poll_output(cx);
            Poll::Ready(())
        })
        .await;
        harness.startup.stop(AskCommandOutcome::Interrupted);
    } else {
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| harness.startup.poll(cx, &mut harness.signals)),
        )
        .await
        .unwrap();
    }
}

#[test]
fn startup_pending_enter_is_revoked_by_partial_input_resize_signal_eof_and_output_failure() {
    for change in 0..5 {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fixture = support::Fixture::new();
        let mut harness = runtime.block_on(async {
            saved_tool_turn(&fixture).await;
            let mut harness = harness(&fixture).await;
            hold_selectable_flush(&mut harness).await;
            enter_before_ack(&mut harness).await;
            revoke_startup(&mut harness, change).await;
            assert!(!harness.startup.picker.has_pending_selection());
            assert!(harness.startup.pending.is_none());
            assert!(harness.startup.owner.is_none());
            assert_eq!(count(&fixture).await, 1);
            harness
        });
        let completion = harness.startup.input.input.completion();
        let Err(mut final_output) = harness.startup.into_result(harness.inbox).unwrap() else {
            panic!("revoked selection cannot create a driver")
        };
        completion.wait_on_worker().unwrap();
        fixture.finish();
        runtime.block_on(async {
            if change == 2 || change == 3 {
                harness
                    .ack
                    .try_send(OutputAcknowledgement::Succeeded)
                    .unwrap();
            }
            tokio::time::timeout(
                Duration::from_secs(10),
                poll_fn(|cx| {
                    let result = final_output.poll(cx, &mut harness.signals);
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
            .unwrap();
        });
    }
}
