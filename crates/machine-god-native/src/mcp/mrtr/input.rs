use super::{
    Error, McpElicitationAction, McpElicitationMode, McpMrtrLimits, Result, bounds,
    elicitation::McpElicitationRequest, sampling,
};
use crate::mcp::protocol::ProtocolVersion;
use serde_json::value::RawValue;
use std::{collections::BTreeMap, fmt, sync::Arc};

/// A closed method union, with no implementation or transport attached.
pub enum McpInputRequestPayload {
    Sampling { params: Box<RawValue> },
    Roots { params: Option<Box<RawValue>> },
    // Shared unchanged into human presentation: reparsing would incorrectly
    // replace inherited MRTR bounds with standalone form limits. The existing
    // 256-byte per-node retained charge includes the Arc control block.
    Elicitation(Arc<McpElicitationRequest>),
}
impl McpInputRequestPayload {
    #[must_use]
    pub const fn method(&self) -> &'static str {
        match self {
            Self::Sampling { .. } => "sampling/createMessage",
            Self::Roots { .. } => "roots/list",
            Self::Elicitation(_) => "elicitation/create",
        }
    }
    #[must_use]
    pub fn params_json(&self) -> Option<&RawValue> {
        match self {
            Self::Sampling { params } => Some(params),
            Self::Roots { params } => params.as_deref(),
            Self::Elicitation(request) => Some(request.raw_params_json()),
        }
    }
}
impl fmt::Debug for McpInputRequestPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.method())
    }
}
pub struct McpInputRequest {
    key: Box<str>,
    payload: McpInputRequestPayload,
}
impl McpInputRequest {
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub const fn payload(&self) -> &McpInputRequestPayload {
        &self.payload
    }
}
impl fmt::Debug for McpInputRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.payload.fmt(f)
    }
}

