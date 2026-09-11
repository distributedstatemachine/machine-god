use super::{
    McpAuthError, Result,
    codec::{Credentials, McpAuthIdentity},
    redacted,
};
use crate::bounded_profile_file::{
    ProfileFile, ProfileFileError, ProfileFileKind, ProfileObservation, PublicationDurability,
    UpdateMode, validate_directory,
};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    path::{Path, PathBuf},
};

/// Explicit native credential-file authority, never an fx path or ambient root.
pub struct NativeMcpCredentialStore {
    file: ProfileFile,
    path: PathBuf,
}
pub(super) struct Snapshot {
    observed: ProfileObservation,
    entries: Vec<Credentials>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    credentials: Vec<Credentials>,
}
redacted!(NativeMcpCredentialStore, Snapshot);

impl NativeMcpCredentialStore {
    /// Inert admission of a selected absolute private profile directory.
    /// # Errors
    /// Rejects unsafe, relative and over-budget paths without creating files.
    pub fn new(directory: PathBuf) -> Result<Self> {
        validate_directory(&directory).map_err(map)?;
        let path = directory.join("mcp-credentials.json");
        Ok(Self {
            file: ProfileFile::new(directory, ProfileFileKind::McpCredentials),
            path,
        })
    }
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub(super) fn load(&self) -> Result<Snapshot> {
        let observed = self.file.observe().map_err(map)?;
        let entries = decode(observed.bytes())?;
        Ok(Snapshot { observed, entries })
    }
    pub(super) fn publish(
        &self,
        snapshot: &Snapshot,
        identity: &McpAuthIdentity,
        replacement: Option<&Credentials>,
    ) -> Result<PublicationDurability> {
        let mut entries = snapshot.entries.clone();
        entries.retain(|entry| &entry.identity != identity);
        if let Some(replacement) = replacement {
            replacement.validate()?;
            if &replacement.identity != identity {
                return Err(McpAuthError::Invalid);
            }
            entries.push(replacement.clone());
        }
        if entries == snapshot.entries {
            self.file
                .validate_unchanged(&snapshot.observed)
                .map_err(map)?;
            return Ok(PublicationDurability::Confirmed);
        }
        if entries.len() > 64 {
            return Err(McpAuthError::Limit);
        }
        let mut encoded = serde_json::to_vec(&Document {
            version: 1,
            credentials: entries,
        })
        .map_err(|_| McpAuthError::Invalid)?;
        let result = (|| {
            if encoded.len() > 1024 * 1024 {
                return Err(McpAuthError::Limit);
            }
            let update = self
                .file
                .begin(&snapshot.observed, UpdateMode::CompareAndSwap)
                .map_err(map)?;
            update
                .publish(&encoded, |root| rustix::fs::fsync(root))
                .map_err(map)
        })();
        encoded.fill(0);
        result
    }
}
impl Snapshot {
    pub(super) fn get(&self, identity: &McpAuthIdentity) -> Option<&Credentials> {
        self.entries
            .iter()
            .find(|entry| &entry.identity == identity)
    }
}
fn decode(bytes: Option<&[u8]>) -> Result<Vec<Credentials>> {
    let Some(bytes) = bytes else {
        return Ok(Vec::new());
    };
    if bytes.len() > 1024 * 1024 {
        return Err(McpAuthError::Limit);
    }
    let value = machine_god_core::json::from_slice(bytes).map_err(|_| McpAuthError::Invalid)?;
    let document: Document = serde_json::from_value(value).map_err(|_| McpAuthError::Invalid)?;
    if document.version != 1 || document.credentials.len() > 64 {
        return Err(McpAuthError::Invalid);
    }
    for (index, entry) in document.credentials.iter().enumerate() {
        entry.validate()?;
        if document.credentials[..index]
            .iter()
            .any(|other| other.identity == entry.identity)
        {
            return Err(McpAuthError::Invalid);
        }
    }
    Ok(document.credentials)
}
fn map(error: ProfileFileError) -> McpAuthError {
    match error {
        ProfileFileError::Conflict => McpAuthError::Conflict,
        ProfileFileError::Busy => McpAuthError::Busy,
        ProfileFileError::TooLarge => McpAuthError::Limit,
        ProfileFileError::UnsafePath
        | ProfileFileError::Persistence
        | ProfileFileError::Unreadable => McpAuthError::Persistence,
    }
}
