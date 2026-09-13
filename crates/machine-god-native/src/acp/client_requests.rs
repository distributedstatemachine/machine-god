//! Connection-lifetime custody for native human prompts and their wire replies.

use std::{
    fmt,
    sync::Arc,
    task::{Context, Poll},
};

use machine_god_core::{BackgroundOutputOwner, ContentBlock};
use serde_json::Value;

use crate::{
    NativeInteractivePromptBridge, NativeInteractivePromptInbox, NativeInteractivePromptLimits,
    NativeInteractivePromptView, NativePermissionContexts,
};

use super::{
    interaction::NativeAcpElicitationPresenter,
    projection::{self, NativeAcpReplyKind},
    protocol::{self, AcpId, AcpMessage, AcpPendingRequests, AcpRpcError, AcpScope},
};

/// Payload-free errors. Remote errors and rejected answers are never diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAcpClientRequestError {
    Closed,
    Unavailable,
    InvalidResponse,
    Stale,
    Limit,
}
impl fmt::Display for NativeAcpClientRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ACP client interaction unavailable")
    }
}
impl std::error::Error for NativeAcpClientRequestError {}

struct Pending {
    id: AcpId,
    scope: AcpScope,
    view: NativeInteractivePromptView,
}

/// Owns one connection's reusable native inbox and monotonically allocated RPC
/// identifiers. The native inbox displays one request at a time; this owner
/// retains at most that one view, not a second queue of copied prompt payloads.
///
/// A caller may poll for a frame only when its bounded output slot is empty.
/// A returned frame is admitted output, not proof that the peer received it.
/// Keep this owner for the entire connection, including session replacements.
pub struct NativeAcpClientRequests {
    bridge: Arc<NativeInteractivePromptBridge>,
    inbox: NativeInteractivePromptInbox,
    presenter: Arc<NativeAcpElicitationPresenter>,
    contexts: Arc<NativePermissionContexts>,
    ids: AcpPendingRequests,
    pending: Option<Pending>,
    owner: Option<BackgroundOutputOwner>,
    epoch: u64,
    closed: bool,
}
impl fmt::Debug for NativeAcpClientRequests {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAcpClientRequests { <redacted> }")
    }
}
impl NativeAcpClientRequests {
    /// Retains explicit permission context routes; construction performs no I/O.
    /// # Errors
    /// Rejects unavailable native inbox construction.
    pub fn new(
        contexts: Arc<NativePermissionContexts>,
    ) -> Result<Self, NativeAcpClientRequestError> {
        let (bridge, inbox) =
            NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default())
                .map_err(|_| NativeAcpClientRequestError::Unavailable)?;
        let presenter = Arc::new(NativeAcpElicitationPresenter::new(Arc::clone(&bridge)));
        Ok(Self {
            bridge,
            inbox,
            presenter,
            contexts,
            ids: AcpPendingRequests::new(),
            pending: None,
            owner: None,
            epoch: 0,
            closed: false,
        })
    }

    /// Shared permission/question endpoint for explicitly composed native hosts.
    #[must_use]
    pub fn bridge(&self) -> Arc<NativeInteractivePromptBridge> {
        Arc::clone(&self.bridge)
    }

    /// Explicit modern client URL endpoint, never a local browser fallback.
    #[must_use]
    pub fn presenter(&self) -> Arc<NativeAcpElicitationPresenter> {
        Arc::clone(&self.presenter)
    }

    /// Activates an already committed exact native principal. Even same-ID
    /// reactivation invalidates old tokens and never reuses an RPC identifier.
    /// # Errors
    /// Rejects closed ownership or counter exhaustion before changing the owner.
    pub fn activate(
        &mut self,
        owner: BackgroundOutputOwner,
    ) -> Result<(), NativeAcpClientRequestError> {
        if self.closed {
            return Err(NativeAcpClientRequestError::Closed);
        }
        let epoch = self
            .epoch
            .checked_add(1)
            .ok_or(NativeAcpClientRequestError::Limit)?;
        self.inbox
            .activate(owner.clone())
            .map_err(|_| NativeAcpClientRequestError::Unavailable)?;
        self.presenter.activate(owner.clone());
        self.pending = None;
        let _ = self.ids.clear();
        self.owner = Some(owner);
        self.epoch = epoch;
        Ok(())
    }

    /// Invalidates pending and already accepted replies without claiming native
    /// turn, checkpoint, process or socket completion.
    pub fn deactivate(&mut self) {
        self.inbox.deactivate();
        self.presenter.deactivate();
        self.pending = None;
        let _ = self.ids.clear();
        self.owner = None;
    }

    /// EOF/output failure closes interaction admission. The session driver must
    /// independently cancel and keep polling its native owners to settlement.
    pub fn close(&mut self) {
        self.deactivate();
        self.inbox.close();
        self.closed = true;
    }

    /// Encodes at most one actual native prompt into an available output slot.
    /// Repeated polls never resubmit an unanswered request.
    pub fn poll_request(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<Vec<u8>>, NativeAcpClientRequestError>> {
        if self.owner.is_none() || self.closed {
            return Poll::Ready(Ok(None));
        }
        if let Some(pending) = &self.pending {
            if self
                .inbox
                .poll_pending(pending.view.token(), cx)
                .is_pending()
            {
                return Poll::Pending;
            }
            let _ = self.ids.complete(&pending.id, pending.scope);
            self.pending = None;
        }
        let view = match self.inbox.poll_prompt(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(None) => return Poll::Ready(Ok(None)),
            Poll::Ready(Some(view)) => view,
        };
        let result = self.encode_request(&view);
        match result {
            Ok((id, scope, bytes)) => {
                self.pending = Some(Pending { id, scope, view });
                Poll::Ready(Ok(Some(bytes)))
            }
            Err(error) => {
                let _ = self.inbox.cancel(view.token());
                Poll::Ready(Err(error))
            }
        }
    }

    fn encode_request(
        &mut self,
        view: &NativeInteractivePromptView,
    ) -> Result<(AcpId, AcpScope, Vec<u8>), NativeAcpClientRequestError> {
        let url_id = view
            .elicitation()
            .and_then(|request| self.presenter.id_for(request).ok());
        let request = if let Some(permission) = view.permission() {
            let context = self
                .contexts
                .snapshot(permission)
                .map_err(|_| NativeAcpClientRequestError::Stale)?;
            let Some(ContentBlock::ToolCall { call }) = context.pending_assistant().content.first()
            else {
                return Err(NativeAcpClientRequestError::Stale);
            };
            projection::project_permission(view, call)
        } else {
            projection::project_prompt(view, url_id)
        }
        .map_err(|_| NativeAcpClientRequestError::Unavailable)?;
        // These are only correlation labels. Exact native incarnation, turn,
        // invocation and continuation round remain in the retained inbox view.
        let scope = AcpScope {
            session: self.epoch,
            turn: 0,
            operation: view.token().generation(),
            round: 0,
        };
        let id = self
            .ids
            .reserve(scope)
            .map_err(|_| NativeAcpClientRequestError::Limit)?;
        let message = AcpMessage::Request {
            id: id.clone(),
            method: request.method.into(),
            params: Some(request.params),
        };
        let bytes = match protocol::encode_frame(&message) {
            Ok(bytes) => bytes,
            Err(_) => {
                let _ = self.ids.complete(&id, scope);
                return Err(NativeAcpClientRequestError::Limit);
            }
        };
        if request.reply_kind == NativeAcpReplyKind::Url {
            let submitted = url_id.is_some_and(|id| self.presenter.mark_submitted(id).is_ok());
            if !submitted {
                let _ = self.ids.complete(&id, scope);
                return Err(NativeAcpClientRequestError::Stale);
            }
        }
        Ok((id, scope, bytes))
    }

    /// Settles only this connection's exact retained native token. Invalid or
    /// error replies cancel that request; neither can authorize another one.
    /// # Errors
    /// Rejects unknown/duplicate IDs, stale native tokens and invalid answers.
    pub fn reply(
        &mut self,
        id: &AcpId,
        outcome: Result<Value, AcpRpcError>,
    ) -> Result<(), NativeAcpClientRequestError> {
        let pending = self
            .pending
            .as_ref()
            .ok_or(NativeAcpClientRequestError::Stale)?;
        if &pending.id != id {
            return Err(NativeAcpClientRequestError::Stale);
        }
        self.ids
            .complete(id, pending.scope)
            .map_err(|_| NativeAcpClientRequestError::Stale)?;
        let pending = self
            .pending
            .take()
            .ok_or(NativeAcpClientRequestError::Stale)?;
        let response = outcome
            .map_err(|_| ())
            .and_then(|value| projection::decode_reply(&pending.view, &value).map_err(|_| ()));
        match response {
            Ok(response) => self
                .inbox
                .reply(pending.view.token(), response)
                .map_err(|_| {
                    let _ = self.inbox.cancel(pending.view.token());
                    NativeAcpClientRequestError::Stale
                }),
            Err(()) => {
                let _ = self.inbox.cancel(pending.view.token());
                Err(NativeAcpClientRequestError::InvalidResponse)
            }
        }
    }

    /// Encodes an actual completed modern URL registration into an empty output
    /// slot. Call before deactivating a finalized turn; this never infers success
    /// from a client reply or from a streamed terminal engine event.
    pub fn poll_complete(
        &self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<Vec<u8>>, NativeAcpClientRequestError>> {
        let complete = match self.presenter.poll_complete(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(None) => return Poll::Ready(Ok(None)),
            Poll::Ready(Some(complete)) => complete,
        };
        if self.owner.as_ref() != Some(&complete.owner) {
            return Poll::Ready(Err(NativeAcpClientRequestError::Stale));
        }
        Poll::Ready(protocol::encode_frame(&AcpMessage::Notification {
            method: "elicitation/complete".into(),
            params: Some(serde_json::json!({ "sessionId": complete.owner.session_id().as_str(), "elicitationId": complete.id.to_string() })),
        }).map(Some).map_err(|_| NativeAcpClientRequestError::Limit))
    }
}

impl Drop for NativeAcpClientRequests {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests;
