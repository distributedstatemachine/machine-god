//! Shared bounded, descriptor-relative profile file transactions.
//!
//! This module knows fixed filenames and filesystem policy, never configuration
//! schemas. Callers validate candidates before requesting publication effects.

use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, FlockOperation, Mode, OFlags};

pub(crate) mod parents;
mod source;
#[cfg(test)]
mod tests;

use source::ObservedFile;

const READ: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProfileFileKind {
    Settings,
    Mcp,
    #[cfg(feature = "mcp-http")]
    McpCredentials,
}

impl ProfileFileKind {
    fn data(self) -> &'static str {
        match self {
            Self::Settings => "config.json",
            Self::Mcp => "mcp.json",
            #[cfg(feature = "mcp-http")]
            Self::McpCredentials => "mcp-credentials.json",
        }
    }
    fn lock(self) -> &'static str {
        match self {
            Self::Settings => ".config.lock",
            Self::Mcp => ".mcp.lock",
            #[cfg(feature = "mcp-http")]
            Self::McpCredentials => ".mcp-credentials.lock",
        }
    }
    fn temp(self) -> &'static str {
        match self {
            Self::Settings => ".config.tmp",
            Self::Mcp => ".mcp.tmp",
            #[cfg(feature = "mcp-http")]
            Self::McpCredentials => ".mcp-credentials.tmp",
        }
    }
    fn limit(self) -> usize {
        match self {
            Self::Settings => crate::MAX_CONFIG_BYTES,
            Self::Mcp => crate::mcp::config::MAX_CONFIG_BYTES,
            #[cfg(feature = "mcp-http")]
            Self::McpCredentials => 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProfileFileError {
    UnsafePath,
    Busy,
    Conflict,
    Persistence,
    TooLarge,
    Unreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdateMode {
    CompareAndSwap,
    MergeLatest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublicationDurability {
    Confirmed,
    Ambiguous,
}

pub(crate) struct ProfileFile {
    directory: PathBuf,
    kind: ProfileFileKind,
    identity: Arc<()>,
}

pub(crate) struct ProfileObservation {
    parent: parents::ParentObservation,
    root: Option<OwnedFd>,
    current: ObservedFile,
    identity: Arc<()>,
}

impl ProfileObservation {
    pub(crate) fn bytes(&self) -> Option<&[u8]> {
        self.current.bytes()
    }
}

impl fmt::Debug for ProfileFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileFile")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for ProfileObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileObservation").finish_non_exhaustive()
    }
}

pub(crate) fn validate_directory(directory: &Path) -> Result<&std::ffi::OsStr, ProfileFileError> {
    if !directory.is_absolute()
        || directory.as_os_str().as_bytes().len() > 4096
        || directory.as_os_str().as_bytes().contains(&0)
        || directory.components().count() > 64
        || directory
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(ProfileFileError::UnsafePath);
    }
    directory.file_name().ok_or(ProfileFileError::UnsafePath)
}

impl ProfileFile {
    pub(crate) fn new(directory: PathBuf, kind: ProfileFileKind) -> Self {
        Self {
            directory,
            kind,
            identity: Arc::new(()),
        }
    }

    pub(crate) fn observe(&self) -> Result<ProfileObservation, ProfileFileError> {
        let name = validate_directory(&self.directory)?;
        let parent = parents::ParentObservation::observe(
            self.directory
                .parent()
                .ok_or(ProfileFileError::UnsafePath)?,
        )?;
        let resolved = parent.resolve(false)?;
        let root = resolved
            .as_ref()
            .map(|p| open_root(p.descriptor(), name))
            .transpose()?
            .flatten();
        let current = root
            .as_ref()
            .map(|r| source::read_current(r, self.kind))
            .transpose()?
            .unwrap_or_default();
        if let Some(resolved) = &resolved {
            resolved.validate()?;
            if self.kind != ProfileFileKind::Settings
                && let Some(root) = &root
            {
                validate_link(resolved.descriptor(), name, root)?;
            }
        }
        drop(resolved);
        Ok(ProfileObservation {
            parent,
            root,
            current,
            identity: self.identity.clone(),
        })
    }

