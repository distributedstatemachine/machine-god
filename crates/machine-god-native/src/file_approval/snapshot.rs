//! Descriptor-owned approval evidence. This is deliberately not an undo entry.

use machine_god_core::CancellationToken;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    MAX_NATIVE_FILE_APPROVAL_PREIMAGE_BYTES, NativeFileApprovalError as Error,
    NativeFileApprovalKind as Kind, NativeFileApprovalPreimage,
};

pub(super) struct Snapshot {
    source: Option<Location>,
    target: Location,
    after: After,
}

struct Location {
    root: OwnedFd,
    path: String,
    logical_path: String,
    parent: OwnedFd,
    name: String,
    before: Preimage,
}

enum Preimage {
    Missing,
    File {
        descriptor: OwnedFd,
        stat: Stat,
        bytes: Vec<u8>,
    },
    EmptyDirectory {
        descriptor: OwnedFd,
        stat: Stat,
    },
}

enum After {
    Absent,
    Content(Vec<u8>),
    Source,
}

fn edit_error(error: machine_god_core::ToolError) -> Error {
    let kind = error.kind;
    drop(error);
    if kind == machine_god_core::ToolErrorKind::Cancelled {
        Error::Cancelled
    } else {
        Error::Invalid
    }
}

pub(super) fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
}

pub(super) fn check(cancellation: &CancellationToken) -> Result<(), Error> {
    if cancellation.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

fn call<T>(
    cancellation: &CancellationToken,
    operation: impl FnOnce() -> Result<T, rustix::io::Errno>,
) -> Result<T, Error> {
    check(cancellation)?;
    let result = operation();
    check(cancellation)?;
    result.map_err(|_| Error::Unavailable)
}

pub(super) fn same_identity(a: &Stat, b: &Stat) -> bool {
    a.st_dev == b.st_dev
        && a.st_ino == b.st_ino
        && FileType::from_raw_mode(a.st_mode) == FileType::from_raw_mode(b.st_mode)
}

fn stable(a: &Stat, b: &Stat) -> bool {
    same_identity(a, b)
        && a.st_mode == b.st_mode
        && a.st_size == b.st_size
        && a.st_mtime == b.st_mtime
        && a.st_mtime_nsec == b.st_mtime_nsec
        && a.st_ctime == b.st_ctime
        && a.st_ctime_nsec == b.st_ctime_nsec
}

fn linked_root(root: BorrowedFd<'_>, cancellation: &CancellationToken) -> Result<(), Error> {
    let stat = call(cancellation, || rustix::fs::fstat(root))?;
    if !FileType::from_raw_mode(stat.st_mode).is_dir() || stat.st_nlink == 0 {
        return Err(Error::Changed);
    }
    #[cfg(target_os = "macos")]
    {
        let path = call(cancellation, || rustix::fs::getpath(root))?;
        if path.as_bytes() != b"/" {
            let name = path
                .as_bytes()
                .rsplit(|byte| *byte == b'/')
                .next()
                .filter(|name| !name.is_empty())
                .ok_or(Error::Changed)?;
            let name = std::ffi::CString::new(name).map_err(|_| Error::Changed)?;
            let parent = call(cancellation, || {
                rustix::fs::openat(root, "..", directory_flags(), Mode::empty())
            })?;
            let named = call(cancellation, || {
                rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW)
            })?;
            if !same_identity(&stat, &named) {
                return Err(Error::Changed);
            }
        }
    }
    Ok(())
}

fn parent(
    root: BorrowedFd<'_>,
    path: &str,
    cancellation: &CancellationToken,
) -> Result<(OwnedFd, String), Error> {
    // Callers have already checked the concrete tool's canonical path grammar.
    linked_root(root, cancellation)?;
    let mut held = call(cancellation, || {
        rustix::fs::openat(root, ".", directory_flags(), Mode::empty())
    })?;
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        if components.peek().is_none() {
            return Ok((held, component.to_owned()));
        }
        held = call(cancellation, || {
            rustix::fs::openat(&held, component, directory_flags(), Mode::empty())
        })?;
    }
    Err(Error::Invalid)
}

