use super::*;
use crate::{
    CreateFolderTool, FileInfoTool, NATIVE_SESSION_METADATA_KEY, NativeConversation,
    NativeConversationTurn, NativeSessionMetadata, NativeWorkspaceAuthority,
    NativeWorkspaceEntrySpec, NativeWorkspaceSource,
};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, Engine, FilesystemAccess, ModelEvent,
    PermissionDecision, PermissionError, PermissionGrantScope, PermissionHandler,
    PermissionRequest, SessionId, SessionIncarnationId, SessionRecord, SessionRevision, StopReason,
    Tool, ToolCall, ToolCallId, ToolName, ToolOutput,
};
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};
use rustix::fs::{Mode, OFlags};
use serde_json::json;
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
            "mg-workspace-path-tools-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let primary = base.join("primary");
        let additional = base.join("additional");
        let state = base.join("state");
        for path in [&primary, &additional, &state] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(primary.join("same"), "primary").unwrap();
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
    fn info(&self) -> FileInfoTool {
        FileInfoTool::open(&self.primary)
            .unwrap()
            .with_workspace_contexts(self.contexts.clone())
    }
    fn create(&self) -> CreateFolderTool {
        CreateFolderTool::open(&self.primary)
            .unwrap()
            .with_workspace_contexts(self.contexts.clone())
    }
    fn conversation(
        &self,
        steps: Vec<ModelProviderStep>,
        permission: Option<Arc<Allow>>,
    ) -> NativeConversation {
        let mut record = SessionRecord::empty(
            SessionId::new("session").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        );
        record.revision = SessionRevision(1);
        record.metadata.insert(
            NATIVE_SESSION_METADATA_KEY.into(),
            NativeSessionMetadata::default().to_value(),
        );
        let mut builder = Engine::builder()
            .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
                record.id.clone(),
                record,
            )])))
            .provider(ScriptedModelProvider::new("test", steps))
            .tool(self.info())
            .tool(self.create());
        // Core requires a handler, but no native permission controller/context
        // composition is involved in these direct exact-turn lookup tests.
        builder = builder.shared_permission_handler(permission.unwrap_or_default());
        let engine = builder.build().unwrap();
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
        call_id: ToolCallId::new("path").unwrap(),
    }
}
fn call(name: &str, path: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(name).unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments: json!({"path":path}),
    }
}
fn execute(tool: &impl Tool, context: &ToolContext, path: &str) -> Result<ToolOutput, ToolError> {
    let prepared = tool.prepare_for_turn(context, call(tool.spec().name.as_str(), path))?;
    block_on(tool.execute(
        context.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
}

#[test]
fn actual_engine_approves_qualified_metadata_and_create_targets() {
    let fixture = Fixture::new();
    let file = fixture.additional.join("same").to_str().unwrap().to_owned();
    let folder = fixture
        .additional
        .join("new/nested")
        .to_str()
        .unwrap()
        .to_owned();
    let permission = Arc::new(Allow::default());
    let conversation = fixture.conversation(
        vec![
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: call("file_info", &file),
                },
                ModelEvent::ToolCall {
                    call: call("create_folder", &folder),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            finished(),
        ],
        Some(permission.clone()),
    );
    let events = block_on(
        block_on(conversation.prompt("inspect and create".into(), 1))
            .unwrap()
            .collect::<Vec<_>>(),
    );
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    assert!(fixture.additional.join("new/nested").is_dir());
    assert!(!fixture.primary.join("new").exists());
    assert_eq!(
        *permission.0.lock().unwrap(),
        vec![
            Capability::Filesystem {
                access: FilesystemAccess::Metadata,
                path: file
            },
            Capability::Filesystem {
                access: FilesystemAccess::Create,
                path: folder
            }
        ]
    );
}

#[test]
fn paths_and_results_remain_distinct_without_permission_composition() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation(vec![finished()], None);
    let turn = block_on(conversation.prompt("paths".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    assert_eq!(
        execute(&fixture.info(), &key, "./same").unwrap().content["size_bytes"],
        7
    );
    let path = fixture.additional.join("same");
    let info = execute(&fixture.info(), &key, path.to_str().unwrap()).unwrap();
    assert_eq!(info.content["size_bytes"], 18);
    assert_eq!(info.content["path"], path.to_str().unwrap());
    let root = execute(&fixture.info(), &key, fixture.additional.to_str().unwrap()).unwrap();
    assert_eq!(root.content["path"], fixture.additional.to_str().unwrap());
    assert_eq!(root.content["kind"], "directory");
    let target = fixture.additional.join("new/folder");
    assert_eq!(
        execute(&fixture.create(), &key, target.to_str().unwrap())
            .unwrap()
            .content["path"],
        target.to_str().unwrap()
    );
    assert!(fixture.additional.join("new/folder").is_dir());
    execute(&fixture.create(), &key, "./local//folder/").unwrap();
    assert!(fixture.primary.join("local/folder").is_dir());
}

#[test]
fn old_turn_retains_renamed_descriptor_and_next_turn_observes_removal() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation(vec![finished(), finished()], None);
    let turn = block_on(conversation.prompt("first".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let moved = fixture.base.join("retained");
    std::fs::rename(&fixture.additional, &moved).unwrap();
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    let target = fixture.additional.join("created");
    execute(&fixture.create(), &key, target.to_str().unwrap()).unwrap();
    assert!(moved.join("created").is_dir());
    assert!(!target.exists());
    assert_eq!(
        execute(
            &fixture.info(),
            &key,
            fixture.additional.join("same").to_str().unwrap()
        )
        .unwrap()
        .content["size_bytes"],
        18
    );
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let next = block_on(conversation.prompt("next".into(), 2)).unwrap();
    let key = context(&conversation, &next);
    assert!(execute(&fixture.create(), &key, target.to_str().unwrap()).is_err());
    assert!(
        execute(
            &fixture.info(),
            &key,
            fixture.additional.join("same").to_str().unwrap()
        )
        .is_err()
    );
}

#[test]
fn foreign_expired_cancelled_and_unpolled_calls_have_no_fallback_or_effects() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation(vec![finished()], None);
    let turn = block_on(conversation.prompt("first".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = fixture.create();
    let target = fixture.additional.join("new");
    let prepared = tool
        .prepare_for_turn(&key, call("create_folder", target.to_str().unwrap()))
        .unwrap();
    assert!(!target.exists());
    let pending = tool.execute(
        key.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    );
    drop(pending);
    assert!(!target.exists());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        block_on(tool.execute(key.clone(), prepared.arguments().clone(), cancelled))
            .unwrap_err()
            .kind,
        machine_god_core::ToolErrorKind::Cancelled
    );
    let mut foreign = key.clone();
    foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert!(execute(&fixture.info(), &foreign, "same").is_err());
    assert!(execute(&tool, &foreign, "new").is_err());
    drop(turn);
    assert!(
        block_on(tool.execute(
            key.clone(),
            prepared.arguments().clone(),
            CancellationToken::new()
        ))
        .is_err()
    );
    assert!(execute(&fixture.info(), &key, "same").is_err());
    assert!(!target.exists());
    assert!(!fixture.primary.join("new").exists());
}

#[test]
fn traversal_state_and_ancestor_symlinks_are_rejected_but_final_metadata_is_not_followed() {
    let fixture = Fixture::new();
    std::os::unix::fs::symlink(&fixture.primary, fixture.additional.join("link")).unwrap();
    let conversation = fixture.conversation(vec![finished()], None);
    let turn = block_on(conversation.prompt("paths".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    for path in [
        fixture.base.join("outside"),
        fixture.base.join("state/new"),
        fixture.additional.join("../primary/new"),
        fixture.additional.join("link/new"),
    ] {
        assert!(execute(&fixture.create(), &key, path.to_str().unwrap()).is_err());
        assert!(execute(&fixture.info(), &key, path.to_str().unwrap()).is_err());
    }
    let link = execute(
        &fixture.info(),
        &key,
        fixture.additional.join("link").to_str().unwrap(),
    )
    .unwrap();
    assert_eq!(link.content["kind"], "symlink");
    assert!(!fixture.primary.join("new").exists());
}

#[test]
fn absolute_root_spelling_cannot_bypass_tool_character_or_byte_limits() {
    let fixture = Fixture::new();
    let root = fixture.base.join("forbidden\nroot");
    std::fs::create_dir(&root).unwrap();
    fixture
        .authority
        .install(
            fixture
                .authority
                .prepare_blocking(
                    vec![
                        NativeWorkspaceEntrySpec::new(
                            NativeWorkspaceSource::new(root.clone(), root.clone(), true).unwrap(),
                            true,
                            false,
                        )
                        .unwrap(),
                    ],
                    false,
                )
                .unwrap(),
        )
        .unwrap();
    let conversation = fixture.conversation(vec![finished()], None);
    let turn = block_on(conversation.prompt("paths".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    for (name, tool) in [
        ("file_info", Box::new(fixture.info()) as Box<dyn Tool>),
        ("create_folder", Box::new(fixture.create()) as Box<dyn Tool>),
    ] {
        for path in [
            root.join("child").to_str().unwrap().to_owned(),
            "x".repeat(4097),
            "nul\0path".into(),
        ] {
            assert!(tool.prepare_for_turn(&key, call(name, &path)).is_err());
        }
    }
    assert!(!root.join("child").exists());
}

#[test]
fn scope_expiry_at_first_mkdir_prevents_effect_but_committed_completion_is_retained() {
    use crate::create_folder::{CreateFolderCheckpoint, CreateFolderEvidence};
    struct Evidence {
        scope: crate::NativeWorkspaceTurnScope,
        turn: Option<NativeConversationTurn>,
        expire_after_commit: bool,
    }
    impl CreateFolderEvidence for Evidence {
        fn checkpoint(&mut self, checkpoint: CreateFolderCheckpoint, _: &CancellationToken) {
            let expire = if self.expire_after_commit {
                checkpoint == CreateFolderCheckpoint::AfterCommit
            } else {
                matches!(checkpoint, CreateFolderCheckpoint::BeforeMkdir(_, _))
            };
            if expire {
                self.turn.take();
            }
        }
        fn check_authority(&self) -> Result<(), ToolError> {
            ensure_live(&self.scope)
        }
    }
    for expire_after_commit in [false, true] {
        let fixture = Fixture::new();
        let conversation = fixture.conversation(vec![finished()], None);
        let turn = block_on(conversation.prompt("create".into(), 1)).unwrap();
        let key = context(&conversation, &turn);
        let projection = project(
            &fixture.contexts,
            &key,
            fixture.additional.join("new/nested").to_str().unwrap(),
            crate::MAX_CREATE_FOLDER_PATH_BYTES,
            |path| Ok(path.to_owned()),
            unavailable,
        )
        .unwrap();
        let root = projection.route.root_descriptor().try_clone().unwrap();
        let mut evidence = Evidence {
            scope: projection.scope,
            turn: Some(turn),
            expire_after_commit,
        };
        let result = CreateFolderTool::from_root_descriptor(root).execute_supported_with_evidence(
            &projection.relative,
            &CancellationToken::new(),
            &mut evidence,
        );
        if expire_after_commit {
            assert!(result.is_ok());
            assert!(fixture.additional.join("new/nested").is_dir());
        } else {
            assert_eq!(result.unwrap_err().code, "workspace_context_unavailable");
            assert!(!fixture.additional.join("new").exists());
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn scoped_open_preserves_custom_launcher_logical_identity_and_exact_descriptor() {
    use crate::{
        OpenFileLaunch, OpenFileLaunchOutcome, OpenFileLaunchRequest, OpenFileLauncher,
        OpenFileTool,
    };
    struct Launcher(Arc<Mutex<Vec<(String, String)>>>);
    impl OpenFileLauncher for Launcher {
        fn launch(
            &self,
            request: OpenFileLaunchRequest,
            cancellation: CancellationToken,
        ) -> OpenFileLaunch {
            let received = self.0.clone();
            Box::pin(async move {
                if cancellation.is_cancelled() || !request.workspace_is_live() {
                    return OpenFileLaunchOutcome::Cancelled;
                }
                received.lock().unwrap().push((
                    request.path().to_owned(),
                    std::fs::read_to_string(request.proc_path()).unwrap(),
                ));
                OpenFileLaunchOutcome::Accepted
            })
        }
    }
    let fixture = Fixture::new();
    let conversation = fixture.conversation(vec![finished()], None);
    let turn = block_on(conversation.prompt("open".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let received = Arc::new(Mutex::new(Vec::new()));
    let tool = OpenFileTool::open_with_launcher(&fixture.primary, Launcher(received.clone()))
        .unwrap()
        .with_workspace_contexts(fixture.contexts.clone());
    let target = fixture.additional.join("same");
    let prepared = tool
        .prepare_for_turn(&key, call("open_file", target.to_str().unwrap()))
        .unwrap();
    assert_eq!(
        prepared.capability(),
        Some(&Capability::OpenFile {
            path: target.to_str().unwrap().to_owned()
        })
    );
    drop(tool.execute(
        key.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ));
    assert!(received.lock().unwrap().is_empty());
    std::fs::rename(&fixture.additional, fixture.base.join("retained")).unwrap();
    let output = block_on(tool.execute(
        key.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(output.content["path"], target.to_str().unwrap());
    assert_eq!(
        *received.lock().unwrap(),
        vec![(
            target.to_str().unwrap().to_owned(),
            "additional content".to_owned()
        )]
    );
    drop(turn);
    assert!(
        block_on(tool.execute(key, prepared.arguments().clone(), CancellationToken::new()))
            .is_err()
    );
    assert_eq!(received.lock().unwrap().len(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn scoped_open_rejects_noncanonical_spelling_before_custom_launcher() {
    use crate::{OpenFileLaunch, OpenFileLaunchRequest, OpenFileLauncher, OpenFileTool};
    struct Never;
    impl OpenFileLauncher for Never {
        fn launch(&self, _: OpenFileLaunchRequest, _: CancellationToken) -> OpenFileLaunch {
            panic!("must not launch")
        }
    }
    let fixture = Fixture::new();
    let conversation = fixture.conversation(vec![finished()], None);
    let turn = block_on(conversation.prompt("open".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = OpenFileTool::open_with_launcher(&fixture.primary, Never)
        .unwrap()
        .with_workspace_contexts(fixture.contexts.clone());
    for path in [
        "./same".to_owned(),
        "same/".to_owned(),
        format!("{}/./same", fixture.additional.display()),
        fixture
            .additional
            .join("../primary/same")
            .to_str()
            .unwrap()
            .to_owned(),
        "x".repeat(4097),
    ] {
        assert!(
            tool.prepare_for_turn(&key, call("open_file", &path))
                .is_err()
        );
    }
}
