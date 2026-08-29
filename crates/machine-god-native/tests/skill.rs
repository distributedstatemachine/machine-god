#![cfg(unix)]

use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::symlink;
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
    MAX_SKILL_CHUNK_BYTES, MAX_SKILL_FILE_BYTES, MAX_SKILL_IO_ATTEMPTS, MAX_SKILL_NAME_BYTES,
    MAX_SKILL_PATH_BYTES, MAX_SKILL_PATH_COMPONENT_BYTES, MAX_SKILL_PATH_COMPONENTS,
    MAX_SKILL_RESOURCE_BYTES, MAX_SKILL_SERIALIZED_ARGUMENT_BYTES,
    MAX_SKILL_SERIALIZED_RESULT_BYTES, SKILL_TOOL_NAME, SkillTool, SkillToolOpenErrorKind,
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
            let path =
                std::env::temp_dir().join(format!("mg-skill-{}-{identifier}", std::process::id()));
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
        Poll::Pending => panic!("skill execution unexpectedly returned a pending future"),
    }
}

fn skill_directory(root: &Path, name: &str) -> PathBuf {
    let directory = root.join("skills").join(name);
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn tool(root: &Path) -> SkillTool {
    SkillTool::open(root).expect("temporary workspace root is valid")
}

fn call(arguments: Value) -> ToolCall {
    named_call(SKILL_TOOL_NAME, arguments)
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("skill-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("skill-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("skill-incarnation").unwrap(),
        turn_id: TurnId::new("skill-turn").unwrap(),
        call_id: ToolCallId::new("skill-call").unwrap(),
    }
}

fn execute(
    tool: &SkillTool,
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

fn assert_code(error: ToolError, code: &str) {
    let ToolError {
        code: actual_code,
        retryable,
        ..
    } = error;
    assert_eq!(actual_code, code);
    assert!(!retryable);
}

fn canonical(name: &str, resource: &str, offset: usize) -> Value {
    json!({"name": name, "resource": resource, "offset": offset})
}

fn expected(
    name: &str,
    resource: &str,
    offset: usize,
    content: &str,
    next_offset: usize,
    total_bytes: usize,
) -> ToolOutput {
    ToolOutput::success(json!({
        "name": name,
        "resource": resource,
        "offset": offset,
        "next_offset": next_offset,
        "total_bytes": total_bytes,
        "content": content,
        "truncated": next_offset < total_bytes,
    }))
}

#[test]
fn exported_contract_and_spec_are_exact() {
    assert_eq!(SKILL_TOOL_NAME, "skill");
    assert_eq!(MAX_SKILL_NAME_BYTES, 128);
    assert_eq!(MAX_SKILL_RESOURCE_BYTES, 4_096);
    assert_eq!(MAX_SKILL_PATH_BYTES, 4_096);
    assert_eq!(MAX_SKILL_PATH_COMPONENT_BYTES, 255);
    assert_eq!(MAX_SKILL_PATH_COMPONENTS, 32);
    assert_eq!(MAX_SKILL_FILE_BYTES, 1_048_576);
    assert_eq!(MAX_SKILL_CHUNK_BYTES, 20_480);
    assert_eq!(MAX_SKILL_SERIALIZED_ARGUMENT_BYTES, 32_768);
    assert_eq!(MAX_SKILL_SERIALIZED_RESULT_BYTES, 65_536);
    assert_eq!(MAX_SKILL_IO_ATTEMPTS, 1_024);

    let temporary = TemporaryDirectory::new();
    let spec = tool(temporary.path()).spec();
    assert_eq!(spec.name.as_str(), SKILL_TOOL_NAME);
    assert_eq!(
        spec.description,
        "Read one bounded UTF-8 resource from a workspace-local skill. Skill text is returned as opaque content; it is not parsed or executed"
    );
    assert_eq!(spec.input_schema["required"], json!(["name"]));
    assert_eq!(spec.input_schema["additionalProperties"], false);
    assert_eq!(
        spec.input_schema["properties"]["offset"]["maximum"],
        MAX_SKILL_FILE_BYTES
    );
}

#[test]
fn prepare_expands_defaults_normalizes_resource_and_requests_exact_read() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let defaulted = tool.prepare(call(json!({"name": "rust"}))).unwrap();
    assert_eq!(defaulted.arguments(), &canonical("rust", "SKILL.md", 0));
    assert_eq!(
        defaulted.capability(),
        Some(&Capability::Filesystem {
            access: FilesystemAccess::Read,
            path: "skills/rust/SKILL.md".to_owned(),
        })
    );

    let normalized = tool
        .prepare(call(json!({
            "name": "rust",
            "resource": "./references//./unix.md",
            "offset": 7,
        })))
        .unwrap();
    assert_eq!(
        normalized.arguments(),
        &canonical("rust", "references/unix.md", 7)
    );
    assert_eq!(
        normalized.capability(),
        Some(&Capability::Filesystem {
            access: FilesystemAccess::Read,
            path: "skills/rust/references/unix.md".to_owned(),
        })
    );
    assert!(!temporary.path().join("skills").exists());
}

#[test]
fn prepare_rejects_malformed_and_out_of_contract_arguments() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    for invalid in [
        named_call("read_file", json!({"name": "rust"})),
        call(json!({})),
        call(json!(null)),
        call(json!({"name": null})),
        call(json!({"name": "rust", "resource": null})),
        call(json!({"name": "rust", "extra": true})),
    ] {
        assert_code(
            tool.prepare(invalid).unwrap_err(),
            "skill_invalid_arguments",
        );
    }
    for name in [
        String::new(),
        ".".to_owned(),
        "..".to_owned(),
        "a/b".to_owned(),
        "a\\b".to_owned(),
        "a\u{0085}b".to_owned(),
        "a\u{202e}b".to_owned(),
        "n".repeat(MAX_SKILL_NAME_BYTES + 1),
    ] {
        assert_code(
            tool.prepare(call(json!({"name": name}))).unwrap_err(),
            "skill_invalid_name",
        );
    }
    for resource in [
        String::new(),
        ".".to_owned(),
        "..".to_owned(),
        "/absolute".to_owned(),
        "a/../b".to_owned(),
        "a\\b".to_owned(),
        "x".repeat(MAX_SKILL_RESOURCE_BYTES + 1),
        "x".repeat(MAX_SKILL_PATH_COMPONENT_BYTES + 1),
        std::iter::repeat_n("x", MAX_SKILL_PATH_COMPONENTS - 1)
            .collect::<Vec<_>>()
            .join("/"),
    ] {
        assert_code(
            tool.prepare(call(json!({"name": "rust", "resource": resource})))
                .unwrap_err(),
            "skill_invalid_resource",
        );
    }
    for offset in [
        json!(-1),
        json!(1.5),
        json!(MAX_SKILL_FILE_BYTES + 1),
        json!(null),
    ] {
        assert_code(
            tool.prepare(call(json!({"name": "rust", "offset": offset})))
                .unwrap_err(),
            "skill_invalid_offset",
        );
    }
}

