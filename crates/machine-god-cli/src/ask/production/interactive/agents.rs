//! Thin native-navigation adapter. This owns only rendering/flush and editor custody.
mod drafts;
mod forms;
mod models;
mod render;
use super::{Driver, InputBinding, Render, composer::ComposerEvent, principal};
use machine_god_core::{
    BackgroundOutputOwner, ManagedInspectSection as Section, ManagedLifecycleAction as Lifecycle,
    ManagedRelationshipAction, ManagedSubagentCommand,
};
use machine_god_native::{
    NativeManagedCatalogFilter as Filter, NativeManagedEditorIdentity, NativeManagedFormKind,
    NativeManagedFrameIdentity, NativeManagedHistoryMode as HistoryMode,
    NativeManagedNavigationAction as Action, NativeManagedNavigationRoute as Route,
    NativeManagedProcessScope as Scope,
};

pub(super) struct Ui {
    parent: BackgroundOutputOwner,
    drawn: Option<NativeManagedFrameIdentity>,
    acknowledged: Option<NativeManagedFrameIdentity>,
    draft_dirty: bool,
    detail_offset: usize,
    detail_source: Option<NativeManagedFrameIdentity>,
    form_editor: Option<NativeManagedEditorIdentity>,
    draft_editor: Option<(NativeManagedEditorIdentity, u64)>,
}

impl Driver {
    fn open_agents(&mut self) {
        if self.frontend.is_none()
            || self.modal.is_some()
            || self.saved_rule.is_some()
            || self.picker_open()
            || self.skills_query_open()
            || self.shutting_down
        {
            self.note(b"\n[agent navigation unavailable in this input view]\n");
            return;
        }
        self.reset_skills();
        if self.owner.open_managed_navigation().is_err() {
            self.owner.close_managed_navigation();
            self.note(b"\n[agent navigation unavailable or still settling]\n");
            return;
        }
        let editor = self
            .owner
            .managed_navigation()
            .expect("opened navigation")
            .editor;
        if self.input.open_managed_editor(editor).is_err() {
            self.owner.close_managed_navigation();
            self.note(b"\n[agent editor unavailable]\n");
            return;
        }
        self.agents = Some(Ui {
            parent: principal(&self.owner),
            drawn: None,
            acknowledged: None,
            draft_dirty: true,
            detail_offset: 0,
            detail_source: None,
            form_editor: None,
            draft_editor: None,
        });
    }

    pub(super) fn sync_agents(&mut self) {
        if self.owner.managed_navigation().is_none()
            && let Some(ui) = self.agents.take()
        {
            self.input
                .close_managed_editor(!self.shutting_down && ui.parent == principal(&self.owner));
            self.reset_skills();
            if let Some(frontend) = &mut self.frontend {
                frontend.dirty = true;
            }
        }
    }

    pub(super) fn agents_binding(&self) -> Option<InputBinding> {
        if self.modal.is_some() || self.saved_rule.is_some() {
            return None;
        }
        let view = self.owner.managed_navigation()?;
        let ui = self.agents.as_ref()?;
        Some(InputBinding::Agents {
            editor: view.editor,
            frame: ui
                .acknowledged
                .as_ref()
                .filter(|frame| **frame == view.frame)
                .cloned(),
        })
    }

    pub(super) fn acknowledge_agents(&mut self, binding: &InputBinding) {
        if let InputBinding::Agents {
            frame: Some(frame), ..
        } = binding
            && self.owner.acknowledge_managed_frame(frame).is_ok()
            && let Some(ui) = &mut self.agents
        {
            ui.acknowledged = Some(frame.clone());
        }
    }

    pub(super) fn invalidate_agents(&mut self) {
        if let Some(ui) = &mut self.agents {
            ui.drawn = None;
            ui.acknowledged = None;
            if self.owner.invalidate_managed_frame().is_err() {
                self.owner.close_managed_navigation();
            }
        }
    }

