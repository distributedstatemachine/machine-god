#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::error::Error;
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    CancellationToken, Capability, FilesystemAccess, SessionId, SessionIncarnationId, Tool,
    ToolCall, ToolCallId, ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    MAX_SEMANTIC_SEARCH_DEPTH, MAX_SEMANTIC_SEARCH_FILE_BYTES, MAX_SEMANTIC_SEARCH_KEYWORDS,
    MAX_SEMANTIC_SEARCH_MATCH_STEPS, MAX_SEMANTIC_SEARCH_PATH_BYTES,
    MAX_SEMANTIC_SEARCH_QUERY_BYTES, MAX_SEMANTIC_SEARCH_RESULT_LINE_BYTES,
    MAX_SEMANTIC_SEARCH_RESULT_PATH_BYTES, MAX_SEMANTIC_SEARCH_RETAINED_RESULTS,
    MAX_SEMANTIC_SEARCH_SERIALIZED_RESULT_BYTES, MAX_SEMANTIC_SEARCH_SHOWN_RESULTS,
    MAX_SEMANTIC_SEARCH_TOTAL_CONTENT_BYTES, MAX_SEMANTIC_SEARCH_TOTAL_ENTRY_NAME_BYTES,
    MAX_SEMANTIC_SEARCH_TOTAL_RESULT_LINE_BYTES, MAX_SEMANTIC_SEARCH_TOTAL_RESULT_PATH_BYTES,
    MAX_SEMANTIC_SEARCH_VISITED_ENTRIES, SEMANTIC_SEARCH_TOOL_NAME, SemanticSearchTool,
    SemanticSearchToolOpenError, SemanticSearchToolOpenErrorKind,
};
use serde_json::{Value, json};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-semantic-search-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create temporary directory: {error}"),
            }
        }
        panic!("failed to allocate a unique temporary directory");
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.path);
        if std::thread::panicking() {
            return;
        }
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove temporary directory: {error}"),
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(Pin::as_mut(&mut future), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("semantic_search unexpectedly returned a pending future"),
    }
}

fn tool(root: &Path) -> SemanticSearchTool {
    SemanticSearchTool::open(root).expect("temporary root is valid")
}

fn call(arguments: Value) -> ToolCall {
    named_call(SEMANTIC_SEARCH_TOOL_NAME, arguments)
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("semantic-search-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("semantic-search-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("semantic-search-incarnation").unwrap(),
        turn_id: TurnId::new("semantic-search-turn").unwrap(),
        call_id: ToolCallId::new("semantic-search-call").unwrap(),
    }
}

fn canonical(query: &str, path: &str) -> Value {
    json!({"query": query, "path": path})
}

fn execute(
    tool: &SemanticSearchTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_ready(tool.execute(context(), arguments, cancellation))
}

fn run(tool: &SemanticSearchTool, query: &str, path: &str) -> ToolOutput {
    execute(tool, canonical(query, path), CancellationToken::new()).unwrap()
}

fn assert_tool_error(
    error: ToolError,
    kind: ToolErrorKind,
    code: &str,
    message: &str,
    retryable: bool,
) {
    let display = error.to_string();
    let debug = format!("{error:?}");
    let ToolError {
        kind: actual_kind,
        code: actual_code,
        message: actual_message,
        retryable: actual_retryable,
    } = error;
    assert_eq!(actual_kind, kind);
    assert_eq!(actual_code, code);
    assert_eq!(actual_message, message);
    assert_eq!(actual_retryable, retryable);
    assert_eq!(display, format!("{code}: {message}"));
    assert_eq!(
        debug,
        format!(
            "ToolError {{ kind: {kind:?}, code: \"{code}\", message: \"{message}\", retryable: {retryable} }}"
        )
    );
}

fn assert_invalid_arguments(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "semantic_search_invalid_arguments",
        "semantic_search arguments are invalid",
        false,
    );
}

fn assert_invalid_query(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "semantic_search_invalid_query",
        "semantic_search query is invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "semantic_search_invalid_path",
        "semantic_search path is invalid",
        false,
    );
}

