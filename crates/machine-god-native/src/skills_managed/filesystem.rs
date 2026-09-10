use super::{
    MAX_MANAGED_SKILL_ENTRIES, MAX_MANAGED_SKILL_FILE_BYTES, MAX_MANAGED_SKILL_OPERATIONS,
    MAX_MANAGED_SKILL_TOTAL_BYTES, NativeSkillManagedErrorKind as Error,
};
use machine_god_core::CancellationToken;
use rustix::fd::AsFd;
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Identity(pub i128, pub i128);

pub(super) fn identity(file: &File) -> Result<Identity, Error> {
    let stat = rustix::fs::fstat(file).map_err(|_| Error::Unavailable)?;
    Ok(Identity(i128::from(stat.st_dev), i128::from(stat.st_ino)))
}
pub(super) fn named_identity(parent: &File, name: &str) -> Result<Option<Identity>, Error> {
    match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(Identity(
            i128::from(stat.st_dev),
            i128::from(stat.st_ino),
        ))),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(None),
        Err(_) => Err(Error::Unavailable),
    }
}
pub(super) fn verify_named(parent: &File, name: &str, file: &File) -> Result<(), Error> {
    if named_identity(parent, name)? == Some(identity(file)?) {
        Ok(())
    } else {
        Err(Error::Changed)
    }
}

pub(super) const fn directory_flags() -> OFlags {
    OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::NONBLOCK)
        .union(OFlags::CLOEXEC)
}
pub(super) fn open_absolute_directory(path: &Path) -> Result<File, Error> {
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err(Error::InvalidSource);
    }
    // Only the explicitly selected absolute anchor uses host ancestor resolution.
    let fd =
        rustix::fs::open(path, directory_flags(), Mode::empty()).map_err(|_| Error::Unavailable)?;
    Ok(File::from(fd))
}
pub(super) fn open_directory(parent: &File, name: &str) -> Result<File, Error> {
    rustix::fs::openat(parent, name, directory_flags(), Mode::empty())
        .map(File::from)
        .map_err(|_| Error::Unavailable)
}
pub(super) fn open_path(parent: &File, path: &Path) -> Result<File, Error> {
    let mut current = parent.try_clone().map_err(|_| Error::Unavailable)?;
    if path.components().count() > 32 {
        return Err(Error::ResourceLimit);
    }
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                current = File::from(
                    rustix::fs::openat(&current, name, directory_flags(), Mode::empty())
                        .map_err(|_| Error::Unavailable)?,
                );
            }
            Component::CurDir => {}
            _ => return Err(Error::InvalidSource),
        }
    }
    Ok(current)
}
pub(super) fn create_directory(parent: &File, name: &str) -> Result<File, Error> {
    rustix::fs::mkdirat(parent, name, Mode::from_raw_mode(0o700))
        .map_err(|_| Error::Unavailable)?;
    match open_directory(parent, name) {
        Ok(file) => Ok(file),
        Err(error) => {
            // Before retaining identity, only an empty-directory removal is safe.
            let _ = rustix::fs::unlinkat(parent, name, AtFlags::REMOVEDIR);
            Err(error)
        }
    }
}

