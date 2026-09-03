//! Descriptor-confined persistence used by the native background supervisor.

#![cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::io::{Read, Write};

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{AtFlags, Dir, FileType, FlockOperation, Mode, OFlags, RenameFlags};

use crate::background_inspection::{
    BACKGROUND_DIRECTORY, MAX_BACKGROUND_DIRECTORY_ENTRIES, MAX_BACKGROUND_PATH_BYTES,
    MAX_BACKGROUND_RECORD_BYTES, MAX_BACKGROUND_TOTAL_RECORD_BYTES, NativeBackgroundState,
    StoredBackgroundRecord, background_record_name, background_workspace_name,
    is_background_record_name, is_canonical_absolute_background_path,
    supported::decode_stored_record, valid_background_record,
};

const PRIVATE_DIRECTORY_MODE: Mode = Mode::from_raw_mode(0o700);
const PRIVATE_FILE_MODE: Mode = Mode::from_raw_mode(0o600);
const GROUP_OR_OTHER_PERMISSIONS: u64 = 0o077;
const CONTROL_DIRECTORY: &str = "control-v1";
const ALLOCATOR_LOCK_NAME: &str = "allocator.lock";
const ALLOCATOR_COUNTER_NAME: &str = "allocator.counter";
const ALLOCATOR_COUNTER_TEMP_NAME: &str = "allocator.counter.tmp";
const COUNTER_BYTES: usize = size_of::<u64>();
const COUNTER_BYTES_I64: i64 = 8;
const MAX_RETAINED_BACKGROUND_RECORDS: usize = 100;
// Lifecycle scans may inspect 1,024 entries, but maintenance must never turn
// that namespace bound into an equivalent descriptor-authority requirement.
const MAX_COMPACTION_AUTHORITY_FDS: usize = 8;

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_PUBLISHED_ROOT_SYNC: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

/// Stable category for a descriptor-confined background-store failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackgroundStoreErrorKind {
    InvalidWorkspace,
    Corrupt,
    ResourceLimit,
    IdExhausted,
    Conflict,
    Unavailable,
}

/// Fixed and redacted descriptor-confined background-store failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct BackgroundStoreError {
    kind: BackgroundStoreErrorKind,
}

impl BackgroundStoreError {
    pub(crate) const fn kind(self) -> BackgroundStoreErrorKind {
        self.kind
    }

    const fn new(kind: BackgroundStoreErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for BackgroundStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundStoreError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for BackgroundStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            BackgroundStoreErrorKind::InvalidWorkspace => "native background workspace is invalid",
            BackgroundStoreErrorKind::Corrupt => "native background store is corrupt",
            BackgroundStoreErrorKind::ResourceLimit => {
                "native background store reached a resource limit"
            }
            BackgroundStoreErrorKind::IdExhausted => {
                "native background identifier space is exhausted"
            }
            BackgroundStoreErrorKind::Conflict => "native background record conflicts",
            BackgroundStoreErrorKind::Unavailable => "native background store is unavailable",
        })
    }
}

impl Error for BackgroundStoreError {}

/// A retained exclusive per-record authority.
///
/// Dropping the lease releases the authority. A later bounded compaction may
/// unlink the lock entry only after proving that no live lease owns it.
pub(crate) struct BackgroundRecordLease {
    id: u64,
    root_device: i128,
    root_inode: u128,
    control_device: i128,
    control_inode: u128,
    _lock: ExclusiveFileLock,
}

impl BackgroundRecordLease {
    pub(crate) const fn id(&self) -> u64 {
        self.id
    }
}

impl fmt::Debug for BackgroundRecordLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundRecordLease")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BackgroundReconciliation {
    pub(crate) inspected: usize,
    pub(crate) active: usize,
    pub(crate) marked_stale: usize,
}

/// Descriptor-confined schema-v1 background record store.
pub(crate) struct BackgroundStore {
    _state_root: OwnedFd,
    root: OwnedFd,
    control: OwnedFd,
    workspace: String,
    root_device: i128,
    root_inode: u128,
    control_device: i128,
    control_inode: u128,
}

impl BackgroundStore {
    /// Prepares fixed private child directories beneath an already-retained
    /// `machine-god` state-root descriptor.
    pub(crate) fn prepare(
        state_root: OwnedFd,
        workspace: String,
    ) -> Result<Self, BackgroundStoreError> {
        if workspace.len() > MAX_BACKGROUND_PATH_BYTES
            || workspace.contains('\0')
            || !is_canonical_absolute_background_path(&workspace)
        {
            return Err(error(BackgroundStoreErrorKind::InvalidWorkspace));
        }
        Self::validate_state_root(&state_root)?;
        let background = prepare_private_directory(state_root.as_fd(), BACKGROUND_DIRECTORY)?;
        let workspace_name = background_workspace_name(&workspace);
        let root = prepare_private_directory(background.as_fd(), &workspace_name)?;
        let metadata = validate_private_directory(&root)?;
        let control = prepare_private_directory(root.as_fd(), CONTROL_DIRECTORY)?;
        let control_metadata = validate_private_directory(&control)?;

        let store = Self {
            _state_root: state_root,
            root,
            control,
            workspace,
            root_device: i128::from(metadata.st_dev),
            root_inode: u128::from(metadata.st_ino),
            control_device: i128::from(control_metadata.st_dev),
            control_inode: u128::from(control_metadata.st_ino),
        };
        store.prepare_allocator()?;
        Ok(store)
    }

    /// Validates an already-retained state-root descriptor without creating or
    /// reconciling any background namespace entries.
    pub(crate) fn validate_state_root(state_root: &OwnedFd) -> Result<(), BackgroundStoreError> {
        validate_private_directory(state_root).map(|_| ())
    }

    pub(crate) fn workspace(&self) -> &str {
        &self.workspace
    }

    /// Durably advances the global workspace counter, then acquires and
    /// returns the permanent lock authority for the reserved nonzero ID.
    pub(crate) fn reserve_id(&self) -> Result<BackgroundRecordLease, BackgroundStoreError> {
        self.validate_root()?;
        let _allocator = acquire_exclusive(open_private_lock(
            self.control.as_fd(),
            ALLOCATOR_LOCK_NAME,
        )?)?;
        let current = read_counter(self.control.as_fd())?;
        let id = current
            .checked_add(1)
            .ok_or_else(|| error(BackgroundStoreErrorKind::IdExhausted))?;
        self.compact_history_locked(MAX_RETAINED_BACKGROUND_RECORDS - 1, current)?;
        replace_counter(self.control.as_fd(), id)?;

        let lock = acquire_exclusive(open_private_lock(
            self.control.as_fd(),
            &record_lock_name(id),
        )?)?;
        Ok(BackgroundRecordLease {
            id,
            root_device: self.root_device,
            root_inode: self.root_inode,
            control_device: self.control_device,
            control_inode: self.control_inode,
            _lock: lock,
        })
    }

    /// Publishes a new running record without replacing an existing record.
    pub(crate) fn publish_initial(
        &self,
        lease: &BackgroundRecordLease,
        record: &StoredBackgroundRecord,
    ) -> Result<(), BackgroundStoreError> {
        self.validate_lease(lease)?;
        if record.id != lease.id
            || record.state != NativeBackgroundState::Running
            || !valid_background_record(record, &self.workspace)
        {
            return Err(error(BackgroundStoreErrorKind::Corrupt));
        }
        let bytes = serialize_record(record)?;
        publish_no_clobber(
            self.root.as_fd(),
            self.control.as_fd(),
            &self.workspace,
            &record_names(lease.id),
            &bytes,
        )
    }

    /// Atomically replaces a record while its caller retains the record lease.
    pub(crate) fn replace(
        &self,
        lease: &BackgroundRecordLease,
        replacement: &StoredBackgroundRecord,
    ) -> Result<(), BackgroundStoreError> {
        self.validate_lease(lease)?;
        if replacement.id != lease.id || !valid_background_record(replacement, &self.workspace) {
            return Err(error(BackgroundStoreErrorKind::Corrupt));
        }
        let names = record_names(lease.id);
        let (current, _) = read_record(self.root.as_fd(), &self.workspace, &names.data)?
            .ok_or_else(|| error(BackgroundStoreErrorKind::Conflict))?;
        if !valid_replacement(&current, replacement) {
            return Err(error(BackgroundStoreErrorKind::Conflict));
        }
        let bytes = serialize_record(replacement)?;
        publish_replace(self.root.as_fd(), self.control.as_fd(), &names, &bytes)
    }

    /// Validates a complete bounded snapshot before marking unlocked running
    /// records stale. It performs no PID probe and sends no signal.
    pub(crate) fn reconcile(&self) -> Result<BackgroundReconciliation, BackgroundStoreError> {
        self.reconcile_inner(|| {})
    }

    fn reconcile_inner(
        &self,
        after_snapshot: impl FnOnce(),
    ) -> Result<BackgroundReconciliation, BackgroundStoreError> {
        self.validate_root()?;
        let _allocator = acquire_exclusive(open_private_lock(
            self.control.as_fd(),
            ALLOCATOR_LOCK_NAME,
        )?)?;
        let current = read_counter(self.control.as_fd())?;
        self.compact_history_locked(MAX_RETAINED_BACKGROUND_RECORDS, current)?;
        let records = scan_complete_records(self.root.as_fd(), &self.workspace)?;
        let mut result = BackgroundReconciliation {
            inspected: records.len(),
            active: 0,
            marked_stale: 0,
        };
        after_snapshot();

        for observed in records {
            if observed.state != NativeBackgroundState::Running {
                continue;
            }
            let lock =
                open_existing_private_file(self.control.as_fd(), &record_lock_name(observed.id))?;
            let Some(_authority) = try_acquire_exclusive(lock)? else {
                result.active += 1;
                continue;
            };

            // The preflight snapshot proves global completeness. Re-read under
            // this record's authority so a just-finished owner is never
            // overwritten with the earlier running value.
            let names = record_names(observed.id);
            let Some((current, _)) = read_record(self.root.as_fd(), &self.workspace, &names.data)?
            else {
                return Err(unavailable());
            };
            if current.state != NativeBackgroundState::Running {
                continue;
            }
            let mut stale = current;
            stale.state = NativeBackgroundState::Stale;
            let bytes = serialize_record(&stale)?;
            publish_replace(self.root.as_fd(), self.control.as_fd(), &names, &bytes)?;
            result.marked_stale += 1;
        }
        Ok(result)
    }

