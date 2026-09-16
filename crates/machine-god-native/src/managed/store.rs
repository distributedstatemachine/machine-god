//! Descriptor-owned managed control journal, separate from session transcripts.

mod filesystem;
mod records;
#[cfg(test)]
mod tests;
mod transaction;

use crate::NativeOwnedWorkerScope;
use machine_god_core::BoxFuture;
pub(crate) use records::*;
use rustix::fd::OwnedFd;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JournalError {
    Busy,
    Limit,
    Invalid,
    Conflict,
    Missing,
    Persistence,
    Ambiguous,
    Exhausted,
    Worker,
    RecoveryRequired,
}

/// Physical retained bytes include old/new files, temporary/orphan files and
/// bounded bookkeeping. Per-operation memory is reserved before serialization.
#[derive(Clone, Copy, Debug)]
pub(crate) struct JournalLimits {
    pub aggregate_bytes: usize,
    pub head_bytes: usize,
    pub page_bytes: usize,
    pub queue_entries: usize,
    pub directory_entries: usize,
}
impl Default for JournalLimits {
    fn default() -> Self {
        Self {
            aggregate_bytes: 256 * 1024 * 1024,
            head_bytes: 128 * 1024,
            page_bytes: 1024 * 1024,
            queue_entries: 64,
            directory_entries: 65_536,
        }
    }
}
impl JournalLimits {
    fn validate(self) -> Result<Self, JournalError> {
        if !(1024..=1024 * 1024).contains(&self.head_bytes)
            || !(512 * 1024..=8 * 1024 * 1024).contains(&self.page_bytes)
            || !(1..=256).contains(&self.queue_entries)
            || !(1..=1_048_576).contains(&self.directory_entries)
            || self.aggregate_bytes < 4 * self.head_bytes + 8 * self.page_bytes + 1024
            || u64::try_from(self.aggregate_bytes).is_ok_and(|bytes| bytes > 4 * 1024 * 1024 * 1024)
        {
            return Err(JournalError::Limit);
        }
        Ok(self)
    }
}

/// A clone retains actual exclusive owner custody, not only a head CAS token.
/// The outer manager retains this through child/control/worker settlement.
pub(crate) struct JournalOwner {
    // Keep the exclusive file lock alive through actual child/worker cleanup.
    shared: Arc<Shared>,
}
impl Clone for JournalOwner {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}
impl JournalOwner {
    #[cfg(test)]
    pub(crate) fn belongs_to(&self, journal: &ManagedJournal) -> bool {
        Arc::ptr_eq(&self.shared, &journal.shared)
    }
}
impl fmt::Debug for JournalOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JournalOwner { .. }")
    }
}

#[derive(Clone)]
pub(crate) struct ManagedJournal {
    shared: Arc<Shared>,
    workers: NativeOwnedWorkerScope,
}
struct Shared {
    root: OwnedFd,
    owner_lock: OwnedFd,
    epoch: u64,
    limits: JournalLimits,
    busy: AtomicBool,
    state: Mutex<Accounting>,
    #[cfg(test)]
    failure: Mutex<Option<tests::FailurePoint>>,
}
struct Accounting {
    used: usize,
    reserved: usize,
    pending: Option<transaction::PendingPublication>,
    next_operation: u64,
}
impl fmt::Debug for ManagedJournal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ManagedJournal { .. }")
    }
}

