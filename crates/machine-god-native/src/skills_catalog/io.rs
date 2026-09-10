use std::{
    collections::VecDeque,
    path::{Component, Path, PathBuf},
};

use machine_god_core::CancellationToken;
use rustix::{
    fd::{AsFd, BorrowedFd, OwnedFd},
    fs::{AtFlags, FileType, Mode, OFlags, Stat},
    io::Errno,
};

use super::{
    MAX_NATIVE_SKILL_DIRECTORY_BYTES, MAX_NATIVE_SKILL_DISCOVERY_BYTES,
    MAX_NATIVE_SKILL_IO_ATTEMPTS, MAX_NATIVE_SKILL_LINK_HOPS, MAX_NATIVE_SKILL_PATH_BYTES,
    MAX_NATIVE_SKILL_PATH_COMPONENTS, MAX_NATIVE_SKILL_VISITED_ENTRIES,
    NativeSkillCatalogError as Error, NativeSkillLinkPolicy, NativeSkillRoot, Result, check,
};
use crate::skills_metadata::{MAX_NATIVE_SKILL_HEADER_BYTES, closing_delimiter, header_start};

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
const FILE_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

#[derive(Default)]
pub(super) struct Budget {
    attempts: usize,
    bytes: usize,
    visited: usize,
}

impl Budget {
    pub(super) fn exhausted(&self) -> bool {
        self.attempts >= MAX_NATIVE_SKILL_IO_ATTEMPTS
            || self.bytes >= MAX_NATIVE_SKILL_DISCOVERY_BYTES
            || self.visited >= MAX_NATIVE_SKILL_VISITED_ENTRIES
    }

    pub(super) fn charge(&mut self, cancellation: &CancellationToken) -> Result<()> {
        check(cancellation)?;
        if self.attempts == MAX_NATIVE_SKILL_IO_ATTEMPTS {
            return Err(Error::ResourceLimit);
        }
        self.attempts += 1;
        Ok(())
    }

    pub(super) fn call<T>(
        &mut self,
        cancellation: &CancellationToken,
        operation: impl FnOnce() -> std::result::Result<T, Errno>,
    ) -> Result<T> {
        self.charge(cancellation)?;
        let result = operation();
        check(cancellation)?;
        result.map_err(map_error)
    }
}

pub(super) fn map_error(error: Errno) -> Error {
    match error {
        Errno::NOENT => Error::NotFound,
        Errno::LOOP | Errno::NOTDIR => Error::PathRejected,
        _ => Error::Unavailable,
    }
}