    fn prepare_allocator(&self) -> Result<(), BackgroundStoreError> {
        let _allocator = acquire_exclusive(open_private_lock(
            self.control.as_fd(),
            ALLOCATOR_LOCK_NAME,
        )?)?;
        let current = if let Some(current) = read_counter_optional(self.control.as_fd())? {
            current
        } else {
            let control = scan_control_snapshot(self.control.as_fd())?;
            if !scan_complete_records(self.root.as_fd(), &self.workspace)?.is_empty()
                || !control.lock_names.is_empty()
                || !control.temp_names.is_empty()
            {
                return Err(corrupt());
            }
            publish_counter_initial(self.control.as_fd())?;
            0
        };
        self.compact_history_locked(MAX_RETAINED_BACKGROUND_RECORDS, current)
    }

    /// Compacts only after complete record and control snapshots validate.
    /// The caller holds the workspace allocator lock, serializing compactors
    /// and ID allocation across cooperating processes.
    fn compact_history_locked(
        &self,
        maximum_occupancy: usize,
        allocator_value: u64,
    ) -> Result<(), BackgroundStoreError> {
        self.compact_history_locked_inner(maximum_occupancy, allocator_value, || {})
    }

    fn compact_history_locked_inner(
        &self,
        maximum_occupancy: usize,
        allocator_value: u64,
        after_record_snapshot: impl FnOnce(),
    ) -> Result<(), BackgroundStoreError> {
        let records = scan_complete_records(self.root.as_fd(), &self.workspace)?;
        if records.iter().any(|record| record.id > allocator_value) {
            return Err(corrupt());
        }
        after_record_snapshot();
        let control = scan_control_snapshot(self.control.as_fd())?;
        let expected_locks: BTreeSet<String> = records
            .iter()
            .map(|record| record_lock_name(record.id))
            .collect();
        if !expected_locks.is_subset(&control.lock_names) {
            return Err(corrupt());
        }

        let mut orphan_locks = BTreeSet::new();
        let mut removable_temps = BTreeSet::new();
        let mut contended_locks = BTreeSet::new();

        let pending = self.classify_orphan_locks(
            &control,
            &expected_locks,
            &mut orphan_locks,
            &mut contended_locks,
        )?;
        self.schedule_temp_cleanup(&control, &mut removable_temps, &mut contended_locks)?;
        let maximum_records = maximum_occupancy
            .checked_sub(pending)
            .ok_or_else(|| error(BackgroundStoreErrorKind::ResourceLimit))?;
        let remove_count = records.len().saturating_sub(maximum_records);
        let victims = self.select_terminal_victims(&records, remove_count, &mut contended_locks)?;
        if victims.len() != remove_count {
            return Err(error(BackgroundStoreErrorKind::ResourceLimit));
        }

        self.commit_compaction(&victims, &orphan_locks, &removable_temps)
    }

    fn classify_orphan_locks(
        &self,
        control: &ControlSnapshot,
        expected_locks: &BTreeSet<String>,
        orphan_locks: &mut BTreeSet<String>,
        contended_locks: &mut BTreeSet<String>,
    ) -> Result<usize, BackgroundStoreError> {
        let mut pending = 0_usize;
        for name in &control.lock_names {
            if expected_locks.contains(name) {
                continue;
            }
            let lock = open_existing_private_file(self.control.as_fd(), name)?;
            if let Some(_authority) = try_acquire_exclusive(lock)? {
                // Record publication and the control snapshot are not one
                // atomic namespace operation. If a publisher completed in
                // that interval and released its lease, this acquired lock is
                // required rather than orphaned. Abort before any mutation;
                // the next allocator-serialized attempt takes a fresh pair of
                // snapshots.
                let data_name = data_name_for_lock(name);
                if read_record(self.root.as_fd(), &self.workspace, &data_name)?.is_some() {
                    return Err(unavailable());
                }
                orphan_locks.insert(name.clone());
            } else {
                contended_locks.insert(name.clone());
                pending = pending
                    .checked_add(1)
                    .ok_or_else(|| error(BackgroundStoreErrorKind::ResourceLimit))?;
            }
        }
        Ok(pending)
    }

    fn schedule_temp_cleanup(
        &self,
        control: &ControlSnapshot,
        removable_temps: &mut BTreeSet<String>,
        contended_locks: &mut BTreeSet<String>,
    ) -> Result<(), BackgroundStoreError> {
        for temp_name in &control.temp_names {
            let lock_name = temp_lock_name(temp_name);
            if contended_locks.contains(&lock_name) {
                continue;
            }
            if !control.lock_names.contains(&lock_name) {
                removable_temps.insert(temp_name.clone());
                continue;
            }
            let lock = open_existing_private_file(self.control.as_fd(), &lock_name)?;
            if let Some(_authority) = try_acquire_exclusive(lock)? {
                removable_temps.insert(temp_name.clone());
            } else {
                contended_locks.insert(lock_name);
            }
        }
        Ok(())
    }

    fn select_terminal_victims(
        &self,
        records: &[StoredBackgroundRecord],
        remove_count: usize,
        contended_locks: &mut BTreeSet<String>,
    ) -> Result<Vec<TerminalVictim>, BackgroundStoreError> {
        let mut terminal: Vec<_> = records
            .iter()
            .filter(|record| record.state != NativeBackgroundState::Running)
            .collect();
        terminal.sort_unstable_by(|left, right| {
            left.updated_at_ms
                .cmp(&right.updated_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut victims = Vec::with_capacity(remove_count);
        for record in terminal {
            if victims.len() == remove_count {
                break;
            }
            let lock_name = record_lock_name(record.id);
            if contended_locks.contains(&lock_name) {
                continue;
            }
            let lock = open_existing_private_file(self.control.as_fd(), &lock_name)?;
            let Some(_authority) = try_acquire_exclusive(lock)? else {
                contended_locks.insert(lock_name);
                continue;
            };
            let data_name = background_record_name(record.id);
            let Some((current, _)) = read_record(self.root.as_fd(), &self.workspace, &data_name)?
            else {
                return Err(unavailable());
            };
            if &current == record && current.state != NativeBackgroundState::Running {
                victims.push(TerminalVictim {
                    data_name,
                    lock_name,
                    record: current,
                });
            }
        }
        Ok(victims)
    }

    fn commit_compaction(
        &self,
        victims: &[TerminalVictim],
        orphan_locks: &BTreeSet<String>,
        removable_temps: &BTreeSet<String>,
    ) -> Result<(), BackgroundStoreError> {
        // Capacity is proven for the complete victim set before phase two.
        // Revalidation and mutation then proceed in fixed-authority batches;
        // cooperating writers cannot create a lease for an old terminal ID
        // while the allocator lock is held.
        for batch in victims.chunks(MAX_COMPACTION_AUTHORITY_FDS) {
            let mut authorities = Vec::with_capacity(batch.len());
            for victim in batch {
                let lock = open_existing_private_file(self.control.as_fd(), &victim.lock_name)?;
                let Some(authority) = try_acquire_exclusive(lock)? else {
                    return Err(unavailable());
                };
                let Some((current, _)) =
                    read_record(self.root.as_fd(), &self.workspace, &victim.data_name)?
                else {
                    return Err(unavailable());
                };
                if current != victim.record || current.state == NativeBackgroundState::Running {
                    return Err(unavailable());
                }
                authorities.push(authority);
            }
            for victim in batch {
                rustix::fs::unlinkat(self.root.as_fd(), &victim.data_name, AtFlags::empty())
                    .map_err(|_| unavailable())?;
            }
            sync_directory(self.root.as_fd())?;
            for victim in batch {
                rustix::fs::unlinkat(self.control.as_fd(), &victim.lock_name, AtFlags::empty())
                    .map_err(|_| unavailable())?;
            }
            sync_directory(self.control.as_fd())?;
            drop(authorities);
        }

        // Orphan locks are garbage, so successful batches may make bounded
        // progress independently. Each batch is re-proven orphaned under its
        // authority before its namespace entries are removed.
        for batch in orphan_locks
            .iter()
            .collect::<Vec<_>>()
            .chunks(MAX_COMPACTION_AUTHORITY_FDS)
        {
            let mut authorities = Vec::with_capacity(batch.len());
            for name in batch {
                let lock = open_existing_private_file(self.control.as_fd(), name)?;
                let Some(authority) = try_acquire_exclusive(lock)? else {
                    continue;
                };
                if read_record(
                    self.root.as_fd(),
                    &self.workspace,
                    &data_name_for_lock(name),
                )?
                .is_some()
                {
                    return Err(unavailable());
                }
                authorities.push(((*name).clone(), authority));
            }
            for (name, _) in &authorities {
                rustix::fs::unlinkat(self.control.as_fd(), name, AtFlags::empty())
                    .map_err(|_| unavailable())?;
            }
            if !authorities.is_empty() {
                sync_directory(self.control.as_fd())?;
            }
        }

        // Temps are never valid history. Recheck the corresponding lock one
        // at a time and preserve any temp whose owner is currently active.
        let mut removed_temp = false;
        for temp_name in removable_temps {
            let lock_name = temp_lock_name(temp_name);
            let lock = open_existing_private_file_optional(self.control.as_fd(), &lock_name)?;
            let _authority = match lock {
                Some(lock) => {
                    let Some(authority) = try_acquire_exclusive(lock)? else {
                        continue;
                    };
                    Some(authority)
                }
                None => None,
            };
            match rustix::fs::unlinkat(self.control.as_fd(), temp_name, AtFlags::empty()) {
                Ok(()) => removed_temp = true,
                Err(error) if error == rustix::io::Errno::NOENT => {}
                Err(_) => return Err(unavailable()),
            }
        }
        if removed_temp {
            sync_directory(self.control.as_fd())?;
        }
        Ok(())
    }

    fn validate_root(&self) -> Result<(), BackgroundStoreError> {
        let metadata = validate_private_directory(&self.root)?;
        if i128::from(metadata.st_dev) != self.root_device
            || u128::from(metadata.st_ino) != self.root_inode
        {
            return Err(unavailable());
        }
        let control = validate_private_directory(&self.control)?;
        if i128::from(control.st_dev) != self.control_device
            || u128::from(control.st_ino) != self.control_inode
        {
            return Err(unavailable());
        }
        Ok(())
    }

    fn validate_lease(&self, lease: &BackgroundRecordLease) -> Result<(), BackgroundStoreError> {
        self.validate_root()?;
        if lease.root_device != self.root_device
            || lease.root_inode != self.root_inode
            || lease.control_device != self.control_device
            || lease.control_inode != self.control_inode
        {
            return Err(error(BackgroundStoreErrorKind::Conflict));
        }
        Ok(())
    }
}

impl fmt::Debug for BackgroundStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundStore")
            .finish_non_exhaustive()
    }
}

struct RecordNames {
    data: String,
    temp: String,
}

struct ControlSnapshot {
    lock_names: BTreeSet<String>,
    temp_names: BTreeSet<String>,
}

struct TerminalVictim {
    data_name: String,
    lock_name: String,
    record: StoredBackgroundRecord,
}

/// Owns one successfully acquired flock authority.
///
/// Closing the descriptor alone is insufficient when a concurrently spawned
/// child inherited the same open-file description before `exec`. Explicitly
/// unlocking on logical release updates that shared description immediately.
struct ExclusiveFileLock {
    file: OwnedFd,
}

impl AsFd for ExclusiveFileLock {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        let _ = retry_interrupted(|| rustix::fs::flock(&self.file, FlockOperation::Unlock));
    }
}

