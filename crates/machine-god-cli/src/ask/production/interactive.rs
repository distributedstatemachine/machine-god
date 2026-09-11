//! Interactive presentation host. Native owns sessions, policy and persistence.

const MAX_PRESENTATION_OUTPUT_BYTES: usize = 64 * 1024;

fn bounded_output() -> crate::bounded_output::BoundedOutput {
    crate::bounded_output::BoundedOutput::with_capacity(MAX_PRESENTATION_OUTPUT_BYTES, 1024)
}

mod allowlist_view;
mod background_open;
mod clipboard;
mod commands;
mod composer;
mod composer_view;
mod driver;
mod framing;
mod history_view;
mod input_lines;
mod picker;
mod picker_driver;
mod picker_startup;
mod presentation;
#[cfg(test)]
mod recording_lifetime_tests;
#[cfg(test)]
mod recording_process_tests;
mod resize;
mod saved_rules;
mod skills_driver;
mod skills_receipts;
mod skills_view;
#[cfg(test)]
mod terminal_lifetime_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
use machine_god_native as native;
#[cfg(test)]
#[path = "../../../../machine-god-native/tests/interactive_session/support.rs"]
mod support;

use super::{
    AskCommandOutcome, AskSignal, AskSignalControlSender, AskSignalController, AskSignals,
    OutputAcknowledgement, OutputBridge, OutputWork, PreparedConversationHost, SIGNAL_OUTPUT_GRACE,
    TurnDriveResult, map_thread_spawn, prepare_conversation_host_with_activation, serve_output,
    wall_clock_ms,
};
use crate::ask::InteractiveSessionSelection;
use driver::FinalPresentation;
use input_lines::{InputBinding, InputLines};
use machine_god_core::BackgroundOutputOwner;
use machine_god_native::{
    NativeInteractiveControlOutcome, NativeInteractiveCopyOutcome, NativeInteractiveInitialSession,
    NativeInteractiveInput, NativeInteractiveInputHelper, NativeInteractiveInputSource,
    NativeInteractiveOutcome, NativeInteractivePromptBridge, NativeInteractivePromptInbox,
    NativeInteractivePromptLimits, NativeInteractiveSession, NativeInteractiveSessionOptions,
    NativeInteractiveTerminal, NativeResumeTarget,
};
use presentation::Modal;
use std::{
    future::poll_fn,
    sync::Arc,
    task::{Context, Poll},
};

pub(super) fn execute(
    launch: &crate::workspace::launch::LaunchWorkspaceOptions,
    record_requested: bool,
    selection: InteractiveSessionSelection,
    output: &mut dyn std::io::Write,
    controller: AskSignalController,
) -> (AskCommandOutcome, AskSignalController) {
    execute_with_preparation(
        record_requested,
        selection,
        output,
        controller,
        |bridge, control| {
            prepare_conversation_host_with_activation(
                launch,
                bridge.clone(),
                bridge,
                || control.activate_turn(),
                true,
            )
        },
        capture_input,
    )
}

type CapturedInput = (NativeInteractiveInputSource, NativeInteractiveTerminal);

/// Private effect-boundary injection for owned subprocess fixtures. Production
/// passes the same captured-input and prepared-host factories used ordinarily.
fn execute_with_preparation(
    record_requested: bool,
    selection: InteractiveSessionSelection,
    output: &mut dyn std::io::Write,
    mut controller: AskSignalController,
    prepare: impl FnOnce(
        Arc<NativeInteractivePromptBridge>,
        &AskSignalControlSender,
    ) -> Result<PreparedConversationHost, ()>
    + Send,
    capture: impl FnOnce() -> Result<CapturedInput, ()> + Send,
) -> (AskCommandOutcome, AskSignalController) {
    let control = controller.control();
    let Ok(signals) = controller.take_signals() else {
        return (AskCommandOutcome::OperationalFailure, controller);
    };
    let result = std::thread::scope(|scope| {
        let (work, received) = tokio::sync::mpsc::channel(1);
        let (acknowledged, acknowledgements) = tokio::sync::mpsc::channel(1);
        let worker = map_thread_spawn(
            std::thread::Builder::new()
                .name("machine-god-interactive".into())
                .spawn_scoped(scope, move || {
                    run_interactive(
                        record_requested,
                        selection,
                        OutputBridge {
                            work,
                            acknowledgements,
                            tape: None,
                        },
                        signals,
                        &control,
                        prepare,
                        capture,
                    )
                }),
        )?;
        serve_output(received, &acknowledged, output);
        worker.join().map_err(|_| ())?
    });
    let outcome = result.unwrap_or_else(|()| {
        let _ = controller.enter_final();
        AskCommandOutcome::OperationalFailure
    });
    (outcome, controller)
}

