//! Descriptor-confined profile accounting transactions. Native worker only.
//!
//! The permanent profile lock serializes cooperating hosts' short accounting
//! operations, independently of resident catalog and journal writer lifetimes.
//! Inventory is stat-only accounting, not recovery or process authority. All
//! cooperating publishers must hold this transaction while changing artifacts.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fmt;

use machine_god_core::{BackgroundOutputOwner, TerminalSessionId};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{FlockOperation, Mode, OFlags};

use crate::terminal_catalog::{
    TerminalCatalog, TerminalCatalogError, canonical_workspace, names, open_directory, owner_name,
    prepare_directory, private, same_entry, sync_child, sync_parent,
};
use crate::terminal_journal::{
    TerminalJournal, TerminalJournalError, TerminalJournalPhysicalUsage,
};

const NAMESPACE: &str = "terminal-v1";
const LOCK: &str = "profile-lock";
const CATALOG_LOCK: &str = "catalog-lock";
const SESSIONS: &str = "sessions";
pub(crate) const MAX_PROFILE_OWNERS: usize = 256;
const MAX_OWNER_SESSIONS: usize = 256;
pub(crate) const MAX_PROFILE_SESSIONS: usize = 1024;
const LOCK_FLAGS: OFlags = OFlags::RDWR
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalProfileStoreError {
    Invalid,
    NotFound,
    Busy,
    Conflict,
    Corrupt,
    ResourceLimit,
    Unavailable,
    Journal(TerminalJournalError),
}

impl fmt::Display for TerminalProfileStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid terminal profile request",
            Self::NotFound => "terminal profile entry unavailable",
            Self::Busy => "terminal profile transaction busy",
            Self::Conflict => "terminal profile entry already exists",
            Self::Corrupt => "terminal profile corrupt",
            Self::ResourceLimit => "terminal profile resource limit",
            Self::Unavailable => "terminal profile operation unavailable",
            Self::Journal(_) => "terminal profile journal accounting failed",
        })
    }
}

impl std::error::Error for TerminalProfileStoreError {}

impl From<TerminalCatalogError> for TerminalProfileStoreError {
    fn from(error: TerminalCatalogError) -> Self {
        match error {
            TerminalCatalogError::Invalid => Self::Invalid,
            TerminalCatalogError::NotFound => Self::NotFound,
            TerminalCatalogError::Busy => Self::Busy,
            TerminalCatalogError::Conflict => Self::Conflict,
            TerminalCatalogError::Corrupt => Self::Corrupt,
            TerminalCatalogError::ResourceLimit => Self::ResourceLimit,
            TerminalCatalogError::Unavailable => Self::Unavailable,
        }
    }
}

impl From<TerminalJournalError> for TerminalProfileStoreError {
    fn from(error: TerminalJournalError) -> Self {
        Self::Journal(error)
    }
}

type Result<T> = std::result::Result<T, TerminalProfileStoreError>;

pub(crate) struct TerminalProfileStore {
    state_root: OwnedFd,
    namespace: OwnedFd,
    lock: OwnedFd,
}

pub(crate) struct TerminalProfileTransaction<'a> {
    store: &'a TerminalProfileStore,
    // A fresh open file description: duplicating store.lock would share flock.
    lock: OwnedFd,
}

pub(crate) struct TerminalProfileInventory {
    /// Includes recognized incomplete owner namespaces with no sessions yet.
    pub(crate) owner_count: usize,
    pub(crate) sessions: Vec<TerminalProfileSessionUsage>,
    pub(crate) usage: TerminalJournalPhysicalUsage,
}

pub(crate) struct TerminalProfileSessionUsage {
    pub(crate) owner_namespace: String,
    pub(crate) session_id: TerminalSessionId,
    pub(crate) usage: TerminalJournalPhysicalUsage,
}

