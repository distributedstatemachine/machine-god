#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::error::Error;
use std::fs::{self, File};
use std::future::Future;
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    CancellationToken, Capability, Tool, ToolCall, ToolCallId, ToolContext, ToolError,
    ToolErrorKind, ToolName, ToolOutput,
};
use machine_god_native::{
    MAX_RENAME_FILE_PATH_BYTES, MAX_RENAME_FILE_PATH_COMPONENTS,
    MAX_RENAME_FILE_SERIALIZED_ARGUMENT_BYTES, MAX_RENAME_FILE_SERIALIZED_RESULT_BYTES,
    RENAME_FILE_TOOL_NAME, RenameFileTool, RenameFileToolOpenError, RenameFileToolOpenErrorKind,
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
                "mg-rename-file-{}-{identifier}",
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

fn poll_immediately_ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("rename_file unexpectedly yielded"),
    }
}

fn tool(root: &Path) -> RenameFileTool {
    RenameFileTool::open(root).unwrap()
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("rename-file-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn call(arguments: Value) -> ToolCall {
    named_call(RENAME_FILE_TOOL_NAME, arguments)
}

fn context() -> ToolContext {
    ToolContext {
        session_id: machine_god_core::SessionId::new("rename-session").unwrap(),
        session_incarnation_id: machine_god_core::SessionIncarnationId::new("rename-incarnation")
            .unwrap(),
        turn_id: machine_god_core::TurnId::new("rename-turn").unwrap(),
        call_id: ToolCallId::new("rename-file-call").unwrap(),
    }
}

fn arguments(old_path: &str, new_path: &str) -> Value {
    json!({"old_path": old_path, "new_path": new_path})
}

fn execute(
    tool: &RenameFileTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_immediately_ready(tool.execute(context(), arguments, cancellation))
}

fn rename(tool: &RenameFileTool, old_path: &str, new_path: &str) -> Result<ToolOutput, ToolError> {
    execute(
        tool,
        arguments(old_path, new_path),
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
    assert_eq!(error.kind, kind);
    assert_eq!(error.code, code);
    assert_eq!(error.message, message);
    assert_eq!(error.retryable, retryable);
    assert_eq!(error.to_string(), format!("{code}: {message}"));
    assert_eq!(
        format!("{error:?}"),
        format!(
            "ToolError {{ kind: {kind:?}, code: {code:?}, message: {message:?}, retryable: {retryable} }}"
        )
    );
    drop(error);
}

fn assert_invalid_arguments(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "rename_file_invalid_arguments",
        "rename_file arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "rename_file_invalid_path",
        "rename_file path is invalid",
        false,
    );
}

fn assert_path_rejected(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::PermissionDenied,
        "rename_file_path_rejected",
        "requested rename path is not confined",
        false,
    );
}

fn assert_not_found(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "rename_file_not_found",
        "rename source is unavailable",
        false,
    );
}

fn create_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("invoke mkfifo");
    assert!(status.success(), "mkfifo failed with {status}");
}

#[test]
fn exported_contract_limits_and_spec_are_exact() {
    assert_eq!(RENAME_FILE_TOOL_NAME, "rename_file");
    assert_eq!(MAX_RENAME_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_RENAME_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_RENAME_FILE_SERIALIZED_ARGUMENT_BYTES, 65_536);
    assert_eq!(MAX_RENAME_FILE_SERIALIZED_RESULT_BYTES, 16_384);

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), RENAME_FILE_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Rename one existing regular file to an absent path within the configured workspace"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "old_path": {
                    "type": "string",
                    "description": "Current workspace-relative regular-file path"
                },
                "new_path": {
                    "type": "string",
                    "description": "New workspace-relative file path"
                }
            },
            "required": ["old_path", "new_path"],
            "additionalProperties": false
        })
    );
}

#[test]
fn prepare_requires_exact_name_two_required_strings_and_no_unknown_fields() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for invalid in [
        json!(null),
        json!([]),
        json!({}),
        json!({"old_path": "old"}),
        json!({"new_path": "new"}),
        json!({"old_path": 1, "new_path": "new"}),
        json!({"old_path": "old", "new_path": false}),
        json!({"old_path": "old", "new_path": "new", "overwrite": false}),
    ] {
        assert_invalid_arguments(tool.prepare(call(invalid)).unwrap_err());
    }
    assert_invalid_arguments(
        tool.prepare(named_call("write_file", arguments("old", "new")))
            .unwrap_err(),
    );
}

