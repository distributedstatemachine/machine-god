//! Explicit, descriptor-bound publication of native user defaults.

use std::fmt;
use std::fs::File;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, FlockOperation, Mode, OFlags};

use crate::config::{
    NativeConfiguredPermissionMutation, NativeConfiguredPermissionMutationOutcome,
    NativeConfiguredPermissionScope, parse_config_bytes, read_bounded,
};
use crate::{LoadedNativeConfig, NativeConfigError, NativeModelPreferences};

const DATA: &str = "config.json";
const LOCK: &str = ".config.lock";
const TEMP: &str = ".config.tmp";
const READ: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

/// A fixed, redacted user-default persistence outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeUserConfigError {
    /// The explicitly granted path or an entry is not safe to use.
    UnsafePath,
    /// A competing writer currently owns the settings lock.
    Busy,
    /// The snapshot no longer describes this store's current contents.
    Conflict,
    /// Loaded or proposed configuration is invalid; the file was not overwritten.
    InvalidConfig(NativeConfigError),
    /// Publication failed before replacing the configuration.
    Persistence,
    /// Replacement occurred but directory durability could not be confirmed.
    CommitAmbiguous,
}

impl fmt::Display for NativeUserConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsafePath => "native user configuration path is unsafe",
            Self::Busy => "native user configuration writer is busy",
            Self::Conflict => "native user configuration snapshot conflicts",
            Self::InvalidConfig(_) => "native user configuration is invalid",
            Self::Persistence => "native user configuration publication failed",
            Self::CommitAmbiguous => "native user configuration publication is indeterminate",
        })
    }
}
impl std::error::Error for NativeUserConfigError {}

/// Explicit authority over one configuration directory; construction is inert.
pub struct NativeUserConfigStore {
    directory: PathBuf,
    identity: Arc<()>,
}

impl fmt::Debug for NativeUserConfigStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeUserConfigStore")
            .finish_non_exhaustive()
    }
}

/// Read-only observation and exact-byte compare-and-swap token.
///
/// Retains the parent and any existing root descriptor, and is bound to the
/// originating store instance. It cannot authorize a different store.
pub struct NativeUserConfigSnapshot {
    loaded: LoadedNativeConfig,
    bytes: Option<Vec<u8>>,
    parent: OwnedFd,
    root: Option<OwnedFd>,
    identity: Arc<()>,
}

/// Confirmed persistent result, independent of any later runtime reload.
#[derive(Debug)]
pub struct NativeUserPermissionCommit {
    pub outcome: NativeConfiguredPermissionMutationOutcome,
    pub loaded: LoadedNativeConfig,
}

impl NativeUserConfigSnapshot {
    /// Returns the observed configuration, retaining legacy source schema labels.
    #[must_use]
    pub const fn loaded(&self) -> &LoadedNativeConfig {
        &self.loaded
    }
}
impl fmt::Debug for NativeUserConfigSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeUserConfigSnapshot")
            .finish_non_exhaustive()
    }
}

