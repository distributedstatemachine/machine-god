//! Pure lossless saved-directory values. Parsing never probes the filesystem.

use std::{collections::BTreeSet, fmt};

use serde::{Deserialize, Serialize};

use super::{NativeConfig, NativeConfigError, NativeConfigErrorKind};

const MAX_PATH_BYTES: usize = 4096;
const MAX_DIRECTORIES: usize = 16;

/// A saved source spelling and its retained identity, not an execution grant.
/// Both paths retain raw Unix bytes independently of their UTF-8 validity.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeSavedWorkspaceDirectory {
    source: Vec<u8>,
    identity: Vec<u8>,
    identity_canonical: bool,
}

impl NativeSavedWorkspaceDirectory {
    /// Validates normalized absolute paths without resolving or opening either.
    /// # Errors
    /// Rejects NUL, noncanonical spelling and paths beyond 4,096 bytes.
    pub fn new(
        source: &[u8],
        identity: &[u8],
        identity_canonical: bool,
    ) -> Result<Self, NativeConfigError> {
        validate_path(source)?;
        validate_path(identity)?;
        Ok(Self {
            source: source.to_vec(),
            identity: identity.to_vec(),
            identity_canonical,
        })
    }

    #[must_use]
    pub fn source_bytes(&self) -> &[u8] {
        &self.source
    }

    #[must_use]
    pub fn identity_bytes(&self) -> &[u8] {
        &self.identity
    }

    #[must_use]
    pub const fn identity_canonical(&self) -> bool {
        self.identity_canonical
    }
}

impl fmt::Debug for NativeSavedWorkspaceDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeSavedWorkspaceDirectory { .. }")
    }
}

impl Serialize for NativeSavedWorkspaceDirectory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct View {
            source_hex: String,
            identity_hex: String,
            identity_canonical: bool,
        }
        View {
            source_hex: encode_hex(&self.source),
            identity_hex: encode_hex(&self.identity),
            identity_canonical: self.identity_canonical,
        }
        .serialize(serializer)
    }
}

/// A selected-workspace storage edit, independent of tool and path authority.
#[derive(Clone, Eq, PartialEq)]
pub enum NativeWorkspaceDirectoryMutation {
    Add(NativeSavedWorkspaceDirectory),
    Remove(Vec<u8>),
    Clear,
}

impl fmt::Debug for NativeWorkspaceDirectoryMutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeWorkspaceDirectoryMutation { .. }")
    }
}

impl NativeWorkspaceDirectoryMutation {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn validate(&self, primary: &[u8]) -> Result<(), NativeConfigError> {
        validate_path(primary)?;
        match self {
            Self::Add(record) => validate_record(primary, record),
            Self::Remove(identity) => {
                validate_path(identity)?;
                if identity == primary {
                    return Err(invalid());
                }
                Ok(())
            }
            Self::Clear => Ok(()),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub(super) struct WorkspaceDirectories {
    workspace_hex: String,
    additional_directories: Vec<NativeSavedWorkspaceDirectory>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireWorkspaceDirectories {
    workspace_hex: String,
    additional_directories: Vec<WireSavedDirectory>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSavedDirectory {
    source_hex: String,
    identity_hex: String,
    identity_canonical: bool,
}

fn invalid() -> NativeConfigError {
    NativeConfigError::new(NativeConfigErrorKind::InvalidFormat)
}

fn validate_path(bytes: &[u8]) -> Result<(), NativeConfigError> {
    if bytes.len() > MAX_PATH_BYTES {
        return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
    }
    if !bytes.starts_with(b"/")
        || bytes.contains(&0)
        || (bytes != b"/"
            && bytes[1..]
                .split(|byte| *byte == b'/')
                .any(|part| part.is_empty() || part == b"." || part == b".."))
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_record(
    primary: &[u8],
    record: &NativeSavedWorkspaceDirectory,
) -> Result<(), NativeConfigError> {
    if record.identity == primary || record.source == primary {
        return Err(invalid());
    }
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 15)]));
    }
    output
}

fn decode_hex(encoded: &str) -> Result<Vec<u8>, NativeConfigError> {
    if encoded.len() > MAX_PATH_BYTES * 2 {
        return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
    }
    if !encoded.len().is_multiple_of(2) {
        return Err(invalid());
    }
    let nibble = |byte| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(invalid()),
    };
    let bytes = encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok(nibble(pair[0])? * 16 + nibble(pair[1])?))
        .collect::<Result<Vec<_>, NativeConfigError>>()?;
    validate_path(&bytes)?;
    Ok(bytes)
}

