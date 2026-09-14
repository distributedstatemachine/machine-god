//! Bounded connection orchestration. The CLI supplies bytes, not product state.

use super::{
    client_requests::NativeAcpClientRequests,
    protocol::{self, AcpId, AcpMessage, AcpProtocolError, AcpRpcError},
    selection::{NativeAcpHostFactory, NativeAcpSelectionId, NativeAcpSelectionOwner},
    session::NativeAcpHistory,
};
use crate::{NativeSessionCatalogPage, NativeSessionCatalogReadError};
use machine_god_core::{BackgroundOutputOwner, BoxFuture, CancellationToken};
use serde_json::Value;
use std::{
    fmt,
    sync::Arc,
    task::{Context, Poll, Waker},
};

mod dispatch;
mod output;
mod request;
#[cfg(test)]
mod tests;

/// Data-free terminal connection diagnostic. Native owners still require polling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAcpConnectionError {
    Protocol,
    Output,
    Native,
}
impl fmt::Display for NativeAcpConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Protocol => "ACP protocol processing failed",
            Self::Output => "ACP output failed",
            Self::Native => "ACP native finalization failed",
        })
    }
}
impl std::error::Error for NativeAcpConnectionError {}

struct PromptRequest {
    id: AcpId,
    owner: BackgroundOutputOwner,
}
enum Control {
    Selection {
        id: AcpId,
        selection: NativeAcpSelectionId,
    },
    Model {
        id: AcpId,
    },
    List {
        id: AcpId,
        cancellation: CancellationToken,
        future: BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>>,
        result: Option<Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>>,
    },
    History {
        id: AcpId,
        history: NativeAcpHistory,
    },
}
impl Control {
    fn id(&self) -> &AcpId {
        match self {
            Self::Selection { id, .. }
            | Self::Model { id }
            | Self::List { id, .. }
            | Self::History { id, .. } => id,
        }
    }
}

/// One connection, one prompt, one control operation and one ready reply.
/// Output is acquired one frame at a time; no transport or ambient authority is
/// acquired here. Call `poll_progress` even while the acquired frame is blocked.
/// EOF must call `begin_shutdown` and continue polling until `is_closed`.
pub struct NativeAcpConnection {
    factory: Arc<dyn NativeAcpHostFactory>,
    selection: NativeAcpSelectionOwner,
    clients: NativeAcpClientRequests,
    prompt: Option<PromptRequest>,
    control: Option<Control>,
    reply: Option<AcpMessage>,
    initialized: bool,
    shutting_down: bool,
    error: Option<NativeAcpConnectionError>,
    wake: Option<Waker>,
}
impl fmt::Debug for NativeAcpConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAcpConnection { .. }")
    }
}
impl Drop for NativeAcpConnection {
    fn drop(&mut self) {
        // This cutoff does not manufacture a settlement receipt. Hosts must
        // still explicitly drive shutdown before dropping the connection.
        self.clients.close();
        if let Some(Control::List { cancellation, .. }) = &self.control {
            cancellation.cancel();
        }
        self.selection.request_shutdown();
    }
}
impl NativeAcpConnection {
    /// Inert construction using explicitly captured host effects and endpoints.
    #[must_use]
    pub fn new(factory: Arc<dyn NativeAcpHostFactory>, clients: NativeAcpClientRequests) -> Self {
        Self {
            selection: NativeAcpSelectionOwner::new(factory.clone()),
            factory,
            clients,
            prompt: None,
            control: None,
            reply: None,
            initialized: false,
            shutting_down: false,
            error: None,
            wake: None,
        }
    }

    /// Whether a normal request can enter the bounded reply/control lanes.
    /// Client responses and cancel notifications can still enter when false.
    #[must_use]
    pub fn can_receive(&self) -> bool {
        !self.shutting_down && self.reply.is_none() && self.control.is_none()
    }

