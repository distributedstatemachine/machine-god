//! Bounded session-backed paging for projected tool results.

use crate::session_store::{
    JsonValueOwner, MAX_STORED_JSON_DEPTH, MAX_STORED_JSON_NODES, RecordOwner,
};
use crate::tool_output_serializer::{
    CompactToolOutputError, CompactToolOutputLimits, measure_json_value_compact,
    serialize_tool_output_compact,
};
use crate::tool_result_projection::{tool_result_digest, valid_tool_result_handle};
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, PreparedToolCall, Role, SessionRecord,
    SessionStore, Tool, ToolCall, ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput,
    ToolSpec,
};
use serde_json::{Map, Number, Value, json};
use std::fmt;
use std::future::{Future, poll_fn};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Exact registered tool name.
pub const READ_TOOL_RESULT_TOOL_NAME: &str = "read_tool_result";

const DESCRIPTION: &str = "Read a UTF-8-safe byte range from a prior tool result using its session-scoped preview handle.";
const MAX_ARGUMENT_BYTES: usize = 512;
const DEFAULT_START_BYTE: usize = 1;
const MAX_START_BYTE: usize = 65_537;
const DEFAULT_PAGE_BYTES: usize = 8_192;
const MIN_PAGE_BYTES: usize = 4;
const MAX_PAGE_BYTES: usize = 16_384;
const DEFAULT_MAX_ACTIVE_READS: usize = 2;
const HARD_MAX_ACTIVE_READS: usize = 8;
const DEFAULT_MAX_SCANNED_RESULTS: usize = 4_096;
const DEFAULT_MAX_SERIALIZED_SCAN_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCANNED_MESSAGES: usize = 4_096;
const MAX_SCANNED_CONTENT_BLOCKS: usize = 65_536;
const MAX_SERIALIZED_TOOL_RESULT_BYTES: usize = 65_536;
const TOOL_RESULT_HANDLE_PREFIX: &str = "tool-result-sha256-";

/// Stable construction-error category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReadToolResultConfigErrorKind {
    /// One or more resource limits are zero or exceed the production ceiling.
    InvalidLimits,
}

/// Fixed, redacted reader construction failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ReadToolResultConfigError {
    kind: ReadToolResultConfigErrorKind,
}

impl ReadToolResultConfigError {
    /// Returns the stable category.
    #[must_use]
    pub const fn kind(self) -> ReadToolResultConfigErrorKind {
        self.kind
    }

    const fn invalid_limits() -> Self {
        Self {
            kind: ReadToolResultConfigErrorKind::InvalidLimits,
        }
    }
}

impl fmt::Debug for ReadToolResultConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadToolResultConfigError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for ReadToolResultConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid read_tool_result limits")
    }
}

impl std::error::Error for ReadToolResultConfigError {}

/// Bounded scan and concurrency limits for [`ReadToolResultTool`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadToolResultLimits {
    active_reads: usize,
    scanned_tool_results: usize,
    serialized_scan_bytes: usize,
}

impl ReadToolResultLimits {
    /// Constructs limits no broader than the production defaults.
    ///
    /// # Errors
    ///
    /// Returns a fixed error for zero values or values above the hard ceiling.
    pub const fn new(
        max_active_reads: usize,
        max_scanned_tool_results: usize,
        max_serialized_scan_bytes: usize,
    ) -> Result<Self, ReadToolResultConfigError> {
        if max_active_reads == 0
            || max_active_reads > HARD_MAX_ACTIVE_READS
            || max_scanned_tool_results == 0
            || max_scanned_tool_results > DEFAULT_MAX_SCANNED_RESULTS
            || max_serialized_scan_bytes == 0
            || max_serialized_scan_bytes > DEFAULT_MAX_SERIALIZED_SCAN_BYTES
        {
            return Err(ReadToolResultConfigError::invalid_limits());
        }
        Ok(Self {
            active_reads: max_active_reads,
            scanned_tool_results: max_scanned_tool_results,
            serialized_scan_bytes: max_serialized_scan_bytes,
        })
    }

    /// Maximum simultaneous active reads.
    #[must_use]
    pub const fn max_active_reads(self) -> usize {
        self.active_reads
    }

