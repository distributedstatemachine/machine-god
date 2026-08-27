#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs::{self, FileTimes};
use std::future::Future;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, SystemTime};

use machine_god_core::{
    CancellationToken, Capability, FilesystemAccess, SessionId, SessionIncarnationId, Tool,
    ToolCall, ToolCallId, ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    FILE_INFO_TOOL_NAME, FileInfoTool, FileInfoToolOpenError, FileInfoToolOpenErrorKind,
    MAX_FILE_INFO_PATH_BYTES,
};
use rustix::fd::AsFd;
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
                .join(format!("mg-file-info-{}-{identifier}", std::process::id()));
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
        Poll::Pending => panic!("file_info execution unexpectedly returned a pending future"),
    }
}

fn tool(root: &Path) -> FileInfoTool {
    FileInfoTool::open(root).expect("temporary workspace root is valid")
}

fn tool_name(name: &str) -> ToolName {
    ToolName::new(name).expect("test tool name is valid")
}

fn call(arguments: Value) -> ToolCall {
    named_call(FILE_INFO_TOOL_NAME, arguments)
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("file-info-call").unwrap(),
        name: tool_name(name),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("file-info-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("file-info-incarnation").unwrap(),
        turn_id: TurnId::new("file-info-turn").unwrap(),
        call_id: ToolCallId::new("file-info-call").unwrap(),
    }
}

fn execute(
    tool: &FileInfoTool,
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
        "file_info_invalid_arguments",
        "file_info arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "file_info_invalid_path",
        "file_info path is invalid",
        false,
    );
}

fn assert_path_rejected(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::PermissionDenied,
        "file_info_path_rejected",
        "requested path is not confined to the workspace",
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

fn output_content(tool: &FileInfoTool, path: &str) -> Value {
    execute(tool, json!({ "path": path }), CancellationToken::new())
        .unwrap()
        .content
}

#[test]
fn exported_contract_and_spec_are_exact() {
    assert_eq!(FILE_INFO_TOOL_NAME, "file_info");
    assert_eq!(MAX_FILE_INFO_PATH_BYTES, 4_096);
    assert_eq!(
        format!("{:?}", FileInfoToolOpenErrorKind::UnsupportedPlatform),
        "UnsupportedPlatform"
    );

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), FILE_INFO_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Inspect metadata for one path within the configured workspace"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative path to inspect"
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
fn prepare_normalizes_root_dots_and_repeated_separators_for_exact_authority() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());

    for (requested, normalized) in [
        (".", "."),
        ("././", "."),
        ("./src///./nested//file.rs", "src/nested/file.rs"),
        (r"directory\file.txt", r"directory\file.txt"),
        (" leading and trailing ", " leading and trailing "),
    ] {
        let prepared = tool.prepare(call(json!({ "path": requested }))).unwrap();
        assert_eq!(
            prepared
                .capability()
                .expect("file_info requires permission authority"),
            &Capability::Filesystem {
                access: FilesystemAccess::Metadata,
                path: normalized.to_owned(),
            }
        );
        assert_eq!(prepared.arguments(), &json!({ "path": normalized }));
    }
}

#[test]
fn prepare_enforces_the_exact_utf8_byte_limit_and_rejects_one_more() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let exact = "λ".repeat(MAX_FILE_INFO_PATH_BYTES / "λ".len());
    let prepared = tool.prepare(call(json!({ "path": exact }))).unwrap();
    assert_eq!(
        prepared.arguments()["path"].as_str().unwrap().len(),
        MAX_FILE_INFO_PATH_BYTES
    );

    let oversized = "λ".repeat(MAX_FILE_INFO_PATH_BYTES / "λ".len() + 1);
    assert_invalid_path(
        tool.prepare(call(json!({ "path": oversized })))
            .unwrap_err(),
    );
}

#[test]
fn prepare_rejects_empty_parent_absolute_control_and_bidi_paths() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let cases = [
        String::new(),
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
    ];

    for path in cases {
        let error = tool.prepare(call(json!({ "path": path }))).unwrap_err();
        assert_invalid_path(error);
    }
}

