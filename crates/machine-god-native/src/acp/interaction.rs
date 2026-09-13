//! Bounded ACP client URL correlation on the native human prompt inbox.

use crate::{
    NativeInteractivePromptBridge,
    mcp::interaction::{
        McpClientUrlCompletion, McpClientUrlCompletionObserver, McpClientUrlEndpoint,
        McpClientUrlOutcome, McpElicitationAnswer, McpElicitationPresenter,
        McpElicitationPromptError, McpElicitationPromptRequest, McpElicitationPromptSource,
    },
};
use machine_god_core::{BackgroundOutputOwner, BoxFuture, CancellationToken};
use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll, Waker},
};

const MAX_REGISTRATIONS: usize = 256;
const MAX_RETAINED_BYTES: usize = 8 * 1024 * 1024;

/// Host-generated correlation label, not authority. IDs never repeat within
/// this endpoint, including after deactivation and same-session reactivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeAcpElicitationId(u64);
impl fmt::Display for NativeAcpElicitationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "machine-god-url-{}", self.0)
    }
}

/// Only accepted URLs whose actual native operation completed reach this queue.
/// Retain the exact principal when staging a wire notification.
#[derive(Clone)]
pub struct NativeAcpElicitationComplete {
    pub id: NativeAcpElicitationId,
    pub owner: BackgroundOutputOwner,
}
impl fmt::Debug for NativeAcpElicitationComplete {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAcpElicitationComplete { <redacted> }")
    }
}

struct Entry {
    request: Option<McpElicitationPromptRequest>,
    owner: BackgroundOutputOwner,
    charge: usize,
    submitted: bool,
}
#[derive(Default)]
struct State {
    owner: Option<BackgroundOutputOwner>,
    next_id: u64,
    entries: BTreeMap<u64, Entry>,
    ready: VecDeque<NativeAcpElicitationComplete>,
    bytes: usize,
    waker: Option<Waker>,
}

/// Explicit ACP presentation endpoint; constructing it performs no effects.
/// Permission/question presentation uses the same injected bridge directly.
pub struct NativeAcpElicitationPresenter {
    bridge: Arc<NativeInteractivePromptBridge>,
    state: Arc<Mutex<State>>,
}
impl fmt::Debug for NativeAcpElicitationPresenter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAcpElicitationPresenter { <redacted> }")
    }
}
impl NativeAcpElicitationPresenter {
    #[must_use]
    pub fn new(bridge: Arc<NativeInteractivePromptBridge>) -> Self {
        Self {
            bridge,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    /// Change the exact client principal, invalidating all old registrations and
    /// queued completions. The driver also activates its actual native inbox.
    pub fn activate(&self, owner: BackgroundOutputOwner) {
        self.replace_owner(Some(owner));
    }

    /// EOF, close or replacement invalidates custody, never synthesizes success.
    pub fn deactivate(&self) {
        self.replace_owner(None);
    }

    fn replace_owner(&self, owner: Option<BackgroundOutputOwner>) {
        let wake = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.entries.clear();
            state.ready.clear();
            state.bytes = 0;
            state.owner = owner;
            state.waker.take()
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }

    /// Finds a host registration by exact native request allocation and source,
    /// not by server-supplied URL, message, ID or another request's answer.
    /// # Errors
    /// Fails for stale, unregistered or already-submitted requests.
    pub fn id_for(
        &self,
        request: &McpElicitationPromptRequest,
    ) -> Result<NativeAcpElicitationId, McpElicitationPromptError> {
        let state = self
            .state
            .lock()
            .map_err(|_| McpElicitationPromptError::Unavailable)?;
        state
            .entries
            .iter()
            .find_map(|(&id, entry)| {
                entry
                    .request
                    .as_ref()
                    .filter(|selected| same_request(selected, request))
                    .map(|_| NativeAcpElicitationId(id))
            })
            .ok_or(McpElicitationPromptError::InvalidSource)
    }

    /// Mark an exact request admitted to the client's bounded output lane.
    /// This does not accept its answer or complete its native operation.
    /// # Errors
    /// Rejects stale, unknown and duplicate submission without mutation.
    pub fn mark_submitted(
        &self,
        id: NativeAcpElicitationId,
    ) -> Result<(), McpElicitationPromptError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| McpElicitationPromptError::Unavailable)?;
        let entry = state
            .entries
            .get_mut(&id.0)
            .ok_or(McpElicitationPromptError::InvalidSource)?;
        if entry.submitted {
            return Err(McpElicitationPromptError::InvalidSource);
        }
        entry.submitted = true;
        entry.request = None;
        let released = entry.charge;
        entry.charge = 0;
        state.bytes -= released;
        Ok(())
    }

