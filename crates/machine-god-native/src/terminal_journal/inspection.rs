//! Read-only committed metadata/facts validation, without a journal writer.
//!
//! The caller must hold a shared existing-profile transaction. Raw output,
//! screen checkpoints and event bodies are NOT hashed or interpreted here.
//! Their committed references are checked for safe files and recorded lengths.
//! Uncommitted raw suffixes and orphan files are never repaired or removed.

use super::{
    AtFlags, Envelope, MAX_META, META, OFlags, OwnedFd, Sha256, TerminalCursor, TerminalSessionId,
    checkpoint_name, cursor, event_name, manifest_hash, private, raw_name, state_name,
    validate_manifest, validate_visible_file,
};
use crate::background_terminal_inspection::{
    HistoryReadBudget, NativeTerminalBackgroundHistoryError as Error, Result,
};
use crate::terminal_session_record::TerminalSessionFacts;
use rustix::fd::AsFd;
use rustix::fs::Mode;
use sha2::Digest;

pub(crate) struct VerifiedHistory {
    pub(crate) facts: TerminalSessionFacts,
    pub(crate) earliest: TerminalCursor,
    pub(crate) latest: TerminalCursor,
}

pub(crate) fn read_history(
    root: impl AsFd,
    session: &TerminalSessionId,
    budget: &mut HistoryReadBudget<'_>,
) -> Result<VerifiedHistory> {
    budget.checkpoint()?;
    private(root.as_fd(), true)?;
    let lock = open_read(root.as_fd(), super::LOCK)?;
    validate_visible_file(root.as_fd(), super::LOCK, &lock, 0)?;
    let metadata = open_read(root.as_fd(), META)?;
    let metadata_len = super::file_size(&metadata)?;
    if metadata_len > MAX_META {
        return Err(Error::ResourceLimit);
    }
    budget.charge(metadata_len)?;
    let mut encoded = vec![0; metadata_len];
    read_at(&metadata, 0, &mut encoded, budget)?;
    let envelope: Envelope = serde_json::from_slice(&encoded).map_err(|_| Error::Corrupt)?;
    if envelope.version != 1
        || &envelope.manifest.session != session
        || manifest_hash(&envelope.manifest)? != envelope.sha256
    {
        return Err(Error::Corrupt);
    }
    validate_manifest(&envelope.manifest)?;
    validate_visible_file(root.as_fd(), META, &metadata, metadata_len)?;
    let manifest = envelope.manifest;
    let state = manifest.state.as_ref().ok_or(Error::Corrupt)?;
    budget.charge(state.blob.bytes)?;
    let state_name = state_name(state.blob.id);
    let file = open_read(root.as_fd(), &state_name)?;
    validate_visible_file(root.as_fd(), &state_name, &file, state.blob.bytes)?;
    let header_len = TerminalSessionFacts::PREFIX_HEADER_BYTES;
    if state.blob.bytes <= header_len {
        return Err(Error::Corrupt);
    }
    let mut prefix = vec![0; header_len];
    read_at(&file, 0, &mut prefix, budget)?;
    let prefix_len = TerminalSessionFacts::prefix_hint_len(&prefix, state.blob.bytes)
        .map_err(|_| Error::Corrupt)?;
    let mut hash = Sha256::new();
    hash.update(&prefix);
    prefix.resize(prefix_len, 0);
    let mut buffer = [0_u8; 8192];
    let mut offset = header_len;
    while offset < state.blob.bytes {
        let count = buffer.len().min(state.blob.bytes - offset);
        read_at(&file, offset, &mut buffer[..count], budget)?;
        hash.update(&buffer[..count]);
        if offset < prefix_len {
            let copy_len = count.min(prefix_len - offset);
            prefix[offset..offset + copy_len].copy_from_slice(&buffer[..copy_len]);
        }
        offset += count;
    }
    if <[u8; 32]>::from(hash.finalize()) != state.blob.sha256 {
        return Err(Error::Corrupt);
    }
    // The prefix becomes recorded facts only AFTER the full blob hash succeeds.
    // Monitor semantic restoration is deliberately outside this data-only view.
    let facts =
        TerminalSessionFacts::decode_prefix_hint(&prefix, state.blob.bytes, session, &state.source)
            .map_err(|_| Error::Corrupt)?;
    for raw in &manifest.segments {
        check_reference(
            root.as_fd(),
            &raw_name(raw.id),
            raw.bytes,
            manifest.limits.segment_bytes,
            budget,
        )?;
    }
    if let Some(checkpoint) = &manifest.checkpoint {
        check_reference(
            root.as_fd(),
            &checkpoint_name(checkpoint.blob.id),
            checkpoint.blob.bytes,
            checkpoint.blob.bytes,
            budget,
        )?;
    }
    for event in &manifest.events {
        check_reference(
            root.as_fd(),
            &event_name(event.id),
            event.bytes,
            event.bytes,
            budget,
        )?;
    }
    validate_visible_file(root.as_fd(), &state_name, &file, state.blob.bytes)?;
    validate_visible_file(root.as_fd(), META, &metadata, metadata_len)?;
    validate_visible_file(root.as_fd(), super::LOCK, &lock, 0)?;
    private(&file, false)?;
    private(&metadata, false)?;
    private(&lock, false)?;
    private(root.as_fd(), true)?;
    budget.checkpoint()?;
    Ok(VerifiedHistory {
        facts,
        earliest: manifest
            .segments
            .first()
            .map_or_else(|| manifest.latest.clone(), |blob| cursor(blob.id, 0)),
        latest: manifest.latest,
    })
}

fn open_read(root: impl AsFd, name: &str) -> Result<OwnedFd> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    #[cfg(target_os = "linux")]
    let flags = flags | OFlags::NOATIME;
    let file = rustix::fs::openat(root.as_fd(), name, flags, Mode::empty()).map_err(io_error)?;
    private(&file, false)?;
    let visible = rustix::fs::statat(root, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    let held = rustix::fs::fstat(&file).map_err(io_error)?;
    if held.st_dev != visible.st_dev || held.st_ino != visible.st_ino {
        return Err(Error::Corrupt);
    }
    Ok(file)
}

fn check_reference(
    root: impl AsFd,
    name: &str,
    minimum: usize,
    maximum: usize,
    budget: &HistoryReadBudget<'_>,
) -> Result<()> {
    budget.checkpoint()?;
    let file = open_read(root.as_fd(), name)?;
    let size = super::file_size(&file)?;
    if size < minimum || size > maximum {
        return Err(Error::Corrupt);
    }
    validate_visible_file(root, name, &file, size)?;
    Ok(())
}

fn read_at(
    file: impl AsFd,
    mut offset: usize,
    mut bytes: &mut [u8],
    budget: &HistoryReadBudget<'_>,
) -> Result<()> {
    while !bytes.is_empty() {
        budget.checkpoint()?;
        let count =
            rustix::io::pread(file.as_fd(), &mut *bytes, offset as u64).map_err(io_error)?;
        if count == 0 {
            return Err(Error::Corrupt);
        }
        offset += count;
        bytes = &mut bytes[count..];
    }
    Ok(())
}

fn io_error(error: rustix::io::Errno) -> Error {
    match error {
        rustix::io::Errno::NOENT | rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => {
            Error::Corrupt
        }
        _ => Error::Unavailable,
    }
}
