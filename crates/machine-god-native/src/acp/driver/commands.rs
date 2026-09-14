use super::{CommandRequest, NativeAcpConnection, NativeAcpConnectionError};
use crate::acp::{
    commands::{self, NativeAcpCommandError, NativeAcpCommandOwner},
    prompt::NativeAcpPrompt,
    protocol::{AcpId, AcpMessage},
    session::AcpSessionError,
};
use machine_god_core::SessionId;
use serde_json::json;

impl NativeAcpConnection {
    pub(super) fn begin_command(
        &mut self,
        id: &AcpId,
        session: &SessionId,
        prompt: &NativeAcpPrompt,
        now_ms: i64,
    ) -> Result<bool, AcpSessionError> {
        let Some(command) = commands::classify(prompt).map_err(command_error)? else {
            return Ok(false);
        };
        let current = self.selection.current_mut().ok_or(AcpSessionError::Busy)?;
        let previous_model = current.runtime().model_preferences().model().to_owned();
        let mut owner = Box::new(NativeAcpCommandOwner::new());
        owner
            .begin(current, session, command, now_ms)
            .map_err(command_error)?;
        let configuration = (current.runtime().model_preferences().model() != previous_model)
            .then(|| current.retain_command_configuration());
        self.command = Some(CommandRequest {
            id: id.clone(),
            principal: current.principal(),
            owner,
            configuration,
        });
        Ok(true)
    }

    pub(super) fn cancel_prompt(&mut self, expected: &SessionId) -> Result<bool, AcpSessionError> {
        let accepted = self.selection.request_cancel(expected)?;
        if accepted && let Some(command) = &mut self.command {
            command
                .owner
                .note_cancellation_requested(&command.principal)
                .map_err(command_error)?;
        }
        Ok(accepted)
    }

    // During normal operation this is called only upon output acquisition. The
    // facade's retained receipt is the old-selection retirement barrier.
    pub(super) fn command_progress(&mut self) {
        let Some(command) = &mut self.command else {
            return;
        };
        if self.selection.current().is_some_and(|current| {
            current.principal() == command.principal && current.cancellation_requested()
        }) && command
            .owner
            .note_cancellation_requested(&command.principal)
            .is_err()
        {
            self.fail(NativeAcpConnectionError::Native);
            return;
        }
        let mut receipt = self.selection.take_command_control_outcome();
        if receipt.is_some() && command.owner.complete(&mut receipt).is_err() {
            // Retain a mismatched native receipt instead of laundering it into
            // another command's completion or discarding its resource custody.
            self.failed_control = receipt.map(Box::new);
            self.command = None;
            self.fail(NativeAcpConnectionError::Native);
        }
    }

    pub(super) fn command_output(&mut self) -> Option<AcpMessage> {
        self.command_progress();
        let command = self.command.as_mut()?;
        let result = command.owner.result()?;
        if command.principal != *result.principal() {
            self.fail(NativeAcpConnectionError::Native);
            return None;
        }
        if command.configuration.is_some() {
            let Some(current) = self
                .selection
                .current()
                .filter(|current| current.principal() == command.principal)
            else {
                self.fail(NativeAcpConnectionError::Native);
                return None;
            };
            // Native preferences may already be accepted even if persistence
            // failed or cancellation was observed. Project actual live state,
            // not a claim about saving it. This acquisition follows the exact
            // native receipt drain, before another poll can retire its session.
            // Keep the owned result until the following command-result frame.
            let Ok(mut update) = super::session_projection::config_response(current) else {
                self.fail(NativeAcpConnectionError::Protocol);
                return None;
            };
            update["sessionUpdate"] = json!("config_option_update");
            command.configuration = None;
            return Some(super::output::update_message(
                command.principal.session_id(),
                update,
            ));
        }
        let result = command.owner.take_result()?;
        let update =
            super::output::update_message(result.principal().session_id(), result.update());
        let Some(CommandRequest { id, .. }) = self.command.take() else {
            unreachable!()
        };
        self.respond(
            Some(id),
            Ok(json!({"stopReason":if result.cancelled(){"cancelled"}else{"end_turn"}})),
        );
        self.refresh_commands();
        Some(update)
    }

    pub(super) fn refresh_commands(&mut self) {
        if self.shutting_down || self.selection.is_busy() {
            return;
        }
        self.commands_update = self
            .selection
            .current()
            .map(super::super::session::NativeAcpSession::principal);
    }

    pub(super) fn commands_output(&mut self) -> Option<AcpMessage> {
        if self.selection.is_busy() {
            return None;
        }
        let expected = self.commands_update.take()?;
        let current = self.selection.current()?;
        if current.principal() != expected {
            return None;
        }
        Some(super::output::update_message(
            expected.session_id(),
            commands::available_commands(current),
        ))
    }

    pub(super) fn discard_settled_command(&mut self) {
        self.command_progress();
        if let Some(command) = &mut self.command
            && !command.owner.has_pending()
        {
            if command
                .owner
                .take_result()
                .is_some_and(|result| result.failed())
            {
                self.error.get_or_insert(NativeAcpConnectionError::Native);
            }
            self.command = None;
            self.notify();
        }
    }
}

fn command_error(error: NativeAcpCommandError) -> AcpSessionError {
    match error {
        NativeAcpCommandError::Invalid => AcpSessionError::InvalidPrompt,
        NativeAcpCommandError::Unsupported => AcpSessionError::UnsupportedContent,
        NativeAcpCommandError::Limit => AcpSessionError::Limit,
        NativeAcpCommandError::Busy => AcpSessionError::Busy,
        NativeAcpCommandError::WrongSession => AcpSessionError::WrongSession,
        NativeAcpCommandError::Unavailable => AcpSessionError::Unavailable,
    }
}