    /// Maximum prior tool-result blocks inspected by one execution.
    #[must_use]
    pub const fn max_scanned_tool_results(self) -> usize {
        self.scanned_tool_results
    }

    /// Maximum aggregate compact result bytes serialized by one execution.
    #[must_use]
    pub const fn max_serialized_scan_bytes(self) -> usize {
        self.serialized_scan_bytes
    }
}

impl Default for ReadToolResultLimits {
    fn default() -> Self {
        Self {
            active_reads: DEFAULT_MAX_ACTIVE_READS,
            scanned_tool_results: DEFAULT_MAX_SCANNED_RESULTS,
            serialized_scan_bytes: DEFAULT_MAX_SERIALIZED_SCAN_BYTES,
        }
    }
}

/// Rootless bounded reader over an explicitly injected session store.
pub struct ReadToolResultTool {
    session_store: Arc<dyn SessionStore>,
    limits: ReadToolResultLimits,
    active_reads: Arc<AtomicUsize>,
}

impl ReadToolResultTool {
    /// Constructs a reader using production defaults.
    #[must_use]
    pub fn shared_session_store(session_store: Arc<dyn SessionStore>) -> Self {
        Self {
            session_store,
            limits: ReadToolResultLimits::default(),
            active_reads: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Constructs a reader with explicit bounded limits.
    ///
    /// # Errors
    ///
    /// Returns a fixed error if `limits` could not have been constructed under
    /// this version's production ceilings.
    pub fn with_limits(
        session_store: Arc<dyn SessionStore>,
        limits: ReadToolResultLimits,
    ) -> Result<Self, ReadToolResultConfigError> {
        let limits = ReadToolResultLimits::new(
            limits.active_reads,
            limits.scanned_tool_results,
            limits.serialized_scan_bytes,
        )?;
        Ok(Self {
            session_store,
            limits,
            active_reads: Arc::new(AtomicUsize::new(0)),
        })
    }
}

impl fmt::Debug for ReadToolResultTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadToolResultTool")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Tool for ReadToolResultTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new(READ_TOOL_RESULT_TOOL_NAME)
                .expect("read_tool_result is a valid static tool name"),
            description: DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "handle": {
                        "type": "string",
                        "minLength": 83,
                        "maxLength": 83,
                        "pattern": "^tool-result-sha256-[0-9a-f]{64}$"
                    },
                    "start_byte": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_START_BYTE,
                        "default": DEFAULT_START_BYTE
                    },
                    "byte_count": {
                        "type": "integer",
                        "minimum": MIN_PAGE_BYTES,
                        "maximum": MAX_PAGE_BYTES,
                        "default": DEFAULT_PAGE_BYTES
                    }
                },
                "required": ["handle"],
                "additionalProperties": false
            }),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        let ToolCall {
            name, arguments, ..
        } = call;
        let arguments = JsonValueOwner::new(arguments);
        if name.as_str() != READ_TOOL_RESULT_TOOL_NAME {
            return Err(invalid_arguments());
        }
        let arguments = normalize_arguments(arguments.get(), &CancellationToken::new())?;
        Ok(PreparedToolCall::without_authority(arguments.into_value()))
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        let session_store = Arc::clone(&self.session_store);
        let limits = self.limits;
        let active_reads = Arc::clone(&self.active_reads);
        let arguments = JsonValueOwner::new(arguments);
        Box::pin(async move {
            check_cancellation(&cancellation)?;
            let normalized = match normalize_arguments(arguments.get(), &cancellation) {
                Ok(arguments) => arguments,
                Err(error) => return checked_error(&cancellation, error),
            };
            check_cancellation(&cancellation)?;
            let Some(_permit) = try_acquire(active_reads, limits.active_reads) else {
                check_cancellation(&cancellation)?;
                return Err(read_busy());
            };
            check_cancellation(&cancellation)?;

            let mut load = session_store.load(context.session_id.clone());
            let mut cancellation_wait = Box::pin(cancellation.cancelled());
            let load_result = poll_fn(|poll_context| {
                if cancellation_wait.as_mut().poll(poll_context).is_ready() {
                    return std::task::Poll::Ready(Err(cancelled()));
                }
                let result = load
                    .as_mut()
                    .poll(poll_context)
                    .map(|result| result.map(|record| record.map(RecordOwner::new)));
                if cancellation.is_cancelled() {
                    return std::task::Poll::Ready(Err(cancelled()));
                }
                result.map(|result| result.map_err(|error| store_unavailable(error.retryable)))
            })
            .await;
            drop(cancellation_wait);
            drop(load);
            check_cancellation(&cancellation)?;
            let loaded = match load_result {
                Ok(loaded) => loaded,
                Err(error) => return checked_error(&cancellation, error),
            };
            let Some(record) = loaded else {
                return checked_error(&cancellation, not_found());
            };
            if record.get().id != context.session_id
                || record.get().incarnation_id != context.session_incarnation_id
            {
                return checked_error(&cancellation, not_found());
            }
            check_cancellation(&cancellation)?;
            let prior_message_end = match prepass_record(record.get(), &context, &cancellation) {
                Ok(prior_message_end) => prior_message_end,
                Err(error) => return checked_error(&cancellation, error),
            };
            check_cancellation(&cancellation)?;

            match scan_record(
                record.get(),
                prior_message_end,
                &context,
                &normalized,
                limits,
                &cancellation,
            ) {
                Ok(output) => {
                    check_cancellation(&cancellation)?;
                    Ok(output)
                }
                Err(error) => checked_error(&cancellation, error),
            }
        })
    }
}

