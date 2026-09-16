//! Bounded native form drafts. Editing/validation never grants execution authority.
mod command;
#[cfg(test)]
mod tests;

use machine_god_core::{
    MAX_SUBAGENT_MILESTONES, MAX_SUBAGENT_MODEL_BYTES, MAX_SUBAGENT_NAME_BYTES,
    MAX_SUBAGENT_PROMPT_BYTES, ManagedAgentMode, ManagedConfiguration, ManagedNotifications,
    ManagedPermissionMode, ManagedSubagentCommand,
};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeManagedFormKind {
    Create,
    Configure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeManagedFormField {
    Name,
    Mode,
    Model,
    Prompt,
    Effort,
    Permission,
    Completed,
    Failed,
    Cancelled,
    Started,
    Milestones,
    Interval,
    Duration,
}

const CREATE_FIELDS: &[NativeManagedFormField] = &[
    NativeManagedFormField::Name,
    NativeManagedFormField::Mode,
    NativeManagedFormField::Model,
    NativeManagedFormField::Prompt,
    NativeManagedFormField::Effort,
    NativeManagedFormField::Permission,
    NativeManagedFormField::Completed,
    NativeManagedFormField::Failed,
    NativeManagedFormField::Cancelled,
    NativeManagedFormField::Started,
    NativeManagedFormField::Milestones,
    NativeManagedFormField::Interval,
    NativeManagedFormField::Duration,
];
const CONFIGURE_FIELDS: &[NativeManagedFormField] = &[
    NativeManagedFormField::Name,
    NativeManagedFormField::Model,
    NativeManagedFormField::Effort,
    NativeManagedFormField::Permission,
    NativeManagedFormField::Completed,
    NativeManagedFormField::Failed,
    NativeManagedFormField::Cancelled,
    NativeManagedFormField::Started,
    NativeManagedFormField::Milestones,
    NativeManagedFormField::Interval,
    NativeManagedFormField::Duration,
];

impl NativeManagedFormField {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Mode => "Mode",
            Self::Model => "Model (blank inherits on create)",
            Self::Prompt => "Standalone initial prompt",
            Self::Effort => "Reasoning effort",
            Self::Permission => "Permission mode",
            Self::Completed => "Notify completion",
            Self::Failed => "Notify failure",
            Self::Cancelled => "Notify cancellation",
            Self::Started => "Notify start",
            Self::Milestones => "Milestones (JSON string array)",
            Self::Interval => "Report interval (positive milliseconds)",
            Self::Duration => "Report duration (positive milliseconds)",
        }
    }
    #[must_use]
    pub const fn byte_limit(self) -> Option<usize> {
        match self {
            Self::Name => Some(MAX_SUBAGENT_NAME_BYTES),
            Self::Model => Some(MAX_SUBAGENT_MODEL_BYTES),
            Self::Prompt => Some(MAX_SUBAGENT_PROMPT_BYTES),
            Self::Effort => Some(crate::MAX_NATIVE_REASONING_EFFORT_BYTES),
            Self::Milestones => {
                Some(MAX_SUBAGENT_MILESTONES * (MAX_SUBAGENT_NAME_BYTES * 6 + 3) + 2)
            }
            Self::Interval | Self::Duration => Some(20),
            _ => None,
        }
    }
    const fn text_index(self) -> Option<usize> {
        match self {
            Self::Name => Some(0),
            Self::Model => Some(1),
            Self::Prompt => Some(2),
            Self::Effort => Some(3),
            Self::Milestones => Some(4),
            Self::Interval => Some(5),
            Self::Duration => Some(6),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeManagedFormError {
    InvalidField,
    TooLarge(NativeManagedFormField),
    InvalidValue(NativeManagedFormField),
    InvalidNotifications,
    NoChanges,
}
impl fmt::Display for NativeManagedFormError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField => f.write_str("field is not editable in this form"),
            Self::TooLarge(field) => write!(f, "{} exceeds its byte limit", field.label()),
            Self::InvalidValue(field) => write!(f, "{} is invalid", field.label()),
            Self::InvalidNotifications => f.write_str("notification policy is invalid"),
            Self::NoChanges => f.write_str("no configuration changes selected"),
        }
    }
}
impl std::error::Error for NativeManagedFormError {}

