//! Decoder synchronization only; query, catalog and selection remain native.
use super::{Driver, InputBinding, NativeManagedEditorIdentity};
impl Driver {
    pub(super) fn sync_agent_model_editor(&mut self, binding: &InputBinding) {
        let InputBinding::Agents { editor, .. } = binding else {
            return;
        };
        let Some(models) = self.owner.managed_navigation().and_then(|view| view.models) else {
            return;
        };
        let Some(ui) = &mut self.agents else {
            return;
        };
        if ui.form_editor.as_ref() == Some(editor) {
            return;
        }
        match self
            .input
            .seed_managed_draft(editor, models.picker.query, models.picker.cursor)
        {
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
    pub(super) fn edit_agent_model_editor(&mut self, editor: &NativeManagedEditorIdentity) {
        let Some((query, cursor)) = self.input.raw_draft() else {
            return;
        };
        if self
            .owner
            .edit_managed_models(editor, query, cursor)
            .is_err()
        {
            if let Some(ui) = &mut self.agents {
                ui.form_editor = None;
            }
            self.sync_agent_model_editor(&InputBinding::Agents {
                editor: editor.clone(),
                frame: None,
            });
            self.invalidate_agents();
            self.note(b"\n[model query rejected; previous query retained]\n");
        }
    }
}
