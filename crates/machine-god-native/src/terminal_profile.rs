//! Profile-wide admission on a blocking owner, under one retained transaction.
//!
//! Retained output, protected state/events and metadata have separate ledgers.
//! Temporary allocation is gross, not the net size of a replacement. Mutation
//! owners must declare and obey their allocation demand before performing I/O.
//! Every admission and completion measures disk again: abandoning a reservation
//! cannot turn an orphan or an ambiguous publication into free capacity.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fmt;

use crate::terminal_journal::TerminalJournalPhysicalUsage;
use crate::terminal_profile_store::{
    MAX_PROFILE_OWNERS, MAX_PROFILE_SESSIONS, TerminalProfileStoreError, TerminalProfileTransaction,
};

const MIB: u64 = 1024 * 1024;
const MAX_METADATA: u64 = 128 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalProfileError {
    Invalid,
    ResourceLimit,
    AccountingMismatch,
    Store(TerminalProfileStoreError),
}

impl fmt::Display for TerminalProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal profile admission unavailable")
    }
}
impl std::error::Error for TerminalProfileError {}
impl From<TerminalProfileStoreError> for TerminalProfileError {
    fn from(error: TerminalProfileStoreError) -> Self {
        Self::Store(error)
    }
}
type Result<T> = std::result::Result<T, TerminalProfileError>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(
    clippy::struct_field_names,
    reason = "ledger fields retain explicit byte units"
)]
pub(crate) struct TerminalProfileLedgers {
    pub(crate) output_bytes: u64,
    pub(crate) protected_bytes: u64,
    pub(crate) metadata_bytes: u64,
}

impl TerminalProfileLedgers {
    fn physical(usage: TerminalJournalPhysicalUsage) -> Result<Self> {
        let ledgers = Self {
            output_bytes: usage.output_bytes,
            protected_bytes: add(usage.state_bytes, usage.event_bytes)?,
            metadata_bytes: usage.metadata_bytes,
        };
        if add(usage.raw_bytes, usage.checkpoint_bytes)? != ledgers.output_bytes
            || ledgers.total()? != usage.total_bytes
        {
            return Err(TerminalProfileError::AccountingMismatch);
        }
        Ok(ledgers)
    }

    fn total(self) -> Result<u64> {
        add(
            add(self.output_bytes, self.protected_bytes)?,
            self.metadata_bytes,
        )
    }

    fn plus(self, growth: Self) -> Result<Self> {
        Ok(Self {
            output_bytes: add(self.output_bytes, growth.output_bytes)?,
            protected_bytes: add(self.protected_bytes, growth.protected_bytes)?,
            metadata_bytes: add(self.metadata_bytes, growth.metadata_bytes)?,
        })
    }