#[test]
fn prepare_of_a_missing_path_is_effect_free_and_preserves_exact_arguments() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("existing.txt"), b"unchanged").unwrap();
    let tool = tool(temporary.path());
    let prepared = tool
        .prepare(call(json!({ "path": "missing//./nested/file.txt" })))
        .unwrap();

    assert_eq!(
        prepared
            .capability()
            .expect("file_info requires permission authority"),
        &Capability::Filesystem {
            access: FilesystemAccess::Metadata,
            path: "missing/nested/file.txt".to_owned(),
        }
    );
    assert_eq!(
        prepared.arguments(),
        &json!({ "path": "missing/nested/file.txt" })
    );
    assert!(!temporary.path().join("missing").exists());
    assert_eq!(
        fs::read(temporary.path().join("existing.txt")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn execute_reports_exact_regular_file_size_timestamp_and_extension() {
    let temporary = TemporaryDirectory::new();
    let path = temporary.path().join("archive.tar.gz");
    let contents = b"known-metadata";
    fs::write(&path, contents).unwrap();
    let modified = SystemTime::UNIX_EPOCH + Duration::new(1_700_000_123, 456_789_012);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    let tool = tool(temporary.path());

    assert_eq!(
        output_content(&tool, "archive.tar.gz"),
        json!({
            "path": "archive.tar.gz",
            "kind": "file",
            "size_bytes": contents.len(),
            "modified": {
                "unix_seconds": 1_700_000_123_i64,
                "nanoseconds": 456_789_012_u32
            },
            "extension": "gz"
        })
    );
}

#[test]
fn execute_preserves_pre_epoch_signed_seconds_and_normalized_nanoseconds() {
    let temporary = TemporaryDirectory::new();
    let path = temporary.path().join("pre-epoch.txt");
    fs::write(&path, b"timestamp").unwrap();
    let modified = SystemTime::UNIX_EPOCH - Duration::new(1, 876_543_211);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    let tool = tool(temporary.path());

    assert_eq!(
        output_content(&tool, "pre-epoch.txt")["modified"],
        json!({
            "unix_seconds": -2_i64,
            "nanoseconds": 123_456_789_u32,
        })
    );
}

#[test]
fn execute_classifies_directory_root_and_regular_file_extension_edges() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("folder.with-dot")).unwrap();
    for name in [".bashrc", ".config.json", "name.", "plain"] {
        fs::write(temporary.path().join(name), []).unwrap();
    }
    let tool = tool(temporary.path());

    for (path, extension) in [
        (".bashrc", Value::Null),
        (".config.json", json!("json")),
        ("name.", Value::Null),
        ("plain", Value::Null),
    ] {
        let output = output_content(&tool, path);
        assert_eq!(output["path"], path);
        assert_eq!(output["kind"], "file");
        assert_eq!(output["extension"], extension);
    }

    let directory = output_content(&tool, "folder.with-dot");
    assert_eq!(directory["path"], "folder.with-dot");
    assert_eq!(directory["kind"], "directory");
    assert_eq!(directory["extension"], Value::Null);

    let root = output_content(&tool, ".");
    assert_eq!(root["path"], ".");
    assert_eq!(root["kind"], "directory");
    assert_eq!(root["extension"], Value::Null);
    assert!(root["size_bytes"].is_u64());
    assert!(root["modified"]["unix_seconds"].is_i64());
    assert!(root["modified"]["nanoseconds"].is_u64());
}

#[test]
fn execute_reports_final_and_dangling_symlink_metadata_without_following() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    let secret = outside.path().join("outside-secret.txt");
    fs::write(&secret, b"outside secret contents").unwrap();
    symlink(&secret, temporary.path().join("target.link")).unwrap();
    symlink(
        outside.path().join("missing-target"),
        temporary.path().join("dangling.link"),
    )
    .unwrap();
    let tool = tool(temporary.path());

    for path in ["target.link", "dangling.link"] {
        let output = output_content(&tool, path);
        assert_eq!(output["path"], path);
        assert_eq!(output["kind"], "symlink");
        assert_eq!(output["extension"], Value::Null);
        assert!(output["size_bytes"].as_u64().unwrap() > 0);
    }
    assert_eq!(fs::read(secret).unwrap(), b"outside secret contents");
}