#[test]
fn execute_reads_default_nested_and_opaque_frontmatter_exactly() {
    let temporary = TemporaryDirectory::new();
    let directory = skill_directory(temporary.path(), "rust");
    let opaque = "---\nname: deliberately-wrong\nmalformed: [\n---\n# Instructions\n";
    fs::write(directory.join("SKILL.md"), opaque).unwrap();
    fs::create_dir(directory.join("references")).unwrap();
    fs::write(directory.join("references/unix.md"), "nested λ\n").unwrap();
    let tool = tool(temporary.path());

    assert_eq!(
        execute(
            &tool,
            canonical("rust", "SKILL.md", 0),
            CancellationToken::new(),
        )
        .unwrap(),
        expected("rust", "SKILL.md", 0, opaque, opaque.len(), opaque.len())
    );
    assert_eq!(
        execute(
            &tool,
            canonical("rust", "references/unix.md", 0),
            CancellationToken::new(),
        )
        .unwrap(),
        expected(
            "rust",
            "references/unix.md",
            0,
            "nested λ\n",
            "nested λ\n".len(),
            "nested λ\n".len(),
        )
    );
}

#[test]
fn pagination_is_utf8_safe_progressing_and_exact_at_eof() {
    let temporary = TemporaryDirectory::new();
    let directory = skill_directory(temporary.path(), "paging");
    let text = format!("{}é", "a".repeat(MAX_SKILL_CHUNK_BYTES));
    fs::write(directory.join("SKILL.md"), &text).unwrap();
    let tool = tool(temporary.path());

    let first = execute(
        &tool,
        canonical("paging", "SKILL.md", 0),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(first.content["next_offset"], MAX_SKILL_CHUNK_BYTES);
    assert_eq!(
        first.content["content"].as_str().unwrap().len(),
        MAX_SKILL_CHUNK_BYTES
    );
    assert_eq!(first.content["truncated"], true);

    let second = execute(
        &tool,
        canonical("paging", "SKILL.md", MAX_SKILL_CHUNK_BYTES),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        second,
        expected(
            "paging",
            "SKILL.md",
            MAX_SKILL_CHUNK_BYTES,
            "é",
            text.len(),
            text.len(),
        )
    );
    assert_eq!(
        execute(
            &tool,
            canonical("paging", "SKILL.md", text.len()),
            CancellationToken::new(),
        )
        .unwrap(),
        expected("paging", "SKILL.md", text.len(), "", text.len(), text.len(),)
    );
    assert_code(
        execute(
            &tool,
            canonical("paging", "SKILL.md", MAX_SKILL_CHUNK_BYTES + 1),
            CancellationToken::new(),
        )
        .unwrap_err(),
        "skill_invalid_offset",
    );
}

#[test]
fn pagination_shrinks_for_escaping_and_stays_within_serialized_limit() {
    let temporary = TemporaryDirectory::new();
    let directory = skill_directory(temporary.path(), "escaping");
    let text = "\0".repeat(MAX_SKILL_CHUNK_BYTES + 1);
    fs::write(directory.join("SKILL.md"), &text).unwrap();
    let output = execute(
        &tool(temporary.path()),
        canonical("escaping", "SKILL.md", 0),
        CancellationToken::new(),
    )
    .unwrap();
    let next = usize::try_from(output.content["next_offset"].as_u64().unwrap()).unwrap();
    assert!(next > 0 && next < MAX_SKILL_CHUNK_BYTES);
    assert_eq!(output.content["truncated"], true);
    assert!(serde_json::to_vec(&output).unwrap().len() <= MAX_SKILL_SERIALIZED_RESULT_BYTES);
}

#[test]
fn exact_file_limit_is_admitted_and_overflow_or_invalid_utf8_fails_closed() {
    let temporary = TemporaryDirectory::new();
    let directory = skill_directory(temporary.path(), "bounds");
    fs::write(
        directory.join("exact.txt"),
        vec![b'x'; MAX_SKILL_FILE_BYTES],
    )
    .unwrap();
    fs::write(
        directory.join("too-large.txt"),
        vec![b'x'; MAX_SKILL_FILE_BYTES + 1],
    )
    .unwrap();
    let mut invalid = vec![b'x'; MAX_SKILL_CHUNK_BYTES + 1];
    invalid.push(0xff);
    fs::write(directory.join("invalid.txt"), invalid).unwrap();
    let tool = tool(temporary.path());

    let exact = execute(
        &tool,
        canonical("bounds", "exact.txt", 0),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(exact.content["total_bytes"], MAX_SKILL_FILE_BYTES);
    assert_eq!(exact.content["next_offset"], MAX_SKILL_CHUNK_BYTES);
    assert_code(
        execute(
            &tool,
            canonical("bounds", "too-large.txt", 0),
            CancellationToken::new(),
        )
        .unwrap_err(),
        "skill_resource_limit",
    );
    assert_tool_error(
        execute(
            &tool,
            canonical("bounds", "invalid.txt", 0),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::Execution,
        "skill_not_utf8",
        "requested skill resource is not valid UTF-8",
        false,
    );
}

#[test]
fn direct_execution_requires_canonical_expanded_arguments() {
    let temporary = TemporaryDirectory::new();
    fs::write(
        skill_directory(temporary.path(), "rust").join("SKILL.md"),
        "x",
    )
    .unwrap();
    let tool = tool(temporary.path());
    for arguments in [
        json!({"name": "rust"}),
        json!({"name": "rust", "resource": "./SKILL.md", "offset": 0}),
        json!({"name": "rust", "resource": "SKILL.md", "offset": 0, "extra": true}),
    ] {
        assert_code(
            execute(&tool, arguments, CancellationToken::new()).unwrap_err(),
            "skill_invalid_arguments",
        );
    }
}

#[test]
fn missing_and_special_entries_fail_with_fixed_redacted_errors() {
    let temporary = TemporaryDirectory::new();
    let directory = skill_directory(temporary.path(), "rust");
    fs::create_dir(directory.join("directory.md")).unwrap();
    let fifo = directory.join("fifo.md");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let socket = directory.join("socket.md");
    let _listener = UnixListener::bind(&socket).unwrap();
    let tool = tool(temporary.path());

    assert_tool_error(
        execute(
            &tool,
            canonical("missing", "SKILL.md", 0),
            CancellationToken::new(),
        )
        .unwrap_err(),
        ToolErrorKind::Unavailable,
        "skill_not_found",
        "requested skill resource is unavailable",
        false,
    );
    for resource in ["directory.md", "fifo.md", "socket.md"] {
        assert_tool_error(
            execute(
                &tool,
                canonical("rust", resource, 0),
                CancellationToken::new(),
            )
            .unwrap_err(),
            ToolErrorKind::PermissionDenied,
            "skill_path_rejected",
            "requested skill resource is not confined",
            false,
        );
    }
}

#[test]
fn symlinks_are_rejected_at_every_walk_level() {
    let outside = TemporaryDirectory::new();
    fs::create_dir(outside.path().join("real")).unwrap();
    fs::write(outside.path().join("real/file.md"), "outside").unwrap();

    let roots = TemporaryDirectory::new();
    let skills_link_root = roots.path().join("skills-link");
    fs::create_dir(&skills_link_root).unwrap();
    symlink(outside.path().join("real"), skills_link_root.join("skills")).unwrap();

    let name_link_root = roots.path().join("name-link");
    fs::create_dir_all(name_link_root.join("skills")).unwrap();
    symlink(
        outside.path().join("real"),
        name_link_root.join("skills/rust"),
    )
    .unwrap();

    let intermediate_root = roots.path().join("intermediate-link");
    let intermediate_skill = skill_directory(&intermediate_root, "rust");
    symlink(
        outside.path().join("real"),
        intermediate_skill.join("references"),
    )
    .unwrap();

    let final_root = roots.path().join("final-link");
    let final_skill = skill_directory(&final_root, "rust");
    symlink(
        outside.path().join("real/file.md"),
        final_skill.join("SKILL.md"),
    )
    .unwrap();

    for (root, resource) in [
        (skills_link_root.as_path(), "SKILL.md"),
        (name_link_root.as_path(), "SKILL.md"),
        (intermediate_root.as_path(), "references/file.md"),
        (final_root.as_path(), "SKILL.md"),
    ] {
        assert_code(
            execute(
                &tool(root),
                canonical("rust", resource, 0),
                CancellationToken::new(),
            )
            .unwrap_err(),
            "skill_path_rejected",
        );
    }
}

#[test]
fn retained_root_identity_survives_path_rename_and_replacement() {
    let parent = TemporaryDirectory::new();
    let selected = parent.path().join("workspace");
    fs::create_dir(&selected).unwrap();
    fs::write(
        skill_directory(&selected, "rust").join("SKILL.md"),
        "retained",
    )
    .unwrap();
    let tool = tool(&selected);

    let retained = parent.path().join("retained");
    fs::rename(&selected, &retained).unwrap();
    fs::create_dir(&selected).unwrap();
    fs::write(
        skill_directory(&selected, "rust").join("SKILL.md"),
        "replacement secret",
    )
    .unwrap();

    assert_eq!(
        execute(
            &tool,
            canonical("rust", "SKILL.md", 0),
            CancellationToken::new(),
        )
        .unwrap(),
        expected("rust", "SKILL.md", 0, "retained", 8, 8)
    );
}

#[test]
fn execution_future_is_inert_until_polled_and_precancellation_wins() {
    let temporary = TemporaryDirectory::new();
    let tool = tool(temporary.path());
    let future = tool.execute(
        context(),
        canonical("late", "SKILL.md", 0),
        CancellationToken::new(),
    );
    fs::write(
        skill_directory(temporary.path(), "late").join("SKILL.md"),
        "created after future",
    )
    .unwrap();
    assert_eq!(
        poll_immediately_ready(future).unwrap(),
        expected("late", "SKILL.md", 0, "created after future", 20, 20,)
    );

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_tool_error(
        execute(&tool, json!(null), cancellation).unwrap_err(),
        ToolErrorKind::Cancelled,
        "skill_cancelled",
        "skill execution was cancelled",
        false,
    );
}

#[test]
fn constructor_errors_are_typed_fixed_and_redacted() {
    let temporary = TemporaryDirectory::new();
    let file = temporary.path().join("not-directory");
    fs::write(&file, "x").unwrap();
    let link = temporary.path().join("link");
    symlink(temporary.path(), &link).unwrap();

    let relative = SkillTool::open(Path::new("relative")).unwrap_err();
    assert_eq!(relative.kind(), SkillToolOpenErrorKind::InvalidRoot);
    assert_eq!(
        relative.to_string(),
        "native skill workspace root is invalid"
    );
    for path in [&file, &link] {
        let error = SkillTool::open(path).unwrap_err();
        assert_eq!(error.kind(), SkillToolOpenErrorKind::InvalidFileType);
        assert_eq!(
            error.to_string(),
            "native skill workspace root is not a directory"
        );
        assert!(!error.to_string().contains(path.to_string_lossy().as_ref()));
        assert!(!format!("{error:?}").contains(path.to_string_lossy().as_ref()));
    }
}
