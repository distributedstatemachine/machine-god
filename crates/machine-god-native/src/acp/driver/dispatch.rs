use super::{Control, NativeAcpConnection, PromptRequest, request::Request, rpc_error};
use crate::acp::{
    projection,
    protocol::AcpId,
    session::{AcpSessionError, NativeAcpConfigChange},
};
use machine_god_core::CancellationToken;
use serde_json::json;

impl NativeAcpConnection {
    pub(super) fn dispatch(&mut self, id: AcpId, request: Request, now_ms: i64) {
        if !self.initialized && !matches!(request, Request::Initialize) {
            self.respond(
                Some(id),
                Err(rpc_error(
                    -32600,
                    "Initialize must precede session requests",
                )),
            );
            return;
        }
        match self.execute(&id, request, now_ms) {
            Ok(Some(value)) => self.respond(Some(id), Ok(value)),
            Ok(None) => {}
            Err(error) => self.respond(Some(id), Err(session_error(&error))),
        }
    }

    fn execute(
        &mut self,
        id: &AcpId,
        request: Request,
        now_ms: i64,
    ) -> Result<Option<serde_json::Value>, AcpSessionError> {
        match request {
            Request::Initialize => {
                if self.initialized {
                    return Err(AcpSessionError::InvalidConfiguration);
                }
                self.initialized = true;
                Ok(Some(projection::initialize_result()))
            }
            Request::Select {
                selection,
                cwd,
                mcp,
            } => {
                let selection = self.selection.request(selection, cwd, mcp, now_ms)?;
                self.control = Some(Control::Selection {
                    id: id.clone(),
                    selection,
                });
                Ok(None)
            }
            Request::Close { session } => {
                let selection = self.selection.request_close(&session)?;
                self.control = Some(Control::Selection {
                    id: id.clone(),
                    selection,
                });
                Ok(None)
            }
            Request::List { cwd, cursor } => {
                if self.prompt.is_some() || self.command.is_some() {
                    return Err(AcpSessionError::Busy);
                }
                let cancellation = CancellationToken::new();
                let future = self.factory.list(cwd, cursor, cancellation.clone());
                self.control = Some(Control::List {
                    id: id.clone(),
                    cancellation,
                    future,
                    result: None,
                });
                Ok(None)
            }
            Request::Prompt { session, prompt } => {
                if self.prompt.is_some() || self.command.is_some() {
                    return Err(AcpSessionError::Busy);
                }
                if self.begin_command(id, &session, &prompt, now_ms)? {
                    return Ok(None);
                }
                let current = self.selection.current_mut().ok_or(AcpSessionError::Busy)?;
                current.enqueue(&session, prompt)?;
                self.prompt = Some(PromptRequest {
                    id: id.clone(),
                    owner: current.principal(),
                });
                Ok(None)
            }
            Request::Cancel { session } => {
                self.cancel_prompt(&session)?;
                Ok(Some(serde_json::Value::Null))
            }
            Request::SetMode { session, mode } => {
                self.selection
                    .current()
                    .ok_or(AcpSessionError::WrongSession)?
                    .set_mode(&session, &mode)?;
                Ok(Some(json!({})))
            }
            Request::SetConfig {
                session,
                config,
                value,
            } => {
                if self.prompt.is_some() || self.command.is_some() {
                    return Err(AcpSessionError::Busy);
                }
                let current = self.selection.current_mut().ok_or(AcpSessionError::Busy)?;
                match current.set_config_option(&session, &config, &value)? {
                    NativeAcpConfigChange::Model { .. } => {
                        current.request_model_save(&session, now_ms)?;
                        self.control = Some(Control::Model { id: id.clone() });
                        Ok(None)
                    }
                    NativeAcpConfigChange::Mode(_) => {
                        Ok(Some(super::output::config_response(current)?))
                    }
                }
            }
        }
    }
}

pub(super) fn session_error(error: &AcpSessionError) -> crate::acp::protocol::AcpRpcError {
    let code = match error {
        AcpSessionError::InvalidPrompt
        | AcpSessionError::UnsupportedContent
        | AcpSessionError::WrongSession
        | AcpSessionError::InvalidConfiguration => -32602,
        _ => -32603,
    };
    rpc_error(code, &error.to_string())
}