#[test]
fn prepare_normalizes_both_paths_and_requests_exact_two_endpoint_authority() {
    let temporary = TemporaryDirectory::new();
    let prepared = tool(temporary.path())
        .prepare(call(arguments(
            "./old//literal\\ λ.txt",
            "./new//literal\\ λ.txt",
        )))
        .unwrap();
    assert_eq!(
        prepared.capability(),
        &Capability::FilesystemRename {
            old_path: "old/literal\\ λ.txt".to_owned(),
            new_path: "new/literal\\ λ.txt".to_owned(),
        }
    );
    assert_eq!(
        prepared.arguments(),
        &arguments("old/literal\\ λ.txt", "new/literal\\ λ.txt")
    );
}

#[test]
fn prepare_enforces_each_requested_canonical_and_component_bound() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let exact_old = "o".repeat(MAX_RENAME_FILE_PATH_BYTES);
    let exact_new = "n".repeat(MAX_RENAME_FILE_PATH_BYTES);
    let prepared = tool
        .prepare(call(arguments(&exact_old, &exact_new)))
        .unwrap();
    assert_eq!(prepared.arguments(), &arguments(&exact_old, &exact_new));

    for invalid in [
        arguments(&format!("{exact_old}x"), "new"),
        arguments("old", &format!("{exact_new}x")),
    ] {
        assert_invalid_path(tool.prepare(call(invalid)).unwrap_err());
    }

    let exact_components = (0..MAX_RENAME_FILE_PATH_COMPONENTS)
        .map(|index| format!("c{index}"))
        .collect::<Vec<_>>()
        .join("/");
    let different_exact = format!("{exact_components}-new");
    assert!(
        tool.prepare(call(arguments(&exact_components, &different_exact)))
            .is_ok()
    );
    let one_over = format!("{exact_components}/extra");
    for invalid in [arguments(&one_over, "new"), arguments("old", &one_over)] {
        assert_invalid_path(tool.prepare(call(invalid)).unwrap_err());
    }
}

#[test]
fn prepare_rejects_escapes_controls_roots_and_equal_canonical_paths() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for (old_path, new_path) in [
        ("", "new"),
        ("old", ""),
        ("/old", "new"),
        ("old", "/new"),
        ("../old", "new"),
        ("old", "new/../escape"),
        (".", "new"),
        ("old", "."),
        ("old\n", "new"),
        ("old", "new\u{202e}"),
    ] {
        assert_invalid_path(
            tool.prepare(call(arguments(old_path, new_path)))
                .unwrap_err(),
        );
    }
    for (old_path, new_path) in [("same", "same"), ("./same", "same/.")] {
        assert_invalid_arguments(
            tool.prepare(call(arguments(old_path, new_path)))
                .unwrap_err(),
        );
    }
}

#[test]
fn preparation_is_effect_free_for_every_runtime_shape() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("source"), b"source").unwrap();
    fs::write(temporary.path().join("destination"), b"destination").unwrap();
    let before = fs::read_dir(temporary.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    let tool = tool(temporary.path());
    for pair in [
        ("missing", "new"),
        ("source", "destination"),
        ("source", "missing/parent/new"),
    ] {
        assert!(tool.prepare(call(arguments(pair.0, pair.1))).is_ok());
    }
    let after = fs::read_dir(temporary.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(before, after);
    assert_eq!(
        fs::read(temporary.path().join("source")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(temporary.path().join("destination")).unwrap(),
        b"destination"
    );
}

#[test]
fn execute_renames_same_and_distinct_parent_regular_files_without_reading_content() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("source-parent")).unwrap();
    fs::create_dir(temporary.path().join("destination-parent")).unwrap();
    fs::write(temporary.path().join("same-old"), b"same parent").unwrap();
    let cross_source = temporary.path().join("source-parent/old");
    fs::write(&cross_source, b"private unread bytes").unwrap();
    fs::set_permissions(&cross_source, fs::Permissions::from_mode(0o000)).unwrap();
    let tool = tool(temporary.path());

    assert_eq!(
        rename(&tool, "same-old", "same-new").unwrap(),
        ToolOutput::success(json!({"old_path": "same-old", "new_path": "same-new"}))
    );
    assert_eq!(
        rename(&tool, "source-parent/old", "destination-parent/new").unwrap(),
        ToolOutput::success(json!({
            "old_path": "source-parent/old",
            "new_path": "destination-parent/new"
        }))
    );
    assert!(!temporary.path().join("same-old").exists());
    assert_eq!(
        fs::read(temporary.path().join("same-new")).unwrap(),
        b"same parent"
    );
    assert!(!cross_source.exists());
    let destination = temporary.path().join("destination-parent/new");
    assert_eq!(fs::metadata(&destination).unwrap().mode() & 0o777, 0);
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(destination).unwrap(), b"private unread bytes");
}

