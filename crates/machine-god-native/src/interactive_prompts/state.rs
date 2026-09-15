use super::payload::{AcceptedResponse, Payload};
use super::{
    MAX_NATIVE_INTERACTIVE_PROMPT_PAGE_BYTES, MAX_NATIVE_INTERACTIVE_PROMPT_PRINCIPALS,
    MAX_NATIVE_INTERACTIVE_PROMPTS, NativeInteractivePromptError as Error,
    NativeInteractivePromptKind as Kind, NativeInteractivePromptLimits,
    NativeInteractivePromptPage as Page, NativeInteractivePromptResponse as Response,
    NativeInteractivePromptScope as Scope, NativeInteractivePromptSummary as Summary,
    NativeInteractivePromptToken as Token, NativeInteractivePromptView as View, PrincipalKey,
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
    principals: Vec<PrincipalKey>,
    next_scope: u64,
    next_request: u64,
    bytes: usize,
    response_bytes: usize,
    entries: VecDeque<Entry>,
    ui_wake: Option<Waker>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            closed: false,
            principals: Vec::new(),
            next_scope: 1,
            next_request: 1,
            bytes: 0,
            response_bytes: 0,
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
    answered: bool,
    response: Option<AcceptedResponse>,
    response_bytes: usize,
    wake: Option<Waker>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.invalidate_rule();
    }
}

impl Entry {
    fn invalidate_rule(&self) {
        if let Payload::Permission {
            rule: Some(rule), ..
        } = self.payload.as_ref()
        {
            rule.invalidate();
        }
    }
}

impl Shared {
    pub fn page(
        &self,
        cx: &mut Context<'_>,
        after: Option<&Token>,
        limit: usize,
    ) -> Result<Page, Error> {
        if !(1..=MAX_NATIVE_INTERACTIVE_PROMPTS).contains(&limit) {
            return Err(Error::Limit);
        }
        if after.is_some_and(|token| !Arc::ptr_eq(&token.identity, &self.identity)) {
            return Err(Error::Stale);
        }
        let wake = cx.waker().clone();
        let (page, old) = {
            let mut state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let mut entries = Vec::with_capacity(limit);
            let mut bytes = std::mem::size_of::<Page>() + limit * std::mem::size_of::<Summary>();
            let mut more = false;
            for entry in state.entries.iter().filter(|entry| {
                !entry.answered
                    && after.is_none_or(|after| entry.token.generation > after.generation)
            }) {
                // Reserve both the row's strings and a possible continuation
                // clone before allocation. There are no payload Arc clones.
                let identity_bytes = entry.token.owner.session_id().as_str().len()
                    + entry.token.owner.session_incarnation_id().as_str().len();
                if entries.len() == limit
                    || bytes + 2 * identity_bytes > MAX_NATIVE_INTERACTIVE_PROMPT_PAGE_BYTES
                {
                    more = true;
                    break;
                }
                bytes += 2 * identity_bytes;
                let kind = match entry.payload.as_ref() {
                    Payload::Permission { .. } => Kind::Permission,
                    Payload::Question { .. } => Kind::Question,
                    Payload::Elicitation { .. } => Kind::Elicitation,
                    Payload::UrlRecovery { .. } => Kind::UrlRecovery,
                };
                entries.push(Summary {
                    token: entry.token.clone(),
                    kind,
                });
            }
            let next = if more {
                entries.last().map(|entry| entry.token.clone())
            } else {
                None
            };
            (Page { entries, next }, state.ui_wake.replace(wake))
        };
        drop(old);
        Ok(page)
    }

    pub fn select_prompt(&self, token: &Token) -> Result<View, Error> {
        let mut state = self.lock();
        if state.closed {
            return Err(Error::Closed);
        }
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| entry.token == *token && !entry.answered)
            .ok_or(Error::Stale)?;
        entry.displayed = true;
        Ok(View {
            token: entry.token.clone(),
            payload: Arc::clone(&entry.payload),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poison| {
            let mut state = poison.into_inner();
            // Never recover possible partial admission into new authority.
            state.closed = true;
            for entry in &state.entries {
                entry.invalidate_rule();
            }
            state
        })
    }

    pub fn capture(&self, bound: Option<&PrincipalKey>, payload: &Payload) -> Option<PrincipalKey> {
        let state = self.lock();
        if state.closed {
            return None;
        }
        state
            .principals
            .iter()
            .find(|key| bound.is_none_or(|bound| bound == *key) && payload.belongs_to(&key.owner))
            .cloned()
    }

    fn active(&self, key: &PrincipalKey) -> bool {
        let state = self.lock();
        !state.closed && state.principals.contains(key)
    }

