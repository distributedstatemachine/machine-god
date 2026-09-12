use super::{
    Authority, McpAuthBrowser, McpAuthChallenge, McpAuthClock, McpAuthConfig, McpAuthEntropy,
    McpAuthError, McpAuthIdentity, McpAuthInvalidation, McpAuthLocalRemoval, McpAuthLogoutReceipt,
    McpAuthNetwork, McpAuthRemoteRevocation, NativeMcpCredentialStore, Result, codec::Credentials,
    redacted,
};
use crate::NativeOwnedWorkerScope;
use machine_god_core::{BoxFuture, CancellationToken};
use std::{fmt, sync::Arc, time::Instant};

mod lifetime;
mod persistence;
mod state;
#[cfg(test)]
mod tests;
use state::{Inner, Operation};

/// One host-owned credential coordinator. Construction performs no I/O and
/// never creates or closes the injected host worker scope.
pub struct NativeMcpAuthService {
    inner: Arc<Inner>,
}

/// Auth-operation completion only. The actual host separately joins its shared
/// worker scope, including thread-local destruction and collector completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeMcpAuthCleanup {
    pub complete: bool,
    pub pending_operations: usize,
    pub pending_workers: usize,
}

/// Exact retained credential generation. A successful publication may return a
/// cancelled lease when retirement won before its completion was observed.
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
    /// Rejects a generation invalidated by refresh, logout or owner cutoff.
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
        workers: NativeOwnedWorkerScope,
    ) -> Self {
        Self {
            inner: Arc::new(Inner::new(
                store,
                Authority {
                    network,
                    clock,
                    entropy,
                },
                invalidation,
                workers,
            )),
        }
    }

    /// Synchronous read-only status for an explicitly selected caller worker.
    /// Never call this on an async polling thread; use [`Self::status_owned`].
    /// # Errors
    /// Invalid selected stores remain errors, never anonymous fallback.
    pub fn status(&self, identity: &McpAuthIdentity) -> Result<bool> {
        Ok(self.inner.store.load()?.get(identity).is_some())
    }

    /// Caller-polled, worker-owned status. No worker starts before poll.
    /// # Errors
    /// Reports closed/busy admission, cancellation, bounds or store errors.
    pub async fn status_owned(
        &self,
        identity: &McpAuthIdentity,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<bool> {
        let operation = self.inner.begin(identity, cancellation, deadline)?;
        let snapshot = operation.load(cancellation, deadline).await?;
        Ok(snapshot.get(identity).is_some())
    }

    /// Actual discovery, approved callback and token flow. Only persistence runs
    /// on workers; network and browser futures stay on the caller runtime.
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
        let operation = self
            .inner
            .begin(config.identity(), cancellation, deadline)?;
        let snapshot = operation.load(cancellation, deadline).await?;
        let previous = snapshot.get(config.identity()).map(|c| c.scope.as_ref());
        let credentials = operation
            .run(
                self.inner.authority.authorize(
                    config,
                    challenge,
                    previous,
                    browser,
                    &operation.guard.cancellation,
                    deadline,
                ),
                cancellation,
                deadline,
            )
            .await?;
        operation
            .commit(snapshot, credentials, cancellation, deadline)
            .await
    }

    /// Refreshes within the pinned 60-second expiry skew, without POST replay.
    /// # Errors
    /// Missing means no matching record. Expired credentials without refresh
    /// authority, busy and malformed stores are not anonymous fallback.
    pub async fn access_token(
        &self,
        identity: &McpAuthIdentity,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<McpAuthLease> {
        let operation = self.inner.begin(identity, cancellation, deadline)?;
        let snapshot = operation.load(cancellation, deadline).await?;
        let credentials = snapshot.get(identity).ok_or(McpAuthError::Missing)?;
        if credentials.expires_ms.saturating_sub(60_000) > self.inner.authority.clock.unix_millis()
        {
            return operation.lease(credentials.clone(), cancellation, deadline);
        }
        let replacement = operation
            .run(
                self.inner
                    .authority
                    .refresh(credentials, &operation.guard.cancellation, deadline),
                cancellation,
                deadline,
            )
            .await?;
        operation
            .commit(snapshot, replacement, cancellation, deadline)
            .await
    }

    /// First poll cuts off live authority. Deletion follows acknowledged prior
    /// work, so older publication cannot finish after successful logout.
    /// # Errors
    /// A failed wait after cutoff cannot claim completed local deletion.
    pub async fn logout(
        &self,
        identity: &McpAuthIdentity,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<McpAuthLogoutReceipt> {
        let (operation, previous) = self.inner.cutoff(identity, cancellation, deadline)?;
        let in_flight = !previous.is_empty();
        operation
            .wait_previous(&previous, cancellation, deadline)
            .await?;
        let (local, credentials) = operation.remove(cancellation, deadline).await?;
        let remote = match credentials {
            Some(credentials) => operation.revoke(&credentials, cancellation, deadline).await,
            None => McpAuthRemoteRevocation::NotAttempted,
        };
        Ok(McpAuthLogoutReceipt {
            local,
            remote: if in_flight {
                McpAuthRemoteRevocation::Ambiguous
            } else {
                remote
            },
        })
    }

    /// Acknowledged local-only retirement. First poll cuts off live leases;
    /// success proves prior admitted work cannot later publish. Retirement itself
    /// introduces no store mutation or network request.
    /// # Errors
    /// Cancellation/deadline may end observation without undoing the cutoff.
    pub async fn retire(
        &self,
        identity: &McpAuthIdentity,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<()> {
        let (operation, previous) = self.inner.cutoff(identity, cancellation, deadline)?;
        operation
            .wait_previous(&previous, cancellation, deadline)
            .await?;
        self.inner.release_retired(&operation.guard);
        Ok(())
    }
}

impl Drop for NativeMcpAuthService {
    fn drop(&mut self) {
        self.inner.close();
    }
}
