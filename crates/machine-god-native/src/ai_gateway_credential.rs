//! Bounded, redacted AI Gateway credential discovery for native hosts.

use std::env;
use std::ffi::OsString;
use std::fmt;

#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

use crate::ai_gateway_http::AiGatewayBearerToken;

/// Environment variable containing Vercel's preferred OIDC credential.
pub const VERCEL_OIDC_TOKEN_ENV: &str = "VERCEL_OIDC_TOKEN";

/// Environment variable containing the fallback AI Gateway API key.
pub const AI_GATEWAY_API_KEY_ENV: &str = "AI_GATEWAY_API_KEY";

/// Source selected for a discovered AI Gateway credential.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AiGatewayCredentialSource {
    /// The `VERCEL_OIDC_TOKEN` environment value.
    VercelOidcToken,
    /// The `AI_GATEWAY_API_KEY` environment value.
    AiGatewayApiKey,
}

impl AiGatewayCredentialSource {
    /// Returns the stable, machine-readable name of this source.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VercelOidcToken => "vercel_oidc_token",
            Self::AiGatewayApiKey => "ai_gateway_api_key",
        }
    }
}

enum CredentialEnvironmentValue {
    Absent,
    InvalidEnvironment,
    InvalidBearerToken,
    Token(AiGatewayBearerToken),
}

/// Owned, prevalidated snapshot of AI Gateway credential environment values.
///
/// Construction performs no ambient environment read. It immediately replaces
/// each input with an absent, invalid, or validated state and never retains a
/// malformed raw [`OsString`]. Accepted tokens use the existing 4,096-byte
/// bearer-token bound and move through discovery without cloning.
///
/// Secret clearing is best effort, not comprehensive zeroization. Owned token
/// bytes and rejected Unicode token bytes are overwritten before release. On
/// Unix, an owned non-Unicode environment buffer is also overwritten before
/// release; other platforms provide no portable mutable representation for
/// that operation. Caller copies, the process environment, standard-library
/// lookup internals, allocator history, and later HTTP header copies are outside
/// this snapshot's control. In particular, [`Self::from_process`] may allocate
/// an oversized process value before this API can reject it.
pub struct AiGatewayCredentialEnvironment {
    vercel_oidc_token: CredentialEnvironmentValue,
    ai_gateway_api_key: CredentialEnvironmentValue,
}

impl AiGatewayCredentialEnvironment {
    /// Creates a credential snapshot from explicitly injected owned values.
    ///
    /// Only a zero-length value is treated as absent. Values are not trimmed or
    /// otherwise normalized.
    #[must_use]
    pub fn new(vercel_oidc_token: Option<OsString>, ai_gateway_api_key: Option<OsString>) -> Self {
        Self {
            vercel_oidc_token: classify_environment_value(vercel_oidc_token),
            ai_gateway_api_key: classify_environment_value(ai_gateway_api_key),
        }
    }

    /// Captures and prevalidates the two supported process environment values.
    #[must_use]
    pub fn from_process() -> Self {
        Self::new(
            env::var_os(VERCEL_OIDC_TOKEN_ENV),
            env::var_os(AI_GATEWAY_API_KEY_ENV),
        )
    }
}

impl fmt::Debug for AiGatewayCredentialEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AiGatewayCredentialEnvironment(<redacted>)")
    }
}

/// A validated AI Gateway bearer token together with its non-secret source.
///
/// The token is non-cloneable and its debug representation is always redacted.
pub struct DiscoveredAiGatewayCredential {
    source: AiGatewayCredentialSource,
    bearer_token: AiGatewayBearerToken,
}

/// Credential state selected for a model-catalog request.
///
/// Unlike model generation, the catalog has an explicit anonymous mode. A
/// completely absent credential therefore selects [`Self::PublicOnly`], while
/// a selected malformed value still fails closed.
pub enum DiscoveredAiGatewayCatalogCredential {
    /// No supported credential was present; use the public catalog.
    PublicOnly,
    /// A validated credential is available for authenticated catalog access.
    Authenticated(DiscoveredAiGatewayCredential),
}