fn scan_record(
    record: &SessionRecord,
    prior_message_end: usize,
    context: &ToolContext,
    arguments: &NormalizedArguments,
    limits: ReadToolResultLimits,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    let mut scanned_results = 0_usize;
    let mut scanned_bytes = 0_usize;
    let mut serialized = Vec::new();
    for message in record.messages[..prior_message_end].iter().rev() {
        check_cancellation(cancellation)?;
        for block in message.content.iter().rev() {
            check_cancellation(cancellation)?;
            let ContentBlock::ToolResult {
                ref call_id,
                ref output,
            } = *block
            else {
                continue;
            };
            scanned_results = match scanned_results.checked_add(1) {
                Some(value) if value <= limits.scanned_tool_results => value,
                _ => return checked_error(cancellation, not_found()),
            };
            let Some(remaining) = limits.serialized_scan_bytes.checked_sub(scanned_bytes) else {
                return checked_error(cancellation, not_found());
            };
            if let Err(error) = serialize_output_bounded(
                output,
                &mut serialized,
                remaining.min(MAX_SERIALIZED_TOOL_RESULT_BYTES),
                cancellation,
            ) {
                return checked_error(cancellation, error);
            }
            scanned_bytes = match scanned_bytes.checked_add(serialized.len()) {
                Some(value) if value <= limits.serialized_scan_bytes => value,
                _ => return checked_error(cancellation, not_found()),
            };
            check_cancellation(cancellation)?;
            let candidate_digest: [u8; 32] = tool_result_digest(
                &context.session_id,
                &context.session_incarnation_id,
                call_id,
                &serialized,
            )
            .into();
            if candidate_digest == arguments.digest {
                let output = match page_output(arguments, &serialized) {
                    Ok(output) => output,
                    Err(error) => return checked_error(cancellation, error),
                };
                check_cancellation(cancellation)?;
                return Ok(output);
            }
        }
    }
    checked_error(cancellation, not_found())
}

fn prepass_record(
    record: &SessionRecord,
    context: &ToolContext,
    cancellation: &CancellationToken,
) -> Result<usize, ToolError> {
    prepass_record_observed(record, context, cancellation, || {})
}

