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
    DELETE_FILE_TOOL_NAME, DeleteFileTool, DeleteFileToolOpenError, DeleteFileToolOpenErrorKind,
    MAX_DELETE_FILE_PATH_BYTES, MAX_DELETE_FILE_PATH_COMPONENTS,
    MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES,
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
                "mg-delete-file-{}-{identifier}",
                std::process::id()
            ));
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
        Poll::Pending => panic!("delete_file execution unexpectedly returned a pending future"),
    }
}

fn tool(root: &Path) -> DeleteFileTool {
    DeleteFileTool::open(root).expect("temporary workspace root is valid")
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("delete-file-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn call(arguments: Value) -> ToolCall {
    named_call(DELETE_FILE_TOOL_NAME, arguments)
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("delete-file-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("delete-file-incarnation").unwrap(),
        turn_id: TurnId::new("delete-file-turn").unwrap(),
        call_id: ToolCallId::new("delete-file-call").unwrap(),
    }
}

fn arguments(path: &str) -> Value {
    json!({ "path": path })
}

fn execute(
    tool: &DeleteFileTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_immediately_ready(tool.execute(context(), arguments, cancellation))
}

fn delete(tool: &DeleteFileTool, path: &str) -> Result<ToolOutput, ToolError> {
    execute(tool, arguments(path), CancellationToken::new())
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
        "delete_file_invalid_arguments",
        "delete_file arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "delete_file_invalid_path",
        "delete_file path is invalid",
        false,
    );
}

fn assert_path_rejected(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::PermissionDenied,
        "delete_file_path_rejected",
        "requested path is not a confined regular file or empty directory",
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

fn serialized_arguments_of_size(size: usize) -> Value {
    let base = json!({"path": "x", "padding": ""});
    let base_size = serde_json::to_vec(&base).unwrap().len();
    assert!(base_size <= size);
    let value = json!({"path": "x", "padding": "x".repeat(size - base_size)});
    assert_eq!(serde_json::to_vec(&value).unwrap().len(), size);
    value
}

#[test]
fn exported_contract_limits_and_spec_are_exact() {
    assert_eq!(DELETE_FILE_TOOL_NAME, "delete_file");
    assert_eq!(MAX_DELETE_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_DELETE_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES, 65_536);
    assert_eq!(MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES, 16_384);
    assert_eq!(
        format!("{:?}", DeleteFileToolOpenErrorKind::UnsupportedPlatform),
        "UnsupportedPlatform"
    );

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), DELETE_FILE_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Delete one regular file or empty directory within the configured workspace"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative file or empty-directory path"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    );
}

#[test]
fn prepare_requires_exact_name_one_required_string_and_no_unknown_fields() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for invalid_call in [
        named_call("other_tool", arguments("file.txt")),
        call(json!({})),
        call(json!({"path": null})),
        call(json!({"path": 1})),
        call(json!({"path": []})),
        call(json!({"path": "file.txt", "extra": true})),
        call(json!("file.txt")),
        call(json!(["file.txt"])),
    ] {
        assert_invalid_arguments(tool.prepare(invalid_call).unwrap_err());
    }
}

#[test]
fn prepare_normalizes_literal_names_and_requests_exact_delete_authority() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let prepared = tool
        .prepare(call(arguments("./src///./nested//file.rs")))
        .unwrap();
    assert_eq!(
        prepared.capability(),
        &Capability::Filesystem {
            access: FilesystemAccess::Delete,
            path: "src/nested/file.rs".to_owned(),
        }
    );
    assert_eq!(prepared.arguments(), &arguments("src/nested/file.rs"));

    for path in [r"C:\x", r"directory\file.txt", "space name/λ.txt"] {
        let prepared = tool.prepare(call(arguments(path))).unwrap();
        assert_eq!(prepared.arguments(), &arguments(path));
        assert_eq!(
            prepared.capability(),
            &Capability::Filesystem {
                access: FilesystemAccess::Delete,
                path: path.to_owned(),
            }
        );
    }
}

