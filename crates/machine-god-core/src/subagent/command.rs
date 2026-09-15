use super::{
    DEFAULT_SUBAGENT_PAGE_LIMIT, MAX_SUBAGENT_ARGUMENT_BYTES, MAX_SUBAGENT_INSPECT_WAIT_MS,
    MAX_SUBAGENT_JSON_NODES, MAX_SUBAGENT_MESSAGE_BYTES, MAX_SUBAGENT_MILESTONES,
    MAX_SUBAGENT_MODEL_BYTES, MAX_SUBAGENT_NAME_BYTES, MAX_SUBAGENT_PAGE_LIMIT,
    MAX_SUBAGENT_PROMPT_BYTES, MAX_SUBAGENT_STOP_CONDITIONS, ManagedSubagentError, Value,
    drop_json_value_iterative, id_valid, json_bounded, serialized_json_size_bounded, text_valid,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
fn page_default() -> usize {
    DEFAULT_SUBAGENT_PAGE_LIMIT
}
fn terminal_default() -> Vec<ManagedStopCondition> {
    vec![ManagedStopCondition::Terminal]
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSubagentCommand {
    Create(ManagedCreate),
    Inspect(ManagedInspect),
    Message(ManagedMessage),
    Relationship(ManagedRelationship),
    Configure(ManagedConfigure),
    Lifecycle(ManagedLifecycle),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentMode {
    OneOff,
    Persistent,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedPermissionMode {
    Ask,
    Auto,
    Yolo,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedCreate {
    pub name: String,
    pub mode: ManagedAgentMode,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub prompt: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub model: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub effort: Option<String>,
    /// None means inherit the admitted parent's same-or-stricter policy.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub permission_mode: Option<ManagedPermissionMode>,
    #[serde(default)]
    pub notifications: ManagedNotifications,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedInspect {
    pub id: String,
    pub sections: Vec<ManagedInspectSection>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub cursor: Option<String>,
    #[serde(default = "page_default")]
    pub limit: usize,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub wait: Option<ManagedInspectWait>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedInspectSection {
    Status,
    Messages,
    ToolActivity,
    Events,
    Configuration,
    Relationship,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedInspectWait {
    pub until: ManagedWaitUntil,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_generation: Option<u64>,
    pub timeout_ms: u64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedWaitUntil {
    Settled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedMessage {
    Send(ManagedSend),
    Milestone(ManagedMilestone),
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedSend {
    pub id: String,
    pub content: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedMilestone {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedRelationship {
    pub action: ManagedRelationshipAction,
    pub id: String,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_id: Option<String>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedRelationshipAction {
    Attach,
    Detach,
    Reparent,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedConfigure {
    pub id: String,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub model: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub effort: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub permission_mode: Option<ManagedPermissionMode>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub notifications: Option<ManagedNotifications>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedLifecycle {
    pub id: String,
    pub action: ManagedLifecycleAction,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedLifecycleAction {
    Cancel,
    Resume,
    Close,
    Reopen,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedNotifications {
    #[serde(default)]
    pub terminal: ManagedTerminalNotifications,
    /// Explicit modern extension: notify start without starting an idle parent.
    #[serde(default)]
    pub started: bool,
    #[serde(default)]
    pub milestones: Vec<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub report_interval_ms: Option<u64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub report_duration_ms: Option<u64>,
    #[serde(default = "terminal_default")]
    pub stop_conditions: Vec<ManagedStopCondition>,
}
impl Default for ManagedNotifications {
    fn default() -> Self {
        Self {
            terminal: ManagedTerminalNotifications::default(),
            started: false,
            milestones: vec![],
            report_interval_ms: None,
            report_duration_ms: None,
            stop_conditions: terminal_default(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedTerminalNotifications {
    #[serde(default = "yes")]
    pub completed: bool,
    #[serde(default = "yes")]
    pub failed: bool,
    #[serde(default = "yes")]
    pub cancelled: bool,
}
impl Default for ManagedTerminalNotifications {
    fn default() -> Self {
        Self {
            completed: true,
            failed: true,
            cancelled: true,
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedStopCondition {
    Terminal,
    DurationElapsed,
}

/// Generation-bound offset in the selected durable projection. History is
/// included by Messages, not a separate model-visible command/section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManagedCursor {
    pub generation: u64,
    pub offset: u64,
}
impl ManagedCursor {
    /// # Errors
    /// Rejects malformed, noncanonical, oversized and overflowing cursors.
    pub fn parse(value: &str) -> Result<Self, ManagedSubagentError> {
        let mut parts = value.split(':');
        if parts.next() != Some("v1") {
            return Err(ManagedSubagentError::InvalidArguments);
        }
        let generation = canonical_integer(parts.next())?;
        let offset = canonical_integer(parts.next())?;
        if parts.next().is_some() {
            return Err(ManagedSubagentError::InvalidArguments);
        }
        Ok(Self { generation, offset })
    }
    #[must_use]
    pub fn encode(self) -> String {
        format!("v1:{}:{}", self.generation, self.offset)
    }
}
fn canonical_integer(value: Option<&str>) -> Result<u64, ManagedSubagentError> {
    let value = value.ok_or(ManagedSubagentError::InvalidArguments)?;
    if value.is_empty()
        || value.len() > 20
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(ManagedSubagentError::InvalidArguments);
    }
    value
        .parse()
        .map_err(|_| ManagedSubagentError::InvalidArguments)
}

fn effort_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn distinct<T: PartialEq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .all(|(index, value)| !values[..index].contains(value))
}
impl ManagedNotifications {
    /// Validates policy and makes the duration stop explicit.
    /// # Errors
    /// Rejects duplicate milestones/stops and inconsistent or overflowing timers.
    pub fn normalize(&mut self) -> Result<(), ManagedSubagentError> {
        // Native uses signed millisecond timestamps; reserve checked conversion.
        let duration_valid = |ms: u64| ms > 0 && i64::try_from(ms).is_ok();
        if self.milestones.len() > MAX_SUBAGENT_MILESTONES
            || self.stop_conditions.len() > MAX_SUBAGENT_STOP_CONDITIONS
            || !distinct(&self.milestones)
            || !distinct(&self.stop_conditions)
            || self
                .milestones
                .iter()
                .any(|v| !text_valid(v, MAX_SUBAGENT_NAME_BYTES))
            || self.report_interval_ms.is_some_and(|v| !duration_valid(v))
            || self.report_duration_ms.is_some_and(|v| !duration_valid(v))
            || (self.report_duration_ms.is_some() && self.report_interval_ms.is_none())
            || (self
                .stop_conditions
                .contains(&ManagedStopCondition::DurationElapsed)
                && self.report_duration_ms.is_none())
        {
            return Err(ManagedSubagentError::InvalidArguments);
        }
        if self.report_duration_ms.is_some()
            && !self
                .stop_conditions
                .contains(&ManagedStopCondition::DurationElapsed)
        {
            if self.stop_conditions.len() == MAX_SUBAGENT_STOP_CONDITIONS {
                return Err(ManagedSubagentError::InvalidArguments);
            }
            self.stop_conditions
                .push(ManagedStopCondition::DurationElapsed);
        }
        Ok(())
    }
}
impl ManagedSubagentCommand {
    /// Strict bounded decoder. Depth and node checks precede recursive serde.
    /// # Errors
    /// Returns only fixed errors, never an untrusted payload or serde diagnostic.
    pub fn decode(arguments: Value) -> Result<Self, ManagedSubagentError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Envelope {
            command: ManagedSubagentCommand,
        }
        if !json_bounded(
            &arguments,
            MAX_SUBAGENT_ARGUMENT_BYTES,
            MAX_SUBAGENT_JSON_NODES,
        ) {
            drop_json_value_iterative(arguments);
            return Err(ManagedSubagentError::ResourceLimit);
        }
        let mut command = serde_json::from_value::<Envelope>(arguments)
            .map_err(|_| ManagedSubagentError::InvalidArguments)?
            .command;
        command.normalize()?;
        Ok(command)
    }
    /// Validates a trusted host DTO before converting it to the model contract.
    /// # Errors
    /// Rejects invalid fields, bounds, branches and notification policies.
    pub fn to_arguments(&self) -> Result<Value, ManagedSubagentError> {
        if !serialized_json_size_bounded(self, MAX_SUBAGENT_ARGUMENT_BYTES)
            .is_ok_and(|size| size.is_some())
        {
            return Err(ManagedSubagentError::ResourceLimit);
        }
        let mut command = self.clone();
        command.normalize()?;
        let value = json!({ "command": command });
        if !json_bounded(&value, MAX_SUBAGENT_ARGUMENT_BYTES, MAX_SUBAGENT_JSON_NODES) {
            return Err(ManagedSubagentError::ResourceLimit);
        }
        Ok(value)
    }
    /// Validates and resolves effect-free command defaults.
    /// # Errors
    /// Invalid input is rejected before any authority is consulted.
    #[allow(clippy::too_many_lines)]
    pub fn normalize(&mut self) -> Result<(), ManagedSubagentError> {
        let invalid = ManagedSubagentError::InvalidArguments;
        match self {
            Self::Create(v) => {
                if !text_valid(&v.name, MAX_SUBAGENT_NAME_BYTES)
                    || (v.mode == ManagedAgentMode::OneOff && v.prompt.is_none())
                    || v.prompt
                        .as_ref()
                        .is_some_and(|s| !text_valid(s, MAX_SUBAGENT_PROMPT_BYTES))
                    || v.model
                        .as_ref()
                        .is_some_and(|s| !text_valid(s, MAX_SUBAGENT_MODEL_BYTES))
                    || v.effort.as_ref().is_some_and(|s| !effort_valid(s))
                {
                    return Err(invalid);
                }
                v.notifications.normalize()?;
            }
            Self::Inspect(v) => {
                if !id_valid(&v.id)
                    || v.sections.is_empty()
                    || v.sections.len() > 6
                    || !distinct(&v.sections)
                    || v.limit == 0
                    || v.limit > MAX_SUBAGENT_PAGE_LIMIT
                {
                    return Err(invalid);
                }
                if let Some(cursor) = &v.cursor {
                    ManagedCursor::parse(cursor)?;
                }
                if let Some(wait) = &v.wait
                    && (v.cursor.is_some()
                        || !v.sections.contains(&ManagedInspectSection::Status)
                        || wait.timeout_ms == 0
                        || wait.timeout_ms > MAX_SUBAGENT_INSPECT_WAIT_MS)
                {
                    return Err(invalid);
                }
            }
            Self::Message(ManagedMessage::Send(v)) => {
                if !id_valid(&v.id) || !text_valid(&v.content, MAX_SUBAGENT_MESSAGE_BYTES) {
                    return Err(invalid);
                }
            }
            Self::Message(ManagedMessage::Milestone(v)) => {
                if !text_valid(&v.name, MAX_SUBAGENT_NAME_BYTES) {
                    return Err(invalid);
                }
            }
            Self::Relationship(v) => {
                if !id_valid(&v.id)
                    || v.parent_id
                        .as_ref()
                        .is_some_and(|p| !id_valid(p) || p == &v.id)
                    || (v.action == ManagedRelationshipAction::Detach && v.parent_id.is_some())
                    || (v.action == ManagedRelationshipAction::Reparent && v.parent_id.is_none())
                {
                    return Err(invalid);
                }
            }
            Self::Configure(v) => {
                if !id_valid(&v.id)
                    || (v.name.is_none()
                        && v.model.is_none()
                        && v.effort.is_none()
                        && v.permission_mode.is_none()
                        && v.notifications.is_none())
                    || v.name
                        .as_ref()
                        .is_some_and(|s| !text_valid(s, MAX_SUBAGENT_NAME_BYTES))
                    || v.model
                        .as_ref()
                        .is_some_and(|s| !text_valid(s, MAX_SUBAGENT_MODEL_BYTES))
                    || v.effort.as_ref().is_some_and(|s| !effort_valid(s))
                {
                    return Err(invalid);
                }
                if let Some(policy) = &mut v.notifications {
                    policy.normalize()?;
                }
            }
            Self::Lifecycle(v) => {
                if !id_valid(&v.id) {
                    return Err(invalid);
                }
            }
        }
        Ok(())
    }
}
