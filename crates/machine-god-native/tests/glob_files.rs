#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::ffi::OsString;
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    CancellationToken, Capability, FilesystemAccess, SessionId, SessionIncarnationId, Tool,
    ToolCall, ToolCallId, ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    GLOB_FILES_TOOL_NAME, GlobFilesTool, GlobFilesToolOpenError, GlobFilesToolOpenErrorKind,
    MAX_GLOB_FILES_DEPTH, MAX_GLOB_FILES_MATCH_STEPS, MAX_GLOB_FILES_MATCHES,
    MAX_GLOB_FILES_PATH_BYTES, MAX_GLOB_FILES_PATTERN_BYTES, MAX_GLOB_FILES_RESULT_PATH_BYTES,
    MAX_GLOB_FILES_TOTAL_ENTRY_NAME_BYTES, MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES,
    MAX_GLOB_FILES_VISITED_ENTRIES,
};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{AtFlags, Mode, OFlags};
use serde_json::{Value, json};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("mg-glob-files-{}-{identifier}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create a temporary directory: {error}"),
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
            Err(error) => panic!("failed to remove a temporary directory: {error}"),
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_immediately_ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(Pin::as_mut(&mut future), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("glob_files execution unexpectedly returned a pending future"),
    }
}

fn tool(root: &Path) -> GlobFilesTool {
    GlobFilesTool::open(root).expect("temporary workspace root is valid")
}

fn tool_name(name: &str) -> ToolName {
    ToolName::new(name).expect("test tool name is valid")
}

fn call(arguments: Value) -> ToolCall {
    named_call(GLOB_FILES_TOOL_NAME, arguments)
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("glob-files-call").unwrap(),
        name: tool_name(name),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("glob-files-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("glob-files-incarnation").unwrap(),
        turn_id: TurnId::new("glob-files-turn").unwrap(),
        call_id: ToolCallId::new("glob-files-call").unwrap(),
    }
}

fn arguments(pattern: &str, path: &str, mode: &str) -> Value {
    json!({ "pattern": pattern, "path": path, "mode": mode })
}

fn execute(
    tool: &GlobFilesTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_immediately_ready(tool.execute(context(), arguments, cancellation))
}

fn matches(tool: &GlobFilesTool, pattern: &str, path: &str) -> ToolOutput {
    execute(
        tool,
        arguments(pattern, path, "matches"),
        CancellationToken::new(),
    )
    .unwrap()
}

fn count(tool: &GlobFilesTool, pattern: &str, path: &str) -> ToolOutput {
    execute(
        tool,
        arguments(pattern, path, "count"),
        CancellationToken::new(),
    )
    .unwrap()
}

fn match_paths(output: &ToolOutput) -> Vec<&str> {
    output.content["matches"]
        .as_array()
        .expect("matches is an array")
        .iter()
        .map(|path| path.as_str().expect("match path is a string"))
        .collect()
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
        "glob_files_invalid_arguments",
        "glob_files arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "glob_files_invalid_path",
        "glob_files path is invalid",
        false,
    );
}

fn assert_invalid_pattern(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "glob_files_invalid_pattern",
        "glob_files pattern is invalid",
        false,
    );
}

fn assert_path_rejected(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::PermissionDenied,
        "glob_files_path_rejected",
        "requested path is not a confined directory",
        false,
    );
}

fn assert_scan_limit(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::Execution,
        "glob_files_scan_limit",
        "requested glob search exceeds the scan limit",
        false,
    );
}

fn create_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("failed to invoke the POSIX mkfifo utility");
    assert!(status.success(), "mkfifo failed with {status}");
}

fn write_files(root: &Path, names: &[&str]) {
    for name in names {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, []).unwrap();
    }
}

