use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolAuthorization, SessionId, SessionIncarnationId, Tool,
    ToolCall, ToolCallId, ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    MAX_MCP_SEARCH_QUERY_BYTES, MAX_MCP_SEARCH_QUERY_TOKENS,
    MAX_MCP_SEARCH_SERIALIZED_RESULT_BYTES, MAX_MCP_TOOL_CATALOG_ENTRIES,
    MCP_SEARCH_TOOLS_DEFAULT_LIMIT, MCP_SEARCH_TOOLS_MAX_LIMIT, MCP_SEARCH_TOOLS_TOOL_NAME,
    McpSearchToolsTool, McpToolCatalog, McpToolCatalogError, McpToolCatalogErrorKind,
    McpToolCatalogSnapshot, McpToolMetadata,
};
use serde_json::{Value, json};

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("mcp_search_tools unexpectedly yielded"),
    }
}

#[derive(Clone)]
struct FakeCatalog {
    entries: Vec<McpToolMetadata>,
    snapshots: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
}

impl FakeCatalog {
    fn new(entries: Vec<McpToolMetadata>) -> Self {
        Self {
            entries,
            snapshots: Arc::new(AtomicUsize::new(0)),
            polls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn snapshot_count(&self) -> usize {
        self.snapshots.load(Ordering::SeqCst)
    }

    fn poll_count(&self) -> usize {
        self.polls.load(Ordering::SeqCst)
    }
}

impl McpToolCatalog for FakeCatalog {
    fn snapshot(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        Box::pin(async move {
            self.snapshots.fetch_add(1, Ordering::SeqCst);
            self.polls.fetch_add(1, Ordering::SeqCst);
            if cancellation.is_cancelled() {
                return Err(McpToolCatalogError::new(McpToolCatalogErrorKind::Cancelled));
            }
            Ok(McpToolCatalogSnapshot::new(self.entries.clone())
                .expect("the fake catalog contains valid bounded metadata"))
        })
    }
}

#[derive(Default)]
struct CancellationCatalog {
    polls: AtomicUsize,
}

impl McpToolCatalog for CancellationCatalog {
    fn snapshot(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        Box::pin(async move {
            self.polls.fetch_add(1, Ordering::SeqCst);
            cancellation.cancelled().await;
            Err(McpToolCatalogError::new(McpToolCatalogErrorKind::Cancelled))
        })
    }
}

fn metadata(
    name: &str,
    server: &str,
    description: &str,
    usage: &[&str],
    search_text: &str,
) -> McpToolMetadata {
    McpToolMetadata::new(
        name.to_owned(),
        server.to_owned(),
        description.to_owned(),
        search_text.to_owned(),
        usage.iter().map(|value| (*value).to_owned()).collect(),
    )
    .expect("fixture metadata is valid")
}

fn catalog_entries() -> Vec<McpToolMetadata> {
    vec![
        metadata(
            "mcp_github_create_issue",
            "github",
            "Create an issue in a repository",
            &["mcp", "github", "issue", "create"],
            "github create_issue repository issue inputSchema title PRIVATE_SCHEMA_TOKEN",
        ),
        metadata(
            "mcp_github_close_issue",
            "github",
            "Close one issue",
            &["mcp", "github", "issue", "close"],
            "github close_issue issue state inputSchema issue_number",
        ),
        metadata(
            "mcp_browser_open_page",
            "browser",
            "Open a web page",
            &["mcp", "browser", "page", "open"],
            "browser open_page navigate url inputSchema",
        ),
    ]
}

fn tool_with(entries: Vec<McpToolMetadata>) -> (McpSearchToolsTool, FakeCatalog) {
    let catalog = FakeCatalog::new(entries);
    let tool = McpSearchToolsTool::shared_catalog(Arc::new(catalog.clone()));
    (tool, catalog)
}

fn call(arguments: Value) -> ToolCall {
    named_call(MCP_SEARCH_TOOLS_TOOL_NAME, arguments)
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("mcp-search-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("mcp-search-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("mcp-search-incarnation").unwrap(),
        turn_id: TurnId::new("mcp-search-turn").unwrap(),
        call_id: ToolCallId::new("mcp-search-call").unwrap(),
    }
}

fn execute(
    tool: &McpSearchToolsTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_ready(tool.execute(context(), arguments, cancellation))
}

fn prepare(tool: &McpSearchToolsTool, arguments: Value) -> Value {
    tool.prepare(call(arguments))
        .expect("arguments are valid")
        .arguments()
        .clone()
}

fn search(tool: &McpSearchToolsTool, arguments: Value) -> ToolOutput {
    let prepared = prepare(tool, arguments);
    execute(tool, prepared, CancellationToken::new()).expect("search succeeds")
}

fn assert_tool_error(
    error: ToolError,
    kind: ToolErrorKind,
    code: &str,
    message: &str,
    retryable: bool,
) {
    assert_eq!(error.kind, kind);
    assert_eq!(error.code, code);
    assert_eq!(error.message, message);
    assert_eq!(error.retryable, retryable);
    assert_eq!(error.to_string(), format!("{code}: {message}"));
}

fn assert_invalid_arguments(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "mcp_search_tools_invalid_arguments",
        "mcp_search_tools arguments are invalid",
        false,
    );
}

fn assert_invalid_query(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "mcp_search_tools_invalid_query",
        "mcp_search_tools query is invalid",
        false,
    );
}

