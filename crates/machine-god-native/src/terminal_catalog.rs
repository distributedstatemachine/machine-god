//! Descriptor-relative terminal session namespace. Call only on a native worker.
//!
//! The retained lock coordinates cooperating hosts, not hostile same-UID code.
//! This catalog bounds session directory count, not profile-wide payload bytes.
//! No directory is removed or repaired and no persisted process authority exists.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fmt;

use machine_god_core::{BackgroundOutputOwner, TerminalSessionId};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{AtFlags, Dir, FileType, FlockOperation, Mode, OFlags};
use sha2::{Digest, Sha256};

const NAMESPACE: &str = "terminal-v1";
const SESSIONS: &str = "sessions";
const LOCK: &str = "catalog-lock";
const CAPACITY: usize = 256;
const DIRECTORY_MODE: Mode = Mode::from_raw_mode(0o700);
const FILE_MODE: Mode = Mode::from_raw_mode(0o600);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalCatalogError {
    Invalid,
    NotFound,
    Busy,
    Conflict,
    Corrupt,
    ResourceLimit,
    Unavailable,
}

impl fmt::Display for TerminalCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "invalid terminal catalog request",
            Self::NotFound => "terminal catalog entry unavailable",
            Self::Busy => "terminal catalog owner busy",
            Self::Conflict => "terminal catalog entry already exists",
            Self::Corrupt => "terminal catalog corrupt",
            Self::ResourceLimit => "terminal catalog resource limit",
            Self::Unavailable => "terminal catalog operation unavailable",
        })
    }
}

impl std::error::Error for TerminalCatalogError {}
type Result<T> = std::result::Result<T, TerminalCatalogError>;

pub(crate) struct TerminalCatalog {
    state_root: OwnedFd,
    namespace: OwnedFd,
    owner_root: OwnedFd,
    sessions: OwnedFd,
    lock: OwnedFd,
    owner_name: String,
    poisoned: bool,
}

/// An exact-spelling, inode-bound batch under one retained owner catalog.
/// Callers hold profile authority until final `validate`; opening a member
/// neither rescans all siblings nor performs directory durability barriers.
pub(crate) struct TerminalCatalogSnapshot<'a> {
    catalog: &'a TerminalCatalog,
    entries: Vec<(TerminalSessionId, i128, u128)>,
}

impl fmt::Debug for TerminalCatalogSnapshot<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalCatalogSnapshot")
            .finish_non_exhaustive()
    }
}

impl TerminalCatalogSnapshot<'_> {
    pub(crate) fn ids(&self) -> impl Iterator<Item = &TerminalSessionId> {
        self.entries.iter().map(|(id, _, _)| id)
    }

    pub(crate) fn open(&self, id: &TerminalSessionId) -> Result<OwnedFd> {
        self.catalog.validate()?;
        let index = self
            .entries
            .binary_search_by(|(candidate, _, _)| candidate.as_str().cmp(id.as_str()))
            .map_err(|_| TerminalCatalogError::NotFound)?;
        let (_, device, inode) = &self.entries[index];
        let directory = open_directory(&self.catalog.sessions, id.as_str())?;
        let stat = private(&directory, true)?;
        if i128::from(stat.st_dev) != *device || u128::from(stat.st_ino) != *inode {
            return Err(TerminalCatalogError::Corrupt);
        }
        Ok(directory)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.catalog.snapshot()?.entries != self.entries {
            return Err(TerminalCatalogError::Corrupt);
        }
        Ok(())
    }
}

impl fmt::Debug for TerminalCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalCatalog")
            .finish_non_exhaustive()
    }
}

