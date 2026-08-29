#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::error::Error;
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    CancellationToken, Capability, PreparedToolAuthorization, SessionId, SessionIncarnationId,
    Tool, ToolCall, ToolCallId, ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput,
    TurnId,
};
use machine_god_native::{
    MAX_MEMORY_FACT_BYTES, MAX_MEMORY_FACTS, MAX_MEMORY_FILE_BYTES, MAX_MEMORY_IO_ATTEMPTS,
    MAX_MEMORY_SERIALIZED_ARGUMENT_BYTES, MAX_MEMORY_SERIALIZED_RESULT_BYTES,
    MAX_MEMORY_TOTAL_FACT_BYTES, MEMORY_SCHEMA_VERSION, MEMORY_TOOL_NAME, MemoryTool,
    MemoryToolOpenErrorKind,
};
use rustix::fs::FlockOperation;
use serde_json::{Value, json};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-memory-{label}-{}-{identifier}",
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

fn poll_ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("memory unexpectedly yielded"),
    }
}

fn tool(root: &Path) -> MemoryTool {
    MemoryTool::open(root).expect("temporary state root is valid")
}

fn named_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("memory-call").unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments,
    }
}

fn call(arguments: Value) -> ToolCall {
    named_call(MEMORY_TOOL_NAME, arguments)
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("memory-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("memory-incarnation").unwrap(),
        turn_id: TurnId::new("memory-turn").unwrap(),
        call_id: ToolCallId::new("memory-call").unwrap(),
    }
}

fn execute(
    tool: &MemoryTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_ready(tool.execute(context(), arguments, cancellation))
}

fn run(tool: &MemoryTool, arguments: Value) -> ToolOutput {
    execute(tool, arguments, CancellationToken::new()).unwrap()
}

fn save(tool: &MemoryTool, fact: &str) -> ToolOutput {
    run(tool, json!({"action": "save", "fact": fact}))
}

fn list(tool: &MemoryTool) -> ToolOutput {
    run(tool, json!({"action": "list"}))
}

fn clear(tool: &MemoryTool) -> ToolOutput {
    run(tool, json!({"action": "clear"}))
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
    drop(error);
}

fn assert_invalid_arguments(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "memory_invalid_arguments",
        "memory arguments are invalid",
        false,
    );
}

fn assert_invalid_fact(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "memory_invalid_fact",
        "memory fact is invalid",
        false,
    );
}

fn assert_resource_limit(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::InvalidInput,
        "memory_resource_limit",
        "memory resource limit was exceeded",
        false,
    );
}

fn assert_corrupt(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::Execution,
        "memory_state_corrupt",
        "memory state is corrupt",
        false,
    );
}

fn assert_cancelled(error: ToolError) {
    assert_tool_error(
        error,
        ToolErrorKind::Cancelled,
        "memory_cancelled",
        "memory operation was cancelled",
        false,
    );
}

fn data_path(root: &Path) -> PathBuf {
    root.join("memories.json")
}

fn lock_path(root: &Path) -> PathBuf {
    root.join("memories.lock")
}

fn temp_path(root: &Path) -> PathBuf {
    root.join("memories.tmp")
}

#[test]
fn public_contract_spec_and_exact_capability_are_frozen() {
    assert_eq!(MEMORY_SCHEMA_VERSION, 1);
    assert_eq!(MEMORY_TOOL_NAME, "memory");
    assert_eq!(MAX_MEMORY_FACT_BYTES, 4_096);
    assert_eq!(MAX_MEMORY_FACTS, 128);
    assert_eq!(MAX_MEMORY_TOTAL_FACT_BYTES, 32_768);
    assert_eq!(MAX_MEMORY_FILE_BYTES, 49_152);
    assert_eq!(MAX_MEMORY_SERIALIZED_ARGUMENT_BYTES, 32_768);
    assert_eq!(MAX_MEMORY_SERIALIZED_RESULT_BYTES, 65_536);
    assert_eq!(MAX_MEMORY_IO_ATTEMPTS, 65_536);

    let temporary = TemporaryDirectory::new("contract");
    let tool = tool(temporary.path());
    let spec = tool.spec();
    assert_eq!(spec.name.as_str(), MEMORY_TOOL_NAME);
    assert!(spec.description.contains("explicit user request"));
    assert!(spec.description.contains("Do not store secrets"));
    assert_eq!(spec.input_schema["oneOf"].as_array().unwrap().len(), 3);

    for arguments in [
        json!({"action": "save", "fact": " Preserve me exactly \n"}),
        json!({"action": "list"}),
        json!({"action": "clear"}),
    ] {
        let prepared = tool.prepare(call(arguments.clone())).unwrap();
        assert_eq!(prepared.arguments(), &arguments);
        assert_eq!(
            prepared.authorization(),
            &PreparedToolAuthorization::PermissionRequired(Capability::Custom {
                name: MEMORY_TOOL_NAME.to_owned(),
                details: arguments,
            })
        );
    }

    assert_invalid_arguments(
        tool.prepare(named_call("read_file", json!({"action": "list"})))
            .unwrap_err(),
    );
    assert!(fs::read_dir(temporary.path()).unwrap().next().is_none());
}