fn empty_directory(
    descriptor: BorrowedFd<'_>,
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    let mut entries = call(cancellation, || rustix::fs::Dir::read_from(descriptor))?;
    // At most the two dot entries and an end/nonempty witness, never an
    // unbounded enumeration of an unapproved directory tree.
    for _ in 0..3 {
        check(cancellation)?;
        let next = entries.next();
        check(cancellation)?;
        match next {
            None => return Ok(()),
            Some(Ok(entry)) if matches!(entry.file_name().to_bytes(), b"." | b"..") => {}
            Some(Ok(_)) => return Err(Error::Invalid),
            Some(Err(_)) => return Err(Error::Unavailable),
        }
    }
    Err(Error::Limit)
}

/// Exact bounded read/compare; a stable size is not a substitute for content.
fn read_file(
    descriptor: BorrowedFd<'_>,
    expected: Option<&[u8]>,
    maximum: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, Error> {
    let before = call(cancellation, || rustix::fs::fstat(descriptor))?;
    let size = usize::try_from(before.st_size).map_err(|_| Error::Limit)?;
    if !FileType::from_raw_mode(before.st_mode).is_file() || size > maximum {
        return Err(Error::Limit);
    }
    if expected.is_some_and(|bytes| bytes.len() != size) {
        return Err(Error::Changed);
    }
    let mut bytes = Vec::new();
    if expected.is_none() {
        bytes.try_reserve_exact(size).map_err(|_| Error::Limit)?;
    }
    let mut buffer = [0_u8; 8192];
    let mut offset = 0_usize;
    let mut interruptions = 0;
    for _ in 0..4096 {
        check(cancellation)?;
        let requested = buffer
            .len()
            .min(size.saturating_sub(offset).saturating_add(1));
        let result = rustix::io::pread(descriptor, &mut buffer[..requested], offset as u64);
        check(cancellation)?;
        match result {
            Ok(0) => {
                if offset != size {
                    return Err(Error::Changed);
                }
                let after = call(cancellation, || rustix::fs::fstat(descriptor))?;
                if !stable(&before, &after) {
                    return Err(Error::Changed);
                }
                return Ok(bytes);
            }
            Ok(count) => {
                let end = offset
                    .checked_add(count)
                    .filter(|end| *end <= size)
                    .ok_or(Error::Changed)?;
                if let Some(expected) = expected {
                    if expected[offset..end] != buffer[..count] {
                        return Err(Error::Changed);
                    }
                } else {
                    bytes.extend_from_slice(&buffer[..count]);
                }
                offset = end;
            }
            Err(rustix::io::Errno::INTR) if interruptions < 15 => interruptions += 1,
            Err(_) => return Err(Error::Unavailable),
        }
    }
    Err(Error::Limit)
}

fn capture(
    root: BorrowedFd<'_>,
    path: &str,
    directory: bool,
    maximum: usize,
    cancellation: &CancellationToken,
) -> Result<Location, Error> {
    let (parent, name) = parent(root, path, cancellation)?;
    check(cancellation)?;
    let named = rustix::fs::statat(&parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW);
    check(cancellation)?;
    let before = match named {
        Err(rustix::io::Errno::NOENT) => Preimage::Missing,
        Err(_) => return Err(Error::Unavailable),
        Ok(named) => {
            let kind = FileType::from_raw_mode(named.st_mode);
            if !(kind.is_file() || directory && kind.is_dir()) {
                return Err(Error::Invalid);
            }
            let flags = if kind.is_dir() {
                directory_flags()
            } else {
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
            };
            let descriptor = call(cancellation, || {
                rustix::fs::openat(&parent, name.as_str(), flags, Mode::empty())
            })?;
            let stat = call(cancellation, || rustix::fs::fstat(&descriptor))?;
            if !stable(&named, &stat) {
                return Err(Error::Changed);
            }
            let before = if kind.is_file() {
                let bytes = read_file(descriptor.as_fd(), None, maximum, cancellation)?;
                Preimage::File {
                    descriptor,
                    stat,
                    bytes,
                }
            } else {
                empty_directory(descriptor.as_fd(), cancellation)?;
                Preimage::EmptyDirectory { descriptor, stat }
            };
            verify_preimage(&parent, &name, &before, cancellation)?;
            before
        }
    };
    Ok(Location {
        root: call(cancellation, || rustix::io::fcntl_dupfd_cloexec(root, 3))?,
        path: path.to_owned(),
        logical_path: path.to_owned(),
        parent,
        name,
        before,
    })
}

fn verify_preimage(
    parent: &OwnedFd,
    name: &str,
    expected: &Preimage,
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    check(cancellation)?;
    let named = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW);
    check(cancellation)?;
    match expected {
        Preimage::Missing => {
            if matches!(named, Err(rustix::io::Errno::NOENT)) {
                Ok(())
            } else {
                Err(Error::Changed)
            }
        }
        Preimage::File {
            descriptor,
            stat,
            bytes,
        } => {
            let named = named.map_err(|_| Error::Changed)?;
            let held = call(cancellation, || rustix::fs::fstat(descriptor))?;
            if !stable(stat, &named) || !stable(stat, &held) {
                return Err(Error::Changed);
            }
            read_file(
                descriptor.as_fd(),
                Some(bytes),
                MAX_NATIVE_FILE_APPROVAL_PREIMAGE_BYTES,
                cancellation,
            )?;
            let named_after = call(cancellation, || {
                rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
            })?;
            let held_after = call(cancellation, || rustix::fs::fstat(descriptor))?;
            if !stable(stat, &named_after) || !stable(stat, &held_after) {
                return Err(Error::Changed);
            }
            Ok(())
        }
        Preimage::EmptyDirectory { descriptor, stat } => {
            let named = named.map_err(|_| Error::Changed)?;
            let held = call(cancellation, || rustix::fs::fstat(descriptor))?;
            if !stable(stat, &named) || !stable(stat, &held) {
                return Err(Error::Changed);
            }
            empty_directory(descriptor.as_fd(), cancellation)?;
            let after = call(cancellation, || rustix::fs::fstat(descriptor))?;
            if !stable(stat, &after) {
                return Err(Error::Changed);
            }
            Ok(())
        }
    }
}

