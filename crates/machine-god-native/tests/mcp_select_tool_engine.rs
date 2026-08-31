use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, Engine, Message, ModelEvent,
    PermissionDecision, PermissionGrantScope, Role, SessionId, SessionIncarnationId, StopReason,
    Tool, ToolCall, ToolCallId, ToolContext, ToolError, ToolName, ToolOutput, ToolSpec, TurnEvent,
};
use machine_god_native::{
    MCP_SELECT_TOOL_NAME, McpSelectTool, McpToolCatalog, McpToolCatalogError,
    McpToolCatalogSnapshot, McpToolMetadata,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::json;

struct ReadyCatalog {
    snapshots: AtomicUsize,
    executions: Arc<AtomicUsize>,
}

impl ReadyCatalog {
    fn new() -> Self {
        Self {
            snapshots: AtomicUsize::new(0),
            executions: Arc::new(AtomicUsize::new(0)),
        }
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
                    "PRIVATE_DESCRIPTION_SENTINEL",
                    "PRIVATE_EXECUTABLE_SCHEMA_SENTINEL",
                    vec!["PRIVATE_TAG_SENTINEL".to_owned()],
                )
                .unwrap()
                .with_tool(DynamicTool {
                    executions: Arc::clone(&self.executions),
                })
                .unwrap(),
            ])
            .unwrap())
        })
    }
}

struct DynamicTool {
    executions: Arc<AtomicUsize>,
}

impl Tool for DynamicTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("mcp_github_create_issue").unwrap(),
            description: "Create one GitHub issue".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {"title": {"type": "string"}},
                "required": ["title"],
            }),
        }
    }

    fn execute(
        &self,
        _context: ToolContext,
        arguments: serde_json::Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(ToolOutput::success(json!({"created": arguments["title"]}))) })
    }
}

fn provider() -> ScriptedModelProvider {
    let call = ToolCall {
        id: ToolCallId::new("mcp-select-call").unwrap(),
        name: ToolName::new(MCP_SELECT_TOOL_NAME).unwrap(),
        arguments: json!({"name": "mcp_github_create_issue"}),
    };
    ScriptedModelProvider::new(
        "mcp-select-engine",
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall { call },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("dynamic-call").unwrap(),
                        name: ToolName::new("mcp_github_create_issue").unwrap(),
                        arguments: json!({"title": "bounded issue"}),
                    },
                },
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

fn second_request_tool_output(provider: &ScriptedModelProvider) -> (Message, ToolOutput) {
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].request.tools.len(), 1);
    assert_eq!(
        requests[0].request.tools[0].name.as_str(),
        MCP_SELECT_TOOL_NAME
    );
    for request in &requests[1..] {
        assert_eq!(request.request.tools.len(), 2);
        assert_eq!(request.request.tools[0].name.as_str(), MCP_SELECT_TOOL_NAME);
        assert_eq!(
            request.request.tools[1].name.as_str(),
            "mcp_github_create_issue"
        );
    }
    let message = requests[1].request.messages[2].clone();
    assert_eq!(message.role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = message.content.as_slice() else {
        panic!("expected one durable tool result")
    };
    let output = output.clone();
    (message, output)
}

#[test]
fn engine_advertises_on_the_next_round_and_executes_the_selected_tool() {
    let catalog = Arc::new(ReadyCatalog::new());
    let provider = provider();
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(McpSelectTool::shared_catalog(catalog.clone()))
        .build()
        .unwrap();
    let session_id = SessionId::new("mcp-select-engine-session").unwrap();
    let session = engine
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new("mcp-select-engine-incarnation").unwrap(),
        )
        .unwrap();
    let events = futures_executor::block_on(async {
        session
            .prompt("select the admitted GitHub issue tool")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    });

    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));
    assert_eq!(policy.requests().len(), 1);
    assert!(matches!(
        &policy.requests()[0].capability,
        Capability::Tool { name, .. } if name.as_str() == "mcp_github_create_issue"
    ));
    assert_eq!(catalog.snapshots.load(Ordering::SeqCst), 1);
    assert_eq!(catalog.executions.load(Ordering::SeqCst), 1);

    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(
        output.content,
        "Selected dynamic MCP tool `mcp_github_create_issue`. Its executable schema will be available on the next model step; call `mcp_github_create_issue` with arguments matching the selected schema."
    );
    let request_json = serde_json::to_string(&provider.requests()[1].request).unwrap();
    let message_json = serde_json::to_string(&message).unwrap();
    for secret in ["PRIVATE_DESCRIPTION_SENTINEL", "PRIVATE_TAG_SENTINEL"] {
        assert!(!request_json.contains(secret));
        assert!(!message_json.contains(secret));
    }

    let record = store.record(&session_id).unwrap();
    assert_eq!(record.messages[2], message);
    assert_eq!(engine.tool_specs().len(), 1);
    assert_eq!(engine.tool_specs()[0].name.as_str(), MCP_SELECT_TOOL_NAME);
}
