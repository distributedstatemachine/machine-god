use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::*;
use machine_god_native::NativePermissionGovernedTool;
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};
use serde_json::{Value, json};

struct HostInteraction {
    executed: Arc<AtomicUsize>,
    archived: Arc<AtomicUsize>,
    prepared_bytes: usize,
}

impl Tool for HostInteraction {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("interaction").unwrap(),
            description: "Explicit host interaction".into(),
            input_schema: json!({"type":"object"}),
        }
    }

    fn prepare(&self, _: ToolCall) -> Result<PreparedToolCall, ToolError> {
        Ok(PreparedToolCall::without_authority(json!({
            "normalized": "x".repeat(self.prepared_bytes)
        })))
    }

    fn persist_arguments<'a>(
        &'a self,
        _: ToolContext,
        _: &'a Value,
        _: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        Box::pin(async {
            self.archived.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
    }

    fn execute(
        &self,
        _: ToolContext,
        arguments: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            self.executed.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput::success(arguments))
        })
    }

    fn execute_for_turn(
        &self,
        _: ToolContext,
        arguments: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        Box::pin(async move {
            self.executed.fetch_add(1, Ordering::SeqCst);
            Ok(ToolExecution::with_persisted_output(
                ToolOutput::success(arguments),
                ToolOutput::success(json!({"reference":"retained"})),
            ))
        })
    }
}

struct Deny(Arc<AtomicUsize>);
impl PermissionHandler for Deny {
    fn authorize(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                request.capability,
                Capability::Tool {
                    name: ToolName::new("interaction").unwrap(),
                    call_id: ToolCallId::new("same-call").unwrap(),
                    arguments: json!({"normalized":"x"}),
                }
            );
            Ok(PermissionDecision::Deny {
                reason: "denied".into(),
            })
        })
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
        turn_id: TurnId::new("turn").unwrap(),
        call_id: ToolCallId::new("same-call").unwrap(),
    }
}

fn call() -> ToolCall {
    ToolCall {
        name: ToolName::new("interaction").unwrap(),
        id: ToolCallId::new("same-call").unwrap(),
        arguments: json!({"provider_input":"not the canonical value"}),
    }
}

#[test]
fn actual_engine_denial_governs_a_previously_no_authority_call() {
    let executed = Arc::new(AtomicUsize::new(0));
    let requested = Arc::new(AtomicUsize::new(0));
    let tool = NativePermissionGovernedTool::new(
        Arc::new(HostInteraction {
            executed: executed.clone(),
            archived: Arc::default(),
            prepared_bytes: 1,
        }),
        EngineLimits::default(),
    );
    let provider = ScriptedModelProvider::new(
        "test",
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall { call: call() },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    );
    let engine = Engine::builder()
        .provider(provider)
        .session_store(InMemorySessionStore::default())
        .permission_handler(Deny(requested.clone()))
        .tool(tool)
        .build()
        .unwrap();
    let session = engine
        .create_session(context().session_id, context().session_incarnation_id)
        .unwrap();
    let mut turn = block_on(session.prompt("perform the interaction")).unwrap();
    let mut completed = false;
    block_on(async {
        while let Some(event) = turn.next().await {
            let event = event.unwrap();
            assert!(
                !matches!(event.payload, TurnEvent::Failed { .. }),
                "{event:?}"
            );
            completed |= matches!(event.payload, TurnEvent::Completed { .. });
        }
    });
    assert!(completed);
    assert_eq!(requested.load(Ordering::SeqCst), 1);
    assert_eq!(executed.load(Ordering::SeqCst), 0);
}

#[test]
fn bounds_are_checked_before_cloning_normalized_arguments_into_capability() {
    let tool = NativePermissionGovernedTool::new(
        Arc::new(HostInteraction {
            executed: Arc::default(),
            archived: Arc::default(),
            prepared_bytes: 100,
        }),
        EngineLimits {
            max_tool_argument_bytes: NonZeroUsize::new(32).unwrap(),
            ..EngineLimits::default()
        },
    );
    assert_eq!(
        tool.prepare(call()).unwrap_err().code,
        "permission_preparation_failed"
    );
    assert_eq!(
        tool.prepare_for_turn(&context(), call()).unwrap_err().code,
        "permission_preparation_failed"
    );
}

struct ContextInteraction(bool);
impl Tool for ContextInteraction {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: call().name,
            description: "context interaction".into(),
            input_schema: json!({}),
        }
    }
    fn prepare(&self, _: ToolCall) -> Result<PreparedToolCall, ToolError> {
        panic!("wrapper must forward the contextual hook")
    }
    fn prepare_for_turn(
        &self,
        context: &ToolContext,
        _: ToolCall,
    ) -> Result<PreparedToolCall, ToolError> {
        let arguments = json!({"context": context});
        Ok(if self.0 {
            PreparedToolCall::new(
                Capability::Filesystem {
                    access: FilesystemAccess::Read,
                    path: "retained".into(),
                },
                arguments,
            )
        } else {
            PreparedToolCall::without_authority(arguments)
        })
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        panic!("preparation cannot execute the inner tool")
    }
}

#[test]
fn contextual_wrapper_preserves_selection_and_still_requires_permission() {
    for concrete in [false, true] {
        let tool = NativePermissionGovernedTool::new(
            Arc::new(ContextInteraction(concrete)),
            EngineLimits::default(),
        );
        let prepared = tool.prepare_for_turn(&context(), call()).unwrap();
        let arguments = json!({"context": context()});
        assert_eq!(prepared.arguments(), &arguments);
        assert_eq!(
            prepared.capability(),
            Some(&if concrete {
                Capability::Filesystem {
                    access: FilesystemAccess::Read,
                    path: "retained".into(),
                }
            } else {
                Capability::Tool {
                    name: call().name,
                    call_id: call().id,
                    arguments,
                }
            })
        );
    }
}

#[test]
fn wrapper_preserves_owned_archive_and_extended_execution_futures() {
    let executed = Arc::new(AtomicUsize::new(0));
    let archived = Arc::new(AtomicUsize::new(0));
    let tool = NativePermissionGovernedTool::new(
        Arc::new(HostInteraction {
            executed: executed.clone(),
            archived: archived.clone(),
            prepared_bytes: 1,
        }),
        EngineLimits::default(),
    );
    let arguments = json!({"selected":true});
    let publication = tool.persist_arguments(context(), &arguments, CancellationToken::new());
    assert_eq!(archived.load(Ordering::SeqCst), 0);
    assert_eq!(block_on(publication).unwrap(), None);
    assert_eq!(archived.load(Ordering::SeqCst), 1);
    let execution = tool.execute_for_turn(context(), arguments.clone(), CancellationToken::new());
    assert_eq!(executed.load(Ordering::SeqCst), 0);
    let result = block_on(execution).unwrap();
    assert_eq!(result.tool_output().content, arguments);
    assert_eq!(
        result.persisted_output().unwrap().content,
        json!({"reference":"retained"})
    );
    assert_eq!(executed.load(Ordering::SeqCst), 1);
}
