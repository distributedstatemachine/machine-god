use std::error::Error;
use std::fmt;
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::BTreeMap;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::io::{self, Read, Write};

use machine_god_core::{
    BoxFuture, ContentBlock, SessionId, SessionRecord, SessionRevision, SessionStore,
    SessionStoreError, SessionStoreErrorKind,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_core::{
    Message, Role, SessionIncarnationId, ToolCall, ToolCallId, ToolName, ToolOutput,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, Visitor, value::MapAccessDeserializer},
};
use serde_json::Value;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{AtFlags, Dir, FileType, FlockOperation, Mode, OFlags};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use sha2::{Digest, Sha256};

/// Schema version written by [`FileSessionStore`].
pub const FILE_SESSION_SCHEMA_VERSION: u32 = 1;

/// Maximum number of serialized bytes in one file session record.
pub const MAX_FILE_SESSION_BYTES: usize = 8_651_165;

/// Maximum number of session IDs returned by one native listing.
pub const MAX_LIST_SESSIONS: usize = 100;

/// Maximum number of non-dot directory entries processed by one native listing.
///
/// One additional entry name may be fetched solely to prove overflow.
pub const MAX_LIST_SESSION_DIRECTORY_ENTRIES: usize = 1_024;

/// Maximum aggregate record bytes accepted and decoded by one native listing.
///
/// Concurrent file growth may transfer one additional byte solely to prove
/// overflow; that witness may be retained transiently, but is not accepted,
/// decoded, or returned.
pub const MAX_LIST_SESSION_TOTAL_RECORD_BYTES: usize = 64 * 1_024 * 1_024;

pub(crate) const MAX_STORED_JSON_DEPTH: usize = 64;
pub(crate) const MAX_STORED_JSON_NODES: usize = 65_536;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SESSION_INSPECTION_READ_BUFFER_BYTES: usize = 4 * 1_024;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const JSON_KEY_TRACKER_INITIAL_BUCKETS: usize = 8;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const JSON_KEY_TRACKER_MAX_BUCKETS: usize = MAX_STORED_JSON_NODES * 2;
#[cfg(any(target_os = "linux", target_os = "macos"))]
/// `serde_json` starts with 128 remaining recursion slots and rejects a JSON
/// container when entering it would reduce that counter to zero. The ordinary
/// store decoder therefore accepts at most 127 simultaneously active arrays or
/// objects, including the typed envelope surrounding an arbitrary JSON value.
const MAX_SERDE_JSON_ACTIVE_CONTAINERS: usize = 127;
/// Active typed containers before a top-level metadata value is decoded:
/// envelope object, record object, and metadata map.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const METADATA_JSON_PARENT_CONTAINERS: usize = 3;
/// Active typed containers before a `json` content value is decoded: envelope,
/// record, messages, message, content, and internally tagged content block.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const CONTENT_JSON_PARENT_CONTAINERS: usize = 6;
/// A tool-call or tool-result payload has the same parents as `json` content
/// plus its `call` or `output` object.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TOOL_JSON_PARENT_CONTAINERS: usize = 7;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const JSON_KEY_VERIFICATION_DOMAIN: &[u8] = b"machine-god:json-key-verification:v1:";

#[cfg(any(target_os = "linux", target_os = "macos"))]
const FILE_NAME_DOMAIN: &[u8] = b"machine-god:file-session:v1:";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SESSION_FILE_PREFIX: &[u8] = b"session-";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SESSION_DATA_SUFFIX: &[u8] = b".json";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SESSION_DIGEST_BYTES: usize = 64;

/// Stable category for failure to acquire a session directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSessionStoreOpenErrorKind {
    /// Native file sessions are not supported on this platform.
    UnsupportedPlatform,
    /// The injected path was not absolute.
    InvalidRoot,
    /// The injected path did not resolve to a real directory.
    InvalidFileType,
    /// The injected directory could not be safely opened or inspected.
    Unavailable,
}

/// Redacted failure to construct a [`FileSessionStore`].
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct FileSessionStoreOpenError {
    kind: FileSessionStoreOpenErrorKind,
}

impl FileSessionStoreOpenError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> FileSessionStoreOpenErrorKind {
        self.kind
    }

    const fn new(kind: FileSessionStoreOpenErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for FileSessionStoreOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileSessionStoreOpenError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for FileSessionStoreOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            FileSessionStoreOpenErrorKind::UnsupportedPlatform => {
                "native file session store is unsupported on this platform"
            }
            FileSessionStoreOpenErrorKind::InvalidRoot => {
                "native file session store root is invalid"
            }
            FileSessionStoreOpenErrorKind::InvalidFileType => {
                "native file session store root is not a directory"
            }
            FileSessionStoreOpenErrorKind::Unavailable => {
                "native file session store root is unavailable"
            }
        })
    }
}

impl Error for FileSessionStoreOpenError {}

/// Descriptor-confined durable storage for provider-neutral session records.
///
/// Construction is the only ambient filesystem operation. On Linux and macOS,
/// the store retains a no-follow descriptor for the explicitly supplied
/// directory and performs all later operations relative to it. `load` and
/// `save` do no work before their returned future is first polled and start no
/// detached work. Successful transferred bytes and parser work are capped, but
/// advisory-lock wait, filesystem latency, syncing, and interrupted-operation
/// retries have no wall-clock or attempt bound and may block the polling thread.
pub struct FileSessionStore {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) struct FileSessionList {
    pub(crate) session_ids: Vec<SessionId>,
    pub(crate) truncated: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
pub(crate) struct FileSessionInspection {
    pub(crate) session_id: SessionId,
    pub(crate) incarnation_id: SessionIncarnationId,
    pub(crate) revision: SessionRevision,
    pub(crate) next_turn_sequence: u64,
    pub(crate) message_count: usize,
    pub(crate) metadata_entry_count: usize,
    #[cfg(test)]
    pub(crate) bytes_read: usize,
    #[cfg(test)]
    pub(crate) max_read_request: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum StoredRecordRead {
    Missing,
    Record {
        record: SessionRecord,
        bytes_read: usize,
    },
    ByteLimit,
}

impl FileSessionStore {
    /// Opens an existing absolute directory without following its final path
    /// component. This method never discovers or creates a directory.
    ///
    /// # Errors
    ///
    /// Returns a fixed, redacted error when the platform is unsupported or the
    /// supplied directory cannot be retained safely.
    pub fn open(root: &Path) -> Result<Self, FileSessionStoreOpenError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = root;
            Err(FileSessionStoreOpenError::new(
                FileSessionStoreOpenErrorKind::UnsupportedPlatform,
            ))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let lexical_root = root.components().collect::<std::path::PathBuf>();
            if !lexical_root.is_absolute() {
                return Err(FileSessionStoreOpenError::new(
                    FileSessionStoreOpenErrorKind::InvalidRoot,
                ));
            }
            let descriptor = rustix::fs::open(
                &lexical_root,
                OFlags::RDONLY
                    | OFlags::DIRECTORY
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC
                    | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|error| map_root_open_error(&lexical_root, error))?;
            let metadata = rustix::fs::fstat(&descriptor).map_err(|_| {
                FileSessionStoreOpenError::new(FileSessionStoreOpenErrorKind::Unavailable)
            })?;
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(FileSessionStoreOpenError::new(
                    FileSessionStoreOpenErrorKind::InvalidFileType,
                ));
            }
            Ok(Self { root: descriptor })
        }
    }
}

impl fmt::Debug for FileSessionStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileSessionStore")
            .finish_non_exhaustive()
    }
}