fn assert_invalid_limit(error: ToolError) {
    assert_invalid_arguments(error);
}

fn assert_resource_limit(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "mcp_search_tools_resource_limit",
        "mcp_search_tools resource limit exceeded",
        false,
    );
}

fn assert_cancelled(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::Cancelled,
        "mcp_search_tools_cancelled",
        "mcp_search_tools was cancelled",
        false,
    );
}

fn result_names(output: &ToolOutput) -> Vec<&str> {
    output.content["tools"]
        .as_array()
        .expect("tools is an array")
        .iter()
        .map(|entry| entry["name"].as_str().expect("name is a string"))
        .collect()
}

#[test]
fn public_contract_schema_and_no_authority_preflight_are_frozen() {
    assert_eq!(MCP_SEARCH_TOOLS_TOOL_NAME, "mcp_search_tools");
    assert_eq!(MCP_SEARCH_TOOLS_DEFAULT_LIMIT, 8);
    assert_eq!(MCP_SEARCH_TOOLS_MAX_LIMIT, 20);
    assert!(MAX_MCP_SEARCH_QUERY_BYTES >= 256);
    assert!(MAX_MCP_SEARCH_QUERY_TOKENS >= 2);
    assert!(MAX_MCP_TOOL_CATALOG_ENTRIES >= MCP_SEARCH_TOOLS_MAX_LIMIT);
    assert!(MAX_MCP_SEARCH_SERIALIZED_RESULT_BYTES >= 4 * 1024);

    let (tool, catalog) = tool_with(catalog_entries());
    let spec = tool.spec();
    assert_eq!(spec.name.as_str(), MCP_SEARCH_TOOLS_TOOL_NAME);
    assert!(spec.description.contains("configured MCP"));
    assert!(spec.description.contains("metadata"));
    assert_eq!(spec.input_schema["type"], "object");
    assert_eq!(spec.input_schema["required"], json!(["query"]));
    assert_eq!(spec.input_schema["additionalProperties"], false);
    assert_eq!(spec.input_schema["properties"]["query"]["type"], "string");
    assert_eq!(
        spec.input_schema["properties"]["query"]["maxLength"],
        MAX_MCP_SEARCH_QUERY_BYTES
    );
    assert_eq!(spec.input_schema["properties"]["limit"]["type"], "integer");
    assert_eq!(spec.input_schema["properties"]["limit"]["minimum"], 1);
    assert_eq!(
        spec.input_schema["properties"]["limit"]["maximum"],
        MCP_SEARCH_TOOLS_MAX_LIMIT
    );

    let arguments = json!({"query": "github issue", "limit": 3});
    let prepared = tool.prepare(call(arguments.clone())).unwrap();
    assert_eq!(prepared.arguments(), &arguments);
    assert_eq!(
        prepared.authorization(),
        &PreparedToolAuthorization::NoAuthorityRequired
    );
    assert_eq!(catalog.snapshot_count(), 0);
    assert_eq!(catalog.poll_count(), 0);

    assert_invalid_arguments(
        tool.prepare(named_call("mcp_select_tool", arguments))
            .unwrap_err(),
    );
    assert_eq!(catalog.snapshot_count(), 0);
}

