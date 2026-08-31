use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolAuthorization, SessionId, SessionIncarnationId, Tool,
    ToolCall, ToolCallId, ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, ToolSpec,
    TurnId,
};
use machine_god_native::{
    MAX_MCP_SELECT_SERIALIZED_ARGUMENT_BYTES, MAX_MCP_SELECT_SERIALIZED_RESULT_BYTES,
    MAX_MCP_SELECTED_TOOL_SPEC_BYTES, MAX_MCP_TOOL_SCHEMA_NODES, MCP_SELECT_TOOL_NAME,
    McpSelectTool, McpToolCatalog, McpToolCatalogBuildErrorKind, McpToolCatalogError,
    McpToolCatalogErrorKind, McpToolCatalogSnapshot, McpToolMetadata,
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
        Poll::Pending => panic!("mcp_select_tool unexpectedly yielded"),
    }
}

#[derive(Clone)]
struct FakeCatalog {
    snapshot: McpToolCatalogSnapshot,
    snapshots: Arc<AtomicUsize>,
}

impl FakeCatalog {
    fn new(entries: Vec<McpToolMetadata>) -> Self {
        Self {
            snapshot: McpToolCatalogSnapshot::new(entries).unwrap(),
            snapshots: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn snapshot_count(&self) -> usize {
        self.snapshots.load(Ordering::SeqCst)
    }
}

impl McpToolCatalog for FakeCatalog {
    fn snapshot(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        Box::pin(async move {
            self.snapshots.fetch_add(1, Ordering::SeqCst);
            if cancellation.is_cancelled() {
                return Err(McpToolCatalogError::new(McpToolCatalogErrorKind::Cancelled));
            }
            Ok(self.snapshot.clone())
        })
    }
}

struct DiscoveringCatalog;

impl McpToolCatalog for DiscoveringCatalog {
    fn snapshot(
        &self,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        Box::pin(async { Ok(McpToolCatalogSnapshot::discovering()) })
    }
}

struct ErrorCatalog(McpToolCatalogErrorKind);

impl McpToolCatalog for ErrorCatalog {
    fn snapshot(
        &self,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        let kind = self.0;
        Box::pin(async move { Err(McpToolCatalogError::new(kind)) })
    }
}

#[derive(Clone, Default)]
struct PendingCatalog {
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

struct PendingSnapshot {
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl Future for PendingSnapshot {
    type Output = Result<McpToolCatalogSnapshot, McpToolCatalogError>;

    fn poll(self: std::pin::Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for PendingSnapshot {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl McpToolCatalog for PendingCatalog {
    fn snapshot(
        &self,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        Box::pin(PendingSnapshot {
            polls: Arc::clone(&self.polls),
            drops: Arc::clone(&self.drops),
        })
    }
}

struct SamePollCancellationCatalog(McpToolCatalogErrorKind);

impl McpToolCatalog for SamePollCancellationCatalog {
    fn snapshot(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        let kind = self.0;
        Box::pin(async move {
            cancellation.cancel();
            if kind == McpToolCatalogErrorKind::Unavailable {
                Err(McpToolCatalogError::new(kind))
            } else {
                Ok(McpToolCatalogSnapshot::new(Vec::new()).unwrap())
            }
        })
    }
}

struct CountingWake(Arc<AtomicUsize>);

impl Wake for CountingWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn metadata(name: &str, server: &str, private_search: &str) -> McpToolMetadata {
    McpToolMetadata::new(
        name,
        server,
        "PRIVATE_DESCRIPTION_SENTINEL",
        private_search,
        vec!["PRIVATE_TAG_SENTINEL".to_owned()],
    )
    .unwrap()
    .with_tool(DynamicTool {
        name: ToolName::new(name).unwrap(),
    })
    .unwrap()
}

struct DynamicTool {
    name: ToolName,
}

struct SpecTool(ToolSpec);

impl Tool for SpecTool {
    fn spec(&self) -> ToolSpec {
        self.0.clone()
    }

    fn execute(
        &self,
        _context: ToolContext,
        _arguments: Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async { Ok(ToolOutput::success(json!({"executed": true}))) })
    }
}

impl Tool for DynamicTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.clone(),
            description: "PRIVATE_EXECUTABLE_DESCRIPTION_SENTINEL".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {"value": {"type": "string"}},
            }),
        }
    }

    fn execute(
        &self,
        _context: ToolContext,
        _arguments: Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async { Ok(ToolOutput::success(json!({"executed": true}))) })
    }
}

