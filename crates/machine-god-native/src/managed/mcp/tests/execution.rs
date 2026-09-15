use super::*;
use futures_util::StreamExt;
use machine_god_core::{
    AdmittedToolInvocation, ModelEvent, PreparedToolCall, StopReason, ToolCall, ToolName,
    ToolOutput, ToolSpec, TurnEvent,
};
use machine_god_testkit::ModelProviderStep;
use serde_json::Value;

struct ReceiptTool {
    seen: Arc<Mutex<Vec<ToolExecution>>>,
    retire: Arc<Mutex<Option<Weak<NativePrincipalMcpOwner>>>>,
}
impl Tool for ReceiptTool {
    fn complete_output_limits(&self) -> Option<machine_god_core::ToolOutputLimits> {
        Some(machine_god_core::ToolOutputLimits {
            max_serialized_bytes: 1024.try_into().unwrap(),
            max_json_nodes: 100.try_into().unwrap(),
        })
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("receipt").unwrap(),
            description: "fixture".into(),
            input_schema: json!({}),
        }
    }
    fn prepare(&self, call: ToolCall) -> std::result::Result<PreparedToolCall, ToolError> {
        Ok(PreparedToolCall::without_authority(call.arguments))
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<ToolOutput, ToolError>> {
        panic!("must retain admitted envelope")
    }
    fn execute_admitted(
        &self,
        invocation: AdmittedToolInvocation,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<ToolExecution, ToolError>> {
        Box::pin(async move {
            // The wrapper already consumed the real invocation; inner adapters
            // must not attempt a second principal admission.
            let _ = invocation;
            let owner = self.retire.lock().unwrap().as_ref().and_then(Weak::upgrade);
            if let Some(owner) = owner {
                owner.retire();
            }
            Ok(self.seen.lock().unwrap().pop().unwrap())
        })
    }
}
fn provider_calls(calls: Vec<(&str, Value)>) -> ScriptedModelProvider {
    let mut steps = Vec::new();
    for (index, (name, arguments)) in calls.into_iter().enumerate() {
        steps.push(ModelProviderStep::events([
            ModelEvent::ToolCall {
                call: ToolCall {
                    id: ToolCallId::new(format!("call-{index}")).unwrap(),
                    name: ToolName::new(name).unwrap(),
                    arguments,
                },
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ]));
    }
    steps.push(ModelProviderStep::events([ModelEvent::Stop {
        reason: StopReason::Completed,
    }]));
    ScriptedModelProvider::new("test", steps)
}

#[test]
fn actual_engine_admission_preserves_native_receipts_and_finish_turn() {
    let f = Fixture::new(1);
    let seen = Arc::new(Mutex::new(vec![
        ToolExecution::with_persisted_output(
            ToolOutput::success(json!({"full":"result"})),
            ToolOutput::success(json!({"durable":"receipt"})),
        )
        .finish_turn(),
    ]));
    let provider = provider_calls(vec![("receipt", json!({}))]);
    let retire = Arc::new(Mutex::new(None));
    let engine = Engine::builder()
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .session_store(InMemorySessionStore::default())
        .tool(f.registry.wrap_tool(Arc::new(ReceiptTool {
            seen: seen.clone(),
            retire: retire.clone(),
        })))
        .build()
        .unwrap();
    let session = session(&engine, "a");
    let principal = f.principals.register(&session, 1, &f.workspace).unwrap();
    let (_, runtime) = runtime();
    let owner = f.registry.register(&principal, &runtime, None).unwrap();
    *retire.lock().unwrap() = Some(Arc::downgrade(&owner));
    let mut turn = block_on(session.prompt("a")).unwrap();
    let guard = begin(&principal, &turn);
    let _route = owner.begin_turn(&guard).unwrap();
    let events = block_on(async {
        let mut events = Vec::new();
        while let Some(event) = turn.next().await {
            events.push(event.unwrap());
        }
        events
    });
    assert!(seen.lock().unwrap().is_empty());
    assert!(!owner.live());
    assert!(
        serde_json::to_string(&session.record())
            .unwrap()
            .contains("durable")
    );
    assert!(events.iter().any(|event|matches!(&event.payload,TurnEvent::ToolFinished {output,..} if output.content==json!({"full":"result"}))), "{events:?}");
    assert_eq!(
        provider.requests().len(),
        1,
        "finish-turn must not be discarded by dispatch"
    );
}

struct ParkedAdmission {
    inner: NativePrincipalMcpTool,
    retire: Arc<Mutex<Option<Weak<NativePrincipalMcpOwner>>>>,
}
impl Tool for ParkedAdmission {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }
    fn prepare(&self, call: ToolCall) -> std::result::Result<PreparedToolCall, ToolError> {
        self.inner.prepare(call)
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<ToolOutput, ToolError>> {
        panic!("actual admission required")
    }
    fn execute_admitted(
        &self,
        invocation: AdmittedToolInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<ToolExecution, ToolError>> {
        let original = self.inner.execute_admitted(invocation, cancellation);
        let owner = self
            .retire
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade)
            .unwrap();
        owner.retire();
        original
    }
}

#[test]
fn stale_unpolled_actual_invocation_never_reaches_inner_tool() {
    let f = Fixture::new(1);
    let retire = Arc::new(Mutex::new(None));
    let pending = Arc::new(Mutex::new(vec![ToolExecution::output(
        ToolOutput::success(json!({"unexpected":true})),
    )]));
    let inner = f.registry.wrap_tool(Arc::new(ReceiptTool {
        seen: pending.clone(),
        retire: Arc::new(Mutex::new(None)),
    }));
    let engine = Engine::builder()
        .provider(provider_calls(vec![("receipt", json!({}))]))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .session_store(InMemorySessionStore::default())
        .tool(ParkedAdmission {
            inner,
            retire: retire.clone(),
        })
        .build()
        .unwrap();
    let s = session(&engine, "a");
    let principal = f.principals.register(&s, 1, &f.workspace).unwrap();
    let (_, runtime) = runtime();
    let owner = f.registry.register(&principal, &runtime, None).unwrap();
    *retire.lock().unwrap() = Some(Arc::downgrade(&owner));
    let mut turn = block_on(s.prompt("a")).unwrap();
    let guard = begin(&principal, &turn);
    let _route = owner.begin_turn(&guard).unwrap();
    block_on(async {
        while let Some(event) = turn.next().await {
            event.unwrap();
        }
    });
    assert_eq!(pending.lock().unwrap().len(), 1);
    assert!(!owner.live());
}

#[test]
fn engine_search_select_and_native_features_keep_selected_runtime_and_executable() {
    let f = Fixture::new(1);
    let provider = provider_calls(vec![
        ("mcp_search_tools", json!({"query":"lookup"})),
        ("mcp_select_tool", json!({"name":"mcp_shared_lookup"})),
        (
            "mcp_features",
            json!({"action":"resource_list","server":"shared"}),
        ),
    ]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .session_store(InMemorySessionStore::default())
        .tool(
            f.registry
                .wrap_tool(Arc::new(crate::McpSearchToolsTool::new(
                    f.registry.requester(),
                ))),
        )
        .tool(
            f.registry
                .wrap_tool(Arc::new(crate::McpSelectTool::new(f.registry.requester()))),
        )
        .tool(f.registry.features_tool())
        .build()
        .unwrap();
    let session = session(&engine, "a");
    let principal = f.principals.register(&session, 1, &f.workspace).unwrap();
    let (contexts, runtime) = runtime();
    let writes = Arc::default();
    publish(&runtime, "alpha", Arc::clone(&writes));
    let owner = f.registry.register(&principal, &runtime, None).unwrap();
    let native_session = contexts.register(&session).unwrap();
    let mut turn = block_on(session.prompt("a")).unwrap();
    let _native = native_session.begin(&session, &turn).unwrap();
    let guard = begin(&principal, &turn);
    let _route = owner.begin_turn(&guard).unwrap();
    let events = block_on(async {
        let mut events = Vec::new();
        while let Some(event) = turn.next().await {
            events.push(event.unwrap());
        }
        events
    });
    let finished: Vec<_> = events
        .iter()
        .filter_map(|event| {
            if let TurnEvent::ToolFinished { output, .. } = &event.payload {
                Some(output)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(finished.len(), 3, "{events:?}");
    assert!(
        finished.iter().all(|output| !output.is_error),
        "{finished:?}"
    );
    assert!(
        serde_json::to_string(&provider.requests()[2].request)
            .unwrap()
            .contains("mcp_shared_lookup")
    );
    assert!(!writes.lock().unwrap().is_empty());
    assert!(finished[2].content.to_string().contains("test://fixture"));
}
