//! Private store operations share its exact descriptor, lock namespace and codec.
use super::{
    FILE_SESSION_SCHEMA_VERSION, FileSessionScanControl, FileSessionStore, MAX_FILE_SESSION_BYTES,
    ObjectOnly, SessionNames, StoredEnvelope, create_new_temp, ensure_listing_root_is_linked,
    ensure_regular, open_lock, probe_data, serialize_record, validate_record_json,
};
use crate::session_maintenance::{
    NativeSessionMaintenanceError as Error, NativeSessionMigration, NativeSessionRecovery,
};
use crate::{NATIVE_SESSION_METADATA_KEY, NativeSessionMetadata};
use machine_god_core::{SessionId, SessionIncarnationId, SessionRecord, SessionRevision};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{AtFlags, Mode, OFlags};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "storage/cleanup.rs"]
mod cleanup;
#[path = "storage/recovery.rs"]
mod recovery;

impl FileSessionStore {
    #[cfg(test)]
    pub(crate) fn maintenance_test_after_commit_failure<T>(operation: impl FnOnce() -> T) -> T {
        let _hook = tests::HookGuard::install(|stage| {
            if stage == tests::PublicationStage::AfterCommit {
                Err(Error::Unavailable)
            } else {
                Ok(())
            }
        });
        operation()
    }
    pub(crate) fn maintenance_migrate(
        &self,
        id: &SessionId,
        control: &FileSessionScanControl,
    ) -> Result<NativeSessionMigration, Error> {
        control.check()?;
        let names = SessionNames::for_id(id);
        let source = read_source(self.root.as_fd(), &names.data, control)?;
        let lock = open_lock(self.root.as_fd(), &names.lock).map_err(|_| Error::Unavailable)?;
        let _guard = control.lock(&lock)?;
        source.check(self.root.as_fd(), &names.data)?;
        let mut record = decode(&source.bytes, id)?;
        if !migrate_metadata(&mut record)? {
            control.check()?;
            return Ok(NativeSessionMigration::AlreadyCurrent(record));
        }
        publish(self.root.as_fd(), &names, &record, control, || {
            source.check(self.root.as_fd(), &names.data)
        })?;
        Ok(NativeSessionMigration::Migrated(record))
    }

    pub(crate) fn maintenance_recover(
        &self,
        source_id: &SessionId,
        destination: &SessionId,
        incarnation: &SessionIncarnationId,
        control: &FileSessionScanControl,
    ) -> Result<NativeSessionRecovery, Error> {
        if source_id == destination {
            return Err(Error::InvalidDestination);
        }
        control.check()?;
        let source_names = SessionNames::for_id(source_id);
        let source = read_source(self.root.as_fd(), &source_names.data, control)?;
        let source_lock =
            open_lock(self.root.as_fd(), &source_names.lock).map_err(|_| Error::Unavailable)?;
        let _source_guard = control.lock(&source_lock)?;
        source.check(self.root.as_fd(), &source_names.data)?;
        let (mut record, truncated_source) = recovery::decode_source(&source.bytes, source_id)?;
        if &record.incarnation_id == incarnation {
            return Err(Error::InvalidDestination);
        }
        let unknown_tool_results = recovery::close_tools(&mut record)?;
        record.id = destination.clone();
        record.incarnation_id = incarnation.clone();
        record.revision = SessionRevision(1);
        record.metadata.clear();
        record.metadata.insert(
            NATIVE_SESSION_METADATA_KEY.to_owned(),
            serde_json::json!({"schema_version":2,"origin":"recovered"}),
        );
        recovery::validate_limits(&record)?;
        let names = SessionNames::for_id(destination);
        let lock = open_lock(self.root.as_fd(), &names.lock).map_err(|_| Error::Unavailable)?;
        let _guard = control.lock(&lock)?;
        if probe_data(self.root.as_fd(), &names.data).map_err(|_| Error::Unavailable)? {
            return Err(Error::DestinationExists);
        }
        publish(self.root.as_fd(), &names, &record, control, || {
            source.check(self.root.as_fd(), &source_names.data)?;
            if probe_data(self.root.as_fd(), &names.data).map_err(|_| Error::Unavailable)? {
                return Err(Error::DestinationExists);
            }
            Ok(())
        })?;
        Ok(NativeSessionRecovery {
            record,
            truncated_source,
            unknown_tool_results,
        })
    }
}

fn migrate_metadata(record: &mut SessionRecord) -> Result<bool, Error> {
    let metadata = NativeSessionMetadata::from_metadata(&record.metadata).map_err(|error| {
        if error == crate::NativeSessionMetadataError::UnsupportedVersion {
            Error::UnsupportedVersion
        } else {
            Error::Corrupt
        }
    })?;
    if record
        .metadata
        .get(NATIVE_SESSION_METADATA_KEY)
        .and_then(|value| value.get("schema_version"))
        .and_then(Value::as_u64)
        == Some(2)
    {
        return Ok(false);
    }
    record
        .metadata
        .insert(NATIVE_SESSION_METADATA_KEY.to_owned(), metadata.to_value());
    record.revision = SessionRevision(record.revision.0.checked_add(1).ok_or(Error::Corrupt)?);
    Ok(true)
}

