//! Runtime-neutral codec for the pinned Vercel AI Gateway v3 wire contract.

use futures_core::Stream;
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, MAX_SAFE_JSON_DEPTH, Message, ModelEvent,
    ModelEventStream, ModelProvider, ModelRequest, ProviderError, ProviderErrorKind, Role,
    StopReason, TokenUsage, ToolCall, ToolCallId, ToolName,
};
use serde::Serialize;
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::future::{Future, poll_fn};
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

/// Stable provider identifier.
pub const AI_GATEWAY_PROVIDER_NAME: &str = "vercel_ai_gateway";
/// Built-in model used by the native host when configuration does not select one.
pub const AI_GATEWAY_DEFAULT_MODEL: &str = "zai/glm-5.2";
/// Maximum number of bytes accepted in an AI Gateway model identifier.
pub const AI_GATEWAY_MAX_MODEL_BYTES: usize = 128;
/// Pinned Gateway protocol version.
pub const AI_GATEWAY_PROTOCOL_VERSION: &str = "0.0.1";
/// Pinned language-model specification version.
pub const AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION: &str = "4";

const CONTENT_TYPE: &str = "application/json";

/// Byte stream supplied by an explicitly injected native transport.
pub type AiGatewayByteStream =
    Pin<Box<dyn Stream<Item = Result<Vec<u8>, ProviderError>> + Send + 'static>>;

/// Resource limits enforced independently by the adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AiGatewayLimits {
    pub max_request_bytes: usize,
    pub max_chunk_bytes: usize,
    pub max_record_bytes: usize,
    pub max_undecoded_bytes: usize,
    pub max_total_response_bytes: usize,
    pub max_records: usize,
    pub max_messages: usize,
    pub max_tools: usize,
    pub max_streamed_tool_calls: usize,
    pub max_tool_calls: usize,
    pub max_tool_arguments_bytes: usize,
    pub max_json_nodes: usize,
}

impl Default for AiGatewayLimits {
    fn default() -> Self {
        Self {
            max_request_bytes: 12 * 1024 * 1024,
            max_chunk_bytes: 1024 * 1024,
            max_record_bytes: 1024 * 1024,
            max_undecoded_bytes: 1024 * 1024,
            max_total_response_bytes: 16 * 1024 * 1024,
            max_records: 8_192,
            max_messages: 4_096,
            max_tools: 1_024,
            max_streamed_tool_calls: 64,
            max_tool_calls: 64,
            max_tool_arguments_bytes: 64 * 1024,
            max_json_nodes: 262_144,
        }
    }
}

impl AiGatewayLimits {
    fn is_valid(self) -> bool {
        self.max_request_bytes > 0
            && self.max_chunk_bytes > 0
            && self.max_record_bytes > 0
            && self.max_undecoded_bytes > 0
            && self.max_total_response_bytes > 0
            && self.max_records > 0
            && self.max_messages > 0
            && self.max_tools > 0
            && self.max_streamed_tool_calls > 0
            && self.max_tool_calls > 0
            && self.max_tool_arguments_bytes > 0
            && self.max_json_nodes > 0
    }
}

/// Stable construction-error category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AiGatewayConfigErrorKind {
    InvalidModel,
    InvalidLimits,
}

/// Redacted construction failure.
#[derive(Clone, Eq, PartialEq)]
pub struct AiGatewayConfigError {
    kind: AiGatewayConfigErrorKind,
}

impl AiGatewayConfigError {
    #[must_use]
    pub const fn kind(&self) -> AiGatewayConfigErrorKind {
        self.kind
    }
}

impl fmt::Debug for AiGatewayConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayConfigError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for AiGatewayConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            AiGatewayConfigErrorKind::InvalidModel => {
                formatter.write_str("invalid model identifier")
            }
            AiGatewayConfigErrorKind::InvalidLimits => {
                formatter.write_str("invalid gateway limits")
            }
        }
    }
}

impl std::error::Error for AiGatewayConfigError {}

/// One owned HTTP-style header passed to the injected transport.
#[derive(Clone, Eq, PartialEq)]
pub struct AiGatewayHeader {
    name: String,
    value: String,
}

impl AiGatewayHeader {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    #[must_use]
    pub fn into_parts(self) -> (String, String) {
        (self.name, self.value)
    }
}

impl fmt::Debug for AiGatewayHeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayHeader")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Bounded request passed to the injected transport.
pub struct AiGatewayTransportRequest {
    headers: Vec<AiGatewayHeader>,
    body: Vec<u8>,
}

impl AiGatewayTransportRequest {
    #[must_use]
    pub fn headers(&self) -> &[AiGatewayHeader] {
        &self.headers
    }

    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    #[must_use]
    pub fn into_body(self) -> Vec<u8> {
        self.body
    }

    #[must_use]
    pub fn into_parts(self) -> (Vec<AiGatewayHeader>, Vec<u8>) {
        (self.headers, self.body)
    }
}

impl fmt::Debug for AiGatewayTransportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayTransportRequest")
            .field("header_count", &self.headers.len())
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Runtime- and HTTP-client-neutral streaming transport.
pub trait AiGatewayTransport: Send + Sync + 'static {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>>;
}

/// Vercel AI Gateway v3 protocol adapter over an injected byte transport.
pub struct AiGatewayProvider {
    default_model: String,
    transport: Arc<dyn AiGatewayTransport>,
    limits: AiGatewayLimits,
}

impl AiGatewayProvider {
    /// Constructs an adapter with conservative default limits.
    ///
    /// # Errors
    ///
    /// Returns an error when the model identifier is invalid.
    pub fn new(
        default_model: impl Into<String>,
        transport: Arc<dyn AiGatewayTransport>,
    ) -> Result<Self, AiGatewayConfigError> {
        Self::with_limits(default_model, transport, AiGatewayLimits::default())
    }

