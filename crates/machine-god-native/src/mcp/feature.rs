//! Complete native MCP feature codecs. All public values are data, not authority.
use std::fmt;

pub(crate) mod content;
mod request;
mod response;
mod template;
#[cfg(test)]
mod tests;

pub use content::{McpContent, McpContentKind, McpResourceContent, McpResourceData};
pub use request::{McpFeatureExchange, McpFeatureExchangeOptions, McpFeatureIdentity};
pub use response::{
    McpFeatureCacheHints, McpFeatureCatalogLoad, McpFeatureOutcome, McpFeatureResponse,
    McpPromptMessage, McpPromptRole,
};
pub use template::{McpTemplateMatchBudget, matches_resource_template};

/// Inclusive, lowerable-only native codec bounds, independent of model output.
#[derive(Clone, Copy, Debug)]
pub struct McpFeatureCodecLimits {
    pub max_response_bytes: usize,
    pub max_nodes: usize,
    pub max_content_bytes: usize,
    pub max_content_field_bytes: usize,
    pub max_content_items: usize,
    pub max_retained_bytes: usize,
}
impl Default for McpFeatureCodecLimits {
    fn default() -> Self {
        Self {
            max_response_bytes: 16 * 1024 * 1024,
            max_nodes: 262_144,
            max_content_bytes: 4 * 1024 * 1024,
            max_content_field_bytes: 1024 * 1024,
            max_content_items: 256,
            max_retained_bytes: 64 * 1024 * 1024,
        }
    }
}
impl McpFeatureCodecLimits {
    pub(crate) fn validate(self) -> Result<Self> {
        let cap = Self::default();
        for (value, maximum) in [
            (self.max_response_bytes, cap.max_response_bytes),
            (self.max_nodes, cap.max_nodes),
            (self.max_content_bytes, cap.max_content_bytes),
            (self.max_content_field_bytes, cap.max_content_field_bytes),
            (self.max_content_items, cap.max_content_items),
            (self.max_retained_bytes, cap.max_retained_bytes),
        ] {
            if value == 0 || value > maximum {
                return Err(Error::InvalidLimits);
            }
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpFeatureCodecError {
    InvalidLimits,
    InvalidRequest,
    MissingCatalog,
    NotFound,
    Unsupported,
    InvalidResponse,
    Correlation,
    Limit,
    Closed,
}
impl fmt::Display for McpFeatureCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP feature codec rejected data")
    }
}
impl std::error::Error for McpFeatureCodecError {}
impl From<crate::mcp::catalog::McpCatalogError> for McpFeatureCodecError {
    fn from(error: crate::mcp::catalog::McpCatalogError) -> Self {
        if matches!(error, crate::mcp::catalog::McpCatalogError::Limit) {
            Self::Limit
        } else {
            Self::InvalidResponse
        }
    }
}
type Error = McpFeatureCodecError;
type Result<T> = std::result::Result<T, Error>;

fn charge(total: &mut usize, bytes: usize, limit: usize) -> Result<()> {
    let next = total.checked_add(bytes).ok_or(Error::Limit)?;
    if next > limit {
        return Err(Error::Limit);
    }
    *total = next;
    Ok(())
}