#[test]
fn prepare_enforces_requested_canonical_component_and_serialized_bounds() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());

    let exact_requested = format!("a{}", "/".repeat(MAX_DELETE_FILE_PATH_BYTES - 1));
    assert_eq!(exact_requested.len(), MAX_DELETE_FILE_PATH_BYTES);
    let prepared = tool.prepare(call(arguments(&exact_requested))).unwrap();
    assert_eq!(prepared.arguments(), &arguments("a"));
    assert_invalid_path(
        tool.prepare(call(arguments(&format!("{exact_requested}/"))))
            .unwrap_err(),
    );

    let prefix = "a/".repeat(MAX_DELETE_FILE_PATH_COMPONENTS - 1);
    let exact_canonical = format!(
        "{prefix}{}",
        "x".repeat(MAX_DELETE_FILE_PATH_BYTES - prefix.len())
    );
    assert_eq!(exact_canonical.len(), MAX_DELETE_FILE_PATH_BYTES);
    tool.prepare(call(arguments(&exact_canonical))).unwrap();
    assert_invalid_path(
        tool.prepare(call(arguments(&format!("{exact_canonical}x"))))
            .unwrap_err(),
    );
    let over_components = std::iter::repeat_n("a", MAX_DELETE_FILE_PATH_COMPONENTS + 1)
        .collect::<Vec<_>>()
        .join("/");
    assert_invalid_path(tool.prepare(call(arguments(&over_components))).unwrap_err());

    assert_invalid_arguments(
        tool.prepare(call(serialized_arguments_of_size(
            MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES,
        )))
        .unwrap_err(),
    );
    assert_invalid_arguments(
        tool.prepare(call(serialized_arguments_of_size(
            MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES + 1,
        )))
        .unwrap_err(),
    );
}

#[test]
fn prepare_rejects_root_escape_controls_and_ambiguous_paths() {
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
        "delete\u{007f}control",
        "delete\u{0085}control",
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
        assert_invalid_path(tool.prepare(call(arguments(path))).unwrap_err());
    }
}

#[test]
fn preparation_is_effect_free_for_all_runtime_target_shapes() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    fs::write(temporary.path().join("regular"), b"private content").unwrap();
    fs::create_dir(temporary.path().join("empty-directory")).unwrap();
    fs::create_dir(temporary.path().join("nonempty-directory")).unwrap();
    fs::write(
        temporary.path().join("nonempty-directory/child"),
        b"private child",
    )
    .unwrap();
    fs::write(outside.path().join("outside"), b"outside sentinel").unwrap();
    symlink(outside.path(), temporary.path().join("link")).unwrap();
    let before = directory_entries(temporary.path());
    let tool = tool(temporary.path());

    for path in [
        "regular",
        "empty-directory",
        "nonempty-directory",
        "missing/target",
        "link/outside",
    ] {
        tool.prepare(call(arguments(path))).unwrap();
    }

    assert_eq!(directory_entries(temporary.path()), before);
    assert_eq!(
        fs::read(outside.path().join("outside")).unwrap(),
        b"outside sentinel"
    );
    assert_eq!(
        fs::read(temporary.path().join("regular")).unwrap(),
        b"private content"
    );
}

#[test]
fn execute_deletes_regular_files_and_empty_directories_with_exact_result() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("nested")).unwrap();
    fs::write(temporary.path().join("nested/λ file"), b"unread content").unwrap();
    fs::set_permissions(
        temporary.path().join("nested/λ file"),
        fs::Permissions::from_mode(0o0),
    )
    .unwrap();
    fs::write(temporary.path().join(r"literal\name"), b"literal").unwrap();
    fs::create_dir(temporary.path().join("empty directory")).unwrap();
    let tool = tool(temporary.path());

    for path in ["nested/λ file", r"literal\name", "empty directory"] {
        let output = delete(&tool, path).unwrap();
        assert_eq!(output, ToolOutput::success(json!({"path": path})));
        assert!(
            serde_json::to_vec(&output).unwrap().len() <= MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES
        );
        assert!(!temporary.path().join(path).exists());
    }
    assert!(temporary.path().join("nested").is_dir());
}

