use super::*;
use crate::NativeFileApprovalPolicy;
use crate::{
    NATIVE_SESSION_METADATA_KEY, NativeConversation, NativeConversationTurn, NativeSessionMetadata,
    NativeWorkspaceAuthority, NativeWorkspaceEntrySpec, NativeWorkspaceSource,
};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    Engine, ModelEvent, SessionId, SessionIncarnationId, SessionRecord, SessionRevision,
    StopReason, ToolCallId, ToolName,
};
use machine_god_core::{
    PermissionError, PermissionExecutionAdmission, PermissionInvocation, PermissionRequest,
    PermissionRequestId, PermissionRisk,
};
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

struct Policy {
    calls: std::sync::atomic::AtomicUsize,
    cancel: Option<machine_god_core::TurnHandle>,
}

struct Allow;
impl machine_god_core::PermissionHandler for Allow {
    fn authorize(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<machine_god_core::PermissionDecision, PermissionError>> {
        Box::pin(async {
            Ok(machine_god_core::PermissionDecision::Allow {
                scope: machine_god_core::PermissionGrantScope::Once,
            })
        })
    }
}
impl NativeFileApprovalPolicy for Policy {
    fn revalidate(&self) -> Result<(), PermissionError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 2
            && let Some(turn) = &self.cancel
        {
            assert!(turn.cancel());
        }
        Ok(())
    }
}

fn admit(
    fixture: &Fixture,
    kind: Kind,
    registry: &Arc<NativeFileApprovalRegistry>,
    context: &ToolContext,
    args: &Value,
    cancel: Option<machine_god_core::TurnHandle>,
) {
    let scope = fixture.contexts.snapshot_for_tool(context).unwrap();
    let projection = project(&scope.snapshot().unwrap(), kind, args).unwrap();
    let request = PermissionRequest {
        id: PermissionRequestId::new("request").unwrap(),
        session_id: context.session_id.clone(),
        session_incarnation_id: context.session_incarnation_id.clone(),
        turn_id: context.turn_id.clone(),
        capability: projection.capability(),
        risk: PermissionRisk::High,
        reason: "test".into(),
    };
    let tool_name = ToolName::new(name(kind)).unwrap();
    let prepared = block_on(
        registry.prepare_endpoints(
            projection
                .source()
                .map(WorkspaceMutationEndpoint::retain)
                .transpose()
                .unwrap(),
            projection.target().retain().unwrap(),
            &request,
            PermissionInvocation {
                tool_name: &tool_name,
                call_id: &context.call_id,
                arguments: args,
            },
            CancellationToken::new(),
        ),
    )
    .unwrap();
    Box::new(prepared.admit(Arc::new(Policy {
        calls: std::sync::atomic::AtomicUsize::new(0),
        cancel,
    })))
    .admit()
    .unwrap();
}

