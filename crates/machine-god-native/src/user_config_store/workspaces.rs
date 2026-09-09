//! Latest-state workspace mutations; the existing snapshot-CAS routes are separate.

use std::fs::File;

use rustix::fd::OwnedFd;
use rustix::fs::{Mode, OFlags};

use super::{
    DATA, LOCK, NativeUserConfigError, NativeUserConfigStore, TEMP, decode, lock_config, open_lock,
    open_root, read_current, remove_owned_temp, same_file, validate_link, validate_private,
    write_bounded,
};
use crate::LoadedNativeConfig;
use crate::config::{NativeSavedWorkspaceDirectory, NativeWorkspaceDirectoryMutation};

/// Ephemeral identity evidence from the accepted retained workspace authority.
/// It applies only while the entire saved record remains exactly unchanged.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceDirectoryAlias {
    record: NativeSavedWorkspaceDirectory,
    canonical_identity: Vec<u8>,
}

impl WorkspaceDirectoryAlias {
    pub(crate) fn new(
        record: NativeSavedWorkspaceDirectory,
        canonical_identity: &[u8],
    ) -> Result<Self, NativeUserConfigError> {
        NativeSavedWorkspaceDirectory::new(canonical_identity, canonical_identity, true)
            .map_err(NativeUserConfigError::InvalidConfig)?;
        if record.identity_canonical() && record.identity_bytes() != canonical_identity {
            return Err(NativeUserConfigError::Conflict);
        }
        Ok(Self {
            record,
            canonical_identity: canonical_identity.to_vec(),
        })
    }

    pub(crate) fn identity_for(&self, record: &NativeSavedWorkspaceDirectory) -> Option<&[u8]> {
        (&self.record == record).then_some(self.canonical_identity.as_slice())
    }
}

/// Whether the published candidate's directory durability was confirmed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeWorkspaceCommitDurability {
    Confirmed,
    /// Replacement happened, but the owner must reload and reconcile before use.
    Ambiguous,
}

/// Saved sets around this transaction, independent of runtime root installation.
/// `loaded` is the intended merged candidate, not a fresh post-commit observation.
/// In particular, `Ambiguous` requires the owner to reload and reconcile it.
/// An unchanged result is an observation, never a reservation against later edits.
#[derive(Debug)]
pub struct NativeUserWorkspaceCommit {
    pub before: Vec<NativeSavedWorkspaceDirectory>,
    pub after: Vec<NativeSavedWorkspaceDirectory>,
    pub loaded: LoadedNativeConfig,
    pub changed: bool,
    pub durability: NativeWorkspaceCommitDurability,
}

impl NativeUserConfigStore {
    /// Mutates only the selected primary's saved directories in the latest config.
    /// Unlike model/permission edits, this API accepts no stale snapshot CAS token.
    /// The future is inert until polled and owns one bounded synchronous transaction;
    /// it starts no detached worker and lock contention returns immediately.
    /// Initial no-ops create nothing. If a changed preflight becomes a no-op under
    /// the lock, acquiring that lock may have created it, but no temp or rewrite occurs.
    /// `launch_identities` contains staged surviving launch roots. Its union with
    /// the latest candidate's saved identities must fit the shared sixteen-root cap.
    /// # Errors
    /// Rejects invalid requests before effects, unsafe/replaced entries, contention,
    /// invalid latest config and pre-replacement publication failures.
    /// Post-replacement uncertainty returns an explicitly ambiguous receipt.
    #[allow(clippy::unused_async)] // Inert borrowed future; no detached blocking task.
    pub async fn apply_workspace_directory_mutation(
        &self,
        primary: &[u8],
        mutation: &NativeWorkspaceDirectoryMutation,
        launch_identities: &[Vec<u8>],
    ) -> Result<NativeUserWorkspaceCommit, NativeUserConfigError> {
        self.publish_workspace_mutation(
            primary,
            mutation,
            || {},
            |root| rustix::fs::fsync(root),
            launch_identities,
        )
    }

    fn publish_workspace_mutation(
        &self,
        primary: &[u8],
        mutation: &NativeWorkspaceDirectoryMutation,
        before_lock: impl FnOnce(),
        sync_directory: impl FnOnce(&OwnedFd) -> rustix::io::Result<()>,
        launch_identities: &[Vec<u8>],
    ) -> Result<NativeUserWorkspaceCommit, NativeUserConfigError> {
        self.publish_workspace_mutation_observed(
            primary,
            mutation,
            before_lock,
            sync_directory,
            launch_identities,
            &[],
        )
    }

