//! Bounded missing-parent observation and descriptor-relative first-use creation.

use super::{NativeUserConfigError as Error, READ, validate_link, validate_private};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, Mode, OFlags};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(super) struct ParentObservation {
    ancestor: OwnedFd,
    identity: PathBuf,
    missing: Vec<OsString>,
}

impl ParentObservation {
    pub(super) fn observe(path: &Path) -> Result<Self, Error> {
        let mut candidate = path;
        let mut missing = Vec::new();
        loop {
            match rustix::fs::open(candidate, READ | OFlags::DIRECTORY, Mode::empty()) {
                Ok(ancestor) => {
                    let identity =
                        std::fs::canonicalize(candidate).map_err(|_| Error::UnsafePath)?;
                    missing.reverse();
                    let observation = Self {
                        ancestor,
                        identity,
                        missing,
                    };
                    observation.validate_ancestor()?;
                    return Ok(observation);
                }
                Err(rustix::io::Errno::NOENT) => {
                    // A dangling symlink is not an absent namespace component.
                    match std::fs::symlink_metadata(candidate) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        _ => return Err(Error::UnsafePath),
                    }
                    missing.push(
                        candidate
                            .file_name()
                            .ok_or(Error::UnsafePath)?
                            .to_os_string(),
                    );
                    candidate = candidate.parent().ok_or(Error::UnsafePath)?;
                }
                Err(_) => return Err(Error::UnsafePath),
            }
        }
    }

    fn validate_ancestor(&self) -> Result<(), Error> {
        let linked = rustix::fs::statat(rustix::fs::CWD, &self.identity, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| Error::Conflict)?;
        let opened = rustix::fs::fstat(&self.ancestor).map_err(|_| Error::Persistence)?;
        if linked.st_dev != opened.st_dev || linked.st_ino != opened.st_ino {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    pub(super) fn resolve(&self, create: bool) -> Result<Option<ResolvedParent<'_>>, Error> {
        self.resolve_with_sync(create, |fd| rustix::fs::fsync(fd))
    }

    fn resolve_with_sync(
        &self,
        create: bool,
        mut sync: impl FnMut(&OwnedFd) -> rustix::io::Result<()>,
    ) -> Result<Option<ResolvedParent<'_>>, Error> {
        let mut resolved = ResolvedParent {
            observation: self,
            chain: Vec::with_capacity(self.missing.len()),
        };
        resolved.validate()?;
        for name in &self.missing {
            let parent = resolved.descriptor();
            let descriptor = match open(parent, name)? {
                Some(descriptor) => descriptor,
                None if !create => {
                    resolved.validate()?;
                    return Ok(None);
                }
                None => {
                    let created =
                        match rustix::fs::mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
                            Ok(()) => true,
                            Err(rustix::io::Errno::EXIST) => false,
                            Err(_) => return Err(Error::Persistence),
                        };
                    let descriptor = open(parent, name)?.ok_or(Error::Persistence)?;
                    if created {
                        validate_private(&descriptor, true)?;
                        sync(&descriptor).map_err(|_| Error::Persistence)?;
                        sync(parent).map_err(|_| Error::Persistence)?;
                    }
                    descriptor
                }
            };
            resolved.chain.push(descriptor);
            resolved.validate()?;
        }
        Ok(Some(resolved))
    }
}

fn open(parent: &OwnedFd, name: &std::ffi::OsStr) -> Result<Option<OwnedFd>, Error> {
    match rustix::fs::openat(parent, name, READ | OFlags::DIRECTORY, Mode::empty()) {
        Ok(descriptor) => Ok(Some(descriptor)),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(_) => Err(Error::UnsafePath),
    }
}

pub(super) struct ResolvedParent<'a> {
    observation: &'a ParentObservation,
    chain: Vec<OwnedFd>,
}

impl ResolvedParent<'_> {
    pub(super) fn descriptor(&self) -> &OwnedFd {
        self.chain.last().unwrap_or(&self.observation.ancestor)
    }

    pub(super) fn validate(&self) -> Result<(), Error> {
        self.observation.validate_ancestor()?;
        let mut parent = &self.observation.ancestor;
        for (name, opened) in self.observation.missing.iter().zip(&self.chain) {
            validate_link(parent, name, opened)?;
            parent = opened;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
