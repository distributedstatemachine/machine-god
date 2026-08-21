#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::ffi::{OsStr, OsString};
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
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
    LIST_FILES_TOOL_NAME, ListFilesTool, ListFilesToolOpenError, ListFilesToolOpenErrorKind,
    MAX_LIST_FILES_ENTRIES, MAX_LIST_FILES_PATH_BYTES, MAX_LIST_FILES_TOTAL_NAME_BYTES,
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
                .join(format!("mg-list-files-{}-{identifier}", std::process::id()));
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
        Poll::Pending => panic!("list_files execution unexpectedly returned a pending future"),
    }
}

fn tool(root: &Path) -> ListFilesTool {
    ListFilesTool::open(root).expect("temporary workspace root is valid")
}

fn tool_name(name: &str) -> ToolName {
    ToolName::new(name).expect("test tool name is valid")
}

fn call(arguments: Value) -> ToolCall {
    named_call(LIST_FILES_TOOL_NAME, arguments)
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("list-files-call").unwrap(),
        name: tool_name(name),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("list-files-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("list-files-incarnation").unwrap(),
        turn_id: TurnId::new("list-files-turn").unwrap(),
        call_id: ToolCallId::new("list-files-call").unwrap(),
    }
}

fn execute(
    tool: &ListFilesTool,
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
        "list_files_invalid_arguments",
        "list_files arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "list_files_invalid_path",
        "list_files path is invalid",
        false,
    );
}

fn assert_path_rejected(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::PermissionDenied,
        "list_files_path_rejected",
        "requested path is not a confined directory",
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

fn output_names(output: &ToolOutput) -> Vec<&str> {
    output.content["entries"]
        .as_array()
        .expect("entries is an array")
        .iter()
        .map(|entry| entry["name"].as_str().expect("entry name is a string"))
        .collect()
}

fn long_name(length: usize, index: usize) -> String {
    let prefix = format!("{index:03}");
    assert!(prefix.len() <= length);
    format!("{prefix}{}", "x".repeat(length - prefix.len()))
}

#[test]
fn exported_contract_and_spec_are_exact() {
    assert_eq!(LIST_FILES_TOOL_NAME, "list_files");
    assert_eq!(MAX_LIST_FILES_PATH_BYTES, 4_096);
    assert_eq!(MAX_LIST_FILES_ENTRIES, 100);
    assert_eq!(MAX_LIST_FILES_TOTAL_NAME_BYTES, 16 * 1_024);
    assert_eq!(
        format!("{:?}", ListFilesToolOpenErrorKind::UnsupportedPlatform),
        "UnsupportedPlatform"
    );

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), LIST_FILES_TOOL_NAME);
    assert_eq!(
        spec.description,
        "List one directory within the configured workspace"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative directory path; defaults to the workspace root"
                }
            },
            "additionalProperties": false
        })
    );
}

#[test]
fn prepare_defaults_the_omitted_path_to_the_workspace_root() {
    let temporary = TemporaryDirectory::new();
    let prepared = tool(temporary.path()).prepare(call(json!({}))).unwrap();

    assert_eq!(
        prepared.capability(),
        &Capability::Filesystem {
            access: FilesystemAccess::Enumerate,
            path: ".".to_owned(),
        }
    );
    assert_eq!(prepared.arguments(), &json!({ "path": "." }));
}

#[test]
fn prepare_requires_the_exact_tool_name_and_at_most_the_optional_string_path() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let invalid_calls = [
        named_call("other_tool", json!({})),
        call(json!({ "path": null })),
        call(json!({ "path": true })),
        call(json!({ "path": 1 })),
        call(json!({ "path": ["directory"] })),
        call(json!({ "path": { "nested": "directory" } })),
        call(json!({ "path": "directory", "extra": false })),
        call(json!({ "extra": false })),
        call(json!("directory")),
        call(json!(["directory"])),
    ];

    for invalid_call in invalid_calls {
        assert_invalid_arguments(tool.prepare(invalid_call).unwrap_err());
    }
}

