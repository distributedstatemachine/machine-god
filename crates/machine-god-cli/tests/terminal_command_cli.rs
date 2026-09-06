//! A reentrant native host fixture: the production supervisor re-execs its host
//! with the private helper argument, which the ordinary Rust test harness rejects.

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments == [machine_god_native::BACKGROUND_PROCESS_HELPER_ARGUMENT] {
        let _ = machine_god_native::run_background_process_helper();
        return;
    }
    full_command_roundtrip();
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn main() {}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn full_command_roundtrip() {
    use std::fs;
    use std::process::Command;
    use std::time::{Duration, Instant};

    let fixture = Fixture::new();
    let workspace = fixture.0.join("workspace");
    let state = fixture.0.join("state");
    let store = state.join("machine-god");
    private_directory(&workspace);
    private_directory(&store);
    let workspace = fs::canonicalize(workspace).unwrap();
    let supervisor =
        machine_god_native::NativeBackgroundSupervisor::open(&workspace, &store).unwrap();
    let command = format!(
        "#{}",
        "\u{1b}".repeat(machine_god_native::MAX_TERMINAL_COMMAND_BYTES - 1)
    );
    let request =
        machine_god_core::BackgroundStartRequest::new(command, workspace.to_str().unwrap())
            .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let handle = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            supervisor.start(request, machine_god_core::CancellationToken::new()),
        )
        .await
        .unwrap()
        .unwrap()
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let expected = format!(
        "\"command\":\"#{}\"",
        "\\u001b".repeat(machine_god_native::MAX_TERMINAL_COMMAND_BYTES - 1)
    );
    loop {
        let output = Command::new(env!("CARGO_BIN_EXE_machine-god"))
            .current_dir(&workspace)
            .env_remove("HOME")
            .env("XDG_STATE_HOME", fs::canonicalize(&state).unwrap())
            .args(["background", &handle.id().to_string(), "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty());
        let json = String::from_utf8(output.stdout).unwrap();
        assert!(json.contains(&expected));
        if json.contains("\"state\":\"exited\"") {
            assert!(json.contains("\"exit_code\":0"));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "background completion was not published"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(supervisor);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn private_directory(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct Fixture(std::path::PathBuf);

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Fixture {
    fn new() -> Self {
        for attempt in 0..100 {
            let path = std::env::temp_dir()
                .join(format!("mg-command-cli-{}-{attempt}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("temporary fixture failed: {error}"),
            }
        }
        panic!("temporary fixture attempts exhausted");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
