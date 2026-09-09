use super::super::support;
use super::*;
use machine_god_core::{CancellationToken, SessionRecord, TurnEvent};
use machine_god_native::{
    NativeInteractiveInputSource, NativeInteractiveOutcome, NativeInteractivePromptBridge,
    NativeInteractivePromptLimits, NativeInteractiveTerminalSizeReader, NativeModelPreferences,
    NativeReasoningEffort, NativeSessionCatalog, NativeSessionCatalogQuery,
};
use std::{fs::File, future::poll_fn, io::Write as _, os::fd::OwnedFd};

struct Harness {
    startup: Startup,
    inbox: NativeInteractivePromptInbox,
    signals: AskSignals,
    _signal: tokio::sync::mpsc::Sender<AskSignal>,
    work: tokio::sync::mpsc::Receiver<super::super::super::OutputWork>,
    ack: tokio::sync::mpsc::Sender<OutputAcknowledgement>,
    input_writer: std::io::PipeWriter,
    _master: File,
}

fn pty() -> (File, File) {
    use rustix::fs::{Mode, OFlags};
    let master = rustix::fs::open(
        "/dev/ptmx",
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .unwrap();
    rustix::pty::grantpt(&master).unwrap();
    rustix::pty::unlockpt(&master).unwrap();
    #[cfg(target_os = "linux")]
    let slave = rustix::pty::ioctl_tiocgptpeer(
        &master,
        rustix::pty::OpenptFlags::RDWR
            | rustix::pty::OpenptFlags::NOCTTY
            | rustix::pty::OpenptFlags::CLOEXEC,
    )
    .unwrap();
    #[cfg(target_os = "macos")]
    let slave = {
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap()
    };
    rustix::termios::tcsetwinsize(
        &master,
        rustix::termios::Winsize {
            ws_col: 80,
            ws_row: 24,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .unwrap();
    (master.into(), slave.into())
}

async fn harness(fixture: &support::Fixture) -> Harness {
    let (master, slave) = pty();
    let mut resize = Resize::new(NativeInteractiveTerminalSizeReader::new(slave)).unwrap();
    let dimensions = resize.initial_dimensions().await.unwrap();
    let (input, writer) = std::io::pipe().unwrap();
    let input = NativeInteractiveInput::new(
        NativeInteractiveInputSource::AdoptNonblockingStatus(OwnedFd::from(input).into()),
        CancellationToken::new(),
    );
    let (work, received) = tokio::sync::mpsc::channel(1);
    let (ack, acknowledgements) = tokio::sync::mpsc::channel(1);
    let (signal, signals) = tokio::sync::mpsc::channel(1);
    let (_, inbox) =
        NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
    let options = NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        fixture.host.loaded_config().config().model_preferences(),
    )
    .unwrap();
    Harness {
        startup: Startup::new(
            fixture.host.clone(),
            options,
            input,
            OutputBridge {
                tape: None,
                work,
                acknowledgements,
            },
            fixture.host.session_catalog_reader().unwrap(),
            resize,
            dimensions,
        ),
        inbox,
        signals: AskSignals::new(signals),
        _signal: signal,
        work: received,
        ack,
        input_writer: writer,
        _master: master,
    }
}

fn acknowledge(harness: &mut Harness, cx: &Context<'_>) {
    if harness.work.try_recv().is_ok() {
        harness
            .ack
            .try_send(OutputAcknowledgement::Succeeded)
            .unwrap();
        cx.waker().wake_by_ref();
    }
}

async fn count(fixture: &support::Fixture) -> usize {
    NativeSessionCatalog::new(fixture.host.session_lifecycle().session_store().clone())
        .list(NativeSessionCatalogQuery::new(100).unwrap())
        .await
        .unwrap()
        .entries()
        .len()
}

async fn native_outcome(owner: &mut NativeInteractiveSession) -> NativeInteractiveOutcome {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 100);
            while owner.take_presentation().is_some() {}
            owner.take_outcome().map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap()
}

async fn picker_ready(harness: &mut Harness) {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            assert!(harness.startup.poll(cx, &mut harness.signals).is_pending());
            acknowledge(harness, cx);
            if harness
                .startup
                .picker
                .identity()
                .is_some_and(|(_, revision)| revision > 0)
                && harness.startup.render.is_none()
                && harness.startup.in_flight.is_none()
            {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }),
    )
    .await
    .unwrap();
}

