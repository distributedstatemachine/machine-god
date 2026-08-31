//! Bounded deterministic search over an explicitly injected MCP tool catalog.

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::sync::Arc;

use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolCall, Tool, ToolCall, ToolContext, ToolError,
    ToolErrorKind, ToolName, ToolOutput, ToolSpec,
};
use serde::Serialize;
use serde_json::{Value, json};

/// Registered name of [`McpSearchToolsTool`].
pub const MCP_SEARCH_TOOLS_TOOL_NAME: &str = "mcp_search_tools";
/// Default number of matching metadata entries returned by one search.
pub const MCP_SEARCH_TOOLS_DEFAULT_LIMIT: usize = 8;
/// Maximum caller-selected number of matching metadata entries.
pub const MCP_SEARCH_TOOLS_MAX_LIMIT: usize = 20;
/// Maximum UTF-8 bytes accepted in one query.
pub const MAX_MCP_SEARCH_QUERY_BYTES: usize = 4 * 1024;
/// Maximum searchable tokens accepted in one query.
pub const MAX_MCP_SEARCH_QUERY_TOKENS: usize = 64;
/// Maximum entries in one immutable catalog snapshot.
pub const MAX_MCP_TOOL_CATALOG_ENTRIES: usize = 1_024;
/// Maximum aggregate retained metadata and private search-text bytes.
pub const MAX_MCP_TOOL_CATALOG_BYTES: usize = 8 * 1024 * 1024;
/// Maximum UTF-8 bytes in one configured server identity.
pub const MAX_MCP_TOOL_SERVER_BYTES: usize = 128;
/// Maximum UTF-8 bytes accepted in one source description.
pub const MAX_MCP_TOOL_DESCRIPTION_BYTES: usize = 8 * 1024;
/// Maximum UTF-8 bytes from one description projected to the model.
pub const MAX_MCP_SEARCH_DESCRIPTION_BYTES: usize = 1_024;
/// Maximum private schema-derived search bytes retained for one tool.
pub const MAX_MCP_TOOL_SEARCH_TEXT_BYTES: usize = 64 * 1024;
/// Maximum tags retained for one tool.
pub const MAX_MCP_TOOL_TAGS: usize = 32;
/// Maximum UTF-8 bytes retained in one tag.
pub const MAX_MCP_TOOL_TAG_BYTES: usize = 128;
/// Maximum charged substring-search work in one execution.
pub const MAX_MCP_SEARCH_MATCH_STEPS: usize = 64 * 1024 * 1024;
/// Maximum serialized canonical prepared arguments.
pub const MAX_MCP_SEARCH_SERIALIZED_ARGUMENT_BYTES: usize = 8 * 1024;
/// Maximum serialized complete [`ToolOutput`].
pub const MAX_MCP_SEARCH_SERIALIZED_RESULT_BYTES: usize = 16 * 1024;

const DESCRIPTION: &str = "Search bounded metadata for configured MCP tools without exposing executable schemas. Include the configured server and requested use case in the query";

/// Stable reason that injected catalog acquisition failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum McpToolCatalogErrorKind {
    /// The catalog source is temporarily unavailable.
    Unavailable,
    /// The catalog source exceeded one of its own fixed resource bounds.
    ResourceLimit,
    /// Catalog acquisition observed cancellation.
    Cancelled,
}

/// Fixed redacted failure from an injected catalog implementation.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct McpToolCatalogError {
    kind: McpToolCatalogErrorKind,
}

impl McpToolCatalogError {
    /// Creates a fixed catalog failure.
    #[must_use]
    pub const fn new(kind: McpToolCatalogErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(self) -> McpToolCatalogErrorKind {
        self.kind
    }
}

impl fmt::Debug for McpToolCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpToolCatalogError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for McpToolCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MCP tool catalog acquisition failed")
    }
}

impl std::error::Error for McpToolCatalogError {}

/// Stable reason that catalog metadata or a snapshot was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum McpToolCatalogBuildErrorKind {
    /// A field is empty, malformed, duplicated, or individually oversized.
    InvalidMetadata,
    /// A complete snapshot exceeded its entry or aggregate-byte bound.
    ResourceLimit,
}

/// Fixed redacted metadata-construction failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct McpToolCatalogBuildError {
    kind: McpToolCatalogBuildErrorKind,
}

