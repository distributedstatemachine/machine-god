#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::error::Error;
use std::fs::{self, File, FileTimes};
use std::future::Future;
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, SystemTime};

use machine_god_core::{
    CancellationToken, Capability, Tool, ToolCall, ToolCallId, ToolContext, ToolError,
    ToolErrorKind, ToolName, ToolOutput,
};
use machine_god_native::{
    COPY_FILE_TOOL_NAME, CopyFileTool, CopyFileToolOpenError, CopyFileToolOpenErrorKind,
    MAX_COPY_FILE_CHUNK_BYTES, MAX_COPY_FILE_IO_CALLS, MAX_COPY_FILE_PATH_BYTES,
    MAX_COPY_FILE_PATH_COMPONENTS, MAX_COPY_FILE_SERIALIZED_ARGUMENT_BYTES,
    MAX_COPY_FILE_SERIALIZED_RESULT_BYTES, MAX_COPY_FILE_SOURCE_BYTES, MAX_COPY_FILE_TEMP_ATTEMPTS,
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
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("mg-copy-file-{}-{identifier}", std::process::id()));
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
        Poll::Pending => panic!("copy_file unexpectedly yielded"),
    }
}

fn tool(root: &Path) -> CopyFileTool {
    CopyFileTool::open(root).unwrap()
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("copy-file-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn call(arguments: Value) -> ToolCall {
    named_call(COPY_FILE_TOOL_NAME, arguments)
}

fn context() -> ToolContext {
    ToolContext {
        session_id: machine_god_core::SessionId::new("copy-session").unwrap(),
        session_incarnation_id: machine_god_core::SessionIncarnationId::new("copy-incarnation")
            .unwrap(),
        turn_id: machine_god_core::TurnId::new("copy-turn").unwrap(),
        call_id: ToolCallId::new("copy-file-call").unwrap(),
    }
}

fn arguments(source: &str, destination: &str) -> Value {
    json!({"source": source, "destination": destination})
}

fn execute(
    tool: &CopyFileTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_immediately_ready(tool.execute(context(), arguments, cancellation))
}

fn copy(tool: &CopyFileTool, source: &str, destination: &str) -> Result<ToolOutput, ToolError> {
    execute(
        tool,
        arguments(source, destination),
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
        "copy_file_invalid_arguments",
        "copy_file arguments are invalid",
        false,
    );
}

fn assert_invalid_path(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "copy_file_invalid_path",
        "copy_file path is invalid",
        false,
    );
}

fn assert_not_found(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::Unavailable,
        "copy_file_not_found",
        "copy source or destination parent is unavailable",
        false,
    );
}

fn assert_path_rejected(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::PermissionDenied,
        "copy_file_path_rejected",
        "requested copy path is not confined",
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

fn stage_entries(root: &Path) -> Vec<String> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".machine-god-copy-"))
        .collect()
}

#[test]
fn exported_contract_limits_and_spec_are_exact() {
    assert_eq!(COPY_FILE_TOOL_NAME, "copy_file");
    assert_eq!(MAX_COPY_FILE_PATH_BYTES, 4_096);
    assert_eq!(MAX_COPY_FILE_PATH_COMPONENTS, 256);
    assert_eq!(MAX_COPY_FILE_SOURCE_BYTES, 16_777_216);
    assert_eq!(MAX_COPY_FILE_CHUNK_BYTES, 65_536);
    assert_eq!(MAX_COPY_FILE_IO_CALLS, 4_096);
    assert_eq!(MAX_COPY_FILE_TEMP_ATTEMPTS, 8);
    assert_eq!(MAX_COPY_FILE_SERIALIZED_ARGUMENT_BYTES, 65_536);
    assert_eq!(MAX_COPY_FILE_SERIALIZED_RESULT_BYTES, 16_384);

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), COPY_FILE_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Copy one existing regular file to an absent path within the configured workspace"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "source": {
                    "type": "string",
                    "description": "Source workspace-relative regular-file path"
                },
                "destination": {
                    "type": "string",
                    "description": "Destination workspace-relative file path"
                }
            },
            "required": ["source", "destination"],
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
        json!({"source": "source"}),
        json!({"destination": "destination"}),
        json!({"source": 1, "destination": "destination"}),
        json!({"source": "source", "destination": false}),
        json!({"source": "source", "destination": "destination", "overwrite": false}),
    ] {
        assert_invalid_arguments(tool.prepare(call(invalid)).unwrap_err());
    }
    assert_invalid_arguments(
        tool.prepare(named_call(
            "rename_file",
            arguments("source", "destination"),
        ))
        .unwrap_err(),
    );
}

