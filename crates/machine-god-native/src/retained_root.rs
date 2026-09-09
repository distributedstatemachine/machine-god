//! macOS retained-root identity observation, not a new source of authority.
//!
//! Callers keep the retained descriptor, fstat/getpath observations, cancellation
//! checkpoints, syscall-error ordering, and phase-specific error mapping. This
//! staged value only shares basename selection and the parent-entry comparison.
//! It neither rediscovers a workspace nor makes the observations atomic.

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat};
use std::ffi::{CStr, CString};

pub(crate) struct RetainedRootObservation<'fd> {
    root: BorrowedFd<'fd>,
    device: i128,
    inode: i128,
    name: CString,
}

impl<'fd> RetainedRootObservation<'fd> {
    /// None preserves the filesystem-root early return, before parent I/O.
    pub(crate) fn new(
        root: BorrowedFd<'fd>,
        metadata: &Stat,
        path: &CStr,
    ) -> Result<Option<Self>, ()> {
        let path = path.to_bytes();
        if path == b"/" {
            return Ok(None);
        }
        let name = path
            .rsplit(|byte| *byte == b'/')
            .next()
            .filter(|name| !name.is_empty())
            .ok_or(())?;
        let name = CString::new(name).map_err(|_| ())?;
        Ok(Some(Self {
            root,
            device: i128::from(metadata.st_dev),
            inode: i128::from(metadata.st_ino),
            name,
        }))
    }

    pub(crate) fn open_parent(&self) -> Result<OwnedFd, rustix::io::Errno> {
        rustix::fs::openat(
            self.root,
            "..",
            OFlags::RDONLY
                | OFlags::DIRECTORY
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::empty(),
        )
    }

    pub(crate) fn stat_link(&self, parent: impl AsFd) -> Result<Stat, rustix::io::Errno> {
        rustix::fs::statat(parent, &self.name, AtFlags::SYMLINK_NOFOLLOW)
    }

    pub(crate) fn matches(&self, linked: &Stat) -> bool {
        i128::from(linked.st_dev) == self.device
            && i128::from(linked.st_ino) == self.inode
            && FileType::from_raw_mode(linked.st_mode).is_dir()
    }
}

#[cfg(test)]
pub(crate) mod tests;