#[test]
fn construction_is_redacted_no_follow_and_effect_free() {
    let temporary = TemporaryDirectory::new("open");
    let state = temporary.path().join("state");
    fs::create_dir(&state).unwrap();
    let state_link = temporary.path().join("state-link");
    symlink(&state, &state_link).unwrap();
    let state_file = temporary.path().join("state-file");
    fs::write(&state_file, b"not a directory").unwrap();

    let opened = MemoryTool::open(&state).unwrap();
    assert_eq!(format!("{opened:?}"), "MemoryTool { .. }");
    assert!(fs::read_dir(&state).unwrap().next().is_none());

    let relative = MemoryTool::open(Path::new("relative-state")).unwrap_err();
    assert_eq!(relative.kind(), MemoryToolOpenErrorKind::InvalidRoot);
    assert_eq!(relative.to_string(), "native memory state root is invalid");

    for path in [&state_link, &state_file] {
        let error = MemoryTool::open(path).unwrap_err();
        assert_eq!(error.kind(), MemoryToolOpenErrorKind::InvalidFileType);
        assert_eq!(
            error.to_string(),
            "native memory state root is not a directory"
        );
        assert!(!error.to_string().contains(path.to_str().unwrap()));
        assert!(!format!("{error:?}").contains(path.to_str().unwrap()));
        assert!(error.source().is_none());
    }

    let missing = temporary.path().join("PRIVATE_MISSING_MEMORY_ROOT");
    let error = MemoryTool::open(&missing).unwrap_err();
    assert_eq!(error.kind(), MemoryToolOpenErrorKind::Unavailable);
    assert_eq!(error.to_string(), "native memory state root is unavailable");
    assert!(!error.to_string().contains("PRIVATE"));
}

#[test]
fn strict_arguments_are_effect_free_and_pre_cancel_wins_before_decode() {
    let temporary = TemporaryDirectory::new("arguments");
    let tool = tool(temporary.path());
    let invalid = [
        Value::Null,
        json!([]),
        json!({}),
        json!({"action": null}),
        json!({"action": "unknown"}),
        json!({"action": "save"}),
        json!({"action": "save", "fact": null}),
        json!({"action": "save", "fact": "x", "extra": true}),
        json!({"action": "list", "fact": "x"}),
        json!({"action": "clear", "fact": "x"}),
        json!({"action": "list", "extra": true}),
    ];
    for arguments in invalid {
        assert_invalid_arguments(tool.prepare(call(arguments.clone())).unwrap_err());
        assert_invalid_arguments(execute(&tool, arguments, CancellationToken::new()).unwrap_err());
    }

    for fact in [String::new(), "x".repeat(MAX_MEMORY_FACT_BYTES + 1)] {
        let arguments = json!({"action": "save", "fact": fact});
        assert_invalid_fact(tool.prepare(call(arguments.clone())).unwrap_err());
        assert_invalid_fact(execute(&tool, arguments, CancellationToken::new()).unwrap_err());
    }

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_cancelled(execute(&tool, Value::Null, cancellation).unwrap_err());
    assert!(fs::read_dir(temporary.path()).unwrap().next().is_none());
}

#[test]
fn execute_is_inert_until_polled() {
    let temporary = TemporaryDirectory::new("inert");
    let tool = tool(temporary.path());
    let future = tool.execute(
        context(),
        json!({"action": "save", "fact": "future"}),
        CancellationToken::new(),
    );
    assert!(fs::read_dir(temporary.path()).unwrap().next().is_none());
    drop(future);
    assert!(fs::read_dir(temporary.path()).unwrap().next().is_none());
}