#[test]
fn prepare_normalizes_both_paths_and_requests_exact_copy_authority() {
    let temporary = TemporaryDirectory::new();
    let prepared = tool(temporary.path())
        .prepare(call(arguments(
            "./source//literal\\ λ.bin",
            "./destination//literal\\ λ.bin",
        )))
        .unwrap();
    assert_eq!(
        prepared.capability(),
        &Capability::FilesystemCopy {
            source: "source/literal\\ λ.bin".to_owned(),
            destination: "destination/literal\\ λ.bin".to_owned(),
        }
    );
    assert_eq!(
        prepared.arguments(),
        &arguments("source/literal\\ λ.bin", "destination/literal\\ λ.bin")
    );
}

#[test]
fn prepare_enforces_each_path_and_component_bound() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let exact_source = "s".repeat(MAX_COPY_FILE_PATH_BYTES);
    let exact_destination = "d".repeat(MAX_COPY_FILE_PATH_BYTES);
    assert!(
        tool.prepare(call(arguments(&exact_source, &exact_destination)))
            .is_ok()
    );
    for invalid in [
        arguments(&format!("{exact_source}x"), "destination"),
        arguments("source", &format!("{exact_destination}x")),
    ] {
        assert_invalid_path(tool.prepare(call(invalid)).unwrap_err());
    }

    let exact_requested_source = format!("./{}", "r".repeat(MAX_COPY_FILE_PATH_BYTES - 2));
    let exact_requested_destination = format!("./{}", "q".repeat(MAX_COPY_FILE_PATH_BYTES - 2));
    let prepared = tool
        .prepare(call(arguments(
            &exact_requested_source,
            &exact_requested_destination,
        )))
        .unwrap();
    assert_eq!(
        prepared.arguments(),
        &arguments(
            &"r".repeat(MAX_COPY_FILE_PATH_BYTES - 2),
            &"q".repeat(MAX_COPY_FILE_PATH_BYTES - 2)
        )
    );
    for invalid in [
        arguments(
            &format!("./{}", "r".repeat(MAX_COPY_FILE_PATH_BYTES - 1)),
            "destination",
        ),
        arguments(
            "source",
            &format!("./{}", "q".repeat(MAX_COPY_FILE_PATH_BYTES - 1)),
        ),
    ] {
        assert_invalid_path(tool.prepare(call(invalid)).unwrap_err());
    }

    let exact_components = (0..MAX_COPY_FILE_PATH_COMPONENTS)
        .map(|index| format!("c{index}"))
        .collect::<Vec<_>>()
        .join("/");
    assert!(
        tool.prepare(call(arguments(
            &exact_components,
            &format!("{exact_components}-copy")
        )))
        .is_ok()
    );
    let one_over = format!("{exact_components}/extra");
    for invalid in [
        arguments(&one_over, "destination"),
        arguments("source", &one_over),
    ] {
        assert_invalid_path(tool.prepare(call(invalid)).unwrap_err());
    }
}

#[test]
fn prepare_rejects_escapes_controls_roots_and_equal_canonical_paths() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for (source, destination) in [
        ("", "destination"),
        ("source", ""),
        ("/source", "destination"),
        ("source", "/destination"),
        ("../source", "destination"),
        ("source", "destination/../escape"),
        (".", "destination"),
        ("source", "."),
        ("source\n", "destination"),
        ("source\u{0085}", "destination"),
        ("source", "destination\u{061c}"),
        ("source", "destination\u{200e}"),
        ("source", "destination\u{2028}"),
        ("source", "destination\u{2029}"),
        ("source", "destination\u{202e}"),
        ("source", "destination\u{2066}"),
        ("source", "destination\u{2069}"),
    ] {
        assert_invalid_path(
            tool.prepare(call(arguments(source, destination)))
                .unwrap_err(),
        );
    }
    for (source, destination) in [("same", "same"), ("./same", "same/.")] {
        assert_invalid_arguments(
            tool.prepare(call(arguments(source, destination)))
                .unwrap_err(),
        );
    }
}