fn result_paths(output: &ToolOutput) -> Vec<&str> {
    output.content["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["path"].as_str().unwrap())
        .collect()
}

#[test]
fn public_limits_are_frozen_and_coherent() {
    assert_eq!(MAX_SEMANTIC_SEARCH_QUERY_BYTES, 4_096);
    assert_eq!(MAX_SEMANTIC_SEARCH_PATH_BYTES, 4_096);
    assert_eq!(MAX_SEMANTIC_SEARCH_RESULT_PATH_BYTES, 4_096);
    assert_eq!(MAX_SEMANTIC_SEARCH_DEPTH, 256);
    assert_eq!(MAX_SEMANTIC_SEARCH_VISITED_ENTRIES, 2_000);
    assert_eq!(MAX_SEMANTIC_SEARCH_TOTAL_ENTRY_NAME_BYTES, 8_388_608);
    assert_eq!(MAX_SEMANTIC_SEARCH_FILE_BYTES, 102_400);
    assert_eq!(MAX_SEMANTIC_SEARCH_TOTAL_CONTENT_BYTES, 67_108_864);
    assert_eq!(MAX_SEMANTIC_SEARCH_RESULT_LINE_BYTES, 2_000);
    assert_eq!(MAX_SEMANTIC_SEARCH_MATCH_STEPS, 2_147_483_648);
    assert_eq!(MAX_SEMANTIC_SEARCH_KEYWORDS, 16);
    assert_eq!(MAX_SEMANTIC_SEARCH_RETAINED_RESULTS, 200);
    assert_eq!(MAX_SEMANTIC_SEARCH_SHOWN_RESULTS, 100);
    assert_eq!(MAX_SEMANTIC_SEARCH_TOTAL_RESULT_PATH_BYTES, 819_200);
    assert_eq!(MAX_SEMANTIC_SEARCH_TOTAL_RESULT_LINE_BYTES, 400_000);
    assert_eq!(MAX_SEMANTIC_SEARCH_SERIALIZED_RESULT_BYTES, 49_152);
}

#[test]
fn spec_and_preflight_are_strict_effect_free_and_capability_exact() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let spec = tool.spec();
    assert_eq!(spec.name.as_str(), SEMANTIC_SEARCH_TOOL_NAME);
    assert_eq!(spec.input_schema["type"], "object");
    assert_eq!(spec.input_schema["required"], json!(["query"]));
    assert_eq!(spec.input_schema["additionalProperties"], false);

    let prepared = tool
        .prepare(call(json!({"query": "Alpha topic", "path": "./scope//."})))
        .unwrap();
    let expected_capability = Capability::Filesystem {
        access: FilesystemAccess::SearchContent,
        path: "scope".to_owned(),
    };
    assert_eq!(prepared.capability(), Some(&expected_capability));
    assert_eq!(
        prepared.arguments(),
        &json!({"query": "Alpha topic", "path": "scope"})
    );

    for arguments in [
        Value::Null,
        json!([]),
        json!({}),
        json!({"query": null}),
        json!({"query": 1}),
        json!({"query": "alpha", "path": null}),
        json!({"query": "alpha", "unknown": true}),
    ] {
        assert_invalid_arguments(tool.prepare(call(arguments)).unwrap_err());
    }
    assert_invalid_arguments(
        tool.prepare(named_call("grep_files", json!({"query": "alpha"})))
            .unwrap_err(),
    );
    assert!(temporary.path().read_dir().unwrap().next().is_none());
}