fn tool_with(entries: Vec<McpToolMetadata>) -> (McpSelectTool, FakeCatalog) {
    let catalog = FakeCatalog::new(entries);
    let tool = McpSelectTool::shared_catalog(Arc::new(catalog.clone()));
    (tool, catalog)
}

fn call(arguments: Value) -> ToolCall {
    named_call(MCP_SELECT_TOOL_NAME, arguments)
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("mcp-select-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("mcp-select-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("mcp-select-incarnation").unwrap(),
        turn_id: TurnId::new("mcp-select-turn").unwrap(),
        call_id: ToolCallId::new("mcp-select-call").unwrap(),
    }
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}

#[test]
fn spec_and_preparation_are_strict_canonical_and_authority_free() {
    let (tool, catalog) = tool_with(Vec::new());
    let spec = tool.spec();
    assert_eq!(spec.name.as_str(), MCP_SELECT_TOOL_NAME);
    assert_eq!(spec.input_schema["required"], json!(["name"]));
    assert!(spec.input_schema.get("additionalProperties").is_none());
    assert!(spec.description.contains("next model step"));

    let prepared = tool
        .prepare(call(json!({"name": "mcp_github_create_issue"})))
        .unwrap();
    assert_eq!(
        prepared.authorization(),
        &PreparedToolAuthorization::NoAuthorityRequired
    );
    assert_eq!(
        prepared.arguments(),
        &json!({"name": "mcp_github_create_issue"})
    );
    assert!(
        serde_json::to_vec(prepared.arguments()).unwrap().len()
            <= MAX_MCP_SELECT_SERIALIZED_ARGUMENT_BYTES
    );
    assert_eq!(catalog.snapshot_count(), 0);

    for invalid in [
        Value::Null,
        json!({}),
        json!({"name": 7}),
        json!({"name": ""}),
        json!({"name": "not/a/tool"}),
    ] {
        let error = tool.prepare(call(invalid)).unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::InvalidInput);
        assert_eq!(error.code, "mcp_select_tool_invalid_arguments");
    }
    let prepared = tool
        .prepare(call(json!({"name": "mcp_valid", "extra": true})))
        .unwrap();
    assert_eq!(prepared.arguments(), &json!({"name": "mcp_valid"}));

    let direct_error = poll_ready(tool.execute(
        context(),
        json!({"name": "mcp_valid"}),
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(
        direct_error.code,
        "mcp_select_tool_turn_orchestration_required"
    );
    assert_eq!(catalog.snapshot_count(), 0);
    let error = tool
        .prepare(named_call(
            MCP_SEARCH_NAME_FOR_WRONG_CALL,
            json!({"name": "mcp_valid"}),
        ))
        .unwrap_err();
    assert_eq!(error.code, "mcp_select_tool_invalid_arguments");
    assert_eq!(catalog.snapshot_count(), 0);
}

const MCP_SEARCH_NAME_FOR_WRONG_CALL: &str = "mcp_search_tools";

#[test]
fn exact_ready_selection_returns_only_bounded_encoded_admitted_identity() {
    let (tool, catalog) = tool_with(vec![
        metadata(
            "mcp_github_create_issue",
            "github&<server",
            "PRIVATE_SCHEMA_SENTINEL",
        ),
        metadata("mcp_other_tool", "other", "OTHER_PRIVATE_SCHEMA"),
    ]);
    let execution = poll_ready(tool.execute_for_turn(
        context(),
        json!({"name": "mcp_github_create_issue"}),
        CancellationToken::new(),
    ))
    .unwrap();
    let output = execution.tool_output();
    assert!(execution.next_round_tool().is_some());
    assert_eq!(catalog.snapshot_count(), 1);
    assert!(!output.is_error);
    assert_eq!(
        output.content,
        "Selected dynamic MCP tool `mcp_github_create_issue`. Its executable schema will be available on the next model step; call `mcp_github_create_issue` with arguments matching the selected schema."
    );
    let serialized = serde_json::to_string(&output).unwrap();
    assert!(serialized.len() <= MAX_MCP_SELECT_SERIALIZED_RESULT_BYTES);
    for secret in [
        "PRIVATE_DESCRIPTION_SENTINEL",
        "PRIVATE_SCHEMA_SENTINEL",
        "PRIVATE_TAG_SENTINEL",
        "PRIVATE_EXECUTABLE_DESCRIPTION_SENTINEL",
    ] {
        assert!(!serialized.contains(secret));
    }
}

#[test]
fn lookup_is_exact_case_sensitive_and_uses_one_snapshot() {
    for requested in ["mcp_github_create", "MCP_GITHUB_CREATE_ISSUE"] {
        let (tool, catalog) = tool_with(vec![metadata(
            "mcp_github_create_issue",
            "github",
            "PRIVATE_SCHEMA_SENTINEL",
        )]);
        let error = poll_ready(tool.execute_for_turn(
            context(),
            json!({"name": requested}),
            CancellationToken::new(),
        ))
        .unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::InvalidInput);
        assert_eq!(error.code, "mcp_select_tool_not_found");
        assert!(!error.to_string().contains(requested));
        assert_eq!(catalog.snapshot_count(), 1);
    }
}