impl McpToolCatalogBuildError {
    /// Returns the stable rejection category.
    #[must_use]
    pub const fn kind(self) -> McpToolCatalogBuildErrorKind {
        self.kind
    }

    const fn new(kind: McpToolCatalogBuildErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for McpToolCatalogBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpToolCatalogBuildError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for McpToolCatalogBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid MCP tool catalog metadata")
    }
}

impl std::error::Error for McpToolCatalogBuildError {}

/// Validated metadata for one configured dynamic MCP tool.
///
/// `search_text` may contain schema-derived text, but it is retained privately
/// and is never projected into tool output.
#[derive(Clone)]
pub struct McpToolMetadata {
    name: String,
    server: String,
    description: String,
    tags: Vec<String>,
    search_haystack: String,
    retained_bytes: usize,
}

impl McpToolMetadata {
    /// Validates and owns one metadata entry.
    ///
    /// # Errors
    ///
    /// Returns a fixed rejection when an identity, description, private
    /// search field, tag, or aggregate entry size is invalid.
    pub fn new(
        name: impl Into<String>,
        server: impl Into<String>,
        description: impl Into<String>,
        search_text: impl Into<String>,
        tags: Vec<String>,
    ) -> Result<Self, McpToolCatalogBuildError> {
        let name = name.into();
        let server = server.into();
        let description = description.into();
        let search_text = search_text.into();
        validate_metadata_fields(&name, &server, &description, &search_text, &tags)?;

        let mut normalized_tags = Vec::with_capacity(tags.len());
        let mut seen_tags = BTreeSet::new();
        for mut tag in tags {
            tag.make_ascii_lowercase();
            if seen_tags.insert(tag.clone()) {
                normalized_tags.push(tag);
            }
        }

        let source_bytes =
            metadata_retained_bytes(&name, &server, &description, &search_text, &normalized_tags)?;
        let mut search_haystack = String::with_capacity(source_bytes);
        for field in [&name, &server, &description, &search_text] {
            search_haystack.push_str(field);
            search_haystack.push(' ');
        }
        for tag in &normalized_tags {
            search_haystack.push_str(tag);
            search_haystack.push(' ');
        }
        search_haystack.make_ascii_lowercase();
        let retained_bytes = source_bytes
            .checked_add(search_haystack.len())
            .ok_or_else(|| {
                McpToolCatalogBuildError::new(McpToolCatalogBuildErrorKind::ResourceLimit)
            })?;

        Ok(Self {
            name,
            server,
            description,
            tags: normalized_tags,
            search_haystack,
            retained_bytes,
        })
    }

    /// Returns the exact deconflicted dynamic tool name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the exact configured server identity.
    #[must_use]
    pub fn server(&self) -> &str {
        &self.server
    }

    /// Returns the source description before model-projection truncation.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns normalized, stable-deduplicated metadata tags.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }
}

impl fmt::Debug for McpToolMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpToolMetadata")
            .field("name", &self.name)
            .field("server", &self.server)
            .field("description_bytes", &self.description.len())
            .field("tag_count", &self.tags.len())
            .finish_non_exhaustive()
    }
}

/// Availability represented by one immutable point-in-time catalog snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum McpToolCatalogState {
    /// Discovery is complete and the snapshot may contain tools.
    Ready,
    /// Discovery is still running; callers should retry later.
    Discovering,
}

/// Bounded immutable point-in-time catalog returned by [`McpToolCatalog`].
#[derive(Clone)]
pub struct McpToolCatalogSnapshot {
    state: McpToolCatalogState,
    tools: Vec<McpToolMetadata>,
}

impl McpToolCatalogSnapshot {
    /// Validates a ready catalog snapshot.
    ///
    /// # Errors
    ///
    /// Returns a fixed rejection for duplicate names or aggregate bounds.
    pub fn new(tools: Vec<McpToolMetadata>) -> Result<Self, McpToolCatalogBuildError> {
        validate_snapshot(&tools)?;
        Ok(Self {
            state: McpToolCatalogState::Ready,
            tools,
        })
    }

    /// Returns an empty snapshot whose discovery is still in progress.
    #[must_use]
    pub const fn discovering() -> Self {
        Self {
            state: McpToolCatalogState::Discovering,
            tools: Vec::new(),
        }
    }

    /// Returns this immutable snapshot's availability.
    #[must_use]
    pub const fn state(&self) -> McpToolCatalogState {
        self.state
    }

