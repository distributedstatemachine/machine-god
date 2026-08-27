#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::error::Error;
use std::fs::{self, Permissions};
use std::future::Future;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    CancellationToken, Capability, FilesystemAccess, Tool, ToolCall, ToolCallId, ToolContext,
    ToolError, ToolErrorKind, ToolName, ToolOutput,
};
use machine_god_native::{
    CREATE_FOLDER_TOOL_NAME, CreateFolderTool, CreateFolderToolOpenError,
    CreateFolderToolOpenErrorKind, MAX_CREATE_FOLDER_MKDIR_CALLS, MAX_CREATE_FOLDER_PATH_BYTES,
    MAX_CREATE_FOLDER_PATH_COMPONENTS, MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES,
    MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES, MAX_CREATE_FOLDER_SYNC_CALLS,
};
use serde_json::{Value, json};

#[cfg(target_os = "macos")]
use rustix::fd::AsFd;
#[cfg(target_os = "macos")]
use rustix::fs::{Mode, OFlags};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

#[cfg(target_os = "macos")]
struct MacAclCleanup(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for MacAclCleanup {
    fn drop(&mut self) {
        let _ = Command::new("/bin/chmod").arg("-N").arg(&self.0).status();
    }
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-create-folder-{label}-{}-{identifier}",
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
        let _ = make_tree_owner_accessible(&self.path);
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

fn make_tree_owner_accessible(path: &Path) -> io::Result<()> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if !metadata.file_type().is_dir() {
        return Ok(());
    }
    fs::set_permissions(path, Permissions::from_mode(0o700))?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            make_tree_owner_accessible(&entry.path())?;
        }
    }
    Ok(())
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
        Poll::Pending => panic!("create_folder unexpectedly yielded"),
    }
}

fn tool(root: &Path) -> CreateFolderTool {
    CreateFolderTool::open(root).unwrap()
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("create-folder-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn call(arguments: Value) -> ToolCall {
    named_call(CREATE_FOLDER_TOOL_NAME, arguments)
}

fn context() -> ToolContext {
    ToolContext {
        session_id: machine_god_core::SessionId::new("create-folder-session").unwrap(),
        session_incarnation_id: machine_god_core::SessionIncarnationId::new(
            "create-folder-incarnation",
        )
        .unwrap(),
        turn_id: machine_god_core::TurnId::new("create-folder-turn").unwrap(),
        call_id: ToolCallId::new("create-folder-call").unwrap(),
    }
}

fn arguments(path: &str) -> Value {
    json!({"path": path})
}

fn execute(
    tool: &CreateFolderTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_immediately_ready(tool.execute(context(), arguments, cancellation))
}

fn create(tool: &CreateFolderTool, path: &str) -> Result<ToolOutput, ToolError> {
    execute(tool, arguments(path), CancellationToken::new())
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
        "create_folder_invalid_arguments",
        "create_folder arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "create_folder_invalid_path",
        "create_folder path is invalid",
        false,
    );
}

fn assert_path_rejected(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::PermissionDenied,
        "create_folder_path_rejected",
        "requested folder path is not confined",
        false,
    );
}

