//! Host-local access handoff. Storage owners and backend handles never move.
use super::{HostState, Requester};
use crate::terminal_catalog_view::TerminalCatalogViewError;
use crate::terminal_owner::{TerminalOwnerContext, TerminalOwnerError};
use crate::terminal_profile_store::{MAX_PROFILE_OWNERS, MAX_PROFILE_SESSIONS};
use crate::terminal_registry::TerminalRegistry;
use crate::terminal_runtime::TerminalRuntimeError;
use crate::terminal_session::TerminalSessionBackend;
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, TerminalActorRole, TerminalClosePolicy,
    TerminalSessionId,
};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

/// Fixed redacted failure; a failed preparation never changes access routes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTerminalTransitionError {
    Cancelled,
    Closed,
    Conflict,
    Capacity,
    Preparation,
    /// The owner may have started effects; do not infer rollback or retry them.
    Uncertain,
}
impl fmt::Display for NativeTerminalTransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("terminal transition unavailable")
    }
}
impl std::error::Error for NativeTerminalTransitionError {}
type Result<T> = std::result::Result<T, NativeTerminalTransitionError>;

#[cfg(test)]
#[path = "terminal_host_lifecycle/tests.rs"]
mod tests;

#[derive(Debug)]
pub struct NativeTerminalHandoffReceipt {
    transferred: usize,
}
impl NativeTerminalHandoffReceipt {
    #[must_use]
    pub fn transferred(&self) -> usize {
        self.transferred
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTerminalResetOutcome {
    StoppedAndForgotten,
    AlreadyTerminatedForgotten,
    RetainedIndeterminate,
}
#[derive(Debug)]
pub struct NativeTerminalResetEntry {
    id: TerminalSessionId,
    outcome: NativeTerminalResetOutcome,
}
impl NativeTerminalResetEntry {
    #[must_use]
    pub fn id(&self) -> &TerminalSessionId {
        &self.id
    }
    #[must_use]
    pub fn outcome(&self) -> NativeTerminalResetOutcome {
        self.outcome
    }
}
#[derive(Debug)]
pub struct NativeTerminalResetReceipt {
    entries: Vec<NativeTerminalResetEntry>,
}
impl NativeTerminalResetReceipt {
    #[must_use]
    pub fn entries(&self) -> &[NativeTerminalResetEntry] {
        &self.entries
    }
}

/// A requester, not a host lifetime vote. Construction and futures are inert.
#[derive(Clone)]
pub struct NativeTerminalLifecycleRequester {
    pub(super) requester: Requester,
}
impl fmt::Debug for NativeTerminalLifecycleRequester {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeTerminalLifecycleRequester { .. }")
    }
}
impl NativeTerminalLifecycleRequester {
    /// Explicitly resume a retired conversation with a fresh access generation.
    /// Previously captured operations never regain their cancelled generation.
    #[must_use]
    pub fn activate_session(
        &self,
        owner: BackgroundOutputOwner,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<()>> {
        let requester = self.requester.clone();
        Box::pin(async move {
            requester
                .request_with_context(cancellation, move |context| {
                    context.state.access.activate(owner)
                })
                .await
                .map_err(runtime_error)?
        })
    }

    /// After enqueue, dropping this future does not establish rollback. Drive
    /// the operation to its receipt before admitting a competing transition.
    #[must_use]
    pub fn handoff(
        &self,
        source: BackgroundOutputOwner,
        destination: BackgroundOutputOwner,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalHandoffReceipt>> {
        let requester = self.requester.clone();
        Box::pin(async move {
            requester
                .request_with_context(cancellation, move |context| {
                    handoff(context, &source, &destination)
                })
                .await
                .map_err(runtime_error)?
        })
    }

    /// Stops only exact resources routed to this principal in this host's
    /// retained workspace. Cancellation after work begins still returns all
    /// per-resource outcomes. Indeterminate authority stays retained.
    #[must_use]
    pub fn reset_current_workspace(
        &self,
        owner: BackgroundOutputOwner,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeTerminalResetReceipt>> {
        let requester = self.requester.clone();
        Box::pin(async move {
            requester
                .request_with_context(cancellation, move |mut context| reset(&mut context, &owner))
                .await
                .map_err(runtime_error)?
        })
    }
}

fn runtime_error(error: TerminalRuntimeError) -> NativeTerminalTransitionError {
    match error {
        TerminalRuntimeError::Owner(TerminalOwnerError::Cancelled) => {
            NativeTerminalTransitionError::Cancelled
        }
        TerminalRuntimeError::Owner(TerminalOwnerError::Closed) => {
            NativeTerminalTransitionError::Closed
        }
        _ => NativeTerminalTransitionError::Uncertain,
    }
}

#[derive(Clone)]
struct Principal {
    owner: BackgroundOutputOwner,
    revoked: CancellationToken,
    writer: crate::terminal_input::TerminalWriterId,
}
#[derive(Default)]
struct PrincipalState {
    principals: Vec<Principal>,
    generation: u64,
    closed: bool,
}
/// Pure bounded ingress state. No filesystem/worker/host lifetime authority;
/// cancellation wakers are never invoked while this mutex is held.
#[derive(Clone, Default)]
pub(crate) struct TerminalAccessPrincipals(Arc<Mutex<PrincipalState>>);
impl TerminalAccessPrincipals {
    /// Observation only: background queries cannot activate a principal.
    pub(super) fn current(
        &self,
        owner: &BackgroundOutputOwner,
    ) -> Option<(CancellationToken, crate::terminal_input::TerminalWriterId)> {
        let state = self.lock();
        if state.closed {
            return None;
        }
        state
            .principals
            .iter()
            .find(|p| &p.owner == owner)
            .filter(|p| !p.revoked.is_cancelled())
            .map(|p| (p.revoked.clone(), p.writer))
    }

    pub(super) fn same_registry(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(super) fn retire_all(&self) {
        let tokens: Vec<_> = {
            let mut state = self.lock();
            state.closed = true;
            state
                .principals
                .iter()
                .map(|principal| principal.revoked.clone())
                .collect()
        };
        for token in tokens {
            token.cancel();
        }
    }

    fn lock(&self) -> MutexGuard<'_, PrincipalState> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(crate) fn acquire(
        &self,
        owner: BackgroundOutputOwner,
    ) -> Result<(CancellationToken, crate::terminal_input::TerminalWriterId)> {
        let mut state = self.lock();
        if state.closed {
            return Err(NativeTerminalTransitionError::Closed);
        }
        if let Some(principal) = state.principals.iter().find(|p| p.owner == owner) {
            return if principal.revoked.is_cancelled() {
                Err(NativeTerminalTransitionError::Conflict)
            } else {
                Ok((principal.revoked.clone(), principal.writer))
            };
        }
        state.activate(owner)?;
        let principal = state.principals.last().expect("inserted principal");
        Ok((principal.revoked.clone(), principal.writer))
    }
    fn activate(&self, owner: BackgroundOutputOwner) -> Result<()> {
        self.lock().activate(owner)
    }
    fn contains(&self, owner: &BackgroundOutputOwner) -> bool {
        self.lock().principals.iter().any(|p| &p.owner == owner)
    }
    fn retire(&self, owner: &BackgroundOutputOwner) {
        let revoked = self
            .lock()
            .principals
            .iter()
            .find(|p| &p.owner == owner)
            .map(|p| p.revoked.clone());
        if let Some(revoked) = revoked {
            revoked.cancel();
        }
    }
    fn writer(&self, owner: &BackgroundOutputOwner) -> crate::terminal_input::TerminalWriterId {
        self.lock()
            .principals
            .iter()
            .find(|p| &p.owner == owner)
            .map_or_else(
                || crate::terminal_input::TerminalWriterId::new(std::num::NonZeroU64::MIN),
                |p| p.writer,
            )
    }
}
#[derive(Clone)]
pub(crate) struct AccessRoute {
    pub(crate) storage: BackgroundOutputOwner,
    pub(crate) id: TerminalSessionId,
    current: Option<BackgroundOutputOwner>,
}
#[derive(Clone, Default)]
pub(crate) struct TerminalAccessRoutes {
    pub(super) principals: TerminalAccessPrincipals,
    routes: Vec<AccessRoute>,
}
impl TerminalAccessRoutes {
    pub(super) fn new(principals: TerminalAccessPrincipals) -> Self {
        Self {
            principals,
            routes: Vec::new(),
        }
    }
    #[cfg(test)]
    pub(crate) fn acquire(
        &mut self,
        owner: BackgroundOutputOwner,
    ) -> Result<(CancellationToken, crate::terminal_input::TerminalWriterId)> {
        self.principals.acquire(owner)
    }
    pub(super) fn activate(&mut self, owner: BackgroundOutputOwner) -> Result<()> {
        self.principals.activate(owner)
    }
    fn retire(&mut self, owner: &BackgroundOutputOwner) {
        self.principals.retire(owner);
    }
}
impl PrincipalState {
    fn activate(&mut self, owner: BackgroundOutputOwner) -> Result<()> {
        if self.closed {
            return Err(NativeTerminalTransitionError::Closed);
        }
        if self
            .principals
            .iter()
            .any(|p| p.owner == owner && !p.revoked.is_cancelled())
        {
            return Ok(());
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(NativeTerminalTransitionError::Capacity)?;
        let writer = crate::terminal_input::TerminalWriterId::new(
            std::num::NonZeroU64::new(generation).expect("positive generation"),
        );
        if let Some(principal) = self.principals.iter_mut().find(|p| p.owner == owner) {
            principal.revoked = CancellationToken::new();
            principal.writer = writer;
        } else {
            if self.principals.len() == MAX_PROFILE_OWNERS {
                return Err(NativeTerminalTransitionError::Capacity);
            }
            self.principals.push(Principal {
                owner,
                revoked: CancellationToken::new(),
                writer,
            });
        }
        self.generation = generation;
        Ok(())
    }
}
impl TerminalAccessRoutes {
    pub(crate) fn resolve(
        &self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> std::result::Result<BackgroundOutputOwner, TerminalCatalogViewError> {
        self.check(owner)?;
        if let Some(route) = self
            .routes
            .iter()
            .find(|r| r.id == *id && r.current.as_ref() == Some(owner))
        {
            return Ok(route.storage.clone());
        }
        if self
            .routes
            .iter()
            .any(|r| r.storage == *owner && r.id == *id)
        {
            return Err(TerminalCatalogViewError::Invalid);
        }
        Ok(owner.clone())
    }
    pub(crate) fn check(
        &self,
        owner: &BackgroundOutputOwner,
    ) -> std::result::Result<(), TerminalCatalogViewError> {
        if self
            .principals
            .lock()
            .principals
            .iter()
            .any(|p| p.owner == *owner && p.revoked.is_cancelled())
        {
            Err(TerminalCatalogViewError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub(crate) fn visible(
        &self,
        owner: &BackgroundOutputOwner,
        storage: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> bool {
        self.routes
            .iter()
            .find(|r| r.storage == *storage && r.id == *id)
            .map_or(owner == storage, |r| r.current.as_ref() == Some(owner))
    }
    pub(crate) fn origins(&self, owner: &BackgroundOutputOwner) -> Vec<BackgroundOutputOwner> {
        let mut result = vec![owner.clone()];
        for route in &self.routes {
            if route.current.as_ref() == Some(owner) && !result.contains(&route.storage) {
                result.push(route.storage.clone());
            }
        }
        result
    }
    fn selected<B: TerminalSessionBackend>(
        &self,
        registry: &TerminalRegistry<B>,
        owner: &BackgroundOutputOwner,
    ) -> Vec<AccessRoute> {
        let mut result: Vec<_> = self
            .routes
            .iter()
            .filter(|r| r.current.as_ref() == Some(owner))
            .cloned()
            .collect();
        for id in registry.owner_ids(owner) {
            if !self
                .routes
                .iter()
                .any(|r| r.storage == *owner && r.id == id)
            {
                result.push(AccessRoute {
                    storage: owner.clone(),
                    id,
                    current: Some(owner.clone()),
                });
            }
        }
        result
    }
    fn preflight(&self, selected: &[AccessRoute], owners: &[&BackgroundOutputOwner]) -> Result<()> {
        let additional = selected
            .iter()
            .filter(|r| {
                !self
                    .routes
                    .iter()
                    .any(|saved| saved.storage == r.storage && saved.id == r.id)
            })
            .count();
        let state = self.principals.lock();
        let principals = owners
            .iter()
            .filter(|owner| !state.principals.iter().any(|p| &p.owner == **owner))
            .count();
        if self.routes.len() + additional > MAX_PROFILE_SESSIONS
            || state.principals.len() + principals > MAX_PROFILE_OWNERS
        {
            return Err(NativeTerminalTransitionError::Capacity);
        }
        if state.generation.checked_add(2).is_none() {
            return Err(NativeTerminalTransitionError::Capacity);
        }
        Ok(())
    }
    fn set(&mut self, mut route: AccessRoute, current: Option<BackgroundOutputOwner>) {
        route.current = current;
        if let Some(saved) = self
            .routes
            .iter_mut()
            .find(|r| r.storage == route.storage && r.id == route.id)
        {
            *saved = route;
        } else {
            self.routes.push(route);
        }
    }
    fn collisions(
        &self,
        selected: &[AccessRoute],
        destination: &BackgroundOutputOwner,
        destination_ids: &[TerminalSessionId],
    ) -> Result<()> {
        for (index, route) in selected.iter().enumerate() {
            if selected[..index].iter().any(|other| other.id == route.id)
                || (&route.storage != destination && destination_ids.contains(&route.id))
                || self
                    .routes
                    .iter()
                    .any(|r| r.current.as_ref() == Some(destination) && r.id == route.id)
            {
                return Err(NativeTerminalTransitionError::Conflict);
            }
        }
        Ok(())
    }
}

pub(super) fn handoff<B: TerminalSessionBackend>(
    mut context: TerminalOwnerContext<'_, B, HostState>,
    source: &BackgroundOutputOwner,
    destination: &BackgroundOutputOwner,
) -> Result<NativeTerminalHandoffReceipt> {
    if source == destination {
        return Err(NativeTerminalTransitionError::Conflict);
    }
    let selected = context.state.access.selected(context.registry, source);
    context
        .state
        .access
        .preflight(&selected, &[source, destination])?;
    let catalog = context
        .state
        .catalogs
        .catalog(context.store, destination, context.cancellation)
        .map_err(|_| NativeTerminalTransitionError::Preparation)?;
    let snapshot = catalog
        .snapshot()
        .map_err(|_| NativeTerminalTransitionError::Preparation)?;
    let destination_ids: Vec<_> = snapshot
        .ids()
        .cloned()
        .chain(context.registry.owner_ids(destination))
        .collect();
    context
        .state
        .access
        .collisions(&selected, destination, &destination_ids)?;
    snapshot
        .validate()
        .map_err(|_| NativeTerminalTransitionError::Preparation)?;
    for route in &selected {
        if context.cancellation.is_cancelled() {
            return Err(NativeTerminalTransitionError::Cancelled);
        }
        prepare_attention(&mut context, source, route)?;
    }
    if context.cancellation.is_cancelled() {
        return Err(NativeTerminalTransitionError::Cancelled);
    }
    // Ingress may have added another bounded principal during preparation.
    // Recheck allocation before changing any route, without holding a lock
    // during profile I/O or invoking cancellation wakers under that lock.
    if !context.state.access.principals.contains(source) {
        context.state.access.activate(source.clone())?;
    }
    context.state.access.activate(destination.clone())?;
    context.state.access.retire(source);
    let transferred = selected.len();
    for route in selected {
        context
            .state
            .probes
            .retire_session(&route.storage, &route.id);
        context.state.access.set(route, Some(destination.clone()));
    }
    Ok(NativeTerminalHandoffReceipt { transferred })
}

impl crate::terminal_host_dispatch::TerminalAccessView for TerminalAccessRoutes {
    fn check(
        &self,
        owner: &BackgroundOutputOwner,
    ) -> std::result::Result<(), TerminalCatalogViewError> {
        Self::check(self, owner)
    }
    fn resolve(
        &self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> std::result::Result<BackgroundOutputOwner, TerminalCatalogViewError> {
        Self::resolve(self, owner, id)
    }
    fn visible(
        &self,
        owner: &BackgroundOutputOwner,
        storage: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> bool {
        Self::visible(self, owner, storage, id)
    }
    fn origins(&self, owner: &BackgroundOutputOwner) -> Vec<BackgroundOutputOwner> {
        Self::origins(self, owner)
    }
    fn snapshot(&self) -> Box<dyn crate::terminal_host_dispatch::TerminalAccessView> {
        Box::new(self.clone())
    }
}

fn prepare_attention<B: TerminalSessionBackend>(
    context: &mut TerminalOwnerContext<'_, B, HostState>,
    source: &BackgroundOutputOwner,
    route: &AccessRoute,
) -> Result<()> {
    if context
        .registry
        .authorize_resident(&route.storage, &route.id)
        .is_err()
    {
        return Ok(());
    }
    context.waits.cancel_session(&route.storage, &route.id);
    let writer = context.state.access.principals.writer(source);
    context
        .registry
        .mutate_with_profile(
            context.store,
            context.budget,
            &route.storage,
            &route.id,
            |session, persistence| {
                session
                    .finish_attention_with(
                        persistence,
                        &route.storage,
                        TerminalActorRole::Agent,
                        writer,
                        context.now_ms,
                        true,
                    )
                    .map(|_| ())
            },
        )
        .map_err(|_| NativeTerminalTransitionError::Preparation)
}

fn reset<B: TerminalSessionBackend>(
    context: &mut TerminalOwnerContext<'_, B, HostState>,
    owner: &BackgroundOutputOwner,
) -> Result<NativeTerminalResetReceipt> {
    let selected = context.state.access.selected(context.registry, owner);
    context.state.access.preflight(&selected, &[owner])?;
    if !context.state.access.principals.contains(owner) {
        context.state.access.activate(owner.clone())?;
    }
    context.state.access.retire(owner);
    let mut entries = Vec::with_capacity(selected.len());
    // Once reset starts, complete the bounded pass even if its caller cancels.
    for route in selected {
        context
            .state
            .probes
            .retire_session(&route.storage, &route.id);
        context.waits.cancel_session(&route.storage, &route.id);
        let outcome = if context
            .registry
            .authorize_resident(&route.storage, &route.id)
            .is_ok()
        {
            let live = context
                .registry
                .live_mut(&route.storage, &route.id)
                .is_ok_and(|session| session.owns_backend());
            let closed = context.registry.mutate_with_profile(
                context.store,
                context.budget,
                &route.storage,
                &route.id,
                |session, persistence| {
                    session.close_with(
                        persistence,
                        &route.storage,
                        TerminalClosePolicy::Force,
                        context.now_ms,
                    )
                },
            );
            if closed.is_ok() && context.registry.release(&route.storage, &route.id).is_ok() {
                if live {
                    NativeTerminalResetOutcome::StoppedAndForgotten
                } else {
                    NativeTerminalResetOutcome::AlreadyTerminatedForgotten
                }
            } else {
                NativeTerminalResetOutcome::RetainedIndeterminate
            }
        } else {
            // A journal is not native process authority. Only validated final
            // history with an observed exit/signal may be forgotten. Merely
            // closing a recovered Lost history does not prove native cleanup.
            match context.state.catalogs.with_recovered(
                context.store,
                *context.budget,
                &route.storage,
                &route.id,
                context.now_ms,
                &CancellationToken::new(),
                |session, _| {
                    let facts = session.facts(&route.storage)?;
                    Ok(confirmed_history_outcome(
                        facts.context.lifecycle,
                        facts.outcome,
                    ))
                },
            ) {
                Ok(true) => NativeTerminalResetOutcome::AlreadyTerminatedForgotten,
                _ => NativeTerminalResetOutcome::RetainedIndeterminate,
            }
        };
        entries.push(NativeTerminalResetEntry {
            id: route.id.clone(),
            outcome,
        });
        if outcome != NativeTerminalResetOutcome::RetainedIndeterminate {
            context.state.access.set(route, None);
        }
    }
    Ok(NativeTerminalResetReceipt { entries })
}

fn confirmed_history_outcome(
    lifecycle: machine_god_core::TerminalLifecycle,
    outcome: Option<crate::terminal_monitor::TerminalProcessOutcome>,
) -> bool {
    matches!(
        lifecycle,
        machine_god_core::TerminalLifecycle::Closed | machine_god_core::TerminalLifecycle::Exited
    ) && outcome.is_some()
}