macro_rules! redacted {
    ($($name:ty),+ $(,)?) => {$(
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted!(
    TerminalProfileStore,
    TerminalProfileTransaction<'_>,
    TerminalProfileInventory,
    TerminalProfileSessionUsage,
);

impl TerminalProfileStore {
    pub(crate) fn prepare(state_root: OwnedFd) -> Result<Self> {
        private(&state_root, true)?;
        let namespace = prepare_directory(&state_root, NAMESPACE)?;
        let mode = Mode::from_raw_mode(0o600);
        let lock = match rustix::fs::openat(
            &namespace,
            LOCK,
            LOCK_FLAGS | OFlags::CREATE | OFlags::EXCL,
            mode,
        ) {
            Ok(fd) => {
                // Only the object exclusively created by this attempt is chmodded.
                rustix::fs::fchmod(&fd, mode).map_err(io_error)?;
                fd
            }
            Err(rustix::io::Errno::EXIST) => {
                rustix::fs::openat(&namespace, LOCK, LOCK_FLAGS, Mode::empty()).map_err(io_error)?
            }
            Err(error) => return Err(io_error(error)),
        };
        let store = Self {
            state_root,
            namespace,
            lock,
        };
        store.validate()?;
        // Retry both publication barriers even for an existing permanent lock.
        sync_child(&store.lock)?;
        #[cfg(test)]
        if FAIL_LOCK_PARENT_SYNC.with(|failure| failure.replace(false)) {
            return Err(TerminalProfileStoreError::Unavailable);
        }
        sync_parent(&store.namespace)?;
        store.validate()?;
        Ok(store)
    }

    pub(crate) fn transaction(&self) -> Result<TerminalProfileTransaction<'_>> {
        self.validate()?;
        let lock = rustix::fs::openat(&self.namespace, LOCK, LOCK_FLAGS, Mode::empty())
            .map_err(io_error)?;
        same_entry(&self.namespace, LOCK, &lock, false)?;
        if identity(&lock, false)? != identity(&self.lock, false)? {
            return Err(TerminalProfileStoreError::Corrupt);
        }
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive).map_err(io_error)?;
        let transaction = TerminalProfileTransaction { store: self, lock };
        transaction.validate()?;
        Ok(transaction)
    }

    fn validate(&self) -> Result<()> {
        private(&self.state_root, true)?;
        same_entry(&self.state_root, NAMESPACE, &self.namespace, true)?;
        same_entry(&self.namespace, LOCK, &self.lock, false)?;
        Ok(())
    }
}

impl TerminalProfileTransaction<'_> {
    pub(crate) fn validate(&self) -> Result<()> {
        self.store.validate()?;
        same_entry(&self.store.namespace, LOCK, &self.lock, false)?;
        Ok(())
    }

    /// Prepare one owner catalog under the already-held profile transaction.
    /// Existing and partial owner namespaces already consume their count slot.
    /// The returned catalog retains its independent owner lock after this short
    /// transaction ends. No journal payload is created by this operation.
    /// The exclusive borrow also serializes callers sharing this transaction;
    /// the filesystem lock alone only excludes other transactions.
    pub(crate) fn prepare_catalog(
        &mut self,
        workspace: String,
        owner: BackgroundOutputOwner,
    ) -> Result<TerminalCatalog> {
        if !canonical_workspace(&workspace) {
            return Err(TerminalProfileStoreError::Invalid);
        }
        // Owner IDs are validated core types; use the catalog's one canonical
        // length-framed hash instead of introducing another identity scheme.
        let key = owner_name(&workspace, &owner);
        let topology = self.topology()?;
        if !topology.iter().any(|entry| entry.name == key) && topology.len() == MAX_PROFILE_OWNERS {
            return Err(TerminalProfileStoreError::ResourceLimit);
        }
        self.validate()?;
        let root = rustix::io::fcntl_dupfd_cloexec(&self.store.state_root, 3).map_err(io_error)?;
        let result = TerminalCatalog::prepare(root, workspace, owner).map_err(Into::into);
        // Also revalidate after an error: preparation may have published known
        // partial directories. Never delete those or disguise catalog errors.
        self.validate()?;
        result
    }

    /// Admit an empty retained-session directory before mkdir. The catalog's
    /// existing per-owner limit, duplicate errors, fsync and poison rules remain
    /// authoritative; this adds exact profile binding and the global limit.
    pub(crate) fn create_session(
        &mut self,
        catalog: &mut TerminalCatalog,
        id: &TerminalSessionId,
    ) -> Result<OwnedFd> {
        self.validate()?;
        catalog.validate_profile_binding(&self.store.state_root)?;
        let topology = self.topology()?;
        let existing = topology
            .iter()
            .find(|entry| entry.name == catalog.namespace_key())
            .is_some_and(|entry| entry.sessions.iter().any(|session| session.id == *id));
        let count: usize = topology.iter().map(|entry| entry.sessions.len()).sum();
        if !existing && count == MAX_PROFILE_SESSIONS {
            return Err(TerminalProfileStoreError::ResourceLimit);
        }
        self.validate()?;
        let result = catalog.create(id).map_err(Into::into);
        self.validate()?;
        result
    }

    pub(crate) fn inventory(&self) -> Result<TerminalProfileInventory> {
        let before = self.topology()?;
        let mut inventory = TerminalProfileInventory {
            owner_count: before.len(),
            sessions: Vec::new(),
            usage: TerminalJournalPhysicalUsage::default(),
        };
        for owner in &before {
            if owner.sessions.is_empty() {
                continue;
            }
            let root = open_directory(&self.store.namespace, &owner.name)?;
            let sessions = open_directory(&root, SESSIONS)?;
            for session in &owner.sessions {
                let directory = open_directory(&sessions, session.id.as_str())?;
                if identity(&directory, true)? != session.identity {
                    return Err(TerminalProfileStoreError::Corrupt);
                }
                let usage = TerminalJournal::inspect_physical(directory)?;
                add_usage(&mut inventory.usage, usage)?;
                inventory.sessions.push(TerminalProfileSessionUsage {
                    owner_namespace: owner.name.clone(),
                    session_id: session.id.clone(),
                    usage,
                });
            }
        }
        // Revalidate every directory and lock identity, including earlier owners,
        // without retaining a descriptor per session. No writer lock is acquired.
        #[cfg(test)]
        BEFORE_RECHECK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook();
            }
        });
        if self.topology()? != before {
            return Err(TerminalProfileStoreError::Corrupt);
        }
        Ok(inventory)
    }

    pub(crate) fn open_session(
        &self,
        owner_namespace: &str,
        id: &TerminalSessionId,
    ) -> Result<OwnedFd> {
        if !valid_owner(owner_namespace) {
            return Err(TerminalProfileStoreError::Invalid);
        }
        let before = self.topology()?;
        let expected = before
            .iter()
            .find(|owner| owner.name == owner_namespace)
            .and_then(|owner| owner.sessions.iter().find(|session| session.id == *id))
            .ok_or(TerminalProfileStoreError::NotFound)?;
        let owner = open_directory(&self.store.namespace, owner_namespace)?;
        let sessions = open_directory(&owner, SESSIONS)?;
        let directory = open_directory(&sessions, id.as_str())?;
        if identity(&directory, true)? != expected.identity || self.topology()? != before {
            return Err(TerminalProfileStoreError::Corrupt);
        }
        Ok(directory)
    }

    fn topology(&self) -> Result<Vec<OwnerTopology>> {
        self.validate()?;
        let entries = sorted_names(&self.store.namespace, MAX_PROFILE_OWNERS + 1)?;
        if !entries.iter().any(|name| name == LOCK) {
            return Err(TerminalProfileStoreError::Corrupt);
        }
        let mut owners = Vec::new();
        let mut session_count = 0;
        for name in &entries {
            if name == LOCK {
                continue;
            }
            if !valid_owner(name) {
                return Err(TerminalProfileStoreError::Corrupt);
            }
            let root = open_directory(&self.store.namespace, name)?;
            let owner = owner_topology(&root, name.clone(), &mut session_count)?;
            same_entry(&self.store.namespace, &owner.name, &root, true)?;
            owners.push(owner);
        }
        if sorted_names(&self.store.namespace, MAX_PROFILE_OWNERS + 1)? != entries {
            return Err(TerminalProfileStoreError::Corrupt);
        }
        self.validate()?;
        Ok(owners)
    }
}