    pub(super) fn agents_event(&mut self, event: &ComposerEvent, binding: &InputBinding) -> bool {
        if matches!(event, ComposerEvent::AgentsRequested) {
            if let InputBinding::Agents { editor, .. } = binding
                && self
                    .owner
                    .managed_navigation()
                    .is_some_and(|view| view.editor == *editor)
            {
                self.owner.close_managed_navigation();
                self.sync_agents();
            } else if matches!(binding, InputBinding::Command)
                || self.skills_command_binding(binding)
            {
                self.open_agents();
            }
            return true;
        }
        let InputBinding::Agents { editor, frame } = binding else {
            return false;
        };
        if self
            .owner
            .managed_navigation()
            .is_none_or(|view| view.editor != *editor)
        {
            return true;
        }
        if self.scroll_agents(event, frame.as_ref()) {
            return true;
        }
        let action = match event {
            ComposerEvent::HistoryToggle => match self.agent_history_toggle() {
                Some(action) => action,
                None => return true,
            },
            ComposerEvent::HistorySummary => Action::HistoryMode(HistoryMode::Transcript),
            ComposerEvent::HistoryFull => Action::HistoryMode(HistoryMode::Full),
            ComposerEvent::Changed | ComposerEvent::CancelRequested
                if matches!(event, ComposerEvent::Changed)
                    || self
                        .owner
                        .managed_navigation()
                        .is_some_and(|view| view.form.is_some() || view.models.is_some()) =>
            {
                self.edit_agent_form(editor);
                if let Some(ui) = &mut self.agents {
                    ui.draft_dirty = true;
                }
                return true;
            }
            ComposerEvent::PickerPrevious => Action::Previous,
            ComposerEvent::PickerNext => Action::Next,
            ComposerEvent::PickerToggleScope => Action::CycleFormField,
            ComposerEvent::FormRefreshRequested => Action::Refresh,
            ComposerEvent::EscapeRequested => Action::Back,
            ComposerEvent::CancelRequested => Action::Lifecycle(Lifecycle::Cancel),
            ComposerEvent::Submit(line) => {
                if let Ok(action) = self.agent_line_action(line) {
                    action
                } else {
                    self.note(b"\n[agent command rejected; use the displayed commands]\n");
                    return true;
                }
            }
            ComposerEvent::InputError(_) => {
                self.invalidate_agents();
                self.note(b"\n[agent input rejected; draft retained]\n");
                return true;
            }
            ComposerEvent::ExitRequested => {
                self.shutdown();
                return true;
            }
            _ => return true,
        };
        let exiting = matches!(action, Action::Exit);
        let result = frame.as_ref().ok_or(()).and_then(|frame| {
            if let ComposerEvent::Submit(text) = event {
                self.owner.submit_managed_frame(frame, action, text)
            } else {
                self.owner.act_on_managed_frame(frame, action)
            }
            .map_err(|_| ())
        });
        if result.is_err() {
            self.note(
                b"\n[agent view changed, is busy, or has not been displayed; input retained]\n",
            );
        } else if exiting {
            self.shutdown();
        }
        self.sync_agents();
        true
    }

    fn agent_history_toggle(&self) -> Option<Action> {
        let history = self.owner.managed_navigation()?.history?;
        Some(Action::HistoryMode(
            if history.mode == HistoryMode::Conversation {
                HistoryMode::Transcript
            } else {
                HistoryMode::Conversation
            },
        ))
    }

    fn scroll_agents(
        &mut self,
        event: &ComposerEvent,
        frame: Option<&NativeManagedFrameIdentity>,
    ) -> bool {
        let previous = matches!(
            event,
            ComposerEvent::PickerPrevious | ComposerEvent::HistoryPageUp
        );
        let page = matches!(
            event,
            ComposerEvent::HistoryPageUp | ComposerEvent::HistoryPageDown
        );
        if !previous
            && !matches!(
                event,
                ComposerEvent::PickerNext | ComposerEvent::HistoryPageDown
            )
        {
            return false;
        }
        let Some(view) = self.owner.managed_navigation() else {
            return false;
        };
        if !matches!(
            view.route,
            Route::Conversation | Route::Agent(_) | Route::Processes(_)
        ) {
            return false;
        }
        let Some(ui) = &mut self.agents else {
            return true;
        };
        if frame != Some(&view.frame) || ui.acknowledged.as_ref() != frame || view.busy {
            return true;
        }
        if view.route == Route::Conversation {
            if let Some(frontend) = &self.frontend
                && let Some(action) =
                    render::scroll_history(&view, frontend.columns, frontend.rows, previous, page)
            {
                let frame = view.frame;
                let _ = self.owner.act_on_managed_frame(&frame, action);
            }
            return true;
        }
        let offset = if ui.detail_source.as_ref() == Some(&view.frame) {
            ui.detail_offset
        } else {
            0
        };
        let count = match view.route {
            Route::Processes(_) => render::process_count(view.processes),
            _ => render::detail_count(view.result),
        };
        let next = if previous {
            offset.saturating_sub(1)
        } else {
            offset.saturating_add(1).min(count.saturating_sub(1))
        };
        if next != offset {
            self.invalidate_agents();
            if let Some(view) = self.owner.managed_navigation()
                && let Some(ui) = &mut self.agents
            {
                ui.detail_offset = next;
                ui.detail_source = Some(view.frame);
            }
        }
        true
    }

