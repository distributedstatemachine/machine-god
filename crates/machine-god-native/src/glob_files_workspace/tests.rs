use super::super::{
    MAX_GLOB_FILES_RESULT_PATH_BYTES, MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES, join_workspace_path,
};
use super::*;
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{SessionIncarnationId, Tool, ToolCallId, ToolErrorKind};
use std::sync::Arc;

use crate::list_files::enumeration_fixture::{Allow, Fixture, context};

fn tool(fixture: &Fixture) -> GlobFilesTool {
    GlobFilesTool::open(&fixture.primary)
        .unwrap()
        .with_workspace_contexts(fixture.contexts.clone())
}

fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("enumerate").unwrap(),
        name: glob_files_name(),
        arguments,
    }
}

fn run(tool: &GlobFilesTool, key: &ToolContext, arguments: Value) -> Result<ToolOutput, ToolError> {
    let prepared = tool.prepare_for_turn(key, call(arguments))?;
    block_on(tool.execute(
        key.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
}

#[test]
fn real_engine_approves_exact_additional_search_and_qualifies_matches() {
    let fixture = Fixture::new("glob");
    let target = fixture.additional.to_str().unwrap();
    let policy = Arc::new(Allow::default());
    let conversation = fixture.conversation(
        tool(&fixture),
        vec![call(json!({"path":target,"pattern":"*.txt"}))],
        policy.clone(),
    );
    let turn = block_on(conversation.prompt("glob".into(), 1)).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok));
    let output = format!("{events:?}");
    assert!(
        output.contains(fixture.additional.join("additional.txt").to_str().unwrap()),
        "{output}"
    );
    assert!(!output.contains("primary.txt"));
    assert_eq!(
        *policy.0.lock().unwrap(),
        vec![Capability::Filesystem {
            access: FilesystemAccess::EnumerateRecursive,
            path: target.to_owned(),
        }]
    );
}