fn record_names(id: u64) -> RecordNames {
    let data = background_record_name(id);
    let stem = data
        .strip_suffix(".json")
        .expect("canonical background record name has fixed suffix");
    let temp = format!("{stem}.tmp");
    RecordNames { data, temp }
}

fn record_lock_name(id: u64) -> String {
    let data = background_record_name(id);
    let stem = data
        .strip_suffix(".json")
        .expect("canonical background record name has fixed suffix");
    format!("{stem}.lock")
}

fn data_name_for_lock(lock_name: &str) -> String {
    let stem = lock_name
        .strip_suffix(".lock")
        .expect("validated record lock has a fixed suffix");
    format!("{stem}.json")
}

fn temp_lock_name(temp_name: &str) -> String {
    let stem = temp_name
        .strip_suffix(".tmp")
        .expect("validated record temp has a fixed suffix");
    format!("{stem}.lock")
}

fn valid_replacement(
    current: &StoredBackgroundRecord,
    replacement: &StoredBackgroundRecord,
) -> bool {
    current.id == replacement.id
        && current.workspace == replacement.workspace
        && current.started_at_ms == replacement.started_at_ms
        && current.command == replacement.command
        && current.cwd == replacement.cwd
        && current.pid == replacement.pid
        && replacement.updated_at_ms >= current.updated_at_ms
        && current.state == NativeBackgroundState::Running
}

fn serialize_record(record: &StoredBackgroundRecord) -> Result<Vec<u8>, BackgroundStoreError> {
    let mut output = BoundedSerialization::new();
    if serde_json::to_writer(&mut output, record).is_err() {
        return Err(if output.overflowed {
            error(BackgroundStoreErrorKind::ResourceLimit)
        } else {
            corrupt()
        });
    }
    if output.bytes.len() > MAX_BACKGROUND_RECORD_BYTES {
        return Err(error(BackgroundStoreErrorKind::ResourceLimit));
    }
    Ok(output.bytes)
}

struct BoundedSerialization {
    bytes: Vec<u8>,
    overflowed: bool,
}

impl BoundedSerialization {
    fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(MAX_BACKGROUND_RECORD_BYTES + 1),
            overflowed: false,
        }
    }
}

