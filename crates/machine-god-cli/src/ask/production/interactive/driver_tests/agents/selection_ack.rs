//! Real byte receipt with a held frame flush, never a simulated display ACK.
use super::*;

fn release(harness: &Harness) {
    harness
        .ack
        .try_send(OutputAcknowledgement::Succeeded)
        .unwrap();
}

async fn pending(harness: &mut Harness) {
    harness.input_writer.write_all(b"\r").unwrap();
    input_until(harness, Driver::has_pending_agents_selection).await;
    assert!(matches!(
        harness.driver.owner.managed_navigation().unwrap().route,
        Route::Catalog(_)
    ));
    assert!(matches!(
        harness.driver.agents_binding(),
        Some(InputBinding::Agents { frame: None, .. })
    ));
}

#[test]
fn catalog_bare_enter_selects_once_after_delayed_or_already_ready_flush_ack() {
    for queued_ack in [false, true] {
        let runtime = executor();
        let (fixture, mut harness) = runtime.block_on(prepared());
        let result = runtime.block_on(async {
            harness.input_writer.write_all(b"\x18").unwrap();
            hold_frame(&mut harness).await;
            let parent = harness.driver.owner.runtime().id();
            if queued_ack {
                release(&harness);
            }
            // Poll input alone even when the real ACK is ready: this forces
            // the driver's input-before-output schedule deterministically.
            pending(&mut harness).await;
            if !queued_ack {
                release(&harness);
            }
            pump_until(&mut harness, |driver| {
                displayed(driver)
                    && driver.owner.managed_navigation().unwrap().route == Route::Conversation
            })
            .await;
            assert!(!harness.driver.has_pending_agents_selection());
            assert_eq!(harness.driver.owner.runtime().id(), parent);
            assert!(fixture.transport.requests().is_empty());
            assert_eq!(harness.driver.owner.managed_agents().len(), 1);
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn catalog_pending_enter_is_revoked_at_receipt_of_edits_navigation_or_partial_input() {
    for bytes in [
        b"x".as_slice(),
        b"\x1b[B",
        b"\t",
        b"\xc3",
        b"\x1b[",
        b"\r/close\r",
    ] {
        let runtime = executor();
        let (fixture, mut harness) = runtime.block_on(prepared());
        let result = runtime.block_on(async {
            harness.input_writer.write_all(b"\x18").unwrap();
            hold_frame(&mut harness).await;
            pending(&mut harness).await;
            harness.input_writer.write_all(bytes).unwrap();
            input_until(&mut harness, |driver| {
                !driver.has_pending_agents_selection()
            })
            .await;
            release(&harness);
            pump_until(&mut harness, presentation_idle).await;
            assert!(matches!(
                harness.driver.owner.managed_navigation().unwrap().route,
                Route::Catalog(_)
            ));
            assert!(fixture.transport.requests().is_empty());
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn catalog_mixed_chunks_and_commands_never_gain_deferred_selection_authority() {
    for bytes in [
        b"\r\r".as_slice(),
        b"\r/close\r",
        b"/create\r",
        b"/close\r",
        b"message\r",
        b" \r",
    ] {
        let runtime = executor();
        let (fixture, mut harness) = runtime.block_on(prepared());
        let result = runtime.block_on(async {
            harness.input_writer.write_all(b"\x18").unwrap();
            hold_frame(&mut harness).await;
            harness.input_writer.write_all(bytes).unwrap();
            input_until(&mut harness, |driver| driver.notice.is_some()).await;
            assert!(!harness.driver.has_pending_agents_selection());
            release(&harness);
            pump_until(&mut harness, presentation_idle).await;
            assert!(matches!(
                harness.driver.owner.managed_navigation().unwrap().route,
                Route::Catalog(_)
            ));
            assert_eq!(harness.driver.owner.managed_agents().len(), 1);
            assert!(fixture.transport.requests().is_empty());
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn catalog_pending_enter_cannot_cross_frame_replacement_or_editor_reopening() {
    for reopen in [false, true] {
        let runtime = executor();
        let (fixture, mut harness) = runtime.block_on(prepared());
        let result = runtime.block_on(async {
            harness.input_writer.write_all(b"\x18").unwrap();
            hold_frame(&mut harness).await;
            pending(&mut harness).await;
            if reopen {
                let binding = harness.driver.agents_binding().unwrap();
                harness
                    .driver
                    .agents_event(&ComposerEvent::AgentsRequested, &binding);
                harness
                    .driver
                    .agents_event(&ComposerEvent::AgentsRequested, &InputBinding::Command);
            } else {
                // Resize uses this same invalidation, preserving the editor.
                harness.driver.invalidate_agents();
            }
            assert!(!harness.driver.has_pending_agents_selection());
            release(&harness);
            pump_until(&mut harness, displayed).await;
            assert!(matches!(
                harness.driver.owner.managed_navigation().unwrap().route,
                Route::Catalog(_)
            ));
            assert!(fixture.transport.requests().is_empty());
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}

#[test]
fn pending_catalog_selection_is_revoked_by_native_busy_replacement() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        harness.input_writer.write_all(b"\x18").unwrap();
        hold_frame(&mut harness).await;
        pending(&mut harness).await;
        // Independently replace the native catalog while its CLI receipt is
        // still held. A stale flush cannot authorize the refreshed target.
        let frame = harness.driver.owner.managed_navigation().unwrap().frame;
        harness
            .driver
            .owner
            .acknowledge_managed_frame(&frame)
            .unwrap();
        harness
            .driver
            .owner
            .act_on_managed_frame(&frame, native::NativeManagedNavigationAction::Refresh)
            .unwrap();
        assert!(harness.driver.owner.managed_navigation().unwrap().busy);
        harness.driver.sync_agents();
        assert!(!harness.driver.has_pending_agents_selection());
        assert!(matches!(
            harness.driver.agents_binding(),
            Some(InputBinding::Agents {
                pending_frame: None,
                ..
            })
        ));
        release(&harness);
        pump_until(&mut harness, displayed).await;
        assert!(matches!(
            harness.driver.owner.managed_navigation().unwrap().route,
            Route::Catalog(_)
        ));
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn bare_enter_on_unacknowledged_close_confirmation_is_never_deferred() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        enter_child(&mut harness).await;
        harness.input_writer.write_all(b"/close\r").unwrap();
        hold_frame(&mut harness).await;
        assert_eq!(
            harness.driver.owner.managed_navigation().unwrap().route,
            Route::ConfirmClose
        );
        harness.input_writer.write_all(b"\r").unwrap();
        input_until(&mut harness, |driver| driver.notice.is_some()).await;
        assert!(!harness.driver.has_pending_agents_selection());
        release(&harness);
        pump_until(&mut harness, displayed).await;
        assert_eq!(
            harness.driver.owner.managed_navigation().unwrap().route,
            Route::ConfirmClose
        );
        assert_eq!(harness.driver.owner.managed_agents().len(), 1);
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn catalog_render_revision_rejects_old_ack_and_old_byte_binding() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        harness.input_writer.write_all(b"\x18").unwrap();
        hold_frame(&mut harness).await;
        let old = harness.driver.agents_binding().unwrap();
        let native_frame = harness.driver.owner.managed_navigation().unwrap().frame;
        pending(&mut harness).await;
        // Even an empty editor redraw can share the native target identity.
        // It still needs its own presentation revision and real flush ACK.
        harness.driver.agents_event(&ComposerEvent::Changed, &old);
        assert!(harness.driver.prepare_agents_render());
        assert_eq!(
            harness.driver.owner.managed_navigation().unwrap().frame,
            native_frame
        );
        let replacement = harness.driver.agents_binding().unwrap();
        assert!(old != replacement);
        release(&harness);
        poll_fn(|cx| {
            harness.driver.poll_output(cx);
            Poll::Ready(())
        })
        .await;
        harness
            .driver
            .agents_event(&ComposerEvent::Submit(String::new()), &old);
        assert!(!harness.driver.has_pending_agents_selection());
        assert!(matches!(
            harness.driver.agents_binding(),
            Some(InputBinding::Agents { frame: None, .. })
        ));
        // Before the replacement ACK, the replacement's exact event may wait,
        // but the old native-frame ACK above cannot authorize it.
        harness
            .driver
            .agents_event(&ComposerEvent::Submit(String::new()), &replacement);
        assert!(harness.driver.has_pending_agents_selection());
        assert!(matches!(
            harness.driver.owner.managed_navigation().unwrap().route,
            Route::Catalog(_)
        ));
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && driver.owner.managed_navigation().unwrap().route == Route::Conversation
        })
        .await;
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn exact_catalog_byte_binding_survives_ack_before_event_decode_only_for_that_frame() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        harness.input_writer.write_all(b"\x18").unwrap();
        hold_frame(&mut harness).await;
        let received = harness.driver.agents_binding().unwrap();
        release(&harness);
        poll_fn(|cx| {
            harness.driver.poll_output(cx);
            Poll::Ready(())
        })
        .await;
        assert!(matches!(
            harness.driver.agents_binding(),
            Some(InputBinding::Agents { frame: Some(_), .. })
        ));
        // Simulate the event decoded after the receipt's exact ACK. It retains
        // its original pending binding, never a newly acquired frame binding.
        harness
            .driver
            .agents_event(&ComposerEvent::Submit(String::new()), &received);
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && driver.owner.managed_navigation().unwrap().route == Route::Conversation
        })
        .await;
        harness
            .driver
            .agents_event(&ComposerEvent::Submit(String::new()), &received);
        assert!(!harness.driver.has_pending_agents_selection());
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn catalog_pending_selection_is_revoked_by_modal_input_owner_and_shutdown() {
    for shutdown in [false, true] {
        let runtime = executor();
        let (fixture, mut harness) = runtime.block_on(prepared());
        let result = runtime.block_on(async {
            harness.input_writer.write_all(b"\x18").unwrap();
            hold_frame(&mut harness).await;
            pending(&mut harness).await;
            if shutdown {
                harness.driver.shutdown();
            } else {
                poll_fn(|cx| {
                    harness
                        .driver
                        .poll_raw_input(cx, InputBinding::AwaitingPrompt, 101);
                    Poll::Ready(())
                })
                .await;
            }
            assert!(!harness.driver.has_pending_agents_selection());
            release(&harness);
            if !shutdown {
                pump_until(&mut harness, displayed).await;
                assert!(matches!(
                    harness.driver.owner.managed_navigation().unwrap().route,
                    Route::Catalog(_)
                ));
            }
            assert!(fixture.transport.requests().is_empty());
            finish_signal(&mut harness).await
        });
        let mut tail = dispose(harness, fixture, result);
        runtime.block_on(finish_raw_tail(&mut tail));
    }
}
