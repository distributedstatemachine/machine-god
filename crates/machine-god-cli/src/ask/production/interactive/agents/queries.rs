//! Decoder synchronization only; query, catalog and selection remain native.
use super::{Driver, InputBinding, NativeManagedEditorIdentity};
impl Driver {
    pub(super) fn sync_agent_query_editor(&mut self, binding: &InputBinding) {
        let InputBinding::Agents { editor, .. } = binding else {
            return;
        };
        let Some(view) = self.owner.managed_navigation() else {
            return;
        };
        let query = view
            .models
            .as_ref()
            .map(|models| (models.picker.query, models.picker.cursor))
            .or_else(|| {
                view.skills
                    .as_ref()
                    .map(|skills| (skills.query, view.skills_cursor))
            });
        let Some((query, cursor)) = query else {
            return;
        };
        let Some(ui) = &mut self.agents else {
            return;
        };
        if ui.form_editor.as_ref() == Some(editor) {
            return;
        }
        match self.input.seed_managed_draft(editor, query, cursor) {
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
    pub(super) fn edit_agent_query_editor(&mut self, editor: &NativeManagedEditorIdentity) {
        let Some((query, cursor)) = self.input.raw_draft() else {
            return;
        };
        let result = if self
            .owner
            .managed_navigation()
            .is_some_and(|view| view.skills.is_some())
        {
            self.owner.edit_managed_skill_query(editor, query, cursor)
        } else {
            self.owner.edit_managed_models(editor, query, cursor)
        };
        if result.is_err() {
            if let Some(ui) = &mut self.agents {
                ui.form_editor = None;
            }
            self.sync_agent_query_editor(&InputBinding::Agents {
                editor: editor.clone(),
                frame: None,
            });
            self.invalidate_agents();
            self.note(b"\n[query rejected; previous query retained]\n");
        }
    }
}