    fn fits(self, ceiling: Self) -> bool {
        self.output_bytes <= ceiling.output_bytes
            && self.protected_bytes <= ceiling.protected_bytes
            && self.metadata_bytes <= ceiling.metadata_bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalProfileLimits {
    pub(crate) retained: TerminalProfileLedgers,
    pub(crate) temporary_bytes: u64,
}

impl Default for TerminalProfileLimits {
    fn default() -> Self {
        Self {
            retained: TerminalProfileLedgers {
                output_bytes: 512 * MIB,
                // Existing journal bounds: 33 MiB state, 256 x 4 KiB events,
                // and two bounded metadata files, per retained session.
                protected_bytes: 34 * MIB * MAX_PROFILE_SESSIONS as u64,
                metadata_bytes: 2 * MAX_METADATA * MAX_PROFILE_SESSIONS as u64,
            },
            // One largest checkpoint plus its metadata publication. Owners
            // split larger batches into individually admitted mutations.
            temporary_bytes: 64 * MIB + MAX_METADATA,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminalProfileDemand {
    pub(crate) additional_owners: usize,
    pub(crate) additional_sessions: usize,
    /// Positive committed growth only. Never subtract a not-yet-removed file
    /// from the current physical inventory. Replacement credit belongs solely
    /// in this mutation's net growth, after its owner validates the old blob.
    pub(crate) retained_growth: TerminalProfileLedgers,
    /// The full bytes allocated before publication and cleanup, including
    /// replacement blobs and temporary metadata, not merely retained growth.
    pub(crate) allocation_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TerminalProfileBudget {
    limits: TerminalProfileLimits,
}

impl TerminalProfileBudget {
    pub(crate) fn new(limits: TerminalProfileLimits) -> Result<Self> {
        let maximum = TerminalProfileLimits::default();
        if limits.retained.output_bytes == 0
            || limits.retained.metadata_bytes == 0
            || !limits.retained.fits(maximum.retained)
            || limits.temporary_bytes == 0
            || limits.temporary_bytes > maximum.temporary_bytes
        {
            return Err(TerminalProfileError::Invalid);
        }
        Ok(Self { limits })
    }

    /// The exclusive mutable borrow prevents multiple outstanding reservations
    /// under one lock. All cooperating namespace/journal mutations must share
    /// the profile transaction; the budget grants no journal/process authority.
    pub(crate) fn reserve<'a, 'store>(
        &self,
        transaction: &'a mut TerminalProfileTransaction<'store>,
        demand: TerminalProfileDemand,
    ) -> Result<TerminalProfileReservation<'a, 'store>> {
        if demand.allocation_bytes < demand.retained_growth.total()? {
            return Err(TerminalProfileError::Invalid);
        }
        if demand.allocation_bytes > self.limits.temporary_bytes {
            return Err(TerminalProfileError::ResourceLimit);
        }
        let inventory = transaction.inventory()?;
        let owners = inventory
            .owner_count
            .checked_add(demand.additional_owners)
            .filter(|count| *count <= MAX_PROFILE_OWNERS)
            .ok_or(TerminalProfileError::ResourceLimit)?;
        let sessions = inventory
            .sessions
            .len()
            .checked_add(demand.additional_sessions)
            .filter(|count| *count <= MAX_PROFILE_SESSIONS)
            .ok_or(TerminalProfileError::ResourceLimit)?;
        let baseline = TerminalProfileLedgers::physical(inventory.usage)?;
        let ceiling = baseline.plus(demand.retained_growth)?;
        if !ceiling.fits(self.limits.retained) {
            return Err(TerminalProfileError::ResourceLimit);
        }
        // Check the actual gross-allocation envelope, even though the separate
        // ledger checks above are stronger for ordinary bounded inputs.
        if add(baseline.total()?, demand.allocation_bytes)?
            > add(self.limits.retained.total()?, self.limits.temporary_bytes)?
        {
            return Err(TerminalProfileError::ResourceLimit);
        }
        Ok(TerminalProfileReservation {
            transaction,
            ceiling,
            owners,
            sessions,
        })
    }
}

pub(crate) struct TerminalProfileReservation<'a, 'store> {
    transaction: &'a mut TerminalProfileTransaction<'store>,
    ceiling: TerminalProfileLedgers,
    owners: usize,
    sessions: usize,
}

impl TerminalProfileReservation<'_, '_> {
    /// Preserve the mutation's receipt independently of post-publication
    /// accounting. A committed operation must not become a retryable write
    /// because final inventory failed. Error and unwind paths never credit
    /// capacity: the next admission must account for what is actually on disk.
    /// An outer error means the callback was never dispatched.
    pub(crate) fn run<T, E>(
        self,
        operation: impl FnOnce() -> std::result::Result<T, E>,
    ) -> Result<TerminalProfileCompletion<T, E>> {
        self.transaction.validate()?;
        let operation = operation();
        let accounting = self.reconcile();
        Ok(TerminalProfileCompletion {
            operation,
            accounting,
        })
    }

    fn reconcile(&self) -> Result<TerminalProfileLedgers> {
        let inventory = self.transaction.inventory()?;
        let actual = TerminalProfileLedgers::physical(inventory.usage)?;
        if !actual.fits(self.ceiling)
            || inventory.owner_count > self.owners
            || inventory.sessions.len() > self.sessions
        {
            return Err(TerminalProfileError::AccountingMismatch);
        }
        Ok(actual)
    }
}

pub(crate) struct TerminalProfileCompletion<T, E> {
    pub(crate) operation: std::result::Result<T, E>,
    pub(crate) accounting: Result<TerminalProfileLedgers>,
}

impl<T, E> fmt::Debug for TerminalProfileCompletion<T, E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalProfileCompletion")
            .finish_non_exhaustive()
    }
}

fn add(left: u64, right: u64) -> Result<u64> {
    left.checked_add(right)
        .ok_or(TerminalProfileError::ResourceLimit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_catalog::TerminalCatalog;
    use crate::terminal_journal::{
        TerminalJournal, TerminalJournalError, TerminalJournalEviction, TerminalJournalLimits,
    };
    use crate::terminal_profile_store::TerminalProfileStore;
    use machine_god_core::{
        BackgroundOutputOwner, SessionId, SessionIncarnationId, TerminalSessionId,
    };
    use rustix::fd::{AsFd, OwnedFd};
    use rustix::fs::{Mode, OFlags};
    use std::fs::{DirBuilder, File};
    use std::io::Write;
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "terminal-profile-budget-{:032x}",
                u128::from_le_bytes(random)
            ));
            DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }

        fn fd(&self) -> OwnedFd {
            rustix::fs::open(
                &self.0,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap()
        }

        fn store(&self) -> TerminalProfileStore {
            TerminalProfileStore::prepare(self.fd()).unwrap()
        }

        fn journal(&self, name: &str) -> (TerminalCatalog, TerminalJournal) {
            let owner = BackgroundOutputOwner::new(
                SessionId::new(name).unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            );
            let mut catalog =
                TerminalCatalog::prepare(self.fd(), "/workspace".into(), owner).unwrap();
            let id = TerminalSessionId::new(name).unwrap();
            let root = catalog.create(&id).unwrap();
            let journal = TerminalJournal::create(root, id, journal_limits()).unwrap();
            (catalog, journal)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn journal_limits() -> TerminalJournalLimits {
        TerminalJournalLimits {
            segment_bytes: 8,
            session_bytes: 64,
        }
    }

    fn budget() -> TerminalProfileBudget {
        TerminalProfileBudget::new(TerminalProfileLimits {
            retained: TerminalProfileLedgers {
                output_bytes: 8,
                protected_bytes: 8192,
                metadata_bytes: MIB,
            },
            temporary_bytes: 64 * MIB + MAX_METADATA,
        })
        .unwrap()
    }

    fn demand(
        output_bytes: u64,
        protected_bytes: u64,
        new_blob_bytes: u64,
    ) -> TerminalProfileDemand {
        TerminalProfileDemand {
            additional_owners: 0,
            additional_sessions: 0,
            retained_growth: TerminalProfileLedgers {
                output_bytes,
                protected_bytes,
                metadata_bytes: MAX_METADATA,
            },
            allocation_bytes: new_blob_bytes + MAX_METADATA,
        }
    }

    fn session_fd(transaction: &TerminalProfileTransaction<'_>, name: &str) -> OwnedFd {
        let inventory = transaction.inventory().unwrap();
        let entry = inventory
            .sessions
            .iter()
            .find(|entry| entry.session_id.as_str() == name)
            .unwrap();
        transaction
            .open_session(&entry.owner_namespace, &entry.session_id)
            .unwrap()
    }

    fn begin_transaction(store: &TerminalProfileStore) -> TerminalProfileTransaction<'_> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match store.transaction() {
                Err(TerminalProfileStoreError::Busy) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                result => return result.unwrap(),
            }
        }
    }

    fn reopen(transaction: &TerminalProfileTransaction<'_>, name: &str) -> TerminalJournal {
        // CLOEXEC descriptors can survive briefly between a parallel test's
        // fork and exec. Retry Busy only, never replay a mutation after failure.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match TerminalJournal::open_existing(
                session_fd(transaction, name),
                &TerminalSessionId::new(name).unwrap(),
                journal_limits(),
            ) {
                Err(TerminalJournalError::Busy) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                result => return result.unwrap(),
            }
        }
    }

    fn put(root: impl AsFd, name: &str, bytes: &[u8]) {
        let fd = rustix::fs::openat(
            root,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .unwrap();
        File::from(fd).write_all(bytes).unwrap();
    }

    #[test]
    fn admission_counts_nonresident_and_busy_foreign_owner_output_before_dispatch() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let (first_catalog, mut first) = fixture.journal("first");
        first.append(b"old!").unwrap();
        drop(first);
        drop(first_catalog);
        let (_second_catalog, mut second) = fixture.journal("second");
        second.append(b"live").unwrap();
        let mut transaction = begin_transaction(&store);
        assert_eq!(transaction.inventory().unwrap().usage.output_bytes, 8);
        assert!(matches!(
            budget().reserve(&mut transaction, demand(1, 0, 1)),
            Err(TerminalProfileError::ResourceLimit)
        ));
        assert_eq!(second.usage().raw_bytes, 4);

        // A separate retention decision obtains the completed journal and
        // commits eviction while the same profile transaction is held.
        let mut completed = reopen(&transaction, "first");
        assert_eq!(
            completed
                .evict(&TerminalJournalEviction::CompletedOutput)
                .unwrap(),
            4
        );
        let completion = budget()
            .reserve(&mut transaction, demand(4, 0, 4))
            .unwrap()
            .run(|| second.append(b"more"))
            .unwrap();
        assert_eq!(completion.operation.unwrap(), second.latest());
        assert_eq!(completion.accounting.unwrap().output_bytes, 8);
    }

    #[test]
    fn protected_admission_is_independent_from_a_full_output_budget() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let (_catalog, mut journal) = fixture.journal("protected");
        journal.append(b"full-raw").unwrap();
        let mut transaction = begin_transaction(&store);
        let completion = budget()
            .reserve(&mut transaction, demand(0, 4096, 4096))
            .unwrap()
            .run(|| journal.publish_state(journal.latest(), &[b's'; 4096]))
            .unwrap();
        assert!(completion.operation.is_ok());
        let usage = completion.accounting.unwrap();
        assert_eq!(usage.output_bytes, 8);
        assert_eq!(usage.protected_bytes, 4096);
        let completion = budget()
            .reserve(&mut transaction, demand(0, 4096, 4096))
            .unwrap()
            .run(|| journal.append_event(&[b'e'; 4096]))
            .unwrap();
        assert_eq!(completion.operation.unwrap(), 1);
        assert_eq!(completion.accounting.unwrap().protected_bytes, 8192);
        assert!(matches!(
            budget().reserve(&mut transaction, demand(0, 1, 1)),
            Err(TerminalProfileError::ResourceLimit)
        ));
        assert_eq!(journal.usage().output_bytes, 8);
    }

    #[test]
    fn checkpoint_replacement_reserves_full_allocation_not_net_growth() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let (_catalog, mut journal) = fixture.journal("checkpoint");
        journal
            .publish_checkpoint(journal.latest(), b"old-grid")
            .unwrap();
        let mut transaction = begin_transaction(&store);
        let mut small = budget();
        small.limits.temporary_bytes = MAX_METADATA + 7;
        assert!(matches!(
            small.reserve(&mut transaction, demand(0, 0, 8)),
            Err(TerminalProfileError::ResourceLimit)
        ));
        assert_eq!(
            journal.load_checkpoint().unwrap().unwrap().bytes,
            b"old-grid"
        );
        let completion = budget()
            .reserve(&mut transaction, demand(0, 0, 8))
            .unwrap()
            .run(|| journal.publish_checkpoint(journal.latest(), b"new-grid"))
            .unwrap();
        assert!(completion.operation.is_ok());
        assert_eq!(completion.accounting.unwrap().output_bytes, 8);
        assert_eq!(
            journal.load_checkpoint().unwrap().unwrap().bytes,
            b"new-grid"
        );
    }

    #[test]
    fn failed_publication_remains_charged_until_physical_recovery() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let (_catalog, mut journal) = fixture.journal("ambiguous");
        journal
            .publish_checkpoint(journal.latest(), b"old-grid")
            .unwrap();
        let mut transaction = begin_transaction(&store);
        put(
            session_fd(&transaction, "ambiguous"),
            "tj-meta.tmp",
            b"interrupted",
        );
        let completion = budget()
            .reserve(&mut transaction, demand(0, 0, 8))
            .unwrap()
            .run(|| journal.publish_checkpoint(journal.latest(), b"new-grid"))
            .unwrap();
        assert!(completion.operation.is_err());
        assert_eq!(
            completion.accounting,
            Err(TerminalProfileError::AccountingMismatch)
        );
        assert_eq!(transaction.inventory().unwrap().usage.output_bytes, 16);
        drop(journal);
        drop(transaction);
        let mut transaction = begin_transaction(&store);
        assert!(matches!(
            budget().reserve(&mut transaction, demand(0, 0, 8)),
            Err(TerminalProfileError::ResourceLimit)
        ));
        let mut recovered = reopen(&transaction, "ambiguous");
        assert_eq!(
            recovered.load_checkpoint().unwrap().unwrap().bytes,
            b"old-grid"
        );
        let completion = budget()
            .reserve(&mut transaction, demand(0, 0, 8))
            .unwrap()
            .run(|| recovered.publish_checkpoint(recovered.latest(), b"new-grid"))
            .unwrap();
        assert!(completion.operation.is_ok());
        assert_eq!(completion.accounting.unwrap().output_bytes, 8);
    }

    #[test]
    fn accounting_failure_preserves_committed_receipt_and_debug_redacts_it() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let (_catalog, mut journal) = fixture.journal("receipt");
        let mut transaction = begin_transaction(&store);
        // Simulate an integration bug: the owner underdeclares output growth.
        let completion = budget()
            .reserve(&mut transaction, demand(0, 0, 1))
            .unwrap()
            .run(|| journal.append(b"x"))
            .unwrap();
        assert_eq!(
            format!("{completion:?}"),
            "TerminalProfileCompletion { .. }"
        );
        assert_eq!(completion.operation.unwrap(), journal.latest());
        assert_eq!(
            completion.accounting,
            Err(TerminalProfileError::AccountingMismatch)
        );
        assert_eq!(transaction.inventory().unwrap().usage.output_bytes, 1);
    }

    #[test]
    fn unwind_cannot_release_profile_lock_or_hide_committed_bytes() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let contender = fixture.store();
        let (_catalog, mut journal) = fixture.journal("unwind");
        let mut transaction = begin_transaction(&store);
        let reservation = budget().reserve(&mut transaction, demand(8, 0, 8)).unwrap();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            reservation
                .run(|| -> std::result::Result<(), ()> {
                    journal.append(b"retained").unwrap();
                    panic!("owner interrupted after commit");
                })
                .unwrap();
        }));
        assert!(panic.is_err());
        assert!(matches!(
            contender.transaction(),
            Err(TerminalProfileStoreError::Busy)
        ));
        assert!(matches!(
            budget().reserve(&mut transaction, demand(1, 0, 1)),
            Err(TerminalProfileError::ResourceLimit)
        ));
        drop(transaction);
        assert_eq!(
            begin_transaction(&contender)
                .inventory()
                .unwrap()
                .usage
                .output_bytes,
            8
        );
    }

    #[test]
    fn replaced_profile_lock_rejects_before_reserved_operation() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut transaction = begin_transaction(&store);
        let reservation = budget().reserve(&mut transaction, demand(0, 0, 0)).unwrap();
        let namespace = fixture.0.join("terminal-v1");
        std::fs::rename(
            namespace.join("profile-lock"),
            namespace.join("replaced-lock"),
        )
        .unwrap();
        let mut called = false;
        let result = reservation.run(|| -> std::result::Result<(), ()> {
            called = true;
            Ok(())
        });
        assert!(result.is_err());
        assert!(!called);
    }

    #[test]
    fn namespace_creation_is_admitted_and_empty_owners_consume_capacity() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut transaction = begin_transaction(&store);
        let mut request = demand(0, 0, 0);
        request.additional_owners = 1;
        request.additional_sessions = 1;
        let completion = budget()
            .reserve(&mut transaction, request)
            .unwrap()
            .run(|| Ok::<_, ()>(fixture.journal("new-owner")))
            .unwrap();
        assert!(completion.operation.is_ok());
        assert!(completion.accounting.is_ok());
        let inventory = transaction.inventory().unwrap();
        assert_eq!(inventory.owner_count, 1);
        assert_eq!(inventory.sessions.len(), 1);

        request.additional_sessions = MAX_PROFILE_SESSIONS;
        assert!(matches!(
            budget().reserve(&mut transaction, request),
            Err(TerminalProfileError::ResourceLimit)
        ));
        request.additional_sessions = 0;
        request.additional_owners = MAX_PROFILE_OWNERS;
        assert!(matches!(
            budget().reserve(&mut transaction, request),
            Err(TerminalProfileError::ResourceLimit)
        ));
        request.additional_owners = usize::MAX;
        assert!(matches!(
            budget().reserve(&mut transaction, request),
            Err(TerminalProfileError::ResourceLimit)
        ));

        // A recognized empty namespace still occupies one owner slot.
        DirBuilder::new()
            .mode(0o700)
            .create(fixture.0.join("terminal-v1").join("a".repeat(64)))
            .unwrap();
        let inventory = transaction.inventory().unwrap();
        assert_eq!(inventory.owner_count, 2);
        assert_eq!(inventory.sessions.len(), 1);
        request.additional_owners = MAX_PROFILE_OWNERS - 1;
        assert!(matches!(
            budget().reserve(&mut transaction, request),
            Err(TerminalProfileError::ResourceLimit)
        ));
    }

    #[test]
    fn invalid_demands_and_overflows_reject_before_operation() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut transaction = begin_transaction(&store);
        let mut request = demand(1, 0, 1);
        request.allocation_bytes = 0;
        assert!(matches!(
            budget().reserve(&mut transaction, request),
            Err(TerminalProfileError::Invalid)
        ));
        request.retained_growth.output_bytes = u64::MAX;
        assert!(matches!(
            budget().reserve(&mut transaction, request),
            Err(TerminalProfileError::ResourceLimit)
        ));
        assert!(transaction.inventory().unwrap().sessions.is_empty());
        let mut limits = TerminalProfileLimits::default();
        limits.retained.output_bytes += 1;
        assert!(matches!(
            TerminalProfileBudget::new(limits),
            Err(TerminalProfileError::Invalid)
        ));
    }
}
