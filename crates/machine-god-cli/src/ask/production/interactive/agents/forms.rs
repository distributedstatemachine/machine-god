use super::{Driver, InputBinding, NativeManagedEditorIdentity};

impl Driver {
    pub(in crate::ask::production::interactive) fn sync_agent_form_editor(
        &mut self,
        binding: &InputBinding,
    ) {
        if self
            .owner
            .managed_navigation()
            .is_some_and(|view| view.models.is_some() || view.skills.is_some())
        {
            self.sync_agent_query_editor(binding);
            return;
        }
        let InputBinding::Agents { editor, .. } = binding else {
            return;
        };
        let Some(view) = self.owner.managed_navigation() else {
            return;
        };
        let Some(form) = view.form else {
            self.sync_agent_draft_editor(binding);
            return;
        };
        let Some(ui) = &mut self.agents else {
            return;
        };
        if ui.form_editor.as_ref() == Some(editor) {
            return;
        }
        let text = if form.fields[form.selected].byte_limit().is_some() {
            form.values[form.selected]
        } else {
            ""
        };
        match self.input.seed_managed_form(editor, text) {
            Ok(true) => {
                ui.form_editor = Some(editor.clone());
                ui.draft_dirty = true;
            }
            Ok(false) => {}
            Err(()) => {
                self.native_failed = true;
                self.shutdown();
            }
        }
    }

    pub(super) fn edit_agent_form(&mut self, editor: &NativeManagedEditorIdentity) {
        if self
            .owner
            .managed_navigation()
            .is_some_and(|view| view.models.is_some() || view.skills.is_some())
        {
            self.edit_agent_query_editor(editor);
            return;
        }
        let Some(view) = self.owner.managed_navigation() else {
            return;
        };
        let Some(form) = view.form else {
            self.edit_agent_draft(editor);
            return;
        };
        if view.busy {
            return;
        }
        if form.fields[form.selected].byte_limit().is_none() {
            // Pasting/typing into a toggle is never a toggle or a submission.
            // Retire its old ACK as well as the ignored text, so a trailing
            // Enter in that same received chunk cannot submit the form.
            let _ = self.input.seed_managed_form(editor, "");
            self.invalidate_agents();
            return;
        }
        let Some((text, _)) = self.input.raw_draft() else {
            return;
        };
        if self.owner.edit_managed_form(editor, text).is_err() {
            self.note(b"\n[form field rejected; correct the retained draft before submitting]\n");
        }
    }
}
