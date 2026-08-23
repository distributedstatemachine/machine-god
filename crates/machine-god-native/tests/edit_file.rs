#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File};
use std::future::Future;
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
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
    EDIT_FILE_TOOL_NAME, EditFileTool, EditFileToolOpenError, EditFileToolOpenErrorKind,
    MAX_EDIT_FILE_CHUNK_BYTES, MAX_EDIT_FILE_EXISTING_BYTES, MAX_EDIT_FILE_MATCH_WORK_STEPS,
    MAX_EDIT_FILE_NEW_STRING_BYTES, MAX_EDIT_FILE_OLD_STRING_BYTES, MAX_EDIT_FILE_PATH_BYTES,
    MAX_EDIT_FILE_PATH_COMPONENTS, MAX_EDIT_FILE_RESULTING_BYTES,
    MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_EDIT_FILE_SERIALIZED_RESULT_BYTES,
    MAX_EDIT_FILE_TEMP_ATTEMPTS,
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
            let path = std::env::temp_dir()
                .join(format!("mg-edit-file-{}-{identifier}", std::process::id()));
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
        Poll::Pending => panic!("edit_file execution unexpectedly returned a pending future"),
    }
}

fn tool(root: &Path) -> EditFileTool {
    EditFileTool::open(root).expect("temporary workspace root is valid")
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("edit-file-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn call(arguments: Value) -> ToolCall {
    named_call(EDIT_FILE_TOOL_NAME, arguments)
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("edit-file-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("edit-file-incarnation").unwrap(),
        turn_id: TurnId::new("edit-file-turn").unwrap(),
        call_id: ToolCallId::new("edit-file-call").unwrap(),
    }
}

fn arguments(path: &str, old_string: &str, new_string: &str) -> Value {
    json!({ "path": path, "old_string": old_string, "new_string": new_string })
}

fn execute(
    tool: &EditFileTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_immediately_ready(tool.execute(context(), arguments, cancellation))
}

fn edit(
    tool: &EditFileTool,
    path: &str,
    old_string: &str,
    new_string: &str,
) -> Result<ToolOutput, ToolError> {
    execute(
        tool,
        arguments(path, old_string, new_string),
        CancellationToken::new(),
    )
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
        "edit_file_invalid_arguments",
        "edit_file arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "edit_file_invalid_path",
        "edit_file path is invalid",
        false,
    );
}

fn assert_text_too_large(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "edit_file_text_too_large",
        "edit_file text exceeds the supported size limit",
        false,
    );
}

fn assert_path_rejected(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::PermissionDenied,
        "edit_file_path_rejected",
        "requested path is not a confined regular file",
        false,
    );
}