#[test]
fn query_and_path_bounds_are_byte_exact_and_direct_execution_revalidates_canonical_form() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let maximum_query = "q".repeat(MAX_SEMANTIC_SEARCH_QUERY_BYTES);
    let prepared = tool.prepare(call(json!({"query": maximum_query}))).unwrap();
    assert_eq!(prepared.arguments()["path"], ".");
    assert_invalid_query(tool.prepare(call(json!({"query": ""}))).unwrap_err());
    for query in ["   \t", "bad\0query", "bad\nquery", "bad\u{202e}query"] {
        assert_invalid_query(tool.prepare(call(json!({"query": query}))).unwrap_err());
    }
    let tab_query = tool.prepare(call(json!({"query": "alpha\tbeta"}))).unwrap();
    assert_eq!(tab_query.arguments()["query"], "alpha\tbeta");
    assert_invalid_query(
        tool.prepare(call(json!({
            "query": "q".repeat(MAX_SEMANTIC_SEARCH_QUERY_BYTES + 1)
        })))
        .unwrap_err(),
    );
    for path in [
        "",
        "/absolute",
        "../escape",
        "scope/../../escape",
        "bad\nname",
    ] {
        assert_invalid_path(
            tool.prepare(call(json!({"query": "alpha", "path": path})))
                .unwrap_err(),
        );
    }

    assert_invalid_arguments(
        execute(&tool, json!({"query": "alpha"}), CancellationToken::new()).unwrap_err(),
    );
    assert_invalid_arguments(
        execute(
            &tool,
            canonical("alpha", "./scope"),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
}

#[test]
fn lexical_scoring_best_line_filename_bonus_case_fold_and_order_are_exact() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("src")).unwrap();
    fs::write(
        temporary.path().join("src/alpha.rs"),
        "Alpha topic\nbeta ALPHA\n",
    )
    .unwrap();
    fs::write(temporary.path().join("src/beta.rs"), "alpha beta\n").unwrap();
    fs::write(temporary.path().join("src/gamma-alpha.rs"), "unrelated\n").unwrap();
    fs::write(temporary.path().join("src/tie-a.rs"), "alpha\n").unwrap();
    fs::write(temporary.path().join("src/tie-b.rs"), "alpha\n").unwrap();

    let output = run(&tool(temporary.path()), "alpha beta", "src");
    assert!(!output.is_error);
    assert_eq!(output.content["query"], "alpha beta");
    assert_eq!(output.content["path"], "src");
    assert_eq!(output.content["keywords"], json!(["alpha", "beta"]));
    assert_eq!(
        result_paths(&output),
        [
            "src/alpha.rs",
            "src/beta.rs",
            "src/gamma-alpha.rs",
            "src/tie-a.rs",
            "src/tie-b.rs"
        ]
    );
    let results = output.content["results"].as_array().unwrap();
    assert_eq!(results[0]["score"], 6);
    assert_eq!(results[0]["line_number"], 2);
    assert_eq!(results[0]["line"], "beta ALPHA");
    assert_eq!(results[1]["score"], 5);
    assert_eq!(results[2]["score"], 3);
    assert_eq!(results[2]["line_number"], 0);
    assert_eq!(results[2]["line"], "");
    assert_eq!(output.content["matching_files"], 5);
    assert_eq!(output.content["incomplete"], false);
}

#[test]
fn splitters_stopwords_short_tokens_and_sixteen_keyword_cap_match_pinned_behavior() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("concept.txt"), "k01 k16 k17 useful\n").unwrap();
    let query = "a an the x useful,first.second;third:fourth?fifth!sixth\tseventh eighth ninth tenth eleventh twelfth thirteenth fourteenth fifteenth sixteenth seventeenth";
    let output = run(&tool(temporary.path()), query, ".");
    let keywords = output.content["keywords"].as_array().unwrap();
    assert_eq!(keywords.len(), MAX_SEMANTIC_SEARCH_KEYWORDS);
    assert_eq!(keywords[0], "useful");
    assert_eq!(keywords[15], "fifteenth");

    let empty = run(&tool(temporary.path()), "the and it", "missing");
    assert_eq!(empty.content["keywords"], json!([]));
    assert_eq!(empty.content["results"], json!([]));
    for field in [
        "visited_entries",
        "candidate_files",
        "searched_files",
        "skipped_oversized_files",
        "skipped_non_text_files",
        "skipped_symlink_entries",
        "matching_files",
    ] {
        assert_eq!(empty.content[field], 0, "field {field}");
    }
    assert_eq!(empty.content["incomplete"], false);
    assert_eq!(empty.content["incomplete_reasons"], json!([]));
}