#[test]
fn execute_rejects_an_ancestor_symlink_without_reflecting_paths() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    fs::write(outside.path().join("PRIVATE_OUTSIDE_SECRET"), b"secret").unwrap();
    symlink(outside.path(), temporary.path().join("private-link")).unwrap();
    let tool = tool(temporary.path());
    let requested = "private-link/PRIVATE_OUTSIDE_SECRET";

    let error = execute(
        &tool,
        json!({ "path": requested }),
        CancellationToken::new(),
    )
    .unwrap_err();
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert_path_rejected(error);
    for secret in [requested, "PRIVATE_OUTSIDE_SECRET"] {
        assert!(!debug.contains(secret));
        assert!(!display.contains(secret));
    }
}

#[test]
fn execute_classifies_fifo_and_socket_as_other_without_blocking() {
    let temporary = TemporaryDirectory::new();
    create_fifo(&temporary.path().join("pipe"));
    let listener = UnixListener::bind(temporary.path().join("socket")).unwrap();
    let tool = tool(temporary.path());

    for path in ["pipe", "socket"] {
        let output = output_content(&tool, path);
        assert_eq!(output["path"], path);
        assert_eq!(output["kind"], "other");
        assert_eq!(output["extension"], Value::Null);
    }
    drop(listener);
}

#[test]
fn metadata_for_an_unreadable_regular_file_succeeds_when_the_mode_is_enforced() {
    let temporary = TemporaryDirectory::new();
    let path = temporary.path().join("unreadable.txt");
    fs::write(&path, b"contents need not be readable").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let tool = tool(temporary.path());

    let output = output_content(&tool, "unreadable.txt");
    assert_eq!(output["kind"], "file");
    assert_eq!(output["size_bytes"], 29);
    assert_eq!(output["extension"], "txt");

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn missing_permission_and_generic_metadata_failures_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let missing = "PRIVATE_MISSING_PARENT/private-file.txt";
    let error = execute(&tool, json!({ "path": missing }), CancellationToken::new()).unwrap_err();
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "file_info_not_found",
        "requested path is unavailable",
        false,
    );
    assert!(!debug.contains(missing));
    assert!(!display.contains(missing));
    assert!(!temporary.path().join("PRIVATE_MISSING_PARENT").exists());

    let too_long = format!("private-{}", "x".repeat(300));
    let error = execute(
        &tool,
        json!({ "path": &too_long }),
        CancellationToken::new(),
    )
    .unwrap_err();
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "file_info_unavailable",
        "requested path metadata is unavailable",
        true,
    );
    assert!(!debug.contains(&too_long));
    assert!(!display.contains(&too_long));

    let inaccessible = temporary.path().join("PRIVATE_INACCESSIBLE");
    fs::create_dir(&inaccessible).unwrap();
    fs::write(inaccessible.join("secret.txt"), b"secret").unwrap();
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o000)).unwrap();
    let operating_system_enforces_mode = fs::metadata(inaccessible.join("secret.txt")).is_err();
    if operating_system_enforces_mode {
        let error = execute(
            &tool,
            json!({ "path": "PRIVATE_INACCESSIBLE/secret.txt" }),
            CancellationToken::new(),
        )
        .unwrap_err();
        let debug = format!("{error:?}");
        let display = error.to_string();
        assert_tool_error(
            error,
            ToolErrorKind::PermissionDenied,
            "file_info_permission_denied",
            "requested path metadata cannot be inspected",
            false,
        );
        assert!(!debug.contains("PRIVATE_INACCESSIBLE"));
        assert!(!display.contains("PRIVATE_INACCESSIBLE"));
    }
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn retained_root_rename_and_path_replacement_cannot_redirect_metadata() {
    let temporary = TemporaryDirectory::new();
    let original = temporary.path().join("workspace");
    let retained = temporary.path().join("retained-workspace");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("retained.txt"), b"retained").unwrap();
    let tool = tool(&original);

    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(
        original.join("retained.txt"),
        b"replacement-decoy-is-longer",
    )
    .unwrap();

    let output = output_content(&tool, "retained.txt");
    assert_eq!(output["kind"], "file");
    assert_eq!(output["size_bytes"], 8);
    assert_eq!(
        fs::read(retained.join("retained.txt")).unwrap(),
        b"retained"
    );
}