fn directory_entries(path: &Path) -> Vec<OsString> {
    let mut entries = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn create_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("failed to invoke the POSIX mkfifo utility");
    assert!(status.success(), "mkfifo failed with {status}");
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

fn exact_serialized_arguments() -> Value {
    let escaped_old = "\u{0001}".repeat(10_000);
    let base = arguments("file.txt", &escaped_old, "y");
    let base_size = serde_json::to_vec(&base).unwrap().len();
    assert!(base_size < MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES);
    let new_string = format!(
        "y{}",
        "x".repeat(MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES - base_size)
    );
    let value = arguments("file.txt", &escaped_old, &new_string);
    assert_eq!(
        serde_json::to_vec(&value).unwrap().len(),
        MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES
    );
    assert!(new_string.len() <= MAX_EDIT_FILE_NEW_STRING_BYTES);
    value
}

#[test]
fn exported_contract_limits_and_spec_are_exact() {
    assert_eq!(EDIT_FILE_TOOL_NAME, "edit_file");
    assert_eq!(MAX_EDIT_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_EDIT_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_EDIT_FILE_OLD_STRING_BYTES, 48 * 1_024);
    assert_eq!(MAX_EDIT_FILE_NEW_STRING_BYTES, 48 * 1_024);
    assert_eq!(MAX_EDIT_FILE_SERIALIZED_ARGUMENT_BYTES, 64 * 1_024);
    assert_eq!(MAX_EDIT_FILE_EXISTING_BYTES, 48 * 1_024);
    assert_eq!(MAX_EDIT_FILE_RESULTING_BYTES, 48 * 1_024);
    assert_eq!(MAX_EDIT_FILE_CHUNK_BYTES, 8 * 1_024);
    assert_eq!(MAX_EDIT_FILE_MATCH_WORK_STEPS, 393_216);
    assert_eq!(MAX_EDIT_FILE_TEMP_ATTEMPTS, 8);
    assert_eq!(MAX_EDIT_FILE_SERIALIZED_RESULT_BYTES, 16 * 1_024);
    assert_eq!(
        format!("{:?}", EditFileToolOpenErrorKind::UnsupportedPlatform),
        "UnsupportedPlatform"
    );

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), EDIT_FILE_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Replace one exact text occurrence in an existing workspace file"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Workspace-relative file path"},
                "old_string": {"type": "string", "description": "Exact UTF-8 text to replace"},
                "new_string": {"type": "string", "description": "UTF-8 replacement text"}
            },
            "required": ["path", "old_string", "new_string"],
            "additionalProperties": false
        })
    );
}

#[test]
fn prepare_requires_exact_name_three_required_strings_and_no_unknown_fields() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let invalid_calls = [
        named_call("other_tool", arguments("file.txt", "a", "b")),
        call(json!({})),
        call(json!({ "path": "file.txt", "old_string": "a" })),
        call(json!({ "path": "file.txt", "new_string": "b" })),
        call(json!({ "old_string": "a", "new_string": "b" })),
        call(json!({ "path": null, "old_string": "a", "new_string": "b" })),
        call(json!({ "path": "file.txt", "old_string": 1, "new_string": "b" })),
        call(json!({ "path": "file.txt", "old_string": "a", "new_string": [] })),
        call(json!({ "path": "file.txt", "old_string": "a", "new_string": "b", "extra": true })),
        call(json!("file.txt")),
        call(json!(["file.txt", "a", "b"])),
    ];
    for invalid_call in invalid_calls {
        assert_invalid_arguments(tool.prepare(invalid_call).unwrap_err());
    }
}

#[test]
fn prepare_normalizes_path_preserves_text_and_requests_exact_edit_authority() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let old_string = "first\0second\nλ";
    let new_string = "replacement\0\nЖ";
    let prepared = tool
        .prepare(call(arguments(
            "./src///./nested//file.rs",
            old_string,
            new_string,
        )))
        .unwrap();
    assert_eq!(
        prepared.capability(),
        &Capability::Filesystem {
            access: FilesystemAccess::Edit,
            path: "src/nested/file.rs".to_owned(),
        }
    );
    assert_eq!(
        prepared.arguments(),
        &arguments("src/nested/file.rs", old_string, new_string)
    );
}

#[test]
fn prepare_treats_backslashes_spaces_and_unicode_as_literal_filename_bytes() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for path in [r"C:\x", r"directory\file.txt", "space name/λ.txt"] {
        let prepared = tool.prepare(call(arguments(path, "a", "b"))).unwrap();
        assert_eq!(prepared.arguments()["path"], path);
        assert_eq!(
            prepared.capability(),
            &Capability::Filesystem {
                access: FilesystemAccess::Edit,
                path: path.to_owned(),
            }
        );
    }
}

