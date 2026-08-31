//! Bounded deterministic search over an explicitly injected MCP tool catalog.

use std::collections::BTreeSet;
use std::fmt;
use std::future::{Future, poll_fn};
use std::io;
use std::sync::Arc;
use std::task::Poll;

use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolCall, Tool, ToolCall, ToolContext, ToolError,
    ToolErrorKind, ToolName, ToolOutput, ToolSpec, TurnToolRegistration,
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
/// Maximum serialized provider-visible specification retained for selection.
pub const MAX_MCP_SELECTED_TOOL_SPEC_BYTES: usize = 64 * 1024;
/// Maximum JSON container depth admitted in one executable MCP input schema.
pub const MAX_MCP_TOOL_SCHEMA_DEPTH: usize = 64;
/// Maximum JSON nodes admitted in one executable MCP input schema.
pub const MAX_MCP_TOOL_SCHEMA_NODES: usize = 4_096;
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
    name: Box<str>,
    server: Box<str>,
    description: Box<str>,
    tags: Box<[Box<str>]>,
    search_haystack: Box<str>,
    executable: Option<Arc<TurnToolRegistration>>,
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
                normalized_tags.push(tag.into_boxed_str());
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
            name: name.into_boxed_str(),
            server: server.into_boxed_str(),
            description: description.into_boxed_str(),
            tags: normalized_tags.into_boxed_slice(),
            search_haystack: search_haystack.into_boxed_str(),
            executable: None,
            retained_bytes,
        })
    }

    /// Attaches one executable implementation whose captured specification
    /// exactly matches this entry's admitted dynamic name.
    ///
    /// # Errors
    ///
    /// Returns a fixed rejection when the executable name differs, its input
    /// schema is not a bounded object schema, or its complete provider-visible
    /// specification exceeds the selected-schema budget.
    pub fn with_tool(self, tool: impl Tool) -> Result<Self, McpToolCatalogBuildError> {
        self.with_shared_tool(Arc::new(tool))
    }

    /// Attaches one explicitly shared executable implementation.
    ///
    /// # Errors
    ///
    /// Returns the same fixed rejections as [`Self::with_tool`].
    pub fn with_shared_tool(
        mut self,
        tool: Arc<dyn Tool>,
    ) -> Result<Self, McpToolCatalogBuildError> {
        if self.executable.is_some() {
            return Err(McpToolCatalogBuildError::new(
                McpToolCatalogBuildErrorKind::InvalidMetadata,
            ));
        }
        let registration = Arc::new(TurnToolRegistration::shared(tool));
        let spec = registration.spec();
        if spec.name.as_str() != self.name.as_ref() || !spec.input_schema.is_object() {
            return Err(McpToolCatalogBuildError::new(
                McpToolCatalogBuildErrorKind::InvalidMetadata,
            ));
        }
        validate_executable_schema(&spec.input_schema)?;
        let spec_bytes =
            serialized_value_size(spec, MAX_MCP_SELECTED_TOOL_SPEC_BYTES).ok_or_else(|| {
                McpToolCatalogBuildError::new(McpToolCatalogBuildErrorKind::ResourceLimit)
            })?;
        self.retained_bytes = self.retained_bytes.checked_add(spec_bytes).ok_or_else(|| {
            McpToolCatalogBuildError::new(McpToolCatalogBuildErrorKind::ResourceLimit)
        })?;
        self.executable = Some(registration);
        Ok(self)
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
    pub fn tags(&self) -> &[Box<str>] {
        &self.tags
    }

    pub(crate) fn executable(&self) -> Option<Arc<TurnToolRegistration>> {
        self.executable.as_ref().map(Arc::clone)
    }
}

