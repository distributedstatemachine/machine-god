use crate::{BoxFuture, CancellationToken, ProviderError};
use core::fmt;

/// The reason a model identifier was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InvalidModelIdReason {
    /// The identifier contained no bytes.
    Empty,
    /// The identifier exceeded the 128-byte limit.
    TooLong,
    /// The identifier contained a byte outside visible ASCII.
    NotVisibleAscii,
}

/// A model identifier failed validation.
///
/// The rejected input is not retained or reflected in diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidModelId {
    reason: InvalidModelIdReason,
}

impl InvalidModelId {
    /// Returns the stable rejection reason.
    #[must_use]
    pub const fn reason(&self) -> InvalidModelIdReason {
        self.reason
    }
}

impl fmt::Display for InvalidModelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.reason {
            InvalidModelIdReason::Empty => "must not be empty",
            InvalidModelIdReason::TooLong => "must be at most 128 bytes",
            InvalidModelIdReason::NotVisibleAscii => "must contain only visible ASCII bytes",
        };
        write!(formatter, "invalid model ID: {reason}")
    }
}

impl std::error::Error for InvalidModelId {}

/// One validated model returned by a provider's catalog.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AvailableModel {
    id: String,
}

impl AvailableModel {
    /// Validates and owns one provider model identifier.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidModelId`] when `id` is empty, longer than 128 bytes,
    /// or contains a byte outside visible ASCII `0x21` through `0x7e`.
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidModelId> {
        let id = id.into();
        if id.is_empty() {
            return Err(InvalidModelId {
                reason: InvalidModelIdReason::Empty,
            });
        }
        if id.len() > 128 {
            return Err(InvalidModelId {
                reason: InvalidModelIdReason::TooLong,
            });
        }
        if !id.bytes().all(|byte| (b'!'..=b'~').contains(&byte)) {
            return Err(InvalidModelId {
                reason: InvalidModelIdReason::NotVisibleAscii,
            });
        }
        Ok(Self { id })
    }

    /// Returns the validated provider model identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Why an operation used only the public model catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PublicCatalogReason {
    /// No credential was available for the request.
    NoCredential,
    /// The provider rejected the supplied credential.
    AuthenticatedCredentialRejected,
}

/// The access level used to obtain a model catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ModelCatalogAccess {
    /// The provider accepted an authenticated catalog request.
    Authenticated,
    /// Only the public catalog was available.
    PublicOnly {
        /// Why authenticated catalog access was not used.
        reason: PublicCatalogReason,
    },
}

/// An owned, ordered provider model catalog.
///
/// The constructor preserves the provider's order exactly. Providers must
/// therefore supply a deterministic order when their source does not already
/// define one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelCatalog {
    models: Vec<AvailableModel>,
    access: ModelCatalogAccess,
}

impl ModelCatalog {
    /// Creates a catalog while preserving the supplied model order.
    #[must_use]
    pub fn new(models: Vec<AvailableModel>, access: ModelCatalogAccess) -> Self {
        Self { models, access }
    }

    /// Returns the models in provider-defined deterministic order.
    #[must_use]
    pub fn models(&self) -> &[AvailableModel] {
        &self.models
    }

    /// Returns the access level used to obtain this catalog.
    #[must_use]
    pub const fn access(&self) -> ModelCatalogAccess {
        self.access
    }

    /// Consumes the catalog and returns its ordered models.
    #[must_use]
    pub fn into_models(self) -> Vec<AvailableModel> {
        self.models
    }
}

/// Provider-neutral, object-safe model-catalog interface.
pub trait ModelCatalogProvider: Send + Sync + 'static {
    /// Stable provider identifier for diagnostics.
    fn name(&self) -> &str;

    /// Lists the models available through this provider.
    ///
    /// Implementations must observe `cancellation` and return a
    /// [`crate::ProviderErrorKind::Cancelled`] error when it wins.
    fn list_models(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>>;
}
