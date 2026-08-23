#![cfg(any(target_os = "linux", target_os = "macos"))]

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
    MAX_WRITE_FILE_CHUNK_BYTES, MAX_WRITE_FILE_CONTENT_BYTES, MAX_WRITE_FILE_PATH_BYTES,
    MAX_WRITE_FILE_PATH_COMPONENTS, MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES,
    MAX_WRITE_FILE_SERIALIZED_RESULT_BYTES, MAX_WRITE_FILE_TEMP_ATTEMPTS, WRITE_FILE_TOOL_NAME,
    WriteFileTool, WriteFileToolOpenError, WriteFileToolOpenErrorKind,
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
                .join(format!("mg-write-file-{}-{identifier}", std::process::id()));
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
        Poll::Pending => panic!("write_file execution unexpectedly returned a pending future"),
    }
}

fn tool(root: &Path) -> WriteFileTool {
    WriteFileTool::open(root).expect("temporary workspace root is valid")
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("write-file-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn call(arguments: Value) -> ToolCall {
    named_call(WRITE_FILE_TOOL_NAME, arguments)
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("write-file-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("write-file-incarnation").unwrap(),
        turn_id: TurnId::new("write-file-turn").unwrap(),
        call_id: ToolCallId::new("write-file-call").unwrap(),
    }
}

fn execute(
    tool: &WriteFileTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_immediately_ready(tool.execute(context(), arguments, cancellation))
}

fn write(tool: &WriteFileTool, path: &str, content: &str) -> Result<ToolOutput, ToolError> {
    execute(
        tool,
        json!({ "path": path, "content": content }),
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
        "write_file_invalid_arguments",
        "write_file arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "write_file_invalid_path",
        "write_file path is invalid",
        false,
    );
}

fn assert_content_too_large(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "write_file_content_too_large",
        "write_file content exceeds the supported size limit",
        false,
    );
}

fn assert_path_rejected(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::PermissionDenied,
        "write_file_path_rejected",
        "requested path is not a confined regular file target",
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

fn serialized_arguments(content: &str) -> Value {
    json!({ "path": "file.txt", "content": content })
}

fn content_with_exact_serialized_size(size: usize) -> String {
    let mut content = "\u{0001}".repeat(10_000);
    let current = serde_json::to_vec(&serialized_arguments(&content))
        .unwrap()
        .len();
    assert!(current <= size);
    content.push_str(&"x".repeat(size - current));
    assert_eq!(
        serde_json::to_vec(&serialized_arguments(&content))
            .unwrap()
            .len(),
        size
    );
    assert!(content.len() <= MAX_WRITE_FILE_CONTENT_BYTES);
    content
}

#[test]
fn exported_contract_limits_and_spec_are_exact() {
    assert_eq!(WRITE_FILE_TOOL_NAME, "write_file");
    assert_eq!(MAX_WRITE_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_WRITE_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_WRITE_FILE_CONTENT_BYTES, 48 * 1_024);
    assert_eq!(MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES, 64 * 1_024);
    assert_eq!(MAX_WRITE_FILE_CHUNK_BYTES, 8 * 1_024);
    assert_eq!(MAX_WRITE_FILE_TEMP_ATTEMPTS, 8);
    assert_eq!(MAX_WRITE_FILE_SERIALIZED_RESULT_BYTES, 16 * 1_024);
    assert_eq!(
        format!("{:?}", WriteFileToolOpenErrorKind::UnsupportedPlatform),
        "UnsupportedPlatform"
    );

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), WRITE_FILE_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Write one file within the configured workspace"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative file path"
                },
                "content": {
                    "type": "string",
                    "description": "UTF-8 content to write"
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    );
}

#[test]
fn prepare_requires_exact_name_two_required_strings_and_no_unknown_fields() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let invalid_calls = [
        named_call("other_tool", json!({ "path": "file.txt", "content": "x" })),
        call(json!({})),
        call(json!({ "path": "file.txt" })),
        call(json!({ "content": "x" })),
        call(json!({ "path": null, "content": "x" })),
        call(json!({ "path": 1, "content": "x" })),
        call(json!({ "path": "file.txt", "content": null })),
        call(json!({ "path": "file.txt", "content": 1 })),
        call(json!({ "path": "file.txt", "content": [] })),
        call(json!({ "path": "file.txt", "content": "x", "extra": true })),
        call(json!("file.txt")),
        call(json!(["file.txt", "x"])),
    ];

    for invalid_call in invalid_calls {
        assert_invalid_arguments(tool.prepare(invalid_call).unwrap_err());
    }
}