    pub fn register(&self, owner: BackgroundOutputOwner) -> Result<PrincipalKey, Error> {
        let mut state = self.lock();
        if state.closed {
            return Err(Error::Closed);
        }
        if state.principals.len() >= MAX_NATIVE_INTERACTIVE_PROMPT_PRINCIPALS
            || state.principals.iter().any(|key| key.owner == owner)
        {
            return Err(Error::Busy);
        }
        let next = state.next_scope.checked_add(1).ok_or(Error::Exhausted)?;
        let key = PrincipalKey {
            scope: Scope(state.next_scope),
            owner,
        };
        state.next_scope = next;
        state.principals.push(key.clone());
        Ok(key)
    }

    pub fn retire(&self, key: &PrincipalKey) {
        let (old, retired, wake) = {
            let mut state = self.lock();
            let Some(index) = state.principals.iter().position(|active| active == key) else {
                return;
            };
            let retired = state.principals.remove(index);
            let mut old = VecDeque::new();
            let mut index = 0;
            while index < state.entries.len() {
                if state.entries[index].token.scope != key.scope {
                    index += 1;
                    continue;
                }
                let entry = state.entries.remove(index).expect("existing entry");
                entry.invalidate_rule();
                state.bytes -= entry.bytes;
                state.response_bytes -= entry.response_bytes;
                old.push_back(entry);
            }
            (old, retired, state.ui_wake.take())
        };
        drop(retired);
        discard(old);
        notify(wake);
    }

    pub fn close(&self) {
        let (old, principals, wake) = {
            let mut state = self.lock();
            state.closed = true;
            for entry in &state.entries {
                entry.invalidate_rule();
            }
            state.bytes = 0;
            state.response_bytes = 0;
            (
                std::mem::take(&mut state.entries),
                std::mem::take(&mut state.principals),
                state.ui_wake.take(),
            )
        };
        drop(principals);
        discard(old);
        notify(wake);
    }

