#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::symlink;
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
    MAX_READ_FILE_BYTES, MAX_READ_FILE_PATH_BYTES, READ_FILE_TOOL_NAME, ReadFileTool,
    ReadFileToolOpenError, ReadFileToolOpenErrorKind,
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
                .join(format!("mg-read-file-{}-{identifier}", std::process::id()));
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
        Poll::Pending => panic!("read_file execution unexpectedly returned a pending future"),
    }
}

fn tool(root: &Path) -> ReadFileTool {
    ReadFileTool::open(root).expect("temporary workspace root is valid")
}

fn tool_name(name: &str) -> ToolName {
    ToolName::new(name).expect("test tool name is valid")
}

fn call(arguments: Value) -> ToolCall {
    named_call(READ_FILE_TOOL_NAME, arguments)
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("read-file-call").unwrap(),
        name: tool_name(name),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("read-file-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("read-file-incarnation").unwrap(),
        turn_id: TurnId::new("read-file-turn").unwrap(),
        call_id: ToolCallId::new("read-file-call").unwrap(),
    }
}

fn execute(
    tool: &ReadFileTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_immediately_ready(tool.execute(context(), arguments, cancellation))
}

fn assert_tool_error(
    error: ToolError,
    kind: ToolErrorKind,
    code: &str,
    message: &str,
    retryable: bool,
) {
    let display = error.to_string();
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
}

fn assert_invalid_arguments(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "read_file_invalid_arguments",
        "read_file arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "read_file_invalid_path",
        "read_file path is invalid",
        false,
    );
}

fn assert_path_rejected(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::PermissionDenied,
        "read_file_path_rejected",
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

#[test]
fn exported_contract_and_spec_are_exact() {
    assert_eq!(READ_FILE_TOOL_NAME, "read_file");
    assert_eq!(MAX_READ_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_READ_FILE_BYTES, 8_192);

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), READ_FILE_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Read one UTF-8 file within the configured workspace"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative file path"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    );
}

#[test]
fn prepare_requires_the_exact_tool_name_and_sole_string_path() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let invalid_calls = [
        named_call("other_tool", json!({ "path": "file.txt" })),
        call(json!({})),
        call(json!({ "path": null })),
        call(json!({ "path": true })),
        call(json!({ "path": 1 })),
        call(json!({ "path": ["file.txt"] })),
        call(json!({ "path": { "nested": "file.txt" } })),
        call(json!({ "path": "file.txt", "extra": false })),
        call(json!("file.txt")),
        call(json!(["file.txt"])),
    ];

    for invalid_call in invalid_calls {
        assert_invalid_arguments(tool.prepare(invalid_call).unwrap_err());
    }
}

#[test]
fn prepare_normalizes_dot_and_repeated_separators_for_policy_and_execution() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let prepared = tool
        .prepare(call(json!({ "path": "./src///./nested//file.rs" })))
        .unwrap();

    assert_eq!(
        prepared
            .capability()
            .expect("read_file requires permission authority"),
        &Capability::Filesystem {
            access: FilesystemAccess::Read,
            path: "src/nested/file.rs".to_owned(),
        }
    );
    assert_eq!(
        prepared.arguments(),
        &json!({ "path": "src/nested/file.rs" })
    );
}

#[test]
fn prepare_treats_windows_looking_backslashes_as_literal_unix_filenames() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());

    for path in [r"C:\x", r"directory\file.txt"] {
        let prepared = tool
            .prepare(call(json!({ "path": path })))
            .expect("backslashes are literal filename bytes on supported Unix targets");
        assert_eq!(
            prepared
                .capability()
                .expect("read_file requires permission authority"),
            &Capability::Filesystem {
                access: FilesystemAccess::Read,
                path: path.to_owned(),
            }
        );
        assert_eq!(prepared.arguments(), &json!({ "path": path }));
    }
}

#[test]
fn prepare_accepts_the_exact_path_byte_limit() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let path = "x".repeat(MAX_READ_FILE_PATH_BYTES);
    let prepared = tool.prepare(call(json!({ "path": path }))).unwrap();

    assert_eq!(prepared.arguments()["path"].as_str().unwrap().len(), 4_096);
}

