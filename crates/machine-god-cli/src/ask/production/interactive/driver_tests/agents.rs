use super::*;
use crate::ask::production::managed_startup;
use machine_god_core::ManagedSubagentCommand;
use native::{NativeManagedInteractiveStartup, NativeManagedNavigationRoute as Route};
use std::io::Write as _;

async fn prepared() -> (support::Fixture, Harness) {
    let inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let bridge = inbox.router();
    let mut fixture = support::Fixture::with_workspace_options_and_prompter(
        |options| options.with_managed_agents(managed_startup::options(&inbox)),
        bridge.clone(),
    );
    let state = std::fs::File::open(fixture.state_root()).unwrap();
    let host = Arc::get_mut(&mut fixture.host).unwrap();
    let preferences = host.loaded_config().config().model_preferences();
    let agents = host
        .open_workspace_managed_agents(
            state.into(),
            preferences.clone(),
            native::NativeSessionOrigin::Cli,
        )
        .await
        .unwrap();
    let options =
        NativeInteractiveSessionOptions::new(fixture.workspace.clone(), preferences).unwrap();
    let mut startup =
        NativeManagedInteractiveStartup::new(fixture.host.clone(), options, agents).unwrap();
    startup
        .request_open(NativeInteractiveInitialSession::Fresh, 100)
        .unwrap();
    let owner = poll_fn(|cx| startup.poll_open(cx, 100))
        .await
        .unwrap()
        .unwrap();
    drop(startup);
    let (read, write) = std::io::pipe().unwrap();
    let input = NativeInteractiveInput::new(
        NativeInteractiveInputSource::AdoptNonblockingStatus(
            std::os::fd::OwnedFd::from(read).into(),
        ),
        CancellationToken::new(),
    );
    let (send, work) = tokio::sync::mpsc::channel(1);
    let (ack, acknowledgements) = tokio::sync::mpsc::channel(1);
    let (signal, received) = tokio::sync::mpsc::channel(1);
    let mut harness = Harness {
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
        .unwrap()
        .with_raw_input(80, None),
        input_writer: write,
        work,
        ack,
        signal,
        signals: AskSignals::new(received),
        bridge,
    };
    let mut created = harness
        .driver
        .owner
        .request_managed_command(
            ManagedSubagentCommand::decode(
                serde_json::json!({"command":{"create":{"name":"worker","mode":"persistent"}}}),
            )
            .unwrap(),
            CancellationToken::new(),
        )
        .unwrap();
    poll_fn(|cx| {
        let progress = harness.driver.owner.poll_progress(cx, 100);
        if let Poll::Ready(result) = created.as_mut().poll(cx) {
            assert!(result.unwrap().ok);
            return Poll::Ready(());
        }
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await;
    pump_until(&mut harness, presentation_idle).await;
    (fixture, harness)
}

fn displayed(driver: &Driver) -> bool {
    driver
        .owner
        .managed_navigation()
        .is_some_and(|view| !view.busy)
        && matches!(
            driver.agents_binding(),
            Some(InputBinding::Agents { frame: Some(_), .. })
        )
        && presentation_idle(driver)
}

#[test]
fn process_navigation_preserves_parent_draft_and_rejects_agent_lifecycle_intent() {
    use native::NativeManagedProcessScope as Scope;
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        let parent = principal(&harness.driver.owner);
        harness.input_writer.write_all(b"parent draft").unwrap();
        input_until(&mut harness, |driver| {
            driver.input.raw_draft() == Some(("parent draft", 12))
        })
        .await;
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, displayed).await;
        harness.input_writer.write_all(b"/processes\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && driver.owner.managed_navigation().unwrap().route
                    == Route::Processes(Scope::Parent)
        })
        .await;
        let view = harness.driver.owner.managed_navigation().unwrap();
        assert_eq!(view.process_owner, Some(parent.clone()));
        assert!(view.processes.is_some());
        harness
            .input_writer
            .write_all(b"/agent-processes\r")
            .unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && driver.owner.managed_navigation().unwrap().route
                    == Route::Processes(Scope::SelectedAgent)
        })
        .await;
        assert_ne!(
            harness
                .driver
                .owner
                .managed_navigation()
                .unwrap()
                .process_owner,
            Some(parent.clone())
        );
        harness.input_writer.write_all(b"/close\r").unwrap();
        pump_until(&mut harness, displayed).await;
        assert_eq!(
            harness.driver.owner.managed_navigation().unwrap().route,
            Route::Processes(Scope::SelectedAgent)
        );
        assert_eq!(harness.driver.owner.managed_agents().len(), 1);
        assert_eq!(principal(&harness.driver.owner), parent);
        assert!(fixture.transport.requests().is_empty());
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, |driver| {
            driver.agents.is_none() && presentation_idle(driver)
        })
        .await;
        assert_eq!(harness.driver.input.raw_draft(), Some(("parent draft", 12)));
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn create_form_uses_native_fields_and_restores_the_parent_draft() {
    use native::NativeManagedFormField as Field;
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        harness.input_writer.write_all(b"parent draft").unwrap();
        input_until(&mut harness, |driver| {
            driver.input.raw_draft() == Some(("parent draft", 12))
        })
        .await;
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, displayed).await;
        harness.input_writer.write_all(b"/create\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver) && driver.owner.managed_navigation().unwrap().form.is_some()
        })
        .await;
        harness
            .input_writer
            .write_all("UI child é".as_bytes())
            .unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && driver
                    .owner
                    .managed_navigation()
                    .unwrap()
                    .form
                    .unwrap()
                    .values[0]
                    == "UI child é"
        })
        .await;
        harness.input_writer.write_all(b"\t").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && driver
                    .owner
                    .managed_navigation()
                    .unwrap()
                    .form
                    .is_some_and(|form| form.fields[form.selected] == Field::Mode)
        })
        .await;
        assert_eq!(harness.driver.input.raw_draft(), Some(("", 0)));
        for mode in ["one-off", "persistent"] {
            harness.input_writer.write_all(b" ").unwrap();
            pump_until(&mut harness, |driver| {
                displayed(driver)
                    && driver
                        .owner
                        .managed_navigation()
                        .unwrap()
                        .form
                        .is_some_and(|form| form.values[form.selected] == mode)
            })
            .await;
        }
        harness.input_writer.write_all(b"\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && driver
                    .owner
                    .managed_navigation()
                    .unwrap()
                    .result
                    .is_some_and(|result| {
                        result.ok && result.status == machine_god_core::ManagedResultStatus::Created
                    })
        })
        .await;
        assert!(
            harness
                .driver
                .owner
                .managed_navigation()
                .unwrap()
                .form
                .is_none()
        );
        assert!(fixture.transport.requests().is_empty());
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, |driver| {
            driver.agents.is_none() && presentation_idle(driver)
        })
        .await;
        assert_eq!(harness.driver.input.raw_draft(), Some(("parent draft", 12)));
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn rejected_form_paste_cannot_submit_old_values_and_ctrl_c_clears_native_draft() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, displayed).await;
        harness.input_writer.write_all(b"/create\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver) && driver.owner.managed_navigation().unwrap().form.is_some()
        })
        .await;
        harness.input_writer.write_all(b"retained name").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver) && driver.input.raw_draft() == Some(("retained name", 13))
        })
        .await;
        let old_frame = harness.driver.owner.managed_navigation().unwrap().frame;
        // The paste decoder rejects NUL before the native field sees an edit.
        harness
            .input_writer
            .write_all(b"\x1b[200~\0\x1b[201~\r")
            .unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver) && driver.owner.managed_navigation().unwrap().frame != old_frame
        })
        .await;
        assert_eq!(harness.driver.owner.managed_agents().len(), 1);
        assert_eq!(
            harness
                .driver
                .owner
                .managed_navigation()
                .unwrap()
                .form
                .unwrap()
                .values[0],
            "retained name"
        );
        harness.input_writer.write_all(b"\x03").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && driver
                    .owner
                    .managed_navigation()
                    .unwrap()
                    .form
                    .is_some_and(|form| form.values[0].is_empty())
        })
        .await;
        assert_eq!(harness.driver.input.raw_draft(), Some(("", 0)));
        assert_eq!(harness.driver.owner.managed_agents().len(), 1);
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn a_partial_form_edit_cannot_be_relabelled_or_acknowledge_the_next_field() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, displayed).await;
        harness.input_writer.write_all(b"/create\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver) && driver.owner.managed_navigation().unwrap().form.is_some()
        })
        .await;
        harness.input_writer.write_all(b"\xf0").unwrap();
        input_until(&mut harness, |driver| driver.input.has_pending_raw_input()).await;
        let frame = harness.driver.owner.managed_navigation().unwrap().frame;
        harness
            .driver
            .owner
            .act_on_managed_frame(&frame, native::NativeManagedNavigationAction::Next)
            .unwrap();
        pump_until(&mut harness, presentation_idle).await;
        assert!(matches!(
            harness.driver.agents_binding(),
            Some(InputBinding::Agents { frame: None, .. })
        ));
        harness.input_writer.write_all(b"\x9f\x98\x80 \r").unwrap();
        pump_until(&mut harness, displayed).await;
        let form = harness
            .driver
            .owner
            .managed_navigation()
            .unwrap()
            .form
            .unwrap();
        assert_eq!(
            form.fields[form.selected],
            native::NativeManagedFormField::Mode
        );
        assert_eq!(form.values[form.selected], "persistent");
        assert_eq!(form.values[0], "");
        assert_eq!(harness.driver.input.raw_draft(), Some(("", 0)));
        assert_eq!(harness.driver.owner.managed_agents().len(), 1);
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn ctrl_x_preserves_parent_draft_and_a_same_chunk_enter_cannot_confirm_close() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        let parent = harness.driver.owner.runtime().id();
        harness.input_writer.write_all(b"parent draft").unwrap();
        input_until(&mut harness, |driver| {
            driver
                .input
                .raw_draft()
                .is_some_and(|(text, _)| text == "parent draft")
        })
        .await;
        harness.input_writer.write_all(b"\x18/close\r").unwrap();
        pump_until(&mut harness, displayed).await;
        assert!(matches!(
            harness.driver.owner.managed_navigation().unwrap().route,
            Route::Catalog(_)
        ));
        assert_eq!(harness.driver.input.raw_draft(), Some(("", 0)));
        harness.input_writer.write_all(b"\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && matches!(
                    driver.owner.managed_navigation().unwrap().route,
                    Route::Agent(_)
                )
        })
        .await;
        harness.input_writer.write_all(b"/close\r\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && driver.owner.managed_navigation().unwrap().route == Route::ConfirmClose
        })
        .await;
        assert_eq!(harness.driver.owner.managed_agents().len(), 1);
        assert!(
            harness
                .driver
                .owner
                .managed_navigation()
                .unwrap()
                .result
                .is_none()
        );
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, |driver| {
            driver.agents.is_none() && presentation_idle(driver)
        })
        .await;
        assert_eq!(harness.driver.input.raw_draft(), Some(("parent draft", 12)));
        assert_eq!(harness.driver.owner.runtime().id(), parent);
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