#[test]
fn prepare_normalizes_path_preserves_content_and_requests_exact_write_authority() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let content = "first\0second\nλ";
    let prepared = tool
        .prepare(call(json!({
            "path": "./src///./nested//file.rs",
            "content": content
        })))
        .unwrap();

    assert_eq!(
        prepared.capability(),
        &Capability::Filesystem {
            access: FilesystemAccess::Write,
            path: "src/nested/file.rs".to_owned(),
        }
    );
    assert_eq!(
        prepared.arguments(),
        &json!({ "path": "src/nested/file.rs", "content": content })
    );
}

#[test]
fn prepare_treats_backslashes_as_literal_unix_filename_bytes() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for path in [r"C:\x", r"directory\file.txt"] {
        let prepared = tool
            .prepare(call(json!({ "path": path, "content": "x" })))
            .unwrap();
        assert_eq!(
            prepared.capability(),
            &Capability::Filesystem {
                access: FilesystemAccess::Write,
                path: path.to_owned(),
            }
        );
        assert_eq!(
            prepared.arguments(),
            &json!({ "path": path, "content": "x" })
        );
    }
}

#[test]
fn prepare_enforces_exact_normalized_path_and_component_limits() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let prefix = "a/".repeat(MAX_WRITE_FILE_PATH_COMPONENTS - 1);
    let exact = format!(
        "{prefix}{}",
        "x".repeat(MAX_WRITE_FILE_PATH_BYTES - prefix.len())
    );
    assert_eq!(exact.len(), MAX_WRITE_FILE_PATH_BYTES);
    assert_eq!(exact.split('/').count(), MAX_WRITE_FILE_PATH_COMPONENTS);
    let prepared = tool
        .prepare(call(json!({ "path": exact, "content": "" })))
        .unwrap();
    assert_eq!(
        prepared.arguments()["path"].as_str().unwrap().len(),
        MAX_WRITE_FILE_PATH_BYTES
    );

    let over_bytes = format!(
        "{prefix}{}",
        "x".repeat(MAX_WRITE_FILE_PATH_BYTES - prefix.len() + 1)
    );
    assert_invalid_path(
        tool.prepare(call(json!({ "path": over_bytes, "content": "" })))
            .unwrap_err(),
    );
    let over_components = (0..=MAX_WRITE_FILE_PATH_COMPONENTS)
        .map(|_| "a")
        .collect::<Vec<_>>()
        .join("/");
    assert_invalid_path(
        tool.prepare(call(json!({ "path": over_components, "content": "" })))
            .unwrap_err(),
    );

    let dotted_exact = (0..MAX_WRITE_FILE_PATH_COMPONENTS)
        .map(|_| "a/./")
        .collect::<String>();
    let prepared = tool
        .prepare(call(json!({ "path": dotted_exact, "content": "" })))
        .unwrap();
    assert_eq!(
        prepared.arguments()["path"]
            .as_str()
            .unwrap()
            .split('/')
            .count(),
        MAX_WRITE_FILE_PATH_COMPONENTS
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
        "src/..",
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
        assert_invalid_path(
            tool.prepare(call(json!({ "path": path, "content": "x" })))
                .unwrap_err(),
        );
    }
}

#[test]
fn prepare_enforces_raw_and_serialized_argument_boundaries_with_exact_precedence() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let exact_raw = "x".repeat(MAX_WRITE_FILE_CONTENT_BYTES);
    let prepared = tool
        .prepare(call(json!({ "path": "file.txt", "content": exact_raw })))
        .unwrap();
    assert_eq!(
        prepared.arguments()["content"].as_str().unwrap().len(),
        MAX_WRITE_FILE_CONTENT_BYTES
    );

    assert_content_too_large(
        tool.prepare(call(json!({
            "path": "file.txt",
            "content": "x".repeat(MAX_WRITE_FILE_CONTENT_BYTES + 1)
        })))
        .unwrap_err(),
    );

    let exact_serialized =
        content_with_exact_serialized_size(MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES);
    let exact_value = serialized_arguments(&exact_serialized);
    assert_eq!(
        serde_json::to_vec(&exact_value).unwrap().len(),
        MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES
    );
    tool.prepare(call(exact_value)).unwrap();

    let mut over_serialized = exact_serialized;
    over_serialized.push('x');
    assert_invalid_arguments(
        tool.prepare(call(serialized_arguments(&over_serialized)))
            .unwrap_err(),
    );

    assert_invalid_path(
        tool.prepare(call(json!({
            "path": "../invalid",
            "content": "x".repeat(MAX_WRITE_FILE_CONTENT_BYTES + 1)
        })))
        .unwrap_err(),
    );
    assert_content_too_large(
        tool.prepare(call(json!({
            "path": "file.txt",
            "content": "\u{0001}".repeat(MAX_WRITE_FILE_CONTENT_BYTES + 1)
        })))
        .unwrap_err(),
    );
}