#[derive(Eq, PartialEq)]
struct OwnerTopology {
    name: String,
    identity: Identity,
    catalog_lock: Option<Identity>,
    sessions_root: Option<Identity>,
    sessions: Vec<SessionTopology>,
}

#[derive(Eq, PartialEq)]
struct SessionTopology {
    id: TerminalSessionId,
    identity: Identity,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Identity {
    device: i128,
    inode: u128,
}

fn identity(fd: impl AsFd, directory: bool) -> Result<Identity> {
    let stat = private(fd, directory)?;
    Ok(Identity {
        device: i128::from(stat.st_dev),
        inode: u128::from(stat.st_ino),
    })
}

fn owner_topology(root: impl AsFd, name: String, count: &mut usize) -> Result<OwnerTopology> {
    let mut owner = OwnerTopology {
        name,
        identity: identity(root.as_fd(), true)?,
        catalog_lock: None,
        sessions_root: None,
        sessions: Vec::new(),
    };
    let entries = sorted_names(root.as_fd(), 2)?;
    for entry in &entries {
        match entry.as_str() {
            CATALOG_LOCK => {
                let lock = rustix::fs::openat(
                    root.as_fd(),
                    CATALOG_LOCK,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(io_error)?;
                same_entry(root.as_fd(), CATALOG_LOCK, &lock, false)?;
                owner.catalog_lock = Some(identity(&lock, false)?);
            }
            SESSIONS => {
                let sessions = open_directory(root.as_fd(), SESSIONS)?;
                owner.sessions_root = Some(identity(&sessions, true)?);
                let names = sorted_names(&sessions, MAX_OWNER_SESSIONS)?;
                *count = count
                    .checked_add(names.len())
                    .filter(|count| *count <= MAX_PROFILE_SESSIONS)
                    .ok_or(TerminalProfileStoreError::ResourceLimit)?;
                for name in &names {
                    let id = TerminalSessionId::new(name.clone())
                        .map_err(|_| TerminalProfileStoreError::Corrupt)?;
                    let directory = open_directory(&sessions, id.as_str())?;
                    owner.sessions.push(SessionTopology {
                        id,
                        identity: identity(&directory, true)?,
                    });
                }
                if sorted_names(&sessions, MAX_OWNER_SESSIONS)? != names {
                    return Err(TerminalProfileStoreError::Corrupt);
                }
                same_entry(root.as_fd(), SESSIONS, &sessions, true)?;
            }
            _ => return Err(TerminalProfileStoreError::Corrupt),
        }
    }
    if sorted_names(root, 2)? != entries {
        return Err(TerminalProfileStoreError::Corrupt);
    }
    Ok(owner)
}

fn valid_owner(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sorted_names(root: impl AsFd, maximum: usize) -> Result<Vec<String>> {
    let mut entries = names(root, maximum)?;
    entries.sort_unstable();
    Ok(entries)
}

fn add_usage(
    total: &mut TerminalJournalPhysicalUsage,
    usage: TerminalJournalPhysicalUsage,
) -> Result<()> {
    macro_rules! add {
        ($($field:ident),+ $(,)?) => {$(
            total.$field = total.$field.checked_add(usage.$field)
                .ok_or(TerminalProfileStoreError::ResourceLimit)?;
        )+};
    }
    add!(
        raw_bytes,
        checkpoint_bytes,
        state_bytes,
        event_bytes,
        metadata_bytes,
        output_bytes,
        total_bytes
    );
    Ok(())
}

fn io_error(error: rustix::io::Errno) -> TerminalProfileStoreError {
    match error {
        rustix::io::Errno::NOENT => TerminalProfileStoreError::NotFound,
        rustix::io::Errno::WOULDBLOCK => TerminalProfileStoreError::Busy,
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => TerminalProfileStoreError::Corrupt,
        _ => TerminalProfileStoreError::Unavailable,
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_LOCK_PARENT_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static BEFORE_RECHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_catalog::TerminalCatalog;
    use crate::terminal_journal::TerminalJournalLimits;
    use machine_god_core::{BackgroundOutputOwner, SessionId, SessionIncarnationId};
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "terminal-profile-store-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            directory(&root);
            Self(root)
        }

        fn store(&self) -> TerminalProfileStore {
            TerminalProfileStore::prepare(self.fd()).unwrap()
        }

        fn fd(&self) -> OwnedFd {
            rustix::fs::open(
                &self.0,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap()
        }

        fn namespace(&self) -> PathBuf {
            self.0.join(NAMESPACE)
        }

        fn owner(&self, number: usize) -> PathBuf {
            let owner = self.namespace().join(format!("{number:064x}"));
            directory(&owner);
            owner
        }

        fn session(&self, owner: &Path, name: &str) -> PathBuf {
            assert!(owner.starts_with(self.namespace()));
            let sessions = owner.join(SESSIONS);
            if !sessions.exists() {
                directory(&sessions);
            }
            let session = sessions.join(name);
            directory(&session);
            session
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn directory(path: &Path) {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn file(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn id(name: &str) -> TerminalSessionId {
        TerminalSessionId::new(name).unwrap()
    }

    fn begin(store: &TerminalProfileStore) -> TerminalProfileTransaction<'_> {
        // Parallel PTY tests can briefly inherit a CLOEXEC lock between fork and
        // exec. Only expected-success acquisition retries; live-owner assertions
        // below still test the immediate nonblocking Busy response directly.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match store.transaction() {
                Err(TerminalProfileStoreError::Busy) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                result => return result.unwrap(),
            }
        }
    }

    fn logical_owner(name: &str) -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new(name).unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        )
    }

    fn catalog(transaction: &mut TerminalProfileTransaction<'_>, name: &str) -> TerminalCatalog {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match transaction.prepare_catalog("/workspace".into(), logical_owner(name)) {
                Err(TerminalProfileStoreError::Busy) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                result => return result.unwrap(),
            }
        }
    }

