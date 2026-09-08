use super::invalid;
use machine_god_core::PermissionError;
use rustix::{
    fd::{AsFd, OwnedFd},
    fs::{AtFlags, FileType, Mode, OFlags},
};
use std::fs::File;

pub(super) fn validate_workspace(path: &str) -> Result<(), PermissionError> {
    if !path.starts_with('/')
        || path.len() > 4096
        || path.contains('\0')
        || (path != "/"
            && path[1..]
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == ".."))
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn absolute(workspace: &str, relative: &str) -> Result<String, PermissionError> {
    if relative.len() > 4096
        || relative.starts_with('/')
        || relative.contains('\0')
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(invalid());
    }
    Ok(format!("{}/{relative}", workspace.trim_end_matches('/')))
}

pub(super) fn validate_root(root: &File, spelling: &str) -> Result<(), PermissionError> {
    let retained = rustix::fs::fstat(root).map_err(|_| invalid())?;
    let named = rustix::fs::stat(spelling).map_err(|_| invalid())?;
    if !FileType::from_raw_mode(retained.st_mode).is_dir()
        || Identity::of(&retained) != Identity::of(&named)
    {
        return Err(invalid());
    }
    Ok(())
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Identity {
    device: i128,
    inode: u64,
    kind: FileType,
}
impl Identity {
    fn of(stat: &rustix::fs::Stat) -> Self {
        Self {
            device: i128::from(stat.st_dev),
            inode: stat.st_ino,
            kind: FileType::from_raw_mode(stat.st_mode),
        }
    }
}

pub(super) struct Observation {
    relative: String,
    create: bool,
    metadata_only: bool,
    entries: Vec<Option<Identity>>,
    selected: Option<OwnedFd>,
}
impl Observation {
    pub(super) fn is_regular(&self) -> bool {
        self.entries
            .last()
            .is_some_and(|entry| entry.is_some_and(|identity| identity.kind.is_file()))
    }
    pub(super) fn revalidate(&self, root: &File) -> Result<(), PermissionError> {
        let current = observe(root, &self.relative, self.create, self.metadata_only)?;
        if current.entries != self.entries {
            return Err(invalid());
        }
        if let Some(selected) = &self.selected {
            let current = rustix::fs::fstat(selected).map_err(|_| invalid())?;
            if self.entries.last().copied().flatten() != Some(Identity::of(&current)) {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

pub(super) fn observe(
    root: &File,
    relative: &str,
    create: bool,
    metadata_only: bool,
) -> Result<Observation, PermissionError> {
    if relative.is_empty()
        || relative.len() > 4096
        || relative.contains('\0')
        || relative.starts_with('/')
    {
        return Err(invalid());
    }
    let mut observation = Observation {
        relative: relative.to_owned(),
        create,
        metadata_only,
        entries: Vec::new(),
        selected: None,
    };
    let mut directory = rustix::io::dup(root).map_err(|_| invalid())?;
    if relative == "." {
        let stat = rustix::fs::fstat(&directory).map_err(|_| invalid())?;
        observation.entries.push(Some(Identity::of(&stat)));
        observation.selected = Some(directory);
        return Ok(observation);
    }
    let mut components = relative.split('/').peekable();
    let mut missing = false;
    while let Some(part) = components.next() {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.len() > 255
            || observation.entries.len() == 256
        {
            return Err(invalid());
        }
        if missing {
            observation.entries.push(None);
            continue;
        }
        let stat = match rustix::fs::statat(&directory, part, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) if create => {
                missing = true;
                observation.entries.push(None);
                continue;
            }
            Err(_) => return Err(invalid()),
        };
        observation.entries.push(Some(Identity::of(&stat)));
        let kind = FileType::from_raw_mode(stat.st_mode);
        let last = components.peek().is_none();
        if last {
            // Metadata-only final entries may be links/devices. Never follow or
            // open them. Content consumers retain their separate concrete checks.
            if !metadata_only && (kind.is_file() || kind.is_dir()) {
                let descriptor = open(&directory, part, kind.is_dir())?;
                if Identity::of(&rustix::fs::fstat(&descriptor).map_err(|_| invalid())?)
                    != Identity::of(&stat)
                {
                    return Err(invalid());
                }
                observation.selected = Some(descriptor);
            }
        } else {
            if !kind.is_dir() {
                return Err(invalid());
            }
            directory = open(&directory, part, true)?;
            if Identity::of(&rustix::fs::fstat(&directory).map_err(|_| invalid())?)
                != Identity::of(&stat)
            {
                return Err(invalid());
            }
        }
    }
    Ok(observation)
}

fn open(parent: impl AsFd, name: &str, directory: bool) -> Result<OwnedFd, PermissionError> {
    let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    if directory {
        flags |= OFlags::DIRECTORY;
    }
    rustix::fs::openat(parent, name, flags, Mode::empty()).map_err(|_| invalid())
}