#[test]
fn prepare_is_effect_free_for_missing_parents_and_existing_targets() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("existing.txt"), b"unchanged").unwrap();
    let entries = directory_entries(temporary.path());
    let tool = tool(temporary.path());

    let prepared = tool
        .prepare(call(json!({
            "path": "missing//./nested/file.txt",
            "content": "new content"
        })))
        .unwrap();
    assert_eq!(prepared.arguments()["path"], "missing/nested/file.txt");
    assert_eq!(directory_entries(temporary.path()), entries);
    assert!(!temporary.path().join("missing").exists());
    assert_eq!(
        fs::read(temporary.path().join("existing.txt")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn execute_creates_exact_utf8_bytes_and_reports_byte_length_without_residue() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("nested")).unwrap();
    let content = "hello\0world\nUTF-8: λ\n";
    let tool = tool(temporary.path());

    let output = write(&tool, "nested/file.txt", content).unwrap();

    assert_eq!(
        output,
        ToolOutput::success(json!({
            "path": "nested/file.txt",
            "bytes_written": content.len()
        }))
    );
    assert_eq!(
        fs::read(temporary.path().join("nested/file.txt")).unwrap(),
        content.as_bytes()
    );
    assert_eq!(
        directory_entries(&temporary.path().join("nested")),
        [OsString::from("file.txt")]
    );
    assert!(serde_json::to_vec(&output).unwrap().len() <= MAX_WRITE_FILE_SERIALIZED_RESULT_BYTES);
}

#[test]
fn execute_accepts_the_exact_content_limit_across_bounded_write_chunks() {
    let temporary = TemporaryDirectory::new();
    let content = "λ".repeat(MAX_WRITE_FILE_CONTENT_BYTES / "λ".len());
    assert_eq!(content.len(), MAX_WRITE_FILE_CONTENT_BYTES);
    let tool = tool(temporary.path());

    let output = write(&tool, "exact.txt", &content).unwrap();

    assert_eq!(
        output,
        ToolOutput::success(json!({
            "path": "exact.txt",
            "bytes_written": MAX_WRITE_FILE_CONTENT_BYTES
        }))
    );
    assert_eq!(
        fs::read(temporary.path().join("exact.txt")).unwrap(),
        content.as_bytes()
    );
    assert_eq!(mode(&temporary.path().join("exact.txt")), 0o644);
}

#[test]
fn direct_execute_accepts_the_exact_serialized_argument_limit() {
    let temporary = TemporaryDirectory::new();
    let content = content_with_exact_serialized_size(MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES);
    let arguments = serialized_arguments(&content);
    assert_eq!(
        serde_json::to_vec(&arguments).unwrap().len(),
        MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES
    );

    let output = execute(&tool(temporary.path()), arguments, CancellationToken::new()).unwrap();

    assert_eq!(
        output,
        ToolOutput::success(json!({
            "path": "file.txt",
            "bytes_written": content.len()
        }))
    );
    assert_eq!(
        fs::read(temporary.path().join("file.txt")).unwrap(),
        content.as_bytes()
    );
}

#[test]
fn execute_accepts_empty_content_and_replaces_the_inode_even_when_bytes_are_identical() {
    let temporary = TemporaryDirectory::new();
    let empty = temporary.path().join("empty.txt");
    fs::write(&empty, b"old").unwrap();
    let tool = tool(temporary.path());
    assert_eq!(
        write(&tool, "empty.txt", "").unwrap(),
        ToolOutput::success(json!({ "path": "empty.txt", "bytes_written": 0 }))
    );
    assert_eq!(fs::read(&empty).unwrap(), b"");

    let identical = temporary.path().join("identical.txt");
    fs::write(&identical, b"same bytes").unwrap();
    let before = fs::metadata(&identical).unwrap().ino();
    write(&tool, "identical.txt", "same bytes").unwrap();
    let after = fs::metadata(&identical).unwrap().ino();
    assert_ne!(
        before, after,
        "an identical write must still replace the inode"
    );
    assert_eq!(fs::read(&identical).unwrap(), b"same bytes");
}

