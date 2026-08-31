use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolAuthorization, SessionId, SessionIncarnationId, Tool,
    ToolCall, ToolCallId, ToolContext, ToolErrorKind, ToolName, TurnId,
};
use machine_god_native::{
    MAX_MCP_FEATURE_ARGUMENTS_BYTES, MAX_MCP_FEATURE_COMPLETION_VALUE_BYTES,
    MAX_MCP_FEATURE_CONTEXT_PAIRS, MAX_MCP_FEATURE_JSON_DEPTH, MAX_MCP_FEATURE_JSON_NODES,
    MAX_MCP_FEATURE_NAME_BYTES, MAX_MCP_FEATURE_SERIALIZED_ARGUMENT_BYTES,
    MAX_MCP_FEATURE_SERIALIZED_RESULT_BYTES, MAX_MCP_FEATURE_SERVER_BYTES,
    MAX_MCP_FEATURE_URI_BYTES, MCP_FEATURES_TOOL_NAME, McpFeatureAction, McpFeatureAuthority,
    McpFeatureError, McpFeatureErrorKind, McpFeaturePayload, McpFeatureRequest, McpFeaturesTool,
};
use serde_json::{Value, json};

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_once<F: Future>(future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    match poll_once(future.as_mut()) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("mcp_features unexpectedly yielded"),
    }
}

fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("mcp-features-call").unwrap(),
        name: ToolName::new(MCP_FEATURES_TOOL_NAME).unwrap(),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("mcp-features-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("mcp-features-incarnation").unwrap(),
        turn_id: TurnId::new("mcp-features-turn").unwrap(),
        call_id: ToolCallId::new("mcp-features-call").unwrap(),
    }
}

#[derive(Clone, Default)]
struct EchoAuthority {
    calls: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<McpFeatureRequest>>>,
}