    #[allow(clippy::unused_async)] // Inert caller-owned future, like the public route.
    pub(crate) async fn apply_workspace_directory_mutation_observed(
        &self,
        primary: &[u8],
        mutation: &NativeWorkspaceDirectoryMutation,
        launch_identities: &[Vec<u8>],
        aliases: &[WorkspaceDirectoryAlias],
    ) -> Result<NativeUserWorkspaceCommit, NativeUserConfigError> {
        self.publish_workspace_mutation_observed(
            primary,
            mutation,
            || {},
            |root| rustix::fs::fsync(root),
            launch_identities,
            aliases,
        )
    }

    fn publish_workspace_mutation_observed(
        &self,
        primary: &[u8],
        mutation: &NativeWorkspaceDirectoryMutation,
        before_lock: impl FnOnce(),
        sync_directory: impl FnOnce(&OwnedFd) -> rustix::io::Result<()>,
        launch_identities: &[Vec<u8>],
        aliases: &[WorkspaceDirectoryAlias],
    ) -> Result<NativeUserWorkspaceCommit, NativeUserConfigError> {
        mutation
            .validate(primary)
            .map_err(NativeUserConfigError::InvalidConfig)?;
        crate::config::validate_workspace_directory_launch(primary, launch_identities)
            .map_err(NativeUserConfigError::InvalidConfig)?;
        validate_aliases(primary, aliases)?;
        let snapshot = self.load()?;
        let (candidate, changed) = snapshot
            .loaded
            .config()
            .with_workspace_directory_mutation(primary, mutation)
            .map_err(NativeUserConfigError::InvalidConfig)?;
        validate_capacity(&candidate, primary, launch_identities, aliases)?;
        if !changed {
            self.validate_unchanged(&snapshot)?;
            return unchanged(snapshot.loaded, primary);
        }
        before_lock();
        let name = self.component()?;
        let observed = open_root(&snapshot.parent, name)?;
        let root = match (&snapshot.root, observed) {
            (Some(expected), Some(actual)) if same_file(expected, &actual)? => actual,
            (None, Some(actual)) => actual,
            (None, None) => {
                match rustix::fs::mkdirat(&snapshot.parent, name, Mode::from_raw_mode(0o700)) {
                    Ok(()) => {}
                    Err(error) if error == rustix::io::Errno::EXIST => {}
                    Err(_) => return Err(NativeUserConfigError::Persistence),
                }
                let root =
                    open_root(&snapshot.parent, name)?.ok_or(NativeUserConfigError::Persistence)?;
                rustix::fs::fsync(&snapshot.parent)
                    .map_err(|_| NativeUserConfigError::Persistence)?;
                root
            }
            _ => return Err(NativeUserConfigError::Conflict),
        };
        let lock = open_lock(&root)?;
        let _guard = lock_config(&lock)?;
        validate_link(&snapshot.parent, name, &root)?;
        validate_link(&root, LOCK, &lock)?;
        let bytes = read_current(&root)?;
        let latest = decode(bytes.as_deref())?;
        let (candidate, changed) = latest
            .config()
            .with_workspace_directory_mutation(primary, mutation)
            .map_err(NativeUserConfigError::InvalidConfig)?;
        validate_capacity(&candidate, primary, launch_identities, aliases)?;
        if !changed {
            return unchanged(latest, primary);
        }
        let before = latest
            .config()
            .saved_workspace_directories(primary)
            .map_err(NativeUserConfigError::InvalidConfig)?
            .to_vec();
        let after = candidate
            .saved_workspace_directories(primary)
            .map_err(NativeUserConfigError::InvalidConfig)?
            .to_vec();
        let encoded = candidate
            .serialize_current()
            .map_err(NativeUserConfigError::InvalidConfig)?;
        let publication = WorkspacePublication {
            parent: &snapshot.parent,
            name,
            root: &root,
            lock: &lock,
            previous: bytes.as_deref(),
        };
        let durability = publication.publish(&encoded, sync_directory)?;
        Ok(NativeUserWorkspaceCommit {
            before,
            after,
            loaded: LoadedNativeConfig::from_file(candidate),
            changed: true,
            durability,
        })
    }
}