#[test]
fn prepare_normalizes_root_dot_and_repeated_separators_for_policy_and_execution() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());

    for (path, normalized) in [
        (".", "."),
        ("././", "."),
        ("./src///./nested//", "src/nested"),
    ] {
        let prepared = tool.prepare(call(json!({ "path": path }))).unwrap();
        assert_eq!(
            prepared.capability(),
            &Capability::Filesystem {
                access: FilesystemAccess::Enumerate,
                path: normalized.to_owned(),
            }
        );
        assert_eq!(prepared.arguments(), &json!({ "path": normalized }));
    }
}

#[test]
fn prepare_treats_unix_whitespace_and_backslashes_as_literal_filename_bytes() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());

    for path in [" leading and trailing ", r"C:\x", r"directory\child"] {
        let prepared = tool
            .prepare(call(json!({ "path": path })))
            .expect("safe whitespace and backslashes are literal Unix filename bytes");
        assert_eq!(
            prepared.capability(),
            &Capability::Filesystem {
                access: FilesystemAccess::Enumerate,
                path: path.to_owned(),
            }
        );
        assert_eq!(prepared.arguments(), &json!({ "path": path }));
    }
}

#[test]
fn prepare_enforces_the_path_limit_in_utf8_bytes() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let exact = "λ".repeat(MAX_LIST_FILES_PATH_BYTES / "λ".len());
    let prepared = tool.prepare(call(json!({ "path": exact }))).unwrap();
    assert_eq!(
        prepared.arguments()["path"].as_str().unwrap().len(),
        MAX_LIST_FILES_PATH_BYTES
    );

    let oversized = "λ".repeat(MAX_LIST_FILES_PATH_BYTES / "λ".len() + 1);
    assert_invalid_path(
        tool.prepare(call(json!({ "path": oversized })))
            .unwrap_err(),
    );
}

#[test]
fn prepare_rejects_unsafe_or_ambiguous_paths() {
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
        "x".repeat(MAX_LIST_FILES_PATH_BYTES + 1),
    ];

    for path in cases {
        assert_invalid_path(tool.prepare(call(json!({ "path": path }))).unwrap_err());
    }
}

#[test]
fn prepare_of_a_missing_path_is_effect_free() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("existing.txt"), b"unchanged").unwrap();
    let tool = tool(temporary.path());
    let entries_before = directory_entries(temporary.path());

    let prepared = tool
        .prepare(call(json!({ "path": "missing/nested" })))
        .unwrap();

    assert_eq!(
        prepared.capability(),
        &Capability::Filesystem {
            access: FilesystemAccess::Enumerate,
            path: "missing/nested".to_owned(),
        }
    );
    assert_eq!(prepared.arguments(), &json!({ "path": "missing/nested" }));
    assert_eq!(directory_entries(temporary.path()), entries_before);
    assert!(!temporary.path().join("missing").exists());
    assert_eq!(
        fs::read(temporary.path().join("existing.txt")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn execute_lists_one_level_classifies_without_following_and_sorts_all_entries() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    let nested = temporary.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::create_dir(nested.join("directory")).unwrap();
    fs::write(
        nested.join("directory/real-nested-secret-name"),
        b"nested secret",
    )
    .unwrap();
    fs::write(nested.join("zeta.txt"), b"unchanged").unwrap();
    fs::write(nested.join(".hidden"), b"hidden").unwrap();
    fs::write(outside.path().join("outside-secret-name"), b"secret").unwrap();
    symlink(outside.path(), nested.join("target-link")).unwrap();
    create_fifo(&nested.join("pipe"));
    let entries_before = directory_entries(&nested);
    let tool = tool(temporary.path());

    let output = execute(&tool, json!({ "path": "nested" }), CancellationToken::new()).unwrap();

    assert_eq!(
        output,
        ToolOutput::success(json!({
            "path": "nested",
            "entries": [
                { "name": ".hidden", "kind": "file" },
                { "name": "directory", "kind": "directory" },
                { "name": "pipe", "kind": "other" },
                { "name": "target-link", "kind": "symlink" },
                { "name": "zeta.txt", "kind": "file" }
            ],
            "truncated": false
        }))
    );
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("outside-secret-name")
    );
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("real-nested-secret-name")
    );
    assert!(!output_names(&output).contains(&"."));
    assert!(!output_names(&output).contains(&".."));
    assert_eq!(directory_entries(&nested), entries_before);
    assert_eq!(fs::read(nested.join("zeta.txt")).unwrap(), b"unchanged");
    assert_eq!(
        fs::read(outside.path().join("outside-secret-name")).unwrap(),
        b"secret"
    );
}

