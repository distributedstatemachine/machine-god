//! Descriptor-confined terminal journal. Synchronous operations belong on a native worker.
//!
//! The permanent nonblocking lock coordinates cooperating writers, not hostile
//! same-account processes. Checksums detect corruption; they are not authentication.
//! Metadata publication is the commit point. An I/O error can be ambiguous and
//! poisons the handle until reopen/reconciliation. No persisted PID is consulted.
//! The session budget covers committed retained payload. Atomic publication
//! temporarily needs up to the submitted payload's length in additional disk
//! space, plus bounded metadata. The profile coordinator must reserve that
//! headroom before dispatch; this module does not silently enforce a global cap.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::BTreeSet;
use std::fmt;

use machine_god_core::{TerminalCursor, TerminalGap, TerminalSessionId};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{AtFlags, Dir, FileType, FlockOperation, Mode, OFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const META: &str = "tj-meta";
const TEMP: &str = "tj-meta.tmp";
const LOCK: &str = "tj-lock";
const MAX_META: usize = 128 * 1024;
const MAX_SEGMENTS: usize = 128;
const MAX_EVENTS: usize = 256;
const MAX_EVENT_BYTES: usize = 4096;
const MAX_PAGE_BYTES: usize = 64 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 1024;
const MAX_CHECKPOINT_BYTES: usize = 64 * 1024 * 1024;
const DOMAIN: &[u8] = b"machine-god:terminal-journal:v1:";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalJournalError {
    Invalid,
    NotFound,
    Busy,
    Conflict,
    Corrupt,
    ResourceLimit,
    Unavailable,
}
impl fmt::Display for TerminalJournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid terminal journal request",
            Self::NotFound => "terminal journal unavailable",
            Self::Busy => "terminal journal writer busy",
            Self::Conflict => "terminal journal already exists",
            Self::Corrupt => "terminal journal corrupt",
            Self::ResourceLimit => "terminal journal resource limit",
            Self::Unavailable => "terminal journal operation unavailable",
        })
    }
}
impl std::error::Error for TerminalJournalError {}
type Result<T> = std::result::Result<T, TerminalJournalError>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminalJournalLimits {
    pub(crate) segment_bytes: usize,
    pub(crate) session_bytes: usize,
}
impl Default for TerminalJournalLimits {
    fn default() -> Self {
        Self {
            segment_bytes: 1024 * 1024,
            session_bytes: 64 * 1024 * 1024,
        }
    }
}
impl TerminalJournalLimits {
    fn validate(self) -> Result<()> {
        ensure(
            self.segment_bytes > 0
                && self.segment_bytes <= 1024 * 1024
                && self.session_bytes >= self.segment_bytes
                && self.session_bytes <= MAX_CHECKPOINT_BYTES
                && self.session_bytes.div_ceil(self.segment_bytes) <= MAX_SEGMENTS,
            TerminalJournalError::Invalid,
        )
    }
}

macro_rules! redacted {
    ($name:ident) => {
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    };
}

pub(crate) struct TerminalJournalPage {
    pub(crate) bytes: Vec<u8>,
    pub(crate) next: TerminalCursor,
    pub(crate) earliest: TerminalCursor,
    pub(crate) latest: TerminalCursor,
    pub(crate) gap: Option<TerminalGap>,
}
redacted!(TerminalJournalPage);

pub(crate) struct TerminalJournalCheckpoint {
    pub(crate) source: TerminalCursor,
    pub(crate) bytes: Vec<u8>,
}
redacted!(TerminalJournalCheckpoint);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalJournalCheckpointStatus {
    Missing,
    Available,
    RetentionEvicted,
}

pub(crate) struct TerminalJournalEvent {
    pub(crate) id: u64,
    pub(crate) payload: Vec<u8>,
}
redacted!(TerminalJournalEvent);