#[test]
fn exported_contract_limits_and_spec_are_exact() {
    assert_eq!(GLOB_FILES_TOOL_NAME, "glob_files");
    assert_eq!(MAX_GLOB_FILES_PATTERN_BYTES, 4_096);
    assert_eq!(MAX_GLOB_FILES_PATH_BYTES, 4_096);
    assert_eq!(MAX_GLOB_FILES_RESULT_PATH_BYTES, 4_096);
    assert_eq!(MAX_GLOB_FILES_MATCHES, 100);
    assert_eq!(MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES, 16 * 1_024);
    assert_eq!(MAX_GLOB_FILES_VISITED_ENTRIES, 100_000);
    assert_eq!(MAX_GLOB_FILES_TOTAL_ENTRY_NAME_BYTES, 16 * 1_024 * 1_024);
    assert_eq!(MAX_GLOB_FILES_MATCH_STEPS, 8 * 1_024 * 1_024);
    assert_eq!(MAX_GLOB_FILES_DEPTH, 256);
    assert_eq!(
        format!("{:?}", GlobFilesToolOpenErrorKind::UnsupportedPlatform),
        "UnsupportedPlatform"
    );

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), GLOB_FILES_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Find file paths matching a glob pattern within the configured workspace"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern relative to the search root, such as src/**/*.rs or *.md"
                },
                "path": {
                    "type": "string",
                    "description": "Workspace-relative directory search root; defaults to the workspace root"
                },
                "mode": {
                    "type": "string",
                    "enum": ["matches", "count"],
                    "description": "Return matching paths or an exact count; defaults to matches"
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    );
}

#[test]
fn prepare_requires_exact_name_required_pattern_and_typed_optional_fields() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let invalid_calls = [
        named_call("other_tool", json!({ "pattern": "*" })),
        call(json!({})),
        call(json!({ "pattern": null })),
        call(json!({ "pattern": 1 })),
        call(json!({ "pattern": [] })),
        call(json!({ "pattern": "*", "path": null })),
        call(json!({ "pattern": "*", "path": 1 })),
        call(json!({ "pattern": "*", "mode": null })),
        call(json!({ "pattern": "*", "mode": "all" })),
        call(json!({ "pattern": "*", "extra": true })),
        call(json!("*")),
        call(json!(["*"])),
    ];

    for invalid_call in invalid_calls {
        assert_invalid_arguments(tool.prepare(invalid_call).unwrap_err());
    }
}

#[test]
fn prepare_defaults_and_normalizes_exact_policy_and_execution_arguments() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let defaulted = tool.prepare(call(json!({ "pattern": "*.rs" }))).unwrap();
    assert_eq!(
        defaulted.capability(),
        &Capability::Filesystem {
            access: FilesystemAccess::EnumerateRecursive,
            path: ".".to_owned(),
        }
    );
    assert_eq!(
        defaulted.arguments(),
        &json!({ "pattern": "*.rs", "path": ".", "mode": "matches" })
    );

    let normalized = tool
        .prepare(call(json!({
            "pattern": "./src//./**//*.rs/",
            "path": "./scope///./nested//",
            "mode": "count"
        })))
        .unwrap();
    assert_eq!(
        normalized.capability(),
        &Capability::Filesystem {
            access: FilesystemAccess::EnumerateRecursive,
            path: "scope/nested".to_owned(),
        }
    );
    assert_eq!(
        normalized.arguments(),
        &json!({
            "pattern": "src/**/*.rs",
            "path": "scope/nested",
            "mode": "count"
        })
    );

    let root = tool
        .prepare(call(json!({ "pattern": "foo/", "path": "././" })))
        .unwrap();
    assert_eq!(
        root.arguments(),
        &json!({ "pattern": "foo", "path": ".", "mode": "matches" })
    );
}

#[test]
fn prepare_accepts_exact_path_and_pattern_byte_limits_and_rejects_one_more() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let exact_path = "λ".repeat(MAX_GLOB_FILES_PATH_BYTES / "λ".len());
    let exact_pattern = "π".repeat(MAX_GLOB_FILES_PATTERN_BYTES / "π".len());
    let prepared = tool
        .prepare(call(json!({
            "pattern": exact_pattern,
            "path": exact_path,
            "mode": "matches"
        })))
        .unwrap();
    assert_eq!(
        prepared.arguments()["path"].as_str().unwrap().len(),
        MAX_GLOB_FILES_PATH_BYTES
    );
    assert_eq!(
        prepared.arguments()["pattern"].as_str().unwrap().len(),
        MAX_GLOB_FILES_PATTERN_BYTES
    );

    let path_over = "λ".repeat(MAX_GLOB_FILES_PATH_BYTES / "λ".len() + 1);
    assert_invalid_path(
        tool.prepare(call(json!({ "pattern": "*", "path": path_over })))
            .unwrap_err(),
    );
    let pattern_over = "π".repeat(MAX_GLOB_FILES_PATTERN_BYTES / "π".len() + 1);
    assert_invalid_pattern(
        tool.prepare(call(json!({ "pattern": pattern_over })))
            .unwrap_err(),
    );
}

