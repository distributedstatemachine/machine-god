use super::super::{
    MAX_GREP_FILES_TOTAL_RESULT_PATH_BYTES, checked_descendant_path_length,
    checked_workspace_path_length, join_workspace_path,
};
use super::*;
use crate::list_files::enumeration_fixture::{Allow, Fixture, context};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    PermissionRequestId, PermissionRisk, SessionIncarnationId, Tool, ToolCallId, ToolErrorKind,
};
use serde_json::json;
use std::sync::Arc;

fn tool(fixture: &Fixture) -> GrepFilesTool {
    GrepFilesTool::open(&fixture.primary)
        .unwrap()
        .with_workspace_contexts(fixture.contexts.clone())
}
fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("grep").unwrap(),
        name: grep_files_name(),
        arguments,
    }
}
fn run(tool: &GrepFilesTool, key: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
    let prepared = tool.prepare_for_turn(key, call(arguments))?;
    block_on(tool.execute(
        key.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
}

#[test]
fn real_engine_approves_exact_additional_scope_and_returns_qualified_matches() {
    let fixture = Fixture::new("grep-engine");
    let policy = Arc::new(Allow::default());
    let conversation = fixture.conversation(
        tool(&fixture),
        vec![call(json!({
            "path": fixture.additional, "pattern":"additional"
        }))],
        policy.clone(),
    );
    let turn = block_on(conversation.prompt("grep".into(), 1)).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    let output = format!("{events:?}");
    assert!(
        output.contains(fixture.additional.join("additional.txt").to_str().unwrap()),
        "{output}"
    );
    assert!(!output.contains("primary.txt"));
    assert_eq!(
        *policy.0.lock().unwrap(),
        vec![Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: fixture.additional.to_str().unwrap().to_owned(),
        }]
    );
}

#[test]
fn defaults_single_file_and_all_modes_keep_matching_and_pagination_semantics() {
    let fixture = Fixture::new("grep-modes");
    std::fs::create_dir_all(fixture.additional.join("src/nested")).unwrap();
    std::fs::write(
        fixture.additional.join("src/nested/match.rs"),
        "before\nneedle one\nneedle two\nafter\n",
    )
    .unwrap();
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("grep".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = tool(&fixture);
    let primary = run(&tool, &key, json!({"pattern":"primary"})).unwrap();
    assert_eq!(primary.content["path"], ".");
    assert_eq!(primary.content["matches"][0]["path"], "primary.txt");
    let path = fixture.additional.join("src");
    let output = run(&tool, &key, json!({"path":format!("{}/./",path.display()),"pattern":"needle","include":"nested/*.rs","head_limit":1,"offset":1,"context_lines":1})).unwrap();
    assert_eq!(output.content["path"], path.to_str().unwrap());
    assert_eq!(output.content["total_matches"], 2);
    assert_eq!(output.content["matches"][0]["line_number"], 3);
    assert_eq!(
        output.content["matches"][0]["context_before"][0]["line"],
        "needle one"
    );
    assert_eq!(
        output.content["matches"][0]["path"],
        path.join("nested/match.rs").to_str().unwrap()
    );
    assert_eq!(output.content["next_offset"], Value::Null);
    for mode in ["files_with_matches", "count"] {
        let output = run(
            &tool,
            &key,
            json!({"path":path,"pattern":"needle","include":"nested/*.rs","mode":mode}),
        )
        .unwrap();
        assert_eq!(output.content["matching_lines"], 2);
        if mode == "files_with_matches" {
            assert_eq!(
                output.content["files"],
                json!([path.join("nested/match.rs")])
            );
        } else {
            assert_eq!(output.content["matching_files"], 1);
        }
    }
    let file = path.join("nested/match.rs");
    let selected = run(
        &tool,
        &key,
        json!({"path":file,"pattern":"needle","include":"*.rs","mode":"count"}),
    )
    .unwrap();
    assert_eq!(selected.content["matching_lines"], 2);
    let excluded = run(
        &tool,
        &key,
        json!({"path":file,"pattern":"needle","include":"nested/*.rs","mode":"count"}),
    )
    .unwrap();
    assert_eq!(excluded.content["candidate_files"], 0);
}

#[test]
fn permission_validation_preserves_prepared_null_and_exact_scope_capability() {
    let fixture = Fixture::new("grep-permission");
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("grep".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = tool(&fixture);
    let prepared = tool
        .prepare_for_turn(
            &key,
            call(json!({"path":fixture.additional,"pattern":"additional"})),
        )
        .unwrap();
    assert_eq!(prepared.arguments().as_object().unwrap().len(), 8);
    assert_eq!(prepared.arguments()["include"], Value::Null);
    let mut request = PermissionRequest {
        id: PermissionRequestId::new("request").unwrap(),
        session_id: key.session_id.clone(),
        session_incarnation_id: key.session_incarnation_id.clone(),
        turn_id: key.turn_id.clone(),
        capability: Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: fixture.additional.to_str().unwrap().to_owned(),
        },
        risk: PermissionRisk::Low,
        reason: "search".into(),
    };
    let name = grep_files_name();
    let validate = |request: &PermissionRequest, args: &Value| {
        tool.validate_permission_preparation(
            request,
            PermissionInvocation {
                tool_name: &name,
                call_id: &key.call_id,
                arguments: args,
            },
        )
    };
    validate(&request, prepared.arguments()).unwrap();
    for field in ["include", "mode", "path"] {
        let mut forged = prepared.arguments().clone();
        forged.as_object_mut().unwrap().remove(field);
        assert!(validate(&request, &forged).is_err());
    }
    let mut forged = prepared.arguments().clone();
    forged["path"] = json!(format!("{}/./", fixture.additional.display()));
    assert!(validate(&request, &forged).is_err());
    let real = request.capability.clone();
    request.capability = Capability::Filesystem {
        access: FilesystemAccess::SearchContent,
        path: ".".into(),
    };
    assert!(validate(&request, prepared.arguments()).is_err());
    request.capability = real;
    request.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert_eq!(
        validate(&request, prepared.arguments()).unwrap_err().code,
        "workspace_context_unavailable"
    );
}

#[test]
fn retained_scope_survives_root_replacement_and_does_not_follow_new_root_set() {
    let fixture = Fixture::new("grep-retained");
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("first".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = tool(&fixture);
    let args = json!({"path":fixture.additional,"pattern":"additional"});
    let prepared = tool.prepare_for_turn(&key, call(args.clone())).unwrap();
    let pending = tool.execute(
        key.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    );
    std::fs::rename(&fixture.additional, fixture.base.join("retained")).unwrap();
    std::fs::create_dir(&fixture.additional).unwrap();
    std::fs::write(fixture.additional.join("replacement.txt"), "additional").unwrap();
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    let output = block_on(pending).unwrap();
    assert_eq!(
        output.content["matches"][0]["path"],
        fixture.additional.join("additional.txt").to_str().unwrap()
    );
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let next = block_on(conversation.prompt("next".into(), 2)).unwrap();
    assert!(run(&tool, &context(&conversation, &next), args).is_err());
}

#[test]
fn pure_preparation_and_missing_foreign_cancelled_expired_scope_never_fallback() {
    let fixture = Fixture::new("grep-scopes");
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("grep".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = tool(&fixture);
    let missing = fixture.additional.join("absent/file");
    assert!(
        tool.prepare_for_turn(&key, call(json!({"path":missing,"pattern":"x"})))
            .is_ok()
    );
    std::os::unix::fs::symlink(&fixture.primary, fixture.additional.join("link")).unwrap();
    for path in [
        fixture.base.clone(),
        fixture.base.join("state"),
        fixture.additional.join("../primary"),
        fixture.additional.join("link"),
    ] {
        assert!(run(&tool, &key, json!({"path":path,"pattern":"primary"})).is_err());
    }
    for path in ["", "x\n", "x\u{202e}"] {
        assert!(
            tool.prepare_for_turn(&key, call(json!({"path":path,"pattern":"x"})))
                .is_err()
        );
    }
    let prepared = tool
        .prepare_for_turn(&key, call(json!({"pattern":"primary"})))
        .unwrap();
    let mut foreign = key.clone();
    foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert!(
        block_on(tool.execute(
            foreign,
            prepared.arguments().clone(),
            CancellationToken::new()
        ))
        .is_err()
    );
    let missing_tool = GrepFilesTool::open(&fixture.primary)
        .unwrap()
        .with_workspace_contexts(Arc::new(NativeWorkspaceContexts::new()));
    assert!(
        block_on(missing_tool.execute(
            key.clone(),
            prepared.arguments().clone(),
            CancellationToken::new()
        ))
        .is_err()
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        block_on(tool.execute(key.clone(), prepared.arguments().clone(), cancelled))
            .unwrap_err()
            .kind,
        ToolErrorKind::Cancelled
    );
    let old = tool.execute(key, prepared.arguments().clone(), CancellationToken::new());
    drop(turn);
    assert_eq!(
        block_on(old).unwrap_err().code,
        "workspace_context_unavailable"
    );
}

#[test]
fn qualified_paths_consume_original_output_limits_and_root_slash_is_canonical() {
    let fixture = Fixture::new("grep-bounds");
    std::fs::remove_file(fixture.additional.join("additional.txt")).unwrap();
    let mut paths = Vec::new();
    for index in 0..100 {
        let path = fixture
            .additional
            .join(format!("{index:03}{}", "x".repeat(240)));
        std::fs::write(&path, "needle").unwrap();
        paths.push(path);
    }
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("grep".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let output = run(
        &tool(&fixture),
        &key,
        json!({"path":fixture.additional,"pattern":"needle","mode":"files_with_matches"}),
    )
    .unwrap();
    let emitted = MAX_GREP_FILES_TOTAL_RESULT_PATH_BYTES / paths[0].to_str().unwrap().len();
    assert_eq!(output.content["files"], json!(paths[..emitted]));
    assert_eq!(output.content["total_files"], 100);
    assert_eq!(output.content["next_offset"], emitted);
    assert_eq!(output.content["truncated"], true);
    let count = run(
        &tool(&fixture),
        &key,
        json!({"path":fixture.additional,"pattern":"needle","mode":"count"}),
    )
    .unwrap();
    assert_eq!(count.content["matching_files"], 100);
    assert_eq!(join_workspace_path("/", "child").unwrap(), "/child");
    assert_eq!(checked_workspace_path_length("/", 5).unwrap(), 6);
    assert!(
        checked_descendant_path_length(&"a".repeat(MAX_GREP_FILES_PATH_BYTES), "", "x").is_err()
    );
}

#[test]
fn scope_revocation_interrupts_real_scan_at_existing_checkpoint() {
    use std::cell::Cell;
    struct Revoke<'a> {
        checks: Cell<usize>,
        turn: machine_god_core::TurnHandle,
        check: ScopedCheck<'a>,
    }
    impl ScanCheck for Revoke<'_> {
        fn check(&self) -> Result<(), ToolError> {
            self.checks.set(self.checks.get() + 1);
            if self.checks.get() == 20 {
                assert!(self.turn.cancel());
            }
            self.check.check()
        }
    }
    let fixture = Fixture::new("grep-revoke");
    std::fs::write(fixture.additional.join("long.txt"), "needle\n".repeat(1000)).unwrap();
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("grep".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let scope = fixture.contexts.snapshot_for_tool(&key).unwrap();
    let cancellation = CancellationToken::new();
    let check = Revoke {
        checks: Cell::new(0),
        turn: turn.handle(),
        check: ScopedCheck {
            scope: &scope,
            cancellation: &cancellation,
        },
    };
    let arguments = decode_execution_arguments(
        tool(&fixture)
            .prepare_for_turn(
                &key,
                call(json!({"path":fixture.additional,"pattern":"needle"})),
            )
            .unwrap()
            .arguments()
            .clone(),
    )
    .unwrap();
    let (route, relative) = canonical_route(&scope, &arguments).unwrap();
    let tool = GrepFilesTool::from_root_descriptor(route.root_descriptor().try_clone().unwrap());
    let result = tool.execute_unix_at(&arguments, &relative, &check);
    assert_eq!(result.unwrap_err().code, "workspace_context_unavailable");
    assert_eq!(check.checks.get(), 20);
    assert!(!cancellation.is_cancelled());
}