#[test]
fn execute_lists_an_empty_root_and_a_normalized_nested_directory() {
    let temporary = TemporaryDirectory::new();
    let empty_root = tool(temporary.path());
    assert_eq!(
        execute(
            &empty_root,
            json!({ "path": "." }),
            CancellationToken::new()
        )
        .unwrap(),
        ToolOutput::success(json!({
            "path": ".",
            "entries": [],
            "truncated": false
        }))
    );

    fs::create_dir_all(temporary.path().join("one/two")).unwrap();
    fs::write(temporary.path().join("one/two/child"), b"contents").unwrap();
    let prepared = empty_root
        .prepare(call(json!({ "path": "./one//two/." })))
        .unwrap();
    assert_eq!(prepared.arguments(), &json!({ "path": "one/two" }));
    assert_eq!(
        execute(
            &empty_root,
            prepared.arguments().clone(),
            CancellationToken::new()
        )
        .unwrap(),
        ToolOutput::success(json!({
            "path": "one/two",
            "entries": [{ "name": "child", "kind": "file" }],
            "truncated": false
        }))
    );
}

#[test]
fn retained_root_descriptor_is_not_reopened_by_path() {
    let temporary = TemporaryDirectory::new();
    let original = temporary.path().join("workspace");
    let retained = temporary.path().join("retained-workspace");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("retained.txt"), b"retained").unwrap();
    let tool = tool(&original);

    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("replacement-secret.txt"), b"replacement").unwrap();

    let output = execute(&tool, json!({ "path": "." }), CancellationToken::new()).unwrap();
    assert_eq!(
        output,
        ToolOutput::success(json!({
            "path": ".",
            "entries": [{ "name": "retained.txt", "kind": "file" }],
            "truncated": false
        }))
    );
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("replacement-secret")
    );
}

#[test]
fn execute_rejects_final_and_intermediate_symlinks_files_and_fifos_as_targets() {
    let temporary = TemporaryDirectory::new();
    let outside = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("directory")).unwrap();
    fs::write(
        temporary.path().join("private-target-file"),
        b"private contents",
    )
    .unwrap();
    create_fifo(&temporary.path().join("private-target-pipe"));
    fs::create_dir(outside.path().join("private-secret-directory")).unwrap();
    fs::write(
        outside.path().join("private-secret-directory/secret-name"),
        b"outside secret",
    )
    .unwrap();
    symlink(
        outside.path().join("private-secret-directory"),
        temporary.path().join("private-final-link"),
    )
    .unwrap();
    symlink(
        outside.path(),
        temporary.path().join("private-intermediate-link"),
    )
    .unwrap();
    let tool = tool(temporary.path());

    for path in [
        "private-target-file",
        "private-target-pipe",
        "private-final-link",
        "private-intermediate-link/private-secret-directory",
    ] {
        let error = execute(&tool, json!({ "path": path }), CancellationToken::new()).unwrap_err();
        let debug = format!("{error:?}");
        let display = error.to_string();
        assert_path_rejected(error);
        assert!(!debug.contains(path));
        assert!(!display.contains(path));
    }
    assert_eq!(
        fs::read(outside.path().join("private-secret-directory/secret-name")).unwrap(),
        b"outside secret"
    );
}

