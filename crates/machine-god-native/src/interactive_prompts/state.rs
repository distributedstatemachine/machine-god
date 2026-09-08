use super::payload::Payload;
use super::{
    NativeInteractivePromptError as Error, NativeInteractivePromptLimits,
    NativeInteractivePromptResponse as Response, NativeInteractivePromptScope as Scope,
    NativeInteractivePromptToken as Token, NativeInteractivePromptView as View,
};
use crate::{PermissionPromptDecision, QuestionPromptOutcome};
use machine_god_core::BackgroundOutputOwner;
use std::collections::VecDeque;
use std::future::poll_fn;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

pub(super) struct Shared {
    pub identity: Arc<()>,
    pub limits: NativeInteractivePromptLimits,
    pub state: Mutex<State>,
}

pub(super) struct State {
    closed: bool,
    scope: Option<(Scope, BackgroundOutputOwner)>,
    next_scope: u64,
    next_request: u64,
    bytes: usize,
    entries: VecDeque<Entry>,
    ui_wake: Option<Waker>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            closed: false,
            scope: None,
            next_scope: 1,
            next_request: 1,
            bytes: 0,
            entries: VecDeque::new(),
            ui_wake: None,
        }
    }
}

struct Entry {
    token: Token,
    payload: Arc<Payload>,
    bytes: usize,
    displayed: bool,
    response: Option<Response>,
    wake: Option<Waker>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poison| {
            let mut state = poison.into_inner();
            // Never recover possible partial admission into new authority.
            state.closed = true;
            state
        })
    }

    pub fn scope(&self) -> Option<Scope> {
        let state = self.lock();
        (!state.closed)
            .then(|| state.scope.as_ref().map(|(scope, _)| *scope))
            .flatten()
    }

    pub fn activate(&self, owner: BackgroundOutputOwner) -> Result<Scope, Error> {
        let (scope, old, wake) = {
            let mut state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let next = state.next_scope.checked_add(1).ok_or(Error::Exhausted)?;
            let scope = Scope(state.next_scope);
            state.next_scope = next;
            state.scope = Some((scope, owner));
            state.bytes = 0;
            (
                scope,
                std::mem::take(&mut state.entries),
                state.ui_wake.take(),
            )
        };
        discard(old);
        notify(wake);
        Ok(scope)
    }

    pub fn deactivate(&self, close: bool) {
        let (old, wake) = {
            let mut state = self.lock();
            state.closed |= close;
            state.scope = None;
            state.bytes = 0;
            (std::mem::take(&mut state.entries), state.ui_wake.take())
        };
        discard(old);
        notify(wake);
    }

    pub async fn request(
        self: &Arc<Self>,
        scope: Option<Scope>,
        payload: Arc<Payload>,
    ) -> Result<Response, Error> {
        let remaining = {
            let state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let (active, owner) = state.scope.as_ref().ok_or(Error::Stale)?;
            if scope != Some(*active) || !payload.belongs_to(owner) {
                return Err(Error::Stale);
            }
            if state.entries.len() >= self.limits.max_pending {
                return Err(Error::Busy);
            }
            self.limits.max_payload_bytes - state.bytes
        };
        let bytes = payload.bytes(remaining)?;
        let (token, wake) = {
            let mut state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let (active, owner) = state.scope.as_ref().ok_or(Error::Stale)?;
            if scope != Some(*active) || !payload.belongs_to(owner) {
                return Err(Error::Stale);
            }
            let total = state.bytes.checked_add(bytes).ok_or(Error::Limit)?;
            if state.entries.len() >= self.limits.max_pending {
                return Err(Error::Busy);
            }
            if total > self.limits.max_payload_bytes {
                return Err(Error::Limit);
            }
            let next = state.next_request.checked_add(1).ok_or(Error::Exhausted)?;
            let token = Token {
                identity: Arc::clone(&self.identity),
                scope: *active,
                generation: state.next_request,
            };
            state.next_request = next;
            state.bytes = total;
            state.entries.push_back(Entry {
                token: token.clone(),
                payload,
                bytes,
                displayed: false,
                response: None,
                wake: None,
            });
            (token, state.ui_wake.take())
        };
        // Install cleanup ownership before delivering any reentrant callback.
        let pending = Pending {
            shared: Arc::clone(self),
            token,
        };
        let expected = pending.token.scope;
        notify(wake);
        let result = poll_fn(|cx| pending.poll(cx)).await;
        drop(pending);
        if self.scope() != Some(expected) {
            return Err(Error::Stale);
        }
        result
    }

    pub fn poll_prompt(&self, cx: &mut Context<'_>) -> Poll<Option<View>> {
        let wake = cx.waker().clone();
        let (result, old) = {
            let mut state = self.lock();
            if state.closed {
                (Poll::Ready(None), Some(wake))
            } else if let Some(entry) = state
                .entries
                .iter_mut()
                .find(|entry| entry.response.is_none())
            {
                entry.displayed = true;
                (
                    Poll::Ready(Some(View {
                        token: entry.token.clone(),
                        payload: Arc::clone(&entry.payload),
                    })),
                    Some(wake),
                )
            } else {
                (Poll::Pending, state.ui_wake.replace(wake))
            }
        };
        drop(old);
        result
    }

    pub fn reply(&self, token: &Token, response: Response) -> Result<(), Error> {
        let (prompt_wake, ui_wake) = {
            let mut state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let entry = state
                .entries
                .iter_mut()
                .find(|entry| entry.token == *token)
                .ok_or(Error::Stale)?;
            if !entry.displayed || entry.response.is_some() {
                return Err(Error::Stale);
            }
            entry.payload.validate_response(&response)?;
            entry.response = Some(response);
            (entry.wake.take(), state.ui_wake.take())
        };
        notify(prompt_wake);
        notify(ui_wake);
        Ok(())
    }

    pub fn cancel(&self, token: &Token) -> Result<(), Error> {
        let response = {
            let state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let entry = state
                .entries
                .iter()
                .find(|entry| entry.token == *token)
                .ok_or(Error::Stale)?;
            match entry.payload.as_ref() {
                Payload::Permission(_) => Response::Permission(PermissionPromptDecision::Deny),
                Payload::Question { .. } => Response::Question(QuestionPromptOutcome::Cancelled),
            }
        };
        self.reply(token, response)
    }

    fn remove(&self, token: &Token) -> (Option<Entry>, Option<Waker>) {
        let mut state = self.lock();
        let removed = state
            .entries
            .iter()
            .position(|entry| entry.token == *token)
            .and_then(|index| state.entries.remove(index));
        let wake = if let Some(entry) = &removed {
            state.bytes -= entry.bytes;
            state.ui_wake.take()
        } else {
            None
        };
        (removed, wake)
    }
}

