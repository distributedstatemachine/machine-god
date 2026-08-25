//! Shared credential and TLS helpers for native AI Gateway HTTP transports.

use reqwest::Certificate;
use reqwest::header::HeaderValue;
use std::fmt;
use std::sync::OnceLock;

/// Maximum accepted bearer-token size.
pub const AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES: usize = 4 * 1024;

/// Stable construction-error category for native AI Gateway HTTP transports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AiGatewayHttpConfigErrorKind {
    /// The bearer token is empty, oversized, or not RFC 6750 `b64token` syntax.
    InvalidBearerToken,
    /// The endpoint is not the pinned endpoint or a strict loopback test URL.
    InvalidEndpoint,
    /// One or more transport resource limits are invalid.
    InvalidLimits,
    /// The HTTP backend could not be initialized.
    ClientInitialization,
}

/// Redacted native HTTP-transport construction failure.
#[derive(Clone, Eq, PartialEq)]
pub struct AiGatewayHttpConfigError {
    kind: AiGatewayHttpConfigErrorKind,
}

impl AiGatewayHttpConfigError {
    /// Returns the stable error category.
    #[must_use]
    pub const fn kind(&self) -> AiGatewayHttpConfigErrorKind {
        self.kind
    }

    pub(crate) const fn new(kind: AiGatewayHttpConfigErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for AiGatewayHttpConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayHttpConfigError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for AiGatewayHttpConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            AiGatewayHttpConfigErrorKind::InvalidBearerToken => "invalid bearer token",
            AiGatewayHttpConfigErrorKind::InvalidEndpoint => "invalid AI Gateway HTTP endpoint",
            AiGatewayHttpConfigErrorKind::InvalidLimits => "invalid AI Gateway HTTP limits",
            AiGatewayHttpConfigErrorKind::ClientInitialization => {
                "AI Gateway HTTP client initialization failed"
            }
        })
    }
}

impl std::error::Error for AiGatewayHttpConfigError {}

/// Owned bearer credential with redacted formatting and best-effort drop clearing.
pub struct AiGatewayBearerToken {
    bytes: Box<[u8]>,
}

impl AiGatewayBearerToken {
    /// Validates an RFC 6750 `b64token` value bounded to 4 KiB.
    ///
    /// # Errors
    ///
    /// Returns a redacted error for an empty, oversized, or malformed token.
    pub fn new(token: impl Into<String>) -> Result<Self, AiGatewayHttpConfigError> {
        let mut token = token.into().into_bytes();
        if !valid_bearer_token(&token) {
            token.fill(0);
            return Err(AiGatewayHttpConfigError::new(
                AiGatewayHttpConfigErrorKind::InvalidBearerToken,
            ));
        }
        Ok(Self {
            bytes: token.into_boxed_slice(),
        })
    }
}

impl fmt::Debug for AiGatewayBearerToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AiGatewayBearerToken(<redacted>)")
    }
}

impl Drop for AiGatewayBearerToken {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

fn valid_bearer_token(token: &[u8]) -> bool {
    if token.is_empty() || token.len() > AI_GATEWAY_HTTP_MAX_BEARER_TOKEN_BYTES {
        return false;
    }
    let Some(first_padding) = token.iter().position(|byte| *byte == b'=') else {
        return token.iter().copied().all(valid_b64token_byte);
    };
    first_padding > 0
        && token[..first_padding]
            .iter()
            .copied()
            .all(valid_b64token_byte)
        && token[first_padding..].iter().all(|byte| *byte == b'=')
}

const fn valid_b64token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
}

pub(crate) fn root_certificates() -> Result<Vec<Certificate>, AiGatewayHttpConfigError> {
    static CERTIFICATES: OnceLock<Result<Vec<Certificate>, ()>> = OnceLock::new();
    CERTIFICATES
        .get_or_init(|| {
            webpki_root_certs::TLS_SERVER_ROOT_CERTS
                .iter()
                .map(|certificate| Certificate::from_der(certificate.as_ref()).map_err(|_| ()))
                .collect()
        })
        .clone()
        .map_err(|()| {
            AiGatewayHttpConfigError::new(AiGatewayHttpConfigErrorKind::ClientInitialization)
        })
}

pub(crate) fn authorization_value(
    token: &AiGatewayBearerToken,
) -> Result<HeaderValue, AiGatewayHttpConfigError> {
    let mut bytes = Vec::with_capacity("Bearer ".len() + token.bytes.len());
    bytes.extend_from_slice(b"Bearer ");
    bytes.extend_from_slice(&token.bytes);
    let result = HeaderValue::from_bytes(&bytes).map_err(|_| {
        AiGatewayHttpConfigError::new(AiGatewayHttpConfigErrorKind::InvalidBearerToken)
    });
    bytes.fill(0);
    let mut value = result?;
    value.set_sensitive(true);
    Ok(value)
}
