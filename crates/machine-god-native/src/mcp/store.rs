//! Explicit private profile MCP persistence, independent of native settings.
//!
//! Publication uses the shared descriptor-bound profile-file transaction. A
//! snapshot binds its originating store, ancestor/root identities, exact opened
//! data-file incarnation, metadata revision and bytes. Equal-byte replacement by
//! another inode is stale. This is cooperative writer CAS, not an atomic defense
//! against arbitrary writers that ignore the lock and race the final rename.

use std::fmt;
use std::path::{Path, PathBuf};

use rustix::fd::OwnedFd;

use super::config::{MAX_SERVER_NAME_BYTES, McpConfig, McpConfigError, McpServerConfig};
use crate::bounded_profile_file::{
    ProfileFile, ProfileFileError, ProfileFileKind, ProfileObservation, PublicationDurability,
    UpdateMode, validate_directory,
};

#[cfg(test)]
mod tests;

/// Redacted profile configuration persistence error. These errors precede rename;
/// post-rename uncertainty is returned in a committed receipt instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeMcpConfigStoreError {
    /// The supplied directory or a filesystem entry is unsafe or unavailable.
    UnsafePath,
    /// Another cooperative writer owns the nonblocking profile lock.
    Busy,
    /// The snapshot is foreign, stale or no longer identifies the observed source.
    Conflict,
    /// Observed or proposed configuration failed bounded codec admission.
    InvalidConfig(McpConfigError),
    /// Publication or bounded observation failed before replacing the data file.
    Persistence,
}

impl fmt::Display for NativeMcpConfigStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsafePath => "MCP configuration path is unsafe",
            Self::Busy => "MCP configuration writer is busy",
            Self::Conflict => "MCP configuration observation conflicts",
            Self::InvalidConfig(_) => "MCP configuration is invalid",
            Self::Persistence => "MCP configuration persistence failed",
        })
    }
}
impl std::error::Error for NativeMcpConfigStoreError {}
impl From<ProfileFileError> for NativeMcpConfigStoreError {
    fn from(error: ProfileFileError) -> Self {
        match error {
            ProfileFileError::UnsafePath => Self::UnsafePath,
            ProfileFileError::Busy => Self::Busy,
            ProfileFileError::Conflict => Self::Conflict,
            ProfileFileError::TooLarge => Self::InvalidConfig(McpConfigError::Limit),
            ProfileFileError::Persistence | ProfileFileError::Unreadable => Self::Persistence,
        }
    }
}
impl From<McpConfigError> for NativeMcpConfigStoreError {
    fn from(error: McpConfigError) -> Self {
        Self::InvalidConfig(error)
    }
}

/// Explicit authority over `mcp.json` in one native profile directory.
pub struct NativeMcpConfigStore {
    file: ProfileFile,
    path: PathBuf,
}

/// Exact read-only observation; does not grant runtime, credential or tool access.
pub struct NativeMcpConfigSnapshot {
    config: McpConfig,
    observed: ProfileObservation,
}

/// One explicit profile change; replacement must be selected deliberately.
#[derive(Clone, PartialEq, Eq)]
pub enum McpConfigMutation {
    /// Adds a new alias; an existing alias is an error even if values are equal.
    Insert(McpServerConfig),
    /// Replaces an exact alias, preserving its position; absent aliases append.
    Replace(McpServerConfig),
    /// Removes an exact case-sensitive alias. An unknown valid alias is unchanged.
    Remove(Box<str>),
}

/// Filesystem publication durability, independent of runtime reload success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpConfigCommitDurability {
    /// The publication and its directory durability were confirmed, or no write
    /// was necessary. Neither case reserves the file against later changes.
    Confirmed,
    /// Rename occurred but durability or subsequent source/link checks failed.
    /// Reload and reconcile the receipt; do not automatically retry the mutation.
    Ambiguous,
}

/// Previous and intended configurations, not a fresh post-commit observation.
pub struct NativeMcpConfigCommit {
    before: McpConfig,
    intended: McpConfig,
    changed: bool,
    durability: McpConfigCommitDurability,
}

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($ty), " { <redacted> }"))
            }
        }
    )+};
}
redacted_debug!(
    NativeMcpConfigStore,
    NativeMcpConfigSnapshot,
    McpConfigMutation,
    NativeMcpConfigCommit
);

impl NativeMcpConfigStore {
    /// Admits and compacts one explicit absolute profile directory without effects.
    /// No environment lookup, directory creation or configuration loading occurs.
    ///
    /// # Errors
    /// Rejects directories exceeding 4096 raw bytes/64 components, root-only or
    /// relative paths, NUL bytes and lexical parent components.
    pub fn new(directory: PathBuf) -> Result<Self, NativeMcpConfigStoreError> {
        validate_directory(&directory)?;
        let directory = directory.into_boxed_path().into_path_buf();
        let path = directory.join("mcp.json").into_boxed_path().into_path_buf();
        Ok(Self {
            file: ProfileFile::new(directory, ProfileFileKind::Mcp),
            path,
        })
    }

