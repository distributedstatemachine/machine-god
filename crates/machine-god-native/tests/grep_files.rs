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
    GREP_FILES_TOOL_NAME, GrepFilesTool, GrepFilesToolOpenError, GrepFilesToolOpenErrorKind,
    MAX_GREP_FILES_CANDIDATE_FILES, MAX_GREP_FILES_CONTENT_MATCH_STEPS,
    MAX_GREP_FILES_CONTEXT_LINES, MAX_GREP_FILES_DEPTH, MAX_GREP_FILES_FILE_BYTES,
    MAX_GREP_FILES_HEAD_LIMIT, MAX_GREP_FILES_INCLUDE_BYTES, MAX_GREP_FILES_INCLUDE_MATCH_STEPS,
    MAX_GREP_FILES_OFFSET, MAX_GREP_FILES_PATH_BYTES, MAX_GREP_FILES_PATTERN_BYTES,
    MAX_GREP_FILES_RESULT_LINE_BYTES, MAX_GREP_FILES_RESULT_PATH_BYTES,
    MAX_GREP_FILES_SERIALIZED_RESULT_BYTES, MAX_GREP_FILES_TOTAL_CONTENT_BYTES,
    MAX_GREP_FILES_TOTAL_ENTRY_NAME_BYTES, MAX_GREP_FILES_TOTAL_RESULT_PATH_BYTES,
    MAX_GREP_FILES_TOTAL_RESULT_TEXT_BYTES, MAX_GREP_FILES_VISITED_ENTRIES,
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
                .join(format!("mg-grep-files-{}-{identifier}", std::process::id()));
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
        Poll::Pending => panic!("grep_files execution unexpectedly returned a pending future"),
    }
}

fn tool(root: &Path) -> GrepFilesTool {
    GrepFilesTool::open(root).expect("temporary workspace root is valid")
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("grep-files-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn call(arguments: Value) -> ToolCall {
    named_call(GREP_FILES_TOOL_NAME, arguments)
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("grep-files-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("grep-files-incarnation").unwrap(),
        turn_id: TurnId::new("grep-files-turn").unwrap(),
        call_id: ToolCallId::new("grep-files-call").unwrap(),
    }
}

fn canonical(pattern: &str, path: &str, include: Option<&str>, mode: &str) -> Value {
    json!({
        "pattern": pattern,
        "path": path,
        "include": include,
        "case_insensitive": false,
        "mode": mode,
        "head_limit": 100,
        "offset": 0,
        "context_lines": 0
    })
}

fn execute(
    tool: &GrepFilesTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_immediately_ready(tool.execute(context(), arguments, cancellation))
}

fn run(tool: &GrepFilesTool, arguments: Value) -> ToolOutput {
    execute(tool, arguments, CancellationToken::new()).unwrap()
}

fn matches(tool: &GrepFilesTool, pattern: &str, path: &str) -> ToolOutput {
    run(tool, canonical(pattern, path, None, "matches"))
}

fn count(tool: &GrepFilesTool, pattern: &str, path: &str) -> ToolOutput {
    run(tool, canonical(pattern, path, None, "count"))
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
        "grep_files_invalid_arguments",
        "grep_files arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "grep_files_invalid_path",
        "grep_files path is invalid",
        false,
    );
}

fn assert_invalid_pattern(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "grep_files_invalid_pattern",
        "grep_files pattern is invalid",
        false,
    );
}

fn assert_invalid_include(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "grep_files_invalid_include",
        "grep_files include pattern is invalid",
        false,
    );
}

fn assert_scan_limit(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::Execution,
        "grep_files_scan_limit",
        "requested content search exceeds the scan limit",
        false,
    );
}

fn common_result(
    pattern: &str,
    path: &str,
    include: Option<&str>,
    mode: &str,
    candidate_files: usize,
    searched_files: usize,
) -> Value {
    json!({
        "pattern": pattern,
        "path": path,
        "include": include,
        "case_insensitive": false,
        "mode": mode,
        "head_limit": 100,
        "offset": 0,
        "context_lines": 0,
        "candidate_files": candidate_files,
        "searched_files": searched_files,
        "skipped_oversized_files": 0,
        "skipped_non_text_files": 0
    })
}

fn write_files(root: &Path, files: &[(&str, &[u8])]) {
    for (name, contents) in files {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }
}

fn create_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("failed to invoke the POSIX mkfifo utility");
    assert!(status.success(), "mkfifo failed with {status}");
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
    assert_eq!(exact_path.len(), MAX_GREP_FILES_RESULT_PATH_BYTES);
    (directories, component, exact_name, overflow_name)
}

fn create_file_at(parent: &OwnedFd, name: &str, contents: &[u8]) {
    let file = rustix::fs::openat(
        parent.as_fd(),
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .unwrap();
    assert_eq!(rustix::io::write(&file, contents).unwrap(), contents.len());
}

fn create_directory_at(parent: &OwnedFd, name: &str) {
    rustix::fs::mkdirat(parent.as_fd(), name, Mode::from_raw_mode(0o700)).unwrap();
}

fn create_symlink_at(parent: &OwnedFd, name: &str) {
    rustix::fs::symlinkat("harmless-missing-target", parent.as_fd(), name).unwrap();
}

fn remove_deep_fixture_entries(
    directories: &[OwnedFd],
    component: &str,
    entries: &[(&str, AtFlags)],
) {
    let deepest = directories.last().unwrap();
    for (name, flags) in entries {
        rustix::fs::unlinkat(deepest.as_fd(), *name, *flags).unwrap();
    }
    for parent in directories.iter().take(16).rev() {
        rustix::fs::unlinkat(parent.as_fd(), component, AtFlags::REMOVEDIR).unwrap();
    }
}

#[test]
fn exported_contract_limits_and_spec_are_exact() {
    assert_eq!(GREP_FILES_TOOL_NAME, "grep_files");
    assert_eq!(MAX_GREP_FILES_PATTERN_BYTES, 4_096);
    assert_eq!(MAX_GREP_FILES_PATH_BYTES, 4_096);
    assert_eq!(MAX_GREP_FILES_INCLUDE_BYTES, 4_096);
    assert_eq!(MAX_GREP_FILES_RESULT_PATH_BYTES, 4_096);
    assert_eq!(MAX_GREP_FILES_HEAD_LIMIT, 100);
    assert_eq!(MAX_GREP_FILES_OFFSET, 64 * 1_024 * 1_024);
    assert_eq!(MAX_GREP_FILES_CONTEXT_LINES, 5);
    assert_eq!(MAX_GREP_FILES_FILE_BYTES, 200 * 1_024);
    assert_eq!(MAX_GREP_FILES_RESULT_LINE_BYTES, 4_096);
    assert_eq!(MAX_GREP_FILES_VISITED_ENTRIES, 100_000);
    assert_eq!(MAX_GREP_FILES_TOTAL_ENTRY_NAME_BYTES, 16 * 1_024 * 1_024);
    assert_eq!(MAX_GREP_FILES_CANDIDATE_FILES, 10_000);
    assert_eq!(MAX_GREP_FILES_TOTAL_CONTENT_BYTES, 64 * 1_024 * 1_024);
    assert_eq!(MAX_GREP_FILES_INCLUDE_MATCH_STEPS, 8 * 1_024 * 1_024);
    assert_eq!(MAX_GREP_FILES_CONTENT_MATCH_STEPS, 256 * 1_024 * 1_024);
    assert_eq!(MAX_GREP_FILES_DEPTH, 256);
    assert_eq!(MAX_GREP_FILES_TOTAL_RESULT_PATH_BYTES, 8 * 1_024);
    assert_eq!(MAX_GREP_FILES_TOTAL_RESULT_TEXT_BYTES, 8 * 1_024);
    assert_eq!(MAX_GREP_FILES_SERIALIZED_RESULT_BYTES, 48 * 1_024);
    assert_eq!(
        format!("{:?}", GrepFilesToolOpenErrorKind::UnsupportedPlatform),
        "UnsupportedPlatform"
    );

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), GREP_FILES_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Search UTF-8 text files for a literal substring within the configured workspace"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Literal plain-text pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Workspace-relative regular file or directory search root; defaults to the workspace root"
                },
                "include": {
                    "type": "string",
                    "description": "Optional glob pattern applied to candidate paths before file contents are read"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Search case-insensitively using ASCII case folding when true"
                },
                "mode": {
                    "type": "string",
                    "enum": ["matches", "files_with_matches", "count"],
                    "description": "Return matching lines, unique files with matches, or exact matching-line and matching-file counts"
                },
                "head_limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum results to return for matches or files_with_matches; defaults to 100"
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 64 * 1_024 * 1_024,
                    "description": "Zero-based result offset for matches or files_with_matches; defaults to 0"
                },
                "context_lines": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 5,
                    "description": "Lines before and after each emitted match in matches mode; defaults to 0"
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    );
}

