//! Inert binding to the shared, bounded native worker collector.

use crate::background_supervisor::worker_ownership_registry;
use std::fmt;

/// Fixed, redacted failure to admit or start an owned native worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeOwnedWorkerSpawnError;

impl fmt::Display for NativeOwnedWorkerSpawnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("owned native worker unavailable")
    }
}
impl std::error::Error for NativeOwnedWorkerSpawnError {}

/// Starts workers whose handles belong to the production collector before work
/// is released. This zero-state binding does not create a collector or thread
/// until `spawn` is called, and dropping it does not detach any running worker.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeOwnedWorkerSpawner;

impl NativeOwnedWorkerSpawner {
    /// Constructs the binding without threads, native effects or reservations.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Admits one worker against the shared process-wide capacity. The job must
    /// own its shutdown/cleanup protocol; an abandoned response cannot transfer
    /// that responsibility to its caller or the collector.
    ///
    /// # Errors
    /// Returns a fixed failure if collection, capacity or thread admission
    /// fails. In that case the job has not executed; any failed-registration
    /// thread is collected before returning.
    pub fn spawn(
        &self,
        operation: impl FnOnce() + Send + 'static,
    ) -> Result<(), NativeOwnedWorkerSpawnError> {
        let registry = worker_ownership_registry().map_err(|()| NativeOwnedWorkerSpawnError)?;
        let reservation = registry
            .reserve_partitioned(&[1])
            .map_err(|()| NativeOwnedWorkerSpawnError)?
            .pop()
            .ok_or(NativeOwnedWorkerSpawnError)?;
        reservation
            .spawn_one(registry, "machine-god-owned-worker", operation)
            .map_err(|()| NativeOwnedWorkerSpawnError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    #[test]
    fn worker_uses_collector_and_outlives_submitter_scope() {
        const SPAWNER: NativeOwnedWorkerSpawner = NativeOwnedWorkerSpawner::new();
        let caller = std::thread::current().id();
        let (started_tx, started_rx) = sync_channel(1);
        let (release_tx, release_rx) = sync_channel(1);
        let (finished_tx, finished_rx) = sync_channel(1);
        {
            let spawner = SPAWNER;
            spawner
                .spawn(move || {
                    started_tx.send(std::thread::current().id()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    finished_tx.send(()).unwrap();
                })
                .unwrap();
        }
        assert_ne!(
            started_rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            caller
        );
        release_tx.send(()).unwrap();
        finished_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    }
}
