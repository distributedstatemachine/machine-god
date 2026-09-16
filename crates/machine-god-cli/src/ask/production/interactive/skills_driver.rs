//! Thin input/output adapter. Native owns draft spans, catalog matching and
//! selection authority; this module owns only editor epochs and acknowledgements.

use super::{Driver, InputBinding, Render, composer::ComposerEvent, principal};
use machine_god_core::BackgroundOutputOwner;
use machine_god_native::{
    NativeInteractiveControl, NativeInteractiveControlId, NativeInteractiveControlOutcome,
    NativeInteractiveControlReceipt, NativeSkillDraftIdentity, NativeSkillFrameIdentity,
    NativeSkillPicker, NativeSkillPickerMode, NativeSkillSnapshot, NativeSkillsCommand,
    NativeSkillsNotice, NativeSkillsServiceResult,
};
use std::{ops::Range, sync::Arc};

#[cfg(test)]
mod tests;

pub(super) struct SkillsUi {
    picker: NativeSkillPicker,
    epoch: NativeSkillDraftIdentity,
    snapshot: Option<Arc<NativeSkillSnapshot>>,
    drawn: Option<NativeSkillFrameIdentity>,
    acknowledged: Option<NativeSkillFrameIdentity>,
    pending_frame: Option<NativeSkillFrameIdentity>,
    pending_selection: Option<InputBinding>,
    request: Option<Request>,
    refresh: Option<BackgroundOutputOwner>,
    input_owner: Option<InputBinding>,
}

struct Request {
    id: NativeInteractiveControlId,
    source: BackgroundOutputOwner,
    epoch: NativeSkillDraftIdentity,
    open: bool,
}

impl SkillsUi {
    fn binding(&self) -> InputBinding {
        let view = self.picker.view();
        let frame = view
            .as_ref()
            .and_then(|view| {
                self.acknowledged
                    .as_ref()
                    .filter(|identity| **identity == view.identity)
            })
            .cloned();
        InputBinding::Skills {
            epoch: self.epoch.clone(),
            frame,
            pending_frame: view.as_ref().and_then(|view| {
                self.pending_frame
                    .as_ref()
                    .filter(|identity| **identity == view.identity)
                    .cloned()
            }),
            query: view.is_some_and(|view| view.mode == NativeSkillPickerMode::Menu),
        }
    }

    fn acknowledge(&mut self, binding: &InputBinding) {
        if !self.matches(binding) {
            return;
        }
        if let InputBinding::Skills {
            frame: Some(frame), ..
        } = binding
            && self.picker.acknowledge(frame).is_ok()
        {
            self.acknowledged = Some(frame.clone());
            if self.pending_frame.as_ref() == Some(frame) {
                self.pending_frame = None;
            }
        }
    }

    fn invalidate_frame(&mut self) {
        self.drawn = None;
        self.acknowledged = None;
        self.pending_frame = None;
        self.pending_selection = None;
        if self.picker.invalidate_frame().is_err() {
            self.picker.close();
        }
    }
    pub(super) fn new(snapshot: Option<Arc<NativeSkillSnapshot>>) -> Self {
        let picker = NativeSkillPicker::new(String::new(), 0).expect("empty draft");
        Self {
            epoch: picker.draft_identity().clone(),
            picker,
            snapshot,
            drawn: None,
            acknowledged: None,
            pending_frame: None,
            pending_selection: None,
            request: None,
            refresh: None,
            input_owner: None,
        }
    }

    fn matches(&self, binding: &InputBinding) -> bool {
        matches!(binding, InputBinding::Skills { epoch, .. } if epoch == &self.epoch)
    }

    pub(super) fn edit(
        &mut self,
        binding: &InputBinding,
        range: Range<usize>,
        inserted: &str,
        cursor: usize,
    ) -> Result<(), ()> {
        if !self.matches(binding) {
            return Err(());
        }
        self.pending_selection = None;
        if matches!(binding, InputBinding::Skills { query: true, .. }) {
            return Ok(()); // query decoder is forwarded after the atomic edit.
        }
        self.picker
            .apply_edit(
                &self.picker.draft_identity().clone(),
                range,
                inserted,
                cursor,
            )
            .map_err(|_| ())
    }

