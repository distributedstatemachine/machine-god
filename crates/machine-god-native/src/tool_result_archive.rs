//! Immutable worker-only storage for complete compact tool outputs.
//! The injected descriptor is a dedicated private archive root. Cooperating
//! publishers use its permanent lock; this is not isolation from hostile same-UID code.
//! No path discovery, automatic eviction, result deserialization or daemon lives here.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use machine_god_core::{CancellationToken, ToolContext};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{AtFlags, Dir, FileType, FlockOperation, Mode, OFlags, RenameFlags};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Full terminal result plus its compact `ToolOutput` wrapper.
pub const TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES: usize = 210_763_712 + 64;
/// Independent physical/logical archive budget, including interrupted publications.
pub const TOOL_RESULT_ARCHIVE_MAX_BYTES: u64 = 512 * 1024 * 1024;
/// Bounded root inventory; this is not a separate eviction policy.
pub const TOOL_RESULT_ARCHIVE_MAX_ENTRIES: usize = 8192;
/// Maximum UTF-8-safe page payload.
pub const TOOL_RESULT_ARCHIVE_MAX_PAGE_BYTES: usize = 16 * 1024;
/// Opaque archive handles are distinct from legacy transcript-preview handles.
pub const TOOL_RESULT_ARCHIVE_HANDLE_PREFIX: &str = "tool-archive-v1-";
const CHUNK_BYTES: usize = 64 * 1024;
const HEADER_BYTES: usize = 52;
const MAX_CHUNKS: usize = TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES.div_ceil(CHUNK_BYTES);
const MAX_INDEX_BYTES: usize = HEADER_BYTES + MAX_CHUNKS * 32;
const MAX_FILE_BYTES: usize = MAX_INDEX_BYTES + TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES;
const ENTRY_MARGIN: u64 = 64 * 1024;
const LOCK: &str = "archive-lock-v1";
const MAGIC: &[u8; 8] = b"MGAR0001";
const FILE_MODE: Mode = Mode::from_raw_mode(0o600);

/// Fixed, data-free failures. A failed publication never returns an advertised handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolResultArchiveError {
    Invalid,
    Denied,
    NotFound,
    Busy,
    Corrupt,
    Capacity,
    Unavailable,
    Cancelled,
}
impl fmt::Display for ToolResultArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid tool result archive request",
            Self::Denied => "tool result archive scope mismatch",
            Self::NotFound => "tool result archive entry unavailable",
            Self::Busy => "tool result archive busy",
            Self::Corrupt => "tool result archive corrupt",
            Self::Capacity => "tool result archive capacity exceeded",
            Self::Unavailable => "tool result archive operation unavailable",
            Self::Cancelled => "tool result archive publication cancelled",
        })
    }
}
impl std::error::Error for ToolResultArchiveError {}
type Result<T> = std::result::Result<T, ToolResultArchiveError>;

/// Content/index and exact source-context identity; not caller authority by itself.
#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ToolResultArchiveHandle(String);
impl ToolResultArchiveHandle {
    /// Parses only the bounded canonical spelling; performs no lookup.
    /// # Errors
    /// Rejects malformed, noncanonical or oversized handles.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let suffix = value
            .strip_prefix(TOOL_RESULT_ARCHIVE_HANDLE_PREFIX)
            .ok_or(ToolResultArchiveError::Invalid)?;
        if suffix.len() != 129
            || suffix.as_bytes()[64] != b'-'
            || !is_hex(&suffix[..64])
            || !is_hex(&suffix[65..])
        {
            return Err(ToolResultArchiveError::Invalid);
        }
        Ok(Self(value))
    }
    /// Returns its canonical opaque spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    fn suffix(&self) -> &str {
        &self.0[TOOL_RESULT_ARCHIVE_HANDLE_PREFIX.len()..]
    }
    fn filename(&self) -> String {
        format!("entry-{}", self.suffix())
    }
}
impl<'de> Deserialize<'de> for ToolResultArchiveHandle {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Receipt returned only after immutable file and directory durability barriers.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivedToolResult {
    pub handle: ToolResultArchiveHandle,
    pub source_total_bytes: usize,
    /// Host retains this in its durable reference. A reader must separately verify
    /// that its current session/incarnation owns this original source context.
    pub source_context: ToolContext,
}

/// A one-based, inclusive byte page. At EOF `start_byte == total + 1` and
/// `end_byte == total`; text is empty. `truncated` means some source bytes are omitted.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResultArchivePage {
    pub source_total_bytes: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub truncated: bool,
    pub text: String,
}

/// Dedicated retained root capability. Creation takes ownership but is inert.
pub struct ToolResultArchive {
    root: OwnedFd,
}
macro_rules! redacted {
    ($($name:ty),+) => {$(impl fmt::Debug for $name {
        fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result { f.debug_struct(stringify!($name)).finish_non_exhaustive() }
    })+};
}
redacted!(
    ToolResultArchiveHandle,
    ArchivedToolResult,
    ToolResultArchivePage,
    ToolResultArchive
);

impl ToolResultArchive {
    /// Takes a dedicated host-prepared root descriptor without performing I/O.
    #[must_use]
    pub const fn from_root_descriptor(root: OwnedFd) -> Self {
        Self { root }
    }