impl fmt::Debug for McpToolMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpToolMetadata")
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
    tools: Arc<[McpToolMetadata]>,
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
            tools: tools.into(),
        })
    }

    /// Returns an empty snapshot whose discovery is still in progress.
    #[must_use]
    pub fn discovering() -> Self {
        Self {
            state: McpToolCatalogState::Discovering,
            tools: Arc::from([]),
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
            .finish_non_exhaustive()
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

            let snapshot = acquire_catalog_snapshot(
                self.catalog.as_ref(),
                &cancellation,
                map_catalog_error,
                cancelled,
            )
            .await?;
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
                "description": "Keyword query over configured dynamic tool metadata; at most 4096 UTF-8 bytes"
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "description": "Optional result count capped at 20; defaults to 8"
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
        || !server.is_ascii()
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
    tags: &[Box<str>],
) -> Result<usize, McpToolCatalogBuildError> {
    [
        name.len(),
        server.len(),
        description.len(),
        search_text.len(),
    ]
    .into_iter()
    .chain(tags.iter().map(|tag| tag.len()))
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
        if !names.insert(&*tool.name) {
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

fn validate_executable_schema(schema: &Value) -> Result<(), McpToolCatalogBuildError> {
    let mut stack = vec![(schema, 1usize)];
    let mut nodes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        nodes = nodes.checked_add(1).ok_or_else(|| {
            McpToolCatalogBuildError::new(McpToolCatalogBuildErrorKind::ResourceLimit)
        })?;
        if nodes > MAX_MCP_TOOL_SCHEMA_NODES {
            return Err(McpToolCatalogBuildError::new(
                McpToolCatalogBuildErrorKind::ResourceLimit,
            ));
        }
        match value {
            Value::Array(values) => {
                if depth > MAX_MCP_TOOL_SCHEMA_DEPTH {
                    return Err(McpToolCatalogBuildError::new(
                        McpToolCatalogBuildErrorKind::ResourceLimit,
                    ));
                }
                let child_depth = depth.checked_add(1).ok_or_else(|| {
                    McpToolCatalogBuildError::new(McpToolCatalogBuildErrorKind::ResourceLimit)
                })?;
                stack.extend(values.iter().map(|value| (value, child_depth)));
            }
            Value::Object(values) => {
                if depth > MAX_MCP_TOOL_SCHEMA_DEPTH {
                    return Err(McpToolCatalogBuildError::new(
                        McpToolCatalogBuildErrorKind::ResourceLimit,
                    ));
                }
                let child_depth = depth.checked_add(1).ok_or_else(|| {
                    McpToolCatalogBuildError::new(McpToolCatalogBuildErrorKind::ResourceLimit)
                })?;
                stack.extend(values.values().map(|value| (value, child_depth)));
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    Ok(())
}

pub(crate) async fn acquire_catalog_snapshot(
    catalog: &dyn McpToolCatalog,
    cancellation: &CancellationToken,
    map_error: fn(McpToolCatalogError) -> ToolError,
    cancelled_error: fn() -> ToolError,
) -> Result<McpToolCatalogSnapshot, ToolError> {
    let mut snapshot = catalog.snapshot(cancellation.clone());
    let mut cancellation_wait = Box::pin(cancellation.cancelled());
    let result = poll_fn(|poll_context| {
        if cancellation_wait.as_mut().poll(poll_context).is_ready() {
            return Poll::Ready(Err(cancelled_error()));
        }
        let snapshot_result = snapshot.as_mut().poll(poll_context);
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(cancelled_error()));
        }
        snapshot_result.map(|result| result.map_err(map_error))
    })
    .await;
    drop(cancellation_wait);
    drop(snapshot);
    result
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
    let match_target = arguments.limit + 1;
    let mut matches = Vec::with_capacity(match_target);
    let mut work = 0usize;
    for tool in snapshot.tools.iter() {
        check_cancellation(cancellation)?;
        if metadata_matches(tool, &tokens, arguments.query.is_empty(), &mut work)? {
            matches.push(tool);
            if matches.len() == match_target {
                break;
            }
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
            let name = encode_model_scalar(&tool.name);
            let server = encode_model_scalar(&tool.server);
            let description =
                bounded_encoded_scalar(&tool.description, MAX_MCP_SEARCH_DESCRIPTION_BYTES);
            let usage = tool
                .tags
                .iter()
                .map(|tag| encode_model_scalar(tag))
                .collect::<Vec<_>>();
            json!({
                "name": name,
                "server": server,
                "description": description,
                "purpose": description,
                "usage": usage,
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

fn bounded_encoded_scalar(value: &str, max_bytes: usize) -> String {
    let mut encoded = encode_model_scalar(value);
    if encoded.len() <= max_bytes {
        return encoded;
    }
    let prefix = utf8_prefix(&encoded, max_bytes);
    let mut prefix_bytes = prefix.len();
    if let Some(ampersand) = prefix.rfind('&')
        && !prefix[ampersand..].contains(';')
    {
        prefix_bytes = ampersand;
    }
    encoded.truncate(prefix_bytes);
    encoded
}

pub(crate) fn encode_model_scalar(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => encoded.push_str("&amp;"),
            '<' => encoded.push_str("&lt;"),
            '>' => encoded.push_str("&gt;"),
            '"' => encoded.push_str("&quot;"),
            '\u{0085}' => encoded.push_str("&#x85;"),
            '\u{2028}' => encoded.push_str("&#x2028;"),
            '\u{2029}' => encoded.push_str("&#x2029;"),
            '\0'..='\u{001f}' | '\u{007f}' => {
                use std::fmt::Write as _;
                write!(encoded, "&#x{:02x};", u32::from(character))
                    .expect("writing to a string is infallible");
            }
            _ => encoded.push(character),
        }
    }
    encoded
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
    serialized_value_size(value, limit).is_some()
}

fn serialized_value_size(value: &(impl Serialize + ?Sized), limit: usize) -> Option<usize> {
    let mut counter = JsonByteCounter { written: 0, limit };
    serde_json::to_writer(&mut counter, value).ok()?;
    Some(counter.written)
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
        assert_eq!(
            metadata
                .tags()
                .iter()
                .map(<Box<str> as AsRef<str>>::as_ref)
                .collect::<Vec<_>>(),
            ["mcp", "read"]
        );
        assert_eq!(format!("{metadata:?}"), "McpToolMetadata { .. }");
    }

    #[test]
    fn catalog_normalizes_excess_capacity_and_snapshot_clones_share_storage() {
        fn reserved(value: &str) -> String {
            let mut result = String::with_capacity(MAX_MCP_TOOL_SEARCH_TEXT_BYTES * 4);
            result.push_str(value);
            result
        }

        let metadata = McpToolMetadata::new(
            reserved("mcp_docs_lookup"),
            reserved("stdio/path alias"),
            reserved("Lookup documentation"),
            reserved("private schema"),
            vec![reserved("MCP"), reserved("docs")],
        )
        .unwrap();
        let mut tools = Vec::with_capacity(MAX_MCP_TOOL_CATALOG_ENTRIES * 2);
        tools.push(metadata);
        let snapshot = McpToolCatalogSnapshot::new(tools).unwrap();
        let clone = snapshot.clone();
        assert_eq!(snapshot.tools.len(), 1);
        assert_eq!(snapshot.tools.as_ptr(), clone.tools.as_ptr());
        assert_eq!(format!("{snapshot:?}"), "McpToolCatalogSnapshot { .. }");
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