pub(super) struct Budget<'a> {
    pub cancellation: &'a CancellationToken,
    operations: usize,
    entries: usize,
    bytes: usize,
    names: usize,
    paths: usize,
    cleanup: bool,
}
impl<'a> Budget<'a> {
    pub const fn new(cancellation: &'a CancellationToken) -> Self {
        Self {
            cancellation,
            operations: 0,
            entries: 0,
            bytes: 0,
            names: 0,
            paths: 0,
            cleanup: false,
        }
    }
    pub fn charge(&mut self) -> Result<(), Error> {
        if self.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.operations += 1;
        let limit = if self.cleanup {
            4 * MAX_MANAGED_SKILL_OPERATIONS
        } else {
            MAX_MANAGED_SKILL_OPERATIONS
        };
        if self.operations > limit {
            return Err(Error::ResourceLimit);
        }
        Ok(())
    }
    fn entry(&mut self, name: &str) -> Result<(), Error> {
        self.entries += 1;
        self.names += name.len();
        let entry_limit = if self.cleanup {
            2 * MAX_MANAGED_SKILL_ENTRIES + 2
        } else {
            MAX_MANAGED_SKILL_ENTRIES
        };
        let name_limit = if self.cleanup {
            2 * 1024 * 1024
        } else {
            1024 * 1024
        };
        if self.entries > entry_limit || self.names > name_limit || name.len() > 4096 {
            return Err(Error::ResourceLimit);
        }
        self.charge()
    }
    fn bytes(&mut self, count: usize) -> Result<(), Error> {
        self.bytes = self.bytes.checked_add(count).ok_or(Error::ResourceLimit)?;
        if self.bytes > MAX_MANAGED_SKILL_TOTAL_BYTES {
            return Err(Error::ResourceLimit);
        }
        Ok(())
    }
    fn path(&mut self, path: &str) -> Result<(), Error> {
        self.paths = self
            .paths
            .checked_add(path.len())
            .ok_or(Error::ResourceLimit)?;
        if self.paths > 1024 * 1024 {
            return Err(Error::ResourceLimit);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Entry {
    pub path: String,
    pub identity: Identity,
    pub bytes: Option<std::sync::Arc<[u8]>>,
}
#[derive(Clone)]
pub(super) struct Tree {
    pub root: Identity,
    pub entries: Vec<Entry>,
}
impl Tree {
    pub fn empty(root: Identity) -> Self {
        Self {
            root,
            entries: Vec::new(),
        }
    }
    pub fn bytes(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|entry| entry.bytes.as_ref())
            .map(|bytes| bytes.len())
            .sum()
    }
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(self.root.0.to_le_bytes());
        hash.update(self.root.1.to_le_bytes());
        for entry in &self.entries {
            hash.update((entry.path.len() as u64).to_le_bytes());
            hash.update(entry.path.as_bytes());
            hash.update(entry.identity.0.to_le_bytes());
            hash.update(entry.identity.1.to_le_bytes());
            hash.update([u8::from(entry.bytes.is_some())]);
            if let Some(bytes) = &entry.bytes {
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
        }
        hash.finalize().into()
    }
    pub fn same_content(&self, other: &Self) -> bool {
        self.entries.len() == other.entries.len()
            && self
                .entries
                .iter()
                .zip(&other.entries)
                .all(|(a, b)| a.path == b.path && a.bytes == b.bytes)
    }
    pub fn subset(&self, prefix: &str) -> Result<Self, Error> {
        if prefix.is_empty() {
            return Ok(self.clone());
        }
        let root = self
            .entries
            .iter()
            .find(|entry| entry.path == prefix && entry.bytes.is_none())
            .ok_or(Error::Changed)?
            .identity;
        let prefix = format!("{prefix}/");
        Ok(Self {
            root,
            entries: self
                .entries
                .iter()
                .filter_map(|entry| {
                    entry.path.strip_prefix(&prefix).map(|path| Entry {
                        path: path.to_owned(),
                        identity: entry.identity,
                        bytes: entry.bytes.clone(),
                    })
                })
                .collect(),
        })
    }
}

pub(super) fn read_tree(
    root: &File,
    skip_git: bool,
    budget: &mut Budget<'_>,
) -> Result<Tree, Error> {
    budget.charge()?;
    let mut tree = Tree::empty(identity(root)?);
    read_directory(root, "", 0, skip_git, &mut tree.entries, budget)?;
    tree.entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(tree)
}
fn names(directory: &File, budget: &mut Budget<'_>) -> Result<Vec<String>, Error> {
    budget.charge()?;
    let stream = Dir::read_from(directory.as_fd()).map_err(|_| Error::Unavailable)?;
    let mut names = Vec::new();
    for entry in stream {
        budget.charge()?;
        let entry = entry.map_err(|_| Error::Unavailable)?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        let name = std::str::from_utf8(raw).map_err(|_| Error::InvalidEntry)?;
        if name.is_empty()
            || name.len() > 255
            || name
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\'))
        {
            return Err(Error::InvalidEntry);
        }
        budget.entry(name)?;
        names.push(name.to_owned());
    }
    names.sort();
    Ok(names)
}
fn read_directory(
    directory: &File,
    prefix: &str,
    depth: usize,
    skip_git: bool,
    output: &mut Vec<Entry>,
    budget: &mut Budget<'_>,
) -> Result<(), Error> {
    if depth > 32 {
        return Err(Error::ResourceLimit);
    }
    budget.charge()?;
    let before = rustix::fs::fstat(directory).map_err(|_| Error::Unavailable)?;
    for name in names(directory, budget)? {
        if skip_git && name.starts_with(".git") {
            continue;
        }
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if path.len() > 4096 {
            return Err(Error::ResourceLimit);
        }
        budget.path(&path)?;
        budget.charge()?;
        let fd = rustix::fs::openat(
            directory,
            &name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Error::InvalidEntry)?;
        budget.charge()?;
        let before = rustix::fs::fstat(&fd).map_err(|_| Error::Unavailable)?;
        let file = File::from(fd);
        let entry_identity = Identity(i128::from(before.st_dev), i128::from(before.st_ino));
        match FileType::from_raw_mode(before.st_mode) {
            FileType::Directory => {
                output.push(Entry {
                    path: path.clone(),
                    identity: entry_identity,
                    bytes: None,
                });
                read_directory(&file, &path, depth + 1, skip_git, output, budget)?;
            }
            FileType::RegularFile => {
                let bytes = read_file(&file, &before, budget)?;
                output.push(Entry {
                    path,
                    identity: entry_identity,
                    bytes: Some(bytes.into()),
                });
            }
            _ => return Err(Error::InvalidEntry),
        }
        budget.charge()?;
        verify_named(directory, &name, &file)?;
    }
    budget.charge()?;
    let after = rustix::fs::fstat(directory).map_err(|_| Error::Unavailable)?;
    if before.st_mtime != after.st_mtime
        || before.st_mtime_nsec != after.st_mtime_nsec
        || before.st_ctime != after.st_ctime
        || before.st_ctime_nsec != after.st_ctime_nsec
    {
        return Err(Error::Changed);
    }
    Ok(())
}
fn read_file(
    mut file: &File,
    before: &rustix::fs::Stat,
    budget: &mut Budget<'_>,
) -> Result<Vec<u8>, Error> {
    let length = usize::try_from(before.st_size).map_err(|_| Error::ResourceLimit)?;
    if length > MAX_MANAGED_SKILL_FILE_BYTES {
        return Err(Error::ResourceLimit);
    }
    budget.bytes(length)?;
    let mut bytes = Vec::with_capacity(length);
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        budget.charge()?;
        let count = file.read(&mut buffer).map_err(|_| Error::Unavailable)?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > length {
            return Err(Error::Changed);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    budget.charge()?;
    let after = rustix::fs::fstat(file).map_err(|_| Error::Unavailable)?;
    if bytes.len() != length
        || before.st_size != after.st_size
        || before.st_mtime != after.st_mtime
        || before.st_mtime_nsec != after.st_mtime_nsec
        || before.st_ctime != after.st_ctime
        || before.st_ctime_nsec != after.st_ctime_nsec
    {
        return Err(Error::Changed);
    }
    Ok(bytes)
}

pub(super) fn write_tree(root: &File, tree: &Tree, budget: &mut Budget<'_>) -> Result<(), Error> {
    for entry in &tree.entries {
        let (parent, name) = entry.path.rsplit_once('/').unwrap_or(("", &entry.path));
        budget.charge()?;
        let directory = open_path(root, Path::new(parent))?;
        budget.charge()?;
        if let Some(bytes) = &entry.bytes {
            let fd = rustix::fs::openat(
                &directory,
                name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| Error::Unavailable)?;
            let mut file = File::from(fd);
            for chunk in bytes.chunks(64 * 1024) {
                let mut pending = chunk;
                while !pending.is_empty() {
                    budget.charge()?;
                    let written = file.write(pending).map_err(|_| Error::Unavailable)?;
                    if written == 0 {
                        return Err(Error::Unavailable);
                    }
                    pending = &pending[written..];
                }
            }
            budget.charge()?;
            file.sync_all().map_err(|_| Error::Unavailable)?;
        } else {
            create_directory(&directory, name)?;
        }
    }
    for entry in tree
        .entries
        .iter()
        .rev()
        .filter(|entry| entry.bytes.is_none())
    {
        budget.charge()?;
        open_path(root, Path::new(&entry.path))?
            .sync_all()
            .map_err(|_| Error::Unavailable)?;
    }
    budget.charge()?;
    root.sync_all().map_err(|_| Error::Unavailable)
}

pub(super) fn cleanup(parent: &File, name: &str, retained: &File) -> Result<(), Error> {
    verify_named(parent, name, retained)?;
    let cancellation = CancellationToken::new();
    let mut budget = Budget::new(&cancellation);
    budget.cleanup = true;
    remove_children(retained, 0, &mut budget)?;
    verify_named(parent, name, retained)?;
    rustix::fs::unlinkat(parent, name, AtFlags::REMOVEDIR).map_err(|_| Error::Unavailable)
}
fn remove_children(directory: &File, depth: usize, budget: &mut Budget<'_>) -> Result<(), Error> {
    if depth > 34 {
        return Err(Error::ResourceLimit);
    }
    for name in names(directory, budget)? {
        budget.charge()?;
        let stat = rustix::fs::statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| Error::Unavailable)?;
        if FileType::from_raw_mode(stat.st_mode).is_dir() {
            let child = open_directory(directory, &name)?;
            if identity(&child)? != Identity(i128::from(stat.st_dev), i128::from(stat.st_ino)) {
                return Err(Error::Changed);
            }
            remove_children(&child, depth + 1, budget)?;
            verify_named(directory, &name, &child)?;
            budget.charge()?;
            rustix::fs::unlinkat(directory, &name, AtFlags::REMOVEDIR)
                .map_err(|_| Error::Unavailable)?;
        } else {
            budget.charge()?;
            rustix::fs::unlinkat(directory, &name, AtFlags::empty())
                .map_err(|_| Error::Unavailable)?;
        }
    }
    Ok(())
}

pub(super) fn random_name() -> Result<String, Error> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| Error::Unavailable)?;
    let mut name = String::from(".machine-god-skill-");
    for byte in bytes {
        write!(&mut name, "{byte:02x}").map_err(|_| Error::Unavailable)?;
    }
    Ok(name)
}
