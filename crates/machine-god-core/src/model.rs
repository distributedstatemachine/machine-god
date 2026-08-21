use crate::{
    BoxFuture, CancellationToken, SessionId, SessionIncarnationId, ToolCall, ToolSpec, TurnId,
};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::pin::Pin;

use crate::ProviderError;

/// Conversation participant represented by a message.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Provider-neutral message content.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Json {
        value: Value,
    },
    ToolCall {
        call: ToolCall,
    },
    ToolResult {
        call_id: crate::ToolCallId,
        output: crate::ToolOutput,
    },
}

/// A provider-neutral conversation message.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    /// Creates a one-block text message.
    #[must_use]
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }
}

/// Portable inference controls; providers ignore unsupported optional fields.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct InferenceOptions {
    pub model: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub metadata: BTreeMap<String, Value>,
}

/// Complete input to a model provider.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ModelRequest {
    pub session_id: SessionId,
    pub session_incarnation_id: SessionIncarnationId,
    pub turn_id: TurnId,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub options: InferenceOptions,
}

/// Why model generation stopped.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
#[non_exhaustive]
pub enum StopReason {
    Completed,
    ToolCalls,
    MaxOutputTokens,
    ContentFilter,
    Cancelled,
    Other(String),
}

/// Token counters reported by a provider.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
}

/// One ordered item from a model response stream.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ModelEvent {
    TextDelta { text: String },
    ReasoningDelta { text: String },
    ToolCall { call: ToolCall },
    Usage { usage: TokenUsage },
    Stop { reason: StopReason },
}

/// A sendable stream returned by a model provider.
pub type ModelEventStream =
    Pin<Box<dyn Stream<Item = Result<ModelEvent, ProviderError>> + Send + 'static>>;

/// Provider-neutral, object-safe streaming model interface.
pub trait ModelProvider: Send + Sync + 'static {
    /// Stable provider identifier for diagnostics.
    fn name(&self) -> &str;

    /// Starts a response stream. Implementations must observe cancellation and
    /// must emit at most one terminal [`ModelEvent::Stop`] event.
    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>>;
}