    /// Constructs an adapter with explicit nonzero limits.
    ///
    /// # Errors
    ///
    /// Returns an error when the model identifier or any limit is invalid.
    pub fn with_limits(
        default_model: impl Into<String>,
        transport: Arc<dyn AiGatewayTransport>,
        limits: AiGatewayLimits,
    ) -> Result<Self, AiGatewayConfigError> {
        let default_model = default_model.into();
        if !valid_model(&default_model) {
            return Err(AiGatewayConfigError {
                kind: AiGatewayConfigErrorKind::InvalidModel,
            });
        }
        if !limits.is_valid() {
            return Err(AiGatewayConfigError {
                kind: AiGatewayConfigErrorKind::InvalidLimits,
            });
        }
        Ok(Self {
            default_model,
            transport,
            limits,
        })
    }
}

impl fmt::Debug for AiGatewayProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayProvider")
            .field("default_model", &"<redacted>")
            .field("transport", &"<redacted>")
            .field("limits", &self.limits)
            .finish()
    }
}

impl ModelProvider for AiGatewayProvider {
    fn name(&self) -> &str {
        AI_GATEWAY_PROVIDER_NAME
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        let mut request = ModelRequestGuard::new(request);
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(cancelled_error());
            }
            validate_request_envelope(
                request.get(),
                &self.default_model,
                self.limits,
                &cancellation,
            )?;
            validate_request_json(request.get(), self.limits, &cancellation)?;
            let transport_request = build_request(
                request.take(),
                &self.default_model,
                self.limits,
                &cancellation,
            )?;
            if cancellation.is_cancelled() {
                return Err(cancelled_error());
            }
            let mut startup = self
                .transport
                .stream(transport_request, cancellation.clone());
            let mut cancelled = Box::pin(cancellation.cancelled());
            let bytes = poll_fn(|context| {
                if cancelled.as_mut().poll(context).is_ready() {
                    return Poll::Ready(Err(cancelled_error()));
                }
                let result = startup.as_mut().poll(context);
                if result.is_ready() && cancellation.is_cancelled() {
                    Poll::Ready(Err(cancelled_error()))
                } else {
                    result
                }
            })
            .await?;
            if cancellation.is_cancelled() {
                return Err(cancelled_error());
            }
            Ok(
                Box::pin(GatewayEventStream::new(bytes, &cancellation, self.limits))
                    as ModelEventStream,
            )
        })
    }
}

struct ModelRequestGuard {
    request: Option<ModelRequest>,
}

impl ModelRequestGuard {
    fn new(request: ModelRequest) -> Self {
        Self {
            request: Some(request),
        }
    }

    fn get(&self) -> &ModelRequest {
        self.request.as_ref().expect("request guard is armed")
    }

    fn take(&mut self) -> ModelRequest {
        self.request.take().expect("request guard is armed")
    }
}

impl Drop for ModelRequestGuard {
    fn drop(&mut self) {
        let Some(request) = self.request.as_mut() else {
            return;
        };
        for value in request.options.metadata.values_mut() {
            drop_json_value_iterative(value);
        }
        for tool in &mut request.tools {
            drop_json_value_iterative(&mut tool.input_schema);
        }
        for message in &mut request.messages {
            for block in &mut message.content {
                match block {
                    ContentBlock::Json { value } => drop_json_value_iterative(value),
                    ContentBlock::ToolCall { call } => {
                        drop_json_value_iterative(&mut call.arguments);
                    }
                    ContentBlock::ToolResult { output, .. } => {
                        drop_json_value_iterative(&mut output.content);
                    }
                    ContentBlock::Text { .. } | _ => {}
                }
            }
        }
    }
}

