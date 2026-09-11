//! Bounded, effect-free resolution of explicitly supplied MCP remote headers.
//!
//! Profile headers precede captured environment bindings. Active OAuth tokens
//! take precedence over bearer environment lookup; OAuth configuration alone is
//! not an active credential. Resolved/ACP Authorization is allowed even though
//! profile configuration forbids it. Protocol-owned names and all duplicate
//! case-insensitive names are rejected, never overwritten.

use std::fmt;
use std::sync::Arc;

use super::config::McpRemoteConfig;

#[cfg(test)]
mod tests;

/// Maximum final header count, including generated Authorization.
pub const MAX_HEADERS: usize = 128;
/// Maximum individual header-name or header-value byte length.
pub const MAX_HEADER_FIELD_BYTES: usize = 16 * 1024;
/// Maximum combined name/value bytes, including a generated `Bearer ` prefix.
pub const MAX_HEADER_BYTES: usize = 512 * 1024;
/// Maximum explicit authentication identity encoding, including length framing.
pub const MAX_AUTHENTICATION_IDENTITY_BYTES: usize = MAX_HEADER_BYTES + MAX_HEADERS * 8 + 8;

/// Fixed diagnostics without header identities, environment names or secrets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpHeaderError {
    /// An inclusive count, field or aggregate byte bound was exceeded.
    Limit,
    /// A name is empty or is not an ASCII HTTP token.
    InvalidName,
    /// A value contains a forbidden control byte (HTAB is permitted).
    InvalidValue,
    /// A protocol-owned name was supplied by a caller.
    Reserved,
    /// Two final names identify the same case-insensitive HTTP field.
    Duplicate,
    /// A configured header environment reference is absent from the capture.
    MissingHeaderEnvironment,
    /// The selected bearer environment reference is absent from the capture.
    MissingBearerEnvironment,
}

impl fmt::Display for McpHeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Limit => "MCP header limit exceeded",
            Self::InvalidName => "invalid MCP header name",
            Self::InvalidValue => "invalid MCP header value",
            Self::Reserved => "MCP header is protocol-owned",
            Self::Duplicate => "duplicate MCP header",
            Self::MissingHeaderEnvironment => "MCP header environment value is missing",
            Self::MissingBearerEnvironment => "MCP bearer environment value is missing",
        })
    }
}
impl std::error::Error for McpHeaderError {}

struct Header {
    name: Box<str>,
    value: Box<[u8]>,
}

/// Admitted immutable header bytes. Reading values requires explicit iteration.
/// No environment, network, filesystem or credential refresh effects occur.
#[derive(Clone)]
pub struct McpResolvedHeaders {
    headers: Arc<[Header]>,
}

impl fmt::Debug for McpResolvedHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpResolvedHeaders { <redacted> }")
    }
}

impl McpResolvedHeaders {
    /// Resolves profile fields against a caller-supplied captured byte lookup.
    /// `additional` is separately supplied resolved/ACP input, not profile data.
    /// It is revalidated and cannot override another field.
    ///
    /// Ordering is static headers, header environment bindings, additional fields,
    /// then generated Authorization. An active OAuth token bypasses bearer lookup,
    /// even when empty. Absent active credentials permit configured bearer lookup
    /// even if OAuth is configured. Captured empty values are present values.
    ///
    /// # Errors
    /// Rejects missing selected environment values, invalid/protocol-owned names,
    /// injection controls, case-insensitive duplicates and finite bounds. Bounds
    /// and validation precede owned name/value copies; no partial result escapes.
    pub fn resolve<'env>(
        remote: &McpRemoteConfig,
        mut lookup: impl FnMut(&str) -> Option<&'env [u8]>,
        active_oauth_access_token: Option<&[u8]>,
        additional: &[(&str, &[u8])],
    ) -> Result<Self, McpHeaderError> {
        let generated = active_oauth_access_token.is_some() || remote.bearer_token_env().is_some();
        let count = remote.headers().len() + remote.header_env().len() + usize::from(generated);
        if additional.len() > MAX_HEADERS.saturating_sub(count) || count > MAX_HEADERS {
            return Err(McpHeaderError::Limit);
        }
        let mut pending = Pending::new(count + additional.len());
        for (name, value) in remote.headers() {
            pending.push(name, Value::Raw(value.as_bytes()))?;
        }
        for (name, environment) in remote.header_env() {
            let value = lookup(environment).ok_or(McpHeaderError::MissingHeaderEnvironment)?;
            pending.push(name, Value::Raw(value))?;
        }
        for &(name, value) in additional {
            pending.push(name, Value::Raw(value))?;
        }
        let bearer = match active_oauth_access_token {
            Some(token) => Some(token),
            None => remote
                .bearer_token_env()
                .map(|name| lookup(name).ok_or(McpHeaderError::MissingBearerEnvironment))
                .transpose()?,
        };
        if let Some(token) = bearer {
            pending.push("Authorization", Value::Bearer(token))?;
        }
        Ok(pending.finish())
    }

    /// Admits an explicitly resolved header set, including ACP Authorization,
    /// without consulting profile configuration or any environment lookup.
    ///
    /// # Errors
    /// Uses the same count, byte, name, value and duplicate policy as `resolve`.
    pub fn from_resolved(headers: &[(&str, &[u8])]) -> Result<Self, McpHeaderError> {
        if headers.len() > MAX_HEADERS {
            return Err(McpHeaderError::Limit);
        }
        let mut pending = Pending::new(headers.len());
        for &(name, value) in headers {
            pending.push(name, Value::Raw(value))?;
        }
        Ok(pending.finish())
    }

    /// Intentionally exposes exact admitted names and secret-bearing value bytes
    /// for a separately authorized transport. Do not log or serialize this view.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &[u8])> {
        self.headers
            .iter()
            .map(|header| (&*header.name, &*header.value))
    }

    /// Returns the final admitted header count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.headers.len()
    }

    /// Whether this set contains no caller-supplied or generated fields.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }

    /// Intentionally copies secret-bearing identity bytes for trusted runtime
    /// binding. Do not log, serialize as public state or expose these bytes.
    ///
    /// Encoding is `MGH1`, a big-endian u32 count, then entries sorted by
    /// lowercase ASCII name: u32 name length, lowercase name, u32 value length,
    /// exact raw value. No hash collisions, case or ordering ambiguities occur.
    /// Allocation is bounded by `MAX_AUTHENTICATION_IDENTITY_BYTES` and is not
    /// retained by this object. Runtime must separately bind endpoint, generation,
    /// session and turn. This is not constant-time credential verification.
    #[must_use]
    pub fn authentication_identity_bytes(&self) -> Box<[u8]> {
        let mut ordered: Vec<_> = self.headers.iter().collect();
        ordered.sort_unstable_by(|a, b| {
            a.name
                .bytes()
                .map(|b| b.to_ascii_lowercase())
                .cmp(b.name.bytes().map(|b| b.to_ascii_lowercase()))
        });
        let size = 8 + ordered
            .iter()
            .map(|h| 8 + h.name.len() + h.value.len())
            .sum::<usize>();
        let mut bytes = Vec::with_capacity(size);
        bytes.extend_from_slice(b"MGH1");
        encode_length(&mut bytes, ordered.len());
        for header in ordered {
            encode_length(&mut bytes, header.name.len());
            bytes.extend(header.name.bytes().map(|b| b.to_ascii_lowercase()));
            encode_length(&mut bytes, header.value.len());
            bytes.extend_from_slice(&header.value);
        }
        bytes.into_boxed_slice()
    }
}

