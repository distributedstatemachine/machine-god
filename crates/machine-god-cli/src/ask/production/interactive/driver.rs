//! Fair, bounded polling. No output acknowledgement owns native progress.

use super::input_lines::LineError;
use super::{
    AskCommandOutcome, AskSignals, Context, Driver, InFlight, InputBinding, Modal,
    NativeInteractiveControlOutcome, NativeInteractiveCopyOutcome, NativeInteractiveOutcome,
    OutputAcknowledgement, OutputWork, Poll, ReceiptKind, Render, SIGNAL_OUTPUT_GRACE,
    TurnDriveResult, principal, wall_clock_ms,
};
use machine_god_core::{EngineEvent, ModelEvent, TurnEvent};
use machine_god_native::{
    NativeInteractiveControlReceipt, NativeModelPreferencePersistence, NativeTerminalResetOutcome,
};
use std::fmt::Write;
use std::future::Future;

const OUTPUT_CHUNK_BYTES: usize = 4096;

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;

impl Driver {
    pub(super) fn poll(
        &mut self,
        cx: &mut Context<'_>,
        signals: &mut AskSignals,
    ) -> Poll<TurnDriveResult> {
        self.poll_signals(cx, signals);
        if self.output.poll_tape(cx).is_err() {
            self.output_failed = true;
            if !self.shutting_down {
                self.shutdown();
            }
        }
        let now_ms = if let Ok(now) = wall_clock_ms() {
            now
        } else {
            self.native_failed = true;
            if !self.shutting_down {
                self.shutdown();
            }
            // Shutdown does not create session timestamps or fresh work.
            0
        };
        // Accepted native saves, turn finalization, terminal handoff and shutdown
        // continue even when every presentation slot is occupied.
        let _ = self.owner.poll_progress(cx, now_ms);
        self.observe_outcomes();
        if self.owner.shutdown_error().is_some() {
            self.native_failed = true;
            if !self.shutting_down {
                self.shutdown();
            }
        }
        self.poll_modal(cx);
        self.poll_resize(cx);
        if let Some(picker) = &mut self.picker {
            picker.poll(cx);
        }
        if !self.shutting_down && !self.input_ended {
            self.poll_input(cx, now_ms);
        }
        self.poll_output(cx);
        self.poll_output_grace(cx);

        let native_settled = self.owner.is_closed() || self.owner.shutdown_error().is_some();
        if self.shutting_down && native_settled && !self.owner.has_pending_copy() {
            return Poll::Ready(TurnDriveResult {
                outcome: self.signal.map_or_else(
                    || {
                        if self.output_failed {
                            AskCommandOutcome::OutputFailure
                        } else if self.native_failed {
                            AskCommandOutcome::OperationalFailure
                        } else {
                            AskCommandOutcome::Completed
                        }
                    },
                    super::AskSignal::outcome,
                ),
                stalled_output_after_signal: self.stalled_output,
            });
        }
        Poll::Pending
    }

    pub(super) fn into_presentation(mut self, result: TurnDriveResult) -> FinalPresentation {
        // Closed native ownership can still retain a second typed receipt behind
        // our bounded presentation slot. Transfer both, never flatten them into
        // a success flag or discard a late failure on the way to host cleanup.
        let mut outcomes = std::collections::VecDeque::with_capacity(2);
        outcomes.extend(self.outcome.take());
        outcomes.extend(self.owner.take_outcome());
        let mut controls = std::collections::VecDeque::with_capacity(2);
        controls.extend(self.control_outcome.take());
        controls.extend(self.owner.take_control_outcome());
        let mut copies = std::collections::VecDeque::with_capacity(2);
        copies.extend(self.copy_outcome.take());
        copies.extend(self.owner.take_copy_outcome());
        let native_failed = self.native_failed
            || outcomes.iter().any(|outcome| {
                outcome_failed_for_picker(outcome, self.picker_rejection)
                    && outcome_failed_for_picker(outcome, self.picker_request)
            })
            || controls.iter().any(control_failed);
        FinalPresentation {
            output: self.output,
            render: self.render,
            in_flight: self.in_flight,
            notice: self.notice,
            outcomes,
            controls,
            copies,
            result,
            native_failed,
            output_failed: self.output_failed,
            signal: self.signal,
            grace: self.grace,
            final_flush_sent: self.final_flush_sent && self.frontend.is_none(),
            terminal_cleanup: self.frontend.is_some().then(|| {
                terminal_cleanup(
                    self.frontend
                        .as_ref()
                        .and_then(|frontend| frontend.menu_height),
                )
            }),
        }
        // The remaining owner, inbox and input fields drop here. FinalPresentation
        // has no lifetime vote in the conversation host or input worker scope.
    }

