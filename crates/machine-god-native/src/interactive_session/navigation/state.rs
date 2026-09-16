//! Native page ownership and bounded asynchronous navigation work.
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
use crate::{NativeManagedEditorIdentity, NativeManagedFormKind};
use machine_god_core::{
    CancellationToken, ManagedInspect, ManagedInspectSection, ManagedLifecycle,
    ManagedLifecycleAction, ManagedMessage, ManagedRelationship, ManagedSend,
    ManagedSubagentCommand, ManagedSubagentResult,
};
use std::{
    sync::Arc,
    task::{Context, Poll},
};

type Error = NativeManagedNavigationError;
type Action = NativeManagedNavigationAction;
type Route = NativeManagedNavigationRoute;

enum Pending {
    Catalog {
        request: NativeManagedCatalogRequest,
        epoch: u64,
    },
    Command {
        response: NativeManagedCommandResponse,
        cancellation: CancellationToken,
        epoch: u64,
    },
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
        }
    }
}

impl Navigation {
    pub(super) fn owns_catalog(&self) -> bool {
        matches!(self.pending, Some(Pending::Catalog { .. }))
    }
    fn frame(&self) -> NativeManagedFrameIdentity {
        NativeManagedFrameIdentity {
            editor: editor(&self.identity, self.epoch),
            revision: self.revision,
        }
    }
    pub(super) fn view(&self) -> Option<NativeManagedNavigationView<'_>> {
        self.open.then(|| NativeManagedNavigationView {
            editor: editor(&self.identity, self.epoch),
            frame: self.frame(),
            route: self.route,
            rows: &self.rows,
            selected: self.selected,
            target: self.target().ok(),
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
        Ok(())
    }
    pub(super) fn invalidate(&mut self) -> Result<(), Error> {
        self.change(false)
    }
    pub(super) fn close(&mut self) {
        self.open = false;
        self.displayed = None;
        self.rows.clear();
        self.selected = None;
        self.start = None;
        self.next = None;
        self.result = None;
        self.error = None;
        self.form = None;
        if let Some(Pending::Command { cancellation, .. }) = &self.pending {
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
    fn command(
        &mut self,
        owner: &mut NativeInteractiveSession,
        command: ManagedSubagentCommand,
    ) -> Result<(), Error> {
        let cancellation = CancellationToken::new();
        let response = if matches!(command, ManagedSubagentCommand::Create(_)) {
            owner.request_managed_command(command, cancellation.clone())
        } else {
            owner.request_observed_managed_command(
                self.target()?.observation.clone(),
                command,
                cancellation.clone(),
            )
        }
        .map_err(|_| Error::Unavailable)?;
        self.result = None;
        self.pending = Some(Pending::Command {
            response,
            cancellation,
            epoch: self.epoch,
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
            | Action::Inspect(_)
            | Action::Message(_)
            | Action::Configure(_)
            | Action::Relationship { .. }
            | Action::Lifecycle(_)
            | Action::ConfirmClose
            | Action::OpenForm(NativeManagedFormKind::Configure) => {
                self.target()?;
            }
            _ => {}
        }
        if matches!(action, Action::ConfirmClose) && self.route != Route::ConfirmClose {
            return Err(Error::InvalidAction);
        }
        if self.route == Route::ConfirmClose
            && !matches!(action, Action::ConfirmClose | Action::Back)
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
        let editor_changed = !matches!(action, Action::Previous | Action::Next | Action::Refresh)
            || (matches!(self.route, Route::Form(_))
                && matches!(action, Action::Previous | Action::Next));
        self.change(editor_changed)?;
        self.error = None;
        let result = match action {
            Action::Previous | Action::Next => {
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
            Action::Refresh => self.catalog(owner),
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
                Route::ConfirmClose | Route::Form(_) => Err(Error::InvalidAction),
            },
            Action::Select | Action::Inspect(ManagedInspectSection::Status) => {
                self.route = Route::Agent(ManagedInspectSection::Status);
                self.inspect(owner, ManagedInspectSection::Status, None)
            }
            Action::Inspect(section) => {
                self.route = Route::Agent(section);
                self.inspect(owner, section, None)
            }
            Action::Back => {
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
        let Some(pending) = self.pending.take() else {
            return;
        };
        let mut refresh_detail = None;
        let mut editor_changed = false;
        let epoch = match pending {
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
                            if let Route::Agent(section) = self.route {
                                refresh_detail = Some(section);
                            }
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
            } => {
                let Poll::Ready(result) = response.as_mut().poll(cx) else {
                    self.pending = Some(Pending::Command {
                        response,
                        cancellation,
                        epoch,
                    });
                    return;
                };
                if self.open && epoch == self.epoch {
                    match result {
                        Ok(result) => self.result = Some(result),
                        Err(_) => self.error = Some(Error::Unavailable),
                    }
                    editor_changed = self.finish_form_command();
                    // Confirmation is a single submission, not a reusable button.
                    // Show its receipt/rejection on the detail route and retire
                    // any bytes captured for the confirmation editor.
                    if self.route == Route::ConfirmClose {
                        self.route = Route::Agent(ManagedInspectSection::Status);
                        editor_changed = true;
                    }
                }
                epoch
            }
        };
        if self.open && epoch == self.epoch && self.change(editor_changed).is_err() {
            self.close();
        }
        if self.open
            && epoch == self.epoch
            && let Some(section) = refresh_detail
            && let Err(error) = self.inspect(owner, section, None)
        {
            self.error = Some(error);
        }
        cx.waker().wake_by_ref();
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
            Route::Agent(_) | Route::ConfirmClose | Route::Form(NativeManagedFormKind::Configure)
        ) && self
            .target()
            .ok()
            .map(|target| (&target.id, target.generation))
            != previous.as_ref().map(|(id, generation)| (id, *generation))
        {
            self.route = Route::Catalog(self.filter);
            self.result = None;
            self.form = None;
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
                Some(form) => self.form = Some(form),
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