pub(crate) struct Fixture {
    pub(crate) base: PathBuf,
    pub(crate) primary: PathBuf,
    pub(crate) additional: PathBuf,
    pub(crate) authority: NativeWorkspaceAuthority,
    pub(crate) contexts: Arc<NativeWorkspaceContexts>,
    pub(crate) undo: Arc<FileUndoTracker>,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "mg-workspace-mutation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let primary = base.join("primary");
        let additional = base.join("additional");
        let state = base.join("state");
        for path in [&primary, &additional, &state] {
            fs::create_dir(path).unwrap();
        }
        fs::write(primary.join("a"), b"before").unwrap();
        fs::write(additional.join("a"), b"before").unwrap();
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
            undo: Arc::new(FileUndoTracker::new()),
        }
    }
    pub(crate) fn tool(
        &self,
        kind: Kind,
        registry: Option<Arc<NativeFileApprovalRegistry>>,
    ) -> WorkspaceMutationTool {
        let primary: Arc<dyn Tool> = match kind {
            Kind::Write => Arc::new(crate::WriteFileTool::open(&self.primary).unwrap()),
            Kind::Edit => Arc::new(crate::EditFileTool::open(&self.primary).unwrap()),
            Kind::Delete => Arc::new(crate::DeleteFileTool::open(&self.primary).unwrap()),
            Kind::Copy => Arc::new(crate::CopyFileTool::open(&self.primary).unwrap()),
            Kind::Rename => Arc::new(crate::RenameFileTool::open(&self.primary).unwrap()),
        };
        WorkspaceMutationTool::new(
            kind,
            primary,
            self.contexts.clone(),
            registry,
            Some(self.undo.clone()),
        )
    }
    pub(crate) fn conversation(&self) -> NativeConversation {
        self.conversation_with_tool(None)
    }
    fn conversation_with_tool(&self, kind: Option<Kind>) -> NativeConversation {
        let mut record = SessionRecord::empty(
            SessionId::new("session").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        );
        record.revision = SessionRevision(1);
        record.metadata.insert(
            NATIVE_SESSION_METADATA_KEY.into(),
            NativeSessionMetadata::default().to_value(),
        );
        let mut steps = Vec::new();
        if let Some(kind) = kind {
            steps.push(ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: call(kind, self.args(kind)),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]));
        }
        steps.push(ModelProviderStep::events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }]));
        let mut builder = Engine::builder()
            .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
                record.id.clone(),
                record,
            )])))
            .provider(ScriptedModelProvider::new("test", steps))
            .permission_handler(Allow);
        if let Some(kind) = kind {
            builder = builder.tool(self.tool(kind, None));
        }
        let engine = builder.build().unwrap();
        let session = block_on(engine.load_session(SessionId::new("session").unwrap()))
            .unwrap()
            .unwrap();
        NativeConversation::from_session(session)
            .unwrap()
            .with_workspace_contexts(self.authority.clone(), &self.contexts)
            .unwrap()
    }
    pub(crate) fn args(&self, kind: Kind) -> Value {
        let path = self.additional.join("a").to_str().unwrap().to_owned();
        let destination = self.additional.join("b").to_str().unwrap().to_owned();
        match kind {
            Kind::Write => json!({"path":path,"content":"after"}),
            Kind::Edit => json!({"path":path,"old_string":"before","new_string":"after"}),
            Kind::Delete => json!({"path":path}),
            Kind::Copy => json!({"source":"a","destination":destination}),
            Kind::Rename => json!({"old_path":"a","new_path":destination}),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

pub(crate) fn context(
    conversation: &NativeConversation,
    turn: &NativeConversationTurn,
) -> ToolContext {
    ToolContext {
        session_id: conversation.id(),
        session_incarnation_id: conversation.incarnation_id(),
        turn_id: turn.handle().id().clone(),
        call_id: ToolCallId::new("mutation").unwrap(),
    }
}
fn call(kind: Kind, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("mutation").unwrap(),
        name: ToolName::new(name(kind)).unwrap(),
        arguments,
    }
}

const KINDS: [Kind; 5] = [
    Kind::Write,
    Kind::Edit,
    Kind::Delete,
    Kind::Copy,
    Kind::Rename,
];

#[test]
fn all_five_tools_route_actual_effects_and_logical_undo_receipts() {
    for kind in KINDS {
        let fixture = Fixture::new();
        let conversation = fixture.conversation();
        let turn = block_on(conversation.prompt("test".into(), 1)).unwrap();
        let context = context(&conversation, &turn);
        let tool = fixture.tool(kind, None);
        let args = fixture.args(kind);
        let prepared = tool
            .prepare_for_turn(&context, call(kind, args.clone()))
            .unwrap();
        assert_eq!(prepared.arguments(), &args);
        let output = block_on(tool.execute(context, args, CancellationToken::new())).unwrap();
        let label = if matches!(kind, Kind::Copy | Kind::Rename) {
            fixture.additional.join("b")
        } else {
            fixture.additional.join("a")
        };
        let field = match kind {
            Kind::Copy => "destination",
            Kind::Rename => "new_path",
            _ => "path",
        };
        assert_eq!(output.content[field].as_str(), label.to_str());
        if kind == Kind::Delete {
            assert!(!label.exists());
        } else {
            assert_eq!(
                fs::read(&label).unwrap(),
                if matches!(kind, Kind::Copy | Kind::Rename) {
                    b"before".as_slice()
                } else {
                    b"after".as_slice()
                }
            );
        }
        let outcome = fixture.undo.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(
            outcome,
            match kind {
                Kind::Copy => crate::FileUndoOutcome::Removed(label.to_str().unwrap().into()),
                Kind::Rename => crate::FileUndoOutcome::Restored("a".into()),
                _ => crate::FileUndoOutcome::Restored(label.to_str().unwrap().into()),
            }
        );
        assert_eq!(fs::read(fixture.primary.join("a")).unwrap(), b"before");
        assert_eq!(fs::read(fixture.additional.join("a")).unwrap(), b"before");
    }
}

#[test]
fn real_engine_routes_all_five_tools_through_exact_workspace_context() {
    for kind in KINDS {
        let fixture = Fixture::new();
        let conversation = fixture.conversation_with_tool(Some(kind));
        let turn = block_on(conversation.prompt("execute routed mutation".into(), 1)).unwrap();
        let events = block_on(turn.collect::<Vec<_>>());
        assert!(events.iter().all(Result::is_ok), "{events:?}");
        let target = fixture
            .additional
            .join(if matches!(kind, Kind::Copy | Kind::Rename) {
                "b"
            } else {
                "a"
            });
        if kind == Kind::Delete {
            assert!(!target.exists());
        } else {
            assert_eq!(
                fs::read(target).unwrap(),
                if matches!(kind, Kind::Copy | Kind::Rename) {
                    b"before".as_slice()
                } else {
                    b"after".as_slice()
                }
            );
        }
        assert_ne!(
            fixture.undo.undo_last(&CancellationToken::new()).unwrap(),
            crate::FileUndoOutcome::Empty
        );
        assert_eq!(fs::read(fixture.primary.join("a")).unwrap(), b"before");
        assert_eq!(fs::read(fixture.additional.join("a")).unwrap(), b"before");
    }
}

#[test]
fn missing_foreign_and_expired_contexts_never_fall_back_to_primary() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation();
    let turn = block_on(conversation.prompt("test".into(), 1)).unwrap();
    let exact = context(&conversation, &turn);
    let tool = fixture.tool(Kind::Delete, None);
    let future = tool.execute(exact.clone(), json!({"path":"a"}), CancellationToken::new());
    drop(turn);
    assert!(block_on(future).is_err());
    for context in [
        exact.clone(),
        ToolContext {
            turn_id: machine_god_core::TurnId::new("foreign").unwrap(),
            ..exact
        },
    ] {
        assert!(
            tool.prepare_for_turn(&context, call(Kind::Delete, json!({"path":"a"})))
                .is_err()
        );
        assert!(
            block_on(tool.execute(context, json!({"path":"a"}), CancellationToken::new())).is_err()
        );
    }
    assert_eq!(fs::read(fixture.primary.join("a")).unwrap(), b"before");
}