#[test]
fn prepare_enforces_exact_normalized_path_and_component_limits() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let prefix = "a/".repeat(MAX_EDIT_FILE_PATH_COMPONENTS - 1);
    let exact = format!(
        "{prefix}{}",
        "x".repeat(MAX_EDIT_FILE_PATH_BYTES - prefix.len())
    );
    assert_eq!(exact.len(), MAX_EDIT_FILE_PATH_BYTES);
    tool.prepare(call(arguments(&exact, "a", "b"))).unwrap();

    let over_bytes = format!(
        "{prefix}{}",
        "x".repeat(MAX_EDIT_FILE_PATH_BYTES - prefix.len() + 1)
    );
    assert_invalid_path(
        tool.prepare(call(arguments(&over_bytes, "a", "b")))
            .unwrap_err(),
    );
    let over_components = std::iter::repeat_n("a", MAX_EDIT_FILE_PATH_COMPONENTS + 1)
        .collect::<Vec<_>>()
        .join("/");
    assert_invalid_path(
        tool.prepare(call(arguments(&over_components, "a", "b")))
            .unwrap_err(),
    );
}

#[test]
fn prepare_rejects_unsafe_ambiguous_or_non_file_paths() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for path in [
        "",
        ".",
        "././",
        "..",
        "src/../secret",
        "/absolute",
        "nul\0path",
        "tab\tpath",
        "line\npath",
        "line\u{2028}separator",
        "paragraph\u{2029}separator",
        "arabic\u{061c}mark",
        "left-to-right\u{200e}mark",
        "right-to-left\u{200f}mark",
        "embedding\u{202a}text",
        "override\u{202e}text",
        "isolate\u{2066}text",
        "isolate\u{2069}text",
    ] {
        assert_invalid_path(tool.prepare(call(arguments(path, "a", "b"))).unwrap_err());
    }
}

#[test]
fn prepare_enforces_text_rules_raw_limits_and_serialized_boundary() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    assert_tool_error(
        tool.prepare(call(arguments("file.txt", "", "x")))
            .unwrap_err(),
        ToolErrorKind::InvalidInput,
        "edit_file_old_string_empty",
        "edit_file old_string must not be empty",
        false,
    );
    assert_tool_error(
        tool.prepare(call(arguments("file.txt", "same", "same")))
            .unwrap_err(),
        ToolErrorKind::InvalidInput,
        "edit_file_strings_identical",
        "edit_file old_string and new_string must differ",
        false,
    );

    tool.prepare(call(arguments(
        "file.txt",
        &"x".repeat(MAX_EDIT_FILE_OLD_STRING_BYTES),
        "",
    )))
    .unwrap();
    tool.prepare(call(arguments(
        "file.txt",
        "x",
        &"y".repeat(MAX_EDIT_FILE_NEW_STRING_BYTES),
    )))
    .unwrap();
    assert_text_too_large(
        tool.prepare(call(arguments(
            "file.txt",
            &"x".repeat(MAX_EDIT_FILE_OLD_STRING_BYTES + 1),
            "",
        )))
        .unwrap_err(),
    );
    assert_text_too_large(
        tool.prepare(call(arguments(
            "file.txt",
            "x",
            &"y".repeat(MAX_EDIT_FILE_NEW_STRING_BYTES + 1),
        )))
        .unwrap_err(),
    );

    let exact = exact_serialized_arguments();
    tool.prepare(call(exact.clone())).unwrap();
    let mut over = exact;
    let longer = format!("{}x", over["new_string"].as_str().unwrap());
    over["new_string"] = Value::String(longer);
    assert_invalid_arguments(tool.prepare(call(over)).unwrap_err());
}

#[test]
fn prepare_is_effect_free_for_missing_non_utf8_and_ambiguous_targets() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("invalid.txt"), [0xff, 0xfe]).unwrap();
    fs::write(temporary.path().join("ambiguous.txt"), b"aaa").unwrap();
    let entries = directory_entries(temporary.path());
    let tool = tool(temporary.path());
    for value in [
        arguments("missing/nested/file.txt", "a", "b"),
        arguments("invalid.txt", "a", "b"),
        arguments("ambiguous.txt", "aa", "b"),
    ] {
        tool.prepare(call(value)).unwrap();
    }
    assert_eq!(directory_entries(temporary.path()), entries);
    assert!(!temporary.path().join("missing").exists());
    assert_eq!(
        fs::read(temporary.path().join("ambiguous.txt")).unwrap(),
        b"aaa"
    );
}