    pub(crate) fn validate_identity(
        &self,
        observed: &ProfileObservation,
    ) -> Result<(), ProfileFileError> {
        if Arc::ptr_eq(&self.identity, &observed.identity) {
            Ok(())
        } else {
            Err(ProfileFileError::Conflict)
        }
    }

    pub(crate) fn validate_unchanged(
        &self,
        observed: &ProfileObservation,
    ) -> Result<(), ProfileFileError> {
        self.validate_identity(observed)?;
        let name = validate_directory(&self.directory)?;
        let Some(parent) = observed.parent.resolve(false)? else {
            return if observed.root.is_none() && observed.bytes().is_none() {
                Ok(())
            } else {
                Err(ProfileFileError::Conflict)
            };
        };
        let current = match (&observed.root, open_root(parent.descriptor(), name)?) {
            (Some(expected), Some(actual)) if same_file(expected, &actual)? => {
                let current = source::read_current(&actual, self.kind)?;
                validate_link(parent.descriptor(), name, &actual)?;
                current
            }
            (None, None) => ObservedFile::default(),
            _ => return Err(ProfileFileError::Conflict),
        };
        observed.current.compare(&current, self.kind)?;
        parent.validate()
    }

    pub(crate) fn begin<'a>(
        &'a self,
        observed: &'a ProfileObservation,
        mode: UpdateMode,
    ) -> Result<LockedProfileUpdate<'a>, ProfileFileError> {
        self.validate_identity(observed)?;
        let name = validate_directory(&self.directory)?;
        let parent = observed
            .parent
            .resolve(true)?
            .ok_or(ProfileFileError::Persistence)?;
        let root = match (&observed.root, open_root(parent.descriptor(), name)?) {
            (Some(expected), Some(actual)) if same_file(expected, &actual)? => actual,
            (None, Some(actual)) if mode == UpdateMode::MergeLatest => actual,
            (None, None) => {
                match rustix::fs::mkdirat(parent.descriptor(), name, Mode::from_raw_mode(0o700)) {
                    Ok(()) => {}
                    Err(error)
                        if error == rustix::io::Errno::EXIST && mode == UpdateMode::MergeLatest => {
                    }
                    Err(_) => {
                        return Err(if mode == UpdateMode::CompareAndSwap {
                            ProfileFileError::Conflict
                        } else {
                            ProfileFileError::Persistence
                        });
                    }
                }
                let root =
                    open_root(parent.descriptor(), name)?.ok_or(ProfileFileError::Persistence)?;
                rustix::fs::fsync(parent.descriptor())
                    .map_err(|_| ProfileFileError::Persistence)?;
                root
            }
            _ => return Err(ProfileFileError::Conflict),
        };
        parent.validate()?;
        let lock = lock_config(open_lock(&root, self.kind)?, self.kind)?;
        parent.validate()?;
        validate_link(parent.descriptor(), name, &root)?;
        validate_link(&root, self.kind.lock(), &lock.fd)?;
        let current = source::read_current(&root, self.kind)?;
        Ok(LockedProfileUpdate {
            parent,
            name,
            root,
            lock,
            current,
            kind: self.kind,
            observed,
            mode,
        })
    }
}

pub(crate) struct LockedProfileUpdate<'a> {
    parent: parents::ResolvedParent<'a>,
    name: &'a std::ffi::OsStr,
    root: OwnedFd,
    lock: ConfigLockGuard,
    current: ObservedFile,
    kind: ProfileFileKind,
    observed: &'a ProfileObservation,
    mode: UpdateMode,
}