#[test]
fn save_list_duplicate_and_clear_are_exact_and_durable() {
    let temporary = TemporaryDirectory::new("lifecycle");
    let first = tool(temporary.path());
    let exact = "  Prefer concise commits\n";

    assert_eq!(
        save(&first, exact).content,
        json!({"action": "save", "stored": true, "count": 1})
    );
    assert_eq!(
        save(&first, exact).content,
        json!({"action": "save", "stored": false, "count": 1})
    );
    assert_eq!(
        save(&first, "Use British spelling").content,
        json!({"action": "save", "stored": true, "count": 2})
    );
    let second = tool(temporary.path());
    assert_eq!(
        list(&second).content,
        json!({
            "action": "list",
            "memories": [exact, "Use British spelling"],
            "count": 2
        })
    );
    assert_eq!(
        fs::read_to_string(data_path(temporary.path())).unwrap(),
        format!(
            "{{\"schema_version\":1,\"memories\":[{},{}]}}",
            serde_json::to_string(exact).unwrap(),
            serde_json::to_string("Use British spelling").unwrap()
        )
    );
    assert!(!temp_path(temporary.path()).exists());

    assert_eq!(
        clear(&second).content,
        json!({"action": "clear", "cleared": 2})
    );
    assert!(!data_path(temporary.path()).exists());
    assert_eq!(
        list(&first).content,
        json!({"action": "list", "memories": [], "count": 0})
    );
    assert_eq!(
        clear(&first).content,
        json!({"action": "clear", "cleared": 0})
    );
}

#[test]
fn created_persistent_children_are_private_regular_files() {
    let temporary = TemporaryDirectory::new("modes");
    let tool = tool(temporary.path());
    save(&tool, "private");

    for path in [lock_path(temporary.path()), data_path(temporary.path())] {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(metadata.file_type().is_file());
        assert_eq!(metadata.mode() & 0o777, 0o600);
    }
    assert!(!temp_path(temporary.path()).exists());
}

#[test]
fn descriptor_identity_survives_selected_path_replacement() {
    let temporary = TemporaryDirectory::new("identity");
    let selected = temporary.path().join("selected");
    let retained = temporary.path().join("retained");
    fs::create_dir(&selected).unwrap();
    let tool = tool(&selected);
    fs::rename(&selected, &retained).unwrap();
    fs::create_dir(&selected).unwrap();

    save(&tool, "retained identity");
    assert!(data_path(&retained).exists());
    assert!(!data_path(&selected).exists());
}

#[test]
fn incompatible_lock_contention_is_retryable_busy() {
    let temporary = TemporaryDirectory::new("busy");
    let tool = tool(temporary.path());
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path(temporary.path()))
        .unwrap();
    rustix::fs::flock(&held, FlockOperation::NonBlockingLockExclusive).unwrap();

    for arguments in [
        json!({"action": "list"}),
        json!({"action": "save", "fact": "blocked"}),
        json!({"action": "clear"}),
    ] {
        assert_tool_error(
            execute(&tool, arguments, CancellationToken::new()).unwrap_err(),
            ToolErrorKind::Unavailable,
            "memory_busy",
            "memory state is busy",
            true,
        );
    }
    drop(held);
    assert_eq!(list(&tool).content["count"], 0);
}

#[test]
fn corrupt_documents_fail_closed_without_replacement() {
    let temporary = TemporaryDirectory::new("corrupt");
    let tool = tool(temporary.path());
    let too_many = (0..=MAX_MEMORY_FACTS)
        .map(|index| format!("m{index}"))
        .collect::<Vec<_>>();
    let over_total = (0..9)
        .map(|index| {
            char::from(b'a' + index)
                .to_string()
                .repeat(MAX_MEMORY_FACT_BYTES)
        })
        .collect::<Vec<_>>();
    let oversized = "x".repeat(MAX_MEMORY_FACT_BYTES + 1);
    let cases = vec![
        b"{".to_vec(),
        vec![0xff],
        b"{\"schema_version\":1,\"memories\":[]} trailing".to_vec(),
        b"{\"schema_version\":1,\"memories\":[],\"extra\":true}".to_vec(),
        b"{\"schema_version\":1,\"schema_version\":1,\"memories\":[]}".to_vec(),
        b"{\"schema_version\":2,\"memories\":[]}".to_vec(),
        b"{\"schema_version\":1,\"memories\":[\"\"]}".to_vec(),
        b"{\"schema_version\":1,\"memories\":[\"same\",\"same\"]}".to_vec(),
        serde_json::to_vec(&json!({"schema_version": 1, "memories": too_many})).unwrap(),
        serde_json::to_vec(&json!({"schema_version": 1, "memories": over_total})).unwrap(),
        serde_json::to_vec(&json!({"schema_version": 1, "memories": [oversized]})).unwrap(),
        vec![b' '; MAX_MEMORY_FILE_BYTES + 1],
    ];

    for bytes in cases {
        fs::write(data_path(temporary.path()), &bytes).unwrap();
        assert_corrupt(
            execute(&tool, json!({"action": "list"}), CancellationToken::new()).unwrap_err(),
        );
        assert_eq!(fs::read(data_path(temporary.path())).unwrap(), bytes);
    }
}

