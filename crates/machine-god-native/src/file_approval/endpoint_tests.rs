use super::tests::{Fixture, Policy};
use super::*;
use futures_executor::block_on;
use machine_god_core::{PermissionRisk, Tool, ToolCall};
use serde_json::json;
use std::fs;

fn endpoint(fixture: &Fixture, private: &str, logical: &str) -> NativeFileEndpoint {
    NativeFileEndpoint::new(
        Arc::new(File::open(&fixture.path).unwrap()),
        private.into(),
        logical.into(),
    )
    .unwrap()
}

fn pair_tool(
    kind: NativeFileApprovalKind,
    source: NativeFileEndpoint,
    target: NativeFileEndpoint,
    tracker: Arc<crate::FileUndoTracker>,
    registry: Arc<NativeFileApprovalRegistry>,
) -> Box<dyn Tool> {
    match kind {
        NativeFileApprovalKind::Copy => Box::new(
            crate::CopyFileTool::from_endpoints(source, target)
                .with_undo_tracker(tracker)
                .with_file_approvals(registry),
        ),
        NativeFileApprovalKind::Rename => Box::new(
            crate::RenameFileTool::from_endpoints(source, target)
                .with_undo_tracker(tracker)
                .with_file_approvals(registry),
        ),
        _ => unreachable!(),
    }
}

fn pair_args(kind: NativeFileApprovalKind) -> Value {
    match kind {
        NativeFileApprovalKind::Copy => json!({"source":"/first/a","destination":"/second/a"}),
        NativeFileApprovalKind::Rename => json!({"old_path":"/first/a","new_path":"/second/a"}),
        _ => unreachable!(),
    }
}

fn prepare(
    registry: &Arc<NativeFileApprovalRegistry>,
    tool: &dyn Tool,
    source: Option<NativeFileEndpoint>,
    target: NativeFileEndpoint,
    args: &Value,
) -> PreparedFileApproval {
    let context = Fixture::context();
    let name = tool.spec().name;
    let capability = tool
        .prepare(ToolCall {
            id: context.call_id.clone(),
            name: name.clone(),
            arguments: args.clone(),
        })
        .unwrap()
        .capability()
        .unwrap()
        .clone();
    let request = PermissionRequest {
        id: PermissionRequestId::new("request").unwrap(),
        session_id: context.session_id,
        session_incarnation_id: context.session_incarnation_id,
        turn_id: context.turn_id,
        capability,
        risk: PermissionRisk::High,
        reason: "bounded test".into(),
    };
    block_on(registry.prepare_endpoints(
        source,
        target,
        &request,
        PermissionInvocation {
            tool_name: &name,
            call_id: &context.call_id,
            arguments: args,
        },
        CancellationToken::new(),
    ))
    .unwrap()
}

#[test]
fn cross_root_same_basename_approval_effect_and_undo_preserve_logical_endpoints() {
    for kind in [NativeFileApprovalKind::Copy, NativeFileApprovalKind::Rename] {
        let first = Fixture::new();
        let second = Fixture::new();
        first.seed();
        let source = endpoint(&first, "a", "/first/a");
        let target = endpoint(&second, "a", "/second/a");
        let tracker = Arc::new(crate::FileUndoTracker::new());
        let tool = pair_tool(
            kind,
            source.clone(),
            target.clone(),
            tracker.clone(),
            first.registry.clone(),
        );
        let args = pair_args(kind);
        let prepared = prepare(&first.registry, tool.as_ref(), Some(source), target, &args);
        assert_eq!(prepared.source_path(), Some("/first/a"));
        assert_eq!(prepared.target_path(), "/second/a");
        assert_eq!(prepared.source_preimage(), Some(b"before".as_slice()));
        Box::new(prepared.admit(Policy::new(usize::MAX)))
            .admit()
            .unwrap();
        let output =
            block_on(tool.execute(Fixture::context(), args, CancellationToken::new())).unwrap();
        let output = format!("{output:?}");
        assert!(output.contains("/first/a") && output.contains("/second/a"));
        assert_eq!(fs::read(second.path.join("a")).unwrap(), b"before");
        assert_eq!(
            first.path.join("a").exists(),
            kind == NativeFileApprovalKind::Copy
        );
        let outcome = tracker.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(
            outcome,
            if kind == NativeFileApprovalKind::Copy {
                crate::FileUndoOutcome::Removed("/second/a".into())
            } else {
                crate::FileUndoOutcome::Restored("/first/a".into())
            }
        );
        assert_eq!(fs::read(first.path.join("a")).unwrap(), b"before");
        assert!(!second.path.join("a").exists());
    }
}