impl ManagedJournal {
    pub(crate) fn workspace_directory(
        root: OwnedFd,
        workspace: std::path::PathBuf,
        origin: crate::NativeSessionOrigin,
        workers: NativeOwnedWorkerScope,
    ) -> BoxFuture<'static, Result<OwnedFd, JournalError>> {
        Box::pin(async move {
            workers
                .run(move || filesystem::workspace_directory(&root, &workspace, origin))
                .await
                .map_err(|_| JournalError::Worker)?
        })
    }

    /// Inert until first poll. The explicit private directory is never reopened
    /// through a path. Another actual owner fails Busy, including in-process.
    pub(crate) fn open(
        root: OwnedFd,
        workers: NativeOwnedWorkerScope,
        limits: JournalLimits,
    ) -> BoxFuture<'static, Result<Self, JournalError>> {
        let execution = workers.clone();
        Box::pin(async move {
            execution
                .run(move || {
                    let limits = limits.validate()?;
                    let (owner_lock, used, epoch) = filesystem::acquire(&root, limits)?;
                    Ok(Self {
                        shared: Arc::new(Shared {
                            root,
                            owner_lock,
                            epoch,
                            limits,
                            busy: AtomicBool::new(false),
                            state: Mutex::new(Accounting {
                                used,
                                reserved: 0,
                                pending: None,
                                next_operation: 1,
                            }),
                            #[cfg(test)]
                            failure: Mutex::new(None),
                        }),
                        workers,
                    })
                })
                .await
                .map_err(|_| JournalError::Worker)?
        })
    }
    pub(crate) fn owner_lease(&self) -> JournalOwner {
        JournalOwner {
            shared: self.shared.clone(),
        }
    }

    /// Conservative manager head/decode/work/presentation allowance. Runtime and
    /// mailbox payloads retain their separate aggregate owners and limits.
    pub(crate) fn resident_reservation_bytes(&self) -> usize {
        self.shared.limits.head_bytes * 4 + 256 * 1024
    }

    fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Arc<Shared>) -> Result<T, JournalError> + Send + 'static,
    ) -> BoxFuture<'static, Result<T, JournalError>> {
        let shared = self.shared.clone();
        let future = self.workers.run(move || {
            let _slot = OperationSlot::acquire(shared.clone())?;
            filesystem::validate_owner(&shared)?;
            operation(&shared)
        });
        Box::pin(async move { future.await.map_err(|_| JournalError::Worker)? })
    }
    pub(crate) fn inspect(
        &self,
        id: String,
    ) -> BoxFuture<'static, Result<JournalSnapshot, JournalError>> {
        self.run(move |shared| transaction::inspect(shared, &id))
    }
    pub(crate) fn catalog(
        &self,
        after: Option<JournalCatalogCursor>,
        limit: usize,
    ) -> BoxFuture<'static, Result<JournalCatalogPage, JournalError>> {
        self.run(move |shared| transaction::catalog(shared, after.as_ref(), limit))
    }
    pub(crate) fn create(
        &self,
        record: JournalCreate,
    ) -> BoxFuture<'static, Result<JournalPublication, JournalError>> {
        self.run(move |shared| transaction::create(shared, record))
    }
    pub(crate) fn mutate(
        &self,
        snapshot: JournalSnapshot,
        mutation: JournalMutation,
    ) -> BoxFuture<'static, Result<JournalPublication, JournalError>> {
        self.run(move |shared| transaction::mutate(shared, snapshot, mutation))
    }
    #[cfg(test)]
    pub(crate) fn recover(
        &self,
        snapshot: JournalSnapshot,
    ) -> BoxFuture<'static, Result<JournalPublication, JournalError>> {
        self.mutate(snapshot, JournalMutation::Recover)
    }
    /// Repairs directory durability and checks the exact publication, not just
    /// readback. No mutation can bypass an outstanding ambiguous receipt.
    pub(crate) fn reconcile(
        &self,
        receipt: JournalReceipt,
    ) -> BoxFuture<'static, Result<JournalPublication, JournalError>> {
        self.run(move |shared| transaction::reconcile(shared, &receipt))
    }
    #[cfg(test)]
    pub(crate) fn pending_receipt(&self) -> Option<JournalReceipt> {
        let state = self.shared.state.lock().ok()?;
        state
            .pending
            .as_ref()
            .map(|pending| pending.receipt.clone())
    }
    pub(crate) fn read_work(
        &self,
        reference: JournalPageRef,
    ) -> BoxFuture<'static, Result<JournalWork, JournalError>> {
        self.run(move |shared| transaction::read_work(shared, &reference))
    }
    pub(crate) fn history(
        &self,
        snapshot: JournalSnapshot,
        after: Option<JournalHistoryCursor>,
        limit: usize,
    ) -> BoxFuture<'static, Result<JournalHistoryPage, JournalError>> {
        self.run(move |shared| transaction::history(shared, &snapshot, after.as_ref(), limit))
    }
}

struct OperationSlot(Arc<Shared>);
impl OperationSlot {
    fn acquire(shared: Arc<Shared>) -> Result<Self, JournalError> {
        shared
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| JournalError::Busy)?;
        Ok(Self(shared))
    }
}
impl Drop for OperationSlot {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::Release);
    }
}