/// Values are unsanitized display data, never authority or a submitted command.
pub struct NativeManagedFormView<'a> {
    pub kind: NativeManagedFormKind,
    pub fields: &'static [NativeManagedFormField],
    pub selected: usize,
    pub values: [&'a str; 13],
    pub error: Option<NativeManagedFormError>,
}
impl fmt::Debug for NativeManagedFormView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeManagedFormView")
            .field("kind", &self.kind)
            .field("selected", &self.selected)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

pub(super) struct Form {
    kind: NativeManagedFormKind,
    target: Option<String>,
    selected: usize,
    text: [String; 7],
    changed: [bool; 7],
    mode: ManagedAgentMode,
    permission: Option<ManagedPermissionMode>,
    permission_changed: bool,
    notices: ManagedNotifications,
    notices_changed: bool,
    error: Option<NativeManagedFormError>,
    rejected_edit: Option<NativeManagedFormError>,
}
impl fmt::Debug for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedAgentForm")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl Form {
    pub(super) fn create() -> Self {
        Self {
            kind: NativeManagedFormKind::Create,
            target: None,
            selected: 0,
            text: std::array::from_fn(|_| String::new()),
            changed: [false; 7],
            mode: ManagedAgentMode::Persistent,
            permission: None,
            permission_changed: false,
            notices: ManagedNotifications::default(),
            notices_changed: false,
            error: None,
            rejected_edit: None,
        }
    }

    pub(super) fn configure(
        id: String,
        configuration: ManagedConfiguration,
    ) -> Result<Self, NativeManagedFormError> {
        if id.len() > 255
            || configuration.name.len() > MAX_SUBAGENT_NAME_BYTES
            || configuration
                .model
                .as_ref()
                .is_some_and(|value| value.len() > MAX_SUBAGENT_MODEL_BYTES)
            || configuration
                .effort
                .as_ref()
                .is_some_and(|value| value.len() > crate::MAX_NATIVE_REASONING_EFFORT_BYTES)
            || configuration.notifications.milestones.len() > MAX_SUBAGENT_MILESTONES
            || configuration
                .notifications
                .milestones
                .iter()
                .any(|name| name.len() > MAX_SUBAGENT_NAME_BYTES)
        {
            return Err(NativeManagedFormError::InvalidField);
        }
        let mut validation =
            ManagedSubagentCommand::Configure(machine_god_core::ManagedConfigure {
                id: id.clone(),
                name: Some(configuration.name.clone()),
                model: configuration.model.clone(),
                effort: configuration.effort.clone(),
                permission_mode: Some(configuration.permission_mode),
                notifications: Some(configuration.notifications.clone()),
            });
        validation
            .normalize()
            .map_err(|_| NativeManagedFormError::InvalidField)?;
        let mut form = Self::create();
        form.kind = NativeManagedFormKind::Configure;
        form.target = Some(id);
        for (field, value) in [
            (NativeManagedFormField::Name, configuration.name),
            (
                NativeManagedFormField::Model,
                configuration.model.unwrap_or_default(),
            ),
            (
                NativeManagedFormField::Effort,
                configuration.effort.unwrap_or_default(),
            ),
            (
                NativeManagedFormField::Milestones,
                serde_json::to_string(&configuration.notifications.milestones)
                    .map_err(|_| NativeManagedFormError::InvalidNotifications)?,
            ),
            (
                NativeManagedFormField::Interval,
                configuration
                    .notifications
                    .report_interval_ms
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
            ),
            (
                NativeManagedFormField::Duration,
                configuration
                    .notifications
                    .report_duration_ms
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
            ),
        ] {
            form.replace(field, &value)?;
        }
        form.permission = Some(configuration.permission_mode);
        form.notices = configuration.notifications;
        form.changed.fill(false);
        Ok(form)
    }

    pub(super) fn fields(&self) -> &'static [NativeManagedFormField] {
        match self.kind {
            NativeManagedFormKind::Create => CREATE_FIELDS,
            NativeManagedFormKind::Configure => CONFIGURE_FIELDS,
        }
    }
    pub(super) fn current(&self) -> NativeManagedFormField {
        self.fields()[self.selected]
    }
    pub(super) fn select(&mut self, previous: bool) -> Result<(), NativeManagedFormError> {
        self.can_leave_field()?;
        self.selected = if previous {
            self.selected.saturating_sub(1)
        } else {
            (self.selected + 1).min(self.fields().len() - 1)
        };
        Ok(())
    }
    pub(super) fn can_leave_field(&self) -> Result<(), NativeManagedFormError> {
        self.rejected_edit.map_or(Ok(()), Err)
    }
    pub(super) fn replace(
        &mut self,
        field: NativeManagedFormField,
        value: &str,
    ) -> Result<(), NativeManagedFormError> {
        let result = self.replace_text(field, value);
        self.rejected_edit = result.as_ref().err().copied();
        self.error = self.rejected_edit;
        result
    }
    fn replace_text(
        &mut self,
        field: NativeManagedFormField,
        value: &str,
    ) -> Result<(), NativeManagedFormError> {
        if !self.fields().contains(&field) {
            return Err(NativeManagedFormError::InvalidField);
        }
        let index = field
            .text_index()
            .ok_or(NativeManagedFormError::InvalidField)?;
        if value.len()
            > field
                .byte_limit()
                .ok_or(NativeManagedFormError::InvalidField)?
        {
            return Err(NativeManagedFormError::TooLarge(field));
        }
        if value.contains('\0') {
            return Err(NativeManagedFormError::InvalidValue(field));
        }
        if self.text[index] != value {
            value.clone_into(&mut self.text[index]);
            self.changed[index] = true;
            self.error = None;
        }
        Ok(())
    }
    pub(super) fn cycle(&mut self) -> Result<(), NativeManagedFormError> {
        use NativeManagedFormField as Field;
        if let Some(error) = self.rejected_edit {
            return Err(error);
        }
        match self.current() {
            Field::Mode => {
                self.mode = match self.mode {
                    ManagedAgentMode::Persistent => ManagedAgentMode::OneOff,
                    ManagedAgentMode::OneOff => ManagedAgentMode::Persistent,
                }
            }
            Field::Permission => {
                self.permission = match self.permission {
                    Some(ManagedPermissionMode::Ask) => Some(ManagedPermissionMode::Auto),
                    Some(ManagedPermissionMode::Auto) => Some(ManagedPermissionMode::Yolo),
                    Some(ManagedPermissionMode::Yolo)
                        if self.kind == NativeManagedFormKind::Create =>
                    {
                        None
                    }
                    None | Some(ManagedPermissionMode::Yolo) => Some(ManagedPermissionMode::Ask),
                };
                self.permission_changed = true;
            }
            Field::Completed => {
                self.notices.terminal.completed ^= true;
                self.notices_changed = true;
            }
            Field::Failed => {
                self.notices.terminal.failed ^= true;
                self.notices_changed = true;
            }
            Field::Cancelled => {
                self.notices.terminal.cancelled ^= true;
                self.notices_changed = true;
            }
            Field::Started => {
                self.notices.started ^= true;
                self.notices_changed = true;
            }
            _ => return Err(NativeManagedFormError::InvalidField),
        }
        self.error = None;
        Ok(())
    }
    pub(super) fn view(&self) -> NativeManagedFormView<'_> {
        NativeManagedFormView {
            kind: self.kind,
            fields: self.fields(),
            selected: self.selected,
            values: std::array::from_fn(|index| {
                self.fields()
                    .get(index)
                    .map_or("", |field| self.value(*field))
            }),
            error: self.error,
        }
    }
    fn value(&self, field: NativeManagedFormField) -> &str {
        if let Some(index) = field.text_index() {
            return &self.text[index];
        }
        match field {
            NativeManagedFormField::Mode => match self.mode {
                ManagedAgentMode::Persistent => "persistent",
                ManagedAgentMode::OneOff => "one-off",
            },
            NativeManagedFormField::Permission => match self.permission {
                None => "inherit",
                Some(ManagedPermissionMode::Ask) => "ask",
                Some(ManagedPermissionMode::Auto) => "auto",
                Some(ManagedPermissionMode::Yolo) => "yolo",
            },
            NativeManagedFormField::Completed => flag(self.notices.terminal.completed),
            NativeManagedFormField::Failed => flag(self.notices.terminal.failed),
            NativeManagedFormField::Cancelled => flag(self.notices.terminal.cancelled),
            NativeManagedFormField::Started => flag(self.notices.started),
            _ => "",
        }
    }
}

const fn flag(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