    /// Accept a frame or return the unchanged message on bounded backpressure.
    /// Semantic failures become correlated, payload-free JSON-RPC errors.
    /// # Errors
    /// Returns ownership of a message which has not been admitted.
    pub fn receive(&mut self, message: AcpMessage, now_ms: i64) -> Result<(), AcpMessage> {
        match message {
            AcpMessage::Response { id, outcome } => {
                if let Some(id) = id {
                    let _ = self.clients.reply(&id, outcome);
                }
            }
            AcpMessage::Notification { method, params } => {
                if method == "session/cancel"
                    && self.initialized
                    && !self.shutting_down
                    && let Ok(request::Request::Cancel { session }) =
                        request::decode(&method, params)
                {
                    let _ = self.selection.request_cancel(&session);
                }
            }
            AcpMessage::Request { id, method, params } => {
                // Cancellation needs a reply slot but does not wait for a
                // selection. Other requests retain their exact input at caller.
                if self.shutting_down
                    || self.reply.is_some()
                    || (self.control.is_some() && method != "session/cancel")
                {
                    return Err(AcpMessage::Request { id, method, params });
                }
                if self.prompt.as_ref().is_some_and(|prompt| prompt.id == id)
                    || self
                        .control
                        .as_ref()
                        .is_some_and(|control| control.id() == &id)
                {
                    self.respond(
                        Some(id),
                        Err(rpc_error(-32600, "Duplicate active request identifier")),
                    );
                } else {
                    match request::decode(&method, params) {
                        Ok(request) => self.dispatch(id, request, now_ms),
                        Err(error) => self.respond(Some(id), Err(error)),
                    }
                }
            }
        }
        self.notify();
        Ok(())
    }

    /// # Errors
    /// Returns an unconsumed framing error while its one reply slot is occupied.
    pub fn receive_error(&mut self, error: AcpProtocolError) -> Result<(), AcpProtocolError> {
        if self.reply.is_some() || self.shutting_down {
            return Err(error);
        }
        let code = if matches!(
            error,
            AcpProtocolError::ParseError | AcpProtocolError::TruncatedFrame
        ) {
            -32700
        } else {
            -32600
        };
        self.respond(None, Err(rpc_error(code, "Invalid ACP frame")));
        self.notify();
        Ok(())
    }

    /// Cut off admission, cancel native waiters and retain actual cleanup work.
    pub fn begin_shutdown(&mut self) {
        if self.shutting_down {
            return;
        }
        self.shutting_down = true;
        self.clients.close();
        if let Some(Control::List { cancellation, .. }) = &self.control {
            cancellation.cancel();
        }
        self.selection.request_shutdown();
        self.notify();
    }
    pub fn output_failed(&mut self) {
        self.error.get_or_insert(NativeAcpConnectionError::Output);
        self.reply = None;
        self.begin_shutdown();
    }
    #[must_use]
    pub fn error(&self) -> Option<&NativeAcpConnectionError> {
        self.error.as_ref()
    }
    /// A settled connection can retain one final protocol reply for the I/O grace.
    #[must_use]
    pub fn has_output(&self) -> bool {
        self.reply.is_some()
    }
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.shutting_down
            && self.selection.is_closed()
            && self.control.is_none()
            && self.prompt.is_none()
    }

    /// Advances native effects without pulling a second encoded output frame.
    pub fn poll_progress(&mut self, cx: &mut Context<'_>, now_ms: i64) -> Poll<()> {
        if self
            .wake
            .as_ref()
            .is_none_or(|wake| !wake.will_wake(cx.waker()))
        {
            self.wake = Some(cx.waker().clone());
        }
        let progress = self.selection.poll_progress(cx, now_ms);
        if let Some(Control::List { future, result, .. }) = &mut self.control
            && result.is_none()
            && let Poll::Ready(value) = future.as_mut().poll(cx)
        {
            *result = Some(value);
        }
        if self.shutting_down {
            self.drain_shutdown(cx);
        }
        if self.is_closed() {
            Poll::Ready(())
        } else {
            progress
        }
    }

    fn respond(&mut self, id: Option<AcpId>, outcome: Result<Value, AcpRpcError>) {
        debug_assert!(self.reply.is_none());
        self.reply = Some(AcpMessage::Response { id, outcome });
    }
    fn fail(&mut self, error: NativeAcpConnectionError) {
        self.error.get_or_insert(error);
        self.begin_shutdown();
    }
    fn notify(&mut self) {
        if let Some(wake) = self.wake.take() {
            wake.wake();
        }
    }
}
fn rpc_error(code: i64, message: &str) -> AcpRpcError {
    AcpRpcError {
        code,
        message: message.to_owned(),
        data: None,
    }
}
