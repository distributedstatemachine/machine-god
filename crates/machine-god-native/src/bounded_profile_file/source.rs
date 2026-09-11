//! Raw source observations. MCP retains its exact opened file incarnation.
use super::{
    File, FileType, Mode, OwnedFd, ProfileFileError as Error, ProfileFileKind, READ, read_bounded,
    validate_link, validate_private,
};

#[derive(Default)]
pub(super) struct ObservedFile {
    bytes: Option<Vec<u8>>,
    source: Option<Source>,
}

struct Source {
    file: File,
    revision: Revision,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Revision([i128; 11]);

impl Revision {
    fn read(file: &impl rustix::fd::AsFd) -> Result<Self, Error> {
        let stat = rustix::fs::fstat(file).map_err(|_| Error::Persistence)?;
        Ok(Self([
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
        ]))
    }
}

impl ObservedFile {
    pub(super) fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }

    pub(super) fn compare(&self, other: &Self, kind: ProfileFileKind) -> Result<(), Error> {
        if self.bytes != other.bytes {
            return Err(Error::Conflict);
        }
        if kind == ProfileFileKind::Mcp {
            match (&self.source, &other.source) {
                (None, None) => {}
                (Some(expected), Some(actual)) if expected.revision == actual.revision => {
                    // Keep the original descriptor alive and reject in-place source
                    // changes as well as equal-byte replacement by another inode.
                    validate_private(&expected.file, false)?;
                    if Revision::read(&expected.file)? != expected.revision {
                        return Err(Error::Conflict);
                    }
                }
                _ => return Err(Error::Conflict),
            }
        }
        Ok(())
    }
}

pub(super) fn read_current(root: &OwnedFd, kind: ProfileFileKind) -> Result<ObservedFile, Error> {
    read_with_hook(root, kind, |_| {})
}

fn read_with_hook(
    root: &OwnedFd,
    kind: ProfileFileKind,
    after_read: impl FnOnce(&File),
) -> Result<ObservedFile, Error> {
    let fd = match rustix::fs::openat(root, kind.data(), READ, Mode::empty()) {
        Ok(fd) => fd,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(ObservedFile::default()),
        Err(_) => return Err(Error::UnsafePath),
    };
    if kind == ProfileFileKind::Mcp {
        validate_private(&fd, false)?;
        validate_link(root, kind.data(), &fd)?;
    } else {
        // Preserve legacy settings' readable-file compatibility exactly.
        let stat = rustix::fs::fstat(&fd).map_err(|_| Error::Persistence)?;
        if !FileType::from_raw_mode(stat.st_mode).is_file()
            || stat.st_nlink != 1
            || stat.st_uid != nix::unistd::Uid::effective().as_raw()
            || stat.st_mode & 0o022 != 0
        {
            return Err(Error::UnsafePath);
        }
    }
    let mut file = File::from(fd);
    let revision = if kind == ProfileFileKind::Mcp {
        Some(Revision::read(&file)?)
    } else {
        None
    };
    let bytes = read_bounded(&mut file, kind.limit())?;
    after_read(&file);
    let source = if let Some(revision) = revision {
        validate_link(root, kind.data(), &file)?;
        validate_private(&file, false)?;
        if Revision::read(&file)? != revision {
            return Err(Error::Conflict);
        }
        Some(Source { file, revision })
    } else {
        None
    };
    Ok(ObservedFile {
        bytes: Some(bytes),
        source,
    })
}

#[cfg(test)]
mod tests;
