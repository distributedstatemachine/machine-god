use super::{JournalError as Error, JournalLimits, Shared};
use crate::bounded_profile_file::{read_bounded, validate_link, validate_private, write_bounded};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, Dir, FlockOperation, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::sync::Arc;

mod accounting;
pub(super) use accounting::{PublicationAccounting, Usage, scan_usage};

const READ: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
const OWNER: &str = ".owner.lock";
const EPOCH: &str = ".owner-epoch";
pub(super) const FILE_OVERHEAD: usize = 256;

pub(super) fn workspace_directory(
    root: &OwnedFd,
    workspace: &std::path::Path,
    origin: crate::NativeSessionOrigin,
) -> Result<OwnedFd, Error> {
    use std::os::unix::ffi::OsStrExt;
    validate_private(root, true).map_err(|_| Error::Invalid)?;
    let mut hash = Sha256::new();
    hash.update(b"machine-god-managed-workspace-v1\0");
    hash.update(origin.as_str().as_bytes());
    hash.update([0]);
    hash.update(workspace.as_os_str().as_bytes());
    let name = format!("managed-{}", hex(&hash.finalize()));
    match rustix::fs::mkdirat(root, &name, Mode::RWXU) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(_) => return Err(Error::Persistence),
    }
    let directory = rustix::fs::openat(root, &name, READ | OFlags::DIRECTORY, Mode::empty())
        .map_err(|_| Error::Invalid)?;
    validate_private(&directory, true).map_err(|_| Error::Invalid)?;
    validate_link(root, &name, &directory).map_err(|_| Error::Conflict)?;
    rustix::fs::fsync(root).map_err(|_| Error::Persistence)?;
    Ok(directory)
}

pub(super) struct Source {
    file: File,
    revision: [i128; 11],
}
impl Source {
    pub(super) fn revision(&self) -> [i128; 11] {
        self.revision
    }
}
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Source { .. }")
    }
}
fn revision(file: &impl rustix::fd::AsFd) -> Result<[i128; 11], Error> {
    let stat = rustix::fs::fstat(file).map_err(|_| Error::Persistence)?;
    Ok([
        i128::from(stat.st_dev),
        i128::from(stat.st_ino),
        i128::from(stat.st_size),
        i128::from(stat.st_mtime),
        i128::from(stat.st_mtime_nsec),
        i128::from(stat.st_ctime),
        i128::from(stat.st_ctime_nsec),
        i128::from(stat.st_mode),
        i128::from(stat.st_nlink),
        i128::from(stat.st_uid),
        i128::from(stat.st_gid),
    ])
}
pub(super) fn directory_revision(root: &OwnedFd) -> Result<[i128; 11], Error> {
    revision(root)
}
pub(super) fn validate_source(root: &OwnedFd, name: &str, source: &Source) -> Result<(), Error> {
    validate_private(&source.file, false).map_err(|_| Error::Invalid)?;
    validate_link(root, name, &source.file).map_err(|_| Error::Conflict)?;
    if revision(&source.file)? != source.revision {
        return Err(Error::Conflict);
    }
    Ok(())
}

pub(super) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").expect("string writer");
    }
    result
}
pub(super) fn temporary_name(bytes: &[u8]) -> String {
    format!("t-{}.tmp", hex(&digest(bytes)))
}
pub(super) fn head_name(id: &str) -> String {
    format!("h-{}.json", hex(&digest(id.as_bytes())))
}
pub(super) fn page_name(reference: &super::JournalPageRef) -> String {
    format!(
        "p-{}-{}-{}-{}.json",
        hex(&digest(reference.child_id.as_bytes())),
        reference.generation,
        reference.sequence,
        hex(&reference.digest)
    )
}

pub(super) fn acquire(
    root: &OwnedFd,
    limits: JournalLimits,
) -> Result<(OwnedFd, Usage, u64, [i128; 11]), Error> {
    validate_private(root, true).map_err(|_| Error::Invalid)?;
    let flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let owner = match rustix::fs::openat(
        root,
        OWNER,
        flags | OFlags::CREATE | OFlags::EXCL,
        Mode::RUSR | Mode::WUSR,
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::EXIST) => {
            rustix::fs::openat(root, OWNER, flags, Mode::empty()).map_err(|_| Error::Invalid)?
        }
        Err(_) => return Err(Error::Persistence),
    };
    validate_private(&owner, false).map_err(|_| Error::Invalid)?;
    rustix::fs::flock(&owner, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
        if error == rustix::io::Errno::WOULDBLOCK {
            Error::Busy
        } else {
            Error::Persistence
        }
    })?;
    validate_link(root, OWNER, &owner).map_err(|_| Error::Conflict)?;
    rustix::fs::fsync(&owner).map_err(|_| Error::Persistence)?;
    rustix::fs::fsync(root).map_err(|_| Error::Persistence)?;
    let used = scan_usage(root, limits)?;
    if used
        .bytes
        .checked_add(2 * FILE_OVERHEAD + 40)
        .is_none_or(|n| n > limits.aggregate_bytes)
        || used
            .entries
            .checked_add(1)
            .is_none_or(|n| n > limits.directory_entries)
    {
        return Err(Error::Limit);
    }
    let epoch = advance_epoch(root, &owner)?;
    let namespace_revision = directory_revision(root)?;
    let used = scan_usage(root, limits)?;
    if directory_revision(root)? != namespace_revision {
        return Err(Error::Conflict);
    }
    Ok((owner, used, epoch, namespace_revision))
}

