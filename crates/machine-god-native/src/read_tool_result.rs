//! Bounded session-backed paging for projected tool results.

use crate::tool_result_projection::{tool_result_handle, valid_tool_result_handle};
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, PreparedToolCall, SessionStore, Tool, ToolCall,
    ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde_json::{Map, Number, Value, json};
use std::fmt;
use std::future::{Future, poll_fn};
use std::io::{self, Write};
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
    max_active_reads: usize,
    max_scanned_tool_results: usize,
    max_serialized_scan_bytes: usize,
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
            max_active_reads,
            max_scanned_tool_results,
            max_serialized_scan_bytes,
        })
    }

    /// Maximum simultaneous active reads.
    #[must_use]
    pub const fn max_active_reads(self) -> usize {
        self.max_active_reads
    }

    /// Maximum prior tool-result blocks inspected by one execution.
    #[must_use]
    pub const fn max_scanned_tool_results(self) -> usize {
        self.max_scanned_tool_results
    }

    /// Maximum aggregate compact result bytes serialized by one execution.
    #[must_use]
    pub const fn max_serialized_scan_bytes(self) -> usize {
        self.max_serialized_scan_bytes
    }
}

impl Default for ReadToolResultLimits {
    fn default() -> Self {
        Self {
            max_active_reads: DEFAULT_MAX_ACTIVE_READS,
            max_scanned_tool_results: DEFAULT_MAX_SCANNED_RESULTS,
            max_serialized_scan_bytes: DEFAULT_MAX_SERIALIZED_SCAN_BYTES,
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
            limits.max_active_reads,
            limits.max_scanned_tool_results,
            limits.max_serialized_scan_bytes,
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
        if call.name.as_str() != READ_TOOL_RESULT_TOOL_NAME {
            return Err(invalid_arguments());
        }
        let arguments = normalize_arguments(&call.arguments)?;
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
        Box::pin(async move {
            check_cancellation(&cancellation)?;
            let normalized = normalize_arguments(&arguments)?;
            check_cancellation(&cancellation)?;
            let Some(permit) = try_acquire(active_reads, limits.max_active_reads) else {
                check_cancellation(&cancellation)?;
                return Err(read_busy());
            };
            check_cancellation(&cancellation)?;

            let mut load = session_store.load(context.session_id.clone());
            let mut cancellation_wait = Box::pin(cancellation.cancelled());
            let loaded = poll_fn(|poll_context| {
                if cancellation_wait.as_mut().poll(poll_context).is_ready() {
                    return std::task::Poll::Ready(Err(cancelled()));
                }
                match load.as_mut().poll(poll_context) {
                    std::task::Poll::Ready(result) => {
                        if cancellation.is_cancelled() {
                            std::task::Poll::Ready(Err(cancelled()))
                        } else {
                            std::task::Poll::Ready(
                                result.map_err(|error| store_unavailable(error.retryable)),
                            )
                        }
                    }
                    std::task::Poll::Pending => std::task::Poll::Pending,
                }
            })
            .await?;
            let Some(record) = loaded else {
                return Err(not_found());
            };
            if record.id != context.session_id
                || record.incarnation_id != context.session_incarnation_id
            {
                return Err(not_found());
            }

            let mut scanned_results = 0_usize;
            let mut scanned_bytes = 0_usize;
            for message in &record.messages {
                check_cancellation(&cancellation)?;
                for block in &message.content {
                    let (call_id, output) = match block {
                        ContentBlock::ToolResult { call_id, output } => (call_id, output),
                        _ => continue,
                    };
                    scanned_results = scanned_results.checked_add(1).ok_or_else(not_found)?;
                    if scanned_results > limits.max_scanned_tool_results {
                        return Err(not_found());
                    }
                    let serialized = serde_json::to_vec(output).map_err(|_| resource_limit())?;
                    scanned_bytes = scanned_bytes
                        .checked_add(serialized.len())
                        .ok_or_else(not_found)?;
                    if scanned_bytes > limits.max_serialized_scan_bytes {
                        return Err(not_found());
                    }
                    check_cancellation(&cancellation)?;
                    let candidate = tool_result_handle(
                        &context.session_id,
                        &context.session_incarnation_id,
                        call_id,
                        &serialized,
                    );
                    if candidate == normalized.handle {
                        let output = page_output(normalized, &serialized)?;
                        check_cancellation(&cancellation)?;
                        drop(permit);
                        return Ok(output);
                    }
                }
            }
            Err(not_found())
        })
    }
}

#[derive(Debug)]
struct NormalizedArguments {
    handle: String,
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

fn normalize_arguments(arguments: &Value) -> Result<NormalizedArguments, ToolError> {
    check_argument_bytes(arguments)?;
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
        .ok_or_else(invalid_arguments)?
        .to_owned();
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

fn check_argument_bytes(arguments: &Value) -> Result<(), ToolError> {
    let mut counter = BoundedCounter::new(MAX_ARGUMENT_BYTES);
    match serde_json::to_writer(&mut counter, arguments) {
        Ok(()) => Ok(()),
        Err(_) if counter.exceeded => Err(resource_limit()),
        Err(_) => Err(invalid_arguments()),
    }
}

fn page_output(arguments: NormalizedArguments, serialized: &[u8]) -> Result<ToolOutput, ToolError> {
    let source = std::str::from_utf8(serialized).map_err(|_| resource_limit())?;
    let start = arguments.start_byte - 1;
    if start > source.len() || !source.is_char_boundary(start) {
        return Err(invalid_arguments());
    }
    let mut end = start
        .checked_add(arguments.byte_count)
        .unwrap_or(usize::MAX)
        .min(source.len());
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

struct BoundedCounter {
    remaining: usize,
    exceeded: bool,
}

impl BoundedCounter {
    const fn new(limit: usize) -> Self {
        Self {
            remaining: limit,
            exceeded: false,
        }
    }
}

impl Write for BoundedCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            self.exceeded = true;
            return Err(io::Error::other("limit"));
        }
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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