pub(super) fn decode_workspaces(
    wire: Vec<WireWorkspaceDirectories>,
) -> Result<Vec<WorkspaceDirectories>, NativeConfigError> {
    let mut primaries = BTreeSet::new();
    wire.into_iter()
        .map(|workspace| {
            let primary = decode_hex(&workspace.workspace_hex)?;
            if !primaries.insert(workspace.workspace_hex.clone()) {
                return Err(invalid());
            }
            if workspace.additional_directories.len() > MAX_DIRECTORIES {
                return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
            }
            let mut sources = BTreeSet::new();
            let mut identities = BTreeSet::new();
            let directories = workspace
                .additional_directories
                .into_iter()
                .map(|wire| {
                    let record = NativeSavedWorkspaceDirectory {
                        source: decode_hex(&wire.source_hex)?,
                        identity: decode_hex(&wire.identity_hex)?,
                        identity_canonical: wire.identity_canonical,
                    };
                    validate_record(&primary, &record)?;
                    if !sources.insert(record.source.clone())
                        || !identities.insert(record.identity.clone())
                    {
                        return Err(invalid());
                    }
                    Ok(record)
                })
                .collect::<Result<Vec<_>, NativeConfigError>>()?;
            Ok(WorkspaceDirectories {
                workspace_hex: workspace.workspace_hex,
                additional_directories: directories,
            })
        })
        .collect()
}

impl NativeConfig {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn validate_workspace_directory_capacity(
        &self,
        primary: &[u8],
        launch_identities: &[Vec<u8>],
    ) -> Result<(), NativeConfigError> {
        self.validate_workspace_directory_capacity_with_identity(
            primary,
            launch_identities,
            NativeSavedWorkspaceDirectory::identity_bytes,
        )
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn validate_workspace_directory_capacity_with_identity<'a>(
        &'a self,
        primary: &[u8],
        launch_identities: &'a [Vec<u8>],
        identity: impl Fn(&'a NativeSavedWorkspaceDirectory) -> &'a [u8],
    ) -> Result<(), NativeConfigError> {
        validate_launch(primary, launch_identities)?;
        let mut identities = launch_identities
            .iter()
            .map(Vec::as_slice)
            .collect::<BTreeSet<_>>();
        for directory in self.saved_workspace_directories(primary)? {
            identities.insert(identity(directory));
        }
        if identities.len() > MAX_DIRECTORIES {
            return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
        }
        Ok(())
    }

    /// Returns saved sources for one exact primary; no paths are opened.
    /// Empty includes absent entries. This does not include launch-only roots.
    /// # Errors
    /// Rejects an invalid or oversized normalized absolute Unix primary label.
    pub fn saved_workspace_directories(
        &self,
        primary: &[u8],
    ) -> Result<&[NativeSavedWorkspaceDirectory], NativeConfigError> {
        validate_path(primary)?;
        let key = encode_hex(primary);
        Ok(self
            .workspace_directories
            .iter()
            .find(|workspace| workspace.workspace_hex == key)
            .map_or(&[], |workspace| workspace.additional_directories.as_slice()))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn with_workspace_directory_mutation(
        &self,
        primary: &[u8],
        mutation: &NativeWorkspaceDirectoryMutation,
    ) -> Result<(Self, bool), NativeConfigError> {
        mutation.validate(primary)?;
        let key = encode_hex(primary);
        let index = self
            .workspace_directories
            .iter()
            .position(|workspace| workspace.workspace_hex == key);
        let before = self.saved_workspace_directories(primary)?;
        let mut after = before.to_vec();
        match mutation {
            NativeWorkspaceDirectoryMutation::Add(record) => {
                if after.iter().any(|item| item.identity == record.identity) {
                    return Ok((self.clone(), false));
                }
                if after.iter().any(|item| item.source == record.source) {
                    return Err(invalid());
                }
                if after.len() == MAX_DIRECTORIES {
                    return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
                }
                after.push(record.clone());
            }
            NativeWorkspaceDirectoryMutation::Remove(identity) => {
                after.retain(|item| item.identity != *identity);
            }
            NativeWorkspaceDirectoryMutation::Clear => after.clear(),
        }
        if after == before {
            return Ok((self.clone(), false));
        }
        let mut candidate = self.clone();
        match index {
            Some(index) if after.is_empty() => {
                candidate.workspace_directories.remove(index);
            }
            Some(index) => candidate.workspace_directories[index].additional_directories = after,
            None => candidate.workspace_directories.push(WorkspaceDirectories {
                workspace_hex: key,
                additional_directories: after,
            }),
        }
        candidate.schema_version = super::CONFIG_SCHEMA_VERSION;
        candidate.serialize_current()?;
        Ok((candidate, true))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn validate_launch(
    primary: &[u8],
    identities: &[Vec<u8>],
) -> Result<(), NativeConfigError> {
    validate_path(primary)?;
    if identities.len() > MAX_DIRECTORIES {
        return Err(NativeConfigError::new(NativeConfigErrorKind::TooLarge));
    }
    let mut distinct = BTreeSet::new();
    for identity in identities {
        validate_path(identity)?;
        if identity == primary || !distinct.insert(identity) {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[path = "config_workspaces/tests.rs"]
mod tests;
