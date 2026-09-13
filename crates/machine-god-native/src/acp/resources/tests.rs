use super::*;
use crate::{NativeWorkspaceAuthority, acp::session::decode_prompt_input};
use futures_executor::block_on;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    base: PathBuf,
    root: PathBuf,
    reader: NativeAcpResourceContextReader,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "machine-god-acp-resource-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let root = base.join("workspace");
        let state = base.join("state");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&state).unwrap();
        let open = |path: &Path| {
            rustix::fs::open(
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap()
        };
        let authority = NativeWorkspaceAuthority::open_blocking(
            open(&root),
            root.clone(),
            Some(open(&state)),
            state,
            vec![],
            false,
        )
        .unwrap();
        let reader = NativeAcpResourceContextReader::new(
            authority.snapshot().unwrap(),
            NativeOwnedWorkerScope::new(),
        );
        Self { base, root, reader }
    }
    fn input(paths: &[PathBuf]) -> NativeAcpPrompt {
        let mut blocks = vec![json!({"type":"text","text":"unchanged user text"})];
        for path in paths {
            blocks.push(
                json!({"type":"resource","resource":{"uri":format!("file://{}", path.display())}}),
            );
        }
        decode_prompt_input(&json!({"prompt":blocks})).unwrap()
    }
    fn read(&self, paths: &[PathBuf]) -> NativeAcpMaterializedPrompt {
        block_on(
            self.reader
                .materialize(Self::input(paths), CancellationToken::new()),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}

#[test]
fn gathers_only_applicable_root_and_nested_instructions_without_target_content() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.root.join("src")).unwrap();
    std::fs::create_dir(fixture.root.join("other")).unwrap();
    std::fs::write(fixture.root.join("AGENTS.md"), "root instructions").unwrap();
    std::fs::write(fixture.root.join("src/AGENTS.md"), "nested instructions").unwrap();
    std::fs::write(
        fixture.root.join("other/AGENTS.md"),
        "unrelated instructions",
    )
    .unwrap();
    std::fs::write(fixture.root.join("src/file.rs"), "target content sentinel").unwrap();
    let output = fixture.read(&[fixture.root.join("src/file.rs")]);
    assert_eq!(output.prompt.text, "unchanged user text");
    let text = output.context.unwrap();
    assert!(text.text().contains("root instructions"));
    assert!(text.text().contains("nested instructions"));
    assert!(!text.text().contains("unrelated instructions"));
    assert!(!text.text().contains("target content sentinel"));
    assert!(text.text().find("root instructions") < text.text().find("nested instructions"));
    assert!(output.omissions.is_empty());
}

#[test]
fn shared_ancestors_are_not_repeated() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.root.join("src")).unwrap();
    std::fs::write(fixture.root.join("AGENTS.md"), "root sentinel").unwrap();
    std::fs::write(fixture.root.join("src/AGENTS.md"), "nested sentinel").unwrap();
    for name in ["a", "b"] {
        std::fs::write(fixture.root.join("src").join(name), "").unwrap();
    }
    let output = fixture.read(&[fixture.root.join("src/a"), fixture.root.join("src/b")]);
    let text = output.context.unwrap();
    assert_eq!(text.text().matches("root sentinel").count(), 1);
    assert_eq!(text.text().matches("nested sentinel").count(), 1);
}

#[test]
fn symlink_targets_ancestors_and_instruction_files_cannot_escape() {
    let fixture = Fixture::new();
    let outside = fixture.base.join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("AGENTS.md"), "outside secret").unwrap();
    std::fs::write(outside.join("file"), "outside target").unwrap();
    std::os::unix::fs::symlink(&outside, fixture.root.join("linked")).unwrap();
    std::os::unix::fs::symlink(outside.join("file"), fixture.root.join("file")).unwrap();
    std::os::unix::fs::symlink(outside.join("AGENTS.md"), fixture.root.join("AGENTS.md")).unwrap();
    let output = fixture.read(&[
        fixture.root.join("linked/file"),
        fixture.root.join("file"),
        outside.join("file"),
        fixture.base.join("state/record"),
    ]);
    assert_eq!(output.omissions.len(), 5);
    assert!(!output.context.unwrap().text().contains("outside secret"));
}

#[test]
fn missing_nonregular_and_oversized_instructions_have_omission_evidence() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.root.join("AGENTS.md"),
        vec![b'x'; MAX_ACP_CONTEXT_FILE_BYTES + 1],
    )
    .unwrap();
    std::fs::create_dir(fixture.root.join("directory")).unwrap();
    let output = fixture.read(&[fixture.root.join("missing"), fixture.root.join("directory")]);
    assert_eq!(output.omissions.len(), 3);
    assert!(output.context.unwrap().text().contains("context_limit=1"));
}

