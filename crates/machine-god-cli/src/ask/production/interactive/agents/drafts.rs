//! The native owner retains child drafts; the CLI only synchronizes its decoder.
use super::{Driver, InputBinding, NativeManagedEditorIdentity};

impl Driver {
    pub(super) fn consume_local_agent_line(&mut self, editor: &NativeManagedEditorIdentity) {
        // Replace text without resetting CRLF suppression or relabeling any
        // already received bytes. Native admission has accepted the command.
        if self.input.seed_managed_form(editor, "") == Ok(true) {
            if let Some(ui) = &mut self.agents {
                ui.draft_dirty = true;
            }
        } else {
            self.native_failed = true;
            self.shutdown();
        }
    }

    pub(in crate::ask::production::interactive) fn reject_agent_draft_edit(&mut self) {
        if let Some(ui) = &mut self.agents {
            ui.draft_editor = None;
        }
        if let Some(view) = self.owner.managed_navigation() {
            self.sync_agent_draft_editor(&InputBinding::Agents {
                editor: view.editor,
                frame: None,
                pending_frame: None,
            });
        }
        self.invalidate_agents();
        self.note(b"\n[agent draft edit rejected; previous draft retained]\n");
    }
    pub(super) fn sync_agent_draft_editor(&mut self, binding: &InputBinding) {
        let InputBinding::Agents { editor, .. } = binding else {
            return;
        };
        let Some(view) = self.owner.managed_navigation() else {
            return;
        };
        let Some(draft) = view.draft else {
            return;
        };
        let Some(ui) = &mut self.agents else {
            return;
        };
        if ui
            .draft_editor
            .as_ref()
            .is_some_and(|(seeded, revision)| seeded == editor && *revision == draft.revision)
        {
            return;
        }
        match self
            .input
            .seed_managed_draft(editor, draft.text, draft.cursor)
        {
            Ok(true) => {
                ui.draft_editor = Some((editor.clone(), draft.revision));
                ui.draft_dirty = true;
            }
            Ok(false) => {}
            Err(()) => {
                self.native_failed = true;
                self.shutdown();
            }
        }
    }

    pub(super) fn edit_agent_draft(&mut self, editor: &NativeManagedEditorIdentity) {
        if self
            .owner
            .managed_navigation()
            .is_none_or(|view| view.draft.is_none())
        {
            return;
        }
        let Some((text, cursor)) = self.input.raw_draft() else {
            return;
        };
        if self.owner.edit_managed_draft(editor, text, cursor).is_err() {
            // Roll back only the rejected edit. No capacity failure may evict
            // another child's nonempty draft or submit a truncated prefix.
            if let Some(ui) = &mut self.agents {
                ui.draft_editor = None;
            }
            self.sync_agent_draft_editor(&InputBinding::Agents {
                editor: editor.clone(),
                frame: None,
                pending_frame: None,
            });
            self.invalidate_agents();
            self.note(b"\n[agent draft edit rejected; previous draft retained]\n");
        } else if let Some(draft) = self.owner.managed_navigation().and_then(|view| view.draft)
            && let Some(ui) = &mut self.agents
        {
            ui.draft_editor = Some((editor.clone(), draft.revision));
        }
    }
}