impl Write for BoundedSerialization {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let remaining = (MAX_BACKGROUND_RECORD_BYTES + 1).saturating_sub(self.bytes.len());
        if remaining == 0 {
            self.overflowed = true;
            return Err(std::io::Error::other("background serialization limit"));
        }
        let accepted = remaining.min(buffer.len());
        self.bytes.extend_from_slice(&buffer[..accepted]);
        if accepted < buffer.len() {
            self.overflowed = true;
        }
        Ok(accepted)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn scan_complete_records(
    root: BorrowedFd<'_>,
    workspace: &str,
) -> Result<Vec<StoredBackgroundRecord>, BackgroundStoreError> {
    let duplicate = rustix::fs::openat(root, ".", directory_open_flags(), Mode::empty())
        .map_err(|_| unavailable())?;
    let mut directory = Dir::new(duplicate).map_err(|_| unavailable())?;
    let mut names = Vec::new();
    let mut entries = 0_usize;
    loop {
        let Some(entry) = directory.next() else {
            break;
        };
        let entry = entry.map_err(|_| unavailable())?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        if entries == MAX_BACKGROUND_DIRECTORY_ENTRIES {
            return Err(error(BackgroundStoreErrorKind::ResourceLimit));
        }
        entries += 1;
        if is_background_record_name(name) {
            names.push(
                std::str::from_utf8(name)
                    .expect("canonical background names are ASCII")
                    .to_owned(),
            );
        }
    }
    names.sort_unstable();
    names.dedup();

    // Reconciliation must validate every canonical candidate in its bounded
    // snapshot. The reader's smaller presentation limit does not constrain
    // writer lifecycle maintenance; the directory-entry and aggregate-byte
    // budgets above and below remain the hard reconciliation bounds.
    let mut records = Vec::with_capacity(names.len());
    let mut total = 0_usize;
    for name in names {
        let (record, encoded_bytes) = read_record(root, workspace, &name)?.ok_or_else(corrupt)?;
        total = total
            .checked_add(encoded_bytes)
            .filter(|total| *total <= MAX_BACKGROUND_TOTAL_RECORD_BYTES)
            .ok_or_else(|| error(BackgroundStoreErrorKind::ResourceLimit))?;
        records.push(record);
    }
    Ok(records)
}

fn scan_control_snapshot(control: BorrowedFd<'_>) -> Result<ControlSnapshot, BackgroundStoreError> {
    let duplicate = rustix::fs::openat(control, ".", directory_open_flags(), Mode::empty())
        .map_err(|_| unavailable())?;
    let mut directory = Dir::new(duplicate).map_err(|_| unavailable())?;
    let mut lock_names = BTreeSet::new();
    let mut temp_names = BTreeSet::new();
    let mut entries = 0_usize;
    let mut temp_bytes = 0_usize;
    loop {
        let Some(entry) = directory.next() else {
            break;
        };
        let entry = entry.map_err(|_| unavailable())?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        if entries == MAX_BACKGROUND_DIRECTORY_ENTRIES {
            return Err(error(BackgroundStoreErrorKind::ResourceLimit));
        }
        entries += 1;
        if is_background_record_lock_name(name) {
            let name = std::str::from_utf8(name)
                .expect("canonical background lock names are ASCII")
                .to_owned();
            let file = open_existing_private_file(control, &name)?;
            validate_private_file(&file)?;
            lock_names.insert(name);
        } else if is_background_record_temp_name(name) {
            let name = std::str::from_utf8(name)
                .expect("canonical background temp names are ASCII")
                .to_owned();
            let file = open_existing_private_file(control, &name)?;
            let metadata = validate_private_file(&file)?;
            let bytes = usize::try_from(metadata.st_size).map_err(|_| corrupt())?;
            if bytes > MAX_BACKGROUND_RECORD_BYTES {
                return Err(error(BackgroundStoreErrorKind::ResourceLimit));
            }
            temp_bytes = temp_bytes
                .checked_add(bytes)
                .filter(|total| *total <= MAX_BACKGROUND_TOTAL_RECORD_BYTES)
                .ok_or_else(|| error(BackgroundStoreErrorKind::ResourceLimit))?;
            temp_names.insert(name);
        }
    }
    Ok(ControlSnapshot {
        lock_names,
        temp_names,
    })
}

fn is_background_record_lock_name(name: &[u8]) -> bool {
    is_background_control_record_name(name, b".lock")
}

fn is_background_record_temp_name(name: &[u8]) -> bool {
    is_background_control_record_name(name, b".tmp")
}

fn is_background_control_record_name(name: &[u8], suffix: &[u8]) -> bool {
    const PREFIX: &[u8] = b"record-";
    name.len() == PREFIX.len() + 64 + suffix.len()
        && name.starts_with(PREFIX)
        && name.ends_with(suffix)
        && name[PREFIX.len()..name.len() - suffix.len()]
            .iter()
            .all(u8::is_ascii_hexdigit)
        && !name[PREFIX.len()..name.len() - suffix.len()]
            .iter()
            .any(u8::is_ascii_uppercase)
}

fn read_record(
    root: BorrowedFd<'_>,
    workspace: &str,
    name: &str,
) -> Result<Option<(StoredBackgroundRecord, usize)>, BackgroundStoreError> {
    #[cfg(target_os = "macos")]
    let preflight = match rustix::fs::openat(
        root,
        name,
        OFlags::from_bits_retain(libc::O_EVTONLY as _)
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(file) => file,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(error) if is_rejected_type_error(error) => return Err(corrupt()),
        Err(error)
            if error == rustix::io::Errno::ACCESS
                && rustix::fs::statat(root, name, AtFlags::SYMLINK_NOFOLLOW).is_ok_and(
                    |metadata| {
                        FileType::from_raw_mode(metadata.st_mode).is_file()
                            && metadata.st_uid == rustix::process::geteuid().as_raw()
                            && u64::from(metadata.st_mode) & GROUP_OR_OTHER_PERMISSIONS == 0
                            && u64::from(metadata.st_mode) & 0o400 != 0
                    },
                ) =>
        {
            return Err(corrupt());
        }
        Err(_) => return Err(unavailable()),
    };
    #[cfg(target_os = "macos")]
    let preflight_metadata = validate_private_file(&preflight)?;

    let file = match rustix::fs::openat(root, name, record_open_flags(), Mode::empty()) {
        Ok(file) => file,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(error) if is_rejected_type_error(error) => return Err(corrupt()),
        #[cfg(target_os = "macos")]
        Err(error) if error == rustix::io::Errno::ACCESS => return Err(corrupt()),
        Err(_) => return Err(unavailable()),
    };
    #[cfg(target_os = "linux")]
    validate_private_file(&file)?;
    #[cfg(target_os = "macos")]
    let metadata = validate_private_file(&file)?;
    #[cfg(target_os = "macos")]
    if metadata.st_dev != preflight_metadata.st_dev || metadata.st_ino != preflight_metadata.st_ino
    {
        return Err(unavailable());
    }
    let mut bytes = Vec::with_capacity(MAX_BACKGROUND_RECORD_BYTES + 1);
    std::fs::File::from(file)
        .take((MAX_BACKGROUND_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() > MAX_BACKGROUND_RECORD_BYTES {
        return Err(corrupt());
    }
    let encoded_bytes = bytes.len();
    decode_stored_record(&bytes, workspace, name)
        .map(|record| Some((record, encoded_bytes)))
        .map_err(|_| corrupt())
}

fn publish_no_clobber(
    root: BorrowedFd<'_>,
    control: BorrowedFd<'_>,
    workspace: &str,
    names: &RecordNames,
    bytes: &[u8],
) -> Result<(), BackgroundStoreError> {
    match read_record(root, workspace, &names.data) {
        Ok(None) => {}
        Ok(Some(_)) => return Err(error(BackgroundStoreErrorKind::Conflict)),
        Err(error) if error.kind() == BackgroundStoreErrorKind::Corrupt => return Err(error),
        Err(_) => return Err(unavailable()),
    }
    let temp = create_private_temp(control, &names.temp)?;
    if let Err(result) = write_and_sync(&temp, bytes) {
        cleanup_temp(control, &names.temp);
        return Err(result);
    }
    if let Err(rename_error) = rustix::fs::renameat_with(
        control,
        &names.temp,
        root,
        &names.data,
        RenameFlags::NOREPLACE,
    ) {
        cleanup_temp(control, &names.temp);
        return Err(if rename_error == rustix::io::Errno::EXIST {
            error(BackgroundStoreErrorKind::Conflict)
        } else {
            unavailable()
        });
    }
    sync_published_root(root)?;
    sync_directory(control)
}

fn publish_replace(
    root: BorrowedFd<'_>,
    control: BorrowedFd<'_>,
    names: &RecordNames,
    bytes: &[u8],
) -> Result<(), BackgroundStoreError> {
    let temp = create_private_temp(control, &names.temp)?;
    if let Err(result) = write_and_sync(&temp, bytes) {
        cleanup_temp(control, &names.temp);
        return Err(result);
    }
    if rustix::fs::renameat(control, &names.temp, root, &names.data).is_err() {
        cleanup_temp(control, &names.temp);
        return Err(unavailable());
    }
    sync_published_root(root)?;
    sync_directory(control)
}

fn sync_published_root(root: BorrowedFd<'_>) -> Result<(), BackgroundStoreError> {
    #[cfg(test)]
    if FAIL_NEXT_PUBLISHED_ROOT_SYNC.with(|failure| failure.replace(false)) {
        return Err(unavailable());
    }
    sync_directory(root)
}

fn prepare_private_directory(
    parent: BorrowedFd<'_>,
    name: &str,
) -> Result<OwnedFd, BackgroundStoreError> {
    let mut created = false;
    match rustix::fs::mkdirat(parent, name, PRIVATE_DIRECTORY_MODE) {
        Ok(()) => {
            rustix::fs::chmodat(parent, name, PRIVATE_DIRECTORY_MODE, AtFlags::empty())
                .map_err(|_| unavailable())?;
            created = true;
        }
        Err(error) if error == rustix::io::Errno::EXIST => {}
        Err(_) => return Err(unavailable()),
    }
    let before =
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| unavailable())?;
    if !FileType::from_raw_mode(before.st_mode).is_dir() {
        return Err(corrupt());
    }
    let directory = rustix::fs::openat(parent, name, directory_open_flags(), Mode::empty())
        .map_err(|_| unavailable())?;
    let after = validate_private_directory(&directory)?;
    if before.st_dev != after.st_dev || before.st_ino != after.st_ino {
        return Err(unavailable());
    }
    if created {
        sync_directory(parent)?;
    }
    Ok(directory)
}

fn open_private_lock(root: BorrowedFd<'_>, name: &str) -> Result<OwnedFd, BackgroundStoreError> {
    open_or_create_private_file(root, name, true)
}

fn open_existing_private_file(
    root: BorrowedFd<'_>,
    name: &str,
) -> Result<OwnedFd, BackgroundStoreError> {
    open_existing_private_file_optional(root, name)?.ok_or_else(unavailable)
}

fn open_existing_private_file_optional(
    root: BorrowedFd<'_>,
    name: &str,
) -> Result<Option<OwnedFd>, BackgroundStoreError> {
    let file = rustix::fs::openat(
        root,
        name,
        OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    );
    let file = match file {
        Ok(file) => file,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(error) => return Err(map_entry_error(root, name, error)),
    };
    validate_private_file(&file)?;
    Ok(Some(file))
}

fn open_or_create_private_file(
    root: BorrowedFd<'_>,
    name: &str,
    sync_if_created: bool,
) -> Result<OwnedFd, BackgroundStoreError> {
    let created = rustix::fs::openat(
        root,
        name,
        OFlags::RDWR
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK,
        PRIVATE_FILE_MODE,
    );
    let (file, was_created) = match created {
        Ok(file) => (file, true),
        Err(error) if error == rustix::io::Errno::EXIST => {
            let file = rustix::fs::openat(
                root,
                name,
                OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|error| map_entry_error(root, name, error))?;
            (file, false)
        }
        Err(error) => return Err(map_entry_error(root, name, error)),
    };
    if was_created {
        rustix::fs::fchmod(&file, PRIVATE_FILE_MODE).map_err(|_| unavailable())?;
        if sync_if_created {
            sync_file(&file)?;
            sync_directory(root)?;
        }
    }
    validate_private_file(&file)?;
    Ok(file)
}

fn create_private_temp(root: BorrowedFd<'_>, name: &str) -> Result<OwnedFd, BackgroundStoreError> {
    match rustix::fs::openat(
        root,
        name,
        OFlags::WRONLY
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK,
        PRIVATE_FILE_MODE,
    ) {
        Ok(file) => {
            if let Err(failure) =
                rustix::fs::fchmod(&file, PRIVATE_FILE_MODE).map_err(|_| unavailable())
            {
                cleanup_temp(root, name);
                return Err(failure);
            }
            if let Err(failure) = validate_private_file(&file) {
                cleanup_temp(root, name);
                return Err(failure);
            }
            Ok(file)
        }
        Err(error) if error == rustix::io::Errno::EXIST => {
            let stale = rustix::fs::openat(
                root,
                name,
                OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|error| map_entry_error(root, name, error))?;
            validate_private_file(&stale)?;
            rustix::fs::unlinkat(root, name, AtFlags::empty()).map_err(|_| unavailable())?;
            let file = rustix::fs::openat(
                root,
                name,
                OFlags::WRONLY
                    | OFlags::CREATE
                    | OFlags::EXCL
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC
                    | OFlags::NONBLOCK,
                PRIVATE_FILE_MODE,
            )
            .map_err(|_| unavailable())?;
            if let Err(failure) =
                rustix::fs::fchmod(&file, PRIVATE_FILE_MODE).map_err(|_| unavailable())
            {
                cleanup_temp(root, name);
                return Err(failure);
            }
            if let Err(failure) = validate_private_file(&file) {
                cleanup_temp(root, name);
                return Err(failure);
            }
            Ok(file)
        }
        Err(error) => Err(map_entry_error(root, name, error)),
    }
}

fn publish_counter_initial(root: BorrowedFd<'_>) -> Result<(), BackgroundStoreError> {
    let temp = create_private_temp(root, ALLOCATOR_COUNTER_TEMP_NAME)?;
    if let Err(result) = write_and_sync(&temp, &0_u64.to_be_bytes()) {
        cleanup_temp(root, ALLOCATOR_COUNTER_TEMP_NAME);
        return Err(result);
    }
    if let Err(rename_error) = rustix::fs::renameat_with(
        root,
        ALLOCATOR_COUNTER_TEMP_NAME,
        root,
        ALLOCATOR_COUNTER_NAME,
        RenameFlags::NOREPLACE,
    ) {
        cleanup_temp(root, ALLOCATOR_COUNTER_TEMP_NAME);
        return Err(if rename_error == rustix::io::Errno::EXIST {
            corrupt()
        } else {
            unavailable()
        });
    }
    sync_directory(root)
}

fn replace_counter(root: BorrowedFd<'_>, value: u64) -> Result<(), BackgroundStoreError> {
    let temp = create_private_temp(root, ALLOCATOR_COUNTER_TEMP_NAME)?;
    if let Err(result) = write_and_sync(&temp, &value.to_be_bytes()) {
        cleanup_temp(root, ALLOCATOR_COUNTER_TEMP_NAME);
        return Err(result);
    }
    if rustix::fs::renameat(
        root,
        ALLOCATOR_COUNTER_TEMP_NAME,
        root,
        ALLOCATOR_COUNTER_NAME,
    )
    .is_err()
    {
        cleanup_temp(root, ALLOCATOR_COUNTER_TEMP_NAME);
        return Err(unavailable());
    }
    sync_directory(root)
}

fn read_counter(root: BorrowedFd<'_>) -> Result<u64, BackgroundStoreError> {
    read_counter_optional(root)?.ok_or_else(corrupt)
}

fn read_counter_optional(root: BorrowedFd<'_>) -> Result<Option<u64>, BackgroundStoreError> {
    let file = match rustix::fs::openat(
        root,
        ALLOCATOR_COUNTER_NAME,
        record_open_flags(),
        Mode::empty(),
    ) {
        Ok(file) => file,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(error) => return Err(map_entry_error(root, ALLOCATOR_COUNTER_NAME, error)),
    };
    let metadata = validate_private_file(&file)?;
    if metadata.st_size != COUNTER_BYTES_I64 {
        return Err(corrupt());
    }
    let mut bytes = [0_u8; COUNTER_BYTES];
    let mut reader = std::fs::File::from(file);
    reader.read_exact(&mut bytes).map_err(|_| corrupt())?;
    let mut overflow = [0_u8; 1];
    if reader.read(&mut overflow).map_err(|_| unavailable())? != 0 {
        return Err(corrupt());
    }
    Ok(Some(u64::from_be_bytes(bytes)))
}

fn validate_private_directory(file: &OwnedFd) -> Result<rustix::fs::Stat, BackgroundStoreError> {
    let metadata = rustix::fs::fstat(file).map_err(|_| unavailable())?;
    validate_private_directory_metadata(&metadata)?;
    #[cfg(target_os = "macos")]
    validate_acl(file)?;
    Ok(metadata)
}

fn validate_private_directory_metadata(
    metadata: &rustix::fs::Stat,
) -> Result<(), BackgroundStoreError> {
    if !FileType::from_raw_mode(metadata.st_mode).is_dir()
        || metadata.st_uid != rustix::process::geteuid().as_raw()
        || u64::from(metadata.st_mode) & GROUP_OR_OTHER_PERMISSIONS != 0
        || metadata.st_nlink == 0
    {
        return Err(corrupt());
    }
    Ok(())
}

fn validate_private_file(file: &OwnedFd) -> Result<rustix::fs::Stat, BackgroundStoreError> {
    let metadata = rustix::fs::fstat(file).map_err(|_| unavailable())?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file()
        || metadata.st_uid != rustix::process::geteuid().as_raw()
        || u64::from(metadata.st_mode) & GROUP_OR_OTHER_PERMISSIONS != 0
        || metadata.st_nlink == 0
    {
        return Err(corrupt());
    }
    #[cfg(target_os = "macos")]
    validate_acl(file)?;
    Ok(metadata)
}

#[cfg(target_os = "macos")]
fn validate_acl(file: &OwnedFd) -> Result<(), BackgroundStoreError> {
    let acl = calcifer_macos_acl::read_acl(file.as_fd()).map_err(|_| unavailable())?;
    if acl.flags != 0
        || acl.entries.iter().any(|entry| {
            entry.tag != calcifer_macos_acl::TAG_DENY
                || entry.flags != 0
                || entry.permissions != calcifer_macos_acl::PERMISSION_DELETE
        })
    {
        return Err(corrupt());
    }
    Ok(())
}

fn write_and_sync(file: &OwnedFd, bytes: &[u8]) -> Result<(), BackgroundStoreError> {
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match retry_interrupted(|| rustix::io::write(file, remaining)) {
            Ok(0) | Err(_) => return Err(unavailable()),
            Ok(written) => remaining = &remaining[written..],
        }
    }
    sync_file(file)
}

fn sync_file(file: impl AsFd) -> Result<(), BackgroundStoreError> {
    retry_interrupted(|| rustix::fs::fsync(file.as_fd())).map_err(|_| unavailable())
}

fn sync_directory(directory: BorrowedFd<'_>) -> Result<(), BackgroundStoreError> {
    retry_interrupted(|| rustix::fs::fsync(directory)).map_err(|_| unavailable())
}

fn acquire_exclusive(file: OwnedFd) -> Result<ExclusiveFileLock, BackgroundStoreError> {
    match retry_interrupted(|| rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive)) {
        Ok(()) => Ok(ExclusiveFileLock { file }),
        Err(error) if is_lock_contention(error) => Err(unavailable()),
        Err(_) => Err(unavailable()),
    }
}