    fn reset(&mut self, text: &str, cursor: usize) {
        self.picker
            .reset(text.to_owned(), cursor)
            .expect("bounded composer draft");
        self.epoch = self.picker.draft_identity().clone();
        self.drawn = None;
        self.acknowledged = None;
        self.pending_frame = None;
        self.pending_selection = None;
    }

    fn observe_cursor(
        &mut self,
        cursor: usize,
    ) -> Result<(), machine_god_native::NativeSkillPickerError> {
        // Actual edit receipts already update the native cursor and filter.
        // Only a distinct cursor-only event needs another native transition.
        if cursor == self.picker.cursor() {
            Ok(())
        } else {
            self.pending_selection = None;
            self.picker
                .move_cursor(&self.picker.draft_identity().clone(), cursor)
        }
    }
}

impl Driver {
    #[cfg(test)]
    pub(super) fn bound_skill_count(&self) -> usize {
        self.skills
            .as_ref()
            .map_or(0, |skills| skills.picker.bindings().len())
    }

    #[cfg(test)]
    pub(super) fn has_pending_skills_selection(&self) -> bool {
        self.skills
            .as_ref()
            .is_some_and(|skills| skills.pending_selection.is_some())
    }

    pub(super) fn discard_pending_skills_selection(&mut self) {
        if let Some(skills) = &mut self.skills {
            skills.pending_selection = None;
        }
    }

    pub(super) fn sync_skills_input_owner(&mut self, binding: &InputBinding) {
        let Some(skills) = &mut self.skills else {
            return;
        };
        let next = (!matches!(binding, InputBinding::Skills { .. } | InputBinding::Command))
            .then(|| binding.clone());
        if skills.input_owner != next {
            skills.input_owner = next;
            self.input.reset_raw_draft();
            self.reset_skills();
        }
    }

    pub(super) fn with_skills_snapshot(
        mut self,
        snapshot: Option<Arc<NativeSkillSnapshot>>,
    ) -> Self {
        self.set_skills_snapshot(snapshot);
        self
    }

    pub(super) fn set_skills_snapshot(&mut self, snapshot: Option<Arc<NativeSkillSnapshot>>) {
        self.owner.set_managed_skills_snapshot(snapshot.clone());
        if self.owner.skills_catalog().is_some() {
            self.skills = Some(SkillsUi::new(snapshot));
        }
    }

    pub(super) fn skills_open(&self) -> bool {
        self.skills
            .as_ref()
            .is_some_and(|skills| skills.picker.view().is_some())
    }

    pub(super) fn skills_query_open(&self) -> bool {
        self.skills
            .as_ref()
            .and_then(|skills| skills.picker.view())
            .is_some_and(|view| view.mode == NativeSkillPickerMode::Menu)
    }

    pub(super) fn skills_binding(&self) -> Option<InputBinding> {
        if self.frontend.is_none() || !self.scope_active || self.modal.is_some() {
            return None;
        }
        Some(self.skills.as_ref()?.binding())
    }

    pub(super) fn skills_command_binding(&self, binding: &InputBinding) -> bool {
        self.skills
            .as_ref()
            .is_some_and(|skills| skills.matches(binding))
            && !matches!(binding, InputBinding::Skills { query: true, .. })
    }

    /// Context transfer invalidates even identical text. Neither an old query
    /// decoder nor its buffered remainder can become an answer to a new owner.
    pub(super) fn reset_skills(&mut self) {
        self.input.close_skills_query();
        if let Some(skills) = &mut self.skills {
            let (text, cursor) = self.input.original_draft().unwrap_or(("", 0));
            skills.reset(text, cursor);
        }
    }

    pub(super) fn finish_skills_submission(&mut self) {
        if self.skills.as_ref().is_some_and(|skills| {
            self.input.original_draft().is_some_and(|(text, cursor)| {
                text != skills.picker.draft() || cursor != skills.picker.cursor()
            })
        }) {
            self.reset_skills();
        }
    }