#[test]
fn direct_file_root_and_utf8_line_clipping_are_bounded() {
    let temporary = TemporaryDirectory::new();
    let line = format!(
        "needle {}é",
        "x".repeat(MAX_SEMANTIC_SEARCH_RESULT_LINE_BYTES)
    );
    fs::write(temporary.path().join("direct.txt"), &line).unwrap();
    let output = run(&tool(temporary.path()), "needle", "direct.txt");
    assert_eq!(output.content["visited_entries"], 0);
    assert_eq!(output.content["candidate_files"], 1);
    assert_eq!(output.content["searched_files"], 1);
    let result = &output.content["results"][0];
    assert_eq!(result["path"], "direct.txt");
    assert_eq!(result["line_number"], 1);
    assert!(result["line"].as_str().unwrap().len() <= MAX_SEMANTIC_SEARCH_RESULT_LINE_BYTES);
    assert!(
        result["line"]
            .as_str()
            .unwrap()
            .is_char_boundary(result["line"].as_str().unwrap().len())
    );
    assert_eq!(result["line_truncated"], true);
}

#[test]
fn ignored_directories_non_text_oversized_and_symlink_entries_have_exact_stats() {
    let temporary = TemporaryDirectory::new();
    for ignored in [
        ".git",
        ".zig-cache",
        "zig-out",
        "node_modules",
        ".next",
        "dist",
        "build",
        "coverage",
        "target",
    ] {
        fs::create_dir(temporary.path().join(ignored)).unwrap();
        fs::write(temporary.path().join(ignored).join("ignored.txt"), "needle").unwrap();
    }
    fs::write(temporary.path().join("binary.bin"), b"needle\0binary").unwrap();
    fs::write(
        temporary.path().join("invalid.txt"),
        [b'n', b'e', b'e', b'd', b'l', b'e', 0xff],
    )
    .unwrap();
    fs::write(
        temporary.path().join("oversized.txt"),
        vec![b'n'; MAX_SEMANTIC_SEARCH_FILE_BYTES + 1],
    )
    .unwrap();
    fs::write(temporary.path().join("real.txt"), "needle").unwrap();
    symlink("real.txt", temporary.path().join("internal-link.txt")).unwrap();
    let external = TemporaryDirectory::new();
    fs::write(external.path().join("outside.txt"), "needle").unwrap();
    symlink(
        external.path().join("outside.txt"),
        temporary.path().join("external-link.txt"),
    )
    .unwrap();

    let output = run(&tool(temporary.path()), "needle", ".");
    assert_eq!(result_paths(&output), ["real.txt"]);
    assert_eq!(output.content["searched_files"], 1);
    assert_eq!(output.content["skipped_non_text_files"], 2);
    assert_eq!(output.content["skipped_oversized_files"], 1);
    assert_eq!(output.content["skipped_symlink_entries"], 2);
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("ignored.txt")
    );
}