fn drop_json_value_iterative(value: &mut Value) {
    enum OwnedJsonFrame {
        Array(std::vec::IntoIter<Value>),
        Object(serde_json::map::IntoValues),
    }

    impl OwnedJsonFrame {
        fn next(&mut self) -> Option<Value> {
            match self {
                Self::Array(values) => values.next(),
                Self::Object(values) => values.next(),
            }
        }
    }

    let mut frames: Vec<OwnedJsonFrame> = Vec::new();
    let mut current = Some(std::mem::take(value));
    loop {
        let Some(value) = current.take() else {
            let Some(frame) = frames.last_mut() else {
                return;
            };
            if let Some(next) = frame.next() {
                current = Some(next);
            } else {
                frames.pop();
            }
            continue;
        };
        match value {
            Value::Array(values) => {
                let mut frame = OwnedJsonFrame::Array(values.into_iter());
                current = frame.next();
                frames.push(frame);
            }
            Value::Object(values) => {
                let mut frame = OwnedJsonFrame::Object(values.into_values());
                current = frame.next();
                frames.push(frame);
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

pub(crate) fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= AI_GATEWAY_MAX_MODEL_BYTES
        && model.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

#[cfg(test)]
mod model_validation_tests {
    use super::{AI_GATEWAY_DEFAULT_MODEL, AI_GATEWAY_MAX_MODEL_BYTES, valid_model};

    #[test]
    fn built_in_and_boundary_model_identifiers_are_valid() {
        assert!(valid_model(AI_GATEWAY_DEFAULT_MODEL));
        assert!(valid_model("!"));
        assert!(valid_model(&"~".repeat(AI_GATEWAY_MAX_MODEL_BYTES)));
    }

    #[test]
    fn model_identifiers_reject_empty_oversized_and_non_visible_ascii_values() {
        assert!(!valid_model(""));
        assert!(!valid_model(&"!".repeat(AI_GATEWAY_MAX_MODEL_BYTES + 1)));
        for invalid in ["model name", "model\nname", "model\u{7f}", "modèle"] {
            assert!(!valid_model(invalid));
        }
    }
}

fn validate_request_envelope(
    request: &ModelRequest,
    default_model: &str,
    limits: AiGatewayLimits,
    cancellation: &CancellationToken,
) -> Result<(), ProviderError> {
    check_cancel(cancellation)?;
    if request.messages.is_empty()
        || request.messages.len() > limits.max_messages
        || request.tools.len() > limits.max_tools
    {
        return Err(invalid_request("gateway_request_count_limit"));
    }
    if request.options.max_output_tokens == Some(0) {
        return Err(invalid_request("gateway_invalid_max_output_tokens"));
    }
    let model = request.options.model.as_deref().unwrap_or(default_model);
    if !valid_model(model) {
        return Err(invalid_request("gateway_invalid_model"));
    }
    for message in &request.messages {
        check_cancel(cancellation)?;
        let valid_count = match message.role {
            Role::System | Role::User | Role::Tool => message.content.len() == 1,
            Role::Assistant => message.content.len() <= limits.max_tool_calls.saturating_add(1),
            _ => false,
        };
        if !valid_count {
            return Err(invalid_request("gateway_invalid_history"));
        }
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GatewayRequest {
    prompt: Vec<GatewayMessage>,
    tools: Vec<GatewayTool>,
    tool_choice: GatewayToolChoice,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
}

#[derive(Serialize)]
struct GatewayToolChoice {
    r#type: &'static str,
}

#[derive(Serialize)]
struct GatewayTool {
    r#type: &'static str,
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

#[derive(Serialize)]
struct GatewayMessage {
    role: &'static str,
    content: GatewayContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum GatewayContent {
    System(String),
    Parts(Vec<GatewayPart>),
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum GatewayPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool-call")]
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        input: Value,
    },
    #[serde(rename = "tool-result")]
    ToolResult {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        output: GatewayToolOutput,
    },
}

#[derive(Serialize)]
struct GatewayToolOutput {
    r#type: &'static str,
    value: String,
}

fn build_request(
    request: ModelRequest,
    default_model: &str,
    limits: AiGatewayLimits,
    cancellation: &CancellationToken,
) -> Result<AiGatewayTransportRequest, ProviderError> {
    if request.messages.is_empty()
        || request.messages.len() > limits.max_messages
        || request.tools.len() > limits.max_tools
    {
        return Err(invalid_request("gateway_request_count_limit"));
    }
    if request.options.max_output_tokens == Some(0) {
        return Err(invalid_request("gateway_invalid_max_output_tokens"));
    }
    let model = request.options.model.as_deref().unwrap_or(default_model);
    if !valid_model(model) {
        return Err(invalid_request("gateway_invalid_model"));
    }

    let mut projection_remaining = limits.max_request_bytes;
    let prompt = build_prompt(
        request.messages,
        limits,
        &mut projection_remaining,
        cancellation,
    )?;
    let mut tools = Vec::new();
    for tool in request.tools {
        check_cancel(cancellation)?;
        tools.push(GatewayTool {
            r#type: "function",
            name: tool.name.to_string(),
            description: tool.description,
            input_schema: tool.input_schema,
        });
    }
    let tool_choice = GatewayToolChoice {
        r#type: if tools.is_empty() { "none" } else { "auto" },
    };
    let body = serialize_bounded(
        &GatewayRequest {
            prompt,
            tools,
            tool_choice,
            max_output_tokens: request.options.max_output_tokens,
        },
        limits.max_request_bytes,
        cancellation,
    )?;
    let session = request.session_id.to_string();
    Ok(build_gateway_transport_request(model, &session, body))
}

pub(crate) fn build_gateway_transport_request(
    model: &str,
    session: &str,
    body: Vec<u8>,
) -> AiGatewayTransportRequest {
    let headers = [
        ("content-type", CONTENT_TYPE),
        ("ai-gateway-protocol-version", AI_GATEWAY_PROTOCOL_VERSION),
        (
            "ai-language-model-specification-version",
            AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION,
        ),
        ("ai-language-model-id", model),
        ("ai-language-model-streaming", "true"),
        ("x-session-id", session),
        ("x-session-affinity", session),
    ]
    .into_iter()
    .map(|(name, value)| AiGatewayHeader {
        name: name.to_owned(),
        value: value.to_owned(),
    })
    .collect();
    AiGatewayTransportRequest { headers, body }
}

fn build_prompt(
    messages: Vec<Message>,
    limits: AiGatewayLimits,
    projection_remaining: &mut usize,
    cancellation: &CancellationToken,
) -> Result<Vec<GatewayMessage>, ProviderError> {
    let mut prompt = Vec::with_capacity(messages.len());
    let mut pending: BTreeMap<ToolCallId, ToolName> = BTreeMap::new();
    for message in messages {
        check_cancel(cancellation)?;
        if !pending.is_empty() && message.role != Role::Tool {
            return Err(invalid_request("gateway_invalid_history"));
        }
        let built = match message.role {
            Role::System => GatewayMessage {
                role: "system",
                content: GatewayContent::System(exact_text(message.content)?),
            },
            Role::User => GatewayMessage {
                role: "user",
                content: GatewayContent::Parts(vec![GatewayPart::Text {
                    text: exact_text(message.content)?,
                }]),
            },
            Role::Assistant => {
                let mut parts = Vec::new();
                let mut saw_call = false;
                for block in message.content {
                    match block {
                        ContentBlock::Text { text } if !saw_call && parts.is_empty() => {
                            parts.push(GatewayPart::Text { text });
                        }
                        ContentBlock::ToolCall { call } => {
                            saw_call = true;
                            if pending.len() >= limits.max_tool_calls
                                || pending.insert(call.id.clone(), call.name.clone()).is_some()
                            {
                                return Err(invalid_request("gateway_invalid_history"));
                            }
                            drop(serialize_bounded(
                                &call.arguments,
                                limits.max_tool_arguments_bytes,
                                cancellation,
                            )?);
                            parts.push(GatewayPart::ToolCall {
                                tool_call_id: call.id.to_string(),
                                tool_name: call.name.to_string(),
                                input: call.arguments,
                            });
                        }
                        ContentBlock::Text { .. }
                        | ContentBlock::Json { .. }
                        | ContentBlock::ToolResult { .. } => {
                            return Err(invalid_request("gateway_invalid_history"));
                        }
                        _ => return Err(invalid_request("gateway_invalid_history")),
                    }
                }
                GatewayMessage {
                    role: "assistant",
                    content: GatewayContent::Parts(parts),
                }
            }
            Role::Tool => {
                let mut content = message.content;
                if content.len() != 1 {
                    return Err(invalid_request("gateway_invalid_history"));
                }
                let ContentBlock::ToolResult { call_id, output } =
                    content.pop().expect("length checked")
                else {
                    return Err(invalid_request("gateway_invalid_history"));
                };
                let Some(name) = pending.remove(&call_id) else {
                    return Err(invalid_request("gateway_invalid_history"));
                };
                let value = String::from_utf8(serialize_bounded(
                    &output,
                    *projection_remaining,
                    cancellation,
                )?)
                .map_err(|_| protocol_error("gateway_internal_encoding"))?;
                *projection_remaining = projection_remaining
                    .checked_sub(value.len())
                    .expect("bounded serialization cannot exceed remaining budget");
                GatewayMessage {
                    role: "tool",
                    content: GatewayContent::Parts(vec![GatewayPart::ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: name.to_string(),
                        output: GatewayToolOutput {
                            r#type: "text",
                            value,
                        },
                    }]),
                }
            }
            _ => return Err(invalid_request("gateway_invalid_history")),
        };
        prompt.push(built);
    }
    if !pending.is_empty() {
        return Err(invalid_request("gateway_invalid_history"));
    }
    Ok(prompt)
}

fn exact_text(content: Vec<ContentBlock>) -> Result<String, ProviderError> {
    let mut content = content;
    if content.len() != 1 {
        return Err(invalid_request("gateway_invalid_history"));
    }
    match content.pop().expect("length checked") {
        ContentBlock::Text { text } => Ok(text),
        _ => Err(invalid_request("gateway_invalid_history")),
    }
}

fn validate_request_json(
    request: &ModelRequest,
    limits: AiGatewayLimits,
    cancellation: &CancellationToken,
) -> Result<(), ProviderError> {
    let mut nodes = 0;
    for value in request.options.metadata.values() {
        check_cancel(cancellation)?;
        validate_json(value, &mut nodes, limits.max_json_nodes, cancellation)?;
    }
    for tool in &request.tools {
        check_cancel(cancellation)?;
        validate_json(
            &tool.input_schema,
            &mut nodes,
            limits.max_json_nodes,
            cancellation,
        )?;
    }
    for message in &request.messages {
        check_cancel(cancellation)?;
        for block in &message.content {
            check_cancel(cancellation)?;
            let value = match block {
                ContentBlock::Json { value } => Some(value),
                ContentBlock::ToolCall { call } => Some(&call.arguments),
                ContentBlock::ToolResult { output, .. } => Some(&output.content),
                ContentBlock::Text { .. } | _ => None,
            };
            if let Some(value) = value {
                validate_json(value, &mut nodes, limits.max_json_nodes, cancellation)?;
            }
        }
    }
    Ok(())
}

enum JsonFrame<'a> {
    Array(std::slice::Iter<'a, Value>, usize),
    Object(serde_json::map::Values<'a>, usize),
}

impl<'a> JsonFrame<'a> {
    fn next(&mut self) -> Option<(&'a Value, usize)> {
        match self {
            Self::Array(values, depth) => values.next().map(|value| (value, *depth)),
            Self::Object(values, depth) => values.next().map(|value| (value, *depth)),
        }
    }
}

fn validate_json(
    root: &Value,
    nodes: &mut usize,
    max_nodes: usize,
    cancellation: &CancellationToken,
) -> Result<(), ProviderError> {
    let mut frames: Vec<JsonFrame<'_>> = Vec::new();
    let mut current = Some((root, 0_usize));
    loop {
        let Some((value, depth)) = current.take() else {
            let Some(frame) = frames.last_mut() else {
                return Ok(());
            };
            if let Some(next) = frame.next() {
                current = Some(next);
            } else {
                frames.pop();
            }
            continue;
        };
        check_cancel(cancellation)?;
        *nodes = nodes
            .checked_add(1)
            .filter(|nodes| *nodes <= max_nodes)
            .ok_or_else(|| invalid_request("gateway_json_node_limit"))?;
        match value {
            Value::Array(values) => {
                if depth >= MAX_SAFE_JSON_DEPTH {
                    return Err(invalid_request("gateway_json_depth_limit"));
                }
                let mut frame = JsonFrame::Array(values.iter(), depth + 1);
                current = frame.next();
                frames.push(frame);
            }
            Value::Object(values) => {
                if depth >= MAX_SAFE_JSON_DEPTH {
                    return Err(invalid_request("gateway_json_depth_limit"));
                }
                let mut frame = JsonFrame::Object(values.values(), depth + 1);
                current = frame.next();
                frames.push(frame);
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

struct BoundedWriter<'a> {
    bytes: Vec<u8>,
    max: usize,
    cancellation: &'a CancellationToken,
    failed: Option<ProviderError>,
}

impl Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancellation.is_cancelled() {
            self.failed = Some(cancelled_error());
            return Err(io::Error::other("cancelled"));
        }
        if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
            self.failed = Some(invalid_request("gateway_request_byte_limit"));
            return Err(io::Error::other("limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialize_bounded<T: Serialize>(
    value: &T,
    max: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, ProviderError> {
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        max,
        cancellation,
        failed: None,
    };
    if serde_json::to_writer(&mut writer, value).is_err() {
        return Err(writer
            .failed
            .unwrap_or_else(|| protocol_error("gateway_internal_encoding")));
    }
    Ok(writer.bytes)
}

fn check_cancel(cancellation: &CancellationToken) -> Result<(), ProviderError> {
    if cancellation.is_cancelled() {
        Err(cancelled_error())
    } else {
        Ok(())
    }
}

fn cancelled_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Cancelled,
        "gateway_cancelled",
        "gateway operation cancelled",
        false,
    )
}

fn invalid_request(code: &'static str) -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::InvalidRequest,
        code,
        "gateway request rejected",
        false,
    )
}

fn protocol_error(code: &'static str) -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Protocol,
        code,
        "gateway response rejected",
        false,
    )
}

fn provider_failure() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Other,
        "gateway_provider_failure",
        "gateway provider failed",
        false,
    )
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, ProviderError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error("gateway_invalid_event"))
}

fn parse_call_id(value: &str) -> Result<ToolCallId, ProviderError> {
    ToolCallId::new(value).map_err(|_| protocol_error("gateway_invalid_tool_call"))
}

fn parse_tool_name(value: &str) -> Result<ToolName, ProviderError> {
    ToolName::new(value).map_err(|_| protocol_error("gateway_invalid_tool_call"))
}

struct CountingWriter {
    bytes: usize,
    max: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|total| *total <= self.max)
            .ok_or_else(|| io::Error::other("limit"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ProtocolArgumentsWriter {
    bytes: Vec<u8>,
    max: usize,
    exceeded: bool,
}

impl Write for ProtocolArgumentsWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn canonical_arguments(value: &Value, max_bytes: usize) -> Result<Vec<u8>, ProviderError> {
    fn normalize_signed_zero(value: &mut Value) {
        match value {
            Value::Array(values) => {
                for value in values {
                    normalize_signed_zero(value);
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    normalize_signed_zero(value);
                }
            }
            Value::Number(number)
                if number.is_f64() && number.as_f64().is_some_and(|value| value == 0.0) =>
            {
                *number = serde_json::Number::from_f64(0.0).expect("zero is finite");
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    let mut normalized = value.clone();
    normalize_signed_zero(&mut normalized);
    let mut writer = ProtocolArgumentsWriter {
        bytes: Vec::new(),
        max: max_bytes,
        exceeded: false,
    };
    if serde_json::to_writer(&mut writer, &normalized).is_err() {
        return Err(if writer.exceeded {
            protocol_error("gateway_tool_arguments_byte_limit")
        } else {
            protocol_error("gateway_invalid_tool_arguments")
        });
    }
    Ok(writer.bytes)
}

fn parse_final_arguments(
    value: &Value,
    max_bytes: usize,
    max_nodes: usize,
) -> Result<Value, ProviderError> {
    match value {
        Value::String(text) => parse_argument_text(text, max_bytes, max_nodes),
        Value::Array(_) | Value::Object(_) => {
            let mut writer = CountingWriter {
                bytes: 0,
                max: max_bytes,
            };
            if serde_json::to_writer(&mut writer, value).is_err() {
                return Err(protocol_error("gateway_tool_arguments_byte_limit"));
            }
            Ok(value.clone())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {
            Err(protocol_error("gateway_invalid_tool_arguments"))
        }
    }
}

#[derive(Clone, Copy)]
struct StrictValueSeed<'a> {
    depth: usize,
    nodes: &'a Cell<usize>,
    max_nodes: usize,
}

impl<'de> DeserializeSeed<'de> for StrictValueSeed<'_> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let nodes = self
            .nodes
            .get()
            .checked_add(1)
            .filter(|nodes| *nodes <= self.max_nodes)
            .ok_or_else(|| D::Error::custom("JSON node limit exceeded"))?;
        self.nodes.set(nodes);
        deserializer.deserialize_any(StrictValueVisitor {
            depth: self.depth,
            nodes: self.nodes,
            max_nodes: self.max_nodes,
        })
    }
}

struct StrictValueVisitor<'a> {
    depth: usize,
    nodes: &'a Cell<usize>,
    max_nodes: usize,
}

impl<'de> Visitor<'de> for StrictValueVisitor<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_string(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if self.depth >= MAX_SAFE_JSON_DEPTH {
            return Err(A::Error::custom("JSON depth limit exceeded"));
        }
        let mut values = Vec::new();
        let child = StrictValueSeed {
            depth: self.depth + 1,
            nodes: self.nodes,
            max_nodes: self.max_nodes,
        };
        while let Some(value) = sequence.next_element_seed(child)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut entries: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        if self.depth >= MAX_SAFE_JSON_DEPTH {
            return Err(A::Error::custom("JSON depth limit exceeded"));
        }
        let mut values = serde_json::Map::new();
        let child = StrictValueSeed {
            depth: self.depth + 1,
            nodes: self.nodes,
            max_nodes: self.max_nodes,
        };
        while let Some(key) = entries.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom("duplicate JSON object key"));
            }
            values.insert(key, entries.next_value_seed(child)?);
        }
        Ok(Value::Object(values))
    }
}

pub(crate) fn parse_strict_json(text: &str, max_nodes: usize) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let nodes = Cell::new(0);
    let value = StrictValueSeed {
        depth: 0,
        nodes: &nodes,
        max_nodes,
    }
    .deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

fn parse_argument_text(
    text: &str,
    max_bytes: usize,
    max_nodes: usize,
) -> Result<Value, ProviderError> {
    if text.len() > max_bytes {
        return Err(protocol_error("gateway_tool_arguments_byte_limit"));
    }
    let value = parse_strict_json(text, max_nodes)
        .map_err(|_| protocol_error("gateway_invalid_tool_arguments"))?;
    match value {
        Value::Array(_) | Value::Object(_) => Ok(value),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            Err(protocol_error("gateway_invalid_tool_arguments"))
        }
    }
}

fn parse_usage(value: &Value) -> Result<TokenUsage, ProviderError> {
    let usage = value
        .as_object()
        .ok_or_else(|| protocol_error("gateway_invalid_usage"))?;
    let input = required_token_group(usage, "inputTokens", true)?;
    let output = required_token_group(usage, "outputTokens", false)?;
    Ok(TokenUsage {
        input_tokens: input.total,
        output_tokens: output.total,
        cached_input_tokens: input.cache_read,
    })
}

#[derive(Clone, Copy)]
struct TokenGroup {
    total: u64,
    cache_read: u64,
}

fn required_token_group(
    usage: &serde_json::Map<String, Value>,
    name: &str,
    allow_cache_read: bool,
) -> Result<TokenGroup, ProviderError> {
    let value = usage
        .get(name)
        .ok_or_else(|| protocol_error("gateway_invalid_usage"))?;
    let group = value
        .as_object()
        .ok_or_else(|| protocol_error("gateway_invalid_usage"))?;
    let total = required_nonnegative_integer(group, "total")?;
    let cache_read = if allow_cache_read {
        optional_nonnegative_integer(group, "cacheRead")?.unwrap_or(0)
    } else {
        0
    };
    if cache_read > total {
        return Err(protocol_error("gateway_invalid_usage"));
    }
    Ok(TokenGroup { total, cache_read })
}

fn required_nonnegative_integer(
    object: &serde_json::Map<String, Value>,
    name: &str,
) -> Result<u64, ProviderError> {
    optional_nonnegative_integer(object, name)?
        .ok_or_else(|| protocol_error("gateway_invalid_usage"))
}

fn optional_nonnegative_integer(
    object: &serde_json::Map<String, Value>,
    name: &str,
) -> Result<Option<u64>, ProviderError> {
    object
        .get(name)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| protocol_error("gateway_invalid_usage"))
        })
        .transpose()
}