    pub(super) fn invalidate_skills_frame(&mut self) {
        if let Some(skills) = &mut self.skills {
            skills.invalidate_frame();
        }
    }

    pub(super) fn close_skills(&mut self) {
        self.input.close_skills_query();
        if let Some(skills) = &mut self.skills {
            skills.picker.close();
            skills.drawn = None;
            skills.acknowledged = None;
            skills.pending_frame = None;
            skills.pending_selection = None;
        }
        if let Some(frontend) = &mut self.frontend {
            frontend.dirty = true;
        }
    }

    pub(super) fn skills_event(&mut self, event: &ComposerEvent, binding: &InputBinding) -> bool {
        if matches!(event, ComposerEvent::StaleInput) {
            self.note(b"\n[stale editor input ignored]\n");
            return true;
        }
        let Some(skills) = &mut self.skills else {
            return false;
        };
        if !skills.matches(binding) {
            return false;
        }
        match event {
            ComposerEvent::Changed => {
                skills.pending_selection = None;
                let Some((text, cursor)) = self.input.raw_draft() else {
                    return true;
                };
                let result = if matches!(binding, InputBinding::Skills { query: true, .. }) {
                    skills.picker.query_menu(text)
                } else if text == skills.picker.draft() {
                    skills.observe_cursor(cursor)
                } else {
                    self.reset_skills();
                    return true;
                };
                if result.is_err() {
                    self.reset_skills();
                    return true;
                }
                if skills.picker.view().is_none()
                    && skills.picker.inline_query().is_some()
                    && let Some(snapshot) = &skills.snapshot
                {
                    let _ = skills.picker.open_inline(snapshot.clone());
                }
                true
            }
            ComposerEvent::PickerNext | ComposerEvent::PickerPrevious
                if skills.picker.view().is_some() =>
            {
                skills.pending_selection = None;
                let _ = skills
                    .picker
                    .move_selection(matches!(event, ComposerEvent::PickerNext));
                true
            }
            ComposerEvent::SkillSelected => {
                let InputBinding::Skills {
                    frame,
                    pending_frame,
                    ..
                } = binding
                else {
                    unreachable!("matched skills binding");
                };
                if let Some(frame) = frame {
                    self.choose_skill(frame);
                } else if let Some(frame) = pending_frame {
                    if skills.acknowledged.as_ref() == Some(frame) {
                        // The real flush may finish between receiving these
                        // bytes and decoding their selection event.
                        self.choose_skill(frame);
                    } else if skills.pending_frame.as_ref() == Some(frame)
                        && skills
                            .picker
                            .view()
                            .is_some_and(|view| view.identity == *frame)
                    {
                        // One exact intent, never a queue of subsequent Enter
                        // submissions. No draft mutation before real flush ACK.
                        skills
                            .pending_selection
                            .get_or_insert_with(|| binding.clone());
                    } else {
                        self.note(b"\n[stale skill frame; selection ignored]\n");
                    }
                } else {
                    self.note(b"\n[skill selection waits for the displayed frame]\n");
                }
                true
            }
            ComposerEvent::EscapeRequested if skills.picker.view().is_some() => {
                self.close_skills();
                true
            }
            ComposerEvent::CancelRequested => {
                self.close_skills();
                let status = self.owner.runtime().status();
                if !status.active && status.queued_jobs == 0 {
                    self.input.reset_raw_draft();
                }
                self.reset_skills();
                false
            }
            _ => false,
        }
    }