#[test]
fn destination_is_never_overwritten_and_missing_parents_are_never_created() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    fs::write(&source, b"source").unwrap();
    fs::write(&destination, b"destination").unwrap();
    assert_tool_error(
        rename(&tool(temporary.path()), "source", "destination").unwrap_err(),
        ToolErrorKind::Execution,
        "rename_file_destination_exists",
        "rename destination already exists",
        false,
    );
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert_eq!(fs::read(&destination).unwrap(), b"destination");

    assert!(rename(&tool(temporary.path()), "source", "missing/parent/new").is_err());
    assert!(!temporary.path().join("missing").exists());
    assert_eq!(fs::read(source).unwrap(), b"source");
}

#[test]
fn missing_source_and_ancestor_are_fixed_not_found() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for source in ["missing", "missing/ancestor/source"] {
        assert_not_found(rename(&tool, source, "new").unwrap_err());
    }
    assert!(fs::read_dir(temporary.path()).unwrap().next().is_none());
}

#[test]
fn all_source_and_destination_entry_types_are_rejected_without_escape() {
    let workspace = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    fs::write(outside.path().join("sentinel"), b"outside").unwrap();
    fs::write(workspace.path().join("source"), b"source").unwrap();
    fs::create_dir(workspace.path().join("directory")).unwrap();
    symlink(outside.path(), workspace.path().join("ancestor-link")).unwrap();
    symlink(
        outside.path().join("sentinel"),
        workspace.path().join("final-link"),
    )
    .unwrap();
    create_fifo(&workspace.path().join("fifo"));
    let listener = UnixListener::bind(workspace.path().join("socket")).unwrap();
    let tool = tool(workspace.path());

    for source in [
        "directory",
        "final-link",
        "fifo",
        "socket",
        "ancestor-link/sentinel",
    ] {
        assert_path_rejected(rename(&tool, source, "new").unwrap_err());
    }
    for destination in ["directory", "final-link", "fifo", "socket"] {
        assert_tool_error(
            rename(&tool, "source", destination).unwrap_err(),
            ToolErrorKind::Execution,
            "rename_file_destination_exists",
            "rename destination already exists",
            false,
        );
    }
    assert_path_rejected(rename(&tool, "source", "ancestor-link/new").unwrap_err());
    drop(listener);
    assert_eq!(
        fs::read(workspace.path().join("source")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(outside.path().join("sentinel")).unwrap(),
        b"outside"
    );
}

#[test]
fn file_identity_hard_links_and_open_descriptors_survive_the_move() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.path().join("source");
    let hard_link = temporary.path().join("hard-link");
    fs::write(&source, b"retained bytes").unwrap();
    fs::hard_link(&source, &hard_link).unwrap();
    let inode = fs::metadata(&source).unwrap().ino();
    let mut opened = File::open(&source).unwrap();

    rename(&tool(temporary.path()), "source", "destination").unwrap();

    let destination = temporary.path().join("destination");
    assert_eq!(fs::metadata(&destination).unwrap().ino(), inode);
    assert_eq!(fs::metadata(&hard_link).unwrap().ino(), inode);
    let mut bytes = Vec::new();
    opened.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"retained bytes");
    assert_eq!(fs::read(hard_link).unwrap(), b"retained bytes");
}

#[test]
fn retained_root_rename_replacement_and_removal_cannot_redirect_authority() {
    let temporary = TemporaryDirectory::new();
    let original = temporary.path().join("workspace");
    let retained = temporary.path().join("retained");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("source"), b"retained source").unwrap();
    let rename_tool = tool(&original);
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("source"), b"replacement source").unwrap();

    rename(&rename_tool, "source", "destination").unwrap();
    assert!(!retained.join("source").exists());
    assert_eq!(
        fs::read(retained.join("destination")).unwrap(),
        b"retained source"
    );
    assert_eq!(
        fs::read(original.join("source")).unwrap(),
        b"replacement source"
    );
    assert!(!original.join("destination").exists());

    let removed = temporary.path().join("removed-root");
    fs::create_dir(&removed).unwrap();
    let removed_tool = tool(&removed);
    fs::remove_dir(&removed).unwrap();
    fs::create_dir(&removed).unwrap();
    fs::write(removed.join("source"), b"replacement").unwrap();
    assert_tool_error(
        rename(&removed_tool, "source", "destination").unwrap_err(),
        ToolErrorKind::Unavailable,
        "rename_file_unavailable",
        "requested rename is unavailable",
        true,
    );
    assert_eq!(fs::read(removed.join("source")).unwrap(), b"replacement");
}