#[test]
fn prepare_rejects_unsafe_paths_and_empty_absolute_parent_control_or_bidi_patterns() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for path in [
        "",
        "..",
        "scope/../secret",
        "/absolute",
        "nul\0path",
        "tab\tpath",
        "bidi\u{202e}path",
    ] {
        assert_invalid_path(
            tool.prepare(call(json!({ "pattern": "*", "path": path })))
                .unwrap_err(),
        );
    }
    for pattern in [
        "",
        ".",
        "././",
        "..",
        "src/../secret",
        "/absolute",
        "nul\0pattern",
        "tab\tpattern",
        "line\npattern",
        "arabic\u{061c}pattern",
        "bidi\u{202e}pattern",
        "isolate\u{2066}pattern",
    ] {
        assert_invalid_pattern(
            tool.prepare(call(json!({ "pattern": pattern })))
                .unwrap_err(),
        );
    }
}

#[test]
fn prepare_is_effect_free_for_a_missing_search_root() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("existing.txt"), b"unchanged").unwrap();
    let prepared = tool(temporary.path())
        .prepare(call(json!({
            "pattern": "./**//*.rs",
            "path": "missing//./nested"
        })))
        .unwrap();
    assert_eq!(
        prepared.capability(),
        &Capability::Filesystem {
            access: FilesystemAccess::EnumerateRecursive,
            path: "missing/nested".to_owned(),
        }
    );
    assert_eq!(
        prepared.arguments(),
        &json!({
            "pattern": "**/*.rs",
            "path": "missing/nested",
            "mode": "matches"
        })
    );
    assert!(!temporary.path().join("missing").exists());
    assert_eq!(
        fs::read(temporary.path().join("existing.txt")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn slash_free_patterns_match_basenames_recursively_and_return_exact_default_shape() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[
            "root.rs",
            "root.txt",
            "nested/child.rs",
            "nested/deep/leaf.rs",
        ],
    );
    let tool = tool(temporary.path());

    assert_eq!(
        matches(&tool, "*.rs", "."),
        ToolOutput::success(json!({
            "path": ".",
            "pattern": "*.rs",
            "mode": "matches",
            "matches": ["nested/child.rs", "nested/deep/leaf.rs", "root.rs"],
            "truncated": false
        }))
    );
}

#[test]
fn slashful_star_and_exact_globstar_have_distinct_segment_semantics() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[
            "root.rs",
            "src/top.rs",
            "src/deep/nested.rs",
            "a/b3.txt",
            "a/mid/b2.txt",
            "a/one/two/b1.txt",
        ],
    );
    let tool = tool(temporary.path());

    assert_eq!(
        match_paths(&matches(&tool, "src/*.rs", ".")),
        ["src/top.rs"]
    );
    assert_eq!(
        match_paths(&matches(&tool, "**/*.rs", ".")),
        ["root.rs", "src/deep/nested.rs", "src/top.rs"]
    );
    assert_eq!(
        match_paths(&matches(&tool, "a/**/b?.txt", ".")),
        ["a/b3.txt", "a/mid/b2.txt", "a/one/two/b1.txt"]
    );
}

#[test]
fn matcher_treats_question_as_one_utf8_byte_and_brackets_braces_backslash_as_literals() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[
            "a.rs",
            "ab.rs",
            "λ.rs",
            "[abc].rs",
            "{a,b}.rs",
            r"literal\name.rs",
        ],
    );
    let tool = tool(temporary.path());

    assert_eq!(match_paths(&matches(&tool, "?.rs", ".")), ["a.rs"]);
    assert_eq!(
        match_paths(&matches(&tool, "??.rs", ".")),
        ["ab.rs", "λ.rs"]
    );
    assert_eq!(match_paths(&matches(&tool, "λ.rs", ".")), ["λ.rs"]);
    for literal in ["[abc].rs", "{a,b}.rs", r"literal\name.rs"] {
        assert_eq!(match_paths(&matches(&tool, literal, ".")), [literal]);
    }
}