    fn choose_skill(&mut self, frame: &NativeSkillFrameIdentity) {
        let Some(skills) = &mut self.skills else {
            return;
        };
        skills.pending_selection = None;
        if self.input.original_draft() != Some((skills.picker.draft(), skills.picker.cursor())) {
            self.reset_skills();
            self.note(b"\n[skill draft changed; selection ignored]\n");
            return;
        }
        match skills.picker.choose(frame) {
            Ok(insertion) => {
                if self.input.apply_skill_insertion(&insertion).is_err() {
                    self.reset_skills();
                    self.note(b"\n[skill draft changed; selection ignored]\n");
                } else {
                    // Both immediate and deferred selection fence all bytes
                    // received under the old editor, including Tab + Enter.
                    skills.epoch = skills.picker.draft_identity().clone();
                    skills.drawn = None;
                    skills.acknowledged = None;
                    skills.pending_frame = None;
                    if let Some(frontend) = &mut self.frontend {
                        frontend.dirty = true;
                    }
                }
            }
            Err(_) => self.note(b"\n[stale skill frame; selection ignored]\n"),
        }
    }

    pub(super) fn enqueue_skills_prompt(&mut self, prompt: &str) {
        if self.owner.skills_catalog().is_none() {
            if self.owner.enqueue(prompt.into()).is_err() {
                self.note(b"\n[prompt queue unavailable]\n");
            }
            return;
        }
        let Some(skills) = &self.skills else {
            self.note(b"\n[skills discovery required; run /skills before submitting]\n");
            return;
        };
        let Some(snapshot) = &skills.snapshot else {
            self.note(b"\n[skills discovery required; run /skills before submitting]\n");
            return;
        };
        let explicit = if self.frontend.is_some() {
            if skills.picker.draft() != prompt {
                self.note(b"\n[skill draft changed; prompt not queued]\n");
                return;
            }
            let Ok(selections) = skills.picker.selections(skills.picker.draft_identity()) else {
                self.note(b"\n[skill binding changed; prompt not queued]\n");
                return;
            };
            selections
        } else {
            Vec::new()
        };
        match self
            .owner
            .enqueue_with_skills(prompt.into(), snapshot, &explicit)
        {
            Ok(receipt) if receipt.automatic_matching_incomplete => {
                const WARNING: &[u8] =
                    b"\n[skills discovery incomplete; automatic matching suppressed]\n";
                self.skills_warning = Some(WARNING);
            }
            Ok(_) => {}
            Err(_) => self.note(b"\n[skill prompt rejected; refresh /skills and try again]\n"),
        }
    }

    pub(super) fn skills_command(&mut self, command: NativeSkillsCommand, now_ms: i64) {
        if self.control_outcome.is_some() {
            self.note(b"\n[previous control is still pending]\n");
            return;
        }
        if self.skills.is_none() && self.owner.skills_catalog().is_some() {
            self.skills = Some(SkillsUi::new(None));
        }
        self.finish_skills_submission();
        match self
            .owner
            .request_control(NativeInteractiveControl::Skills { command }, now_ms)
        {
            Ok(id) => {
                if let Some(skills) = &mut self.skills {
                    skills.request = Some(Request {
                        id,
                        source: principal(&self.owner),
                        epoch: skills.epoch.clone(),
                        open: true,
                    });
                }
            }
            Err(_) => self.note(b"\n[skills command unavailable or busy]\n"),
        }
    }

    pub(super) fn observe_skills_control(&mut self, outcome: &NativeInteractiveControlOutcome) {
        let Some(skills) = &mut self.skills else {
            return;
        };
        let Some(request) = skills.request.take() else {
            return;
        };
        if request.id != outcome.id {
            skills.request = Some(request);
            return;
        }
        if self.shutting_down
            || !self.scope_active
            || outcome.source != request.source
            || outcome.source != principal(&self.owner)
        {
            return;
        }
        let Ok(NativeInteractiveControlReceipt::Skills(result)) = &outcome.result else {
            return;
        };
        match result {
            NativeSkillsServiceResult::Managed(_) => {
                // A mutation (including partial/uncertain publication) makes
                // the previous discovery unsuitable for a fresh invocation.
                skills.snapshot = None;
                self.owner.set_managed_skills_snapshot(None);
                skills.refresh = Some(outcome.source.clone());
                self.close_skills();
            }
            NativeSkillsServiceResult::Catalog(view) => {
                let snapshot = Arc::new(view.snapshot.clone());
                skills.snapshot = Some(snapshot.clone());
                self.owner
                    .set_managed_skills_snapshot(Some(snapshot.clone()));
                if !request.open
                    || request.epoch != skills.epoch
                    || self.frontend.is_none()
                    || self.modal.is_some()
                    || self.saved_rule.is_some()
                    || self
                        .picker
                        .as_ref()
                        .is_some_and(super::picker::Picker::is_open)
                    || view.notice == Some(NativeSkillsNotice::NotFound)
                {
                    return;
                }
                self.input.close_skills_query();
                skills.pending_selection = None;
                skills.pending_frame = None;
                skills.picker.close();
                if self.input.original_draft()
                    != Some((skills.picker.draft(), skills.picker.cursor()))
                    || self.input.open_skills_query(&view.query).is_err()
                {
                    return;
                }
                if skills.picker.open_menu(snapshot, &view.query).is_err() {
                    self.input.close_skills_query();
                    return;
                }
                if let Some(focus) = &view.focus
                    && skills.picker.focus(focus).is_err()
                {
                    self.close_skills();
                    return;
                }
                skills.drawn = None;
                skills.acknowledged = None;
                skills.pending_frame = None;
                skills.pending_selection = None;
            }
            NativeSkillsServiceResult::Path(_) => {}
        }
    }