#[test]
fn projection_is_pure_bounded_and_preserves_canonical_logical_arguments() {
    let fixture = Fixture::new();
    let scope = fixture.authority.snapshot().unwrap();
    let raw = format!("{}/./missing//child", fixture.additional.display());
    let projection = project(
        &scope,
        Kind::Write,
        &json!({"path":raw,"content":"unchanged"}),
    )
    .unwrap();
    assert_eq!(
        projection.private_arguments(),
        &json!({"path":"missing/child","content":"unchanged"})
    );
    assert_eq!(
        projection.logical_arguments()["path"],
        fixture.additional.join("missing/child").to_str().unwrap()
    );
    assert!(!fixture.additional.join("missing").exists());
    for args in [
        json!({"path":"../escape","content":"x"}),
        json!({"path":".","content":"x"}),
        json!({"path":"a","content":"x","extra":"x"}),
        json!({"path":"a","content":"x".repeat(crate::MAX_WRITE_FILE_CONTENT_BYTES + 1)}),
    ] {
        assert!(project(&scope, Kind::Write, &args).is_err());
    }
}

#[test]
fn all_five_wrappers_consume_exact_owned_endpoint_approvals() {
    for kind in KINDS {
        let fixture = Fixture::new();
        let conversation = fixture.conversation();
        let turn = block_on(conversation.prompt("approved".into(), 1)).unwrap();
        let context = context(&conversation, &turn);
        let registry = Arc::new(NativeFileApprovalRegistry::new());
        let tool = fixture.tool(kind, Some(registry.clone()));
        let args = fixture.args(kind);
        admit(&fixture, kind, &registry, &context, &args, None);
        block_on(tool.execute(context.clone(), args.clone(), CancellationToken::new())).unwrap();
        assert!(block_on(tool.execute(context, args, CancellationToken::new())).is_err());
    }
}

#[test]
fn old_unpolled_outer_future_cannot_take_later_approval_with_reused_call_id() {
    for has_old_grant in [false, true] {
        let fixture = Fixture::new();
        let conversation = fixture.conversation();
        let turn = block_on(conversation.prompt("approved".into(), 1)).unwrap();
        let context = context(&conversation, &turn);
        let registry = Arc::new(NativeFileApprovalRegistry::new());
        let tool = fixture.tool(Kind::Write, Some(registry.clone()));
        let args = fixture.args(Kind::Write);
        if has_old_grant {
            admit(&fixture, Kind::Write, &registry, &context, &args, None);
        }
        let old = tool.execute(context.clone(), args.clone(), CancellationToken::new());
        if has_old_grant {
            block_on(tool.execute(context.clone(), args.clone(), CancellationToken::new()))
                .unwrap();
        }
        admit(&fixture, Kind::Write, &registry, &context, &args, None);
        assert!(block_on(old).is_err());
        block_on(tool.execute(context, args, CancellationToken::new())).unwrap();
        assert_eq!(fs::read(fixture.additional.join("a")).unwrap(), b"after");
    }
}

