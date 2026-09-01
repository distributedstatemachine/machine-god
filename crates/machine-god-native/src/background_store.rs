//! Descriptor-confined persistence used by the native background supervisor.

#![cfg(any(target_os = "linux", target_os = "macos"))]
#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "wired into the concurrently implemented native background supervisor"
    )
)]

use std::error::Error;
use std::fmt;
use std::io::{Read, Write};

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{AtFlags, Dir, FileType, FlockOperation, Mode, OFlags, RenameFlags};

use crate::background_inspection::{
    BACKGROUND_DIRECTORY, MAX_BACKGROUND_DIRECTORY_ENTRIES, MAX_BACKGROUND_PATH_BYTES,
    MAX_BACKGROUND_RECORD_BYTES, MAX_BACKGROUND_RECORDS, MAX_BACKGROUND_TOTAL_RECORD_BYTES,
    NativeBackgroundState, StoredBackgroundRecord, background_record_name,
    background_workspace_name, is_background_record_name, is_canonical_absolute_background_path,
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
/// Dropping the lease releases the authority. The lock entry itself is
/// permanent and is never unlinked or replaced by the store.
pub(crate) struct BackgroundRecordLease {
    id: u64,
    root_device: i128,
    root_inode: u128,
    control_device: i128,
    control_inode: u128,
    _lock: OwnedFd,
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
        validate_private_directory(&state_root)?;
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

    pub(crate) fn workspace(&self) -> &str {
        &self.workspace
    }

    /// Durably advances the global workspace counter, then acquires and
    /// returns the permanent lock authority for the reserved nonzero ID.
    pub(crate) fn reserve_id(&self) -> Result<BackgroundRecordLease, BackgroundStoreError> {
        self.validate_root()?;
        let allocator = open_private_lock(self.control.as_fd(), ALLOCATOR_LOCK_NAME)?;
        lock_exclusive(&allocator)?;
        let current = read_counter(self.control.as_fd())?;
        let id = current
            .checked_add(1)
            .ok_or_else(|| error(BackgroundStoreErrorKind::IdExhausted))?;
        replace_counter(self.control.as_fd(), id)?;

        let lock = open_private_lock(self.control.as_fd(), &record_lock_name(id))?;
        lock_exclusive(&lock)?;
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
        self.validate_root()?;
        let records = scan_complete_records(self.root.as_fd(), &self.workspace)?;
        let mut result = BackgroundReconciliation {
            inspected: records.len(),
            active: 0,
            marked_stale: 0,
        };

        for observed in records {
            if observed.state != NativeBackgroundState::Running {
                continue;
            }
            let lock = open_private_lock(self.control.as_fd(), &record_lock_name(observed.id))?;
            match retry_interrupted(|| {
                rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
            }) {
                Ok(()) => {}
                Err(error) if is_lock_contention(error) => {
                    result.active += 1;
                    continue;
                }
                Err(_) => return Err(unavailable()),
            }

            // The preflight snapshot proves global completeness. Re-read under
            // this record's authority so a just-finished owner is never
            // overwritten with the earlier running value.
            let names = record_names(observed.id);
            let Some((current, _)) = read_record(self.root.as_fd(), &self.workspace, &names.data)?
            else {
                return Err(corrupt());
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
        let allocator = open_private_lock(self.control.as_fd(), ALLOCATOR_LOCK_NAME)?;
        lock_exclusive(&allocator)?;
        match read_counter_optional(self.control.as_fd())? {
            Some(_) => Ok(()),
            None => publish_counter_initial(self.control.as_fd()),
        }
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

fn valid_replacement(
    current: &StoredBackgroundRecord,
    replacement: &StoredBackgroundRecord,
) -> bool {
    current.id == replacement.id
        && current.workspace == replacement.workspace
        && current.started_at_ms == replacement.started_at_ms
        && current.command == replacement.command
        && current.cwd == replacement.cwd
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
    if names.len() > MAX_BACKGROUND_RECORDS {
        return Err(error(BackgroundStoreErrorKind::ResourceLimit));
    }

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
    sync_directory(root)?;
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
    sync_directory(root)?;
    sync_directory(control)
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
    if !FileType::from_raw_mode(metadata.st_mode).is_dir()
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

fn lock_exclusive(file: &OwnedFd) -> Result<(), BackgroundStoreError> {
    retry_interrupted(|| rustix::fs::flock(file, FlockOperation::LockExclusive))
        .map_err(|_| unavailable())
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
    use std::sync::atomic::{AtomicU64, Ordering};

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

    #[test]
    fn writer_round_trips_through_strict_reader_and_replaces_atomically() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let lease = store.reserve_id().unwrap();
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
    fn identifiers_are_monotonic_across_store_instances_and_no_clobber_is_enforced() {
        let fixture = Fixture::new();
        let first_store = fixture.store();
        let first = first_store.reserve_id().unwrap();
        first_store
            .publish_initial(&first, &running(&first_store, first.id(), 10))
            .unwrap();
        drop(first);

        let second_store = fixture.store();
        let second = second_store.reserve_id().unwrap();
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
        let first = store.reserve_id().unwrap();
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

        let third = store.reserve_id().unwrap();
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
        let lease = store.reserve_id().unwrap();
        fs::write(result, lease.id().to_string()).unwrap();
    }

    #[test]
    fn reconciliation_marks_only_unlocked_running_records_stale() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let active = store.reserve_id().unwrap();
        store
            .publish_initial(&active, &running(&store, active.id(), 10))
            .unwrap();
        let abandoned = store.reserve_id().unwrap();
        let abandoned_id = abandoned.id();
        store
            .publish_initial(&abandoned, &running(&store, abandoned_id, 20))
            .unwrap();
        drop(abandoned);

        let result = store.reconcile().unwrap();
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
        let lease = store.reserve_id().unwrap();
        let id = lease.id();
        store
            .publish_initial(&lease, &running(&store, id, 10))
            .unwrap();
        drop(lease);

        let corrupt_id = store.reserve_id().unwrap().id();
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
        let lease = store.reserve_id().unwrap();
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
    fn permanent_control_entries_do_not_consume_the_record_scan_budget() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let lease = store.reserve_id().unwrap();
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

        let result = store.reconcile().unwrap();
        assert_eq!(result.inspected, 1);
        assert_eq!(result.marked_stale, 1);
        let NativeBackgroundInspection::List(list) = block_on(inspect_native_background(
            fixture.environment(),
            PathBuf::from(&fixture.workspace),
            NativeBackgroundQuery::List,
        ))
        .unwrap() else {
            panic!("expected list");
        };
        assert!(!list.truncated());
        assert_eq!(list.records().len(), 1);
    }
}