impl NativeUserConfigStore {
    /// Grants the final configuration directory beneath an existing parent.
    /// No environment is read and no filesystem effects occur here.
    #[must_use]
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            identity: Arc::new(()),
        }
    }

    fn component(&self) -> Result<&std::ffi::OsStr, NativeUserConfigError> {
        if !self.directory.is_absolute()
            || self
                .directory
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(NativeUserConfigError::UnsafePath);
        }
        self.directory
            .file_name()
            .ok_or(NativeUserConfigError::UnsafePath)
    }

    /// Reads a bounded snapshot without creating files, directories or locks.
    ///
    /// # Errors
    /// Rejects unsafe roots, invalid configurations and unavailable parent authority.
    pub fn load(&self) -> Result<NativeUserConfigSnapshot, NativeUserConfigError> {
        let component = self.component()?;
        let parent = rustix::fs::open(
            self.directory
                .parent()
                .ok_or(NativeUserConfigError::UnsafePath)?,
            READ | OFlags::DIRECTORY,
            Mode::empty(),
        )
        .map_err(|_| NativeUserConfigError::UnsafePath)?;
        let root = open_root(&parent, component)?;
        let bytes = root.as_ref().map(read_current).transpose()?.flatten();
        let loaded = decode(bytes.as_deref())?;
        Ok(NativeUserConfigSnapshot {
            loaded,
            bytes,
            parent,
            root,
            identity: self.identity.clone(),
        })
    }

    /// Atomically upgrades and changes only the requested default model controls.
    ///
    /// The borrowed future is inert until polled. Its one bounded synchronous
    /// transaction never spawns work; dropping it cannot leave a detached writer.
    /// Lock contention returns `Busy` rather than blocking an executor thread.
    ///
    /// # Errors
    /// Returns a conflict for changed contents/root or a foreign snapshot. Errors
    /// after rename are `CommitAmbiguous` and must be reconciled by a fresh load.
    #[allow(clippy::unused_async)] // Explicitly inert borrowed future; no detached blocking task.
    pub async fn set_model_preferences(
        &self,
        snapshot: &NativeUserConfigSnapshot,
        preferences: &NativeModelPreferences,
    ) -> Result<LoadedNativeConfig, NativeUserConfigError> {
        self.publish_preferences(snapshot, preferences, |root| rustix::fs::fsync(root))
    }

    fn publish_preferences(
        &self,
        snapshot: &NativeUserConfigSnapshot,
        preferences: &NativeModelPreferences,
        sync_directory: impl FnOnce(&OwnedFd) -> rustix::io::Result<()>,
    ) -> Result<LoadedNativeConfig, NativeUserConfigError> {
        let config = snapshot.loaded.config().with_model_preferences(preferences);
        self.publish_config(snapshot, config, sync_directory)
    }

    /// Edits the global or exact workspace-local configured permission list.
    /// This is not tool-registry validation, human confirmation or a live grant.
    /// The borrowed future is inert before polling and never detaches a writer.
    /// No-op edits make no writes and retain the observed source schema. Their
    /// result is an observation, not a reservation against subsequent changes.
    /// # Errors
    /// Rejects malformed/oversized candidates before creating publication files,
    /// stale or foreign snapshots, contention, unsafe entries and failed writes.
    /// An error after replacement remains `CommitAmbiguous`, not a safe retry.
    #[allow(clippy::unused_async)] // One synchronous owned transaction, inert before poll.
    pub async fn apply_permission_mutation(
        &self,
        snapshot: &NativeUserConfigSnapshot,
        workspace: &Path,
        scope: NativeConfiguredPermissionScope,
        mutation: &NativeConfiguredPermissionMutation,
    ) -> Result<NativeUserPermissionCommit, NativeUserConfigError> {
        self.publish_permission_mutation(snapshot, workspace, scope, mutation, |root| {
            rustix::fs::fsync(root)
        })
    }

    fn publish_permission_mutation(
        &self,
        snapshot: &NativeUserConfigSnapshot,
        workspace: &Path,
        scope: NativeConfiguredPermissionScope,
        mutation: &NativeConfiguredPermissionMutation,
        sync_directory: impl FnOnce(&OwnedFd) -> rustix::io::Result<()>,
    ) -> Result<NativeUserPermissionCommit, NativeUserConfigError> {
        if !Arc::ptr_eq(&self.identity, &snapshot.identity) {
            return Err(NativeUserConfigError::Conflict);
        }
        let (config, outcome) = snapshot
            .loaded
            .config()
            .with_permission_mutation(workspace, scope, mutation)
            .map_err(NativeUserConfigError::InvalidConfig)?;
        let loaded = if outcome == NativeConfiguredPermissionMutationOutcome::Unchanged {
            self.validate_unchanged(snapshot)?;
            snapshot.loaded.clone()
        } else {
            self.publish_config(snapshot, config, sync_directory)?
        };
        Ok(NativeUserPermissionCommit { outcome, loaded })
    }

    fn validate_unchanged(
        &self,
        snapshot: &NativeUserConfigSnapshot,
    ) -> Result<(), NativeUserConfigError> {
        let name = self.component()?;
        let observed = open_root(&snapshot.parent, name)?;
        let bytes = match (&snapshot.root, observed) {
            (Some(expected), Some(actual)) if same_file(expected, &actual)? => {
                let bytes = read_current(&actual)?;
                validate_link(&snapshot.parent, name, &actual)?;
                bytes
            }
            (None, None) => None,
            _ => return Err(NativeUserConfigError::Conflict),
        };
        if bytes != snapshot.bytes {
            return Err(NativeUserConfigError::Conflict);
        }
        Ok(())
    }

    fn publish_config(
        &self,
        snapshot: &NativeUserConfigSnapshot,
        config: crate::NativeConfig,
        sync_directory: impl FnOnce(&OwnedFd) -> rustix::io::Result<()>,
    ) -> Result<LoadedNativeConfig, NativeUserConfigError> {
        if !Arc::ptr_eq(&self.identity, &snapshot.identity) {
            return Err(NativeUserConfigError::Conflict);
        }
        // Bound the complete candidate before any directory, lock or temp creation.
        let encoded = config
            .serialize_current()
            .map_err(NativeUserConfigError::InvalidConfig)?;
        let name = self.component()?;
        let observed = open_root(&snapshot.parent, name)?;
        let root = match (&snapshot.root, observed) {
            (Some(expected), Some(actual)) if same_file(expected, &actual)? => actual,
            (None, None) => {
                rustix::fs::mkdirat(&snapshot.parent, name, Mode::from_raw_mode(0o700))
                    .map_err(|_| NativeUserConfigError::Conflict)?;
                let root =
                    open_root(&snapshot.parent, name)?.ok_or(NativeUserConfigError::Persistence)?;
                rustix::fs::fsync(&snapshot.parent)
                    .map_err(|_| NativeUserConfigError::Persistence)?;
                root
            }
            _ => return Err(NativeUserConfigError::Conflict),
        };
        let lock = open_lock(&root)?;
        let _lock_guard = lock_config(&lock)?;
        validate_link(&root, LOCK, &lock)?;
        let bytes = read_current(&root)?;
        decode(bytes.as_deref())?;
        if bytes != snapshot.bytes {
            return Err(NativeUserConfigError::Conflict);
        }
        let temp = rustix::fs::openat(
            &root,
            TEMP,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| NativeUserConfigError::Persistence)?;
        let mut temp = File::from(temp);
        if rustix::fs::fchmod(&temp, Mode::RUSR | Mode::WUSR).is_err()
            || validate_private(&temp, false).is_err()
        {
            remove_owned_temp(&root, &temp);
            return Err(NativeUserConfigError::Persistence);
        }
        let prepared = write_bounded(&mut temp, &encoded).and_then(|()| temp.sync_all());
        if prepared.is_err() {
            remove_owned_temp(&root, &temp);
            return Err(NativeUserConfigError::Persistence);
        }
        // Recheck both links and bytes while holding the cooperative writer lock.
        let checked = (|| {
            validate_link(&snapshot.parent, name, &root)?;
            validate_link(&root, LOCK, &lock)?;
            validate_link(&root, TEMP, &temp)?;
            if read_current(&root)? != snapshot.bytes {
                return Err(NativeUserConfigError::Conflict);
            }
            Ok(())
        })();
        if let Err(error) = checked {
            remove_owned_temp(&root, &temp);
            return Err(error);
        }
        if rustix::fs::renameat(&root, TEMP, &root, DATA).is_err() {
            remove_owned_temp(&root, &temp);
            return Err(NativeUserConfigError::Persistence);
        }
        sync_directory(&root).map_err(|_| NativeUserConfigError::CommitAmbiguous)?;
        validate_link(&snapshot.parent, name, &root)
            .map_err(|_| NativeUserConfigError::CommitAmbiguous)?;
        Ok(LoadedNativeConfig::from_file(config))
    }
}

