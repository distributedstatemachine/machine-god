use super::{Error, McpMrtrLimits, Result, bounds, form::McpFormSchema, strings};
use crate::mcp::protocol::ProtocolVersion;
use serde_json::value::RawValue;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpElicitationMode {
    Form,
    Url,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum McpElicitationAction {
    Accept,
    Decline,
    Cancel,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpHostClassification {
    Ordinary,
    Punycode,
    NonAscii,
}

pub struct McpElicitationRequest {
    version: ProtocolVersion,
    mode: McpElicitationMode,
    message: Box<str>,
    raw: Box<RawValue>,
    form: Option<McpFormSchema>,
    url: Option<Box<str>>,
    host: Option<Box<[u8]>>,
    id: Option<Box<str>>,
    limits: McpMrtrLimits,
}
impl fmt::Debug for McpElicitationRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpElicitationRequest")
            .field("version", &self.version)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}
impl McpElicitationRequest {
    /// Admit inert elicitation parameters, including direct legacy form requests.
    /// # Errors
    /// Unsupported revision/mode, unsafe form fields, invalid URL and bounds fail.
    pub fn parse(
        params: &RawValue,
        version: ProtocolVersion,
        limits: McpMrtrLimits,
    ) -> Result<Self> {
        bounds::admit(params, limits)?;
        Self::parse_bounded(params, version, limits, bounds::FormBounds::direct(limits))
    }
    pub(super) fn parse_admitted(
        params: &RawValue,
        version: ProtocolVersion,
        limits: McpMrtrLimits,
    ) -> Result<Self> {
        Self::parse_bounded(params, version, limits, bounds::FormBounds::nested(limits))
    }
    fn parse_bounded(
        params: &RawValue,
        version: ProtocolVersion,
        limits: McpMrtrLimits,
        form_bounds: bounds::FormBounds,
    ) -> Result<Self> {
        if !matches!(
            version,
            ProtocolVersion::Modern
                | ProtocolVersion::Legacy20250618
                | ProtocolVersion::Legacy20251125
        ) {
            return Err(Error::UnsupportedMode);
        }
        let fields = bounds::object(params)?;
        let mode = match fields.get("mode") {
            None => McpElicitationMode::Form,
            Some(_) if version == ProtocolVersion::Legacy20250618 => {
                return Err(Error::UnsupportedMode);
            }
            Some(raw) => match bounds::text(raw, limits.max_name_bytes)?.as_ref() {
                "form" => McpElicitationMode::Form,
                "url" => McpElicitationMode::Url,
                _ => return Err(Error::UnsupportedMode),
            },
        };
        let message = bounds::text(bounds::required(&fields, "message")?, form_bounds.message)?;
        let (form, url, host, id) = match mode {
            McpElicitationMode::Form => (
                Some(McpFormSchema::parse_admitted(
                    bounds::required(&fields, "requestedSchema")?,
                    version,
                    limits,
                    form_bounds,
                )?),
                None,
                None,
                None,
            ),
            McpElicitationMode::Url => {
                if fields.contains_key("requestedSchema") {
                    return Err(Error::InvalidRequest);
                }
                let url = bounds::text(bounds::required(&fields, "url")?, limits.max_string_bytes)?;
                let host = strings::url_host(&url)?;
                let id = if version == ProtocolVersion::Legacy20251125 {
                    Some(bounds::text(
                        bounds::required(&fields, "elicitationId")?,
                        limits.max_name_bytes,
                    )?)
                } else {
                    if fields.contains_key("elicitationId") {
                        return Err(Error::InvalidRequest);
                    }
                    None
                };
                (None, Some(url), Some(host), id)
            }
        };
        Ok(Self {
            version,
            mode,
            message,
            raw: super::raw(params),
            form,
            url,
            host,
            id,
            limits,
        })
    }
    #[must_use]
    pub const fn version(&self) -> ProtocolVersion {
        self.version
    }
    #[must_use]
    pub const fn mode(&self) -> McpElicitationMode {
        self.mode
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
    #[must_use]
    pub fn raw_params_json(&self) -> &RawValue {
        &self.raw
    }
    #[must_use]
    pub const fn form_schema(&self) -> Option<&McpFormSchema> {
        self.form.as_ref()
    }
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
    #[must_use]
    pub fn url_host(&self) -> Option<&str> {
        self.url_host_bytes()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
    }
    /// Lossless decoded host bytes, including percent-encoded non-UTF-8.
    #[must_use]
    pub fn url_host_bytes(&self) -> Option<&[u8]> {
        self.host.as_deref()
    }
    #[must_use]
    pub fn elicitation_id(&self) -> Option<&str> {
        self.id.as_deref()
    }
    #[must_use]
    pub fn host_classification(&self) -> Option<McpHostClassification> {
        self.url_host_bytes().map(strings::classify_host)
    }
    /// Validate a response and return the minimal originating-wire response.
    /// Acceptance here is merely an action string, never evidence of consent.
    /// # Errors
    /// Invalid actions, invalid accepted form content or URL content fail.
    pub fn validate_response(
        &self,
        response: &RawValue,
    ) -> Result<(McpElicitationAction, Box<RawValue>)> {
        bounds::admit(response, self.limits)?;
        self.response_admitted(response)
    }
    pub(super) fn response_admitted(
        &self,
        response: &RawValue,
    ) -> Result<(McpElicitationAction, Box<RawValue>)> {
        #[derive(serde::Serialize)]
        struct Wire<'a> {
            action: McpElicitationAction,
            #[serde(skip_serializing_if = "Option::is_none")]
            content: Option<&'a RawValue>,
        }
        let fields = bounds::object(response).map_err(|_| Error::InvalidResponse)?;
        let action = match bounds::text(
            bounds::required(&fields, "action")?,
            self.limits.max_name_bytes,
        )?
        .as_ref()
        {
            "accept" => McpElicitationAction::Accept,
            "decline" => McpElicitationAction::Decline,
            "cancel" => McpElicitationAction::Cancel,
            _ => return Err(Error::InvalidResponse),
        };
        let content = if action == McpElicitationAction::Accept {
            if let Some(form) = &self.form {
                let value = fields
                    .get("content")
                    .copied()
                    .ok_or(Error::InvalidResponse)?;
                form.validate_admitted(value, self.limits)?;
                Some(value)
            } else {
                if fields.contains_key("content") {
                    return Err(Error::InvalidResponse);
                }
                None
            }
        } else {
            None
        };
        Ok((
            action,
            super::encode(&Wire { action, content }, self.limits)?,
        ))
    }
}