#[test]
fn prepare_rejects_unsafe_ambiguous_or_oversized_paths() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let cases = [
        String::new(),
        ".".to_owned(),
        "././".to_owned(),
        "..".to_owned(),
        "src/../secret".to_owned(),
        "src/..".to_owned(),
        "/absolute".to_owned(),
        "nul\0byte".to_owned(),
        "tab\tpath".to_owned(),
        "line\npath".to_owned(),
        "line\u{2028}separator".to_owned(),
        "paragraph\u{2029}separator".to_owned(),
        "arabic\u{061c}mark".to_owned(),
        "left-to-right\u{200e}mark".to_owned(),
        "right-to-left\u{200f}mark".to_owned(),
        "embedding\u{202a}text".to_owned(),
        "override\u{202e}text".to_owned(),
        "isolate\u{2066}text".to_owned(),
        "isolate\u{2069}text".to_owned(),
        "x".repeat(MAX_READ_FILE_PATH_BYTES + 1),
    ];

    for path in cases {
        let error = tool.prepare(call(json!({ "path": path }))).unwrap_err();
        assert_invalid_path(error);
    }
}

#[test]
fn prepare_of_a_missing_path_is_effect_free() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("existing.txt"), b"unchanged").unwrap();
    let tool = tool(temporary.path());
    let entries_before = directory_entries(temporary.path());

    let prepared = tool
        .prepare(call(json!({ "path": "missing/nested/file.txt" })))
        .unwrap();

    assert_eq!(
        prepared
            .capability()
            .expect("read_file requires permission authority"),
        &Capability::Filesystem {
            access: FilesystemAccess::Read,
            path: "missing/nested/file.txt".to_owned(),
        }
    );
    assert_eq!(
        prepared.arguments(),
        &json!({ "path": "missing/nested/file.txt" })
    );
    assert_eq!(directory_entries(temporary.path()), entries_before);
    assert!(!temporary.path().join("missing").exists());
    assert_eq!(
        fs::read(temporary.path().join("existing.txt")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn execute_returns_exact_utf8_content_without_modifying_the_workspace() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("src")).unwrap();
    let path = temporary.path().join("src/file.txt");
    let contents = "hello, world\nUTF-8: λ\n";
    fs::write(&path, contents.as_bytes()).unwrap();
    let entries_before = directory_entries(temporary.path());
    let tool = tool(temporary.path());

    let output = execute(
        &tool,
        json!({ "path": "src/file.txt" }),
        CancellationToken::new(),
    )
    .unwrap();

    assert_eq!(output, ToolOutput::success(json!({ "content": contents })));
    assert_eq!(fs::read(&path).unwrap(), contents.as_bytes());
    assert_eq!(directory_entries(temporary.path()), entries_before);
}

#[test]
fn execute_accepts_the_exact_file_limit_and_rejects_one_byte_more() {
    let temporary = TemporaryDirectory::new();
    let exact = "e".repeat(MAX_READ_FILE_BYTES);
    fs::write(temporary.path().join("exact.txt"), exact.as_bytes()).unwrap();
    fs::write(
        temporary.path().join("too-large.txt"),
        vec![b'x'; MAX_READ_FILE_BYTES + 1],
    )
    .unwrap();
    let tool = tool(temporary.path());

    assert_eq!(
        execute(
            &tool,
            json!({ "path": "exact.txt" }),
            CancellationToken::new(),
        )
        .unwrap(),
        ToolOutput::success(json!({ "content": exact }))
    );
    assert_tool_error(
        execute(
            &tool,
            json!({ "path": "too-large.txt" }),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::Execution,
        "read_file_too_large",
        "requested file exceeds the read limit",
        false,
    );
}

#[test]
fn execute_rejects_invalid_utf8_with_a_fixed_redacted_error() {
    let temporary = TemporaryDirectory::new();
    let private_path = "private-invalid-utf8.txt";
    fs::write(
        temporary.path().join(private_path),
        b"valid-prefix\xffsecret",
    )
    .unwrap();
    let tool = tool(temporary.path());

    let error = execute(
        &tool,
        json!({ "path": private_path }),
        CancellationToken::new(),
    )
    .unwrap_err();
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert_tool_error(
        error,
        ToolErrorKind::Execution,
        "read_file_not_utf8",
        "requested file is not valid UTF-8",
        false,
    );
    assert!(!debug.contains(private_path));
    assert!(!debug.contains("secret"));
    assert!(!display.contains(private_path));
    assert!(!display.contains("secret"));
}

#[test]
fn execute_classifies_missing_files_without_creating_paths() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());

    let error = execute(
        &tool,
        json!({ "path": "private/missing.txt" }),
        CancellationToken::new(),
    )
    .unwrap_err();

    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "read_file_not_found",
        "requested file is unavailable",
        false,
    );
    assert!(!temporary.path().join("private").exists());
}