impl fmt::Debug for DiscoveredAiGatewayCatalogCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PublicOnly => {
                formatter.write_str("DiscoveredAiGatewayCatalogCredential::PublicOnly")
            }
            Self::Authenticated(credential) => formatter
                .debug_tuple("DiscoveredAiGatewayCatalogCredential::Authenticated")
                .field(&credential.source())
                .field(&"<redacted>")
                .finish(),
        }
    }
}

impl DiscoveredAiGatewayCredential {
    /// Returns the environment source selected by discovery.
    #[must_use]
    pub const fn source(&self) -> AiGatewayCredentialSource {
        self.source
    }

    /// Consumes the discovered credential and returns its bearer token.
    #[must_use]
    pub fn into_bearer_token(self) -> AiGatewayBearerToken {
        self.bearer_token
    }
}

impl fmt::Debug for DiscoveredAiGatewayCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscoveredAiGatewayCredential")
            .field("source", &self.source)
            .field("bearer_token", &"<redacted>")
            .finish()
    }
}

/// Stable category for an AI Gateway credential discovery failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AiGatewayCredentialErrorKind {
    /// Neither supported source contains a nonempty value.
    Missing,
    /// The selected nonempty environment value is not Unicode.
    InvalidEnvironment,
    /// The selected value is malformed or exceeds the bearer-token bound.
    InvalidBearerToken,
}

/// Fixed, redacted AI Gateway credential discovery failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AiGatewayCredentialError {
    kind: AiGatewayCredentialErrorKind,
}

impl AiGatewayCredentialError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> AiGatewayCredentialErrorKind {
        self.kind
    }

    const fn new(kind: AiGatewayCredentialErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for AiGatewayCredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayCredentialError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for AiGatewayCredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            AiGatewayCredentialErrorKind::Missing => "AI Gateway credential is missing",
            AiGatewayCredentialErrorKind::InvalidEnvironment => {
                "AI Gateway credential environment is invalid"
            }
            AiGatewayCredentialErrorKind::InvalidBearerToken => {
                "AI Gateway bearer token is invalid"
            }
        })
    }
}

impl std::error::Error for AiGatewayCredentialError {}

/// Discovers a credential from an explicitly injected environment snapshot.
///
/// A nonempty `VERCEL_OIDC_TOKEN` has precedence over a nonempty
/// `AI_GATEWAY_API_KEY`. An empty value is absent and falls through. A selected
/// nonempty invalid value fails closed without falling back to the lower source.
/// The snapshot is consumed so the selected token can move without cloning and
/// every unselected validated token is promptly dropped and cleared.
///
/// # Errors
///
/// Returns [`AiGatewayCredentialError`] when no credential is present or the
/// selected value is not Unicode, malformed, or oversized.
pub fn discover_ai_gateway_credential(
    environment: AiGatewayCredentialEnvironment,
) -> Result<DiscoveredAiGatewayCredential, AiGatewayCredentialError> {
    let AiGatewayCredentialEnvironment {
        vercel_oidc_token,
        ai_gateway_api_key,
    } = environment;

    match vercel_oidc_token {
        CredentialEnvironmentValue::Absent => discover_api_key(ai_gateway_api_key),
        CredentialEnvironmentValue::InvalidEnvironment => Err(AiGatewayCredentialError::new(
            AiGatewayCredentialErrorKind::InvalidEnvironment,
        )),
        CredentialEnvironmentValue::InvalidBearerToken => Err(AiGatewayCredentialError::new(
            AiGatewayCredentialErrorKind::InvalidBearerToken,
        )),
        CredentialEnvironmentValue::Token(bearer_token) => Ok(DiscoveredAiGatewayCredential {
            source: AiGatewayCredentialSource::VercelOidcToken,
            bearer_token,
        }),
    }
}

