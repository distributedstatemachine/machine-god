use super::{Error, McpFeatureExchange, Result, charge, content};
use crate::McpFeatureAction as Action;
use crate::mcp::{
    catalog::{McpDescriptorCatalog, McpDescriptorLimits, fields},
    pagination::{McpCatalogBuilder, McpCatalogCacheScope, McpCatalogKind, McpCatalogLimits},
    protocol::{ProtocolVersion, RpcKind, WireLimits, parse_envelope},
    schema::{McpSchema, McpSchemaLimits, McpSchemaValidation, McpSchemaViolation},
};
use serde_json::value::RawValue;
use std::fmt;

#[derive(Clone, Copy, Debug)]
pub struct McpFeatureCacheHints {
    pub ttl_ms: Option<u64>,
    pub scope: McpCatalogCacheScope,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpPromptRole {
    User,
    Assistant,
}
pub struct McpPromptMessage {
    raw: Box<RawValue>,
    role: McpPromptRole,
    content: content::McpContent,
}
impl fmt::Debug for McpPromptMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpPromptMessage")
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}
impl McpPromptMessage {
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
    #[must_use]
    pub fn role(&self) -> McpPromptRole {
        self.role
    }
    #[must_use]
    pub fn content(&self) -> &content::McpContent {
        &self.content
    }
}
pub enum McpFeatureOutcome {
    Resource {
        contents: Box<[content::McpResourceContent]>,
        cache: McpFeatureCacheHints,
    },
    Prompt {
        description: Option<Box<str>>,
        messages: Box<[McpPromptMessage]>,
        cache: McpFeatureCacheHints,
    },
    Completion {
        values: Box<[Box<str>]>,
        total: Option<u64>,
        has_more: Option<bool>,
    },
    ProtocolFailure {
        code: i64,
    },
    /// Correlated, size-bounded data only. A separate MRTR decoder must validate
    /// requests/state and obtain explicit consent before any continuation.
    UnvalidatedInputRequired,
}
impl fmt::Debug for McpFeatureOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpFeatureOutcome { .. }")
    }
}
pub struct McpFeatureResponse {
    raw: Box<RawValue>,
    result: Box<RawValue>,
    outcome: McpFeatureOutcome,
}
impl fmt::Debug for McpFeatureResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpFeatureResponse { .. }")
    }
}
impl McpFeatureResponse {
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
    /// Complete original result/error object, including unknown metadata.
    #[must_use]
    pub fn result_json(&self) -> &RawValue {
        &self.result
    }
    #[must_use]
    pub fn outcome(&self) -> &McpFeatureOutcome {
        &self.outcome
    }
}
impl McpFeatureExchange {
    /// Admits only this exchange's correlated read/get/completion response.
    /// List exchanges instead use `McpFeatureCatalogLoad` for atomic pagination.
    /// # Errors
    /// Rejects malformed/cross-request responses and all bounded-resource errors.
    pub fn admit_response(&self, bytes: &[u8]) -> Result<McpFeatureResponse> {
        if super::request::list_kind(self.request.action()).is_some() {
            return Err(Error::InvalidRequest);
        }
        let kind = envelope(self, bytes)?;
        let raw: &RawValue = serde_json::from_slice(bytes).map_err(|_| Error::InvalidResponse)?;
        let object = fields::object(raw)?;
        let result = *object
            .get(if kind == RpcKind::Error {
                "error"
            } else {
                "result"
            })
            .ok_or(Error::InvalidResponse)?;
        let fields = fields::object(result)?;
        let mut retained = 0;
        charge(
            &mut retained,
            bytes.len().checked_mul(5).ok_or(Error::Limit)?,
            self.limits.max_retained_bytes,
        )?;
        let outcome = if kind == RpcKind::Error {
            let code: i64 =
                serde_json::from_str(fields.get("code").ok_or(Error::InvalidResponse)?.get())
                    .map_err(|_| Error::InvalidResponse)?;
            fields::optional(&fields, "message", 64 * 1024)?.ok_or(Error::InvalidResponse)?;
            if let Some(data) = fields.get("data") {
                fields::metadata(data, 32, false)?;
            }
            McpFeatureOutcome::ProtocolFailure { code }
        } else if fields::optional(&fields, "resultType", 32)?.as_deref() == Some("input_required")
        {
            if self.options.protocol.version != ProtocolVersion::Modern
                || !matches!(
                    self.request.action(),
                    Action::ResourceRead | Action::PromptGet
                )
            {
                return Err(Error::Unsupported);
            }
            fields::metadata(result, 32, true)?;
            McpFeatureOutcome::UnvalidatedInputRequired
        } else {
            if fields::optional(&fields, "resultType", 32)?
                .is_some_and(|value| value.as_ref() != "complete")
            {
                return Err(Error::Unsupported);
            }
            match self.request.action() {
                Action::ResourceRead => self.resource_result(&fields, &mut retained)?,
                Action::PromptGet => self.prompt_result(&fields, &mut retained)?,
                Action::PromptComplete | Action::ResourceComplete => {
                    let completion =
                        fields::object(fields.get("completion").ok_or(Error::InvalidResponse)?)?;
                    let items =
                        fields::array(completion.get("values").ok_or(Error::InvalidResponse)?)?;
                    if items.len() > 100 {
                        return Err(Error::Limit);
                    }
                    charge(
                        &mut retained,
                        items.len().checked_mul(64).ok_or(Error::Limit)?,
                        self.limits.max_retained_bytes,
                    )?;
                    let mut values = Vec::new();
                    let mut total_bytes = 0;
                    for raw in items {
                        let text: String =
                            serde_json::from_str(raw.get()).map_err(|_| Error::InvalidResponse)?;
                        if text.len() > 4096 {
                            return Err(Error::Limit);
                        }
                        charge(&mut total_bytes, text.len(), 64 * 1024)?;
                        values.push(text.into_boxed_str());
                    }
                    let total = completion
                        .get("total")
                        .map(|raw| fields::size(raw.get()))
                        .transpose()?;
                    let has_more = completion
                        .get("hasMore")
                        .map(|raw| fields::boolean(raw))
                        .transpose()?;
                    McpFeatureOutcome::Completion {
                        values: values.into_boxed_slice(),
                        total,
                        has_more,
                    }
                }
                _ => return Err(Error::InvalidRequest),
            }
        };
        Ok(McpFeatureResponse {
            raw: raw.to_owned(),
            result: result.to_owned(),
            outcome,
        })
    }
    fn resource_result(
        &self,
        fields: &fields::Object<'_>,
        retained: &mut usize,
    ) -> Result<McpFeatureOutcome> {
        let items = fields::array(fields.get("contents").ok_or(Error::InvalidResponse)?)?;
        self.charge_items(items.len(), retained)?;
        let mut total = 0;
        let contents = items
            .into_iter()
            .map(|raw| content::resource(raw, self.limits, &mut total))
            .collect::<Result<Vec<_>>>()?
            .into_boxed_slice();
        Ok(McpFeatureOutcome::Resource {
            contents,
            cache: cache(fields)?,
        })
    }
    fn prompt_result(
        &self,
        fields: &fields::Object<'_>,
        retained: &mut usize,
    ) -> Result<McpFeatureOutcome> {
        let description = fields::optional(fields, "description", 64 * 1024)?;
        let items = fields::array(fields.get("messages").ok_or(Error::InvalidResponse)?)?;
        self.charge_items(items.len(), retained)?;
        let mut total = description.as_ref().map_or(0, |text| text.len());
        if total > self.limits.max_content_bytes {
            return Err(Error::Limit);
        }
        let mut messages = Vec::new();
        for raw in items {
            let message = fields::object(raw)?;
            let role = match fields::required(&message, "role", 9)?.as_ref() {
                "user" => McpPromptRole::User,
                "assistant" => McpPromptRole::Assistant,
                _ => return Err(Error::InvalidResponse),
            };
            let content = content::admit(
                message.get("content").ok_or(Error::InvalidResponse)?,
                self.limits,
                &mut total,
            )?;
            messages.push(McpPromptMessage {
                raw: raw.to_owned(),
                role,
                content,
            });
        }
        Ok(McpFeatureOutcome::Prompt {
            description,
            messages: messages.into_boxed_slice(),
            cache: cache(fields)?,
        })
    }
    fn charge_items(&self, count: usize, retained: &mut usize) -> Result<()> {
        if count > self.limits.max_content_items {
            return Err(Error::Limit);
        }
        charge(
            retained,
            count.checked_mul(512).ok_or(Error::Limit)?,
            self.limits.max_retained_bytes,
        )
    }
}
fn envelope(exchange: &McpFeatureExchange, bytes: &[u8]) -> Result<RpcKind> {
    let envelope = parse_envelope(
        bytes,
        WireLimits {
            max_frame_bytes: exchange.limits.max_response_bytes,
            max_depth: 33,
            max_nodes: exchange.limits.max_nodes,
        },
    )
    .map_err(|_| Error::InvalidResponse)?;
    envelope
        .correlate(&exchange.request_id(), false)
        .map_err(|_| Error::Correlation)?;
    Ok(envelope.kind())
}
fn cache(fields: &fields::Object<'_>) -> Result<McpFeatureCacheHints> {
    let scope = match fields::optional(fields, "cacheScope", 7)?.as_deref() {
        None | Some("private") => McpCatalogCacheScope::Private,
        Some("public") => McpCatalogCacheScope::Public,
        _ => return Err(Error::InvalidResponse),
    };
    let ttl_ms = fields
        .get("ttlMs")
        .map(|raw| {
            let schema = McpSchema::parse(
                br#"{"type":"number","minimum":0}"#,
                McpSchemaLimits::default(),
            )
            .map_err(|_| Error::InvalidResponse)?;
            match schema
                .validate_json(raw.get().as_bytes())
                .map_err(|_| Error::InvalidResponse)?
            {
                McpSchemaValidation::Invalid(McpSchemaViolation::Minimum) => Ok(0),
                McpSchemaValidation::Valid => fields::size(raw.get()).map_err(Into::into),
                _ => Err(Error::InvalidResponse),
            }
        })
        .transpose()?;
    Ok(McpFeatureCacheHints { ttl_ms, scope })
}

