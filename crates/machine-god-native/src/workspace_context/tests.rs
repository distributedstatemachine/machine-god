use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::task::Context;

use crate::{
    NATIVE_SESSION_METADATA_KEY, NativeConversation, NativeConversationRuntime,
    NativeModelPreferences, NativeReasoningEffort, NativeSessionMetadata,
};
use futures_executor::block_on;
use futures_util::{StreamExt, task::noop_waker};
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, Engine, FilesystemAccess, InferenceOptions,
    ModelEvent, PermissionDecision, PermissionError, PermissionGrantScope, PermissionHandler,
    PreparedToolCall, SessionRecord, SessionRevision, SessionStoreError, SessionStoreErrorKind,
    StopReason, Tool, ToolCall, ToolCallId, ToolError, ToolName, ToolOutput, ToolSpec,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
    SessionStoreScript, SessionStoreStep,
};
use serde_json::{Value, json};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: PathBuf,
    authority: NativeWorkspaceAuthority,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "machine-god-workspace-turns-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let primary = base.join("primary");
        let state = base.join("state");
        std::fs::create_dir(&primary).unwrap();
        std::fs::create_dir(&state).unwrap();
        let open = |path: &Path| {
            rustix::fs::open(
                path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            )
            .unwrap()
        };
        let authority = NativeWorkspaceAuthority::open_blocking(
            open(&primary),
            primary,
            Some(open(&state)),
            state,
            vec![],
            false,
        )
        .unwrap();
        Self { base, authority }
    }
    fn install(&self) {
        self.authority
            .install(self.authority.prepare_blocking(vec![], false).unwrap())
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}

fn record(id: &str, incarnation: &str) -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new(id).unwrap(),
        SessionIncarnationId::new(incarnation).unwrap(),
    );
    record.revision = SessionRevision(1);
    record.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.to_owned(),
        NativeSessionMetadata::default().to_value(),
    );
    record
}

fn finished() -> ModelProviderStep {
    ModelProviderStep::events([ModelEvent::Stop {
        reason: StopReason::Completed,
    }])
}

fn preferences() -> NativeModelPreferences {
    NativeModelPreferences::new(
        "model",
        NativeReasoningEffort::parse("high").unwrap(),
        false,
    )
    .unwrap()
}

fn setup(
    fixture: &Fixture,
    contexts: &Arc<NativeWorkspaceContexts>,
    script: SessionStoreScript,
) -> (
    NativeConversation,
    InMemorySessionStore,
    ScriptedModelProvider,
) {
    let record = record("session", "incarnation");
    let store = InMemorySessionStore::configured(
        BTreeMap::from([(record.id.clone(), record)]),
        script,
        128,
    );
    let provider = ScriptedModelProvider::new("test", [finished(), finished(), finished()]);
    let engine = Engine::builder()
        .session_store(store.clone())
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(SessionId::new("session").unwrap()))
        .unwrap()
        .unwrap();
    let conversation = NativeConversation::from_session(session)
        .unwrap()
        .with_workspace_contexts(fixture.authority.clone(), contexts)
        .unwrap();
    (conversation, store, provider)
}

fn context(conversation: &NativeConversation, turn: &crate::NativeConversationTurn) -> ToolContext {
    ToolContext {
        session_id: conversation.id(),
        session_incarnation_id: conversation.incarnation_id(),
        turn_id: turn.handle().id().clone(),
        call_id: ToolCallId::new("call").unwrap(),
    }
}

fn active_count(contexts: &NativeWorkspaceContexts) -> usize {
    contexts
        .routes
        .lock()
        .unwrap()
        .iter()
        .filter_map(Weak::upgrade)
        .filter(|owner| {
            owner
                .active
                .lock()
                .unwrap()
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some()
        })
        .count()
}