#[test]
fn execute_replaces_beginning_middle_end_unicode_and_nul_exactly_once() {
    let temporary = TemporaryDirectory::new();
    let cases = [
        (
            "begin.txt",
            "old middle end",
            "old",
            "new",
            "new middle end",
        ),
        ("middle.txt", "begin old end", "old", "λ\0", "begin λ\0 end"),
        ("end.txt", "begin old", "old", "終", "begin 終"),
        (
            "nul.txt",
            "before\0old\0after",
            "old\0",
            "new\0",
            "before\0new\0after",
        ),
    ];
    let tool = tool(temporary.path());
    for (path, preimage, old_string, new_string, expected) in cases {
        fs::write(temporary.path().join(path), preimage).unwrap();
        let output = edit(&tool, path, old_string, new_string).unwrap();
        assert_eq!(
            output,
            ToolOutput::success(json!({"path": path, "bytes_written": expected.len()}))
        );
        assert_eq!(
            fs::read(temporary.path().join(path)).unwrap(),
            expected.as_bytes()
        );
        assert!(
            serde_json::to_vec(&output).unwrap().len() <= MAX_EDIT_FILE_SERIALIZED_RESULT_BYTES
        );
    }
}

#[test]
fn complete_match_can_be_deleted_to_an_empty_file() {
    let temporary = TemporaryDirectory::new();
    let target = temporary.path().join("target.txt");
    fs::write(&target, "λ\0complete").unwrap();
    let output = edit(&tool(temporary.path()), "target.txt", "λ\0complete", "").unwrap();
    assert_eq!(
        output,
        ToolOutput::success(json!({"path": "target.txt", "bytes_written": 0}))
    );
    assert_eq!(fs::read(&target).unwrap(), b"");
}

#[test]
fn zero_two_and_overlapping_matches_fail_without_mutation() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for (name, bytes, old_string, code, message) in [
        (
            "zero.txt",
            "abcdef",
            "missing",
            "edit_file_match_not_found",
            "old_string was not found",
        ),
        (
            "two.txt",
            "old--old",
            "old",
            "edit_file_match_ambiguous",
            "old_string occurs more than once",
        ),
        (
            "overlap.txt",
            "aaa",
            "aa",
            "edit_file_match_ambiguous",
            "old_string occurs more than once",
        ),
    ] {
        let path = temporary.path().join(name);
        fs::write(&path, bytes).unwrap();
        assert_tool_error(
            edit(&tool, name, old_string, "new").unwrap_err(),
            ToolErrorKind::Execution,
            code,
            message,
            false,
        );
        assert_eq!(fs::read(&path).unwrap(), bytes.as_bytes());
    }
}

#[test]
fn exact_existing_and_result_boundaries_are_accepted_and_one_over_rejected() {
    let temporary = TemporaryDirectory::new();
    let exact = "a".repeat(MAX_EDIT_FILE_EXISTING_BYTES);
    fs::write(temporary.path().join("exact.txt"), &exact).unwrap();
    let tool = tool(temporary.path());
    edit(&tool, "exact.txt", "a", "b").unwrap_err();
    let unique = format!("{}x", "a".repeat(MAX_EDIT_FILE_EXISTING_BYTES - 1));
    fs::write(temporary.path().join("exact.txt"), &unique).unwrap();
    let output = edit(&tool, "exact.txt", "x", "y").unwrap();
    assert_eq!(
        output.content["bytes_written"],
        MAX_EDIT_FILE_RESULTING_BYTES
    );

    let oversized = format!("{}x", "a".repeat(MAX_EDIT_FILE_EXISTING_BYTES));
    fs::write(temporary.path().join("oversized.txt"), &oversized).unwrap();
    assert_tool_error(
        edit(&tool, "oversized.txt", "x", "y").unwrap_err(),
        ToolErrorKind::InvalidInput,
        "edit_file_existing_too_large",
        "requested file exceeds the supported size limit",
        false,
    );

    let preimage = format!("{}x", "a".repeat(MAX_EDIT_FILE_RESULTING_BYTES - 1));
    fs::write(temporary.path().join("result-over.txt"), &preimage).unwrap();
    assert_tool_error(
        edit(&tool, "result-over.txt", "x", "yy").unwrap_err(),
        ToolErrorKind::Execution,
        "edit_file_result_too_large",
        "edited file exceeds the supported size limit",
        false,
    );
    assert_eq!(
        fs::read_to_string(temporary.path().join("result-over.txt")).unwrap(),
        preimage
    );
}