impl LockedProfileUpdate<'_> {
    pub(crate) fn current_bytes(&self) -> Option<&[u8]> {
        self.current.bytes()
    }

    pub(crate) fn publish(
        self,
        encoded: &[u8],
        sync_directory: impl FnOnce(&OwnedFd) -> rustix::io::Result<()>,
    ) -> Result<PublicationDurability, ProfileFileError> {
        if self.mode == UpdateMode::CompareAndSwap {
            self.observed.current.compare(&self.current, self.kind)?;
        }
        if encoded.len() > self.kind.limit() {
            return Err(ProfileFileError::TooLarge);
        }
        let temp = rustix::fs::openat(
            &self.root,
            self.kind.temp(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| ProfileFileError::Persistence)?;
        let mut temp = File::from(temp);
        if rustix::fs::fchmod(&temp, Mode::RUSR | Mode::WUSR).is_err()
            || validate_private(&temp, false).is_err()
            || write_bounded(&mut temp, encoded).is_err()
            || temp.sync_all().is_err()
        {
            remove_owned_temp(&self.root, &temp, self.kind);
            return Err(ProfileFileError::Persistence);
        }
        let checked = (|| {
            self.parent.validate()?;
            validate_link(self.parent.descriptor(), self.name, &self.root)?;
            validate_link(&self.root, self.kind.lock(), &self.lock.fd)?;
            validate_link(&self.root, self.kind.temp(), &temp)?;
            self.current
                .compare(&source::read_current(&self.root, self.kind)?, self.kind)
        })();
        if let Err(error) = checked {
            remove_owned_temp(&self.root, &temp, self.kind);
            return Err(error);
        }
        if rustix::fs::renameat(&self.root, self.kind.temp(), &self.root, self.kind.data()).is_err()
        {
            remove_owned_temp(&self.root, &temp, self.kind);
            return Err(ProfileFileError::Persistence);
        }
        let checked = (|| {
            sync_directory(&self.root).map_err(|_| ProfileFileError::Persistence)?;
            self.parent.validate()?;
            validate_link(self.parent.descriptor(), self.name, &self.root)?;
            if self.kind != ProfileFileKind::Settings {
                validate_link(&self.root, self.kind.data(), &temp)?;
                if source::read_current(&self.root, self.kind)?.bytes() != Some(encoded) {
                    return Err(ProfileFileError::Conflict);
                }
                validate_link(&self.root, self.kind.data(), &temp)?;
                self.parent.validate()?;
                validate_link(self.parent.descriptor(), self.name, &self.root)?;
            }
            Ok(())
        })();
        Ok(if checked.is_ok() {
            PublicationDurability::Confirmed
        } else {
            PublicationDurability::Ambiguous
        })
    }
}

#[must_use = "retain the guard until the profile transaction completes"]
struct ConfigLockGuard {
    fd: OwnedFd,
    kind: ProfileFileKind,
}
impl Drop for ConfigLockGuard {
    fn drop(&mut self) {
        // Explicit unlock is needed even if a duplicated open description survives.
        // MCP bounds interruptions; closing this descriptor remains the fallback.
        let _ = retry_lock_interrupted(self.kind, || {
            rustix::fs::flock(&self.fd, FlockOperation::Unlock)
        });
    }
}

fn lock_config(fd: OwnedFd, kind: ProfileFileKind) -> Result<ConfigLockGuard, ProfileFileError> {
    retry_lock_interrupted(kind, || {
        rustix::fs::flock(&fd, FlockOperation::NonBlockingLockExclusive)
    })
    .map_err(|error| {
        if error == rustix::io::Errno::WOULDBLOCK {
            ProfileFileError::Busy
        } else {
            ProfileFileError::Persistence
        }
    })?;
    Ok(ConfigLockGuard { fd, kind })
}

fn retry_lock_interrupted<T>(
    kind: ProfileFileKind,
    mut operation: impl FnMut() -> rustix::io::Result<T>,
) -> rustix::io::Result<T> {
    let mut interruptions = 0;
    loop {
        match operation() {
            Err(rustix::io::Errno::INTR) => {
                if kind != ProfileFileKind::Settings {
                    interruptions += 1;
                    if interruptions == 16 {
                        return Err(rustix::io::Errno::INTR);
                    }
                }
            }
            result => return result,
        }
    }
}

fn remove_owned_temp(root: &OwnedFd, temp: &File, kind: ProfileFileKind) {
    if validate_link(root, kind.temp(), temp).is_ok() {
        let _ = rustix::fs::unlinkat(root, kind.temp(), AtFlags::empty());
    }
}