struct StreamedToolInput {
    name: ToolName,
    arguments: String,
    parsed_arguments: Option<Value>,
    canonical_arguments: Option<Vec<u8>>,
    invalid_arguments: bool,
    ended: bool,
}

struct GatewayEventStream {
    source: Option<AiGatewayByteStream>,
    cancellation_token: CancellationToken,
    cancellation: Option<Pin<Box<machine_god_core::Cancelled>>>,
    limits: AiGatewayLimits,
    undecoded: Vec<u8>,
    total_bytes: usize,
    records: usize,
    streamed: BTreeMap<ToolCallId, StreamedToolInput>,
    reconciliation: BTreeMap<ToolName, BTreeMap<Vec<u8>, BTreeSet<ToolCallId>>>,
    finalized_streamed: BTreeSet<ToolCallId>,
    emitted_calls: BTreeSet<ToolCallId>,
    pending: VecDeque<Result<ModelEvent, ProviderError>>,
    finished: bool,
    terminal: bool,
}

impl GatewayEventStream {
    fn new(
        source: AiGatewayByteStream,
        cancellation: &CancellationToken,
        limits: AiGatewayLimits,
    ) -> Self {
        Self {
            source: Some(source),
            cancellation_token: cancellation.clone(),
            cancellation: None,
            limits,
            undecoded: Vec::new(),
            total_bytes: 0,
            records: 0,
            streamed: BTreeMap::new(),
            reconciliation: BTreeMap::new(),
            finalized_streamed: BTreeSet::new(),
            emitted_calls: BTreeSet::new(),
            pending: VecDeque::new(),
            finished: false,
            terminal: false,
        }
    }