#[test]
fn removed_retained_root_is_fixed_unavailable() {
    let temporary = TemporaryDirectory::new();
    let root = temporary.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let tool = tool(&root);
    fs::remove_dir(&root).unwrap();

    assert_tool_error(
        execute(&tool, json!({ "path": "." }), CancellationToken::new()).unwrap_err(),
        ToolErrorKind::Unavailable,
        "file_info_unavailable",
        "requested path metadata is unavailable",
        true,
    );
}

#[test]
fn execution_future_is_inert_until_polled_and_drop_detaches_no_work() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let future = tool.execute(
        context(),
        json!({ "path": "late.txt" }),
        CancellationToken::new(),
    );
    fs::write(temporary.path().join("late.txt"), b"created after future").unwrap();
    let output = poll_immediately_ready(future).unwrap();
    assert_eq!(output.content["size_bytes"], 20);

    let dropped = tool.execute(
        context(),
        json!({ "path": "dropped.txt" }),
        CancellationToken::new(),
    );
    drop(dropped);
    fs::write(temporary.path().join("dropped.txt"), b"unchanged").unwrap();
    assert_eq!(
        fs::read(temporary.path().join("dropped.txt")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn execute_observes_pre_cancellation_before_inspection() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("private.txt"), b"unchanged").unwrap();
    let tool = tool(temporary.path());
    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());

    assert_tool_error(
        execute(&tool, json!({ "path": "private.txt" }), cancellation).unwrap_err(),
        ToolErrorKind::Cancelled,
        "file_info_cancelled",
        "file_info execution was cancelled",
        false,
    );
}

#[test]
fn direct_execute_requires_the_exact_prepared_canonical_path() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("file.txt"), b"contents").unwrap();
    let tool = tool(temporary.path());

    for arguments in [
        json!({}),
        json!({ "path": null }),
        json!({ "path": 1 }),
        json!({ "path": "file.txt", "extra": true }),
        json!({ "path": "./file.txt" }),
        json!({ "path": "folder//file.txt" }),
        json!("file.txt"),
    ] {
        assert_invalid_arguments(execute(&tool, arguments, CancellationToken::new()).unwrap_err());
    }

    for path in ["", "../secret", "/absolute", "nul\0byte"] {
        assert_invalid_path(
            execute(&tool, json!({ "path": path }), CancellationToken::new()).unwrap_err(),
        );
    }
    assert_eq!(output_content(&tool, ".")["kind"], "directory");
}

#[test]
fn constructor_and_debug_contracts_are_typed_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let root_file = temporary.path().join("PRIVATE_WORKSPACE_FILE");
    let root_link = temporary.path().join("PRIVATE_WORKSPACE_LINK");
    fs::write(&root_file, b"not a directory").unwrap();
    symlink(temporary.path(), &root_link).unwrap();

    assert_open_error(
        FileInfoTool::open(Path::new("PRIVATE_RELATIVE_ROOT")).unwrap_err(),
        FileInfoToolOpenErrorKind::InvalidRoot,
        "native file_info workspace root is invalid",
        &["PRIVATE_RELATIVE_ROOT"],
    );
    let missing = temporary.path().join("PRIVATE_MISSING_ROOT");
    assert_open_error(
        FileInfoTool::open(&missing).unwrap_err(),
        FileInfoToolOpenErrorKind::Unavailable,
        "native file_info workspace root is unavailable",
        &["PRIVATE_MISSING_ROOT"],
    );
    for path in [&root_file, &root_link] {
        assert_open_error(
            FileInfoTool::open(path).unwrap_err(),
            FileInfoToolOpenErrorKind::InvalidFileType,
            "native file_info workspace root is not a directory",
            &[path.file_name().unwrap().to_str().unwrap()],
        );
    }

    let debug = format!("{:?}", tool(temporary.path()));
    assert_eq!(debug, "FileInfoTool { .. }");
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));
}

