use super::super::McpAuthInvalidated;
#[cfg(test)]
use super::tests;
use super::{
    Arc, Authority, CancellationToken, Credentials, Instant, McpAuthError, McpAuthIdentity,
    McpAuthInvalidation, McpAuthLease, McpAuthRemoteRevocation, NativeMcpCredentialStore,
    NativeOwnedWorkerScope, Result,
};
use futures_util::future::{Either, select};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

const MAX_OPERATIONS: usize = 128;
const MAX_IDENTITIES: usize = 64;

pub(super) struct Inner {
    pub store: Arc<NativeMcpCredentialStore>,
    pub authority: Authority,
    pub invalidation: Arc<dyn McpAuthInvalidation>,
    pub workers: NativeOwnedWorkerScope,
    pub state: Mutex<State>,
    #[cfg(test)]
    pub hooks: tests::Hooks,
}
#[derive(Default)]
pub(super) struct State {
    pub closed: bool,
    entries: BTreeMap<McpAuthIdentity, Entry>,
    pub operations: Vec<Arc<Observation>>,
}
struct Entry {
    slot: Arc<Slot>,
    operations: Vec<Arc<Observation>>,
    retirement: Option<Arc<Observation>>,
}
pub(super) struct Slot {
    pub generation: Mutex<CancellationToken>,
    pub retired: CancellationToken,
    pub cutoff: AtomicBool,
}
pub(super) struct Observation {
    pub done: CancellationToken,
    pub cancellation: CancellationToken,
    pub workers: AtomicUsize,
}
pub(super) struct Guard {
    pub inner: Arc<Inner>,
    pub slot: Arc<Slot>,
    pub identity: McpAuthIdentity,
    pub observation: Arc<Observation>,
    pub cancellation: CancellationToken,
}
pub(super) struct Operation {
    pub guard: Arc<Guard>,
}
impl Drop for Operation {
    fn drop(&mut self) {
        cancel(&self.guard.cancellation);
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        cancel(&self.observation.done);
    }
}
pub(super) fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub(super) fn contain(operation: impl FnOnce()) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        std::mem::forget(payload);
    }
}
pub(super) fn cancel(token: &CancellationToken) {
    contain(|| {
        token.cancel();
    });
}