    fn poll_signals(&mut self, cx: &mut Context<'_>, signals: &mut AskSignals) {
        if self.signal.is_none()
            && let Poll::Ready(signal) = signals.poll_signal(cx)
        {
            self.signal = Some(signal);
            if signal == super::AskSignal::Interrupt
                && let Some(tape) = &mut self.output.tape
            {
                tape.sigint();
            }
            self.grace = Some(Box::pin(tokio::time::sleep(SIGNAL_OUTPUT_GRACE)));
            self.shutdown();
        }
    }

    fn observe_outcomes(&mut self) {
        if self.copy_outcome.is_none() {
            self.copy_outcome = self.owner.take_copy_outcome();
        }
        if self.control_outcome.is_none()
            && let Some(outcome) = self.owner.take_control_outcome()
        {
            self.native_failed |= control_failed(&outcome);
            self.control_outcome = Some(outcome);
        }
        if self.outcome.is_none()
            && let Some(outcome) = self.owner.take_outcome()
        {
            let picker_request = self.picker_request;
            self.picker_outcome(&outcome);
            self.native_failed |= outcome_failed_for_picker(&outcome, picker_request);
            if matches!(&outcome, NativeInteractiveOutcome::Rejected { request, .. } if Some(*request) == picker_request)
            {
                // Keep the request identity until the typed receipt is flushed,
                // including transfer into the native-free shutdown tail.
                self.picker_rejection = picker_request;
            }
            if matches!(&outcome, NativeInteractiveOutcome::Transition(receipt) if !receipt.unchanged)
                && !self.shutting_down
            {
                self.replace_history();
            }
            match &outcome {
                NativeInteractiveOutcome::Transition(_)
                | NativeInteractiveOutcome::Rejected { .. }
                    if !self.shutting_down =>
                {
                    // Superseded is deliberately excluded: its replacement
                    // transition still owns the quiescing session.
                    if self.inbox.activate(principal(&self.owner)).is_err() {
                        self.native_failed = true;
                        self.shutdown();
                    } else {
                        self.scope_active = true;
                        self.modal.take();
                    }
                }
                NativeInteractiveOutcome::Indeterminate { .. } => {
                    self.inbox.deactivate();
                    self.scope_active = false;
                    self.modal.take();
                }
                NativeInteractiveOutcome::Shutdown => self.shutdown(),
                _ => {}
            }
            self.outcome = Some(outcome);
        }
    }

    fn poll_modal(&mut self, cx: &mut Context<'_>) {
        if self.shutting_down || !self.scope_active {
            return;
        }
        match self.inbox.poll_prompt(cx) {
            Poll::Ready(Some(view)) => {
                if self
                    .modal
                    .as_ref()
                    .is_none_or(|modal| modal.view.token() != view.token())
                {
                    self.modal = Some(Modal::new(view));
                }
            }
            Poll::Pending => {
                self.modal.take();
            }
            Poll::Ready(None) => {
                self.native_failed = true;
                self.shutdown();
            }
        }
    }

    fn poll_input(&mut self, cx: &mut Context<'_>, now_ms: i64) {
        let binding = self.picker_binding().unwrap_or_else(|| {
            self.modal.as_ref().map_or_else(
                || {
                    if self.scope_active {
                        InputBinding::Command
                    } else {
                        InputBinding::AwaitingPrompt
                    }
                },
                Modal::binding,
            )
        });
        if self.frontend.is_some() {
            self.poll_raw_input(cx, binding, now_ms);
            return;
        }
        let tape = &mut self.output.tape;
        match self.input.poll_line_recorded(cx, binding, |bytes| {
            if let Some(tape) = tape {
                tape.stdin(bytes);
            }
        }) {
            Poll::Pending => {}
            Poll::Ready(None) => {
                self.input_ended = true;
                self.shutdown();
            }
            Poll::Ready(Some(Err(LineError::Input(error)))) => {
                self.note(
                    if error == machine_god_native::NativeInteractiveInputError::Cancelled {
                        b"\n[input cancelled]\n"
                    } else {
                        b"\n[input unavailable]\n"
                    },
                );
                self.input_ended = true;
                self.native_failed = true;
                self.shutdown();
            }
            Poll::Ready(Some(Err(LineError::Frame(error)))) => {
                use super::framing::InteractiveInputFrameError;
                self.note(match error {
                    InteractiveInputFrameError::TooLong => {
                        b"\n[input rejected: oversized line]\n> "
                    }
                    InteractiveInputFrameError::InvalidUtf8 => {
                        b"\n[input rejected: invalid UTF-8]\n> "
                    }
                    InteractiveInputFrameError::ContainsNul => b"\n[input rejected: NUL byte]\n> ",
                });
            }
            Poll::Ready(Some(Ok((line, binding)))) => self.line(&line, &binding, now_ms),
        }
    }

