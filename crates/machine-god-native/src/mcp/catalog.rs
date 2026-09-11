//! Immutable descriptor admission and deterministic, non-executable candidates.

use std::fmt;
use std::sync::Arc;

use super::pagination::{McpCatalogCacheScope, McpCatalogKind, McpRawCatalog};
use super::protocol::ProtocolVersion;
use super::schema::McpSchemaError;

mod candidate;
mod descriptors;
mod fields;
mod names;
mod template;
#[cfg(test)]
mod tests;

pub use candidate::{
    McpCandidateServer, McpCatalogCandidate, McpCatalogServerInput, McpExcludedTool,
    McpExposedTool, McpToolEligibility, McpToolExposureDecision, McpToolExposurePolicy,
    McpToolModelProjection,
};
pub use descriptors::{
    McpDescriptor, McpPromptArgument, McpPromptDescriptor, McpResourceDescriptor,
    McpResourceTemplateDescriptor, McpToolDescriptor,
};

/// Bounds apply before input copies. Schema internals retain their own bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpDescriptorLimits {
    pub max_tools: usize,
    pub max_features: usize,
    pub max_catalog_bytes: usize,
    pub max_candidate_bytes: usize,
    pub max_servers: usize,
    pub max_reserved_names: usize,
    pub max_name_attempts: usize,
}
impl Default for McpDescriptorLimits {
    fn default() -> Self {
        Self {
            max_tools: 2048,
            max_features: 4096,
            max_catalog_bytes: 64 * 1024 * 1024,
            max_candidate_bytes: 64 * 1024 * 1024,
            max_servers: 64,
            max_reserved_names: 131_072,
            max_name_attempts: 1_000_000,
        }
    }
}
impl McpDescriptorLimits {
    fn validate(self) -> Result<Self> {
        let cap = Self::default();
        for (value, maximum) in [
            (self.max_tools, cap.max_tools),
            (self.max_features, cap.max_features),
            (self.max_catalog_bytes, cap.max_catalog_bytes),
            (self.max_candidate_bytes, cap.max_candidate_bytes),
            (self.max_servers, cap.max_servers),
            (self.max_reserved_names, cap.max_reserved_names),
            (self.max_name_attempts, cap.max_name_attempts),
        ] {
            if value == 0 || value > maximum {
                return Err(McpCatalogError::Limit);
            }
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpCatalogError {
    InvalidDescriptor,
    Schema(McpSchemaError),
    InvalidServer,
    DuplicateServer,
    DuplicateFamily,
    ProtocolMismatch,
    InvalidEligibility,
    InvalidReservedName,
    Limit,
}
impl fmt::Display for McpCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP descriptor catalog rejected")
    }
}
impl std::error::Error for McpCatalogError {}
type Result<T> = std::result::Result<T, McpCatalogError>;

/// A fully admitted catalog family; metadata is not runtime publication authority.
#[derive(Clone)]
pub struct McpDescriptorCatalog(Arc<AdmittedCatalog>);
struct AdmittedCatalog {
    kind: McpCatalogKind,
    version: ProtocolVersion,
    fetched_at_ms: u64,
    expires_at_ms: u64,
    cache_scope: McpCatalogCacheScope,
    descriptors: Box<[McpDescriptor]>,
    retained_bytes: usize,
}
impl fmt::Debug for McpDescriptorCatalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpDescriptorCatalog")
            .field("kind", &self.0.kind)
            .field("count", &self.0.descriptors.len())
            .finish_non_exhaustive()
    }
}
impl McpDescriptorCatalog {
    /// Admits every descriptor before publishing any result, retaining original JSON.
    ///
    /// # Errors
    /// Rejects malformed descriptors, schemas or exhausted aggregate bounds.
    pub fn admit(raw: McpRawCatalog, limits: McpDescriptorLimits) -> Result<Self> {
        let limits = limits.validate()?;
        let maximum = if raw.kind() == McpCatalogKind::Tools {
            limits.max_tools
        } else {
            limits.max_features
        };
        if raw.items().len() > maximum {
            return Err(McpCatalogError::Limit);
        }
        let mut retained_bytes = 0;
        // Charge original JSON, owned extracted fields and schema JSON arenas
        // conservatively before allocating descriptor copies. Fixed containers
        // and compiled schema structures have independent count/state bounds.
        for (_, value) in raw.items() {
            charge(
                &mut retained_bytes,
                value
                    .get()
                    .len()
                    .checked_mul(4)
                    .ok_or(McpCatalogError::Limit)?,
                limits.max_catalog_bytes,
            )?;
        }
        let descriptors = raw
            .items()
            .map(|(identity, value)| descriptors::parse(raw.kind(), identity, value))
            .collect::<Result<Vec<_>>>()?;
        let admitted = AdmittedCatalog {
            kind: raw.kind(),
            version: raw.version(),
            fetched_at_ms: raw.fetched_at_ms(),
            expires_at_ms: raw.expires_at_ms(),
            cache_scope: raw.cache_scope(),
            descriptors: descriptors.into_boxed_slice(),
            retained_bytes,
        };
        // Consume and release raw assembly storage before sharing the admitted
        // replacement; retaining both copies is not part of this API.
        drop(raw);
        Ok(Self(Arc::new(admitted)))
    }
    #[must_use]
    pub fn kind(&self) -> McpCatalogKind {
        self.0.kind
    }
    #[must_use]
    pub fn version(&self) -> ProtocolVersion {
        self.0.version
    }
    #[must_use]
    pub fn fetched_at_ms(&self) -> u64 {
        self.0.fetched_at_ms
    }
    #[must_use]
    pub fn expires_at_ms(&self) -> u64 {
        self.0.expires_at_ms
    }
    #[must_use]
    pub fn cache_scope(&self) -> McpCatalogCacheScope {
        self.0.cache_scope
    }
    #[must_use]
    pub fn descriptors(&self) -> &[McpDescriptor] {
        &self.0.descriptors
    }
    #[must_use]
    pub fn retained_byte_charge(&self) -> usize {
        self.0.retained_bytes
    }
}
fn charge(total: &mut usize, bytes: usize, limit: usize) -> Result<()> {
    *total = total.checked_add(bytes).ok_or(McpCatalogError::Limit)?;
    if *total > limit {
        return Err(McpCatalogError::Limit);
    }
    Ok(())
}