fn try_acquire_exclusive(file: OwnedFd) -> Result<Option<ExclusiveFileLock>, BackgroundStoreError> {
    match retry_interrupted(|| rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive)) {
        Ok(()) => Ok(Some(ExclusiveFileLock { file })),
        Err(error) if is_lock_contention(error) => Ok(None),
        Err(_) => Err(unavailable()),
    }
}

fn retry_interrupted<T>(
    mut operation: impl FnMut() -> Result<T, rustix::io::Errno>,
) -> Result<T, rustix::io::Errno> {
    loop {
        match operation() {
            Err(error) if error == rustix::io::Errno::INTR => {}
            result => return result,
        }
    }
}

fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

fn record_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK | noatime_flag()
}

#[cfg(target_os = "linux")]
fn noatime_flag() -> OFlags {
    OFlags::from_bits_retain(libc::O_NOATIME as _)
}

#[cfg(target_os = "macos")]
const fn noatime_flag() -> OFlags {
    OFlags::empty()
}

fn cleanup_temp(root: BorrowedFd<'_>, name: &str) {
    let _ = rustix::fs::unlinkat(root, name, AtFlags::empty());
}

fn map_entry_error(
    root: BorrowedFd<'_>,
    name: &str,
    open_error: rustix::io::Errno,
) -> BackgroundStoreError {
    if is_rejected_type_error(open_error)
        || rustix::fs::statat(root, name, AtFlags::SYMLINK_NOFOLLOW)
            .is_ok_and(|metadata| !FileType::from_raw_mode(metadata.st_mode).is_file())
    {
        corrupt()
    } else {
        unavailable()
    }
}

fn is_rejected_type_error(error: rustix::io::Errno) -> bool {
    matches!(
        error,
        rustix::io::Errno::LOOP
            | rustix::io::Errno::ISDIR
            | rustix::io::Errno::NOTDIR
            | rustix::io::Errno::NXIO
            | rustix::io::Errno::OPNOTSUPP
    )
}

fn is_lock_contention(error: rustix::io::Errno) -> bool {
    error == rustix::io::Errno::AGAIN || error == rustix::io::Errno::WOULDBLOCK
}

const fn error(kind: BackgroundStoreErrorKind) -> BackgroundStoreError {
    BackgroundStoreError::new(kind)
}

const fn corrupt() -> BackgroundStoreError {
    error(BackgroundStoreErrorKind::Corrupt)
}

