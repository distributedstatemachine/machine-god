use super::super::{AskSignalControl, recording_startup};
use super::*;
use machine_god_core::CancellationToken;
use machine_god_native::{TerminalTapeRecordingOptions, TokioWebSearchDeadline};
use std::{
    fs::File,
    sync::atomic::{AtomicBool, Ordering},
};

#[test]
fn recorded_settlement_defers_final_phase_until_native_and_tape_collectors_join() {
    for unwind in [false, true] {
        let fixture = support::Fixture::new();
        let runtime = TokioWebSearchDeadline::build_runtime_pair().unwrap().0;
        let (signal_sender, signal_receiver) = tokio::sync::mpsc::channel(1);
        let mut signals = AskSignals::new(signal_receiver);
        let recording = recording_startup::Settlement::default();
        let started = recording
            .start(
                &runtime,
                recording_startup::Selection::from_values(
                    true,
                    &fixture.workspace,
                    Some("lifecycle.fxtape".into()),
                    None,
                ),
                fixture.host.session_store().clone(),
                fixture.workspace.clone(),
                TerminalTapeRecordingOptions::new(20, 3, 100, b"test".to_vec()),
                &mut signals,
            )
            .unwrap();
        let recorder = started.recorder.unwrap();
        let tape_completion = recorder.completion();
        let host_completion = fixture.host.terminal_shutdown_completion().unwrap();
        let host = Arc::try_unwrap(fixture.host).unwrap_or_else(|_| panic!("single fixture host"));
        let (read, write) = std::io::pipe().unwrap();
        drop(write);
        let input = NativeInteractiveInput::new(
            NativeInteractiveInputSource::AdoptNonblockingStatus(
                std::os::fd::OwnedFd::from(read).into(),
            ),
            CancellationToken::new(),
        );
        let input_completion = input.completion();
        let observed_input = input.completion();
        let terminal = NativeInteractiveTerminal::new(File::open("/dev/null").unwrap());
        let final_seen = Arc::new(AtomicBool::new(false));
        let (control, controller) = final_controller(
            host_completion.clone(),
            observed_input,
            tape_completion.clone(),
            final_seen.clone(),
        );
        let (output, output_worker) = output_worker(recorder);
        let result = settle_with_recording(
            host,
            InputSettlement {
                input_completion,
                terminal,
                runtime: &runtime,
                size_completion: None,
            },
            signals,
            &control,
            |_, _, _| {
                drop(input);
                Ok(FinalPresentation::startup(
                    output,
                    None,
                    None,
                    AskCommandOutcome::Completed,
                    None,
                    None,
                ))
            },
            |mut presentation, signals| {
                assert!(
                    host_completion.is_complete(),
                    "native host cleanup precedes tape finalization"
                );
                assert!(!final_seen.load(Ordering::Acquire));
                assert!(!tape_completion.workers().is_complete());
                assert!(!tape_completion.status().closed);
                assert!(!unwind, "injected final-presentation unwind");
                Ok(runtime.block_on(poll_fn(|cx| presentation.poll(cx, signals))))
            },
            Some(&recording),
        )
        .unwrap();
        assert_eq!(
            result,
            if unwind {
                AskCommandOutcome::OperationalFailure
            } else {
                AskCommandOutcome::Completed
            }
        );
        assert!(final_seen.load(Ordering::Acquire));
        assert!(tape_completion.workers().is_complete());
        assert_eq!(tape_completion.status().complete, !unwind);
        drop(control);
        drop(signal_sender);
        controller.join().unwrap();
        output_worker.join().unwrap();
    }
}

fn final_controller(
    host: machine_god_native::NativeOwnedWorkerCompletion,
    input: machine_god_native::NativeOwnedWorkerCompletion,
    tape: machine_god_native::TerminalTapeRecordingCompletion,
    observed: Arc<AtomicBool>,
) -> (AskSignalControlSender, std::thread::JoinHandle<()>) {
    let (sender, mut controls) = tokio::sync::mpsc::channel(1);
    let controller = std::thread::spawn(move || {
        let Some(AskSignalControl::EnterFinal(ready)) = controls.blocking_recv() else {
            panic!("one final transition");
        };
        assert!(host.is_complete());
        assert!(input.is_complete());
        assert!(
            tape.workers().is_complete(),
            "Final phase cannot expose early process exit while tape work lives"
        );
        observed.store(true, Ordering::Release);
        ready.send(()).unwrap();
    });
    (AskSignalControlSender { sender }, controller)
}

fn output_worker(
    recorder: machine_god_native::TerminalTapeRecorder,
) -> (OutputBridge, std::thread::JoinHandle<()>) {
    let (work, mut output_work) = tokio::sync::mpsc::channel(1);
    let (acknowledged, acknowledgements) = tokio::sync::mpsc::channel(1);
    let worker = std::thread::spawn(move || {
        while output_work.blocking_recv().is_some() {
            if acknowledged
                .blocking_send(OutputAcknowledgement::Succeeded)
                .is_err()
            {
                break;
            }
        }
    });
    (
        OutputBridge {
            work,
            acknowledgements,
            tape: Some(super::super::output::tape::TapeLane::new(recorder, false)),
        },
        worker,
    )
}