#[test]
fn execute_reports_missing_inaccessible_and_other_open_failures_with_redaction() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let missing_path = "private-missing/directory";
    let error = execute(
        &tool,
        json!({ "path": missing_path }),
        CancellationToken::new(),
    )
    .unwrap_err();
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "list_files_not_found",
        "requested directory is unavailable",
        false,
    );
    assert!(!debug.contains(missing_path));
    assert!(!display.contains(missing_path));
    assert!(!temporary.path().join("private-missing").exists());

    let too_long_component = format!("private-{}", "x".repeat(300));
    let error = execute(
        &tool,
        json!({ "path": &too_long_component }),
        CancellationToken::new(),
    )
    .unwrap_err();
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "list_files_unavailable",
        "requested directory is unavailable",
        true,
    );
    assert!(!debug.contains(&too_long_component));
    assert!(!display.contains(&too_long_component));

    let inaccessible = temporary.path().join("private-inaccessible");
    fs::create_dir(&inaccessible).unwrap();
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o000)).unwrap();
    let operating_system_enforces_mode = fs::read_dir(&inaccessible).is_err();
    if operating_system_enforces_mode {
        let error = execute(
            &tool,
            json!({ "path": "private-inaccessible" }),
            CancellationToken::new(),
        )
        .unwrap_err();
        let debug = format!("{error:?}");
        let display = error.to_string();
        assert_tool_error(
            error,
            ToolErrorKind::PermissionDenied,
            "list_files_permission_denied",
            "requested directory cannot be listed",
            false,
        );
        assert!(!debug.contains("private-inaccessible"));
        assert!(!display.contains("private-inaccessible"));
    }
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn execute_classifies_revoked_root_access_as_permission_denied() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o000)).unwrap();
    let operating_system_enforces_mode = fs::read_dir(temporary.path()).is_err();
    let result = execute(&tool, json!({ "path": "." }), CancellationToken::new());
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700)).unwrap();

    if operating_system_enforces_mode {
        assert_tool_error(
            result.unwrap_err(),
            ToolErrorKind::PermissionDenied,
            "list_files_permission_denied",
            "requested directory cannot be listed",
            false,
        );
    }
}

