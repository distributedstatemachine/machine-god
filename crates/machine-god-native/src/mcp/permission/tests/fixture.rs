use super::*;
use crate::mcp::{
    protocol::{NegotiatedProtocol, ProtocolVersion, TransportKind},
    schema::{McpSchema, McpSchemaLimits},
    submission::{
        McpSubmissionRuntime, McpSubmissionRuntimeBinding, McpSubmissionRuntimeOwner,
        McpSubmissionWriter, McpToolCallOptions,
    },
};
use machine_god_core::*;
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};
use serde_json::{Value, json};
use std::{
    fs::File,
    io,
    sync::{Mutex, Weak},
    task::{Context, Poll},
    time::Instant,
};

pub(super) const NAME: &str = "calendar_lookup";
pub(super) struct Clock(Instant);
impl NativePermissionReviewClock for Clock {
    fn now(&self) -> Instant {
        self.0
    }
    fn wait_until(&self, _: Instant) -> BoxFuture<'static, ()> {
        Box::pin(std::future::pending())
    }
}
#[derive(Default)]
pub(super) struct Hooks {
    pub owner: Mutex<Weak<NativePermissionSession>>,
    pub revoke: AtomicUsize,
}
impl Hooks {
    fn run(&self, point: usize) {
        if self.revoke.load(Ordering::SeqCst) == point {
            self.owner
                .lock()
                .unwrap()
                .upgrade()
                .unwrap()
                .reset()
                .unwrap();
        }
    }
}
pub(super) struct Transport {
    pub wire: Mutex<Vec<Value>>,
    pub decision: &'static str,
    pub hooks: Arc<Hooks>,
    pub drops: AtomicUsize,
}
struct PendingReview<'a>(&'a AtomicUsize);
impl Drop for PendingReview<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl AiGatewayTransport for Transport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        Box::pin(async move {
            self.wire
                .lock()
                .unwrap()
                .push(machine_god_core::json::from_slice(request.body()).unwrap());
            self.hooks.run(1);
            if self.decision == "pending" {
                let _guard = PendingReview(&self.drops);
                return std::future::pending().await;
            }
            let answer = json!({"type":"tool-call", "toolCallId":"assessment", "toolName":"permission_decision",
                "input":{"risk":"low","authorization":"unknown","decision":self.decision,"rationale":"Requested development task."}});
            let finish = json!({"type":"finish","finishReason":{"unified":"tool-calls"}});
            let bytes = format!("data: {answer}\n\ndata: {finish}\n\n").into_bytes();
            Ok(Box::pin(futures_util::stream::iter([Ok(bytes)])) as AiGatewayByteStream)
        })
    }
}
pub(super) struct Prompter {
    pub calls: AtomicUsize,
    pub reusable: AtomicUsize,
    pub decision: PermissionPromptDecision,
}
impl PermissionPrompter for Prompter {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.decision)
        })
    }
    fn prompt_with_rule(
        &self,
        request: PermissionRequest,
        rule: Option<NativePermissionRulePrompt>,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async move {
            if rule.is_some() {
                self.reusable.fetch_add(1, Ordering::SeqCst);
            }
            self.prompt(request).await
        })
    }
}
#[derive(Default)]
pub(super) struct Builtin {
    pub calls: AtomicUsize,
    pub closed: AtomicUsize,
}
impl NativePermissionActionPreparer for Builtin {
    fn close_turn(&self, _: &SessionId, _: &SessionIncarnationId, _: &TurnId) {
        self.closed.fetch_add(1, Ordering::SeqCst);
    }
    fn prepare<'a>(
        &'a self,
        _: &'a PermissionRequest,
        _: PermissionInvocation<'a>,
        _: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn NativePreparedPermissionAction>, PermissionError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(invalid())
        })
    }
}
pub(super) struct Authority {
    pub owner: McpSubmissionRuntimeOwner,
    pub runtime: Arc<McpSubmissionRuntime>,
    pub schema: McpSchema,
    pub calls: AtomicUsize,
    pub fault: AtomicUsize,
    pub pending: Mutex<Vec<crate::mcp::submission::McpPendingToolReservation>>,
}
impl Authority {
    pub fn new(schema: &str) -> Self {
        let schema = McpSchema::parse(schema.as_bytes(), McpSchemaLimits::default()).unwrap();
        let owner = McpSubmissionRuntimeOwner::new();
        let runtime = owner
            .install(
                McpSubmissionRuntimeBinding::new(
                    "calendar",
                    ToolName::new(NAME).unwrap(),
                    "lookup",
                    b"secret-config",
                    schema.raw_json().as_bytes(),
                    b"secret-credential",
                )
                .unwrap(),
            )
            .unwrap();
        Self {
            owner,
            runtime,
            schema,
            calls: AtomicUsize::new(0),
            fault: AtomicUsize::new(0),
            pending: Mutex::default(),
        }
    }
    pub fn project(
        &self,
        invocation: PermissionInvocation<'_>,
    ) -> Result<McpToolRequest, PermissionError> {
        McpToolRequest::new(
            self.runtime.clone(),
            &self.schema,
            invocation,
            McpToolCallOptions::new(
                NegotiatedProtocol {
                    transport: TransportKind::Stdio,
                    version: ProtocolVersion::Modern,
                },
                42,
            )
            .unwrap(),
        )
        .map_err(|_| invalid())
    }
}
impl NativeMcpPermissionAuthority for Authority {
    fn resolve<'a>(
        &'a self,
        _: &'a PermissionRequest,
        invocation: PermissionInvocation<'a>,
        context: &'a NativeMcpTurnContext,
        _: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<McpToolRequest>, PermissionError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            context.revalidate().map_err(|_| invalid())?;
            match self.fault.load(Ordering::SeqCst) {
                1 => return Ok(None),
                2 => return Err(invalid()),
                3 => self.owner.retire(),
                4 => {
                    let changed = json!({"different": true});
                    return self
                        .project(PermissionInvocation {
                            arguments: &changed,
                            ..invocation
                        })
                        .map(Some);
                }
                5 => {
                    let changed = ToolCallId::new("different").unwrap();
                    return self
                        .project(PermissionInvocation {
                            call_id: &changed,
                            ..invocation
                        })
                        .map(Some);
                }
                6 => context.registry().map_err(|_| invalid())?.retire(),
                _ => {}
            }
            let (pending, lease) = crate::mcp::submission::McpPendingToolReservation::leased(
                crate::mcp::protocol::RpcId::Integer(42),
            );
            self.pending.lock().unwrap().push(pending);
            self.project(invocation)?
                .with_reservation(lease)
                .map(Some)
                .map_err(|_| invalid())
        })
    }
}
struct Writer(Arc<Mutex<Vec<u8>>>);
impl McpSubmissionWriter for Writer {
    fn poll_write(&mut self, _: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
pub(super) struct Executable {
    pub authority: Arc<Authority>,
    pub contexts: Arc<NativeMcpContexts>,
    pub writes: Arc<Mutex<Vec<u8>>>,
    pub hooks: Arc<Hooks>,
}
impl Tool for Executable {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new(NAME).unwrap(),
            description: "Look up a calendar".into(),
            input_schema: machine_god_core::json::from_str(self.authority.schema.raw_json())
                .unwrap(),
        }
    }
    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            let error =
                || ToolError::new(ToolErrorKind::PermissionDenied, "fixture", "fixture", false);
            let registry = self
                .contexts
                .snapshot_for_tool(&context)
                .map_err(|_| error())?
                .registry()
                .map_err(|_| error())?;
            self.hooks.run(2);
            let submission = registry
                .claim(
                    context,
                    &ToolName::new(NAME).unwrap(),
                    &arguments,
                    self.authority.runtime.clone(),
                    cancellation,
                )
                .await
                .map_err(|_| error())?;
            assert!(
                self.authority
                    .pending
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .matches(submission.rpc_id(), submission.tool_reservation())
            );
            self.hooks.run(3);
            submission
                .into_writer(Writer(self.writes.clone()))
                .await
                .map_err(|_| error())?;
            Ok(ToolOutput::success(json!({"executed":true})))
        })
    }
}
pub(super) struct Fixture {
    pub runtime: NativeConversationRuntime,
    pub authority: Arc<Authority>,
    pub preparer: Arc<NativeMcpPermissionPreparer>,
    pub transport: Arc<Transport>,
    pub prompt: Arc<Prompter>,
    pub writes: Arc<Mutex<Vec<u8>>>,
    pub hooks: Arc<Hooks>,
    pub builtin: Arc<Builtin>,
    pub _engine: Engine,
}
impl Fixture {
    pub fn new(
        mode: PermissionMode,
        decision: &'static str,
        prompt_decision: PermissionPromptDecision,
        schema: &str,
        values: Vec<Value>,
        rules: NativeConfiguredPermissionRules,
    ) -> Self {
        Self::with_workspace(mode, decision, prompt_decision, schema, values, rules, None)
    }

