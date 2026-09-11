use super::{DOCUMENT_LIMIT, McpAuthError, Result, SECRET_LIMIT, redacted};
use crate::mcp::config::McpRemoteConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use url::Url;

mod challenge;
pub use challenge::McpAuthChallenge;

/// Exact selected endpoint and client/auth configuration digest. The endpoint
/// includes its query; canonical resource identity deliberately does not.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct McpAuthIdentity {
    pub(super) endpoint: Box<str>,
    pub(super) selection: Box<str>,
}
pub struct McpAuthConfig {
    pub(super) identity: McpAuthIdentity,
    pub(super) resource: Box<str>,
    pub(super) issuer: Option<Box<str>>,
    pub(super) client_id: Option<Box<str>>,
    pub(super) client_secret: Option<Secret>,
    pub(super) client_metadata_url: Option<Box<str>>,
    pub(super) scopes: Box<[Box<str>]>,
}
impl McpAuthConfig {
    /// Captured environment and authentication identity are supplied explicitly.
    /// No environment read or credential file access occurs.
    /// # Errors
    /// Rejects missing selected secrets, invalid URLs and finite input bounds.
    pub fn new<'a>(
        remote: &McpRemoteConfig,
        authentication_identity: &[u8],
        mut environment: impl FnMut(&str) -> Option<&'a [u8]>,
    ) -> Result<Self> {
        if authentication_identity.len() > crate::mcp::headers::MAX_AUTHENTICATION_IDENTITY_BYTES {
            return Err(McpAuthError::Limit);
        }
        secure_url(remote.url())?;
        let oauth = remote.oauth();
        let resource = canonical(oauth.and_then(|v| v.resource()).unwrap_or(remote.url()))?;
        let issuer = oauth.and_then(|v| v.issuer()).map(str::to_owned);
        if let Some(value) = &issuer {
            oauth_url(value, &resource)?;
        }
        let client_id = oauth.and_then(|v| v.client_id()).map(Into::into);
        let client_secret = oauth
            .and_then(|v| v.client_secret_env())
            .map(|key| {
                environment(key)
                    .ok_or(McpAuthError::Missing)
                    .and_then(Secret::new)
            })
            .transpose()?;
        let client_metadata_url = oauth.and_then(|v| v.client_metadata_url()).map(Into::into);
        let scopes = oauth.map_or_else(|| Box::new([]) as Box<[Box<str>]>, |v| v.scopes().into());
        let mut digest = Sha256::new();
        digest.update(authentication_identity.len().to_be_bytes());
        digest.update(authentication_identity);
        let selected = serde_json::to_vec(&oauth).map_err(|_| McpAuthError::Invalid)?;
        digest.update(selected.len().to_be_bytes());
        digest.update(selected);
        if let Some(secret) = &client_secret {
            digest.update(secret.bytes());
        }
        let selection =
            digest
                .finalize()
                .iter()
                .fold(String::with_capacity(64), |mut output, byte| {
                    const HEX: &[u8] = b"0123456789abcdef";
                    output.push(char::from(HEX[usize::from(byte >> 4)]));
                    output.push(char::from(HEX[usize::from(byte & 15)]));
                    output
                });
        Ok(Self {
            identity: McpAuthIdentity {
                endpoint: remote.url().into(),
                selection: selection.into(),
            },
            resource,
            issuer: issuer.map(Into::into),
            client_id,
            client_secret,
            client_metadata_url,
            scopes,
        })
    }
    #[must_use]
    pub fn identity(&self) -> &McpAuthIdentity {
        &self.identity
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct Secret(String);
impl Secret {
    pub(super) fn new(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > SECRET_LIMIT || bytes.iter().any(u8::is_ascii_control)
        {
            return Err(McpAuthError::Invalid);
        }
        Ok(Self(
            std::str::from_utf8(bytes)
                .map_err(|_| McpAuthError::Invalid)?
                .into(),
        ))
    }
    pub(super) fn text(&self) -> &str {
        &self.0
    }
    pub(super) fn bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
    }
}
redacted!(
    Secret,
    McpAuthIdentity,
    McpAuthConfig,
    Credentials,
    Metadata,
    Registration
);

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Credentials {
    pub identity: McpAuthIdentity,
    pub resource: Box<str>,
    pub issuer: Box<str>,
    pub registration: Registration,
    pub access: Secret,
    pub refresh: Option<Secret>,
    pub scope: Box<str>,
    pub expires_ms: i64,
    pub authorization_endpoint: Box<str>,
    pub token_endpoint: Box<str>,
    pub revocation_endpoint: Option<Box<str>>,
}
impl Credentials {
    pub(super) fn validate(&self) -> Result<()> {
        secure_url(&self.identity.endpoint)?;
        if self.identity.selection.len() != 64
            || !self
                .identity
                .selection
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(McpAuthError::Invalid);
        }
        canonical(&self.resource)?;
        for url in [
            &self.issuer,
            &self.authorization_endpoint,
            &self.token_endpoint,
        ] {
            oauth_url(url, &self.resource)?;
        }
        if let Some(url) = &self.revocation_endpoint {
            oauth_url(url, &self.resource)?;
        }
        self.registration.validate()?;
        Secret::new(self.access.bytes())?;
        if let Some(value) = &self.refresh {
            Secret::new(value.bytes())?;
        }
        scopes(&[], Some(&self.scope), &[], None, false)?;
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Registration {
    pub id: Secret,
    pub secret: Option<Secret>,
    pub method: Box<str>,
}
impl Registration {
    pub(super) fn validate(&self) -> Result<()> {
        Secret::new(self.id.bytes())?;
        if let Some(secret) = &self.secret {
            Secret::new(secret.bytes())?;
        }
        match self.method.as_ref() {
            "none" => Ok(()),
            "client_secret_basic" | "client_secret_post" if self.secret.is_some() => Ok(()),
            _ => Err(McpAuthError::Invalid),
        }
    }
}
pub(super) struct Metadata {
    pub issuer: Box<str>,
    pub authorization: Box<str>,
    pub token: Box<str>,
    pub registration: Option<Box<str>>,
    pub revocation: Option<Box<str>>,
    pub scopes: Vec<Box<str>>,
    pub grants: Vec<Box<str>>,
    pub methods: Vec<Box<str>>,
    pub client_document: bool,
    pub require_issuer: bool,
}

pub(super) fn json(bytes: &[u8]) -> Result<Value> {
    if bytes.len() > DOCUMENT_LIMIT {
        return Err(McpAuthError::Limit);
    }
    let value = machine_god_core::json::from_slice(bytes).map_err(|_| McpAuthError::Invalid)?;
    if !value.is_object() {
        return Err(McpAuthError::Invalid);
    }
    Ok(value)
}
pub(super) fn required<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= SECRET_LIMIT)
        .ok_or(McpAuthError::Invalid)
}
pub(super) fn optional(value: &Value, field: &str) -> Result<Option<Box<str>>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        _ => required(value, field).map(|s| Some(s.into())),
    }
}
pub(super) fn strings(value: &Value, field: &str) -> Result<Vec<Box<str>>> {
    let Some(value) = value.get(field) else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or(McpAuthError::Invalid)?;
    if values.len() > 64 {
        return Err(McpAuthError::Limit);
    }
    values
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty() && s.len() <= 4096)
                .map(Into::into)
                .ok_or(McpAuthError::Invalid)
        })
        .collect()
}
pub(super) fn secure_url(value: &str) -> Result<Url> {
    if value.len() > 4096 || value.bytes().any(|b| b.is_ascii_control() || b == b' ') {
        return Err(McpAuthError::Invalid);
    }
    let url = Url::parse(value).map_err(|_| McpAuthError::Invalid)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || !(url.scheme() == "https"
            || (url.scheme() == "http" && loopback(&url) && explicit_port(value)))
    {
        return Err(McpAuthError::Invalid);
    }
    Ok(url)
}
fn explicit_port(value: &str) -> bool {
    value.split_once("://").is_some_and(|(_, rest)| {
        rest.split(['/', '?', '#']).next().is_some_and(|authority| {
            authority.rsplit_once(':').is_some_and(|(_, port)| {
                !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
            })
        })
    })
}
pub(super) fn loopback(url: &Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}
pub(super) fn oauth_url(value: &str, resource: &str) -> Result<Url> {
    let url = secure_url(value)?;
    if url.scheme() == "http" && !loopback(&secure_url(resource)?) {
        return Err(McpAuthError::Invalid);
    }
    Ok(url)
}
pub(super) fn canonical(value: &str) -> Result<Box<str>> {
    let url = secure_url(value)?;
    Ok(format!("{}{}", origin(&url), url.path()).into())
}
pub(super) fn origin(url: &Url) -> String {
    if url.scheme() == "http" {
        format!(
            "http://{}:{}",
            url.host_str().unwrap_or_default(),
            url.port().unwrap_or(80)
        )
    } else {
        url.origin().ascii_serialization()
    }
}
pub(super) fn covers(resource: &str, endpoint: &str) -> bool {
    resource == endpoint
        || endpoint
            .strip_prefix(resource)
            .is_some_and(|suffix| resource.ends_with('/') || suffix.starts_with('/'))
}
pub(super) fn scopes(
    configured: &[Box<str>],
    challenge: Option<&str>,
    metadata: &[Box<str>],
    previous: Option<&str>,
    offline: bool,
) -> Result<Box<str>> {
    let mut tokens: Vec<&str> = Vec::new();
    let source: Vec<&str> = previous
        .into_iter()
        .chain(match challenge {
            Some(value) => vec![value],
            None => if configured.is_empty() {
                metadata
            } else {
                configured
            }
            .iter()
            .map(AsRef::as_ref)
            .collect(),
        })
        .chain(offline.then_some("offline_access"))
        .collect();
    for token in source.iter().flat_map(|s| s.split_ascii_whitespace()) {
        if token.len() > 256
            || !token
                .bytes()
                .all(|b| b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b))
        {
            return Err(McpAuthError::Invalid);
        }
        if !tokens.contains(&token) {
            if tokens.len() == 64 {
                return Err(McpAuthError::Limit);
            }
            tokens.push(token);
        }
    }
    Ok(tokens.join(" ").into())
}