#[test]
fn invalid_utf8_missing_and_nonregular_targets_are_fixed_and_confined() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    fs::write(temporary.path().join("invalid.txt"), [0xff, b'a']).unwrap();
    fs::write(temporary.path().join("regular.txt"), b"old").unwrap();
    fs::write(outside.path().join("secret.txt"), b"outside old").unwrap();
    fs::create_dir(temporary.path().join("directory")).unwrap();
    symlink("regular.txt", temporary.path().join("final-link")).unwrap();
    symlink(outside.path(), temporary.path().join("ancestor-link")).unwrap();
    create_fifo(&temporary.path().join("pipe"));
    let listener = UnixListener::bind(temporary.path().join("socket")).unwrap();
    let tool = tool(temporary.path());

    assert_tool_error(
        edit(&tool, "invalid.txt", "a", "b").unwrap_err(),
        ToolErrorKind::InvalidInput,
        "edit_file_invalid_utf8",
        "requested file is not valid UTF-8",
        false,
    );
    assert_tool_error(
        edit(&tool, "missing.txt", "a", "b").unwrap_err(),
        ToolErrorKind::Unavailable,
        "edit_file_not_found",
        "requested file is unavailable",
        false,
    );
    for path in [
        "directory",
        "final-link",
        "ancestor-link/secret.txt",
        "pipe",
        "socket",
    ] {
        assert_path_rejected(edit(&tool, path, "old", "new").unwrap_err());
    }
    assert_eq!(
        fs::read(outside.path().join("secret.txt")).unwrap(),
        b"outside old"
    );
    assert_eq!(
        fs::read(temporary.path().join("regular.txt")).unwrap(),
        b"old"
    );
    drop(listener);
}

#[test]
fn replacement_preserves_only_rwx_replaces_inode_and_separates_old_views() {
    let temporary = TemporaryDirectory::new();
    let target = temporary.path().join("target.txt");
    let hard_link = temporary.path().join("hard-link.txt");
    fs::write(&target, b"old contents").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o6751)).unwrap();
    fs::hard_link(&target, &hard_link).unwrap();
    let mut old_descriptor = File::open(&target).unwrap();
    let old_inode = fs::metadata(&target).unwrap().ino();

    edit(&tool(temporary.path()), "target.txt", "old", "new").unwrap();

    let mut observed_old = Vec::new();
    old_descriptor.read_to_end(&mut observed_old).unwrap();
    assert_eq!(observed_old, b"old contents");
    assert_eq!(fs::read(&target).unwrap(), b"new contents");
    assert_eq!(fs::read(&hard_link).unwrap(), b"old contents");
    assert_ne!(fs::metadata(&target).unwrap().ino(), old_inode);
    assert_eq!(fs::metadata(&hard_link).unwrap().ino(), old_inode);
    assert_eq!(mode(&target), 0o751);
}

#[test]
fn retained_root_rename_and_path_replacement_cannot_redirect_the_edit() {
    let temporary = TemporaryDirectory::new();
    let original = temporary.path().join("workspace");
    let retained = temporary.path().join("retained-workspace");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("target.txt"), b"retained old").unwrap();
    let tool = tool(&original);
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("target.txt"), b"replacement old").unwrap();

    edit(&tool, "target.txt", "old", "new").unwrap();

    assert_eq!(
        fs::read(retained.join("target.txt")).unwrap(),
        b"retained new"
    );
    assert_eq!(
        fs::read(original.join("target.txt")).unwrap(),
        b"replacement old"
    );
}