    fn poll_resize(&mut self, cx: &mut Context<'_>) {
        if self.shutting_down {
            return;
        }
        let Some(frontend) = &mut self.frontend else {
            return;
        };
        let Some(resize) = &mut frontend.resize else {
            return;
        };
        match resize.poll(cx) {
            Poll::Ready(Ok(dimensions)) => {
                frontend.columns = dimensions.columns().get();
                frontend.rows = dimensions.rows().get();
                if let Some(tape) = &mut self.output.tape {
                    tape.resize(frontend.columns, frontend.rows);
                }
                frontend.dirty = true;
                if let Some(picker) = &mut self.picker {
                    picker.resize(frontend.rows);
                }
            }
            Poll::Ready(Err(())) => {
                self.native_failed = true;
                self.shutdown();
            }
            Poll::Pending => {}
        }
    }

    fn poll_raw_input(&mut self, cx: &mut Context<'_>, binding: InputBinding, now_ms: i64) {
        use super::composer::{ComposerContext, ComposerEvent};
        let status = self.owner.runtime().status();
        let context = ComposerContext {
            active_response: status.active || status.queued_jobs != 0,
            session_picker: self.picker_open(),
        };
        let tape = &mut self.output.tape;
        let polled = self
            .input
            .poll_event_recorded(cx, binding, context, |bytes| {
                if let Some(tape) = tape {
                    tape.stdin(bytes);
                }
            });
        if self.input.take_cancel_disarm() {
            self.frontend.as_mut().expect("raw frontend").cancel_armed = None;
        }
        let event = match polled {
            Poll::Pending => return,
            Poll::Ready(None) => {
                // Physical EOF/hangup is not the empty-idle Ctrl-D gesture.
                self.input_ended = true;
                self.native_failed = true;
                self.shutdown();
                return;
            }
            Poll::Ready(Some(Err(_))) => {
                self.note(b"\n[input unavailable]\n");
                self.input_ended = true;
                self.native_failed = true;
                self.shutdown();
                return;
            }
            Poll::Ready(Some(Ok(event))) => event,
        };
        let frontend = self.frontend.as_mut().expect("raw frontend");
        frontend.dirty = true;
        if !matches!(event.0, ComposerEvent::CancelRequested) {
            frontend.cancel_armed = None;
        }
        if self.picker_event(&event.0, &event.1, now_ms) {
            return;
        }
        let frontend = self.frontend.as_mut().expect("raw frontend");
        match event {
            (ComposerEvent::Submit(line), binding) => self.line(&line, &binding, now_ms),
            (ComposerEvent::ExitRequested, _) => self.shutdown(),
            (ComposerEvent::CancelRequested, _) => {
                let now = std::time::Instant::now();
                if frontend.cancel_armed.is_some_and(|armed| {
                    now.duration_since(armed) < std::time::Duration::from_secs(3)
                }) {
                    self.shutdown();
                } else {
                    frontend.cancel_armed = Some(now);
                    self.owner.request_cancel();
                    self.note(b"\n[press Ctrl-C again within 3 seconds to exit]\n");
                }
            }
            (ComposerEvent::InputError(_), _) => self.note(b"\n[input rejected; draft retained]\n"),
            (ComposerEvent::SessionPickerRequested, InputBinding::Command) => {
                self.open_picker(machine_god_native::NativeSessionCatalogScope::All);
            }
            _ => {}
        }
    }

    fn line(&mut self, line: &str, binding: &InputBinding, now_ms: i64) {
        if matches!(line.trim(), "/quit" | "/exit" | "/cancel") {
            self.command(line.trim(), now_ms);
            return;
        }
        if let Some(modal) = &mut self.modal {
            match modal.answer(line, binding) {
                Ok(Some(response)) => {
                    let token = modal.view.token().clone();
                    if self.inbox.reply(&token, response).is_err() {
                        self.note(b"\n[prompt expired; response ignored]\n> ");
                    }
                    self.modal.take();
                }
                Ok(None) => {} // The next page needs its own completed flush.
                Err(()) => self.note(b"\n[response does not match the displayed prompt]\n> "),
            }
        } else if matches!(binding, InputBinding::Command) && self.scope_active {
            self.command(line, now_ms);
        } else {
            self.note(b"\n[stale prompt input ignored]\n> ");
        }
    }

    fn poll_output(&mut self, cx: &mut Context<'_>) {
        if self.output_failed || self.stalled_output {
            return;
        }
        if self.in_flight.is_some() {
            match self.output.poll_acknowledgement(cx) {
                Poll::Pending => return,
                Poll::Ready(Some(OutputAcknowledgement::Succeeded)) => {
                    self.acknowledge();
                    // Releasing our receipt slot may expose a separately held
                    // native receipt even when there is no next output item.
                    cx.waker().wake_by_ref();
                }
                Poll::Ready(Some(_) | None) => {
                    self.output_failed = true;
                    self.shutdown();
                    return;
                }
            }
        }
        if self.render.is_none() {
            self.prepare_render(cx);
        }
        match next_render_work(&mut self.render) {
            Ok(Some((work, in_flight))) => self.send(work, in_flight, cx),
            Err(()) => {
                self.native_failed = true;
                self.output_failed = true;
                self.shutdown();
            }
            Ok(None) if self.shutting_down && !self.final_flush_sent => {
                self.final_flush_sent = true;
                self.send(
                    OutputWork::Flush,
                    InFlight::Flush {
                        confirm: None,
                        receipt: None,
                    },
                    cx,
                );
            }
            Ok(None) => {}
        }
    }