#[must_use = "retain the guard until the configuration transaction completes"]
struct ConfigLockGuard<'a>(&'a OwnedFd);

impl Drop for ConfigLockGuard<'_> {
    fn drop(&mut self) {
        // An inherited or duplicated open-file description may outlive the local
        // descriptor. Explicit nonblocking unlock releases its shared lock now;
        // local close remains the fallback on a non-interruption OS failure.
        let _ = retry_lock_interrupted(|| rustix::fs::flock(self.0, FlockOperation::Unlock));
    }
}

fn lock_config(lock: &OwnedFd) -> Result<ConfigLockGuard<'_>, NativeUserConfigError> {
    retry_lock_interrupted(|| rustix::fs::flock(lock, FlockOperation::NonBlockingLockExclusive))
        .map_err(|error| {
            if error == rustix::io::Errno::WOULDBLOCK {
                NativeUserConfigError::Busy
            } else {
                NativeUserConfigError::Persistence
            }
        })?;
    Ok(ConfigLockGuard(lock))
}

fn retry_lock_interrupted<T>(
    mut operation: impl FnMut() -> rustix::io::Result<T>,
) -> rustix::io::Result<T> {
    loop {
        match operation() {
            Err(rustix::io::Errno::INTR) => {}
            result => return result,
        }
    }
}

fn remove_owned_temp(root: &OwnedFd, temp: &File) {
    if validate_link(root, TEMP, temp).is_ok() {
        let _ = rustix::fs::unlinkat(root, TEMP, AtFlags::empty());
    }
}