#[test]
fn prepare_requires_exact_name_shape_types_modes_and_numeric_ranges() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let invalid_calls = [
        named_call("other_tool", json!({"pattern": "x"})),
        call(json!({})),
        call(json!({"pattern": null})),
        call(json!({"pattern": 3})),
        call(json!({"pattern": "x", "path": null})),
        call(json!({"pattern": "x", "include": null})),
        call(json!({"pattern": "x", "include": 3})),
        call(json!({"pattern": "x", "case_insensitive": 1})),
        call(json!({"pattern": "x", "mode": "files"})),
        call(json!({"pattern": "x", "head_limit": 0})),
        call(json!({"pattern": "x", "head_limit": 101})),
        call(json!({"pattern": "x", "head_limit": 1.0})),
        call(json!({"pattern": "x", "offset": -1})),
        call(json!({"pattern": "x", "offset": MAX_GREP_FILES_OFFSET + 1})),
        call(json!({"pattern": "x", "context_lines": -1})),
        call(json!({"pattern": "x", "context_lines": 6})),
        call(json!({"pattern": "x", "extra": true})),
        call(json!("x")),
    ];

    for invalid_call in invalid_calls {
        assert_invalid_arguments(tool.prepare(invalid_call).unwrap_err());
    }
}

#[test]
fn prepare_defaults_and_normalizes_exact_search_capability_and_arguments() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let defaulted = tool.prepare(call(json!({"pattern": "Needle"}))).unwrap();
    assert_eq!(
        defaulted.capability(),
        &Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: ".".to_owned(),
        }
    );
    assert_eq!(
        defaulted.arguments(),
        &json!({
            "pattern": "Needle",
            "path": ".",
            "include": null,
            "case_insensitive": false,
            "mode": "matches",
            "head_limit": 100,
            "offset": 0,
            "context_lines": 0
        })
    );

    let normalized = tool
        .prepare(call(json!({
            "pattern": "a/../[literal]*",
            "path": "./scope///./nested//",
            "include": "./src//./**//*.rs/",
            "case_insensitive": true,
            "mode": "files_with_matches",
            "head_limit": 1,
            "offset": 100_000,
            "context_lines": 5
        })))
        .unwrap();
    assert_eq!(
        normalized.capability(),
        &Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: "scope/nested".to_owned(),
        }
    );
    assert_eq!(
        normalized.arguments(),
        &json!({
            "pattern": "a/../[literal]*",
            "path": "scope/nested",
            "include": "src/**/*.rs",
            "case_insensitive": true,
            "mode": "files_with_matches",
            "head_limit": 1,
            "offset": 100_000,
            "context_lines": 5
        })
    );
}

#[test]
fn prepare_accepts_exact_string_limits_and_rejects_one_byte_more() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let exact_pattern = "π".repeat(MAX_GREP_FILES_PATTERN_BYTES / "π".len());
    let exact_path = "λ".repeat(MAX_GREP_FILES_PATH_BYTES / "λ".len());
    let exact_include = format!("{}*", "i".repeat(MAX_GREP_FILES_INCLUDE_BYTES - 1));
    let prepared = tool
        .prepare(call(json!({
            "pattern": exact_pattern,
            "path": exact_path,
            "include": exact_include
        })))
        .unwrap();
    assert_eq!(
        prepared.arguments()["pattern"].as_str().unwrap().len(),
        4_096
    );
    assert_eq!(prepared.arguments()["path"].as_str().unwrap().len(), 4_096);
    assert_eq!(
        prepared.arguments()["include"].as_str().unwrap().len(),
        4_096
    );

    let over_pattern = "π".repeat(MAX_GREP_FILES_PATTERN_BYTES / "π".len() + 1);
    assert_invalid_pattern(
        tool.prepare(call(json!({"pattern": over_pattern})))
            .unwrap_err(),
    );
    let over_path = "λ".repeat(MAX_GREP_FILES_PATH_BYTES / "λ".len() + 1);
    assert_invalid_path(
        tool.prepare(call(json!({"pattern": "x", "path": over_path})))
            .unwrap_err(),
    );
    let over_include = format!("{}*", "i".repeat(MAX_GREP_FILES_INCLUDE_BYTES));
    assert_invalid_include(
        tool.prepare(call(json!({"pattern": "x", "include": over_include})))
            .unwrap_err(),
    );
}