fn run_interactive(
    record_requested: bool,
    selection: InteractiveSessionSelection,
    mut output: OutputBridge,
    signals: AskSignals,
    control: &AskSignalControlSender,
    prepare: impl FnOnce(
        Arc<NativeInteractivePromptBridge>,
        &AskSignalControlSender,
    ) -> Result<PreparedConversationHost, ()>,
    capture: impl FnOnce() -> Result<CapturedInput, ()>,
) -> Result<AskCommandOutcome, ()> {
    let (bridge, inbox) =
        NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default())
            .map_err(|_| ())?;
    let (source, terminal) = capture()?;
    let size_reader = capture_output_size()?;
    let input = NativeInteractiveInput::new(source, machine_god_core::CancellationToken::new());
    let input_completion = input.completion();
    let Ok(PreparedConversationHost {
        host,
        runtime,
        workspace,
        state_path,
        catalog,
        model_routes: _model_routes,
        observations: _observations,
        catalog_cache: _catalog_cache,
        user_config,
        skills_snapshot,
    }) = prepare(bridge, control)
    else {
        return super::finish_setup_failure(signals, control);
    };
    let clipboard = clipboard::capture(&workspace);
    let background_opener = background_open::capture();
    let recording_selection =
        super::recording_startup::Selection::capture(record_requested, &workspace);
    let recording = super::recording_startup::Settlement::default();
    settle_with_recording(
        host,
        InputSettlement {
            input_completion,
            terminal,
            runtime: &runtime,
            size_completion: Some(size_reader.completion()),
        },
        signals,
        control,
        |host, signals, terminal| {
            let prepared = prepare_terminal_presentation(
                &runtime,
                terminal,
                size_reader,
                &host,
                RecordingSetup {
                    selection: recording_selection,
                    state_path,
                    owner: &recording,
                },
                signals,
            )?;
            output.tape = prepared.tape;
            runtime.block_on(async {
                let mut options = NativeInteractiveSessionOptions::new(
                    workspace,
                    host.loaded_config().config().model_preferences(),
                )
                .map_err(|_| ())?;
                if let Some(catalog) = &catalog {
                    options = options.with_catalog(catalog.clone());
                }
                let options = clipboard::configure(options, clipboard);
                let options = background_open::configure(options, background_opener);
                let opening = InitialPresentation {
                    selection,
                    input,
                    inbox,
                    output,
                    resize: prepared.resize,
                    dimensions: prepared.dimensions,
                    startup_notice: prepared.notice,
                };
                let mut driver = match opening.open(host, options, signals).await? {
                    Ok(driver) => driver
                        .with_resources(catalog, user_config)
                        .with_skills_snapshot(skills_snapshot),
                    Err(presentation) => return Ok(presentation),
                };
                let result = poll_fn(|cx| driver.poll(cx, signals)).await;
                Ok(driver.into_presentation(result))
            })
        },
        |mut presentation, signals| {
            Ok(runtime.block_on(poll_fn(|cx| presentation.poll(cx, signals))))
        },
        Some(&recording),
    )
}

struct RecordingSetup<'a> {
    selection: Option<super::recording_startup::Selection>,
    state_path: std::path::PathBuf,
    owner: &'a super::recording_startup::Settlement,
}

struct PreparedTerminalPresentation {
    resize: resize::Resize,
    dimensions: machine_god_native::NativeInteractiveTerminalDimensions,
    tape: Option<super::output::tape::TapeLane>,
    notice: Option<Vec<u8>>,
}

/// Native terminal/header preparation completes before any session admission.
fn prepare_terminal_presentation(
    runtime: &machine_god_native::TokioWebSearchRuntime,
    terminal: &mut NativeInteractiveTerminal,
    size_reader: machine_god_native::NativeInteractiveTerminalSizeReader,
    host: &machine_god_native::NativeReferenceHost,
    recording: RecordingSetup<'_>,
    signals: &mut AskSignals,
) -> Result<PreparedTerminalPresentation, ()> {
    let (resize, dimensions) = runtime.block_on(async {
        terminal.activate().await.map_err(|_| ())?;
        let mut resize = resize::Resize::new(size_reader)?;
        let dimensions = resize.initial_dimensions().await?;
        Ok::<_, ()>((resize, dimensions))
    })?;
    let started = recording.owner.start(
        runtime,
        recording.selection,
        host.session_store().clone(),
        recording.state_path,
        machine_god_native::TerminalTapeRecordingOptions::new(
            dimensions.columns().get(),
            dimensions.rows().get(),
            wall_clock_ms()?,
            env!("CARGO_PKG_VERSION").as_bytes().to_vec(),
        ),
        signals,
    )?;
    let tape = started.recorder.map(|recorder| {
        let mut tape = super::output::tape::TapeLane::new(recorder, started.record_stdin);
        tape.marker(b"machine-god:interactive");
        tape
    });
    Ok(PreparedTerminalPresentation {
        resize,
        dimensions,
        tape,
        notice: started.notice,
    })
}