#[test]
fn execute_rejects_invalid_utf8_control_and_bidi_entry_names_with_redaction() {
    let temporary = TemporaryDirectory::new();
    let directory = temporary.path().join("directory");
    fs::create_dir(&directory).unwrap();
    let tool = tool(temporary.path());
    let unsafe_names = [
        OsString::from_vec(b"private-invalid-\xff-name".to_vec()),
        OsString::from("private-control-\t-name"),
        OsString::from("private-control-\n-name"),
        OsString::from("private-arabic-\u{061c}-name"),
        OsString::from("private-ltr-\u{200e}-name"),
        OsString::from("private-rtl-\u{200f}-name"),
        OsString::from("private-line-separator-\u{2028}-name"),
        OsString::from("private-paragraph-separator-\u{2029}-name"),
        OsString::from("private-embedding-\u{202a}-name"),
        OsString::from("private-embedding-\u{202b}-name"),
        OsString::from("private-pop-formatting-\u{202c}-name"),
        OsString::from("private-override-\u{202d}-name"),
        OsString::from("private-override-\u{202e}-name"),
        OsString::from("private-isolate-\u{2066}-name"),
        OsString::from("private-isolate-\u{2067}-name"),
        OsString::from("private-isolate-\u{2068}-name"),
        OsString::from("private-pop-isolate-\u{2069}-name"),
    ];

    for unsafe_name in unsafe_names {
        let path = directory.join(&unsafe_name);
        match fs::write(&path, b"private contents") {
            Ok(()) => {}
            Err(error) if unsafe_name.to_str().is_none() => {
                assert!(
                    [libc::EILSEQ, libc::EINVAL].contains(
                        &error
                            .raw_os_error()
                            .expect("non-Unicode name rejection has an OS error")
                    ),
                    "unexpected non-Unicode name rejection: {error}"
                );
                continue;
            }
            Err(error) => panic!("failed to create unsafe test entry: {error}"),
        }
        let error = execute(
            &tool,
            json!({ "path": "directory" }),
            CancellationToken::new(),
        )
        .unwrap_err();
        let debug = format!("{error:?}");
        let display = error.to_string();
        assert_tool_error(
            error,
            ToolErrorKind::Execution,
            "list_files_invalid_entry_name",
            "requested directory contains an unsupported entry name",
            false,
        );
        assert!(!debug.contains("private-"));
        assert!(!display.contains("private-"));
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn entry_count_limit_distinguishes_exactly_one_hundred_from_one_hundred_one() {
    let temporary = TemporaryDirectory::new();
    let exact = temporary.path().join("exact");
    let overflow = temporary.path().join("overflow");
    fs::create_dir(&exact).unwrap();
    fs::create_dir(&overflow).unwrap();
    for index in 0..MAX_LIST_FILES_ENTRIES {
        let name = format!("entry-{index:03}");
        fs::write(exact.join(&name), []).unwrap();
        fs::write(overflow.join(&name), []).unwrap();
    }
    fs::write(overflow.join("entry-100"), []).unwrap();
    let tool = tool(temporary.path());

    let exact_output =
        execute(&tool, json!({ "path": "exact" }), CancellationToken::new()).unwrap();
    assert_eq!(output_names(&exact_output).len(), MAX_LIST_FILES_ENTRIES);
    assert_eq!(exact_output.content["truncated"], false);
    assert_eq!(output_names(&exact_output).first(), Some(&"entry-000"));
    assert_eq!(output_names(&exact_output).last(), Some(&"entry-099"));

    let overflow_output = execute(
        &tool,
        json!({ "path": "overflow" }),
        CancellationToken::new(),
    )
    .unwrap();
    let names = output_names(&overflow_output);
    assert_eq!(names.len(), MAX_LIST_FILES_ENTRIES);
    assert_eq!(overflow_output.content["truncated"], true);
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(names.iter().all(|name| name.starts_with("entry-")));
}

#[test]
fn aggregate_name_limit_accepts_the_exact_boundary_and_marks_an_overflow() {
    let temporary = TemporaryDirectory::new();
    let exact = temporary.path().join("exact-name-bytes");
    let overflow = temporary.path().join("overflow-name-bytes");
    fs::create_dir(&exact).unwrap();
    fs::create_dir(&overflow).unwrap();
    let mut names = (0..64)
        .map(|index| long_name(255, index))
        .collect::<Vec<_>>();
    names.push(long_name(64, 64));
    assert_eq!(
        names.iter().map(String::len).sum::<usize>(),
        MAX_LIST_FILES_TOTAL_NAME_BYTES
    );
    for name in &names {
        fs::write(exact.join(name), []).unwrap();
        fs::write(overflow.join(name), []).unwrap();
    }
    fs::write(overflow.join("overflow"), []).unwrap();
    let tool = tool(temporary.path());

    let exact_output = execute(
        &tool,
        json!({ "path": "exact-name-bytes" }),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(exact_output.content["truncated"], false);
    assert_eq!(output_names(&exact_output).len(), names.len());
    assert_eq!(
        output_names(&exact_output)
            .iter()
            .map(|name| name.len())
            .sum::<usize>(),
        MAX_LIST_FILES_TOTAL_NAME_BYTES
    );

    let overflow_output = execute(
        &tool,
        json!({ "path": "overflow-name-bytes" }),
        CancellationToken::new(),
    )
    .unwrap();
    let overflow_names = output_names(&overflow_output);
    assert_eq!(overflow_output.content["truncated"], true);
    assert!(overflow_names.len() < names.len() + 1);
    assert!(
        overflow_names.iter().map(|name| name.len()).sum::<usize>()
            <= MAX_LIST_FILES_TOTAL_NAME_BYTES
    );
    assert!(overflow_names.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn execute_observes_cancellation_before_enumerating_the_directory() {
    let temporary = TemporaryDirectory::new();
    let secret_name = "private-cancelled-secret-name";
    fs::write(temporary.path().join(secret_name), b"unchanged").unwrap();
    let tool = tool(temporary.path());
    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());

    let error = execute(&tool, json!({ "path": "." }), cancellation).unwrap_err();
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert_tool_error(
        error,
        ToolErrorKind::Cancelled,
        "list_files_cancelled",
        "list_files execution was cancelled",
        false,
    );
    assert!(!debug.contains(secret_name));
    assert!(!display.contains(secret_name));
    assert_eq!(
        fs::read(temporary.path().join(secret_name)).unwrap(),
        b"unchanged"
    );
}

#[test]
fn direct_execute_requires_the_exact_prepared_sole_normalized_path() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("directory")).unwrap();
    let tool = tool(temporary.path());

    for arguments in [
        json!({}),
        json!({ "path": null }),
        json!({ "path": 1 }),
        json!({ "path": ".", "extra": true }),
        json!({ "path": "./directory" }),
        json!({ "path": "directory//" }),
        json!("directory"),
    ] {
        assert_invalid_arguments(execute(&tool, arguments, CancellationToken::new()).unwrap_err());
    }

    for path in ["", "../secret", "/absolute", "nul\0byte"] {
        assert_invalid_path(
            execute(&tool, json!({ "path": path }), CancellationToken::new()).unwrap_err(),
        );
    }
}

#[test]
fn constructor_rejects_relative_missing_file_and_final_symlink_roots() {
    let temporary = TemporaryDirectory::new();
    let root_file = temporary.path().join("private-workspace-file");
    let root_link = temporary.path().join("private-workspace-link");
    fs::write(&root_file, b"not a directory").unwrap();
    symlink(temporary.path(), &root_link).unwrap();

    assert_open_error(
        ListFilesTool::open(Path::new("private-relative-root")).unwrap_err(),
        ListFilesToolOpenErrorKind::InvalidRoot,
        "native list_files workspace root is invalid",
        &["private-relative-root"],
    );

    let missing_path = temporary.path().join("private-missing-root");
    assert_open_error(
        ListFilesTool::open(&missing_path).unwrap_err(),
        ListFilesToolOpenErrorKind::Unavailable,
        "native list_files workspace root is unavailable",
        &[
            "private-missing-root",
            temporary.path().to_string_lossy().as_ref(),
        ],
    );

    assert_open_error(
        ListFilesTool::open(&root_file).unwrap_err(),
        ListFilesToolOpenErrorKind::InvalidFileType,
        "native list_files workspace root is not a directory",
        &[
            "private-workspace-file",
            temporary.path().to_string_lossy().as_ref(),
        ],
    );
    assert_open_error(
        ListFilesTool::open(&root_link).unwrap_err(),
        ListFilesToolOpenErrorKind::InvalidFileType,
        "native list_files workspace root is not a directory",
        &[
            "private-workspace-link",
            temporary.path().to_string_lossy().as_ref(),
        ],
    );
}

#[test]
fn constructor_applies_no_follow_to_decorated_final_root_symlinks_and_accepts_root() {
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
            ListFilesTool::open(Path::new(&spelling)).unwrap_err(),
            ListFilesToolOpenErrorKind::InvalidFileType,
            "native list_files workspace root is not a directory",
            &["private-linked-workspace", &spelling],
        );
    }

    let real_root = real_root.to_str().expect("temporary paths are UTF-8");
    for spelling in [format!("{real_root}/"), format!("{real_root}/.")] {
        ListFilesTool::open(Path::new(&spelling))
            .expect("terminal separators and dot preserve a real root directory");
    }
    ListFilesTool::open(Path::new("/"))
        .expect("lexical root normalization must preserve the filesystem root");
}