#[test]
fn prepare_rejects_unsafe_paths_patterns_and_include_globs_without_effects() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("unchanged"), b"unchanged").unwrap();
    let tool = tool(temporary.path());
    for path in [
        "",
        "..",
        "a/../b",
        "/absolute",
        "nul\0path",
        "bidi\u{202e}path",
    ] {
        assert_invalid_path(
            tool.prepare(call(json!({"pattern": "x", "path": path})))
                .unwrap_err(),
        );
    }
    for pattern in ["", "nul\0pattern", "line\npattern", "bidi\u{202e}pattern"] {
        assert_invalid_pattern(tool.prepare(call(json!({"pattern": pattern}))).unwrap_err());
    }
    for include in [
        "",
        ".",
        "..",
        "a/../b",
        "/absolute",
        "nul\0glob",
        "bidi\u{202e}glob",
    ] {
        assert_invalid_include(
            tool.prepare(call(json!({"pattern": "x", "include": include})))
                .unwrap_err(),
        );
    }

    let prepared = tool
        .prepare(call(json!({"pattern": "x", "path": "missing//./nested"})))
        .unwrap();
    assert_eq!(prepared.arguments()["path"], "missing/nested");
    assert!(!temporary.path().join("missing").exists());
    assert_eq!(
        fs::read(temporary.path().join("unchanged")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn literal_matching_is_one_record_per_lf_line_retains_cr_and_uses_ascii_only_folding() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[
            (
                "a.txt",
                b"alpha\nNeedle and Needle\r\nNEEDLE\nStra\xc3\x9fe\n",
            ),
            ("b.txt", b"Needle in b"),
        ],
    );
    let tool = tool(temporary.path());

    let mut expected = common_result("Needle", ".", None, "matches", 2, 2);
    expected["matches"] = json!([
        {
            "path": "a.txt",
            "line_number": 2,
            "match_start_byte": 0,
            "excerpt_start_byte": 0,
            "line": "Needle and Needle\r",
            "line_truncated": false,
            "context_before": [],
            "context_after": [],
            "context_truncated": false
        },
        {
            "path": "b.txt",
            "line_number": 1,
            "match_start_byte": 0,
            "excerpt_start_byte": 0,
            "line": "Needle in b",
            "line_truncated": false,
            "context_before": [],
            "context_after": [],
            "context_truncated": false
        }
    ]);
    expected["total_matches"] = json!(2);
    expected["matching_files"] = json!(2);
    expected["truncated"] = json!(false);
    expected["next_offset"] = Value::Null;
    assert_eq!(matches(&tool, "Needle", "."), ToolOutput::success(expected));

    let mut folded = canonical("needle", ".", None, "count");
    folded["case_insensitive"] = json!(true);
    assert_eq!(run(&tool, folded).content["matching_lines"], 3);
    let mut ascii_only = canonical("STRASSE", ".", None, "count");
    ascii_only["case_insensitive"] = json!(true);
    assert_eq!(run(&tool, ascii_only).content["matching_lines"], 0);
    let mut non_ascii_exact = canonical("straße", ".", None, "count");
    non_ascii_exact["case_insensitive"] = json!(true);
    assert_eq!(run(&tool, non_ascii_exact).content["matching_lines"], 1);

    fs::write(temporary.path().join("literal.txt"), b"a.*?[x]{y}\\z").unwrap();
    assert_eq!(
        count(&tool, r".*?[x]{y}\z", ".").content["matching_lines"],
        1
    );
}

#[test]
fn matches_are_globally_path_then_line_sorted_with_exact_pagination_metadata() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[
            ("z.txt", b"needle z"),
            ("a.txt", b"zero\nneedle a2\none\nneedle a4"),
        ],
    );
    let tool = tool(temporary.path());

    let mut arguments = canonical("needle", ".", None, "matches");
    arguments["head_limit"] = json!(1);
    arguments["offset"] = json!(1);
    let output = run(&tool, arguments);
    assert_eq!(
        output.content,
        json!({
            "pattern": "needle",
            "path": ".",
            "include": null,
            "case_insensitive": false,
            "mode": "matches",
            "head_limit": 1,
            "offset": 1,
            "context_lines": 0,
            "candidate_files": 2,
            "searched_files": 2,
            "skipped_oversized_files": 0,
            "skipped_non_text_files": 0,
            "matches": [{
                "path": "a.txt",
                "line_number": 4,
                "match_start_byte": 0,
                "excerpt_start_byte": 0,
                "line": "needle a4",
                "line_truncated": false,
                "context_before": [],
                "context_after": [],
                "context_truncated": false
            }],
            "total_matches": 3,
            "matching_files": 2,
            "truncated": true,
            "next_offset": 2
        })
    );

    for (offset, expected_line, next_offset) in [(0, 2, Some(1)), (2, 1, None)] {
        let mut arguments = canonical("needle", ".", None, "matches");
        arguments["head_limit"] = json!(1);
        arguments["offset"] = json!(offset);
        let output = run(&tool, arguments);
        assert_eq!(output.content["matches"][0]["line_number"], expected_line);
        assert_eq!(output.content["next_offset"], json!(next_offset));
        assert_eq!(output.content["truncated"], true);
    }
}

#[test]
fn continuation_after_the_old_offset_ceiling_is_accepted_and_reusable() {
    let temporary = TemporaryDirectory::new();
    let matching_lines = 100_101_usize;
    fs::write(
        temporary.path().join("many-lines.txt"),
        "x\n".repeat(matching_lines),
    )
    .unwrap();
    let tool = tool(temporary.path());

    let mut first_arguments = canonical("x", "many-lines.txt", None, "matches");
    first_arguments["head_limit"] = json!(100);
    first_arguments["offset"] = json!(100_000);
    let first = run(&tool, first_arguments);
    assert_eq!(first.content["total_matches"], matching_lines);
    assert_eq!(first.content["matches"].as_array().unwrap().len(), 100);
    assert_eq!(first.content["matches"][0]["line_number"], 100_001);
    assert_eq!(first.content["matches"][99]["line_number"], 100_100);
    assert_eq!(first.content["next_offset"], 100_100);
    assert_eq!(first.content["truncated"], true);

    let continuation = first.content["next_offset"].as_u64().unwrap();
    let prepared = tool
        .prepare(call(json!({
            "pattern": "x",
            "path": "many-lines.txt",
            "mode": "matches",
            "head_limit": 100,
            "offset": continuation
        })))
        .unwrap();
    assert_eq!(prepared.arguments()["offset"], continuation);
    let last = run(&tool, prepared.arguments().clone());
    assert_eq!(last.content["matches"].as_array().unwrap().len(), 1);
    assert_eq!(last.content["matches"][0]["line_number"], 100_101);
    assert_eq!(last.content["next_offset"], Value::Null);
    assert_eq!(last.content["truncated"], true);
}

#[test]
fn files_with_matches_and_count_have_exact_distinct_shapes_and_complete_totals() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[
            ("z.txt", b"needle"),
            ("a.txt", b"needle\nneedle"),
            ("none.txt", b"nothing"),
        ],
    );
    let tool = tool(temporary.path());

    let mut arguments = canonical("needle", ".", None, "files_with_matches");
    arguments["head_limit"] = json!(1);
    arguments["offset"] = json!(1);
    arguments["context_lines"] = json!(5);
    assert_eq!(
        run(&tool, arguments).content,
        json!({
            "pattern": "needle",
            "path": ".",
            "include": null,
            "case_insensitive": false,
            "mode": "files_with_matches",
            "head_limit": 1,
            "offset": 1,
            "context_lines": 5,
            "candidate_files": 3,
            "searched_files": 3,
            "skipped_oversized_files": 0,
            "skipped_non_text_files": 0,
            "files": ["z.txt"],
            "matching_lines": 3,
            "total_files": 2,
            "truncated": true,
            "next_offset": null
        })
    );

    let mut arguments = canonical("needle", ".", None, "count");
    arguments["head_limit"] = json!(1);
    arguments["offset"] = json!(100_000);
    arguments["context_lines"] = json!(5);
    assert_eq!(
        run(&tool, arguments).content,
        json!({
            "pattern": "needle",
            "path": ".",
            "include": null,
            "case_insensitive": false,
            "mode": "count",
            "head_limit": 1,
            "offset": 100_000,
            "context_lines": 5,
            "candidate_files": 3,
            "searched_files": 3,
            "skipped_oversized_files": 0,
            "skipped_non_text_files": 0,
            "matching_lines": 3,
            "matching_files": 2
        })
    );
}