async fn saved_tool_turn(fixture: &support::Fixture) -> SessionRecord {
    let preferences = NativeModelPreferences::new(
        "saved/picker",
        NativeReasoningEffort::parse("high").unwrap(),
        true,
    )
    .unwrap();
    let mut owner = NativeInteractiveSession::open(
        fixture.host.clone(),
        NativeInteractiveSessionOptions::new(fixture.workspace.clone(), preferences).unwrap(),
        NativeInteractiveInitialSession::Fresh,
        100,
    )
    .await
    .unwrap();
    fixture.transport.push(support::call(
        "write_file",
        &serde_json::json!({"path":"picker-history.txt","content":"original"}),
    ));
    fixture.transport.push(support::answer());
    owner.enqueue("saved picker request".into()).unwrap();
    assert!(matches!(
        native_outcome(&mut owner).await,
        NativeInteractiveOutcome::Turn(Ok(event))
            if matches!(event.payload, TurnEvent::Completed { .. })
    ));
    let saved = owner.runtime().record();
    owner.request_shutdown();
    assert!(matches!(
        native_outcome(&mut owner).await,
        NativeInteractiveOutcome::Shutdown
    ));
    assert!(owner.is_closed());
    saved
}

#[test]
fn acknowledged_startup_selection_hands_off_exact_history_without_new_work() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fixture = support::Fixture::new();
    runtime.block_on(async {
        let saved = saved_tool_turn(&fixture).await;
        let file = fixture.workspace.join("picker-history.txt");
        assert_eq!(std::fs::read(&file).unwrap(), b"original");
        std::fs::write(&file, b"changed after original turn").unwrap();
        let requests = fixture.transport.requests().len();
        assert_eq!(requests, 2);
        let mut harness = harness(&fixture).await;
        picker_ready(&mut harness).await;
        assert_eq!(count(&fixture).await, 1);
        // Enter arrives through the real input adapter only after frame acknowledgement.
        harness.input_writer.write_all(b"\r").unwrap();
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
            panic!("resumed driver")
        };
        assert_eq!(driver.owner.runtime().id(), saved.id);
        let preferences = driver.owner.runtime().model_preferences();
        assert_eq!(preferences.model(), "saved/picker");
        assert_eq!(preferences.effort().label(), "high");
        assert!(preferences.requested_fast());
        assert!(driver.history.is_some());
        let mut output = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                assert!(driver.poll(cx, &mut harness.signals).is_pending());
                if let Ok(work) = harness.work.try_recv() {
                    if let super::super::super::OutputWork::Write(bytes) = work {
                        assert!(output.len() + bytes.len() <= 64 * 1024);
                        output.extend(bytes);
                    }
                    harness
                        .ack
                        .try_send(OutputAcknowledgement::Succeeded)
                        .unwrap();
                    cx.waker().wake_by_ref();
                }
                if driver.history.is_none() && driver.render.is_none() && driver.in_flight.is_none()
                {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("saved picker request"));
        assert!(output.contains("complete"));
        assert!(output.contains("write_file"));
        assert_eq!(driver.owner.runtime().record().messages, saved.messages);
        assert_eq!(fixture.transport.requests().len(), requests);
        assert_eq!(std::fs::read(file).unwrap(), b"changed after original turn");
        assert_eq!(count(&fixture).await, 1);
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
        drop(driver);
    });
    fixture.finish();
}

#[test]
fn startup_picker_allocates_nothing_until_escape_then_creates_one_writer() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fixture = support::Fixture::new();
    runtime.block_on(async {
        let mut harness = harness(&fixture).await;
        picker_ready(&mut harness).await;
        assert_eq!(count(&fixture).await, 0);
        let (generation, revision) = harness.startup.picker.identity().unwrap();
        harness.startup.event(
            &ComposerEvent::Submit(String::new()),
            &InputBinding::Picker {
                generation,
                revision,
            },
        );
        assert!(
            harness.startup.pending.is_none(),
            "empty Enter is never a fresh writer or prompt"
        );
        harness.startup.event(
            &ComposerEvent::EscapeRequested,
            &InputBinding::Picker {
                generation,
                revision,
            },
        );
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
        assert_eq!(count(&fixture).await, 1);
        assert!(harness.startup.owner.is_some());
        let Ok(mut driver) = harness.startup.into_result(harness.inbox).unwrap() else {
            panic!("fresh driver")
        };
        assert!(driver.history.is_none());
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
        drop(driver);
    });
    fixture.finish();
}

#[test]
fn startup_ctrl_d_exits_without_allocating_any_session() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fixture = support::Fixture::new();
    runtime.block_on(async {
        let mut harness = harness(&fixture).await;
        harness
            .startup
            .event(&ComposerEvent::ExitRequested, &InputBinding::Command);
        assert_eq!(count(&fixture).await, 0);
        let Err(mut final_output) = harness.startup.into_result(harness.inbox).unwrap() else {
            panic!("no driver")
        };
        let result = tokio::time::timeout(
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
        assert_eq!(result.outcome, AskCommandOutcome::Completed);
    });
    fixture.finish();
}