fn assert_open_error(
    error: ListFilesToolOpenError,
    kind: ListFilesToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    let debug = format!("{error:?}");
    assert_eq!(
        debug,
        format!("ListFilesToolOpenError {{ kind: {kind:?} }}")
    );
    for secret in forbidden {
        assert!(!error.to_string().contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn tool_debug_and_preparation_errors_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let private_root = temporary.path().join("private-workspace-root");
    fs::create_dir(&private_root).unwrap();
    let tool = tool(&private_root);
    let debug = format!("{tool:?}");
    assert_eq!(debug, "ListFilesTool { .. }");
    assert!(!debug.contains("private-workspace-root"));
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));

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
            json!({ "path": "private-secret-directory" }),
        ))
        .unwrap_err();
    let arguments_debug = format!("{arguments_error:?}");
    let arguments_display = arguments_error.to_string();
    assert_invalid_arguments(arguments_error);
    for secret in [private_tool, "private-secret-directory"] {
        assert!(!arguments_debug.contains(secret));
        assert!(!arguments_display.contains(secret));
    }
}

#[test]
fn safe_entry_name_predicate_does_not_reject_ordinary_spaces() {
    let temporary = TemporaryDirectory::new();
    let name = OsStr::new(" ordinary name ");
    fs::write(temporary.path().join(name), []).unwrap();
    let output = execute(
        &tool(temporary.path()),
        json!({ "path": "." }),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(output_names(&output), [" ordinary name "]);
}