#[test]
fn unexpected_fixed_child_types_fail_closed_and_remain_untouched() {
    let temporary = TemporaryDirectory::new("child-types");
    let tool = tool(temporary.path());
    let target = temporary.path().join("target");
    fs::write(&target, b"target").unwrap();

    symlink(&target, data_path(temporary.path())).unwrap();
    assert_corrupt(
        execute(&tool, json!({"action": "list"}), CancellationToken::new()).unwrap_err(),
    );
    assert!(
        fs::symlink_metadata(data_path(temporary.path()))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_file(data_path(temporary.path())).unwrap();

    save(&tool, "live");
    let before = fs::read(data_path(temporary.path())).unwrap();
    symlink(&target, temp_path(temporary.path())).unwrap();
    assert_corrupt(
        execute(
            &tool,
            json!({"action": "save", "fact": "live"}),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_corrupt(
        execute(
            &tool,
            json!({"action": "save", "fact": "new"}),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_eq!(fs::read(data_path(temporary.path())).unwrap(), before);
    assert!(
        fs::symlink_metadata(temp_path(temporary.path()))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn stale_regular_temp_is_cleaned_by_save_and_clear() {
    let temporary = TemporaryDirectory::new("stale");
    let tool = tool(temporary.path());
    fs::write(temp_path(temporary.path()), b"stale").unwrap();
    save(&tool, "first");
    assert!(!temp_path(temporary.path()).exists());

    fs::write(temp_path(temporary.path()), b"stale again").unwrap();
    assert_eq!(clear(&tool).content["cleared"], 1);
    assert!(!temp_path(temporary.path()).exists());
    assert!(!data_path(temporary.path()).exists());

    fs::write(temp_path(temporary.path()), b"orphan").unwrap();
    assert_eq!(clear(&tool).content["cleared"], 0);
    assert!(!temp_path(temporary.path()).exists());
}

#[test]
fn count_total_and_escaping_serialization_limits_preserve_live_state() {
    let temporary = TemporaryDirectory::new("limits");
    let tool = tool(temporary.path());

    let full_count = (0..MAX_MEMORY_FACTS)
        .map(|index| format!("m{index}"))
        .collect::<Vec<_>>();
    fs::write(
        data_path(temporary.path()),
        serde_json::to_vec(&json!({"schema_version": 1, "memories": full_count})).unwrap(),
    )
    .unwrap();
    let before = fs::read(data_path(temporary.path())).unwrap();
    assert_resource_limit(
        execute(
            &tool,
            json!({"action": "save", "fact": "overflow"}),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_eq!(fs::read(data_path(temporary.path())).unwrap(), before);

    let full_total = (0..8)
        .map(|index| {
            char::from(b'a' + index)
                .to_string()
                .repeat(MAX_MEMORY_FACT_BYTES)
        })
        .collect::<Vec<_>>();
    fs::write(
        data_path(temporary.path()),
        serde_json::to_vec(&json!({"schema_version": 1, "memories": full_total})).unwrap(),
    )
    .unwrap();
    let before = fs::read(data_path(temporary.path())).unwrap();
    assert_resource_limit(
        execute(
            &tool,
            json!({"action": "save", "fact": "x"}),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_eq!(fs::read(data_path(temporary.path())).unwrap(), before);

    fs::remove_file(data_path(temporary.path())).unwrap();
    save(&tool, &"\0".repeat(MAX_MEMORY_FACT_BYTES));
    let before = fs::read(data_path(temporary.path())).unwrap();
    assert_resource_limit(
        execute(
            &tool,
            json!({"action": "save", "fact": "\u{1}".repeat(MAX_MEMORY_FACT_BYTES)}),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_eq!(fs::read(data_path(temporary.path())).unwrap(), before);
}
