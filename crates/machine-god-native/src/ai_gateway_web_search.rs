//! Dedicated AI Gateway worker codec for the native web-search tool.

use crate::ai_gateway::{build_gateway_transport_request, parse_strict_json, valid_model};
use crate::{
    AiGatewayTransport, MAX_WEB_SEARCH_JSON_NODES, MAX_WEB_SEARCH_REQUEST_BYTES,
    MAX_WEB_SEARCH_RESPONSE_BYTES, MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES,
    MAX_WEB_SEARCH_RESPONSE_RECORDS, MAX_WEB_SEARCH_SOURCES, WebSearchConfigError,
    WebSearchConfigErrorKind, WebSearchRequest, WebSearchResponse, WebSearchSource,
    WebSearchTransport, WebSearchTransportError, WebSearchTransportErrorKind,
};
use machine_god_core::{BoxFuture, CancellationToken, ProviderError, ProviderErrorKind};
use serde_json::{Value, json};
use std::fmt;
use std::future::poll_fn;
use std::sync::Arc;

const SEARCH_SYSTEM_PROMPT: &str =
    "Research the user's query with the web_search tool and preserve sources for citation.";
const PROVIDER_TOOL_NAME: &str = "perplexity_search";
const PROVIDER_TOOL_ID: &str = "gateway.perplexity_search";
const MAX_OUTPUT_TOKENS: u32 = 4_096;
const MAX_PROVIDER_RESULTS: usize = 10;

/// One-shot Perplexity search worker over an existing authenticated Gateway transport.
pub struct AiGatewayWebSearchTransport {
    model: String,
    inner: Arc<dyn AiGatewayTransport>,
}

impl AiGatewayWebSearchTransport {
    /// Constructs the dedicated worker codec without performing I/O.
    ///
    /// # Errors
    ///
    /// Returns a fixed error when the selected model is not a valid Gateway
    /// model identifier.
    pub fn new(
        model: impl Into<String>,
        inner: Arc<dyn AiGatewayTransport>,
    ) -> Result<Self, WebSearchConfigError> {
        let model = model.into();
        if !valid_model(&model) {
            return Err(WebSearchConfigError::new(
                WebSearchConfigErrorKind::InvalidModel,
            ));
        }
        Ok(Self { model, inner })
    }
}

impl fmt::Debug for AiGatewayWebSearchTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayWebSearchTransport")
            .field("model", &"<redacted>")
            .field("transport", &"<redacted>")
            .finish()
    }
}