    /// Poll one bounded completion without blocking or performing output I/O.
    pub fn poll_complete(
        &self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<NativeAcpElicitationComplete>> {
        // RawWaker clone/drop hooks are caller code, just like wake. Keep all
        // three outside the state mutex so reentrant observers cannot deadlock.
        let mut incoming = Some(cx.waker().clone());
        let (result, previous) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(completion) = state.ready.pop_front() {
                (Poll::Ready(Some(completion)), None)
            } else if state.owner.is_none() {
                (Poll::Ready(None), None)
            } else {
                (
                    Poll::Pending,
                    std::mem::replace(&mut state.waker, incoming.take()),
                )
            }
        };
        drop(previous);
        result
    }
}

fn same_request(a: &McpElicitationPromptRequest, b: &McpElicitationPromptRequest) -> bool {
    if !Arc::ptr_eq(a.request(), b.request()) || a.server() != b.server() {
        return false;
    }
    match (a.source(), b.source()) {
        (
            McpElicitationPromptSource::ModelTool {
                context: a,
                tool: at,
            },
            McpElicitationPromptSource::ModelTool {
                context: b,
                tool: bt,
            },
        ) => {
            a.session_id == b.session_id
                && a.session_incarnation_id == b.session_incarnation_id
                && a.turn_id == b.turn_id
                && a.call_id == b.call_id
                && at == bt
        }
        (
            McpElicitationPromptSource::HumanFeature {
                owner: a,
                action: aa,
            },
            McpElicitationPromptSource::HumanFeature {
                owner: b,
                action: ba,
            },
        ) => a == b && aa == ba,
        _ => false,
    }
}

impl McpClientUrlEndpoint for NativeAcpElicitationPresenter {
    fn register(
        &self,
        request: &McpElicitationPromptRequest,
    ) -> Result<McpClientUrlCompletion, McpElicitationPromptError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| McpElicitationPromptError::Unavailable)?;
        let owner = state
            .owner
            .as_ref()
            .filter(|owner| request.source().belongs_to(owner))
            .ok_or(McpElicitationPromptError::InvalidSource)?
            .clone();
        if request.request().mode() != crate::mcp::mrtr::McpElicitationMode::Url {
            return Err(McpElicitationPromptError::InvalidSource);
        }
        let bytes = state
            .bytes
            .checked_add(request.retained_byte_charge())
            .filter(|bytes| *bytes <= MAX_RETAINED_BYTES)
            .ok_or(McpElicitationPromptError::Limit)?;
        if state.entries.len() + state.ready.len() >= MAX_REGISTRATIONS {
            return Err(McpElicitationPromptError::Limit);
        }
        if state.entries.values().any(|entry| {
            entry
                .request
                .as_ref()
                .is_some_and(|selected| same_request(selected, request))
        }) {
            return Err(McpElicitationPromptError::InvalidSource);
        }
        let id = state
            .next_id
            .checked_add(1)
            .ok_or(McpElicitationPromptError::Limit)?;
        state.next_id = id;
        state.bytes = bytes;
        state.entries.insert(
            id,
            Entry {
                request: Some(request.clone()),
                owner,
                charge: request.retained_byte_charge(),
                submitted: false,
            },
        );
        Ok(McpClientUrlCompletion::new(Box::new(Completion {
            state: Arc::downgrade(&self.state),
            id,
        })))
    }
}
impl McpElicitationPresenter for NativeAcpElicitationPresenter {
    fn client_urls(&self) -> Option<&dyn McpClientUrlEndpoint> {
        Some(self)
    }
    fn present(
        &self,
        request: McpElicitationPromptRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpElicitationAnswer, McpElicitationPromptError>> {
        self.bridge.present(request, cancellation)
    }
}
struct Completion {
    state: Weak<Mutex<State>>,
    id: u64,
}
impl McpClientUrlCompletionObserver for Completion {
    fn finish(self: Box<Self>, outcome: McpClientUrlOutcome) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let wake = {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(entry) = state.entries.remove(&self.id) else {
                return;
            };
            state.bytes -= entry.charge;
            if outcome == McpClientUrlOutcome::Completed
                && entry.submitted
                && state.owner.as_ref() == Some(&entry.owner)
            {
                state.ready.push_back(NativeAcpElicitationComplete {
                    id: NativeAcpElicitationId(self.id),
                    owner: entry.owner,
                });
                state.waker.take()
            } else {
                None
            }
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}

#[cfg(test)]
mod tests;