    fn poll_cancellation(&mut self, context: &mut Context<'_>) -> bool {
        let cancellation = self
            .cancellation
            .get_or_insert_with(|| Box::pin(self.cancellation_token.cancelled()));
        if cancellation.as_mut().poll(context).is_ready() {
            self.cancellation = None;
            true
        } else {
            false
        }
    }

    fn clear_cancellation_waiter(&mut self) {
        self.cancellation = None;
    }

    fn consume_chunk(&mut self, chunk: &[u8]) -> Result<(), ProviderError> {
        check_cancel(&self.cancellation_token)?;
        if chunk.is_empty() {
            return Err(protocol_error("gateway_empty_chunk"));
        }
        if chunk.len() > self.limits.max_chunk_bytes {
            return Err(protocol_error("gateway_chunk_byte_limit"));
        }
        self.total_bytes = self
            .total_bytes
            .checked_add(chunk.len())
            .filter(|total| *total <= self.limits.max_total_response_bytes)
            .ok_or_else(|| protocol_error("gateway_response_byte_limit"))?;

        let mut remaining = chunk;
        while let Some(newline) = remaining.iter().position(|byte| *byte == b'\n') {
            check_cancel(&self.cancellation_token)?;
            self.append_record_fragment(&remaining[..newline])?;
            let line = std::mem::take(&mut self.undecoded);
            self.consume_line(&line)?;
            remaining = &remaining[newline + 1..];
        }
        self.append_record_fragment(remaining)
    }