    fn send(&mut self, work: OutputWork, in_flight: InFlight, cx: &mut Context<'_>) {
        // Exactly one in-flight item: an acknowledgement frees the one-slot
        // channel before another work item can be submitted.
        if self.output.work.try_send(work).is_err() {
            self.output_failed = true;
            self.shutdown();
        } else {
            self.in_flight = Some(in_flight);
            // Register acknowledgement readiness next poll, without polling a
            // second acknowledgement/work item in this bounded step.
            cx.waker().wake_by_ref();
        }
    }

    fn acknowledge(&mut self) {
        if let Some(InFlight::Flush { confirm, receipt }) = self.in_flight.take() {
            if let Some(binding) = &confirm {
                self.acknowledge_picker(binding);
            }
            if let (Some(binding), Some(modal)) = (confirm, &mut self.modal)
                && modal.presentation_binding() == binding
                && self.scope_active
                && !self.shutting_down
            {
                modal.displayed = true;
            }
            match receipt {
                Some(ReceiptKind::Outcome) => {
                    self.outcome.take();
                    self.picker_rejection.take();
                }
                Some(ReceiptKind::Control) => {
                    self.control_outcome.take();
                }
                Some(ReceiptKind::Copy) => {
                    self.copy_outcome.take();
                }
                None => {}
            }
        }
    }

    fn prepare_render(&mut self, cx: &mut Context<'_>) {
        let previous_menu = self
            .frontend
            .as_mut()
            .and_then(|frontend| frontend.menu_height.take());
        self.prepare_content_render(cx);
        if let Some(height) = previous_menu {
            if let Some(render) = &mut self.render {
                let prefix = if height == 0 {
                    "\r\x1b[J".to_owned()
                } else {
                    format!("\r\x1b[{height}A\x1b[J")
                };
                render.bytes.splice(..0, prefix.bytes());
                if !matches!(render.confirm, Some(InputBinding::Picker { .. }))
                    && let Some(picker) = &mut self.picker
                {
                    picker.redraw();
                }
            } else if self.picker_open() {
                self.frontend.as_mut().expect("menu frontend").menu_height = Some(height);
                return;
            } else {
                let bytes = if height == 0 {
                    "\r\x1b[J".to_owned()
                } else {
                    format!("\r\x1b[{height}A\x1b[J")
                };
                self.render = Some(Render {
                    bytes: bytes.into_bytes(),
                    offset: 0,
                    history: false,
                    clear_row: false,
                    confirm: None,
                    receipt: None,
                    model_text: false,
                });
            }
        }
        let picker_open = self.picker_open();
        let Some(frontend) = &mut self.frontend else {
            return;
        };
        if let Some(render) = &mut self.render {
            render.clear_row |= std::mem::take(&mut frontend.visible);
            frontend.dirty = true;
            return;
        }
        // Keep streamed model text contiguous. Draft projection resumes when
        // the stream settles, or while a human prompt owns input.
        if !frontend.dirty
            || self.shutting_down
            || self.history.is_some()
            || picker_open
            || (self.owner.runtime().status().active && self.modal.is_none())
        {
            return;
        }
        let Some((text, cursor)) = self.input.raw_draft() else {
            return;
        };
        if let Ok(bytes) = super::composer_view::render(text, cursor, frontend.columns) {
            frontend.dirty = false;
            frontend.visible = true;
            self.render = Some(Render {
                history: false,
                clear_row: false,
                bytes,
                offset: 0,
                confirm: None,
                receipt: None,
                model_text: false,
            });
        } else {
            self.native_failed = true;
            self.shutdown();
        }
    }

    fn prepare_content_render(&mut self, cx: &mut Context<'_>) {
        let (bytes, confirm, receipt) = if let Some(outcome) = &self.outcome {
            (render_outcome(outcome), None, Some(ReceiptKind::Outcome))
        } else if let Some(outcome) = &self.control_outcome {
            (render_control(outcome), None, Some(ReceiptKind::Control))
        } else if let Some(outcome) = &self.copy_outcome {
            (
                super::clipboard::render(outcome),
                None,
                Some(ReceiptKind::Copy),
            )
        } else if let Some(notice) = self.notice.take() {
            (Ok(notice), None, None)
        } else if let Some(modal) = &self.modal {
            if modal.displayed {
                return;
            }
            (modal.render(), Some(modal.presentation_binding()), None)
        } else if !self.shutting_down {
            if self.prepare_picker_render() {
                return;
            }
            if self.prepare_history_render(cx) {
                return;
            }
            let Some(event) = self.owner.take_presentation() else {
                return;
            };
            match event.payload {
                TurnEvent::Model {
                    event: ModelEvent::TextDelta { text },
                } => {
                    self.render = Some(Render {
                        history: false,
                        clear_row: false,
                        bytes: text.into_bytes(),
                        model_text: true,
                        offset: 0,
                        confirm: None,
                        receipt: None,
                    });
                    return;
                }
                _ => return,
            }
        } else {
            return;
        };
        if let Ok(bytes) = bytes {
            self.render = Some(Render {
                history: false,
                clear_row: false,
                bytes,
                model_text: false,
                offset: 0,
                confirm,
                receipt,
            });
        } else {
            self.native_failed = true;
            self.shutdown();
            self.render = Some(Render {
                history: false,
                clear_row: false,
                bytes: b"\n[presentation exceeded its bound; session stopping]\n".to_vec(),
                model_text: false,
                offset: 0,
                confirm: None,
                receipt,
            });
        }
    }

