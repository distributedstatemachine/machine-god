use super::super::{McpAuthInvalidated, store::Snapshot};
use super::{
    Arc, BoxFuture, CancellationToken, Credentials, Instant, McpAuthError, McpAuthLease,
    McpAuthLocalRemoval, Operation, Result, state,
};
use crate::bounded_profile_file::PublicationDurability;
use state::{Guard, cancel, contain, lock};
use std::sync::atomic::Ordering;

struct Worker(Arc<Guard>);
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.observation.workers.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Operation {
    /// Result custody retains the operation reservation through completion even
    /// if its caller drops the future while the actual scoped worker is running.
    fn worker<T: Send + 'static>(
        &self,
        job: impl FnOnce(&Guard) -> Result<T> + Send + 'static,
    ) -> BoxFuture<'static, Result<T>> {
        let guard = self.guard.clone();
        guard.observation.workers.fetch_add(1, Ordering::AcqRel);
        let worker = Worker(guard.clone());
        let workers = guard.inner.workers.clone();
        let task = workers.run(move || {
            let result = job(&guard);
            drop(worker);
            (guard, result)
        });
        Box::pin(async move {
            let (_guard, result) = task.await.map_err(|_| McpAuthError::Unavailable)?;
            result
        })
    }

    pub async fn load(&self, caller: &CancellationToken, deadline: Instant) -> Result<Snapshot> {
        let cancellation = caller.clone();
        let future = self.worker(move |guard| {
            #[cfg(test)]
            guard.inner.hooks.before_load();
            guard.check(&cancellation, deadline, false)?;
            guard.inner.store.load()
        });
        self.run(future, caller, deadline).await
    }

    pub async fn commit(
        &self,
        snapshot: Snapshot,
        credentials: Credentials,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<McpAuthLease> {
        let cancellation = caller.clone();
        // Do not select an admitted publication away. The worker returns its
        // actual durability result even if cancellation arrives during fsync.
        self.worker(move |guard| {
            #[cfg(test)]
            guard.inner.hooks.before_commit();
            guard.check(&cancellation, deadline, false)?;
            #[cfg(test)]
            guard.inner.hooks.admitted_commit();
            let durability =
                guard
                    .inner
                    .store
                    .publish(&snapshot, &guard.identity, Some(&credentials))?;
            let generation = CancellationToken::new();
            let old = {
                let state = lock(&guard.inner.state);
                let retired = state.closed || guard.slot.cutoff.load(Ordering::Acquire);
                if retired {
                    None
                } else {
                    Some(std::mem::replace(
                        &mut *lock(&guard.slot.generation),
                        generation.clone(),
                    ))
                }
            };
            if let Some(old) = old {
                cancel(&old);
                contain(|| {
                    guard.inner.invalidation.invalidate(McpAuthInvalidated {
                        identity: guard.identity.clone(),
                        generation: old,
                    });
                });
            } else {
                // Publication won its reservation, but retirement won authority.
                cancel(&generation);
            }
            #[cfg(test)]
            guard.inner.hooks.after_commit();
            if durability == PublicationDurability::Ambiguous {
                cancel(&generation);
                return Err(McpAuthError::AmbiguousPublication);
            }
            Ok(McpAuthLease {
                credentials: Arc::new(credentials),
                generation,
            })
        })
        .await
    }

    pub async fn remove(
        &self,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<(McpAuthLocalRemoval, Option<Credentials>)> {
        let cancellation = caller.clone();
        self.worker(move |guard| {
            guard.check(&cancellation, deadline, true)?;
            #[cfg(test)]
            guard.inner.hooks.before_remove();
            let Ok(snapshot) = guard.inner.store.load() else {
                return Ok((McpAuthLocalRemoval::Failed, None));
            };
            let credentials = snapshot.get(&guard.identity).cloned();
            let local = match guard.inner.store.publish(&snapshot, &guard.identity, None) {
                Ok(PublicationDurability::Confirmed) if credentials.is_some() => {
                    McpAuthLocalRemoval::Removed
                }
                Ok(PublicationDurability::Confirmed) => McpAuthLocalRemoval::Unchanged,
                Ok(PublicationDurability::Ambiguous) => McpAuthLocalRemoval::Ambiguous,
                Err(_) => McpAuthLocalRemoval::Failed,
            };
            #[cfg(test)]
            guard.inner.hooks.after_remove();
            Ok((local, credentials))
        })
        .await
    }
}