#[test]
fn cross_root_approval_rejects_either_rebound_root_and_source_replacement() {
    for kind in [NativeFileApprovalKind::Copy, NativeFileApprovalKind::Rename] {
        for fault in 0..3 {
            let first = Fixture::new();
            let second = Fixture::new();
            let decoy = Fixture::new();
            first.seed();
            decoy.seed();
            let source = endpoint(&first, "a", "/first/a");
            let target = endpoint(&second, "a", "/second/a");
            let tracker = Arc::new(crate::FileUndoTracker::new());
            let original = pair_tool(
                kind,
                source.clone(),
                target.clone(),
                tracker.clone(),
                first.registry.clone(),
            );
            let args = pair_args(kind);
            Box::new(
                prepare(
                    &first.registry,
                    original.as_ref(),
                    Some(source.clone()),
                    target.clone(),
                    &args,
                )
                .admit(Policy::new(usize::MAX)),
            )
            .admit()
            .unwrap();
            let (source, target) = match fault {
                0 => (endpoint(&decoy, "a", "/first/a"), target),
                1 => (source, endpoint(&decoy, "missing", "/second/a")),
                _ => {
                    fs::rename(first.path.join("a"), first.path.join("retained")).unwrap();
                    first.seed();
                    (source, target)
                }
            };
            let tool = pair_tool(
                kind,
                source,
                target,
                tracker.clone(),
                first.registry.clone(),
            );
            assert!(
                block_on(tool.execute(Fixture::context(), args, CancellationToken::new())).is_err()
            );
            assert_eq!(fs::read(first.path.join("a")).unwrap(), b"before");
            assert!(!second.path.join("a").exists());
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()).unwrap(),
                crate::FileUndoOutcome::Empty
            );
        }
    }
}

#[test]
fn endpoint_constructor_and_routed_prepare_are_inert_and_strict() {
    let fixture = Fixture::new();
    let file = fixture.path.join("ordinary-file");
    fs::write(&file, b"inert").unwrap();
    let root = Arc::new(File::open(&file).unwrap()); // Deliberately not a directory.
    let source = NativeFileEndpoint::new(root.clone(), "a".into(), "/first/a".into()).unwrap();
    let target = NativeFileEndpoint::new(root.clone(), "a".into(), "/second/a".into()).unwrap();
    let tool = crate::CopyFileTool::from_endpoints(source, target);
    let args = pair_args(NativeFileApprovalKind::Copy);
    assert!(
        tool.prepare(ToolCall {
            id: Fixture::context().call_id,
            name: tool.spec().name,
            arguments: args.clone()
        })
        .is_ok()
    );
    drop(tool.execute(Fixture::context(), args, CancellationToken::new()));
    assert_eq!(fs::read(file).unwrap(), b"inert");
    for path in ["", "../a", "a/../b", "/absolute", "a//b", "a/./b", "a\0b"] {
        assert!(NativeFileEndpoint::new(root.clone(), path.into(), "/logical".into()).is_err());
    }
}

#[test]
fn cross_root_undo_rejects_destination_same_bytes_replacement() {
    let first = Fixture::new();
    let second = Fixture::new();
    first.seed();
    let tracker = Arc::new(crate::FileUndoTracker::new());
    let tool = crate::RenameFileTool::from_endpoints(
        endpoint(&first, "a", "/first/a"),
        endpoint(&second, "a", "/second/a"),
    )
    .with_undo_tracker(tracker.clone());
    block_on(tool.execute(
        Fixture::context(),
        pair_args(NativeFileApprovalKind::Rename),
        CancellationToken::new(),
    ))
    .unwrap();
    fs::rename(second.path.join("a"), second.path.join("retained")).unwrap();
    second.seed();
    assert_eq!(
        tracker.undo_last(&CancellationToken::new()).unwrap_err(),
        crate::FileUndoError::Changed
    );
    assert!(!first.path.join("a").exists());
    assert_eq!(fs::read(second.path.join("a")).unwrap(), b"before");
}

fn admit_single_endpoint(fixture: &Fixture, kind: NativeFileApprovalKind) -> Value {
    admit_single_endpoint_path(fixture, kind, "/qualified/a")
}