fn advance_epoch(root: &OwnedFd, owner: &OwnedFd) -> Result<u64, Error> {
    let previous = read(root, EPOCH, 20)?;
    let epoch = match previous {
        None => 1,
        Some(bytes) => {
            let text = std::str::from_utf8(&bytes).map_err(|_| Error::Invalid)?;
            let value: u64 = text.parse().map_err(|_| Error::Invalid)?;
            if value == 0 || value.to_string() != text {
                return Err(Error::Invalid);
            }
            value.checked_add(1).ok_or(Error::Exhausted)?
        }
    };
    let name = "t-owner-epoch.tmp";
    match rustix::fs::openat(root, name, READ, Mode::empty()) {
        Ok(old) => {
            validate_private(&old, false).map_err(|_| Error::Invalid)?;
            validate_link(root, name, &old).map_err(|_| Error::Conflict)?;
            rustix::fs::unlinkat(root, name, AtFlags::empty()).map_err(|_| Error::Persistence)?;
            rustix::fs::fsync(root).map_err(|_| Error::Persistence)?;
        }
        Err(rustix::io::Errno::NOENT) => {}
        Err(_) => return Err(Error::Invalid),
    }
    let fd = rustix::fs::openat(
        root,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|_| Error::Persistence)?;
    validate_private(&fd, false).map_err(|_| Error::Invalid)?;
    let mut file = File::from(fd);
    write_bounded(&mut file, epoch.to_string().as_bytes()).map_err(|_| Error::Persistence)?;
    file.sync_all().map_err(|_| Error::Persistence)?;
    validate_link(root, name, &file).map_err(|_| Error::Conflict)?;
    validate_link(root, OWNER, owner).map_err(|_| Error::Conflict)?;
    rustix::fs::renameat(root, name, root, EPOCH).map_err(|_| Error::Persistence)?;
    rustix::fs::fsync(root).map_err(|_| Error::Persistence)?;
    validate_link(root, EPOCH, &file).map_err(|_| Error::Conflict)?;
    Ok(epoch)
}

pub(super) fn validate_owner(shared: &Shared) -> Result<(), Error> {
    validate_private(&shared.root, true).map_err(|_| Error::Invalid)?;
    validate_link(&shared.root, OWNER, &shared.owner_lock).map_err(|_| Error::Conflict)
}

pub(super) fn head_candidates(
    shared: &Shared,
    after: &str,
    limit: usize,
) -> Result<Vec<String>, Error> {
    let directory = rustix::fs::openat(&shared.root, ".", READ | OFlags::DIRECTORY, Mode::empty())
        .map_err(|_| Error::Persistence)?;
    let mut stream = Dir::new(directory).map_err(|_| Error::Persistence)?;
    let mut candidates = Vec::new();
    let mut count = 0;
    for entry in &mut stream {
        let entry = entry.map_err(|_| Error::Persistence)?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        count += 1;
        if count > shared.limits.directory_entries {
            return Err(Error::Limit);
        }
        let name = std::str::from_utf8(name).map_err(|_| Error::Invalid)?;
        if !name.starts_with("h-") {
            continue;
        }
        if name.len() != 71
            || name.as_bytes().get(66..) != Some(b".json")
            || !name.as_bytes()[2..66].iter().all(u8::is_ascii_hexdigit)
        {
            return Err(Error::Invalid);
        }
        if name <= after {
            continue;
        }
        let position = candidates.partition_point(|value: &String| value.as_str() < name);
        if position < limit {
            candidates.insert(position, name.to_owned());
            candidates.truncate(limit);
        }
    }
    Ok(candidates)
}

pub(super) fn read(root: &OwnedFd, name: &str, limit: usize) -> Result<Option<Vec<u8>>, Error> {
    Ok(observe(root, name, limit)?.map(|(bytes, _)| bytes))
}
type Observation = (Vec<u8>, Arc<Source>);