#[test]
fn execute_rejects_directories_fifos_and_final_or_intermediate_symlinks() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("directory")).unwrap();
    fs::write(temporary.path().join("target.txt"), b"inside").unwrap();
    fs::write(outside.path().join("secret.txt"), b"outside secret").unwrap();
    create_fifo(&temporary.path().join("pipe"));
    symlink("target.txt", temporary.path().join("final-link")).unwrap();
    symlink(outside.path(), temporary.path().join("directory-link")).unwrap();
    let tool = tool(temporary.path());

    for path in [
        "directory",
        "pipe",
        "final-link",
        "directory-link/secret.txt",
    ] {
        let error = execute(&tool, json!({ "path": path }), CancellationToken::new()).unwrap_err();
        assert_path_rejected(error);
    }

    assert_eq!(
        fs::read(outside.path().join("secret.txt")).unwrap(),
        b"outside secret"
    );
}

#[test]
fn execute_observes_cancellation_before_opening_the_file() {
    let temporary = TemporaryDirectory::new();
    let path = temporary.path().join("file.txt");
    fs::write(&path, b"unchanged").unwrap();
    let tool = tool(temporary.path());
    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());

    assert_tool_error(
        execute(&tool, json!({ "path": "file.txt" }), cancellation).unwrap_err(),
        ToolErrorKind::Cancelled,
        "read_file_cancelled",
        "read_file execution was cancelled",
        false,
    );
    assert_eq!(fs::read(path).unwrap(), b"unchanged");
}

#[test]
fn direct_execute_rejects_malformed_and_non_normalized_arguments() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("file.txt"), b"private contents").unwrap();
    let tool = tool(temporary.path());

    for arguments in [
        json!({}),
        json!({ "path": 1 }),
        json!({ "path": "file.txt", "extra": true }),
        json!({ "path": "./file.txt" }),
        json!({ "path": "folder//file.txt" }),
    ] {
        let error = execute(&tool, arguments, CancellationToken::new()).unwrap_err();
        assert_invalid_arguments(error);
    }
    assert_eq!(
        fs::read(temporary.path().join("file.txt")).unwrap(),
        b"private contents"
    );
}

#[test]
fn direct_execute_preserves_lexical_path_validation() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());

    for path in ["", ".", "../secret", "/absolute", "nul\0byte"] {
        let error = execute(&tool, json!({ "path": path }), CancellationToken::new()).unwrap_err();
        assert_invalid_path(error);
    }
}