async fn hold_frame(harness: &mut Harness) {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            assert!(harness.driver.poll(cx, &mut harness.signals).is_pending());
            if let Ok(work) = harness.work.try_recv() {
                if matches!(work, OutputWork::Flush)
                    && matches!(
                        &harness.driver.in_flight,
                        Some(InFlight::Flush {
                            confirm: Some(InputBinding::Agents { frame: Some(frame), .. }),
                            ..
                        }) if harness.driver.owner.managed_navigation()
                            .is_some_and(|view| !view.busy && view.frame == *frame)
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

#[test]
fn scrolling_invalidates_the_display_ack_without_replacing_the_editor_or_target() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, displayed).await;
        harness.input_writer.write_all(b"\r").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver)
                && matches!(
                    driver.owner.managed_navigation().unwrap().route,
                    Route::Agent(_)
                )
        })
        .await;
        let view = harness.driver.owner.managed_navigation().unwrap();
        let old = view.frame;
        let editor = view.editor;
        let target = view.target.unwrap().id.clone();
        harness.input_writer.write_all(b"unsent draft").unwrap();
        pump_until(&mut harness, |driver| {
            displayed(driver) && driver.input.raw_draft() == Some(("unsent draft", 12))
        })
        .await;
        harness.input_writer.write_all(b"\x1b[B").unwrap();
        hold_frame(&mut harness).await;
        let view = harness.driver.owner.managed_navigation().unwrap();
        assert_ne!(view.frame, old);
        assert_eq!(view.editor, editor);
        assert_eq!(view.target.unwrap().id, target);
        assert_eq!(harness.driver.input.raw_draft(), Some(("unsent draft", 12)));
        assert!(matches!(
            harness.driver.agents_binding(),
            Some(InputBinding::Agents { frame: None, .. })
        ));
        assert_eq!(
            harness.driver.owner.act_on_managed_frame(
                &old,
                native::NativeManagedNavigationAction::Lifecycle(
                    machine_god_core::ManagedLifecycleAction::Close
                )
            ),
            Err(native::NativeManagedNavigationError::StaleFrame)
        );
        harness
            .ack
            .try_send(OutputAcknowledgement::Succeeded)
            .unwrap();
        pump_until(&mut harness, displayed).await;
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn narrow_catalog_displays_the_full_selected_identity_before_acknowledgement() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        let frontend = harness.driver.frontend.as_mut().unwrap();
        frontend.columns = 40;
        frontend.rows = 11;
        harness.input_writer.write_all(b"\x18").unwrap();
        let output = pump_until(&mut harness, displayed).await;
        let view = harness.driver.owner.managed_navigation().unwrap();
        let target = view.target.unwrap();
        let text = String::from_utf8(output).unwrap().replace("\r\n", "");
        assert!(text.contains(&format!("id: {}", target.id)), "{text}");
        assert!(text.contains(&format!("generation {}", target.generation)));
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn undersized_navigation_cannot_acknowledge_a_selectable_frame() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        harness.driver.frontend.as_mut().unwrap().rows = 10;
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, |driver| {
            driver
                .owner
                .managed_navigation()
                .is_some_and(|view| !view.busy)
                && presentation_idle(driver)
        })
        .await;
        assert!(matches!(
            harness.driver.agents_binding(),
            Some(InputBinding::Agents { frame: None, .. })
        ));
        harness.input_writer.write_all(b"\r").unwrap();
        input_until(&mut harness, |driver| driver.notice.is_some()).await;
        assert!(matches!(
            harness.driver.owner.managed_navigation().unwrap().route,
            Route::Catalog(_)
        ));
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, |driver| {
            driver.agents.is_none() && presentation_idle(driver)
        })
        .await;
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn held_agent_flush_does_not_authorize_enter_or_hold_child_execution() {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    fixture.transport.push(support::answer());
    let result = runtime.block_on(async {
        harness.input_writer.write_all(b"\x18").unwrap();
        hold_frame(&mut harness).await;
        harness.input_writer.write_all(b"\r").unwrap();
        input_until(&mut harness, |driver| driver.notice.is_some()).await;
        assert!(matches!(harness.driver.owner.managed_navigation().unwrap().route, Route::Catalog(_)));
        let child = harness.driver.owner.managed_agents()[0].id.clone();
        let mut response = harness.driver.owner.request_managed_command(
            ManagedSubagentCommand::decode(serde_json::json!({"command":{"message":{"send":{"id":child,"content":"hidden work"}}}})).unwrap(),
            CancellationToken::new(),
        ).unwrap();
        poll_fn(|cx| {
            assert!(harness.driver.poll(cx, &mut harness.signals).is_pending());
            response.as_mut().poll(cx)
        }).await.unwrap();
        until(&mut harness, |driver| fixture.transport.requests().len() == 1
            && driver.owner.managed_agents().iter().any(|agent| agent.id == child && agent.state == machine_god_core::ManagedAgentState::Idle)).await;
        assert!(matches!(harness.driver.in_flight, Some(InFlight::Flush { .. })));
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}

#[test]
fn closing_an_editor_mid_paste_drains_its_original_bytes_before_parent_input() {
    retired_input(b"\x1b[200~old paste", b"/quit\r\x1b[201~\r");
}

#[test]
fn closing_an_editor_mid_utf8_drains_its_original_bytes_before_parent_input() {
    retired_input(b"\xf0\x9f", b"\x98\x80/quit\r");
}

fn retired_input(initial: &[u8], remaining: &[u8]) {
    let runtime = executor();
    let (fixture, mut harness) = runtime.block_on(prepared());
    let result = runtime.block_on(async {
        harness.input_writer.write_all(b"\x18").unwrap();
        pump_until(&mut harness, displayed).await;
        harness.input_writer.write_all(initial).unwrap();
        // The original decoder remains pending until the atomic input completes.
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                harness.driver.poll_input(cx, 101);
                if harness.driver.input.has_pending_raw_input() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        harness.driver.owner.close_managed_navigation();
        harness.driver.sync_agents();
        harness.input_writer.write_all(remaining).unwrap();
        input_until(&mut harness, |driver| {
            !driver.input.draining_managed_editor()
        })
        .await;
        assert!(!harness.driver.shutting_down);
        assert_eq!(harness.driver.owner.runtime().status().queued_jobs, 0);
        assert_eq!(harness.driver.input.raw_draft(), Some(("", 0)));
        assert!(fixture.transport.requests().is_empty());
        finish_signal(&mut harness).await
    });
    let mut tail = dispose(harness, fixture, result);
    runtime.block_on(finish_raw_tail(&mut tail));
}
