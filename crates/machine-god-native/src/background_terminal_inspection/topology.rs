use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{Dir, FlockOperation, Mode, OFlags};

use super::{Error, HistoryReadBudget, Result};
use crate::terminal_catalog::{private, same_entry};
use crate::terminal_profile_store::{MAX_PROFILE_OWNERS, MAX_PROFILE_SESSIONS};
use machine_god_core::TerminalSessionId;

const NAMESPACE: &str = "terminal-v1";
const LOCK: &str = "profile-lock";
const MAX_OWNER_SESSIONS: usize = 256;

pub(super) struct ReadProfile<'a> {
    state_root: BorrowedFd<'a>,
    namespace: OwnedFd,
    lock: SharedLock,
}

struct SharedLock(OwnedFd);

impl Drop for SharedLock {
    fn drop(&mut self) {
        // End this observation even if a concurrent fork inherited its fd.
        let _ = rustix::fs::flock(&self.0, FlockOperation::Unlock);
    }
}

impl<'a> ReadProfile<'a> {
    pub(super) fn open(state_root: BorrowedFd<'a>) -> Result<Option<Self>> {
        private(state_root, true)?;
        let namespace = match open_directory(state_root, NAMESPACE) {
            Ok(directory) => directory,
            Err(Error::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        let lock = open_lock(&namespace, LOCK).map_err(missing_is_corrupt)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockShared).map_err(io_error)?;
        let profile = Self {
            state_root,
            namespace,
            lock: SharedLock(lock),
        };
        profile.validate()?;
        Ok(Some(profile))
    }

    pub(super) fn validate(&self) -> Result<()> {
        private(self.state_root, true)?;
        same_entry(self.state_root, NAMESPACE, &self.namespace, true)?;
        same_entry(&self.namespace, LOCK, &self.lock.0, false)?;
        Ok(())
    }

    pub(super) fn topology(&self, budget: &HistoryReadBudget<'_>) -> Result<Vec<Owner>> {
        self.validate()?;
        let entries = names(&self.namespace, MAX_PROFILE_OWNERS + 1, budget)?;
        if !entries.iter().any(|name| name == LOCK) {
            return Err(Error::Corrupt);
        }
        let mut owners = Vec::new();
        let mut count = 0_usize;
        for name in entries {
            budget.checkpoint()?;
            if name == LOCK {
                continue;
            }
            if name.len() != 64
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(Error::Corrupt);
            }
            let root = open_directory(&self.namespace, &name)?;
            let mut owner = Owner {
                name,
                identity: identity(&root, true)?,
                lock: None,
                sessions_root: None,
                sessions: Vec::new(),
            };
            for entry in names_in_owner(&root, budget)? {
                match entry.as_str() {
                    "catalog-lock" => {
                        owner.lock = Some(identity(open_lock(&root, &entry)?, false)?);
                    }
                    "sessions" => {
                        let sessions = open_directory(&root, &entry)?;
                        owner.sessions_root = Some(identity(&sessions, true)?);
                        for name in names(&sessions, MAX_OWNER_SESSIONS, budget)? {
                            budget.checkpoint()?;
                            count += 1;
                            if count > MAX_PROFILE_SESSIONS {
                                return Err(Error::ResourceLimit);
                            }
                            let id = TerminalSessionId::new(name).map_err(|_| Error::Corrupt)?;
                            let directory = open_directory(&sessions, id.as_str())?;
                            owner.sessions.push(Session {
                                id,
                                identity: identity(directory, true)?,
                            });
                        }
                        same_entry(&root, &entry, &sessions, true)?;
                    }
                    _ => return Err(Error::Corrupt),
                }
            }
            if !owner.sessions.is_empty() && owner.lock.is_none() {
                return Err(Error::Corrupt);
            }
            same_entry(&self.namespace, &owner.name, &root, true)?;
            owners.push(owner);
        }
        self.validate()?;
        Ok(owners)
    }

    pub(super) fn open_session(&self, owner: &Owner, session: &Session) -> Result<OwnedFd> {
        self.validate()?;
        let root = open_directory(&self.namespace, &owner.name)?;
        let sessions = open_directory(&root, "sessions")?;
        let directory = open_directory(&sessions, session.id.as_str())?;
        if identity(&root, true)? != owner.identity
            || Some(identity(&sessions, true)?) != owner.sessions_root
            || identity(&directory, true)? != session.identity
        {
            return Err(Error::Corrupt);
        }
        Ok(directory)
    }
}

fn names_in_owner(root: impl AsFd, budget: &HistoryReadBudget<'_>) -> Result<Vec<String>> {
    names(root, 2, budget)
}

#[derive(Eq, PartialEq)]
pub(super) struct Owner {
    pub(super) name: String,
    identity: Identity,
    lock: Option<Identity>,
    sessions_root: Option<Identity>,
    pub(super) sessions: Vec<Session>,
}

#[derive(Eq, PartialEq)]
pub(super) struct Session {
    pub(super) id: TerminalSessionId,
    identity: Identity,
}

#[derive(Eq, PartialEq)]
struct Identity {
    device: i128,
    inode: u128,
}

fn identity(fd: impl AsFd, directory: bool) -> Result<Identity> {
    let stat = private(fd, directory)?;
    Ok(Identity {
        device: i128::from(stat.st_dev),
        inode: u128::from(stat.st_ino),
    })
}

fn read_flags() -> OFlags {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    #[cfg(target_os = "linux")]
    let flags = flags | OFlags::NOATIME;
    flags
}

fn open_directory(parent: impl AsFd, name: &str) -> Result<OwnedFd> {
    let directory = rustix::fs::openat(
        parent.as_fd(),
        name,
        read_flags() | OFlags::DIRECTORY,
        Mode::empty(),
    )
    .map_err(io_error)?;
    same_entry(parent, name, &directory, true)?;
    Ok(directory)
}

fn open_lock(parent: impl AsFd, name: &str) -> Result<OwnedFd> {
    let lock =
        rustix::fs::openat(parent.as_fd(), name, read_flags(), Mode::empty()).map_err(io_error)?;
    same_entry(parent, name, &lock, false)?;
    Ok(lock)
}

fn names(root: impl AsFd, maximum: usize, budget: &HistoryReadBudget<'_>) -> Result<Vec<String>> {
    let mut directory = Dir::new(open_directory(root, ".")?).map_err(io_error)?;
    let mut names = Vec::new();
    loop {
        budget.checkpoint()?;
        let Some(entry) = directory.next() else {
            break;
        };
        let entry = entry.map_err(io_error)?;
        let bytes = entry.file_name().to_bytes();
        if matches!(bytes, b"." | b"..") {
            continue;
        }
        if names.len() == maximum {
            return Err(Error::ResourceLimit);
        }
        names.push(String::from_utf8(bytes.to_vec()).map_err(|_| Error::Corrupt)?);
    }
    names.sort_unstable();
    Ok(names)
}

fn missing_is_corrupt(error: Error) -> Error {
    if error == Error::NotFound {
        Error::Corrupt
    } else {
        error
    }
}

fn io_error(error: rustix::io::Errno) -> Error {
    match error {
        rustix::io::Errno::NOENT => Error::NotFound,
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => Error::Corrupt,
        rustix::io::Errno::WOULDBLOCK => Error::Busy,
        _ => Error::Unavailable,
    }
}
