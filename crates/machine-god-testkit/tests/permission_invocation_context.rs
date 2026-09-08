//! The public locator is scoped to authorization, not a search by call ID.
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, Capability, ContentBlock, Engine, FilesystemAccess, ModelEvent, PermissionDecision,
    PermissionError, PermissionGrantScope, PermissionHandler, PermissionInvocationSnapshot,
    PermissionRequest, Session, SessionId, SessionIncarnationId, StopReason, ToolCall, ToolCallId,
    ToolName, ToolOutput, ToolSpec,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPreparedTool,
    ToolPrepareStep, ToolStep,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    session: Mutex<Option<Session>>,
    snapshots: Mutex<Vec<(PermissionRequest, PermissionInvocationSnapshot)>>,
}
struct Legacy(Arc<State>);
impl PermissionHandler for Legacy {
    fn authorize(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        let session = self.0.session.lock().unwrap().clone().unwrap();
        // Available during construction as well as policy future polling.
        let snapshot = session.permission_invocation_snapshot(&request).unwrap();
        assert!(snapshot.is_live());
        Box::pin(async move {
            assert!(session.permission_invocation_snapshot(&request).is_some());
            let mut wrong = request.clone();
            wrong.turn_id = machine_god_core::TurnId::new("wrong").unwrap();
            assert!(session.permission_invocation_snapshot(&wrong).is_none());
            self.0.snapshots.lock().unwrap().push((request, snapshot));
            Ok(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })
        })
    }
}
#[test]
fn legacy_policy_observes_original_call_only_inside_exact_authorization() {
    let state = Arc::new(State::default());
    let original = json!({"source":"original"});
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::new())
        .provider(ScriptedModelProvider::new(
            "fixture",
            [
                ModelProviderStep::events([
                    ModelEvent::ToolCall {
                        call: ToolCall {
                            id: ToolCallId::new("call").unwrap(),
                            name: ToolName::new("fixture").unwrap(),
                            arguments: original.clone(),
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
        ))
        .permission_handler(Legacy(state.clone()))
        .tool(ScriptedPreparedTool::new(
            ToolSpec {
                name: ToolName::new("fixture").unwrap(),
                description: "fixture".into(),
                input_schema: json!({}),
            },
            [ToolPrepareStep::Prepared {
                capability: Capability::Filesystem {
                    access: FilesystemAccess::Read,
                    path: "file".into(),
                },
                arguments: json!({"prepared":true}),
            }],
            [ToolStep::Output(ToolOutput::success("done"))],
        ))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("session").unwrap(),
            SessionIncarnationId::new("life").unwrap(),
        )
        .unwrap();
    *state.session.lock().unwrap() = Some(session.clone());
    let turn = block_on(session.prompt("root")).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok));
    state.session.lock().unwrap().take();
    let snapshots = state.snapshots.lock().unwrap();
    assert_eq!(snapshots.len(), 1);
    let (request, snapshot) = &snapshots[0];
    assert!(!snapshot.is_live());
    assert!(session.permission_invocation_snapshot(request).is_none());
    assert_eq!(snapshot.source_cursor(), (1, 0));
    let [ContentBlock::ToolCall { call }] = snapshot.pending_assistant().content.as_slice() else {
        panic!("one exact call")
    };
    assert_eq!(call.arguments, original);
    assert!(!format!("{snapshot:?}").contains("original"));
}