impl Snapshot {
    pub(super) fn prepare(
        root: BorrowedFd<'_>,
        kind: Kind,
        arguments: &Value,
        cancellation: &CancellationToken,
    ) -> Result<Self, Error> {
        Self::prepare_roots(root, root, kind, arguments, cancellation)
    }

    pub(super) fn prepare_endpoints(
        source: Option<&super::NativeFileEndpoint>,
        target: &super::NativeFileEndpoint,
        kind: Kind,
        arguments: &Value,
        cancellation: &CancellationToken,
    ) -> Result<Self, Error> {
        let mut snapshot = Self::prepare_roots(
            source.unwrap_or(target).root().as_fd(),
            target.root().as_fd(),
            kind,
            arguments,
            cancellation,
        )?;
        target
            .logical_path()
            .clone_into(&mut snapshot.target.logical_path);
        if let (Some(location), Some(endpoint)) = (&mut snapshot.source, source) {
            endpoint
                .logical_path()
                .clone_into(&mut location.logical_path);
        }
        Ok(snapshot)
    }

    fn prepare_roots(
        source_root: BorrowedFd<'_>,
        target_root: BorrowedFd<'_>,
        kind: Kind,
        arguments: &Value,
        cancellation: &CancellationToken,
    ) -> Result<Self, Error> {
        let get = |key: &str| arguments[key].as_str().ok_or(Error::Invalid);
        let (source_path, target_path) = match kind {
            Kind::Copy => (Some(get("source")?), get("destination")?),
            Kind::Rename => (Some(get("old_path")?), get("new_path")?),
            _ => (None, get("path")?),
        };
        let source = source_path
            .map(|path| {
                capture(
                    source_root,
                    path,
                    false,
                    MAX_NATIVE_FILE_APPROVAL_PREIMAGE_BYTES,
                    cancellation,
                )
            })
            .transpose()?;
        if source
            .as_ref()
            .is_some_and(|source| !matches!(source.before, Preimage::File { .. }))
        {
            return Err(Error::Invalid);
        }
        let maximum = match kind {
            Kind::Edit => crate::MAX_EDIT_FILE_EXISTING_BYTES,
            Kind::Copy | Kind::Rename => 0,
            _ => MAX_NATIVE_FILE_APPROVAL_PREIMAGE_BYTES,
        };
        let target = capture(
            target_root,
            target_path,
            kind == Kind::Delete,
            maximum,
            cancellation,
        )?;
        if matches!(kind, Kind::Copy | Kind::Rename) && !matches!(target.before, Preimage::Missing)
        {
            return Err(Error::Changed);
        }
        let after = match kind {
            Kind::Write => After::Content(get("content")?.as_bytes().to_vec()),
            Kind::Edit => {
                let Preimage::File { bytes, .. } = &target.before else {
                    return Err(Error::Invalid);
                };
                std::str::from_utf8(bytes).map_err(|_| Error::Invalid)?;
                let old = get("old_string")?.as_bytes();
                let offset = crate::edit_file::find_unique_match_with_budget(
                    bytes,
                    old,
                    cancellation,
                    crate::MAX_EDIT_FILE_MATCH_WORK_STEPS,
                    |_| {},
                )
                .map_err(edit_error)?;
                After::Content(
                    crate::edit_file::build_postimage_with_budget(
                        bytes,
                        offset,
                        old.len(),
                        get("new_string")?.as_bytes(),
                        cancellation,
                        crate::MAX_EDIT_FILE_RESULTING_BYTES,
                        |_| {},
                    )
                    .map_err(edit_error)?,
                )
            }
            Kind::Delete => {
                if matches!(target.before, Preimage::Missing) {
                    return Err(Error::Invalid);
                }
                After::Absent
            }
            Kind::Copy | Kind::Rename => After::Source,
        };
        let snapshot = Self {
            source,
            target,
            after,
        };
        snapshot.revalidate(cancellation)?;
        Ok(snapshot)
    }