impl SessionStore for FileSessionStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        Box::pin(async move {
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = id;
                Err(unavailable(false))
            }
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.load_unix(&id)
            }
        })
    }

    fn save(
        &self,
        record: SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        let record = RecordOwner::new(record);
        Box::pin(async move {
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = (record, expected_revision);
                Err(unavailable(false))
            }
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                let mut record = record;
                self.save_unix(record.get_mut(), expected_revision)
            }
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FileSessionStore {
    pub(crate) const fn from_root_descriptor(root: OwnedFd) -> Self {
        Self { root }
    }

    pub(crate) fn create_empty_record(
        &self,
        mut record: SessionRecord,
    ) -> Result<SessionRecord, SessionStoreError> {
        if !is_empty_unsaved_record(&record) || validate_record_json(&record).is_err() {
            return Err(serialization_failed());
        }
        let names = SessionNames::for_id(&record.id);
        let lock = open_lock(self.root.as_fd(), &names.lock)?;
        lock_exclusive(&lock)?;
        if read_record(self.root.as_fd(), &names.data, &record.id)?.is_some() {
            return Err(revision_conflict());
        }
        record.revision = SessionRevision(1);
        publish_record(self.root.as_fd(), &names, &record)?;
        Ok(record)
    }

    /// Reads and validates one durable record while retaining only its
    /// structural inspection projection. Transcript bodies and embedded JSON
    /// values are consumed directly from a fixed-size input buffer.
    pub(crate) fn inspect_session_summary(
        &self,
        id: SessionId,
    ) -> Result<Option<FileSessionInspection>, SessionStoreError> {
        let names = SessionNames::for_id(&id);
        if !probe_data(self.root.as_fd(), &names.data)? {
            return Ok(None);
        }
        let lock = open_lock(self.root.as_fd(), &names.lock)?;
        lock_exclusive(&lock)?;
        let inspection = read_session_inspection(self.root.as_fd(), &names.data, &id);
        drop(id);
        inspection
    }

    pub(crate) fn reset_record(
        &self,
        observed: &SessionRecord,
        mut replacement: SessionRecord,
    ) -> Result<SessionRecord, SessionStoreError> {
        if observed.id != replacement.id
            || !is_empty_unsaved_record(&replacement)
            || validate_record_json(&replacement).is_err()
        {
            return Err(serialization_failed());
        }
        let names = SessionNames::for_id(&observed.id);
        let lock = open_lock(self.root.as_fd(), &names.lock)?;
        lock_exclusive(&lock)?;
        let Some(current) = read_record(self.root.as_fd(), &names.data, &observed.id)? else {
            return Err(revision_conflict());
        };
        if current.incarnation_id != observed.incarnation_id
            || current.revision != observed.revision
        {
            return Err(revision_conflict());
        }
        replacement.revision = SessionRevision(
            observed
                .revision
                .0
                .checked_add(1)
                .ok_or_else(revision_exhausted)?,
        );
        publish_record(self.root.as_fd(), &names, &replacement)?;
        Ok(replacement)
    }

    pub(crate) fn list_session_ids(&self) -> Result<FileSessionList, SessionStoreError> {
        self.list_session_ids_after_directory_open(|| {})
    }

    fn list_session_ids_after_directory_open(
        &self,
        after_directory_open: impl FnOnce(),
    ) -> Result<FileSessionList, SessionStoreError> {
        let directory = rustix::fs::openat(
            self.root.as_fd(),
            ".",
            OFlags::RDONLY
                | OFlags::DIRECTORY
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(map_io_error)?;
        after_directory_open();
        ensure_listing_root_is_linked(directory.as_fd())?;
        let mut stream = Dir::new(directory).map_err(map_io_error)?;
        let mut candidates = Vec::new();
        let mut scanned_entries = 0_usize;
        let mut truncated = false;

        loop {
            let Some(entry) = stream.next() else {
                break;
            };
            let entry = entry.map_err(map_io_error)?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if scanned_entries >= MAX_LIST_SESSION_DIRECTORY_ENTRIES {
                truncated = true;
                break;
            }
            scanned_entries += 1;
            if is_session_data_name(name) {
                let name = std::str::from_utf8(name)
                    .expect("canonical session data names are ASCII")
                    .to_owned();
                candidates.push(name);
            }
        }

        candidates.sort_unstable();
        candidates.dedup();

        let mut session_ids = Vec::new();
        let mut total_record_bytes = 0_usize;
        for data_name in candidates {
            if session_ids.len() >= MAX_LIST_SESSIONS {
                truncated = true;
                break;
            }
            let remaining_bytes = MAX_LIST_SESSION_TOTAL_RECORD_BYTES
                .checked_sub(total_record_bytes)
                .expect("listing byte accounting cannot exceed its limit");
            if remaining_bytes == 0 {
                truncated = true;
                break;
            }

            if !probe_data(self.root.as_fd(), &data_name)? {
                continue;
            }
            let lock_name = lock_name_for_data_name(&data_name);
            let lock = open_lock(self.root.as_fd(), &lock_name)?;
            lock_exclusive(&lock)?;
            let (record, bytes_read) =
                match read_stored_record(self.root.as_fd(), &data_name, remaining_bytes)? {
                    StoredRecordRead::Missing => continue,
                    StoredRecordRead::Record { record, bytes_read } => (record, bytes_read),
                    StoredRecordRead::ByteLimit => {
                        truncated = true;
                        break;
                    }
                };
            let record = RecordOwner::new(record);
            if SessionNames::for_id(&record.get().id).data != data_name {
                return Err(corrupt());
            }
            total_record_bytes = total_record_bytes
                .checked_add(bytes_read)
                .filter(|total| *total <= MAX_LIST_SESSION_TOTAL_RECORD_BYTES)
                .expect("a successful bounded listing read fits the aggregate limit");
            session_ids.push(record.get().id.clone());
        }

        session_ids.sort_unstable();
        session_ids.dedup();
        Ok(FileSessionList {
            session_ids,
            truncated,
        })
    }

    fn load_unix(&self, id: &SessionId) -> Result<Option<SessionRecord>, SessionStoreError> {
        let names = SessionNames::for_id(id);
        if !probe_data(self.root.as_fd(), &names.data)? {
            return Ok(None);
        }
        let lock = open_lock(self.root.as_fd(), &names.lock)?;
        lock_exclusive(&lock)?;
        let record = read_record(self.root.as_fd(), &names.data, id)?;
        Ok(record)
    }

    fn save_unix(
        &self,
        record: &mut SessionRecord,
        expected_revision: Option<SessionRevision>,
    ) -> Result<SessionRevision, SessionStoreError> {
        if record.next_turn_sequence == 0 || validate_record_json(record).is_err() {
            return Err(serialization_failed());
        }
        let names = SessionNames::for_id(&record.id);
        let lock = open_lock(self.root.as_fd(), &names.lock)?;
        lock_exclusive(&lock)?;
        let current = read_record(self.root.as_fd(), &names.data, &record.id)?;
        match &current {
            Some(stored) => {
                if stored.incarnation_id != record.incarnation_id {
                    return Err(incarnation_conflict());
                }
                if expected_revision != Some(stored.revision) {
                    return Err(revision_conflict());
                }
            }
            None if expected_revision.is_some() => return Err(revision_conflict()),
            None => {}
        }

        let revision_base = current
            .as_ref()
            .map_or(SessionRevision(0), |stored| stored.revision)
            .max(record.revision);
        let revision = SessionRevision(
            revision_base
                .0
                .checked_add(1)
                .ok_or_else(revision_exhausted)?,
        );
        record.revision = revision;
        publish_record(self.root.as_fd(), &names, record)?;
        Ok(revision)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_listing_root_is_linked(
    root: rustix::fd::BorrowedFd<'_>,
) -> Result<(), SessionStoreError> {
    #[cfg(target_os = "linux")]
    {
        if rustix::fs::fstat(root).map_err(map_io_error)?.st_nlink == 0 {
            return Err(unavailable(true));
        }
    }
    #[cfg(target_os = "macos")]
    ensure_macos_root_is_linked(root)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_macos_root_is_linked(root: rustix::fd::BorrowedFd<'_>) -> Result<(), SessionStoreError> {
    let root_metadata = rustix::fs::fstat(root).map_err(map_io_error)?;
    let root_path = rustix::fs::getpath(root).map_err(map_io_error)?;
    let root_path = root_path.as_bytes();
    if root_path == b"/" {
        return Ok(());
    }
    let name = root_path
        .rsplit(|byte| *byte == b'/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| unavailable(true))?;
    let name = std::ffi::CString::new(name).map_err(|_| unavailable(true))?;
    let parent = rustix::fs::openat(
        root,
        "..",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(map_io_error)?;
    let linked_metadata =
        rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(map_io_error)?;
    if linked_metadata.st_dev != root_metadata.st_dev
        || linked_metadata.st_ino != root_metadata.st_ino
        || !FileType::from_raw_mode(linked_metadata.st_mode).is_dir()
    {
        return Err(unavailable(true));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_empty_unsaved_record(record: &SessionRecord) -> bool {
    record.revision == SessionRevision(0)
        && record.next_turn_sequence == 1
        && record.messages.is_empty()
        && record.metadata.is_empty()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn publish_record(
    root: rustix::fd::BorrowedFd<'_>,
    names: &SessionNames,
    record: &SessionRecord,
) -> Result<(), SessionStoreError> {
    let bytes = serialize_record(record)?;
    let temp = create_temp(root, &names.temp)?;
    if let Err(error) = write_all(&temp, &bytes).and_then(|()| sync_file(&temp)) {
        let _ = rustix::fs::unlinkat(root, &names.temp, AtFlags::empty());
        return Err(map_io_error(error));
    }
    if let Err(error) = rustix::fs::renameat(root, &names.temp, root, &names.data) {
        let _ = rustix::fs::unlinkat(root, &names.temp, AtFlags::empty());
        return Err(map_io_error(error));
    }
    if sync_file(root).is_err() {
        return Err(save_ambiguous());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct SessionNames {
    data: String,
    lock: String,
    temp: String,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl SessionNames {
    fn for_id(id: &SessionId) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(FILE_NAME_DOMAIN);
        hasher.update(id.as_str().as_bytes());
        let digest = format!("{:x}", hasher.finalize());
        let stem = format!("session-{digest}");
        Self {
            data: format!("{stem}.json"),
            lock: format!("{stem}.lock"),
            temp: format!("{stem}.tmp"),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_session_data_name(name: &[u8]) -> bool {
    let expected_len = SESSION_FILE_PREFIX.len() + SESSION_DIGEST_BYTES + SESSION_DATA_SUFFIX.len();
    name.len() == expected_len
        && name.starts_with(SESSION_FILE_PREFIX)
        && name.ends_with(SESSION_DATA_SUFFIX)
        && name[SESSION_FILE_PREFIX.len()..SESSION_FILE_PREFIX.len() + SESSION_DIGEST_BYTES]
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(*byte, b'a'..=b'f'))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn lock_name_for_data_name(data_name: &str) -> String {
    let stem = data_name
        .strip_suffix(".json")
        .expect("canonical session data name has its fixed suffix");
    format!("{stem}.lock")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn probe_data(root: rustix::fd::BorrowedFd<'_>, name: &str) -> Result<bool, SessionStoreError> {
    match rustix::fs::openat(
        root,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(file) => {
            ensure_regular(&file)?;
            Ok(true)
        }
        Err(error) if error == rustix::io::Errno::NOENT => Ok(false),
        Err(error) => Err(map_existing_entry_open_error(root, name, error)),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_lock(root: rustix::fd::BorrowedFd<'_>, name: &str) -> Result<OwnedFd, SessionStoreError> {
    let created = rustix::fs::openat(
        root,
        name,
        OFlags::RDWR
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK,
        Mode::from_raw_mode(0o600),
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
            .map_err(|error| map_existing_entry_open_error(root, name, error))?;
            (file, false)
        }
        Err(error) => {
            return Err(if is_rejected_type_error(error) {
                corrupt()
            } else {
                map_io_error(error)
            });
        }
    };
    ensure_regular(&file)?;
    if was_created {
        rustix::fs::fchmod(&file, Mode::RUSR | Mode::WUSR).map_err(map_io_error)?;
    }
    Ok(file)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn prepare_new_temp(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
    file: OwnedFd,
) -> Result<OwnedFd, SessionStoreError> {
    if let Err(error) = ensure_regular(&file)
        .and_then(|_| rustix::fs::fchmod(&file, Mode::RUSR | Mode::WUSR).map_err(map_io_error))
    {
        let _ = rustix::fs::unlinkat(root, name, AtFlags::empty());
        return Err(error);
    }
    Ok(file)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_created_entry_error(error: rustix::io::Errno) -> SessionStoreError {
    if error == rustix::io::Errno::EXIST || is_rejected_type_error(error) {
        corrupt()
    } else {
        map_io_error(error)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_existing_entry_open_error(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
    error: rustix::io::Errno,
) -> SessionStoreError {
    if is_rejected_type_error(error)
        || rustix::fs::statat(root, name, AtFlags::SYMLINK_NOFOLLOW)
            .is_ok_and(|metadata| !FileType::from_raw_mode(metadata.st_mode).is_file())
    {
        corrupt()
    } else {
        map_io_error(error)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn lock_exclusive(file: &OwnedFd) -> Result<(), SessionStoreError> {
    retry_interrupted(|| rustix::fs::flock(file, FlockOperation::LockExclusive))
        .map_err(map_io_error)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_session_inspection(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
    expected_id: &SessionId,
) -> Result<Option<FileSessionInspection>, SessionStoreError> {
    let file = match rustix::fs::openat(
        root,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(file) => file,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(error) => return Err(map_existing_entry_open_error(root, name, error)),
    };
    let metadata = ensure_regular(&file)?;
    if metadata.st_size < 0
        || usize::try_from(metadata.st_size).map_err(|_| corrupt())? > MAX_FILE_SESSION_BYTES
    {
        return Err(corrupt());
    }

    let mut reader = InspectionReader::new(&file);
    let inspection = {
        let mut parser = InspectionParser::new(&mut reader);
        let inspection = parser
            .parse_envelope()
            .map_err(map_inspection_parse_error)?;
        parser.finish().map_err(map_inspection_parse_error)?;
        inspection
    };
    if &inspection.session_id != expected_id {
        return Err(corrupt());
    }
    #[cfg(test)]
    let inspection = {
        let mut inspection = inspection;
        inspection.bytes_read = reader.bytes_read;
        inspection.max_read_request = reader.max_read_request;
        inspection
    };
    Ok(Some(inspection))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
enum InspectionParseError {
    Corrupt,
    Unavailable,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_inspection_parse_error(error: InspectionParseError) -> SessionStoreError {
    match error {
        InspectionParseError::Corrupt => corrupt(),
        InspectionParseError::Unavailable => unavailable(true),
    }
}

/// A descriptor reader with fixed stack storage and an independent one-byte
/// overflow witness. The parser never requests or retains a file-sized input
/// buffer.
#[cfg(any(target_os = "linux", target_os = "macos"))]
struct InspectionReader<'a> {
    file: &'a OwnedFd,
    buffer: [u8; SESSION_INSPECTION_READ_BUFFER_BYTES],
    start: usize,
    end: usize,
    bytes_read: usize,
    #[cfg(test)]
    max_read_request: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'a> InspectionReader<'a> {
    fn new(file: &'a OwnedFd) -> Self {
        Self {
            file,
            buffer: [0; SESSION_INSPECTION_READ_BUFFER_BYTES],
            start: 0,
            end: 0,
            bytes_read: 0,
            #[cfg(test)]
            max_read_request: 0,
        }
    }

    fn peek(&mut self) -> Result<Option<u8>, InspectionParseError> {
        if self.start == self.end {
            self.fill()?;
        }
        Ok((self.start != self.end).then(|| self.buffer[self.start]))
    }

    fn next(&mut self) -> Result<Option<u8>, InspectionParseError> {
        let byte = self.peek()?;
        if byte.is_some() {
            self.start += 1;
        }
        Ok(byte)
    }

    fn fill(&mut self) -> Result<(), InspectionParseError> {
        let remaining = MAX_FILE_SESSION_BYTES
            .saturating_add(1)
            .saturating_sub(self.bytes_read);
        if remaining == 0 {
            return Err(InspectionParseError::Corrupt);
        }
        let request = remaining.min(self.buffer.len());
        #[cfg(test)]
        {
            self.max_read_request = self.max_read_request.max(request);
        }
        let read = retry_interrupted(|| rustix::io::read(self.file, &mut self.buffer[..request]))
            .map_err(|_| InspectionParseError::Unavailable)?;
        self.start = 0;
        self.end = read;
        self.bytes_read = self
            .bytes_read
            .checked_add(read)
            .ok_or(InspectionParseError::Corrupt)?;
        if self.bytes_read > MAX_FILE_SESSION_BYTES {
            return Err(InspectionParseError::Corrupt);
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct InspectionParser<'a, 'fd> {
    reader: &'a mut InspectionReader<'fd>,
    json_nodes: usize,
    json_keys: JsonKeyTracker,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'a, 'fd> InspectionParser<'a, 'fd> {
    fn new(reader: &'a mut InspectionReader<'fd>) -> Self {
        Self {
            reader,
            json_nodes: 0,
            json_keys: JsonKeyTracker::new(),
        }
    }

    fn parse_envelope(&mut self) -> Result<FileSessionInspection, InspectionParseError> {
        self.expect(b'{')?;
        let mut fields = 0_u8;
        let mut schema_version = None;
        let mut record = None;
        if self.consume(b'}')? {
            return Err(InspectionParseError::Corrupt);
        }
        loop {
            let field = self.parse_stack_string::<32>()?;
            self.expect(b':')?;
            match field.as_str() {
                "schema_version" => {
                    mark_field(&mut fields, 1)?;
                    schema_version = Some(self.parse_u64()?);
                }
                "record" => {
                    mark_field(&mut fields, 2)?;
                    record = Some(self.parse_record()?);
                }
                _ => return Err(InspectionParseError::Corrupt),
            }
            if self.consume(b'}')? {
                break;
            }
            self.expect(b',')?;
        }
        if fields != 3 || schema_version != Some(u64::from(FILE_SESSION_SCHEMA_VERSION)) {
            return Err(InspectionParseError::Corrupt);
        }
        record.ok_or(InspectionParseError::Corrupt)
    }

    fn parse_record(&mut self) -> Result<FileSessionInspection, InspectionParseError> {
        self.expect(b'{')?;
        let mut fields = 0_u8;
        let mut session_id = None;
        let mut incarnation_id = None;
        let mut revision = None;
        let mut next_turn_sequence = None;
        let mut message_count = None;
        let mut metadata_entry_count = None;
        if self.consume(b'}')? {
            return Err(InspectionParseError::Corrupt);
        }
        loop {
            let field = self.parse_stack_string::<32>()?;
            self.expect(b':')?;
            match field.as_str() {
                "id" => {
                    mark_field(&mut fields, 1)?;
                    session_id = Some(self.parse_session_id()?);
                }
                "incarnation_id" => {
                    mark_field(&mut fields, 2)?;
                    incarnation_id = Some(self.parse_incarnation_id()?);
                }
                "revision" => {
                    mark_field(&mut fields, 4)?;
                    let value = self.parse_u64()?;
                    if value == 0 {
                        return Err(InspectionParseError::Corrupt);
                    }
                    revision = Some(SessionRevision(value));
                }
                "next_turn_sequence" => {
                    mark_field(&mut fields, 8)?;
                    let value = self.parse_u64()?;
                    if value == 0 {
                        return Err(InspectionParseError::Corrupt);
                    }
                    next_turn_sequence = Some(value);
                }
                "messages" => {
                    mark_field(&mut fields, 16)?;
                    message_count = Some(self.parse_messages()?);
                }
                "metadata" => {
                    mark_field(&mut fields, 32)?;
                    metadata_entry_count = Some(self.parse_metadata()?);
                }
                _ => return Err(InspectionParseError::Corrupt),
            }
            if self.consume(b'}')? {
                break;
            }
            self.expect(b',')?;
        }
        if fields != 63 {
            return Err(InspectionParseError::Corrupt);
        }
        Ok(FileSessionInspection {
            session_id: session_id.ok_or(InspectionParseError::Corrupt)?,
            incarnation_id: incarnation_id.ok_or(InspectionParseError::Corrupt)?,
            revision: revision.ok_or(InspectionParseError::Corrupt)?,
            next_turn_sequence: next_turn_sequence.ok_or(InspectionParseError::Corrupt)?,
            message_count: message_count.ok_or(InspectionParseError::Corrupt)?,
            metadata_entry_count: metadata_entry_count.ok_or(InspectionParseError::Corrupt)?,
            #[cfg(test)]
            bytes_read: 0,
            #[cfg(test)]
            max_read_request: 0,
        })
    }

    fn parse_messages(&mut self) -> Result<usize, InspectionParseError> {
        self.expect(b'[')?;
        let mut count = 0_usize;
        if self.consume(b']')? {
            return Ok(count);
        }
        loop {
            self.parse_message()?;
            count = count.checked_add(1).ok_or(InspectionParseError::Corrupt)?;
            if self.consume(b']')? {
                return Ok(count);
            }
            self.expect(b',')?;
        }
    }

    fn parse_message(&mut self) -> Result<(), InspectionParseError> {
        self.expect(b'{')?;
        let mut fields = 0_u8;
        if self.consume(b'}')? {
            return Err(InspectionParseError::Corrupt);
        }
        loop {
            let field = self.parse_stack_string::<16>()?;
            self.expect(b':')?;
            match field.as_str() {
                "role" => {
                    mark_field(&mut fields, 1)?;
                    match self.parse_stack_string::<16>()?.as_str() {
                        "system" | "user" | "assistant" | "tool" => {}
                        _ => return Err(InspectionParseError::Corrupt),
                    }
                }
                "content" => {
                    mark_field(&mut fields, 2)?;
                    self.parse_content_blocks()?;
                }
                _ => return Err(InspectionParseError::Corrupt),
            }
            if self.consume(b'}')? {
                break;
            }
            self.expect(b',')?;
        }
        if fields == 3 {
            Ok(())
        } else {
            Err(InspectionParseError::Corrupt)
        }
    }

    fn parse_content_blocks(&mut self) -> Result<(), InspectionParseError> {
        self.expect(b'[')?;
        if self.consume(b']')? {
            return Ok(());
        }
        loop {
            self.parse_content_block()?;
            if self.consume(b']')? {
                return Ok(());
            }
            self.expect(b',')?;
        }
    }

    fn parse_content_block(&mut self) -> Result<(), InspectionParseError> {
        self.expect(b'{')?;
        let mut fields = 0_u8;
        let mut kind = None;
        if self.consume(b'}')? {
            return Err(InspectionParseError::Corrupt);
        }
        loop {
            let field = self.parse_stack_string::<16>()?;
            self.expect(b':')?;
            match field.as_str() {
                "type" => {
                    mark_field(&mut fields, 1)?;
                    kind = Some(match self.parse_stack_string::<16>()?.as_str() {
                        "text" => ContentInspectionKind::Text,
                        "json" => ContentInspectionKind::Json,
                        "tool_call" => ContentInspectionKind::ToolCall,
                        "tool_result" => ContentInspectionKind::ToolResult,
                        _ => return Err(InspectionParseError::Corrupt),
                    });
                }
                "text" => {
                    mark_field(&mut fields, 2)?;
                    self.skip_string()?;
                }
                "value" => {
                    mark_field(&mut fields, 4)?;
                    self.parse_and_account_embedded_json(CONTENT_JSON_PARENT_CONTAINERS)?;
                }
                "call" => {
                    mark_field(&mut fields, 8)?;
                    self.parse_tool_call()?;
                }
                "call_id" => {
                    mark_field(&mut fields, 16)?;
                    let id = self.parse_stack_string::<128>()?;
                    ToolCallId::validate(id.as_str()).map_err(|_| InspectionParseError::Corrupt)?;
                }
                "output" => {
                    mark_field(&mut fields, 32)?;
                    self.parse_tool_output()?;
                }
                _ => return Err(InspectionParseError::Corrupt),
            }
            if self.consume(b'}')? {
                break;
            }
            self.expect(b',')?;
        }
        let expected = match kind.ok_or(InspectionParseError::Corrupt)? {
            ContentInspectionKind::Text => 1 | 2,
            ContentInspectionKind::Json => 1 | 4,
            ContentInspectionKind::ToolCall => 1 | 8,
            ContentInspectionKind::ToolResult => 1 | 16 | 32,
        };
        if fields == expected {
            Ok(())
        } else {
            Err(InspectionParseError::Corrupt)
        }
    }

    fn parse_tool_call(&mut self) -> Result<(), InspectionParseError> {
        self.expect(b'{')?;
        let mut fields = 0_u8;
        if self.consume(b'}')? {
            return Err(InspectionParseError::Corrupt);
        }
        loop {
            let field = self.parse_stack_string::<16>()?;
            self.expect(b':')?;
            match field.as_str() {
                "id" => {
                    mark_field(&mut fields, 1)?;
                    let id = self.parse_stack_string::<128>()?;
                    ToolCallId::validate(id.as_str()).map_err(|_| InspectionParseError::Corrupt)?;
                }
                "name" => {
                    mark_field(&mut fields, 2)?;
                    let name = self.parse_stack_string::<128>()?;
                    ToolName::validate(name.as_str()).map_err(|_| InspectionParseError::Corrupt)?;
                }
                "arguments" => {
                    mark_field(&mut fields, 4)?;
                    self.parse_and_account_embedded_json(TOOL_JSON_PARENT_CONTAINERS)?;
                }
                _ => return Err(InspectionParseError::Corrupt),
            }
            if self.consume(b'}')? {
                break;
            }
            self.expect(b',')?;
        }
        if fields == 7 {
            Ok(())
        } else {
            Err(InspectionParseError::Corrupt)
        }
    }

    fn parse_tool_output(&mut self) -> Result<(), InspectionParseError> {
        self.expect(b'{')?;
        let mut fields = 0_u8;
        if self.consume(b'}')? {
            return Err(InspectionParseError::Corrupt);
        }
        loop {
            let field = self.parse_stack_string::<16>()?;
            self.expect(b':')?;
            match field.as_str() {
                "content" => {
                    mark_field(&mut fields, 1)?;
                    self.parse_and_account_embedded_json(TOOL_JSON_PARENT_CONTAINERS)?;
                }
                "is_error" => {
                    mark_field(&mut fields, 2)?;
                    self.parse_bool()?;
                }
                _ => return Err(InspectionParseError::Corrupt),
            }
            if self.consume(b'}')? {
                break;
            }
            self.expect(b',')?;
        }
        if fields == 3 {
            Ok(())
        } else {
            Err(InspectionParseError::Corrupt)
        }
    }

    fn parse_metadata(&mut self) -> Result<usize, InspectionParseError> {
        self.expect(b'{')?;
        let scope = self.json_keys.begin_scope()?;
        if self.consume(b'}')? {
            return Ok(0);
        }
        let mut overflowed = false;
        loop {
            let key = self.parse_string_digest()?;
            self.expect(b':')?;
            let summary =
                self.parse_embedded_json(json_container_budget(METADATA_JSON_PARENT_CONTAINERS))?;
            overflowed |= self.json_keys.upsert(scope, key, summary)?.is_full();
            if self.consume(b'}')? {
                let values = self.json_keys.finish_scope(scope)?;
                if overflowed || values.nodes > MAX_STORED_JSON_NODES {
                    return Err(InspectionParseError::Corrupt);
                }
                if values.max_depth > MAX_STORED_JSON_DEPTH {
                    return Err(InspectionParseError::Corrupt);
                }
                self.add_json_nodes(values.nodes)?;
                return Ok(values.entries);
            }
            self.expect(b',')?;
        }
    }

    /// Validates one arbitrary JSON root and accounts only the final decoded
    /// value. Object duplicates therefore have serde's last-value-wins shape.
    fn parse_and_account_embedded_json(
        &mut self,
        parent_containers: usize,
    ) -> Result<(), InspectionParseError> {
        let summary = self.parse_embedded_json(json_container_budget(parent_containers))?;
        if summary.max_depth > MAX_STORED_JSON_DEPTH {
            return Err(InspectionParseError::Corrupt);
        }
        self.add_json_nodes(summary.nodes)
    }

    fn add_json_nodes(&mut self, nodes: usize) -> Result<(), InspectionParseError> {
        self.json_nodes = capped_json_nodes(self.json_nodes, nodes);
        if self.json_nodes > MAX_STORED_JSON_NODES {
            Err(InspectionParseError::Corrupt)
        } else {
            Ok(())
        }
    }

    fn parse_embedded_json(
        &mut self,
        remaining_containers: usize,
    ) -> Result<JsonSummary, InspectionParseError> {
        self.skip_whitespace()?;
        match self.peek()?.ok_or(InspectionParseError::Corrupt)? {
            b'{' => self.parse_json_object(consume_json_container(remaining_containers)?),
            b'[' => self.parse_json_array(consume_json_container(remaining_containers)?),
            b'"' => {
                self.skip_string()?;
                Ok(JsonSummary::scalar())
            }
            b't' => {
                self.expect_literal(b"true")?;
                Ok(JsonSummary::scalar())
            }
            b'f' => {
                self.expect_literal(b"false")?;
                Ok(JsonSummary::scalar())
            }
            b'n' => {
                self.expect_literal(b"null")?;
                Ok(JsonSummary::scalar())
            }
            b'-' | b'0'..=b'9' => {
                self.parse_json_number()?;
                Ok(JsonSummary::scalar())
            }
            _ => Err(InspectionParseError::Corrupt),
        }
    }

    fn parse_json_object(
        &mut self,
        remaining_containers: usize,
    ) -> Result<JsonSummary, InspectionParseError> {
        self.expect(b'{')?;
        let scope = self.json_keys.begin_scope()?;
        if self.consume(b'}')? {
            return Ok(JsonSummary::container(0, 0));
        }
        let mut overflowed = false;
        loop {
            let key = self.parse_string_digest()?;
            self.expect(b':')?;
            let value = self.parse_embedded_json(remaining_containers)?;
            overflowed |= self.json_keys.upsert(scope, key, value)?.is_full();
            if self.consume(b'}')? {
                let values = self.json_keys.finish_scope(scope)?;
                return Ok(if overflowed {
                    JsonSummary::over_limit()
                } else {
                    JsonSummary::container(values.nodes, values.max_depth)
                });
            }
            self.expect(b',')?;
        }
    }

    fn parse_json_array(
        &mut self,
        remaining_containers: usize,
    ) -> Result<JsonSummary, InspectionParseError> {
        self.expect(b'[')?;
        let mut nodes = 0;
        let mut max_depth = 0;
        if self.consume(b']')? {
            return Ok(JsonSummary::container(nodes, max_depth));
        }
        loop {
            let value = self.parse_embedded_json(remaining_containers)?;
            nodes = capped_json_nodes(nodes, value.nodes);
            max_depth = max_depth.max(value.max_depth);
            if self.consume(b']')? {
                return Ok(JsonSummary::container(nodes, max_depth));
            }
            self.expect(b',')?;
        }
    }

    fn parse_bool(&mut self) -> Result<(), InspectionParseError> {
        self.skip_whitespace()?;
        match self.peek()? {
            Some(b't') => self.expect_literal(b"true"),
            Some(b'f') => self.expect_literal(b"false"),
            _ => Err(InspectionParseError::Corrupt),
        }
    }

    fn parse_u64(&mut self) -> Result<u64, InspectionParseError> {
        self.skip_whitespace()?;
        let first = self.next()?.ok_or(InspectionParseError::Corrupt)?;
        if !first.is_ascii_digit() {
            return Err(InspectionParseError::Corrupt);
        }
        if first == b'0' && self.peek_raw()?.is_some_and(|byte| byte.is_ascii_digit()) {
            return Err(InspectionParseError::Corrupt);
        }
        let mut value = u64::from(first - b'0');
        while let Some(byte) = self.peek_raw()? {
            if !byte.is_ascii_digit() {
                break;
            }
            self.next()?;
            value = value
                .checked_mul(10)
                .and_then(|current| current.checked_add(u64::from(byte - b'0')))
                .ok_or(InspectionParseError::Corrupt)?;
        }
        Ok(value)
    }

    fn parse_json_number(&mut self) -> Result<(), InspectionParseError> {
        self.skip_whitespace()?;
        let mut adapter = JsonNumberReader::new(self.reader);
        serde_json::from_reader::<_, serde_json::Number>(&mut adapter)
            .map_err(|_| adapter.error.unwrap_or(InspectionParseError::Corrupt))?;
        adapter.error.map_or(Ok(()), Err)
    }

    fn parse_stack_string<const N: usize>(
        &mut self,
    ) -> Result<StackString<N>, InspectionParseError> {
        let mut value = StackString::new();
        self.parse_string_chars(|character| value.push(character))?;
        Ok(value)
    }

    fn parse_session_id(&mut self) -> Result<SessionId, InspectionParseError> {
        let value = self.parse_stack_string::<128>()?;
        SessionId::new(value.as_str()).map_err(|_| InspectionParseError::Corrupt)
    }

    fn parse_incarnation_id(&mut self) -> Result<SessionIncarnationId, InspectionParseError> {
        let value = self.parse_stack_string::<128>()?;
        SessionIncarnationId::new(value.as_str()).map_err(|_| InspectionParseError::Corrupt)
    }

    fn parse_string_digest(&mut self) -> Result<JsonKeyFingerprint, InspectionParseError> {
        let mut digest = Sha256::new();
        let mut verifier = Sha256::new();
        verifier.update(JSON_KEY_VERIFICATION_DOMAIN);
        let mut length = 0_usize;
        self.parse_string_chars(|character| {
            let mut bytes = [0_u8; 4];
            let bytes = character.encode_utf8(&mut bytes).as_bytes();
            length = length
                .checked_add(bytes.len())
                .ok_or(InspectionParseError::Corrupt)?;
            digest.update(bytes);
            verifier.update(bytes);
            Ok(())
        })?;
        Ok(JsonKeyFingerprint {
            digest: digest.finalize().into(),
            verifier: verifier.finalize().into(),
            length,
        })
    }

    fn skip_string(&mut self) -> Result<(), InspectionParseError> {
        self.parse_string_chars(|_| Ok(()))
    }

    fn parse_string_chars(
        &mut self,
        mut consume: impl FnMut(char) -> Result<(), InspectionParseError>,
    ) -> Result<(), InspectionParseError> {
        self.expect(b'"')?;
        loop {
            let byte = self.next()?.ok_or(InspectionParseError::Corrupt)?;
            match byte {
                b'"' => return Ok(()),
                b'\\' => {
                    let escaped = self.next()?.ok_or(InspectionParseError::Corrupt)?;
                    let character = match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{0008}',
                        b'f' => '\u{000c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.parse_unicode_escape()?,
                        _ => return Err(InspectionParseError::Corrupt),
                    };
                    consume(character)?;
                }
                0x00..=0x1f => return Err(InspectionParseError::Corrupt),
                0x20..=0x7f => consume(char::from(byte))?,
                _ => consume(self.parse_utf8_character(byte)?)?,
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, InspectionParseError> {
        let first = self.parse_hex_quad()?;
        let scalar = if (0xd800..=0xdbff).contains(&first) {
            if self.next()? != Some(b'\\') || self.next()? != Some(b'u') {
                return Err(InspectionParseError::Corrupt);
            }
            let second = self.parse_hex_quad()?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(InspectionParseError::Corrupt);
            }
            0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&first) {
            return Err(InspectionParseError::Corrupt);
        } else {
            u32::from(first)
        };
        char::from_u32(scalar).ok_or(InspectionParseError::Corrupt)
    }

    fn parse_hex_quad(&mut self) -> Result<u16, InspectionParseError> {
        let mut value = 0_u16;
        for _ in 0..4 {
            let byte = self.next()?.ok_or(InspectionParseError::Corrupt)?;
            let digit = match byte {
                b'0'..=b'9' => u16::from(byte - b'0'),
                b'a'..=b'f' => u16::from(byte - b'a') + 10,
                b'A'..=b'F' => u16::from(byte - b'A') + 10,
                _ => return Err(InspectionParseError::Corrupt),
            };
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn parse_utf8_character(&mut self, first: u8) -> Result<char, InspectionParseError> {
        let length = match first {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return Err(InspectionParseError::Corrupt),
        };
        let mut bytes = [0_u8; 4];
        bytes[0] = first;
        for byte in &mut bytes[1..length] {
            *byte = self.next()?.ok_or(InspectionParseError::Corrupt)?;
            if !matches!(*byte, 0x80..=0xbf) {
                return Err(InspectionParseError::Corrupt);
            }
        }
        let text =
            std::str::from_utf8(&bytes[..length]).map_err(|_| InspectionParseError::Corrupt)?;
        text.chars().next().ok_or(InspectionParseError::Corrupt)
    }

    fn finish(&mut self) -> Result<(), InspectionParseError> {
        self.skip_whitespace()?;
        if self.reader.peek()?.is_none() {
            Ok(())
        } else {
            Err(InspectionParseError::Corrupt)
        }
    }

    fn skip_whitespace(&mut self) -> Result<(), InspectionParseError> {
        while self
            .reader
            .peek()?
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.reader.next()?;
        }
        Ok(())
    }

    fn peek(&mut self) -> Result<Option<u8>, InspectionParseError> {
        self.skip_whitespace()?;
        self.reader.peek()
    }

    fn peek_raw(&mut self) -> Result<Option<u8>, InspectionParseError> {
        self.reader.peek()
    }

    fn next(&mut self) -> Result<Option<u8>, InspectionParseError> {
        self.reader.next()
    }

    fn expect(&mut self, expected: u8) -> Result<(), InspectionParseError> {
        self.skip_whitespace()?;
        if self.reader.next()? == Some(expected) {
            Ok(())
        } else {
            Err(InspectionParseError::Corrupt)
        }
    }

    fn consume(&mut self, expected: u8) -> Result<bool, InspectionParseError> {
        self.skip_whitespace()?;
        if self.reader.peek()? == Some(expected) {
            self.reader.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), InspectionParseError> {
        self.skip_whitespace()?;
        for expected in literal {
            if self.reader.next()? != Some(*expected) {
                return Err(InspectionParseError::Corrupt);
            }
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct JsonNumberReader<'a, 'fd> {
    reader: &'a mut InspectionReader<'fd>,
    error: Option<InspectionParseError>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'a, 'fd> JsonNumberReader<'a, 'fd> {
    fn new(reader: &'a mut InspectionReader<'fd>) -> Self {
        Self {
            reader,
            error: None,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Read for JsonNumberReader<'_, '_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.error.is_some() {
            return Ok(0);
        }
        let mut written = 0;
        while written < output.len() {
            let byte = match self.reader.peek() {
                Ok(Some(byte))
                    if matches!(byte, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') =>
                {
                    byte
                }
                Ok(_) => break,
                Err(error) => {
                    self.error = Some(error);
                    return Err(io::Error::other("session inspection read failed"));
                }
            };
            match self.reader.next() {
                Ok(Some(_)) => {
                    output[written] = byte;
                    written += 1;
                }
                Ok(None) => break,
                Err(error) => {
                    self.error = Some(error);
                    return Err(io::Error::other("session inspection read failed"));
                }
            }
        }
        Ok(written)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct StackString<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<const N: usize> StackString<N> {
    const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    fn push(&mut self, character: char) -> Result<(), InspectionParseError> {
        let mut encoded = [0_u8; 4];
        let bytes = character.encode_utf8(&mut encoded).as_bytes();
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(InspectionParseError::Corrupt)?;
        let destination = self
            .bytes
            .get_mut(self.len..end)
            .ok_or(InspectionParseError::Corrupt)?;
        destination.copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len])
            .expect("stack string contains parser-validated UTF-8")
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
struct JsonSummary {
    nodes: usize,
    max_depth: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl JsonSummary {
    const fn scalar() -> Self {
        Self {
            nodes: 1,
            max_depth: 0,
        }
    }

    fn container(child_nodes: usize, child_depth: usize) -> Self {
        Self {
            nodes: capped_json_nodes(1, child_nodes),
            max_depth: child_depth.saturating_add(1),
        }
    }

    const fn over_limit() -> Self {
        Self {
            nodes: MAX_STORED_JSON_NODES + 1,
            max_depth: 0,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn capped_json_nodes(left: usize, right: usize) -> usize {
    left.saturating_add(right).min(MAX_STORED_JSON_NODES + 1)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn json_container_budget(parent_containers: usize) -> usize {
    MAX_SERDE_JSON_ACTIVE_CONTAINERS - parent_containers
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn consume_json_container(remaining: usize) -> Result<usize, InspectionParseError> {
    remaining
        .checked_sub(1)
        .ok_or(InspectionParseError::Corrupt)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
struct JsonKeyScope {
    id: u64,
    base: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
struct JsonKeyFingerprint {
    digest: [u8; 32],
    verifier: [u8; 32],
    length: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct JsonKeyEntry {
    key: JsonKeyFingerprint,
    scope: u64,
    summary: JsonSummary,
    bucket: usize,
    next: Option<usize>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct JsonKeyTracker {
    entries: Vec<JsonKeyEntry>,
    buckets: Vec<Option<usize>>,
    next_scope: u64,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl JsonKeyTracker {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            buckets: Vec::new(),
            next_scope: 0,
        }
    }

    fn begin_scope(&mut self) -> Result<JsonKeyScope, InspectionParseError> {
        self.next_scope = self
            .next_scope
            .checked_add(1)
            .ok_or(InspectionParseError::Corrupt)?;
        Ok(JsonKeyScope {
            id: self.next_scope,
            base: self.entries.len(),
        })
    }

    fn upsert(
        &mut self,
        scope: JsonKeyScope,
        key: JsonKeyFingerprint,
        summary: JsonSummary,
    ) -> Result<JsonKeyInsert, InspectionParseError> {
        let mut cursor = if self.buckets.is_empty() {
            None
        } else {
            let bucket = json_key_bucket(scope.id, &key.digest, self.buckets.len());
            self.buckets[bucket]
        };
        while let Some(index) = cursor {
            let entry = &mut self.entries[index];
            if entry.scope == scope.id && entry.key.digest == key.digest {
                if entry.key.verifier != key.verifier || entry.key.length != key.length {
                    return Err(InspectionParseError::Corrupt);
                }
                entry.summary = summary;
                return Ok(JsonKeyInsert::Updated);
            }
            cursor = entry.next;
        }
        if self.entries.len() == MAX_STORED_JSON_NODES {
            return Ok(JsonKeyInsert::Full);
        }
        self.reserve_for_new_entry()?;
        let bucket = json_key_bucket(scope.id, &key.digest, self.buckets.len());
        let index = self.entries.len();
        self.entries.push(JsonKeyEntry {
            key,
            scope: scope.id,
            summary,
            bucket,
            next: self.buckets[bucket],
        });
        self.buckets[bucket] = Some(index);
        Ok(JsonKeyInsert::Inserted)
    }

    fn reserve_for_new_entry(&mut self) -> Result<(), InspectionParseError> {
        self.entries
            .try_reserve(1)
            .map_err(|_| InspectionParseError::Unavailable)?;
        let required_entries = self
            .entries
            .len()
            .checked_add(1)
            .ok_or(InspectionParseError::Corrupt)?;
        let mut required_buckets = self.buckets.len().max(JSON_KEY_TRACKER_INITIAL_BUCKETS);
        while required_entries > required_buckets.saturating_mul(3) / 4 {
            required_buckets = required_buckets
                .checked_mul(2)
                .filter(|buckets| *buckets <= JSON_KEY_TRACKER_MAX_BUCKETS)
                .ok_or(InspectionParseError::Corrupt)?;
        }
        if required_buckets != self.buckets.len() {
            self.rehash(required_buckets)?;
        }
        Ok(())
    }

    fn rehash(&mut self, bucket_count: usize) -> Result<(), InspectionParseError> {
        let mut buckets = Vec::new();
        buckets
            .try_reserve_exact(bucket_count)
            .map_err(|_| InspectionParseError::Unavailable)?;
        buckets.resize(bucket_count, None);
        for (index, entry) in self.entries.iter_mut().enumerate() {
            let bucket = json_key_bucket(entry.scope, &entry.key.digest, bucket_count);
            entry.bucket = bucket;
            entry.next = buckets[bucket];
            buckets[bucket] = Some(index);
        }
        self.buckets = buckets;
        Ok(())
    }

    fn finish_scope(
        &mut self,
        scope: JsonKeyScope,
    ) -> Result<JsonScopeSummary, InspectionParseError> {
        let entries = self
            .entries
            .get(scope.base..)
            .ok_or(InspectionParseError::Corrupt)?;
        if entries.iter().any(|entry| entry.scope != scope.id) {
            return Err(InspectionParseError::Corrupt);
        }
        let mut nodes = 0;
        let mut max_depth = 0;
        for entry in entries {
            nodes = capped_json_nodes(nodes, entry.summary.nodes);
            max_depth = max_depth.max(entry.summary.max_depth);
        }
        let entry_count = entries.len();
        for index in (scope.base..self.entries.len()).rev() {
            let entry = &self.entries[index];
            if self.buckets[entry.bucket] != Some(index) {
                return Err(InspectionParseError::Corrupt);
            }
            self.buckets[entry.bucket] = entry.next;
        }
        self.entries.truncate(scope.base);
        Ok(JsonScopeSummary {
            entries: entry_count,
            nodes,
            max_depth,
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn json_key_bucket(scope: u64, digest: &[u8; 32], bucket_count: usize) -> usize {
    debug_assert!(bucket_count.is_power_of_two());
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    let hash = u64::from_le_bytes(prefix) ^ scope.rotate_left(17);
    usize::try_from(hash % bucket_count as u64).expect("JSON key bucket is always representable")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum JsonKeyInsert {
    Inserted,
    Updated,
    Full,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl JsonKeyInsert {
    const fn is_full(&self) -> bool {
        matches!(self, Self::Full)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct JsonScopeSummary {
    entries: usize,
    nodes: usize,
    max_depth: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
enum ContentInspectionKind {
    Text,
    Json,
    ToolCall,
    ToolResult,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn mark_field(fields: &mut u8, bit: u8) -> Result<(), InspectionParseError> {
    if *fields & bit != 0 {
        return Err(InspectionParseError::Corrupt);
    }
    *fields |= bit;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_record(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
    expected_id: &SessionId,
) -> Result<Option<SessionRecord>, SessionStoreError> {
    match read_stored_record(root, name, MAX_FILE_SESSION_BYTES)? {
        StoredRecordRead::Missing => Ok(None),
        StoredRecordRead::Record { record, .. } if &record.id == expected_id => Ok(Some(record)),
        StoredRecordRead::Record { .. } | StoredRecordRead::ByteLimit => Err(corrupt()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_stored_record(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
    byte_limit: usize,
) -> Result<StoredRecordRead, SessionStoreError> {
    let file = match rustix::fs::openat(
        root,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(file) => file,
        Err(error) if error == rustix::io::Errno::NOENT => {
            return Ok(StoredRecordRead::Missing);
        }
        Err(error) => return Err(map_existing_entry_open_error(root, name, error)),
    };
    let metadata = ensure_regular(&file)?;
    if metadata.st_size < 0 {
        return Err(corrupt());
    }
    let metadata_bytes = usize::try_from(metadata.st_size).map_err(|_| corrupt())?;
    if metadata_bytes > MAX_FILE_SESSION_BYTES {
        return Err(corrupt());
    }
    if metadata_bytes > byte_limit {
        return Ok(StoredRecordRead::ByteLimit);
    }
    let mut bytes = Vec::with_capacity(metadata_bytes);
    let mut chunk = [0_u8; 8192];
    loop {
        let file_remaining = (MAX_FILE_SESSION_BYTES + 1).saturating_sub(bytes.len());
        if file_remaining == 0 {
            return Err(corrupt());
        }
        let budget_remaining = byte_limit.saturating_add(1).saturating_sub(bytes.len());
        if budget_remaining == 0 {
            return Ok(StoredRecordRead::ByteLimit);
        }
        let chunk_limit = file_remaining.min(budget_remaining).min(chunk.len());
        let read = retry_interrupted(|| rustix::io::read(&file, &mut chunk[..chunk_limit]))
            .map_err(map_io_error)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > MAX_FILE_SESSION_BYTES {
            return Err(corrupt());
        }
        if bytes.len() > byte_limit {
            return Ok(StoredRecordRead::ByteLimit);
        }
    }
    let ObjectOnly(envelope): ObjectOnly<StoredEnvelope> =
        serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
    if envelope.schema_version != FILE_SESSION_SCHEMA_VERSION {
        return Err(corrupt());
    }
    let record = SessionRecord::from(envelope.record.0);
    if record.revision == SessionRevision(0)
        || record.next_turn_sequence == 0
        || validate_record_json(&record).is_err()
    {
        return Err(corrupt());
    }
    Ok(StoredRecordRead::Record {
        record,
        bytes_read: bytes.len(),
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_regular(file: &OwnedFd) -> Result<rustix::fs::Stat, SessionStoreError> {
    let metadata = rustix::fs::fstat(file).map_err(map_io_error)?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file() {
        return Err(corrupt());
    }
    Ok(metadata)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_temp(root: rustix::fd::BorrowedFd<'_>, name: &str) -> Result<OwnedFd, SessionStoreError> {
    match create_new_temp(root, name) {
        Ok(file) => prepare_new_temp(root, name, file),
        Err(error) if error == rustix::io::Errno::EXIST => {
            let stale = rustix::fs::openat(
                root,
                name,
                OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|error| map_existing_entry_open_error(root, name, error))?;
            ensure_regular(&stale)?;
            rustix::fs::unlinkat(root, name, AtFlags::empty()).map_err(map_io_error)?;
            let file = create_new_temp(root, name).map_err(map_created_entry_error)?;
            prepare_new_temp(root, name, file)
        }
        Err(error) if is_rejected_type_error(error) => Err(corrupt()),
        Err(error) => Err(map_io_error(error)),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_new_temp(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
) -> Result<OwnedFd, rustix::io::Errno> {
    rustix::fs::openat(
        root,
        name,
        OFlags::WRONLY
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK,
        Mode::from_raw_mode(0o600),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_all(file: &OwnedFd, mut bytes: &[u8]) -> Result<(), rustix::io::Errno> {
    while !bytes.is_empty() {
        match retry_interrupted(|| rustix::io::write(file, bytes)) {
            Ok(0) => return Err(rustix::io::Errno::IO),
            Ok(written) => bytes = &bytes[written..],
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sync_file(file: impl AsFd) -> Result<(), rustix::io::Errno> {
    retry_interrupted(|| rustix::fs::fsync(file.as_fd()))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_rejected_type_error(error: rustix::io::Errno) -> bool {
    error == rustix::io::Errno::LOOP
        || error == rustix::io::Errno::NOTDIR
        || error == rustix::io::Errno::ISDIR
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_root_open_error(root: &Path, error: rustix::io::Errno) -> FileSessionStoreOpenError {
    let kind = if is_rejected_type_error(error)
        || std::fs::symlink_metadata(root).is_ok_and(|metadata| !metadata.file_type().is_dir())
    {
        FileSessionStoreOpenErrorKind::InvalidFileType
    } else {
        FileSessionStoreOpenErrorKind::Unavailable
    };
    FileSessionStoreOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Serialize)]
struct WriteEnvelope<'a> {
    schema_version: u32,
    record: &'a SessionRecord,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn serialize_record(record: &SessionRecord) -> Result<Vec<u8>, SessionStoreError> {
    let mut writer = BoundedWriter::new();
    serde_json::to_writer(
        &mut writer,
        &WriteEnvelope {
            schema_version: FILE_SESSION_SCHEMA_VERSION,
            record,
        },
    )
    .map_err(|_| {
        if writer.overflowed {
            too_large()
        } else {
            serialization_failed()
        }
    })?;
    Ok(writer.bytes)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct BoundedWriter {
    bytes: Vec<u8>,
    overflowed: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl BoundedWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(64 * 1024),
            overflowed: false,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_FILE_SESSION_BYTES.saturating_sub(self.bytes.len()) {
            self.overflowed = true;
            return Err(io::Error::other("file session size limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct ObjectOnly<T>(T);

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'de, T> Deserialize<'de> for ObjectOnly<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ObjectOnlyVisitor<T>(std::marker::PhantomData<T>);

        impl<'de, T> Visitor<'de> for ObjectOnlyVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = ObjectOnly<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                T::deserialize(MapAccessDeserializer::new(map)).map(ObjectOnly)
            }
        }

        deserializer.deserialize_map(ObjectOnlyVisitor(std::marker::PhantomData))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct StoredRole(Role);

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'de> Deserialize<'de> for StoredRole {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RoleVisitor;

        impl Visitor<'_> for RoleVisitor {
            type Value = StoredRole;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a stored role string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let role = match value {
                    "system" => Role::System,
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    "tool" => Role::Tool,
                    _ => {
                        return Err(E::unknown_variant(
                            value,
                            &["system", "user", "assistant", "tool"],
                        ));
                    }
                };
                Ok(StoredRole(role))
            }
        }

        deserializer.deserialize_str(RoleVisitor)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredEnvelope {
    schema_version: u32,
    record: ObjectOnly<StoredRecord>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRecord {
    id: SessionId,
    incarnation_id: SessionIncarnationId,
    revision: SessionRevision,
    next_turn_sequence: u64,
    messages: Vec<ObjectOnly<StoredMessage>>,
    metadata: BTreeMap<String, Value>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMessage {
    role: StoredRole,
    content: Vec<StoredContentBlock>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum StoredContentBlock {
    Text {
        text: String,
    },
    Json {
        value: Value,
    },
    ToolCall {
        call: ObjectOnly<StoredToolCall>,
    },
    ToolResult {
        call_id: ToolCallId,
        output: ObjectOnly<StoredToolOutput>,
    },
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredToolCall {
    id: ToolCallId,
    name: ToolName,
    arguments: Value,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredToolOutput {
    content: Value,
    is_error: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl From<StoredRecord> for SessionRecord {
    fn from(record: StoredRecord) -> Self {
        Self {
            id: record.id,
            incarnation_id: record.incarnation_id,
            revision: record.revision,
            next_turn_sequence: record.next_turn_sequence,
            messages: record
                .messages
                .into_iter()
                .map(|message| Message::from(message.0))
                .collect(),
            metadata: record.metadata,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl From<StoredMessage> for Message {
    fn from(message: StoredMessage) -> Self {
        Self {
            role: message.role.0,
            content: message
                .content
                .into_iter()
                .map(ContentBlock::from)
                .collect(),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl From<StoredContentBlock> for ContentBlock {
    fn from(block: StoredContentBlock) -> Self {
        match block {
            StoredContentBlock::Text { text } => Self::Text { text },
            StoredContentBlock::Json { value } => Self::Json { value },
            StoredContentBlock::ToolCall {
                call: ObjectOnly(call),
            } => Self::ToolCall {
                call: ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: call.arguments,
                },
            },
            StoredContentBlock::ToolResult {
                call_id,
                output: ObjectOnly(output),
            } => Self::ToolResult {
                call_id,
                output: ToolOutput {
                    content: output.content,
                    is_error: output.is_error,
                },
            },
        }
    }
}

pub(crate) fn validate_record_json(record: &SessionRecord) -> Result<(), ()> {
    let roots = record
        .metadata
        .values()
        .chain(record.messages.iter().flat_map(|message| {
            message.content.iter().filter_map(|block| match block {
                ContentBlock::Json { value } => Some(value),
                ContentBlock::ToolCall { call } => Some(&call.arguments),
                ContentBlock::ToolResult { output, .. } => Some(&output.content),
                _ => None,
            })
        }));
    let mut budget = JsonValidationBudget { nodes: 0 };
    for root in roots {
        budget.validate(root)?;
    }
    Ok(())
}

struct JsonValidationBudget {
    nodes: usize,
}

impl JsonValidationBudget {
    fn validate(&mut self, root: &Value) -> Result<(), ()> {
        let mut frames = Vec::<JsonFrame<'_>>::new();
        let mut current = Some((root, 0_usize));

        loop {
            if let Some((value, parent_depth)) = current.take() {
                self.nodes = self.nodes.checked_add(1).ok_or(())?;
                if self.nodes > MAX_STORED_JSON_NODES {
                    return Err(());
                }
                let children = match value {
                    Value::Array(values) => Some(JsonChildren::Array(values.iter())),
                    Value::Object(values) => Some(JsonChildren::Object(values.values())),
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
                };
                if let Some(children) = children {
                    let container_depth = parent_depth.checked_add(1).ok_or(())?;
                    if container_depth > MAX_STORED_JSON_DEPTH {
                        return Err(());
                    }
                    frames.push(JsonFrame {
                        container_depth,
                        children,
                    });
                }
            }

            loop {
                let Some(frame) = frames.last_mut() else {
                    return Ok(());
                };
                if let Some(child) = frame.children.next() {
                    current = Some((child, frame.container_depth));
                    break;
                }
                frames.pop();
            }
        }
    }
}

struct JsonFrame<'a> {
    container_depth: usize,
    children: JsonChildren<'a>,
}

enum JsonChildren<'a> {
    Array(std::slice::Iter<'a, Value>),
    Object(serde_json::map::Values<'a>),
}

impl<'a> Iterator for JsonChildren<'a> {
    type Item = &'a Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Array(children) => children.next(),
            Self::Object(children) => children.next(),
        }
    }
}

pub(crate) struct RecordOwner {
    record: Option<SessionRecord>,
}

impl RecordOwner {
    pub(crate) fn new(record: SessionRecord) -> Self {
        Self {
            record: Some(record),
        }
    }

    pub(crate) fn get(&self) -> &SessionRecord {
        self.record.as_ref().expect("record owner is armed")
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn get_mut(&mut self) -> &mut SessionRecord {
        self.record.as_mut().expect("record owner is armed")
    }
}

impl Drop for RecordOwner {
    fn drop(&mut self) {
        if let Some(record) = self.record.as_mut() {
            for value in std::mem::take(&mut record.metadata).into_values() {
                drop_json_value_iterative(value);
            }
            for message in &mut record.messages {
                for block in &mut message.content {
                    match block {
                        ContentBlock::Json { value } => {
                            drop_json_value_iterative(std::mem::take(value));
                        }
                        ContentBlock::ToolCall { call } => {
                            drop_json_value_iterative(std::mem::take(&mut call.arguments));
                        }
                        ContentBlock::ToolResult { output, .. } => {
                            drop_json_value_iterative(std::mem::take(&mut output.content));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

enum OwnedJsonChildren {
    Array(std::vec::IntoIter<Value>),
    Object(serde_json::map::IntoValues),
}

impl Iterator for OwnedJsonChildren {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Array(children) => children.next(),
            Self::Object(children) => children.next(),
        }
    }
}

fn drop_json_value_iterative(root: Value) {
    let mut frames = Vec::<OwnedJsonChildren>::new();
    let mut current = Some(root);
    loop {
        if let Some(value) = current.take() {
            match value {
                Value::Array(values) => frames.push(OwnedJsonChildren::Array(values.into_iter())),
                Value::Object(values) => {
                    frames.push(OwnedJsonChildren::Object(values.into_values()));
                }
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }
        loop {
            let Some(frame) = frames.last_mut() else {
                return;
            };
            if let Some(child) = frame.next() {
                current = Some(child);
                break;
            }
            frames.pop();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn corrupt() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Corrupt,
        "file_session_corrupt",
        "stored session is corrupt",
        false,
    )
}

fn unavailable(retryable: bool) -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Unavailable,
        "file_session_unavailable",
        "file session store is unavailable",
        retryable,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_io_error(_error: rustix::io::Errno) -> SessionStoreError {
    unavailable(true)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn revision_conflict() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Conflict,
        "revision_conflict",
        "stored session revision did not match the expected revision",
        true,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn incarnation_conflict() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Conflict,
        "incarnation_conflict",
        "stored session incarnation did not match the saved record",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn revision_exhausted() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Other,
        "revision_exhausted",
        "session revision counter was exhausted",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn too_large() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Other,
        "session_too_large",
        "serialized session exceeded the size limit",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn serialization_failed() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Other,
        "session_serialization_failed",
        "session could not be serialized",
        false,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn save_ambiguous() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Unavailable,
        "file_session_save_ambiguous",
        "file session save outcome is ambiguous",
        false,
    )
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use machine_god_core::SessionStoreErrorKind;

    use super::{
        FileSessionStore, JsonKeyFingerprint, JsonKeyTracker, JsonSummary, retry_interrupted,
    };

    static NEXT_LISTING_ROOT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn freshly_acquired_listing_descriptor_is_revalidated_after_unlink() {
        let root = loop {
            let sequence = NEXT_LISTING_ROOT.fetch_add(1, Ordering::Relaxed);
            let candidate = std::env::temp_dir().join(format!(
                "mg-session-listing-acquired-root-{}-{sequence}",
                std::process::id()
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create listing test root: {error}"),
            }
        };
        let store = FileSessionStore::open(&root).unwrap();

        let Err(error) =
            store.list_session_ids_after_directory_open(|| std::fs::remove_dir(&root).unwrap())
        else {
            panic!("an acquired descriptor unlinked before validation must be unavailable")
        };

        assert_eq!(error.kind, SessionStoreErrorKind::Unavailable);
        assert!(!root.exists());
    }

    #[test]
    fn interrupted_operations_retry_until_success_but_other_errors_return() {
        let mut attempts = 0_u8;
        let value = retry_interrupted(|| {
            attempts += 1;
            if attempts < 4 {
                Err(rustix::io::Errno::INTR)
            } else {
                Ok(17_u8)
            }
        })
        .unwrap();
        assert_eq!(value, 17);
        assert_eq!(attempts, 4);

        let mut attempts = 0_u8;
        let error = retry_interrupted(|| {
            attempts += 1;
            Err::<(), _>(rustix::io::Errno::IO)
        })
        .unwrap_err();
        assert_eq!(error, rustix::io::Errno::IO);
        assert_eq!(attempts, 1);
    }

    #[test]
    fn json_key_digest_collision_fails_closed() {
        let mut tracker = JsonKeyTracker::new();
        let Ok(scope) = tracker.begin_scope() else {
            panic!("first JSON key scope must be available")
        };
        let first = JsonKeyFingerprint {
            digest: [7; 32],
            verifier: [11; 32],
            length: 4,
        };
        let collision = JsonKeyFingerprint {
            digest: [7; 32],
            verifier: [13; 32],
            length: 4,
        };
        assert!(tracker.upsert(scope, first, JsonSummary::scalar()).is_ok());
        assert!(
            tracker
                .upsert(scope, collision, JsonSummary::scalar())
                .is_err()
        );
    }
}
