#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::{FileTypeExt, symlink};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    CancellationToken, Capability, SessionId, SessionIncarnationId, Tool, ToolCall, ToolCallId,
    ToolContext, ToolError, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    INSTALL_SKILL_TOOL_NAME, InstallSkillTool, InstallSkillToolOpenErrorKind,
    MAX_INSTALL_SKILL_CHUNK_BYTES, MAX_INSTALL_SKILL_COMPONENT_BYTES, MAX_INSTALL_SKILL_ENTRIES,
    MAX_INSTALL_SKILL_ENTRY_NAME_BYTES, MAX_INSTALL_SKILL_FILE_BYTES,
    MAX_INSTALL_SKILL_IO_ATTEMPTS, MAX_INSTALL_SKILL_NAME_BYTES, MAX_INSTALL_SKILL_PATH_BYTES,
    MAX_INSTALL_SKILL_PATH_COMPONENTS, MAX_INSTALL_SKILL_SERIALIZED_ARGUMENT_BYTES,
    MAX_INSTALL_SKILL_SERIALIZED_RESULT_BYTES, MAX_INSTALL_SKILL_SOURCE_BYTES,
    MAX_INSTALL_SKILL_STAGE_ATTEMPTS, MAX_INSTALL_SKILL_TOTAL_BYTES, SkillTool,
};
use serde_json::{Value, json};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mg-install-skill-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create temporary directory: {error}"),
            }
        }
        panic!("allocate temporary directory")
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(Pin::as_mut(&mut future), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("install_skill unexpectedly returned pending"),
    }
}

fn tool(root: &Path) -> InstallSkillTool {
    InstallSkillTool::open(root).expect("valid workspace")
}

fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("install-skill-call").unwrap(),
        name: ToolName::new(INSTALL_SKILL_TOOL_NAME).unwrap(),
        arguments,
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("install-skill-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("install-skill-incarnation").unwrap(),
        turn_id: TurnId::new("install-skill-turn").unwrap(),
        call_id: ToolCallId::new("install-skill-call").unwrap(),
    }
}

fn execute(
    tool: &InstallSkillTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    ready(tool.execute(context(), arguments, cancellation))
}

fn code(error: ToolError) -> String {
    error.code
}

fn source(root: &Path, name: &str, manifest: &[u8]) -> PathBuf {
    let source = root.join("incoming").join(name);
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("SKILL.md"), manifest).unwrap();
    source
}

#[test]
fn exported_contract_and_spec_are_bounded() {
    assert_eq!(INSTALL_SKILL_TOOL_NAME, "install_skill");
    assert_eq!(MAX_INSTALL_SKILL_SOURCE_BYTES, 4_096);
    assert_eq!(MAX_INSTALL_SKILL_NAME_BYTES, 128);
    assert_eq!(MAX_INSTALL_SKILL_PATH_BYTES, 4_096);
    assert_eq!(MAX_INSTALL_SKILL_COMPONENT_BYTES, 255);
    assert_eq!(MAX_INSTALL_SKILL_PATH_COMPONENTS, 32);
    assert_eq!(MAX_INSTALL_SKILL_ENTRIES, 256);
    assert_eq!(MAX_INSTALL_SKILL_FILE_BYTES, 1_048_576);
    assert_eq!(MAX_INSTALL_SKILL_TOTAL_BYTES, 8_388_608);
    assert_eq!(MAX_INSTALL_SKILL_ENTRY_NAME_BYTES, 1_048_576);
    assert_eq!(MAX_INSTALL_SKILL_CHUNK_BYTES, 65_536);
    assert_eq!(MAX_INSTALL_SKILL_IO_ATTEMPTS, 8_192);
    assert_eq!(MAX_INSTALL_SKILL_STAGE_ATTEMPTS, 8);
    assert_eq!(MAX_INSTALL_SKILL_SERIALIZED_ARGUMENT_BYTES, 32_768);
    assert_eq!(MAX_INSTALL_SKILL_SERIALIZED_RESULT_BYTES, 16_384);

    let workspace = TemporaryDirectory::new();
    let spec = tool(workspace.path()).spec();
    assert_eq!(spec.name.as_str(), INSTALL_SKILL_TOOL_NAME);
    assert_eq!(spec.input_schema["required"], json!(["source"]));
    assert_eq!(spec.input_schema["additionalProperties"], false);
}