fn prepass_record_observed(
    record: &SessionRecord,
    context: &ToolContext,
    cancellation: &CancellationToken,
    mut observe: impl FnMut(),
) -> Result<usize, ToolError> {
    check_cancellation(cancellation)?;
    if record.messages.len() > MAX_SCANNED_MESSAGES {
        return checked_error(cancellation, not_found());
    }

    let mut json_budget = StoredJsonBudget { nodes: 0 };
    for value in record.metadata.values() {
        json_budget.validate(value, cancellation, &mut observe)?;
    }

    let mut inspected_blocks = 0_usize;
    let mut current_round = None;
    for (message_index, message) in record.messages.iter().enumerate() {
        observe();
        check_cancellation(cancellation)?;
        inspected_blocks = inspected_blocks
            .checked_add(message.content.len())
            .filter(|blocks| *blocks <= MAX_SCANNED_CONTENT_BLOCKS)
            .ok_or_else(not_found)?;

        let mut contains_current_call = false;
        for block in &message.content {
            observe();
            check_cancellation(cancellation)?;
            match block {
                ContentBlock::Json { value } => {
                    json_budget.validate(value, cancellation, &mut observe)?;
                }
                ContentBlock::ToolCall { call } => {
                    contains_current_call |=
                        message.role == Role::Assistant && call.id == context.call_id;
                    json_budget.validate(&call.arguments, cancellation, &mut observe)?;
                }
                ContentBlock::ToolResult { output, .. } => {
                    json_budget.validate(&output.content, cancellation, &mut observe)?;
                }
                ContentBlock::Text { .. } | _ => {}
            }
        }
        if contains_current_call {
            current_round = Some(message_index);
        }
    }
    check_cancellation(cancellation)?;
    Ok(current_round.unwrap_or(record.messages.len()))
}

struct StoredJsonBudget {
    nodes: usize,
}

impl StoredJsonBudget {
    fn validate(
        &mut self,
        root: &Value,
        cancellation: &CancellationToken,
        observe: &mut impl FnMut(),
    ) -> Result<(), ToolError> {
        let mut frames = Vec::<StoredJsonFrame<'_>>::new();
        let mut current = Some((root, 0_usize));
        loop {
            if let Some((value, parent_depth)) = current.take() {
                observe();
                check_cancellation(cancellation)?;
                self.nodes = self
                    .nodes
                    .checked_add(1)
                    .filter(|nodes| *nodes <= MAX_STORED_JSON_NODES)
                    .ok_or_else(resource_limit)?;
                let children = match value {
                    Value::Array(values) => Some(StoredJsonChildren::Array(values.iter())),
                    Value::Object(values) => Some(StoredJsonChildren::Object(values.values())),
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
                };
                if let Some(children) = children {
                    let container_depth = parent_depth.checked_add(1).ok_or_else(resource_limit)?;
                    if container_depth > MAX_STORED_JSON_DEPTH {
                        return checked_error(cancellation, resource_limit());
                    }
                    frames.push(StoredJsonFrame {
                        container_depth,
                        children,
                    });
                }
            }

            loop {
                let Some(frame) = frames.last_mut() else {
                    return Ok(());
                };
                if let Some(child) = frame.children.next() {
                    current = Some((child, frame.container_depth));
                    break;
                }
                frames.pop();
            }
        }
    }
}

struct StoredJsonFrame<'a> {
    container_depth: usize,
    children: StoredJsonChildren<'a>,
}

enum StoredJsonChildren<'a> {
    Array(std::slice::Iter<'a, Value>),
    Object(serde_json::map::Values<'a>),
}

impl<'a> Iterator for StoredJsonChildren<'a> {
    type Item = &'a Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Array(children) => children.next(),
            Self::Object(children) => children.next(),
        }
    }
}

#[derive(Debug)]
struct NormalizedArguments {
    handle: String,
    digest: [u8; 32],
    start_byte: usize,
    byte_count: usize,
}

impl NormalizedArguments {
    fn into_value(self) -> Value {
        let mut object = Map::new();
        object.insert("handle".to_owned(), Value::String(self.handle));
        object.insert(
            "start_byte".to_owned(),
            Value::Number(Number::from(self.start_byte as u64)),
        );
        object.insert(
            "byte_count".to_owned(),
            Value::Number(Number::from(self.byte_count as u64)),
        );
        Value::Object(object)
    }
}