    /// Returns the explicit native profile filename, without opening it.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads one bounded private configuration without creating any artifacts.
    /// Missing files/directories yield an empty configuration and retain missing
    /// ancestor observations for later publication.
    ///
    /// # Errors
    /// Rejects invalid/over-budget data, nonprivate or unsafe entries, unstable
    /// reads and unavailable retained ancestor authority.
    pub fn load(&self) -> Result<NativeMcpConfigSnapshot, NativeMcpConfigStoreError> {
        let observed = self.file.observe()?;
        let config = decode(observed.bytes())?;
        Ok(NativeMcpConfigSnapshot { config, observed })
    }

    /// Applies one exact-snapshot mutation. This borrowed future is inert until
    /// polled, then executes one input-bounded synchronous owned transaction. It
    /// starts no detached worker; no-op edits create no directories, locks or temps.
    /// Filesystem syscall latency is not a hard wall-clock deadline guarantee.
    ///
    /// # Errors
    /// Candidate errors, stale/foreign observations, lock contention and failures
    /// before rename leave prior data authoritative. Empty created directories may
    /// remain. After rename, uncertainty is an `Ambiguous` receipt, not an error.
    #[allow(clippy::unused_async)] // Inert borrowed future, with no detached writer.
    pub async fn apply(
        &self,
        snapshot: &NativeMcpConfigSnapshot,
        mutation: &McpConfigMutation,
    ) -> Result<NativeMcpConfigCommit, NativeMcpConfigStoreError> {
        self.publish(snapshot, mutation, |root| rustix::fs::fsync(root))
    }

    fn publish(
        &self,
        snapshot: &NativeMcpConfigSnapshot,
        mutation: &McpConfigMutation,
        sync_directory: impl FnOnce(&OwnedFd) -> rustix::io::Result<()>,
    ) -> Result<NativeMcpConfigCommit, NativeMcpConfigStoreError> {
        self.file.validate_identity(&snapshot.observed)?;
        if let McpConfigMutation::Remove(name) = mutation {
            if name.len() > MAX_SERVER_NAME_BYTES {
                return Err(McpConfigError::Limit.into());
            }
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(McpConfigError::Invalid.into());
            }
        }
        let mut intended = snapshot.config.clone();
        match mutation {
            McpConfigMutation::Insert(server) => intended.insert(server.clone())?,
            McpConfigMutation::Replace(server) => {
                intended.replace(server.clone())?;
            }
            McpConfigMutation::Remove(name) => {
                intended.remove(name);
            }
        }
        let changed = intended != snapshot.config;
        let durability = if changed {
            // Complete codec/canonical-output bounds precede all creation effects.
            let encoded = intended.encode()?;
            let transaction = self
                .file
                .begin(&snapshot.observed, UpdateMode::CompareAndSwap)?;
            decode(transaction.current_bytes())?;
            match transaction.publish(&encoded, sync_directory)? {
                PublicationDurability::Confirmed => McpConfigCommitDurability::Confirmed,
                PublicationDurability::Ambiguous => McpConfigCommitDurability::Ambiguous,
            }
        } else {
            self.file.validate_unchanged(&snapshot.observed)?;
            McpConfigCommitDurability::Confirmed
        };
        Ok(NativeMcpConfigCommit {
            before: snapshot.config.clone(),
            intended,
            changed,
            durability,
        })
    }
}

fn decode(bytes: Option<&[u8]>) -> Result<McpConfig, NativeMcpConfigStoreError> {
    bytes.map_or_else(
        || Ok(McpConfig::new()),
        |bytes| McpConfig::decode(bytes).map_err(Into::into),
    )
}

impl NativeMcpConfigSnapshot {
    /// Returns the admitted inert configuration from this exact observation.
    #[must_use]
    pub fn config(&self) -> &McpConfig {
        &self.config
    }
}

impl NativeMcpConfigCommit {
    /// Returns the exact admitted configuration before this mutation.
    #[must_use]
    pub fn before(&self) -> &McpConfig {
        &self.before
    }
    /// Returns the intended configuration; ambiguous publication requires reload
    /// and reconciliation before deciding whether it remains authoritative.
    #[must_use]
    pub fn intended(&self) -> &McpConfig {
        &self.intended
    }
    /// Whether this operation performed replacement, independent of durability.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.changed
    }
    /// Returns the publication durability, not a runtime activation result.
    #[must_use]
    pub fn durability(&self) -> McpConfigCommitDurability {
        self.durability
    }
}