    fn prepare_history_render(&mut self, cx: &mut Context<'_>) -> bool {
        use super::history_view::HistoryViewStep;
        let Some(history) = &mut self.history else {
            return false;
        };
        match history.next_chunk() {
            HistoryViewStep::Chunk(bytes) => {
                self.render = Some(Render {
                    history: true,
                    clear_row: false,
                    bytes,
                    offset: 0,
                    confirm: None,
                    receipt: None,
                    model_text: false,
                });
                true
            }
            HistoryViewStep::Progress => {
                cx.waker().wake_by_ref();
                true
            }
            HistoryViewStep::Done => {
                self.history.take();
                false
            }
        }
    }

    fn poll_output_grace(&mut self, cx: &mut Context<'_>) {
        let Some(grace) = &mut self.grace else {
            return;
        };
        if grace.as_mut().poll(cx).is_ready() {
            // Expiration never stops native cleanup. It only releases output
            // presentation after the outer signal guardian can enforce exit.
            if self.in_flight.is_some() || self.render.is_some() || !self.final_flush_sent {
                self.stalled_output = true;
            }
        }
    }
}

/// Conversation-free tail. The host joins input and its terminal workers before
/// this value is polled. Its independent tape lane records the final output;
/// neither stdout nor tape acknowledgements can postpone conversation cleanup.
pub(super) struct FinalPresentation {
    output: super::OutputBridge,
    render: Option<Render>,
    in_flight: Option<InFlight>,
    notice: Option<Vec<u8>>,
    outcomes: std::collections::VecDeque<NativeInteractiveOutcome>,
    controls: std::collections::VecDeque<NativeInteractiveControlOutcome>,
    copies: std::collections::VecDeque<NativeInteractiveCopyOutcome>,
    result: TurnDriveResult,
    native_failed: bool,
    output_failed: bool,
    signal: Option<super::AskSignal>,
    grace: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
    final_flush_sent: bool,
    terminal_cleanup: Option<Vec<u8>>,
}

impl FinalPresentation {
    pub(super) fn startup(
        output: super::OutputBridge,
        render: Option<Render>,
        in_flight: Option<InFlight>,
        outcome: AskCommandOutcome,
        signal: Option<super::AskSignal>,
        grace: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
        menu_height: Option<u16>,
    ) -> Self {
        Self {
            output,
            render,
            in_flight,
            notice: None,
            outcomes: std::collections::VecDeque::new(),
            controls: std::collections::VecDeque::new(),
            copies: std::collections::VecDeque::new(),
            result: TurnDriveResult {
                outcome,
                stalled_output_after_signal: false,
            },
            native_failed: outcome == AskCommandOutcome::OperationalFailure,
            output_failed: outcome == AskCommandOutcome::OutputFailure,
            signal,
            grace,
            final_flush_sent: false,
            terminal_cleanup: Some(terminal_cleanup(menu_height)),
        }
    }
    pub(super) fn poll(
        &mut self,
        cx: &mut Context<'_>,
        signals: &mut AskSignals,
    ) -> Poll<TurnDriveResult> {
        if self.signal.is_none()
            && let Poll::Ready(signal) = signals.poll_signal(cx)
        {
            self.signal = Some(signal);
            if signal == super::AskSignal::Interrupt
                && let Some(tape) = &mut self.output.tape
            {
                tape.sigint();
            }
            self.grace = Some(Box::pin(tokio::time::sleep(SIGNAL_OUTPUT_GRACE)));
        }
        if !self.output_failed && !self.result.stalled_output_after_signal {
            self.output_failed |= self.output.poll_tape(cx).is_err();
            self.poll_output(cx);
        }
        let output_done = self.final_flush_sent && self.in_flight.is_none();
        let mut tape_done = false;
        if output_done && !self.output_failed && !self.result.stalled_output_after_signal {
            match self.output.poll_finish_tape(cx) {
                Poll::Ready(Ok(())) => tape_done = true,
                Poll::Ready(Err(())) => self.output_failed = true,
                Poll::Pending => {}
            }
        }
        if let Some(grace) = &mut self.grace
            && grace.as_mut().poll(cx).is_ready()
            && (!output_done || !tape_done)
        {
            self.result.stalled_output_after_signal = true;
        }
        if self.output_failed
            || self.result.stalled_output_after_signal
            || (output_done && tape_done)
        {
            if self.output_failed || self.result.stalled_output_after_signal {
                self.output.abort_tape();
            }
            self.result.outcome = self.signal.map_or_else(
                || {
                    if self.output_failed {
                        AskCommandOutcome::OutputFailure
                    } else if self.native_failed {
                        AskCommandOutcome::OperationalFailure
                    } else {
                        self.result.outcome
                    }
                },
                super::AskSignal::outcome,
            );
            Poll::Ready(self.result)
        } else {
            Poll::Pending
        }
    }