    /// Returns the validated metadata entries in source order.
    #[must_use]
    pub fn tools(&self) -> &[McpToolMetadata] {
        &self.tools
    }
}

impl fmt::Debug for McpToolCatalogSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpToolCatalogSnapshot")
            .field("state", &self.state)
            .field("tool_count", &self.tools.len())
            .finish()
    }
}

/// Explicit asynchronous authority for acquiring one bounded MCP catalog.
///
/// Implementations must perform no work until the returned future is polled.
/// Each call returns one immutable point-in-time snapshot. Search,
/// tokenization, ordering, limiting, and projection remain owned by
/// [`McpSearchToolsTool`].
pub trait McpToolCatalog: Send + Sync + 'static {
    /// Acquires one immutable bounded catalog snapshot.
    fn snapshot(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>>;
}

/// Deterministic metadata-only search over an injected MCP catalog.
pub struct McpSearchToolsTool {
    catalog: Arc<dyn McpToolCatalog>,
}

impl McpSearchToolsTool {
    /// Constructs the tool from one owned catalog implementation.
    #[must_use]
    pub fn new(catalog: impl McpToolCatalog) -> Self {
        Self {
            catalog: Arc::new(catalog),
        }
    }

    /// Constructs the tool over one explicitly shared catalog allocation.
    #[must_use]
    pub fn shared_catalog(catalog: Arc<dyn McpToolCatalog>) -> Self {
        Self { catalog }
    }
}

impl fmt::Debug for McpSearchToolsTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpSearchToolsTool")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SearchArguments {
    query: String,
    limit: usize,
}

impl SearchArguments {
    fn as_json(&self) -> Value {
        json!({"query": self.query, "limit": self.limit})
    }
}

impl Tool for McpSearchToolsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: tool_name(),
            description: DESCRIPTION.to_owned(),
            input_schema: input_schema(),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != tool_name() {
            return Err(invalid_arguments());
        }
        let arguments = decode_arguments(&call.arguments, false)?;
        let canonical = arguments.as_json();
        ensure_serialized_arguments(&canonical)?;
        Ok(PreparedToolCall::without_authority(canonical))
    }

    fn execute(
        &self,
        _context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            check_cancellation(&cancellation)?;
            let decoded = decode_arguments(&arguments, true)?;
            let canonical = decoded.as_json();
            ensure_serialized_arguments(&canonical)?;
            if canonical != arguments {
                return Err(invalid_arguments());
            }

            let snapshot = self
                .catalog
                .snapshot(cancellation.clone())
                .await
                .map_err(map_catalog_error)?;
            check_cancellation(&cancellation)?;
            search_snapshot(&snapshot, &decoded, &cancellation)
        })
    }
}

fn tool_name() -> ToolName {
    ToolName::new(MCP_SEARCH_TOOLS_TOOL_NAME).expect("mcp_search_tools is a valid tool name")
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "maxLength": MAX_MCP_SEARCH_QUERY_BYTES,
                "description": "Keyword query over configured dynamic tool metadata"
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": MCP_SEARCH_TOOLS_MAX_LIMIT,
                "description": "Optional maximum results; defaults to 8"
            }
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

fn decode_arguments(arguments: &Value, require_limit: bool) -> Result<SearchArguments, ToolError> {
    let Value::Object(object) = arguments else {
        return Err(invalid_arguments());
    };
    if object.len() > 2 || object.keys().any(|key| key != "query" && key != "limit") {
        return Err(invalid_arguments());
    }
    let Some(Value::String(query)) = object.get("query") else {
        return Err(invalid_arguments());
    };
    validate_query(query)?;
    let limit = match object.get("limit") {
        None if require_limit => return Err(invalid_arguments()),
        None => MCP_SEARCH_TOOLS_DEFAULT_LIMIT,
        Some(Value::Number(number)) => {
            let raw = number.as_u64().ok_or_else(invalid_arguments)?;
            if raw == 0 {
                return Err(invalid_arguments());
            }
            usize::try_from(raw)
                .unwrap_or(usize::MAX)
                .min(MCP_SEARCH_TOOLS_MAX_LIMIT)
        }
        Some(_) => return Err(invalid_arguments()),
    };
    Ok(SearchArguments {
        query: query.clone(),
        limit,
    })
}