#[test]
fn preparation_is_effect_free_for_every_runtime_shape() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("source"), b"source").unwrap();
    fs::write(temporary.path().join("destination"), b"destination").unwrap();
    create_fifo(&temporary.path().join("fifo"));
    symlink("source", temporary.path().join("source-link")).unwrap();
    let before = fs::read_dir(temporary.path()).unwrap().count();
    let tool = tool(temporary.path());
    for pair in [
        ("missing", "new"),
        ("source", "destination"),
        ("source", "missing/parent/new"),
        ("fifo", "new-from-fifo"),
        ("source-link", "new-from-link"),
    ] {
        assert!(tool.prepare(call(arguments(pair.0, pair.1))).is_ok());
    }
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), before);
    assert_eq!(
        fs::read(temporary.path().join("source")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(temporary.path().join("destination")).unwrap(),
        b"destination"
    );
    assert!(stage_entries(temporary.path()).is_empty());
}

#[test]
fn preparation_does_not_reinspect_a_removed_retained_root() {
    let temporary = TemporaryDirectory::new();
    let workspace = temporary.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let tool = tool(&workspace);
    fs::remove_dir(&workspace).unwrap();

    let prepared = tool
        .prepare(call(arguments("missing-source", "missing-destination")))
        .unwrap();

    assert_eq!(
        prepared.capability(),
        &Capability::FilesystemCopy {
            source: "missing-source".to_owned(),
            destination: "missing-destination".to_owned(),
        }
    );
    assert!(!workspace.exists());
}

#[test]
fn execute_copies_empty_text_binary_and_cross_parent_files_without_source_mutation() {
    let temporary = TemporaryDirectory::new();
    fs::create_dir(temporary.path().join("source-parent")).unwrap();
    fs::create_dir(temporary.path().join("destination-parent")).unwrap();
    let cases: [(&str, &str, &[u8]); 3] = [
        ("empty", "empty-copy", b""),
        ("text", "text-copy", b"copy contents\n"),
        (
            "source-parent/binary",
            "destination-parent/binary-copy",
            b"\0\xff\x80binary\0bytes",
        ),
    ];
    let tool = tool(temporary.path());
    for (source, destination, bytes) in cases {
        fs::write(temporary.path().join(source), bytes).unwrap();
        let source_metadata = fs::metadata(temporary.path().join(source)).unwrap();
        assert_eq!(
            copy(&tool, source, destination).unwrap(),
            ToolOutput::success(json!({
                "source": source,
                "destination": destination,
                "bytes_copied": bytes.len()
            }))
        );
        assert_eq!(fs::read(temporary.path().join(source)).unwrap(), bytes);
        assert_eq!(fs::read(temporary.path().join(destination)).unwrap(), bytes);
        let destination_metadata = fs::metadata(temporary.path().join(destination)).unwrap();
        assert_eq!(
            source_metadata.ino(),
            fs::metadata(temporary.path().join(source)).unwrap().ino()
        );
        assert_ne!(source_metadata.ino(), destination_metadata.ino());
    }
    assert!(stage_entries(temporary.path()).is_empty());
    assert!(stage_entries(&temporary.path().join("destination-parent")).is_empty());
}