impl TerminalCatalog {
    pub(crate) fn snapshot(&self) -> Result<TerminalCatalogSnapshot<'_>> {
        self.validate()?;
        let names = names(&self.sessions, CAPACITY)?;
        let mut entries = Vec::with_capacity(names.len());
        for name in names {
            let id = TerminalSessionId::new(name).map_err(|_| TerminalCatalogError::Corrupt)?;
            let stat = private(open_directory(&self.sessions, id.as_str())?, true)?;
            entries.push((id, i128::from(stat.st_dev), u128::from(stat.st_ino)));
        }
        entries.sort_unstable_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
        self.validate()?;
        Ok(TerminalCatalogSnapshot {
            catalog: self,
            entries,
        })
    }

    pub(crate) fn namespace_key(&self) -> &str {
        &self.owner_name
    }

    /// Bind a cooperating profile transaction to this exact retained catalog.
    /// Comparing an owner key alone is insufficient across separate profiles.
    pub(crate) fn validate_profile_binding(&self, state_root: impl AsFd) -> Result<()> {
        self.validate()?;
        let expected = private(state_root, true)?;
        let actual = private(&self.state_root, true)?;
        if expected.st_dev != actual.st_dev || expected.st_ino != actual.st_ino {
            return Err(TerminalCatalogError::Invalid);
        }
        Ok(())
    }

    pub(crate) fn prepare(
        state_root: OwnedFd,
        workspace: String,
        owner: BackgroundOutputOwner,
    ) -> Result<Self> {
        if !canonical_workspace(&workspace) {
            return Err(TerminalCatalogError::Invalid);
        }
        private(&state_root, true)?;
        let namespace = prepare_directory(&state_root, NAMESPACE)?;
        let owner_name = owner_name(&workspace, &owner);
        // Do not retain plaintext identity after deriving the directory key.
        drop(workspace);
        drop(owner);
        let owner_root = prepare_directory(&namespace, &owner_name)?;
        let lock = acquire_lock(&owner_root)?;
        let sessions = prepare_directory(&owner_root, SESSIONS)?;
        let catalog = Self {
            state_root,
            namespace,
            owner_root,
            sessions,
            lock,
            owner_name,
            poisoned: false,
        };
        catalog.list()?;
        Ok(catalog)
    }

    pub(crate) fn create(&mut self, id: &TerminalSessionId) -> Result<OwnedFd> {
        let existing = self.list()?;
        if existing.iter().any(|entry| entry == id) {
            return Err(TerminalCatalogError::Conflict);
        }
        if existing.len() == CAPACITY {
            return Err(TerminalCatalogError::ResourceLimit);
        }
        rustix::fs::mkdirat(&self.sessions, id.as_str(), DIRECTORY_MODE).map_err(io_error)?;
        // After mkdir succeeds, every later failure may leave a published entry.
        // Do not permit retry on this handle or guess that it is safe to remove.
        self.poisoned = true;
        let directory = finish_created_directory(&self.sessions, id.as_str())?;
        self.poisoned = false;
        Ok(directory)
    }

    pub(crate) fn open(&self, id: &TerminalSessionId) -> Result<OwnedFd> {
        // Exact spelling matters even on a case-insensitive filesystem. Listing
        // also rejects unrelated malformed entries instead of silently hiding them.
        if !self.list()?.iter().any(|entry| entry == id) {
            return Err(TerminalCatalogError::NotFound);
        }
        seal_existing_directory(&self.sessions, id.as_str())
    }

    pub(crate) fn list(&self) -> Result<Vec<TerminalSessionId>> {
        self.validate()?;
        let names = names(&self.sessions, CAPACITY)?;
        let mut ids = Vec::with_capacity(names.len());
        for name in names {
            let id = TerminalSessionId::new(name).map_err(|_| TerminalCatalogError::Corrupt)?;
            open_directory(&self.sessions, id.as_str())?;
            ids.push(id);
        }
        ids.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
        Ok(ids)
    }

    fn validate(&self) -> Result<()> {
        if self.poisoned {
            return Err(TerminalCatalogError::Unavailable);
        }
        private(&self.state_root, true)?;
        same_entry(&self.state_root, NAMESPACE, &self.namespace, true)?;
        same_entry(&self.namespace, &self.owner_name, &self.owner_root, true)?;
        same_entry(&self.owner_root, SESSIONS, &self.sessions, true)?;
        same_entry(&self.owner_root, LOCK, &self.lock, false)?;
        let entries = names(&self.owner_root, 2)?;
        if entries.len() != 2 || !entries.iter().all(|name| name == SESSIONS || name == LOCK) {
            return Err(TerminalCatalogError::Corrupt);
        }
        Ok(())
    }
}

pub(crate) fn canonical_workspace(workspace: &str) -> bool {
    workspace.len() <= 4096
        && workspace.starts_with('/')
        && !workspace.contains('\0')
        && (workspace == "/"
            || workspace[1..]
                .split('/')
                .all(|part| !matches!(part, "" | "." | "..")))
}