#[test]
fn include_uses_delivered_glob_grammar_recursively_and_filters_before_reads() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[
            ("root.rs", b"needle"),
            ("src/top.rs", b"needle"),
            ("src/deep/nested.rs", b"needle"),
            ("src/deep/note.txt", b"needle"),
            ("excluded.bin", b"\xffPRIVATE_EXCLUDED"),
        ],
    );
    let tool = tool(temporary.path());

    let cases = [
        ("src/*.rs", 1, 1),
        ("**/*.rs", 3, 3),
        ("*.txt", 1, 1),
        ("src/**/nested.rs", 1, 1),
    ];
    for (include, candidates, matching) in cases {
        let output = run(&tool, canonical("needle", ".", Some(include), "count"));
        assert_eq!(output.content["candidate_files"], candidates);
        assert_eq!(output.content["searched_files"], candidates);
        assert_eq!(output.content["matching_lines"], matching);
        assert_eq!(output.content["skipped_non_text_files"], 0);
    }
}

#[test]
fn hidden_files_and_files_below_hidden_directories_are_searchable_and_sorted() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[
            ("visible.txt", b"needle"),
            (".hidden.txt", b"needle"),
            (".hidden-directory/nested.txt", b"needle"),
        ],
    );
    let output = run(
        &tool(temporary.path()),
        canonical("needle", ".", Some("*.txt"), "files_with_matches"),
    );

    assert_eq!(
        output.content["files"],
        json!([".hidden-directory/nested.txt", ".hidden.txt", "visible.txt"])
    );
    assert_eq!(output.content["candidate_files"], 3);
    assert_eq!(output.content["searched_files"], 3);
    assert_eq!(output.content["matching_lines"], 3);
    assert_eq!(output.content["total_files"], 3);
}

#[test]
fn context_is_chronological_retains_cr_and_uses_exact_record_shapes() {
    let temporary = TemporaryDirectory::new();
    fs::write(
        temporary.path().join("context.txt"),
        b"zero\none\nneedle\r\ntwo\nthree\n",
    )
    .unwrap();
    let tool = tool(temporary.path());
    let mut arguments = canonical("needle", ".", None, "matches");
    arguments["context_lines"] = json!(2);
    let output = run(&tool, arguments);
    assert_eq!(
        output.content["matches"],
        json!([{
            "path": "context.txt",
            "line_number": 3,
            "match_start_byte": 0,
            "excerpt_start_byte": 0,
            "line": "needle\r",
            "line_truncated": false,
            "context_before": [
                {"line_number": 1, "line": "zero", "line_truncated": false},
                {"line_number": 2, "line": "one", "line_truncated": false}
            ],
            "context_after": [
                {"line_number": 4, "line": "two", "line_truncated": false},
                {"line_number": 5, "line": "three", "line_truncated": false}
            ],
            "context_truncated": false
        }])
    );
    assert_eq!(output.content["total_matches"], 1);
    assert_eq!(output.content["truncated"], false);
}

#[test]
fn pagination_selects_match_records_before_attaching_their_exact_context() {
    let temporary = TemporaryDirectory::new();
    fs::write(
        temporary.path().join("context-page.txt"),
        b"zero\nneedle one\nbetween\nneedle two\ntail\nneedle three\nend",
    )
    .unwrap();
    let tool = tool(temporary.path());
    let mut arguments = canonical("needle", ".", None, "matches");
    arguments["head_limit"] = json!(1);
    arguments["offset"] = json!(1);
    arguments["context_lines"] = json!(1);
    let output = run(&tool, arguments);

    assert_eq!(output.content["total_matches"], 3);
    assert_eq!(output.content["matching_files"], 1);
    assert_eq!(output.content["next_offset"], 2);
    assert_eq!(output.content["truncated"], true);
    assert_eq!(
        output.content["matches"],
        json!([{
            "path": "context-page.txt",
            "line_number": 4,
            "match_start_byte": 0,
            "excerpt_start_byte": 0,
            "line": "needle two",
            "line_truncated": false,
            "context_before": [{
                "line_number": 3,
                "line": "between",
                "line_truncated": false
            }],
            "context_after": [{
                "line_number": 5,
                "line": "tail",
                "line_truncated": false
            }],
            "context_truncated": false
        }])
    );
}

#[test]
fn excerpt_window_is_utf8_safe_bounded_and_contains_the_complete_first_match() {
    let temporary = TemporaryDirectory::new();
    let pattern = "n".repeat(MAX_GREP_FILES_PATTERN_BYTES);
    let contents = format!("{}{}{}", "λ".repeat(50), pattern, "ω".repeat(50));
    fs::write(temporary.path().join("long.txt"), contents).unwrap();
    let output = matches(&tool(temporary.path()), &pattern, ".");
    let record = &output.content["matches"][0];
    assert_eq!(record["match_start_byte"], 100);
    assert_eq!(record["excerpt_start_byte"], 100);
    assert_eq!(record["line"].as_str().unwrap(), pattern);
    assert_eq!(record["line"].as_str().unwrap().len(), 4_096);
    assert_eq!(record["line_truncated"], true);
    assert_eq!(record["context_truncated"], false);
}

#[test]
fn long_context_is_utf8_safe_and_aggregate_omission_sets_context_truncated() {
    let temporary = TemporaryDirectory::new();
    let long = "λ".repeat(MAX_GREP_FILES_RESULT_LINE_BYTES / 2 + 10);
    let contents = format!("{long}\n{long}\nneedle\n{long}\n{long}");
    fs::write(temporary.path().join("context.txt"), contents).unwrap();
    let tool = tool(temporary.path());
    let mut arguments = canonical("needle", ".", None, "matches");
    arguments["context_lines"] = json!(2);
    let output = run(&tool, arguments);
    let record = &output.content["matches"][0];
    assert_eq!(record["context_before"].as_array().unwrap().len(), 1);
    assert_eq!(record["context_after"].as_array().unwrap().len(), 0);
    for context in record["context_before"].as_array().unwrap() {
        assert_eq!(context["line"].as_str().unwrap().len(), 4_096);
        assert!(context["line"].as_str().unwrap().is_char_boundary(4_096));
        assert_eq!(context["line_truncated"], true);
    }
    assert_eq!(record["context_truncated"], true);
    assert_eq!(output.content["truncated"], false);
}

