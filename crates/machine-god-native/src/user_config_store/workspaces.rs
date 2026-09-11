//! Latest-state workspace mutations; the existing snapshot-CAS routes are separate.

#[cfg(test)]
use super::{DATA, LOCK, TEMP};
use super::{NativeUserConfigError, NativeUserConfigStore, decode};
use crate::LoadedNativeConfig;
use crate::bounded_profile_file::{PublicationDurability, UpdateMode};
use crate::config::{NativeSavedWorkspaceDirectory, NativeWorkspaceDirectoryMutation};
use rustix::fd::OwnedFd;

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
        let transaction = self
            .file
            .begin(&snapshot.observed, UpdateMode::MergeLatest)?;
        let latest = decode(transaction.current_bytes())?;
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
        let durability = match transaction.publish(&encoded, sync_directory)? {
            PublicationDurability::Confirmed => NativeWorkspaceCommitDurability::Confirmed,
            PublicationDurability::Ambiguous => NativeWorkspaceCommitDurability::Ambiguous,
        };
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

#[cfg(test)]
mod tests;