fn write_bounded(writer: &mut impl Write, mut bytes: &[u8]) -> std::io::Result<()> {
    let mut interruptions = 0;
    while !bytes.is_empty() {
        match writer.write(bytes) {
            Ok(written) if written > 0 && written <= bytes.len() => bytes = &bytes[written..],
            Ok(_) => return Err(std::io::ErrorKind::WriteZero.into()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                interruptions += 1;
                if interruptions >= 16 {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn decode(bytes: Option<&[u8]>) -> Result<LoadedNativeConfig, NativeUserConfigError> {
    bytes.map_or_else(
        || Ok(LoadedNativeConfig::built_in_defaults()),
        |bytes| {
            parse_config_bytes(bytes)
                .map(LoadedNativeConfig::from_file)
                .map_err(NativeUserConfigError::InvalidConfig)
        },
    )
}

fn open_root(
    parent: &OwnedFd,
    name: &std::ffi::OsStr,
) -> Result<Option<OwnedFd>, NativeUserConfigError> {
    match rustix::fs::openat(parent, name, READ | OFlags::DIRECTORY, Mode::empty()) {
        Ok(root) => {
            validate_private(&root, true)?;
            Ok(Some(root))
        }
        Err(e) if e == rustix::io::Errno::NOENT => Ok(None),
        Err(_) => Err(NativeUserConfigError::UnsafePath),
    }
}

fn validate_private(
    fd: &impl rustix::fd::AsFd,
    directory: bool,
) -> Result<(), NativeUserConfigError> {
    let stat = rustix::fs::fstat(fd).map_err(|_| NativeUserConfigError::Persistence)?;
    let kind = FileType::from_raw_mode(stat.st_mode);
    if stat.st_uid != nix::unistd::Uid::effective().as_raw()
        || stat.st_mode & 0o077 != 0
        || if directory {
            !kind.is_dir()
        } else {
            !kind.is_file() || stat.st_nlink != 1
        }
    {
        return Err(NativeUserConfigError::UnsafePath);
    }
    #[cfg(target_os = "macos")]
    {
        let acl = calcifer_macos_acl::read_acl(fd.as_fd())
            .map_err(|_| NativeUserConfigError::UnsafePath)?;
        if acl.flags != 0
            || acl.entries.iter().any(|entry| {
                entry.tag != calcifer_macos_acl::TAG_DENY
                    || entry.flags != 0
                    || entry.permissions != calcifer_macos_acl::PERMISSION_DELETE
            })
        {
            return Err(NativeUserConfigError::UnsafePath);
        }
    }
    Ok(())
}

fn same_file(
    a: &impl rustix::fd::AsFd,
    b: &impl rustix::fd::AsFd,
) -> Result<bool, NativeUserConfigError> {
    let a = rustix::fs::fstat(a).map_err(|_| NativeUserConfigError::Persistence)?;
    let b = rustix::fs::fstat(b).map_err(|_| NativeUserConfigError::Persistence)?;
    Ok(a.st_dev == b.st_dev && a.st_ino == b.st_ino)
}

fn validate_link(
    parent: &OwnedFd,
    name: impl rustix::path::Arg,
    fd: &impl rustix::fd::AsFd,
) -> Result<(), NativeUserConfigError> {
    let linked = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| NativeUserConfigError::Conflict)?;
    let opened = rustix::fs::fstat(fd).map_err(|_| NativeUserConfigError::Persistence)?;
    if linked.st_dev != opened.st_dev || linked.st_ino != opened.st_ino {
        return Err(NativeUserConfigError::Conflict);
    }
    Ok(())
}

fn read_current(root: &OwnedFd) -> Result<Option<Vec<u8>>, NativeUserConfigError> {
    let fd = match rustix::fs::openat(root, DATA, READ, Mode::empty()) {
        Ok(fd) => fd,
        Err(e) if e == rustix::io::Errno::NOENT => return Ok(None),
        Err(_) => return Err(NativeUserConfigError::UnsafePath),
    };
    // Existing legacy files may be world-readable; never accept nonregular or
    // multiply-linked entries. New publications always have private permissions.
    let stat = rustix::fs::fstat(&fd).map_err(|_| NativeUserConfigError::Persistence)?;
    if !FileType::from_raw_mode(stat.st_mode).is_file()
        || stat.st_nlink != 1
        || stat.st_uid != nix::unistd::Uid::effective().as_raw()
        || stat.st_mode & 0o022 != 0
    {
        return Err(NativeUserConfigError::UnsafePath);
    }
    let mut file = File::from(fd);
    read_bounded(&mut file)
        .map(Some)
        .map_err(NativeUserConfigError::InvalidConfig)
}

fn open_lock(root: &OwnedFd) -> Result<OwnedFd, NativeUserConfigError> {
    let flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let fd = match rustix::fs::openat(
        root,
        LOCK,
        flags | OFlags::CREATE | OFlags::EXCL,
        Mode::from_raw_mode(0o600),
    ) {
        Ok(fd) => {
            rustix::fs::fchmod(&fd, Mode::RUSR | Mode::WUSR)
                .map_err(|_| NativeUserConfigError::Persistence)?;
            fd
        }
        Err(e) if e == rustix::io::Errno::EXIST => {
            rustix::fs::openat(root, LOCK, flags, Mode::empty())
                .map_err(|_| NativeUserConfigError::UnsafePath)?
        }
        Err(_) => return Err(NativeUserConfigError::Persistence),
    };
    validate_private(&fd, false)?;
    Ok(fd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn config_lock_scope_releases_a_surviving_open_description_duplicate() {
        let directory =
            std::env::temp_dir().join(format!("mg-user-config-lock-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = rustix::fs::open(&directory, READ | OFlags::DIRECTORY, Mode::empty()).unwrap();
        let survivor = {
            let original = open_lock(&root).unwrap();
            let _guard = lock_config(&original).unwrap();
            let survivor = rustix::io::dup(&original).unwrap();
            let contender = open_lock(&root).unwrap();
            assert!(matches!(
                lock_config(&contender),
                Err(NativeUserConfigError::Busy)
            ));
            survivor
        };
        let contender = open_lock(&root).unwrap();
        let acquired = lock_config(&contender).unwrap();
        drop(acquired);
        drop(survivor);
        drop(contender);
        drop(root);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn config_lock_retries_interruption_but_not_busy_or_other_errors() {
        let mut calls = 0;
        let result = retry_lock_interrupted(|| {
            calls += 1;
            if calls < 3 {
                Err(rustix::io::Errno::INTR)
            } else {
                Ok(7)
            }
        });
        assert_eq!(result, Ok(7));
        assert_eq!(calls, 3);
        for error in [rustix::io::Errno::WOULDBLOCK, rustix::io::Errno::IO] {
            let mut calls = 0;
            let result: rustix::io::Result<()> = retry_lock_interrupted(|| {
                calls += 1;
                Err(error)
            });
            assert_eq!(result, Err(error));
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn write_interruptions_are_bounded_even_with_partial_progress() {
        struct InterruptedWriter(usize);
        impl Write for InterruptedWriter {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                self.0 += 1;
                if self.0.is_multiple_of(2) {
                    Ok(1)
                } else {
                    Err(std::io::ErrorKind::Interrupted.into())
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = InterruptedWriter(0);
        assert_eq!(
            write_bounded(&mut writer, &[0; 64]).unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        assert_eq!(writer.0, 31);
    }

    #[test]
    fn failed_directory_sync_after_rename_is_ambiguous_and_new_bytes_are_observable() {
        let directory =
            std::env::temp_dir().join(format!("mg-user-config-ambiguous-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = NativeUserConfigStore::new(directory.clone());
        let snapshot = store.load().unwrap();
        let preferences = NativeModelPreferences::new(
            "changed/model",
            crate::NativeReasoningEffort::default(),
            true,
        )
        .unwrap();
        assert_eq!(
            store
                .publish_preferences(&snapshot, &preferences, |_| Err(rustix::io::Errno::IO))
                .unwrap_err(),
            NativeUserConfigError::CommitAmbiguous
        );
        assert_eq!(
            store.load().unwrap().loaded().config().model_preferences(),
            preferences
        );
        assert!(!directory.join(TEMP).exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn permission_directory_sync_failure_retains_committed_overlay_as_ambiguous() {
        let directory = std::env::temp_dir().join(format!(
            "mg-user-permission-ambiguous-{}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = NativeUserConfigStore::new(directory.clone());
        let snapshot = store.load().unwrap();
        let mutation = NativeConfiguredPermissionMutation::Add {
            permission: "bash".into(),
            pattern: "saved *".into(),
        };
        assert_eq!(
            store
                .publish_permission_mutation(
                    &snapshot,
                    Path::new("/work"),
                    NativeConfiguredPermissionScope::Local,
                    &mutation,
                    |_| Err(rustix::io::Errno::IO)
                )
                .unwrap_err(),
            NativeUserConfigError::CommitAmbiguous
        );
        let current = store.load().unwrap();
        assert_eq!(current.loaded().config().schema_version(), 6);
        assert_eq!(
            current
                .loaded()
                .config()
                .permission_sources(Path::new("/work"))
                .unwrap()
                .effective()
                .rules()[0]
                .pattern(),
            "saved *"
        );
        assert!(!directory.join(TEMP).exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
