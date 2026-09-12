use super::super::{McpAuthInvalidated, store::Snapshot};
use super::{
    Arc, BoxFuture, CancellationToken, Credentials, Instant, McpAuthError, McpAuthLease,
    McpAuthLocalRemoval, NativeMcpAuthProfile, Operation, Result, state,
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
        job: impl FnOnce(&Guard, Option<&Arc<NativeMcpAuthProfile>>) -> Result<T> + Send + 'static,
    ) -> BoxFuture<'static, Result<T>> {
        let guard = self.guard.clone();
        let profile = self.profile.clone();
        guard.observation.workers.fetch_add(1, Ordering::AcqRel);
        let worker = Worker(guard.clone());
        let workers = guard.inner.workers.clone();
        let task = workers.run(move || {
            let result = job(&guard, profile.as_ref());
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
        let future = self.worker(move |guard, profile| {
            #[cfg(test)]
            guard.inner.hooks.before_load();
            guard.check(&cancellation, deadline, false)?;
            if let Some(profile) = profile {
                profile.validate()?;
            }
            let snapshot = guard.inner.store.load()?;
            if let Some(profile) = profile {
                profile.validate()?;
            }
            Ok(snapshot)
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
        self.worker(move |guard, profile| {
            #[cfg(test)]
            guard.inner.hooks.before_commit();
            let profile_lock = profile.map(|profile| profile.lock()).transpose()?;
            guard.check(&cancellation, deadline, false)?;
            // Map before publication: an unrepresentable deadline cannot erase
            // an already-durable credential outcome. Commit latency only narrows it.
            let issued =
                super::lease::Issuance::new(credentials, guard.inner.authority.clock.clone())?;
            #[cfg(test)]
            guard.inner.hooks.admitted_commit();
            let mut durability =
                guard
                    .inner
                    .store
                    .publish(&snapshot, &guard.identity, Some(&issued.credentials))?;
            if profile.is_some_and(|profile| profile.unchanged().is_err()) {
                // A noncooperative source replacement after credential rename
                // cannot be reported as a clean prepublication failure.
                durability = PublicationDurability::Ambiguous;
            }
            drop(profile_lock);
            let generation = CancellationToken::new();
            let source_closed = profile.is_some_and(|profile| profile.check().is_err());
            let old = {
                let state = lock(&guard.inner.state);
                let retired = state.closed || guard.slot.cutoff.load(Ordering::Acquire);
                if retired {
                    None
                } else {
                    Some(std::mem::replace(
                        &mut *lock(&guard.slot.generation),
                        state::Generation {
                            token: generation.clone(),
                            issued: Some(issued.clone()),
                        },
                    ))
                }
            };
            if let Some(old) = old {
                cancel(&old.token);
                contain(|| {
                    guard.inner.invalidation.invalidate(McpAuthInvalidated {
                        identity: guard.identity.clone(),
                        generation: old.token,
                    });
                });
            } else {
                // Publication won its reservation, but retirement won authority.
                cancel(&generation);
            }
            if source_closed {
                cancel(&generation);
            }
            #[cfg(test)]
            guard.inner.hooks.after_commit();
            if durability == PublicationDurability::Ambiguous {
                cancel(&generation);
                return Err(McpAuthError::AmbiguousPublication);
            }
            Ok(issued.lease(generation, profile.cloned()))
        })
        .await
    }

    pub async fn remove(
        &self,
        caller: &CancellationToken,
        deadline: Instant,
    ) -> Result<(McpAuthLocalRemoval, Option<Credentials>)> {
        let cancellation = caller.clone();
        self.worker(move |guard, profile| {
            let profile_lock = profile.map(|profile| profile.lock()).transpose()?;
            guard.check(&cancellation, deadline, true)?;
            #[cfg(test)]
            guard.inner.hooks.before_remove();
            let Ok(snapshot) = guard.inner.store.load() else {
                return Ok((McpAuthLocalRemoval::Failed, None));
            };
            let credentials = snapshot.get(&guard.identity).cloned();
            let mut local = match guard.inner.store.publish(&snapshot, &guard.identity, None) {
                Ok(PublicationDurability::Confirmed) if credentials.is_some() => {
                    McpAuthLocalRemoval::Removed
                }
                Ok(PublicationDurability::Confirmed) => McpAuthLocalRemoval::Unchanged,
                Ok(PublicationDurability::Ambiguous) => McpAuthLocalRemoval::Ambiguous,
                Err(_) => McpAuthLocalRemoval::Failed,
            };
            if matches!(
                local,
                McpAuthLocalRemoval::Removed | McpAuthLocalRemoval::Unchanged
            ) && profile.is_some_and(|profile| profile.unchanged().is_err())
            {
                local = McpAuthLocalRemoval::Ambiguous;
            }
            drop(profile_lock);
            #[cfg(test)]
            guard.inner.hooks.after_remove();
            Ok((local, credentials))
        })
        .await
    }
}