#[test]
fn missing_targets_and_ancestors_are_fixed_not_found_without_creation() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for path in ["missing", "missing/ancestor/target"] {
        assert_tool_error(
            delete(&tool, path).unwrap_err(),
            ToolErrorKind::Unavailable,
            "delete_file_not_found",
            "requested path is unavailable",
            false,
        );
    }
    assert!(directory_entries(temporary.path()).is_empty());
}

#[test]
fn nonempty_directory_is_not_enumerated_or_deleted() {
    let temporary = TemporaryDirectory::new();
    let directory = temporary.path().join("nonempty");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("PRIVATE_CHILD_SENTINEL"), b"private").unwrap();

    assert_tool_error(
        delete(&tool(temporary.path()), "nonempty").unwrap_err(),
        ToolErrorKind::Execution,
        "delete_file_directory_not_empty",
        "requested directory is not empty",
        false,
    );
    assert_eq!(
        fs::read(directory.join("PRIVATE_CHILD_SENTINEL")).unwrap(),
        b"private"
    );
}

#[test]
fn symlinks_at_every_path_position_are_rejected_without_outside_changes() {
    let workspace = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    fs::create_dir(outside.path().join("nested")).unwrap();
    fs::write(outside.path().join("secret"), b"outside secret").unwrap();
    fs::write(outside.path().join("nested/secret"), b"nested outside").unwrap();
    fs::create_dir(workspace.path().join("real")).unwrap();
    symlink(outside.path(), workspace.path().join("first-link")).unwrap();
    symlink(outside.path(), workspace.path().join("real/deep-link")).unwrap();
    symlink(
        outside.path().join("secret"),
        workspace.path().join("final-link"),
    )
    .unwrap();
    let tool = tool(workspace.path());

    for path in [
        "first-link/secret",
        "real/deep-link/nested/secret",
        "final-link",
    ] {
        assert_path_rejected(delete(&tool, path).unwrap_err());
    }
    assert_eq!(
        fs::read(outside.path().join("secret")).unwrap(),
        b"outside secret"
    );
    assert_eq!(
        fs::read(outside.path().join("nested/secret")).unwrap(),
        b"nested outside"
    );
    assert!(workspace.path().join("first-link").is_symlink());
    assert!(workspace.path().join("final-link").is_symlink());
}

#[test]
fn fifo_socket_and_device_targets_are_rejected_without_blocking() {
    let temporary = TemporaryDirectory::new();
    let fifo = temporary.path().join("pipe");
    let socket = temporary.path().join("socket");
    create_fifo(&fifo);
    let listener = UnixListener::bind(&socket).unwrap();
    let tool = tool(temporary.path());

    for path in ["pipe", "socket"] {
        assert_path_rejected(delete(&tool, path).unwrap_err());
        assert!(temporary.path().join(path).exists());
    }
    drop(listener);

    let device_tool = DeleteFileTool::open(Path::new("/dev")).unwrap();
    assert_path_rejected(delete(&device_tool, "null").unwrap_err());
    assert!(Path::new("/dev/null").exists());
}

#[test]
fn permission_failure_is_fixed_and_redacted_when_modes_are_enforced() {
    let temporary = TemporaryDirectory::new();
    let inaccessible = temporary.path().join("PRIVATE_INACCESSIBLE_PARENT");
    fs::create_dir(&inaccessible).unwrap();
    let target = inaccessible.join("PRIVATE_DELETE_TARGET");
    let probe = inaccessible.join("mode-probe");
    fs::write(&target, b"private target").unwrap();
    fs::write(&probe, b"probe").unwrap();
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o500)).unwrap();
    let operating_system_enforces_mode = fs::remove_file(&probe).is_err();
    let result = delete(
        &tool(temporary.path()),
        "PRIVATE_INACCESSIBLE_PARENT/PRIVATE_DELETE_TARGET",
    );
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o700)).unwrap();

    if operating_system_enforces_mode {
        let error = result.unwrap_err();
        let display = error.to_string();
        let debug = format!("{error:?}");
        assert_tool_error(
            error,
            ToolErrorKind::PermissionDenied,
            "delete_file_permission_denied",
            "requested path cannot be deleted",
            false,
        );
        for secret in ["PRIVATE_INACCESSIBLE_PARENT", "PRIVATE_DELETE_TARGET"] {
            assert!(!display.contains(secret));
            assert!(!debug.contains(secret));
        }
        assert_eq!(fs::read(&target).unwrap(), b"private target");
    }
}