fn encode_length(output: &mut Vec<u8>, length: usize) {
    // All call sites use admitted count/field limits below u32::MAX.
    output.extend_from_slice(
        &u32::try_from(length)
            .expect("admitted header length")
            .to_be_bytes(),
    );
}

enum Value<'a> {
    Raw(&'a [u8]),
    Bearer(&'a [u8]),
}
impl Value<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Raw(bytes) | Self::Bearer(bytes) => bytes,
        }
    }
    fn prefix(&self) -> &[u8] {
        match self {
            Self::Raw(_) => b"",
            Self::Bearer(_) => b"Bearer ",
        }
    }
}

struct Pending<'a> {
    headers: Vec<(&'a str, Value<'a>)>,
    bytes: usize,
}
impl<'a> Pending<'a> {
    fn new(count: usize) -> Self {
        Self {
            headers: Vec::with_capacity(count),
            bytes: 0,
        }
    }
    fn push(&mut self, name: &'a str, value: Value<'a>) -> Result<(), McpHeaderError> {
        if self.headers.len() == MAX_HEADERS
            || name.len() > MAX_HEADER_FIELD_BYTES
            || value.bytes().len() > MAX_HEADER_FIELD_BYTES - value.prefix().len()
        {
            return Err(McpHeaderError::Limit);
        }
        let bytes = name.len() + value.bytes().len() + value.prefix().len();
        if bytes > MAX_HEADER_BYTES - self.bytes {
            return Err(McpHeaderError::Limit);
        }
        validate_name(name)?;
        if value
            .bytes()
            .iter()
            .any(|&b| (b < 0x20 && b != b'\t') || b == 0x7f)
        {
            return Err(McpHeaderError::InvalidValue);
        }
        if self
            .headers
            .iter()
            .any(|(prior, _)| prior.eq_ignore_ascii_case(name))
        {
            return Err(McpHeaderError::Duplicate);
        }
        self.bytes += bytes;
        self.headers.push((name, value));
        Ok(())
    }
    fn finish(self) -> McpResolvedHeaders {
        let headers: Vec<_> = self
            .headers
            .into_iter()
            .map(|(name, value)| {
                let mut bytes = Vec::with_capacity(value.prefix().len() + value.bytes().len());
                bytes.extend_from_slice(value.prefix());
                bytes.extend_from_slice(value.bytes());
                Header {
                    name: name.into(),
                    value: bytes.into_boxed_slice(),
                }
            })
            .collect();
        McpResolvedHeaders {
            headers: headers.into(),
        }
    }
}

fn validate_name(name: &str) -> Result<(), McpHeaderError> {
    const RESERVED: &[&str] = &[
        "accept",
        "accept-encoding",
        "connection",
        "content-length",
        "content-type",
        "host",
        "last-event-id",
        "mcp-method",
        "mcp-name",
        "mcp-protocol-version",
        "mcp-session-id",
        "transfer-encoding",
    ];
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
    {
        return Err(McpHeaderError::InvalidName);
    }
    if RESERVED
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
        || name
            .get(..10)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("mcp-param-"))
    {
        return Err(McpHeaderError::Reserved);
    }
    Ok(())
}
