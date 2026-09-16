//! Native page ownership and bounded asynchronous navigation work.
mod models;
mod skills;
use super::super::agent_form::Form;
use super::{
    NativeInteractiveSession,
    view::{
        Identity, NativeManagedFrameIdentity, NativeManagedNavigationAction,
        NativeManagedNavigationError, NativeManagedNavigationRoute, NativeManagedNavigationView,
        editor,
    },
};
use crate::{
    NativeManagedCatalogCursor, NativeManagedCatalogEntry, NativeManagedCatalogFilter,
    NativeManagedCatalogPage, NativeManagedCatalogRequest, NativeManagedCommandResponse,
};
use crate::{
    NativeManagedEditorIdentity, NativeManagedFormKind, NativeManagedProcessScope as Scope,
};
use machine_god_core::{
    CancellationToken, ManagedInspect, ManagedInspectSection, ManagedLifecycle,
    ManagedLifecycleAction, ManagedMessage, ManagedRelationship, ManagedSend,
    ManagedSubagentCommand, ManagedSubagentResult,
};
pub use models::NativeManagedModelsView;
use std::{
    sync::Arc,
    task::{Context, Poll},
};

type Error = NativeManagedNavigationError;
type Action = NativeManagedNavigationAction;
type Route = NativeManagedNavigationRoute;

fn mutation_receipt(
    result: &Result<ManagedSubagentResult, machine_god_core::ManagedSubagentError>,
) -> bool {
    result.as_ref().is_ok_and(|result| {
        result.ok
            && matches!(
                result.requested,
                Some(machine_god_core::ManagedRequested::Receipt(_))
            )
    })
}

enum Pending {
    History {
        request: crate::NativeManagedHistoryRequest,
        epoch: u64,
    },
    Processes {
        future: super::processes::Snapshot,
        cancellation: CancellationToken,
        epoch: u64,
    },
    Catalog {
        request: NativeManagedCatalogRequest,
        epoch: u64,
    },
    Command {
        response: NativeManagedCommandResponse,
        cancellation: CancellationToken,
        epoch: u64,
        draft: Option<super::drafts::Submission>,
    },
}

#[derive(Clone, Copy)]
enum Refresh {
    Inspect(ManagedInspectSection),
    History,
    Catalog,
}

pub(in crate::interactive_session) struct Navigation {
    identity: Arc<Identity>,
    epoch: u64,
    revision: u64,
    displayed: Option<NativeManagedFrameIdentity>,
    open: bool,
    route: Route,
    filter: NativeManagedCatalogFilter,
    rows: Vec<NativeManagedCatalogEntry>,
    selected: Option<usize>,
    start: Option<NativeManagedCatalogCursor>,
    next: Option<NativeManagedCatalogCursor>,
    pending: Option<Pending>,
    result: Option<ManagedSubagentResult>,
    error: Option<Error>,
    form: Option<Form>,
    process_owner: Option<machine_god_core::BackgroundOutputOwner>,
    processes: Option<crate::NativeTerminalBackgroundSnapshot>,
    drafts: super::drafts::Drafts,
    history: super::history::History,
    history_resident: bool,
    history_retry: Option<u64>,
    models: models::Models,
    skills_snapshot: Option<Arc<crate::NativeSkillSnapshot>>,
    skills_cursor: usize,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            identity: Arc::new(Identity),
            epoch: 0,
            revision: 0,
            displayed: None,
            open: false,
            route: Route::Catalog(NativeManagedCatalogFilter::Current),
            filter: NativeManagedCatalogFilter::Current,
            rows: Vec::new(),
            selected: None,
            start: None,
            next: None,
            pending: None,
            result: None,
            error: None,
            form: None,
            process_owner: None,
            processes: None,
            drafts: super::drafts::Drafts::default(),
            history: super::history::History::default(),
            history_resident: false,
            history_retry: None,
            models: models::Models::default(),
            skills_snapshot: None,
            skills_cursor: 0,
        }
    }
}