    pub(super) fn poll_skills_refresh(&mut self, now_ms: i64) {
        if self.shutting_down || !self.scope_active || self.control_outcome.is_some() {
            return;
        }
        let Some(skills) = &mut self.skills else {
            return;
        };
        let Some(source) = &skills.refresh else {
            return;
        };
        if source != &principal(&self.owner) {
            skills.refresh = None;
            return;
        }
        if skills.request.is_some() {
            return;
        }
        if let Ok(id) = self.owner.request_control(
            NativeInteractiveControl::Skills {
                command: NativeSkillsCommand::List,
            },
            now_ms,
        ) {
            skills.request = Some(Request {
                id,
                source: source.clone(),
                epoch: skills.epoch.clone(),
                open: false,
            });
            skills.refresh = None;
        }
    }

    pub(super) fn acknowledge_skills(&mut self, binding: &InputBinding) {
        let Some(skills) = &mut self.skills else {
            return;
        };
        if self.shutting_down || !self.scope_active {
            skills.pending_selection = None;
            return;
        }
        skills.acknowledge(binding);
        if let Some(intent) = skills.pending_selection.take()
            && skills.matches(&intent)
            && let InputBinding::Skills {
                pending_frame: Some(frame),
                query,
                ..
            } = intent
            && skills.acknowledged.as_ref() == Some(&frame)
            && skills.picker.view().is_some_and(|view| {
                view.identity == frame && (view.mode == NativeSkillPickerMode::Menu) == query
            })
        {
            self.choose_skill(&frame);
        }
    }

    pub(super) fn prepare_skills_render(&mut self) -> bool {
        // As with the ordinary draft, do not split an active model stream.
        if self.owner.runtime().status().active {
            return false;
        }
        let Some(skills) = &mut self.skills else {
            return false;
        };
        let Some(view) = skills.picker.view() else {
            return false;
        };
        if skills.drawn.as_ref() == Some(&view.identity) {
            return true;
        }
        let Some(frontend) = &mut self.frontend else {
            return false;
        };
        if let Ok(frame) = super::skills_view::render(&view, frontend.columns, frontend.rows) {
            skills.drawn = Some(frame.identity.clone());
            skills.pending_frame = frame.selection_visible.then(|| frame.identity.clone());
            skills.pending_selection = None;
            frontend.menu_height = Some(frame.height);
            self.render = Some(Render {
                bytes: frame.bytes,
                offset: 0,
                history: false,
                clear_row: true,
                confirm: Some(InputBinding::Skills {
                    epoch: skills.epoch.clone(),
                    frame: frame.selection_visible.then_some(frame.identity),
                    pending_frame: None,
                    query: view.mode == NativeSkillPickerMode::Menu,
                }),
                receipt: None,
                model_text: false,
            });
        } else {
            self.close_skills();
            self.note(b"\n[skills menu unavailable at this terminal size]\n");
        }
        true
    }
}