/// Captures the process environment and discovers an AI Gateway credential.
///
/// This is the only credential-discovery convenience function that reads the
/// ambient process environment.
///
/// # Errors
///
/// Returns [`AiGatewayCredentialError`] under the same conditions as
/// [`discover_ai_gateway_credential`].
pub fn discover_process_ai_gateway_credential()
-> Result<DiscoveredAiGatewayCredential, AiGatewayCredentialError> {
    discover_ai_gateway_credential(AiGatewayCredentialEnvironment::from_process())
}

/// Discovers optional catalog authentication from an injected snapshot.
///
/// Source precedence and validation are identical to
/// [`discover_ai_gateway_credential`]. The sole difference is that a completely
/// missing credential selects public-only access instead of returning an error.
///
/// # Errors
///
/// Returns [`AiGatewayCredentialError`] when the selected nonempty value is
/// non-Unicode, malformed, or oversized.
pub fn discover_ai_gateway_catalog_credential(
    environment: AiGatewayCredentialEnvironment,
) -> Result<DiscoveredAiGatewayCatalogCredential, AiGatewayCredentialError> {
    match discover_ai_gateway_credential(environment) {
        Ok(credential) => Ok(DiscoveredAiGatewayCatalogCredential::Authenticated(
            credential,
        )),
        Err(error) if error.kind() == AiGatewayCredentialErrorKind::Missing => {
            Ok(DiscoveredAiGatewayCatalogCredential::PublicOnly)
        }
        Err(error) => Err(error),
    }
}

/// Captures the process environment and discovers optional catalog authentication.
///
/// This is the only catalog-credential convenience function that reads the
/// ambient process environment.
///
/// # Errors
///
/// Returns [`AiGatewayCredentialError`] under the same conditions as
/// [`discover_ai_gateway_catalog_credential`].
pub fn discover_process_ai_gateway_catalog_credential()
-> Result<DiscoveredAiGatewayCatalogCredential, AiGatewayCredentialError> {
    discover_ai_gateway_catalog_credential(AiGatewayCredentialEnvironment::from_process())
}

fn discover_api_key(
    value: CredentialEnvironmentValue,
) -> Result<DiscoveredAiGatewayCredential, AiGatewayCredentialError> {
    match value {
        CredentialEnvironmentValue::Absent => Err(AiGatewayCredentialError::new(
            AiGatewayCredentialErrorKind::Missing,
        )),
        CredentialEnvironmentValue::InvalidEnvironment => Err(AiGatewayCredentialError::new(
            AiGatewayCredentialErrorKind::InvalidEnvironment,
        )),
        CredentialEnvironmentValue::InvalidBearerToken => Err(AiGatewayCredentialError::new(
            AiGatewayCredentialErrorKind::InvalidBearerToken,
        )),
        CredentialEnvironmentValue::Token(bearer_token) => Ok(DiscoveredAiGatewayCredential {
            source: AiGatewayCredentialSource::AiGatewayApiKey,
            bearer_token,
        }),
    }
}

fn classify_environment_value(value: Option<OsString>) -> CredentialEnvironmentValue {
    let Some(value) = value else {
        return CredentialEnvironmentValue::Absent;
    };
    if value.is_empty() {
        return CredentialEnvironmentValue::Absent;
    }
    let value = match value.into_string() {
        Ok(value) => value,
        Err(value) => {
            discard_invalid_environment_value(value);
            return CredentialEnvironmentValue::InvalidEnvironment;
        }
    };
    match AiGatewayBearerToken::new(value) {
        Ok(token) => CredentialEnvironmentValue::Token(token),
        Err(_) => CredentialEnvironmentValue::InvalidBearerToken,
    }
}

#[cfg(unix)]
fn discard_invalid_environment_value(value: OsString) {
    let mut bytes = value.into_vec();
    bytes.fill(0);
}

#[cfg(not(unix))]
fn discard_invalid_environment_value(value: OsString) {
    drop(value);
}