#[test]
fn discovering_snapshot_returns_bounded_retry_without_selection_claim() {
    let tool = McpSelectTool::new(DiscoveringCatalog);
    let execution = poll_ready(tool.execute_for_turn(
        context(),
        json!({"name": "mcp_github_create_issue"}),
        CancellationToken::new(),
    ))
    .unwrap();
    let output = execution.tool_output();
    assert!(execution.next_round_tool().is_none());
    assert_eq!(
        output.content,
        json!({
            "name": "mcp_github_create_issue",
            "selected": false,
            "state": "discovering",
            "retryable": true,
            "schema_advertised": false,
        })
    );
}

#[test]
fn catalog_failures_map_to_fixed_select_codes() {
    for (kind, code, retryable) in [
        (
            McpToolCatalogErrorKind::Unavailable,
            "mcp_select_tool_unavailable",
            true,
        ),
        (
            McpToolCatalogErrorKind::ResourceLimit,
            "mcp_select_tool_resource_limit",
            false,
        ),
        (
            McpToolCatalogErrorKind::Cancelled,
            "mcp_select_tool_cancelled",
            false,
        ),
    ] {
        let tool = McpSelectTool::new(ErrorCatalog(kind));
        let error = poll_ready(tool.execute_for_turn(
            context(),
            json!({"name": "mcp_private_name"}),
            CancellationToken::new(),
        ))
        .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.retryable, retryable);
        assert!(!format!("{error:?}").contains("mcp_private_name"));
    }
}

#[test]
fn future_is_inert_before_poll_and_precancellation_skips_catalog() {
    let (tool, catalog) = tool_with(Vec::new());
    let cancellation = CancellationToken::new();
    let future = tool.execute_for_turn(
        context(),
        json!({"name": "mcp_github_create_issue"}),
        cancellation.clone(),
    );
    assert_eq!(catalog.snapshot_count(), 0);
    drop(future);
    assert_eq!(catalog.snapshot_count(), 0);

    cancellation.cancel();
    let error = poll_ready(tool.execute_for_turn(
        context(),
        json!({"name": "mcp_github_create_issue"}),
        cancellation,
    ))
    .unwrap_err();
    assert_eq!(error.code, "mcp_select_tool_cancelled");
    assert_eq!(catalog.snapshot_count(), 0);
}

#[test]
fn cancellation_wakes_and_drops_a_non_cooperative_catalog_future() {
    let catalog = PendingCatalog::default();
    let tool = McpSelectTool::new(catalog.clone());
    let cancellation = CancellationToken::new();
    let mut future = Box::pin(tool.execute_for_turn(
        context(),
        json!({"name": "mcp_github_create_issue"}),
        cancellation.clone(),
    ));
    let wake_count = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(CountingWake(Arc::clone(&wake_count))));
    let mut poll_context = Context::from_waker(&waker);
    assert!(matches!(
        future.as_mut().poll(&mut poll_context),
        Poll::Pending
    ));
    assert_eq!(catalog.polls.load(Ordering::SeqCst), 1);
    cancellation.cancel();
    assert!(wake_count.load(Ordering::SeqCst) >= 1);
    let Poll::Ready(Err(error)) = future.as_mut().poll(&mut poll_context) else {
        panic!("cancelled selection did not complete")
    };
    assert_eq!(error.code, "mcp_select_tool_cancelled");
    drop(future);
    assert_eq!(catalog.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_wins_over_ready_and_error_in_the_same_poll() {
    for kind in [
        McpToolCatalogErrorKind::Cancelled,
        McpToolCatalogErrorKind::Unavailable,
    ] {
        let tool = McpSelectTool::new(SamePollCancellationCatalog(kind));
        let error = poll_ready(tool.execute_for_turn(
            context(),
            json!({"name": "mcp_github_create_issue"}),
            CancellationToken::new(),
        ))
        .unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::Cancelled);
        assert_eq!(error.code, "mcp_select_tool_cancelled");
    }
}

