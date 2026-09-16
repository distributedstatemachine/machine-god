use super::{
    Form, NativeManagedFormError as Error, NativeManagedFormField as Field,
    NativeManagedFormKind as Kind,
};
use machine_god_core::{
    ManagedAgentMode, ManagedConfigure, ManagedCreate, ManagedNotifications, ManagedStopCondition,
    ManagedSubagentCommand,
};

impl Form {
    /// Returns typed intent only. Native navigation still requires a fresh frame
    /// ACK and the original observed target before actual human admission.
    pub(in crate::interactive_session) fn command(
        &mut self,
    ) -> Result<ManagedSubagentCommand, Error> {
        let result = self.build();
        self.error = result.as_ref().err().copied();
        result
    }

    fn build(&self) -> Result<ManagedSubagentCommand, Error> {
        if let Some(error) = self.rejected_edit {
            return Err(error);
        }
        let mut command = match self.kind {
            Kind::Create => ManagedSubagentCommand::Create(ManagedCreate {
                name: self.required(Field::Name)?.to_owned(),
                mode: self.mode,
                prompt: match self.optional(Field::Prompt) {
                    Some(value) => Some(value.to_owned()),
                    None if self.mode == ManagedAgentMode::OneOff => {
                        return Err(Error::InvalidValue(Field::Prompt));
                    }
                    None => None,
                },
                model: self.model()?,
                effort: self.effort()?,
                permission_mode: self.permission,
                notifications: self.notifications()?,
            }),
            Kind::Configure => self.configuration()?,
        };
        command.normalize().map_err(|_| Error::InvalidField)?;
        Ok(command)
    }

    fn configuration(&self) -> Result<ManagedSubagentCommand, Error> {
        if !self.changed.iter().any(|changed| *changed)
            && !self.permission_changed
            && !self.notices_changed
        {
            return Err(Error::NoChanges);
        }
        let model = if self.changed[1] {
            Some(self.model()?.ok_or(Error::InvalidValue(Field::Model))?)
        } else {
            None
        };
        let effort = if self.changed[3] {
            Some(self.effort()?.ok_or(Error::InvalidValue(Field::Effort))?)
        } else {
            None
        };
        let notifications_changed =
            self.notices_changed || self.changed[4..].iter().any(|changed| *changed);
        Ok(ManagedSubagentCommand::Configure(ManagedConfigure {
            id: self.target.as_ref().ok_or(Error::InvalidField)?.clone(),
            name: self.changed[0]
                .then(|| self.required(Field::Name).map(str::to_owned))
                .transpose()?,
            model,
            effort,
            permission_mode: self.permission_changed.then_some(self.permission).flatten(),
            notifications: notifications_changed
                .then(|| self.notifications())
                .transpose()?,
        }))
    }

    fn required(&self, field: Field) -> Result<&str, Error> {
        self.optional(field).ok_or(Error::InvalidValue(field))
    }
    fn optional(&self, field: Field) -> Option<&str> {
        let value = &self.text[field.text_index()?];
        if value.trim().is_empty() {
            None
        } else {
            Some(value)
        }
    }
    fn model(&self) -> Result<Option<String>, Error> {
        self.optional(Field::Model)
            .map(|model| {
                machine_god_core::validate_model_id(model)
                    .map_err(|_| Error::InvalidValue(Field::Model))?;
                Ok(model.to_owned())
            })
            .transpose()
    }
    fn effort(&self) -> Result<Option<String>, Error> {
        self.optional(Field::Effort)
            .map(|effort| {
                crate::NativeReasoningEffort::parse(effort)
                    .map_err(|_| Error::InvalidValue(Field::Effort))?;
                Ok(effort.to_owned())
            })
            .transpose()
    }
    fn notifications(&self) -> Result<ManagedNotifications, Error> {
        let mut policy = self.notices.clone();
        if self.kind == Kind::Create || self.changed[4] {
            policy.milestones = match self.optional(Field::Milestones) {
                None => Vec::new(),
                Some(value) => serde_json::from_str(value)
                    .map_err(|_| Error::InvalidValue(Field::Milestones))?,
            };
        }
        if self.kind == Kind::Create || self.changed[5] {
            policy.report_interval_ms = self.duration(Field::Interval)?;
        }
        if self.kind == Kind::Create || self.changed[6] {
            policy.report_duration_ms = self.duration(Field::Duration)?;
            if policy.report_duration_ms.is_none() {
                policy
                    .stop_conditions
                    .retain(|condition| *condition != ManagedStopCondition::DurationElapsed);
            }
        }
        policy
            .normalize()
            .map_err(|_| Error::InvalidNotifications)?;
        Ok(policy)
    }
    fn duration(&self, field: Field) -> Result<Option<u64>, Error> {
        self.optional(field)
            .map(|value| {
                if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(Error::InvalidValue(field));
                }
                let milliseconds: u64 = value.parse().map_err(|_| Error::InvalidValue(field))?;
                if milliseconds == 0 || i64::try_from(milliseconds).is_err() {
                    return Err(Error::InvalidValue(field));
                }
                Ok(milliseconds)
            })
            .transpose()
    }
}