#[test]
fn strict_arguments_query_and_limit_bounds_fail_before_catalog_access() {
    let (tool, catalog) = tool_with(catalog_entries());
    let malformed = [
        Value::Null,
        json!([]),
        json!({}),
        json!({"query": null}),
        json!({"query": "github", "extra": true}),
        json!({"query": "github", "limit": null}),
        json!({"query": "github", "limit": 1.5}),
        json!({"query": "github", "limit": "1"}),
    ];
    for arguments in malformed {
        assert_invalid_arguments(tool.prepare(call(arguments)).unwrap_err());
    }

    assert_invalid_query(
        tool.prepare(call(json!({
            "query": "x".repeat(MAX_MCP_SEARCH_QUERY_BYTES + 1)
        })))
        .unwrap_err(),
    );
    assert_resource_limit(
        tool.prepare(call(json!({
            "query": std::iter::repeat_n("x", MAX_MCP_SEARCH_QUERY_TOKENS + 1)
                .collect::<Vec<_>>()
                .join(" ")
        })))
        .unwrap_err(),
    );

    for limit in [0] {
        assert_invalid_limit(
            tool.prepare(call(json!({"query": "github", "limit": limit})))
                .unwrap_err(),
        );
    }

    let exact_bytes = "x".repeat(MAX_MCP_SEARCH_QUERY_BYTES);
    assert!(tool.prepare(call(json!({"query": exact_bytes}))).is_ok());
    let exact_tokens = std::iter::repeat_n("x", MAX_MCP_SEARCH_QUERY_TOKENS)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(tool.prepare(call(json!({"query": exact_tokens}))).is_ok());
    assert!(tool.prepare(call(json!({"query": ""}))).is_ok());
    assert!(tool.prepare(call(json!({"query": " , / \t\r\n"}))).is_ok());
    assert!(
        tool.prepare(call(json!({
            "query": "github",
            "limit": MCP_SEARCH_TOOLS_MAX_LIMIT,
        })))
        .is_ok()
    );
    let capped = tool
        .prepare(call(json!({
            "query": "github",
            "limit": MCP_SEARCH_TOOLS_MAX_LIMIT + 1,
        })))
        .unwrap();
    assert_eq!(
        capped.arguments()["limit"],
        Value::from(MCP_SEARCH_TOOLS_MAX_LIMIT)
    );
    assert_eq!(catalog.snapshot_count(), 0);
    assert_eq!(catalog.poll_count(), 0);
}

#[test]
fn query_tokens_are_ascii_case_insensitive_conjunctive_and_schema_searchable() {
    let (tool, _) = tool_with(catalog_entries());

    let output = search(&tool, json!({"query": "GiTHub, ISSUE"}));
    assert_eq!(
        result_names(&output),
        ["mcp_github_close_issue", "mcp_github_create_issue"]
    );
    assert_eq!(output.content["count"], 2);
    assert!(output.content.get("more_available").is_none());

    let output = search(&tool, json!({"query": "github title"}));
    assert_eq!(result_names(&output), ["mcp_github_create_issue"]);

    let output = search(&tool, json!({"query": "github browser"}));
    assert!(result_names(&output).is_empty());
    assert_eq!(output.content["count"], 0);
}

#[test]
fn output_is_metadata_only_and_never_projects_search_or_schema_text() {
    let (tool, _) = tool_with(catalog_entries());
    let output = search(&tool, json!({"query": "PRIVATE_SCHEMA_TOKEN"}));
    assert_eq!(result_names(&output), ["mcp_github_create_issue"]);
    assert!(!output.is_error);

    let serialized = serde_json::to_string(&output).unwrap();
    assert!(!serialized.contains("PRIVATE_SCHEMA_TOKEN"));
    assert!(!serialized.contains("inputSchema"));
    assert!(!serialized.contains("issue_number"));
    let entry = &output.content["tools"][0];
    assert_eq!(
        entry.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["description", "name", "purpose", "server", "usage"]
    );
    assert_eq!(entry["name"], "mcp_github_create_issue");
    assert_eq!(entry["server"], "github");
    assert_eq!(entry["usage"], json!(["mcp", "github", "issue", "create"]));
}

#[test]
fn snapshot_order_limit_and_more_available_are_deterministic() {
    let entries = (0..=MCP_SEARCH_TOOLS_DEFAULT_LIMIT)
        .map(|index| {
            metadata(
                &format!("mcp_fixture_item_{index:02}"),
                "fixture",
                &format!("Fixture item {index}"),
                &["mcp", "fixture", "item"],
                &format!("fixture item {index}"),
            )
        })
        .collect();
    let (tool, catalog) = tool_with(entries);

    let output = search(&tool, json!({"query": "fixture"}));
    assert_eq!(output.content["count"], MCP_SEARCH_TOOLS_DEFAULT_LIMIT);
    assert_eq!(output.content["more_available"], true);
    assert_eq!(
        result_names(&output),
        (0..MCP_SEARCH_TOOLS_DEFAULT_LIMIT)
            .map(|index| format!("mcp_fixture_item_{index:02}"))
            .collect::<Vec<_>>()
    );

    let output = search(&tool, json!({"query": "fixture", "limit": 2}));
    assert_eq!(
        result_names(&output),
        ["mcp_fixture_item_00", "mcp_fixture_item_01"]
    );
    assert_eq!(output.content["count"], 2);
    assert_eq!(output.content["more_available"], true);
    assert_eq!(catalog.snapshot_count(), 2);
    assert_eq!(catalog.poll_count(), 2);
}