    /// Initializes/validates the permanent private lock on a native worker.
    /// It does not create the injected root or discover any ambient path.
    /// # Errors
    /// Rejects invalid root/lock metadata or unavailable durability operations.
    pub fn prepare(&self) -> Result<()> {
        private(&self.root, true)?;
        let lock = match create_file(&self.root, LOCK) {
            Ok(lock) => lock,
            Err(ToolResultArchiveError::Busy) => open_file(&self.root, LOCK, OFlags::RDWR)?,
            Err(error) => return Err(error),
        };
        validate_lock(&self.root, &lock)?;
        sync(&lock)?;
        sync(&self.root)
    }

    /// Publishes trusted, already-serialized compact `ToolOutput` UTF-8 bytes.
    /// This storage boundary does not deserialize or reinterpret the JSON.
    /// Cancellation wins before rename; after rename publication durability wins.
    /// # Errors
    /// Rejects invalid bytes, cancelled work, contention, quota refusal or I/O failure.
    /// Unadvertised crash leftovers remain charged; existing results are never evicted.
    pub fn publish(
        &self,
        context: &ToolContext,
        compact: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<ArchivedToolResult> {
        if compact.is_empty()
            || compact.len() > TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES
            || std::str::from_utf8(compact).is_err()
        {
            return Err(ToolResultArchiveError::Invalid);
        }
        cancelled(cancellation)?;
        let root = private(&self.root, true)?;
        let scope = scope_digest(context, &root);
        let index = make_index(&scope, compact, cancellation)?;
        let handle = ToolResultArchiveHandle::parse(format!(
            "{TOOL_RESULT_ARCHIVE_HANDLE_PREFIX}{}-{}",
            hex(&scope),
            hex(&Sha256::digest(&index))
        ))?;
        let lock = acquire(&self.root, true)?;
        let usage = inventory(&self.root, TOOL_RESULT_ARCHIVE_MAX_ENTRIES)?;
        match open_file(&self.root, &handle.filename(), OFlags::RDONLY) {
            Ok(file) => {
                let existing = load_index(&file, &handle, &scope)?;
                verify_all(&file, &existing, cancellation)?;
                cancelled(cancellation)?;
                sync(&file)?;
                sync(&self.root)?;
                same_entry(&self.root, &handle.filename(), &file)?;
                validate_lock(&self.root, &lock.0)?;
                return Ok(receipt(context, handle, compact.len()));
            }
            Err(ToolResultArchiveError::NotFound) => {}
            Err(error) => return Err(error),
        }
        let planned = index
            .len()
            .checked_add(compact.len())
            .ok_or(ToolResultArchiveError::Capacity)?;
        if usage.entries == TOOL_RESULT_ARCHIVE_MAX_ENTRIES
            || usage
                .bytes
                .checked_add(planned as u64)
                .and_then(|n| n.checked_add(ENTRY_MARGIN))
                .is_none_or(|total| total > TOOL_RESULT_ARCHIVE_MAX_BYTES)
        {
            return Err(ToolResultArchiveError::Capacity);
        }
        cancelled(cancellation)?;
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| ToolResultArchiveError::Unavailable)?;
        let temporary = format!("pending-{}", hex(&nonce));
        let file = create_file(&self.root, &temporary)?;
        let mut staged = Staged {
            root: &self.root,
            name: temporary,
            file,
            published: false,
        };
        // The whole logical reservation survives an interrupted write. No second
        // copy is allocated by the final no-replace rename.
        rustix::fs::ftruncate(&staged.file, planned as u64).map_err(io_error)?;
        write_at(&staged.file, 0, &index)?;
        for (chunk, bytes) in compact.chunks(CHUNK_BYTES).enumerate() {
            cancelled(cancellation)?;
            write_at(&staged.file, index.len() + chunk * CHUNK_BYTES, bytes)?;
            // This newly created descriptor was privately validated. Quota
            // polling needs allocation metadata, not another ACL traversal per
            // 64 KiB. Full private/binding validation precedes publication.
            if usage
                .bytes
                .checked_add(charged(
                    &rustix::fs::fstat(&staged.file).map_err(io_error)?,
                )?)
                .is_none_or(|total| total > TOOL_RESULT_ARCHIVE_MAX_BYTES)
            {
                return Err(ToolResultArchiveError::Capacity);
            }
        }
        sync(&staged.file)?;
        usage.check_added(&self.root, &staged.file)?;
        same_entry(&self.root, &staged.name, &staged.file)?;
        validate_lock(&self.root, &lock.0)?;
        fault(Fault::BeforeRename)?;
        cancelled(cancellation)?;
        rustix::fs::renameat_with(
            &self.root,
            &staged.name,
            &self.root,
            handle.filename(),
            RenameFlags::NOREPLACE,
        )
        .map_err(io_error)?;
        staged.published = true;
        fault(Fault::AfterRename)?;
        sync(&self.root)?;
        usage.check_added(&self.root, &staged.file)?;
        same_entry(&self.root, &handle.filename(), &staged.file)?;
        validate_lock(&self.root, &lock.0)?;
        Ok(receipt(context, handle, compact.len()))
    }

