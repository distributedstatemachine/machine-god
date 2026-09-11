//! Candidate discovery is independent of selected-tree copy exclusions.

use super::{Budget, Error, Identity, directory_flags, names, read_file, verify_named};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat};
use sha2::{Digest, Sha256};
use std::fs::File;

pub(in crate::skills_managed) struct SkillObservation<'a> {
    pub path: &'a str,
    pub directory: Identity,
    pub directory_mode: Mode,
    pub file: Identity,
    pub file_mode: Mode,
    pub bytes: &'a [u8],
}

pub(in crate::skills_managed) fn read_inventory(
    root: &File,
    budget: &mut Budget<'_>,
    visit: impl FnMut(SkillObservation<'_>) -> Result<(), Error>,
) -> Result<[u8; 32], Error> {
    budget.charge()?;
    let initial = rustix::fs::fstat(root).map_err(|_| Error::Unavailable)?;
    let mut inventory = Inventory {
        budget,
        hash: Sha256::new(),
        visit,
    };
    inventory.record("", &initial);
    inventory.directory(root, "", 0, &initial)?;
    Ok(inventory.hash.finalize().into())
}

struct Inventory<'a, 'b, F> {
    budget: &'a mut Budget<'b>,
    hash: Sha256,
    visit: F,
}
impl<F: FnMut(SkillObservation<'_>) -> Result<(), Error>> Inventory<'_, '_, F> {
    fn record(&mut self, path: &str, stat: &Stat) {
        self.hash.update((path.len() as u64).to_le_bytes());
        self.hash.update(path.as_bytes());
        self.hash.update(revision(stat));
    }

    fn directory(
        &mut self,
        directory: &File,
        prefix: &str,
        depth: usize,
        initial: &Stat,
    ) -> Result<(), Error> {
        if depth > 32 {
            return Err(Error::ResourceLimit);
        }
        for name in names(directory, self.budget)? {
            let length = prefix.len() + usize::from(!prefix.is_empty()) + name.len();
            if length > 4096 {
                return Err(Error::ResourceLimit);
            }
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            self.budget.path(&path)?;
            self.budget.charge()?;
            let before = rustix::fs::statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| Error::Unavailable)?;
            self.record(&path, &before);
            match FileType::from_raw_mode(before.st_mode) {
                FileType::Directory => {
                    let child = self.open(directory, &name, directory_flags(), &before)?;
                    self.directory(&child, &path, depth + 1, &before)?;
                    self.budget.charge()?;
                    verify_named(directory, &name, &child)?;
                }
                FileType::RegularFile if name == "SKILL.md" => {
                    let child = self.open(
                        directory,
                        &name,
                        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                        &before,
                    )?;
                    let bytes = read_file(&child, &before, self.budget)?;
                    self.hash.update((bytes.len() as u64).to_le_bytes());
                    self.hash.update(&bytes);
                    (self.visit)(SkillObservation {
                        path: &path,
                        directory: identity(initial),
                        directory_mode: Mode::from_raw_mode(initial.st_mode),
                        file: identity(&before),
                        file_mode: Mode::from_raw_mode(before.st_mode),
                        bytes: &bytes,
                    })?;
                    self.budget.charge()?;
                    verify_named(directory, &name, &child)?;
                }
                _ if name == "SKILL.md" => return Err(Error::InvalidEntry),
                // Record unselected resources without opening or following them.
                _ => {}
            }
        }
        self.budget.charge()?;
        let after = rustix::fs::fstat(directory).map_err(|_| Error::Unavailable)?;
        if revision(initial) != revision(&after) {
            return Err(Error::Changed);
        }
        Ok(())
    }

    fn open(
        &mut self,
        directory: &File,
        name: &str,
        flags: OFlags,
        expected: &Stat,
    ) -> Result<File, Error> {
        self.budget.charge()?;
        let file = File::from(
            rustix::fs::openat(directory, name, flags, Mode::empty())
                .map_err(|_| Error::InvalidEntry)?,
        );
        self.budget.charge()?;
        let observed = rustix::fs::fstat(&file).map_err(|_| Error::Unavailable)?;
        if revision(expected) != revision(&observed) {
            return Err(Error::Changed);
        }
        Ok(file)
    }
}

fn identity(stat: &Stat) -> Identity {
    Identity(i128::from(stat.st_dev), i128::from(stat.st_ino))
}

fn revision(stat: &Stat) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(i128::from(stat.st_dev).to_le_bytes());
    hash.update(i128::from(stat.st_ino).to_le_bytes());
    hash.update(u64::from(stat.st_mode).to_le_bytes());
    hash.update(stat.st_size.to_le_bytes());
    hash.update(stat.st_mtime.to_le_bytes());
    hash.update(stat.st_mtime_nsec.to_le_bytes());
    hash.update(stat.st_ctime.to_le_bytes());
    hash.update(stat.st_ctime_nsec.to_le_bytes());
    hash.finalize().into()
}