pub struct McpInputRequired {
    raw: Box<RawValue>,
    requests: Box<[McpInputRequest]>,
    state: Option<Box<RawValue>>,
    legacy: bool,
    limits: McpMrtrLimits,
    retained_bytes: usize,
}
impl fmt::Debug for McpInputRequired {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpInputRequired")
            .field("request_count", &self.requests.len())
            .field("legacy", &self.legacy)
            .finish_non_exhaustive()
    }
}
impl McpInputRequired {
    /// Admit a modern input-required result. The caller owns envelope/resultType
    /// correlation; this entry independently bounds all JSON, including state.
    /// # Errors
    /// Rejects malformed, duplicate, unknown-method or over-budget input.
    pub fn parse(result: &RawValue, limits: McpMrtrLimits) -> Result<Self> {
        let retained_bytes = bounds::admit(result, limits)?;
        let object = bounds::object(result)?;
        if !object.contains_key("inputRequests") && !object.contains_key("requestState") {
            return Err(Error::InvalidRequest);
        }
        let requests = object
            .get("inputRequests")
            .map(|value| parse_requests(value, limits))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            raw: super::raw(result),
            requests: requests.into_boxed_slice(),
            state: object.get("requestState").map(|value| super::raw(value)),
            legacy: false,
            limits,
            retained_bytes,
        })
    }
    /// Admit data from the pinned -32042 URL-required error, only on 2025-11-25.
    /// This flag describes wire behavior, not browser consent or retry proof.
    /// # Errors
    /// Older/modern protocols, duplicate IDs, forms and malformed URLs fail.
    pub fn parse_legacy_url_required(
        data: &RawValue,
        version: ProtocolVersion,
        limits: McpMrtrLimits,
    ) -> Result<Self> {
        let retained_bytes = bounds::admit(data, limits)?;
        if version != ProtocolVersion::Legacy20251125 {
            return Err(Error::UnsupportedMode);
        }
        let object = bounds::object(data)?;
        let entries = bounds::array(bounds::required(&object, "elicitations")?)?;
        if entries.is_empty() || entries.len() > limits.max_requests {
            return Err(Error::InvalidRequest);
        }
        let mut requests = Vec::with_capacity(entries.len());
        for params in entries {
            let request = McpElicitationRequest::parse(params, version, limits)?;
            if request.mode() != McpElicitationMode::Url {
                return Err(Error::InvalidRequest);
            }
            let key = request.elicitation_id().ok_or(Error::InvalidRequest)?;
            if key.is_empty() {
                return Err(Error::InvalidRequest);
            }
            if requests
                .iter()
                .any(|prior: &McpInputRequest| prior.key() == key)
            {
                return Err(Error::InvalidRequest);
            }
            requests.push(McpInputRequest {
                key: key.into(),
                payload: McpInputRequestPayload::Elicitation(Arc::new(request)),
            });
        }
        let required = Self {
            raw: super::raw(data),
            requests: requests.into_boxed_slice(),
            state: None,
            legacy: true,
            limits,
            retained_bytes,
        };
        // The pin renders and reparses this map before handing it to a UI.
        // Rendering adds method/key overhead to the original error-data array.
        required.render_requests_json()?;
        Ok(required)
    }
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
    #[must_use]
    pub fn requests(&self) -> &[McpInputRequest] {
        &self.requests
    }
    #[must_use]
    pub fn request_state_json(&self) -> Option<&RawValue> {
        self.state.as_deref()
    }
    #[must_use]
    pub const fn legacy_retry_without_responses(&self) -> bool {
        self.legacy
    }
    #[must_use]
    pub const fn retained_byte_charge(&self) -> usize {
        self.retained_bytes
    }
    /// Produce the bounded typed request map for a UI adapter, never a request
    /// to execute sampling/roots or a proof to submit a continuation.
    /// # Errors
    /// The rendered map must fit the same JSON budget as incoming data.
    pub fn render_requests_json(&self) -> Result<Box<RawValue>> {
        #[derive(serde::Serialize)]
        struct Entry<'a> {
            method: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            params: Option<&'a RawValue>,
        }
        struct Requests<'a>(&'a [McpInputRequest]);
        impl serde::Serialize for Requests<'_> {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(self.0.len()))?;
                for request in self.0 {
                    map.serialize_entry(
                        request.key(),
                        &Entry {
                            method: request.payload.method(),
                            params: request.payload.params_json(),
                        },
                    )?;
                }
                map.end()
            }
        }
        super::encode(&Requests(&self.requests), self.limits)
    }
    /// Validate exact response keys and the result for each closed method.
    /// The returned object is inert and does not consume this input request.
    /// # Errors
    /// Rejects missing/extra keys, malformed method results and budget failures.
    pub fn validate_responses(&self, responses: &RawValue) -> Result<McpValidatedResponses> {
        let retained_bytes = bounds::admit(responses, self.limits)?;
        let object = bounds::object(responses).map_err(|_| Error::InvalidResponse)?;
        if object.len() != self.requests.len() {
            return Err(Error::InvalidResponse);
        }
        let mut entries = Vec::with_capacity(object.len());
        for request in &self.requests {
            let response = object.get(request.key()).ok_or(Error::InvalidResponse)?;
            let (kind, wire) = match request.payload() {
                McpInputRequestPayload::Sampling { .. } => {
                    sampling::result(response, self.limits).map_err(|_| Error::InvalidResponse)?;
                    (McpInputResponseKind::Sampling, super::raw(response))
                }
                McpInputRequestPayload::Roots { .. } => {
                    sampling::roots_result(response, self.limits)
                        .map_err(|_| Error::InvalidResponse)?;
                    (McpInputResponseKind::Roots, super::raw(response))
                }
                McpInputRequestPayload::Elicitation(request) => {
                    let (action, wire) = request.response_admitted(response)?;
                    (McpInputResponseKind::Elicitation(action), wire)
                }
            };
            entries.push(McpInputResponse {
                key: request.key().into(),
                raw: super::raw(response),
                wire,
                kind,
            });
        }
        let map: BTreeMap<_, _> = entries
            .iter()
            .map(|entry| (entry.key.as_ref(), entry.wire.as_ref()))
            .collect();
        let wire = super::encode(&map, self.limits)?;
        Ok(McpValidatedResponses {
            raw: super::raw(responses),
            wire,
            entries: entries.into_boxed_slice(),
            retained_bytes,
        })
    }
}
fn parse_requests(raw: &RawValue, limits: McpMrtrLimits) -> Result<Vec<McpInputRequest>> {
    let object = bounds::entries(raw)?;
    if object.len() > limits.max_requests {
        return Err(Error::Limit);
    }
    let mut requests = Vec::with_capacity(object.len());
    for (key, raw) in object {
        if key.is_empty() {
            return Err(Error::InvalidRequest);
        }
        let request = bounds::object(raw)?;
        let method = bounds::text(
            bounds::required(&request, "method")?,
            limits.max_string_bytes,
        )?;
        let payload = match method.as_ref() {
            "sampling/createMessage" => {
                let params = bounds::required(&request, "params")?;
                sampling::params(params, limits)?;
                McpInputRequestPayload::Sampling {
                    params: super::raw(params),
                }
            }
            "roots/list" => {
                if let Some(params) = request.get("params") {
                    let fields = bounds::object(params)?;
                    if let Some(meta) = fields.get("_meta") {
                        bounds::object(meta)?;
                    }
                }
                McpInputRequestPayload::Roots {
                    params: request.get("params").map(|value| super::raw(value)),
                }
            }
            "elicitation/create" => McpInputRequestPayload::Elicitation(Arc::new(
                McpElicitationRequest::parse_admitted(
                    bounds::required(&request, "params")?,
                    ProtocolVersion::Modern,
                    limits,
                )?,
            )),
            _ => return Err(Error::InvalidRequest),
        };
        requests.push(McpInputRequest {
            key: key.into_boxed_str(),
            payload,
        });
    }
    Ok(requests)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpInputResponseKind {
    Sampling,
    Roots,
    Elicitation(McpElicitationAction),
}
pub struct McpInputResponse {
    key: Box<str>,
    raw: Box<RawValue>,
    wire: Box<RawValue>,
    kind: McpInputResponseKind,
}
impl McpInputResponse {
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
    #[must_use]
    pub const fn kind(&self) -> McpInputResponseKind {
        self.kind
    }
}
impl fmt::Debug for McpInputResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)
    }
}
pub struct McpValidatedResponses {
    raw: Box<RawValue>,
    wire: Box<RawValue>,
    entries: Box<[McpInputResponse]>,
    retained_bytes: usize,
}
impl McpValidatedResponses {
    #[must_use]
    pub fn raw_json(&self) -> &RawValue {
        &self.raw
    }
    #[must_use]
    pub fn wire_json(&self) -> &RawValue {
        &self.wire
    }
    #[must_use]
    pub fn responses(&self) -> &[McpInputResponse] {
        &self.entries
    }
    #[must_use]
    pub const fn retained_byte_charge(&self) -> usize {
        self.retained_bytes
    }
}
impl fmt::Debug for McpValidatedResponses {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpValidatedResponses")
            .field("count", &self.entries.len())
            .finish_non_exhaustive()
    }
}