#[test]
fn repeated_stars_are_segment_wildcards_unless_the_entire_segment_is_double_star() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &["alpha.rs", "ab.rs", "foo/bar", "foo/x/bar", "foo/x/y/bar"],
    );
    let tool = tool(temporary.path());

    assert_eq!(
        match_paths(&matches(&tool, "a***.rs", ".")),
        ["ab.rs", "alpha.rs"]
    );
    assert_eq!(match_paths(&matches(&tool, "a**a.rs", ".")), ["alpha.rs"]);
    assert_eq!(
        match_paths(&matches(&tool, "foo/***/bar", ".")),
        ["foo/x/bar"]
    );
    assert_eq!(
        match_paths(&matches(&tool, "foo/**x/bar", ".")),
        ["foo/x/bar"]
    );
    assert_eq!(
        match_paths(&matches(&tool, "foo/**/bar", ".")),
        ["foo/bar", "foo/x/bar", "foo/x/y/bar"]
    );
}

#[test]
fn traversal_includes_hidden_files_and_final_symlinks_but_not_directories_or_specials() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[".hidden", "plain", "nested/child", "nested/deeper/leaf"],
    );
    fs::write(outside.path().join("PRIVATE_OUTSIDE_SECRET"), b"secret").unwrap();
    symlink(
        outside.path().join("PRIVATE_OUTSIDE_SECRET"),
        temporary.path().join("file-link"),
    )
    .unwrap();
    symlink(outside.path(), temporary.path().join("directory-link")).unwrap();
    symlink(
        outside.path().join("missing"),
        temporary.path().join("dangling-link"),
    )
    .unwrap();
    create_fifo(&temporary.path().join("pipe"));
    let listener = UnixListener::bind(temporary.path().join("socket")).unwrap();
    let tool = tool(temporary.path());

    assert_eq!(
        match_paths(&matches(&tool, "*", ".")),
        [
            ".hidden",
            "dangling-link",
            "directory-link",
            "file-link",
            "nested/child",
            "nested/deeper/leaf",
            "plain"
        ]
    );
    let serialized = serde_json::to_string(&matches(&tool, "*", ".")).unwrap();
    assert!(!serialized.contains("PRIVATE_OUTSIDE_SECRET"));
    assert!(!serialized.contains("pipe"));
    assert!(!serialized.contains("socket"));
    drop(listener);
}

#[test]
fn selected_path_scopes_slashful_matching_but_results_remain_workspace_relative() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[
            "scope/src/inside.rs",
            "scope/src/deep/not-direct.rs",
            "scope/other.rs",
            "outside/src/outside.rs",
        ],
    );
    let tool = tool(temporary.path());

    assert_eq!(
        matches(&tool, "src/*.rs", "scope"),
        ToolOutput::success(json!({
            "path": "scope",
            "pattern": "src/*.rs",
            "mode": "matches",
            "matches": ["scope/src/inside.rs"],
            "truncated": false
        }))
    );
}

#[test]
fn results_are_globally_sorted_by_full_workspace_relative_utf8_bytes() {
    let temporary = TemporaryDirectory::new();
    for name in [
        "z/last.txt",
        "a/z.txt",
        "root.txt",
        "a/a.txt",
        ".hidden.txt",
    ] {
        let path = temporary.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, []).unwrap();
    }
    let tool = tool(temporary.path());

    assert_eq!(
        match_paths(&matches(&tool, "*.txt", ".")),
        [
            ".hidden.txt",
            "a/a.txt",
            "a/z.txt",
            "root.txt",
            "z/last.txt"
        ]
    );
}

