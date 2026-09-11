use super::*;
use machine_god_core::{
    Capability, PermissionDecision, PermissionGrantScope, PreparedToolCall, ToolContext, ToolSpec,
};
use machine_god_testkit::{ModelProviderStep, PermissionStep};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
pub(super) struct FinishTool {
    factory: Arc<dyn Fn(usize) -> Result<ToolExecution, ToolError> + Send + Sync>,
    pub preparations: Arc<AtomicUsize>,
    pub executions: Arc<AtomicUsize>,
    pub completion_wins: bool,
    pub cancel_on_poll: bool,
}
impl FinishTool {
    pub fn new(
        factory: impl Fn(usize) -> Result<ToolExecution, ToolError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            factory: Arc::new(factory),
            preparations: Arc::default(),
            executions: Arc::default(),
            completion_wins: false,
            cancel_on_poll: false,
        }
    }
}
impl Tool for FinishTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("action").unwrap(),
            description: "fixture".into(),
            input_schema: json!({"type":"object"}),
        }
    }
    fn complete_output_limits(&self) -> Option<ToolOutputLimits> {
        Some(ToolOutputLimits {
            max_serialized_bytes: (256 * 1024).try_into().unwrap(),
            max_json_nodes: 4096.try_into().unwrap(),
        })
    }
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        self.preparations.fetch_add(1, Ordering::SeqCst);
        let prepared = PreparedToolCall::new(
            Capability::Tool {
                name: call.name,
                call_id: call.id,
                arguments: call.arguments.clone(),
            },
            call.arguments,
        );
        Ok(if self.completion_wins {
            prepared.completion_wins_after_first_poll()
        } else {
            prepared
        })
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async { panic!("core must preserve turn effects") })
    }
    fn execute_for_turn(
        &self,
        _: ToolContext,
        _: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        Box::pin(async move {
            let index = self.executions.fetch_add(1, Ordering::SeqCst);
            if self.cancel_on_poll {
                cancellation.cancel();
            }
            (self.factory)(index)
        })
    }
}

pub(super) struct Fixture {
    pub session: Session,
    pub provider: ScriptedModelProvider,
    pub permissions: ScriptedPermissionHandler,
    pub store: InMemorySessionStore,
    pub tool: FinishTool,
}
impl Fixture {
    pub fn new(tool: FinishTool, count: usize) -> Self {
        Self::with_store(tool, count, InMemorySessionStore::new())
    }
    pub fn with_store(tool: FinishTool, count: usize, store: InMemorySessionStore) -> Self {
        Self::with_sink(
            tool,
            count,
            store,
            Arc::new(machine_god_core::NoopEventSink),
        )
    }
    pub fn with_sink(
        tool: FinishTool,
        count: usize,
        store: InMemorySessionStore,
        sink: Arc<dyn machine_god_core::EventSink>,
    ) -> Self {
        let mut events: Vec<_> = (0..count)
            .map(|index| ModelEvent::ToolCall {
                call: ToolCall {
                    id: ToolCallId::new(format!("call-{index}")).unwrap(),
                    name: ToolName::new("action").unwrap(),
                    arguments: json!({"index":index}),
                },
            })
            .collect();
        events.push(ModelEvent::Usage {
            usage: TokenUsage {
                input_tokens: 3,
                output_tokens: 5,
                ..TokenUsage::default()
            },
        });
        events.push(ModelEvent::Stop {
            reason: StopReason::ToolCalls,
        });
        let provider = ScriptedModelProvider::new(
            "fixture",
            [
                ModelProviderStep::events(events),
                ModelProviderStep::events([
                    ModelEvent::TextDelta {
                        text: "next round".into(),
                    },
                    ModelEvent::Stop {
                        reason: StopReason::Completed,
                    },
                ]),
            ],
        );
        let permissions = ScriptedPermissionHandler::new((0..count).map(|_| {
            PermissionStep::Decision(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })
        }));
        let engine = Engine::builder()
            .provider(provider.clone())
            .session_store(store.clone())
            .permission_handler(permissions.clone())
            .shared_event_sink(sink)
            .tool(tool.clone())
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("finish").unwrap(),
                SessionIncarnationId::new("life").unwrap(),
            )
            .unwrap();
        Self {
            session,
            provider,
            permissions,
            store,
            tool,
        }
    }
    pub fn start(&self) -> Turn {
        block_on(self.session.prompt("go")).unwrap()
    }
    pub fn run(&self) -> Vec<EngineEvent> {
        collect(self.start())
    }
    pub fn record(&self) -> SessionRecord {
        self.store.record(&self.session.id()).unwrap()
    }
    pub fn output(&self, index: usize) -> ToolOutput {
        let record = self.record();
        let ContentBlock::ToolResult { call_id, output } = &record.messages[index + 2].content[0]
        else {
            panic!("missing result")
        };
        assert_eq!(call_id.as_str(), format!("call-{index}"));
        output.clone()
    }
    pub fn assert_calls(&self, expected: usize) {
        assert_eq!(self.tool.preparations.load(Ordering::SeqCst), expected);
        assert_eq!(self.tool.executions.load(Ordering::SeqCst), expected);
        assert_eq!(self.permissions.requests().len(), expected);
    }
}
pub(super) fn collect(turn: Turn) -> Vec<EngineEvent> {
    block_on(turn.collect::<Vec<_>>())
        .into_iter()
        .collect::<Result<_, _>>()
        .unwrap()
}
pub(super) fn next(turn: &mut Turn) -> EngineEvent {
    block_on(turn.next()).unwrap().unwrap()
}
pub(super) fn completed(events: &[EngineEvent], reason: &StopReason) {
    assert!(
        matches!(&events.last().unwrap().payload, TurnEvent::Completed { reason: actual, .. } if actual == reason),
        "{events:?}"
    );
}
pub(super) fn finished(events: &[EngineEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
        .count()
}