fn write_bounded(writer: &mut impl Write, mut bytes: &[u8]) -> std::io::Result<()> {
    let mut interruptions = 0;
    while !bytes.is_empty() {
        match writer.write(bytes) {
            Ok(written) if written > 0 && written <= bytes.len() => bytes = &bytes[written..],
            Ok(_) => return Err(std::io::ErrorKind::WriteZero.into()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                interruptions += 1;
                if interruptions >= 16 {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn read_bounded(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>, ProfileFileError> {
    let mut bytes = vec![0; limit + 1];
    let mut length = 0;
    let mut interruptions = 0;
    loop {
        match reader.read(&mut bytes[length..]) {
            Ok(0) => break,
            Ok(read) if read <= bytes.len() - length => {
                length += read;
                if length > limit {
                    return Err(ProfileFileError::TooLarge);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                interruptions += 1;
                if interruptions == 16 {
                    return Err(ProfileFileError::Unreadable);
                }
            }
            _ => return Err(ProfileFileError::Unreadable),
        }
    }
    bytes.truncate(length);
    Ok(bytes.into_boxed_slice().into_vec())
}

fn open_lock(root: &OwnedFd, kind: ProfileFileKind) -> Result<OwnedFd, ProfileFileError> {
    let flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let fd = match rustix::fs::openat(
        root,
        kind.lock(),
        flags | OFlags::CREATE | OFlags::EXCL,
        Mode::from_raw_mode(0o600),
    ) {
        Ok(fd) => {
            rustix::fs::fchmod(&fd, Mode::RUSR | Mode::WUSR)
                .map_err(|_| ProfileFileError::Persistence)?;
            fd
        }
        Err(error) if error == rustix::io::Errno::EXIST => {
            rustix::fs::openat(root, kind.lock(), flags, Mode::empty())
                .map_err(|_| ProfileFileError::UnsafePath)?
        }
        Err(_) => return Err(ProfileFileError::Persistence),
    };
    validate_private(&fd, false)?;
    Ok(fd)
}

fn open_root(
    parent: &OwnedFd,
    name: &std::ffi::OsStr,
) -> Result<Option<OwnedFd>, ProfileFileError> {
    match rustix::fs::openat(parent, name, READ | OFlags::DIRECTORY, Mode::empty()) {
        Ok(root) => {
            validate_private(&root, true)?;
            Ok(Some(root))
        }
        Err(e) if e == rustix::io::Errno::NOENT => Ok(None),
        Err(_) => Err(ProfileFileError::UnsafePath),
    }
}

fn validate_private(fd: &impl rustix::fd::AsFd, directory: bool) -> Result<(), ProfileFileError> {
    let stat = rustix::fs::fstat(fd).map_err(|_| ProfileFileError::Persistence)?;
    let kind = FileType::from_raw_mode(stat.st_mode);
    if stat.st_uid != nix::unistd::Uid::effective().as_raw()
        || stat.st_mode & 0o077 != 0
        || if directory {
            !kind.is_dir()
        } else {
            !kind.is_file() || stat.st_nlink != 1
        }
    {
        return Err(ProfileFileError::UnsafePath);
    }
    #[cfg(target_os = "macos")]
    {
        let acl =
            calcifer_macos_acl::read_acl(fd.as_fd()).map_err(|_| ProfileFileError::UnsafePath)?;
        if acl.flags != 0
            || acl.entries.iter().any(|entry| {
                entry.tag != calcifer_macos_acl::TAG_DENY
                    || entry.flags != 0
                    || entry.permissions != calcifer_macos_acl::PERMISSION_DELETE
            })
        {
            return Err(ProfileFileError::UnsafePath);
        }
    }
    Ok(())
}

fn same_file(
    a: &impl rustix::fd::AsFd,
    b: &impl rustix::fd::AsFd,
) -> Result<bool, ProfileFileError> {
    let a = rustix::fs::fstat(a).map_err(|_| ProfileFileError::Persistence)?;
    let b = rustix::fs::fstat(b).map_err(|_| ProfileFileError::Persistence)?;
    Ok(a.st_dev == b.st_dev && a.st_ino == b.st_ino)
}

fn validate_link(
    parent: &OwnedFd,
    name: impl rustix::path::Arg,
    fd: &impl rustix::fd::AsFd,
) -> Result<(), ProfileFileError> {
    let linked = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| ProfileFileError::Conflict)?;
    let opened = rustix::fs::fstat(fd).map_err(|_| ProfileFileError::Persistence)?;
    if linked.st_dev != opened.st_dev || linked.st_ino != opened.st_ino {
        return Err(ProfileFileError::Conflict);
    }
    Ok(())
}