#[test]
fn execute_preserves_exact_bytes_across_the_public_chunk_boundary() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for (label, length) in [
        ("exact-chunk", MAX_COPY_FILE_CHUNK_BYTES),
        ("one-over-chunk", MAX_COPY_FILE_CHUNK_BYTES + 1),
    ] {
        let bytes = (0..length)
            .map(|index| u8::try_from(index % 251).unwrap())
            .collect::<Vec<_>>();
        let destination = format!("{label}-copy");
        fs::write(temporary.path().join(label), &bytes).unwrap();

        assert_eq!(
            copy(&tool, label, &destination).unwrap(),
            ToolOutput::success(json!({
                "source": label,
                "destination": destination,
                "bytes_copied": length
            }))
        );
        assert_eq!(fs::read(temporary.path().join(label)).unwrap(), bytes);
        assert_eq!(
            fs::read(temporary.path().join(format!("{label}-copy"))).unwrap(),
            bytes
        );
    }
}

#[test]
fn execute_accepts_exact_source_limit_and_rejects_one_over_without_residue() {
    let temporary = TemporaryDirectory::new();
    let exact = temporary.path().join("exact");
    let bytes = vec![0xA5; MAX_COPY_FILE_SOURCE_BYTES];
    fs::write(&exact, &bytes).unwrap();
    let tool = tool(temporary.path());
    assert_eq!(
        copy(&tool, "exact", "exact-copy").unwrap(),
        ToolOutput::success(json!({
            "source": "exact",
            "destination": "exact-copy",
            "bytes_copied": MAX_COPY_FILE_SOURCE_BYTES
        }))
    );
    assert_eq!(
        fs::read(temporary.path().join("exact-copy")).unwrap(),
        bytes
    );

    let oversized = File::create(temporary.path().join("oversized")).unwrap();
    oversized
        .set_len(u64::try_from(MAX_COPY_FILE_SOURCE_BYTES + 1).unwrap())
        .unwrap();
    assert_tool_error(
        copy(&tool, "oversized", "oversized-copy").unwrap_err(),
        ToolErrorKind::InvalidInput,
        "copy_file_source_too_large",
        "copy source exceeds the supported size limit",
        false,
    );
    assert!(!temporary.path().join("oversized-copy").exists());
    assert!(stage_entries(temporary.path()).is_empty());
}

#[test]
fn destination_is_never_overwritten_and_missing_parents_are_never_created() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("source"), b"source").unwrap();
    fs::write(temporary.path().join("destination"), b"destination").unwrap();
    let tool = tool(temporary.path());
    assert_tool_error(
        copy(&tool, "source", "destination").unwrap_err(),
        ToolErrorKind::Execution,
        "copy_file_destination_exists",
        "copy destination already exists",
        false,
    );
    assert_eq!(
        fs::read(temporary.path().join("source")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(temporary.path().join("destination")).unwrap(),
        b"destination"
    );

    assert_not_found(copy(&tool, "source", "missing/parent/new").unwrap_err());
    assert!(!temporary.path().join("missing").exists());
    assert!(stage_entries(temporary.path()).is_empty());
}

#[test]
fn unreadable_source_is_preserved_and_reports_fixed_permission_denied() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.path().join("source");
    fs::write(&source, b"private bytes").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o000)).unwrap();

    let result = copy(&tool(temporary.path()), "source", "destination");

    fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
    assert_tool_error(
        result.unwrap_err(),
        ToolErrorKind::PermissionDenied,
        "copy_file_permission_denied",
        "requested copy is not permitted",
        false,
    );
    assert_eq!(fs::read(source).unwrap(), b"private bytes");
    assert!(!temporary.path().join("destination").exists());
    assert!(stage_entries(temporary.path()).is_empty());
}

#[test]
fn source_mode_is_copied_as_ordinary_bits_and_special_bits_are_stripped() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.path().join("executable");
    fs::write(&source, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o6751)).unwrap();
    let observed_source_mode = fs::metadata(&source).unwrap().mode() & 0o777;

    copy(&tool(temporary.path()), "executable", "executable-copy").unwrap();

    assert_eq!(
        fs::metadata(temporary.path().join("executable-copy"))
            .unwrap()
            .mode()
            & 0o7777,
        observed_source_mode
    );
    assert_eq!(fs::read(source).unwrap(), b"#!/bin/sh\n");
}