struct InitialPresentation {
    selection: InteractiveSessionSelection,
    input: NativeInteractiveInput,
    inbox: NativeInteractivePromptInbox,
    output: OutputBridge,
    resize: resize::Resize,
    dimensions: machine_god_native::NativeInteractiveTerminalDimensions,
    startup_notice: Option<Vec<u8>>,
}

impl InitialPresentation {
    async fn open(
        self,
        host: Arc<machine_god_native::NativeReferenceHost>,
        options: NativeInteractiveSessionOptions,
        signals: &mut AskSignals,
    ) -> Result<Result<Driver, FinalPresentation>, ()> {
        let picker_reader = host.session_catalog_reader().map_err(|_| ())?;
        let replay_history = !matches!(self.selection, InteractiveSessionSelection::Fresh);
        let dimensions = self.dimensions;
        if let Some(initial) = initial_selection(self.selection) {
            let owner = NativeInteractiveSession::open(host, options, initial, wall_clock_ms()?)
                .await
                .map_err(|_| ())?;
            let mut driver = Driver::new(owner, self.input, self.inbox, self.output)?
                .with_history(replay_history)
                .with_raw_input(dimensions.columns().get(), Some(self.resize))
                .with_startup_notice(self.startup_notice);
            driver.frontend.as_mut().expect("raw frontend").rows = dimensions.rows().get();
            driver.picker = Some(picker::Picker::new(
                picker_reader,
                Some(driver.owner.runtime().id()),
                dimensions.rows().get(),
            ));
            Ok(Ok(driver))
        } else {
            let mut startup = picker_startup::Startup::new(
                host,
                options,
                self.input,
                self.output,
                picker_reader,
                self.resize,
                dimensions,
            )
            .with_startup_notice(self.startup_notice);
            poll_fn(|cx| startup.poll(cx, signals)).await;
            startup.into_result(self.inbox)
        }
    }
}

fn initial_selection(
    selection: InteractiveSessionSelection,
) -> Option<NativeInteractiveInitialSession> {
    match selection {
        InteractiveSessionSelection::Picker => None,
        InteractiveSessionSelection::Fresh => Some(NativeInteractiveInitialSession::Fresh),
        InteractiveSessionSelection::Latest => Some(NativeInteractiveInitialSession::Resume(
            NativeResumeTarget::Latest,
        )),
        InteractiveSessionSelection::Exact(id) => Some(NativeInteractiveInitialSession::Resume(
            NativeResumeTarget::Exact(id),
        )),
    }
}

fn capture_input() -> Result<(NativeInteractiveInputSource, NativeInteractiveTerminal), ()> {
    use std::os::fd::AsFd;
    let path = std::env::current_exe().map_err(|_| ())?;
    let executable = std::fs::File::open(&path).map_err(|_| ())?;
    let helper = NativeInteractiveInputHelper::new(&path, executable).map_err(|_| ())?;
    let input = std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|_| ())?;
    let input = std::fs::File::from(input);
    let terminal = NativeInteractiveTerminal::new(input.try_clone().map_err(|_| ())?);
    Ok((
        NativeInteractiveInputSource::PreserveShared { input, helper },
        terminal,
    ))
}

fn capture_output_size() -> Result<machine_god_native::NativeInteractiveTerminalSizeReader, ()> {
    use std::os::fd::AsFd;
    let output = std::io::stdout()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|_| ())?;
    Ok(machine_god_native::NativeInteractiveTerminalSizeReader::new(output.into()))
}

struct InputSettlement<'a> {
    input_completion: machine_god_native::NativeOwnedWorkerCompletion,
    terminal: NativeInteractiveTerminal,
    runtime: &'a machine_god_native::TokioWebSearchRuntime,
    size_completion: Option<machine_god_native::NativeOwnedWorkerCompletion>,
}