const fn unavailable() -> BackgroundStoreError {
    error(BackgroundStoreErrorKind::Unavailable)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use futures_executor::block_on;

    use super::*;
    use crate::{
        NativeBackgroundInspection, NativeBackgroundQuery, NativeEnvironment,
        inspect_native_background,
    };

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        state_base: PathBuf,
        state_root: PathBuf,
        workspace: String,
    }

    impl Fixture {
        fn new() -> Self {
            loop {
                let suffix = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let root = std::env::temp_dir().join(format!(
                    "machine-god-background-store-{}-{suffix}",
                    std::process::id()
                ));
                match fs::create_dir(&root) {
                    Ok(()) => {
                        private_directory(&root);
                        let root = fs::canonicalize(root).unwrap();
                        let state_base = root.join("state");
                        let state_root = state_base.join(crate::STATE_NAMESPACE);
                        let workspace_path = root.join("workspace");
                        fs::create_dir(&state_base).unwrap();
                        fs::create_dir(&state_root).unwrap();
                        fs::create_dir(&workspace_path).unwrap();
                        private_directory(&state_base);
                        private_directory(&state_root);
                        private_directory(&workspace_path);
                        let workspace = fs::canonicalize(workspace_path)
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_owned();
                        return Self {
                            root,
                            state_base,
                            state_root,
                            workspace,
                        };
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create fixture: {error}"),
                }
            }
        }

        fn store(&self) -> BackgroundStore {
            let descriptor =
                rustix::fs::open(&self.state_root, directory_open_flags(), Mode::empty()).unwrap();
            BackgroundStore::prepare(descriptor, self.workspace.clone()).unwrap()
        }

        fn environment(&self) -> NativeEnvironment {
            NativeEnvironment::new(None, Some(self.state_base.clone().into_os_string()), None)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn private_directory(path: &Path) {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn read_only_state_root_validation_rejects_a_wrong_owner_when_privileged() {
        if rustix::process::geteuid().as_raw() != 0 {
            return;
        }
        let fixture = Fixture::new();
        let descriptor =
            rustix::fs::open(&fixture.state_root, directory_open_flags(), Mode::empty()).unwrap();
        let status = Command::new("chown")
            .arg("1")
            .arg(&fixture.state_root)
            .status()
            .expect("chown executable is available");
        assert!(status.success(), "failed to install wrong-owner fixture");

        let error = BackgroundStore::validate_state_root(&descriptor).unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::Corrupt);
    }

    #[test]
    fn private_directory_metadata_rejects_a_wrong_owner() {
        let fixture = Fixture::new();
        let descriptor =
            rustix::fs::open(&fixture.state_root, directory_open_flags(), Mode::empty()).unwrap();
        let mut metadata = rustix::fs::fstat(&descriptor).unwrap();
        metadata.st_uid = metadata.st_uid.wrapping_add(1);

        let error = validate_private_directory_metadata(&metadata).unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::Corrupt);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn read_only_state_root_validation_rejects_an_access_granting_acl() {
        let fixture = Fixture::new();
        let status = Command::new("/bin/chmod")
            .args(["+a", "everyone allow search"])
            .arg(&fixture.state_root)
            .status()
            .expect("macOS chmod executable is available");
        assert!(status.success(), "failed to install ACL fixture: {status}");
        let descriptor =
            rustix::fs::open(&fixture.state_root, directory_open_flags(), Mode::empty()).unwrap();

        let error = BackgroundStore::validate_state_root(&descriptor).unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::Corrupt);
    }

    fn running(store: &BackgroundStore, id: u64, updated_at_ms: u64) -> StoredBackgroundRecord {
        StoredBackgroundRecord {
            version: 1,
            workspace: store.workspace().to_owned(),
            id,
            started_at_ms: 10,
            updated_at_ms,
            command: "cargo test --workspace".to_owned(),
            cwd: store.workspace().to_owned(),
            state: NativeBackgroundState::Running,
            pid: Some(std::process::id()),
            exit_code: None,
            server_url: None,
            diagnostic: None,
        }
    }

    fn reserve_eventually(store: &BackgroundStore) -> BackgroundRecordLease {
        store.reserve_id().expect("reserve background test ID")
    }

    fn reconcile_eventually(
        store: &BackgroundStore,
        expected_active: usize,
        expected_marked_stale: usize,
    ) -> BackgroundReconciliation {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let result = store.reconcile().unwrap();
            if (result.active == expected_active && result.marked_stale == expected_marked_stale)
                || Instant::now() >= deadline
            {
                return result;
            }
            std::thread::yield_now();
        }
    }

    #[test]
    fn writer_round_trips_through_strict_reader_and_replaces_atomically() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let lease = reserve_eventually(&store);
        assert_eq!(lease.id(), 1);
        let record = running(&store, lease.id(), 10);
        store.publish_initial(&lease, &record).unwrap();

        let NativeBackgroundInspection::Detail(detail) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(lease.id()),
        ))
        .unwrap() else {
            panic!("expected detail");
        };
        assert_eq!(detail.id(), lease.id());
        assert_eq!(detail.state(), NativeBackgroundState::Running);
        assert_eq!(detail.command(), record.command);

        let mut exited = record.clone();
        exited.updated_at_ms = 20;
        exited.state = NativeBackgroundState::Exited;
        exited.exit_code = Some(0);
        let mut substituted_pid = exited.clone();
        substituted_pid.pid = Some(if record.pid == Some(1) { 2 } else { 1 });
        assert_eq!(
            store.replace(&lease, &substituted_pid).unwrap_err().kind(),
            BackgroundStoreErrorKind::Conflict
        );
        store.replace(&lease, &exited).unwrap();
        let NativeBackgroundInspection::Detail(detail) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(lease.id()),
        ))
        .unwrap() else {
            panic!("expected detail");
        };
        assert_eq!(detail.state(), NativeBackgroundState::Exited);
        assert_eq!(detail.exit_code(), Some(0));
    }

    #[test]
    #[allow(clippy::used_underscore_binding)]
    fn lease_release_unlocks_an_inherited_open_file_description() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let lease = reserve_eventually(&store);
        let id = lease.id();
        let inherited = rustix::io::dup(lease._lock.as_fd()).unwrap();

        drop(lease);

        let reopened = open_private_lock(store.control.as_fd(), &record_lock_name(id)).unwrap();
        let authority = acquire_exclusive(reopened).unwrap();
        drop(authority);
        drop(inherited);
    }

    #[test]
    fn identifiers_are_monotonic_across_store_instances_and_no_clobber_is_enforced() {
        let fixture = Fixture::new();
        let first_store = fixture.store();
        let first = reserve_eventually(&first_store);
        first_store
            .publish_initial(&first, &running(&first_store, first.id(), 10))
            .unwrap();
        drop(first);

        let second_store = fixture.store();
        let second = reserve_eventually(&second_store);
        assert_eq!(second.id(), 2);
        let mut wrong_id = running(&second_store, second.id(), 10);
        wrong_id.id += 1;
        let error = second_store
            .publish_initial(&second, &wrong_id)
            .unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::Corrupt);

        let record = running(&second_store, second.id(), 10);
        second_store.publish_initial(&second, &record).unwrap();
        let error = second_store.publish_initial(&second, &record).unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::Conflict);
    }

    #[test]
    fn identifiers_are_monotonic_across_processes() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let first = reserve_eventually(&store);
        assert_eq!(first.id(), 1);
        drop(first);

        let child_result = fixture.root.join("child-id");
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("background_store::tests::cross_process_reserve_helper")
            .arg("--test-threads=1")
            .env("MACHINE_GOD_BG_STORE_CHILD_STATE", &fixture.state_root)
            .env("MACHINE_GOD_BG_STORE_CHILD_WORKSPACE", &fixture.workspace)
            .env("MACHINE_GOD_BG_STORE_CHILD_RESULT", &child_result)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(fs::read_to_string(child_result).unwrap(), "2");

        let third = reserve_eventually(&store);
        assert_eq!(third.id(), 3);
    }

    #[test]
    fn cross_process_reserve_helper() {
        let Some(state_root) = std::env::var_os("MACHINE_GOD_BG_STORE_CHILD_STATE") else {
            return;
        };
        let workspace = std::env::var("MACHINE_GOD_BG_STORE_CHILD_WORKSPACE").unwrap();
        let result = std::env::var_os("MACHINE_GOD_BG_STORE_CHILD_RESULT").unwrap();
        let descriptor = rustix::fs::open(
            PathBuf::from(state_root),
            directory_open_flags(),
            Mode::empty(),
        )
        .unwrap();
        let store = BackgroundStore::prepare(descriptor, workspace).unwrap();
        let lease = reserve_eventually(&store);
        fs::write(result, lease.id().to_string()).unwrap();
    }

    #[test]
    fn reconciliation_marks_only_unlocked_running_records_stale() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let active = reserve_eventually(&store);
        store
            .publish_initial(&active, &running(&store, active.id(), 10))
            .unwrap();
        let abandoned = reserve_eventually(&store);
        let abandoned_id = abandoned.id();
        store
            .publish_initial(&abandoned, &running(&store, abandoned_id, 20))
            .unwrap();
        drop(abandoned);

        let result = reconcile_eventually(&store, 1, 1);
        assert_eq!(result.inspected, 2);
        assert_eq!(result.active, 1);
        assert_eq!(result.marked_stale, 1);

        let NativeBackgroundInspection::Detail(detail) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(abandoned_id),
        ))
        .unwrap() else {
            panic!("expected detail");
        };
        assert_eq!(detail.state(), NativeBackgroundState::Stale);
    }

    #[test]
    fn reconciliation_fails_closed_before_mutation_on_corrupt_scan() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let lease = reserve_eventually(&store);
        let id = lease.id();
        store
            .publish_initial(&lease, &running(&store, id, 10))
            .unwrap();
        drop(lease);

        let corrupt_id = reserve_eventually(&store).id();
        let root = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace));
        let corrupt_path = root.join(background_record_name(corrupt_id));
        fs::write(&corrupt_path, b"{").unwrap();
        fs::set_permissions(&corrupt_path, fs::Permissions::from_mode(0o600)).unwrap();
        let error = store.reconcile().unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::Corrupt);

        let NativeBackgroundInspection::Detail(detail) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(id),
        ))
        .unwrap() else {
            panic!("expected detail");
        };
        assert_eq!(detail.state(), NativeBackgroundState::Running);
    }

    #[test]
    fn publication_rejects_serialized_overflow_without_exposing_a_record() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let lease = reserve_eventually(&store);
        let mut record = running(&store, lease.id(), 10);
        record.command = "\u{1}".repeat(crate::MAX_BACKGROUND_COMMAND_BYTES);
        let error = store.publish_initial(&lease, &record).unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::ResourceLimit);

        let inspection = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(lease.id()),
        ))
        .unwrap_err();
        assert_eq!(
            inspection.kind(),
            crate::NativeBackgroundInspectionErrorKind::NotFound
        );
    }

    #[test]
    fn corrupt_and_exhausted_counters_fail_closed() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let workspace_root = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace))
            .join(CONTROL_DIRECTORY);
        let counter = workspace_root.join(ALLOCATOR_COUNTER_NAME);
        fs::write(&counter, b"short").unwrap();
        fs::set_permissions(&counter, fs::Permissions::from_mode(0o600)).unwrap();
        let error = store.reserve_id().unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::Corrupt);

        fs::write(&counter, u64::MAX.to_be_bytes()).unwrap();
        let error = store.reserve_id().unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::IdExhausted);
    }

    #[test]
    fn bounded_control_scan_fails_before_reconciliation_mutates_records() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let lease = reserve_eventually(&store);
        let id = lease.id();
        store
            .publish_initial(&lease, &running(&store, id, 10))
            .unwrap();
        drop(lease);

        let control = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace))
            .join(CONTROL_DIRECTORY);
        for index in 0..=MAX_BACKGROUND_DIRECTORY_ENTRIES {
            fs::write(control.join(format!("unrelated-{index}")), b"").unwrap();
        }

        let error = store.reconcile().unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::ResourceLimit);
        let NativeBackgroundInspection::Detail(detail) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(id),
        ))
        .unwrap() else {
            panic!("expected detail");
        };
        assert_eq!(detail.state(), NativeBackgroundState::Running);
    }

    #[test]
    fn completed_history_beyond_listing_limit_reopens_and_reconciles() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let completed_count = crate::MAX_BACKGROUND_RECORDS + 1;
        let workspace_root = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace));
        let control = workspace_root.join(CONTROL_DIRECTORY);
        seed_completed_history(&store, &workspace_root, &control, completed_count);
        assert_eq!(count_record_files(&workspace_root), completed_count);
        assert_eq!(count_record_locks(&control), completed_count);
        drop(store);

        let reopened = fixture.store();
        let reconciliation = reopened.reconcile().unwrap();
        assert_eq!(reconciliation.inspected, MAX_RETAINED_BACKGROUND_RECORDS);
        assert_eq!(reconciliation.active, 0);
        assert_eq!(reconciliation.marked_stale, 0);
        assert_eq!(
            count_record_files(&workspace_root),
            MAX_RETAINED_BACKGROUND_RECORDS
        );
        assert_eq!(
            count_record_locks(&control),
            MAX_RETAINED_BACKGROUND_RECORDS
        );
        let oldest = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(1),
        ))
        .unwrap_err();
        assert_eq!(
            oldest.kind(),
            crate::NativeBackgroundInspectionErrorKind::NotFound
        );
        let NativeBackgroundInspection::Detail(newest) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(completed_count as u64),
        ))
        .unwrap() else {
            panic!("expected detail");
        };
        assert_eq!(newest.state(), NativeBackgroundState::Exited);

        let active = reserve_eventually(&reopened);
        let active_id = active.id();
        assert_eq!(active_id, completed_count as u64 + 1);
        reopened
            .publish_initial(&active, &running(&reopened, active_id, 1_000))
            .unwrap();
        let NativeBackgroundInspection::List(list) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::List,
        ))
        .unwrap() else {
            panic!("expected list");
        };
        assert!(!list.truncated());
        assert_eq!(list.records().len(), crate::MAX_BACKGROUND_RECORDS);
        let NativeBackgroundInspection::Detail(detail) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(active_id),
        ))
        .unwrap() else {
            panic!("expected detail");
        };
        assert_eq!(detail.state(), NativeBackgroundState::Running);

        drop(reopened);
        let reopened = fixture.store();
        let reconciliation = reopened.reconcile().unwrap();
        assert_eq!(reconciliation.active, 1);
        assert_eq!(reconciliation.marked_stale, 0);
        drop(active);
        let reconciliation = reconcile_eventually(&reopened, 0, 1);
        assert_eq!(reconciliation.marked_stale, 1);
        let continued = reserve_eventually(&reopened);
        assert_eq!(continued.id(), active_id + 1);
        assert_eq!(
            count_record_files(&workspace_root),
            MAX_RETAINED_BACKGROUND_RECORDS - 1
        );
        assert_eq!(
            count_record_locks(&control),
            MAX_RETAINED_BACKGROUND_RECORDS
        );
    }

    fn seed_completed_history(
        store: &BackgroundStore,
        workspace_root: &Path,
        control: &Path,
        count: usize,
    ) {
        for id in 1..=count as u64 {
            let mut record = running(store, id, 10 + id);
            record.updated_at_ms += 1;
            record.state = NativeBackgroundState::Exited;
            record.exit_code = Some(0);
            let data = workspace_root.join(background_record_name(id));
            fs::write(&data, serde_json::to_vec(&record).unwrap()).unwrap();
            fs::set_permissions(&data, fs::Permissions::from_mode(0o600)).unwrap();
            let lock = control.join(record_lock_name(id));
            fs::write(&lock, b"").unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }
        fs::write(
            control.join(ALLOCATOR_COUNTER_NAME),
            (count as u64).to_be_bytes(),
        )
        .unwrap();
    }

    fn seed_running_history(
        store: &BackgroundStore,
        workspace_root: &Path,
        control: &Path,
        count: usize,
    ) {
        for id in 1..=count as u64 {
            let record = running(store, id, 10 + id);
            let data = workspace_root.join(background_record_name(id));
            fs::write(&data, serde_json::to_vec(&record).unwrap()).unwrap();
            fs::set_permissions(&data, fs::Permissions::from_mode(0o600)).unwrap();
            let lock = control.join(record_lock_name(id));
            fs::write(&lock, b"").unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }
        fs::write(
            control.join(ALLOCATOR_COUNTER_NAME),
            (count as u64).to_be_bytes(),
        )
        .unwrap();
    }

    fn count_record_files(root: &Path) -> usize {
        fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| is_background_record_name(entry.file_name().as_encoded_bytes()))
            .count()
    }

    fn count_record_locks(control: &Path) -> usize {
        fs::read_dir(control)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| is_background_record_lock_name(entry.file_name().as_encoded_bytes()))
            .count()
    }

    #[test]
    fn allocator_and_record_lock_contention_return_promptly() {
        let fixture = Fixture::new();
        let store = Arc::new(fixture.store());
        let control = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace))
            .join(CONTROL_DIRECTORY);
        let descriptor = rustix::fs::open(&control, directory_open_flags(), Mode::empty()).unwrap();
        let holder =
            acquire_exclusive(open_private_lock(descriptor.as_fd(), ALLOCATOR_LOCK_NAME).unwrap())
                .unwrap();

        let state_root = fixture.state_root.clone();
        let workspace = fixture.workspace.clone();
        let (sender, receiver) = mpsc::channel();
        let open_thread = std::thread::spawn(move || {
            let descriptor =
                rustix::fs::open(state_root, directory_open_flags(), Mode::empty()).unwrap();
            sender
                .send(BackgroundStore::prepare(descriptor, workspace).map(|_| ()))
                .unwrap();
        });
        let open_result = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("contended store preparation must return promptly");
        assert_eq!(
            open_result.unwrap_err().kind(),
            BackgroundStoreErrorKind::Unavailable
        );
        open_thread.join().unwrap();

        let (sender, receiver) = mpsc::channel();
        let contender = Arc::clone(&store);
        let reserve_thread = std::thread::spawn(move || {
            sender
                .send(contender.reserve_id().map(|lease| lease.id()))
                .unwrap();
        });
        let result = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("nonblocking allocator contention must return promptly");
        assert_eq!(
            result.unwrap_err().kind(),
            BackgroundStoreErrorKind::Unavailable
        );
        drop(holder);
        reserve_thread.join().unwrap();

        let record_holder =
            acquire_exclusive(open_private_lock(descriptor.as_fd(), &record_lock_name(1)).unwrap())
                .unwrap();
        let (sender, receiver) = mpsc::channel();
        let contender = Arc::clone(&store);
        let record_thread = std::thread::spawn(move || {
            sender
                .send(contender.reserve_id().map(|lease| lease.id()))
                .unwrap();
        });
        let record_result = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("contended per-record lock must return promptly");
        assert_eq!(
            record_result.unwrap_err().kind(),
            BackgroundStoreErrorKind::Unavailable
        );
        drop(record_holder);
        record_thread.join().unwrap();
    }

    #[test]
    fn running_records_consume_total_capacity_without_reader_overflow() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let workspace_root = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace));
        let control = workspace_root.join(CONTROL_DIRECTORY);
        seed_running_history(
            &store,
            &workspace_root,
            &control,
            MAX_RETAINED_BACKGROUND_RECORDS,
        );

        let error = store.reserve_id().unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::ResourceLimit);
        let NativeBackgroundInspection::List(list) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::List,
        ))
        .unwrap() else {
            panic!("expected list");
        };
        assert!(!list.truncated());
        assert_eq!(list.records().len(), crate::MAX_BACKGROUND_RECORDS);

        let reconciliation = store.reconcile().unwrap();
        assert_eq!(reconciliation.active, 0);
        assert_eq!(reconciliation.marked_stale, MAX_RETAINED_BACKGROUND_RECORDS);
        let continued = reserve_eventually(&store);
        assert_eq!(continued.id(), MAX_RETAINED_BACKGROUND_RECORDS as u64 + 1);
    }

    #[test]
    fn insufficient_compaction_capacity_preserves_the_entire_namespace() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let workspace_root = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace));
        let control = workspace_root.join(CONTROL_DIRECTORY);
        seed_running_history(
            &store,
            &workspace_root,
            &control,
            MAX_RETAINED_BACKGROUND_RECORDS + 2,
        );
        for id in 1..=2 {
            let mut terminal = running(&store, id, 10 + id);
            terminal.state = NativeBackgroundState::Exited;
            terminal.exit_code = Some(0);
            fs::write(
                workspace_root.join(background_record_name(id)),
                serde_json::to_vec(&terminal).unwrap(),
            )
            .unwrap();
        }

        let control_descriptor =
            rustix::fs::open(&control, directory_open_flags(), Mode::empty()).unwrap();
        let _terminal_holder = acquire_exclusive(
            open_existing_private_file(control_descriptor.as_fd(), &record_lock_name(2)).unwrap(),
        )
        .unwrap();
        let _allocator = acquire_exclusive(
            open_existing_private_file(control_descriptor.as_fd(), ALLOCATOR_LOCK_NAME).unwrap(),
        )
        .unwrap();
        let records_before = record_file_names(&workspace_root);
        let control_before = control_lifecycle_names(&control);

        let error = store
            .compact_history_locked(
                MAX_RETAINED_BACKGROUND_RECORDS,
                (MAX_RETAINED_BACKGROUND_RECORDS + 2) as u64,
            )
            .unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::ResourceLimit);
        assert_eq!(record_file_names(&workspace_root), records_before);
        assert_eq!(control_lifecycle_names(&control), control_before);
    }

    #[test]
    fn reconciliation_reclaims_orphan_temps_but_preserves_an_owned_temp() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let workspace_root = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace));
        let control = workspace_root.join(CONTROL_DIRECTORY);

        let active = reserve_eventually(&store);
        let active_names = record_names(active.id());
        fs::write(control.join(&active_names.temp), b"complete-temp").unwrap();
        fs::set_permissions(
            control.join(&active_names.temp),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let no_lock_temp = record_names(10_000).temp;
        fs::write(control.join(&no_lock_temp), b"orphan").unwrap();
        fs::set_permissions(
            control.join(&no_lock_temp),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();

        let result = store.reconcile().unwrap();
        assert_eq!(result.inspected, 0);
        assert!(control.join(&active_names.temp).exists());
        assert!(control.join(record_lock_name(active.id())).exists());
        assert!(!control.join(&no_lock_temp).exists());

        drop(active);
        store.reconcile().unwrap();
        assert!(!control.join(&active_names.temp).exists());
        assert!(!control.join(record_lock_name(1)).exists());
        assert_eq!(count_record_files(&workspace_root), 0);
    }

    #[test]
    fn canonical_temp_scan_has_an_aggregate_byte_bound_and_is_nonmutating() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let control = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace))
            .join(CONTROL_DIRECTORY);
        let count = MAX_BACKGROUND_TOTAL_RECORD_BYTES / MAX_BACKGROUND_RECORD_BYTES + 1;
        let bytes = vec![b'x'; MAX_BACKGROUND_RECORD_BYTES];
        for id in 1..=count as u64 {
            let temp = control.join(record_names(id).temp);
            fs::write(&temp, &bytes).unwrap();
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let before = control_lifecycle_names(&control);

        let error = store.reconcile().unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::ResourceLimit);
        assert_eq!(control_lifecycle_names(&control), before);
    }

    #[test]
    fn reconciliation_serializes_completion_and_compaction_without_orphaning_a_lock() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let workspace_root = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace));
        let control = workspace_root.join(CONTROL_DIRECTORY);
        seed_completed_history(
            &store,
            &workspace_root,
            &control,
            MAX_RETAINED_BACKGROUND_RECORDS - 1,
        );
        let target = reserve_eventually(&store);
        let target_id = target.id();
        let initial = running(&store, target_id, 10);
        store.publish_initial(&target, &initial).unwrap();
        let mut completed = initial;
        completed.state = NativeBackgroundState::Exited;
        completed.exit_code = Some(0);
        let contender = fixture.store();

        let result = store
            .reconcile_inner(|| {
                store.replace(&target, &completed).unwrap();
                drop(target);
                let error = contender.reserve_id().unwrap_err();
                assert_eq!(error.kind(), BackgroundStoreErrorKind::Unavailable);
            })
            .unwrap();
        assert_eq!(result.inspected, MAX_RETAINED_BACKGROUND_RECORDS);
        assert_eq!(result.active, 0);
        assert_eq!(result.marked_stale, 0);
        assert!(
            workspace_root
                .join(background_record_name(target_id))
                .exists()
        );
        assert!(control.join(record_lock_name(target_id)).exists());

        let next = reserve_eventually(&store);
        assert_eq!(next.id(), target_id + 1);
        assert!(
            !workspace_root
                .join(background_record_name(target_id))
                .exists()
        );
        assert!(!control.join(record_lock_name(target_id)).exists());
    }

    #[test]
    fn compaction_rejects_publication_between_record_and_control_snapshots() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let workspace_root = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace));
        let control = workspace_root.join(CONTROL_DIRECTORY);
        let lease = reserve_eventually(&store);
        let id = lease.id();
        let initial = running(&store, id, 10);
        let publisher = &store;

        let error = store
            .compact_history_locked_inner(MAX_RETAINED_BACKGROUND_RECORDS, id, move || {
                publisher.publish_initial(&lease, &initial).unwrap();
                drop(lease);
            })
            .unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::Unavailable);
        assert!(workspace_root.join(background_record_name(id)).exists());
        assert!(control.join(record_lock_name(id)).exists());

        let result = store.reconcile().unwrap();
        assert_eq!(result.inspected, 1);
        assert_eq!(result.marked_stale, 1);
        assert!(control.join(record_lock_name(id)).exists());
    }

    #[test]
    fn maintenance_reclaims_many_orphans_at_supported_fd_floor() {
        const ORPHAN_COUNT: u64 = 160;
        const SUPPORTED_NOFILE_FLOOR: u64 = 24;

        let fixture = Fixture::new();
        let store = fixture.store();
        let control = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace))
            .join(CONTROL_DIRECTORY);
        drop(store);
        for id in 1..=ORPHAN_COUNT {
            let lock = control.join(record_lock_name(id));
            fs::write(&lock, b"").unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
            let temp = control.join(record_names(id).temp);
            fs::write(&temp, b"orphan").unwrap();
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let result = fixture.root.join("reduced-fd-result");
        let command = format!(
            "ulimit -n {SUPPORTED_NOFILE_FLOOR} && exec \"$1\" --exact \
             background_store::tests::reduced_fd_maintenance_helper --test-threads=1"
        );
        let status = Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .arg("machine-god-background-store-rlimit")
            .arg(std::env::current_exe().unwrap())
            .env("MACHINE_GOD_BG_STORE_RLIMIT_STATE", &fixture.state_root)
            .env("MACHINE_GOD_BG_STORE_RLIMIT_WORKSPACE", &fixture.workspace)
            .env("MACHINE_GOD_BG_STORE_RLIMIT_RESULT", &result)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(fs::read_to_string(result).unwrap(), "0:0");
        assert_eq!(count_record_locks(&control), 0);
        assert!(
            control_lifecycle_names(&control)
                .iter()
                .all(|name| !is_background_record_temp_name(name.as_encoded_bytes()))
        );
    }

    #[test]
    fn reduced_fd_maintenance_helper() {
        let Some(state_root) = std::env::var_os("MACHINE_GOD_BG_STORE_RLIMIT_STATE") else {
            return;
        };
        let workspace = std::env::var("MACHINE_GOD_BG_STORE_RLIMIT_WORKSPACE").unwrap();
        let result = std::env::var_os("MACHINE_GOD_BG_STORE_RLIMIT_RESULT").unwrap();
        let descriptor = rustix::fs::open(
            PathBuf::from(state_root),
            directory_open_flags(),
            Mode::empty(),
        )
        .unwrap();
        let store = BackgroundStore::prepare(descriptor, workspace).unwrap();
        store.reconcile().unwrap();
        let control = scan_control_snapshot(store.control.as_fd()).unwrap();
        fs::write(
            result,
            format!("{}:{}", control.lock_names.len(), control.temp_names.len()),
        )
        .unwrap();
    }

    #[test]
    fn post_rename_sync_failure_leaves_only_a_complete_record_that_can_be_closed() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let lease = reserve_eventually(&store);
        let initial = running(&store, lease.id(), 10);
        FAIL_NEXT_PUBLISHED_ROOT_SYNC.with(|failure| failure.set(true));

        let error = store.publish_initial(&lease, &initial).unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::Unavailable);
        let NativeBackgroundInspection::Detail(detail) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(lease.id()),
        ))
        .unwrap() else {
            panic!("expected complete running detail");
        };
        assert_eq!(detail.state(), NativeBackgroundState::Running);

        let mut dead = initial;
        dead.updated_at_ms = 20;
        dead.state = NativeBackgroundState::Dead;
        store.replace(&lease, &dead).unwrap();
        drop(lease);
        let result = store.reconcile().unwrap();
        assert_eq!(result.inspected, 1);
        assert_eq!(result.marked_stale, 0);
        let NativeBackgroundInspection::Detail(detail) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::Id(dead.id),
        ))
        .unwrap() else {
            panic!("expected complete dead detail");
        };
        assert_eq!(detail.state(), NativeBackgroundState::Dead);
    }

    #[test]
    fn compaction_corrupt_preflight_never_deletes_valid_history() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let workspace_root = fixture
            .state_root
            .join(BACKGROUND_DIRECTORY)
            .join(background_workspace_name(&fixture.workspace));
        let control = workspace_root.join(CONTROL_DIRECTORY);
        seed_completed_history(
            &store,
            &workspace_root,
            &control,
            MAX_RETAINED_BACKGROUND_RECORDS + 1,
        );
        let corrupt = workspace_root.join(background_record_name(10_000));
        fs::write(&corrupt, b"{").unwrap();
        fs::set_permissions(&corrupt, fs::Permissions::from_mode(0o600)).unwrap();
        let before = record_file_names(&workspace_root);
        drop(store);

        let descriptor =
            rustix::fs::open(&fixture.state_root, directory_open_flags(), Mode::empty()).unwrap();
        let error = BackgroundStore::prepare(descriptor, fixture.workspace.clone()).unwrap_err();
        assert_eq!(error.kind(), BackgroundStoreErrorKind::Corrupt);
        assert_eq!(record_file_names(&workspace_root), before);
    }

    fn record_file_names(root: &Path) -> Vec<std::ffi::OsString> {
        let mut names: Vec<_> = fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| is_background_record_name(name.as_encoded_bytes()))
            .collect();
        names.sort_unstable();
        names
    }

    fn control_lifecycle_names(root: &Path) -> Vec<std::ffi::OsString> {
        let mut names: Vec<_> = fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| {
                is_background_record_lock_name(name.as_encoded_bytes())
                    || is_background_record_temp_name(name.as_encoded_bytes())
            })
            .collect();
        names.sort_unstable();
        names
    }
}
