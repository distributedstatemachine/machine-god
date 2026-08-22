use std::error::Error;
use std::fmt;
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::BTreeMap;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::io::{self, Write};

use machine_god_core::{
    BoxFuture, ContentBlock, SessionId, SessionRecord, SessionRevision, SessionStore,
    SessionStoreError, SessionStoreErrorKind,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_core::{
    Message, Role, SessionIncarnationId, ToolCall, ToolCallId, ToolName, ToolOutput,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{AtFlags, FileType, FlockOperation, Mode, OFlags};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use sha2::{Digest, Sha256};

/// Schema version written by [`FileSessionStore`].
pub const FILE_SESSION_SCHEMA_VERSION: u32 = 1;

/// Maximum number of serialized bytes in one file session record.
pub const MAX_FILE_SESSION_BYTES: usize = 8_651_165;

#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_STORED_JSON_DEPTH: usize = 64;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_STORED_JSON_NODES: usize = 65_536;

#[cfg(any(target_os = "linux", target_os = "macos"))]
const FILE_NAME_DOMAIN: &[u8] = b"machine-god:file-session:v1:";

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
/// detached work. Their bounded filesystem I/O, advisory locking, and syncing
/// may block the thread polling the future.
pub struct FileSessionStore {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    root: OwnedFd,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unsupported: std::convert::Infallible,
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
        let bytes = serialize_record(record)?;
        let temp = create_temp(self.root.as_fd(), &names.temp)?;
        if let Err(error) = write_all(&temp, &bytes).and_then(|()| sync_file(&temp)) {
            let _ = rustix::fs::unlinkat(self.root.as_fd(), &names.temp, AtFlags::empty());
            return Err(map_io_error(error));
        }
        if let Err(error) = rustix::fs::renameat(
            self.root.as_fd(),
            &names.temp,
            self.root.as_fd(),
            &names.data,
        ) {
            let _ = rustix::fs::unlinkat(self.root.as_fd(), &names.temp, AtFlags::empty());
            return Err(map_io_error(error));
        }
        if sync_file(&self.root).is_err() {
            return Err(save_ambiguous());
        }
        Ok(revision)
    }
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
fn read_record(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
    expected_id: &SessionId,
) -> Result<Option<SessionRecord>, SessionStoreError> {
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
        || u64::try_from(metadata.st_size).unwrap_or(u64::MAX) > MAX_FILE_SESSION_BYTES as u64
    {
        return Err(corrupt());
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.st_size)
            .unwrap_or(MAX_FILE_SESSION_BYTES)
            .min(MAX_FILE_SESSION_BYTES),
    );
    let mut chunk = [0_u8; 8192];
    loop {
        let remaining = (MAX_FILE_SESSION_BYTES + 1).saturating_sub(bytes.len());
        if remaining == 0 {
            return Err(corrupt());
        }
        let chunk_limit = remaining.min(chunk.len());
        let read = retry_interrupted(|| rustix::io::read(&file, &mut chunk[..chunk_limit]))
            .map_err(map_io_error)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > MAX_FILE_SESSION_BYTES {
            return Err(corrupt());
        }
    }
    let envelope: StoredEnvelope = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
    if envelope.schema_version != FILE_SESSION_SCHEMA_VERSION {
        return Err(corrupt());
    }
    let record = SessionRecord::from(envelope.record);
    if &record.id != expected_id
        || record.revision == SessionRevision(0)
        || record.next_turn_sequence == 0
        || validate_record_json(&record).is_err()
    {
        return Err(corrupt());
    }
    Ok(Some(record))
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
fn sync_file(file: &OwnedFd) -> Result<(), rustix::io::Errno> {
    retry_interrupted(|| rustix::fs::fsync(file))
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredEnvelope {
    schema_version: u32,
    record: StoredRecord,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRecord {
    id: SessionId,
    incarnation_id: SessionIncarnationId,
    revision: SessionRevision,
    next_turn_sequence: u64,
    messages: Vec<StoredMessage>,
    metadata: BTreeMap<String, Value>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMessage {
    role: Role,
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
        call: StoredToolCall,
    },
    ToolResult {
        call_id: ToolCallId,
        output: StoredToolOutput,
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
            messages: record.messages.into_iter().map(Message::from).collect(),
            metadata: record.metadata,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl From<StoredMessage> for Message {
    fn from(message: StoredMessage) -> Self {
        Self {
            role: message.role,
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
            StoredContentBlock::ToolCall { call } => Self::ToolCall {
                call: ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: call.arguments,
                },
            },
            StoredContentBlock::ToolResult { call_id, output } => Self::ToolResult {
                call_id,
                output: ToolOutput {
                    content: output.content,
                    is_error: output.is_error,
                },
            },
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_record_json(record: &SessionRecord) -> Result<(), ()> {
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct JsonValidationBudget {
    nodes: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct JsonFrame<'a> {
    container_depth: usize,
    children: JsonChildren<'a>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum JsonChildren<'a> {
    Array(std::slice::Iter<'a, Value>),
    Object(serde_json::map::Values<'a>),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'a> Iterator for JsonChildren<'a> {
    type Item = &'a Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Array(children) => children.next(),
            Self::Object(children) => children.next(),
        }
    }
}

struct RecordOwner {
    record: Option<SessionRecord>,
}

impl RecordOwner {
    fn new(record: SessionRecord) -> Self {
        Self {
            record: Some(record),
        }
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
    use super::retry_interrupted;

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
}