#[test]
fn selected_symlink_missing_root_and_removed_retained_root_fail_closed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("real.txt"), "needle").unwrap();
    symlink("real.txt", temporary.path().join("link.txt")).unwrap();
    let search_tool = tool(temporary.path());
    assert_tool_error(
        execute(
            &search_tool,
            canonical("needle", "link.txt"),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::PermissionDenied,
        "semantic_search_path_rejected",
        "requested path is not a confined regular file or directory",
        false,
    );
    assert_tool_error(
        execute(
            &search_tool,
            canonical("needle", "missing"),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::Unavailable,
        "semantic_search_not_found",
        "requested search root is unavailable",
        false,
    );

    let removed = TemporaryDirectory::new();
    let retained = tool(removed.path());
    fs::remove_dir(removed.path()).unwrap();
    assert_tool_error(
        execute(
            &retained,
            canonical("needle", "."),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::Unavailable,
        "semantic_search_unavailable",
        "requested semantic search is unavailable",
        true,
    );
}

#[test]
fn traversal_result_and_output_caps_are_reported_in_stable_order_with_global_best_retention() {
    let result_root = TemporaryDirectory::new();
    for index in 0..MAX_SEMANTIC_SEARCH_RETAINED_RESULTS {
        fs::write(
            result_root.path().join(format!("low-{index:03}.txt")),
            "needle\n",
        )
        .unwrap();
    }
    fs::write(
        result_root.path().join("zzz-needle-best.txt"),
        "needle needle\n",
    )
    .unwrap();
    let output = run(&tool(result_root.path()), "needle", ".");
    assert_eq!(output.content["matching_files"], 201);
    assert_eq!(output.content["results"].as_array().unwrap().len(), 100);
    assert_eq!(output.content["results"][0]["path"], "zzz-needle-best.txt");
    assert_eq!(
        output.content["incomplete_reasons"],
        json!(["result_cap", "output_cap"])
    );

    let traversal_root = TemporaryDirectory::new();
    for index in 0..=MAX_SEMANTIC_SEARCH_VISITED_ENTRIES {
        fs::write(
            traversal_root.path().join(format!("entry-{index:04}.txt")),
            "absent\n",
        )
        .unwrap();
    }
    let capped = run(&tool(traversal_root.path()), "needle", ".");
    assert_eq!(
        capped.content["visited_entries"],
        MAX_SEMANTIC_SEARCH_VISITED_ENTRIES
    );
    assert_eq!(
        capped.content["incomplete_reasons"],
        json!(["traversal_cap"])
    );
}

#[test]
fn serialized_output_is_bounded_and_output_cap_trims_only_low_ranked_results() {
    let temporary = TemporaryDirectory::new();
    let long_line = format!(
        "needle {}",
        "x".repeat(MAX_SEMANTIC_SEARCH_RESULT_LINE_BYTES)
    );
    for index in 0..MAX_SEMANTIC_SEARCH_SHOWN_RESULTS {
        fs::write(
            temporary
                .path()
                .join(format!("result-{index:03}-needle.txt")),
            &long_line,
        )
        .unwrap();
    }
    let output = run(&tool(temporary.path()), "needle", ".");
    assert!(output.content["results"].as_array().unwrap().len() < 100);
    assert_eq!(output.content["incomplete_reasons"], json!(["output_cap"]));
    assert!(
        serde_json::to_vec(&output).unwrap().len() <= MAX_SEMANTIC_SEARCH_SERIALIZED_RESULT_BYTES
    );
    assert_eq!(
        output.content["results"][0]["path"],
        "result-000-needle.txt"
    );
}

#[test]
fn execution_future_is_inert_until_poll_and_precancelled_execution_is_exact() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("secret.txt"), "needle").unwrap();
    let tool = tool(temporary.path());
    let cancellation = CancellationToken::new();
    let future = tool.execute(
        context(),
        canonical("needle", "secret.txt"),
        cancellation.clone(),
    );
    cancellation.cancel();
    let error = poll_ready(future).unwrap_err();
    assert_tool_error(
        error,
        ToolErrorKind::Cancelled,
        "semantic_search_cancelled",
        "semantic_search execution was cancelled",
        false,
    );
}

#[test]
fn constructor_failures_are_exact_redacted_and_do_not_follow_final_symlinks() {
    let relative_secret = "PRIVATE_RELATIVE_SEMANTIC_ROOT";
    assert_open_error(
        SemanticSearchTool::open(Path::new(relative_secret)).unwrap_err(),
        SemanticSearchToolOpenErrorKind::InvalidRoot,
        "native semantic_search workspace root is invalid",
        relative_secret,
    );

    let temporary = TemporaryDirectory::new();
    let file_secret = temporary.path().join("PRIVATE_FILE_SEMANTIC_ROOT");
    fs::write(&file_secret, "content").unwrap();
    assert_open_error(
        SemanticSearchTool::open(&file_secret).unwrap_err(),
        SemanticSearchToolOpenErrorKind::InvalidFileType,
        "native semantic_search workspace root is not a directory",
        file_secret.to_str().unwrap(),
    );
    let link_secret = temporary.path().join("PRIVATE_LINK_SEMANTIC_ROOT");
    symlink(temporary.path(), &link_secret).unwrap();
    assert_open_error(
        SemanticSearchTool::open(&link_secret).unwrap_err(),
        SemanticSearchToolOpenErrorKind::InvalidFileType,
        "native semantic_search workspace root is not a directory",
        link_secret.to_str().unwrap(),
    );
}

fn assert_open_error(
    error: SemanticSearchToolOpenError,
    kind: SemanticSearchToolOpenErrorKind,
    message: &str,
    private: &str,
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), message);
    assert_eq!(
        format!("{error:?}"),
        format!("SemanticSearchToolOpenError {{ kind: {kind:?} }}")
    );
    assert!(error.source().is_none());
    assert!(!error.to_string().contains(private));
    assert!(!format!("{error:?}").contains(private));
}