    fn append_record_fragment(&mut self, fragment: &[u8]) -> Result<(), ProviderError> {
        check_cancel(&self.cancellation_token)?;
        let new_len = self
            .undecoded
            .len()
            .checked_add(fragment.len())
            .ok_or_else(|| protocol_error("gateway_record_byte_limit"))?;
        if new_len > self.limits.max_record_bytes || new_len > self.limits.max_undecoded_bytes {
            return Err(protocol_error("gateway_record_byte_limit"));
        }
        self.undecoded.extend_from_slice(fragment);
        Ok(())
    }

    fn consume_eof(&mut self) -> Result<(), ProviderError> {
        check_cancel(&self.cancellation_token)?;
        if !self.undecoded.is_empty() {
            let line = std::mem::take(&mut self.undecoded);
            self.consume_line(&line)?;
        }
        if self.finished {
            Ok(())
        } else {
            Err(protocol_error("gateway_finish_missing"))
        }
    }

    fn consume_line(&mut self, line: &[u8]) -> Result<(), ProviderError> {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line.starts_with(b":") {
            return Ok(());
        }
        let Some(data) = line.strip_prefix(b"data: ") else {
            return Ok(());
        };
        self.records = self
            .records
            .checked_add(1)
            .filter(|records| *records <= self.limits.max_records)
            .ok_or_else(|| protocol_error("gateway_record_count_limit"))?;
        if data == b"[DONE]" {
            return if self.finished {
                Ok(())
            } else {
                Err(protocol_error("gateway_finish_missing"))
            };
        }
        if self.finished {
            return Err(protocol_error("gateway_event_after_finish"));
        }
        let text = std::str::from_utf8(data).map_err(|_| protocol_error("gateway_invalid_utf8"))?;
        let event = parse_strict_json(text, self.limits.max_json_nodes)
            .map_err(|_| protocol_error("gateway_invalid_json"))?;
        let object = event
            .as_object()
            .ok_or_else(|| protocol_error("gateway_invalid_event"))?;
        let event_type = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error("gateway_invalid_event"))?;
        match event_type {
            "text-delta" => self.consume_delta(object, false),
            "reasoning-delta" => self.consume_delta(object, true),
            "tool-input-start" => self.consume_tool_input_start(object),
            "tool-input-delta" => self.consume_tool_input_delta(object),
            "tool-input-end" => self.consume_tool_input_end(object),
            "tool-call" => self.consume_tool_call(object),
            "tool-result" => Err(protocol_error("gateway_provider_tool_result")),
            "error" => Err(provider_failure()),
            "finish" => self.consume_finish(object),
            "response-metadata" | "start" | "start-step" | "finish-step" | "text-start"
            | "text-end" | "reasoning-start" | "reasoning-end" | "source" | "file" | "raw" => {
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn consume_delta(
        &mut self,
        object: &serde_json::Map<String, Value>,
        reasoning: bool,
    ) -> Result<(), ProviderError> {
        let delta = required_string(object, "delta")?;
        self.pending.push_back(Ok(if reasoning {
            ModelEvent::ReasoningDelta {
                text: delta.to_owned(),
            }
        } else {
            ModelEvent::TextDelta {
                text: delta.to_owned(),
            }
        }));
        Ok(())
    }

    fn consume_tool_input_start(
        &mut self,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), ProviderError> {
        if self.streamed.len() >= self.limits.max_streamed_tool_calls {
            return Err(protocol_error("gateway_streamed_tool_call_limit"));
        }
        let id = parse_call_id(required_string(object, "id")?)?;
        let name = parse_tool_name(required_string(object, "toolName")?)?;
        if self.streamed.contains_key(&id)
            || self.finalized_streamed.contains(&id)
            || self.emitted_calls.contains(&id)
        {
            return Err(protocol_error("gateway_duplicate_tool_call"));
        }
        self.streamed.insert(
            id,
            StreamedToolInput {
                name,
                arguments: String::new(),
                parsed_arguments: None,
                canonical_arguments: None,
                invalid_arguments: false,
                ended: false,
            },
        );
        Ok(())
    }

    fn consume_tool_input_delta(
        &mut self,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), ProviderError> {
        let id = parse_call_id(required_string(object, "id")?)?;
        let delta = required_string(object, "delta")?;
        if self.finalized_streamed.contains(&id) {
            return if delta.len() <= self.limits.max_tool_arguments_bytes {
                Ok(())
            } else {
                Err(protocol_error("gateway_tool_arguments_byte_limit"))
            };
        }
        let streamed = self
            .streamed
            .get_mut(&id)
            .ok_or_else(|| protocol_error("gateway_unmatched_tool_input"))?;
        if streamed.ended {
            return Err(protocol_error("gateway_late_tool_input"));
        }
        if delta.len()
            > self
                .limits
                .max_tool_arguments_bytes
                .saturating_sub(streamed.arguments.len())
        {
            return Err(protocol_error("gateway_tool_arguments_byte_limit"));
        }
        streamed.arguments.push_str(delta);
        Ok(())
    }

    fn consume_tool_input_end(
        &mut self,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), ProviderError> {
        let max_bytes = self.limits.max_tool_arguments_bytes;
        let max_nodes = self.limits.max_json_nodes;
        let id = parse_call_id(required_string(object, "id")?)?;
        if self.finalized_streamed.contains(&id) {
            return Ok(());
        }
        let streamed = self
            .streamed
            .get_mut(&id)
            .ok_or_else(|| protocol_error("gateway_unmatched_tool_input"))?;
        if streamed.ended {
            return Err(protocol_error("gateway_duplicate_tool_input_end"));
        }
        let parsed = parse_argument_text(&streamed.arguments, max_bytes, max_nodes);
        streamed.arguments = String::new();
        streamed.ended = true;
        match parsed {
            Ok(parsed) => {
                let canonical = canonical_arguments(&parsed, max_bytes)?;
                let name = streamed.name.clone();
                streamed.parsed_arguments = Some(parsed);
                streamed.canonical_arguments = Some(canonical.clone());
                self.reconciliation
                    .entry(name)
                    .or_default()
                    .entry(canonical)
                    .or_default()
                    .insert(id);
            }
            Err(_) => streamed.invalid_arguments = true,
        }
        Ok(())
    }

    fn reconciled_streamed_id(
        &self,
        final_id: &ToolCallId,
        explicit_name: Option<&ToolName>,
        explicit_canonical: Option<&[u8]>,
    ) -> Result<Option<ToolCallId>, ProviderError> {
        if self.streamed.contains_key(final_id) {
            return Ok(Some(final_id.clone()));
        }
        let (Some(name), Some(canonical)) = (explicit_name, explicit_canonical) else {
            return Ok(None);
        };
        let Some(matches) = self
            .reconciliation
            .get(name)
            .and_then(|by_arguments| by_arguments.get(canonical))
        else {
            return Ok(None);
        };
        if matches.len() != 1 {
            return Err(protocol_error("gateway_ambiguous_tool_call"));
        }
        Ok(matches.first().cloned())
    }

    fn remove_streamed(&mut self, id: &ToolCallId) {
        let Some(streamed) = self.streamed.remove(id) else {
            return;
        };
        self.finalized_streamed.insert(id.clone());
        let Some(canonical) = streamed.canonical_arguments else {
            return;
        };
        let name = streamed.name;
        let remove_name = self
            .reconciliation
            .get_mut(&name)
            .is_some_and(|by_arguments| {
                let remove_arguments =
                    by_arguments
                        .get_mut(canonical.as_slice())
                        .is_some_and(|matches| {
                            matches.remove(id);
                            matches.is_empty()
                        });
                if remove_arguments {
                    by_arguments.remove(canonical.as_slice());
                }
                by_arguments.is_empty()
            });
        if remove_name {
            self.reconciliation.remove(&name);
        }
    }

    fn consume_tool_call(
        &mut self,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), ProviderError> {
        if self.emitted_calls.len() >= self.limits.max_tool_calls {
            return Err(protocol_error("gateway_tool_call_limit"));
        }
        if let Some(provider_executed) = object.get("providerExecuted") {
            match provider_executed.as_bool() {
                Some(false) => {}
                Some(true) | None => {
                    return Err(protocol_error("gateway_provider_executed_tool"));
                }
            }
        }
        let id = parse_call_id(required_string(object, "toolCallId")?)?;
        let explicit_name = object.get("toolName");
        let explicit_name = match explicit_name {
            Some(value) => parse_tool_name(
                value
                    .as_str()
                    .ok_or_else(|| protocol_error("gateway_invalid_tool_call"))?,
            )
            .map(Some)?,
            None => None,
        };
        let explicit_arguments = object
            .get("input")
            .map(|input| {
                parse_final_arguments(
                    input,
                    self.limits.max_tool_arguments_bytes,
                    self.limits.max_json_nodes,
                )
            })
            .transpose()?;
        let explicit_canonical = explicit_arguments
            .as_ref()
            .map(|arguments| canonical_arguments(arguments, self.limits.max_tool_arguments_bytes))
            .transpose()?;
        let streamed_id = self.reconciled_streamed_id(
            &id,
            explicit_name.as_ref(),
            explicit_canonical.as_deref(),
        )?;
        let streamed = streamed_id
            .as_ref()
            .and_then(|streamed_id| self.streamed.get(streamed_id));
        let name = explicit_name
            .or_else(|| streamed.map(|input| input.name.clone()))
            .ok_or_else(|| protocol_error("gateway_invalid_tool_call"))?;
        if let Some(input) = streamed
            && input.name != name
        {
            return Err(protocol_error("gateway_conflicting_tool_call"));
        }
        if self.emitted_calls.contains(&id) {
            return Err(protocol_error("gateway_duplicate_tool_call"));
        }

        let arguments = if let Some(arguments) = explicit_arguments {
            arguments
        } else {
            let streamed = streamed
                .filter(|input| input.ended)
                .ok_or_else(|| protocol_error("gateway_incomplete_tool_input"))?;
            if streamed.invalid_arguments {
                return Err(protocol_error("gateway_invalid_tool_arguments"));
            }
            streamed
                .parsed_arguments
                .clone()
                .ok_or_else(|| protocol_error("gateway_incomplete_tool_input"))?
        };
        if let Some(streamed_id) = streamed_id {
            self.remove_streamed(&streamed_id);
        }
        self.emitted_calls.insert(id.clone());
        self.pending.push_back(Ok(ModelEvent::ToolCall {
            call: ToolCall {
                id,
                name,
                arguments,
            },
        }));
        Ok(())
    }

    fn consume_finish(
        &mut self,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), ProviderError> {
        let reason = object
            .get("finishReason")
            .and_then(Value::as_object)
            .and_then(|reason| reason.get("unified"))
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error("gateway_invalid_finish"))?;
        let has_calls = !self.emitted_calls.is_empty();
        let reason = match reason {
            "stop" if !has_calls => StopReason::Completed,
            "length" if !has_calls => StopReason::MaxOutputTokens,
            "content-filter" if !has_calls => StopReason::ContentFilter,
            "tool-calls" if has_calls => StopReason::ToolCalls,
            "other" if !has_calls => StopReason::Other("other".to_owned()),
            "error" => return Err(provider_failure()),
            "stop" | "length" | "content-filter" | "tool-calls" | "other" => {
                return Err(protocol_error("gateway_finish_call_mismatch"));
            }
            _ => return Err(protocol_error("gateway_invalid_finish")),
        };
        if !self.streamed.is_empty() {
            return Err(protocol_error("gateway_incomplete_tool_input"));
        }
        if let Some(usage) = object.get("usage") {
            self.pending.push_back(Ok(ModelEvent::Usage {
                usage: parse_usage(usage)?,
            }));
        }
        self.pending.push_back(Ok(ModelEvent::Stop { reason }));
        self.finished = true;
        // The terminal event has been fully validated, so no later source bytes
        // can affect the provider event stream. Releasing the byte stream here
        // also releases any transport-owned HTTP capacity before a nested tool
        // starts from the queued Stop event.
        drop(self.source.take());
        Ok(())
    }
}

impl fmt::Debug for GatewayEventStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GatewayEventStream")
            .field("total_bytes", &self.total_bytes)
            .field("records", &self.records)
            .field("finished", &self.finished)
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

impl Stream for GatewayEventStream {
    type Item = Result<ModelEvent, ProviderError>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.terminal {
            this.clear_cancellation_waiter();
            return Poll::Ready(None);
        }
        if this.poll_cancellation(context) {
            this.terminal = true;
            return Poll::Ready(Some(Err(cancelled_error())));
        }
        if let Some(event) = this.pending.pop_front() {
            this.clear_cancellation_waiter();
            if matches!(event, Ok(ModelEvent::Stop { .. })) {
                this.terminal = true;
            }
            return Poll::Ready(Some(event));
        }
        let Some(source) = this.source.as_mut() else {
            this.clear_cancellation_waiter();
            this.terminal = true;
            return Poll::Ready(None);
        };
        let source_result = source.as_mut().poll_next(context);
        if source_result.is_ready() && this.cancellation_token.is_cancelled() {
            this.clear_cancellation_waiter();
            this.terminal = true;
            return Poll::Ready(Some(Err(cancelled_error())));
        }
        match source_result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Err(error))) => {
                this.clear_cancellation_waiter();
                this.terminal = true;
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(Some(Ok(chunk))) => {
                if let Err(error) = this.consume_chunk(&chunk) {
                    this.clear_cancellation_waiter();
                    this.terminal = true;
                    return Poll::Ready(Some(Err(error)));
                }
                if let Some(event) = this.pending.pop_front() {
                    this.clear_cancellation_waiter();
                    if matches!(event, Ok(ModelEvent::Stop { .. })) {
                        this.terminal = true;
                    }
                    return Poll::Ready(Some(event));
                }
                context.waker().wake_by_ref();
                Poll::Pending
            }
            Poll::Ready(None) => {
                if let Err(error) = this.consume_eof() {
                    this.clear_cancellation_waiter();
                    this.terminal = true;
                    return Poll::Ready(Some(Err(error)));
                }
                if let Some(event) = this.pending.pop_front() {
                    this.clear_cancellation_waiter();
                    if matches!(event, Ok(ModelEvent::Stop { .. })) {
                        this.terminal = true;
                    }
                    return Poll::Ready(Some(event));
                }
                this.clear_cancellation_waiter();
                this.terminal = true;
                Poll::Ready(None)
            }
        }
    }
}