impl InputSettlement<'_> {
    /// Readers release their termios reservation before restoration. Every join
    /// is attempted even if an earlier receipt reports failure.
    fn finish(mut self) -> Result<(), ()> {
        let input_result = self.input_completion.wait_on_worker();
        drop(self.input_completion);
        let completion = self.terminal.completion();
        let restoration = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.block_on(self.terminal.restore())
        }));
        drop(self.terminal);
        let joined = completion.wait_on_worker();
        let size_result = self
            .size_completion
            .map(|completion| completion.wait_on_worker())
            .transpose();
        input_result.map_err(|_| ())?;
        joined.map_err(|_| ())?;
        size_result.map_err(|_| ())?;
        restoration.map_err(std::mem::forget)?.map_err(|_| ())?;
        Ok(())
    }
}

/// Retain signal observation outside the unwind boundary and settle the exact
/// full host once, after all interactive owners have been dropped.
fn settle_with_recording(
    host: machine_god_native::NativeReferenceHost,
    mut input: InputSettlement<'_>,
    mut signals: AskSignals,
    control: &AskSignalControlSender,
    operation: impl FnOnce(
        Arc<machine_god_native::NativeReferenceHost>,
        &mut AskSignals,
        &mut NativeInteractiveTerminal,
    ) -> Result<FinalPresentation, ()>,
    render: impl FnOnce(FinalPresentation, &mut AskSignals) -> Result<TurnDriveResult, ()>,
    recording: Option<&super::recording_startup::Settlement>,
) -> Result<AskCommandOutcome, ()> {
    let completion = host.terminal_shutdown_completion().ok_or(())?;
    let host = Arc::new(host);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        operation(host.clone(), &mut signals, &mut input.terminal)
    }));
    drop(host);
    // Both joins run on the dedicated caller worker, outside async polling.
    // Attempt both even when one reports a failure.
    let input_result = input.finish();
    let host_result = completion.wait_on_worker();
    // A first signal in Final may immediately exit. Keep latching while any
    // recording worker remains owned, including throughout final output.
    let recording_live = recording.is_some_and(super::recording_startup::Settlement::is_live);
    let early_final = if recording_live {
        Ok(())
    } else {
        control.enter_final()
    };
    let result = result
        .map_err(|payload| {
            std::mem::forget(payload);
        })
        .and_then(std::convert::identity)
        .and_then(|presentation| {
            input_result?;
            host_result.map_err(|_| ())?;
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                render(presentation, &mut signals)
            }))
            .map_err(|payload| {
                std::mem::forget(payload);
            })?
        });
    let recording_result = recording
        .map(super::recording_startup::Settlement::finish)
        .transpose();
    if recording_live {
        control.enter_final()?;
    }
    let result = result.and_then(|value| {
        early_final?;
        recording_result?;
        Ok(value)
    });
    let operation_failed = result.is_err();
    let mut result = result.unwrap_or(TurnDriveResult {
        outcome: AskCommandOutcome::OperationalFailure,
        stalled_output_after_signal: false,
    });
    if let Some(signal) = signals
        .first_observed
        .or_else(|| signals.receiver.try_recv().ok())
    {
        result.outcome = signal.outcome();
    }
    if result.stalled_output_after_signal
        || (operation_failed
            && matches!(
                result.outcome,
                AskCommandOutcome::Interrupted | AskCommandOutcome::Terminated
            ))
    {
        control.finish(result.outcome.exit_code())?;
    }
    Ok(result.outcome)
}

struct Render {
    history: bool,
    clear_row: bool,
    bytes: Vec<u8>,
    offset: usize,
    confirm: Option<InputBinding>,
    receipt: Option<ReceiptKind>,
    model_text: bool,
}
enum ReceiptKind {
    Outcome,
    Control,
    Copy,
}
enum InFlight {
    Bytes,
    Flush {
        confirm: Option<InputBinding>,
        receipt: Option<ReceiptKind>,
    },
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "input, stdout acknowledgement, native lifecycle and signal lanes progress independently"
)]
struct Driver {
    owner: NativeInteractiveSession,
    input: InputLines,
    inbox: NativeInteractivePromptInbox,
    output: OutputBridge,
    modal: Option<Modal>,
    saved_rule: Option<saved_rules::Confirmation>,
    rule_generation: u64,
    render: Option<Render>,
    in_flight: Option<InFlight>,
    notice: Option<Vec<u8>>,
    skills_warning: Option<&'static [u8]>,
    outcome: Option<NativeInteractiveOutcome>,
    control_outcome: Option<NativeInteractiveControlOutcome>,
    copy_outcome: Option<NativeInteractiveCopyOutcome>,
    scope_active: bool,
    shutting_down: bool,
    input_ended: bool,
    native_failed: bool,
    output_failed: bool,
    signal: Option<AskSignal>,
    final_flush_sent: bool,
    catalog: Option<Arc<machine_god_native::NativeModelCatalog>>,
    user_config: Option<Arc<machine_god_native::NativeUserConfigStore>>,
    frontend: Option<Frontend>,
    history: Option<history_view::HistoryView>,
    picker: Option<picker::Picker>,
    picker_request: Option<machine_god_native::NativeInteractiveRequestId>,
    picker_rejection: Option<machine_god_native::NativeInteractiveRequestId>,
    skills: Option<skills_driver::SkillsUi>,
}