    /// Reads a verified page using the ORIGINAL publishing context. Caller
    /// authorization must establish current reader ownership before this call.
    /// Only the <=104 KiB index and at most two 64 KiB payload chunks are read.
    /// # Errors
    /// Rejects scope mismatch, malformed bounds, a non-character-boundary start,
    /// tampered index/needed chunks, invalid metadata or unavailable I/O.
    pub fn read(
        &self,
        context: &ToolContext,
        handle: &ToolResultArchiveHandle,
        start_byte: usize,
        max_bytes: usize,
    ) -> Result<ToolResultArchivePage> {
        if start_byte == 0
            || start_byte > TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES + 1
            || !(4..=TOOL_RESULT_ARCHIVE_MAX_PAGE_BYTES).contains(&max_bytes)
        {
            return Err(ToolResultArchiveError::Invalid);
        }
        let root = private(&self.root, true)?;
        let scope = scope_digest(context, &root);
        if hex(&scope) != handle.suffix()[..64] {
            return Err(ToolResultArchiveError::Denied);
        }
        let lock = acquire(&self.root, false)?;
        let file = open_file(&self.root, &handle.filename(), OFlags::RDONLY)?;
        let index = load_index(&file, handle, &scope)?;
        let start = start_byte - 1;
        if start > index.total {
            return Err(ToolResultArchiveError::Invalid);
        }
        let mut end = start.saturating_add(max_bytes).min(index.total);
        let text = if start == index.total {
            String::new()
        } else {
            let first = start / CHUNK_BYTES;
            let last = end.min(index.total - 1) / CHUNK_BYTES;
            let mut window = Vec::with_capacity((last - first + 1) * CHUNK_BYTES);
            for chunk in first..=last {
                window.extend(read_chunk(&file, &index, chunk)?);
            }
            let base = first * CHUNK_BYTES;
            if continuation(window[start - base]) {
                return Err(ToolResultArchiveError::Invalid);
            }
            while end < index.total && continuation(window[end - base]) {
                end -= 1;
            }
            std::str::from_utf8(&window[start - base..end - base])
                .map_err(|_| ToolResultArchiveError::Corrupt)?
                .to_owned()
        };
        same_entry(&self.root, &handle.filename(), &file)?;
        validate_lock(&self.root, &lock.0)?;
        Ok(ToolResultArchivePage {
            source_total_bytes: index.total,
            start_byte,
            end_byte: end,
            truncated: start > 0 || end < index.total,
            text,
        })
    }
}

fn receipt(
    context: &ToolContext,
    handle: ToolResultArchiveHandle,
    total: usize,
) -> ArchivedToolResult {
    ArchivedToolResult {
        handle,
        source_total_bytes: total,
        source_context: context.clone(),
    }
}
fn continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}
fn cancelled(token: &CancellationToken) -> Result<()> {
    if token.is_cancelled() {
        Err(ToolResultArchiveError::Cancelled)
    } else {
        Ok(())
    }
}
fn is_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    out
}
fn scope_digest(context: &ToolContext, root: &rustix::fs::Stat) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"machine-god-tool-archive-scope-v1\0");
    digest.update(i128::from(root.st_dev).to_be_bytes());
    digest.update(u128::from(root.st_ino).to_be_bytes());
    for value in [
        context.session_id.as_str(),
        context.session_incarnation_id.as_str(),
        context.turn_id.as_str(),
        context.call_id.as_str(),
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    digest.finalize().into()
}
fn make_index(scope: &[u8; 32], bytes: &[u8], token: &CancellationToken) -> Result<Vec<u8>> {
    let chunks = bytes.len().div_ceil(CHUNK_BYTES);
    let mut index = Vec::with_capacity(HEADER_BYTES + chunks * 32);
    index.extend(MAGIC);
    index.extend((bytes.len() as u64).to_be_bytes());
    index.extend(scope);
    index.extend(
        u32::try_from(chunks)
            .map_err(|_| ToolResultArchiveError::Invalid)?
            .to_be_bytes(),
    );
    for bytes in bytes.chunks(CHUNK_BYTES) {
        cancelled(token)?;
        index.extend(Sha256::digest(bytes));
    }
    Ok(index)
}
struct Index {
    bytes: Vec<u8>,
    total: usize,
}
fn load_index(file: &OwnedFd, handle: &ToolResultArchiveHandle, scope: &[u8; 32]) -> Result<Index> {
    let mut header = [0u8; HEADER_BYTES];
    read_at(file, 0, &mut header)?;
    if &header[..8] != MAGIC || &header[16..48] != scope {
        return Err(ToolResultArchiveError::Corrupt);
    }
    let total = usize::try_from(u64::from_be_bytes(
        header[8..16]
            .try_into()
            .map_err(|_| ToolResultArchiveError::Corrupt)?,
    ))
    .map_err(|_| ToolResultArchiveError::Corrupt)?;
    let chunks = u32::from_be_bytes(
        header[48..52]
            .try_into()
            .map_err(|_| ToolResultArchiveError::Corrupt)?,
    ) as usize;
    if total == 0
        || total > TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES
        || chunks != total.div_ceil(CHUNK_BYTES)
    {
        return Err(ToolResultArchiveError::Corrupt);
    }
    let mut bytes = vec![0; HEADER_BYTES + chunks * 32];
    bytes[..HEADER_BYTES].copy_from_slice(&header);
    read_at(file, HEADER_BYTES, &mut bytes[HEADER_BYTES..])?;
    if hex(&Sha256::digest(&bytes)) != handle.suffix()[65..]
        || usize::try_from(private(file, false)?.st_size).ok() != Some(bytes.len() + total)
    {
        return Err(ToolResultArchiveError::Corrupt);
    }
    Ok(Index { bytes, total })
}
fn read_chunk(file: &OwnedFd, index: &Index, chunk: usize) -> Result<Vec<u8>> {
    let offset = chunk
        .checked_mul(CHUNK_BYTES)
        .ok_or(ToolResultArchiveError::Corrupt)?;
    if offset >= index.total {
        return Err(ToolResultArchiveError::Corrupt);
    }
    let mut bytes = vec![0; (index.total - offset).min(CHUNK_BYTES)];
    read_at(file, index.bytes.len() + offset, &mut bytes)?;
    if Sha256::digest(&bytes).as_slice()
        != &index.bytes[HEADER_BYTES + chunk * 32..HEADER_BYTES + (chunk + 1) * 32]
    {
        return Err(ToolResultArchiveError::Corrupt);
    }
    Ok(bytes)
}
fn verify_all(file: &OwnedFd, index: &Index, token: &CancellationToken) -> Result<()> {
    for chunk in 0..index.total.div_ceil(CHUNK_BYTES) {
        cancelled(token)?;
        read_chunk(file, index, chunk)?;
    }
    Ok(())
}

