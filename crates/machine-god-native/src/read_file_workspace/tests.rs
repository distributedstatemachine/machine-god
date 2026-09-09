use super::*;
use crate::{
    NATIVE_SESSION_METADATA_KEY, NativeConversation, NativeConversationTurn, NativeSessionMetadata,
    NativeWorkspaceAuthority, NativeWorkspaceEntrySpec, NativeWorkspaceSource,
};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, Engine, ModelEvent, PermissionDecision, PermissionError, PermissionGrantScope,
    PermissionHandler, PermissionInvocation, PermissionRequest, PermissionRequestId,
    PermissionRisk, SessionId, SessionIncarnationId, SessionRecord, SessionRevision, StopReason,
    Tool, ToolCallId,
};
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};
use rustix::fs::{Mode, OFlags};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: PathBuf,
    primary: PathBuf,
    additional: PathBuf,
    authority: NativeWorkspaceAuthority,
    contexts: Arc<NativeWorkspaceContexts>,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "machine-god-workspace-read-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let primary = base.join("primary");
        let additional = base.join("additional");
        let state = base.join("state");
        for path in [&primary, &additional, &state] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(primary.join("same"), "primary content").unwrap();
        std::fs::write(additional.join("same"), "additional content").unwrap();
        let open = |path: &Path| {
            rustix::fs::open(
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .unwrap()
        };
        let authority = NativeWorkspaceAuthority::open_blocking(
            open(&primary),
            primary.clone(),
            Some(open(&state)),
            state,
            vec![
                NativeWorkspaceEntrySpec::new(
                    NativeWorkspaceSource::new(additional.clone(), additional.clone(), true)
                        .unwrap(),
                    true,
                    false,
                )
                .unwrap(),
            ],
            false,
        )
        .unwrap();
        Self {
            base,
            primary,
            additional,
            authority,
            contexts: Arc::new(NativeWorkspaceContexts::new()),
        }
    }

    fn tool(&self) -> ReadFileTool {
        ReadFileTool::open(&self.primary)
            .unwrap()
            .with_workspace_contexts(self.contexts.clone())
    }

    fn conversation(
        &self,
        steps: Vec<ModelProviderStep>,
        policy: Arc<Allow>,
    ) -> NativeConversation {
        let mut record = SessionRecord::empty(
            SessionId::new("session").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        );
        record.revision = SessionRevision(1);
        record.metadata.insert(
            NATIVE_SESSION_METADATA_KEY.to_owned(),
            NativeSessionMetadata::default().to_value(),
        );
        let engine = Engine::builder()
            .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
                record.id.clone(),
                record,
            )])))
            .provider(ScriptedModelProvider::new("test", steps))
            .shared_permission_handler(policy)
            .tool(self.tool())
            .build()
            .unwrap();
        let session = block_on(engine.load_session(SessionId::new("session").unwrap()))
            .unwrap()
            .unwrap();
        NativeConversation::from_session(session)
            .unwrap()
            .with_workspace_contexts(self.authority.clone(), &self.contexts)
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}

#[derive(Default)]
struct Allow(Mutex<Vec<Capability>>);
impl PermissionHandler for Allow {
    fn authorize(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request.capability);
            Ok(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })
        })
    }
}

fn finished() -> ModelProviderStep {
    ModelProviderStep::events([ModelEvent::Stop {
        reason: StopReason::Completed,
    }])
}

fn context(conversation: &NativeConversation, turn: &NativeConversationTurn) -> ToolContext {
    ToolContext {
        session_id: conversation.id(),
        session_incarnation_id: conversation.incarnation_id(),
        turn_id: turn.handle().id().clone(),
        call_id: ToolCallId::new("read").unwrap(),
    }
}

fn call(path: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("read").unwrap(),
        name: read_file_name(),
        arguments: json!({"path": path}),
    }
}

fn read(tool: &ReadFileTool, context: &ToolContext, path: &str) -> Result<ToolOutput, ToolError> {
    let prepared = tool.prepare_for_turn(context, call(path))?;
    block_on(tool.execute(
        context.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
}

#[test]
fn real_engine_approves_qualified_path_and_reads_the_additional_root() {
    let fixture = Fixture::new();
    let target = fixture.additional.join("same").to_str().unwrap().to_owned();
    let policy = Arc::new(Allow::default());
    let conversation = fixture.conversation(
        vec![
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: call(&target),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            finished(),
        ],
        policy.clone(),
    );
    let turn = block_on(conversation.prompt("read the other root".into(), 1)).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok));
    let rendered = format!("{events:?}");
    assert!(rendered.contains("additional content"), "{rendered}");
    assert!(!rendered.contains("primary content"));
    assert_eq!(
        *policy.0.lock().unwrap(),
        vec![Capability::Filesystem {
            access: FilesystemAccess::Read,
            path: target
        }]
    );
}