#[test]
fn direct_prompt_pins_scope_without_permission_composition_and_retires_on_completion() {
    let fixture = Fixture::new();
    let contexts = Arc::new(NativeWorkspaceContexts::new());
    let (conversation, _, _) = setup(&fixture, &contexts, SessionStoreScript::default());
    let turn = block_on(conversation.prompt("first".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let retained = contexts.snapshot_for_tool(&key).unwrap();
    fixture.install();
    assert_eq!(retained.snapshot().unwrap().generation(), 0);
    assert_eq!(
        contexts
            .snapshot_for_tool(&key)
            .unwrap()
            .snapshot()
            .unwrap()
            .generation(),
        0
    );
    assert!(
        block_on(turn.collect::<Vec<_>>())
            .iter()
            .all(std::result::Result::is_ok)
    );
    assert!(!retained.is_live());
    assert!(retained.snapshot().is_err());
    assert!(contexts.snapshot_for_tool(&key).is_err());
    assert_eq!(active_count(&contexts), 0);
    let next = block_on(conversation.prompt("next".into(), 2)).unwrap();
    assert_eq!(
        contexts
            .snapshot_for_tool(&context(&conversation, &next))
            .unwrap()
            .snapshot()
            .unwrap()
            .generation(),
        1
    );
}

#[test]
fn queue_captures_on_take_not_enqueue_and_taken_scope_survives_runtime_drop() {
    let fixture = Fixture::new();
    let contexts = Arc::new(NativeWorkspaceContexts::new());
    let (conversation, _, provider) = setup(&fixture, &contexts, SessionStoreScript::default());
    let runtime = NativeConversationRuntime::new(conversation, preferences(), None).unwrap();
    runtime.enqueue("queued".into()).unwrap();
    let unpolled = runtime.start_next(1);
    fixture.install();
    drop(unpolled);
    assert_eq!(runtime.status().queued_jobs, 1);
    assert!(provider.requests().is_empty());
    let turn = block_on(runtime.start_next(1)).unwrap().unwrap();
    let key = ToolContext {
        session_id: runtime.id(),
        session_incarnation_id: runtime.record().incarnation_id,
        turn_id: turn.handle().unwrap().id().clone(),
        call_id: ToolCallId::new("call").unwrap(),
    };
    fixture.install();
    assert_eq!(
        contexts
            .snapshot_for_tool(&key)
            .unwrap()
            .snapshot()
            .unwrap()
            .generation(),
        1
    );
    drop(runtime);
    assert_eq!(
        contexts
            .snapshot_for_tool(&key)
            .unwrap()
            .snapshot()
            .unwrap()
            .generation(),
        1
    );
    drop(turn);
    assert!(contexts.snapshot_for_tool(&key).is_err());
}

#[test]
fn cancellation_foreign_identity_and_continuation_expire_old_scope() {
    let fixture = Fixture::new();
    let contexts = Arc::new(NativeWorkspaceContexts::new());
    let (conversation, _, _) = setup(&fixture, &contexts, SessionStoreScript::default());
    let turn = block_on(conversation.prompt("first".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let scope = contexts.snapshot_for_tool(&key).unwrap();
    for mut foreign in [key.clone(), key.clone(), key.clone()]
        .into_iter()
        .enumerate()
    {
        match foreign.0 {
            0 => foreign.1.session_id = SessionId::new("other").unwrap(),
            1 => foreign.1.session_incarnation_id = SessionIncarnationId::new("other").unwrap(),
            _ => foreign.1.turn_id = TurnId::new("other").unwrap(),
        }
        assert!(contexts.snapshot_for_tool(&foreign.1).is_err());
    }
    assert!(turn.handle().cancel());
    assert!(!scope.is_live());
    assert!(contexts.snapshot_for_tool(&key).is_err());
    drop(turn);
    fixture.install();
    let resumed = block_on(conversation.continue_turn(InferenceOptions::default(), 2)).unwrap();
    let fresh = context(&conversation, &resumed);
    assert_ne!(fresh.turn_id, key.turn_id);
    assert_eq!(
        contexts
            .snapshot_for_tool(&fresh)
            .unwrap()
            .snapshot()
            .unwrap()
            .generation(),
        1
    );
    assert!(contexts.snapshot_for_tool(&key).is_err());
    drop(resumed);
    assert_eq!(active_count(&contexts), 0);
}

#[test]
fn failed_and_dropped_admission_never_leave_a_live_workspace_route() {
    for step in [
        SessionStoreStep::Error(SessionStoreError::new(
            SessionStoreErrorKind::Unavailable,
            "fixture",
            "fixture",
            false,
        )),
        SessionStoreStep::Pending,
    ] {
        let fixture = Fixture::new();
        let contexts = Arc::new(NativeWorkspaceContexts::new());
        let (conversation, store, provider) = setup(
            &fixture,
            &contexts,
            SessionStoreScript {
                saves: Some(vec![step]),
                ..SessionStoreScript::default()
            },
        );
        let calls = store.calls().len();
        drop(conversation.prompt("unpolled".into(), 1));
        assert_eq!(store.calls().len(), calls);
        assert_eq!(active_count(&contexts), 0);
        let mut admission = conversation.prompt("polled".into(), 1);
        let waker = noop_waker();
        let _ = admission.as_mut().poll(&mut Context::from_waker(&waker));
        drop(admission);
        assert_eq!(active_count(&contexts), 0);
        assert!(!conversation.is_busy());
        assert!(provider.requests().is_empty());
    }
}

#[test]
fn registry_is_bounded_weak_and_retirement_cannot_remove_replacement() {
    let contexts = NativeWorkspaceContexts::new();
    let engine = Engine::builder()
        .provider(ScriptedModelProvider::new("test", []))
        .session_store(InMemorySessionStore::new())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let sessions: Vec<_> = (0..64)
        .map(|index| {
            engine
                .create_session(
                    SessionId::new(format!("session-{index}")).unwrap(),
                    SessionIncarnationId::new("incarnation").unwrap(),
                )
                .unwrap()
        })
        .collect();
    let mut owners: Vec<_> = sessions
        .iter()
        .map(|session| contexts.register(session).unwrap())
        .collect();
    assert!(matches!(
        contexts.register(&sessions[0]),
        Err(NativeWorkspaceContextError::Limit)
    ));
    owners[0].retire();
    let replacement = contexts.register(&sessions[0]).unwrap();
    owners[0].retire();
    assert!(matches!(
        contexts.register(&sessions[0]),
        Err(NativeWorkspaceContextError::Limit)
    ));
    drop(replacement);
    assert!(contexts.register(&sessions[0]).is_ok());
    owners.clear();
    assert!(contexts.register(&sessions[1]).is_ok());
    assert!(contexts.routes.lock().unwrap().len() <= 64);
}

struct ScopedTool {
    contexts: Arc<NativeWorkspaceContexts>,
    seen: Arc<Mutex<Vec<u64>>>,
    requires_permission: bool,
}
impl Tool for ScopedTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("scoped").unwrap(),
            description: "fixture".into(),
            input_schema: json!({}),
        }
    }
    fn prepare_for_turn(
        &self,
        context: &ToolContext,
        call: ToolCall,
    ) -> std::result::Result<PreparedToolCall, ToolError> {
        self.seen.lock().unwrap().push(
            self.contexts
                .snapshot_for_tool(context)
                .unwrap()
                .snapshot()
                .unwrap()
                .generation(),
        );
        Ok(if self.requires_permission {
            PreparedToolCall::new(
                Capability::Filesystem {
                    access: FilesystemAccess::Read,
                    path: "file".into(),
                },
                call.arguments,
            )
        } else {
            PreparedToolCall::without_authority(call.arguments)
        })
    }
    fn execute(
        &self,
        context: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(
                self.contexts
                    .snapshot_for_tool(&context)
                    .unwrap()
                    .snapshot()
                    .unwrap()
                    .generation(),
            );
            Ok(ToolOutput::success("done"))
        })
    }
}
struct ScopedPolicy {
    contexts: Arc<NativeWorkspaceContexts>,
    seen: Arc<Mutex<Vec<u64>>>,
}
impl PermissionHandler for ScopedPolicy {
    fn authorize(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, std::result::Result<PermissionDecision, PermissionError>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(
                self.contexts
                    .snapshot_for_permission(&request)
                    .unwrap()
                    .snapshot()
                    .unwrap()
                    .generation(),
            );
            Ok(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })
        })
    }
}

