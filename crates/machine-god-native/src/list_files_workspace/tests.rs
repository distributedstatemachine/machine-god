use super::super::MAX_LIST_FILES_TOTAL_NAME_BYTES;
use super::*;
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{SessionIncarnationId, Tool, ToolCallId, ToolErrorKind};
use std::sync::Arc;

use crate::list_files::enumeration_fixture::{Allow, Fixture, context};

fn tool(fixture: &Fixture) -> ListFilesTool {
    ListFilesTool::open(&fixture.primary)
        .unwrap()
        .with_workspace_contexts(fixture.contexts.clone())
}

fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("enumerate").unwrap(),
        name: list_files_name(),
        arguments,
    }
}

fn run(tool: &ListFilesTool, key: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
    let prepared = tool.prepare_for_turn(key, call(arguments))?;
    block_on(tool.execute(
        key.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
}

#[test]
fn real_engine_approves_and_reports_exact_additional_directory() {
    let fixture = Fixture::new("list");
    let target = fixture.additional.to_str().unwrap();
    let policy = Arc::new(Allow::default());
    let conversation = fixture.conversation(
        tool(&fixture),
        vec![call(json!({"path":target}))],
        policy.clone(),
    );
    let turn = block_on(conversation.prompt("list".into(), 1)).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok));
    let output = format!("{events:?}");
    assert!(output.contains("additional.txt"), "{output}");
    assert!(!output.contains("primary.txt"));
    assert_eq!(
        *policy.0.lock().unwrap(),
        vec![Capability::Filesystem {
            access: FilesystemAccess::Enumerate,
            path: target.to_owned(),
        }]
    );
}

#[test]
fn defaults_are_primary_and_absolute_directory_returns_logical_identity() {
    let fixture = Fixture::new("list");
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("list".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = tool(&fixture);
    assert_eq!(
        run(&tool, &key, json!({})).unwrap(),
        ToolOutput::success(json!({
            "path":".", "entries":[{"name":"primary.txt","kind":"file"}], "truncated":false,
        }))
    );
    let target = format!("{}/./", fixture.additional.display());
    assert_eq!(
        run(&tool, &key, json!({"path":target})).unwrap(),
        ToolOutput::success(json!({
            "path":fixture.additional, "entries":[{"name":"additional.txt","kind":"file"}], "truncated":false,
        }))
    );
}

#[test]
fn captured_descriptor_survives_path_replacement_and_new_turn_loses_removed_root() {
    let fixture = Fixture::new("list");
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("first".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = tool(&fixture);
    let arguments = json!({"path":fixture.additional});
    std::fs::rename(&fixture.additional, fixture.base.join("retained")).unwrap();
    let prepared = tool
        .prepare_for_turn(&key, call(arguments.clone()))
        .unwrap();
    std::fs::create_dir(&fixture.additional).unwrap();
    std::fs::write(fixture.additional.join("replacement.txt"), "replacement").unwrap();
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    // Pure preparation succeeds without consulting the replaced pathname.
    let output = block_on(tool.execute(
        key.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(output.content["entries"][0]["name"], "additional.txt");
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let next = block_on(conversation.prompt("next".into(), 2)).unwrap();
    assert!(run(&tool, &context(&conversation, &next), arguments).is_err());
}

#[test]
fn exact_context_and_strict_paths_are_required_and_unpolled_future_is_inert() {
    let fixture = Fixture::new("list");
    std::os::unix::fs::symlink(&fixture.primary, fixture.additional.join("link")).unwrap();
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("list".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = tool(&fixture);
    for path in [
        fixture.base.clone(),
        fixture.base.join("state"),
        fixture.additional.join("../primary"),
        fixture.additional.join("link"),
    ] {
        assert!(run(&tool, &key, json!({"path":path})).is_err());
    }
    for path in ["", "x\n", "x\u{202e}"] {
        assert!(
            tool.prepare_for_turn(&key, call(json!({"path":path})))
                .is_err()
        );
    }
    let mut foreign = key.clone();
    foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert!(run(&tool, &foreign, json!({})).is_err());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        block_on(tool.execute(key.clone(), json!({"path":"."}), cancellation))
            .unwrap_err()
            .kind,
        ToolErrorKind::Cancelled
    );
    let pending = tool.execute(
        key.clone(),
        json!({"path":fixture.additional}),
        CancellationToken::new(),
    );
    drop(turn);
    assert_eq!(
        block_on(pending).unwrap_err().code,
        "workspace_context_unavailable"
    );
    assert!(run(&tool, &key, json!({})).is_err());
}

#[test]
fn qualified_directory_keeps_original_entry_name_budget() {
    let fixture = Fixture::new("list");
    std::fs::remove_file(fixture.additional.join("additional.txt")).unwrap();
    for index in 0..100 {
        std::fs::write(
            fixture
                .additional
                .join(format!("{index:03}{}", "x".repeat(247))),
            "",
        )
        .unwrap();
    }
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("list".into(), 1)).unwrap();
    let output = run(
        &tool(&fixture),
        &context(&conversation, &turn),
        json!({"path":fixture.additional}),
    )
    .unwrap();
    let entries = output.content["entries"].as_array().unwrap();
    assert_eq!(entries.len(), MAX_LIST_FILES_TOTAL_NAME_BYTES / 250);
    assert_eq!(output.content["truncated"], true);
    assert_eq!(output.content["path"], fixture.additional.to_str().unwrap());
}
