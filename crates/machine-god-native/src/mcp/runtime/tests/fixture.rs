use super::*;
use crate::mcp::permission::{NativeMcpPermissionAuthority, NativeMcpPermissionPreparer};
use futures_util::StreamExt;
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};

pub(super) struct Clock(Instant);
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        self.0
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
impl NativePermissionReviewClock for Clock {
    fn now(&self) -> Instant {
        self.0
    }
    fn wait_until(&self, _: Instant) -> BoxFuture<'static, ()> {
        Box::pin(std::future::pending())
    }
}
#[derive(Default)]
pub(super) struct Executor {
    pub calls: AtomicUsize,
    pub inputs: AtomicUsize,
    pub skip: AtomicUsize,
    pub values: Mutex<Vec<Value>>,
    pub options: Mutex<Vec<crate::mcp::submission::McpToolCallOptions>>,
}
impl NativeMcpToolExecutor for Executor {
    fn persist_arguments<'a>(
        &'a self,
        _: ToolContext,
        _: &'a ToolName,
        _: &'a Value,
        _: CancellationToken,
    ) -> BoxFuture<'a, std::result::Result<Option<Value>, ToolError>> {
        Box::pin(async move {
            self.inputs.fetch_add(1, Ordering::SeqCst);
            Ok(Some(json!({"fixture_input_archive":"durable"})))
        })
    }
    fn execute(
        &self,
        mut call: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, std::result::Result<ToolExecution, ToolError>> {
        Box::pin(async move {
            let count = self.calls.fetch_add(1, Ordering::SeqCst);
            self.values.lock().unwrap().push(call.arguments().clone());
            self.options.lock().unwrap().push(call.options());
            if count < self.skip.load(Ordering::SeqCst) {
                return Ok(ToolExecution::output(ToolOutput::success(
                    json!({"explicit_fixture_drop":true}),
                )));
            }
            // Constructing/dropping an unpolled exchange leaves the one initial
            // request available; the actual polled exchange consumes it.
            drop(call.first_exchange());
            let response = call.first_exchange().await?;
            assert_eq!(response.request_id(), call.request_id());
            assert!(call.first_exchange().await.is_err());
            call.revalidate()?;
            Ok(ToolExecution::with_persisted_output(
                ToolOutput::success(machine_god_core::json::from_slice(response.bytes()).unwrap()),
                ToolOutput::success(json!({"fixture_archive":"durable"})),
            ))
        })
    }
}
pub(super) fn policy() -> NativeMcpToolExecutionPolicy {
    NativeMcpToolExecutionPolicy {
        progress: true,
        complete_input_limits: Some(ToolInputLimits {
            max_argument_bytes: 65536.try_into().unwrap(),
            max_argument_nodes: 4096.try_into().unwrap(),
            max_prepared_argument_bytes: (68 * 1024).try_into().unwrap(),
            max_prepared_argument_nodes: 4224.try_into().unwrap(),
        }),
        complete_output_limits: Some(ToolOutputLimits {
            max_serialized_bytes: 65536.try_into().unwrap(),
            max_json_nodes: 4096.try_into().unwrap(),
        }),
        ..Default::default()
    }
}
pub(super) fn standalone() -> NativeMcpRuntime {
    standalone_with_limits(NativeMcpRuntimeLimits::default())
}
pub(super) fn standalone_with_limits(limits: NativeMcpRuntimeLimits) -> NativeMcpRuntime {
    NativeMcpRuntime::new(
        Arc::new(NativeMcpContexts::new()),
        Arc::new(Clock(Instant::now())),
        Arc::new(Executor::default()),
        policy(),
        limits,
    )
    .unwrap()
}
struct Builtin;
impl NativePermissionActionPreparer for Builtin {
    fn prepare<'a>(
        &'a self,
        _: &'a PermissionRequest,
        _: PermissionInvocation<'a>,
        _: CancellationToken,
    ) -> BoxFuture<'a, std::result::Result<Box<dyn NativePreparedPermissionAction>, PermissionError>>
    {
        Box::pin(async {
            Err(PermissionError::new(
                "fixture_builtin",
                "Unexpected builtin request",
            ))
        })
    }
}
#[derive(Default)]
pub(super) struct Transport {
    pub reviews: AtomicUsize,
}
impl AiGatewayTransport for Transport {
    fn stream(
        &self,
        _: AiGatewayTransportRequest,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<AiGatewayByteStream, ProviderError>> {
        Box::pin(async move {
            self.reviews.fetch_add(1, Ordering::SeqCst);
            let answer = json!({"type":"tool-call","toolCallId":"assessment","toolName":"permission_decision","input":{"risk":"low","authorization":"unknown","decision":"allow","rationale":"Requested development task."}});
            let finish = json!({"type":"finish","finishReason":{"unified":"tool-calls"}});
            Ok(Box::pin(futures_util::stream::iter([Ok(format!(
                "data: {answer}\n\ndata: {finish}\n\n"
            )
            .into_bytes())])) as AiGatewayByteStream)
        })
    }
}
struct Prompt;
impl PermissionPrompter for Prompt {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, std::result::Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async { Ok(PermissionPromptDecision::AllowOnce) })
    }
}
pub(super) struct Fixture {
    pub prompt_principal: Option<crate::NativeInteractivePromptPrincipal>,
    pub conversation: NativeConversationRuntime,
    pub runtime: Arc<NativeMcpRuntime>,
    pub executor: Arc<Executor>,
    pub transport: Arc<Transport>,
    pub writes: Arc<Mutex<Vec<u8>>>,
    pub provider: ScriptedModelProvider,
    pub store: InMemorySessionStore,
    _engine: Engine,
}
impl Fixture {
    pub fn new(values: &[Value], mode: PermissionMode, skip: usize) -> Self {
        Self::configured(values, mode, skip, None, false, None, |runtime, writes| {
            candidate(runtime, "calendar", &["lookup"], writes)
        })
    }
    pub fn with_executor(
        values: &[Value],
        mode: PermissionMode,
        executor: Arc<dyn NativeMcpToolExecutor>,
        policy: NativeMcpToolExecutionPolicy,
        batch: bool,
        prepare: impl FnOnce(&NativeMcpRuntime, Arc<Mutex<Vec<u8>>>) -> NativeMcpRuntimeCandidate,
    ) -> Self {
        Self::configured(
            values,
            mode,
            0,
            Some((executor, policy)),
            batch,
            None,
            prepare,
        )
    }
    pub fn with_executor_and_clock(
        values: &[Value],
        mode: PermissionMode,
        executor: Arc<dyn NativeMcpToolExecutor>,
        policy: NativeMcpToolExecutionPolicy,
        batch: bool,
        clock: Arc<dyn NativeMcpRuntimeClock>,
        prepare: impl FnOnce(&NativeMcpRuntime, Arc<Mutex<Vec<u8>>>) -> NativeMcpRuntimeCandidate,
    ) -> Self {
        Self::configured(
            values,
            mode,
            0,
            Some((executor, policy)),
            batch,
            Some(clock),
            prepare,
        )
    }
    #[allow(
        clippy::too_many_lines,
        reason = "One fixture keeps matching engine, MCP, permission and prompt-principal ownership together."
    )]
    fn configured(
        values: &[Value],
        mode: PermissionMode,
        skip: usize,
        custom: Option<(Arc<dyn NativeMcpToolExecutor>, NativeMcpToolExecutionPolicy)>,
        batch: bool,
        runtime_clock: Option<Arc<dyn NativeMcpRuntimeClock>>,
        prepare: impl FnOnce(&NativeMcpRuntime, Arc<Mutex<Vec<u8>>>) -> NativeMcpRuntimeCandidate,
    ) -> Self {
        let contexts = Arc::new(NativeMcpContexts::new());
        let review_contexts = Arc::new(NativePermissionContexts::new());
        let clock = Arc::new(Clock(Instant::now()));
        let executor = Arc::new(Executor::default());
        executor.skip.store(skip, Ordering::SeqCst);
        let (selected_executor, selected_policy) =
            custom.unwrap_or_else(|| (executor.clone(), policy()));
        let runtime = Arc::new(
            NativeMcpRuntime::new(
                contexts.clone(),
                runtime_clock.unwrap_or_else(|| clock.clone()),
                selected_executor,
                selected_policy,
                NativeMcpRuntimeLimits::default(),
            )
            .unwrap(),
        );
        let writes: Arc<Mutex<Vec<u8>>> = Arc::default();
        let selected = prepare(&runtime, writes.clone());
        let name = selected.descriptors().tools()[0].name().to_owned();
        runtime.publish(selected).unwrap();
        let transport = Arc::new(Transport::default());
        let reviewer = Arc::new(AiGatewayPermissionReviewer::new(transport.clone(), clock));
        let targets = Arc::new(
            NativePermissionTargetAuthority::new(
                std::fs::File::open("/").unwrap(),
                "/".into(),
                vec![],
            )
            .unwrap(),
        );
        let authority: Arc<dyn NativeMcpPermissionAuthority> = runtime.clone();
        let preparer = Arc::new(
            NativeMcpPermissionPreparer::new(
                targets,
                Arc::new(Builtin),
                authority,
                contexts.clone(),
                review_contexts.clone(),
                reviewer,
                "/workspace",
            )
            .unwrap(),
        );
        let controller = Arc::new(NativePermissionController::new(preparer, Arc::new(Prompt)));
        let provider = if batch {
            batch_provider(values, &name)
        } else {
            provider(values, &name)
        };
        let store = InMemorySessionStore::default();
        let engine = Engine::builder()
            .limits(EngineLimits {
                max_model_rounds: 128.try_into().unwrap(),
                max_tool_calls_per_turn: 128.try_into().unwrap(),
                ..Default::default()
            })
            .session_store(store.clone())
            .provider(provider.clone())
            .shared_permission_handler(controller.clone())
            .tool(McpSelectTool::shared_catalog(runtime.clone()))
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("runtime").unwrap(),
                SessionIncarnationId::new("life").unwrap(),
            )
            .unwrap();
        let conversation = NativeConversation::from_session(session)
            .unwrap()
            .with_permission_controller(
                &controller,
                NativePermissionPolicySnapshot::new(
                    mode,
                    Arc::new(NativeConfiguredPermissionRules::default()),
                ),
            )
            .unwrap()
            .with_permission_contexts(&review_contexts)
            .unwrap()
            .with_mcp_contexts(&contexts)
            .unwrap();
        let conversation = NativeConversationRuntime::new(
            conversation,
            NativeModelPreferences::new("fixture/model", NativeReasoningEffort::default(), false)
                .unwrap(),
            None,
        )
        .unwrap();
        Self {
            conversation,
            prompt_principal: None,
            runtime,
            executor,
            transport,
            writes,
            provider,
            store,
            _engine: engine,
        }
    }
    pub fn run(&self) -> Vec<EngineEvent> {
        self.conversation
            .enqueue("Implement the requested change".into())
            .unwrap();
        let turn = futures_executor::block_on(self.conversation.start_next(1))
            .unwrap()
            .unwrap();
        let events = futures_executor::block_on(turn.collect::<Vec<_>>());
        assert!(events.iter().all(std::result::Result::is_ok), "{events:?}");
        let events: Vec<_> = events
            .into_iter()
            .map(std::result::Result::unwrap)
            .collect();
        assert!(
            events
                .iter()
                .any(|event| matches!(event.payload, TurnEvent::Completed { .. })),
            "{events:?}"
        );
        events
    }
}
fn step(id: &str, name: &str, arguments: Value) -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::ToolCall {
            call: ToolCall {
                id: ToolCallId::new(id).unwrap(),
                name: ToolName::new(name).unwrap(),
                arguments,
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolCalls,
        },
    ])
}

