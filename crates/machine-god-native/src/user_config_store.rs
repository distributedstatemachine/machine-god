//! Explicit, descriptor-bound publication of native user defaults.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::bounded_profile_file::{
    ProfileFile, ProfileFileError, ProfileFileKind, ProfileObservation, PublicationDurability,
    UpdateMode,
};
use rustix::fd::OwnedFd;

use crate::config::{
    NativeConfiguredPermissionMutation, NativeConfiguredPermissionMutationOutcome,
    NativeConfiguredPermissionScope, parse_config_bytes,
};
use crate::{LoadedNativeConfig, NativeConfigError, NativeModelPreferences};

#[cfg(test)]
mod parents;
mod workspaces;
pub(crate) use workspaces::WorkspaceDirectoryAlias;
pub use workspaces::{NativeUserWorkspaceCommit, NativeWorkspaceCommitDurability};

#[cfg(test)]
const DATA: &str = "config.json";
#[cfg(test)]
const LOCK: &str = ".config.lock";
#[cfg(test)]
const TEMP: &str = ".config.tmp";

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
    file: ProfileFile,
}

impl fmt::Debug for NativeUserConfigStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeUserConfigStore")
            .finish_non_exhaustive()
    }
}

/// Read-only observation and exact-byte compare-and-swap token.
///
/// Retains the nearest existing ancestor, unresolved parent components and any
/// existing root descriptor. Bound to its originating store instance.
pub struct NativeUserConfigSnapshot {
    loaded: LoadedNativeConfig,
    observed: ProfileObservation,
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
    /// Grants one bounded configuration namespace, including missing ancestors.
    /// No environment is read and no filesystem effects occur here.
    #[must_use]
    pub fn new(directory: PathBuf) -> Self {
        Self {
            file: ProfileFile::new(directory, ProfileFileKind::Settings),
        }
    }

    /// Reads a bounded snapshot without creating files, directories or locks.
    ///
    /// # Errors
    /// Rejects unsafe roots, invalid configurations and unavailable parent authority.
    pub fn load(&self) -> Result<NativeUserConfigSnapshot, NativeUserConfigError> {
        let observed = self.file.observe()?;
        let loaded = decode(observed.bytes())?;
        Ok(NativeUserConfigSnapshot { loaded, observed })
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
        self.file.validate_identity(&snapshot.observed)?;
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
        self.file
            .validate_unchanged(&snapshot.observed)
            .map_err(Into::into)
    }

    fn publish_config(
        &self,
        snapshot: &NativeUserConfigSnapshot,
        config: crate::NativeConfig,
        sync_directory: impl FnOnce(&OwnedFd) -> rustix::io::Result<()>,
    ) -> Result<LoadedNativeConfig, NativeUserConfigError> {
        self.file.validate_identity(&snapshot.observed)?;
        let encoded = config
            .serialize_current()
            .map_err(NativeUserConfigError::InvalidConfig)?;
        let transaction = self
            .file
            .begin(&snapshot.observed, UpdateMode::CompareAndSwap)?;
        // Keep settings' error precedence: malformed latest data is not a CAS retry.
        decode(transaction.current_bytes())?;
        if transaction.publish(&encoded, sync_directory)? == PublicationDurability::Ambiguous {
            return Err(NativeUserConfigError::CommitAmbiguous);
        }
        Ok(LoadedNativeConfig::from_file(config))
    }
}

impl From<ProfileFileError> for NativeUserConfigError {
    fn from(error: ProfileFileError) -> Self {
        match error {
            ProfileFileError::UnsafePath => Self::UnsafePath,
            ProfileFileError::Busy => Self::Busy,
            ProfileFileError::Conflict => Self::Conflict,
            ProfileFileError::Persistence => Self::Persistence,
            ProfileFileError::TooLarge => Self::InvalidConfig(NativeConfigError::new(
                crate::NativeConfigErrorKind::TooLarge,
            )),
            ProfileFileError::Unreadable => Self::InvalidConfig(NativeConfigError::new(
                crate::NativeConfigErrorKind::Unreadable,
            )),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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
        assert_eq!(current.loaded().config().schema_version(), 7);
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