    fn poll_output(&mut self, cx: &mut Context<'_>) {
        if self.in_flight.is_some() {
            match self.output.poll_acknowledgement(cx) {
                Poll::Pending => return,
                Poll::Ready(Some(OutputAcknowledgement::Succeeded)) => {
                    if let Some(InFlight::Flush { receipt, .. }) = self.in_flight.take() {
                        match receipt {
                            Some(ReceiptKind::Outcome) => {
                                self.outcomes.pop_front();
                            }
                            Some(ReceiptKind::Control) => {
                                self.controls.pop_front();
                            }
                            Some(ReceiptKind::Copy) => {
                                self.copies.pop_front();
                            }
                            None => {}
                        }
                    }
                }
                Poll::Ready(Some(_) | None) => {
                    self.output_failed = true;
                    return;
                }
            }
        }
        if self.render.is_none() {
            let next = if let Some(outcome) = self.outcomes.front() {
                Some((render_outcome(outcome), Some(ReceiptKind::Outcome)))
            } else if let Some(control) = self.controls.front() {
                Some((render_control(control), Some(ReceiptKind::Control)))
            } else if let Some(copy) = self.copies.front() {
                Some((super::clipboard::render(copy), Some(ReceiptKind::Copy)))
            } else if let Some(notice) = self.notice.take() {
                Some((Ok(notice), None))
            } else {
                self.terminal_cleanup
                    .take()
                    .map(|cleanup| (Ok(cleanup), None))
            };
            if let Some((bytes, receipt)) = next {
                if let Ok(bytes) = bytes {
                    self.render = Some(Render {
                        history: false,
                        clear_row: false,
                        bytes,
                        model_text: false,
                        offset: 0,
                        confirm: None,
                        receipt,
                    });
                } else {
                    self.output_failed = true;
                    return;
                }
            }
        }
        let work = match next_render_work(&mut self.render) {
            Ok(Some(work)) => Some(work),
            Err(()) => {
                self.output_failed = true;
                return;
            }
            Ok(None) if !self.final_flush_sent => {
                self.final_flush_sent = true;
                Some((
                    OutputWork::Flush,
                    InFlight::Flush {
                        confirm: None,
                        receipt: None,
                    },
                ))
            }
            Ok(None) => None,
        };
        if let Some((work, in_flight)) = work {
            if self.output.work.try_send(work).is_err() {
                self.output_failed = true;
            } else {
                self.in_flight = Some(in_flight);
                cx.waker().wake_by_ref();
            }
        }
    }
}

fn terminal_cleanup(menu_height: Option<u16>) -> Vec<u8> {
    let mut bytes = menu_height.map_or_else(Vec::new, super::picker_startup::clear_menu);
    bytes.extend_from_slice(b"\r\x1b[2K\x1b[?2004l\n");
    bytes
}

pub(super) fn next_render_work(
    render: &mut Option<Render>,
) -> Result<Option<(OutputWork, InFlight)>, ()> {
    let Some(current) = render else {
        return Ok(None);
    };
    if std::mem::take(&mut current.clear_row) {
        return Ok(Some((
            OutputWork::Write(super::composer_view::clear_row().to_vec()),
            InFlight::Bytes,
        )));
    }
    if current.offset < current.bytes.len() {
        let (bytes, consumed) = output_chunk(&current.bytes[current.offset..], current.model_text)?;
        current.offset += consumed;
        Ok(Some((OutputWork::Write(bytes), InFlight::Bytes)))
    } else {
        let current = render.take().expect("the current render");
        Ok(Some((
            OutputWork::Flush,
            InFlight::Flush {
                confirm: current.confirm,
                receipt: current.receipt,
            },
        )))
    }
}

fn terminal_failed(event: &EngineEvent) -> bool {
    !matches!(event.payload, TurnEvent::Completed { .. })
}

