use super::{
    CancellationToken, Instant, NativeMcpAuthCleanup, NativeMcpAuthService, Result, state,
};
use state::lock;
use std::sync::atomic::Ordering;

impl NativeMcpAuthService {
    /// Immediate admission/live-authority cutoff only, not a completion claim.
    /// It never closes the shared host scope or waits for filesystem syscalls.
    pub fn close(&self) {
        self.inner.close();
    }

    /// Snapshot of this service's obligations, not host thread-join evidence.
    #[must_use]
    pub fn cleanup_status(&self) -> NativeMcpAuthCleanup {
        let state = lock(&self.inner.state);
        let pending_operations = state
            .operations
            .iter()
            .filter(|value| !value.done.is_cancelled())
            .count();
        let pending_workers = state
            .operations
            .iter()
            .map(|value| value.workers.load(Ordering::Acquire))
            .sum();
        NativeMcpAuthCleanup {
            complete: state.closed && pending_operations == 0 && pending_workers == 0,
            pending_operations,
            pending_workers,
        }
    }

    /// Closes auth admission and observes retained operations with an independent
    /// cleanup token/deadline. Dropped observers never detach admitted workers.
    /// The actual host must subsequently join its own worker scope.
    /// # Errors
    /// Deadline/cancellation leaves the same pending obligations observable.
    pub async fn settle(
        &self,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<NativeMcpAuthCleanup> {
        self.close();
        let pending = lock(&self.inner.state).operations.clone();
        self.inner
            .authority
            .bounded(
                async {
                    for observation in pending {
                        observation.done.cancelled().await;
                    }
                    Ok(())
                },
                &cancellation,
                deadline,
            )
            .await?;
        Ok(self.cleanup_status())
    }
}