#[test]
fn eligibility_accepts_exact_file_limit_and_reports_oversized_utf8_and_nul_skips() {
    let temporary = TemporaryDirectory::new();
    let mut exact = vec![b'x'; MAX_GREP_FILES_FILE_BYTES];
    exact[..6].copy_from_slice(b"needle");
    fs::write(temporary.path().join("exact.txt"), exact).unwrap();
    fs::write(
        temporary.path().join("oversized.txt"),
        vec![b'x'; MAX_GREP_FILES_FILE_BYTES + 1],
    )
    .unwrap();
    fs::write(temporary.path().join("invalid.txt"), b"needle\xff").unwrap();
    fs::write(temporary.path().join("nul.txt"), b"needle\0tail").unwrap();
    let output = count(&tool(temporary.path()), "needle", ".");

    assert_eq!(
        output.content,
        json!({
            "pattern": "needle",
            "path": ".",
            "include": null,
            "case_insensitive": false,
            "mode": "count",
            "head_limit": 100,
            "offset": 0,
            "context_lines": 0,
            "candidate_files": 4,
            "searched_files": 1,
            "skipped_oversized_files": 1,
            "skipped_non_text_files": 2,
            "matching_lines": 1,
            "matching_files": 1
        })
    );
}

#[test]
fn one_large_then_many_empty_and_tiny_files_have_exact_output_without_false_scan_limit() {
    const TINY_FILE_COUNT: usize = 384;

    let temporary = TemporaryDirectory::new();
    let mut large = vec![b'x'; MAX_GREP_FILES_FILE_BYTES];
    large[MAX_GREP_FILES_FILE_BYTES - b"needle".len()..].copy_from_slice(b"needle");
    fs::write(temporary.path().join("000-large.txt"), large).unwrap();

    for index in 0..TINY_FILE_COUNT {
        let contents: &[u8] = match index % 4 {
            0 => b"",
            1 => b"x",
            2 => b"needle",
            3 => b"need",
            _ => unreachable!(),
        };
        fs::write(
            temporary.path().join(format!("tiny-{index:03}.txt")),
            contents,
        )
        .unwrap();
    }

    let output = count(&tool(temporary.path()), "needle", ".");
    let mut expected = common_result(
        "needle",
        ".",
        None,
        "count",
        TINY_FILE_COUNT + 1,
        TINY_FILE_COUNT + 1,
    );
    expected["matching_lines"] = json!(TINY_FILE_COUNT / 4 + 1);
    expected["matching_files"] = json!(TINY_FILE_COUNT / 4 + 1);
    assert_eq!(output, ToolOutput::success(expected));
}

#[test]
fn selected_regular_file_is_searchable_and_include_matches_its_workspace_relative_path() {
    let temporary = TemporaryDirectory::new();
    write_files(
        temporary.path(),
        &[
            ("nested/one.txt", b"needle\nneedle"),
            ("other.txt", b"needle"),
        ],
    );
    let tool = tool(temporary.path());

    let selected = count(&tool, "needle", "nested/one.txt");
    assert_eq!(selected.content["candidate_files"], 1);
    assert_eq!(selected.content["searched_files"], 1);
    assert_eq!(selected.content["matching_lines"], 2);
    assert_eq!(selected.content["matching_files"], 1);

    let included = run(
        &tool,
        canonical("needle", "nested/one.txt", Some("*.txt"), "count"),
    );
    assert_eq!(included.content["matching_lines"], 2);
    let excluded = run(
        &tool,
        canonical("needle", "nested/one.txt", Some("*.rs"), "count"),
    );
    assert_eq!(excluded.content["candidate_files"], 0);
    assert_eq!(excluded.content["searched_files"], 0);
}

#[test]
fn selected_regular_file_excluded_by_include_does_not_require_content_open_permission() {
    let temporary = TemporaryDirectory::new();
    let selected = temporary.path().join("excluded.txt");
    fs::write(&selected, b"PRIVATE_UNREADABLE_NEEDLE").unwrap();
    let tool = tool(temporary.path());
    fs::set_permissions(&selected, fs::Permissions::from_mode(0o000)).unwrap();
    let operating_system_enforces_mode = fs::File::open(&selected).is_err();

    let result = execute(
        &tool,
        canonical("PRIVATE", "excluded.txt", Some("*.rs"), "count"),
        CancellationToken::new(),
    );
    fs::set_permissions(&selected, fs::Permissions::from_mode(0o600)).unwrap();

    let output = result.unwrap_or_else(|error| {
        panic!(
            "an include-excluded selected file must not be content-opened; mode enforcement={operating_system_enforces_mode}: {error:?}"
        )
    });
    assert_eq!(output.content["candidate_files"], 0);
    assert_eq!(output.content["searched_files"], 0);
    assert_eq!(output.content["matching_lines"], 0);
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("PRIVATE_UNREADABLE_NEEDLE")
    );
}

#[test]
fn traversal_never_follows_symlinks_or_reads_fifos_sockets_and_other_specials() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    let secret = "PRIVATE_OUTSIDE_GREP_SECRET";
    fs::write(outside.path().join("secret.txt"), secret).unwrap();
    fs::write(temporary.path().join("regular.txt"), b"public").unwrap();
    symlink(
        outside.path().join("secret.txt"),
        temporary.path().join("outside-file-link"),
    )
    .unwrap();
    symlink(
        temporary.path().join("regular.txt"),
        temporary.path().join("inside-file-link"),
    )
    .unwrap();
    symlink(outside.path(), temporary.path().join("directory-link")).unwrap();
    symlink(
        outside.path().join("missing"),
        temporary.path().join("broken-link"),
    )
    .unwrap();
    create_fifo(&temporary.path().join("pipe"));
    let listener = UnixListener::bind(temporary.path().join("socket")).unwrap();
    let tool = tool(temporary.path());

    let output = count(&tool, "PRIVATE", ".");
    assert_eq!(output.content["candidate_files"], 1);
    assert_eq!(output.content["searched_files"], 1);
    assert_eq!(output.content["matching_lines"], 0);
    assert!(!serde_json::to_string(&output).unwrap().contains(secret));

    for path in [
        "outside-file-link",
        "inside-file-link",
        "directory-link",
        "broken-link",
        "pipe",
        "socket",
    ] {
        assert_tool_error(
            execute(
                &tool,
                canonical("x", path, None, "count"),
                CancellationToken::new(),
            )
            .unwrap_err(),
            ToolErrorKind::PermissionDenied,
            "grep_files_path_rejected",
            "requested path is not a confined regular file or directory",
            false,
        );
    }
    drop(listener);
}

