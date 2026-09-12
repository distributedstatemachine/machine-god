use super::{McpHttpError, Result};
use crate::mcp::{stdio::McpStdioControl, submission::McpSubmissionHttpHead};
use std::fmt;

/// Explicit protocol-only HTTP authority, reusing the stdio control admission.
pub struct McpHttpControl(Kind);
enum Kind {
    Protocol(McpStdioControl),
    OAuth { body: Option<Box<[u8]>>, form: bool },
}
impl fmt::Debug for McpHttpControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpHttpControl { <redacted> }")
    }
}
impl McpHttpControl {
    pub(crate) fn feature(
        exchange: &crate::mcp::feature::McpFeatureExchange,
        guard: crate::mcp::control::McpFeatureControlAuthority,
    ) -> Result<Self> {
        Ok(Self(Kind::Protocol(
            McpStdioControl::feature(exchange, guard).map_err(|_| McpHttpError::Cancelled)?,
        )))
    }
    pub(crate) fn feature_guard(&self) -> Option<crate::mcp::control::McpFeatureControlAuthority> {
        match &self.0 {
            Kind::Protocol(control) => control.feature_guard(),
            _ => None,
        }
    }
    // Private OAuth lane: only the native auth codec selects destinations and
    // constructs these fixed GET/registration/token/revocation operations.
    pub(crate) fn oauth_metadata() -> Self {
        Self(Kind::OAuth {
            body: None,
            form: false,
        })
    }
    pub(crate) fn oauth_json(body: Box<[u8]>) -> Result<Self> {
        if body.len() > 256 * 1024 {
            return Err(McpHttpError::Limit);
        }
        Ok(Self(Kind::OAuth {
            body: Some(body),
            form: false,
        }))
    }
    pub(crate) fn oauth_form(body: Box<[u8]>) -> Result<Self> {
        if body.len() > 256 * 1024 {
            return Err(McpHttpError::Limit);
        }
        Ok(Self(Kind::OAuth {
            body: Some(body),
            form: true,
        }))
    }
    /// # Errors
    /// Rejects any method outside the bounded discovery/catalog lane.
    pub fn discovery(bytes: &[u8]) -> Result<Self> {
        McpStdioControl::discovery(bytes)
            .map(|value| Self(Kind::Protocol(value)))
            .map_err(|_| McpHttpError::Invalid)
    }
    pub(super) fn encode(&self, head: &McpSubmissionHttpHead) -> Result<Box<[u8]>> {
        match &self.0 {
            Kind::OAuth { body, form } => {
                let post = head
                    .encode(body.as_deref().unwrap_or_default())
                    .map_err(|_| McpHttpError::Invalid)?;
                let split = post
                    .windows(4)
                    .position(|part| part == b"\r\n\r\n")
                    .ok_or(McpHttpError::Invalid)?;
                let mut bytes = Vec::with_capacity(post.len());
                bytes.extend_from_slice(if body.is_some() { b"POST " } else { b"GET " });
                for (index, line) in post[5..split + 2]
                    .split_inclusive(|b| *b == b'\n')
                    .enumerate()
                {
                    if index != 0 && line.starts_with(b"accept:") {
                        bytes.extend_from_slice(b"accept: application/json\r\n");
                    } else if index != 0 && line.starts_with(b"content-type:") {
                        if body.is_some() {
                            bytes.extend_from_slice(if *form {
                                b"content-type: application/x-www-form-urlencoded\r\n"
                            } else {
                                b"content-type: application/json\r\n"
                            });
                        }
                    } else if index != 0 && line.starts_with(b"content-length:") && body.is_none() {
                    } else {
                        bytes.extend_from_slice(line);
                    }
                }
                bytes.extend_from_slice(b"\r\n");
                if let Some(body) = body {
                    bytes.extend_from_slice(body);
                }
                Ok(bytes.into_boxed_slice())
            }
            Kind::Protocol(control) => head
                .encode(control.json_bytes())
                .map_err(|_| McpHttpError::Invalid),
        }
    }
}