#[test]
fn matches_truncates_at_one_hundred_while_count_remains_complete() {
    let temporary = TemporaryDirectory::new();
    for index in 0..100 {
        fs::write(temporary.path().join(format!("entry-{index:03}.rs")), []).unwrap();
    }
    let tool = tool(temporary.path());

    let exact = matches(&tool, "*.rs", ".");
    assert_eq!(match_paths(&exact).len(), MAX_GLOB_FILES_MATCHES);
    assert_eq!(exact.content["truncated"], false);
    assert_eq!(count(&tool, "*.rs", ".").content["count"], 100);

    fs::write(temporary.path().join("entry-100.rs"), []).unwrap();

    let output = matches(&tool, "*.rs", ".");
    let paths = match_paths(&output);
    assert_eq!(paths.len(), MAX_GLOB_FILES_MATCHES);
    assert_eq!(paths.first(), Some(&"entry-000.rs"));
    assert_eq!(paths.last(), Some(&"entry-099.rs"));
    assert_eq!(output.content["truncated"], true);
    assert_eq!(
        count(&tool, "*.rs", "."),
        ToolOutput::success(json!({
            "path": ".",
            "pattern": "*.rs",
            "mode": "count",
            "count": 101
        }))
    );
}

#[test]
fn aggregate_match_bytes_emit_the_longest_sorted_prefix_without_skipping() {
    let temporary = TemporaryDirectory::new();
    for index in 0..68 {
        let prefix = format!("{index:03}-");
        let name = format!("{prefix}{}", "x".repeat(240 - prefix.len()));
        fs::write(temporary.path().join(name), []).unwrap();
    }
    fs::write(temporary.path().join(format!("068-{}", "x".repeat(60))), []).unwrap();
    fs::write(temporary.path().join("069-overflow"), []).unwrap();
    fs::write(temporary.path().join("zzz"), []).unwrap();
    let tool = tool(temporary.path());

    let output = matches(&tool, "*", ".");
    let paths = match_paths(&output);
    assert_eq!(paths.len(), MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES / 240 + 1);
    assert_eq!(
        paths.iter().map(|path| path.len()).sum::<usize>(),
        MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES
    );
    assert_eq!(paths.first().unwrap().len(), 240);
    assert_eq!(paths.last().unwrap().len(), 64);
    assert!(!paths.contains(&"zzz"));
    assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(output.content["truncated"], true);
    assert_eq!(count(&tool, "*", ".").content["count"], 71);
}

#[test]
fn depth_two_hundred_fifty_six_candidates_are_eligible_but_child_descent_is_limited() {
    let temporary = TemporaryDirectory::new();
    let mut current = temporary.path().to_path_buf();
    for _ in 0..MAX_GLOB_FILES_DEPTH {
        current.push("d");
        fs::create_dir(&current).unwrap();
    }
    fs::write(current.join("at-limit.txt"), []).unwrap();
    let tool = tool(temporary.path());
    let expected = format!("{}at-limit.txt", "d/".repeat(MAX_GLOB_FILES_DEPTH));
    assert_eq!(
        match_paths(&matches(&tool, "at-limit.txt", ".")),
        [expected.as_str()]
    );

    fs::create_dir(current.join("too-deep")).unwrap();
    for mode in ["matches", "count"] {
        assert_scan_limit(
            execute(&tool, arguments("*", ".", mode), CancellationToken::new()).unwrap_err(),
        );
    }
}