#[test]
fn source_timestamp_is_unchanged_and_is_not_copied_to_the_destination() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.path().join("source");
    fs::write(&source, b"timestamp bytes").unwrap();
    let source_modified = SystemTime::UNIX_EPOCH + Duration::new(1_600_000_123, 456_789_000);
    File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(FileTimes::new().set_modified(source_modified))
        .unwrap();

    copy(&tool(temporary.path()), "source", "destination").unwrap();

    assert_eq!(
        fs::metadata(&source).unwrap().modified().unwrap(),
        source_modified
    );
    assert_ne!(
        fs::metadata(temporary.path().join("destination"))
            .unwrap()
            .modified()
            .unwrap(),
        source_modified
    );
}

#[test]
fn copied_mode_is_exact_under_a_hostile_umask() {
    const CHILD_MARKER: &str = "MACHINE_GOD_COPY_FILE_UMASK_CHILD";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let executable = std::env::current_exe().unwrap();
        let status = Command::new("sh")
            .arg("-c")
            .arg(
                "umask 077; exec \"$1\" --exact copied_mode_is_exact_under_a_hostile_umask --nocapture",
            )
            .arg("machine-god-copy-file-umask")
            .arg(executable)
            .env(CHILD_MARKER, "1")
            .status()
            .expect("failed to execute isolated hostile-umask test process");
        assert!(status.success(), "hostile-umask child failed with {status}");
        return;
    }

    let temporary = TemporaryDirectory::new();
    let source = temporary.path().join("source");
    fs::write(&source, b"mode bytes").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o664)).unwrap();

    copy(&tool(temporary.path()), "source", "destination").unwrap();

    assert_eq!(
        fs::metadata(temporary.path().join("destination"))
            .unwrap()
            .mode()
            & 0o777,
        0o664
    );
}