    pub(super) fn target_path(&self) -> &str {
        &self.target.logical_path
    }
    pub(super) fn source_path(&self) -> Option<&str> {
        self.source
            .as_ref()
            .map(|location| location.logical_path.as_str())
    }
    pub(super) fn preimage(&self) -> NativeFileApprovalPreimage<'_> {
        match &self.target.before {
            Preimage::Missing => NativeFileApprovalPreimage::Missing,
            Preimage::File { bytes, .. } => NativeFileApprovalPreimage::File(bytes),
            Preimage::EmptyDirectory { .. } => NativeFileApprovalPreimage::EmptyDirectory,
        }
    }
    pub(super) fn source_preimage(&self) -> Option<&[u8]> {
        match &self.source.as_ref()?.before {
            Preimage::File { bytes, .. } => Some(bytes),
            _ => None,
        }
    }
    pub(super) fn postimage(&self) -> Option<&[u8]> {
        match &self.after {
            After::Absent => None,
            After::Content(bytes) => Some(bytes),
            After::Source => self.source_preimage(),
        }
    }
    pub(super) fn root_matches(&self, root: BorrowedFd<'_>) -> Result<(), Error> {
        let a = rustix::fs::fstat(root).map_err(|_| Error::Unavailable)?;
        for location in self.source.iter().chain(std::iter::once(&self.target)) {
            let b = rustix::fs::fstat(&location.root).map_err(|_| Error::Unavailable)?;
            if !same_identity(&a, &b) {
                return Err(Error::Changed);
            }
        }
        Ok(())
    }
    pub(super) fn endpoints_match(
        &self,
        source: Option<&super::NativeFileEndpoint>,
        target: &super::NativeFileEndpoint,
    ) -> Result<(), Error> {
        if source.is_some() != self.source.is_some() {
            return Err(Error::Changed);
        }
        for (location, endpoint) in self
            .source
            .iter()
            .zip(source)
            .chain(std::iter::once((&self.target, target)))
        {
            if location.path != endpoint.relative_path()
                || location.logical_path != endpoint.logical_path()
            {
                return Err(Error::Changed);
            }
            let held = rustix::fs::fstat(&location.root).map_err(|_| Error::Unavailable)?;
            let supplied = rustix::fs::fstat(endpoint.root()).map_err(|_| Error::Unavailable)?;
            if !same_identity(&held, &supplied) {
                return Err(Error::Changed);
            }
        }
        Ok(())
    }
    pub(super) fn revalidate(&self, cancellation: &CancellationToken) -> Result<(), Error> {
        for location in self.source.iter().chain(std::iter::once(&self.target)) {
            let (current, _) = parent(location.root.as_fd(), &location.path, cancellation)?;
            let current_stat = call(cancellation, || rustix::fs::fstat(&current))?;
            let original = call(cancellation, || rustix::fs::fstat(&location.parent))?;
            if !same_identity(&current_stat, &original) {
                return Err(Error::Changed);
            }
            verify_preimage(
                &location.parent,
                &location.name,
                &location.before,
                cancellation,
            )?;
        }
        check(cancellation)
    }
    pub(super) fn verify_stage(
        &self,
        parent: BorrowedFd<'_>,
        name: &str,
        descriptor: BorrowedFd<'_>,
        cancellation: &CancellationToken,
    ) -> Result<(), Error> {
        let expected = self.postimage().ok_or(Error::Invalid)?;
        let current_parent = call(cancellation, || rustix::fs::fstat(parent))?;
        let approved_parent = call(cancellation, || rustix::fs::fstat(&self.target.parent))?;
        if !same_identity(&current_parent, &approved_parent) {
            return Err(Error::Changed);
        }
        let staged = call(cancellation, || rustix::fs::fstat(descriptor))?;
        let expected_mode = match (&self.after, &self.target.before) {
            (After::Source, _) => match &self.source.as_ref().ok_or(Error::Invalid)?.before {
                Preimage::File { stat, .. } => stat.st_mode & 0o777,
                _ => return Err(Error::Invalid),
            },
            (_, Preimage::File { stat, .. }) => stat.st_mode & 0o777,
            (_, Preimage::Missing) => 0o644,
            _ => return Err(Error::Invalid),
        };
        if !FileType::from_raw_mode(staged.st_mode).is_file()
            || staged.st_mode & 0o7777 != expected_mode
        {
            return Err(Error::Changed);
        }
        // write_file deliberately retains a write-only stage by default. Open
        // just its exact no-follow name and bind it back to the held descriptor.
        let readable = call(cancellation, || {
            rustix::fs::openat(
                parent,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
        })?;
        let opened = call(cancellation, || rustix::fs::fstat(&readable))?;
        if !same_identity(&staged, &opened) {
            return Err(Error::Changed);
        }
        read_file(
            readable.as_fd(),
            Some(expected),
            MAX_NATIVE_FILE_APPROVAL_PREIMAGE_BYTES,
            cancellation,
        )?;
        let after = call(cancellation, || rustix::fs::fstat(&readable))?;
        let named = call(cancellation, || {
            rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        })?;
        if !stable(&opened, &after) || !stable(&after, &named) {
            return Err(Error::Changed);
        }
        Ok(())
    }
    pub(super) fn content_identity(
        &self,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<crate::NativePermissionRuleKey, Error> {
        let arguments = serde_json::to_vec(arguments).map_err(|_| Error::Invalid)?;
        let hash = |bytes: &[u8]| -> String { format!("{:x}", Sha256::digest(bytes)) };
        let preimage = match self.preimage() {
            NativeFileApprovalPreimage::Missing => "missing".to_owned(),
            NativeFileApprovalPreimage::EmptyDirectory => "empty-directory".to_owned(),
            NativeFileApprovalPreimage::File(bytes) => format!("file:{}", hash(bytes)),
        };
        let fields = [
            "machine-god-file-approval-v1".to_owned(),
            tool_name.to_owned(),
            hash(&arguments),
            self.source_path().unwrap_or("").to_owned(),
            self.target_path().to_owned(),
            preimage,
            self.source_preimage()
                .map_or_else(|| "no-source".into(), hash),
            self.postimage().map_or_else(|| "absent".into(), hash),
        ];
        let mut canonical = String::new();
        for field in fields {
            use std::fmt::Write;
            write!(&mut canonical, "{}:{field}", field.len()).map_err(|_| Error::Limit)?;
            if canonical.len() > crate::MAX_NATIVE_PERMISSION_IDENTITY_BYTES {
                return Err(Error::Limit);
            }
        }
        crate::NativePermissionRuleKey::new(
            crate::NativePermissionRuleKind::FileMutation,
            &canonical,
        )
        .map_err(|_| Error::Limit)
    }
}