#[test]
fn prepare_is_effect_free_and_requests_exact_custom_authority() {
    let workspace = TemporaryDirectory::new();
    let prepared = tool(workspace.path())
        .prepare(call(json!({"source":"./incoming//./rust"})))
        .unwrap();
    assert_eq!(
        prepared.arguments(),
        &json!({"source":"incoming/rust","skill":"rust"})
    );
    assert_eq!(
        prepared.capability(),
        Some(&Capability::Custom {
            name: "install_skill".to_owned(),
            details: json!({"source":"incoming/rust","destination":"skills/rust"}),
        })
    );
    assert!(!workspace.path().join("incoming").exists());
    assert!(!workspace.path().join("skills").exists());
}

#[test]
fn strict_input_and_canonical_direct_execution_are_enforced() {
    let workspace = TemporaryDirectory::new();
    let tool = tool(workspace.path());
    for invalid in [
        json!({}),
        json!({"source":"incoming/rust","extra":true}),
        json!({"source":"incoming/rust","skill":null}),
        json!({"source":"../rust"}),
        json!({"source":"/rust"}),
        json!({"source":"skills/rust"}),
        json!({"source":"Skills/rust"}),
        json!({"source":"incoming/rust","skill":"other"}),
    ] {
        assert!(tool.prepare(call(invalid)).is_err());
    }
    let error = execute(
        &tool,
        json!({"source":"./incoming/rust","skill":"rust"}),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(code(error), "install_skill_invalid_arguments");
}

#[test]
fn unicode_case_fold_aliases_of_the_managed_root_are_rejected_as_overlap() {
    let workspace = TemporaryDirectory::new();
    let tool = tool(workspace.path());

    for source in ["ſkills/rust", "s\u{212a}ills/rust"] {
        let error = tool.prepare(call(json!({"source": source}))).unwrap_err();
        assert_eq!(code(error), "install_skill_overlap", "source: {source}");
    }

    assert!(!workspace.path().join("skills").exists());
}

#[test]
fn installs_one_tree_and_the_skill_reader_can_read_it() {
    let workspace = TemporaryDirectory::new();
    let source = source(workspace.path(), "release-checks", b"# Release checks\n");
    fs::create_dir(source.join("references")).unwrap();
    fs::write(source.join("references/linux.md"), b"Run pinned CI.\n").unwrap();

    let output = execute(
        &tool(workspace.path()),
        json!({"source":"incoming/release-checks","skill":"release-checks"}),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        output,
        ToolOutput::success(json!({
            "source":"incoming/release-checks",
            "skill":"release-checks",
            "destination":"skills/release-checks",
            "entries":3,
            "total_bytes":32,
        }))
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("skills/release-checks/SKILL.md")).unwrap(),
        "# Release checks\n"
    );
    assert_eq!(
        fs::read_to_string(
            workspace
                .path()
                .join("skills/release-checks/references/linux.md")
        )
        .unwrap(),
        "Run pinned CI.\n"
    );
    let reader = SkillTool::open(workspace.path()).unwrap();
    let prepared = reader
        .prepare(ToolCall {
            id: ToolCallId::new("skill-call").unwrap(),
            name: ToolName::new("skill").unwrap(),
            arguments: json!({"name":"release-checks"}),
        })
        .unwrap();
    let read = ready(reader.execute(
        context(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(read.content["content"], "# Release checks\n");
}

#[test]
fn existing_skills_directory_and_destination_collision_are_safe() {
    let workspace = TemporaryDirectory::new();
    fs::create_dir(workspace.path().join("skills")).unwrap();
    source(workspace.path(), "rust", b"new\n");
    let tool = tool(workspace.path());
    execute(
        &tool,
        json!({"source":"incoming/rust","skill":"rust"}),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        fs::read(workspace.path().join("skills/rust/SKILL.md")).unwrap(),
        b"new\n"
    );

    fs::remove_dir_all(workspace.path().join("incoming/rust")).unwrap();
    source(workspace.path(), "rust", b"replacement\n");
    let error = execute(
        &tool,
        json!({"source":"incoming/rust","skill":"rust"}),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(code(error), "install_skill_destination_exists");
    assert_eq!(
        fs::read(workspace.path().join("skills/rust/SKILL.md")).unwrap(),
        b"new\n"
    );
}

#[test]
fn missing_or_non_utf8_manifest_is_rejected_without_destination() {
    let workspace = TemporaryDirectory::new();
    let absent = workspace.path().join("incoming/absent");
    fs::create_dir_all(&absent).unwrap();
    let tool = tool(workspace.path());
    assert_eq!(
        code(
            execute(
                &tool,
                json!({"source":"incoming/absent","skill":"absent"}),
                CancellationToken::new()
            )
            .unwrap_err()
        ),
        "install_skill_invalid_manifest"
    );
    source(workspace.path(), "binary", &[0xff]);
    assert_eq!(
        code(
            execute(
                &tool,
                json!({"source":"incoming/binary","skill":"binary"}),
                CancellationToken::new()
            )
            .unwrap_err()
        ),
        "install_skill_invalid_manifest"
    );
    assert!(!workspace.path().join("skills").exists());
}

#[test]
fn links_and_special_entries_are_rejected_without_publication() {
    let workspace = TemporaryDirectory::new();
    let linked = source(workspace.path(), "linked", b"ok\n");
    symlink(workspace.path(), linked.join("escape")).unwrap();
    let tool = tool(workspace.path());
    assert_eq!(
        code(
            execute(
                &tool,
                json!({"source":"incoming/linked","skill":"linked"}),
                CancellationToken::new()
            )
            .unwrap_err()
        ),
        "install_skill_path_rejected"
    );

    let fifo = source(workspace.path(), "fifo", b"ok\n");
    let fifo_path = fifo.join("pipe");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo_path)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(fs::metadata(&fifo_path).unwrap().file_type().is_fifo());
    assert_eq!(
        code(
            execute(
                &tool,
                json!({"source":"incoming/fifo","skill":"fifo"}),
                CancellationToken::new()
            )
            .unwrap_err()
        ),
        "install_skill_path_rejected"
    );
    assert!(!workspace.path().join("skills").exists());
}

#[test]
fn entry_and_file_limits_fail_before_publication() {
    let workspace = TemporaryDirectory::new();
    let many = source(workspace.path(), "many", b"ok\n");
    for index in 0..MAX_INSTALL_SKILL_ENTRIES {
        fs::write(many.join(format!("entry-{index:03}")), b"").unwrap();
    }
    let tool = tool(workspace.path());
    assert_eq!(
        code(
            execute(
                &tool,
                json!({"source":"incoming/many","skill":"many"}),
                CancellationToken::new()
            )
            .unwrap_err()
        ),
        "install_skill_resource_limit"
    );

    let large = source(workspace.path(), "large", b"ok\n");
    let file = fs::File::create(large.join("large.bin")).unwrap();
    file.set_len((MAX_INSTALL_SKILL_FILE_BYTES + 1) as u64)
        .unwrap();
    assert_eq!(
        code(
            execute(
                &tool,
                json!({"source":"incoming/large","skill":"large"}),
                CancellationToken::new()
            )
            .unwrap_err()
        ),
        "install_skill_resource_limit"
    );
    assert!(!workspace.path().join("skills").exists());
}

#[test]
fn exact_entry_limit_publishes_the_complete_inventory_without_stage_residue() {
    let workspace = TemporaryDirectory::new();
    let manifest = b"manifest\n";
    let inventory = source(workspace.path(), "inventory", manifest);
    fs::create_dir(inventory.join("empty-directory")).unwrap();

    let file_count = MAX_INSTALL_SKILL_ENTRIES - 2;
    let mut total_bytes = manifest.len();
    for index in 0..file_count {
        let contents = format!("{index:03}");
        total_bytes += contents.len();
        fs::write(inventory.join(format!("entry-{index:03}")), contents).unwrap();
    }

    let output = execute(
        &tool(workspace.path()),
        json!({"source":"incoming/inventory","skill":"inventory"}),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        output,
        ToolOutput::success(json!({
            "source":"incoming/inventory",
            "skill":"inventory",
            "destination":"skills/inventory",
            "entries":MAX_INSTALL_SKILL_ENTRIES,
            "total_bytes":total_bytes,
        }))
    );

    let installed = workspace.path().join("skills/inventory");
    assert_eq!(fs::read(installed.join("SKILL.md")).unwrap(), manifest);
    assert!(installed.join("empty-directory").is_dir());
    for index in 0..file_count {
        assert_eq!(
            fs::read_to_string(installed.join(format!("entry-{index:03}"))).unwrap(),
            format!("{index:03}")
        );
    }
    assert_eq!(
        fs::read_dir(&installed).unwrap().count(),
        MAX_INSTALL_SKILL_ENTRIES
    );
    assert_eq!(
        fs::read_dir(workspace.path().join("skills"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        [std::ffi::OsString::from("inventory")]
    );
}

#[test]
fn exact_descendant_depth_is_inclusive_and_the_next_child_is_rejected() {
    let accepted_workspace = TemporaryDirectory::new();
    let accepted = source(accepted_workspace.path(), "depth", b"ok\n");
    let mut deepest = accepted;
    for index in 0..MAX_INSTALL_SKILL_PATH_COMPONENTS {
        deepest = deepest.join(format!("d{index:02}"));
        fs::create_dir(&deepest).unwrap();
    }
    execute(
        &tool(accepted_workspace.path()),
        json!({"source":"incoming/depth","skill":"depth"}),
        CancellationToken::new(),
    )
    .unwrap();
    assert!(accepted_workspace.path().join("skills/depth").exists());

    let rejected_workspace = TemporaryDirectory::new();
    let rejected = source(rejected_workspace.path(), "depth", b"ok\n");
    let mut deepest = rejected;
    for index in 0..=MAX_INSTALL_SKILL_PATH_COMPONENTS {
        deepest = deepest.join(format!("d{index:02}"));
        fs::create_dir(&deepest).unwrap();
    }
    assert_eq!(
        code(
            execute(
                &tool(rejected_workspace.path()),
                json!({"source":"incoming/depth","skill":"depth"}),
                CancellationToken::new(),
            )
            .unwrap_err()
        ),
        "install_skill_resource_limit"
    );
    assert!(!rejected_workspace.path().join("skills").exists());
}

#[test]
fn pre_cancelled_execution_is_inert() {
    let workspace = TemporaryDirectory::new();
    source(workspace.path(), "rust", b"ok\n");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = execute(
        &tool(workspace.path()),
        json!({"source":"incoming/rust","skill":"rust"}),
        cancellation,
    )
    .unwrap_err();
    assert_eq!(code(error), "install_skill_cancelled");
    assert!(!workspace.path().join("skills").exists());
}

#[test]
fn constructor_retains_workspace_identity_and_is_redacted() {
    let parent = TemporaryDirectory::new();
    let selected = parent.path().join("selected");
    fs::create_dir(&selected).unwrap();
    source(&selected, "rust", b"retained\n");
    let tool = tool(&selected);
    let retained = parent.path().join("retained");
    fs::rename(&selected, &retained).unwrap();
    fs::create_dir(&selected).unwrap();
    source(&selected, "rust", b"replacement\n");
    execute(
        &tool,
        json!({"source":"incoming/rust","skill":"rust"}),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        fs::read(retained.join("skills/rust/SKILL.md")).unwrap(),
        b"retained\n"
    );
    assert!(!selected.join("skills").exists());

    let error = InstallSkillTool::open(Path::new("relative")).unwrap_err();
    assert_eq!(error.kind(), InstallSkillToolOpenErrorKind::InvalidRoot);
    assert_eq!(
        error.to_string(),
        "native install_skill workspace root is invalid"
    );
}