fn decode(bytes: &[u8], id: &SessionId) -> Result<SessionRecord, Error> {
    recovery::check_duplicate_keys(bytes)?;
    let ObjectOnly(envelope) =
        serde_json::from_slice::<ObjectOnly<StoredEnvelope>>(bytes).map_err(|_| Error::Corrupt)?;
    if envelope.schema_version != FILE_SESSION_SCHEMA_VERSION {
        return Err(Error::UnsupportedVersion);
    }
    let record = SessionRecord::from(envelope.record.0);
    if &record.id != id
        || record.revision.0 == 0
        || record.next_turn_sequence == 0
        || validate_record_json(&record).is_err()
    {
        return Err(Error::Corrupt);
    }
    Ok(record)
}

struct Source {
    bytes: Vec<u8>,
    file: OwnedFd,
    stat: rustix::fs::Stat,
}
impl Source {
    fn check(&self, root: rustix::fd::BorrowedFd<'_>, name: &str) -> Result<(), Error> {
        let current = rustix::fs::fstat(&self.file)?;
        let linked = rustix::fs::statat(root, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if !same(&self.stat, &current) || !same(&self.stat, &linked) {
            return Err(Error::Busy);
        }
        Ok(())
    }
}
fn same(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> bool {
    left.st_dev == right.st_dev
        && left.st_ino == right.st_ino
        && left.st_size == right.st_size
        && left.st_mtime == right.st_mtime
        && left.st_mtime_nsec == right.st_mtime_nsec
        && left.st_ctime == right.st_ctime
        && left.st_ctime_nsec == right.st_ctime_nsec
        && left.st_nlink == right.st_nlink
}
fn read_source(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
    control: &FileSessionScanControl,
) -> Result<Source, Error> {
    read_source_bounded(root, name, control, MAX_FILE_SESSION_BYTES, &mut 0)
}
fn read_source_bounded(
    root: rustix::fd::BorrowedFd<'_>,
    name: &str,
    control: &FileSessionScanControl,
    byte_limit: usize,
    bytes_read: &mut usize,
) -> Result<Source, Error> {
    control.check()?;
    let file = rustix::fs::openat(
        root,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| {
        if error == rustix::io::Errno::NOENT {
            Error::Missing
        } else {
            Error::Unavailable
        }
    })?;
    let stat = ensure_regular(&file).map_err(|_| Error::Corrupt)?;
    let size = usize::try_from(stat.st_size).map_err(|_| Error::Corrupt)?;
    let byte_limit = byte_limit.min(MAX_FILE_SESSION_BYTES);
    if size > byte_limit {
        return Err(Error::Oversized);
    }
    let mut bytes = Vec::with_capacity(size);
    let mut buffer = [0_u8; 8192];
    loop {
        control.check()?;
        let limit = buffer.len().min(byte_limit + 1 - bytes.len());
        let count = match rustix::io::read(&file, &mut buffer[..limit]) {
            Err(rustix::io::Errno::INTR) => continue,
            result => result?,
        };
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        *bytes_read += count;
        #[cfg(test)]
        if let Some(hook) = &control.after_read {
            hook(bytes.len());
        }
        if bytes.len() > byte_limit {
            return Err(Error::Oversized);
        }
    }
    control.check()?;
    let source = Source { bytes, file, stat };
    source.check(root, name)?;
    Ok(source)
}

fn publish(
    root: rustix::fd::BorrowedFd<'_>,
    names: &SessionNames,
    record: &SessionRecord,
    control: &FileSessionScanControl,
    before_commit: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    control.check()?;
    let bytes = serialize_record(record).map_err(|_| Error::Oversized)?;
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| Error::Unavailable)?;
    let digest = format!("{:x}", Sha256::digest(random));
    let temp_name = format!("{}.maintenance-{}.tmp", names.data, &digest[..32]);
    let temp = create_new_temp(root, &temp_name)?;
    rustix::fs::fchmod(&temp, Mode::RUSR | Mode::WUSR)?;
    let result = (|| {
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            control.check()?;
            match rustix::io::write(&temp, &remaining[..remaining.len().min(8192)]) {
                Ok(0) => return Err(Error::Unavailable),
                Ok(count) => remaining = &remaining[count..],
                Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
        loop {
            control.check()?;
            match rustix::fs::fsync(&temp) {
                Ok(()) => break,
                Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
        control.check()?;
        ensure_listing_root_is_linked(root).map_err(|_| Error::Unavailable)?;
        #[cfg(test)]
        tests::publication_checkpoint(tests::PublicationStage::BeforeCommit)?;
        before_commit()?;
        let stat = rustix::fs::fstat(&temp)?;
        let linked = rustix::fs::statat(root, &temp_name, AtFlags::SYMLINK_NOFOLLOW)?;
        if !same(&stat, &linked) {
            return Err(Error::Busy);
        }
        control.check()?;
        rustix::fs::renameat(root, &temp_name, root, &names.data)?;
        #[cfg(test)]
        tests::publication_checkpoint(tests::PublicationStage::AfterCommit)
            .map_err(|_| Error::Indeterminate)?;
        // No cancellation after publication: an uncertain durable receipt is not a rollback.
        rustix::fs::fsync(root).map_err(|_| Error::Indeterminate)
    })();
    if result.is_err() {
        // Remove only this operation's exact still-linked private staging object.
        if let (Ok(stat), Ok(linked)) = (
            rustix::fs::fstat(&temp),
            rustix::fs::statat(root, &temp_name, AtFlags::SYMLINK_NOFOLLOW),
        ) && same(&stat, &linked)
        {
            let _ = rustix::fs::unlinkat(root, &temp_name, AtFlags::empty());
        }
    }
    result
}

#[cfg(test)]
#[path = "storage/tests.rs"]
mod tests;
