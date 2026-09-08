//! Interactive presentation host. Native owns sessions, policy and persistence.

mod commands;
mod composer;
mod composer_view;
mod driver;
mod framing;
mod input_lines;
mod presentation;
mod resize;
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
    NativeInteractiveControlOutcome, NativeInteractiveInitialSession, NativeInteractiveInput,
    NativeInteractiveInputHelper, NativeInteractiveInputSource, NativeInteractiveOutcome,
    NativeInteractivePromptBridge, NativeInteractivePromptInbox, NativeInteractivePromptLimits,
    NativeInteractiveSession, NativeInteractiveSessionOptions, NativeInteractiveTerminal,
    NativeResumeTarget,
};
use presentation::Modal;
use std::{
    future::poll_fn,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

pub(super) fn execute(
    selection: InteractiveSessionSelection,
    output: &mut dyn std::io::Write,
    mut controller: AskSignalController,
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
                    let (bridge, inbox) = NativeInteractivePromptBridge::new(
                        NativeInteractivePromptLimits::default(),
                    )
                    .map_err(|_| ())?;
                    let (source, terminal) = capture_input()?;
                    let size_reader = capture_output_size()?;
                    let input = NativeInteractiveInput::new(
                        source,
                        machine_god_core::CancellationToken::new(),
                    );
                    let input_completion = input.completion();
                    let PreparedConversationHost {
                        host,
                        runtime,
                        workspace,
                        catalog,
                        model_routes: _model_routes,
                        observations: _observations,
                        catalog_cache: _catalog_cache,
                        user_config,
                    } = prepare_conversation_host_with_activation(bridge.clone(), bridge, || {
                        control.activate_turn()
                    })?;
                    settle(
                        host,
                        InputSettlement {
                            input_completion,
                            terminal,
                            runtime: &runtime,
                            size_completion: Some(size_reader.completion()),
                        },
                        signals,
                        &control,
                        |host, signals, terminal| {
                            runtime.block_on(async {
                                let mut options = NativeInteractiveSessionOptions::new(
                                    workspace,
                                    host.loaded_config().config().model_preferences(),
                                )
                                .map_err(|_| ())?;
                                if let Some(catalog) = &catalog {
                                    options = options.with_catalog(catalog.clone());
                                }
                                let owner = NativeInteractiveSession::open(
                                    host,
                                    options,
                                    initial_selection(selection),
                                    wall_clock_ms()?,
                                )
                                .await
                                .map_err(|_| ())?;
                                terminal.activate().await.map_err(|_| ())?;
                                let mut resize = resize::Resize::new(size_reader)?;
                                let columns = resize.initial_columns().await?;
                                let mut driver = Driver::new(
                                    owner,
                                    input,
                                    inbox,
                                    OutputBridge {
                                        work,
                                        acknowledgements,
                                    },
                                )?
                                .with_resources(catalog, user_config)
                                .with_raw_input(columns.get(), Some(resize));
                                let result = poll_fn(|cx| driver.poll(cx, signals)).await;
                                Ok(driver.into_presentation(result))
                            })
                        },
                        |mut presentation, signals| {
                            Ok(runtime.block_on(poll_fn(|cx| presentation.poll(cx, signals))))
                        },
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

fn initial_selection(selection: InteractiveSessionSelection) -> NativeInteractiveInitialSession {
    match selection {
        InteractiveSessionSelection::Fresh => NativeInteractiveInitialSession::Fresh,
        InteractiveSessionSelection::Latest => {
            NativeInteractiveInitialSession::Resume(NativeResumeTarget::Latest)
        }
        InteractiveSessionSelection::Exact(id) => {
            NativeInteractiveInitialSession::Resume(NativeResumeTarget::Exact(id))
        }
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
fn settle(
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
    control.enter_final()?;
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
    render: Option<Render>,
    in_flight: Option<InFlight>,
    notice: Option<Vec<u8>>,
    outcome: Option<NativeInteractiveOutcome>,
    control_outcome: Option<NativeInteractiveControlOutcome>,
    scope_active: bool,
    shutting_down: bool,
    input_ended: bool,
    native_failed: bool,
    output_failed: bool,
    signal: Option<AskSignal>,
    grace: Option<Pin<Box<tokio::time::Sleep>>>,
    stalled_output: bool,
    final_flush_sent: bool,
    catalog: Option<Arc<machine_god_native::NativeModelCatalog>>,
    user_config: Option<Arc<machine_god_native::NativeUserConfigStore>>,
    frontend: Option<Frontend>,
}

struct Frontend {
    columns: u16,
    dirty: bool,
    visible: bool,
    cancel_armed: Option<std::time::Instant>,
    resize: Option<resize::Resize>,
}

impl Driver {
    fn new(
        owner: NativeInteractiveSession,
        input: NativeInteractiveInput,
        mut inbox: NativeInteractivePromptInbox,
        output: OutputBridge,
    ) -> Result<Self, ()> {
        inbox.activate(principal(&owner)).map_err(|_| ())?;
        Ok(Self {
            owner,
            input: InputLines::new(input),
            inbox,
            output,
            modal: None,
            render: None,
            in_flight: None,
            notice: Some(
                "machine-god interactive — /help for commands, /quit to exit\n> "
                    .as_bytes()
                    .to_vec(),
            ),
            outcome: None,
            control_outcome: None,
            scope_active: true,
            shutting_down: false,
            input_ended: false,
            native_failed: false,
            output_failed: false,
            signal: None,
            grace: None,
            stalled_output: false,
            final_flush_sent: false,
            catalog: None,
            user_config: None,
            frontend: None,
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
    fn with_raw_input(mut self, columns: u16, resize: Option<resize::Resize>) -> Self {
        self.input = InputLines::new_raw(self.input.input);
        if let Some(notice) = &mut self.notice {
            notice.splice(..0, b"\x1b[?2004h".iter().copied());
        }
        self.frontend = Some(Frontend {
            columns,
            dirty: true,
            visible: false,
            cancel_armed: None,
            resize,
        });
        self
    }
    fn shutdown(&mut self) {
        if self.shutting_down {
            return;
        }
        self.shutting_down = true;
        self.input.reset_raw_draft();
        self.inbox.close();
        self.scope_active = false;
        self.modal.take();
        self.input.input.request_stop();
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