#[test]
fn defaults_remain_primary_and_slashful_patterns_stay_search_root_relative() {
    let fixture = Fixture::new("glob");
    std::fs::create_dir_all(fixture.additional.join("src/nested")).unwrap();
    std::fs::write(fixture.additional.join("src/nested/result.rs"), "").unwrap();
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("glob".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = tool(&fixture);
    assert_eq!(
        run(&tool, &key, json!({"pattern":"*"})).unwrap(),
        ToolOutput::success(json!({
            "path":".", "pattern":"*", "mode":"matches", "matches":["primary.txt"], "truncated":false,
        }))
    );
    let target = fixture.additional.join("src");
    let output = run(
        &tool,
        &key,
        json!({"path":format!("{}/./", target.display()), "pattern":"nested/*.rs"}),
    )
    .unwrap();
    assert_eq!(
        output,
        ToolOutput::success(json!({
            "path":target, "pattern":"nested/*.rs", "mode":"matches",
            "matches":[target.join("nested/result.rs")], "truncated":false,
        }))
    );
    let count = run(
        &tool,
        &key,
        json!({"path":target, "pattern":"nested/*.rs", "mode":"count"}),
    )
    .unwrap();
    assert_eq!(count.content["count"], 1);
    assert_eq!(count.content["path"], target.to_str().unwrap());
}

#[test]
fn captured_descriptor_survives_replacement_without_retargeting_next_turn() {
    let fixture = Fixture::new("glob");
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("first".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = tool(&fixture);
    let arguments = json!({"path":fixture.additional,"pattern":"*"});
    std::fs::rename(&fixture.additional, fixture.base.join("retained")).unwrap();
    let prepared = tool
        .prepare_for_turn(&key, call(arguments.clone()))
        .unwrap();
    std::fs::create_dir(&fixture.additional).unwrap();
    std::fs::write(fixture.additional.join("replacement.txt"), "").unwrap();
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    let output = block_on(tool.execute(
        key.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(
        output.content["matches"],
        json!([fixture.additional.join("additional.txt")])
    );
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let next = block_on(conversation.prompt("next".into(), 2)).unwrap();
    assert!(run(&tool, &context(&conversation, &next), arguments).is_err());
}

#[test]
fn foreign_stale_cancelled_and_unprepared_calls_do_not_fallback() {
    let fixture = Fixture::new("glob");
    std::os::unix::fs::symlink(&fixture.primary, fixture.additional.join("link")).unwrap();
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("glob".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let tool = tool(&fixture);
    for path in [
        fixture.base.clone(),
        fixture.base.join("state"),
        fixture.additional.join("../primary"),
        fixture.additional.join("link"),
    ] {
        assert!(run(&tool, &key, json!({"path":path,"pattern":"*"})).is_err());
    }
    for path in ["", "x\n", "x\u{202e}"] {
        assert!(
            tool.prepare_for_turn(&key, call(json!({"path":path,"pattern":"*"})))
                .is_err()
        );
    }
    let mut foreign = key.clone();
    foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert!(run(&tool, &foreign, json!({"pattern":"*"})).is_err());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        block_on(tool.execute(
            key.clone(),
            json!({"path":".","pattern":"*","mode":"count"}),
            cancellation
        ))
        .unwrap_err()
        .kind,
        ToolErrorKind::Cancelled
    );
    assert!(
        block_on(tool.execute(
            key.clone(),
            json!({"path":"./","pattern":"*","mode":"count"}),
            CancellationToken::new()
        ))
        .is_err()
    );
    let pending = tool.execute(
        key.clone(),
        json!({"path":fixture.additional,"pattern":"*","mode":"count"}),
        CancellationToken::new(),
    );
    drop(turn);
    assert_eq!(
        block_on(pending).unwrap_err().code,
        "workspace_context_unavailable"
    );
}

#[test]
fn qualified_prefix_is_charged_to_ordered_output_budget_and_count_stays_exact() {
    let fixture = Fixture::new("glob");
    std::fs::remove_file(fixture.additional.join("additional.txt")).unwrap();
    let mut expected = Vec::new();
    for index in 0..100 {
        let path = fixture
            .additional
            .join(format!("{index:03}{}", "x".repeat(247)));
        std::fs::write(&path, "").unwrap();
        expected.push(path.to_str().unwrap().to_owned());
    }
    let conversation = fixture.conversation(tool(&fixture), vec![], Arc::new(Allow::default()));
    let turn = block_on(conversation.prompt("glob".into(), 1)).unwrap();
    let key = context(&conversation, &turn);
    let output = run(
        &tool(&fixture),
        &key,
        json!({"path":fixture.additional,"pattern":"*"}),
    )
    .unwrap();
    let emitted = MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES / expected[0].len();
    assert_eq!(output.content["matches"], json!(expected[..emitted]));
    assert_eq!(output.content["truncated"], true);
    let count = run(
        &tool(&fixture),
        &key,
        json!({"path":fixture.additional,"pattern":"*","mode":"count"}),
    )
    .unwrap();
    assert_eq!(count.content["count"], 100);
}

#[test]
fn qualified_candidate_bound_is_checked_before_matching_and_retention_in_both_modes() {
    let fixture = Fixture::new("glob");
    let tool = tool(&fixture);
    // Synthetic logical prefix isolates the full path limit without requiring
    // the operating system to create a >4KiB pathname. The real scanner and
    // renderer are used, including nonmatches and count mode.
    let suffix = "primary.txt";
    let exact = format!(
        "/{}",
        "x".repeat(MAX_GLOB_FILES_RESULT_PATH_BYTES - suffix.len() - 2)
    );
    for mode in [GlobMode::Matches, GlobMode::Count] {
        let cancellation = CancellationToken::new();
        let root = tool.open_search_root(".", &cancellation).unwrap();
        let results = scan_tree("*", &exact, root, &cancellation).unwrap();
        let output = render_results("*", &exact, mode, results, &cancellation).unwrap();
        if mode == GlobMode::Matches {
            assert_eq!(
                output.content["matches"][0].as_str().unwrap().len(),
                MAX_GLOB_FILES_RESULT_PATH_BYTES
            );
        }
        let over = format!("{exact}x");
        let root = tool.open_search_root(".", &cancellation).unwrap();
        let Err(error) = scan_tree("nonmatching", &over, root, &cancellation) else {
            panic!("overlong qualified candidate must fail before matching");
        };
        assert_eq!(error.code, "glob_files_scan_limit");
    }
    assert_eq!(join_workspace_path("/", "file").unwrap(), "/file");
}