#[test]
fn primary_relative_and_additional_absolute_reads_keep_distinct_identities() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation(vec![finished()], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("read".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = fixture.tool();
    assert_eq!(
        read(&tool, &key, "./same").unwrap(),
        ToolOutput::success(json!({"content":"primary content"}))
    );
    let target = fixture.additional.join("same");
    assert_eq!(
        read(&tool, &key, target.to_str().unwrap()).unwrap(),
        ToolOutput::success(json!({"content":"additional content"}))
    );
    drop(turn);
    assert!(read(&tool, &key, "same").is_err());
}

#[test]
fn captured_root_survives_rename_and_new_turn_observes_removed_scope() {
    let fixture = Fixture::new();
    let conversation =
        fixture.conversation(vec![finished(), finished()], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("first".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = fixture.tool();
    let target = fixture.additional.join("same");
    std::fs::rename(&fixture.additional, fixture.base.join("retained")).unwrap();
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    // Preparation cannot reopen the now-absent pathname; execution uses old FD.
    assert_eq!(
        read(&tool, &key, target.to_str().unwrap()).unwrap(),
        ToolOutput::success(json!({"content":"additional content"}))
    );
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let next = block_on(conversation.prompt("next".into(), 2)).unwrap();
    assert!(
        read(
            &tool,
            &context(&conversation, &next),
            target.to_str().unwrap()
        )
        .is_err()
    );
}

#[test]
fn scope_does_not_allow_foreign_contexts_traversal_symlinks_or_cancelled_reads() {
    let fixture = Fixture::new();
    std::os::unix::fs::symlink(
        fixture.primary.join("same"),
        fixture.additional.join("link"),
    )
    .unwrap();
    let conversation = fixture.conversation(vec![finished()], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("read".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = fixture.tool();
    for path in [
        fixture.base.join("outside"),
        fixture.additional.join("../primary/same"),
        fixture.additional.join("link"),
        fixture.base.join("state/file"),
    ] {
        assert!(read(&tool, &key, path.to_str().unwrap()).is_err());
    }
    let mut foreign = key.clone();
    foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert!(read(&tool, &foreign, "same").is_err());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = block_on(tool.execute(key, json!({"path":"same"}), cancellation)).unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
}

fn targets(
    fixture: &Fixture,
    context: &ToolContext,
    path: &str,
) -> Result<crate::NativePreparedPermissionTargets, PermissionError> {
    let tool = Arc::new(fixture.tool());
    let authority = crate::NativePermissionTargetAuthority::new(
        std::fs::File::open(&fixture.primary).unwrap(),
        fixture.primary.to_str().unwrap().to_owned(),
        vec![crate::NativePermissionTargetTool::Ordinary(tool.clone())],
    )
    .unwrap()
    .with_workspace_contexts(fixture.contexts.clone());
    let call = call(path);
    let prepared = tool
        .prepare_for_turn(context, call.clone())
        .map_err(|_| PermissionError::new("invalid", "invalid"))?;
    let request = PermissionRequest {
        id: PermissionRequestId::new("permission").unwrap(),
        session_id: context.session_id.clone(),
        session_incarnation_id: context.session_incarnation_id.clone(),
        turn_id: context.turn_id.clone(),
        capability: prepared.capability().unwrap().clone(),
        risk: PermissionRisk::Critical,
        reason: "untrusted hint".into(),
    };
    block_on(authority.prepare(
        &request,
        PermissionInvocation {
            tool_name: &call.name,
            call_id: &call.id,
            arguments: prepared.arguments(),
        },
        CancellationToken::new(),
    ))
}

#[test]
fn native_permission_observations_retain_each_selected_root_and_expire_with_turn() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation(vec![finished()], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("read".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let target = fixture.additional.join("same");
    let evidence = targets(&fixture, &key, target.to_str().unwrap()).unwrap();
    assert_eq!(evidence.targets()[0].path(), target.to_str().unwrap());
    assert!(evidence.revalidate().is_ok());
    let retained = fixture.base.join("retained");
    std::fs::rename(&fixture.additional, &retained).unwrap();
    assert!(evidence.revalidate().is_ok());
    let after_rename = targets(&fixture, &key, target.to_str().unwrap()).unwrap();
    assert!(after_rename.revalidate().is_ok());
    std::fs::rename(retained.join("same"), retained.join("original")).unwrap();
    std::fs::write(retained.join("same"), "replacement").unwrap();
    assert!(evidence.revalidate().is_err());
    assert!(after_rename.revalidate().is_err());
    let primary_evidence = targets(&fixture, &key, "same").unwrap();
    drop(turn);
    assert!(primary_evidence.revalidate().is_err());
}

#[test]
fn native_permission_routing_rejects_missing_scope_instead_of_primary_fallback() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation(vec![finished()], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("read".into(), 1)).unwrap();
    let mut key = context(&conversation, &turn);
    key.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert!(targets(&fixture, &key, "same").is_err());
}