    pub async fn request(
        self: &Arc<Self>,
        principal: Option<PrincipalKey>,
        payload: Arc<Payload>,
    ) -> Result<AcceptedResponse, Error> {
        let remaining = {
            let state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let key = principal.as_ref().ok_or(Error::Stale)?;
            if !state.principals.contains(key) || !payload.belongs_to(&key.owner) {
                return Err(Error::Stale);
            }
            if state.entries.len() >= self.limits.pending {
                return Err(Error::Busy);
            }
            self.limits.payload_bytes - state.bytes
        };
        let bytes = payload.bytes(remaining)?;
        let (token, wake) = {
            let mut state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let key = principal.as_ref().ok_or(Error::Stale)?;
            if !state.principals.contains(key) || !payload.belongs_to(&key.owner) {
                return Err(Error::Stale);
            }
            let total = state.bytes.checked_add(bytes).ok_or(Error::Limit)?;
            if state.entries.len() >= self.limits.pending {
                return Err(Error::Busy);
            }
            if total > self.limits.payload_bytes {
                return Err(Error::Limit);
            }
            let next = state.next_request.checked_add(1).ok_or(Error::Exhausted)?;
            let token = Token {
                identity: Arc::clone(&self.identity),
                scope: key.scope,
                generation: state.next_request,
                owner: key.owner.clone(),
            };
            state.next_request = next;
            state.bytes = total;
            state.entries.push_back(Entry {
                token: token.clone(),
                payload,
                bytes,
                displayed: false,
                answered: false,
                response: None,
                response_bytes: 0,
                wake: None,
            });
            (token, state.ui_wake.take())
        };
        // Install cleanup ownership before delivering any reentrant callback.
        let pending = Pending {
            shared: Arc::clone(self),
            token,
        };
        notify(wake);
        let result = poll_fn(|cx| pending.poll(cx)).await;
        drop(pending);
        if !principal.as_ref().is_some_and(|key| self.active(key)) {
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
            } else if let Some(entry) = state.entries.iter_mut().find(|entry| !entry.answered) {
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn poll_pending(&self, token: &Token, cx: &mut Context<'_>) -> Poll<()> {
        let wake = cx.waker().clone();
        let (result, old) = {
            let mut state = self.lock();
            if !state.closed
                && state
                    .entries
                    .iter()
                    .any(|entry| entry.token == *token && !entry.answered)
            {
                (Poll::Pending, state.ui_wake.replace(wake))
            } else {
                (Poll::Ready(()), Some(wake))
            }
        };
        drop(old);
        result
    }

    pub fn reply(&self, token: &Token, response: Response) -> Result<(), Error> {
        let payload = {
            let state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let entry = state
                .entries
                .iter()
                .find(|entry| entry.token == *token && entry.displayed && !entry.answered)
                .ok_or(Error::Stale)?;
            Arc::clone(&entry.payload)
        };
        // Pattern/schema validation and owned-response destruction cannot run
        // inside the queue mutex. The second admission binds the same payload.
        let response = payload.admit_response(response)?;
        let bytes = response.bytes()?;
        let (prompt_wake, ui_wake) = {
            let mut state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let index = state
                .entries
                .iter()
                .position(|entry| entry.token == *token)
                .ok_or(Error::Stale)?;
            let total = state
                .response_bytes
                .checked_add(bytes)
                .ok_or(Error::Limit)?;
            if total > self.limits.response_bytes {
                return Err(Error::Limit);
            }
            let entry = &mut state.entries[index];
            if !entry.displayed || entry.answered || !Arc::ptr_eq(&entry.payload, &payload) {
                return Err(Error::Stale);
            }
            if let Payload::Permission {
                rule: Some(rule), ..
            } = entry.payload.as_ref()
            {
                rule.invalidate();
            }
            entry.response = Some(response);
            entry.answered = true;
            entry.response_bytes = bytes;
            let prompt_wake = entry.wake.take();
            state.response_bytes = total;
            (prompt_wake, state.ui_wake.take())
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
                Payload::Permission { .. } => Response::Permission(PermissionPromptDecision::Deny),
                Payload::Question { .. } => Response::Question(QuestionPromptOutcome::Cancelled),
                Payload::Elicitation { .. } => Response::Elicitation(
                    crate::mcp::interaction::McpElicitationAnswerInput::cancel(),
                ),
                Payload::UrlRecovery { .. } => {
                    Response::UrlRecovery(crate::mcp::interaction::McpUrlRecoveryAnswer::Cancel)
                }
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
            entry.invalidate_rule();
            state.bytes -= entry.bytes;
            state.response_bytes -= entry.response_bytes;
            state.ui_wake.take()
        } else {
            None
        };
        (removed, wake)
    }

    pub fn propose_rule_change(
        &self,
        token: &Token,
        decision: crate::NativePermissionRuleDecision,
    ) -> Result<crate::NativePermissionRuleProposal, Error> {
        let rule = {
            let state = self.lock();
            if state.closed {
                return Err(Error::Closed);
            }
            let entry = state
                .entries
                .iter()
                .find(|entry| entry.token == *token && entry.displayed && !entry.answered)
                .ok_or(Error::Stale)?;
            let Payload::Permission {
                rule: Some(rule), ..
            } = entry.payload.as_ref()
            else {
                return Err(Error::InvalidResponse);
            };
            rule.clone()
        };
        rule.propose(decision).map_err(|_| Error::Stale)
    }
}

struct Pending {
    shared: Arc<Shared>,
    token: Token,
}
impl Pending {
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Result<AcceptedResponse, Error>> {
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
    use super::super::{NativeInteractivePromptInbox, NativeInteractivePromptLimits};
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
        let mut inbox =
            NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
        let lease = inbox.register(principal()).unwrap();
        let bridge = lease.bridge();
        bridge.shared.state.lock().unwrap().next_scope = u64::MAX;
        let other = BackgroundOutputOwner::new(
            SessionId::new("other").unwrap(),
            principal().session_incarnation_id().clone(),
        );
        assert_eq!(inbox.register(other).unwrap_err(), Error::Exhausted);
        assert!(bridge.shared.active(&lease.key));
        bridge.shared.state.lock().unwrap().next_request = u64::MAX;
        assert!(block_on(bridge.prompt(request())).is_err());
        let state = bridge.shared.state.lock().unwrap();
        assert!(state.entries.is_empty());
        assert_eq!(state.bytes, 0);
    }

    #[test]
    fn poisoned_state_fails_closed_and_drop_still_releases_entries() {
        let mut inbox =
            NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
        let lease = inbox.register(principal()).unwrap();
        let bridge = lease.bridge();
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
        assert_eq!(inbox.register(principal()).unwrap_err(), Error::Closed);
        inbox.close();
        let state = bridge.shared.state.lock().err().unwrap().into_inner();
        assert!(state.entries.is_empty());
        assert_eq!(state.bytes, 0);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn pending_observer_waker_callbacks_run_outside_inbox_lock() {
        use machine_god_reentrant_waker_test::{Callback, new};
        for callback in [Callback::Clone, Callback::Drop, Callback::Wake] {
            let mut inbox =
                NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default())
                    .unwrap();
            let lease = inbox.register(principal()).unwrap();
            let bridge = lease.bridge();
            let mut pending = bridge.prompt(request());
            assert!(
                pending
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            let Poll::Ready(Some(view)) =
                inbox.poll_prompt(&mut Context::from_waker(Waker::noop()))
            else {
                panic!("view");
            };
            let shared = Arc::clone(&bridge.shared);
            let (waker, calls) = new(callback, move || {
                assert!(shared.state.try_lock().is_ok());
            });
            assert!(
                inbox
                    .poll_pending(view.token(), &mut Context::from_waker(&waker))
                    .is_pending()
            );
            assert!(
                inbox
                    .poll_pending(view.token(), &mut Context::from_waker(&waker))
                    .is_pending()
            );
            drop(pending);
            assert!(
                inbox
                    .poll_pending(view.token(), &mut Context::from_waker(&waker))
                    .is_ready()
            );
            drop(waker);
            assert!(calls.calls() > 0);
        }
    }
}