fn assert_target_exists(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::Execution,
        "create_folder_target_exists",
        "requested folder path already exists as a non-directory",
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

fn every_forbidden_path_character() -> Vec<char> {
    let mut characters = (0_u32..=0x1f)
        .chain(0x7f..=0x9f)
        .map(|value| char::from_u32(value).unwrap())
        .collect::<Vec<_>>();
    characters.extend([
        '\u{061c}', '\u{200e}', '\u{200f}', '\u{2028}', '\u{2029}', '\u{202a}', '\u{202b}',
        '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ]);
    characters.sort_unstable();
    characters.dedup();
    characters
}

#[test]
fn exported_contract_limits_and_spec_are_exact() {
    assert_eq!(CREATE_FOLDER_TOOL_NAME, "create_folder");
    assert_eq!(MAX_CREATE_FOLDER_PATH_BYTES, 4_096);
    assert_eq!(MAX_CREATE_FOLDER_PATH_COMPONENTS, 256);
    assert_eq!(MAX_CREATE_FOLDER_MKDIR_CALLS, 256);
    assert_eq!(MAX_CREATE_FOLDER_SYNC_CALLS, 4_112);
    assert_eq!(MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES, 65_536);
    assert_eq!(MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES, 16_384);

    let temporary = TemporaryDirectory::new("spec");
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), CREATE_FOLDER_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Create one directory path and missing parents within the configured workspace"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative directory path"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    );
}

#[test]
fn prepare_is_strict_effect_free_and_requests_exact_create_authority() {
    let temporary = TemporaryDirectory::new("prepare");
    fs::write(temporary.path().join("existing"), b"untouched").unwrap();
    let tool = tool(temporary.path());
    for invalid in [
        json!(null),
        json!([]),
        json!({}),
        json!({"path": 1}),
        json!({"path": "folder", "recursive": true}),
    ] {
        assert_invalid_arguments(tool.prepare(call(invalid)).unwrap_err());
    }
    assert_invalid_arguments(
        tool.prepare(named_call("delete_file", arguments("folder")))
            .unwrap_err(),
    );

    let prepared = tool
        .prepare(call(arguments("./parent//literal\\ λ/final")))
        .unwrap();
    assert_eq!(
        prepared
            .capability()
            .expect("create_folder requires permission authority"),
        &Capability::Filesystem {
            access: FilesystemAccess::Create,
            path: "parent/literal\\ λ/final".to_owned(),
        }
    );
    assert_eq!(
        serde_json::to_value(
            prepared
                .capability()
                .expect("create_folder requires permission authority"),
        )
        .unwrap(),
        json!({"type": "filesystem", "access": "create", "path": "parent/literal\\ λ/final"})
    );
    assert_eq!(
        serde_json::to_vec(
            prepared
                .capability()
                .expect("create_folder requires permission authority"),
        )
        .unwrap(),
        r#"{"type":"filesystem","access":"create","path":"parent/literal\\ λ/final"}"#.as_bytes()
    );
    assert_eq!(prepared.arguments(), &arguments("parent/literal\\ λ/final"));
    assert_eq!(
        fs::read(temporary.path().join("existing")).unwrap(),
        b"untouched"
    );
    assert!(!temporary.path().join("parent").exists());
}

#[test]
fn preparation_does_not_reinspect_or_mutate_a_removed_retained_root() {
    let temporary = TemporaryDirectory::new("prepare-retained-root");
    let workspace = temporary.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let tool = tool(&workspace);
    fs::remove_dir(&workspace).unwrap();

    let prepared = tool.prepare(call(arguments("parent/folder"))).unwrap();
    assert_eq!(prepared.arguments(), &arguments("parent/folder"));
    assert_eq!(
        prepared
            .capability()
            .expect("create_folder requires permission authority"),
        &Capability::Filesystem {
            access: FilesystemAccess::Create,
            path: "parent/folder".to_owned(),
        }
    );
    assert!(!workspace.exists());
}

#[test]
fn preparation_enforces_requested_canonical_and_component_bounds() {
    let temporary = TemporaryDirectory::new("prepare-bounds");
    let tool = tool(temporary.path());

    let exact = "p".repeat(MAX_CREATE_FOLDER_PATH_BYTES);
    assert!(tool.prepare(call(arguments(&exact))).is_ok());
    assert_invalid_path(
        tool.prepare(call(arguments(&format!("{exact}p"))))
            .unwrap_err(),
    );

    let exact_requested = format!("./{}", "r".repeat(MAX_CREATE_FOLDER_PATH_BYTES - 2));
    assert_eq!(
        tool.prepare(call(arguments(&exact_requested)))
            .unwrap()
            .arguments(),
        &arguments(&"r".repeat(MAX_CREATE_FOLDER_PATH_BYTES - 2))
    );
    assert_invalid_path(
        tool.prepare(call(arguments(&format!(
            "./{}",
            "r".repeat(MAX_CREATE_FOLDER_PATH_BYTES - 1)
        ))))
        .unwrap_err(),
    );

    let exact_components = (0..MAX_CREATE_FOLDER_PATH_COMPONENTS)
        .map(|index| format!("c{index}"))
        .collect::<Vec<_>>()
        .join("/");
    assert!(tool.prepare(call(arguments(&exact_components))).is_ok());
    assert_invalid_path(
        tool.prepare(call(arguments(&format!("{exact_components}/extra"))))
            .unwrap_err(),
    );
}

#[test]
fn preparation_rejects_roots_escapes_controls_and_formatting_characters() {
    let temporary = TemporaryDirectory::new("invalid-paths");
    let tool = tool(temporary.path());
    for invalid in [
        "",
        ".",
        "./.",
        "/folder",
        "~/folder",
        "~someone/folder",
        "../folder",
        "folder/../escape",
        "folder\0name",
        "folder\nname",
        "folder\u{0085}name",
        "folder\u{061c}name",
        "folder\u{200e}name",
        "folder\u{2028}name",
        "folder\u{2029}name",
        "folder\u{202e}name",
        "folder\u{2066}name",
        "folder\u{2069}name",
    ] {
        assert_invalid_path(tool.prepare(call(arguments(invalid))).unwrap_err());
    }
    for character in every_forbidden_path_character() {
        let invalid = format!("folder{character}name");
        assert_invalid_path(tool.prepare(call(arguments(&invalid))).unwrap_err());
    }
}

#[test]
fn execute_recursively_creates_one_and_two_hundred_fifty_six_components() {
    let single = TemporaryDirectory::new("single");
    assert_eq!(
        create(&tool(single.path()), "folder").unwrap(),
        ToolOutput::success(json!({"path": "folder"}))
    );
    assert!(single.path().join("folder").is_dir());

    let deep = TemporaryDirectory::new("deep");
    let path = vec!["d"; MAX_CREATE_FOLDER_PATH_COMPONENTS].join("/");
    assert_eq!(
        create(&tool(deep.path()), &path).unwrap(),
        ToolOutput::success(json!({"path": path}))
    );
    assert!(deep.path().join(path).is_dir());

    let literal = TemporaryDirectory::new("literal-names");
    let literal_path = "literal\\ space λ/final ";
    assert_eq!(
        create(&tool(literal.path()), literal_path).unwrap(),
        ToolOutput::success(json!({"path": literal_path}))
    );
    assert!(literal.path().join(literal_path).is_dir());
}

#[test]
fn existing_prefix_and_final_directory_are_idempotent_with_path_only_result() {
    let temporary = TemporaryDirectory::new("idempotent");
    fs::create_dir(temporary.path().join("existing")).unwrap();
    let tool = tool(temporary.path());
    assert_eq!(
        create(&tool, "existing/new/final").unwrap(),
        ToolOutput::success(json!({"path": "existing/new/final"}))
    );
    assert_eq!(
        create(&tool, "existing/new/final").unwrap(),
        ToolOutput::success(json!({"path": "existing/new/final"}))
    );
    assert_eq!(
        serde_json::to_vec(&create(&tool, "existing/new/final").unwrap()).unwrap(),
        br#"{"content":{"path":"existing/new/final"},"is_error":false}"#
    );
}

#[test]
fn idempotent_success_does_not_rewrite_existing_directory_permissions() {
    let temporary = TemporaryDirectory::new("existing-mode");
    let existing = temporary.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::set_permissions(&existing, Permissions::from_mode(0o700)).unwrap();

    assert_eq!(
        create(&tool(temporary.path()), "existing").unwrap(),
        ToolOutput::success(json!({"path": "existing"}))
    );
    assert_eq!(
        fs::metadata(existing).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn final_non_directories_are_preserved_and_report_target_exists() {
    let workspace = TemporaryDirectory::new("final-types");
    let outside = TemporaryDirectory::new("final-outside");
    fs::write(workspace.path().join("file"), b"file sentinel").unwrap();
    create_fifo(&workspace.path().join("fifo"));
    let listener = UnixListener::bind(workspace.path().join("socket")).unwrap();
    fs::write(outside.path().join("sentinel"), b"outside sentinel").unwrap();
    symlink(
        outside.path().join("sentinel"),
        workspace.path().join("link"),
    )
    .unwrap();
    let tool = tool(workspace.path());

    for final_path in ["file", "fifo", "socket", "link"] {
        assert_target_exists(create(&tool, final_path).unwrap_err());
    }
    assert_eq!(
        fs::read(workspace.path().join("file")).unwrap(),
        b"file sentinel"
    );
    assert_eq!(
        fs::read(outside.path().join("sentinel")).unwrap(),
        b"outside sentinel"
    );
    drop(listener);
}

#[test]
fn hostile_ancestors_are_rejected_without_outside_effect() {
    let workspace = TemporaryDirectory::new("ancestor-types");
    let outside = TemporaryDirectory::new("ancestor-outside");
    fs::write(workspace.path().join("file"), b"file sentinel").unwrap();
    create_fifo(&workspace.path().join("fifo"));
    let listener = UnixListener::bind(workspace.path().join("socket")).unwrap();
    fs::write(outside.path().join("sentinel"), b"outside sentinel").unwrap();
    symlink(outside.path(), workspace.path().join("link")).unwrap();
    let tool = tool(workspace.path());

    for ancestor in ["file", "fifo", "socket", "link"] {
        assert_path_rejected(create(&tool, &format!("{ancestor}/new")).unwrap_err());
    }
    assert!(!outside.path().join("new").exists());
    assert_eq!(
        fs::read(outside.path().join("sentinel")).unwrap(),
        b"outside sentinel"
    );
    drop(listener);
}

#[test]
fn device_entries_are_rejected_without_mutation() {
    let device_tool = CreateFolderTool::open(Path::new("/dev")).unwrap();
    assert_target_exists(create(&device_tool, "null").unwrap_err());
    assert_path_rejected(create(&device_tool, "null/PRIVATE_CREATE_FOLDER").unwrap_err());
    assert!(!Path::new("/dev/null/PRIVATE_CREATE_FOLDER").exists());
}

#[test]
fn retained_root_rename_replacement_and_removal_cannot_redirect_authority() {
    let temporary = TemporaryDirectory::new("retained-root");
    let original = temporary.path().join("workspace");
    let retained = temporary.path().join("retained");
    fs::create_dir(&original).unwrap();
    let create_tool = tool(&original);
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();

    assert_eq!(
        create(&create_tool, "parent/final").unwrap(),
        ToolOutput::success(json!({"path": "parent/final"}))
    );
    assert!(retained.join("parent/final").is_dir());
    assert!(!original.join("parent").exists());

    let removed = temporary.path().join("removed-root");
    fs::create_dir(&removed).unwrap();
    let removed_tool = tool(&removed);
    fs::remove_dir(&removed).unwrap();
    fs::create_dir(&removed).unwrap();
    assert_tool_error(
        create(&removed_tool, "PRIVATE_FOLDER").unwrap_err(),
        ToolErrorKind::Unavailable,
        "create_folder_unavailable",
        "requested folder is unavailable",
        true,
    );
    assert!(!removed.join("PRIVATE_FOLDER").exists());
}

#[test]
fn requested_mode_honors_host_umask_without_postcreation_rewrite() {
    const CHILD_ENV: &str = "MACHINE_GOD_CREATE_FOLDER_UMASK_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let executable = std::env::current_exe().unwrap();
        let status = Command::new("/bin/sh")
            .env(CHILD_ENV, "1")
            .arg("-c")
            .arg(
                "umask 027; exec \"$1\" --exact requested_mode_honors_host_umask_without_postcreation_rewrite --nocapture",
            )
            .arg("machine-god-create-folder-umask")
            .arg(executable)
            .status()
            .expect("failed to execute isolated umask test process");
        assert!(status.success(), "hostile-umask child failed with {status}");
        return;
    }

    let temporary = TemporaryDirectory::new("umask-child");
    create(&tool(temporary.path()), "parent/final").unwrap();
    assert_eq!(
        fs::metadata(temporary.path().join("parent"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
    assert_eq!(
        fs::metadata(temporary.path().join("parent/final"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
}

#[cfg(target_os = "linux")]
#[test]
fn created_directories_retain_linux_default_acl_without_rewriting_the_parent() {
    let temporary = TemporaryDirectory::new("linux-acl");
    let status = match Command::new("setfacl")
        .args(["-m", "d:u::rwx,d:g::r-x,d:o::---"])
        .arg(temporary.path())
        .status()
    {
        Ok(status) => status,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => panic!("failed to invoke setfacl: {error}"),
    };
    if !status.success() {
        // Some otherwise supported CI filesystems are mounted without ACL support.
        return;
    }

    create(&tool(temporary.path()), "parent/final").unwrap();

    let created_acl = Command::new("getfacl")
        .arg("-cp")
        .arg(temporary.path().join("parent/final"))
        .output()
        .expect("getfacl accompanies setfacl");
    assert!(created_acl.status.success());
    let created_acl = String::from_utf8(created_acl.stdout).unwrap();
    assert!(created_acl.lines().any(|line| line == "group::r-x"));
    assert!(created_acl.lines().any(|line| line == "other::---"));

    let parent_acl = Command::new("getfacl")
        .arg("-cp")
        .arg(temporary.path())
        .output()
        .expect("getfacl accompanies setfacl");
    assert!(parent_acl.status.success());
    let parent_acl = String::from_utf8(parent_acl.stdout).unwrap();
    assert!(parent_acl.lines().any(|line| line == "default:group::r-x"));
    assert!(parent_acl.lines().any(|line| line == "default:other::---"));
}

#[cfg(target_os = "macos")]
#[test]
fn created_directories_retain_inherited_acl_without_rewriting_the_parent() {
    let temporary = TemporaryDirectory::new("acl");
    let status = Command::new("/bin/chmod")
        .args([
            "+a",
            "everyone allow list,search,add_file,add_subdirectory,file_inherit,directory_inherit",
        ])
        .arg(temporary.path())
        .status()
        .expect("macOS chmod executable is available");
    assert!(
        status.success(),
        "failed to install inheritable ACL fixture: {status}"
    );
    let _acl_cleanup = MacAclCleanup(temporary.path().to_owned());

    create(&tool(temporary.path()), "parent/final").unwrap();

    for path in [
        temporary.path().join("parent"),
        temporary.path().join("parent/final"),
    ] {
        let descriptor = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let acl = calcifer_macos_acl::read_acl(descriptor.as_fd()).unwrap();
        assert!(acl.entries.iter().any(|entry| {
            entry.tag == calcifer_macos_acl::TAG_ALLOW
                && entry.flags & calcifer_macos_acl::FLAG_INHERITED != 0
        }));
    }

    let parent = rustix::fs::open(
        temporary.path(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    let parent_acl = calcifer_macos_acl::read_acl(parent.as_fd()).unwrap();
    assert!(
        parent_acl
            .entries
            .iter()
            .any(|entry| entry.tag == calcifer_macos_acl::TAG_ALLOW)
    );
}

#[test]
fn owner_masking_umask_leaves_a_safe_partial_prefix_and_reports_ambiguity() {
    const CHILD_ENV: &str = "MACHINE_GOD_CREATE_FOLDER_OWNER_MASK_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let executable = std::env::current_exe().unwrap();
        let status = Command::new("/bin/sh")
            .env(CHILD_ENV, "1")
            .arg("-c")
            .arg(
                "umask 0777; exec \"$1\" --exact owner_masking_umask_leaves_a_safe_partial_prefix_and_reports_ambiguity --nocapture",
            )
            .arg("machine-god-create-folder-owner-mask")
            .arg(executable)
            .status()
            .expect("failed to execute isolated owner-masking umask test process");
        assert!(
            status.success(),
            "owner-masking umask child failed with {status}"
        );
        return;
    }

    let temporary = TemporaryDirectory::new("owner-mask-child");
    fs::set_permissions(temporary.path(), Permissions::from_mode(0o700)).unwrap();
    if rustix::process::geteuid().as_raw() == 0 {
        return;
    }
    assert_tool_error(
        create(&tool(temporary.path()), "partial/not-created").unwrap_err(),
        ToolErrorKind::Execution,
        "create_folder_commit_ambiguous",
        "requested folder creation status is uncertain",
        false,
    );
    let partial = temporary.path().join("partial");
    assert!(partial.is_dir());
    assert_eq!(
        fs::metadata(&partial).unwrap().permissions().mode() & 0o777,
        0
    );
    assert!(!partial.join("not-created").exists());
}

#[test]
fn execution_future_is_inert_one_poll_drop_safe_and_pre_cancelled() {
    let temporary = TemporaryDirectory::new("future");
    let tool = tool(temporary.path());
    let future = tool.execute(context(), arguments("created"), CancellationToken::new());
    assert!(!temporary.path().join("created").exists());
    assert_eq!(
        poll_immediately_ready(future).unwrap(),
        ToolOutput::success(json!({"path": "created"}))
    );

    let dropped = tool.execute(context(), arguments("dropped"), CancellationToken::new());
    drop(dropped);
    assert!(!temporary.path().join("dropped").exists());

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    assert_tool_error(
        execute(&tool, arguments("cancelled"), cancellation).unwrap_err(),
        ToolErrorKind::Cancelled,
        "create_folder_cancelled",
        "create_folder execution was cancelled",
        false,
    );
    assert!(!temporary.path().join("cancelled").exists());
}

#[test]
fn direct_execute_requires_exact_canonical_arguments_and_reapplies_bounds() {
    let temporary = TemporaryDirectory::new("direct-validation");
    let tool = tool(temporary.path());
    for invalid in [
        json!(null),
        json!({}),
        json!({"path": 1}),
        json!({"path": "folder", "extra": true}),
        arguments("./folder"),
        arguments("folder//nested"),
        arguments("folder/./nested"),
    ] {
        assert_invalid_arguments(execute(&tool, invalid, CancellationToken::new()).unwrap_err());
    }
    assert_invalid_path(
        execute(
            &tool,
            arguments(&"x".repeat(MAX_CREATE_FOLDER_PATH_BYTES + 1)),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    let one_over_components = vec!["c"; MAX_CREATE_FOLDER_PATH_COMPONENTS + 1].join("/");
    for invalid in [
        "/absolute".to_owned(),
        "~/tilde".to_owned(),
        "~someone/tilde".to_owned(),
        "parent/../escape".to_owned(),
        one_over_components,
    ] {
        assert_invalid_path(
            execute(&tool, arguments(&invalid), CancellationToken::new()).unwrap_err(),
        );
    }
    for character in every_forbidden_path_character() {
        let invalid = format!("folder{character}name");
        assert_invalid_path(
            execute(&tool, arguments(&invalid), CancellationToken::new()).unwrap_err(),
        );
    }
    assert!(!temporary.path().join("folder").exists());
}

#[test]
fn direct_execute_distinguishes_exact_path_bytes_from_each_one_over_case() {
    let temporary = TemporaryDirectory::new("direct-path-bytes");
    let tool = tool(temporary.path());
    let exact_canonical = "x".repeat(MAX_CREATE_FOLDER_PATH_BYTES);
    assert_tool_error(
        execute(&tool, arguments(&exact_canonical), CancellationToken::new()).unwrap_err(),
        ToolErrorKind::Unavailable,
        "create_folder_unavailable",
        "requested folder is unavailable",
        true,
    );

    let exact_requested = format!("./{}", "r".repeat(MAX_CREATE_FOLDER_PATH_BYTES - 2));
    assert_invalid_arguments(
        execute(&tool, arguments(&exact_requested), CancellationToken::new()).unwrap_err(),
    );
    let one_over_requested = format!("./{}", "r".repeat(MAX_CREATE_FOLDER_PATH_BYTES - 1));
    assert_invalid_path(
        execute(
            &tool,
            arguments(&one_over_requested),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert!(fs::read_dir(temporary.path()).unwrap().next().is_none());
}

#[test]
fn constructor_tool_and_errors_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new("constructor");
    let root_file = temporary.path().join("PRIVATE_ROOT_FILE");
    let root_link = temporary.path().join("PRIVATE_ROOT_LINK");
    fs::write(&root_file, b"not a directory").unwrap();
    symlink(temporary.path(), &root_link).unwrap();
    assert_open_error(
        CreateFolderTool::open(Path::new("PRIVATE_RELATIVE_ROOT")).unwrap_err(),
        CreateFolderToolOpenErrorKind::InvalidRoot,
        "native create_folder workspace root is invalid",
        &["PRIVATE_RELATIVE_ROOT"],
    );
    let missing = temporary.path().join("PRIVATE_MISSING_ROOT");
    assert_open_error(
        CreateFolderTool::open(&missing).unwrap_err(),
        CreateFolderToolOpenErrorKind::Unavailable,
        "native create_folder workspace root is unavailable",
        &["PRIVATE_MISSING_ROOT"],
    );
    for path in [&root_file, &root_link] {
        assert_open_error(
            CreateFolderTool::open(path).unwrap_err(),
            CreateFolderToolOpenErrorKind::InvalidFileType,
            "native create_folder workspace root is not a directory",
            &[path.file_name().unwrap().to_str().unwrap()],
        );
    }
    let debug = format!("{:?}", tool(temporary.path()));
    assert_eq!(debug, "CreateFolderTool { .. }");
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));
}

fn assert_open_error(
    error: CreateFolderToolOpenError,
    kind: CreateFolderToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    assert!(error.source().is_none());
    let debug = format!("{error:?}");
    assert_eq!(
        debug,
        format!("CreateFolderToolOpenError {{ kind: {kind:?} }}")
    );
    for secret in forbidden {
        assert!(!error.to_string().contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn preparation_and_execution_errors_are_fixed_and_never_reflect_names_or_errno() {
    let temporary = TemporaryDirectory::new("redaction");
    let tool = tool(temporary.path());
    fs::write(
        temporary.path().join("PRIVATE_EXISTING_ENDPOINT"),
        b"PRIVATE_CONTENT_SENTINEL",
    )
    .unwrap();
    let errors = [
        tool.prepare(call(arguments("/PRIVATE_ESCAPE_ENDPOINT")))
            .unwrap_err(),
        create(&tool, "PRIVATE_EXISTING_ENDPOINT").unwrap_err(),
    ];
    for error in errors {
        let display = error.to_string();
        let debug = format!("{error:?}");
        for secret in [
            "PRIVATE_ESCAPE_ENDPOINT",
            "PRIVATE_EXISTING_ENDPOINT",
            "PRIVATE_CONTENT_SENTINEL",
            "EEXIST",
        ] {
            assert!(!display.contains(secret));
            assert!(!debug.contains(secret));
        }
    }
}