impl WebSearchTransport for AiGatewayWebSearchTransport {
    fn search(
        &self,
        request: WebSearchRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<WebSearchResponse, WebSearchTransportError>> {
        Box::pin(async move {
            check_cancelled(&cancellation)?;
            let session = request
                .session_id()
                .ok_or_else(|| transport_error(WebSearchTransportErrorKind::InvalidRequest))?;
            let body = build_worker_body(&request)?;
            check_cancelled(&cancellation)?;
            let gateway_request = build_gateway_transport_request(&self.model, session, body);
            let mut stream = self
                .inner
                .stream(gateway_request, cancellation.clone())
                .await
                .map_err(|error| map_provider_error(&error))?;
            check_cancelled(&cancellation)?;

            let mut decoder = SseDecoder::default();
            loop {
                check_cancelled(&cancellation)?;
                let next = poll_fn(|context| stream.as_mut().poll_next(context)).await;
                let Some(chunk) = next else {
                    break;
                };
                let chunk = chunk.map_err(|error| map_provider_error(&error))?;
                decoder.push(&chunk, &cancellation)?;
            }
            check_cancelled(&cancellation)?;
            decoder.finish(&cancellation)
        })
    }
}

fn build_worker_body(request: &WebSearchRequest) -> Result<Vec<u8>, WebSearchTransportError> {
    let mut args = serde_json::Map::new();
    args.insert("maxResults".to_owned(), json!(MAX_PROVIDER_RESULTS));
    args.insert("maxTokens".to_owned(), json!(MAX_OUTPUT_TOKENS));
    if let Some(allowed) = request.allowed_domains() {
        args.insert("searchDomainFilter".to_owned(), json!(allowed));
    } else if let Some(blocked) = request.blocked_domains() {
        let blocked = blocked
            .iter()
            .map(|domain| format!("-{domain}"))
            .collect::<Vec<_>>();
        args.insert("searchDomainFilter".to_owned(), json!(blocked));
    }

    let body = json!({
        "prompt": [
            {"role": "system", "content": SEARCH_SYSTEM_PROMPT},
            {"role": "user", "content": [{"type": "text", "text": request.query()}]}
        ],
        "tools": [{
            "type": "provider",
            "id": PROVIDER_TOOL_ID,
            "name": PROVIDER_TOOL_NAME,
            "args": Value::Object(args)
        }],
        "toolChoice": {"type": "required"},
        "maxOutputTokens": MAX_OUTPUT_TOKENS
    });
    let bytes = serde_json::to_vec(&body)
        .map_err(|_| transport_error(WebSearchTransportErrorKind::InvalidRequest))?;
    if bytes.len() > MAX_WEB_SEARCH_REQUEST_BYTES {
        return Err(transport_error(WebSearchTransportErrorKind::InvalidRequest));
    }
    Ok(bytes)
}

#[derive(Default)]
struct SseDecoder {
    record: Vec<u8>,
    total_bytes: usize,
    records: usize,
    nodes: usize,
    pending_cr: bool,
    pending_line_end: bool,
    state: ResponseState,
}

impl SseDecoder {
    fn push(
        &mut self,
        chunk: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<(), WebSearchTransportError> {
        self.total_bytes = self
            .total_bytes
            .checked_add(chunk.len())
            .filter(|length| *length <= MAX_WEB_SEARCH_RESPONSE_BYTES)
            .ok_or_else(response_too_large)?;
        check_cancelled(cancellation)?;
        for &byte in chunk {
            if self.pending_cr {
                if byte != b'\n' {
                    return Err(protocol_error());
                }
                self.pending_cr = false;
                self.push_normalized(b'\n', cancellation)?;
            } else if byte == b'\r' {
                self.pending_cr = true;
            } else {
                self.push_normalized(byte, cancellation)?;
            }
        }
        Ok(())
    }

    fn push_normalized(
        &mut self,
        byte: u8,
        cancellation: &CancellationToken,
    ) -> Result<(), WebSearchTransportError> {
        if byte == b'\n' {
            if self.pending_line_end {
                self.pending_line_end = false;
                self.consume_record(cancellation)?;
            } else {
                self.pending_line_end = true;
            }
            return Ok(());
        }
        if self.pending_line_end {
            return Err(protocol_error());
        }
        if self.record.len() == MAX_WEB_SEARCH_RESPONSE_RECORD_BYTES {
            return Err(response_too_large());
        }
        self.record.push(byte);
        Ok(())
    }

    fn consume_record(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), WebSearchTransportError> {
        check_cancelled(cancellation)?;
        if self.record.is_empty() {
            return Ok(());
        }
        self.records = self.records.checked_add(1).ok_or_else(protocol_error)?;
        if self.records > MAX_WEB_SEARCH_RESPONSE_RECORDS {
            return Err(response_too_large());
        }
        let raw_record = std::str::from_utf8(&self.record).map_err(|_| protocol_error())?;
        let data = raw_record
            .strip_prefix("data:")
            .ok_or_else(protocol_error)?
            .strip_prefix(' ')
            .unwrap_or_else(|| raw_record.strip_prefix("data:").expect("prefix checked"));
        if self.state.done {
            return Err(protocol_error());
        }
        if data == "[DONE]" {
            if !self.state.finished {
                return Err(protocol_error());
            }
            self.state.done = true;
            self.record.clear();
            return Ok(());
        }
        let remaining = MAX_WEB_SEARCH_JSON_NODES
            .checked_sub(self.nodes)
            .filter(|remaining| *remaining > 0)
            .ok_or_else(response_too_large)?;
        let event = parse_strict_json(data, remaining).map_err(|_| protocol_error())?;
        self.nodes = self
            .nodes
            .checked_add(json_node_count(&event))
            .filter(|count| *count <= MAX_WEB_SEARCH_JSON_NODES)
            .ok_or_else(response_too_large)?;
        self.state.consume(event)?;
        self.record.clear();
        Ok(())
    }

    fn finish(
        mut self,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchTransportError> {
        check_cancelled(cancellation)?;
        if self.pending_cr {
            return Err(protocol_error());
        }
        if self.pending_line_end || !self.record.is_empty() {
            self.pending_line_end = false;
            self.consume_record(cancellation)?;
        }
        if self.state.call_id.is_none() || self.state.result.is_none() || !self.state.finished {
            return Err(protocol_error());
        }
        decode_provider_result(self.state.result.expect("presence checked"))
    }
}

#[derive(Default)]
struct ResponseState {
    call_id: Option<String>,
    result: Option<Value>,
    finished: bool,
    done: bool,
}

impl ResponseState {
    fn consume(&mut self, event: Value) -> Result<(), WebSearchTransportError> {
        let Value::Object(mut object) = event else {
            return Err(protocol_error());
        };
        let event_type = object
            .remove("type")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(protocol_error)?;
        match event_type.as_str() {
            "response-metadata" if self.call_id.is_none() => Ok(()),
            "tool-call" => self.consume_call(&object),
            "tool-result" => self.consume_result(object),
            "finish" => self.consume_finish(&object),
            _ => Err(protocol_error()),
        }
    }

    fn consume_call(
        &mut self,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), WebSearchTransportError> {
        if self.call_id.is_some() || self.result.is_some() || self.finished {
            return Err(protocol_error());
        }
        let id = required_string(object, "toolCallId")?;
        if id.is_empty()
            || required_string(object, "toolName")? != PROVIDER_TOOL_NAME
            || object.get("providerExecuted").and_then(Value::as_bool) != Some(true)
            || !object.get("input").is_some_and(Value::is_object)
        {
            return Err(protocol_error());
        }
        self.call_id = Some(id.to_owned());
        Ok(())
    }

    fn consume_result(
        &mut self,
        mut object: serde_json::Map<String, Value>,
    ) -> Result<(), WebSearchTransportError> {
        if self.result.is_some() || self.finished {
            return Err(protocol_error());
        }
        let Some(expected_id) = self.call_id.as_deref() else {
            return Err(protocol_error());
        };
        if required_string(&object, "toolCallId")? != expected_id
            || object
                .get("preliminary")
                .is_some_and(|value| value != &Value::Bool(false))
        {
            return Err(protocol_error());
        }
        let result = object.remove("result").ok_or_else(protocol_error)?;
        if result.is_null() {
            return Err(protocol_error());
        }
        self.result = Some(result);
        Ok(())
    }

    fn consume_finish(
        &mut self,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), WebSearchTransportError> {
        if self.call_id.is_none() || self.result.is_none() || self.finished {
            return Err(protocol_error());
        }
        let reason = object
            .get("finishReason")
            .and_then(Value::as_object)
            .and_then(|reason| reason.get("unified"))
            .and_then(Value::as_str);
        if reason != Some("stop") {
            return Err(protocol_error());
        }
        self.finished = true;
        Ok(())
    }
}

fn decode_provider_result(result: Value) -> Result<WebSearchResponse, WebSearchTransportError> {
    let Value::Object(mut object) = result else {
        return Err(protocol_error());
    };
    if object.len() != 1 {
        return Err(protocol_error());
    }
    let Value::Array(values) = object.remove("results").ok_or_else(protocol_error)? else {
        return Err(protocol_error());
    };
    let mut sources = Vec::new();
    let mut truncated = false;
    for value in values {
        let Value::Object(mut source) = value else {
            return Err(protocol_error());
        };
        if source.len() != 2 {
            return Err(protocol_error());
        }
        let title = source
            .remove("title")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(protocol_error)?;
        let url = source
            .remove("url")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(protocol_error)?;
        let candidate = WebSearchSource::new(title, url)?;
        if sources
            .iter()
            .any(|source: &WebSearchSource| source.url() == candidate.url())
        {
            continue;
        }
        if sources.len() < MAX_WEB_SEARCH_SOURCES {
            sources.push(candidate);
        } else {
            truncated = true;
        }
    }
    WebSearchResponse::new(sources, truncated)
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    name: &str,
) -> Result<&'a str, WebSearchTransportError> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(protocol_error)
}

fn json_node_count(value: &Value) -> usize {
    let mut count = 0_usize;
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        count = count.saturating_add(1);
        match value {
            Value::Array(values) => stack.extend(values),
            Value::Object(values) => stack.extend(values.values()),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    count
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), WebSearchTransportError> {
    if cancellation.is_cancelled() {
        Err(transport_error(WebSearchTransportErrorKind::Cancelled))
    } else {
        Ok(())
    }
}

fn map_provider_error(error: &ProviderError) -> WebSearchTransportError {
    let kind = match error.kind {
        ProviderErrorKind::Authentication => WebSearchTransportErrorKind::Authentication,
        ProviderErrorKind::RateLimited => WebSearchTransportErrorKind::RateLimited,
        ProviderErrorKind::InvalidRequest => WebSearchTransportErrorKind::InvalidRequest,
        ProviderErrorKind::Cancelled => WebSearchTransportErrorKind::Cancelled,
        ProviderErrorKind::Unavailable | ProviderErrorKind::Transport => {
            WebSearchTransportErrorKind::Unavailable
        }
        ProviderErrorKind::Protocol | ProviderErrorKind::Other => {
            WebSearchTransportErrorKind::Protocol
        }
        _ => WebSearchTransportErrorKind::Unavailable,
    };
    transport_error(kind)
}

fn protocol_error() -> WebSearchTransportError {
    transport_error(WebSearchTransportErrorKind::Protocol)
}

fn response_too_large() -> WebSearchTransportError {
    transport_error(WebSearchTransportErrorKind::ResponseTooLarge)
}

fn transport_error(kind: WebSearchTransportErrorKind) -> WebSearchTransportError {
    WebSearchTransportError::new(kind)
}