    #[test]
    fn catalog_preparation_and_creation_share_transaction_but_retain_owner_lifetime() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut transaction = begin(&store);
        let mut prepared = catalog(&mut transaction, "owner");
        assert_eq!(
            prepared.namespace_key(),
            owner_name("/workspace", &logical_owner("owner"))
        );
        let session = transaction
            .create_session(&mut prepared, &id("session"))
            .unwrap();
        assert_eq!(names(&session, 1).unwrap(), Vec::<String>::new());
        assert_eq!(prepared.list().unwrap(), [id("session")]);
        assert_eq!(
            transaction.inventory().unwrap().usage,
            TerminalJournalPhysicalUsage::default()
        );
        assert_eq!(
            store.transaction().unwrap_err(),
            TerminalProfileStoreError::Busy
        );
        drop(transaction);
        let mut next = begin(&store);
        assert_eq!(
            next.prepare_catalog("/workspace".into(), logical_owner("owner"))
                .unwrap_err(),
            TerminalProfileStoreError::Busy
        );
        drop(prepared);
        let reopened = catalog(&mut next, "owner");
        assert_eq!(reopened.list().unwrap(), [id("session")]);
    }

    #[test]
    fn invalid_workspace_rejects_before_catalog_filesystem_effects() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut transaction = begin(&store);
        for workspace in ["relative", "/a/..", "/a//b", "/a/", "/a\0b"] {
            assert_eq!(
                transaction
                    .prepare_catalog(workspace.into(), logical_owner("owner"))
                    .unwrap_err(),
                TerminalProfileStoreError::Invalid
            );
        }
        assert_eq!(sorted_names(&store.namespace, 1).unwrap(), [LOCK]);
        assert_eq!(transaction.inventory().unwrap().owner_count, 0);
    }

    #[test]
    fn existing_partial_owner_at_cap_is_prepared_without_another_slot() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let key = owner_name("/workspace", &logical_owner("existing"));
        let owner_root = fixture.namespace().join(&key);
        directory(&owner_root);
        fixture.session(&owner_root, "retained");
        let inode = fs::metadata(&owner_root).unwrap().ino();
        for number in 0..MAX_PROFILE_OWNERS - 1 {
            fixture.owner(number);
        }
        let mut transaction = begin(&store);
        let prepared = catalog(&mut transaction, "existing");
        assert_eq!(prepared.namespace_key(), key);
        assert_eq!(prepared.list().unwrap(), [id("retained")]);
        assert_eq!(fs::metadata(&owner_root).unwrap().ino(), inode);
        assert_eq!(
            transaction.inventory().unwrap().owner_count,
            MAX_PROFILE_OWNERS
        );
        let new_key = owner_name("/workspace", &logical_owner("new"));
        assert_eq!(
            transaction
                .prepare_catalog("/workspace".into(), logical_owner("new"))
                .unwrap_err(),
            TerminalProfileStoreError::ResourceLimit
        );
        assert!(!fixture.namespace().join(new_key).exists());
        drop(prepared);
        assert_eq!(
            catalog(&mut transaction, "existing").list().unwrap(),
            [id("retained")]
        );
    }

    #[test]
    fn empty_partial_owner_is_completed_without_discarding_known_entries() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let key = owner_name("/workspace", &logical_owner("empty"));
        let owner_root = fixture.namespace().join(&key);
        directory(&owner_root);
        let mut transaction = begin(&store);
        let mut prepared = catalog(&mut transaction, "empty");
        assert_eq!(transaction.inventory().unwrap().owner_count, 1);
        transaction
            .create_session(&mut prepared, &id("new"))
            .unwrap();
        assert_eq!(prepared.list().unwrap(), [id("new")]);
        assert_eq!(fs::read_dir(owner_root).unwrap().count(), 2);
    }

    #[test]
    fn global_session_cap_preflights_mkdir_and_duplicates_keep_conflict() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut transaction = begin(&store);
        let mut full = catalog(&mut transaction, "full");
        let full_root = fixture.namespace().join(full.namespace_key());
        for number in 0..MAX_OWNER_SESSIONS {
            fixture.session(&full_root, &format!("s{number}"));
        }
        // The existing catalog enforces its own limit while profile capacity
        // remains available; the wrapper does not temporarily rewrite it.
        assert_eq!(
            transaction
                .create_session(&mut full, &id("overflow"))
                .unwrap_err(),
            TerminalProfileStoreError::ResourceLimit
        );
        for number in 0..3 {
            let owner = fixture.owner(number);
            for session in 0..MAX_OWNER_SESSIONS {
                fixture.session(&owner, &format!("s{session}"));
            }
        }
        let mut empty = catalog(&mut transaction, "empty");
        assert_eq!(
            transaction.inventory().unwrap().sessions.len(),
            MAX_PROFILE_SESSIONS
        );
        assert_eq!(
            transaction
                .create_session(&mut empty, &id("overflow"))
                .unwrap_err(),
            TerminalProfileStoreError::ResourceLimit
        );
        assert!(empty.list().unwrap().is_empty());
        assert!(
            !fixture
                .namespace()
                .join(empty.namespace_key())
                .join(SESSIONS)
                .join("overflow")
                .exists()
        );
        assert_eq!(
            transaction
                .create_session(&mut full, &id("s0"))
                .unwrap_err(),
            TerminalProfileStoreError::Conflict
        );
        assert_eq!(full.list().unwrap().len(), MAX_OWNER_SESSIONS);
    }

    #[test]
    fn cross_profile_catalog_with_identical_owner_key_rejects_before_mkdir() {
        let local = Fixture::new();
        let foreign = Fixture::new();
        let local_store = local.store();
        let foreign_store = foreign.store();
        let mut local_transaction = begin(&local_store);
        let mut foreign_transaction = begin(&foreign_store);
        let mut local_catalog = catalog(&mut local_transaction, "same-owner");
        let mut foreign_catalog = catalog(&mut foreign_transaction, "same-owner");
        assert_eq!(
            local_catalog.namespace_key(),
            foreign_catalog.namespace_key()
        );
        assert_eq!(
            local_transaction
                .create_session(&mut foreign_catalog, &id("wrong-profile"))
                .unwrap_err(),
            TerminalProfileStoreError::Invalid
        );
        assert!(local_catalog.list().unwrap().is_empty());
        assert!(foreign_catalog.list().unwrap().is_empty());
        local_transaction
            .create_session(&mut local_catalog, &id("local"))
            .unwrap();
        assert_eq!(local_catalog.list().unwrap(), [id("local")]);
        assert!(foreign_catalog.list().unwrap().is_empty());
    }

    #[test]
    fn replaced_profile_lock_rejects_both_catalog_mutations_before_effects() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut transaction = begin(&store);
        let mut prepared = catalog(&mut transaction, "existing");
        let lock = fixture.namespace().join(LOCK);
        fs::rename(&lock, fixture.0.join("old-profile-lock")).unwrap();
        file(&lock, b"");
        assert_eq!(
            transaction
                .prepare_catalog("/workspace".into(), logical_owner("new"))
                .unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
        assert_eq!(
            transaction
                .create_session(&mut prepared, &id("new"))
                .unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
        assert!(prepared.list().unwrap().is_empty());
        assert!(
            !fixture
                .namespace()
                .join(owner_name("/workspace", &logical_owner("new")))
                .exists()
        );
    }

    #[test]
    fn fresh_lock_descriptions_exclude_reentry_and_other_store_until_drop() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let other = fixture.store();
        let transaction = begin(&store);
        assert_eq!(
            store.transaction().unwrap_err(),
            TerminalProfileStoreError::Busy
        );
        assert_eq!(
            other.transaction().unwrap_err(),
            TerminalProfileStoreError::Busy
        );
        transaction.validate().unwrap();
        let usage = transaction.inventory().unwrap();
        assert!(usage.sessions.is_empty());
        assert_eq!(usage.usage, TerminalJournalPhysicalUsage::default());
        drop(transaction);
        let next = begin(&other);
        next.validate().unwrap();
        drop(next);
        begin(&store);
    }

    #[test]
    fn profile_lock_excludes_another_process_and_releases_on_drop() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let transaction = begin(&store);
        let probe = |busy: bool| {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "terminal_profile_store::tests::profile_lock_child_probe",
                ])
                .env("MACHINE_GOD_PROFILE_LOCK_PROBE", &fixture.0)
                .env("MACHINE_GOD_PROFILE_LOCK_BUSY", busy.to_string())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        };
        probe(true);
        drop(transaction);
        probe(false);
    }

    #[test]
    fn profile_lock_child_probe() {
        let Some(path) = std::env::var_os("MACHINE_GOD_PROFILE_LOCK_PROBE") else {
            return;
        };
        let root = rustix::fs::open(
            Path::new(&path),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap();
        let store = TerminalProfileStore::prepare(root).unwrap();
        if std::env::var("MACHINE_GOD_PROFILE_LOCK_BUSY").unwrap() == "true" {
            assert_eq!(
                store.transaction().unwrap_err(),
                TerminalProfileStoreError::Busy
            );
        } else {
            begin(&store);
        }
    }

    #[test]
    fn preparation_reuses_exact_private_objects_and_rejects_changed_modes() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let namespace = fixture.namespace();
        let lock = namespace.join(LOCK);
        let namespace_inode = fs::metadata(&namespace).unwrap().ino();
        let lock_inode = fs::metadata(&lock).unwrap().ino();
        let other = fixture.store();
        assert_eq!(fs::metadata(&namespace).unwrap().ino(), namespace_inode);
        assert_eq!(fs::metadata(&lock).unwrap().ino(), lock_inode);
        fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            TerminalProfileStore::prepare(fixture.fd()).unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
        assert_eq!(fs::metadata(&lock).unwrap().mode() & 0o777, 0o644);
        assert_eq!(
            store.transaction().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
        assert_eq!(
            other.transaction().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
    }

    #[test]
    fn failed_lock_publication_is_retried_without_replacing_the_lock() {
        let fixture = Fixture::new();
        FAIL_LOCK_PARENT_SYNC.with(|failure| failure.set(true));
        assert_eq!(
            TerminalProfileStore::prepare(fixture.fd()).unwrap_err(),
            TerminalProfileStoreError::Unavailable
        );
        let lock = fixture.namespace().join(LOCK);
        let inode = fs::metadata(&lock).unwrap().ino();
        let store = fixture.store();
        assert_eq!(fs::metadata(&lock).unwrap().ino(), inode);
        begin(&store).validate().unwrap();
        // A retry of an existing lock still reaches the parent publication barrier.
        FAIL_LOCK_PARENT_SYNC.with(|failure| failure.set(true));
        assert_eq!(
            TerminalProfileStore::prepare(fixture.fd()).unwrap_err(),
            TerminalProfileStoreError::Unavailable
        );
        assert_eq!(fs::metadata(&lock).unwrap().ino(), inode);
    }

    #[test]
    fn incomplete_owner_preparations_count_every_retained_session() {
        let fixture = Fixture::new();
        let store = fixture.store();
        fixture.owner(4);
        let lock_only = fixture.owner(3);
        file(&lock_only.join(CATALOG_LOCK), b"");
        let missing_lock = fixture.owner(2);
        fixture.session(&missing_lock, "b");
        fixture.session(&missing_lock, "a");
        let transaction = begin(&store);
        let inventory = transaction.inventory().unwrap();
        assert_eq!(inventory.sessions.len(), 2);
        assert_eq!(inventory.sessions[0].session_id, id("a"));
        assert_eq!(inventory.sessions[1].session_id, id("b"));
        assert_eq!(inventory.usage, TerminalJournalPhysicalUsage::default());
        transaction
            .open_session(&format!("{:064x}", 2), &id("a"))
            .unwrap();
    }

    #[test]
    fn busy_catalog_and_journal_are_counted_without_writer_authority() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut catalog = TerminalCatalog::prepare(
            fixture.fd(),
            "/workspace".into(),
            BackgroundOutputOwner::new(
                SessionId::new("session").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            ),
        )
        .unwrap();
        let mut journal = TerminalJournal::create(
            catalog.create(&id("retained")).unwrap(),
            id("retained"),
            TerminalJournalLimits::default(),
        )
        .unwrap();
        journal.append(b"payload").unwrap();
        let expected = journal.physical_usage().unwrap();
        let transaction = begin(&store);
        let inventory = transaction.inventory().unwrap();
        assert_eq!(inventory.sessions.len(), 1);
        assert_eq!(inventory.sessions[0].usage, expected);
        assert_eq!(inventory.usage, expected);
        assert_eq!(inventory.usage.raw_bytes, 7);
        drop(journal);
        drop(catalog);
        assert_eq!(transaction.inventory().unwrap().usage, expected);
    }

    #[test]
    fn exact_spelling_and_owner_validation_precede_lookup() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let owner = fixture.owner(10);
        fixture.session(&owner, "MixedCase");
        let transaction = begin(&store);
        let name = format!("{:064x}", 10);
        for invalid in ["../sessions", ".", "", &name.to_uppercase()] {
            assert_eq!(
                transaction
                    .open_session(invalid, &id("MixedCase"))
                    .unwrap_err(),
                TerminalProfileStoreError::Invalid
            );
        }
        assert_eq!(
            transaction
                .open_session(&name, &id("mixedcase"))
                .unwrap_err(),
            TerminalProfileStoreError::NotFound
        );
        transaction.open_session(&name, &id("MixedCase")).unwrap();
    }

    #[test]
    fn unknown_namespace_owner_and_session_artifacts_fail_closed() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let transaction = begin(&store);
        let unknown = fixture.namespace().join("unexpected");
        file(&unknown, b"x");
        assert_eq!(
            transaction.inventory().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
        fs::remove_file(&unknown).unwrap();
        let owner = fixture.owner(1);
        file(&owner.join("unexpected"), b"x");
        assert_eq!(
            transaction.inventory().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
        fs::remove_file(owner.join("unexpected")).unwrap();
        let session = fixture.session(&owner, "retained");
        file(&session.join("unexpected"), b"x");
        assert!(matches!(
            transaction.inventory(),
            Err(TerminalProfileStoreError::Journal(_))
        ));
        assert_eq!(fs::read(session.join("unexpected")).unwrap(), b"x");
    }

    #[test]
    fn unsafe_directories_locks_and_symlinks_are_never_repaired() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let owner = fixture.owner(1);
        let transaction = begin(&store);
        fs::set_permissions(&owner, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            transaction.inventory().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
        assert_eq!(fs::metadata(&owner).unwrap().mode() & 0o777, 0o755);
        fs::set_permissions(&owner, fs::Permissions::from_mode(0o700)).unwrap();
        let lock = owner.join(CATALOG_LOCK);
        file(&lock, b"x");
        assert_eq!(
            transaction.inventory().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
        fs::remove_file(&lock).unwrap();
        symlink(&fixture.0, &lock).unwrap();
        assert_eq!(
            transaction.inventory().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
        fs::remove_file(&lock).unwrap();
        file(&lock, b"");
        fs::hard_link(&lock, fixture.0.join("lock-alias")).unwrap();
        assert_eq!(
            transaction.inventory().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
    }

    #[test]
    fn retained_profile_lock_and_namespace_replacement_invalidate_handles() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let transaction = begin(&store);
        let lock = fixture.namespace().join(LOCK);
        fs::rename(&lock, fixture.0.join("old-lock")).unwrap();
        file(&lock, b"");
        assert_eq!(
            transaction.validate(),
            Err(TerminalProfileStoreError::Corrupt)
        );
        assert_eq!(
            store.transaction().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
        drop(transaction);
        let current = fixture.store();
        fs::rename(fixture.namespace(), fixture.0.join("old-namespace")).unwrap();
        directory(&fixture.namespace());
        file(&fixture.namespace().join(LOCK), b"");
        assert_eq!(
            current.transaction().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
    }

    #[test]
    fn inventory_rechecks_prior_session_identity_after_scanning() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let owner = fixture.owner(1);
        let session = fixture.session(&owner, "retained");
        let moved = fixture.0.join("old-session");
        BEFORE_RECHECK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&session, moved).unwrap();
                directory(&session);
            }));
        });
        assert_eq!(
            begin(&store).inventory().unwrap_err(),
            TerminalProfileStoreError::Corrupt
        );
    }

    #[test]
    fn owner_count_is_bounded_including_empty_partial_owners() {
        let fixture = Fixture::new();
        let store = fixture.store();
        for number in 0..MAX_PROFILE_OWNERS {
            fixture.owner(number);
        }
        let transaction = begin(&store);
        assert!(transaction.inventory().unwrap().sessions.is_empty());
        fixture.owner(MAX_PROFILE_OWNERS);
        assert_eq!(
            transaction.inventory().unwrap_err(),
            TerminalProfileStoreError::ResourceLimit
        );
    }

    #[test]
    fn per_owner_session_count_is_bounded() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let owner = fixture.owner(1);
        for number in 0..MAX_OWNER_SESSIONS {
            fixture.session(&owner, &format!("s{number}"));
        }
        let transaction = begin(&store);
        assert_eq!(
            transaction.inventory().unwrap().sessions.len(),
            MAX_OWNER_SESSIONS
        );
        fixture.session(&owner, "overflow");
        assert_eq!(
            transaction.inventory().unwrap_err(),
            TerminalProfileStoreError::ResourceLimit
        );
    }

    #[test]
    fn total_session_count_is_bounded_across_owner_namespaces() {
        let fixture = Fixture::new();
        let store = fixture.store();
        for number in 0..4 {
            let owner = fixture.owner(number);
            for session in 0..MAX_OWNER_SESSIONS {
                fixture.session(&owner, &format!("s{session}"));
            }
        }
        let transaction = begin(&store);
        assert_eq!(
            transaction.inventory().unwrap().sessions.len(),
            MAX_PROFILE_SESSIONS
        );
        let extra = fixture.owner(4);
        fixture.session(&extra, "overflow");
        assert_eq!(
            transaction.inventory().unwrap_err(),
            TerminalProfileStoreError::ResourceLimit
        );
    }

    #[test]
    fn physical_accounting_addition_is_checked() {
        let usage = TerminalJournalPhysicalUsage {
            raw_bytes: 1,
            checkpoint_bytes: 2,
            state_bytes: 3,
            event_bytes: 4,
            metadata_bytes: 5,
            output_bytes: 3,
            total_bytes: 15,
        };
        let mut total = TerminalJournalPhysicalUsage::default();
        add_usage(&mut total, usage).unwrap();
        assert_eq!(total, usage);
        let max = TerminalJournalPhysicalUsage {
            raw_bytes: u64::MAX,
            checkpoint_bytes: u64::MAX,
            state_bytes: u64::MAX,
            event_bytes: u64::MAX,
            metadata_bytes: u64::MAX,
            output_bytes: u64::MAX,
            total_bytes: u64::MAX,
        };
        assert_eq!(
            add_usage(&mut total, max),
            Err(TerminalProfileStoreError::ResourceLimit)
        );
    }
}