struct Inventory {
    entries: usize,
    bytes: u64,
    root_bytes: u64,
}
impl Inventory {
    fn check_added(&self, root: &OwnedFd, file: &OwnedFd) -> Result<()> {
        let root_bytes = allocated(&private(root, true)?)?;
        let file_bytes = charged(&private(file, false)?)?;
        if self
            .bytes
            .checked_sub(self.root_bytes)
            .and_then(|base| base.checked_add(root_bytes))
            .and_then(|base| base.checked_add(file_bytes))
            .is_none_or(|total| total > TOOL_RESULT_ARCHIVE_MAX_BYTES)
        {
            return Err(ToolResultArchiveError::Capacity);
        }
        Ok(())
    }
}
fn inventory(root: &OwnedFd, maximum: usize) -> Result<Inventory> {
    let directory = rustix::fs::openat(
        root,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?;
    let mut directory = Dir::new(directory).map_err(io_error)?;
    let root_bytes = allocated(&private(root, true)?)?;
    let mut inventory = Inventory {
        entries: 0,
        bytes: root_bytes,
        root_bytes,
    };
    for entry in &mut directory {
        let entry = entry.map_err(io_error)?;
        let name = entry
            .file_name()
            .to_str()
            .map_err(|_| ToolResultArchiveError::Corrupt)?;
        if matches!(name, "." | "..") {
            continue;
        }
        if name == LOCK {
            let lock = open_file(root, name, OFlags::RDONLY)?;
            validate_lock(root, &lock)?;
            inventory.bytes = inventory
                .bytes
                .checked_add(allocated(&private(&lock, false)?)?)
                .filter(|n| *n <= TOOL_RESULT_ARCHIVE_MAX_BYTES)
                .ok_or(ToolResultArchiveError::Capacity)?;
            continue;
        }
        if inventory.entries == maximum {
            return Err(ToolResultArchiveError::Capacity);
        }
        inventory.entries += 1;
        if !(name
            .strip_prefix("pending-")
            .is_some_and(|suffix| suffix.len() == 32 && is_hex(suffix))
            || name.strip_prefix("entry-").is_some_and(|suffix| {
                suffix.len() == 129
                    && suffix.as_bytes()[64] == b'-'
                    && is_hex(&suffix[..64])
                    && is_hex(&suffix[65..])
            }))
        {
            return Err(ToolResultArchiveError::Corrupt);
        }
        let file = open_file(root, name, OFlags::RDONLY)?;
        let stat = private(&file, false)?;
        if usize::try_from(stat.st_size).map_or(true, |size| size > MAX_FILE_BYTES) {
            return Err(ToolResultArchiveError::Corrupt);
        }
        inventory.bytes = inventory
            .bytes
            .checked_add(charged(&stat)?)
            .filter(|n| *n <= TOOL_RESULT_ARCHIVE_MAX_BYTES)
            .ok_or(ToolResultArchiveError::Capacity)?;
    }
    Ok(inventory)
}
fn allocated(stat: &rustix::fs::Stat) -> Result<u64> {
    u64::try_from(stat.st_blocks)
        .ok()
        .and_then(|n| n.checked_mul(512))
        .ok_or(ToolResultArchiveError::Corrupt)
}
fn charged(stat: &rustix::fs::Stat) -> Result<u64> {
    u64::try_from(stat.st_size)
        .map_err(|_| ToolResultArchiveError::Corrupt)?
        .max(allocated(stat)?)
        .checked_add(ENTRY_MARGIN)
        .ok_or(ToolResultArchiveError::Capacity)
}
fn private(fd: impl AsFd, directory: bool) -> Result<rustix::fs::Stat> {
    let stat = rustix::fs::fstat(fd.as_fd()).map_err(io_error)?;
    let kind = FileType::from_raw_mode(stat.st_mode);
    let valid = stat.st_uid == rustix::process::geteuid().as_raw()
        && if directory {
            kind.is_dir() && u64::from(stat.st_mode) & 0o7777 == 0o700 && stat.st_nlink > 0
        } else {
            kind.is_file()
                && u64::from(stat.st_mode) & 0o7777 == 0o600
                && stat.st_nlink == 1
                && stat.st_size >= 0
        };
    if !valid {
        return Err(ToolResultArchiveError::Corrupt);
    }
    #[cfg(target_os = "macos")]
    {
        let acl = calcifer_macos_acl::read_acl(fd.as_fd())
            .map_err(|_| ToolResultArchiveError::Unavailable)?;
        if acl.flags != 0
            || !acl.entries.iter().all(|entry| {
                entry.tag == calcifer_macos_acl::TAG_DENY
                    && entry.flags == 0
                    && entry.permissions == calcifer_macos_acl::PERMISSION_DELETE
            })
        {
            return Err(ToolResultArchiveError::Corrupt);
        }
    }
    Ok(stat)
}
fn same_entry(root: &OwnedFd, name: &str, file: &OwnedFd) -> Result<()> {
    let actual = rustix::fs::statat(root, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    let retained = private(file, false)?;
    if actual.st_dev != retained.st_dev || actual.st_ino != retained.st_ino {
        Err(ToolResultArchiveError::Corrupt)
    } else {
        Ok(())
    }
}
fn open_file(root: &OwnedFd, name: &str, access: OFlags) -> Result<OwnedFd> {
    let file = rustix::fs::openat(
        root,
        name,
        access | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(io_error)?;
    same_entry(root, name, &file)?;
    Ok(file)
}
fn create_file(root: &OwnedFd, name: &str) -> Result<OwnedFd> {
    let file = rustix::fs::openat(
        root,
        name,
        OFlags::RDWR
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK,
        FILE_MODE,
    )
    .map_err(io_error)?;
    rustix::fs::fchmod(&file, FILE_MODE).map_err(io_error)?;
    same_entry(root, name, &file)?;
    Ok(file)
}
fn validate_lock(root: &OwnedFd, lock: &OwnedFd) -> Result<()> {
    same_entry(root, LOCK, lock)?;
    if private(lock, false)?.st_size != 0 {
        Err(ToolResultArchiveError::Corrupt)
    } else {
        Ok(())
    }
}
struct Lock(OwnedFd);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.0, FlockOperation::Unlock);
    }
}
fn acquire(root: &OwnedFd, exclusive: bool) -> Result<Lock> {
    let lock = open_file(root, LOCK, OFlags::RDWR)?;
    validate_lock(root, &lock)?;
    rustix::fs::flock(
        &lock,
        if exclusive {
            FlockOperation::NonBlockingLockExclusive
        } else {
            FlockOperation::NonBlockingLockShared
        },
    )
    .map_err(io_error)?;
    let lock = Lock(lock);
    validate_lock(root, &lock.0)?;
    Ok(lock)
}
struct Staged<'a> {
    root: &'a OwnedFd,
    name: String,
    file: OwnedFd,
    published: bool,
}
impl Drop for Staged<'_> {
    fn drop(&mut self) {
        if !self.published && same_entry(self.root, &self.name, &self.file).is_ok() {
            let _ = rustix::fs::unlinkat(self.root, &self.name, AtFlags::empty());
            let _ = sync(self.root);
        }
    }
}
fn sync(fd: impl AsFd) -> Result<()> {
    rustix::fs::fsync(fd).map_err(io_error)
}
fn write_at(file: &OwnedFd, mut offset: usize, mut bytes: &[u8]) -> Result<()> {
    while !bytes.is_empty() {
        match rustix::io::pwrite(file, bytes, offset as u64) {
            Ok(0) => return Err(ToolResultArchiveError::Unavailable),
            Ok(count) => {
                offset += count;
                bytes = &bytes[count..];
            }
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    Ok(())
}
fn read_at(file: &OwnedFd, mut offset: usize, mut bytes: &mut [u8]) -> Result<()> {
    #[cfg(test)]
    READ_BYTES.with(|count| count.set(count.get() + bytes.len()));
    while !bytes.is_empty() {
        match rustix::io::pread(file, &mut *bytes, offset as u64) {
            Ok(0) => return Err(ToolResultArchiveError::Corrupt),
            Ok(count) => {
                offset += count;
                bytes = &mut bytes[count..];
            }
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    Ok(())
}
fn io_error(error: rustix::io::Errno) -> ToolResultArchiveError {
    match error {
        rustix::io::Errno::NOENT => ToolResultArchiveError::NotFound,
        rustix::io::Errno::WOULDBLOCK | rustix::io::Errno::EXIST => ToolResultArchiveError::Busy,
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => ToolResultArchiveError::Corrupt,
        _ => ToolResultArchiveError::Unavailable,
    }
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum Fault {
    BeforeRename,
    AfterRename,
}
fn fault(point: Fault) -> Result<()> {
    #[cfg(test)]
    CANCEL_AT.with(|cancel| {
        if cancel.borrow().as_ref().is_some_and(|(at, _)| *at == point) {
            let (_, token) = cancel.borrow_mut().take().expect("matched cancellation");
            token.cancel();
        }
    });
    #[cfg(test)]
    if FAULT.with(|fault| fault.get() == Some(point)) {
        FAULT.with(|fault| fault.set(None));
        return Err(ToolResultArchiveError::Unavailable);
    }
    let _ = point;
    Ok(())
}
#[cfg(test)]
thread_local! {
    static READ_BYTES:std::cell::Cell<usize>=const {std::cell::Cell::new(0)};
    static FAULT:std::cell::Cell<Option<Fault>>=const {std::cell::Cell::new(None)};
    static CANCEL_AT:std::cell::RefCell<Option<(Fault,CancellationToken)>>=const {std::cell::RefCell::new(None)};
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{SessionId, SessionIncarnationId, ToolCallId, TurnId};
    use std::fs;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::path::PathBuf;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let mut nonce = [0; 16];
            getrandom::fill(&mut nonce).unwrap();
            let path =
                std::env::temp_dir().join(format!("machine-god-result-archive-{}", hex(&nonce)));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
        fn archive(&self) -> ToolResultArchive {
            ToolResultArchive::from_root_descriptor(
                rustix::fs::open(
                    &self.0,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .unwrap(),
            )
        }
        fn prepared(&self) -> ToolResultArchive {
            let archive = self.archive();
            archive.prepare().unwrap();
            archive
        }
        fn names(&self) -> Vec<String> {
            let mut names = fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect::<Vec<_>>();
            names.sort();
            names
        }
        fn pending(&self, name: &str, size: u64) {
            let file = create_file(&self.archive().root, name).unwrap();
            rustix::fs::ftruncate(&file, size).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("owner").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
            turn_id: TurnId::new("turn").unwrap(),
            call_id: ToolCallId::new("call").unwrap(),
        }
    }
    fn publish(archive: &ToolResultArchive, source: &str) -> ArchivedToolResult {
        archive
            .publish(&context(), source.as_bytes(), &CancellationToken::new())
            .unwrap()
    }

    #[test]
    fn construction_and_rejected_or_cancelled_admission_do_not_write() {
        let fixture = Fixture::new();
        let archive = fixture.archive();
        assert!(fixture.names().is_empty());
        assert_eq!(
            archive
                .publish(&context(), b"", &CancellationToken::new())
                .unwrap_err(),
            ToolResultArchiveError::Invalid
        );
        assert_eq!(
            archive
                .publish(&context(), &[255], &CancellationToken::new())
                .unwrap_err(),
            ToolResultArchiveError::Invalid
        );
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            archive
                .publish(&context(), b"{}", &cancellation)
                .unwrap_err(),
            ToolResultArchiveError::Cancelled
        );
        assert!(fixture.names().is_empty());
        archive.prepare().unwrap();
        assert_eq!(fixture.names(), vec![LOCK]);
        assert_eq!(
            archive
                .publish(&context(), &[255], &CancellationToken::new())
                .unwrap_err(),
            ToolResultArchiveError::Invalid
        );
        assert_eq!(fixture.names(), vec![LOCK]);
    }

    #[test]
    fn real_publish_reopen_and_complete_utf8_paging_are_lossless() {
        let fixture = Fixture::new();
        let archive = fixture.prepared();
        let source = serde_json::to_string(&machine_god_core::ToolOutput::success(
            "é🦀界".repeat(20_000),
        ))
        .unwrap();
        let result = publish(&archive, &source);
        assert_eq!(result.source_context, context());
        assert_eq!(result.source_total_bytes, source.len());
        drop(archive);
        let archive = fixture.archive();
        let mut actual = String::new();
        let mut start = 1;
        loop {
            let page = archive
                .read(&context(), &result.handle, start, 16384)
                .unwrap();
            assert_eq!(page.start_byte, start);
            assert_eq!(page.source_total_bytes, source.len());
            assert!(page.text.len() <= 16384);
            actual.push_str(&page.text);
            if page.end_byte == source.len() {
                break;
            }
            assert!(page.end_byte >= start);
            start = page.end_byte + 1;
        }
        assert_eq!(actual, source);
        let eof = archive
            .read(&context(), &result.handle, source.len() + 1, 4)
            .unwrap();
        assert_eq!(eof.end_byte, source.len());
        assert!(eof.text.is_empty());
        assert_eq!(fixture.names().len(), 2);
    }

    #[test]
    fn every_source_context_component_and_retained_root_are_bound() {
        let fixture = Fixture::new();
        let archive = fixture.prepared();
        let result = publish(&archive, "a result");
        for field in 0..4 {
            let mut other = context();
            match field {
                0 => other.session_id = SessionId::new("other").unwrap(),
                1 => other.session_incarnation_id = SessionIncarnationId::new("other").unwrap(),
                2 => other.turn_id = TurnId::new("other").unwrap(),
                _ => other.call_id = ToolCallId::new("other").unwrap(),
            }
            assert_eq!(
                archive.read(&other, &result.handle, 1, 4).unwrap_err(),
                ToolResultArchiveError::Denied
            );
        }
        let other = Fixture::new();
        let other_archive = other.prepared();
        fs::copy(
            fixture.0.join(result.handle.filename()),
            other.0.join(result.handle.filename()),
        )
        .unwrap();
        assert_eq!(
            other_archive
                .read(&context(), &result.handle, 1, 4)
                .unwrap_err(),
            ToolResultArchiveError::Denied
        );
        assert!(!format!("{result:?}").contains("owner"));
    }

    #[test]
    fn invalid_handles_and_byte_ranges_fail_closed() {
        for value in [
            String::new(),
            "../escape".to_owned(),
            format!(
                "{TOOL_RESULT_ARCHIVE_HANDLE_PREFIX}{}-{}",
                "A".repeat(64),
                "a".repeat(64)
            ),
            format!(
                "{TOOL_RESULT_ARCHIVE_HANDLE_PREFIX}{}-{}",
                "界".repeat(21),
                "a".repeat(65)
            ),
        ] {
            assert!(ToolResultArchiveHandle::parse(value).is_err());
        }
        let fixture = Fixture::new();
        let archive = fixture.prepared();
        let result = publish(&archive, "aé🦀z");
        for (start, size) in [
            (0, 4),
            (3, 4),
            (5, 4),
            (1, 3),
            (1, 16385),
            (10, 4),
            (usize::MAX, 4),
        ] {
            assert_eq!(
                archive
                    .read(&context(), &result.handle, start, size)
                    .unwrap_err(),
                ToolResultArchiveError::Invalid
            );
        }
        let page = archive.read(&context(), &result.handle, 1, 4).unwrap();
        assert_eq!(page.text, "aé");
        assert_eq!(page.end_byte, 3);
        assert!(page.truncated);
        assert_eq!(
            archive.read(&context(), &result.handle, 4, 4).unwrap().text,
            "🦀"
        );
    }

    #[test]
    fn page_io_is_bounded_and_only_needed_chunks_are_verified() {
        let fixture = Fixture::new();
        let archive = fixture.prepared();
        let source = "x".repeat(8 * 1024 * 1024);
        let result = publish(&archive, &source);
        READ_BYTES.with(|n| n.set(0));
        let page = archive
            .read(&context(), &result.handle, CHUNK_BYTES - 3, 16384)
            .unwrap();
        assert_eq!(page.text.len(), 16384);
        let read = READ_BYTES.with(std::cell::Cell::get);
        assert!(read <= MAX_INDEX_BYTES + 2 * CHUNK_BYTES);
        assert!(read < source.len() / 10);
        let file = open_file(&archive.root, &result.handle.filename(), OFlags::RDWR).unwrap();
        let index_size = HEADER_BYTES + source.len().div_ceil(CHUNK_BYTES) * 32;
        write_at(&file, index_size + 6 * CHUNK_BYTES, b"T").unwrap();
        // Unrequested payload is not scanned. Its own requested page must fail.
        assert_eq!(
            archive.read(&context(), &result.handle, 1, 4).unwrap().text,
            "xxxx"
        );
        assert_eq!(
            archive
                .read(&context(), &result.handle, 6 * CHUNK_BYTES + 1, 4)
                .unwrap_err(),
            ToolResultArchiveError::Corrupt
        );
    }

    #[test]
    fn maximum_source_is_preserved_and_plus_one_is_rejected_before_publication() {
        let fixture = Fixture::new();
        let archive = fixture.prepared();
        let mut source = vec![b'x'; TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES + 1];
        assert_eq!(
            archive
                .publish(&context(), &source, &CancellationToken::new())
                .unwrap_err(),
            ToolResultArchiveError::Invalid
        );
        assert_eq!(fixture.names(), vec![LOCK]);
        source.pop();
        let result = archive
            .publish(&context(), &source, &CancellationToken::new())
            .unwrap();
        assert_eq!(
            result.source_total_bytes,
            TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES
        );
        READ_BYTES.with(|n| n.set(0));
        let page = archive
            .read(
                &context(),
                &result.handle,
                TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES - 31,
                16384,
            )
            .unwrap();
        assert_eq!(page.text, "x".repeat(32));
        assert_eq!(page.end_byte, TOOL_RESULT_ARCHIVE_MAX_SOURCE_BYTES);
        assert!(READ_BYTES.with(std::cell::Cell::get) <= MAX_INDEX_BYTES + CHUNK_BYTES);
        assert!(
            inventory(&archive.root, TOOL_RESULT_ARCHIVE_MAX_ENTRIES)
                .unwrap()
                .bytes
                <= TOOL_RESULT_ARCHIVE_MAX_BYTES
        );
    }

    #[test]
    fn index_header_payload_and_file_length_tampering_are_detected() {
        for location in [0, 8, 16, 48, HEADER_BYTES, HEADER_BYTES + 32] {
            let fixture = Fixture::new();
            let archive = fixture.prepared();
            let result = publish(&archive, "payload");
            let file = open_file(&archive.root, &result.handle.filename(), OFlags::RDWR).unwrap();
            let mut byte = [0];
            read_at(&file, location, &mut byte).unwrap();
            byte[0] ^= 1;
            write_at(&file, location, &byte).unwrap();
            assert_eq!(
                archive.read(&context(), &result.handle, 1, 4).unwrap_err(),
                ToolResultArchiveError::Corrupt
            );
        }
        let fixture = Fixture::new();
        let archive = fixture.prepared();
        let result = publish(&archive, "payload");
        let file = open_file(&archive.root, &result.handle.filename(), OFlags::RDWR).unwrap();
        rustix::fs::ftruncate(&file, MAX_FILE_BYTES as u64 + 1).unwrap();
        assert_eq!(
            archive.read(&context(), &result.handle, 1, 4).unwrap_err(),
            ToolResultArchiveError::Corrupt
        );
    }

    #[test]
    fn identical_publication_is_idempotent_and_never_overwrites_corruption() {
        let fixture = Fixture::new();
        let archive = fixture.prepared();
        let first = publish(&archive, "payload");
        let second = publish(&archive, "payload");
        assert_eq!(first.handle, second.handle);
        assert_eq!(fixture.names().len(), 2);
        let file = open_file(&archive.root, &first.handle.filename(), OFlags::RDWR).unwrap();
        write_at(&file, HEADER_BYTES + 32, b"bad").unwrap();
        assert_eq!(
            archive
                .publish(&context(), b"payload", &CancellationToken::new())
                .unwrap_err(),
            ToolResultArchiveError::Corrupt
        );
        let mut actual = [0; 3];
        read_at(&file, HEADER_BYTES + 32, &mut actual).unwrap();
        assert_eq!(&actual, b"bad");
        assert_eq!(fixture.names().len(), 2);
    }

    #[test]
    fn quota_includes_sparse_crash_reservations_and_never_evicts() {
        let fixture = Fixture::new();
        let archive = fixture.prepared();
        for i in 0..2 {
            fixture.pending(&format!("pending-{i:032x}"), MAX_FILE_BYTES as u64);
        }
        let remaining = TOOL_RESULT_ARCHIVE_MAX_BYTES - 2 * (MAX_FILE_BYTES as u64 + ENTRY_MARGIN);
        fixture.pending(&format!("pending-{:032x}", 3), remaining - ENTRY_MARGIN);
        let before = fixture.names();
        assert_eq!(
            archive
                .publish(&context(), b"new result", &CancellationToken::new())
                .unwrap_err(),
            ToolResultArchiveError::Capacity
        );
        assert_eq!(fixture.names(), before);
        assert!(matches!(
            inventory(&archive.root, 2),
            Err(ToolResultArchiveError::Capacity)
        ));
    }

    #[test]
    fn unknown_files_symlinks_hardlinks_and_nonprivate_entries_fail_closed() {
        let fixture = Fixture::new();
        let archive = fixture.prepared();
        fs::write(fixture.0.join("unknown"), "unaccounted").unwrap();
        let before = fixture.names();
        assert_eq!(
            archive
                .publish(&context(), b"payload", &CancellationToken::new())
                .unwrap_err(),
            ToolResultArchiveError::Corrupt
        );
        assert_eq!(fixture.names(), before);
        for kind in 0..3 {
            let fixture = Fixture::new();
            let archive = fixture.prepared();
            let result = publish(&archive, "payload");
            let path = fixture.0.join(result.handle.filename());
            match kind {
                0 => {
                    fs::remove_file(&path).unwrap();
                    std::os::unix::fs::symlink("archive-lock-v1", &path).unwrap();
                }
                1 => fs::hard_link(&path, fixture.0.join("second-link")).unwrap(),
                _ => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            }
            assert_eq!(
                archive.read(&context(), &result.handle, 1, 4).unwrap_err(),
                ToolResultArchiveError::Corrupt
            );
        }
    }

    #[test]
    fn publication_failures_withhold_handles_cleanup_temp_and_retry_directory_barrier() {
        for point in [Fault::BeforeRename, Fault::AfterRename] {
            let fixture = Fixture::new();
            let archive = fixture.prepared();
            FAULT.with(|fault| fault.set(Some(point)));
            assert_eq!(
                archive
                    .publish(&context(), b"payload", &CancellationToken::new())
                    .unwrap_err(),
                ToolResultArchiveError::Unavailable
            );
            let names = fixture.names();
            assert!(!names.iter().any(|name| name.starts_with("pending-")));
            assert_eq!(
                names.len(),
                if point == Fault::BeforeRename { 1 } else { 2 }
            );
            let result = publish(&archive, "payload");
            assert_eq!(
                archive
                    .read(&context(), &result.handle, 1, 16)
                    .unwrap()
                    .text,
                "payload"
            );
            assert_eq!(fixture.names().len(), 2);
        }
    }

    #[test]
    fn cancellation_before_commit_cleans_temp_but_after_commit_returns_durable_receipt() {
        for point in [Fault::BeforeRename, Fault::AfterRename] {
            let fixture = Fixture::new();
            let archive = fixture.prepared();
            let token = CancellationToken::new();
            CANCEL_AT.with(|at| *at.borrow_mut() = Some((point, token.clone())));
            let result = archive.publish(&context(), b"payload", &token);
            assert!(token.is_cancelled());
            if point == Fault::BeforeRename {
                assert_eq!(result.unwrap_err(), ToolResultArchiveError::Cancelled);
                assert_eq!(fixture.names(), vec![LOCK]);
            } else {
                let result = result.unwrap();
                assert_eq!(
                    archive
                        .read(&context(), &result.handle, 1, 16)
                        .unwrap()
                        .text,
                    "payload"
                );
            }
        }
    }

    #[test]
    fn concurrent_descriptor_instances_observe_lock_and_idempotent_publication() {
        let fixture = Fixture::new();
        let first = fixture.prepared();
        let second = fixture.archive();
        let lock = acquire(&first.root, true).unwrap();
        assert_eq!(
            second
                .publish(&context(), b"payload", &CancellationToken::new())
                .unwrap_err(),
            ToolResultArchiveError::Busy
        );
        assert_eq!(fixture.names(), vec![LOCK]);
        drop(lock);
        let result = publish(&second, "payload");
        let lock = acquire(&first.root, true).unwrap();
        assert_eq!(
            second.read(&context(), &result.handle, 1, 4).unwrap_err(),
            ToolResultArchiveError::Busy
        );
        drop(lock);
        assert_eq!(publish(&first, "payload").handle, result.handle);
    }
}