pub(crate) fn owner_name(workspace: &str, owner: &BackgroundOutputOwner) -> String {
    let mut digest = Sha256::new();
    digest.update(b"machine-god:terminal-catalog:v1\0");
    for part in [
        workspace,
        owner.session_id().as_str(),
        owner.session_incarnation_id().as_str(),
    ] {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

pub(crate) fn private(fd: impl AsFd, directory: bool) -> Result<rustix::fs::Stat> {
    let stat = rustix::fs::fstat(fd.as_fd()).map_err(io_error)?;
    let kind = FileType::from_raw_mode(stat.st_mode);
    let valid = stat.st_uid == rustix::process::geteuid().as_raw()
        && if directory {
            kind.is_dir() && u64::from(stat.st_mode) & 0o7777 == 0o700 && stat.st_nlink > 0
        } else {
            kind.is_file()
                && u64::from(stat.st_mode) & 0o7777 == 0o600
                && stat.st_nlink == 1
                && stat.st_size == 0
        };
    if !valid {
        return Err(TerminalCatalogError::Corrupt);
    }
    #[cfg(target_os = "macos")]
    {
        let acl = calcifer_macos_acl::read_acl(fd.as_fd())
            .map_err(|_| TerminalCatalogError::Unavailable)?;
        if acl.flags != 0
            || !acl.entries.iter().all(|entry| {
                entry.tag == calcifer_macos_acl::TAG_DENY
                    && entry.flags == 0
                    && entry.permissions == calcifer_macos_acl::PERMISSION_DELETE
            })
        {
            return Err(TerminalCatalogError::Corrupt);
        }
    }
    Ok(stat)
}

pub(crate) fn same_entry(
    parent: impl AsFd,
    name: &str,
    fd: impl AsFd,
    directory: bool,
) -> Result<()> {
    let before = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    let after = private(fd, directory)?;
    if before.st_dev != after.st_dev || before.st_ino != after.st_ino {
        return Err(TerminalCatalogError::Corrupt);
    }
    Ok(())
}

pub(crate) fn open_directory(parent: impl AsFd, name: &str) -> Result<OwnedFd> {
    let fd = rustix::fs::openat(
        parent.as_fd(),
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?;
    same_entry(parent, name, &fd, true)?;
    Ok(fd)
}

pub(crate) fn prepare_directory(parent: impl AsFd, name: &str) -> Result<OwnedFd> {
    match rustix::fs::mkdirat(parent.as_fd(), name, DIRECTORY_MODE) {
        Ok(()) => finish_created_directory(parent, name),
        Err(rustix::io::Errno::EXIST) => seal_existing_directory(parent, name),
        Err(error) => Err(io_error(error)),
    }
}

fn seal_existing_directory(parent: impl AsFd, name: &str) -> Result<OwnedFd> {
    // A previous attempt can have created this entry but failed its durability
    // barrier. Revalidate without chmod, then repeat both barriers before reuse.
    // Ordinary listing deliberately does not perform this explicit open work.
    let fd = open_directory(parent.as_fd(), name)?;
    sync_child(&fd)?;
    sync_parent(parent)?;
    Ok(fd)
}

fn finish_created_directory(parent: impl AsFd, name: &str) -> Result<OwnedFd> {
    // Only a newly created directory has its umask-filtered mode restored.
    // Never follow a substituted symlink or chmod a pre-existing entry.
    let before =
        rustix::fs::statat(parent.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    if !FileType::from_raw_mode(before.st_mode).is_dir()
        || before.st_uid != rustix::process::geteuid().as_raw()
    {
        return Err(TerminalCatalogError::Corrupt);
    }
    #[cfg(target_os = "linux")]
    restore_created_directory_mode(parent.as_fd(), name, &before, std::path::Path::new("/proc"))?;
    #[cfg(target_os = "macos")]
    rustix::fs::chmodat(
        parent.as_fd(),
        name,
        DIRECTORY_MODE,
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(io_error)?;
    let fd = open_directory(parent.as_fd(), name)?;
    let after = private(&fd, true)?;
    if before.st_dev != after.st_dev || before.st_ino != after.st_ino {
        return Err(TerminalCatalogError::Corrupt);
    }
    sync_child(&fd)?;
    sync_parent(parent)?;
    Ok(fd)
}

/// Linux's pinned chmodat rejects SYMLINK_NOFOLLOW. O_PATH binds even a
/// mode-000 directory without requiring a process-wide umask change. Only a
/// verified procfs magic link to this retained descriptor may restore its mode.
#[cfg(target_os = "linux")]
fn restore_created_directory_mode(
    parent: impl AsFd,
    name: &str,
    expected: &rustix::fs::Stat,
    proc_path: &std::path::Path,
) -> Result<()> {
    use rustix::fd::AsRawFd;

    let target = rustix::fs::openat(
        parent.as_fd(),
        name,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?;
    let retained = rustix::fs::fstat(&target).map_err(io_error)?;
    if !FileType::from_raw_mode(retained.st_mode).is_dir()
        || retained.st_uid != rustix::process::geteuid().as_raw()
        || retained.st_nlink == 0
        || retained.st_dev != expected.st_dev
        || retained.st_ino != expected.st_ino
    {
        return Err(TerminalCatalogError::Corrupt);
    }
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let proc_root = rustix::fs::open(proc_path, flags, Mode::empty()).map_err(io_error)?;
    if u64::try_from(rustix::fs::fstatfs(&proc_root).map_err(io_error)?.f_type).ok() != Some(0x9fa0)
    {
        return Err(TerminalCatalogError::Unavailable);
    }
    let mount = proc_mount_id(&proc_root, "", AtFlags::EMPTY_PATH)?;
    // A proc mount for a different PID namespace must not resolve an arbitrary
    // process with our numeric PID. Its kernel-generated self link must agree.
    let pid = rustix::process::getpid().as_raw_nonzero().get().to_string();
    let mut link = [0_u8; 32];
    let length = rustix::fs::readlinkat_raw(&proc_root, "self", &mut link[..]).map_err(io_error)?;
    if length == link.len()
        || &link[..length] != pid.as_bytes()
        || proc_mount_id(&proc_root, "self", AtFlags::SYMLINK_NOFOLLOW)? != mount
    {
        return Err(TerminalCatalogError::Unavailable);
    }
    let process =
        rustix::fs::openat(&proc_root, pid.as_str(), flags, Mode::empty()).map_err(io_error)?;
    let descriptors = rustix::fs::openat(&process, "fd", flags, Mode::empty()).map_err(io_error)?;
    for directory in [&process, &descriptors] {
        if proc_mount_id(directory, "", AtFlags::EMPTY_PATH)? != mount {
            return Err(TerminalCatalogError::Unavailable);
        }
    }
    let descriptor = target.as_raw_fd().to_string();
    if proc_mount_id(&descriptors, descriptor.as_str(), AtFlags::SYMLINK_NOFOLLOW)? != mount {
        return Err(TerminalCatalogError::Unavailable);
    }
    let resolved = rustix::fs::statat(&descriptors, descriptor.as_str(), AtFlags::empty())
        .map_err(io_error)?;
    if resolved.st_dev != retained.st_dev || resolved.st_ino != retained.st_ino {
        return Err(TerminalCatalogError::Corrupt);
    }
    // No ordinary directory entry is followed here. The exact target descriptor
    // stays owned until this effect completes; fd-number reuse is impossible.
    rustix::fs::chmodat(
        &descriptors,
        descriptor.as_str(),
        DIRECTORY_MODE,
        AtFlags::empty(),
    )
    .map_err(io_error)?;
    private(&target, true)?;
    same_entry(parent, name, &target, true)
}

#[cfg(target_os = "linux")]
fn proc_mount_id(directory: impl AsFd, path: &str, flags: AtFlags) -> Result<u64> {
    use rustix::fs::StatxFlags;
    let stat = rustix::fs::statx(directory, path, flags, StatxFlags::MNT_ID).map_err(io_error)?;
    if stat.stx_mask & StatxFlags::MNT_ID.bits() == 0 {
        return Err(TerminalCatalogError::Unavailable);
    }
    Ok(stat.stx_mnt_id)
}

fn acquire_lock(parent: impl AsFd) -> Result<OwnedFd> {
    let flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let lock = match rustix::fs::openat(
        parent.as_fd(),
        LOCK,
        flags | OFlags::CREATE | OFlags::EXCL,
        FILE_MODE,
    ) {
        Ok(fd) => {
            rustix::fs::fchmod(&fd, FILE_MODE).map_err(io_error)?;
            private(&fd, false)?;
            fd
        }
        Err(rustix::io::Errno::EXIST) => {
            rustix::fs::openat(parent.as_fd(), LOCK, flags, Mode::empty()).map_err(io_error)?
        }
        Err(error) => return Err(io_error(error)),
    };
    same_entry(parent.as_fd(), LOCK, &lock, false)?;
    match rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {}
        Err(rustix::io::Errno::WOULDBLOCK) => return Err(TerminalCatalogError::Busy),
        Err(error) => return Err(io_error(error)),
    }
    // Reused locks can also originate in an interrupted preparation attempt.
    sync_child(&lock)?;
    sync_parent(parent)?;
    Ok(lock)
}

pub(crate) fn names(root: impl AsFd, maximum: usize) -> Result<Vec<String>> {
    let mut directory = Dir::new(open_directory(root, ".")?).map_err(io_error)?;
    let mut names = Vec::new();
    for entry in &mut directory {
        let entry = entry.map_err(io_error)?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        if names.len() == maximum {
            return Err(TerminalCatalogError::ResourceLimit);
        }
        names.push(String::from_utf8(bytes.to_vec()).map_err(|_| TerminalCatalogError::Corrupt)?);
    }
    Ok(names)
}

pub(crate) fn sync_parent(parent: impl AsFd) -> Result<()> {
    #[cfg(test)]
    if FAIL_PARENT_SYNC.with(|failure| failure.replace(false)) {
        return Err(TerminalCatalogError::Unavailable);
    }
    #[cfg(test)]
    probe_sync(parent.as_fd(), true)?;
    rustix::fs::fsync(parent).map_err(io_error)
}

pub(crate) fn sync_child(child: impl AsFd) -> Result<()> {
    #[cfg(test)]
    probe_sync(child.as_fd(), false)?;
    rustix::fs::fsync(child).map_err(io_error)
}

fn io_error(error: rustix::io::Errno) -> TerminalCatalogError {
    match error {
        rustix::io::Errno::NOENT => TerminalCatalogError::NotFound,
        rustix::io::Errno::EXIST => TerminalCatalogError::Conflict,
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => TerminalCatalogError::Corrupt,
        _ => TerminalCatalogError::Unavailable,
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_PARENT_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SYNC_PROBE: std::cell::Cell<Option<SyncProbe>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct SyncProbe {
    inode: u128,
    child_calls: usize,
    parent_calls: usize,
    fail_child: bool,
    fail_parent: bool,
}

#[cfg(test)]
fn probe_sync(fd: impl AsFd, parent: bool) -> Result<()> {
    SYNC_PROBE.with(|cell| {
        let Some(mut probe) = cell.get() else {
            return Ok(());
        };
        if u128::from(rustix::fs::fstat(fd).map_err(io_error)?.st_ino) != probe.inode {
            return Ok(());
        }
        let fail = if parent {
            probe.parent_calls += 1;
            std::mem::replace(&mut probe.fail_parent, false)
        } else {
            probe.child_calls += 1;
            std::mem::replace(&mut probe.fail_child, false)
        };
        cell.set(Some(probe));
        if fail {
            Err(TerminalCatalogError::Unavailable)
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{SessionId, SessionIncarnationId};
    use std::fs;
    #[cfg(target_os = "linux")]
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "terminal-catalog-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            Self(root)
        }
        fn fd(&self) -> OwnedFd {
            rustix::fs::open(
                &self.0,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap()
        }
        fn catalog(&self) -> TerminalCatalog {
            self.try_catalog().unwrap()
        }
        fn try_catalog(&self) -> Result<TerminalCatalog> {
            // Parallel subprocess tests may briefly inherit a CLOEXEC lock
            // between fork and exec. A deliberately live owner's Busy check
            // below still calls prepare directly and must fail immediately.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                match TerminalCatalog::prepare(
                    self.fd(),
                    "/workspace".into(),
                    owner("session", "incarnation"),
                ) {
                    Err(TerminalCatalogError::Busy) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    result => return result,
                }
            }
        }
        fn owner_root(&self) -> PathBuf {
            self.0
                .join(NAMESPACE)
                .join(owner_name("/workspace", &owner("session", "incarnation")))
        }
        fn sessions(&self) -> PathBuf {
            self.owner_root().join(SESSIONS)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn owner(session: &str, incarnation: &str) -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new(session).unwrap(),
            SessionIncarnationId::new(incarnation).unwrap(),
        )
    }
    fn id(value: &str) -> TerminalSessionId {
        TerminalSessionId::new(value).unwrap()
    }

    #[test]
    fn profile_mediated_create_preserves_failed_publication_and_catalog_poison() {
        use crate::terminal_profile_store::{TerminalProfileStore, TerminalProfileStoreError};

        let fixture = Fixture::new();
        let store = TerminalProfileStore::prepare(fixture.fd()).unwrap();
        let mut transaction = store.transaction().unwrap();
        let mut catalog = transaction
            .prepare_catalog("/workspace".into(), owner("session", "incarnation"))
            .unwrap();
        FAIL_PARENT_SYNC.with(|failure| failure.set(true));
        assert_eq!(
            transaction
                .create_session(&mut catalog, &id("partial"))
                .unwrap_err(),
            TerminalProfileStoreError::Unavailable
        );
        assert!(fixture.sessions().join("partial").is_dir());
        assert_eq!(catalog.list(), Err(TerminalCatalogError::Unavailable));
        assert_eq!(transaction.inventory().unwrap().sessions.len(), 1);
        assert_eq!(
            transaction
                .create_session(&mut catalog, &id("later"))
                .unwrap_err(),
            TerminalProfileStoreError::Unavailable
        );
        assert!(!fixture.sessions().join("later").exists());
    }

    #[test]
    fn round_trip_order_collision_lock_and_redaction() {
        let fixture = Fixture::new();
        let mut catalog = fixture.catalog();
        for name in ["z", "A", "a.b_-0", &"x".repeat(255)] {
            let directory = catalog.create(&id(name)).unwrap();
            assert_eq!(private(directory, true).unwrap().st_mode & 0o7777, 0o700);
        }
        let names: Vec<_> = catalog
            .list()
            .unwrap()
            .into_iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        assert_eq!(names, ["A", "a.b_-0", &"x".repeat(255), "z"]);
        catalog.open(&id("A")).unwrap();
        assert_eq!(
            catalog.open(&id("a")).unwrap_err(),
            TerminalCatalogError::NotFound
        );
        assert_eq!(
            catalog.create(&id("A")).unwrap_err(),
            TerminalCatalogError::Conflict
        );
        assert_eq!(
            TerminalCatalog::prepare(
                fixture.fd(),
                "/workspace".into(),
                owner("session", "incarnation")
            )
            .unwrap_err(),
            TerminalCatalogError::Busy
        );
        assert!(!format!("{catalog:?}").contains("workspace"));
        let inode = fs::metadata(fixture.owner_root().join(LOCK)).unwrap().ino();
        drop(catalog);
        let reopened = fixture.catalog();
        assert_eq!(reopened.list().unwrap().len(), 4);
        assert_eq!(
            fs::metadata(fixture.owner_root().join(LOCK)).unwrap().ino(),
            inode
        );
    }

    #[test]
    fn exact_owner_and_workspace_are_length_framed_and_isolated() {
        let fixture = Fixture::new();
        let mut first = fixture.catalog();
        first.create(&id("same")).unwrap();
        for (workspace, session, incarnation) in [
            ("/workspace-other", "session", "incarnation"),
            ("/workspace", "other", "incarnation"),
            ("/workspace", "session", "other"),
        ] {
            let mut other = TerminalCatalog::prepare(
                fixture.fd(),
                workspace.into(),
                owner(session, incarnation),
            )
            .unwrap();
            assert!(other.list().unwrap().is_empty());
            other.create(&id("same")).unwrap();
        }
        assert_ne!(
            owner_name("/a", &owner("bc", "d")),
            owner_name("/ab", &owner("c", "d"))
        );
        assert_ne!(
            owner_name("/a", &owner("b", "cd")),
            owner_name("/a", &owner("bc", "d"))
        );
        assert_eq!(owner_name("/a", &owner("b", "c")).len(), 64);
    }

    #[test]
    fn capacity_is_bounded_without_eviction() {
        let fixture = Fixture::new();
        let mut catalog = fixture.catalog();
        // Seed the existing catalog directly, then exercise the actual final
        // admission boundary. Repeated full scans/fsyncs for all 256 seeds would
        // turn this count test into quadratic I/O alongside real PTY deadlines.
        for number in 0..CAPACITY - 1 {
            rustix::fs::mkdirat(&catalog.sessions, format!("s{number}"), DIRECTORY_MODE).unwrap();
        }
        catalog.create(&id(&format!("s{}", CAPACITY - 1))).unwrap();
        assert_eq!(catalog.list().unwrap().len(), CAPACITY);
        assert_eq!(
            catalog.create(&id("extra")).unwrap_err(),
            TerminalCatalogError::ResourceLimit
        );
        assert_eq!(
            catalog.create(&id("s0")).unwrap_err(),
            TerminalCatalogError::Conflict
        );
        catalog.open(&id("s0")).unwrap();
        fs::create_dir(fixture.sessions().join("unaccounted")).unwrap();
        fs::set_permissions(
            fixture.sessions().join("unaccounted"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert_eq!(
            catalog.list().unwrap_err(),
            TerminalCatalogError::ResourceLimit
        );
    }

    #[test]
    fn malformed_session_entries_fail_closed_without_deletion() {
        for spelling in ["bad space", "bad;name", "non-dir", "link", "mode"] {
            let fixture = Fixture::new();
            let catalog = fixture.catalog();
            let path = fixture.sessions().join(spelling);
            match spelling {
                "non-dir" => fs::write(&path, b"").unwrap(),
                "link" => symlink(&fixture.0, &path).unwrap(),
                _ => {
                    fs::create_dir(&path).unwrap();
                    let mode = if spelling == "mode" { 0o755 } else { 0o700 };
                    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
                }
            }
            assert_eq!(
                catalog.list().unwrap_err(),
                TerminalCatalogError::Corrupt,
                "{spelling}"
            );
            assert!(fs::symlink_metadata(&path).is_ok());
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn non_utf8_session_name_is_rejected() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        let path = fixture
            .sessions()
            .join(std::ffi::OsStr::from_bytes(b"\xff"));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(catalog.list().unwrap_err(), TerminalCatalogError::Corrupt);
    }

    #[test]
    fn root_and_namespace_validation_never_repairs_existing_entries() {
        for kind in [
            "root-mode",
            "namespace-file",
            "namespace-symlink",
            "namespace-mode",
        ] {
            let fixture = Fixture::new();
            let namespace = fixture.0.join(NAMESPACE);
            match kind {
                "root-mode" => {
                    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o755)).unwrap();
                }
                "namespace-file" => fs::write(&namespace, b"").unwrap(),
                "namespace-symlink" => symlink(&fixture.0, &namespace).unwrap(),
                _ => {
                    fs::create_dir(&namespace).unwrap();
                    fs::set_permissions(&namespace, fs::Permissions::from_mode(0o755)).unwrap();
                }
            }
            assert_eq!(
                TerminalCatalog::prepare(
                    fixture.fd(),
                    "/workspace".into(),
                    owner("session", "incarnation")
                )
                .unwrap_err(),
                TerminalCatalogError::Corrupt,
                "{kind}"
            );
        }
        for workspace in [
            "", "relative", "//double", "/a/", "/a/./b", "/a/../b", "/null\0",
        ] {
            let fixture = Fixture::new();
            assert_eq!(
                TerminalCatalog::prepare(
                    fixture.fd(),
                    workspace.into(),
                    owner("session", "incarnation")
                )
                .unwrap_err(),
                TerminalCatalogError::Invalid
            );
            assert!(!fixture.0.join(NAMESPACE).exists());
        }
    }

    #[test]
    fn retained_root_survives_path_replacement_but_internal_replacement_fails() {
        let fixture = Fixture::new();
        let retained = fixture.fd();
        let old = fixture.0.with_extension("retained");
        fs::rename(&fixture.0, &old).unwrap();
        fs::create_dir(&fixture.0).unwrap();
        let mut catalog = TerminalCatalog::prepare(
            retained,
            "/workspace".into(),
            owner("session", "incarnation"),
        )
        .unwrap();
        catalog.create(&id("original")).unwrap();
        assert!(!fixture.0.join(NAMESPACE).exists());
        assert_eq!(catalog.list().unwrap(), vec![id("original")]);
        // Restore the fixture spelling only for deterministic fixture cleanup.
        fs::remove_dir(&fixture.0).unwrap();
        fs::rename(&old, &fixture.0).unwrap();
        fs::rename(
            fixture.sessions(),
            fixture.owner_root().join("old-sessions"),
        )
        .unwrap();
        fs::create_dir(fixture.sessions()).unwrap();
        fs::set_permissions(fixture.sessions(), fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(catalog.list().unwrap_err(), TerminalCatalogError::Corrupt);
    }

    #[test]
    fn permanent_lock_replacement_and_unknown_owner_entries_are_rejected() {
        let fixture = Fixture::new();
        let catalog = fixture.catalog();
        fs::write(fixture.owner_root().join("unknown"), b"").unwrap();
        assert_eq!(
            catalog.list().unwrap_err(),
            TerminalCatalogError::ResourceLimit
        );
        fs::remove_file(fixture.owner_root().join("unknown")).unwrap();
        fs::rename(
            fixture.owner_root().join(LOCK),
            fixture.owner_root().join("old-lock"),
        )
        .unwrap();
        fs::write(fixture.owner_root().join(LOCK), b"").unwrap();
        fs::set_permissions(
            fixture.owner_root().join(LOCK),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert_eq!(catalog.list().unwrap_err(), TerminalCatalogError::Corrupt);
    }

    #[test]
    fn malformed_permanent_locks_are_not_replaced_or_repaired() {
        for kind in ["mode", "content", "hardlink", "directory", "symlink"] {
            let fixture = Fixture::new();
            drop(fixture.catalog());
            let lock = fixture.owner_root().join(LOCK);
            match kind {
                "mode" => fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap(),
                "content" => fs::write(&lock, b"unexpected").unwrap(),
                "hardlink" => fs::hard_link(&lock, fixture.0.join("alias")).unwrap(),
                "directory" => {
                    fs::remove_file(&lock).unwrap();
                    fs::create_dir(&lock).unwrap();
                }
                _ => {
                    fs::remove_file(&lock).unwrap();
                    symlink(&fixture.0, &lock).unwrap();
                }
            }
            assert!(
                TerminalCatalog::prepare(
                    fixture.fd(),
                    "/workspace".into(),
                    owner("session", "incarnation")
                )
                .is_err(),
                "{kind}"
            );
            assert!(fs::symlink_metadata(&lock).is_ok());
        }
    }

    #[test]
    fn failed_publication_poison_is_explicit_and_does_not_delete() {
        let fixture = Fixture::new();
        let mut catalog = fixture.catalog();
        FAIL_PARENT_SYNC.with(|failure| failure.set(true));
        assert_eq!(
            catalog.create(&id("ambiguous")).unwrap_err(),
            TerminalCatalogError::Unavailable
        );
        assert_eq!(
            catalog.list().unwrap_err(),
            TerminalCatalogError::Unavailable
        );
        assert_eq!(
            catalog.create(&id("retry")).unwrap_err(),
            TerminalCatalogError::Unavailable
        );
        assert!(fixture.sessions().join("ambiguous").is_dir());
        drop(catalog);
        assert_eq!(fixture.catalog().list().unwrap(), vec![id("ambiguous")]);
    }

    fn watch_sync(fd: impl AsFd, fail_child: bool, fail_parent: bool) {
        SYNC_PROBE.with(|cell| {
            cell.set(Some(SyncProbe {
                inode: u128::from(rustix::fs::fstat(fd).unwrap().st_ino),
                child_calls: 0,
                parent_calls: 0,
                fail_child,
                fail_parent,
            }));
        });
    }

    #[test]
    fn prepare_retries_failed_ancestor_publication_before_accepting_existing_namespace() {
        let fixture = Fixture::new();
        watch_sync(fixture.fd(), false, true);
        let prepare = || {
            TerminalCatalog::prepare(
                fixture.fd(),
                "/workspace".into(),
                owner("session", "incarnation"),
            )
        };
        assert_eq!(prepare().unwrap_err(), TerminalCatalogError::Unavailable);
        assert!(fixture.0.join(NAMESPACE).is_dir());
        // A second injected failure at this exact ancestor must still reject
        // prepare: merely opening the now-existing namespace is insufficient.
        SYNC_PROBE.with(|cell| {
            let mut probe = cell.get().unwrap();
            assert_eq!(probe.parent_calls, 1);
            probe.fail_parent = true;
            cell.set(Some(probe));
        });
        assert_eq!(prepare().unwrap_err(), TerminalCatalogError::Unavailable);
        let mut catalog = prepare().unwrap();
        SYNC_PROBE.with(|cell| {
            assert_eq!(cell.get().unwrap().parent_calls, 3);
            cell.set(None);
        });
        catalog.create(&id("durable-descendant")).unwrap();
    }

    #[test]
    fn existing_fixed_directories_and_lock_retry_child_barriers() {
        for target in 0..4 {
            let fixture = Fixture::new();
            let catalog = fixture.catalog();
            let descriptor = match target {
                0 => &catalog.namespace,
                1 => &catalog.owner_root,
                2 => &catalog.sessions,
                _ => &catalog.lock,
            };
            watch_sync(descriptor, true, false);
            drop(catalog);
            assert_eq!(
                fixture.try_catalog().unwrap_err(),
                TerminalCatalogError::Unavailable,
                "target {target}"
            );
            let reopened = fixture.catalog();
            assert!(reopened.list().unwrap().is_empty());
            SYNC_PROBE.with(|cell| {
                assert!(cell.get().unwrap().child_calls >= 2, "target {target}");
                cell.set(None);
            });
        }
    }

    #[test]
    fn explicit_session_open_reconciles_child_and_parent_without_syncing_list() {
        let fixture = Fixture::new();
        let mut catalog = fixture.catalog();
        FAIL_PARENT_SYNC.with(|failure| failure.set(true));
        assert_eq!(
            catalog.create(&id("ambiguous")).unwrap_err(),
            TerminalCatalogError::Unavailable
        );
        drop(catalog);
        let catalog = fixture.catalog();
        watch_sync(&catalog.sessions, false, true);
        assert_eq!(catalog.list().unwrap(), vec![id("ambiguous")]);
        SYNC_PROBE.with(|cell| assert_eq!(cell.get().unwrap().parent_calls, 0));
        assert_eq!(
            catalog.open(&id("ambiguous")).unwrap_err(),
            TerminalCatalogError::Unavailable
        );
        assert_eq!(catalog.list().unwrap(), vec![id("ambiguous")]);
        let session = catalog.open(&id("ambiguous")).unwrap();
        SYNC_PROBE.with(|cell| assert_eq!(cell.get().unwrap().parent_calls, 2));
        watch_sync(&session, true, false);
        assert_eq!(
            catalog.open(&id("ambiguous")).unwrap_err(),
            TerminalCatalogError::Unavailable
        );
        assert_eq!(catalog.list().unwrap(), vec![id("ambiguous")]);
        catalog.open(&id("ambiguous")).unwrap();
        SYNC_PROBE.with(|cell| {
            assert_eq!(cell.get().unwrap().child_calls, 2);
            cell.set(None);
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn created_directory_mode_zero_is_restored_through_retained_proc_descriptor() {
        let fixture = Fixture::new();
        let directory = fixture.0.join("created");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0)).unwrap();
        let retained = finish_created_directory(fixture.fd(), "created").unwrap();
        assert_eq!(private(&retained, true).unwrap().st_mode & 0o7777, 0o700);
        // Existing entries never pass through restoration, even if owned.
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
        assert!(prepare_directory(fixture.fd(), "created").is_err());
        assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o7777, 0o500);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn created_directory_rejects_absent_or_untrusted_proc_without_chmod() {
        let fixture = Fixture::new();
        let directory = fixture.0.join("created");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0)).unwrap();
        let parent = fixture.fd();
        let before = rustix::fs::statat(&parent, "created", AtFlags::SYMLINK_NOFOLLOW).unwrap();
        let missing = fixture.0.join("missing-proc");
        let linked = fixture.0.join("linked-proc");
        symlink("/proc", &linked).unwrap();
        for proc_path in [&missing, &fixture.0, &linked] {
            assert!(
                restore_created_directory_mode(&parent, "created", &before, proc_path).is_err()
            );
            assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o7777, 0);
        }
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn created_directory_rejects_substitution_before_mode_effect() {
        let fixture = Fixture::new();
        let directory = fixture.0.join("created");
        fs::create_dir(&directory).unwrap();
        let parent = fixture.fd();
        let before = rustix::fs::statat(&parent, "created", AtFlags::SYMLINK_NOFOLLOW).unwrap();
        fs::rename(&directory, fixture.0.join("original")).unwrap();
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
        assert!(
            restore_created_directory_mode(
                &parent,
                "created",
                &before,
                std::path::Path::new("/proc")
            )
            .is_err()
        );
        assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o7777, 0o500);
        fs::rename(&directory, fixture.0.join("replacement")).unwrap();
        symlink("replacement", &directory).unwrap();
        assert!(
            restore_created_directory_mode(
                &parent,
                "created",
                &before,
                std::path::Path::new("/proc")
            )
            .is_err()
        );
        assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o7777, 0o500);
    }

    #[test]
    fn hostile_umask_is_restored_only_for_new_objects() {
        const CHILD: &str = "MACHINE_GOD_CATALOG_UMASK_TEST";
        if std::env::var_os(CHILD).is_some() {
            let fixture = Fixture::new();
            // This branch runs alone in a separate test process.
            rustix::process::umask(Mode::from_raw_mode(0o777));
            let mut catalog = fixture.catalog();
            catalog.create(&id("private")).unwrap();
            catalog.open(&id("private")).unwrap();
            assert_eq!(
                fs::metadata(fixture.sessions().join("private"))
                    .unwrap()
                    .mode()
                    & 0o7777,
                0o700
            );
            assert_eq!(
                fs::metadata(fixture.owner_root().join(LOCK))
                    .unwrap()
                    .mode()
                    & 0o7777,
                0o600
            );
        } else {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "terminal_catalog::tests::hostile_umask_is_restored_only_for_new_objects",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