fn validate_query(query: &str) -> Result<(), ToolError> {
    if query.len() > MAX_MCP_SEARCH_QUERY_BYTES {
        return Err(invalid_query());
    }
    let mut token_count = 0usize;
    let mut in_token = false;
    for byte in query.bytes() {
        if is_search_byte(byte) {
            if !in_token {
                token_count = token_count.checked_add(1).ok_or_else(resource_limit)?;
                if token_count > MAX_MCP_SEARCH_QUERY_TOKENS {
                    return Err(resource_limit());
                }
                in_token = true;
            }
        } else {
            in_token = false;
        }
    }
    Ok(())
}

fn validate_metadata_fields(
    name: &str,
    server: &str,
    description: &str,
    search_text: &str,
    tags: &[String],
) -> Result<(), McpToolCatalogBuildError> {
    if ToolName::validate(name).is_err()
        || server.is_empty()
        || server.len() > MAX_MCP_TOOL_SERVER_BYTES
        || server.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
        })
        || description.len() > MAX_MCP_TOOL_DESCRIPTION_BYTES
        || search_text.len() > MAX_MCP_TOOL_SEARCH_TEXT_BYTES
        || tags.len() > MAX_MCP_TOOL_TAGS
        || tags
            .iter()
            .any(|tag| tag.is_empty() || tag.len() > MAX_MCP_TOOL_TAG_BYTES)
    {
        return Err(McpToolCatalogBuildError::new(
            McpToolCatalogBuildErrorKind::InvalidMetadata,
        ));
    }
    Ok(())
}

fn metadata_retained_bytes(
    name: &str,
    server: &str,
    description: &str,
    search_text: &str,
    tags: &[String],
) -> Result<usize, McpToolCatalogBuildError> {
    [
        name.len(),
        server.len(),
        description.len(),
        search_text.len(),
    ]
    .into_iter()
    .chain(tags.iter().map(String::len))
    .try_fold(0usize, usize::checked_add)
    .ok_or_else(|| McpToolCatalogBuildError::new(McpToolCatalogBuildErrorKind::ResourceLimit))
}

fn validate_snapshot(tools: &[McpToolMetadata]) -> Result<(), McpToolCatalogBuildError> {
    if tools.len() > MAX_MCP_TOOL_CATALOG_ENTRIES {
        return Err(McpToolCatalogBuildError::new(
            McpToolCatalogBuildErrorKind::ResourceLimit,
        ));
    }
    let mut names = BTreeSet::new();
    let mut bytes = 0usize;
    for tool in tools {
        if !names.insert(tool.name.as_str()) {
            return Err(McpToolCatalogBuildError::new(
                McpToolCatalogBuildErrorKind::InvalidMetadata,
            ));
        }
        bytes = bytes.checked_add(tool.retained_bytes).ok_or_else(|| {
            McpToolCatalogBuildError::new(McpToolCatalogBuildErrorKind::ResourceLimit)
        })?;
        if bytes > MAX_MCP_TOOL_CATALOG_BYTES {
            return Err(McpToolCatalogBuildError::new(
                McpToolCatalogBuildErrorKind::ResourceLimit,
            ));
        }
    }
    Ok(())
}

fn search_snapshot(
    snapshot: &McpToolCatalogSnapshot,
    arguments: &SearchArguments,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    if snapshot.state == McpToolCatalogState::Discovering {
        return bounded_output(json!({
            "tools": [],
            "count": 0,
            "state": "discovering",
            "retryable": true
        }));
    }

    let tokens = query_tokens(&arguments.query);
    let mut matches = Vec::new();
    let mut work = 0usize;
    for tool in &snapshot.tools {
        check_cancellation(cancellation)?;
        if metadata_matches(tool, &tokens, arguments.query.is_empty(), &mut work)? {
            matches.push(tool);
        }
    }
    let match_count = matches.len();
    matches.truncate(arguments.limit);
    let more_available = match_count > matches.len();
    let mut retained = matches.len();
    loop {
        let omitted_for_bytes = matches.len() - retained;
        let value = render_search_result(&matches[..retained], more_available, omitted_for_bytes);
        let output = ToolOutput::success(value);
        if serialized_value_fits(&output, MAX_MCP_SEARCH_SERIALIZED_RESULT_BYTES) {
            return Ok(output);
        }
        if retained == 0 {
            return Err(resource_limit());
        }
        retained -= 1;
    }
}

fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, byte) in query.bytes().enumerate() {
        if is_search_byte(byte) {
            start.get_or_insert(index);
        } else if let Some(token_start) = start.take() {
            let mut token = query[token_start..index].to_owned();
            token.make_ascii_lowercase();
            tokens.push(token);
        }
    }
    if let Some(token_start) = start {
        let mut token = query[token_start..].to_owned();
        token.make_ascii_lowercase();
        tokens.push(token);
    }
    tokens
}

fn is_search_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn metadata_matches(
    metadata: &McpToolMetadata,
    tokens: &[String],
    match_all: bool,
    work: &mut usize,
) -> Result<bool, ToolError> {
    if tokens.is_empty() {
        return Ok(match_all);
    }
    for token in tokens {
        *work = work
            .checked_add(metadata.search_haystack.len())
            .ok_or_else(resource_limit)?;
        if *work > MAX_MCP_SEARCH_MATCH_STEPS {
            return Err(resource_limit());
        }
        if memchr::memmem::find(metadata.search_haystack.as_bytes(), token.as_bytes()).is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn render_search_result(
    tools: &[&McpToolMetadata],
    more_available: bool,
    omitted_for_bytes: usize,
) -> Value {
    let projected = tools
        .iter()
        .map(|tool| {
            let description = utf8_prefix(&tool.description, MAX_MCP_SEARCH_DESCRIPTION_BYTES);
            json!({
                "name": tool.name,
                "server": tool.server,
                "description": description,
                "purpose": description,
                "usage": tool.tags,
            })
        })
        .collect::<Vec<_>>();
    let mut output = json!({
        "tools": projected,
        "count": tools.len(),
    });
    let object = output
        .as_object_mut()
        .expect("static search output is an object");
    if more_available {
        object.insert("more_available".to_owned(), Value::Bool(true));
    }
    if omitted_for_bytes > 0 {
        object.insert(
            "context_limit".to_owned(),
            json!({
                "name": "mcp_search_result_bytes",
                "action": "omitted",
                "omitted_count": omitted_for_bytes,
                "effective_bytes": MAX_MCP_SEARCH_SERIALIZED_RESULT_BYTES,
            }),
        );
    }
    output
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn bounded_output(content: Value) -> Result<ToolOutput, ToolError> {
    let output = ToolOutput::success(content);
    if serialized_value_fits(&output, MAX_MCP_SEARCH_SERIALIZED_RESULT_BYTES) {
        Ok(output)
    } else {
        Err(resource_limit())
    }
}

fn ensure_serialized_arguments(arguments: &Value) -> Result<(), ToolError> {
    if serialized_value_fits(arguments, MAX_MCP_SEARCH_SERIALIZED_ARGUMENT_BYTES) {
        Ok(())
    } else {
        Err(resource_limit())
    }
}

fn serialized_value_fits(value: &(impl Serialize + ?Sized), limit: usize) -> bool {
    let mut counter = JsonByteCounter { written: 0, limit };
    serde_json::to_writer(&mut counter, value).is_ok()
}

struct JsonByteCounter {
    written: usize,
    limit: usize,
}

impl io::Write for JsonByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(written) = self.written.checked_add(bytes.len()) else {
            return Err(io::Error::other("serialized JSON byte count overflowed"));
        };
        if written > self.limit {
            return Err(io::Error::other("serialized JSON exceeded its byte limit"));
        }
        self.written = written;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn map_catalog_error(error: McpToolCatalogError) -> ToolError {
    match error.kind() {
        McpToolCatalogErrorKind::Unavailable => unavailable(),
        McpToolCatalogErrorKind::ResourceLimit => resource_limit(),
        McpToolCatalogErrorKind::Cancelled => cancelled(),
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
        "mcp_search_tools_invalid_arguments",
        "mcp_search_tools arguments are invalid",
        false,
    )
}

fn invalid_query() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "mcp_search_tools_invalid_query",
        "mcp_search_tools query is invalid",
        false,
    )
}

fn resource_limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "mcp_search_tools_resource_limit",
        "mcp_search_tools resource limit exceeded",
        false,
    )
}

fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "mcp_search_tools_unavailable",
        "MCP tool catalog is unavailable",
        true,
    )
}

fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "mcp_search_tools_cancelled",
        "mcp_search_tools was cancelled",
        false,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_executor::block_on;
    use machine_god_core::{SessionId, SessionIncarnationId, ToolCallId, TurnId};

    use super::*;

    #[derive(Clone)]
    struct StaticCatalog {
        snapshots: Arc<AtomicUsize>,
        snapshot: McpToolCatalogSnapshot,
    }

    impl McpToolCatalog for StaticCatalog {
        fn snapshot(
            &self,
            _cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
            Box::pin(async move {
                self.snapshots.fetch_add(1, Ordering::Relaxed);
                Ok(self.snapshot.clone())
            })
        }
    }

    fn metadata(
        name: &str,
        server: &str,
        description: &str,
        private_search: &str,
    ) -> McpToolMetadata {
        McpToolMetadata::new(
            name,
            server,
            description,
            private_search,
            vec!["MCP".to_owned(), "read".to_owned(), "read".to_owned()],
        )
        .unwrap()
    }

    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("session").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
            turn_id: TurnId::new("turn").unwrap(),
            call_id: ToolCallId::new("call").unwrap(),
        }
    }

    #[test]
    fn metadata_normalizes_tags_and_debug_redacts_private_search_text() {
        let metadata = metadata(
            "mcp_docs_lookup",
            "docs",
            "Lookup documentation",
            "private-schema-sentinel",
        );
        assert_eq!(metadata.tags(), &["mcp", "read"]);
        assert!(!format!("{metadata:?}").contains("private-schema-sentinel"));
    }

    #[test]
    fn snapshot_rejects_duplicate_names_and_entry_overflow() {
        let duplicate = metadata("mcp_a_read", "a", "read", "schema");
        assert_eq!(
            McpToolCatalogSnapshot::new(vec![duplicate.clone(), duplicate])
                .unwrap_err()
                .kind(),
            McpToolCatalogBuildErrorKind::InvalidMetadata
        );
        let entries = (0..=MAX_MCP_TOOL_CATALOG_ENTRIES)
            .map(|index| metadata(&format!("mcp_a_{index}"), "a", "read", "schema"))
            .collect();
        assert_eq!(
            McpToolCatalogSnapshot::new(entries).unwrap_err().kind(),
            McpToolCatalogBuildErrorKind::ResourceLimit
        );
    }

    #[test]
    fn future_is_inert_and_search_is_deterministic_without_schema_leakage() {
        let snapshots = Arc::new(AtomicUsize::new(0));
        let snapshot = McpToolCatalogSnapshot::new(vec![
            metadata(
                "mcp_zeta_lookup",
                "zeta",
                "Lookup docs",
                "query secret_schema_marker",
            ),
            metadata(
                "mcp_alpha_search",
                "alpha",
                "Search docs",
                "query other_private_marker",
            ),
        ])
        .unwrap();
        let tool = McpSearchToolsTool::new(StaticCatalog {
            snapshots: Arc::clone(&snapshots),
            snapshot,
        });
        let call = ToolCall {
            id: ToolCallId::new("call").unwrap(),
            name: tool_name(),
            arguments: json!({"query": "docs query"}),
        };
        let prepared = tool.prepare(call).unwrap();
        assert!(prepared.capability().is_none());
        let future = tool.execute(
            context(),
            prepared.arguments().clone(),
            CancellationToken::new(),
        );
        assert_eq!(snapshots.load(Ordering::Relaxed), 0);
        let output = block_on(future).unwrap();
        assert_eq!(snapshots.load(Ordering::Relaxed), 1);
        assert_eq!(output.content["count"], 2);
        assert_eq!(output.content["tools"][0]["name"], "mcp_zeta_lookup");
        let encoded = serde_json::to_string(&output).unwrap();
        assert!(!encoded.contains("secret_schema_marker"));
        assert!(!encoded.contains("other_private_marker"));
        assert!(!encoded.contains("inputSchema"));
    }

    #[test]
    fn discovering_snapshot_has_fixed_retryable_shape() {
        let snapshots = Arc::new(AtomicUsize::new(0));
        let tool = McpSearchToolsTool::new(StaticCatalog {
            snapshots,
            snapshot: McpToolCatalogSnapshot::discovering(),
        });
        let output = block_on(tool.execute(
            context(),
            json!({"query": "docs", "limit": 8}),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(
            output.content,
            json!({
                "tools": [],
                "count": 0,
                "state": "discovering",
                "retryable": true,
            })
        );
    }

    #[test]
    fn preparation_is_strict_and_canonicalizes_default_and_capped_limits() {
        let tool = McpSearchToolsTool::new(StaticCatalog {
            snapshots: Arc::new(AtomicUsize::new(0)),
            snapshot: McpToolCatalogSnapshot::new(Vec::new()).unwrap(),
        });
        let prepare = |arguments| {
            tool.prepare(ToolCall {
                id: ToolCallId::new("call").unwrap(),
                name: tool_name(),
                arguments,
            })
        };

        let defaulted = prepare(json!({"query": "docs"})).unwrap();
        assert_eq!(
            defaulted.arguments(),
            &json!({"query": "docs", "limit": MCP_SEARCH_TOOLS_DEFAULT_LIMIT})
        );
        let capped = prepare(json!({"query": "docs", "limit": 999})).unwrap();
        assert_eq!(
            capped.arguments(),
            &json!({"query": "docs", "limit": MCP_SEARCH_TOOLS_MAX_LIMIT})
        );
        for invalid in [
            json!(null),
            json!({}),
            json!({"query": 1}),
            json!({"query": "docs", "limit": 0}),
            json!({"query": "docs", "extra": true}),
        ] {
            assert_eq!(
                prepare(invalid).unwrap_err().code,
                "mcp_search_tools_invalid_arguments"
            );
        }
        let too_many_tokens = (0..=MAX_MCP_SEARCH_QUERY_TOKENS)
            .map(|_| "x")
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            prepare(json!({"query": too_many_tokens})).unwrap_err().code,
            "mcp_search_tools_resource_limit"
        );
    }

    #[test]
    fn matching_is_ascii_case_insensitive_and_all_tokens_are_required() {
        let snapshots = Arc::new(AtomicUsize::new(0));
        let snapshot = McpToolCatalogSnapshot::new(vec![
            metadata(
                "mcp_docs_lookup",
                "docs",
                "Find references",
                "PrivateSchemaQuery",
            ),
            metadata("mcp_other_lookup", "other", "Find docs", "unrelated"),
        ])
        .unwrap();
        let tool = McpSearchToolsTool::new(StaticCatalog {
            snapshots,
            snapshot,
        });

        let matching = block_on(tool.execute(
            context(),
            json!({"query": "DOCS privateSchema", "limit": 8}),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(matching.content["count"], 1);
        assert_eq!(matching.content["tools"][0]["name"], "mcp_docs_lookup");
        assert!(
            !serde_json::to_string(&matching)
                .unwrap()
                .contains("PrivateSchemaQuery")
        );

        let punctuation = block_on(tool.execute(
            context(),
            json!({"query": "!!!", "limit": 8}),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(punctuation.content["count"], 0);
    }

    #[test]
    fn result_byte_limit_omits_whole_entries_and_remains_valid_json() {
        let snapshots = Arc::new(AtomicUsize::new(0));
        let tools = (0..MCP_SEARCH_TOOLS_MAX_LIMIT)
            .map(|index| {
                metadata(
                    &format!("mcp_server_tool_{index}"),
                    "server",
                    &"description".repeat(700),
                    "search",
                )
            })
            .collect();
        let tool = McpSearchToolsTool::new(StaticCatalog {
            snapshots,
            snapshot: McpToolCatalogSnapshot::new(tools).unwrap(),
        });
        let output = block_on(tool.execute(
            context(),
            json!({"query": "", "limit": MCP_SEARCH_TOOLS_MAX_LIMIT}),
            CancellationToken::new(),
        ))
        .unwrap();
        let serialized = serde_json::to_vec(&output).unwrap();
        assert!(serialized.len() <= MAX_MCP_SEARCH_SERIALIZED_RESULT_BYTES);
        assert!(output.content.get("context_limit").is_some());
        assert!(output.content["count"].as_u64().unwrap() < 20);
    }

    #[test]
    fn cancellation_before_poll_prevents_catalog_acquisition() {
        let snapshots = Arc::new(AtomicUsize::new(0));
        let tool = McpSearchToolsTool::new(StaticCatalog {
            snapshots: Arc::clone(&snapshots),
            snapshot: McpToolCatalogSnapshot::new(Vec::new()).unwrap(),
        });
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error =
            block_on(tool.execute(context(), json!({"query": "", "limit": 8}), cancellation))
                .unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::Cancelled);
        assert_eq!(snapshots.load(Ordering::Relaxed), 0);
    }
}