    #[allow(clippy::too_many_lines)]
    pub fn with_workspace(
        mode: PermissionMode,
        decision: &'static str,
        prompt_decision: PermissionPromptDecision,
        schema: &str,
        values: Vec<Value>,
        rules: NativeConfiguredPermissionRules,
        workspace: Option<(
            Arc<NativeWorkspaceContexts>,
            Option<NativeWorkspaceAuthority>,
        )>,
    ) -> Self {
        let authority = Arc::new(Authority::new(schema));
        let contexts = Arc::new(NativeMcpContexts::new());
        let review_contexts = Arc::new(NativePermissionContexts::new());
        let hooks = Arc::new(Hooks::default());
        let transport = Arc::new(Transport {
            wire: Mutex::default(),
            decision,
            hooks: hooks.clone(),
            drops: AtomicUsize::new(0),
        });
        let reviewer = Arc::new(AiGatewayPermissionReviewer::new(
            transport.clone(),
            Arc::new(Clock(Instant::now())),
        ));
        let builtin = Arc::new(Builtin::default());
        let builtins = Arc::new(
            NativePermissionTargetAuthority::new(File::open("/").unwrap(), "/".into(), vec![])
                .unwrap(),
        );
        let preparer = NativeMcpPermissionPreparer::new(
            builtins,
            builtin.clone(),
            authority.clone(),
            contexts.clone(),
            review_contexts.clone(),
            reviewer,
            "/workspace",
        )
        .unwrap();
        let preparer = Arc::new(match &workspace {
            Some((contexts, _)) => preparer.with_workspace_contexts(contexts.clone()),
            None => preparer,
        });
        let prompt = Arc::new(Prompter {
            calls: AtomicUsize::new(0),
            reusable: AtomicUsize::new(0),
            decision: prompt_decision,
        });
        let controller = Arc::new(NativePermissionController::new(
            preparer.clone(),
            prompt.clone(),
        ));
        let script = values
            .into_iter()
            .enumerate()
            .map(|(index, arguments)| {
                ModelProviderStep::events([
                    ModelEvent::ToolCall {
                        call: ToolCall {
                            id: ToolCallId::new(format!("call-{index}")).unwrap(),
                            name: ToolName::new(NAME).unwrap(),
                            arguments,
                        },
                    },
                    ModelEvent::Stop {
                        reason: StopReason::ToolCalls,
                    },
                ])
            })
            .chain([ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }])]);
        let writes = Arc::new(Mutex::new(Vec::new()));
        let engine = Engine::builder()
            .session_store(InMemorySessionStore::default())
            .provider(ScriptedModelProvider::new("fixture", script))
            .shared_permission_handler(controller.clone())
            .tool(Executable {
                authority: authority.clone(),
                contexts: contexts.clone(),
                writes: writes.clone(),
                hooks: hooks.clone(),
            })
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("permission").unwrap(),
                SessionIncarnationId::new("life").unwrap(),
            )
            .unwrap();
        let conversation = NativeConversation::from_session(session)
            .unwrap()
            .with_permission_controller(
                &controller,
                NativePermissionPolicySnapshot::new(mode, Arc::new(rules)),
            )
            .unwrap()
            .with_permission_contexts(&review_contexts)
            .unwrap()
            .with_mcp_contexts(&contexts)
            .unwrap();
        let conversation = match workspace {
            Some((contexts, Some(authority))) => conversation
                .with_workspace_contexts(authority, &contexts)
                .unwrap(),
            _ => conversation,
        };
        *hooks.owner.lock().unwrap() = Arc::downgrade(conversation.permissions().unwrap());
        let runtime = NativeConversationRuntime::new(
            conversation,
            NativeModelPreferences::new("fixture/model", NativeReasoningEffort::default(), false)
                .unwrap(),
            None,
        )
        .unwrap();
        Self {
            runtime,
            authority,
            preparer,
            transport,
            prompt,
            writes,
            hooks,
            builtin,
            _engine: engine,
        }
    }
    pub fn start(&self) -> NativeConversationRuntimeTurn {
        self.runtime
            .enqueue("Implement the requested change".into())
            .unwrap();
        futures_executor::block_on(self.runtime.start_next(1))
            .unwrap()
            .unwrap()
    }
    pub fn run(&self) {
        let events = futures_executor::block_on(self.start().collect::<Vec<_>>());
        assert!(events.iter().all(Result::is_ok), "{events:?}");
        assert!(
            events.iter().any(|event| matches!(
                event.as_ref().unwrap().payload,
                TurnEvent::Completed { .. }
            )),
            "{events:?}"
        );
    }
    pub fn write_count(&self) -> usize {
        self.writes
            .lock()
            .unwrap()
            .split(|byte| *byte == b'\n')
            .filter(|part| !part.is_empty())
            .count()
    }
    pub fn key(&self, arguments: &Value) -> NativePermissionRuleKey {
        let name = ToolName::new(NAME).unwrap();
        let id = ToolCallId::new("key").unwrap();
        let projection = self
            .authority
            .project(PermissionInvocation {
                tool_name: &name,
                call_id: &id,
                arguments,
            })
            .unwrap();
        let binding = projection.binding();
        identity::key(
            "/workspace",
            binding.server(),
            NAME,
            binding.remote_tool(),
            &identity::runtime_fingerprint(&[
                binding.configuration_bytes(),
                binding.schema_bytes(),
                binding.authentication_bytes(),
            ]),
            projection.arguments_json(),
        )
        .unwrap()
    }
}