#[test]
fn replacement_has_atomic_old_fd_or_new_path_visibility_and_breaks_hard_link_identity() {
    let temporary = TemporaryDirectory::new();
    let target = temporary.path().join("target.txt");
    let hard_link = temporary.path().join("hard-link.txt");
    fs::write(&target, b"old contents").unwrap();
    fs::hard_link(&target, &hard_link).unwrap();
    let mut old_descriptor = File::open(&target).unwrap();
    let old_inode = fs::metadata(&target).unwrap().ino();
    let tool = tool(temporary.path());

    write(&tool, "target.txt", "new contents").unwrap();

    let mut observed_old = Vec::new();
    old_descriptor.read_to_end(&mut observed_old).unwrap();
    assert_eq!(observed_old, b"old contents");
    assert_eq!(fs::read(&target).unwrap(), b"new contents");
    assert_eq!(fs::read(&hard_link).unwrap(), b"old contents");
    assert_ne!(fs::metadata(&target).unwrap().ino(), old_inode);
    assert_eq!(fs::metadata(&hard_link).unwrap().ino(), old_inode);
}

#[test]
fn replacement_does_not_need_read_permission_on_the_existing_target() {
    let temporary = TemporaryDirectory::new();
    let target = temporary.path().join("unreadable.txt");
    fs::write(&target, b"PRIVATE_OLD_CONTENT").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o000)).unwrap();
    let tool = tool(temporary.path());

    let output = write(&tool, "unreadable.txt", "replacement").unwrap();

    assert_eq!(output.content["bytes_written"], "replacement".len());
    assert_eq!(mode(&target), 0o000);
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"replacement");
}

#[test]
fn new_file_mode_is_exact_0644_under_a_hostile_umask() {
    const CHILD_MARKER: &str = "MACHINE_GOD_WRITE_FILE_UMASK_CHILD";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let executable = std::env::current_exe().unwrap();
        let status = Command::new("sh")
            .arg("-c")
            .arg("umask 077; exec \"$1\" --exact new_file_mode_is_exact_0644_under_a_hostile_umask --nocapture")
            .arg("machine-god-write-file-umask")
            .arg(executable)
            .env(CHILD_MARKER, "1")
            .status()
            .expect("failed to execute isolated hostile-umask test process");
        assert!(status.success(), "hostile-umask child failed with {status}");
        return;
    }

    let temporary = TemporaryDirectory::new();
    write(&tool(temporary.path()), "new.txt", "content").unwrap();
    assert_eq!(mode(&temporary.path().join("new.txt")), 0o644);
}

#[test]
fn replacement_preserves_only_observed_rwx_bits_and_strips_special_bits() {
    let temporary = TemporaryDirectory::new();
    let target = temporary.path().join("mode.txt");
    fs::write(&target, b"old").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o6751)).unwrap();
    assert_eq!(mode(&target), 0o6751);

    write(&tool(temporary.path()), "mode.txt", "new").unwrap();

    assert_eq!(mode(&target), 0o751);
    assert_eq!(fs::read(&target).unwrap(), b"new");
}

#[test]
fn missing_parent_is_not_created_and_target_is_unchanged_on_failure() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("sentinel.txt"), b"unchanged").unwrap();
    let tool = tool(temporary.path());

    assert_tool_error(
        write(&tool, "missing/nested/file.txt", "content").unwrap_err(),
        ToolErrorKind::Unavailable,
        "write_file_not_found",
        "requested parent directory is unavailable",
        false,
    );
    assert!(!temporary.path().join("missing").exists());
    assert_eq!(
        fs::read(temporary.path().join("sentinel.txt")).unwrap(),
        b"unchanged"
    );
    assert_eq!(
        directory_entries(temporary.path()),
        [OsString::from("sentinel.txt")]
    );
}