#[cfg(target_os = "macos")]
#[test]
fn published_copy_clears_file_inherited_acl_without_changing_parent_acl() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.path().join("source");
    fs::write(&source, b"acl bytes").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
    let status = Command::new("/bin/chmod")
        .args(["+a", "everyone allow read,write,append,file_inherit"])
        .arg(temporary.path())
        .status()
        .expect("macOS chmod executable is available");
    assert!(
        status.success(),
        "failed to install file-inheritable ACL fixture: {status}"
    );
    let _acl_cleanup = MacAclCleanup(temporary.path().to_owned());

    let witness_path = temporary.path().join("inheritance-witness");
    fs::write(&witness_path, b"witness").unwrap();
    let witness = rustix::fs::open(
        &witness_path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    let witness_acl = calcifer_macos_acl::read_acl(witness.as_fd()).unwrap();
    assert!(witness_acl.entries.iter().any(|entry| {
        entry.tag == calcifer_macos_acl::TAG_ALLOW
            && entry.flags & calcifer_macos_acl::FLAG_INHERITED != 0
    }));
    fs::remove_file(&witness_path).unwrap();

    copy(&tool(temporary.path()), "source", "destination").unwrap();

    let published = rustix::fs::open(
        temporary.path().join("destination"),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    let published_acl = calcifer_macos_acl::read_acl(published.as_fd()).unwrap();
    assert!(
        published_acl.is_empty(),
        "published ACL was not cleared: {published_acl:?}"
    );
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
    assert_eq!(fs::read(&source).unwrap(), b"acl bytes");
    assert_eq!(
        fs::read(temporary.path().join("destination")).unwrap(),
        b"acl bytes"
    );
}

#[test]
fn source_and_destination_entry_types_fail_closed_without_escape() {
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

    assert_not_found(copy(&tool, "missing", "new").unwrap_err());
    for source in [
        "directory",
        "final-link",
        "fifo",
        "socket",
        "ancestor-link/sentinel",
    ] {
        assert_path_rejected(copy(&tool, source, "new").unwrap_err());
    }
    for destination in ["directory", "final-link", "fifo", "socket"] {
        assert_tool_error(
            copy(&tool, "source", destination).unwrap_err(),
            ToolErrorKind::Execution,
            "copy_file_destination_exists",
            "copy destination already exists",
            false,
        );
    }
    assert_path_rejected(copy(&tool, "source", "ancestor-link/new").unwrap_err());
    drop(listener);
    assert_eq!(
        fs::read(workspace.path().join("source")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(outside.path().join("sentinel")).unwrap(),
        b"outside"
    );
    assert!(stage_entries(workspace.path()).is_empty());
}

#[test]
fn device_source_is_rejected_before_any_destination_effect() {
    let device_tool = CopyFileTool::open(Path::new("/dev")).unwrap();

    assert_path_rejected(copy(&device_tool, "null", "PRIVATE_COPY_DESTINATION").unwrap_err());
    assert!(!Path::new("/dev/PRIVATE_COPY_DESTINATION").exists());
}

#[test]
fn copy_breaks_hard_link_identity_and_keeps_source_descriptors_valid() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.path().join("source");
    let hard_link = temporary.path().join("hard-link");
    fs::write(&source, b"retained bytes").unwrap();
    fs::hard_link(&source, &hard_link).unwrap();
    let source_inode = fs::metadata(&source).unwrap().ino();
    let mut opened = File::open(&source).unwrap();

    copy(&tool(temporary.path()), "source", "destination").unwrap();

    let destination = temporary.path().join("destination");
    assert_eq!(fs::metadata(&source).unwrap().ino(), source_inode);
    assert_eq!(fs::metadata(&hard_link).unwrap().ino(), source_inode);
    assert_ne!(fs::metadata(&destination).unwrap().ino(), source_inode);
    let mut opened_bytes = Vec::new();
    opened.read_to_end(&mut opened_bytes).unwrap();
    assert_eq!(opened_bytes, b"retained bytes");
    assert_eq!(fs::read(source).unwrap(), b"retained bytes");
    assert_eq!(fs::read(hard_link).unwrap(), b"retained bytes");
    assert_eq!(fs::read(destination).unwrap(), b"retained bytes");
}

#[test]
fn retained_root_rename_replacement_and_removal_cannot_redirect_authority() {
    let temporary = TemporaryDirectory::new();
    let original = temporary.path().join("workspace");
    let retained = temporary.path().join("retained");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("source"), b"retained source").unwrap();
    let copy_tool = tool(&original);
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("source"), b"replacement source").unwrap();

    copy(&copy_tool, "source", "destination").unwrap();
    assert_eq!(
        fs::read(retained.join("source")).unwrap(),
        b"retained source"
    );
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
        copy(&removed_tool, "source", "destination").unwrap_err(),
        ToolErrorKind::Unavailable,
        "copy_file_unavailable",
        "requested copy is unavailable",
        true,
    );
    assert_eq!(fs::read(removed.join("source")).unwrap(), b"replacement");
    assert!(!removed.join("destination").exists());
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
    assert!(!temporary.path().join("destination").exists());
    assert_eq!(
        poll_immediately_ready(future).unwrap(),
        ToolOutput::success(json!({
            "source": "source",
            "destination": "destination",
            "bytes_copied": 6
        }))
    );

    fs::remove_file(temporary.path().join("destination")).unwrap();
    let dropped = tool.execute(
        context(),
        arguments("source", "destination"),
        CancellationToken::new(),
    );
    drop(dropped);
    assert!(!temporary.path().join("destination").exists());
    assert!(stage_entries(temporary.path()).is_empty());

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    assert_tool_error(
        execute(&tool, arguments("source", "destination"), cancellation).unwrap_err(),
        ToolErrorKind::Cancelled,
        "copy_file_cancelled",
        "copy_file execution was cancelled",
        false,
    );
    assert_eq!(
        fs::read(temporary.path().join("source")).unwrap(),
        b"source"
    );
    assert!(!temporary.path().join("destination").exists());
    assert!(stage_entries(temporary.path()).is_empty());
}

