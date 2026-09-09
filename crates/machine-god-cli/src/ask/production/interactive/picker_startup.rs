//! Startup owns only input and presentation until the user selects a session
//! or explicitly dismisses the picker. No provisional writer is allocated.

use super::{
    AskCommandOutcome, AskSignal, AskSignals, Driver, Frontend, InFlight, InputBinding, InputLines,
    NativeInteractiveInitialSession, NativeInteractiveInput, NativeInteractivePromptInbox,
    NativeInteractiveSession, NativeInteractiveSessionOptions, OutputAcknowledgement, OutputBridge,
    Render, SIGNAL_OUTPUT_GRACE,
    composer::{ComposerContext, ComposerEvent},
    driver::{FinalPresentation, next_render_work},
    picker::{Picker, Selection},
    resize::Resize,
    wall_clock_ms,
};
use machine_god_core::BoxFuture;
use machine_god_native::{
    NativeInteractiveError, NativeInteractiveTerminalDimensions, NativeReferenceHost,
    NativeResumeTarget, NativeSessionCatalogReader, NativeSessionCatalogScope,
};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

#[cfg(test)]
mod tests;

pub(super) struct Startup {
    host: Arc<NativeReferenceHost>,
    options: NativeInteractiveSessionOptions,
    input: InputLines,
    output: OutputBridge,
    picker: Picker,
    resize: Resize,
    dimensions: NativeInteractiveTerminalDimensions,
    pending: Option<BoxFuture<'static, Result<NativeInteractiveSession, NativeInteractiveError>>>,
    owner: Option<NativeInteractiveSession>,
    replay: bool,
    render: Option<Render>,
    in_flight: Option<InFlight>,
    menu_height: Option<u16>,
    cancel_armed: Option<Instant>,
    stopped: Option<AskCommandOutcome>,
    signal: Option<AskSignal>,
    grace: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl Startup {
    pub fn new(
        host: Arc<NativeReferenceHost>,
        options: NativeInteractiveSessionOptions,
        input: NativeInteractiveInput,
        output: OutputBridge,
        reader: NativeSessionCatalogReader,
        resize: Resize,
        dimensions: NativeInteractiveTerminalDimensions,
    ) -> Self {
        let mut picker = Picker::new(reader, None, dimensions.rows().get());
        picker.open(NativeSessionCatalogScope::CurrentWorkspace);
        Self {
            host,
            options,
            input: InputLines::new_raw(input),
            output,
            picker,
            resize,
            dimensions,
            pending: None,
            owner: None,
            replay: false,
            render: Some(Render {
                bytes: b"\x1b[?2004h".to_vec(),
                offset: 0,
                history: false,
                clear_row: true,
                confirm: None,
                receipt: None,
                model_text: false,
            }),
            in_flight: None,
            menu_height: None,
            cancel_armed: None,
            stopped: None,
            signal: None,
            grace: None,
        }
    }

    pub fn poll(&mut self, cx: &mut Context<'_>, signals: &mut AskSignals) -> Poll<()> {
        if self.signal.is_none()
            && let Poll::Ready(signal) = signals.poll_signal(cx)
        {
            self.signal = Some(signal);
            self.grace = Some(Box::pin(tokio::time::sleep(SIGNAL_OUTPUT_GRACE)));
            self.stop(signal.outcome());
        }
        if self.stopped.is_some() {
            return Poll::Ready(());
        }
        self.poll_open(cx);
        self.picker.poll(cx);
        match self.resize.poll(cx) {
            Poll::Ready(Ok(dimensions)) => {
                self.dimensions = dimensions;
                self.picker.resize(dimensions.rows().get());
            }
            Poll::Ready(Err(())) => self.stop(AskCommandOutcome::OperationalFailure),
            Poll::Pending => {}
        }
        if self.owner.is_none() && self.stopped.is_none() {
            self.poll_input(cx);
        }
        self.poll_output(cx);
        if self.owner.is_some()
            && self.render.is_none()
            && self.in_flight.is_none()
            && self.menu_height.is_none()
            || self.stopped.is_some()
        {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }

    fn stop(&mut self, outcome: AskCommandOutcome) {
        self.stopped = Some(outcome);
        self.picker.close();
        self.pending.take();
        self.input.input.request_stop();
    }

    fn poll_open(&mut self, cx: &mut Context<'_>) {
        let Some(pending) = &mut self.pending else {
            return;
        };
        let Poll::Ready(result) = pending.as_mut().poll(cx) else {
            return;
        };
        self.pending = None;
        match result {
            Ok(owner) => {
                self.owner = Some(owner);
                self.picker.close();
                self.input.reset_raw_draft();
            }
            Err(error) if self.replay => self
                .picker
                .selection_failed(super::picker_driver::resume_failure(&error)),
            Err(_) => self.stop(AskCommandOutcome::OperationalFailure),
        }
        cx.waker().wake_by_ref();
    }

    fn open(&mut self, selection: NativeInteractiveInitialSession) {
        let Ok(now_ms) = wall_clock_ms() else {
            self.stop(AskCommandOutcome::OperationalFailure);
            return;
        };
        self.replay = matches!(selection, NativeInteractiveInitialSession::Resume(_));
        self.pending = Some(NativeInteractiveSession::open(
            self.host.clone(),
            self.options.clone(),
            selection,
            now_ms,
        ));
    }

    fn poll_input(&mut self, cx: &mut Context<'_>) {
        let binding = self
            .picker
            .input_binding()
            .unwrap_or(InputBinding::AwaitingPrompt);
        let polled = self.input.poll_event(
            cx,
            binding,
            ComposerContext {
                active_response: false,
                session_picker: true,
            },
        );
        if self.input.take_cancel_disarm() {
            self.cancel_armed = None;
        }
        match polled {
            Poll::Pending => {}
            Poll::Ready(Some(Ok((event, binding)))) => self.event(&event, &binding),
            Poll::Ready(None | Some(Err(_))) => self.stop(AskCommandOutcome::OperationalFailure),
        }
    }

    fn event(&mut self, event: &ComposerEvent, binding: &InputBinding) {
        if !matches!(event, ComposerEvent::CancelRequested) {
            self.cancel_armed = None;
        }
        match event {
            ComposerEvent::ExitRequested => self.stop(AskCommandOutcome::Completed),
            ComposerEvent::CancelRequested => {
                let now = Instant::now();
                if self
                    .cancel_armed
                    .is_some_and(|armed| now.duration_since(armed) < Duration::from_secs(3))
                {
                    self.stop(AskCommandOutcome::Completed);
                } else {
                    self.cancel_armed = Some(now);
                    let _ = self.picker.query("");
                }
            }
            _ if self.pending.is_some() => {}
            _ => self.picker_event(event, binding),
        }
    }

    fn picker_event(&mut self, event: &ComposerEvent, binding: &InputBinding) {
        let Some((generation, revision)) = binding.picker_view() else {
            let _ = self.input.restore_picker_query(self.picker.current_query());
            return;
        };
        if self
            .picker
            .identity()
            .is_none_or(|(current, _)| current != generation)
        {
            let _ = self.input.restore_picker_query(self.picker.current_query());
            return;
        }
        match event {
            ComposerEvent::Submit(_) => {
                let Some(revision) = revision else { return };
                if let Selection::Session(target) = self.picker.select(generation, revision) {
                    self.open(NativeInteractiveInitialSession::Resume(
                        NativeResumeTarget::Observed(target),
                    ));
                }
            }
            ComposerEvent::EscapeRequested => {
                self.picker.close();
                self.input.reset_raw_draft();
                self.open(NativeInteractiveInitialSession::Fresh);
            }
            ComposerEvent::Changed => {
                if let Some((query, _)) = self.input.raw_draft() {
                    let _ = self.picker.query(query);
                }
            }
            ComposerEvent::PickerPrevious => self.picker.move_selection(false),
            ComposerEvent::PickerNext => self.picker.move_selection(true),
            ComposerEvent::PickerToggleScope => self.picker.toggle_scope(),
            _ => {}
        }
    }

    fn poll_output(&mut self, cx: &mut Context<'_>) {
        if self.in_flight.is_some() {
            match self.output.acknowledgements.poll_recv(cx) {
                Poll::Pending => return,
                Poll::Ready(Some(OutputAcknowledgement::Succeeded)) => {
                    if let Some(InFlight::Flush {
                        confirm:
                            Some(InputBinding::Picker {
                                generation,
                                revision,
                            }),
                        ..
                    }) = self.in_flight.take()
                    {
                        self.picker.acknowledge(generation, revision);
                    }
                }
                Poll::Ready(Some(OutputAcknowledgement::Failed) | None) => {
                    self.stop(AskCommandOutcome::OutputFailure);
                    return;
                }
            }
        }
        if self.render.is_none() {
            self.prepare_render();
        }
        match next_render_work(&mut self.render) {
            Ok(Some((work, in_flight))) => {
                if self.output.work.try_send(work).is_err() {
                    self.stop(AskCommandOutcome::OutputFailure);
                } else {
                    self.in_flight = Some(in_flight);
                    cx.waker().wake_by_ref();
                }
            }
            Err(()) => self.stop(AskCommandOutcome::OperationalFailure),
            Ok(None) => {}
        }
    }

    fn prepare_render(&mut self) {
        let frame = self.picker.render(
            self.dimensions.columns().get(),
            self.dimensions.rows().get(),
            wall_clock_ms().unwrap_or(0),
        );
        if frame.is_none() && self.picker.is_open() {
            return;
        }
        let mut bytes = Vec::new();
        if let Some(height) = self.menu_height.take() {
            bytes.extend(clear_menu(height));
        }
        let confirm = frame.map(|frame| {
            self.menu_height = Some(frame.height);
            bytes.extend(frame.bytes);
            InputBinding::Picker {
                generation: frame.generation,
                revision: frame.revision,
            }
        });
        if !bytes.is_empty() {
            self.render = Some(Render {
                bytes,
                offset: 0,
                history: false,
                clear_row: false,
                confirm,
                receipt: None,
                model_text: false,
            });
        }
    }

    pub fn into_result(
        mut self,
        inbox: NativeInteractivePromptInbox,
    ) -> Result<Result<Driver, FinalPresentation>, ()> {
        if let Some(outcome) = self.stopped {
            return Ok(Err(FinalPresentation::startup(
                self.output,
                self.render,
                self.in_flight,
                outcome,
                self.signal,
                self.grace,
                self.menu_height,
            )));
        }
        let owner = self.owner.take().ok_or(())?;
        self.picker.set_current(owner.runtime().id());
        let mut driver =
            Driver::from_lines(owner, self.input, inbox, self.output)?.with_history(self.replay);
        driver.frontend = Some(Frontend {
            columns: self.dimensions.columns().get(),
            rows: self.dimensions.rows().get(),
            menu_height: None,
            dirty: true,
            visible: false,
            cancel_armed: None,
            resize: Some(self.resize),
        });
        driver.picker = Some(self.picker);
        Ok(Ok(driver))
    }
}

pub(super) fn clear_menu(height: u16) -> Vec<u8> {
    if height == 0 {
        b"\r\x1b[J".to_vec()
    } else {
        format!("\r\x1b[{height}A\x1b[J").into_bytes()
    }
}