struct Frontend {
    columns: u16,
    rows: u16,
    menu_height: Option<u16>,
    dirty: bool,
    visible: bool,
    cancel_armed: Option<std::time::Instant>,
    resize: Option<resize::Resize>,
}

impl Driver {
    fn new(
        owner: NativeInteractiveSession,
        input: NativeInteractiveInput,
        inbox: NativeInteractivePromptInbox,
        output: OutputBridge,
    ) -> Result<Self, ()> {
        Self::from_lines(owner, InputLines::new(input), inbox, output)
    }

    fn from_lines(
        owner: NativeInteractiveSession,
        input: InputLines,
        mut inbox: NativeInteractivePromptInbox,
        output: OutputBridge,
    ) -> Result<Self, ()> {
        inbox.activate(principal(&owner)).map_err(|_| ())?;
        let skills = owner
            .skills_catalog()
            .map(|_| skills_driver::SkillsUi::new(None));
        Ok(Self {
            owner,
            input,
            inbox,
            output,
            modal: None,
            saved_rule: None,
            rule_generation: 0,
            render: None,
            in_flight: None,
            notice: Some(
                "machine-god interactive — /help for commands, /quit to exit\n> "
                    .as_bytes()
                    .to_vec(),
            ),
            outcome: None,
            skills_warning: None,
            control_outcome: None,
            copy_outcome: None,
            scope_active: true,
            shutting_down: false,
            input_ended: false,
            native_failed: false,
            output_failed: false,
            signal: None,
            final_flush_sent: false,
            catalog: None,
            user_config: None,
            frontend: None,
            history: None,
            picker: None,
            picker_request: None,
            picker_rejection: None,
            skills,
        })
    }
    fn with_resources(
        mut self,
        catalog: Option<Arc<machine_god_native::NativeModelCatalog>>,
        user_config: Option<Arc<machine_god_native::NativeUserConfigStore>>,
    ) -> Self {
        self.catalog = catalog;
        self.user_config = user_config;
        self
    }
    fn with_startup_notice(mut self, notice: Option<Vec<u8>>) -> Self {
        if let Some(mut notice) = notice {
            if let Some(existing) = self.notice.take() {
                notice.extend(existing);
            }
            self.notice = Some(notice);
        }
        self
    }
    fn with_raw_input(mut self, columns: u16, resize: Option<resize::Resize>) -> Self {
        self.input = InputLines::new_raw(self.input.input);
        if let Some(notice) = &mut self.notice {
            notice.splice(..0, b"\x1b[?2004h".iter().copied());
        }
        self.frontend = Some(Frontend {
            columns,
            rows: 24,
            menu_height: None,
            dirty: true,
            visible: false,
            cancel_armed: None,
            resize,
        });
        self
    }
    fn with_history(mut self, replay: bool) -> Self {
        if replay {
            self.history = Some(history_view::HistoryView::new(
                self.owner.runtime().record(),
            ));
        }
        self
    }

    fn replace_history(&mut self) {
        self.discard_history();
        self.history = Some(history_view::HistoryView::new(
            self.owner.runtime().record(),
        ));
    }

    fn discard_history(&mut self) {
        self.history.take();
        // An acknowledged/in-flight chunk cannot be retracted. Never emit its
        // unsent remainder after confirmed installation of another session.
        if self.render.as_ref().is_some_and(|render| render.history) {
            self.render.take();
        }
    }
    fn shutdown(&mut self) {
        if self.shutting_down {
            return;
        }
        self.shutting_down = true;
        if let Some(picker) = &mut self.picker {
            picker.close();
        }
        self.discard_history();
        self.input.reset_raw_draft();
        self.inbox.close();
        self.scope_active = false;
        self.modal.take();
        self.input.input.request_stop();
        self.saved_rule.take();
        self.owner.request_shutdown();
    }
    fn note(&mut self, text: &'static [u8]) {
        if self.notice.is_none() {
            self.notice = Some(text.to_vec());
        }
    }
}

fn principal(owner: &NativeInteractiveSession) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(owner.runtime().id(), owner.runtime().incarnation_id())
}
