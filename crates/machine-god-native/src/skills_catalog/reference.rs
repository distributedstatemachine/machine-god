//! Durable observations, never serialized filesystem authority.
use super::{
    NativeSkillCatalog, NativeSkillCatalogError as Error, NativeSkillLinkPolicy,
    NativeSkillSelection, NativeSkillSnapshot, NativeSkillSource, Result, validate_path,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, path::Path};

/// A bounded, serializable exact skill observation. Deserialization performs no
/// discovery or I/O and grants no authority. Rebinding requires an independently
/// authorized catalog and a fresh snapshot; materialization still rechecks the
/// selected source under the catalog's retained directory descriptors.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Fields")]
pub struct NativeSkillReference(Fields);

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    version: u8,
    name: String,
    location: String,
    revision: [u8; 32],
}

impl TryFrom<Fields> for NativeSkillReference {
    type Error = Error;
    fn try_from(value: Fields) -> Result<Self> {
        if value.version != 1
            || value.name.is_empty()
            || value.name.len() > crate::skills_metadata::MAX_NATIVE_SKILL_METADATA_NAME_BYTES
            || value.name.chars().any(char::is_control)
        {
            return Err(Error::InvalidQuery);
        }
        validate_path(Path::new(&value.location), true)?;
        Ok(Self(value))
    }
}
impl fmt::Debug for NativeSkillReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeSkillReference { .. }")
    }
}
impl NativeSkillReference {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.0.name
    }
    #[must_use]
    pub fn location(&self) -> &Path {
        Path::new(&self.0.location)
    }
    /// Variable-sized owned bytes; no directory descriptor or skill body is held.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.0.name.len() + self.0.location.len()
    }
}

impl NativeSkillCatalog {
    /// Captures an inert durable reference to an exact selection from this catalog.
    /// # Errors
    /// Rejects foreign authority and invalid bounded path/metadata spellings.
    pub fn reference(&self, selection: &NativeSkillSelection) -> Result<NativeSkillReference> {
        if !self.owns_selection(selection) {
            return Err(Error::WrongAuthority);
        }
        NativeSkillReference::try_from(Fields {
            version: 1,
            name: selection.name().to_owned(),
            location: selection
                .location()
                .to_str()
                .ok_or(Error::InvalidQuery)?
                .to_owned(),
            revision: fingerprint(selection),
        })
    }

    /// Resolves only the original source revision in a supplied fresh observation.
    /// A saved path is never reopened and a matching name at another location is
    /// never substituted. This method itself performs no I/O.
    /// # Errors
    /// Rejects missing/changed observations and snapshots from another authority.
    pub fn resolve_reference(
        &self,
        snapshot: &NativeSkillSnapshot,
        reference: &NativeSkillReference,
    ) -> Result<NativeSkillSelection> {
        let selection = snapshot
            .entries()
            .iter()
            .find(|entry| {
                entry.location() == reference.location() && entry.metadata.name == reference.name()
            })
            .map(super::NativeSkillEntry::selection_ref)
            .ok_or(Error::StaleSelection)?;
        if !self.owns_selection(selection) {
            return Err(Error::WrongAuthority);
        }
        if fingerprint(selection) != reference.0.revision {
            return Err(Error::StaleSelection);
        }
        Ok(selection.clone())
    }
}

fn fingerprint(selection: &NativeSkillSelection) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"machine-god-skill-reference-v1");
    hash.update([source_tag(selection.root.source)]);
    hash.update([match selection.root.links {
        NativeSkillLinkPolicy::Reject => 0,
        NativeSkillLinkPolicy::Contained => 1,
    }]);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    for path in [&selection.root.authority_path, &selection.root.relative] {
        field(&mut hash, path.as_os_str().as_encoded_bytes());
    }
    for path in [&selection.relative, &selection.location] {
        field(&mut hash, path.as_os_str().as_encoded_bytes());
    }
    for value in selection
        .directory_identity
        .iter()
        .chain(selection.revision.iter())
    {
        hash.update(value.to_le_bytes());
    }
    hash.update(selection.prefix_digest);
    hash.update(
        u64::try_from(selection.prefix_bytes)
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    field(&mut hash, selection.metadata.name.as_bytes());
    field(&mut hash, selection.metadata.description.as_bytes());
    hash.update(
        u64::try_from(selection.metadata.body_offset)
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    hash.update([u8::from(selection.metadata.has_frontmatter)]);
    hash.finalize().into()
}
fn field(hash: &mut Sha256, value: &[u8]) {
    hash.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
    hash.update(value);
}
// Stable v1 wire tags, independent of Rust enum discriminants or Debug output.
const fn source_tag(source: NativeSkillSource) -> u8 {
    match source {
        NativeSkillSource::WorkspaceShared => 0,
        NativeSkillSource::WorkspaceOpencode => 1,
        NativeSkillSource::WorkspaceCodex => 2,
        NativeSkillSource::WorkspaceClaude => 3,
        NativeSkillSource::WorkspaceAgents => 4,
        NativeSkillSource::WorkspaceClaw => 5,
        NativeSkillSource::Managed => 6,
        NativeSkillSource::GlobalFx => 7,
        NativeSkillSource::GlobalOpencode => 8,
        NativeSkillSource::GlobalCodex => 9,
        NativeSkillSource::GlobalClaude => 10,
        NativeSkillSource::GlobalAgents => 11,
        NativeSkillSource::GlobalClaw => 12,
    }
}