fn outcome_failed_for_picker(
    outcome: &NativeInteractiveOutcome,
    picker_request: Option<machine_god_native::NativeInteractiveRequestId>,
) -> bool {
    use machine_god_native::{NativeInteractiveError, NativeSessionResumeErrorKind};
    if let NativeInteractiveOutcome::Rejected {
        request,
        error: NativeInteractiveError::Resume(error),
        settled_turn,
        ..
    } = outcome
        && Some(*request) == picker_request
        && matches!(
            error.kind(),
            NativeSessionResumeErrorKind::Busy
                | NativeSessionResumeErrorKind::Conflict
                | NativeSessionResumeErrorKind::NotFound
        )
    {
        return settled_turn.as_ref().is_some_and(terminal_failed);
    }
    outcome_failed(outcome)
}

fn outcome_failed(outcome: &NativeInteractiveOutcome) -> bool {
    match outcome {
        NativeInteractiveOutcome::Turn(result) => result.as_ref().map_or(true, terminal_failed),
        NativeInteractiveOutcome::Transition(receipt) => {
            receipt.settled_turn.as_ref().is_some_and(terminal_failed)
                || receipt.reset.as_ref().is_some_and(|reset| {
                    reset.entries().iter().any(|entry| {
                        entry.outcome() == NativeTerminalResetOutcome::RetainedIndeterminate
                    })
                })
        }
        NativeInteractiveOutcome::Rejected { .. }
        | NativeInteractiveOutcome::Indeterminate { .. } => true,
        NativeInteractiveOutcome::Superseded {
            error,
            settled_turn,
            ..
        } => error.is_some() || settled_turn.as_ref().is_some_and(terminal_failed),
        NativeInteractiveOutcome::Shutdown => false,
    }
}

fn control_failed(outcome: &NativeInteractiveControlOutcome) -> bool {
    outcome.failed()
}

fn output_chunk(bytes: &[u8], model_text: bool) -> Result<(Vec<u8>, usize), ()> {
    if !model_text {
        let end = bytes.len().min(OUTPUT_CHUNK_BYTES);
        return Ok((bytes[..end].to_vec(), end));
    }
    // Every escaped code point uses at most six bytes per source byte. Cut
    // only at UTF-8 boundaries without rescanning the entire retained delta.
    let mut end = bytes.len().min(OUTPUT_CHUNK_BYTES / 6);
    while end > 0 && end < bytes.len() && bytes[end] & 0xc0 == 0x80 {
        end -= 1;
    }
    if end == 0 && !bytes.is_empty() {
        return Err(());
    }
    let text = std::str::from_utf8(&bytes[..end]).map_err(|_| ())?;
    let mut output = crate::ask::production::interactive::bounded_output();
    for character in text.chars() {
        let formatting_control = matches!(character, '\u{061c}' | '\u{200e}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
        if character != '\n' && (character.is_control() || formatting_control) {
            let mut encoded = [0; 4];
            super::presentation::escaped(&mut output, character.encode_utf8(&mut encoded))?;
        } else {
            output.write_char(character).map_err(|_| ())?;
        }
    }
    Ok((output.finish().into_bytes(), end))
}

fn render_outcome(outcome: &NativeInteractiveOutcome) -> Result<Vec<u8>, ()> {
    let mut text = crate::ask::production::interactive::bounded_output();
    match outcome {
        NativeInteractiveOutcome::Turn(result) => {
            text.write_str(
                if result.as_ref().is_ok_and(|event| !terminal_failed(event)) {
                    "\n[turn completed]\n> "
                } else {
                    "\n[turn failed; no automatic retry]\n> "
                },
            )
            .map_err(|_| ())?;
        }
        NativeInteractiveOutcome::Transition(receipt) => {
            write!(
                text,
                "\n[session transition {}: {}]",
                receipt.request.get(),
                if receipt.unchanged {
                    "unchanged"
                } else {
                    "adopted"
                }
            )
            .map_err(|_| ())?;
            if let Some(handoff) = &receipt.handoff {
                write!(text, " [terminals carried: {}]", handoff.transferred()).map_err(|_| ())?;
            }
            if let Some(reset) = &receipt.reset {
                let retained = reset
                    .entries()
                    .iter()
                    .filter(|entry| {
                        entry.outcome() == NativeTerminalResetOutcome::RetainedIndeterminate
                    })
                    .count();
                write!(
                    text,
                    " [terminal results: {}, retained indeterminate: {retained}]",
                    reset.entries().len()
                )
                .map_err(|_| ())?;
            }
            if receipt.settled_turn.as_ref().is_some_and(terminal_failed) {
                text.write_str(" [previous turn failed]").map_err(|_| ())?;
            }
            text.write_str("\n> ").map_err(|_| ())?;
        }
        NativeInteractiveOutcome::Rejected {
            request, candidate, ..
        } => {
            write!(
                text,
                "\n[transition {} rejected; prepared candidate: {}]\n> ",
                request.get(),
                candidate.is_some()
            )
            .map_err(|_| ())?;
        }
        NativeInteractiveOutcome::Superseded {
            request,
            error,
            candidate,
            ..
        } => {
            write!(
                text,
                "\n[transition {} superseded; prepared candidate: {}; preparation error: {}]\n> ",
                request.get(),
                candidate.is_some(),
                error.is_some()
            )
            .map_err(|_| ())?;
        }
        NativeInteractiveOutcome::Indeterminate { request, .. } => {
            write!(
                text,
                "\n[transition {} indeterminate; session fenced; no automatic retry]\n> ",
                request.get()
            )
            .map_err(|_| ())?;
        }
        NativeInteractiveOutcome::Shutdown => {
            text.write_str("\n[session closed]\n").map_err(|_| ())?;
        }
    }
    Ok(text.finish().into_bytes())
}

fn session_save_name(value: &NativeModelPreferencePersistence) -> &'static str {
    match value {
        NativeModelPreferencePersistence::Unchanged => "unchanged",
        NativeModelPreferencePersistence::Deferred => "deferred (not saved)",
        NativeModelPreferencePersistence::Saved { .. } => "saved",
    }
}