#[test]
fn ancestor_and_final_symlinks_directories_fifos_and_sockets_are_rejected_without_escape() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    fs::write(outside.path().join("secret.txt"), b"OUTSIDE_SECRET").unwrap();
    fs::create_dir(temporary.path().join("directory")).unwrap();
    fs::write(temporary.path().join("regular.txt"), b"inside").unwrap();
    symlink("regular.txt", temporary.path().join("final-link")).unwrap();
    symlink(outside.path(), temporary.path().join("ancestor-link")).unwrap();
    create_fifo(&temporary.path().join("pipe"));
    let listener = UnixListener::bind(temporary.path().join("socket")).unwrap();
    let tool = tool(temporary.path());

    for path in [
        "directory",
        "final-link",
        "ancestor-link/secret.txt",
        "pipe",
        "socket",
    ] {
        assert_path_rejected(write(&tool, path, "replacement").unwrap_err());
    }

    assert_eq!(
        fs::read(outside.path().join("secret.txt")).unwrap(),
        b"OUTSIDE_SECRET"
    );
    assert_eq!(
        fs::read(temporary.path().join("regular.txt")).unwrap(),
        b"inside"
    );
    drop(listener);
}

#[test]
fn permission_and_unavailable_failures_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let inaccessible = temporary.path().join("PRIVATE_INACCESSIBLE_PARENT");
    fs::create_dir(&inaccessible).unwrap();
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o500)).unwrap();
    let probe = inaccessible.join("mode-probe");
    let operating_system_enforces_mode = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)
        .is_err();
    if probe.exists() {
        fs::remove_file(&probe).unwrap();
    }
    let result = write(
        &tool(temporary.path()),
        "PRIVATE_INACCESSIBLE_PARENT/file.txt",
        "PRIVATE_CONTENT",
    );
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o700)).unwrap();
    if operating_system_enforces_mode {
        assert_tool_error(
            result.unwrap_err(),
            ToolErrorKind::PermissionDenied,
            "write_file_permission_denied",
            "requested file cannot be written",
            false,
        );
    }

    let too_long = format!("PRIVATE_{}", "x".repeat(300));
    let error = write(&tool(temporary.path()), &too_long, "PRIVATE_CONTENT").unwrap_err();
    let display = error.to_string();
    let debug = format!("{error:?}");
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "write_file_unavailable",
        "requested file is unavailable",
        true,
    );
    for secret in [&too_long, "PRIVATE_CONTENT"] {
        assert!(!display.contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn retained_root_rename_and_path_replacement_cannot_redirect_the_write() {
    let temporary = TemporaryDirectory::new();
    let original = temporary.path().join("workspace");
    let retained = temporary.path().join("retained-workspace");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("target.txt"), b"retained old").unwrap();
    let tool = tool(&original);

    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("target.txt"), b"replacement sentinel").unwrap();
    write(&tool, "target.txt", "retained new").unwrap();

    assert_eq!(
        fs::read(retained.join("target.txt")).unwrap(),
        b"retained new"
    );
    assert_eq!(
        fs::read(original.join("target.txt")).unwrap(),
        b"replacement sentinel"
    );
}

#[test]
fn removed_retained_root_is_retryable_unavailable_and_cannot_write_the_replacement_path() {
    let temporary = TemporaryDirectory::new();
    let root = temporary.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let tool = tool(&root);
    fs::remove_dir(&root).unwrap();

    assert_tool_error(
        write(&tool, "new.txt", "content").unwrap_err(),
        ToolErrorKind::Unavailable,
        "write_file_unavailable",
        "requested file is unavailable",
        true,
    );
    assert!(!root.exists());
}

#[test]
fn execution_future_is_inert_until_polled_drop_detaches_nothing_and_pre_cancel_wins() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let future = tool.execute(
        context(),
        json!({ "path": "late.txt", "content": "late content" }),
        CancellationToken::new(),
    );
    assert!(!temporary.path().join("late.txt").exists());
    assert_eq!(
        poll_immediately_ready(future).unwrap(),
        ToolOutput::success(json!({ "path": "late.txt", "bytes_written": 12 }))
    );
    assert_eq!(
        fs::read(temporary.path().join("late.txt")).unwrap(),
        b"late content"
    );

    let dropped = tool.execute(
        context(),
        json!({ "path": "dropped.txt", "content": "must not appear" }),
        CancellationToken::new(),
    );
    drop(dropped);
    assert!(!temporary.path().join("dropped.txt").exists());

    fs::write(temporary.path().join("cancelled.txt"), b"unchanged").unwrap();
    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    assert_tool_error(
        execute(
            &tool,
            json!({ "path": "cancelled.txt", "content": "replacement" }),
            cancellation,
        )
        .unwrap_err(),
        ToolErrorKind::Cancelled,
        "write_file_cancelled",
        "write_file execution was cancelled",
        false,
    );
    assert_eq!(
        fs::read(temporary.path().join("cancelled.txt")).unwrap(),
        b"unchanged"
    );
    assert_eq!(
        directory_entries(temporary.path()),
        [OsString::from("cancelled.txt"), OsString::from("late.txt")]
    );
}