impl Navigation {
    pub(super) fn submitted_draft(&self, text: &str) -> Option<super::drafts::Submission> {
        if !matches!(self.route, Route::Agent(_) | Route::Conversation) {
            return None;
        }
        self.drafts
            .submission(&self.target().ok()?.observation, text)
    }

    pub(super) fn retain_submission(&mut self, submission: super::drafts::Submission) {
        if let Some(Pending::Command { draft, .. }) = &mut self.pending {
            if draft.is_none() {
                *draft = Some(submission);
            }
        } else {
            self.drafts.accepted(&submission);
            if self.route == Route::Skills
                && let Err(error) = self.open_skills()
            {
                self.error = Some(error);
            }
        }
    }

    pub(super) fn owns_catalog(&self) -> bool {
        matches!(self.pending, Some(Pending::Catalog { .. }))
    }
    pub(super) fn owns_history(&self) -> bool {
        matches!(self.pending, Some(Pending::History { .. }))
    }
    fn frame(&self) -> NativeManagedFrameIdentity {
        NativeManagedFrameIdentity {
            editor: editor(&self.identity, self.epoch),
            revision: self.revision,
        }
    }
    pub(super) fn view(&self) -> Option<NativeManagedNavigationView<'_>> {
        self.open.then(|| NativeManagedNavigationView {
            skills: (self.route == Route::Skills)
                .then(|| {
                    self.target()
                        .ok()
                        .and_then(|target| self.drafts.skills(&target.observation))
                })
                .flatten(),
            skills_cursor: self.skills_cursor,
            skills_incomplete: self
                .skills_snapshot
                .as_ref()
                .is_some_and(|snapshot| !snapshot.complete()),
            models: (self.route == Route::Models).then(|| self.models.view()),
            history: self.history.view(),
            draft: matches!(self.route, Route::Agent(_) | Route::Conversation)
                .then(|| {
                    self.target()
                        .ok()
                        .map(|target| self.drafts.view(&target.observation))
                })
                .flatten(),
            editor: editor(&self.identity, self.epoch),
            frame: self.frame(),
            route: self.route,
            rows: &self.rows,
            selected: self.selected,
            target: if self.route == Route::Processes(Scope::Parent) {
                None
            } else {
                self.target().ok()
            },
            has_next: if matches!(self.route, Route::Agent(_)) {
                self.result
                    .as_ref()
                    .is_some_and(|result| result.cursor.is_some())
            } else if matches!(self.route, Route::Catalog(_)) {
                self.next.is_some()
            } else {
                false
            },
            busy: self.pending.is_some(),
            result: self.result.as_ref(),
            error: self.error,
            form: self.form.as_ref().map(Form::view),
            process_owner: self.process_owner.clone(),
            processes: self.processes.as_ref(),
        })
    }
    fn change(&mut self, editor_changed: bool) -> Result<(), Error> {
        let revision = self.revision.checked_add(1).ok_or(Error::Exhausted)?;
        let epoch = self
            .epoch
            .checked_add(u64::from(editor_changed))
            .ok_or(Error::Exhausted)?;
        self.revision = revision;
        self.epoch = epoch;
        self.displayed = None;
        Ok(())
    }
    fn target(&self) -> Result<&NativeManagedCatalogEntry, Error> {
        self.selected
            .and_then(|index| self.rows.get(index))
            .ok_or(Error::NoSelection)
    }
    pub(super) fn acknowledge(&mut self, frame: &NativeManagedFrameIdentity) -> Result<(), Error> {
        if !self.open || *frame != self.frame() {
            return Err(Error::StaleFrame);
        }
        self.displayed = Some(frame.clone());
        if self.route == Route::Skills {
            self.drafts
                .acknowledge_skill(&self.target()?.observation.clone())?;
        }
        Ok(())
    }
    pub(super) fn invalidate(&mut self) -> Result<(), Error> {
        self.change(false)
    }
    pub(super) fn close(&mut self) {
        self.drafts.close_skills();
        self.open = false;
        self.displayed = None;
        self.rows.clear();
        self.selected = None;
        self.start = None;
        self.next = None;
        self.result = None;
        self.error = None;
        self.form = None;
        self.models.chosen = None;
        self.process_owner = None;
        self.processes = None;
        self.history.clear();
        if let Some(Pending::History { request, .. }) = &self.pending {
            request.cancel();
        }
        if let Some(
            Pending::Command { cancellation, .. } | Pending::Processes { cancellation, .. },
        ) = &self.pending
        {
            cancellation.cancel();
        }
        // Keep each original request/future until the native owner settles it.
    }
    pub(super) fn open(&mut self, owner: &mut NativeInteractiveSession) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        self.change(true)?;
        self.open = true;
        self.filter = NativeManagedCatalogFilter::Current;
        self.route = Route::Catalog(self.filter);
        self.form = None;
        self.rows.clear();
        self.selected = None;
        self.start = None;
        self.next = None;
        self.result = None;
        self.error = None;
        self.process_owner = None;
        self.processes = None;
        let result = self.catalog(owner);
        if let Err(error) = result {
            self.error = Some(error);
        }
        result
    }
    fn catalog(&mut self, owner: &mut NativeInteractiveSession) -> Result<(), Error> {
        let request = owner
            .request_managed_catalog(self.filter, self.start.clone(), 16)
            .map_err(|_| Error::Unavailable)?;
        self.pending = Some(Pending::Catalog {
            request,
            epoch: self.epoch,
        });
        Ok(())
    }
    fn history(&mut self, owner: &mut NativeInteractiveSession) -> Result<(), Error> {
        // Release the prior charged record before reserving the reader's slot.
        // Only the bounded source anchors survive a refresh or route change.
        self.history.clear();
        let request = owner
            .request_managed_history(self.target()?.observation.clone())
            .map_err(|_| Error::Unavailable)?;
        self.pending = Some(Pending::History {
            request,
            epoch: self.epoch,
        });
        Ok(())
    }
    fn command(
        &mut self,
        owner: &mut NativeInteractiveSession,
        command: ManagedSubagentCommand,
    ) -> Result<(), Error> {
        let draft = match &command {
            ManagedSubagentCommand::Message(ManagedMessage::Send(message)) => self
                .drafts
                .submission(&self.target()?.observation, &message.content),
            _ => None,
        };
        let cancellation = CancellationToken::new();
        let skills = match &command {
            ManagedSubagentCommand::Message(ManagedMessage::Send(message)) => {
                self.skill_references(owner, &message.content)?
            }
            _ => Vec::new(),
        };
        let response = if matches!(command, ManagedSubagentCommand::Create(_)) {
            owner.request_managed_command(command, cancellation.clone())
        } else {
            owner.request_observed_managed_command_with_skills(
                self.target()?.observation.clone(),
                command,
                &skills,
                cancellation.clone(),
            )
        }
        .map_err(|_| Error::Unavailable)?;
        self.result = None;
        self.pending = Some(Pending::Command {
            response,
            cancellation,
            epoch: self.epoch,
            draft,
        });
        Ok(())
    }
    fn inspect(
        &mut self,
        owner: &mut NativeInteractiveSession,
        section: ManagedInspectSection,
        cursor: Option<String>,
    ) -> Result<(), Error> {
        self.command(
            owner,
            ManagedSubagentCommand::Inspect(ManagedInspect {
                id: self.target()?.id.clone(),
                sections: vec![section],
                cursor,
                limit: 16,
                wait: None,
            }),
        )
    }

    fn processes(&mut self, owner: &NativeInteractiveSession, scope: Scope) -> Result<(), Error> {
        let cancellation = CancellationToken::new();
        let observed = match scope {
            Scope::Parent => None,
            Scope::SelectedAgent => Some(&self.target()?.observation),
        };
        let (principal, future) =
            super::processes::snapshot(owner, observed, cancellation.clone())?;
        self.route = Route::Processes(scope);
        self.result = None;
        self.processes = None;
        self.process_owner = Some(principal);
        self.pending = Some(Pending::Processes {
            future,
            cancellation,
            epoch: self.epoch,
        });
        Ok(())
    }

    pub(super) fn edit_draft(
        &mut self,
        identity: &NativeManagedEditorIdentity,
        value: &str,
        cursor: usize,
    ) -> Result<(), Error> {
        if !self.open || *identity != self.frame().editor {
            return Err(Error::StaleFrame);
        }
        if !matches!(self.route, Route::Agent(_) | Route::Conversation) {
            return Err(Error::InvalidAction);
        }
        let target = self.target()?.observation.clone();
        self.drafts.replace(&target, value, cursor)
    }

    pub(super) fn edit_form(
        &mut self,
        identity: &NativeManagedEditorIdentity,
        value: &str,
    ) -> Result<(), Error> {
        if !self.open || *identity != self.frame().editor {
            return Err(Error::StaleFrame);
        }
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        let form = self.form.as_mut().ok_or(Error::InvalidAction)?;
        if form.current().byte_limit().is_none() {
            return Err(Error::InvalidAction);
        }
        let result = form.replace(form.current(), value).map_err(Error::Form);
        self.change(false)?;
        self.error = result.as_ref().err().copied();
        result
    }
    #[allow(clippy::too_many_lines)] // Exhaustive routing; all effects use the existing bounded native admissions.
    pub(super) fn action(
        &mut self,
        owner: &mut NativeInteractiveSession,
        frame: &NativeManagedFrameIdentity,
        action: Action,
    ) -> Result<(), Error> {
        if !self.open || *frame != self.frame() {
            return Err(Error::StaleFrame);
        }
        if self.displayed.as_ref() != Some(frame) {
            return Err(Error::NotDisplayed);
        }
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        // Validate selection/action before consuming the displayed frame.
        match &action {
            Action::Select
            | Action::Skills
            | Action::Models
            | Action::Conversation
            | Action::SeekHistory(_)
            | Action::HistoryMode(_)
            | Action::Inspect(_)
            | Action::Message(_)
            | Action::Configure(_)
            | Action::Relationship { .. }
            | Action::Lifecycle(_)
            | Action::ConfirmClose
            | Action::Processes(Scope::SelectedAgent)
            | Action::OpenForm(NativeManagedFormKind::Configure) => {
                self.target()?;
            }
            _ => {}
        }
        if matches!(action, Action::ConfirmClose) && self.route != Route::ConfirmClose {
            return Err(Error::InvalidAction);
        }
        if self.route == Route::Models
            && !matches!(
                action,
                Action::Previous | Action::Next | Action::Select | Action::Back | Action::Refresh
            )
        {
            return Err(Error::InvalidAction);
        }
        if self.route == Route::Skills
            && !matches!(
                action,
                Action::Previous | Action::Next | Action::Select | Action::Back
            )
        {
            return Err(Error::InvalidAction);
        }
        if matches!(action, Action::SeekHistory(_) | Action::HistoryMode(_))
            && self.route != Route::Conversation
        {
            return Err(Error::InvalidAction);
        }
        if self.route == Route::ConfirmClose
            && !matches!(action, Action::ConfirmClose | Action::Back)
        {
            return Err(Error::InvalidAction);
        }
        // Process rows carry no command or lifecycle authority. In particular,
        // Ctrl-C and Enter cannot cancel/message the underlying selected agent.
        if matches!(self.route, Route::Processes(_))
            && !matches!(
                action,
                Action::Back | Action::Refresh | Action::Filter(_) | Action::Processes(_)
            )
        {
            return Err(Error::InvalidAction);
        }
        if matches!(self.route, Route::Form(_))
            && !matches!(
                action,
                Action::Previous
                    | Action::Next
                    | Action::CycleFormField
                    | Action::SubmitForm
                    | Action::Back
                    | Action::Refresh
            )
        {
            return Err(Error::InvalidAction);
        }
        let form_command = if matches!(action, Action::SubmitForm) {
            match self.form.as_mut().ok_or(Error::InvalidAction)?.command() {
                Ok(command) => Some(command),
                Err(error) => {
                    self.change(false)?;
                    self.error = Some(Error::Form(error));
                    return Err(Error::Form(error));
                }
            }
        } else {
            None
        };
        if matches!(
            action,
            Action::Previous | Action::Next | Action::CycleFormField
        ) && let Some(form) = &self.form
        {
            form.can_leave_field().map_err(Error::Form)?;
        }
        if let Action::Configure(configuration) = &action
            && configuration.id != self.target()?.id
        {
            return Err(Error::InvalidAction);
        }
        let editor_changed = !matches!(
            action,
            Action::Previous
                | Action::Next
                | Action::Refresh
                | Action::SeekHistory(_)
                | Action::HistoryMode(_)
        ) || (matches!(self.route, Route::Form(_))
            && matches!(action, Action::Previous | Action::Next));
        self.change(editor_changed)?;
        self.error = None;
        if !matches!(action, Action::Processes(_) | Action::Refresh) {
            self.process_owner = None;
            self.processes = None;
        }
        if matches!(
            action,
            Action::Back
                | Action::Filter(_)
                | Action::Inspect(_)
                | Action::Processes(_)
                | Action::OpenForm(_)
                | Action::Models
                | Action::Skills
                | Action::Lifecycle(ManagedLifecycleAction::Close)
        ) {
            self.history.clear();
        }
        let result = match action {
            Action::Skills => self.open_skills(),
            Action::Models => self.open_models(owner),
            Action::Exit => {
                self.close();
                owner.request_shutdown();
                Ok(())
            }
            Action::SeekHistory(position) => self.history.seek(position),
            Action::HistoryMode(mode) => self.history.set_mode(mode),
            Action::Previous | Action::Next => {
                if self.route == Route::Skills {
                    return self.drafts.move_skill(
                        &self.target()?.observation.clone(),
                        matches!(action, Action::Next),
                    );
                }
                if self.route == Route::Models {
                    self.models
                        .picker
                        .move_selection(matches!(action, Action::Previous));
                    return Ok(());
                }
                if let Some(form) = &mut self.form {
                    return form
                        .select(matches!(action, Action::Previous))
                        .map_err(Error::Form);
                }
                if !matches!(self.route, Route::Catalog(_)) {
                    return Err(Error::InvalidAction);
                }
                if !self.rows.is_empty() {
                    let current = self.selected.unwrap_or(0);
                    self.selected = Some(if matches!(action, Action::Previous) {
                        current.saturating_sub(1)
                    } else {
                        (current + 1).min(self.rows.len() - 1)
                    });
                }
                Ok(())
            }
            Action::Filter(filter) => {
                self.filter = filter;
                self.route = Route::Catalog(filter);
                self.start = None;
                self.result = None;
                self.catalog(owner)
            }
            Action::Refresh => {
                self.history_retry = None;
                match self.route {
                    Route::Models => {
                        self.models.load(&owner.options, true);
                        Ok(())
                    }
                    Route::Processes(scope) => self.processes(owner, scope),
                    _ => self.catalog(owner),
                }
            }
            Action::Processes(scope) => self.processes(owner, scope),
            Action::NextPage => match self.route {
                Route::Catalog(_) => {
                    self.start = Some(self.next.clone().ok_or(Error::NoSelection)?);
                    self.catalog(owner)
                }
                Route::Agent(section) => {
                    let cursor = self
                        .result
                        .as_ref()
                        .and_then(|result| result.cursor.clone())
                        .ok_or(Error::NoSelection)?;
                    self.inspect(owner, section, Some(cursor))
                }
                Route::Conversation
                | Route::Skills
                | Route::Models
                | Route::ConfirmClose
                | Route::Form(_)
                | Route::Processes(_) => Err(Error::InvalidAction),
            },
            Action::Select if self.route == Route::Models => self.select_model(owner),
            Action::Select if self.route == Route::Skills => {
                self.drafts
                    .choose_skill(&self.target()?.observation.clone())?;
                self.route = Route::Conversation;
                self.history(owner)
            }
            Action::Select | Action::Conversation => {
                self.route = Route::Conversation;
                self.result = None;
                self.history_retry = None;
                self.history(owner)
            }
            Action::Inspect(section) => {
                self.route = Route::Agent(section);
                self.inspect(owner, section, None)
            }
            Action::Back => {
                if self.route == Route::Skills {
                    self.drafts.close_skills();
                    self.route = Route::Conversation;
                    return self.history(owner);
                }
                if self.route == Route::Models {
                    self.route = Route::Conversation;
                    return self.history(owner);
                }
                if matches!(self.route, Route::Catalog(_)) {
                    self.close();
                } else {
                    self.route = Route::Catalog(self.filter);
                    self.result = None;
                    self.form = None;
                }
                Ok(())
            }
            Action::Create(create) => self.command(owner, ManagedSubagentCommand::Create(create)),
            Action::OpenForm(kind) => {
                self.models.chosen = None;
                self.route = Route::Form(kind);
                self.result = None;
                if kind == NativeManagedFormKind::Create {
                    self.form = Some(Form::create());
                    Ok(())
                } else {
                    self.form = None;
                    self.inspect(owner, ManagedInspectSection::Configuration, None)
                }
            }
            Action::CycleFormField => self
                .form
                .as_mut()
                .ok_or(Error::InvalidAction)?
                .cycle()
                .map_err(Error::Form),
            Action::SubmitForm => self.command(owner, form_command.ok_or(Error::InvalidAction)?),
            Action::Configure(configure) => {
                self.command(owner, ManagedSubagentCommand::Configure(configure))
            }
            Action::Message(content) => self.command(
                owner,
                ManagedSubagentCommand::Message(ManagedMessage::Send(ManagedSend {
                    id: self.target()?.id.clone(),
                    content,
                })),
            ),
            Action::Relationship { action, parent_id } => self.command(
                owner,
                ManagedSubagentCommand::Relationship(ManagedRelationship {
                    id: self.target()?.id.clone(),
                    action,
                    parent_id,
                }),
            ),
            Action::Lifecycle(ManagedLifecycleAction::Close) => {
                self.route = Route::ConfirmClose;
                self.result = None;
                Ok(())
            }
            Action::Lifecycle(action) => self.command(
                owner,
                ManagedSubagentCommand::Lifecycle(ManagedLifecycle {
                    id: self.target()?.id.clone(),
                    action,
                }),
            ),
            Action::ConfirmClose => self.command(
                owner,
                ManagedSubagentCommand::Lifecycle(ManagedLifecycle {
                    id: self.target()?.id.clone(),
                    action: ManagedLifecycleAction::Close,
                }),
            ),
        };
        if let Err(error) = result {
            self.error = Some(error);
        }
        result
    }
    pub(super) fn poll(&mut self, owner: &mut NativeInteractiveSession, cx: &mut Context<'_>) {
        if self.models.poll(cx)
            && self.open
            && self.route == Route::Models
            && self.change(false).is_err()
        {
            self.close();
        }
        Self::hydrate_catalog(owner);
        let Some(pending) = self.pending.take() else {
            self.refresh_changed_history(owner, cx);
            return;
        };
        let mut refresh = None;
        let mut editor_changed = false;
        let epoch = match pending {
            Pending::History { request, epoch } => {
                let Some(outcome) = owner.take_managed_history_outcome() else {
                    self.pending = Some(Pending::History { request, epoch });
                    return;
                };
                if outcome.request != request {
                    self.error = Some(Error::Unavailable);
                } else if self.open && epoch == self.epoch {
                    refresh = self
                        .accept_history(owner, outcome.result)
                        .then_some(Refresh::Catalog);
                }
                epoch
            }
            Pending::Processes {
                mut future,
                cancellation,
                epoch,
            } => {
                let Poll::Ready(result) = future.as_mut().poll(cx) else {
                    self.pending = Some(Pending::Processes {
                        future,
                        cancellation,
                        epoch,
                    });
                    return;
                };
                if self.open && epoch == self.epoch {
                    match result {
                        Ok(snapshot) => self.processes = Some(snapshot),
                        Err(error) => self.error = Some(error),
                    }
                }
                epoch
            }
            Pending::Catalog { request, epoch } => {
                let Some(outcome) = owner.take_managed_catalog_outcome() else {
                    self.pending = Some(Pending::Catalog { request, epoch });
                    return;
                };
                if outcome.request != request {
                    self.error = Some(Error::Unavailable);
                } else if self.open && epoch == self.epoch {
                    match outcome.result {
                        Ok(page) => {
                            self.replace_page(page);
                            refresh = match self.route {
                                Route::Agent(section) => Some(Refresh::Inspect(section)),
                                Route::Conversation => Some(Refresh::History),
                                _ => None,
                            };
                        }
                        Err(_) => self.error = Some(Error::Unavailable),
                    }
                }
                epoch
            }
            Pending::Command {
                mut response,
                cancellation,
                epoch,
                draft,
            } => {
                let Poll::Ready(result) = response.as_mut().poll(cx) else {
                    self.pending = Some(Pending::Command {
                        response,
                        cancellation,
                        epoch,
                        draft,
                    });
                    return;
                };
                if mutation_receipt(&result) {
                    // A successful mutation advances the durable head. Never
                    // acknowledge another command against the pre-mutation row.
                    refresh = Some(Refresh::Catalog);
                }
                editor_changed = self.accept_command(result, draft, epoch);
                epoch
            }
        };
        self.finish_poll(owner, epoch, editor_changed, refresh);
        cx.waker().wake_by_ref();
    }
    fn finish_poll(
        &mut self,
        owner: &mut NativeInteractiveSession,
        epoch: u64,
        editor_changed: bool,
        refresh: Option<Refresh>,
    ) {
        if !self.open || epoch != self.epoch {
            return;
        }
        if self.change(editor_changed).is_err() {
            self.close();
            return;
        }
        let result = match refresh {
            Some(Refresh::Inspect(section)) => self.inspect(owner, section, None),
            Some(Refresh::History) => self.history(owner),
            Some(Refresh::Catalog) => self.catalog(owner),
            None => Ok(()),
        };
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
    fn refresh_changed_history(
        &mut self,
        owner: &mut NativeInteractiveSession,
        cx: &mut Context<'_>,
    ) {
        // Observe the original resident generation only. No runtime is loaded,
        // and an unchanged record never schedules a polling loop.
        if self.open
            && self.route == Route::Conversation
            && let Some(history) = self.history.view()
            && let Ok(target) = self.target()
            && owner
                .managed
                .as_ref()
                .and_then(|managed| managed.agents.observed_runtime(&target.observation))
                .map_or(self.history_resident, |runtime| {
                    runtime.record_snapshot().revision != history.record.revision
                })
        {
            self.history_retry = None;
            if self
                .change(false)
                .and_then(|()| self.catalog(owner))
                .is_err()
            {
                self.error = Some(Error::Unavailable);
                self.history.clear();
            }
            cx.waker().wake_by_ref();
        }
    }

    fn accept_history(
        &mut self,
        owner: &NativeInteractiveSession,
        result: Result<crate::NativeManagedHistorySnapshot, crate::NativeManagedHistoryError>,
    ) -> bool {
        match result {
            Ok(snapshot) => {
                self.history_resident = owner
                    .managed
                    .as_ref()
                    .and_then(|managed| managed.agents.observed_runtime(snapshot.observation()))
                    .is_some();
                self.history.install(snapshot);
            }
            Err(crate::NativeManagedHistoryError::Stale) => {
                // Child completion and notification settlement may each advance
                // the head between catalog and history reads. Refresh while the
                // rejected head advances, not only once per conversation. An
                // unchanged rejected observation never creates a polling loop.
                if let Ok(target) = self.target()
                    && self
                        .history_retry
                        .is_none_or(|revision| target.revision > revision)
                {
                    self.history_retry = Some(target.revision);
                    return true;
                }
                self.error = Some(Error::Unavailable);
            }
            Err(_) => self.error = Some(Error::Unavailable),
        }
        false
    }

    fn accept_command(
        &mut self,
        result: Result<ManagedSubagentResult, machine_god_core::ManagedSubagentError>,
        draft: Option<super::drafts::Submission>,
        epoch: u64,
    ) -> bool {
        if result.as_ref().is_ok_and(|result| result.ok)
            && let Some(draft) = draft
        {
            // Closing or changing presentation does not undo durable acceptance.
            // A newer unsent edit is never cleared here.
            self.drafts.accepted(&draft);
        }
        if !self.open || epoch != self.epoch {
            return false;
        }
        match result {
            Ok(result) => self.result = Some(result),
            Err(_) => self.error = Some(Error::Unavailable),
        }
        let editor_changed = self.finish_form_command();
        // Confirmation is a single submission, not a reusable button. Its
        // receipt retires the confirmation editor and any buffered input.
        if self.route == Route::ConfirmClose {
            self.route = Route::Agent(ManagedInspectSection::Status);
            true
        } else {
            editor_changed
        }
    }
    fn replace_page(&mut self, page: NativeManagedCatalogPage) {
        let previous = self
            .target()
            .ok()
            .map(|target| (target.id.clone(), target.generation));
        self.result = None;
        self.rows = page.entries;
        self.next = page.next;
        self.selected = previous
            .as_ref()
            .and_then(|(id, _)| self.rows.iter().position(|row| &row.id == id))
            .or_else(|| (!self.rows.is_empty()).then_some(0));
        // An absent target never substitutes another agent in a retained detail view.
        if matches!(
            self.route,
            Route::Conversation
                | Route::Models
                | Route::Skills
                | Route::Agent(_)
                | Route::ConfirmClose
                | Route::Form(NativeManagedFormKind::Configure)
        ) && self
            .target()
            .ok()
            .map(|target| (&target.id, target.generation))
            != previous.as_ref().map(|(id, generation)| (id, *generation))
        {
            self.route = Route::Catalog(self.filter);
            self.result = None;
            self.form = None;
            self.history.clear();
            if self.change(true).is_err() {
                self.close();
            }
        }
    }

    fn finish_form_command(&mut self) -> bool {
        let Route::Form(kind) = self.route else {
            return false;
        };
        if self.form.is_none() {
            let configuration = self
                .result
                .as_ref()
                .filter(|result| result.ok)
                .and_then(|result| result.requested.as_ref())
                .and_then(|requested| match requested {
                    machine_god_core::ManagedRequested::Inspection(inspection) => {
                        inspection.configuration.clone()
                    }
                    _ => None,
                });
            match configuration.and_then(|configuration| {
                self.target()
                    .ok()
                    .and_then(|target| Form::configure(target.id.clone(), configuration).ok())
            }) {
                Some(mut form) => {
                    if let Some(model) = self.models.chosen.take()
                        && let Err(error) =
                            form.replace(crate::NativeManagedFormField::Model, &model)
                    {
                        self.error = Some(Error::Form(error));
                    }
                    self.form = Some(form);
                }
                None => self.route = Route::Agent(ManagedInspectSection::Configuration),
            }
        } else if self.result.as_ref().is_some_and(|result| result.ok) {
            self.form = None;
            self.route = match kind {
                NativeManagedFormKind::Create => Route::Catalog(self.filter),
                NativeManagedFormKind::Configure => Route::Agent(ManagedInspectSection::Status),
            };
        }
        // Input received while a submitted command was pending cannot become
        // edits in its settled/retry editor, including rejected commands.
        true
    }
}