#[test]
fn debug_and_type_surfaces_do_not_project_catalog_data() {
    let tool = McpSelectTool::new(FakeCatalog::new(vec![metadata(
        "mcp_private_name",
        "PRIVATE_SERVER_SENTINEL",
        "PRIVATE_SCHEMA_SENTINEL",
    )]));
    let debug = format!("{tool:?}");
    for secret in [
        "mcp_private_name",
        "PRIVATE_SERVER_SENTINEL",
        "PRIVATE_SCHEMA_SENTINEL",
    ] {
        assert!(!debug.contains(secret));
    }

    assert_send_sync_static::<McpSelectTool>();
}

#[test]
fn executable_admission_rejects_mismatches_unbounded_schemas_and_metadata_only_selection() {
    let metadata_only = McpToolMetadata::new(
        "mcp_metadata_only",
        "server",
        "searchable only",
        "private schema text",
        Vec::new(),
    )
    .unwrap();
    let (tool, _) = tool_with(vec![metadata_only]);
    let error = poll_ready(tool.execute_for_turn(
        context(),
        json!({"name": "mcp_metadata_only"}),
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(error.code, "mcp_select_tool_not_found");

    let base =
        || McpToolMetadata::new("mcp_bounded", "server", "bounded", "schema", Vec::new()).unwrap();
    let mismatch = base()
        .with_tool(SpecTool(ToolSpec {
            name: ToolName::new("mcp_other").unwrap(),
            description: "mismatch".to_owned(),
            input_schema: json!({"type": "object"}),
        }))
        .unwrap_err();
    assert_eq!(
        mismatch.kind(),
        McpToolCatalogBuildErrorKind::InvalidMetadata
    );

    let scalar = base()
        .with_tool(SpecTool(ToolSpec {
            name: ToolName::new("mcp_bounded").unwrap(),
            description: "scalar".to_owned(),
            input_schema: json!(true),
        }))
        .unwrap_err();
    assert_eq!(scalar.kind(), McpToolCatalogBuildErrorKind::InvalidMetadata);

    let too_many_nodes = base()
        .with_tool(SpecTool(ToolSpec {
            name: ToolName::new("mcp_bounded").unwrap(),
            description: "nodes".to_owned(),
            input_schema: json!({
                "type": "object",
                "enum": vec![Value::Null; MAX_MCP_TOOL_SCHEMA_NODES],
            }),
        }))
        .unwrap_err();
    assert_eq!(
        too_many_nodes.kind(),
        McpToolCatalogBuildErrorKind::ResourceLimit
    );

    let oversized = base()
        .with_tool(SpecTool(ToolSpec {
            name: ToolName::new("mcp_bounded").unwrap(),
            description: "x".repeat(MAX_MCP_SELECTED_TOOL_SPEC_BYTES),
            input_schema: json!({"type": "object"}),
        }))
        .unwrap_err();
    assert_eq!(
        oversized.kind(),
        McpToolCatalogBuildErrorKind::ResourceLimit
    );
}

#[test]
fn executable_schema_node_admission_stops_at_the_exact_boundary() {
    let base =
        || McpToolMetadata::new("mcp_bounded", "server", "bounded", "schema", Vec::new()).unwrap();
    let schema_with_nodes = |nodes: usize| {
        assert!(nodes >= 3);
        json!({
            "type": "object",
            "enum": vec![Value::Null; nodes - 3],
        })
    };

    base()
        .with_tool(SpecTool(ToolSpec {
            name: ToolName::new("mcp_bounded").unwrap(),
            description: "exact node limit".to_owned(),
            input_schema: schema_with_nodes(MAX_MCP_TOOL_SCHEMA_NODES),
        }))
        .unwrap();

    let one_over = base()
        .with_tool(SpecTool(ToolSpec {
            name: ToolName::new("mcp_bounded").unwrap(),
            description: "one node over".to_owned(),
            input_schema: schema_with_nodes(MAX_MCP_TOOL_SCHEMA_NODES + 1),
        }))
        .unwrap_err();
    assert_eq!(one_over.kind(), McpToolCatalogBuildErrorKind::ResourceLimit);

    let wide = base()
        .with_tool(SpecTool(ToolSpec {
            name: ToolName::new("mcp_bounded").unwrap(),
            description: "wide rejected schema".to_owned(),
            input_schema: schema_with_nodes(MAX_MCP_TOOL_SCHEMA_NODES * 128),
        }))
        .unwrap_err();
    assert_eq!(wide.kind(), McpToolCatalogBuildErrorKind::ResourceLimit);
}