#[test]
fn turn_entry_point_with_history_cannot_take_a_later_approval() {
    for has_old_grant in [false, true] {
        let fixture = Fixture::new();
        let observations = Arc::new(crate::NativeConversationObservations::new());
        let conversation = fixture.conversation();
        let turn = block_on(conversation.prompt("approved".into(), 1)).unwrap();
        let context = context(&conversation, &turn);
        let observation_session = observations
            .register(
                context.session_id.clone(),
                context.session_incarnation_id.clone(),
            )
            .unwrap();
        observation_session
            .begin_attempt(context.turn_id.clone(), 0, 1)
            .unwrap();
        observation_session
            .bind_call(
                &context,
                crate::NativeHistoryFileSource::new(
                    1,
                    0,
                    context.call_id.clone(),
                    ToolName::new("write_file").unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        let registry = Arc::new(NativeFileApprovalRegistry::new());
        let tool = fixture
            .tool(Kind::Write, Some(registry.clone()))
            .with_observations(Some(observations));
        let args = fixture.args(Kind::Write);
        if has_old_grant {
            admit(&fixture, Kind::Write, &registry, &context, &args, None);
        }
        let old = tool.execute_for_turn(context.clone(), args.clone(), CancellationToken::new());
        if has_old_grant {
            block_on(tool.execute_for_turn(
                context.clone(),
                args.clone(),
                CancellationToken::new(),
            ))
            .unwrap();
        }
        admit(&fixture, Kind::Write, &registry, &context, &args, None);
        let error = block_on(old).unwrap_err();
        assert_eq!(error.code, "file_approval_failed");
        // A second observation for an already completed call ID is deliberately
        // rejected. The no-old-grant case proves the new grant is still usable.
        if !has_old_grant {
            block_on(tool.execute_for_turn(context, args, CancellationToken::new())).unwrap();
        }
        assert_eq!(fs::read(fixture.additional.join("a")).unwrap(), b"after");
    }
}

#[test]
fn cancellation_during_final_approval_check_blocks_all_five_publications() {
    for kind in KINDS {
        let fixture = Fixture::new();
        let conversation = fixture.conversation();
        let turn = block_on(conversation.prompt("cancel".into(), 1)).unwrap();
        let context = context(&conversation, &turn);
        let registry = Arc::new(NativeFileApprovalRegistry::new());
        let tool = fixture.tool(kind, Some(registry.clone()));
        let args = fixture.args(kind);
        admit(
            &fixture,
            kind,
            &registry,
            &context,
            &args,
            Some(turn.handle()),
        );
        assert_eq!(
            block_on(tool.execute(context, args, CancellationToken::new()))
                .unwrap_err()
                .code,
            "workspace_context_unavailable"
        );
        assert_eq!(
            fixture.undo.undo_last(&CancellationToken::new()).unwrap(),
            crate::FileUndoOutcome::Empty
        );
        assert_eq!(fs::read(fixture.primary.join("a")).unwrap(), b"before");
        assert_eq!(fs::read(fixture.additional.join("a")).unwrap(), b"before");
        assert_eq!(fs::read_dir(&fixture.additional).unwrap().count(), 1);
    }
}

#[test]
fn all_five_native_final_guards_work_without_an_approval_registry() {
    for kind in KINDS {
        let fixture = Fixture::new();
        let conversation = fixture.conversation();
        let turn = block_on(conversation.prompt("drop".into(), 1)).unwrap();
        let context = context(&conversation, &turn);
        let scope = Arc::new(fixture.contexts.snapshot_for_tool(&context).unwrap());
        let args = fixture.args(kind);
        let projection = project(&scope.snapshot().unwrap(), kind, &args).unwrap();
        let source = projection
            .source()
            .map(WorkspaceMutationEndpoint::retain)
            .transpose()
            .unwrap();
        let target = projection.target().retain().unwrap();
        macro_rules! bound {
            ($tool:expr) => {
                Box::new(
                    $tool
                        .with_workspace_scope(scope.clone())
                        .with_undo_tracker(fixture.undo.clone()),
                ) as Box<dyn Tool>
            };
        }
        let tool = match kind {
            Kind::Write => bound!(crate::WriteFileTool::from_endpoint(target)),
            Kind::Edit => bound!(crate::EditFileTool::from_endpoint(target)),
            Kind::Delete => bound!(crate::DeleteFileTool::from_endpoint(target)),
            Kind::Copy => bound!(crate::CopyFileTool::from_endpoints(source.unwrap(), target)),
            Kind::Rename => bound!(crate::RenameFileTool::from_endpoints(
                source.unwrap(),
                target
            )),
        };
        let future = tool.execute(context, args, CancellationToken::new());
        drop(turn);
        assert_eq!(
            block_on(future).unwrap_err().code,
            "workspace_context_unavailable"
        );
        assert_eq!(
            fixture.undo.undo_last(&CancellationToken::new()).unwrap(),
            crate::FileUndoOutcome::Empty
        );
        assert_eq!(fs::read(fixture.primary.join("a")).unwrap(), b"before");
        assert_eq!(fs::read(fixture.additional.join("a")).unwrap(), b"before");
        assert_eq!(fs::read_dir(&fixture.additional).unwrap().count(), 1);
    }
}