#[test]
fn removed_retained_root_is_retryable_unavailable_and_cannot_touch_replacement_path() {
    let temporary = TemporaryDirectory::new();
    let root = temporary.path().join("workspace");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("target.txt"), b"old").unwrap();
    let tool = tool(&root);
    fs::remove_file(root.join("target.txt")).unwrap();
    fs::remove_dir(&root).unwrap();

    assert_tool_error(
        edit(&tool, "target.txt", "old", "new").unwrap_err(),
        ToolErrorKind::Unavailable,
        "edit_file_unavailable",
        "requested file is unavailable",
        true,
    );
    assert!(!root.exists());
}

#[test]
fn execution_future_is_inert_until_polled_drop_detaches_nothing_and_pre_cancel_wins() {
    let temporary = TemporaryDirectory::new();
    let target = temporary.path().join("target.txt");
    fs::write(&target, b"old content").unwrap();
    let tool = tool(temporary.path());
    let future = tool.execute(
        context(),
        arguments("target.txt", "old", "new"),
        CancellationToken::new(),
    );
    assert_eq!(fs::read(&target).unwrap(), b"old content");
    assert_eq!(
        poll_immediately_ready(future).unwrap(),
        ToolOutput::success(json!({"path": "target.txt", "bytes_written": 11}))
    );
    assert_eq!(fs::read(&target).unwrap(), b"new content");

    fs::write(&target, b"new content").unwrap();
    let dropped = tool.execute(
        context(),
        arguments("target.txt", "new", "dropped"),
        CancellationToken::new(),
    );
    drop(dropped);
    assert_eq!(fs::read(&target).unwrap(), b"new content");

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    assert_tool_error(
        execute(
            &tool,
            arguments("target.txt", "new", "cancelled"),
            cancellation,
        )
        .unwrap_err(),
        ToolErrorKind::Cancelled,
        "edit_file_cancelled",
        "edit_file execution was cancelled",
        false,
    );
    assert_eq!(fs::read(&target).unwrap(), b"new content");
    assert_eq!(
        directory_entries(temporary.path()),
        [OsString::from("target.txt")]
    );
}

#[test]
fn direct_execute_requires_exact_canonical_arguments_and_reapplies_all_limits() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("file.txt"), b"old").unwrap();
    let tool = tool(temporary.path());
    for invalid in [
        json!({}),
        json!({"path": "file.txt", "old_string": "old"}),
        json!({"path": "file.txt", "old_string": 1, "new_string": "new"}),
        json!({"path": "file.txt", "old_string": "old", "new_string": "new", "extra": true}),
        arguments("./file.txt", "old", "new"),
        arguments("folder//file.txt", "old", "new"),
        json!("file.txt"),
    ] {
        assert_invalid_arguments(execute(&tool, invalid, CancellationToken::new()).unwrap_err());
    }
    assert_invalid_path(edit(&tool, "../secret", "old", "new").unwrap_err());
    assert_text_too_large(
        edit(
            &tool,
            "file.txt",
            &"x".repeat(MAX_EDIT_FILE_OLD_STRING_BYTES + 1),
            "new",
        )
        .unwrap_err(),
    );
    let mut over = exact_serialized_arguments();
    let longer = format!("{}x", over["new_string"].as_str().unwrap());
    over["new_string"] = Value::String(longer);
    assert_invalid_arguments(execute(&tool, over, CancellationToken::new()).unwrap_err());
    assert_eq!(fs::read(temporary.path().join("file.txt")).unwrap(), b"old");
}