impl EchoAuthority {
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<McpFeatureRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl McpFeatureAuthority for EchoAuthority {
    fn call(
        &self,
        request: McpFeatureRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpFeaturePayload, McpFeatureError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(request.clone());
        Box::pin(async move {
            let payload = match request.action() {
                McpFeatureAction::ResourceList => json!({
                    "items": [{"server": request.server(), "identity": "custom://a", "name": "A", "template": false}]
                }),
                McpFeatureAction::ResourceTemplates => json!({
                    "items": [{"server": request.server(), "identity": "custom:///{path}", "name": "A", "template": true}]
                }),
                McpFeatureAction::PromptList => json!({
                    "items": [{"server": request.server(), "identity": "review", "arguments": []}]
                }),
                McpFeatureAction::ResourceRead => json!({
                    "identity": request.identity().unwrap(), "contents": []
                }),
                McpFeatureAction::PromptGet => json!({
                    "identity": request.identity().unwrap(), "messages": []
                }),
                McpFeatureAction::PromptComplete | McpFeatureAction::ResourceComplete => json!({
                    "identity": request.identity().unwrap(),
                    "argument": request.argument().unwrap(),
                    "values": []
                }),
            };
            McpFeaturePayload::new(payload)
        })
    }
}

fn execute(
    tool: &McpFeaturesTool,
    requested: Value,
) -> Result<machine_god_core::ToolOutput, machine_god_core::ToolError> {
    let prepared = tool.prepare(call(requested))?;
    poll_ready(tool.execute(
        context(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
}

fn assert_error(
    result: Result<machine_god_core::ToolOutput, machine_god_core::ToolError>,
    kind: ToolErrorKind,
    code: &str,
) {
    let error = result.unwrap_err();
    assert_eq!(error.kind, kind);
    assert_eq!(error.code, code);
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}

#[test]
fn public_types_are_send_sync_and_debug_is_redacted() {
    assert_send_sync_static::<McpFeaturesTool>();
    assert_send_sync_static::<McpFeaturePayload>();
    assert_send_sync_static::<McpFeatureRequest>();
    assert_send_sync_static::<McpFeatureError>();

    let authority = EchoAuthority::default();
    let tool = McpFeaturesTool::new(authority.clone());
    execute(
        &tool,
        json!({"action":"prompt_get","server":"SECRET_SERVER","prompt":"SECRET_PROMPT","arguments":{"SECRET_KEY":"SECRET_VALUE"}}),
    )
    .unwrap();
    let rendered = format!("{:?}", &authority.requests()[0]);
    assert!(rendered.contains("PromptGet"));
    for secret in [
        "SECRET_SERVER",
        "SECRET_PROMPT",
        "SECRET_KEY",
        "SECRET_VALUE",
    ] {
        assert!(!rendered.contains(secret));
    }
}

#[test]
fn schema_and_preparation_freeze_the_pinned_boundary() {
    let authority = EchoAuthority::default();
    let tool = McpFeaturesTool::new(authority.clone());
    let spec = tool.spec();
    assert_eq!(spec.name.as_str(), MCP_FEATURES_TOOL_NAME);
    assert_eq!(spec.input_schema["required"], json!(["action", "server"]));
    assert_eq!(spec.input_schema["additionalProperties"], false);
    assert_eq!(
        spec.input_schema["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    assert!(spec.description.contains("untrusted external data"));

    let prepared = tool
        .prepare(call(json!({
            "action":"resource_read", "server":"fixture", "uri":"custom://item",
            "prompt":"ignored", "argument":19, "value":{"ignored":true}
        })))
        .unwrap();
    assert_eq!(
        prepared.authorization(),
        &PreparedToolAuthorization::NoAuthorityRequired
    );
    assert_eq!(
        prepared.arguments(),
        &json!({"action":"resource_read","server":"fixture","uri":"custom://item"})
    );
    assert!(
        serde_json::to_vec(prepared.arguments()).unwrap().len()
            <= MAX_MCP_FEATURE_SERIALIZED_ARGUMENT_BYTES
    );
    assert_eq!(authority.call_count(), 0);

    let wrong_name = ToolCall {
        name: ToolName::new("mcp_search_tools").unwrap(),
        ..call(json!({"action":"resource_list","server":"fixture"}))
    };
    assert_eq!(
        tool.prepare(wrong_name).unwrap_err().code,
        "mcp_features_invalid_arguments"
    );
    for invalid in [
        Value::Null,
        json!({}),
        json!({"action":"unknown","server":"fixture"}),
        json!({"action":"resource_read","server":"fixture"}),
        json!({"action":"resource_list","server":"fixture","unknown":true}),
        json!({"action":"resource_list","server":"fixture","arguments":{}}),
        json!({"action":"prompt_complete","server":"fixture","prompt":"p","argument":"a","arguments":{}}),
        json!({"action":"prompt_get","server":"fixture","prompt":"p","context":{}}),
    ] {
        assert_eq!(
            tool.prepare(call(invalid)).unwrap_err().code,
            "mcp_features_invalid_arguments"
        );
    }
}

#[test]
fn all_seven_actions_canonicalize_and_stamp_an_exact_envelope() {
    let authority = EchoAuthority::default();
    let tool = McpFeaturesTool::shared_authority(Arc::new(authority.clone()));
    let cases = [
        json!({"action":"resource_list","server":"fixture"}),
        json!({"action":"resource_templates","server":"fixture"}),
        json!({"action":"resource_read","server":"fixture","uri":"custom://item"}),
        json!({"action":"prompt_list","server":"fixture"}),
        json!({"action":"prompt_get","server":"fixture","prompt":"review","arguments":{"tone":"brief"}}),
        json!({"action":"prompt_complete","server":"fixture","prompt":"review","argument":"tone","value":"br","context":{"language":"en"}}),
        json!({"action":"resource_complete","server":"fixture","uri_template":"custom:///{path}","argument":"path","context":{"root":"src"}}),
    ];
    for requested in cases {
        let expected_action = requested["action"].clone();
        let output = execute(&tool, requested).unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content["trust"], "untrusted_external");
        assert_eq!(output.content["authority"], "none");
        assert_eq!(output.content["action"], expected_action);
        assert_eq!(output.content["server"], "fixture");
        assert!(
            serde_json::to_vec(&output).unwrap().len() <= MAX_MCP_FEATURE_SERIALIZED_RESULT_BYTES
        );
    }
    let requests = authority.requests();
    assert_eq!(requests.len(), 7);
    assert_eq!(
        requests[4]
            .arguments()
            .get("tone")
            .map(|value| value.as_ref()),
        Some("brief")
    );
    assert_eq!(
        requests[5]
            .context()
            .get("language")
            .map(|value| value.as_ref()),
        Some("en")
    );
    assert_eq!(requests[5].value(), "br");
}

#[test]
fn exact_input_bounds_accept_max_and_reject_max_plus_one() {
    let tool = McpFeaturesTool::new(EchoAuthority::default());
    assert!(
        tool.prepare(call(
            json!({"action":"resource_list","server":"s".repeat(MAX_MCP_FEATURE_SERVER_BYTES)})
        ))
        .is_ok()
    );
    assert_eq!(
        tool.prepare(call(
            json!({"action":"resource_list","server":"s".repeat(MAX_MCP_FEATURE_SERVER_BYTES + 1)}),
        ))
        .unwrap_err()
        .code,
        "mcp_features_invalid_arguments"
    );
    assert!(tool.prepare(call(json!({"action":"resource_read","server":"s","uri":"u".repeat(MAX_MCP_FEATURE_URI_BYTES)}))).is_err(), "complete canonical JSON is intentionally the tighter 64 KiB bound");
    assert!(tool.prepare(call(json!({"action":"prompt_get","server":"s","prompt":"p".repeat(MAX_MCP_FEATURE_NAME_BYTES)}))).is_ok());
    assert_eq!(tool.prepare(call(json!({"action":"prompt_get","server":"s","prompt":"p".repeat(MAX_MCP_FEATURE_NAME_BYTES + 1)}))).unwrap_err().code, "mcp_features_invalid_arguments");
    assert!(tool.prepare(call(json!({"action":"prompt_complete","server":"s","prompt":"p","argument":"a","value":"v".repeat(MAX_MCP_FEATURE_COMPLETION_VALUE_BYTES)}))).is_ok());
    assert_eq!(tool.prepare(call(json!({"action":"prompt_complete","server":"s","prompt":"p","argument":"a","value":"v".repeat(MAX_MCP_FEATURE_COMPLETION_VALUE_BYTES + 1)}))).unwrap_err().code, "mcp_features_invalid_arguments");

    let context = (0..MAX_MCP_FEATURE_CONTEXT_PAIRS)
        .map(|index| (format!("k{index:03}"), Value::String(String::new())))
        .collect();
    assert!(tool.prepare(call(json!({"action":"prompt_complete","server":"s","prompt":"p","argument":"a","context":Value::Object(context)}))).is_ok());
    let context = (0..=MAX_MCP_FEATURE_CONTEXT_PAIRS)
        .map(|index| (format!("k{index:03}"), Value::String(String::new())))
        .collect();
    assert_eq!(tool.prepare(call(json!({"action":"prompt_complete","server":"s","prompt":"p","argument":"a","context":Value::Object(context)}))).unwrap_err().code, "mcp_features_resource_limit");

    let overhead = serde_json::to_vec(&json!({})).unwrap().len();
    let nearly_max = "x".repeat(MAX_MCP_FEATURE_ARGUMENTS_BYTES - overhead - 8);
    assert!(
        tool.prepare(call(
            json!({"action":"prompt_get","server":"s","prompt":"p","arguments":{"a":nearly_max}})
        ))
        .is_ok()
    );
}

#[derive(Clone)]
struct FixedAuthority {
    payload: Arc<Mutex<Option<Result<McpFeaturePayload, McpFeatureError>>>>,
}

impl FixedAuthority {
    fn payload(value: Value) -> Self {
        Self {
            payload: Arc::new(Mutex::new(Some(Ok(McpFeaturePayload::new(value).unwrap())))),
        }
    }

    fn error(kind: McpFeatureErrorKind) -> Self {
        Self {
            payload: Arc::new(Mutex::new(Some(Err(McpFeatureError::new(kind))))),
        }
    }
}

impl McpFeatureAuthority for FixedAuthority {
    fn call(
        &self,
        _request: McpFeatureRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpFeaturePayload, McpFeatureError>> {
        let value = self.payload.lock().unwrap().take().expect("called once");
        Box::pin(async move { value })
    }
}

#[test]
fn payload_builder_and_publication_reject_envelope_and_request_mismatches() {
    for key in ["trust", "authority", "action", "server"] {
        let mut object = serde_json::Map::new();
        object.insert(key.to_owned(), Value::Null);
        assert_eq!(
            McpFeaturePayload::new(Value::Object(object))
                .unwrap_err()
                .kind(),
            McpFeatureErrorKind::ResourceLimit
        );
    }
    assert!(McpFeaturePayload::new(Value::Null).is_err());

    let requested = json!({"action":"resource_list","server":"fixture"});
    for invalid in [
        json!({"items":[],"unknown":true}),
        json!({"items":[{"server":"other","identity":"a","name":"A","template":false}]}),
        json!({"items":[{"server":"fixture","identity":"b","name":"B","template":false},{"server":"fixture","identity":"a","name":"A","template":false}]}),
        json!({"items":[{"server":"fixture","identity":"a","name":"A","template":true}]}),
    ] {
        assert_error(
            execute(
                &McpFeaturesTool::new(FixedAuthority::payload(invalid)),
                requested.clone(),
            ),
            ToolErrorKind::InvalidInput,
            "mcp_features_resource_limit",
        );
    }
    assert_error(
        execute(
            &McpFeaturesTool::new(FixedAuthority::payload(
                json!({"identity":"wrong","contents":[]}),
            )),
            json!({"action":"resource_read","server":"fixture","uri":"custom://right"}),
        ),
        ToolErrorKind::InvalidInput,
        "mcp_features_resource_limit",
    );
    assert_error(
        execute(
            &McpFeaturesTool::new(FixedAuthority::payload(
                json!({"identity":"review","argument":"wrong","values":[]}),
            )),
            json!({"action":"prompt_complete","server":"fixture","prompt":"review","argument":"tone"}),
        ),
        ToolErrorKind::InvalidInput,
        "mcp_features_resource_limit",
    );
}

#[test]
fn payload_depth_nodes_and_complete_output_are_bounded() {
    let mut depth = Value::Null;
    for _ in 0..MAX_MCP_FEATURE_JSON_DEPTH - 1 {
        depth = json!([depth]);
    }
    assert!(McpFeaturePayload::new(json!({"items":depth})).is_ok());
    let mut too_deep = Value::Null;
    for _ in 0..MAX_MCP_FEATURE_JSON_DEPTH {
        too_deep = json!([too_deep]);
    }
    assert!(McpFeaturePayload::new(json!({"items":too_deep})).is_err());
    assert!(
        McpFeaturePayload::new(json!({"items": vec![Value::Null; MAX_MCP_FEATURE_JSON_NODES - 2]}))
            .is_ok()
    );
    assert!(
        McpFeaturePayload::new(json!({"items": vec![Value::Null; MAX_MCP_FEATURE_JSON_NODES - 1]}))
            .is_err()
    );

    let oversized_for_envelope = "x".repeat(MAX_MCP_FEATURE_SERIALIZED_RESULT_BYTES - 80);
    let tool = McpFeaturesTool::new(FixedAuthority::payload(json!({
        "identity":"review", "messages":[{"role":"user","contentKind":"text","content":oversized_for_envelope}]
    })));
    assert_error(
        execute(
            &tool,
            json!({"action":"prompt_get","server":"fixture","prompt":"review"}),
        ),
        ToolErrorKind::InvalidInput,
        "mcp_features_resource_limit",
    );
}

#[derive(Clone, Default)]
struct PendingAuthority {
    calls: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

struct PendingOperation {
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl Future for PendingOperation {
    type Output = Result<McpFeaturePayload, McpFeatureError>;

    fn poll(self: std::pin::Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for PendingOperation {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl McpFeatureAuthority for PendingAuthority {
    fn call(
        &self,
        _request: McpFeatureRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpFeaturePayload, McpFeatureError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(PendingOperation {
            polls: Arc::clone(&self.polls),
            drops: Arc::clone(&self.drops),
        })
    }
}

#[test]
fn constructor_prepare_and_execution_future_are_inert_until_poll_and_drop_cleans_pending() {
    let authority = PendingAuthority::default();
    let tool = McpFeaturesTool::new(authority.clone());
    let prepared = tool
        .prepare(call(json!({"action":"resource_list","server":"fixture"})))
        .unwrap();
    assert_eq!(authority.calls.load(Ordering::SeqCst), 0);
    let future = tool.execute(
        context(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    );
    assert_eq!(authority.calls.load(Ordering::SeqCst), 0);
    let mut future = std::pin::pin!(future);
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(authority.calls.load(Ordering::SeqCst), 1);
    assert_eq!(authority.polls.load(Ordering::SeqCst), 1);
    drop(future);
    assert_eq!(authority.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn pending_authority_is_independently_woken_and_cancelled() {
    struct CountingWake(Arc<AtomicUsize>);
    impl Wake for CountingWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let authority = PendingAuthority::default();
    let tool = McpFeaturesTool::new(authority.clone());
    let cancellation = CancellationToken::new();
    let future = tool.execute(
        context(),
        json!({"action":"resource_list","server":"fixture"}),
        cancellation.clone(),
    );
    let mut future = std::pin::pin!(future);
    let wakes = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(CountingWake(Arc::clone(&wakes))));
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    cancellation.cancel();
    assert!(wakes.load(Ordering::SeqCst) > 0);
    let error = match future.as_mut().poll(&mut Context::from_waker(&waker)) {
        Poll::Ready(result) => result.unwrap_err(),
        Poll::Pending => panic!("cancelled mcp_features remained pending"),
    };
    assert_eq!(error.code, "mcp_features_cancelled");
    assert_eq!(authority.drops.load(Ordering::SeqCst), 1);
}

struct SamePollAuthority {
    cancellation: CancellationToken,
    result: McpFeatureErrorKind,
}

impl McpFeatureAuthority for SamePollAuthority {
    fn call(
        &self,
        request: McpFeatureRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpFeaturePayload, McpFeatureError>> {
        let cancellation = self.cancellation.clone();
        let result = self.result;
        Box::pin(async move {
            cancellation.cancel();
            if result == McpFeatureErrorKind::InputRequired {
                McpFeaturePayload::new(
                    json!({"identity":request.identity().unwrap(),"contents":[]}),
                )
            } else {
                Err(McpFeatureError::new(result))
            }
        })
    }
}

#[test]
fn precancellation_and_same_poll_cancellation_win_ready_and_error_results() {
    let pending = PendingAuthority::default();
    let tool = McpFeaturesTool::new(pending.clone());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = poll_ready(tool.execute(
        context(),
        json!({"action":"resource_list","server":"fixture"}),
        cancellation,
    ))
    .unwrap_err();
    assert_eq!(error.code, "mcp_features_cancelled");
    assert_eq!(pending.calls.load(Ordering::SeqCst), 0);

    for result in [
        McpFeatureErrorKind::InputRequired,
        McpFeatureErrorKind::Unavailable,
    ] {
        let cancellation = CancellationToken::new();
        let tool = McpFeaturesTool::new(SamePollAuthority {
            cancellation: cancellation.clone(),
            result,
        });
        let error = poll_ready(tool.execute(
            context(),
            json!({"action":"resource_read","server":"fixture","uri":"custom://item"}),
            cancellation,
        ))
        .unwrap_err();
        assert_eq!(error.code, "mcp_features_cancelled");
    }
}

#[test]
fn authority_errors_map_to_fixed_redacted_results() {
    let requested = json!({"action":"resource_list","server":"SECRET_SERVER"});
    for (kind, tool_kind, code, retryable) in [
        (
            McpFeatureErrorKind::Unavailable,
            ToolErrorKind::Unavailable,
            "mcp_features_unavailable",
            true,
        ),
        (
            McpFeatureErrorKind::ResourceLimit,
            ToolErrorKind::InvalidInput,
            "mcp_features_resource_limit",
            false,
        ),
        (
            McpFeatureErrorKind::Cancelled,
            ToolErrorKind::Cancelled,
            "mcp_features_cancelled",
            false,
        ),
    ] {
        let error = execute(
            &McpFeaturesTool::new(FixedAuthority::error(kind)),
            requested.clone(),
        )
        .unwrap_err();
        assert_eq!(
            (error.kind, error.code.as_str(), error.retryable),
            (tool_kind, code, retryable)
        );
        assert!(!format!("{error:?}").contains("SECRET_SERVER"));
    }
    let output = execute(
        &McpFeaturesTool::new(FixedAuthority::error(McpFeatureErrorKind::InputRequired)),
        requested,
    )
    .unwrap();
    assert!(output.is_error);
    assert_eq!(output.content, json!({"error":"McpInputRequired"}));
}
