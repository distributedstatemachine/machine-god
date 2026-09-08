use crate::{BoxFuture, CancellationToken, ProviderError};
use core::fmt;

/// Maximum UTF-8 bytes in a provider model identifier.
pub const MAX_MODEL_ID_BYTES: usize = 1024;

/// The reason a model identifier was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InvalidModelIdReason {
    /// The identifier contained no bytes.
    Empty,
    /// The identifier exceeded [`MAX_MODEL_ID_BYTES`].
    TooLong,
    /// Legacy rejection reason; no longer returned by model validation.
    NotVisibleAscii,
    /// The identifier contained an ASCII control byte (C0 or DEL).
    ControlCharacter,
    /// The identifier began or ended with space, tab, CR, or LF.
    EdgeWhitespace,
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
            InvalidModelIdReason::TooLong => "must be at most 1024 bytes",
            InvalidModelIdReason::NotVisibleAscii => "must contain only visible ASCII bytes",
            InvalidModelIdReason::ControlCharacter => "must not contain ASCII control bytes",
            InvalidModelIdReason::EdgeWhitespace => "must not have edge whitespace",
        };
        write!(formatter, "invalid model ID: {reason}")
    }
}

impl std::error::Error for InvalidModelId {}

/// Validates a borrowed model ID without allocating or normalizing it.
///
/// IDs are opaque UTF-8 strings: interior spaces and non-ASCII characters are
/// permitted. Consumers must escape untrusted IDs for their output context.
///
/// # Errors
///
/// Rejects empty IDs, more than [`MAX_MODEL_ID_BYTES`] UTF-8 bytes, ASCII
/// control bytes (C0 and DEL), and leading/trailing space, tab, CR, or LF.
pub fn validate_model_id(id: &str) -> Result<(), InvalidModelId> {
    let reason = if id.is_empty() {
        Some(InvalidModelIdReason::Empty)
    } else if id.len() > MAX_MODEL_ID_BYTES {
        Some(InvalidModelIdReason::TooLong)
    } else if id.starts_with([' ', '\t', '\r', '\n']) || id.ends_with([' ', '\t', '\r', '\n']) {
        Some(InvalidModelIdReason::EdgeWhitespace)
    } else if id.bytes().any(|byte| byte.is_ascii_control()) {
        Some(InvalidModelIdReason::ControlCharacter)
    } else {
        None
    };
    match reason {
        Some(reason) => Err(InvalidModelId { reason }),
        None => Ok(()),
    }
}

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
    /// Returns [`InvalidModelId`] when [`validate_model_id`] rejects `id`.
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidModelId> {
        let id = id.into();
        validate_model_id(&id)?;
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