#[test]
fn retained_root_rename_and_path_replacement_cannot_redirect_deletion() {
    let temporary = TemporaryDirectory::new();
    let original = temporary.path().join("workspace");
    let retained = temporary.path().join("retained-workspace");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("target"), b"retained target").unwrap();
    let tool = tool(&original);
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("target"), b"replacement sentinel").unwrap();

    assert_eq!(
        delete(&tool, "target").unwrap(),
        ToolOutput::success(json!({"path": "target"}))
    );
    assert!(!retained.join("target").exists());
    assert_eq!(
        fs::read(original.join("target")).unwrap(),
        b"replacement sentinel"
    );
}

#[test]
fn removed_retained_root_is_retryable_unavailable_and_does_not_touch_replacement() {
    let temporary = TemporaryDirectory::new();
    let root = temporary.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let tool = tool(&root);
    fs::remove_dir(&root).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(root.join("target"), b"replacement sentinel").unwrap();

    assert_tool_error(
        delete(&tool, "target").unwrap_err(),
        ToolErrorKind::Unavailable,
        "delete_file_unavailable",
        "requested path is unavailable",
        true,
    );
    assert_eq!(
        fs::read(root.join("target")).unwrap(),
        b"replacement sentinel"
    );
}

#[test]
fn hard_links_and_open_file_and_directory_descriptors_survive_deletion() {
    let temporary = TemporaryDirectory::new();
    let target = temporary.path().join("target");
    let hard_link = temporary.path().join("hard-link");
    fs::write(&target, b"retained bytes").unwrap();
    fs::hard_link(&target, &hard_link).unwrap();
    let original_inode = fs::metadata(&target).unwrap().ino();
    let mut open_file = File::open(&target).unwrap();
    let empty_directory = temporary.path().join("empty");
    fs::create_dir(&empty_directory).unwrap();
    let open_directory = File::open(&empty_directory).unwrap();
    let tool = tool(temporary.path());

    delete(&tool, "target").unwrap();
    delete(&tool, "empty").unwrap();

    let mut bytes = Vec::new();
    open_file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"retained bytes");
    assert_eq!(fs::read(&hard_link).unwrap(), b"retained bytes");
    assert_eq!(fs::metadata(&hard_link).unwrap().ino(), original_inode);
    assert!(open_directory.metadata().unwrap().is_dir());
    assert!(!target.exists());
    assert!(!empty_directory.exists());
}

#[test]
fn execution_future_is_inert_until_polled_drop_is_effect_free_and_pre_cancel_wins() {
    let temporary = TemporaryDirectory::new();
    let target = temporary.path().join("target");
    fs::write(&target, b"delete me").unwrap();
    let tool = tool(temporary.path());
    let future = tool.execute(context(), arguments("target"), CancellationToken::new());
    assert_eq!(fs::read(&target).unwrap(), b"delete me");
    assert_eq!(
        poll_immediately_ready(future).unwrap(),
        ToolOutput::success(json!({"path": "target"}))
    );
    assert!(!target.exists());

    fs::write(&target, b"drop me later").unwrap();
    let dropped = tool.execute(context(), arguments("target"), CancellationToken::new());
    drop(dropped);
    assert_eq!(fs::read(&target).unwrap(), b"drop me later");

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    assert_tool_error(
        execute(&tool, arguments("target"), cancellation).unwrap_err(),
        ToolErrorKind::Cancelled,
        "delete_file_cancelled",
        "delete_file execution was cancelled",
        false,
    );
    assert_eq!(fs::read(&target).unwrap(), b"drop me later");
}