#[test]
fn execution_future_is_inert_until_polled_drop_is_effect_free_and_pre_cancel_wins() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("source"), b"source").unwrap();
    let tool = tool(temporary.path());
    let future = tool.execute(
        context(),
        arguments("source", "destination"),
        CancellationToken::new(),
    );
    assert!(temporary.path().join("source").exists());
    assert_eq!(
        poll_immediately_ready(future).unwrap(),
        ToolOutput::success(json!({"old_path": "source", "new_path": "destination"}))
    );

    fs::rename(
        temporary.path().join("destination"),
        temporary.path().join("source"),
    )
    .unwrap();
    let dropped = tool.execute(
        context(),
        arguments("source", "destination"),
        CancellationToken::new(),
    );
    drop(dropped);
    assert!(temporary.path().join("source").exists());
    assert!(!temporary.path().join("destination").exists());

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    assert_tool_error(
        execute(&tool, arguments("source", "destination"), cancellation).unwrap_err(),
        ToolErrorKind::Cancelled,
        "rename_file_cancelled",
        "rename_file execution was cancelled",
        false,
    );
    assert!(temporary.path().join("source").exists());
}

#[test]
fn direct_execute_requires_exact_canonical_arguments_and_reapplies_limits() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("source"), b"source").unwrap();
    let tool = tool(temporary.path());
    for invalid in [
        json!({}),
        json!({"old_path": "source"}),
        json!({"old_path": "source", "new_path": 1}),
        json!({"old_path": "source", "new_path": "destination", "extra": true}),
        arguments("./source", "destination"),
        arguments("source", "folder//destination"),
        arguments("source", "source"),
    ] {
        assert_invalid_arguments(execute(&tool, invalid, CancellationToken::new()).unwrap_err());
    }
    assert_invalid_path(
        execute(
            &tool,
            arguments(&"x".repeat(MAX_RENAME_FILE_PATH_BYTES + 1), "destination"),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_eq!(
        fs::read(temporary.path().join("source")).unwrap(),
        b"source"
    );
}

#[test]
fn constructor_tool_and_errors_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let root_file = temporary.path().join("PRIVATE_ROOT_FILE");
    let root_link = temporary.path().join("PRIVATE_ROOT_LINK");
    fs::write(&root_file, b"not a directory").unwrap();
    symlink(temporary.path(), &root_link).unwrap();
    assert_open_error(
        RenameFileTool::open(Path::new("PRIVATE_RELATIVE_ROOT")).unwrap_err(),
        RenameFileToolOpenErrorKind::InvalidRoot,
        "native rename_file workspace root is invalid",
        &["PRIVATE_RELATIVE_ROOT"],
    );
    let missing = temporary.path().join("PRIVATE_MISSING_ROOT");
    assert_open_error(
        RenameFileTool::open(&missing).unwrap_err(),
        RenameFileToolOpenErrorKind::Unavailable,
        "native rename_file workspace root is unavailable",
        &["PRIVATE_MISSING_ROOT"],
    );
    for path in [&root_file, &root_link] {
        assert_open_error(
            RenameFileTool::open(path).unwrap_err(),
            RenameFileToolOpenErrorKind::InvalidFileType,
            "native rename_file workspace root is not a directory",
            &[path.file_name().unwrap().to_str().unwrap()],
        );
    }
    let debug = format!("{:?}", tool(temporary.path()));
    assert_eq!(debug, "RenameFileTool { .. }");
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));
}

fn assert_open_error(
    error: RenameFileToolOpenError,
    kind: RenameFileToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    assert!(error.source().is_none());
    let debug = format!("{error:?}");
    assert_eq!(
        debug,
        format!("RenameFileToolOpenError {{ kind: {kind:?} }}")
    );
    for secret in forbidden {
        assert!(!error.to_string().contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn preparation_and_execution_errors_never_reflect_either_endpoint() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let errors = [
        tool.prepare(call(arguments(
            "/PRIVATE_OLD_ENDPOINT",
            "PRIVATE_NEW_ENDPOINT",
        )))
        .unwrap_err(),
        rename(&tool, "PRIVATE_MISSING_SOURCE", "PRIVATE_DESTINATION").unwrap_err(),
    ];
    for error in errors {
        let display = error.to_string();
        let debug = format!("{error:?}");
        for secret in [
            "PRIVATE_OLD_ENDPOINT",
            "PRIVATE_NEW_ENDPOINT",
            "PRIVATE_MISSING_SOURCE",
            "PRIVATE_DESTINATION",
        ] {
            assert!(!display.contains(secret));
            assert!(!debug.contains(secret));
        }
    }
}