pub(super) fn observe(
    root: &OwnedFd,
    name: &str,
    limit: usize,
) -> Result<Option<Observation>, Error> {
    let fd = match rustix::fs::openat(root, name, READ, Mode::empty()) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(_) => return Err(Error::Invalid),
    };
    validate_private(&fd, false).map_err(|_| Error::Invalid)?;
    validate_link(root, name, &fd).map_err(|_| Error::Conflict)?;
    let expected = revision(&fd)?;
    let mut file = File::from(fd);
    let bytes = read_bounded(&mut file, limit).map_err(|_| Error::Invalid)?;
    validate_link(root, name, &file).map_err(|_| Error::Conflict)?;
    let source = Arc::new(Source {
        file,
        revision: expected,
    });
    validate_source(root, name, &source)?;
    Ok(Some((bytes, source)))
}

pub(super) fn durable(
    shared: &Shared,
    name: &str,
    expected: &[u8],
    limit: usize,
) -> Result<(), Error> {
    let fd =
        rustix::fs::openat(&shared.root, name, READ, Mode::empty()).map_err(|_| Error::Invalid)?;
    validate_private(&fd, false).map_err(|_| Error::Invalid)?;
    validate_link(&shared.root, name, &fd).map_err(|_| Error::Conflict)?;
    rustix::fs::fsync(&fd).map_err(|_| Error::Persistence)?;
    rustix::fs::fsync(&shared.root).map_err(|_| Error::Persistence)?;
    if read(&shared.root, name, limit)?.as_deref() != Some(expected) {
        return Err(Error::Conflict);
    }
    validate_link(&shared.root, name, &fd).map_err(|_| Error::Conflict)?;
    validate_owner(shared)
}

/// Caller has reserved old/new/temp/receipt space and owns the operation slot.
/// Any failure is reconciled by the transaction owner, never presumed rollback.
pub(super) fn publish(
    shared: &Shared,
    name: &str,
    bytes: &[u8],
    immutable: bool,
    expected: Option<&Source>,
) -> Result<(), Error> {
    if immutable && let Some(existing) = read(&shared.root, name, shared.limits.page_bytes)? {
        if existing != bytes {
            return Err(Error::Conflict);
        }
        return durable(shared, name, bytes, shared.limits.page_bytes);
    }
    let temp_name = temporary_name(bytes);
    // Only a journal temporary name can be removed here; no head/page references
    // a temporary. The exact descriptor and private/link checks precede unlink.
    if let Ok(old) = rustix::fs::openat(&shared.root, &temp_name, READ, Mode::empty()) {
        validate_private(&old, false).map_err(|_| Error::Invalid)?;
        validate_link(&shared.root, &temp_name, &old).map_err(|_| Error::Conflict)?;
        rustix::fs::unlinkat(&shared.root, &temp_name, AtFlags::empty())
            .map_err(|_| Error::Persistence)?;
        rustix::fs::fsync(&shared.root).map_err(|_| Error::Persistence)?;
    }
    let fd = rustix::fs::openat(
        &shared.root,
        &temp_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|_| Error::Persistence)?;
    validate_private(&fd, false).map_err(|_| Error::Invalid)?;
    let mut file = File::from(fd);
    write_bounded(&mut file, bytes).map_err(|_| Error::Persistence)?;
    file.sync_all().map_err(|_| Error::Persistence)?;
    validate_link(&shared.root, &temp_name, &file).map_err(|_| Error::Conflict)?;
    validate_owner(shared)?;
    if !immutable {
        if let Some(expected) = expected {
            validate_source(&shared.root, name, expected)?;
        } else if read(&shared.root, name, shared.limits.head_bytes)?.is_some() {
            return Err(Error::Conflict);
        }
    }
    #[cfg(test)]
    super::tests::checkpoint(
        shared,
        if immutable {
            super::tests::FailurePoint::BeforePageRename
        } else {
            super::tests::FailurePoint::BeforeHeadRename
        },
    )?;
    rustix::fs::renameat(&shared.root, &temp_name, &shared.root, name)
        .map_err(|_| Error::Persistence)?;
    #[cfg(test)]
    super::tests::checkpoint(
        shared,
        if immutable {
            super::tests::FailurePoint::AfterPageRename
        } else {
            super::tests::FailurePoint::AfterHeadRename
        },
    )?;
    rustix::fs::fsync(&shared.root).map_err(|_| Error::Persistence)?;
    validate_link(&shared.root, name, &file).map_err(|_| Error::Conflict)?;
    validate_owner(shared)
}