fn validate_aliases(
    primary: &[u8],
    aliases: &[WorkspaceDirectoryAlias],
) -> Result<(), NativeUserConfigError> {
    if aliases.len() > 16 {
        return Err(NativeUserConfigError::Conflict);
    }
    let mut identities = std::collections::BTreeSet::new();
    let mut sources = std::collections::BTreeSet::new();
    let mut canonical_identities = std::collections::BTreeSet::new();
    for alias in aliases {
        NativeWorkspaceDirectoryMutation::Add(alias.record.clone())
            .validate(primary)
            .map_err(NativeUserConfigError::InvalidConfig)?;
        crate::config::validate_workspace_directory_launch(
            primary,
            std::slice::from_ref(&alias.canonical_identity),
        )
        .map_err(NativeUserConfigError::InvalidConfig)?;
        if !identities.insert(alias.record.identity_bytes())
            || !sources.insert(alias.record.source_bytes())
            || !canonical_identities.insert(alias.canonical_identity.as_slice())
            || (alias.record.identity_canonical()
                && alias.record.identity_bytes() != alias.canonical_identity)
        {
            return Err(NativeUserConfigError::Conflict);
        }
    }
    Ok(())
}

fn validate_capacity(
    config: &crate::NativeConfig,
    primary: &[u8],
    launch: &[Vec<u8>],
    aliases: &[WorkspaceDirectoryAlias],
) -> Result<(), NativeUserConfigError> {
    let result = if aliases.is_empty() {
        config.validate_workspace_directory_capacity(primary, launch)
    } else {
        config.validate_workspace_directory_capacity_with_identity(primary, launch, |record| {
            aliases
                .iter()
                .find_map(|alias| alias.identity_for(record))
                .unwrap_or_else(|| record.identity_bytes())
        })
    };
    result.map_err(NativeUserConfigError::InvalidConfig)
}

fn unchanged(
    loaded: LoadedNativeConfig,
    primary: &[u8],
) -> Result<NativeUserWorkspaceCommit, NativeUserConfigError> {
    let saved = loaded
        .config()
        .saved_workspace_directories(primary)
        .map_err(NativeUserConfigError::InvalidConfig)?
        .to_vec();
    Ok(NativeUserWorkspaceCommit {
        before: saved.clone(),
        after: saved,
        loaded,
        changed: false,
        durability: NativeWorkspaceCommitDurability::Confirmed,
    })
}

struct WorkspacePublication<'a> {
    parent: &'a OwnedFd,
    name: &'a std::ffi::OsStr,
    root: &'a OwnedFd,
    lock: &'a OwnedFd,
    previous: Option<&'a [u8]>,
}

impl WorkspacePublication<'_> {
    fn publish(
        &self,
        encoded: &[u8],
        sync_directory: impl FnOnce(&OwnedFd) -> rustix::io::Result<()>,
    ) -> Result<NativeWorkspaceCommitDurability, NativeUserConfigError> {
        let temp = rustix::fs::openat(
            self.root,
            TEMP,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| NativeUserConfigError::Persistence)?;
        let mut temp = File::from(temp);
        if rustix::fs::fchmod(&temp, Mode::RUSR | Mode::WUSR).is_err()
            || validate_private(&temp, false).is_err()
            || write_bounded(&mut temp, encoded).is_err()
            || temp.sync_all().is_err()
        {
            remove_owned_temp(self.root, &temp);
            return Err(NativeUserConfigError::Persistence);
        }
        let checked = (|| {
            validate_link(self.parent, self.name, self.root)?;
            validate_link(self.root, LOCK, self.lock)?;
            validate_link(self.root, TEMP, &temp)?;
            if read_current(self.root)?.as_deref() != self.previous {
                return Err(NativeUserConfigError::Conflict);
            }
            Ok(())
        })();
        if let Err(error) = checked {
            remove_owned_temp(self.root, &temp);
            return Err(error);
        }
        if rustix::fs::renameat(self.root, TEMP, self.root, DATA).is_err() {
            remove_owned_temp(self.root, &temp);
            return Err(NativeUserConfigError::Persistence);
        }
        Ok(
            if sync_directory(self.root).is_err()
                || validate_link(self.parent, self.name, self.root).is_err()
            {
                NativeWorkspaceCommitDurability::Ambiguous
            } else {
                NativeWorkspaceCommitDurability::Confirmed
            },
        )
    }
}

#[cfg(test)]
mod tests;
