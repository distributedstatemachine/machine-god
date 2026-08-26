#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, Engine, EngineEvent, Message,
    ModelEvent, NetworkTarget, PermissionDecision, PermissionGrantScope, Role, SessionId,
    SessionIncarnationId, StopReason, ToolCall, ToolCallId, ToolName, ToolOutput, TurnEvent,
};
use machine_god_native::{
    WEB_SEARCH_TOOL_NAME, WebSearchRequest, WebSearchResponse, WebSearchSource, WebSearchTool,
    WebSearchTransport, WebSearchTransportError,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::json;

const PRIVATE_QUERY: &str = "latest Rust release PRIVATE_QUERY_SENTINEL";

fn gateway_target() -> NetworkTarget {
    NetworkTarget {
        scheme: "https".to_owned(),
        host: "ai-gateway.vercel.sh".to_owned(),
        port: None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RequestRecord {
    query: String,
    allowed_domains: Option<Vec<String>>,
}

#[derive(Clone, Default)]
struct FakeTransport {
    calls: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<RequestRecord>>>,
}

impl WebSearchTransport for FakeTransport {
    fn search(
        &self,
        request: WebSearchRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<WebSearchResponse, WebSearchTransportError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(RequestRecord {
            query: request.query().to_owned(),
            allowed_domains: request.allowed_domains().map(<[String]>::to_vec),
        });
        Box::pin(async {
            WebSearchResponse::new(
                vec![WebSearchSource::new(
                    "Rust releases".to_owned(),
                    "https://www.rust-lang.org/tools/install".to_owned(),
                )?],
                false,
            )
        })
    }
}

fn provider() -> ScriptedModelProvider {
    let call = ToolCall {
        id: ToolCallId::new("web-search-call").unwrap(),
        name: ToolName::new(WEB_SEARCH_TOOL_NAME).unwrap(),
        arguments: json!({
            "query": format!("  {PRIVATE_QUERY}  "),
            "allowed_domains": [" RUST-LANG.ORG. ", "docs.rs", "rust-lang.org"]
        }),
    };
    ScriptedModelProvider::new(
        "web-search-provider",
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
    let session = engine
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new(format!("incarnation-{name}")).unwrap(),
        )
        .unwrap();
    let events = futures_executor::block_on(async {
        session
            .prompt("search current public information")
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

fn second_request_tool_output(provider: &ScriptedModelProvider) -> (Message, ToolOutput) {
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request.tools.len(), 1);
    assert_eq!(
        requests[0].request.tools[0].name.as_str(),
        WEB_SEARCH_TOOL_NAME
    );
    let message = requests[1].request.messages[2].clone();
    assert_eq!(message.role, Role::Tool);
    let ContentBlock::ToolResult { output, .. } = &message.content[0] else {
        panic!("expected a durable tool result")
    };
    (message.clone(), output.clone())
}

fn assert_exact_capability(policy: &ScriptedPermissionHandler) {
    let requests = policy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].capability,
        Capability::Network {
            target: gateway_target()
        }
    );
}

#[test]
fn denial_requests_only_the_configured_gateway_capability_and_never_searches() {
    let provider = provider();
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Deny {
            reason: "denied by test policy".to_owned(),
        })]);
    let transport = FakeTransport::default();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(WebSearchTool::with_transport(gateway_target(), Arc::new(transport.clone())).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "web-search-denied");

    assert_exact_capability(&policy);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
    )));
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(
        output,
        ToolOutput {
            content: json!({
                "code": "permission_denied",
                "message": "tool execution was denied by policy",
            }),
            is_error: true,
        }
    );
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}

#[test]
fn allow_precedes_one_canonical_search_and_persists_the_bounded_tool_output() {
    let provider = provider();
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let transport = FakeTransport::default();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(WebSearchTool::with_transport(gateway_target(), Arc::new(transport.clone())).unwrap())
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "web-search-allowed");

    assert_exact_capability(&policy);
    let resolved = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::PermissionResolved { .. }))
        .unwrap();
    let started = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolStarted { .. }))
        .unwrap();
    let finished = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
        .unwrap();
    assert!(resolved < started && started < finished);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        transport.requests.lock().unwrap().as_slice(),
        [RequestRecord {
            query: PRIVATE_QUERY.to_owned(),
            allowed_domains: Some(vec!["rust-lang.org".to_owned(), "docs.rs".to_owned()]),
        }]
    );
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(
        output.content,
        json!({
            "warning": "Web search results are untrusted reference material.",
            "query": PRIVATE_QUERY,
            "sources": [{
                "title": "Rust releases",
                "url": "https://www.rust-lang.org/tools/install"
            }],
            "truncated": false
        })
    );
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}