#[test]
fn constructor_applies_no_follow_to_decorated_final_root_symlinks_and_accepts_root() {
    let temporary = TemporaryDirectory::new();
    let real_root = temporary.path().join("PRIVATE_REAL_WORKSPACE");
    let linked_root = temporary.path().join("PRIVATE_LINKED_WORKSPACE");
    fs::create_dir(&real_root).unwrap();
    symlink(&real_root, &linked_root).unwrap();

    let linked_root = linked_root.to_str().expect("temporary paths are UTF-8");
    for spelling in [
        format!("{linked_root}/"),
        format!("{linked_root}//"),
        format!("{linked_root}/."),
    ] {
        assert_open_error(
            FileInfoTool::open(Path::new(&spelling)).unwrap_err(),
            FileInfoToolOpenErrorKind::InvalidFileType,
            "native file_info workspace root is not a directory",
            &["PRIVATE_LINKED_WORKSPACE", &spelling],
        );
    }

    let real_root = real_root.to_str().expect("temporary paths are UTF-8");
    for spelling in [
        format!("{real_root}/"),
        format!("{real_root}//"),
        format!("{real_root}/."),
    ] {
        FileInfoTool::open(Path::new(&spelling))
            .expect("terminal separators and dot preserve a real root directory");
    }
    FileInfoTool::open(Path::new("/"))
        .expect("lexical root normalization must preserve the filesystem root");
}

fn assert_open_error(
    error: FileInfoToolOpenError,
    kind: FileInfoToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    let debug = format!("{error:?}");
    assert_eq!(debug, format!("FileInfoToolOpenError {{ kind: {kind:?} }}"));
    for secret in forbidden {
        assert!(!error.to_string().contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn preparation_and_execution_errors_never_reflect_untrusted_paths() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for (error, secret) in [
        (
            tool.prepare(call(json!({ "path": "/PRIVATE_ABSOLUTE_SECRET" })))
                .unwrap_err(),
            "PRIVATE_ABSOLUTE_SECRET",
        ),
        (
            execute(
                &tool,
                json!({ "path": "PRIVATE_MISSING_SECRET" }),
                CancellationToken::new(),
            )
            .unwrap_err(),
            "PRIVATE_MISSING_SECRET",
        ),
    ] {
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }
}

#[test]
fn maximum_escape_heavy_result_with_extension_remains_below_seventeen_kibibytes() {
    let temporary = TemporaryDirectory::new();
    let directory_component = "\\\"".repeat(120);
    let extension = "\\\"".repeat(119);
    let file_name = format!("a.{extension}");
    assert_eq!(directory_component.len(), 240);
    assert_eq!(file_name.len(), 240);
    let root = rustix::fs::open(
        temporary.path(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    let mut directories = vec![root];
    let mut components = Vec::with_capacity(17);
    for _ in 0..16 {
        let parent = directories.last().unwrap();
        rustix::fs::mkdirat(
            parent.as_fd(),
            &directory_component,
            Mode::from_raw_mode(0o700),
        )
        .unwrap();
        let child = rustix::fs::openat(
            parent.as_fd(),
            &directory_component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        directories.push(child);
        components.push(directory_component.clone());
    }
    let file = rustix::fs::openat(
        directories.last().unwrap().as_fd(),
        &file_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .unwrap();
    drop(file);
    components.push(file_name.clone());
    let normalized = components.join("/");
    assert_eq!(normalized.len(), MAX_FILE_INFO_PATH_BYTES);
    let output = execute(
        &tool(temporary.path()),
        json!({ "path": &normalized }),
        CancellationToken::new(),
    )
    .unwrap();
    let serialized_content = serde_json::to_vec(&output.content).unwrap();
    assert!(
        serialized_content.len() < 17 * 1024,
        "serialized content was {} bytes",
        serialized_content.len()
    );
    assert_eq!(output.content["path"], normalized);
    assert_eq!(output.content["kind"], "file");
    assert_eq!(output.content["extension"], extension);
    assert!(output.content["size_bytes"].is_u64());
    assert!(output.content["modified"]["unix_seconds"].is_i64());
    assert!(output.content["modified"]["nanoseconds"].is_u64());

    rustix::fs::unlinkat(
        directories.last().unwrap().as_fd(),
        &file_name,
        AtFlags::empty(),
    )
    .unwrap();
    for parent in directories.iter().take(16).rev() {
        rustix::fs::unlinkat(parent.as_fd(), &directory_component, AtFlags::REMOVEDIR).unwrap();
    }
}
