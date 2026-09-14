use super::{
    Control, NativeAcpConnection, NativeAcpConnectionError,
    dispatch::session_error,
    protocol::{self, AcpMessage},
    rpc_error,
};
use crate::NativeSessionCatalogPage;
use crate::acp::{
    projection,
    selection::NativeAcpSelectionOutcome,
    session::{AcpSessionError, NativeAcpSession},
};
use machine_god_core::{SessionId, StopReason, TurnEvent};
use serde_json::{Value, json};
use std::task::{Context, Poll};

impl NativeAcpConnection {
    /// Acquires at most one frame. The caller must not call this while it retains
    /// another frame. `None` means settled shutdown with no remaining output.
    pub fn poll_output(&mut self, cx: &mut Context<'_>, now_ms: i64) -> Poll<Option<Vec<u8>>> {
        let _ = self.poll_progress(cx, now_ms);
        if let Some(reply) = self.reply.take() {
            return self.encode(reply);
        }
        if self.is_closed() {
            return Poll::Ready(None);
        }
        if self.shutting_down {
            return Poll::Pending;
        }
        // A native load is incremental; its final response follows all history.
        if let Some(Control::History { .. }) = &self.control {
            return self.history_output();
        }
        // Drain old presentation and finalized prompt/URL receipts before a
        // selection receipt activates the next principal and its inbox registry.
        for index in 0..32 {
            let Some((owner, event)) = self.selection.take_presentation() else {
                break;
            };
            match projection::project_event(&event) {
                Ok(Some(update)) => return self.encode(update_message(owner.session_id(), update)),
                Ok(None) => {}
                Err(_) => {
                    self.fail(NativeAcpConnectionError::Protocol);
                    return Poll::Pending;
                }
            }
            if index == 31 {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        }
        match self.clients.poll_complete(cx) {
            Poll::Ready(Ok(Some(frame))) => return Poll::Ready(Some(frame)),
            Poll::Ready(Err(_)) => {
                self.fail(NativeAcpConnectionError::Protocol);
                return Poll::Pending;
            }
            _ => {}
        }
        if let Some(outcome) = self.selection.take_turn_outcome() {
            if let Some(prompt) = self.prompt.take() {
                let result = if prompt.owner != outcome.owner {
                    Err(rpc_error(-32603, "ACP turn ownership mismatch"))
                } else {
                    match outcome.outcome {
                        Ok(event) => stop_result(&event.payload),
                        Err(_) => Err(rpc_error(-32603, "ACP native turn failed")),
                    }
                };
                return self.encode(AcpMessage::Response {
                    id: Some(prompt.id),
                    outcome: result,
                });
            }
            self.fail(NativeAcpConnectionError::Native);
            return Poll::Pending;
        }
        if let Some(outcome) = self.selection.take_outcome() {
            self.selection_output(outcome);
            if let Some(reply) = self.reply.take() {
                return self.encode(reply);
            }
            if matches!(self.control, Some(Control::History { .. })) {
                return self.history_output();
            }
        }
        self.control_output();
        if let Some(reply) = self.reply.take() {
            return self.encode(reply);
        }
        match self.clients.poll_request(cx) {
            Poll::Ready(Ok(Some(frame))) => Poll::Ready(Some(frame)),
            Poll::Ready(Err(_)) => {
                self.fail(NativeAcpConnectionError::Protocol);
                Poll::Pending
            }
            _ => Poll::Pending,
        }
    }

    fn encode(&mut self, message: AcpMessage) -> Poll<Option<Vec<u8>>> {
        match protocol::encode_frame(&message) {
            Ok(frame) => Poll::Ready(Some(frame)),
            Err(_) => {
                self.fail(NativeAcpConnectionError::Protocol);
                Poll::Pending
            }
        }
    }

    fn history_output(&mut self) -> Poll<Option<Vec<u8>>> {
        let Some(Control::History { id, mut history }) = self.control.take() else {
            return Poll::Pending;
        };
        let Some(current) = self.selection.current() else {
            self.fail(NativeAcpConnectionError::Native);
            return Poll::Pending;
        };
        match history.next_update() {
            Ok(Some(update)) => {
                let message = update_message(&current.id(), update);
                self.control = Some(Control::History { id, history });
                self.encode(message)
            }
            Ok(None) => {
                let outcome = selection_response(current).map_err(|error| session_error(&error));
                self.encode(AcpMessage::Response {
                    id: Some(id),
                    outcome,
                })
            }
            Err(error) => self.encode(AcpMessage::Response {
                id: Some(id),
                outcome: Err(session_error(&error)),
            }),
        }
    }

    fn selection_output(&mut self, outcome: NativeAcpSelectionOutcome) {
        let Some(Control::Selection { id, selection }) = self.control.take() else {
            self.fail(NativeAcpConnectionError::Native);
            return;
        };
        let actual = match &outcome {
            NativeAcpSelectionOutcome::Selected { id, .. }
            | NativeAcpSelectionOutcome::Closed { id, .. }
            | NativeAcpSelectionOutcome::Rejected { id, .. }
            | NativeAcpSelectionOutcome::Indeterminate { id, .. } => *id,
        };
        if actual != selection {
            self.respond(
                Some(id),
                Err(rpc_error(-32603, "ACP selection ownership mismatch")),
            );
            self.fail(NativeAcpConnectionError::Native);
            return;
        }
        match outcome {
            NativeAcpSelectionOutcome::Selected { current, .. } => {
                let Some(contexts) = self.selection.current_permission_contexts() else {
                    self.fail(NativeAcpConnectionError::Native);
                    return;
                };
                if self.clients.activate(current, contexts).is_err() {
                    self.fail(NativeAcpConnectionError::Native);
                    return;
                }
                let Some(session) = self.selection.current_mut() else {
                    self.fail(NativeAcpConnectionError::Native);
                    return;
                };
                if let Some(history) = session.take_loaded_history() {
                    self.control = Some(Control::History { id, history });
                } else {
                    let result = selection_response(session).map_err(|error| session_error(&error));
                    self.respond(Some(id), result);
                }
            }
            NativeAcpSelectionOutcome::Closed { .. } => {
                self.clients.deactivate();
                self.respond(Some(id), Ok(json!({})));
            }
            NativeAcpSelectionOutcome::Rejected { error, .. } => {
                self.respond(Some(id), Err(session_error(&error)))
            }
            NativeAcpSelectionOutcome::Indeterminate { error, .. } => {
                self.respond(Some(id), Err(session_error(&error)));
                self.fail(NativeAcpConnectionError::Native);
            }
        }
    }

    fn control_output(&mut self) {
        match &mut self.control {
            Some(Control::List { result, .. }) if result.is_some() => {
                let Some(Control::List {
                    id,
                    result: Some(result),
                    ..
                }) = self.control.take()
                else {
                    unreachable!()
                };
                self.respond(
                    Some(id),
                    result
                        .map(|page| catalog_response(&page))
                        .map_err(|_| rpc_error(-32603, "ACP session listing unavailable")),
                );
            }
            Some(Control::Model { .. }) => {
                if let Some(result) = self.selection.take_model_save_outcome() {
                    let Some(Control::Model { id }) = self.control.take() else {
                        unreachable!()
                    };
                    let result = result.and_then(|_| {
                        config_response(
                            self.selection
                                .current()
                                .ok_or(AcpSessionError::WrongSession)?,
                        )
                    });
                    self.respond(Some(id), result.map_err(|error| session_error(&error)));
                }
            }
            _ => {}
        }
    }

    pub(super) fn drain_shutdown(&mut self, cx: &mut Context<'_>) {
        // EOF/output cutoff cannot wait for a peer to drain presentation. The
        // native checkpoint and cleanup receipts remain mandatory, even though
        // unsent presentation may now be discarded under this terminal cutoff.
        for _ in 0..32 {
            if self.selection.take_presentation().is_none() {
                break;
            }
            cx.waker().wake_by_ref();
        }
        if let Some(outcome) = self.selection.take_turn_outcome() {
            if outcome.outcome.is_err() {
                self.error.get_or_insert(NativeAcpConnectionError::Native);
            }
            self.prompt = None;
        }
        if let Some(result) = self.selection.take_model_save_outcome() {
            if result.is_err() {
                self.error.get_or_insert(NativeAcpConnectionError::Native);
            }
            if matches!(self.control, Some(Control::Model { .. })) {
                self.control = None;
            }
        }
        if let Some(outcome) = self.selection.take_outcome() {
            if matches!(outcome, NativeAcpSelectionOutcome::Indeterminate { .. }) {
                self.error.get_or_insert(NativeAcpConnectionError::Native);
            }
            if matches!(self.control, Some(Control::Selection { .. })) {
                self.control = None;
            }
        }
        if matches!(
            self.control,
            Some(Control::History { .. })
                | Some(Control::List {
                    result: Some(_),
                    ..
                })
        ) {
            self.control = None;
        }
        if self.selection.is_closed() {
            self.prompt = None;
            if !matches!(self.control, Some(Control::List { .. })) {
                self.control = None;
            }
        }
    }
}

fn update_message(session: &SessionId, update: Value) -> AcpMessage {
    let mut params = json!({"sessionId":session.as_str()});
    params["update"] = update;
    AcpMessage::Notification {
        method: "session/update".to_owned(),
        params: Some(params),
    }
}
fn stop_result(event: &TurnEvent) -> Result<Value, super::protocol::AcpRpcError> {
    match event {
        TurnEvent::Completed { reason, .. } => Ok(json!({"stopReason":match reason {
            StopReason::Cancelled=>"cancelled",
            StopReason::MaxOutputTokens=>"max_tokens",
            StopReason::ContentFilter=>"refusal",
            _=>"end_turn",
        }})),
        _ => Err(rpc_error(-32603, "ACP native turn failed")),
    }
}

pub(super) fn config_response(session: &NativeAcpSession) -> Result<Value, AcpSessionError> {
    let mode = session.mode()?.as_str();
    let preferences = session.runtime().model_preferences();
    let catalog = session.runtime().model_catalog();
    let mut models: Vec<Value> = catalog.as_ref().map_or_else(Vec::new, |catalog| {
        catalog
            .entries()
            .iter()
            .map(|entry| json!({"value":entry.model().id(),"name":entry.model().id()}))
            .collect()
    });
    if !models
        .iter()
        .any(|model| model["value"] == preferences.model())
    {
        models.push(json!({"value":preferences.model(),"name":preferences.model()}));
    }
    Ok(json!({"configOptions":[
        {"id":"mode","name":"Permission mode","category":"mode","type":"select","currentValue":mode,
            "options":[{"value":"ask","name":"Ask"},{"value":"auto","name":"Auto"},{"value":"yolo","name":"Yolo"}]},
        {"id":"model","name":"Model","category":"model","type":"select","currentValue":preferences.model(),"options":models}
    ]}))
}
fn selection_response(session: &NativeAcpSession) -> Result<Value, AcpSessionError> {
    let mut result = config_response(session)?;
    result["sessionId"] = Value::String(session.id().as_str().to_owned());
    result["modes"] = json!({"currentModeId":session.mode()?.as_str(),"availableModes":[
        {"id":"ask","name":"Ask"},{"id":"auto","name":"Auto"},{"id":"yolo","name":"Yolo"}
    ]});
    Ok(result)
}
fn catalog_response(page: &NativeSessionCatalogPage) -> Value {
    let sessions: Vec<Value> = page
        .entries()
        .iter()
        .map(|entry| {
            let mut value = json!({"sessionId":entry.id().as_str()});
            if let Some(cwd) = entry
                .native_metadata()
                .workspace()
                .and_then(|path| path.to_str())
            {
                value["cwd"] = Value::String(cwd.to_owned());
            }
            if let Some(title) = entry.native_metadata().title() {
                value["title"] = Value::String(title.to_owned());
            }
            value
        })
        .collect();
    let mut result = json!({"sessions":sessions,"_meta":{"machineGod":{
        "scanComplete":page.scan_complete(),"resultsTruncated":page.results_truncated(),
        "skippedInvalid":page.skipped_invalid()
    }}});
    if let Some(cursor) = page.next_cursor() {
        result["nextCursor"] = Value::String(cursor.to_string());
    }
    result
}