#[test]
fn catalog_snapshot_is_the_complete_policy_visibility_boundary() {
    let visible = metadata(
        "mcp_public_read",
        "public",
        "Read public metadata",
        &["mcp", "public", "read"],
        "public read metadata",
    );
    let hidden_name = "mcp_private_delete";
    let hidden_schema = "PRIVATE_POLICY_SCHEMA_SECRET";
    // A real catalog applies host policy before constructing its snapshot. The
    // tool receives only the visible view and must have no alternate catalog or
    // schema authority from which to rediscover excluded entries.
    let (tool, _) = tool_with(vec![visible]);

    let output = search(&tool, json!({"query": ""}));
    assert_eq!(result_names(&output), ["mcp_public_read"]);
    let serialized = serde_json::to_string(&output).unwrap();
    assert!(!serialized.contains(hidden_name));
    assert!(!serialized.contains(hidden_schema));

    let output = search(&tool, json!({"query": "private delete"}));
    assert!(result_names(&output).is_empty());
}

#[test]
fn execute_is_inert_before_poll_and_pre_cancelled_snapshot_fails_closed() {
    let (tool, catalog) = tool_with(catalog_entries());
    let arguments = prepare(&tool, json!({"query": "github"}));
    let execution = tool.execute(context(), arguments, CancellationToken::new());
    assert_eq!(catalog.snapshot_count(), 0);
    assert_eq!(catalog.poll_count(), 0);
    let output = poll_ready(execution).unwrap();
    assert_eq!(result_names(&output).len(), 2);
    assert_eq!(catalog.snapshot_count(), 1);
    assert_eq!(catalog.poll_count(), 1);

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let arguments = prepare(&tool, json!({"query": "github"}));
    assert_cancelled(execute(&tool, arguments, cancellation).unwrap_err());
    // Pre-cancellation is authoritative before catalog acquisition.
    assert_eq!(catalog.snapshot_count(), 1);
    assert_eq!(catalog.poll_count(), 1);
}

#[test]
fn cancellation_wakes_a_pending_catalog_snapshot_and_discovery_is_explicit() {
    let catalog = Arc::new(CancellationCatalog::default());
    let tool = McpSearchToolsTool::shared_catalog(catalog.clone());
    let cancellation = CancellationToken::new();
    let arguments = prepare(&tool, json!({"query": "github"}));
    let mut execution = tool.execute(context(), arguments, cancellation.clone());
    let wake_count = Arc::new(AtomicUsize::new(0));
    struct CountingWake(Arc<AtomicUsize>);
    impl Wake for CountingWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let waker = Waker::from(Arc::new(CountingWake(Arc::clone(&wake_count))));
    let mut context = Context::from_waker(&waker);
    assert!(matches!(
        execution.as_mut().poll(&mut context),
        Poll::Pending
    ));
    assert_eq!(catalog.polls.load(Ordering::SeqCst), 1);
    assert!(cancellation.cancel());
    assert!(wake_count.load(Ordering::SeqCst) > 0);
    let Poll::Ready(result) = execution.as_mut().poll(&mut context) else {
        panic!("catalog cancellation should finish the search")
    };
    assert_cancelled(result.unwrap_err());

    struct DiscoveringCatalog;
    impl McpToolCatalog for DiscoveringCatalog {
        fn snapshot(
            &self,
            _cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
            Box::pin(async { Ok(McpToolCatalogSnapshot::discovering()) })
        }
    }
    let tool = McpSearchToolsTool::new(DiscoveringCatalog);
    let output = search(&tool, json!({"query": "anything"}));
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
fn result_serialization_and_catalog_cardinality_remain_bounded() {
    let entries = (0..MAX_MCP_TOOL_CATALOG_ENTRIES)
        .map(|index| {
            metadata(
                &format!("mcp_fixture_tool_{index}"),
                "fixture",
                &"d".repeat(256),
                &["mcp", "fixture"],
                "fixture bounded tool",
            )
        })
        .collect();
    let (tool, _) = tool_with(entries);
    let output = search(
        &tool,
        json!({"query": "fixture", "limit": MCP_SEARCH_TOOLS_MAX_LIMIT}),
    );
    assert!(output.content["tools"].as_array().unwrap().len() <= MCP_SEARCH_TOOLS_MAX_LIMIT);
    assert!(output.content["more_available"].as_bool().unwrap());
    assert!(
        serde_json::to_vec(&output).unwrap().len() <= MAX_MCP_SEARCH_SERIALIZED_RESULT_BYTES,
        "complete ToolOutput serialization must stay inside the public ceiling"
    );

    let overflow = (0..=MAX_MCP_TOOL_CATALOG_ENTRIES)
        .map(|index| {
            metadata(
                &format!("mcp_overflow_tool_{index}"),
                "fixture",
                "description",
                &["mcp"],
                "overflow",
            )
        })
        .collect();
    assert!(McpToolCatalogSnapshot::new(overflow).is_err());
}
