use super::{
    Control, NativeAcpConnection, NativeAcpConnectionError,
    dispatch::session_error,
    protocol::{self, AcpMessage},
    rpc_error,
    session_projection::{catalog_response, config_response, selection_response},
};
use crate::acp::{projection, selection::NativeAcpSelectionOutcome, session::AcpSessionError};
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
                let result = if prompt.owner == outcome.owner {
                    stop_result(outcome.outcome.as_ref().map(|event| &event.payload))
                } else {
                    Err(rpc_error(-32603, "ACP turn ownership mismatch"))
                };
                return self.encode(AcpMessage::Response {
                    id: Some(prompt.id),
                    outcome: result,
                });
            }
            self.fail(NativeAcpConnectionError::Native);
            return Poll::Pending;
        }
        if let Some(update) = self.command_output() {
            return self.encode(update);
        }
        if self.shutting_down {
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
        if let Some(update) = self.commands_output() {
            return self.encode(update);
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
        let encoded = protocol::encode_frame(&message);
        // Release the projected tree before shutdown effects or caller output
        // retention; this lane consumes, rather than queues, the message.
        drop(message);
        if let Ok(frame) = encoded {
            Poll::Ready(Some(frame))
        } else {
            self.fail(NativeAcpConnectionError::Protocol);
            Poll::Pending
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
                self.refresh_commands();
            }
            NativeAcpSelectionOutcome::Closed { .. } => {
                self.clients.deactivate();
                self.commands_update = None;
                self.respond(Some(id), Ok(json!({})));
            }
            NativeAcpSelectionOutcome::Rejected { error, .. } => {
                self.respond(Some(id), Err(session_error(&error)));
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
                        .map_err(|_| rpc_error(-32603, "ACP session listing unavailable"))
                        .and_then(|page| {
                            catalog_response(&page).map_err(|error| session_error(&error))
                        }),
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
                    self.refresh_commands();
                }
            }
            _ => {}
        }
    }

    pub(super) fn drain_shutdown(&mut self, cx: &mut Context<'_>) {
        self.discard_settled_command();
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
            if let Err(error) = &outcome.outcome
                && !resource_preparation_cancelled(error)
            {
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
            Some(
                Control::History { .. }
                    | Control::List {
                        result: Some(_),
                        ..
                    }
            )
        ) {
            self.control = None;
        }
        if self.selection.is_closed() {
            self.prompt = None;
            self.failed_control = None;
            if !matches!(self.control, Some(Control::List { .. })) {
                self.control = None;
            }
        }
    }
}

pub(super) fn update_message(session: &SessionId, update: Value) -> AcpMessage {
    let mut params = json!({"sessionId":session.as_str()});
    params["update"] = update;
    AcpMessage::Notification {
        method: "session/update".to_owned(),
        params: Some(params),
    }
}
// Called only after the exact native outcome settles and its principal matches.
// Resource preparation can settle cancellation before a core turn exists; do
// not manufacture an engine event or infer cancellation from requested intent.
fn stop_result(
    outcome: Result<&TurnEvent, &crate::NativeInteractiveError>,
) -> Result<Value, super::protocol::AcpRpcError> {
    match outcome {
        Ok(TurnEvent::Completed { reason, .. }) => Ok(json!({"stopReason":match reason {
            StopReason::Cancelled=>"cancelled",
            StopReason::MaxOutputTokens=>"max_tokens",
            StopReason::ContentFilter=>"refusal",
            _=>"end_turn",
        }})),
        Err(error) if resource_preparation_cancelled(error) => {
            Ok(json!({"stopReason":"cancelled"}))
        }
        _ => Err(rpc_error(-32603, "ACP native turn failed")),
    }
}

fn resource_preparation_cancelled(error: &crate::NativeInteractiveError) -> bool {
    matches!(
        error,
        crate::NativeInteractiveError::Runtime(crate::NativeConversationRuntimeError::Resources(
            crate::acp::resources::NativeAcpResourceContextError::Cancelled,
        ))
    )
}

#[cfg(test)]
mod tests;