#[test]
fn direct_execute_requires_exact_canonical_arguments_and_reapplies_limits() {
    let temporary = TemporaryDirectory::new();
    fs::write(temporary.path().join("source"), b"source").unwrap();
    let tool = tool(temporary.path());
    for invalid in [
        json!({}),
        json!({"source": "source"}),
        json!({"source": "source", "destination": 1}),
        json!({"source": "source", "destination": "destination", "extra": true}),
        arguments("./source", "destination"),
        arguments("source", "folder//destination"),
        arguments("source", "source"),
    ] {
        assert_invalid_arguments(execute(&tool, invalid, CancellationToken::new()).unwrap_err());
    }
    assert_invalid_path(
        execute(
            &tool,
            arguments(&"x".repeat(MAX_COPY_FILE_PATH_BYTES + 1), "destination"),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_eq!(
        fs::read(temporary.path().join("source")).unwrap(),
        b"source"
    );
    assert!(!temporary.path().join("destination").exists());
}

#[test]
fn constructor_tool_and_errors_are_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let root_file = temporary.path().join("PRIVATE_ROOT_FILE");
    let root_link = temporary.path().join("PRIVATE_ROOT_LINK");
    fs::write(&root_file, b"not a directory").unwrap();
    symlink(temporary.path(), &root_link).unwrap();
    assert_open_error(
        CopyFileTool::open(Path::new("PRIVATE_RELATIVE_ROOT")).unwrap_err(),
        CopyFileToolOpenErrorKind::InvalidRoot,
        "native copy_file workspace root is invalid",
        &["PRIVATE_RELATIVE_ROOT"],
    );
    let missing = temporary.path().join("PRIVATE_MISSING_ROOT");
    assert_open_error(
        CopyFileTool::open(&missing).unwrap_err(),
        CopyFileToolOpenErrorKind::Unavailable,
        "native copy_file workspace root is unavailable",
        &["PRIVATE_MISSING_ROOT"],
    );
    for path in [&root_file, &root_link] {
        assert_open_error(
            CopyFileTool::open(path).unwrap_err(),
            CopyFileToolOpenErrorKind::InvalidFileType,
            "native copy_file workspace root is not a directory",
            &[path.file_name().unwrap().to_str().unwrap()],
        );
    }
    let debug = format!("{:?}", tool(temporary.path()));
    assert_eq!(debug, "CopyFileTool { .. }");
    assert!(!debug.contains(temporary.path().to_string_lossy().as_ref()));
}

fn assert_open_error(
    error: CopyFileToolOpenError,
    kind: CopyFileToolOpenErrorKind,
    display: &str,
    forbidden: &[&str],
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), display);
    assert!(error.source().is_none());
    let debug = format!("{error:?}");
    assert_eq!(debug, format!("CopyFileToolOpenError {{ kind: {kind:?} }}"));
    for secret in forbidden {
        assert!(!error.to_string().contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn preparation_and_execution_errors_never_reflect_either_endpoint_or_content() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    fs::write(
        temporary.path().join("PRIVATE_SOURCE_ENDPOINT"),
        b"PRIVATE_SOURCE_CONTENT_SENTINEL",
    )
    .unwrap();
    fs::write(
        temporary.path().join("PRIVATE_EXISTING_DESTINATION"),
        b"PRIVATE_DESTINATION_CONTENT_SENTINEL",
    )
    .unwrap();
    let errors = [
        tool.prepare(call(arguments(
            "/PRIVATE_SOURCE_ENDPOINT",
            "PRIVATE_DESTINATION_ENDPOINT",
        )))
        .unwrap_err(),
        copy(
            &tool,
            "PRIVATE_MISSING_SOURCE",
            "PRIVATE_DESTINATION_ENDPOINT",
        )
        .unwrap_err(),
        copy(
            &tool,
            "PRIVATE_SOURCE_ENDPOINT",
            "PRIVATE_EXISTING_DESTINATION",
        )
        .unwrap_err(),
    ];
    for error in errors {
        let display = error.to_string();
        let debug = format!("{error:?}");
        for secret in [
            "PRIVATE_SOURCE_ENDPOINT",
            "PRIVATE_DESTINATION_ENDPOINT",
            "PRIVATE_MISSING_SOURCE",
            "PRIVATE_EXISTING_DESTINATION",
            "PRIVATE_SOURCE_CONTENT_SENTINEL",
            "PRIVATE_DESTINATION_CONTENT_SENTINEL",
        ] {
            assert!(!display.contains(secret));
            assert!(!debug.contains(secret));
        }
    }
}