pub(super) fn stat(
    fd: impl AsFd,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<Stat> {
    budget.call(cancellation, || rustix::fs::fstat(fd))
}

pub(super) fn identity(stat: &Stat) -> [i128; 2] {
    [i128::from(stat.st_dev), i128::from(stat.st_ino)]
}

pub(super) fn revision(stat: &Stat) -> [i128; 8] {
    [
        i128::from(stat.st_dev),
        i128::from(stat.st_ino),
        i128::from(stat.st_mode),
        i128::from(stat.st_size),
        i128::from(stat.st_mtime),
        i128::from(stat.st_mtime_nsec),
        i128::from(stat.st_ctime),
        i128::from(stat.st_ctime_nsec),
    ]
}

/// Directory-only resolver. Every native lookup is no-follow; symlinks are
/// expanded as bounded lexical components beneath the retained descriptor.
pub(super) fn open_directory(
    root: &NativeSkillRoot,
    relative: &Path,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<OwnedFd> {
    let base = budget.call(cancellation, || {
        rustix::fs::openat(&*root.directory, ".", DIRECTORY_FLAGS, Mode::empty())
    })?;
    let mut stack = vec![base];
    let mut lengths = vec![0_usize];
    let mut pending = components(relative)?;
    let mut hops = 0;
    while let Some(component) = pending.pop_front() {
        check(cancellation)?;
        if component == ".." {
            if stack.len() == 1 {
                return Err(Error::PathRejected);
            }
            stack.pop();
            lengths.pop();
            continue;
        }
        if component == "." {
            continue;
        }
        if stack.len() + pending.len() > MAX_NATIVE_SKILL_PATH_COMPONENTS + 1 {
            return Err(Error::ResourceLimit);
        }
        let directory = stack.last().ok_or(Error::PathRejected)?;
        let observed = budget
            .call(cancellation, || {
                rustix::fs::statat(directory, component.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            })
            .map_err(|error| missing_link_target(error, hops))?;
        if FileType::from_raw_mode(observed.st_mode).is_symlink() {
            if root.links == NativeSkillLinkPolicy::Reject {
                return Err(Error::PathRejected);
            }
            hops += 1;
            if hops > MAX_NATIVE_SKILL_LINK_HOPS {
                return Err(Error::ResourceLimit);
            }
            let target = read_link(directory.as_fd(), &component, budget, cancellation)?;
            let target = if target.is_absolute() {
                stack.truncate(1);
                lengths.truncate(1);
                target
                    .strip_prefix(&root.authority_path)
                    .map_err(|_| Error::PathRejected)?
                    .to_path_buf()
            } else {
                target
            };
            let mut next = components(&target)?;
            next.append(&mut pending);
            if next.iter().map(|part| part.len() + 1).sum::<usize>() + lengths.iter().sum::<usize>()
                > MAX_NATIVE_SKILL_PATH_BYTES
            {
                return Err(Error::ResourceLimit);
            }
            pending = next;
        } else {
            let opened = budget
                .call(cancellation, || {
                    rustix::fs::openat(
                        directory,
                        component.as_str(),
                        DIRECTORY_FLAGS,
                        Mode::empty(),
                    )
                })
                .map_err(|error| missing_link_target(error, hops))?;
            stack.push(opened);
            lengths.push(component.len() + 1);
        }
    }
    check(cancellation)?;
    stack.pop().ok_or(Error::PathRejected)
}

fn missing_link_target(error: Error, hops: usize) -> Error {
    if error == Error::NotFound && hops != 0 {
        Error::PathRejected
    } else {
        error
    }
}

fn components(path: &Path) -> Result<VecDeque<String>> {
    if path.as_os_str().len() > MAX_NATIVE_SKILL_PATH_BYTES
        || path.components().count() > MAX_NATIVE_SKILL_PATH_COMPONENTS
    {
        return Err(Error::ResourceLimit);
    }
    path.components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .filter(|text| !text.contains(['\0', '\\']))
                .map(str::to_owned)
                .ok_or(Error::PathRejected),
            Component::CurDir => Ok(".".to_owned()),
            Component::ParentDir => Ok("..".to_owned()),
            _ => Err(Error::PathRejected),
        })
        .collect()
}

fn read_link(
    directory: BorrowedFd<'_>,
    name: &str,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<PathBuf> {
    let mut buffer = [0_u8; MAX_NATIVE_SKILL_PATH_BYTES + 1];
    let count = budget.call(cancellation, || {
        rustix::fs::readlinkat_raw(directory, name, &mut buffer[..])
    })?;
    if count == 0 || count > MAX_NATIVE_SKILL_PATH_BYTES {
        return Err(Error::ResourceLimit);
    }
    let text = std::str::from_utf8(&buffer[..count]).map_err(|_| Error::PathRejected)?;
    Ok(PathBuf::from(text))
}

pub(super) fn open_file(
    directory: BorrowedFd<'_>,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<(OwnedFd, Stat)> {
    let file = budget.call(cancellation, || {
        rustix::fs::openat(directory, "SKILL.md", FILE_FLAGS, Mode::empty())
    })?;
    let observed = stat(&file, budget, cancellation)?;
    if !FileType::from_raw_mode(observed.st_mode).is_file() {
        return Err(Error::PathRejected);
    }
    Ok((file, observed))
}

/// Prefix discovery does not reject an otherwise valid large body. Reads stop
/// once metadata is known, including at most one fixed 16 KiB read-ahead chunk.
pub(super) fn read_prefix(
    file: &OwnedFd,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    read_bytes(
        file,
        MAX_NATIVE_SKILL_HEADER_BYTES,
        true,
        budget,
        cancellation,
    )
}

pub(super) fn read_bytes(
    file: &OwnedFd,
    limit: usize,
    prefix: bool,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 16 * 1_024];
    loop {
        let remaining = (limit + 1)
            .checked_sub(bytes.len())
            .ok_or(Error::ResourceLimit)?;
        let aggregate = MAX_NATIVE_SKILL_DISCOVERY_BYTES
            .checked_sub(budget.bytes)
            .ok_or(Error::ResourceLimit)?;
        let wanted = chunk.len().min(remaining).min(aggregate);
        if wanted == 0 {
            return Err(Error::ResourceLimit);
        }
        budget.charge(cancellation)?;
        let result = rustix::io::read(file, &mut chunk[..wanted]);
        check(cancellation)?;
        let count = match result {
            Ok(count) => count,
            Err(Errno::INTR) => continue,
            Err(error) => return Err(map_error(error)),
        };
        budget.bytes += count;
        bytes.extend_from_slice(&chunk[..count]);
        if prefix && metadata_complete(&bytes, count == 0) {
            break;
        }
        if bytes.len() > limit {
            return Err(Error::ResourceLimit);
        }
        if count == 0 {
            break;
        }
    }
    Ok(bytes)
}

fn metadata_complete(bytes: &[u8], eof: bool) -> bool {
    if bytes.len() < 5 && !eof {
        return false;
    }
    header_start(bytes).is_none_or(|start| closing_delimiter(bytes, start).is_some())
}

pub(super) struct Names {
    pub names: Vec<String>,
    pub invalid: bool,
}

pub(super) fn read_names(
    directory: BorrowedFd<'_>,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<Names> {
    #[cfg(target_os = "linux")]
    let mut buffer = [std::mem::MaybeUninit::uninit(); 16 * 1_024];
    #[cfg(target_os = "linux")]
    let mut stream = rustix::fs::RawDir::new(directory, &mut buffer);
    #[cfg(target_os = "macos")]
    let mut stream = crate::macos_directory::MacosDirectoryReader::new(directory);
    let mut names = Names {
        names: Vec::new(),
        invalid: false,
    };
    let mut retained_bytes = 0_usize;
    loop {
        check(cancellation)?;
        if budget.visited == MAX_NATIVE_SKILL_VISITED_ENTRIES {
            return Err(Error::ResourceLimit);
        }
        if stream.is_buffer_empty() {
            budget.charge(cancellation)?;
        }
        #[cfg(target_os = "linux")]
        let next = stream
            .next()
            .map(|entry| entry.map(|entry| entry.file_name().to_bytes().to_vec()));
        #[cfg(target_os = "macos")]
        let next = stream.next_name().map(|entry| {
            entry.map(|entry| match entry {
                crate::macos_directory::MacosDirectoryEntry::Name(name) => name,
                crate::macos_directory::MacosDirectoryEntry::Skipped => b".".to_vec(),
            })
        });
        check(cancellation)?;
        let raw = match next {
            None => break,
            Some(Err(Errno::INTR)) => continue,
            Some(Err(error)) => return Err(map_error(error)),
            Some(Ok(raw)) => raw,
        };
        budget.visited += 1;
        if raw == b"." || raw == b".." {
            continue;
        }
        match String::from_utf8(raw) {
            Ok(name)
                if name.len() <= MAX_NATIVE_SKILL_PATH_BYTES
                    && !name.contains(['\0', '/', '\\']) =>
            {
                if retained_bytes.saturating_add(name.len()) > MAX_NATIVE_SKILL_DIRECTORY_BYTES {
                    return Err(Error::ResourceLimit);
                }
                retained_bytes += name.len();
                names.names.push(name);
            }
            _ => names.invalid = true,
        }
    }
    names.names.sort_unstable();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_checks_before_dispatch_and_after_native_return() {
        let cancellation = CancellationToken::new();
        let mut budget = Budget::default();
        for _ in 0..MAX_NATIVE_SKILL_IO_ATTEMPTS {
            budget.charge(&cancellation).unwrap();
        }
        assert_eq!(
            budget.call::<()>(&cancellation, || panic!("exhausted budget dispatched")),
            Err(Error::ResourceLimit)
        );
        cancellation.cancel();
        assert_eq!(
            budget.call::<()>(&cancellation, || panic!("cancelled budget dispatched")),
            Err(Error::Cancelled)
        );
        let cancellation = CancellationToken::new();
        let mut budget = Budget::default();
        assert_eq!(
            budget.call(&cancellation, || {
                cancellation.cancel();
                Ok(42)
            }),
            Err(Error::Cancelled)
        );
    }

    #[test]
    fn path_component_and_expansion_bounds_are_independent() {
        assert!(components(Path::new(&"a/".repeat(MAX_NATIVE_SKILL_PATH_COMPONENTS))).is_ok());
        assert!(
            components(Path::new(
                &"a/".repeat(MAX_NATIVE_SKILL_PATH_COMPONENTS + 1)
            ))
            .is_err()
        );
        assert!(components(Path::new(&"a".repeat(MAX_NATIVE_SKILL_PATH_BYTES + 1))).is_err());
        assert!(components(Path::new("absolute\\separator")).is_err());
    }
}