#[test]
fn aggregate_instruction_budget_omits_whole_files() {
    let fixture = Fixture::new();
    let mut path = fixture.root.clone();
    for index in 0..6 {
        std::fs::write(
            path.join("AGENTS.md"),
            format!("{index}:{}", "x".repeat(15 * 1024)),
        )
        .unwrap();
        path.push("nested");
        std::fs::create_dir(&path).unwrap();
    }
    let target = path.join("file");
    std::fs::write(&target, "").unwrap();
    let output = fixture.read(&[target]);
    assert!(output.context.unwrap().text().len() <= MAX_ACP_CONTEXT_BYTES + 256);
    assert!(
        output
            .omissions
            .iter()
            .any(|item| item.reason == NativeAcpResourceOmissionReason::ContextLimit)
    );
}

#[test]
fn depth_directory_count_and_binary_instruction_limits_are_explicit() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("AGENTS.md"), b"binary\0sentinel").unwrap();
    let deep = fixture
        .root
        .join(vec!["child"; MAX_ACP_RESOURCE_DEPTH + 1].join("/"));
    let output = fixture.read(&[deep]);
    assert_eq!(output.omissions.len(), 2);
    assert!(!output.context.unwrap().text().contains("binary"));
    std::fs::remove_file(fixture.root.join("AGENTS.md")).unwrap();
    let mut targets = Vec::new();
    for index in 0..64 {
        let parent = fixture.root.join(format!("dir{index}"));
        std::fs::create_dir(&parent).unwrap();
        std::fs::create_dir(parent.join("child")).unwrap();
        let target = parent.join("child/file");
        std::fs::write(&target, "").unwrap();
        targets.push(target);
    }
    let output = fixture.read(&targets);
    assert_eq!(output.omissions.len(), 1);
    assert_eq!(
        output.omissions[0].reason,
        NativeAcpResourceOmissionReason::ContextLimit
    );
}

#[test]
fn construction_is_inert_and_cancelled_materialization_does_not_read() {
    let fixture = Fixture::new();
    let cancellation = CancellationToken::new();
    let future = fixture
        .reader
        .materialize(Fixture::input(&[]), cancellation.clone());
    // Creating the future must not read the old bytes.
    std::fs::write(fixture.root.join("AGENTS.md"), "written after construction").unwrap();
    let output = block_on(future).unwrap();
    assert!(
        output
            .context
            .unwrap()
            .text()
            .contains("written after construction")
    );
    cancellation.cancel();
    assert!(matches!(
        block_on(
            fixture
                .reader
                .materialize(Fixture::input(&[]), cancellation)
        ),
        Err(NativeAcpResourceContextError::Cancelled)
    ));
    let cancellation = CancellationToken::new();
    drop(
        fixture
            .reader
            .materialize(Fixture::input(&[]), cancellation.clone()),
    );
    assert!(cancellation.is_cancelled());
}

#[test]
fn retained_workspace_descriptor_survives_path_replacement() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("AGENTS.md"), "original rules").unwrap();
    let old = fixture.base.join("old");
    std::fs::rename(&fixture.root, &old).unwrap();
    std::fs::create_dir(&fixture.root).unwrap();
    std::fs::write(fixture.root.join("AGENTS.md"), "replacement rules").unwrap();
    let output = fixture.read(&[]);
    let text = output.context.unwrap();
    assert!(text.text().contains("original rules"));
    assert!(!text.text().contains("replacement rules"));
}

#[test]
fn dropped_materialization_keeps_fifo_lease_until_cancelled_worker_settles() {
    use std::sync::{Arc, Barrier, atomic::AtomicBool};
    struct Lease(Arc<AtomicBool>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let mut fixture = Fixture::new();
    let entered = Arc::new(Barrier::new(2));
    let released = Arc::new(Barrier::new(2));
    let enter_worker = entered.clone();
    let release_worker = released.clone();
    fixture.reader.before_read = Some(Arc::new(move || {
        enter_worker.wait();
        release_worker.wait();
    }));
    let dropped = Arc::new(AtomicBool::new(false));
    let cancellation = CancellationToken::new();
    let mut future = fixture.reader.materialize_with_lease(
        Fixture::input(&[]),
        Lease(dropped.clone()),
        cancellation.clone(),
    );
    let waker = futures_util::task::noop_waker();
    assert!(
        future
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&waker))
            .is_pending()
    );
    entered.wait();
    drop(future);
    assert!(cancellation.is_cancelled());
    assert!(!dropped.load(Ordering::SeqCst));
    released.wait();
    fixture.reader.workers.close();
    fixture
        .reader
        .workers
        .completion()
        .wait_on_worker()
        .unwrap();
    assert!(dropped.load(Ordering::SeqCst));
}