    fn agent_line_action(&self, line: &str) -> Result<Action, ()> {
        let view = self.owner.managed_navigation().ok_or(())?;
        if view.route == Route::Models {
            return Ok(Action::Select);
        }
        if matches!(view.route, Route::Form(_)) {
            return Ok(Action::SubmitForm);
        }
        let text = line.trim();
        Ok(match text {
            "" if view.route == Route::ConfirmClose => Action::ConfirmClose,
            "" => Action::Select,
            "/back" => Action::Back,
            "/quit" => Action::Exit,
            "/next" => Action::NextPage,
            "/refresh" => Action::Refresh,
            "/archived" => Action::Filter(Filter::Archived),
            "/current" => Action::Filter(Filter::Current),
            "/all" => Action::Filter(Filter::All),
            "/parent-processes" => Action::Processes(Scope::Parent),
            "/agent-processes" => Action::Processes(Scope::SelectedAgent),
            "/processes" => Action::Processes(
                if matches!(
                    view.route,
                    Route::Conversation | Route::Agent(_) | Route::Processes(Scope::SelectedAgent)
                ) {
                    Scope::SelectedAgent
                } else {
                    Scope::Parent
                },
            ),
            "/status" => Action::Inspect(Section::Status),
            "/history" | "/conversation" => Action::Conversation,
            "/messages" => Action::Inspect(Section::Messages),
            "/tools" => Action::Inspect(Section::ToolActivity),
            "/events" => Action::Inspect(Section::Events),
            "/configuration" => Action::Inspect(Section::Configuration),
            "/relationship" => Action::Inspect(Section::Relationship),
            "/create" => Action::OpenForm(NativeManagedFormKind::Create),
            "/configure" => Action::OpenForm(NativeManagedFormKind::Configure),
            "/models" => Action::Models,
            "/cancel" => Action::Lifecycle(Lifecycle::Cancel),
            "/resume" => Action::Lifecycle(Lifecycle::Resume),
            "/reopen" => Action::Lifecycle(Lifecycle::Reopen),
            "/close" => Action::Lifecycle(Lifecycle::Close),
            "/confirm" => Action::ConfirmClose,
            "/detach" => Action::Relationship {
                action: ManagedRelationshipAction::Detach,
                parent_id: None,
            },
            _ if text.starts_with("/attach ") => Action::Relationship {
                action: ManagedRelationshipAction::Attach,
                parent_id: Some(text[8..].trim().to_owned()),
            },
            _ if text.starts_with("/reparent ") => Action::Relationship {
                action: ManagedRelationshipAction::Reparent,
                parent_id: Some(text[10..].trim().to_owned()),
            },
            _ if text.starts_with("/create ") => {
                let create: serde_json::Value = serde_json::from_str(&text[8..]).map_err(|_| ())?;
                match ManagedSubagentCommand::decode(
                    serde_json::json!({"command":{"create":create}}),
                )
                .map_err(|_| ())?
                {
                    ManagedSubagentCommand::Create(create) => Action::Create(create),
                    _ => return Err(()),
                }
            }
            _ if text.starts_with("/configure ") => {
                let mut configure: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_str(&text[11..]).map_err(|_| ())?;
                if configure.contains_key("id") {
                    return Err(());
                }
                configure.insert(
                    "id".into(),
                    serde_json::Value::String(view.target.ok_or(())?.id.clone()),
                );
                match ManagedSubagentCommand::decode(
                    serde_json::json!({"command":{"configure":configure}}),
                )
                .map_err(|_| ())?
                {
                    ManagedSubagentCommand::Configure(configure) => Action::Configure(configure),
                    _ => return Err(()),
                }
            }
            _ if !text.starts_with('/')
                && matches!(view.route, Route::Agent(_) | Route::Conversation) =>
            {
                Action::Message(line.to_owned())
            }
            _ => return Err(()),
        })
    }

    pub(super) fn prepare_agents_render(&mut self) -> bool {
        // A route can change during the input poll. Seed its new field before
        // drawing, rather than acknowledging a frame with the old field's text.
        if let Some(binding @ InputBinding::Agents { .. }) = self.agents_binding() {
            if let InputBinding::Agents { editor, .. } = &binding {
                self.input.sync_managed_editor(Some(editor));
            }
            self.sync_agent_form_editor(&binding);
        }
        let Some(ui) = &mut self.agents else {
            return false;
        };
        let Some(view) = self.owner.managed_navigation() else {
            return false;
        };
        if ui.drawn.as_ref() == Some(&view.frame) && !ui.draft_dirty {
            return true;
        }
        let Some(frontend) = &mut self.frontend else {
            return false;
        };
        let draft = self.input.raw_draft().unwrap_or(("", 0));
        if ui.detail_source.as_ref() != Some(&view.frame) {
            ui.detail_offset = 0;
            ui.detail_source = Some(view.frame.clone());
        }
        let Ok(frame) = render::render(
            &view,
            draft,
            frontend.columns,
            frontend.rows,
            ui.detail_offset,
        ) else {
            self.native_failed = true;
            self.shutdown();
            return true;
        };
        ui.drawn = Some(view.frame.clone());
        ui.draft_dirty = false;
        let confirm = Some(InputBinding::Agents {
            frame: (frame.selectable
                && (view.form.is_none() || ui.form_editor.as_ref() == Some(&view.editor))
                && (view.models.is_none() || ui.form_editor.as_ref() == Some(&view.editor))
                && view.draft.is_none_or(|draft| {
                    ui.draft_editor.as_ref().is_some_and(|(editor, revision)| {
                        editor == &view.editor && *revision == draft.revision
                    })
                }))
            .then_some(view.frame),
            editor: view.editor,
        });
        frontend.menu_height = Some(frame.height);
        self.render = Some(Render {
            bytes: frame.bytes,
            offset: 0,
            history: false,
            clear_row: false,
            confirm,
            receipt: None,
            model_text: false,
        });
        true
    }
}