#[test]
fn constructor_rejects_relative_missing_file_and_final_symlink_roots() {
    let temporary = TemporaryDirectory::new();
    let root_file = temporary.path().join("private-workspace-file");
    let root_link = temporary.path().join("private-workspace-link");
    fs::write(&root_file, b"not a directory").unwrap();
    symlink(temporary.path(), &root_link).unwrap();

    let relative = ReadFileTool::open(Path::new("private-relative-root")).unwrap_err();
    assert_open_error(
        relative,
        ReadFileToolOpenErrorKind::InvalidRoot,
        "native read_file workspace root is invalid",
        &["private-relative-root"],
    );

    let missing_path = temporary.path().join("private-missing-root");
    let missing = ReadFileTool::open(&missing_path).unwrap_err();
    assert_open_error(
        missing,
        ReadFileToolOpenErrorKind::Unavailable,
        "native read_file workspace root is unavailable",
        &[
            "private-missing-root",
            temporary.path().to_string_lossy().as_ref(),
        ],
    );

    let file = ReadFileTool::open(&root_file).unwrap_err();
    assert_open_error(
        file,
        ReadFileToolOpenErrorKind::InvalidFileType,
        "native read_file workspace root is not a directory",
        &[
            "private-workspace-file",
            temporary.path().to_string_lossy().as_ref(),
        ],
    );

    let final_symlink = ReadFileTool::open(&root_link).unwrap_err();
    assert_open_error(
        final_symlink,
        ReadFileToolOpenErrorKind::InvalidFileType,
        "native read_file workspace root is not a directory",
        &[
            "private-workspace-link",
            temporary.path().to_string_lossy().as_ref(),
        ],
    );
}

#[test]
fn constructor_applies_no_follow_to_the_lexical_final_root_component() {
    let temporary = TemporaryDirectory::new();
    let real_root = temporary.path().join("private-real-workspace");
    let linked_root = temporary.path().join("private-linked-workspace");
    fs::create_dir(&real_root).unwrap();
    symlink(&real_root, &linked_root).unwrap();

    let linked_root = linked_root.to_str().expect("temporary paths are UTF-8");
    for spelling in [
        format!("{linked_root}/"),
        format!("{linked_root}//"),
        format!("{linked_root}/."),
    ] {
        assert_open_error(
            ReadFileTool::open(Path::new(&spelling)).unwrap_err(),
            ReadFileToolOpenErrorKind::InvalidFileType,
            "native read_file workspace root is not a directory",
            &["private-linked-workspace", &spelling],
        );
    }

    let real_root = real_root.to_str().expect("temporary paths are UTF-8");
    for spelling in [format!("{real_root}/"), format!("{real_root}/.")] {
        ReadFileTool::open(Path::new(&spelling))
            .expect("terminal separators and dot preserve a real root directory");
    }
    ReadFileTool::open(Path::new("/"))
        .expect("lexical root normalization must preserve the filesystem root");
}

fn assert_open_error(
    error: ReadFileToolOpenError,
    kind: ReadFileToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    let debug = format!("{error:?}");
    assert_eq!(debug, format!("ReadFileToolOpenError {{ kind: {kind:?} }}"));
    for secret in forbidden {
        assert!(!error.to_string().contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn tool_debug_is_fixed_and_omits_the_workspace_root() {
    let temporary = TemporaryDirectory::new();
    let private_root = temporary.path().join("private-workspace-root");
    fs::create_dir(&private_root).unwrap();

    let debug = format!("{:?}", tool(&private_root));

    assert_eq!(debug, "ReadFileTool { .. }");
    assert!(!debug.contains("private-workspace-root"));
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));
}

#[test]
fn preparation_errors_are_fixed_and_redact_untrusted_inputs() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let private_path = "/private-secret-path";

    let path_error = tool
        .prepare(call(json!({ "path": private_path })))
        .unwrap_err();
    let path_debug = format!("{path_error:?}");
    let path_display = path_error.to_string();
    assert_invalid_path(path_error);
    assert!(!path_debug.contains(private_path));
    assert!(!path_display.contains(private_path));

    let private_tool = "private_secret_tool";
    let arguments_error = tool
        .prepare(named_call(
            private_tool,
            json!({ "path": "private-secret-file" }),
        ))
        .unwrap_err();
    let arguments_debug = format!("{arguments_error:?}");
    let arguments_display = arguments_error.to_string();
    assert_invalid_arguments(arguments_error);
    for secret in [private_tool, "private-secret-file"] {
        assert!(!arguments_debug.contains(secret));
        assert!(!arguments_display.contains(secret));
    }
}