#[test]
fn matcher_work_limit_fails_both_modes_after_simple_scan_of_same_deep_tree_succeeds() {
    const LEAF_CANDIDATES: usize = 64;
    let temporary = TemporaryDirectory::new();
    let mut deepest = temporary.path().to_path_buf();
    for _ in 0..MAX_GLOB_FILES_DEPTH {
        deepest.push("d");
        fs::create_dir(&deepest).unwrap();
    }

    let path_prefix = "d/".repeat(MAX_GLOB_FILES_DEPTH);
    let mut expected_paths = Vec::with_capacity(LEAF_CANDIDATES);
    for index in 0..LEAF_CANDIDATES {
        let name = format!("leaf-{index:02}.txt");
        fs::write(deepest.join(&name), []).unwrap();
        expected_paths.push(format!("{path_prefix}{name}"));
    }
    let tool = tool(temporary.path());

    let simple_count = count(&tool, "*", ".");
    assert_eq!(simple_count.content["count"], LEAF_CANDIDATES);
    let simple_matches = matches(&tool, "*", ".");
    let mut retained_bytes = 0_usize;
    let expected_prefix = expected_paths
        .iter()
        .map(String::as_str)
        .take_while(|path| {
            let next = retained_bytes + path.len();
            if next > MAX_GLOB_FILES_TOTAL_MATCH_PATH_BYTES {
                false
            } else {
                retained_bytes = next;
                true
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(match_paths(&simple_matches), expected_prefix);
    assert_eq!(simple_matches.content["truncated"], true);

    let alternating_pattern = (0..MAX_GLOB_FILES_DEPTH)
        .flat_map(|_| ["**", "*"])
        .collect::<Vec<_>>()
        .join("/");
    assert!(alternating_pattern.len() <= MAX_GLOB_FILES_PATTERN_BYTES);
    let candidate_components = MAX_GLOB_FILES_DEPTH + 1;
    let steps_per_candidate =
        MAX_GLOB_FILES_DEPTH * ((candidate_components + 1) + candidate_components);
    assert_eq!(steps_per_candidate, 131_840);
    assert!(steps_per_candidate * (LEAF_CANDIDATES - 1) <= MAX_GLOB_FILES_MATCH_STEPS);
    assert!(steps_per_candidate * LEAF_CANDIDATES > MAX_GLOB_FILES_MATCH_STEPS);

    let outcomes = ["matches", "count"].map(|mode| {
        (
            mode,
            execute(
                &tool,
                arguments(&alternating_pattern, ".", mode),
                CancellationToken::new(),
            ),
        )
    });
    let mut unexpected_successes = Vec::new();
    for (mode, outcome) in outcomes {
        match outcome {
            Ok(_) => unexpected_successes.push(mode),
            Err(error) => assert_scan_limit(error),
        }
    }
    assert!(
        unexpected_successes.is_empty(),
        "matcher work limit returned partial successes: {unexpected_successes:?}"
    );
}

fn create_deep_result_fixture(root: &Path) -> (Vec<OwnedFd>, String, String, String) {
    let root = rustix::fs::open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    let component = "d".repeat(240);
    let mut directories = vec![root];
    let mut components = Vec::new();
    for _ in 0..16 {
        let parent = directories.last().unwrap();
        rustix::fs::mkdirat(parent.as_fd(), &component, Mode::from_raw_mode(0o700)).unwrap();
        let child = rustix::fs::openat(
            parent.as_fd(),
            &component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        directories.push(child);
        components.push(component.clone());
    }
    let exact_name = "e".repeat(240);
    let overflow_name = "o".repeat(241);
    let exact_path = format!("{}/{}", components.join("/"), exact_name);
    assert_eq!(exact_path.len(), MAX_GLOB_FILES_RESULT_PATH_BYTES);
    (directories, component, exact_name, overflow_name)
}

fn create_file_at(parent: &OwnedFd, name: &str) {
    rustix::fs::openat(
        parent.as_fd(),
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .unwrap();
}

#[test]
fn exact_four_kib_result_path_is_allowed_and_one_byte_more_is_scan_limit() {
    let temporary = TemporaryDirectory::new();
    let (directories, component, exact_name, overflow_name) =
        create_deep_result_fixture(temporary.path());
    let deepest = directories.last().unwrap();
    create_file_at(deepest, &exact_name);
    let exact_path = format!("{}/{}", [component.as_str(); 16].join("/"), exact_name);
    let tool = tool(temporary.path());
    assert_eq!(
        match_paths(&matches(&tool, "*", ".")),
        [exact_path.as_str()]
    );

    create_file_at(deepest, &overflow_name);
    for mode in ["matches", "count"] {
        assert_scan_limit(
            execute(&tool, arguments("*", ".", mode), CancellationToken::new()).unwrap_err(),
        );
    }

    rustix::fs::unlinkat(deepest.as_fd(), &exact_name, AtFlags::empty()).unwrap();
    rustix::fs::unlinkat(deepest.as_fd(), &overflow_name, AtFlags::empty()).unwrap();
    for parent in directories.iter().take(16).rev() {
        rustix::fs::unlinkat(parent.as_fd(), &component, AtFlags::REMOVEDIR).unwrap();
    }
}

#[test]
fn invalid_utf8_control_and_bidi_entry_names_fail_even_when_the_pattern_cannot_match() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let names = [
        OsString::from_vec(b"PRIVATE_INVALID_\xff_NAME".to_vec()),
        OsString::from("PRIVATE_CONTROL_\t_NAME"),
        OsString::from("PRIVATE_BIDI_\u{202e}_NAME"),
    ];

    for name in names {
        let path = temporary.path().join(&name);
        match fs::write(&path, []) {
            Ok(()) => {}
            Err(error) if name.to_str().is_none() => {
                assert!(
                    [libc::EILSEQ, libc::EINVAL].contains(
                        &error
                            .raw_os_error()
                            .expect("invalid-name failure has an OS code")
                    )
                );
                continue;
            }
            Err(error) => panic!("failed to create unsafe-name fixture: {error}"),
        }
        for mode in ["matches", "count"] {
            let error = execute(
                &tool,
                arguments("DEFINITELY_NO_MATCH", ".", mode),
                CancellationToken::new(),
            )
            .unwrap_err();
            let display = error.to_string();
            let debug = format!("{error:?}");
            assert_tool_error(
                error,
                ToolErrorKind::Execution,
                "glob_files_invalid_entry_name",
                "requested glob search contains an unsupported entry name",
                false,
            );
            if let Some(name) = name.to_str() {
                assert!(!display.contains(name));
                assert!(!debug.contains(name));
            }
        }
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn missing_rejected_permission_and_unavailable_search_roots_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    fs::write(temporary.path().join("PRIVATE_FILE_ROOT"), []).unwrap();
    symlink(outside.path(), temporary.path().join("PRIVATE_LINK_ROOT")).unwrap();
    let tool = tool(temporary.path());

    let missing = "PRIVATE_MISSING_ROOT";
    let error = execute(
        &tool,
        arguments("*", missing, "matches"),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert!(!format!("{error:?}").contains(missing));
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "glob_files_not_found",
        "requested search root is unavailable",
        false,
    );
    for path in ["PRIVATE_FILE_ROOT", "PRIVATE_LINK_ROOT"] {
        assert_path_rejected(
            execute(
                &tool,
                arguments("*", path, "matches"),
                CancellationToken::new(),
            )
            .unwrap_err(),
        );
    }

    let too_long = format!("PRIVATE_{}", "x".repeat(300));
    assert_tool_error(
        execute(
            &tool,
            arguments("*", &too_long, "count"),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::Unavailable,
        "glob_files_unavailable",
        "requested glob search is unavailable",
        true,
    );

    let inaccessible = temporary.path().join("PRIVATE_INACCESSIBLE");
    fs::create_dir(&inaccessible).unwrap();
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o000)).unwrap();
    let operating_system_enforces_mode = fs::read_dir(&inaccessible).is_err();
    if operating_system_enforces_mode {
        assert_tool_error(
            execute(
                &tool,
                arguments("*", "PRIVATE_INACCESSIBLE", "matches"),
                CancellationToken::new(),
            )
            .unwrap_err(),
            ToolErrorKind::PermissionDenied,
            "glob_files_permission_denied",
            "requested search root cannot be enumerated",
            false,
        );
    }
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn retained_root_rename_and_path_replacement_cannot_redirect_the_search() {
    let temporary = TemporaryDirectory::new();
    let original = temporary.path().join("workspace");
    let retained = temporary.path().join("retained-workspace");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("retained-only.rs"), []).unwrap();
    let tool = tool(&original);

    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("PRIVATE_REPLACEMENT.rs"), []).unwrap();

    let output = matches(&tool, "*.rs", ".");
    assert_eq!(match_paths(&output), ["retained-only.rs"]);
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("PRIVATE_REPLACEMENT")
    );
}

#[test]
fn removed_retained_root_is_fixed_unavailable_in_both_modes() {
    let temporary = TemporaryDirectory::new();
    let root = temporary.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let tool = tool(&root);
    fs::remove_dir(&root).unwrap();

    for mode in ["matches", "count"] {
        assert_tool_error(
            execute(&tool, arguments("*", ".", mode), CancellationToken::new()).unwrap_err(),
            ToolErrorKind::Unavailable,
            "glob_files_unavailable",
            "requested glob search is unavailable",
            true,
        );
    }
}

#[test]
fn execution_future_is_inert_until_polled_drop_detaches_nothing_and_pre_cancel_wins() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let future = tool.execute(
        context(),
        arguments("*.rs", ".", "matches"),
        CancellationToken::new(),
    );
    fs::write(temporary.path().join("late.rs"), []).unwrap();
    assert_eq!(
        match_paths(&poll_immediately_ready(future).unwrap()),
        ["late.rs"]
    );

    let dropped = tool.execute(
        context(),
        arguments("*.rs", ".", "matches"),
        CancellationToken::new(),
    );
    drop(dropped);
    fs::write(temporary.path().join("after-drop.rs"), b"unchanged").unwrap();
    assert_eq!(
        fs::read(temporary.path().join("after-drop.rs")).unwrap(),
        b"unchanged"
    );

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    assert_tool_error(
        execute(&tool, arguments("*", ".", "count"), cancellation).unwrap_err(),
        ToolErrorKind::Cancelled,
        "glob_files_cancelled",
        "glob_files execution was cancelled",
        false,
    );
}

#[test]
fn direct_execute_requires_exact_canonical_three_field_arguments() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for invalid in [
        json!({ "pattern": "*" }),
        json!({ "pattern": "*", "path": "." }),
        json!({ "pattern": "*", "path": ".", "mode": "all" }),
        json!({ "pattern": "*", "path": ".", "mode": "matches", "extra": true }),
        json!({ "pattern": "./*", "path": ".", "mode": "matches" }),
        json!({ "pattern": "*", "path": "./", "mode": "matches" }),
        json!({ "pattern": "a//b", "path": ".", "mode": "matches" }),
        json!("*"),
    ] {
        assert_invalid_arguments(execute(&tool, invalid, CancellationToken::new()).unwrap_err());
    }
    assert_invalid_pattern(
        execute(
            &tool,
            arguments("../secret", ".", "matches"),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_invalid_path(
        execute(
            &tool,
            arguments("*", "../secret", "matches"),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
}

#[test]
fn constructor_tool_and_error_debug_contracts_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let root_file = temporary.path().join("PRIVATE_WORKSPACE_FILE");
    let root_link = temporary.path().join("PRIVATE_WORKSPACE_LINK");
    fs::write(&root_file, b"not a directory").unwrap();
    symlink(temporary.path(), &root_link).unwrap();

    assert_open_error(
        GlobFilesTool::open(Path::new("PRIVATE_RELATIVE_ROOT")).unwrap_err(),
        GlobFilesToolOpenErrorKind::InvalidRoot,
        "native glob_files workspace root is invalid",
        &["PRIVATE_RELATIVE_ROOT"],
    );
    let missing = temporary.path().join("PRIVATE_MISSING_ROOT");
    assert_open_error(
        GlobFilesTool::open(&missing).unwrap_err(),
        GlobFilesToolOpenErrorKind::Unavailable,
        "native glob_files workspace root is unavailable",
        &["PRIVATE_MISSING_ROOT"],
    );
    for path in [&root_file, &root_link] {
        assert_open_error(
            GlobFilesTool::open(path).unwrap_err(),
            GlobFilesToolOpenErrorKind::InvalidFileType,
            "native glob_files workspace root is not a directory",
            &[path.file_name().unwrap().to_str().unwrap()],
        );
    }
    let debug = format!("{:?}", tool(temporary.path()));
    assert_eq!(debug, "GlobFilesTool { .. }");
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));
}

fn assert_open_error(
    error: GlobFilesToolOpenError,
    kind: GlobFilesToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    let debug = format!("{error:?}");
    assert_eq!(
        debug,
        format!("GlobFilesToolOpenError {{ kind: {kind:?} }}")
    );
    for secret in forbidden {
        assert!(!error.to_string().contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn preparation_and_execution_errors_never_reflect_untrusted_inputs() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for (error, secret) in [
        (
            tool.prepare(call(json!({ "pattern": "/PRIVATE_PATTERN" })))
                .unwrap_err(),
            "PRIVATE_PATTERN",
        ),
        (
            execute(
                &tool,
                arguments("*", "PRIVATE_MISSING", "matches"),
                CancellationToken::new(),
            )
            .unwrap_err(),
            "PRIVATE_MISSING",
        ),
    ] {
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }
}