fn provider(values: &[Value], name: &str) -> ScriptedModelProvider {
    let script =
        values.chunks(32).flat_map(|values| {
            std::iter::once(step("select", MCP_SELECT_TOOL_NAME, json!({"name":name})))
                .chain(values.iter().enumerate().map(|(index, arguments)| {
                    step(&format!("call-{index}"), name, arguments.clone())
                }))
                .chain([ModelProviderStep::events([ModelEvent::Stop {
                    reason: StopReason::Completed,
                }])])
        });
    ScriptedModelProvider::new("fixture", script)
}

fn batch_provider(values: &[Value], name: &str) -> ScriptedModelProvider {
    let mut calls: Vec<_> = values
        .iter()
        .enumerate()
        .map(|(index, arguments)| ModelEvent::ToolCall {
            call: ToolCall {
                id: ToolCallId::new(format!("call-{index}")).unwrap(),
                name: ToolName::new(name).unwrap(),
                arguments: arguments.clone(),
            },
        })
        .collect();
    calls.push(ModelEvent::Stop {
        reason: StopReason::ToolCalls,
    });
    ScriptedModelProvider::new(
        "fixture",
        [
            step("select", MCP_SELECT_TOOL_NAME, json!({"name":name})),
            ModelProviderStep::events(calls),
            ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    )
}
