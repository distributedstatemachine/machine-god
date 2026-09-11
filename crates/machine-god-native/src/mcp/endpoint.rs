//! Parsed MCP HTTP destinations, without DNS, credentials or network authority.

use std::fmt;

use url::{Host, Position, Url};

/// Inclusive bound on a configured URL before parsing.
pub const MAX_CONFIGURED_ENDPOINT_BYTES: usize = 4096;
/// Inclusive bound on a deprecated SSE endpoint event before resolution.
pub const MAX_ENDPOINT_EVENT_BYTES: usize = 8192;
/// Inclusive bound on a retained, canonical URL after parsing/resolution.
pub const MAX_CANONICAL_ENDPOINT_BYTES: usize = 16 * 1024;

/// Fixed failures never disclose a URL, query, host or credential.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpEndpointError {
    Invalid,
    Insecure,
    CrossOrigin,
    Limit,
}

impl fmt::Display for McpEndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid MCP endpoint",
            Self::Insecure => "insecure MCP endpoint",
            Self::CrossOrigin => "cross-origin MCP endpoint",
            Self::Limit => "MCP endpoint limit exceeded",
        })
    }
}
impl std::error::Error for McpEndpointError {}

/// Immutable HTTP destination admitted by syntax and endpoint policy only.
///
/// Parsing does not resolve DNS or authorize connections, credentials, redirects
/// or requests. The transport must retain separate explicit effect authority.
/// Canonical URL spelling follows the established `url` parser, not display text.
#[derive(Clone, Eq, PartialEq)]
pub struct McpEndpoint {
    url: Url,
    host: Host<Box<str>>,
    port: u16,
}

impl fmt::Debug for McpEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpEndpoint { <redacted> }")
    }
}

impl McpEndpoint {
    /// Parses a configured HTTPS URL or explicitly ported loopback HTTP URL.
    ///
    /// # Errors
    /// Rejects invalid, insecure or oversized URLs. Credentials, fragments,
    /// whitespace, backslashes and malformed percent escapes are not accepted.
    pub fn parse(value: &str) -> Result<Self, McpEndpointError> {
        parse_absolute(value, MAX_CONFIGURED_ENDPOINT_BYTES)
    }

    /// Resolves an explicitly received deprecated SSE `endpoint` event.
    ///
    /// # Errors
    /// Rejects invalid, oversized or cross-origin destinations. An absolute or
    /// network-path HTTP reference must itself include an explicit loopback port;
    /// a relative path inherits the already admitted base endpoint's port.
    pub fn resolve_message_endpoint(&self, event: &str) -> Result<Self, McpEndpointError> {
        validate_text(event, MAX_ENDPOINT_EVENT_BYTES)?;
        let url = if let Some(authority) = event.strip_prefix("//") {
            let absolute = format!("{}://{authority}", self.url.scheme());
            parse_absolute(&absolute, MAX_ENDPOINT_EVENT_BYTES + 8)?.url
        } else if Url::parse(event).is_ok() {
            parse_absolute(event, MAX_ENDPOINT_EVENT_BYTES)?.url
        } else {
            self.url
                .join(event)
                .map_err(|_| McpEndpointError::Invalid)?
        };
        let result = Self::from_parsed(url)?;
        if !self.same_origin(&result) {
            return Err(McpEndpointError::CrossOrigin);
        }
        Ok(result)
    }

    /// Canonical URL. It can contain a sensitive query and must not be logged.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    /// Typed host for the explicitly owned DNS/TLS connector.
    #[must_use]
    pub fn host(&self) -> Host<&str> {
        match &self.host {
            Host::Domain(name) => Host::Domain(name.as_ref()),
            Host::Ipv4(address) => Host::Ipv4(*address),
            Host::Ipv6(address) => Host::Ipv6(*address),
        }
    }

    /// Effective HTTP/HTTPS port, including a normalized explicit default port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub fn is_tls(&self) -> bool {
        self.url.scheme() == "https"
    }

    /// Canonical Host authority, including brackets around an IPv6 literal.
    #[must_use]
    pub fn authority(&self) -> &str {
        &self.url[Position::BeforeHost..Position::AfterPort]
    }

    /// Canonical origin-form request target, always beginning with `/`.
    #[must_use]
    pub fn request_target(&self) -> &str {
        &self.url[Position::BeforePath..Position::AfterQuery]
    }

    /// Compares normalized scheme, parsed host and effective port only.
    /// This is an endpoint comparison, not permission to forward credentials.
    #[must_use]
    pub fn same_origin(&self, other: &Self) -> bool {
        self.url.scheme() == other.url.scheme()
            && self.url.host() == other.url.host()
            && self.port() == other.port()
    }

    fn from_parsed(url: Url) -> Result<Self, McpEndpointError> {
        if url.as_str().len() > MAX_CANONICAL_ENDPOINT_BYTES {
            return Err(McpEndpointError::Limit);
        }
        if url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(McpEndpointError::Invalid);
        }
        if !matches!(url.scheme(), "http" | "https") {
            return Err(McpEndpointError::Insecure);
        }
        let host = match url.host().ok_or(McpEndpointError::Invalid)? {
            Host::Domain(name) => Host::Domain(name.into()),
            Host::Ipv4(address) => Host::Ipv4(address),
            Host::Ipv6(address) => Host::Ipv6(address),
        };
        let port = url
            .port_or_known_default()
            .ok_or(McpEndpointError::Invalid)?;
        Ok(Self { url, host, port })
    }
}

fn parse_absolute(value: &str, limit: usize) -> Result<McpEndpoint, McpEndpointError> {
    validate_text(value, limit)?;
    // Require an authority-bearing spelling. Url otherwise normalizes special
    // scheme spellings such as `https:host` and silently empty user information.
    let (scheme, rest) = value.split_once("://").ok_or(McpEndpointError::Invalid)?;
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(McpEndpointError::Invalid);
    }
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return Err(McpEndpointError::Insecure);
    }
    let url = Url::parse(value).map_err(|_| McpEndpointError::Invalid)?;
    if url.scheme() == "http" {
        let (host, port) = authority
            .rsplit_once(':')
            .ok_or(McpEndpointError::Insecure)?;
        if !(host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]")
            || port.is_empty()
            || !port.bytes().all(|b| b.is_ascii_digit())
            || port.parse::<u16>().is_err()
        {
            return Err(McpEndpointError::Insecure);
        }
    }
    McpEndpoint::from_parsed(url)
}

fn validate_text(value: &str, limit: usize) -> Result<(), McpEndpointError> {
    if value.len() > limit {
        return Err(McpEndpointError::Limit);
    }
    if value.is_empty()
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || value.contains(['#', '\\'])
    {
        return Err(McpEndpointError::Invalid);
    }
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%'
            && !(bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
                && bytes.next().is_some_and(|b| b.is_ascii_hexdigit()))
        {
            return Err(McpEndpointError::Invalid);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