impl Entry {
    fn new() -> Self {
        Self {
            slot: Arc::new(Slot {
                generation: Mutex::new(CancellationToken::new()),
                retired: CancellationToken::new(),
                cutoff: AtomicBool::new(false),
            }),
            operations: Vec::new(),
            retirement: None,
        }
    }
    fn prune(&mut self) {
        self.operations.retain(|value| !value.done.is_cancelled());
    }
}
impl Inner {
    pub fn new(
        store: Arc<NativeMcpCredentialStore>,
        authority: Authority,
        invalidation: Arc<dyn McpAuthInvalidation>,
        workers: NativeOwnedWorkerScope,
    ) -> Self {
        Self {
            store,
            authority,
            invalidation,
            workers,
            state: Mutex::default(),
            #[cfg(test)]
            hooks: tests::Hooks::default(),
        }
    }
    pub fn begin(
        self: &Arc<Self>,
        identity: &McpAuthIdentity,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<Operation> {
        self.reserve(identity, caller, deadline, false)
            .map(|value| value.0)
    }
    pub fn cutoff(
        self: &Arc<Self>,
        identity: &McpAuthIdentity,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<(Operation, Vec<Arc<Observation>>)> {
        self.reserve(identity, caller, deadline, true)
    }
    fn reserve(
        self: &Arc<Self>,
        identity: &McpAuthIdentity,
        caller: &CancellationToken,
        deadline: Instant,
        cutoff: bool,
    ) -> Result<(Operation, Vec<Arc<Observation>>)> {
        self.authority.check(caller, deadline)?;
        let mut state = lock(&self.state);
        if state.closed {
            return Err(McpAuthError::Unavailable);
        }
        state.operations.retain(|value| !value.done.is_cancelled());
        if state.operations.len() == MAX_OPERATIONS {
            return Err(McpAuthError::Limit);
        }
        if !state.entries.contains_key(identity) && state.entries.len() == MAX_IDENTITIES {
            return Err(McpAuthError::Limit);
        }
        let entry = state
            .entries
            .entry(identity.clone())
            .or_insert_with(Entry::new);
        entry.prune();
        if entry
            .retirement
            .as_ref()
            .is_some_and(|value| !value.done.is_cancelled())
            || !cutoff && !entry.operations.is_empty()
        {
            return Err(McpAuthError::Busy);
        }
        if !cutoff && entry.slot.cutoff.load(Ordering::Acquire) {
            *entry = Entry::new();
        }
        let previous = if cutoff {
            entry.operations.clone()
        } else {
            Vec::new()
        };
        let newly_retired = cutoff && !entry.slot.cutoff.swap(true, Ordering::AcqRel);
        let observation = Arc::new(Observation {
            done: CancellationToken::new(),
            cancellation: CancellationToken::new(),
            workers: AtomicUsize::new(0),
        });
        entry.operations.push(observation.clone());
        if cutoff {
            entry.retirement = Some(observation.clone());
        }
        let slot = entry.slot.clone();
        state.operations.push(observation.clone());
        drop(state);
        let operation = Operation {
            guard: Arc::new(Guard {
                inner: self.clone(),
                slot: slot.clone(),
                identity: identity.clone(),
                cancellation: observation.cancellation.clone(),
                observation,
            }),
        };
        if newly_retired {
            self.invalidate(identity, &slot);
        }
        Ok((operation, previous))
    }
    pub fn invalidate(&self, identity: &McpAuthIdentity, slot: &Slot) {
        let generation = lock(&slot.generation).clone();
        cancel(&slot.retired);
        cancel(&generation);
        contain(|| {
            self.invalidation.invalidate(McpAuthInvalidated {
                identity: identity.clone(),
                generation,
            });
        });
    }
    pub fn release_retired(&self, guard: &Guard) {
        let mut state = lock(&self.state);
        if state
            .entries
            .get(&guard.identity)
            .is_some_and(|entry| Arc::ptr_eq(&entry.slot, &guard.slot))
        {
            state.entries.remove(&guard.identity);
        }
    }
    pub fn close(&self) {
        let (entries, operations) = {
            let mut state = lock(&self.state);
            if state.closed {
                return;
            }
            state.closed = true;
            let entries: Vec<_> = state
                .entries
                .iter()
                .map(|(identity, entry)| {
                    entry.slot.cutoff.store(true, Ordering::Release);
                    (identity.clone(), entry.slot.clone())
                })
                .collect();
            (entries, state.operations.clone())
        };
        for (identity, slot) in entries {
            self.invalidate(&identity, &slot);
        }
        for observation in operations {
            cancel(&observation.cancellation);
        }
    }
}
impl Guard {
    /// Publication reservation linearizes under this short lock. No I/O, clock
    /// callback, cancellation or user waker is invoked while holding it.
    pub fn check(
        &self,
        caller: &CancellationToken,
        deadline: Instant,
        allow_cutoff: bool,
    ) -> Result<()> {
        self.inner.authority.check(caller, deadline)?;
        if self.cancellation.is_cancelled() {
            return Err(McpAuthError::Cancelled);
        }
        let state = lock(&self.inner.state);
        if state.closed {
            return Err(McpAuthError::Unavailable);
        }
        if !state
            .entries
            .get(&self.identity)
            .is_some_and(|entry| Arc::ptr_eq(&entry.slot, &self.slot))
            || !allow_cutoff && self.slot.cutoff.load(Ordering::Acquire)
        {
            return Err(McpAuthError::Conflict);
        }
        Ok(())
    }
}
impl Operation {
    pub async fn run<T>(
        &self,
        future: impl std::future::Future<Output = Result<T>>,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<T> {
        self.guard
            .inner
            .authority
            .bounded(
                async {
                    if self.guard.slot.cutoff.load(Ordering::Acquire) {
                        return Err(McpAuthError::Conflict);
                    }
                    match select(
                        Box::pin(self.guard.slot.retired.cancelled()),
                        Box::pin(future),
                    )
                    .await
                    {
                        Either::Right((result, _)) => result,
                        Either::Left(_) => Err(McpAuthError::Conflict),
                    }
                },
                caller,
                deadline,
            )
            .await
    }
    pub fn lease(
        &self,
        credentials: Credentials,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<McpAuthLease> {
        self.guard.check(caller, deadline, false)?;
        Ok(McpAuthLease {
            credentials: Arc::new(credentials),
            generation: lock(&self.guard.slot.generation).clone(),
        })
    }
    pub async fn wait_previous(
        &self,
        previous: &[Arc<Observation>],
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<()> {
        self.guard
            .inner
            .authority
            .bounded(
                async {
                    let completed = async {
                        for observation in previous {
                            observation.done.cancelled().await;
                        }
                    };
                    match select(
                        Box::pin(completed),
                        Box::pin(self.guard.cancellation.cancelled()),
                    )
                    .await
                    {
                        Either::Left(_) => Ok(()),
                        Either::Right(_) => Err(McpAuthError::Cancelled),
                    }
                },
                caller,
                deadline,
            )
            .await
    }

    pub async fn revoke(
        &self,
        credentials: &Credentials,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> McpAuthRemoteRevocation {
        let mut operation = Box::pin(self.guard.inner.authority.revoke(
            credentials,
            &self.guard.cancellation,
            deadline,
        ));
        let mut cancelled = Box::pin(caller.cancelled());
        // Forward cancellation, then poll the real revocation outcome. Never
        // erase an already accepted local removal or an attempted POST receipt.
        std::future::poll_fn(|cx| {
            if caller.is_cancelled() || cancelled.as_mut().poll(cx).is_ready() {
                cancel(&self.guard.cancellation);
            }
            operation.as_mut().poll(cx)
        })
        .await
    }
}