struct Pending {
    shared: Arc<Shared>,
    token: Token,
}
impl Pending {
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Result<Response, Error>> {
        let wake = cx.waker().clone();
        let (result, old) = {
            let mut state = self.shared.lock();
            if state.closed {
                (Poll::Ready(Err(Error::Closed)), Some(wake))
            } else if let Some(entry) = state
                .entries
                .iter_mut()
                .find(|entry| entry.token == self.token)
            {
                if let Some(response) = entry.response.take() {
                    (Poll::Ready(Ok(response)), Some(wake))
                } else {
                    (Poll::Pending, entry.wake.replace(wake))
                }
            } else {
                (Poll::Ready(Err(Error::Stale)), Some(wake))
            }
        };
        drop(old);
        result
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        let (removed, wake) = self.shared.remove(&self.token);
        drop(removed);
        notify(wake);
    }
}

fn discard(entries: VecDeque<Entry>) {
    for mut entry in entries {
        let wake = entry.wake.take();
        drop(entry);
        notify(wake);
    }
}
fn notify(wake: Option<Waker>) {
    if let Some(wake) = wake {
        wake.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::super::{NativeInteractivePromptBridge, NativeInteractivePromptLimits};
    use super::*;
    use crate::PermissionPrompter;
    use futures_executor::block_on;
    use machine_god_core::{
        Capability, PermissionRequest, PermissionRequestId, PermissionRisk, SessionId,
        SessionIncarnationId, TurnId,
    };
    use serde_json::Value;

    fn principal() -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new("session").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        )
    }
    fn request() -> PermissionRequest {
        PermissionRequest {
            id: PermissionRequestId::new("request").unwrap(),
            session_id: principal().session_id().clone(),
            session_incarnation_id: principal().session_incarnation_id().clone(),
            turn_id: TurnId::new("turn").unwrap(),
            capability: Capability::Custom {
                name: "test".into(),
                details: Value::Null,
            },
            risk: PermissionRisk::Low,
            reason: "reason".into(),
        }
    }

    #[test]
    fn generation_exhaustion_never_wraps_or_discards_existing_scope() {
        let (bridge, mut inbox) =
            NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
        let scope = inbox.activate(principal()).unwrap();
        bridge.shared.state.lock().unwrap().next_scope = u64::MAX;
        assert_eq!(inbox.activate(principal()), Err(Error::Exhausted));
        assert_eq!(bridge.shared.scope(), Some(scope));
        bridge.shared.state.lock().unwrap().next_request = u64::MAX;
        assert!(block_on(bridge.prompt(request())).is_err());
        let state = bridge.shared.state.lock().unwrap();
        assert!(state.entries.is_empty());
        assert_eq!(state.bytes, 0);
    }

    #[test]
    fn poisoned_state_fails_closed_and_drop_still_releases_entries() {
        let (bridge, mut inbox) =
            NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
        inbox.activate(principal()).unwrap();
        let mut future = bridge.prompt(request());
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        let shared = bridge.shared.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = shared.state.lock().unwrap();
                panic!("test poison");
            })
            .join()
            .is_err()
        );
        assert!(block_on(future).is_err());
        assert_eq!(inbox.activate(principal()), Err(Error::Closed));
        inbox.close();
        let state = bridge.shared.state.lock().err().unwrap().into_inner();
        assert!(state.entries.is_empty());
        assert_eq!(state.bytes, 0);
    }
}