fn admit_single_endpoint_path(
    fixture: &Fixture,
    kind: NativeFileApprovalKind,
    logical_path: &str,
) -> Value {
    let private = super::tests::arguments(kind);
    let mut logical = private.clone();
    logical["path"] = logical_path.into();
    let name = fixture.tool(kind).spec().name;
    let context = Fixture::context();
    let access = match kind {
        NativeFileApprovalKind::Write => FilesystemAccess::Write,
        NativeFileApprovalKind::Edit => FilesystemAccess::Edit,
        NativeFileApprovalKind::Delete => FilesystemAccess::Delete,
        _ => unreachable!(),
    };
    let request = PermissionRequest {
        id: PermissionRequestId::new("request").unwrap(),
        session_id: context.session_id,
        session_incarnation_id: context.session_incarnation_id,
        turn_id: context.turn_id,
        capability: Capability::Filesystem {
            path: logical_path.into(),
            access,
        },
        risk: PermissionRisk::High,
        reason: "bounded test".into(),
    };
    let prepared = block_on(fixture.registry.prepare_endpoints(
        None,
        endpoint(fixture, "a", logical_path),
        &request,
        PermissionInvocation {
            tool_name: &name,
            call_id: &context.call_id,
            arguments: &logical,
        },
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(prepared.target_path(), logical_path);
    Box::new(prepared.admit(Policy::new(usize::MAX)))
        .admit()
        .unwrap();
    private
}

#[test]
fn single_endpoint_private_execution_alias_is_exact_and_root_context_bound() {
    for kind in [
        NativeFileApprovalKind::Write,
        NativeFileApprovalKind::Edit,
        NativeFileApprovalKind::Delete,
    ] {
        let fixture = Fixture::new();
        fixture.seed();
        let private = admit_single_endpoint(&fixture, kind);
        block_on(
            fixture
                .tool(kind)
                .execute(Fixture::context(), private, CancellationToken::new()),
        )
        .unwrap();
        if kind == NativeFileApprovalKind::Delete {
            assert!(!fixture.path.join("a").exists());
        } else {
            assert_eq!(fs::read(fixture.path.join("a")).unwrap(), b"after");
        }
    }
    for fault in 0..3 {
        let fixture = Fixture::new();
        let decoy = Fixture::new();
        fixture.seed();
        decoy.seed();
        let mut private = admit_single_endpoint(&fixture, NativeFileApprovalKind::Write);
        let mut context = Fixture::context();
        let tool = if fault == 1 {
            crate::WriteFileTool::open(&decoy.path)
                .unwrap()
                .with_file_approvals(fixture.registry.clone())
        } else {
            crate::WriteFileTool::open(&fixture.path)
                .unwrap()
                .with_file_approvals(fixture.registry.clone())
        };
        if fault == 0 {
            private["path"] = "other".into();
        }
        if fault == 2 {
            context.turn_id = TurnId::new("other-turn").unwrap();
        }
        assert!(block_on(tool.execute(context, private, CancellationToken::new())).is_err());
        assert_eq!(fs::read(fixture.path.join("a")).unwrap(), b"before");
        assert_eq!(fs::read(decoy.path.join("a")).unwrap(), b"before");
        assert!(!fixture.path.join("other").exists());
    }
}

#[test]
fn relative_logical_label_is_never_a_legacy_execution_alias() {
    let fixture = Fixture::new();
    fixture.seed();
    fs::write(fixture.path.join("display-only"), b"not approved").unwrap();
    admit_single_endpoint_path(&fixture, NativeFileApprovalKind::Delete, "display-only");
    assert!(
        block_on(fixture.tool(NativeFileApprovalKind::Delete).execute(
            Fixture::context(),
            json!({"path":"display-only"}),
            CancellationToken::new()
        ))
        .is_err()
    );
    assert_eq!(fs::read(fixture.path.join("a")).unwrap(), b"before");
    assert_eq!(
        fs::read(fixture.path.join("display-only")).unwrap(),
        b"not approved"
    );
}

#[test]
fn cross_root_rename_undo_revalidates_each_retained_parent() {
    for replace_source in [true, false] {
        let first = Fixture::new();
        let second = Fixture::new();
        fs::create_dir(first.path.join("nested")).unwrap();
        fs::create_dir(second.path.join("nested")).unwrap();
        fs::write(first.path.join("nested/a"), b"before").unwrap();
        let tracker = Arc::new(crate::FileUndoTracker::new());
        let tool = crate::RenameFileTool::from_endpoints(
            endpoint(&first, "nested/a", "/first/a"),
            endpoint(&second, "nested/a", "/second/a"),
        )
        .with_undo_tracker(tracker.clone());
        block_on(tool.execute(
            Fixture::context(),
            pair_args(NativeFileApprovalKind::Rename),
            CancellationToken::new(),
        ))
        .unwrap();
        let replaced = if replace_source { &first } else { &second };
        fs::rename(
            replaced.path.join("nested"),
            replaced.path.join("retained-parent"),
        )
        .unwrap();
        fs::create_dir(replaced.path.join("nested")).unwrap();
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()).unwrap_err(),
            crate::FileUndoError::Changed
        );
        assert!(!first.path.join("nested/a").exists());
        let retained = second.path.join(if replace_source {
            "nested/a"
        } else {
            "retained-parent/a"
        });
        assert_eq!(fs::read(retained).unwrap(), b"before");
    }
}
