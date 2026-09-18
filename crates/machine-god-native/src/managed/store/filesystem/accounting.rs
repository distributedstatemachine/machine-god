//! Exact retained-file accounting under the exclusive journal owner.
use super::{
    Dir, EPOCH, Error, FILE_OVERHEAD, JournalLimits, Mode, OFlags, OWNER, OwnedFd, READ, Shared,
    directory_revision, head_name, read, validate_link, validate_private,
};
use crate::managed::store::{Accounting, JournalHead, transaction::capacity::Protection};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::managed::store) struct Usage {
    pub bytes: usize,
    pub entries: usize,
    pub protected_bytes: usize,
    pub protected_entries: usize,
    pub headroom_low_heads: usize,
}

impl Usage {
    fn add(self, other: Self, limits: JournalLimits) -> Result<Self, Error> {
        self.replace(Self::default(), other, limits)
    }

    fn replace(self, old: Self, new: Self, limits: JournalLimits) -> Result<Self, Error> {
        fn delta(current: usize, old: usize, new: usize) -> Result<usize, Error> {
            current
                .checked_sub(old)
                .and_then(|n| n.checked_add(new))
                .ok_or(Error::Limit)
        }
        let next = Self {
            bytes: delta(self.bytes, old.bytes, new.bytes)?,
            entries: delta(self.entries, old.entries, new.entries)?,
            protected_bytes: delta(
                self.protected_bytes,
                old.protected_bytes,
                new.protected_bytes,
            )?,
            protected_entries: delta(
                self.protected_entries,
                old.protected_entries,
                new.protected_entries,
            )?,
            headroom_low_heads: delta(
                self.headroom_low_heads,
                old.headroom_low_heads,
                new.headroom_low_heads,
            )?,
        };
        if next.bytes > limits.aggregate_bytes || next.entries > limits.directory_entries {
            return Err(Error::Limit);
        }
        Ok(next)
    }

    fn from_state(state: &Accounting) -> Self {
        Self {
            bytes: state.used,
            entries: state.entries,
            protected_bytes: state.protected_bytes,
            protected_entries: state.protected_entries,
            headroom_low_heads: state.headroom_low_heads,
        }
    }
}

/// At most a head, a page and their staging names. Include existing orphan
/// pages and stale staging files: reusing either must not double-charge it.
/// No file inventory or lifetime-sized in-memory index is retained.
pub(in crate::managed::store) struct PublicationAccounting {
    names: Vec<String>,
    before: Usage,
}
impl PublicationAccounting {
    pub(in crate::managed::store) fn capture(
        shared: &Shared,
        mut names: Vec<String>,
    ) -> Result<Self, Error> {
        if names.len() > 4 {
            return Err(Error::Invalid);
        }
        names.sort_unstable();
        names.dedup();
        let before = named_usage(shared, &names)?;
        let namespace_revision = directory_revision(&shared.root)?;
        if namespace_revision
            != shared
                .state
                .lock()
                .map_err(|_| Error::Invalid)?
                .namespace_revision
        {
            return Err(Error::Conflict);
        }
        Ok(Self { names, before })
    }

    /// Called only after both publications confirm durability. Any error keeps
    /// the exact pending receipt and operation allowance; explicit reconciliation
    /// reconstructs the inventory before releasing that fence.
    pub(in crate::managed::store) fn commit(self, shared: &Shared) -> Result<(), Error> {
        let namespace_revision = directory_revision(&shared.root)?;
        let after = named_usage(shared, &self.names)?;
        if directory_revision(&shared.root)? != namespace_revision {
            return Err(Error::Conflict);
        }
        let mut state = shared.state.lock().map_err(|_| Error::Invalid)?;
        let next = Usage::from_state(&state).replace(self.before, after, shared.limits)?;
        state.used = next.bytes;
        state.entries = next.entries;
        state.protected_bytes = next.protected_bytes;
        state.protected_entries = next.protected_entries;
        state.headroom_low_heads = next.headroom_low_heads;
        state.namespace_revision = namespace_revision;
        Ok(())
    }
}

fn named_usage(shared: &Shared, names: &[String]) -> Result<Usage, Error> {
    let mut used = Usage::default();
    for name in names {
        if let Some(entry) = entry_usage(&shared.root, name, shared.limits)? {
            used = used.add(entry, shared.limits)?;
        }
    }
    Ok(used)
}

fn entry_usage(root: &OwnedFd, name: &str, limits: JournalLimits) -> Result<Option<Usage>, Error> {
    #[cfg(test)]
    crate::managed::store::tests::accounting_entry_observed();
    let fd = match rustix::fs::openat(root, name, READ, Mode::empty()) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(_) => return Err(Error::Invalid),
    };
    validate_private(&fd, false).map_err(|_| Error::Invalid)?;
    validate_link(root, name, &fd).map_err(|_| Error::Conflict)?;
    let stat = rustix::fs::fstat(&fd).map_err(|_| Error::Persistence)?;
    let length = usize::try_from(stat.st_size).map_err(|_| Error::Limit)?;
    let mut used = Usage {
        bytes: length.checked_add(FILE_OVERHEAD).ok_or(Error::Limit)?,
        entries: 1,
        ..Usage::default()
    };
    if name.starts_with("h-") {
        let bytes = read(root, name, limits.head_bytes)?.ok_or(Error::Conflict)?;
        let head: JournalHead = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        if head_name(&head.id) != name {
            return Err(Error::Invalid);
        }
        let protection = Protection::from_head(&head, limits)?;
        used.protected_bytes = protection.bytes;
        used.protected_entries = protection.entries;
        used.headroom_low_heads = usize::from(
            bytes.len().saturating_add(4096) > limits.head_bytes
                || head.notice_reservations.len() >= 4092,
        );
    }
    Ok(Some(used))
}

pub(in crate::managed::store) fn scan_usage(
    root: &OwnedFd,
    limits: JournalLimits,
) -> Result<Usage, Error> {
    #[cfg(test)]
    crate::managed::store::tests::accounting_scan_started();
    let directory = rustix::fs::openat(root, ".", READ | OFlags::DIRECTORY, Mode::empty())
        .map_err(|_| Error::Persistence)?;
    let mut stream = Dir::new(directory).map_err(|_| Error::Persistence)?;
    let mut used = Usage::default();
    for entry in &mut stream {
        let entry = entry.map_err(|_| Error::Persistence)?;
        let name = entry.file_name();
        let bytes = name.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        // Bound traversal before opening the next entry.
        if used.entries == limits.directory_entries {
            return Err(Error::Limit);
        }
        let name = std::str::from_utf8(bytes).map_err(|_| Error::Invalid)?;
        if name != OWNER
            && name != EPOCH
            && !(name.starts_with("h-") || name.starts_with("p-") || name.starts_with("t-"))
        {
            return Err(Error::Invalid);
        }
        let entry = entry_usage(root, name, limits)?.ok_or(Error::Conflict)?;
        used = used.add(entry, limits)?;
    }
    Ok(used)
}