fn normalize_arguments(
    arguments: &Value,
    cancellation: &CancellationToken,
) -> Result<NormalizedArguments, ToolError> {
    check_argument_bytes(arguments, cancellation)?;
    let Value::Object(object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() > 3
        || object
            .keys()
            .any(|key| !matches!(key.as_str(), "handle" | "start_byte" | "byte_count"))
    {
        return Err(invalid_arguments());
    }
    let handle = object
        .get("handle")
        .and_then(Value::as_str)
        .filter(|handle| valid_tool_result_handle(handle))
        .ok_or_else(invalid_arguments)?;
    let digest = parse_handle_digest(handle).ok_or_else(invalid_arguments)?;
    let handle = handle.to_owned();
    let start_byte = optional_usize(object.get("start_byte"), DEFAULT_START_BYTE)?;
    let byte_count = optional_usize(object.get("byte_count"), DEFAULT_PAGE_BYTES)?;
    if start_byte == 0
        || start_byte > MAX_START_BYTE
        || !(MIN_PAGE_BYTES..=MAX_PAGE_BYTES).contains(&byte_count)
    {
        return Err(invalid_arguments());
    }
    Ok(NormalizedArguments {
        handle,
        digest,
        start_byte,
        byte_count,
    })
}

fn optional_usize(value: Option<&Value>, default: usize) -> Result<usize, ToolError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let number = value.as_u64().ok_or_else(invalid_arguments)?;
    usize::try_from(number).map_err(|_| invalid_arguments())
}

fn parse_handle_digest(handle: &str) -> Option<[u8; 32]> {
    let encoded = handle.strip_prefix(TOOL_RESULT_HANDLE_PREFIX)?.as_bytes();
    if encoded.len() != 64 {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, pair) in encoded.chunks_exact(2).enumerate() {
        digest[index] = decode_hex(pair[0])?
            .checked_mul(16)?
            .checked_add(decode_hex(pair[1])?)?;
    }
    Some(digest)
}

const fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn serialize_output_bounded(
    output: &ToolOutput,
    serialized: &mut Vec<u8>,
    limit: usize,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    serialize_tool_output_compact(
        output,
        serialized,
        CompactToolOutputLimits {
            output_bytes: limit,
            json_depth: MAX_STORED_JSON_DEPTH,
            json_nodes: MAX_STORED_JSON_NODES,
        },
        cancellation,
    )
    .map_err(|error| match error {
        CompactToolOutputError::Cancelled => cancelled(),
        CompactToolOutputError::OutputLimit => not_found(),
        CompactToolOutputError::JsonDepth
        | CompactToolOutputError::JsonNodes
        | CompactToolOutputError::Invalid => resource_limit(),
    })
}

fn check_argument_bytes(
    arguments: &Value,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    measure_json_value_compact(
        arguments,
        CompactToolOutputLimits {
            output_bytes: MAX_ARGUMENT_BYTES,
            json_depth: MAX_STORED_JSON_DEPTH,
            json_nodes: MAX_STORED_JSON_NODES,
        },
        cancellation,
    )
    .map(|_| ())
    .map_err(|error| match error {
        CompactToolOutputError::Cancelled => cancelled(),
        CompactToolOutputError::OutputLimit
        | CompactToolOutputError::JsonDepth
        | CompactToolOutputError::JsonNodes
        | CompactToolOutputError::Invalid => resource_limit(),
    })
}

fn page_output(
    arguments: &NormalizedArguments,
    serialized: &[u8],
) -> Result<ToolOutput, ToolError> {
    let source = std::str::from_utf8(serialized).map_err(|_| resource_limit())?;
    let start = arguments.start_byte - 1;
    if start > source.len() || !source.is_char_boundary(start) {
        return Err(invalid_arguments());
    }
    let mut end = start.saturating_add(arguments.byte_count).min(source.len());
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let end_byte = if start == source.len() {
        source.len()
    } else {
        end
    };
    Ok(ToolOutput::success(json!({
        "handle": arguments.handle,
        "start_byte": arguments.start_byte,
        "end_byte": end_byte,
        "total_bytes": source.len(),
        "serialized_tool_output": &source[start..end],
        "has_more": end < source.len()
    })))
}

struct ActiveReadPermit {
    active_reads: Arc<AtomicUsize>,
}

fn try_acquire(
    active_reads: Arc<AtomicUsize>,
    max_active_reads: usize,
) -> Option<ActiveReadPermit> {
    active_reads
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
            (active < max_active_reads).then(|| active + 1)
        })
        .ok()
        .map(|_| ActiveReadPermit { active_reads })
}