pub(crate) struct TerminalJournalEvents {
    pub(crate) events: Vec<TerminalJournalEvent>,
    pub(crate) gap_through: u64,
    pub(crate) next_event_id: u64,
    pub(crate) acknowledged_through: u64,
}
redacted!(TerminalJournalEvents);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(
    clippy::struct_field_names,
    reason = "accounting field names retain explicit byte units"
)]
pub(crate) struct TerminalJournalUsage {
    pub(crate) raw_bytes: usize,
    pub(crate) checkpoint_bytes: usize,
    pub(crate) event_bytes: usize,
    pub(crate) payload_bytes: usize,
    /// Encoded metadata bytes, separate from the profile payload budget.
    pub(crate) metadata_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminalJournalRecovery {
    pub(crate) discarded_uncommitted_bytes: u64,
    pub(crate) removed_orphan_files: usize,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Blob {
    id: u64,
    bytes: usize,
    sha256: [u8; 32],
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    blob: Blob,
    source: TerminalCursor,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    session: TerminalSessionId,
    limits: TerminalJournalLimits,
    generation: u64,
    next_segment: u64,
    latest: TerminalCursor,
    segments: Vec<Blob>,
    checkpoint: Option<Checkpoint>,
    checkpoint_evicted: bool,
    next_event: u64,
    acknowledged: u64,
    event_gap: u64,
    events: Vec<Blob>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    manifest: Manifest,
    sha256: [u8; 32],
}

/// One exclusive writer and its snapshot readers. Profile-wide accounting is external.
pub(crate) struct TerminalJournal {
    root: OwnedFd,
    lock: OwnedFd,
    manifest: Manifest,
    current_hash: Option<(u64, Sha256)>,
    metadata_bytes: usize,
    recovery: TerminalJournalRecovery,
    poisoned: bool,
    pending_files: BTreeSet<String>,
}
redacted!(TerminalJournal);

impl TerminalJournal {
    pub(crate) fn session_id(&self) -> &TerminalSessionId {
        &self.manifest.session
    }

    pub(crate) fn create(
        root: OwnedFd,
        session: TerminalSessionId,
        limits: TerminalJournalLimits,
    ) -> Result<Self> {
        limits.validate()?;
        private(&root, true)?;
        let lock = writer_lock(&root, true)?;
        match open_file(&root, META, OFlags::RDONLY) {
            Ok(_) => return Err(TerminalJournalError::Conflict),
            Err(TerminalJournalError::NotFound) => {}
            Err(e) => return Err(e),
        }
        let manifest = Manifest {
            session,
            limits,
            generation: 1,
            next_segment: 1,
            latest: cursor(1, 0),
            segments: Vec::new(),
            checkpoint: None,
            checkpoint_evicted: false,
            next_event: 1,
            acknowledged: 0,
            event_gap: 0,
            events: Vec::new(),
        };
        let mut journal = Self {
            root,
            lock,
            manifest,
            current_hash: None,
            metadata_bytes: 0,
            recovery: TerminalJournalRecovery::default(),
            poisoned: false,
            pending_files: BTreeSet::new(),
        };
        // No committed metadata exists: only an abandoned metadata temp is safe
        // to remove. Existing raw/event/checkpoint files require investigation.
        for name in journal.scan_owned()? {
            if name != TEMP {
                return Err(TerminalJournalError::Corrupt);
            }
            journal.remove_checked(&name)?;
        }
        journal.publish_manifest(&journal.manifest.clone())?;
        Ok(journal)
    }

    pub(crate) fn open_existing(
        root: OwnedFd,
        session: &TerminalSessionId,
        limits: TerminalJournalLimits,
    ) -> Result<Self> {
        limits.validate()?;
        private(&root, true)?;
        let lock = writer_lock(&root, false)?;
        let encoded = read_whole(&open_file(&root, META, OFlags::RDONLY)?, MAX_META)?;
        let envelope: Envelope =
            serde_json::from_slice(&encoded).map_err(|_| TerminalJournalError::Corrupt)?;
        ensure(
            envelope.version == 1
                && manifest_hash(&envelope.manifest)? == envelope.sha256
                && &envelope.manifest.session == session
                && envelope.manifest.limits == limits,
            TerminalJournalError::Corrupt,
        )?;
        validate_manifest(&envelope.manifest)?;
        let mut journal = Self {
            root,
            lock,
            manifest: envelope.manifest,
            current_hash: None,
            metadata_bytes: encoded.len(),
            recovery: TerminalJournalRecovery::default(),
            poisoned: false,
            pending_files: BTreeSet::new(),
        };
        journal.reconcile()?;
        Ok(journal)
    }

    pub(crate) fn earliest(&self) -> TerminalCursor {
        self.manifest
            .segments
            .first()
            .map_or_else(|| self.manifest.latest.clone(), |blob| cursor(blob.id, 0))
    }
    pub(crate) fn latest(&self) -> TerminalCursor {
        self.manifest.latest.clone()
    }
    pub(crate) fn checkpoint_status(&self) -> TerminalJournalCheckpointStatus {
        if self.manifest.checkpoint.is_some() {
            TerminalJournalCheckpointStatus::Available
        } else if self.manifest.checkpoint_evicted {
            TerminalJournalCheckpointStatus::RetentionEvicted
        } else {
            TerminalJournalCheckpointStatus::Missing
        }
    }
    pub(crate) const fn recovery(&self) -> TerminalJournalRecovery {
        self.recovery
    }
    pub(crate) fn usage(&self) -> TerminalJournalUsage {
        usage(&self.manifest, self.metadata_bytes)
    }

    /// At most 64 KiB and 128 segment writes per call. Failure after effects
    /// requires reopen; success means bytes and metadata were synchronized.
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<TerminalCursor> {
        self.ready()?;
        ensure(
            !bytes.is_empty()
                && bytes.len() <= MAX_PAGE_BYTES
                && bytes.len().div_ceil(self.manifest.limits.segment_bytes) < MAX_SEGMENTS,
            TerminalJournalError::Invalid,
        )?;
        self.poisoned = true;
        let mut next = self.manifest.clone();
        let mut remaining = bytes;
        while !remaining.is_empty() {
            let append_existing = next
                .segments
                .last()
                .is_some_and(|blob| blob.bytes < next.limits.segment_bytes);
            let (file, mut blob, mut hash) = if append_existing {
                let blob = next.segments.pop().ok_or(TerminalJournalError::Corrupt)?;
                let file = open_file(&self.root, &raw_name(blob.id), OFlags::RDWR)?;
                ensure(
                    file_size(&file)? == blob.bytes,
                    TerminalJournalError::Corrupt,
                )?;
                let hash = match self.current_hash.take() {
                    Some((id, hash)) if id == blob.id => hash,
                    _ => verify_prefix(&file, &blob)?,
                };
                (file, blob, hash)
            } else {
                let id = next.next_segment;
                next.next_segment = id
                    .checked_add(1)
                    .ok_or(TerminalJournalError::ResourceLimit)?;
                let file = create_file(&self.root, &raw_name(id))?;
                self.pending_files.insert(raw_name(id));
                (
                    file,
                    Blob {
                        id,
                        bytes: 0,
                        sha256: [0; 32],
                    },
                    Sha256::new(),
                )
            };
            let count = remaining.len().min(next.limits.segment_bytes - blob.bytes);
            write_at(&file, blob.bytes, &remaining[..count])?;
            sync(&file)?;
            hash.update(&remaining[..count]);
            blob.bytes += count;
            blob.sha256 = hash.clone().finalize().into();
            next.latest = cursor(blob.id, blob.bytes as u64);
            self.current_hash = Some((blob.id, hash));
            next.segments.push(blob);
            remaining = &remaining[count..];
        }
        trim(&mut next, false)?;
        self.commit(next)?;
        Ok(self.latest())
    }

    /// Returns at most one segment per page; a caller continues with `next`.
    pub(crate) fn read(
        &self,
        requested: &TerminalCursor,
        maximum: usize,
    ) -> Result<TerminalJournalPage> {
        self.ready()?;
        ensure(
            maximum > 0
                && maximum <= MAX_PAGE_BYTES
                && requested <= &self.manifest.latest
                && requested.offset() <= self.manifest.limits.segment_bytes as u64,
            TerminalJournalError::Invalid,
        )?;
        let earliest = self.earliest();
        let gap = if requested < &earliest {
            Some(
                TerminalGap::new(requested.clone(), earliest.clone())
                    .map_err(|_| TerminalJournalError::Corrupt)?,
            )
        } else {
            None
        };
        let mut position = if gap.is_some() {
            earliest.clone()
        } else {
            requested.clone()
        };
        let mut bytes = Vec::new();
        if position < self.manifest.latest {
            let index = self
                .manifest
                .segments
                .iter()
                .position(|blob| blob.id == position.segment())
                .ok_or(TerminalJournalError::Invalid)?;
            let mut blob = &self.manifest.segments[index];
            ensure(
                position.offset() <= blob.bytes as u64,
                TerminalJournalError::Invalid,
            )?;
            if position.offset() == blob.bytes as u64 {
                blob = self
                    .manifest
                    .segments
                    .get(index + 1)
                    .ok_or(TerminalJournalError::Invalid)?;
                position = cursor(blob.id, 0);
            }
            let file = open_file(&self.root, &raw_name(blob.id), OFlags::RDONLY)
                .map_err(missing_is_corrupt)?;
            ensure(
                file_size(&file)? == blob.bytes,
                TerminalJournalError::Corrupt,
            )?;
            verify_prefix(&file, blob)?;
            let offset =
                usize::try_from(position.offset()).map_err(|_| TerminalJournalError::Invalid)?;
            bytes.resize(maximum.min(blob.bytes - offset), 0);
            read_at(&file, offset, &mut bytes)?;
            position = cursor(blob.id, (offset + bytes.len()) as u64);
        }
        Ok(TerminalJournalPage {
            bytes,
            next: position,
            earliest,
            latest: self.latest(),
            gap,
        })
    }

    pub(crate) fn publish_checkpoint(
        &mut self,
        source: TerminalCursor,
        bytes: &[u8],
    ) -> Result<()> {
        self.ready()?;
        ensure(
            !bytes.is_empty()
                && bytes.len() <= MAX_CHECKPOINT_BYTES
                && bytes.len() <= self.manifest.limits.session_bytes
                && source <= self.manifest.latest,
            TerminalJournalError::Invalid,
        )?;
        self.validate_position(&source)?;
        self.poisoned = true;
        let mut next = self.manifest.clone();
        let id = next
            .generation
            .checked_add(1)
            .ok_or(TerminalJournalError::ResourceLimit)?;
        let blob = write_blob(&self.root, &checkpoint_name(id), id, bytes)?;
        self.pending_files.insert(checkpoint_name(id));
        next.checkpoint = Some(Checkpoint { blob, source });
        next.checkpoint_evicted = false;
        trim(&mut next, true)?;
        self.commit(next)
    }

    pub(crate) fn load_checkpoint(&self) -> Result<Option<TerminalJournalCheckpoint>> {
        self.ready()?;
        self.manifest
            .checkpoint
            .as_ref()
            .map(|value| {
                Ok(TerminalJournalCheckpoint {
                    source: value.source.clone(),
                    bytes: read_blob(
                        &self.root,
                        &checkpoint_name(value.blob.id),
                        &value.blob,
                        MAX_CHECKPOINT_BYTES,
                    )?,
                })
            })
            .transpose()
    }

    /// Event payloads are opaque bytes supplied by a separately bounded codec.
    pub(crate) fn append_event(&mut self, payload: &[u8]) -> Result<u64> {
        self.ready()?;
        ensure(
            !payload.is_empty()
                && payload.len() <= MAX_EVENT_BYTES
                && payload.len() <= self.manifest.limits.session_bytes,
            TerminalJournalError::Invalid,
        )?;
        self.poisoned = true;
        let mut next = self.manifest.clone();
        let id = next.next_event;
        next.next_event = id
            .checked_add(1)
            .ok_or(TerminalJournalError::ResourceLimit)?;
        next.events
            .push(write_blob(&self.root, &event_name(id), id, payload)?);
        self.pending_files.insert(event_name(id));
        while next.events.len() > MAX_EVENTS {
            evict_event(&mut next);
        }
        trim(&mut next, false)?;
        self.commit(next)?;
        Ok(id)
    }

    pub(crate) fn read_events(&self, after: u64, maximum: usize) -> Result<TerminalJournalEvents> {
        self.ready()?;
        ensure(
            maximum > 0 && maximum <= MAX_EVENTS && after < self.manifest.next_event,
            TerminalJournalError::Invalid,
        )?;
        let mut events = Vec::new();
        for blob in self
            .manifest
            .events
            .iter()
            .filter(|blob| blob.id > after)
            .take(maximum)
        {
            events.push(TerminalJournalEvent {
                id: blob.id,
                payload: read_blob(&self.root, &event_name(blob.id), blob, MAX_EVENT_BYTES)?,
            });
        }
        Ok(TerminalJournalEvents {
            events,
            gap_through: self.manifest.event_gap,
            next_event_id: self.manifest.next_event,
            acknowledged_through: self.manifest.acknowledged,
        })
    }

    pub(crate) fn acknowledge_events(&mut self, through: u64) -> Result<()> {
        self.ready()?;
        ensure(
            through < self.manifest.next_event,
            TerminalJournalError::Invalid,
        )?;
        if through <= self.manifest.acknowledged {
            return Ok(());
        }
        self.poisoned = true;
        let mut next = self.manifest.clone();
        next.acknowledged = through;
        while next.events.first().is_some_and(|event| event.id <= through) {
            evict_event(&mut next);
        }
        self.commit(next)
    }

    fn validate_position(&self, position: &TerminalCursor) -> Result<()> {
        if position == &self.manifest.latest {
            return Ok(());
        }
        ensure(
            self.manifest.segments.iter().any(|blob| {
                blob.id == position.segment() && position.offset() <= blob.bytes as u64
            }),
            TerminalJournalError::Invalid,
        )
    }

    fn ready(&self) -> Result<()> {
        ensure(!self.poisoned, TerminalJournalError::Unavailable)?;
        private(&self.root, true)?;
        private(&self.lock, false)?;
        let visible = open_file(&self.root, LOCK, OFlags::RDONLY)?;
        let held = rustix::fs::fstat(&self.lock).map_err(io_error)?;
        let visible = rustix::fs::fstat(&visible).map_err(io_error)?;
        ensure(
            held.st_dev == visible.st_dev && held.st_ino == visible.st_ino,
            TerminalJournalError::Corrupt,
        )
    }

    fn commit(&mut self, mut next: Manifest) -> Result<()> {
        next.generation = self
            .manifest
            .generation
            .checked_add(1)
            .ok_or(TerminalJournalError::ResourceLimit)?;
        validate_manifest(&next)?;
        self.publish_manifest(&next)?;
        let old = std::mem::replace(&mut self.manifest, next);
        let retained = owned_names(&self.manifest);
        let mut previous = owned_names(&old);
        previous.append(&mut self.pending_files);
        for name in previous.difference(&retained) {
            self.remove_checked(name)?;
        }
        sync(&self.root)?;
        self.poisoned = false;
        Ok(())
    }

    fn publish_manifest(&mut self, manifest: &Manifest) -> Result<()> {
        let envelope = Envelope {
            version: 1,
            manifest: manifest.clone(),
            sha256: manifest_hash(manifest)?,
        };
        let encoded = serde_json::to_vec(&envelope).map_err(|_| TerminalJournalError::Corrupt)?;
        ensure(
            encoded.len() <= MAX_META,
            TerminalJournalError::ResourceLimit,
        )?;
        let temp = create_file(&self.root, TEMP)?;
        write_at(&temp, 0, &encoded)?;
        sync(&temp)?;
        // Commit the new blob directory entries before metadata can reference them.
        sync(&self.root)?;
        match open_file(&self.root, META, OFlags::RDONLY) {
            Ok(_) | Err(TerminalJournalError::NotFound) => {}
            Err(error) => return Err(error),
        }
        rustix::fs::renameat(&self.root, TEMP, &self.root, META).map_err(io_error)?;
        sync(&self.root)?;
        self.metadata_bytes = encoded.len();
        Ok(())
    }

    fn reconcile(&mut self) -> Result<()> {
        // Validate every committed blob before changing any recovery artifact.
        let mut suffixes = Vec::new();
        for blob in &self.manifest.segments {
            let file = open_file(&self.root, &raw_name(blob.id), OFlags::RDWR)
                .map_err(missing_is_corrupt)?;
            let size = file_size(&file)?;
            ensure(
                size >= blob.bytes && size <= self.manifest.limits.segment_bytes,
                TerminalJournalError::Corrupt,
            )?;
            let hash = verify_prefix(&file, blob)?;
            if size > blob.bytes {
                suffixes.push((file, blob.bytes, size - blob.bytes));
            }
            self.current_hash = Some((blob.id, hash));
        }
        if let Some(checkpoint) = &self.manifest.checkpoint {
            let file = open_file(
                &self.root,
                &checkpoint_name(checkpoint.blob.id),
                OFlags::RDONLY,
            )
            .map_err(missing_is_corrupt)?;
            ensure(
                file_size(&file)? == checkpoint.blob.bytes,
                TerminalJournalError::Corrupt,
            )?;
            verify_prefix(&file, &checkpoint.blob)?;
        }
        for event in &self.manifest.events {
            let file = open_file(&self.root, &event_name(event.id), OFlags::RDONLY)
                .map_err(missing_is_corrupt)?;
            ensure(
                file_size(&file)? == event.bytes,
                TerminalJournalError::Corrupt,
            )?;
            verify_prefix(&file, event)?;
        }
        let retained = owned_names(&self.manifest);
        let orphans: Vec<_> = self
            .scan_owned()?
            .into_iter()
            .filter(|name| !retained.contains(name))
            .collect();
        // Prevalidate the entire bounded cleanup set, including hardlink checks.
        for name in &orphans {
            open_file(&self.root, name, OFlags::RDONLY)?;
        }
        for (file, committed, extra) in suffixes {
            rustix::fs::ftruncate(&file, committed as u64).map_err(io_error)?;
            sync(&file)?;
            self.recovery.discarded_uncommitted_bytes += extra as u64;
        }
        for name in orphans {
            self.remove_checked(&name)?;
            self.recovery.removed_orphan_files += 1;
        }
        sync(&self.root)
    }

    fn scan_owned(&self) -> Result<Vec<String>> {
        let duplicate = rustix::fs::openat(
            &self.root,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(io_error)?;
        let mut directory = Dir::new(duplicate).map_err(io_error)?;
        let mut owned = Vec::new();
        let mut count = 0;
        for entry in &mut directory {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            count += 1;
            ensure(
                count <= MAX_DIRECTORY_ENTRIES,
                TerminalJournalError::ResourceLimit,
            )?;
            if name == TEMP.as_bytes() || recognized_blob_name(name) {
                owned.push(
                    String::from_utf8(name.to_vec()).map_err(|_| TerminalJournalError::Corrupt)?,
                );
            }
        }
        Ok(owned)
    }

    fn remove_checked(&self, name: &str) -> Result<()> {
        open_file(&self.root, name, OFlags::RDONLY)?;
        rustix::fs::unlinkat(&self.root, name, AtFlags::empty()).map_err(io_error)
    }
}

fn validate_manifest(m: &Manifest) -> Result<()> {
    m.limits
        .validate()
        .map_err(|_| TerminalJournalError::Corrupt)?;
    ensure(
        m.generation > 0
            && m.next_segment > 0
            && m.segments.len() <= MAX_SEGMENTS
            && m.events.len() <= MAX_EVENTS
            && m.next_event > 0
            && m.acknowledged < m.next_event
            && m.event_gap < m.next_event
            && m.latest.offset() <= m.limits.segment_bytes as u64,
        TerminalJournalError::Corrupt,
    )?;
    let mut previous = 0;
    for (index, segment) in m.segments.iter().enumerate() {
        ensure(
            segment.id > previous
                && segment.id < m.next_segment
                && segment.bytes > 0
                && segment.bytes <= m.limits.segment_bytes,
            TerminalJournalError::Corrupt,
        )?;
        if previous != 0 {
            ensure(segment.id == previous + 1, TerminalJournalError::Corrupt)?;
        }
        ensure(
            index + 1 == m.segments.len() || segment.bytes == m.limits.segment_bytes,
            TerminalJournalError::Corrupt,
        )?;
        previous = segment.id;
    }
    if let Some(last) = m.segments.last() {
        ensure(
            m.latest == cursor(last.id, last.bytes as u64) && m.next_segment == last.id + 1,
            TerminalJournalError::Corrupt,
        )?;
    } else {
        ensure(
            (m.next_segment == 1 && m.latest == cursor(1, 0))
                || m.latest.segment().checked_add(1) == Some(m.next_segment),
            TerminalJournalError::Corrupt,
        )?;
    }
    if let Some(checkpoint) = &m.checkpoint {
        ensure(!m.checkpoint_evicted, TerminalJournalError::Corrupt)?;
        ensure(
            checkpoint.blob.id > 0
                && checkpoint.blob.id <= m.generation
                && checkpoint.blob.bytes > 0
                && checkpoint.blob.bytes <= MAX_CHECKPOINT_BYTES
                && checkpoint.source <= m.latest,
            TerminalJournalError::Corrupt,
        )?;
    }
    previous = m.event_gap;
    for event in &m.events {
        ensure(
            previous.checked_add(1) == Some(event.id)
                && event.id < m.next_event
                && event.id > m.acknowledged
                && event.bytes > 0
                && event.bytes <= MAX_EVENT_BYTES,
            TerminalJournalError::Corrupt,
        )?;
        previous = event.id;
    }
    ensure(
        previous.checked_add(1) == Some(m.next_event),
        TerminalJournalError::Corrupt,
    )?;
    ensure(m.acknowledged <= m.event_gap, TerminalJournalError::Corrupt)?;
    ensure(
        usage(m, 0).payload_bytes <= m.limits.session_bytes,
        TerminalJournalError::Corrupt,
    )
}

fn trim(m: &mut Manifest, preserve_checkpoint: bool) -> Result<()> {
    while usage(m, 0).payload_bytes > m.limits.session_bytes || m.segments.len() > MAX_SEGMENTS {
        if m.segments.len() > 1 {
            m.segments.remove(0);
        } else if !preserve_checkpoint && m.checkpoint.is_some() {
            m.checkpoint = None;
            m.checkpoint_evicted = true;
        } else if !m.events.is_empty() {
            evict_event(m);
        } else if preserve_checkpoint && !m.segments.is_empty() {
            m.segments.clear();
        } else {
            return Err(TerminalJournalError::ResourceLimit);
        }
    }
    Ok(())
}
fn evict_event(m: &mut Manifest) {
    m.event_gap = m.events.remove(0).id;
}
fn usage(m: &Manifest, metadata_bytes: usize) -> TerminalJournalUsage {
    let raw_bytes = m.segments.iter().map(|b| b.bytes).sum();
    let event_bytes = m.events.iter().map(|b| b.bytes).sum();
    let checkpoint_bytes = m.checkpoint.as_ref().map_or(0, |c| c.blob.bytes);
    TerminalJournalUsage {
        raw_bytes,
        checkpoint_bytes,
        event_bytes,
        payload_bytes: raw_bytes + checkpoint_bytes + event_bytes,
        metadata_bytes,
    }
}
fn owned_names(m: &Manifest) -> BTreeSet<String> {
    let mut names: BTreeSet<_> = m
        .segments
        .iter()
        .map(|s| raw_name(s.id))
        .chain(m.events.iter().map(|e| event_name(e.id)))
        .collect();
    if let Some(checkpoint) = &m.checkpoint {
        names.insert(checkpoint_name(checkpoint.blob.id));
    }
    names
}
fn recognized_blob_name(name: &[u8]) -> bool {
    [
        b"tj-raw-".as_slice(),
        b"tj-event-".as_slice(),
        b"tj-checkpoint-".as_slice(),
    ]
    .iter()
    .any(|prefix| {
        name.strip_prefix(*prefix)
            .is_some_and(|suffix| suffix.len() == 20 && suffix.iter().all(u8::is_ascii_digit))
    })
}
fn raw_name(id: u64) -> String {
    format!("tj-raw-{id:020}")
}
fn event_name(id: u64) -> String {
    format!("tj-event-{id:020}")
}
fn checkpoint_name(id: u64) -> String {
    format!("tj-checkpoint-{id:020}")
}
fn cursor(segment: u64, offset: u64) -> TerminalCursor {
    TerminalCursor::new(segment, offset).expect("internal cursor segment is nonzero")
}
fn ensure(condition: bool, error: TerminalJournalError) -> Result<()> {
    if condition { Ok(()) } else { Err(error) }
}
fn io_error(error: rustix::io::Errno) -> TerminalJournalError {
    match error {
        rustix::io::Errno::NOENT => TerminalJournalError::NotFound,
        rustix::io::Errno::LOOP => TerminalJournalError::Corrupt,
        _ => TerminalJournalError::Unavailable,
    }
}
fn missing_is_corrupt(error: TerminalJournalError) -> TerminalJournalError {
    if error == TerminalJournalError::NotFound {
        TerminalJournalError::Corrupt
    } else {
        error
    }
}

fn private(fd: impl AsFd, directory: bool) -> Result<()> {
    let stat = rustix::fs::fstat(fd.as_fd()).map_err(io_error)?;
    let kind = FileType::from_raw_mode(stat.st_mode);
    ensure(
        stat.st_uid == rustix::process::geteuid().as_raw()
            && if directory {
                kind.is_dir() && u64::from(stat.st_mode) & 0o7777 == 0o700 && stat.st_nlink > 0
            } else {
                kind.is_file() && u64::from(stat.st_mode) & 0o7777 == 0o600 && stat.st_nlink == 1
            },
        TerminalJournalError::Corrupt,
    )?;
    #[cfg(target_os = "macos")]
    {
        let acl = calcifer_macos_acl::read_acl(fd.as_fd())
            .map_err(|_| TerminalJournalError::Unavailable)?;
        ensure(
            acl.flags == 0
                && acl.entries.iter().all(|entry| {
                    entry.tag == calcifer_macos_acl::TAG_DENY
                        && entry.flags == 0
                        && entry.permissions == calcifer_macos_acl::PERMISSION_DELETE
                }),
            TerminalJournalError::Corrupt,
        )?;
    }
    Ok(())
}
fn open_file(root: impl AsFd, name: &str, flags: OFlags) -> Result<OwnedFd> {
    let fd = rustix::fs::openat(
        root,
        name,
        flags | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?;
    private(&fd, false)?;
    Ok(fd)
}
fn create_file(root: impl AsFd, name: &str) -> Result<OwnedFd> {
    let fd = rustix::fs::openat(
        root,
        name,
        OFlags::RDWR
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::NONBLOCK
            | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|error| {
        if error == rustix::io::Errno::EXIST {
            TerminalJournalError::Corrupt
        } else {
            io_error(error)
        }
    })?;
    private(&fd, false)?;
    Ok(fd)
}
fn writer_lock(root: impl AsFd, create: bool) -> Result<OwnedFd> {
    let fd = match open_file(root.as_fd(), LOCK, OFlags::RDWR) {
        Err(TerminalJournalError::NotFound) if create => match create_file(root.as_fd(), LOCK) {
            Ok(file) => file,
            Err(TerminalJournalError::Corrupt) => open_file(root.as_fd(), LOCK, OFlags::RDWR)?,
            Err(error) => return Err(error),
        },
        result => result?,
    };
    match rustix::fs::flock(&fd, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {}
        Err(rustix::io::Errno::WOULDBLOCK) => return Err(TerminalJournalError::Busy),
        Err(error) => return Err(io_error(error)),
    }
    Ok(fd)
}
fn sync(fd: impl AsFd) -> Result<()> {
    rustix::fs::fsync(fd).map_err(io_error)
}
fn file_size(fd: impl AsFd) -> Result<usize> {
    usize::try_from(rustix::fs::fstat(fd).map_err(io_error)?.st_size)
        .map_err(|_| TerminalJournalError::Corrupt)
}
fn write_at(fd: impl AsFd, mut offset: usize, mut bytes: &[u8]) -> Result<()> {
    while !bytes.is_empty() {
        let count = rustix::io::pwrite(fd.as_fd(), bytes, offset as u64).map_err(io_error)?;
        ensure(count > 0, TerminalJournalError::Unavailable)?;
        offset += count;
        bytes = &bytes[count..];
    }
    Ok(())
}
fn read_at(fd: impl AsFd, mut offset: usize, mut bytes: &mut [u8]) -> Result<()> {
    while !bytes.is_empty() {
        let count = rustix::io::pread(fd.as_fd(), &mut *bytes, offset as u64).map_err(io_error)?;
        ensure(count > 0, TerminalJournalError::Corrupt)?;
        offset += count;
        bytes = &mut bytes[count..];
    }
    Ok(())
}
fn read_whole(fd: impl AsFd, maximum: usize) -> Result<Vec<u8>> {
    let size = file_size(fd.as_fd())?;
    ensure(size <= maximum, TerminalJournalError::ResourceLimit)?;
    let mut bytes = vec![0; size];
    read_at(fd.as_fd(), 0, &mut bytes)?;
    ensure(
        file_size(fd.as_fd())? == size,
        TerminalJournalError::Corrupt,
    )?;
    Ok(bytes)
}
fn verify_prefix(fd: impl AsFd, blob: &Blob) -> Result<Sha256> {
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 8192];
    let mut offset = 0;
    while offset < blob.bytes {
        let count = buffer.len().min(blob.bytes - offset);
        read_at(fd.as_fd(), offset, &mut buffer[..count])?;
        hash.update(&buffer[..count]);
        offset += count;
    }
    ensure(
        <[u8; 32]>::from(hash.clone().finalize()) == blob.sha256,
        TerminalJournalError::Corrupt,
    )?;
    Ok(hash)
}
fn write_blob(root: impl AsFd, name: &str, id: u64, bytes: &[u8]) -> Result<Blob> {
    let file = create_file(root, name)?;
    write_at(&file, 0, bytes)?;
    sync(&file)?;
    Ok(Blob {
        id,
        bytes: bytes.len(),
        sha256: Sha256::digest(bytes).into(),
    })
}
fn read_blob(root: impl AsFd, name: &str, blob: &Blob, maximum: usize) -> Result<Vec<u8>> {
    let file = open_file(root, name, OFlags::RDONLY).map_err(missing_is_corrupt)?;
    let bytes = read_whole(&file, maximum)?;
    ensure(
        bytes.len() == blob.bytes && <[u8; 32]>::from(Sha256::digest(&bytes)) == blob.sha256,
        TerminalJournalError::Corrupt,
    )?;
    Ok(bytes)
}
fn manifest_hash(manifest: &Manifest) -> Result<[u8; 32]> {
    let bytes = serde_json::to_vec(manifest).map_err(|_| TerminalJournalError::Corrupt)?;
    let mut digest = Sha256::new();
    digest.update(DOMAIN);
    digest.update(bytes);
    Ok(digest.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{DirBuilder, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
    use std::path::PathBuf;

    struct Fixture {
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let suffix = format!("{:032x}", u128::from_le_bytes(random));
            let path = std::env::temp_dir().join(format!("machine-god-terminal-journal-{suffix}"));
            DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self { path }
        }
        fn fd(&self) -> OwnedFd {
            rustix::fs::open(
                &self.path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap()
        }
        fn create(&self, limits: TerminalJournalLimits) -> TerminalJournal {
            TerminalJournal::create(self.fd(), session(), limits).unwrap()
        }
        fn open(&self, limits: TerminalJournalLimits) -> Result<TerminalJournal> {
            after_owner_drop(|| TerminalJournal::open_existing(self.fd(), &session(), limits))
        }
        fn put(&self, name: &str, bytes: &[u8]) {
            let fd = create_file(self.fd(), name).unwrap();
            write_at(&fd, 0, bytes).unwrap();
            sync(fd).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn session() -> TerminalSessionId {
        TerminalSessionId::new("terminal-test").unwrap()
    }
    fn limits(segment_bytes: usize, session_bytes: usize) -> TerminalJournalLimits {
        TerminalJournalLimits {
            segment_bytes,
            session_bytes,
        }
    }
    fn after_owner_drop<T>(mut open: impl FnMut() -> Result<T>) -> Result<T> {
        // Concurrent process-spawn tests can transiently inherit a flock
        // before exec closes the CLOEXEC descriptor. Only post-drop assertions
        // use this bounded wait; live-writer conflict tests call try-open directly.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match open() {
                Err(TerminalJournalError::Busy) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                result => return result,
            }
        }
    }
    fn collect(journal: &TerminalJournal, mut position: TerminalCursor) -> Vec<u8> {
        let mut output = Vec::new();
        for _ in 0..256 {
            let page = journal.read(&position, 3).unwrap();
            output.extend(page.bytes);
            if page.next == page.latest {
                return output;
            }
            assert!(page.next > position);
            position = page.next;
        }
        panic!("page traversal did not terminate");
    }

    #[test]
    fn create_rollover_retention_gap_reopen_and_accounting() {
        let fixture = Fixture::new();
        let limits = limits(4, 12);
        let mut journal = fixture.create(limits);
        assert_eq!(journal.latest(), cursor(1, 0));
        assert_eq!(journal.append(b"abcdefghijklmnop").unwrap(), cursor(4, 4));
        let page = journal.read(&cursor(1, 0), 3).unwrap();
        assert_eq!(page.bytes, b"efg");
        assert_eq!(page.earliest, cursor(2, 0));
        assert_eq!(
            page.gap.unwrap(),
            TerminalGap::new(cursor(1, 0), cursor(2, 0)).unwrap()
        );
        assert_eq!(collect(&journal, cursor(1, 0)), b"efghijklmnop");
        assert_eq!(journal.usage().payload_bytes, 12);
        assert!(journal.usage().metadata_bytes > 0);
        assert!(!fixture.path.join(raw_name(1)).exists());
        drop(journal);
        let mut reopened = fixture.open(limits).unwrap();
        assert_eq!(reopened.recovery(), TerminalJournalRecovery::default());
        assert_eq!(collect(&reopened, cursor(2, 0)), b"efghijklmnop");
        assert_eq!(reopened.append(b"q").unwrap(), cursor(5, 1));
        assert_eq!(collect(&reopened, cursor(1, 0)), b"ijklmnopq");
    }

    #[test]
    fn bounds_invalid_cursors_and_lifetime_lock_are_fail_fast() {
        let fixture = Fixture::new();
        let limits = limits(8, 16);
        let mut journal = fixture.create(limits);
        assert_eq!(
            TerminalJournal::open_existing(fixture.fd(), &session(), limits).unwrap_err(),
            TerminalJournalError::Busy
        );
        assert_eq!(
            TerminalJournal::create(fixture.fd(), session(), limits).unwrap_err(),
            TerminalJournalError::Busy
        );
        assert_eq!(
            journal.append(&[]).unwrap_err(),
            TerminalJournalError::Invalid
        );
        assert_eq!(
            journal.read(&cursor(2, 0), 1).unwrap_err(),
            TerminalJournalError::Invalid
        );
        assert_eq!(
            journal.read(&cursor(1, 0), 0).unwrap_err(),
            TerminalJournalError::Invalid
        );
        journal.append(b"hello").unwrap();
        assert_eq!(
            journal.read(&cursor(1, 6), 1).unwrap_err(),
            TerminalJournalError::Invalid
        );
        drop(journal);
        assert_eq!(
            after_owner_drop(|| TerminalJournal::create(fixture.fd(), session(), limits))
                .unwrap_err(),
            TerminalJournalError::Conflict
        );
        assert!(fixture.open(limits).is_ok());
        assert!(
            TerminalJournalLimits {
                segment_bytes: 1,
                session_bytes: 129
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn metadata_and_committed_segments_are_checksum_validated() {
        let fixture = Fixture::new();
        let limits = limits(8, 16);
        let mut journal = fixture.create(limits);
        journal.append(b"hello").unwrap();
        let mut file = OpenOptions::new()
            .write(true)
            .open(fixture.path.join(raw_name(1)))
            .unwrap();
        file.write_all(b"j").unwrap();
        file.sync_all().unwrap();
        assert_eq!(
            journal.read(&cursor(1, 0), 5).unwrap_err(),
            TerminalJournalError::Corrupt
        );
        drop(journal);
        assert_eq!(
            fixture.open(limits).unwrap_err(),
            TerminalJournalError::Corrupt
        );

        let second = Fixture::new();
        drop(second.create(limits));
        let mut metadata = OpenOptions::new()
            .write(true)
            .open(second.path.join(META))
            .unwrap();
        metadata.seek(SeekFrom::Start(0)).unwrap();
        metadata.write_all(b"!").unwrap();
        assert_eq!(
            second.open(limits).unwrap_err(),
            TerminalJournalError::Corrupt
        );
    }

    #[test]
    fn truncated_committed_output_is_not_invented_or_silently_repaired() {
        let fixture = Fixture::new();
        let limits = limits(8, 16);
        let mut journal = fixture.create(limits);
        journal.append(b"hello").unwrap();
        drop(journal);
        OpenOptions::new()
            .write(true)
            .open(fixture.path.join(raw_name(1)))
            .unwrap()
            .set_len(2)
            .unwrap();
        fixture.put(TEMP, b"abandoned");
        assert_eq!(
            fixture.open(limits).unwrap_err(),
            TerminalJournalError::Corrupt
        );
        assert!(fixture.path.join(TEMP).exists());
        assert_eq!(
            std::fs::metadata(fixture.path.join(raw_name(1)))
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn publication_failure_poisoning_reconciles_back_to_committed_prefix() {
        let fixture = Fixture::new();
        let limits = limits(8, 16);
        let mut journal = fixture.create(limits);
        journal.append(b"old").unwrap();
        fixture.put(TEMP, b"abandoned");
        assert_eq!(
            journal.append(b"new").unwrap_err(),
            TerminalJournalError::Corrupt
        );
        assert_eq!(
            journal.append(b"retry").unwrap_err(),
            TerminalJournalError::Unavailable
        );
        assert_eq!(
            journal.read(&cursor(1, 0), 8).unwrap_err(),
            TerminalJournalError::Unavailable
        );
        drop(journal);
        let journal = fixture.open(limits).unwrap();
        assert_eq!(journal.recovery().discarded_uncommitted_bytes, 3);
        assert_eq!(collect(&journal, cursor(1, 0)), b"old");
    }

    #[test]
    fn reopen_discards_only_proven_uncommitted_suffix_and_owned_orphans() {
        let fixture = Fixture::new();
        let limits = limits(8, 16);
        let mut journal = fixture.create(limits);
        journal.append(b"hello").unwrap();
        drop(journal);
        let mut raw = OpenOptions::new()
            .append(true)
            .open(fixture.path.join(raw_name(1)))
            .unwrap();
        raw.write_all(b"??").unwrap();
        raw.sync_all().unwrap();
        fixture.put(&raw_name(2), b"orphan");
        fixture.put(&event_name(1), b"orphan");
        fixture.put(TEMP, b"abandoned metadata");
        fixture.put("unrelated-runtime-file", b"keep");
        let mut reopened = fixture.open(limits).unwrap();
        assert_eq!(reopened.recovery().discarded_uncommitted_bytes, 2);
        assert_eq!(reopened.recovery().removed_orphan_files, 3);
        assert_eq!(collect(&reopened, cursor(1, 0)), b"hello");
        assert!(fixture.path.join("unrelated-runtime-file").exists());
        assert_eq!(reopened.append(b"!").unwrap(), cursor(1, 6));
        assert_eq!(collect(&reopened, cursor(1, 0)), b"hello!");
    }

    #[test]
    fn checkpoints_retain_exact_source_and_validate_opaque_blob_checksums() {
        let fixture = Fixture::new();
        let limits = limits(8, 24);
        let mut journal = fixture.create(limits);
        let source = journal.append(b"hello").unwrap();
        journal
            .publish_checkpoint(source.clone(), b"grid-v1")
            .unwrap();
        journal.append(b"!").unwrap();
        let checkpoint = journal.load_checkpoint().unwrap().unwrap();
        assert_eq!(checkpoint.source, source);
        assert_eq!(checkpoint.bytes, b"grid-v1");
        assert_eq!(
            journal
                .publish_checkpoint(cursor(9, 0), b"bad")
                .unwrap_err(),
            TerminalJournalError::Invalid
        );
        drop(journal);
        let reopened = fixture.open(limits).unwrap();
        assert_eq!(reopened.load_checkpoint().unwrap().unwrap().source, source);
        let id = reopened.manifest.checkpoint.as_ref().unwrap().blob.id;
        OpenOptions::new()
            .write(true)
            .open(fixture.path.join(checkpoint_name(id)))
            .unwrap()
            .write_all(b"X")
            .unwrap();
        assert_eq!(
            reopened.load_checkpoint().unwrap_err(),
            TerminalJournalError::Corrupt
        );
    }

    #[test]
    fn checkpoint_budget_eviction_and_later_append_preserve_explicit_gaps() {
        let fixture = Fixture::new();
        let limits = limits(4, 8);
        let mut journal = fixture.create(limits);
        journal.append(b"abcdefgh").unwrap();
        journal
            .publish_checkpoint(journal.latest(), b"12345678")
            .unwrap();
        assert_eq!(journal.usage().payload_bytes, 8);
        assert_eq!(journal.earliest(), journal.latest());
        assert!(journal.read(&cursor(1, 0), 8).unwrap().gap.is_some());
        journal.append(b"ij").unwrap();
        assert!(journal.load_checkpoint().unwrap().is_none());
        assert_eq!(
            journal.checkpoint_status(),
            TerminalJournalCheckpointStatus::RetentionEvicted
        );
        assert_eq!(journal.latest(), cursor(3, 2));
        assert_eq!(collect(&journal, cursor(1, 0)), b"ij");
        drop(journal);
        assert_eq!(
            fixture.open(limits).unwrap().checkpoint_status(),
            TerminalJournalCheckpointStatus::RetentionEvicted
        );
    }

    #[test]
    fn events_retain_256_and_acknowledgements_and_gaps_survive_reopen() {
        let fixture = Fixture::new();
        let limits = limits(64, 2048);
        let mut journal = fixture.create(limits);
        for expected in 1..=260 {
            assert_eq!(journal.append_event(b"x").unwrap(), expected);
        }
        let events = journal.read_events(0, 256).unwrap();
        assert_eq!(events.events.len(), 256);
        assert_eq!(events.gap_through, 4);
        assert_eq!(events.next_event_id, 261);
        assert_eq!(events.events[0].id, 5);
        assert_eq!(events.events[0].payload, b"x");
        journal.acknowledge_events(100).unwrap();
        journal.acknowledge_events(99).unwrap();
        assert_eq!(
            journal.acknowledge_events(261).unwrap_err(),
            TerminalJournalError::Invalid
        );
        drop(journal);
        let reopened = fixture.open(limits).unwrap();
        let events = reopened.read_events(0, 256).unwrap();
        assert_eq!(events.events.len(), 160);
        assert_eq!(events.acknowledged_through, 100);
        assert_eq!(events.gap_through, 100);
        assert!(!fixture.path.join(event_name(1)).exists());
    }

    #[test]
    fn hardlinks_symlinks_and_nonprivate_files_are_rejected_without_cleanup() {
        let fixture = Fixture::new();
        let limits = limits(8, 16);
        let mut journal = fixture.create(limits);
        journal.append(b"hello").unwrap();
        drop(journal);
        std::fs::hard_link(fixture.path.join(raw_name(1)), fixture.path.join("alias")).unwrap();
        assert_eq!(
            fixture.open(limits).unwrap_err(),
            TerminalJournalError::Corrupt
        );
        assert!(fixture.path.join("alias").exists());
        std::fs::remove_file(fixture.path.join("alias")).unwrap();
        symlink("unrelated", fixture.path.join(TEMP)).unwrap();
        assert_eq!(
            fixture.open(limits).unwrap_err(),
            TerminalJournalError::Corrupt
        );
        assert!(
            std::fs::symlink_metadata(fixture.path.join(TEMP))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::remove_file(fixture.path.join(TEMP)).unwrap();
        std::fs::set_permissions(
            fixture.path.join(raw_name(1)),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert_eq!(
            fixture.open(limits).unwrap_err(),
            TerminalJournalError::Corrupt
        );
    }

    #[test]
    fn bad_directory_permissions_or_wrong_session_never_grant_a_store() {
        let fixture = Fixture::new();
        let limits = limits(8, 16);
        drop(fixture.create(limits));
        assert_eq!(
            after_owner_drop(|| TerminalJournal::open_existing(
                fixture.fd(),
                &TerminalSessionId::new("other").unwrap(),
                limits
            ))
            .unwrap_err(),
            TerminalJournalError::Corrupt
        );
        std::fs::set_permissions(&fixture.path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            fixture.open(limits).unwrap_err(),
            TerminalJournalError::Corrupt
        );
    }

    #[test]
    fn multi_segment_single_append_removes_newly_created_then_evicted_files() {
        let fixture = Fixture::new();
        let limits = limits(2, 4);
        let mut journal = fixture.create(limits);
        journal.append(b"abcdefghijkl").unwrap();
        assert_eq!(collect(&journal, cursor(1, 0)), b"ijkl");
        for id in 1..5 {
            assert!(!fixture.path.join(raw_name(id)).exists());
        }
        drop(journal);
        assert_eq!(
            fixture
                .open(limits)
                .unwrap()
                .recovery()
                .removed_orphan_files,
            0
        );
    }

    #[test]
    fn errors_and_debug_do_not_reflect_journal_payloads() {
        let fixture = Fixture::new();
        let mut journal = fixture.create(limits(32, 64));
        journal.append(b"PRIVATE_OUTPUT").unwrap();
        assert_eq!(format!("{journal:?}"), "TerminalJournal { .. }");
        assert_eq!(
            format!("{:?}", journal.read(&cursor(1, 0), 32).unwrap()),
            "TerminalJournalPage { .. }"
        );
    }
}