/// Typed list-response staging. Only `finish` admits complete descriptors;
/// neither pages nor a finished catalog publish a live generation.
pub struct McpFeatureCatalogLoad {
    kind: McpCatalogKind,
    version: ProtocolVersion,
    server: Box<str>,
    builder: Option<McpCatalogBuilder>,
    limits: McpDescriptorLimits,
}
impl fmt::Debug for McpFeatureCatalogLoad {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpFeatureCatalogLoad")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl McpFeatureCatalogLoad {
    /// # Errors
    /// Rejects non-list actions and invalid builder limits.
    pub fn new(
        exchange: &McpFeatureExchange,
        pagination: McpCatalogLimits,
        descriptors: McpDescriptorLimits,
    ) -> Result<Self> {
        let kind =
            super::request::list_kind(exchange.request.action()).ok_or(Error::InvalidRequest)?;
        let version = exchange.options.protocol.version;
        let builder =
            McpCatalogBuilder::new(kind, version, pagination).map_err(|_| Error::InvalidLimits)?;
        Ok(Self {
            kind,
            version,
            server: exchange.request.server().into(),
            builder: Some(builder),
            limits: descriptors,
        })
    }
    /// # Errors
    /// Any malformed page, correlation, cursor or family error closes the load.
    pub fn append(
        &mut self,
        exchange: &McpFeatureExchange,
        bytes: &[u8],
        received_at_ms: u64,
    ) -> Result<bool> {
        let mut builder = self.builder.take().ok_or(Error::Closed)?;
        if exchange.request.server() != self.server.as_ref()
            || super::request::list_kind(exchange.request.action()) != Some(self.kind)
            || exchange.options.protocol.version != self.version
        {
            return Err(Error::InvalidRequest);
        }
        if envelope(exchange, bytes)? != RpcKind::Success {
            return Err(Error::InvalidResponse);
        }
        let more = builder
            .append_response(
                bytes,
                &exchange.request_id(),
                exchange.cursor.as_deref(),
                received_at_ms,
            )
            .map_err(|_| Error::InvalidResponse)?;
        self.builder = Some(builder);
        Ok(more)
    }
    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.builder
            .as_ref()
            .and_then(McpCatalogBuilder::next_cursor)
    }
    /// # Errors
    /// Rejects incomplete/failed pagination, malformed descriptors and budgets.
    pub fn finish(self) -> Result<McpDescriptorCatalog> {
        let raw = self
            .builder
            .ok_or(Error::Closed)?
            .finish()
            .map_err(|_| Error::InvalidResponse)?;
        McpDescriptorCatalog::admit(raw, self.limits).map_err(Into::into)
    }
}