#[test]
fn direct_execute_requires_exact_canonical_arguments_and_enforces_limits() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for invalid in [
        json!({}),
        json!({ "path": "file.txt" }),
        json!({ "path": "file.txt", "content": 1 }),
        json!({ "path": "file.txt", "content": "x", "extra": true }),
        json!({ "path": "./file.txt", "content": "x" }),
        json!({ "path": "folder//file.txt", "content": "x" }),
        json!("file.txt"),
    ] {
        assert_invalid_arguments(execute(&tool, invalid, CancellationToken::new()).unwrap_err());
    }
    assert_invalid_path(write(&tool, "../secret", "x").unwrap_err());
    assert_content_too_large(
        write(
            &tool,
            "file.txt",
            &"x".repeat(MAX_WRITE_FILE_CONTENT_BYTES + 1),
        )
        .unwrap_err(),
    );
    let mut over_serialized =
        content_with_exact_serialized_size(MAX_WRITE_FILE_SERIALIZED_ARGUMENT_BYTES);
    over_serialized.push('x');
    assert_invalid_arguments(
        execute(
            &tool,
            serialized_arguments(&over_serialized),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert!(directory_entries(temporary.path()).is_empty());
}

#[test]
fn constructor_tool_and_error_debug_contracts_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let root_file = temporary.path().join("PRIVATE_WORKSPACE_FILE");
    let root_link = temporary.path().join("PRIVATE_WORKSPACE_LINK");
    fs::write(&root_file, b"not a directory").unwrap();
    symlink(temporary.path(), &root_link).unwrap();

    assert_open_error(
        WriteFileTool::open(Path::new("PRIVATE_RELATIVE_ROOT")).unwrap_err(),
        WriteFileToolOpenErrorKind::InvalidRoot,
        "native write_file workspace root is invalid",
        &["PRIVATE_RELATIVE_ROOT"],
    );
    let missing = temporary.path().join("PRIVATE_MISSING_ROOT");
    assert_open_error(
        WriteFileTool::open(&missing).unwrap_err(),
        WriteFileToolOpenErrorKind::Unavailable,
        "native write_file workspace root is unavailable",
        &["PRIVATE_MISSING_ROOT"],
    );
    for path in [&root_file, &root_link] {
        assert_open_error(
            WriteFileTool::open(path).unwrap_err(),
            WriteFileToolOpenErrorKind::InvalidFileType,
            "native write_file workspace root is not a directory",
            &[path.file_name().unwrap().to_str().unwrap()],
        );
    }
    let debug = format!("{:?}", tool(temporary.path()));
    assert_eq!(debug, "WriteFileTool { .. }");
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));
}

fn assert_open_error(
    error: WriteFileToolOpenError,
    kind: WriteFileToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    let debug = format!("{error:?}");
    assert_eq!(
        debug,
        format!("WriteFileToolOpenError {{ kind: {kind:?} }}")
    );
    for secret in forbidden {
        assert!(!error.to_string().contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn preparation_and_execution_errors_never_reflect_untrusted_path_or_content() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let private_path = "/PRIVATE_WRITE_PATH";
    let private_content = "PRIVATE_WRITE_CONTENT";
    for error in [
        tool.prepare(call(json!({
            "path": private_path,
            "content": private_content
        })))
        .unwrap_err(),
        write(&tool, "PRIVATE_MISSING/file.txt", private_content).unwrap_err(),
    ] {
        let display = error.to_string();
        let debug = format!("{error:?}");
        for secret in [private_path, private_content, "PRIVATE_MISSING"] {
            assert!(!display.contains(secret));
            assert!(!debug.contains(secret));
        }
    }
}
