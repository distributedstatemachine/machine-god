#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_executor::block_on;
use machine_god_core::{
    CancellationToken, SessionId, SessionIncarnationId, Tool, ToolCallId, ToolContext, TurnId,
};
use machine_god_native::MemoryTool;
use serde_json::json;

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-memory-hostile-umask-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create temporary directory: {error}"),
            }
        }
        panic!("failed to allocate a unique temporary directory");
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

#[test]
fn newly_created_lock_is_exact_0600_under_an_owner_masking_umask() {
    const CHILD_MARKER: &str = "MACHINE_GOD_MEMORY_UMASK_CHILD";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let status = Command::new("sh")
            .arg("-c")
            .arg(
                "umask 0777; exec \"$1\" --exact \
                 newly_created_lock_is_exact_0600_under_an_owner_masking_umask --nocapture",
            )
            .arg("machine-god-memory-umask")
            .arg(std::env::current_exe().unwrap())
            .env(CHILD_MARKER, "1")
            .status()
            .expect("failed to execute isolated hostile-umask test process");
        assert!(status.success(), "hostile-umask child failed: {status}");
        return;
    }

    let temporary = TemporaryDirectory::new();
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let tool = MemoryTool::open(temporary.path()).unwrap();
    let output = block_on(tool.execute(
        ToolContext {
            session_id: SessionId::new("memory-session").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("memory-incarnation").unwrap(),
            turn_id: TurnId::new("memory-turn").unwrap(),
            call_id: ToolCallId::new("memory-call").unwrap(),
        },
        json!({"action": "list"}),
        CancellationToken::new(),
    ))
    .unwrap();

    assert_eq!(output.content["count"], 0);
    assert_eq!(
        fs::symlink_metadata(temporary.path().join("memories.lock"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
}