#[test]
fn constructor_tool_and_error_debug_contracts_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let root_file = temporary.path().join("PRIVATE_WORKSPACE_FILE");
    let root_link = temporary.path().join("PRIVATE_WORKSPACE_LINK");
    fs::write(&root_file, b"not a directory").unwrap();
    symlink(temporary.path(), &root_link).unwrap();
    assert_open_error(
        EditFileTool::open(Path::new("PRIVATE_RELATIVE_ROOT")).unwrap_err(),
        EditFileToolOpenErrorKind::InvalidRoot,
        "native edit_file workspace root is invalid",
        &["PRIVATE_RELATIVE_ROOT"],
    );
    let missing = temporary.path().join("PRIVATE_MISSING_ROOT");
    assert_open_error(
        EditFileTool::open(&missing).unwrap_err(),
        EditFileToolOpenErrorKind::Unavailable,
        "native edit_file workspace root is unavailable",
        &["PRIVATE_MISSING_ROOT"],
    );
    for path in [&root_file, &root_link] {
        assert_open_error(
            EditFileTool::open(path).unwrap_err(),
            EditFileToolOpenErrorKind::InvalidFileType,
            "native edit_file workspace root is not a directory",
            &[path.file_name().unwrap().to_str().unwrap()],
        );
    }
    let debug = format!("{:?}", tool(temporary.path()));
    assert_eq!(debug, "EditFileTool { .. }");
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));
}

fn assert_open_error(
    error: EditFileToolOpenError,
    kind: EditFileToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    assert!(error.source().is_none());
    let debug = format!("{error:?}");
    assert_eq!(debug, format!("EditFileToolOpenError {{ kind: {kind:?} }}"));
    for secret in forbidden {
        assert!(!error.to_string().contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn preparation_and_execution_errors_never_reflect_untrusted_path_or_text() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let private_path = "/PRIVATE_EDIT_PATH";
    let private_old = "PRIVATE_EDIT_OLD";
    let private_new = "PRIVATE_EDIT_NEW";
    for error in [
        tool.prepare(call(arguments(private_path, private_old, private_new)))
            .unwrap_err(),
        edit(&tool, "PRIVATE_MISSING/file.txt", private_old, private_new).unwrap_err(),
    ] {
        let display = error.to_string();
        let debug = format!("{error:?}");
        for secret in [private_path, private_old, private_new, "PRIVATE_MISSING"] {
            assert!(!display.contains(secret));
            assert!(!debug.contains(secret));
        }
    }
}

#[test]
fn operating_system_name_failure_is_bounded_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let too_long = format!("PRIVATE_{}", "x".repeat(300));
    let error = edit(&tool(temporary.path()), &too_long, "old", "new").unwrap_err();
    let display = error.to_string();
    let debug = format!("{error:?}");
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "edit_file_unavailable",
        "requested file is unavailable",
        true,
    );
    assert!(!display.contains(&too_long));
    assert!(!debug.contains(&too_long));
}

#[test]
fn exact_serialized_arguments_can_execute_directly() {
    let temporary = TemporaryDirectory::new();
    let value = exact_serialized_arguments();
    let old = value["old_string"].as_str().unwrap();
    fs::write(temporary.path().join("file.txt"), old).unwrap();
    let output = execute(
        &tool(temporary.path()),
        value.clone(),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(temporary.path().join("file.txt")).unwrap(),
        value["new_string"].as_str().unwrap()
    );
    assert_eq!(
        output.content["bytes_written"],
        value["new_string"].as_str().unwrap().len()
    );
}

#[test]
fn no_failure_path_leaves_a_staging_file() {
    let temporary = TemporaryDirectory::new();
    let target = temporary.path().join("target.txt");
    fs::write(&target, b"aaa").unwrap();
    let tool = tool(temporary.path());
    for (old, new) in [("missing", "new"), ("aa", "new")] {
        assert!(edit(&tool, "target.txt", old, new).is_err());
        assert_eq!(
            directory_entries(temporary.path()),
            [OsString::from("target.txt")]
        );
        assert_eq!(fs::read(&target).unwrap(), b"aaa");
    }
}
