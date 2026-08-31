use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Engine, EngineEvent, Message, ModelEvent, Role,
    SessionId, SessionIncarnationId, StopReason, ToolCall, ToolCallId, ToolName, ToolOutput,
    TurnEvent,
};
use machine_god_native::{
    MCP_SEARCH_TOOLS_DEFAULT_LIMIT, MCP_SEARCH_TOOLS_TOOL_NAME, McpSearchToolsTool, McpToolCatalog,
    McpToolCatalogError, McpToolCatalogSnapshot, McpToolMetadata,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
};
use serde_json::{Value, json};

struct ReadyCatalog {
    snapshots: AtomicUsize,
}

impl ReadyCatalog {
    const fn new() -> Self {
        Self {
            snapshots: AtomicUsize::new(0),
        }
    }

    fn snapshot_count(&self) -> usize {
        self.snapshots.load(Ordering::SeqCst)
    }
}

impl McpToolCatalog for ReadyCatalog {
    fn snapshot(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        Box::pin(async move {
            assert!(!cancellation.is_cancelled());
            self.snapshots.fetch_add(1, Ordering::SeqCst);
            Ok(McpToolCatalogSnapshot::new(vec![
                McpToolMetadata::new(
                    "mcp_github_create_issue",
                    "github",
                    "Create an issue in a repository",
                    "github create issue title ENGINE_PRIVATE_SCHEMA_TOKEN",
                    vec![
                        "mcp".to_owned(),
                        "github".to_owned(),
                        "issue".to_owned(),
                        "create".to_owned(),
                    ],
                )
                .unwrap(),
                McpToolMetadata::new(
                    "mcp_browser_open_page",
                    "browser",
                    "Open one browser page",
                    "browser open page url",
                    vec!["mcp".to_owned(), "browser".to_owned()],
                )
                .unwrap(),
            ])
            .unwrap())
        })
    }
}

fn provider(arguments: Value, name: &str) -> ScriptedModelProvider {
    let call = ToolCall {
        id: ToolCallId::new("mcp-search-call").unwrap(),
        name: ToolName::new(MCP_SEARCH_TOOLS_TOOL_NAME).unwrap(),
        arguments,
    };
    ScriptedModelProvider::new(
        name,
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall { call },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    )
}

fn collect(engine: &Engine, name: &str) -> (SessionId, Vec<EngineEvent>) {
    let session_id = SessionId::new(name).unwrap();
    let incarnation_id = SessionIncarnationId::new(format!("incarnation-{name}")).unwrap();
    let session = engine
        .create_session(session_id.clone(), incarnation_id)
        .unwrap();
    let events = futures_executor::block_on(async {
        session
            .prompt("discover the configured GitHub issue tool")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    });
    (session_id, events)
}

fn assert_completed(events: &[EngineEvent]) {
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));
}

fn second_request_tool_output(provider: &ScriptedModelProvider) -> (Message, ToolOutput) {
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request.tools.len(), 1);
    assert_eq!(
        requests[0].request.tools[0].name.as_str(),
        MCP_SEARCH_TOOLS_TOOL_NAME
    );
    let message = requests[1].request.messages[2].clone();
    assert_eq!(message.role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = message.content.as_slice() else {
        panic!("expected one durable tool result")
    };
    let output = output.clone();
    (message, output)
}

#[test]
fn engine_round_skips_permission_and_continues_with_metadata_only_result() {
    let catalog = Arc::new(ReadyCatalog::new());
    let provider = provider(json!({"query": "github title"}), "mcp-search-engine");
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(McpSearchToolsTool::shared_catalog(catalog.clone()))
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "mcp-search-engine");
    assert_completed(&events);
    assert!(policy.requests().is_empty());
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::PermissionRequested { .. } | TurnEvent::PermissionResolved { .. }
    )));
    let started = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolStarted { .. }))
        .unwrap();
    let finished = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
        .unwrap();
    assert!(started < finished);
    assert_eq!(catalog.snapshot_count(), 1);

    let expected = ToolOutput::success(json!({
        "tools": [{
            "name": "mcp_github_create_issue",
            "server": "github",
            "description": "Create an issue in a repository",
            "purpose": "Create an issue in a repository",
            "usage": ["mcp", "github", "issue", "create"],
        }],
        "count": 1,
    }));
    assert!(matches!(
        &events[finished].payload,
        TurnEvent::ToolFinished { output, .. } if output == &expected
    ));

    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(output, expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    let request = serde_json::to_string(&provider.requests()[1].request).unwrap();
    assert!(!request.contains("ENGINE_PRIVATE_SCHEMA_TOKEN"));
    assert!(!request.contains("inputSchema"));

    // Preparation canonicalizes the optional limit before the catalog is
    // acquired; the model-visible result remains independent of that internal
    // execution argument.
    assert_eq!(MCP_SEARCH_TOOLS_DEFAULT_LIMIT, 8);
}

#[test]
fn engine_invalid_arguments_never_reach_catalog_or_permission_policy() {
    let catalog = Arc::new(ReadyCatalog::new());
    let secret = "INVALID_ARGUMENT_SCHEMA_SECRET";
    let provider = provider(
        json!({"query": "github", "extra": secret}),
        "mcp-search-invalid",
    );
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(McpSearchToolsTool::shared_catalog(catalog.clone()))
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "mcp-search-invalid");
    assert_completed(&events);
    assert_eq!(catalog.snapshot_count(), 0);
    assert!(policy.requests().is_empty());
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::PermissionRequested { .. }
            | TurnEvent::PermissionResolved { .. }
            | TurnEvent::ToolStarted { .. }
            | TurnEvent::ToolFinished { .. }
    )));
    let (message, output) = second_request_tool_output(&provider);
    assert!(output.is_error);
    assert_eq!(output.content["code"], "tool_error");
    let rendered_error = serde_json::to_string(&output).unwrap();
    assert!(!rendered_error.contains(secret));
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}
