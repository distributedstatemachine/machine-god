#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt;
use machine_god_core::{
    Capability, ContentBlock, Engine, EngineEvent, Message, ModelEvent, PermissionDecision, Role,
    SessionId, SessionIncarnationId, StopReason, ToolCall, ToolCallId, ToolName, ToolOutput,
    TurnEvent,
};
use machine_god_native::{INSTALL_SKILL_TOOL_NAME, InstallSkillTool};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::json;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory(PathBuf);
impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-install-skill-engine-{}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create temp: {error}"),
            }
        }
        panic!("allocate temp")
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn provider(name: &str) -> ScriptedModelProvider {
    ScriptedModelProvider::new(
        name,
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("install-skill-call").unwrap(),
                        name: ToolName::new(INSTALL_SKILL_TOOL_NAME).unwrap(),
                        arguments: json!({"source":"./incoming//release-checks"}),
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

fn collect(engine: &Engine, name: &str) -> Vec<EngineEvent> {
    let session = engine
        .create_session(
            SessionId::new(name).unwrap(),
            SessionIncarnationId::new(format!("incarnation-{name}")).unwrap(),
        )
        .unwrap();
    futures_executor::block_on(async {
        session
            .prompt("install the local skill")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
}

fn second_tool_message(provider: &ScriptedModelProvider) -> Message {
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    requests[1].request.messages[2].clone()
}

fn output(message: &Message) -> &ToolOutput {
    assert_eq!(message.role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = message.content.as_slice() else {
        panic!("expected tool output")
    };
    output
}

#[test]
fn denial_precedes_source_and_destination_observation() {
    let workspace = TemporaryDirectory::new();
    let provider = provider("install-skill-denied");
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Deny {
            reason: "denied by test".to_owned(),
        })]);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(InMemorySessionStore::new())
        .permission_handler(policy.clone())
        .tool(InstallSkillTool::open(workspace.path()).unwrap())
        .build()
        .unwrap();

    let events = collect(&engine, "install-skill-denied");
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
    )));
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));
    let requests = policy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].capability,
        Capability::Custom {
            name: "install_skill".to_owned(),
            details: json!({
                "source":"incoming/release-checks",
                "destination":"skills/release-checks"
            }),
        }
    );
    assert_eq!(
        output(&second_tool_message(&provider)),
        &ToolOutput {
            content: json!({
                "code":"permission_denied",
                "message":"tool execution was denied by policy"
            }),
            is_error: true,
        }
    );
    assert!(!workspace.path().join("incoming").exists());
    assert!(!workspace.path().join("skills").exists());
}