impl Drop for ActiveReadPermit {
    fn drop(&mut self) {
        let previous = self.active_reads.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
    }
}

fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

fn checked_error<T>(cancellation: &CancellationToken, error: ToolError) -> Result<T, ToolError> {
    check_cancellation(cancellation)?;
    Err(error)
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "read_tool_result_invalid_arguments",
        "read_tool_result arguments are invalid",
        false,
    )
}

fn resource_limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "read_tool_result_resource_limit",
        "read_tool_result resource limit exceeded",
        false,
    )
}

fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "read_tool_result_not_found",
        "tool result is unavailable",
        false,
    )
}

fn store_unavailable(retryable: bool) -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "read_tool_result_unavailable",
        "tool result store is unavailable",
        retryable,
    )
}

fn read_busy() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "read_tool_result_busy",
        "read_tool_result is busy",
        true,
    )
}

fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "read_tool_result_cancelled",
        "read_tool_result was cancelled",
        false,
    )
}

#[cfg(test)]
mod prepass_tests {
    use super::*;
    use machine_god_core::{Message, SessionId, SessionIncarnationId, ToolCallId, TurnId};

    fn context(session_id: SessionId, incarnation_id: SessionIncarnationId) -> ToolContext {
        ToolContext {
            session_id,
            session_incarnation_id: incarnation_id,
            turn_id: TurnId::new("prepass-test-turn").unwrap(),
            call_id: ToolCallId::new("prepass-test-call").unwrap(),
        }
    }

    #[test]
    fn cancellation_is_observed_at_the_exact_injected_prepass_inspection() {
        let session_id = SessionId::new("prepass-cancel-session").unwrap();
        let incarnation_id = SessionIncarnationId::new("prepass-cancel-incarnation").unwrap();
        let mut record = SessionRecord::empty(session_id.clone(), incarnation_id.clone());
        record.messages.push(Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Json {
                value: serde_json::json!([0, 1, 2]),
            }],
        });
        let context = context(session_id, incarnation_id);
        let cancellation = CancellationToken::new();
        let cancellation_trigger = cancellation.clone();
        let mut inspections = 0_usize;
        let error = prepass_record_observed(&record, &context, &cancellation, || {
            inspections += 1;
            if inspections == 3 {
                cancellation_trigger.cancel();
            }
        })
        .unwrap_err();

        assert_eq!(inspections, 3);
        assert_eq!(error.kind, ToolErrorKind::Cancelled);
        assert_eq!(error.code, "read_tool_result_cancelled");
    }

    #[test]
    fn structural_overflow_is_rejected_before_excess_content_is_inspected() {
        let session_id = SessionId::new("prepass-structure-session").unwrap();
        let incarnation_id = SessionIncarnationId::new("prepass-structure-incarnation").unwrap();
        let tool_context = context(session_id.clone(), incarnation_id.clone());
        let cancellation = CancellationToken::new();

        let mut message_overflow = SessionRecord::empty(session_id.clone(), incarnation_id.clone());
        message_overflow
            .messages
            .extend((0..=MAX_SCANNED_MESSAGES).map(|_| Message {
                role: Role::Tool,
                content: Vec::new(),
            }));
        let mut message_inspections = 0_usize;
        let error =
            prepass_record_observed(&message_overflow, &tool_context, &cancellation, || {
                message_inspections += 1;
            })
            .unwrap_err();
        assert_eq!(error.code, "read_tool_result_not_found");
        assert_eq!(message_inspections, 0);

        let mut block_overflow = SessionRecord::empty(session_id, incarnation_id);
        block_overflow.messages.push(Message {
            role: Role::Tool,
            content: (0..=MAX_SCANNED_CONTENT_BLOCKS)
                .map(|_| ContentBlock::Text {
                    text: String::new(),
                })
                .collect(),
        });
        let mut block_inspections = 0_usize;
        let error = prepass_record_observed(&block_overflow, &tool_context, &cancellation, || {
            block_inspections += 1;
        })
        .unwrap_err();
        assert_eq!(error.code, "read_tool_result_not_found");
        assert_eq!(block_inspections, 1);
    }
}