pub(super) fn render_control(outcome: &NativeInteractiveControlOutcome) -> Result<Vec<u8>, ()> {
    use machine_god_native::{FileUndoError, FileUndoOutcome, NativeInteractiveControlError};

    if let Ok(NativeInteractiveControlReceipt::Allowlist(receipt)) = &outcome.result {
        return super::allowlist_view::render(outcome.id.get(), receipt);
    }
    if let Ok(NativeInteractiveControlReceipt::Workspace(receipt)) = &outcome.result {
        return crate::workspace::render_control_receipt(
            outcome.id.get(),
            receipt,
            super::MAX_PRESENTATION_OUTPUT_BYTES,
        );
    }

    let mut text = crate::ask::production::interactive::bounded_output();
    write!(text, "\n[control {}: ", outcome.id.get()).map_err(|_| ())?;
    match &outcome.result {
        Err(NativeInteractiveControlError::Undo(FileUndoError::Ambiguous)) => text.write_str(
            "undo outcome uncertain; effects may be partial; recovery artifacts retained; manual inspection required; no automatic retry",
        ),
        Err(NativeInteractiveControlError::Undo(error)) => write!(text, "undo failed: {error}"),
        Err(NativeInteractiveControlError::Allowlist(
            machine_god_native::NativeAllowlistError::Ambiguous
            | machine_god_native::NativeAllowlistError::Config(
                machine_god_native::NativeUserConfigError::CommitAmbiguous,
            ),
        )) => text.write_str("allowlist outcome uncertain; settings may have been saved; authoritative reload required; no automatic retry"),
        Err(NativeInteractiveControlError::Allowlist(_)) => {
            text.write_str("allowlist failed; settings and runtime must be rechecked")
        }
        Err(NativeInteractiveControlError::Workspace(error)) => {
            write!(text, "workspace failed: {}; inspect /workspace before retrying", crate::workspace::control_error(*error))
        }
        Err(_) => text.write_str("failed; publication may require authoritative reload"),
        Ok(NativeInteractiveControlReceipt::Undone(FileUndoOutcome::Empty)) => {
            text.write_str("Nothing to undo.")
        }
        Ok(NativeInteractiveControlReceipt::Undone(FileUndoOutcome::Restored(path))) => {
            text.write_str("Restored ").map_err(|_| ())?;
            super::presentation::escaped(&mut text, path)?;
            Ok(())
        }
        Ok(NativeInteractiveControlReceipt::Undone(FileUndoOutcome::Removed(path))) => {
            text.write_str("Removed ").map_err(|_| ())?;
            super::presentation::escaped(&mut text, path)?;
            text.write_str(" (was newly created)")
        }
        Ok(NativeInteractiveControlReceipt::Renamed(_)) => text.write_str("title saved"),
        Ok(NativeInteractiveControlReceipt::Compacted(changed)) => text.write_str(if *changed {
            "context compacted"
        } else {
            "context unchanged"
        }),
        Ok(NativeInteractiveControlReceipt::Continued(_)) => text.write_str("continuation queued"),
        Ok(NativeInteractiveControlReceipt::ModelSession(value)) => {
            write!(text, "session model {}", session_save_name(value))
        }
        Ok(NativeInteractiveControlReceipt::ModelDefaults(commit)) => {
            let session = commit.session.as_ref().map_or("failed", session_save_name);
            write!(
                text,
                "session model {session}; user defaults {}",
                if commit.user_defaults.is_ok() {
                    "saved"
                } else {
                    "failed"
                }
            )
        }
        Ok(NativeInteractiveControlReceipt::PermissionRuleConfirmed(_)) => {
            text.write_str("permission rule saved")
        }
        Ok(NativeInteractiveControlReceipt::Allowlist(_)) => {
            unreachable!("allowlist uses its separately bounded renderer")
        }
        Ok(NativeInteractiveControlReceipt::Workspace(_)) => {
            unreachable!("workspace uses its separately bounded renderer")
        }
    }
    .map_err(|_| ())?;
    text.write_str("]\n> ").map_err(|_| ())?;
    Ok(text.finish().into_bytes())
}
