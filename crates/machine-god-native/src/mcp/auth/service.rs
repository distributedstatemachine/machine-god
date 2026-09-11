use super::{
    Authority, McpAuthBrowser, McpAuthChallenge, McpAuthClock, McpAuthConfig, McpAuthEntropy,
    McpAuthError, McpAuthIdentity, McpAuthInvalidated, McpAuthInvalidation, McpAuthLocalRemoval,
    McpAuthLogoutReceipt, McpAuthNetwork, McpAuthRemoteRevocation, NativeMcpCredentialStore,
    Result, codec::Credentials, redacted,
};
use crate::bounded_profile_file::PublicationDurability;
use futures_util::future::{Either, select};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::Instant,
};

/// One host-owned credential coordinator. Constructors perform no I/O.
pub struct NativeMcpAuthService {
    store: Arc<NativeMcpCredentialStore>,
    authority: Authority,
    invalidation: Arc<dyn McpAuthInvalidation>,
    slots: Mutex<BTreeMap<McpAuthIdentity, Arc<Slot>>>,
}
struct Slot {
    retired: CancellationToken,
    state: Mutex<SlotState>,
}
struct SlotState {
    busy: bool,
    generation: CancellationToken,
}
struct Operation<'a> {
    service: &'a NativeMcpAuthService,
    identity: McpAuthIdentity,
    slot: Arc<Slot>,
    cancellation: CancellationToken,
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.slot
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .busy = false;
        self.cancellation.cancel();
    }
}
/// Exact retained credential generation. A cancelled lease may not supply a
/// header; callers also retain its cancellation observer through runtime writes.
pub struct McpAuthLease {
    credentials: Arc<Credentials>,
    generation: CancellationToken,
}
redacted!(NativeMcpAuthService, McpAuthLease);
impl McpAuthLease {
    #[must_use]
    pub fn identity(&self) -> &McpAuthIdentity {
        &self.credentials.identity
    }
    /// # Errors
    /// Rejects a generation invalidated by refresh, logout or owner drop.
    pub fn access_token(&self) -> Result<&[u8]> {
        if self.generation.is_cancelled() {
            Err(McpAuthError::Conflict)
        } else {
            Ok(self.credentials.access.bytes())
        }
    }
    #[must_use]
    pub fn generation(&self) -> CancellationToken {
        self.generation.clone()
    }
    #[must_use]
    pub fn cancelled_owned(&self) -> BoxFuture<'static, ()> {
        let generation = self.generation.clone();
        Box::pin(async move { generation.cancelled().await })
    }
}
impl NativeMcpAuthService {
    #[must_use]
    pub fn new(
        store: Arc<NativeMcpCredentialStore>,
        network: Arc<dyn McpAuthNetwork>,
        clock: Arc<dyn McpAuthClock>,
        entropy: Arc<dyn McpAuthEntropy>,
        invalidation: Arc<dyn McpAuthInvalidation>,
    ) -> Self {
        Self {
            store,
            authority: Authority {
                network,
                clock,
                entropy,
            },
            invalidation,
            slots: Mutex::new(BTreeMap::new()),
        }
    }
    /// Read-only persisted status; it does not refresh or activate credentials.
    /// # Errors
    /// Invalid selected stores remain errors, never anonymous fallback.
    pub fn status(&self, identity: &McpAuthIdentity) -> Result<bool> {
        Ok(self.store.load()?.get(identity).is_some())
    }
    /// Actual discovery, registration, approved browser callback and token flow.
    /// # Errors
    /// Returns redacted authority, protocol, cancellation and publication errors.
    pub async fn authenticate(
        &self,
        config: &McpAuthConfig,
        challenge: &McpAuthChallenge,
        browser: &dyn McpAuthBrowser,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<McpAuthLease> {
        self.authority.check(cancellation, deadline)?;
        let operation = self.begin(&config.identity)?;
        let snapshot = self.store.load()?;
        let previous = snapshot.get(&config.identity).map(|c| c.scope.as_ref());
        let credentials = operation
            .run(
                self.authority.authorize(
                    config,
                    challenge,
                    previous,
                    browser,
                    &operation.cancellation,
                    deadline,
                ),
                cancellation,
                deadline,
            )
            .await?;
        operation.commit(&snapshot, credentials, cancellation, deadline)
    }
    /// Loads exact selected credentials and refreshes only when within the pinned
    /// 60-second expiry skew. A refresh POST is never replayed automatically.
    /// # Errors
    /// Concurrent work returns busy; stale generation/store observations conflict.
    /// Missing means no matching record; expired credentials without a refresh
    /// token are unavailable, never an anonymous-fallback signal.
    pub async fn access_token(
        &self,
        identity: &McpAuthIdentity,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<McpAuthLease> {
        self.authority.check(cancellation, deadline)?;
        let operation = self.begin(identity)?;
        let snapshot = self.store.load()?;
        let credentials = snapshot.get(identity).ok_or(McpAuthError::Missing)?;
        if credentials.expires_ms.saturating_sub(60_000) > self.authority.clock.unix_millis() {
            return operation.lease(credentials.clone(), cancellation, deadline);
        }
        let replacement = operation
            .run(
                self.authority
                    .refresh(credentials, &operation.cancellation, deadline),
                cancellation,
                deadline,
            )
            .await?;
        operation.commit(&snapshot, replacement, cancellation, deadline)
    }
    /// Invalidates cached/live authority before attempting local deletion and
    /// optional remote revocation. These independent effects get separate receipts.
    /// # Errors
    /// Rejects already cancelled/expired requests before any effects.
    pub async fn logout(
        &self,
        identity: &McpAuthIdentity,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<McpAuthLogoutReceipt> {
        self.authority.check(cancellation, deadline)?;
        let in_flight = self.retire_active(identity);
        let Ok(snapshot) = self.store.load() else {
            return Ok(McpAuthLogoutReceipt {
                local: McpAuthLocalRemoval::Failed,
                remote: if in_flight {
                    McpAuthRemoteRevocation::Ambiguous
                } else {
                    McpAuthRemoteRevocation::NotAttempted
                },
            });
        };
        let credentials = snapshot.get(identity).cloned();
        let local = match self.store.publish(&snapshot, identity, None) {
            Ok(PublicationDurability::Confirmed) if credentials.is_some() => {
                McpAuthLocalRemoval::Removed
            }
            Ok(PublicationDurability::Confirmed) => McpAuthLocalRemoval::Unchanged,
            Ok(PublicationDurability::Ambiguous) => McpAuthLocalRemoval::Ambiguous,
            Err(_) => McpAuthLocalRemoval::Failed,
        };
        let remote = match credentials {
            Some(credentials) => {
                self.authority
                    .revoke(&credentials, cancellation, deadline)
                    .await
            }
            None => McpAuthRemoteRevocation::NotAttempted,
        };
        let remote = if in_flight {
            McpAuthRemoteRevocation::Ambiguous
        } else {
            remote
        };
        Ok(McpAuthLogoutReceipt { local, remote })
    }
    /// Retires a superseded selected identity without changing persisted data or
    /// contacting OAuth. Runtime reload/removal releases its coordinator slot.
    pub fn retire(&self, identity: &McpAuthIdentity) {
        self.retire_active(identity);
    }
    fn retire_active(&self, identity: &McpAuthIdentity) -> bool {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(identity);
        if let Some(slot) = slot {
            let (in_flight, generation) = {
                let state = slot
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (state.busy, state.generation.clone())
            };
            slot.retired.cancel();
            generation.cancel();
            self.invalidation.invalidate(McpAuthInvalidated {
                identity: identity.clone(),
                generation,
            });
            in_flight
        } else {
            false
        }
    }
    fn begin(&self, identity: &McpAuthIdentity) -> Result<Operation<'_>> {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !slots.contains_key(identity) && slots.len() >= 64 {
            return Err(McpAuthError::Limit);
        }
        let slot = slots
            .entry(identity.clone())
            .or_insert_with(|| {
                Arc::new(Slot {
                    retired: CancellationToken::new(),
                    state: Mutex::new(SlotState {
                        busy: false,
                        generation: CancellationToken::new(),
                    }),
                })
            })
            .clone();
        let mut state = slot
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.busy {
            return Err(McpAuthError::Busy);
        }
        state.busy = true;
        drop(state);
        Ok(Operation {
            service: self,
            identity: identity.clone(),
            slot,
            cancellation: CancellationToken::new(),
        })
    }
}
impl Operation<'_> {
    async fn run<T>(
        &self,
        future: impl std::future::Future<Output = Result<T>>,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<T> {
        self.service
            .authority
            .bounded(
                async {
                    if self.slot.retired.is_cancelled() {
                        return Err(McpAuthError::Conflict);
                    }
                    match select(Box::pin(self.slot.retired.cancelled()), Box::pin(future)).await {
                        Either::Right((result, _)) => result,
                        Either::Left(_) => Err(McpAuthError::Conflict),
                    }
                },
                caller,
                deadline,
            )
            .await
    }
    fn lease(
        &self,
        credentials: Credentials,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<McpAuthLease> {
        self.service.authority.check(caller, deadline)?;
        if self.slot.retired.is_cancelled() {
            return Err(McpAuthError::Conflict);
        }
        let generation = self
            .slot
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generation
            .clone();
        Ok(McpAuthLease {
            credentials: Arc::new(credentials),
            generation,
        })
    }
    fn commit(
        &self,
        snapshot: &super::store::Snapshot,
        credentials: Credentials,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<McpAuthLease> {
        self.service.authority.check(caller, deadline)?;
        let slots = self
            .service
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if caller.is_cancelled() {
            return Err(McpAuthError::Cancelled);
        }
        if !slots
            .get(&self.identity)
            .is_some_and(|slot| Arc::ptr_eq(slot, &self.slot))
        {
            return Err(McpAuthError::Conflict);
        }
        let durability =
            self.service
                .store
                .publish(snapshot, &self.identity, Some(&credentials))?;
        let mut state = self
            .slot
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let old = std::mem::replace(&mut state.generation, CancellationToken::new());
        let generation = state.generation.clone();
        drop(state);
        drop(slots);
        old.cancel();
        self.service.invalidation.invalidate(McpAuthInvalidated {
            identity: self.identity.clone(),
            generation: old,
        });
        if durability == PublicationDurability::Ambiguous {
            generation.cancel();
            return Err(McpAuthError::AmbiguousPublication);
        }
        Ok(McpAuthLease {
            credentials: Arc::new(credentials),
            generation,
        })
    }
}
impl Drop for NativeMcpAuthService {
    fn drop(&mut self) {
        let slots = std::mem::take(
            self.slots
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for (_, slot) in slots {
            slot.retired.cancel();
            let generation = slot
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .generation
                .clone();
            generation.cancel();
        }
    }
}