#[test]
fn traversal_depth_exact_boundary_is_allowed_and_child_descent_fails_closed() {
    let temporary = TemporaryDirectory::new();
    let mut current = temporary.path().to_path_buf();
    for _ in 0..MAX_GREP_FILES_DEPTH {
        current.push("d");
        fs::create_dir(&current).unwrap();
    }
    fs::write(current.join("at-limit.txt"), b"needle").unwrap();
    let tool = tool(temporary.path());
    assert_eq!(count(&tool, "needle", ".").content["matching_lines"], 1);

    fs::create_dir(current.join("too-deep")).unwrap();
    assert_scan_limit(
        execute(
            &tool,
            canonical("needle", ".", None, "count"),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
}

#[test]
fn candidate_file_exact_boundary_succeeds_and_one_more_fails_without_partial_output() {
    let temporary = TemporaryDirectory::new();
    for index in 0..MAX_GREP_FILES_CANDIDATE_FILES {
        fs::write(temporary.path().join(format!("file-{index:05}.txt")), []).unwrap();
    }
    let tool = tool(temporary.path());
    let exact = count(&tool, "needle", ".");
    assert_eq!(exact.content["candidate_files"], 10_000);
    assert_eq!(exact.content["searched_files"], 10_000);
    assert_eq!(exact.content["matching_lines"], 0);

    fs::write(temporary.path().join("overflow.txt"), []).unwrap();
    assert_scan_limit(
        execute(
            &tool,
            canonical("needle", ".", None, "count"),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
}

#[test]
fn exact_four_kib_result_path_is_allowed_and_one_byte_more_is_scan_limit() {
    let temporary = TemporaryDirectory::new();
    let (directories, component, exact_name, overflow_name) =
        create_deep_result_fixture(temporary.path());
    let deepest = directories.last().unwrap();
    create_file_at(deepest, &exact_name, b"needle");
    let exact_path = format!("{}/{}", [component.as_str(); 16].join("/"), exact_name);
    let tool = tool(temporary.path());
    let output = matches(&tool, "needle", ".");
    assert_eq!(output.content["matches"][0]["path"], exact_path);

    create_file_at(deepest, &overflow_name, b"needle");
    for mode in ["matches", "files_with_matches", "count"] {
        assert_scan_limit(
            execute(
                &tool,
                canonical("needle", ".", None, mode),
                CancellationToken::new(),
            )
            .unwrap_err(),
        );
    }

    rustix::fs::unlinkat(deepest.as_fd(), &exact_name, AtFlags::empty()).unwrap();
    rustix::fs::unlinkat(deepest.as_fd(), &overflow_name, AtFlags::empty()).unwrap();
    for parent in directories.iter().take(16).rev() {
        rustix::fs::unlinkat(parent.as_fd(), &component, AtFlags::REMOVEDIR).unwrap();
    }
}

#[test]
fn every_constructed_empty_directory_path_obeys_the_exact_four_kib_boundary() {
    let temporary = TemporaryDirectory::new();
    let (directories, component, exact_name, overflow_name) =
        create_deep_result_fixture(temporary.path());
    let deepest = directories.last().unwrap();
    create_directory_at(deepest, &exact_name);
    let tool = tool(temporary.path());
    let exact = execute(
        &tool,
        canonical("needle", ".", None, "count"),
        CancellationToken::new(),
    );
    create_directory_at(deepest, &overflow_name);
    let overflow = execute(
        &tool,
        canonical("needle", ".", None, "count"),
        CancellationToken::new(),
    );
    remove_deep_fixture_entries(
        &directories,
        &component,
        &[
            (&exact_name, AtFlags::REMOVEDIR),
            (&overflow_name, AtFlags::REMOVEDIR),
        ],
    );

    let exact = exact.unwrap();
    assert_eq!(exact.content["candidate_files"], 0);
    assert_scan_limit(overflow.unwrap_err());
}

#[test]
fn every_constructed_include_excluded_file_path_obeys_the_exact_four_kib_boundary() {
    let temporary = TemporaryDirectory::new();
    let (directories, component, exact_name, overflow_name) =
        create_deep_result_fixture(temporary.path());
    let deepest = directories.last().unwrap();
    create_file_at(deepest, &exact_name, b"PRIVATE_EXCLUDED_NEEDLE");
    let tool = tool(temporary.path());
    let exact = execute(
        &tool,
        canonical("PRIVATE", ".", Some("*.rs"), "count"),
        CancellationToken::new(),
    );
    create_file_at(deepest, &overflow_name, b"PRIVATE_EXCLUDED_NEEDLE");
    let overflow = execute(
        &tool,
        canonical("PRIVATE", ".", Some("*.rs"), "count"),
        CancellationToken::new(),
    );
    remove_deep_fixture_entries(
        &directories,
        &component,
        &[
            (&exact_name, AtFlags::empty()),
            (&overflow_name, AtFlags::empty()),
        ],
    );

    let exact = exact.unwrap();
    assert_eq!(exact.content["candidate_files"], 0);
    assert_eq!(exact.content["searched_files"], 0);
    assert!(
        !serde_json::to_string(&exact)
            .unwrap()
            .contains("PRIVATE_EXCLUDED_NEEDLE")
    );
    assert_scan_limit(overflow.unwrap_err());
}

#[test]
fn every_constructed_symlink_or_special_path_obeys_the_exact_four_kib_boundary() {
    let temporary = TemporaryDirectory::new();
    let (directories, component, exact_name, overflow_name) =
        create_deep_result_fixture(temporary.path());
    let deepest = directories.last().unwrap();
    create_symlink_at(deepest, &exact_name);
    let tool = tool(temporary.path());
    let exact = execute(
        &tool,
        canonical("needle", ".", None, "count"),
        CancellationToken::new(),
    );
    create_symlink_at(deepest, &overflow_name);
    let overflow = execute(
        &tool,
        canonical("needle", ".", None, "count"),
        CancellationToken::new(),
    );
    remove_deep_fixture_entries(
        &directories,
        &component,
        &[
            (&exact_name, AtFlags::empty()),
            (&overflow_name, AtFlags::empty()),
        ],
    );

    let exact = exact.unwrap();
    assert_eq!(exact.content["candidate_files"], 0);
    assert_scan_limit(overflow.unwrap_err());
}

#[test]
fn head_and_aggregate_text_caps_emit_only_a_sorted_prefix_with_exact_resume_offset() {
    let temporary = TemporaryDirectory::new();
    let long_line = format!("needle{}", "x".repeat(MAX_GREP_FILES_RESULT_LINE_BYTES - 6));
    let contents = format!("{long_line}\n{long_line}\n{long_line}");
    fs::write(temporary.path().join("long.txt"), contents).unwrap();
    let tool = tool(temporary.path());
    let output = matches(&tool, "needle", ".");
    assert_eq!(output.content["matches"].as_array().unwrap().len(), 2);
    assert_eq!(
        output.content["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["line"].as_str().unwrap().len())
            .sum::<usize>(),
        MAX_GREP_FILES_TOTAL_RESULT_TEXT_BYTES
    );
    assert_eq!(output.content["total_matches"], 3);
    assert_eq!(output.content["truncated"], true);
    assert_eq!(output.content["next_offset"], 2);

    fs::write(
        temporary.path().join("many.txt"),
        (0..101)
            .map(|index| format!("needle {index:03}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let many = matches(&tool, "needle", "many.txt");
    assert_eq!(many.content["matches"].as_array().unwrap().len(), 100);
    assert_eq!(many.content["total_matches"], 101);
    assert_eq!(many.content["next_offset"], 100);
    assert_eq!(many.content["truncated"], true);
}

#[test]
fn aggregate_path_cap_is_exact_and_never_skips_a_result_to_fill_a_later_hole() {
    let temporary = TemporaryDirectory::new();
    for index in 0..40 {
        let prefix = format!("{index:03}-");
        let name = format!("{prefix}{}", "x".repeat(200 - prefix.len()));
        fs::write(temporary.path().join(name), b"needle").unwrap();
    }
    let exact_prefix = "040-";
    fs::write(
        temporary.path().join(format!(
            "{exact_prefix}{}",
            "x".repeat(192 - exact_prefix.len())
        )),
        b"needle",
    )
    .unwrap();
    fs::write(temporary.path().join("041-overflow"), b"needle").unwrap();
    fs::write(temporary.path().join("zzz-short"), b"needle").unwrap();
    let output = matches(&tool(temporary.path()), "needle", ".");
    let records = output.content["matches"].as_array().unwrap();
    assert_eq!(records.len(), 41);
    assert_eq!(
        records
            .iter()
            .map(|record| record["path"].as_str().unwrap().len())
            .sum::<usize>(),
        MAX_GREP_FILES_TOTAL_RESULT_PATH_BYTES
    );
    assert_eq!(records.last().unwrap()["path"].as_str().unwrap().len(), 192);
    assert_eq!(output.content["total_matches"], 43);
    assert_eq!(output.content["next_offset"], 41);
    assert_eq!(output.content["truncated"], true);
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("zzz-short")
    );
}

#[test]
fn escaped_json_output_is_valid_and_bounded_by_the_serialized_result_cap() {
    let temporary = TemporaryDirectory::new();
    let escaped = format!("needle{}", "\u{0001}".repeat(4_090));
    fs::write(
        temporary.path().join("escaped.txt"),
        format!("{escaped}\n{escaped}\n{escaped}"),
    )
    .unwrap();
    let output = matches(&tool(temporary.path()), "needle", ".");
    let serialized = serde_json::to_vec(&output).unwrap();
    assert!(serialized.len() <= MAX_GREP_FILES_SERIALIZED_RESULT_BYTES);
    let decoded: ToolOutput = serde_json::from_slice(&serialized).unwrap();
    assert_eq!(decoded, output);
    assert_eq!(output.content["matches"].as_array().unwrap().len(), 1);
    assert_eq!(output.content["next_offset"], 1);
    assert_eq!(output.content["truncated"], true);
}

#[test]
fn serialized_result_trimming_removes_context_before_the_match_record() {
    let temporary = TemporaryDirectory::new();
    let before = "\u{0001}".repeat(MAX_GREP_FILES_RESULT_LINE_BYTES);
    let after = "\u{0001}".repeat(
        MAX_GREP_FILES_TOTAL_RESULT_TEXT_BYTES - MAX_GREP_FILES_RESULT_LINE_BYTES - "needle".len(),
    );
    fs::write(
        temporary.path().join("escaped-context.txt"),
        format!("{before}\nneedle\n{after}"),
    )
    .unwrap();
    let tool = tool(temporary.path());
    let mut arguments = canonical("needle", ".", None, "matches");
    arguments["context_lines"] = json!(1);
    let output = run(&tool, arguments);
    let serialized = serde_json::to_vec(&output).unwrap();
    let record = &output.content["matches"][0];

    assert!(serialized.len() <= MAX_GREP_FILES_SERIALIZED_RESULT_BYTES);
    assert_eq!(output.content["matches"].as_array().unwrap().len(), 1);
    assert_eq!(output.content["total_matches"], 1);
    assert_eq!(output.content["next_offset"], Value::Null);
    assert_eq!(output.content["truncated"], false);
    assert_eq!(record["line"], "needle");
    assert_eq!(
        record["context_before"][0]["line"].as_str().unwrap().len(),
        MAX_GREP_FILES_RESULT_LINE_BYTES
    );
    assert_eq!(record["context_after"], json!([]));
    assert_eq!(record["context_truncated"], true);
}

#[test]
fn hostile_repeated_prefix_search_has_exact_complete_result_without_naive_work() {
    let temporary = TemporaryDirectory::new();
    let pattern = format!("{}b", "a".repeat(MAX_GREP_FILES_PATTERN_BYTES - 1));
    let contents = vec![b'a'; MAX_GREP_FILES_FILE_BYTES];
    for index in 0..64 {
        fs::write(
            temporary.path().join(format!("hostile-{index:02}.txt")),
            &contents,
        )
        .unwrap();
    }
    let output = count(&tool(temporary.path()), &pattern, ".");
    assert_eq!(output.content["candidate_files"], 64);
    assert_eq!(output.content["searched_files"], 64);
    assert_eq!(output.content["matching_lines"], 0);
    assert_eq!(output.content["matching_files"], 0);

    let folded_pattern = format!("{}B", "A".repeat(MAX_GREP_FILES_PATTERN_BYTES - 1));
    let mut folded_arguments = canonical(&folded_pattern, ".", None, "count");
    folded_arguments["case_insensitive"] = json!(true);
    let folded = run(&tool(temporary.path()), folded_arguments);
    assert_eq!(folded.content["candidate_files"], 64);
    assert_eq!(folded.content["searched_files"], 64);
    assert_eq!(folded.content["matching_lines"], 0);
    assert_eq!(folded.content["matching_files"], 0);
}

#[test]
fn invalid_entry_names_fail_closed_before_pattern_or_include_can_hide_them() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let names = [
        OsString::from_vec(b"PRIVATE_INVALID_\xff_NAME".to_vec()),
        OsString::from("PRIVATE_CONTROL_\t_NAME"),
        OsString::from("PRIVATE_BIDI_\u{202e}_NAME"),
    ];

    for name in names {
        let path = temporary.path().join(&name);
        match fs::write(&path, b"needle") {
            Ok(()) => {}
            Err(error) if name.to_str().is_none() => {
                assert!(
                    [libc::EILSEQ, libc::EINVAL]
                        .contains(&error.raw_os_error().expect("invalid name OS code"))
                );
                continue;
            }
            Err(error) => panic!("failed to create invalid-name fixture: {error}"),
        }
        let error = execute(
            &tool,
            canonical("DEFINITELY_NO_MATCH", ".", Some("*.rs"), "count"),
            CancellationToken::new(),
        )
        .unwrap_err();
        let display = error.to_string();
        let debug = format!("{error:?}");
        assert_tool_error(
            error,
            ToolErrorKind::Execution,
            "grep_files_invalid_entry_name",
            "requested content search contains an unsupported entry name",
            false,
        );
        if let Some(name) = name.to_str() {
            assert!(!display.contains(name));
            assert!(!debug.contains(name));
        }
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn missing_permission_and_generic_os_failures_use_fixed_redacted_taxonomy() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let missing = "PRIVATE_MISSING_GREP_ROOT";
    let error = execute(
        &tool,
        canonical("x", missing, None, "matches"),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert!(!format!("{error:?}").contains(missing));
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "grep_files_not_found",
        "requested search root is unavailable",
        false,
    );

    let too_long = format!("PRIVATE_{}", "x".repeat(300));
    assert_tool_error(
        execute(
            &tool,
            canonical("x", &too_long, None, "count"),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::Unavailable,
        "grep_files_unavailable",
        "requested content search is unavailable",
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
                canonical("x", "PRIVATE_INACCESSIBLE", None, "count"),
                CancellationToken::new(),
            )
            .unwrap_err(),
            ToolErrorKind::PermissionDenied,
            "grep_files_permission_denied",
            "requested search root cannot be searched",
            false,
        );
    }
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn retained_root_rename_and_replacement_cannot_redirect_content_reads() {
    let temporary = TemporaryDirectory::new();
    let original = temporary.path().join("workspace");
    let retained = temporary.path().join("retained-workspace");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("note.txt"), b"RETAINED_GREP_SENTINEL").unwrap();
    let tool = tool(&original);

    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(
        original.join("note.txt"),
        b"PRIVATE_REPLACEMENT_GREP_SENTINEL",
    )
    .unwrap();

    let output = count(&tool, "RETAINED_GREP", ".");
    assert_eq!(output.content["matching_lines"], 1);
    let serialized = serde_json::to_string(&output).unwrap();
    assert!(!serialized.contains("PRIVATE_REPLACEMENT"));
}

#[test]
fn removed_retained_root_is_fixed_retryable_unavailable_in_every_mode() {
    let temporary = TemporaryDirectory::new();
    let root = temporary.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let tool = tool(&root);
    fs::remove_dir(&root).unwrap();

    for mode in ["matches", "files_with_matches", "count"] {
        assert_tool_error(
            execute(
                &tool,
                canonical("x", ".", None, mode),
                CancellationToken::new(),
            )
            .unwrap_err(),
            ToolErrorKind::Unavailable,
            "grep_files_unavailable",
            "requested content search is unavailable",
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
        canonical("needle", ".", None, "count"),
        CancellationToken::new(),
    );
    fs::write(temporary.path().join("late.txt"), b"needle").unwrap();
    assert_eq!(
        poll_immediately_ready(future).unwrap().content["matching_lines"],
        1
    );

    let dropped = tool.execute(
        context(),
        canonical("needle", ".", None, "count"),
        CancellationToken::new(),
    );
    drop(dropped);
    fs::write(temporary.path().join("after-drop.txt"), b"unchanged").unwrap();
    assert_eq!(
        fs::read(temporary.path().join("after-drop.txt")).unwrap(),
        b"unchanged"
    );

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    assert_tool_error(
        execute(&tool, canonical("needle", ".", None, "count"), cancellation).unwrap_err(),
        ToolErrorKind::Cancelled,
        "grep_files_cancelled",
        "grep_files execution was cancelled",
        false,
    );
}

#[test]
fn first_poll_observes_preexecution_growth_removal_and_special_substitution_without_stale_reads() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("grown.txt"), b"needle").unwrap();
    fs::write(
        temporary.path().join("removed.txt"),
        b"PRIVATE_REMOVED_NEEDLE",
    )
    .unwrap();
    fs::write(
        temporary.path().join("special-substitution.txt"),
        b"PRIVATE_SUBSTITUTED_NEEDLE",
    )
    .unwrap();
    let tool = tool(temporary.path());
    let future = tool.execute(
        context(),
        canonical("NEEDLE", ".", None, "count"),
        CancellationToken::new(),
    );

    fs::write(
        temporary.path().join("grown.txt"),
        vec![b'x'; MAX_GREP_FILES_FILE_BYTES + 1],
    )
    .unwrap();
    fs::remove_file(temporary.path().join("removed.txt")).unwrap();
    fs::remove_file(temporary.path().join("special-substitution.txt")).unwrap();
    create_fifo(&temporary.path().join("special-substitution.txt"));

    let output = poll_immediately_ready(future).unwrap();
    assert_eq!(output.content["candidate_files"], 1);
    assert_eq!(output.content["searched_files"], 0);
    assert_eq!(output.content["skipped_oversized_files"], 1);
    assert_eq!(output.content["matching_lines"], 0);
    assert_eq!(output.content["matching_files"], 0);
    let serialized = serde_json::to_string(&output).unwrap();
    assert!(!serialized.contains("PRIVATE_REMOVED_NEEDLE"));
    assert!(!serialized.contains("PRIVATE_SUBSTITUTED_NEEDLE"));
}

#[test]
fn direct_execute_requires_exact_canonical_eight_field_arguments() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for invalid in [
        json!({"pattern": "x"}),
        json!({
            "pattern": "x", "path": ".", "include": null,
            "case_insensitive": false, "mode": "matches", "head_limit": 100,
            "offset": 0
        }),
        json!({
            "pattern": "x", "path": "./", "include": null,
            "case_insensitive": false, "mode": "matches", "head_limit": 100,
            "offset": 0, "context_lines": 0
        }),
        json!({
            "pattern": "x", "path": ".", "include": "./*.rs",
            "case_insensitive": false, "mode": "matches", "head_limit": 100,
            "offset": 0, "context_lines": 0
        }),
        json!({
            "pattern": "x", "path": ".", "include": null,
            "case_insensitive": false, "mode": "all", "head_limit": 100,
            "offset": 0, "context_lines": 0
        }),
        json!({
            "pattern": "x", "path": ".", "include": null,
            "case_insensitive": false, "mode": "matches", "head_limit": 100,
            "offset": 0, "context_lines": 0, "extra": true
        }),
    ] {
        assert_invalid_arguments(execute(&tool, invalid, CancellationToken::new()).unwrap_err());
    }
    assert_invalid_pattern(
        execute(
            &tool,
            canonical("bidi\u{202e}", ".", None, "count"),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_invalid_path(
        execute(
            &tool,
            canonical("x", "../secret", None, "count"),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_invalid_include(
        execute(
            &tool,
            canonical("x", ".", Some("../secret"), "count"),
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
        GrepFilesTool::open(Path::new("PRIVATE_RELATIVE_ROOT")).unwrap_err(),
        GrepFilesToolOpenErrorKind::InvalidRoot,
        "native grep_files workspace root is invalid",
        &["PRIVATE_RELATIVE_ROOT"],
    );
    let missing = temporary.path().join("PRIVATE_MISSING_ROOT");
    assert_open_error(
        GrepFilesTool::open(&missing).unwrap_err(),
        GrepFilesToolOpenErrorKind::Unavailable,
        "native grep_files workspace root is unavailable",
        &["PRIVATE_MISSING_ROOT"],
    );
    for path in [&root_file, &root_link] {
        assert_open_error(
            GrepFilesTool::open(path).unwrap_err(),
            GrepFilesToolOpenErrorKind::InvalidFileType,
            "native grep_files workspace root is not a directory",
            &[path.file_name().unwrap().to_str().unwrap()],
        );
    }
    let debug = format!("{:?}", tool(temporary.path()));
    assert_eq!(debug, "GrepFilesTool { .. }");
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));
}

fn assert_open_error(
    error: GrepFilesToolOpenError,
    kind: GrepFilesToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    let debug = format!("{error:?}");
    assert_eq!(
        debug,
        format!("GrepFilesToolOpenError {{ kind: {kind:?} }}")
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
            tool.prepare(call(json!({"pattern": "PRIVATE_PATTERN\n"})))
                .unwrap_err(),
            "PRIVATE_PATTERN",
        ),
        (
            execute(
                &tool,
                canonical("x", "PRIVATE_MISSING", None, "matches"),
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
