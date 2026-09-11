use super::{
    Driver, InputBinding, NativeInteractiveOutcome, Render, composer::ComposerEvent,
    picker::Selection,
};
use machine_god_native::{
    NativeInteractiveError, NativeInteractiveTransition, NativeResumeTarget,
    NativeSessionCatalogScope, NativeSessionResumeErrorKind,
};

impl Driver {
    pub(super) fn picker_open(&self) -> bool {
        self.picker
            .as_ref()
            .is_some_and(super::picker::Picker::is_open)
    }

    pub(super) fn open_picker(&mut self, scope: NativeSessionCatalogScope) {
        let status = self.owner.runtime().status();
        if self.frontend.is_none() || self.picker.is_none() {
            self.note(b"\n[session picker requires an interactive terminal]\n");
        } else if status.active
            || status.queued_jobs != 0
            || !self.scope_active
            || self.modal.is_some()
            || self.picker_request.is_some()
        {
            self.note(b"\n[finish the active response before opening sessions]\n");
        } else if self
            .input
            .original_draft()
            .is_some_and(|(text, _)| !text.is_empty())
        {
            self.note(b"\n[clear the draft before opening sessions]\n");
        } else {
            self.input.reset_raw_draft();
            self.reset_skills();
            self.picker.as_mut().expect("configured picker").open(scope);
        }
    }

    pub(super) fn close_picker(&mut self) {
        if let Some(picker) = &mut self.picker {
            picker.close();
        }
        self.input.reset_raw_draft();
        self.reset_skills();
        if let Some(frontend) = &mut self.frontend {
            frontend.dirty = true;
        }
    }

    /// Returns whether the active picker consumed the event. Only a selection
    /// tied to its exact acknowledged frame can request a native transition.
    pub(super) fn picker_event(
        &mut self,
        event: &ComposerEvent,
        binding: &InputBinding,
        now_ms: i64,
    ) -> bool {
        if !self.picker_open() {
            return false;
        }
        if matches!(
            event,
            ComposerEvent::ExitRequested | ComposerEvent::CancelRequested
        ) {
            if matches!(event, ComposerEvent::CancelRequested) {
                let _ = self.picker.as_mut().expect("open picker").query("");
            }
            return false;
        }
        let picker = self.picker.as_mut().expect("open picker");
        let Some((current, _)) = picker.identity() else {
            return false;
        };
        let Some((generation, revision)) = binding.picker_view() else {
            let _ = self.input.restore_picker_query(picker.current_query());
            return true;
        };
        if generation != current {
            let _ = self.input.restore_picker_query(picker.current_query());
            return true;
        }
        match event {
            ComposerEvent::Submit(_) => {
                let Some(revision) = revision else {
                    return true;
                };
                let status = self.owner.runtime().status();
                if status.active || status.queued_jobs != 0 {
                    picker
                        .selection_failed("Finish the active response before selecting a session");
                    return true;
                }
                if let Selection::Session(target) = picker.select(generation, revision) {
                    match self.owner.request_transition(
                        NativeInteractiveTransition::Resume(NativeResumeTarget::Observed(target)),
                        now_ms,
                    ) {
                        Ok(receipt) => {
                            self.picker_request = Some(receipt.id);
                            self.inbox.deactivate();
                            self.scope_active = false;
                            self.modal.take();
                            self.input.reset_raw_draft();
                            self.reset_skills();
                        }
                        Err(_) => picker.selection_failed("Session transition is busy; try again"),
                    }
                }
            }
            ComposerEvent::Changed => {
                if let Some((query, _)) = self.input.raw_draft() {
                    let _ = picker.query(query);
                }
            }
            ComposerEvent::PickerPrevious => picker.move_selection(false),
            ComposerEvent::PickerNext => picker.move_selection(true),
            ComposerEvent::PickerToggleScope => picker.toggle_scope(),
            ComposerEvent::EscapeRequested => self.close_picker(),
            ComposerEvent::InputError(_) => {
                self.note(b"\n[picker input rejected; query retained]\n");
            }
            _ => {}
        }
        true
    }

    pub(super) fn picker_outcome(&mut self, outcome: &NativeInteractiveOutcome) {
        match outcome {
            NativeInteractiveOutcome::Transition(receipt) => {
                if self.picker_request == Some(receipt.request) {
                    self.picker_request = None;
                }
                if let Some(picker) = &mut self.picker {
                    picker.set_current(self.owner.runtime().id());
                }
                self.input.reset_raw_draft();
            }
            NativeInteractiveOutcome::Rejected { request, error, .. }
                if self.picker_request == Some(*request) =>
            {
                self.picker_request = None;
                if let Some(picker) = &mut self.picker {
                    picker.selection_failed(resume_failure(error));
                }
            }
            NativeInteractiveOutcome::Indeterminate { request, .. }
            | NativeInteractiveOutcome::Superseded { request, .. }
                if self.picker_request == Some(*request) =>
            {
                self.picker_request = None;
                self.close_picker();
            }
            _ => {}
        }
    }

    pub(super) fn picker_binding(&self) -> Option<InputBinding> {
        self.picker.as_ref()?.input_binding()
    }

    pub(super) fn acknowledge_picker(&mut self, binding: &InputBinding) {
        if let InputBinding::Picker {
            generation,
            revision,
        } = binding
            && let Some(picker) = &mut self.picker
        {
            picker.acknowledge(*generation, *revision);
        }
    }

    pub(super) fn prepare_picker_render(&mut self) -> bool {
        let Some(picker) = &mut self.picker else {
            return false;
        };
        if !picker.is_open() {
            return false;
        }
        let Some(frontend) = &mut self.frontend else {
            return false;
        };
        if let Some(frame) = picker.render(
            frontend.columns,
            frontend.rows,
            super::wall_clock_ms().unwrap_or(0),
        ) {
            frontend.menu_height = Some(frame.height);
            self.render = Some(Render {
                bytes: frame.bytes,
                offset: 0,
                history: false,
                clear_row: true,
                confirm: Some(InputBinding::Picker {
                    generation: frame.generation,
                    revision: frame.revision,
                }),
                receipt: None,
                model_text: false,
            });
        }
        true
    }
}

pub(super) fn resume_failure(error: &NativeInteractiveError) -> &'static str {
    match error {
        NativeInteractiveError::Resume(error) => match error.kind() {
            NativeSessionResumeErrorKind::Busy => "Session is open elsewhere; try again",
            NativeSessionResumeErrorKind::Conflict => {
                "Session changed; reopen the picker to refresh"
            }
            NativeSessionResumeErrorKind::NotFound => "Session is no longer available",
            _ => "Selected session could not be opened",
        },
        _ => "Selected session could not be opened",
    }
}