#[test]
fn real_core_preparation_uses_scope_before_and_without_permission_authorization() {
    for requires_permission in [false, true] {
        let fixture = Fixture::new();
        let contexts = Arc::new(NativeWorkspaceContexts::new());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let permissions = Arc::new(Mutex::new(Vec::new()));
        let record = record("session", "incarnation");
        let store =
            InMemorySessionStore::from_records(BTreeMap::from([(record.id.clone(), record)]));
        let provider = ScriptedModelProvider::new(
            "test",
            [
                ModelProviderStep::events([
                    ModelEvent::ToolCall {
                        call: ToolCall {
                            id: ToolCallId::new("call").unwrap(),
                            name: ToolName::new("scoped").unwrap(),
                            arguments: json!({}),
                        },
                    },
                    ModelEvent::Stop {
                        reason: StopReason::ToolCalls,
                    },
                ]),
                finished(),
            ],
        );
        let engine = Engine::builder()
            .provider(provider)
            .session_store(store)
            .permission_handler(ScopedPolicy {
                contexts: contexts.clone(),
                seen: permissions.clone(),
            })
            .tool(ScopedTool {
                contexts: contexts.clone(),
                seen: seen.clone(),
                requires_permission,
            })
            .build()
            .unwrap();
        let session = block_on(engine.load_session(SessionId::new("session").unwrap()))
            .unwrap()
            .unwrap();
        let mut conversation = NativeConversation::from_session(session)
            .unwrap()
            .with_workspace_contexts(fixture.authority.clone(), &contexts)
            .unwrap();
        if requires_permission {
            conversation = conversation
                .with_permission_contexts(&Arc::new(crate::NativePermissionContexts::new()))
                .unwrap();
        }
        let turn = block_on(conversation.prompt("run".into(), 1)).unwrap();
        fixture.install();
        let events = block_on(turn.collect::<Vec<_>>());
        assert!(events.iter().all(std::result::Result::is_ok), "{events:?}");
        assert_eq!(*seen.lock().unwrap(), vec![0, 0]);
        assert_eq!(
            *permissions.lock().unwrap(),
            if requires_permission { vec![0] } else { vec![] }
        );
        assert_eq!(active_count(&contexts), 0);
    }
}