#[test]
fn direct_execute_requires_exact_canonical_arguments_and_reapplies_all_limits() {
    let temporary = TemporaryDirectory::new();
    let target = temporary.path().join("target");
    fs::write(&target, b"keep").unwrap();
    let tool = tool(temporary.path());
    for invalid in [
        json!({}),
        json!({"path": 1}),
        json!({"path": "target", "extra": true}),
        arguments("./target"),
        arguments("folder//target"),
        json!("target"),
    ] {
        assert_invalid_arguments(execute(&tool, invalid, CancellationToken::new()).unwrap_err());
    }
    assert_invalid_path(delete(&tool, "../target").unwrap_err());
    assert_invalid_path(delete(&tool, &"x".repeat(MAX_DELETE_FILE_PATH_BYTES + 1)).unwrap_err());
    assert_invalid_arguments(
        execute(
            &tool,
            serialized_arguments_of_size(MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES + 1),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_eq!(fs::read(&target).unwrap(), b"keep");
}

#[test]
fn constructor_tool_and_error_debug_contracts_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let root_file = temporary.path().join("PRIVATE_WORKSPACE_FILE");
    let root_link = temporary.path().join("PRIVATE_WORKSPACE_LINK");
    fs::write(&root_file, b"not a directory").unwrap();
    symlink(temporary.path(), &root_link).unwrap();
    assert_open_error(
        DeleteFileTool::open(Path::new("PRIVATE_RELATIVE_ROOT")).unwrap_err(),
        DeleteFileToolOpenErrorKind::InvalidRoot,
        "native delete_file workspace root is invalid",
        &["PRIVATE_RELATIVE_ROOT"],
    );
    let missing = temporary.path().join("PRIVATE_MISSING_ROOT");
    assert_open_error(
        DeleteFileTool::open(&missing).unwrap_err(),
        DeleteFileToolOpenErrorKind::Unavailable,
        "native delete_file workspace root is unavailable",
        &["PRIVATE_MISSING_ROOT"],
    );
    for path in [&root_file, &root_link] {
        assert_open_error(
            DeleteFileTool::open(path).unwrap_err(),
            DeleteFileToolOpenErrorKind::InvalidFileType,
            "native delete_file workspace root is not a directory",
            &[path.file_name().unwrap().to_str().unwrap()],
        );
    }
    let debug = format!("{:?}", tool(temporary.path()));
    assert_eq!(debug, "DeleteFileTool { .. }");
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));
}

fn assert_open_error(
    error: DeleteFileToolOpenError,
    kind: DeleteFileToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    assert!(error.source().is_none());
    let debug = format!("{error:?}");
    assert_eq!(
        debug,
        format!("DeleteFileToolOpenError {{ kind: {kind:?} }}")
    );
    for secret in forbidden {
        assert!(!error.to_string().contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn preparation_execution_and_os_errors_never_reflect_untrusted_paths() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let private_path = "/PRIVATE_DELETE_PATH";
    let errors = [
        tool.prepare(call(arguments(private_path))).unwrap_err(),
        delete(&tool, "PRIVATE_MISSING/target").unwrap_err(),
    ];
    for error in errors {
        let display = error.to_string();
        let debug = format!("{error:?}");
        for secret in [private_path, "PRIVATE_MISSING"] {
            assert!(!display.contains(secret));
            assert!(!debug.contains(secret));
        }
    }

    let too_long = format!("PRIVATE_{}", "x".repeat(300));
    let error = delete(&tool, &too_long).unwrap_err();
    let display = error.to_string();
    let debug = format!("{error:?}");
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "delete_file_unavailable",
        "requested path is unavailable",
        true,
    );
    assert!(!display.contains(&too_long));
    assert!(!debug.contains(&too_long));
}
