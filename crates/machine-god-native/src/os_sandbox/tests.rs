use super::*;
use crate::{NativeSandboxMode, PermissionMode};
use machine_god_core::CancellationToken;
use std::fs::File;
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

#[test]
fn none_and_yolo_are_immutable_and_need_no_os_authority() {
    let cancellation = CancellationToken::new();
    for (configured, permission) in [
        (NativeSandboxMode::None, PermissionMode::Ask),
        (NativeSandboxMode::Os, PermissionMode::Yolo),
    ] {
        let snapshot = NativeSandboxLaunch::capture(
            configured,
            permission,
            vec![],
            None,
            false,
            Instant::now() + Duration::from_secs(1),
            &cancellation,
        )
        .unwrap();
        assert_eq!(snapshot.configured(), configured);
        assert_eq!(snapshot.effective(), NativeSandboxMode::None);
        let clone = snapshot.clone();
        assert_eq!(
            clone
                .wrap("/bin/sh".into(), vec!["-c".into(), "printf exact".into()])
                .unwrap(),
            ("/bin/sh".into(), vec!["-c".into(), "printf exact".into()])
        );
    }
    cancellation.cancel();
    assert_eq!(
        NativeSandboxLaunch::capture(
            NativeSandboxMode::None,
            PermissionMode::Ask,
            vec![],
            None,
            false,
            Instant::now() + Duration::from_secs(1),
            &cancellation
        )
        .unwrap_err(),
        NativeSandboxError::Cancelled
    );
    assert_eq!(
        NativeSandboxLaunch::capture(
            NativeSandboxMode::None,
            PermissionMode::Ask,
            vec![],
            None,
            false,
            Instant::now(),
            &CancellationToken::new()
        )
        .unwrap_err(),
        NativeSandboxError::Timeout
    );
}

#[test]
fn explicit_os_never_falls_back_without_required_authority() {
    assert_eq!(
        NativeSandboxLaunch::capture(
            NativeSandboxMode::Os,
            PermissionMode::Ask,
            vec![],
            None,
            false,
            Instant::now() + Duration::from_secs(1),
            &CancellationToken::new()
        )
        .unwrap_err(),
        if cfg!(target_os = "macos") {
            NativeSandboxError::Unavailable
        } else {
            NativeSandboxError::Unsupported
        }
    );
}

#[test]
fn root_syntax_and_scheme_quoting_are_bounded_not_interpolated() {
    let directory = File::open(std::env::temp_dir()).unwrap();
    for path in ["", "relative", "/a/../b", "/a/./b", "/a//b", "/a/", "/a\0b"] {
        assert_eq!(
            NativeSandboxRoot::new(directory.try_clone().unwrap(), path.into()).unwrap_err(),
            NativeSandboxError::Invalid
        );
    }
    assert!(
        NativeSandboxRoot::new(
            directory.try_clone().unwrap(),
            format!("/{}", "x".repeat(4096)).into()
        )
        .is_err()
    );
    let roots: Vec<_> = (0..MAX_NATIVE_SANDBOX_ROOTS)
        .map(|_| {
            NativeSandboxRoot::new(
                directory.try_clone().unwrap(),
                format!("/{}", "\"".repeat(4095)).into(),
            )
            .unwrap()
        })
        .collect();
    let profile = build_profile(&roots, true).unwrap();
    assert!(profile.len() <= MAX_NATIVE_SANDBOX_PROFILE_BYTES);
    assert_eq!(profile.matches("(allow file-write* (subpath ").count(), 20);
    let mut quoted = String::new();
    quote(&mut quoted, "\"\\\n\r\t雪");
    assert_eq!(quoted, "\"\\\"\\\\\\n\\r\\t雪\"");
    let mut too_many = roots;
    too_many.push(NativeSandboxRoot::new(directory, "/".into()).unwrap());
    assert_eq!(
        NativeSandboxLaunch::capture(
            NativeSandboxMode::None,
            PermissionMode::Ask,
            too_many,
            None,
            false,
            Instant::now() + Duration::from_secs(1),
            &CancellationToken::new()
        )
        .unwrap_err(),
        NativeSandboxError::Invalid
    );
}

#[test]
fn helper_frame_preserves_original_command_limits_and_full_profile_budget() {
    use crate::terminal_helper::{
        LaunchFrame, TerminalPtyDimensions, read_frame, validate_program_arguments,
    };
    let profile = "p".repeat(MAX_NATIVE_SANDBOX_PROFILE_BYTES);
    let command = "x".repeat(crate::terminal_helper::MAX_ARGUMENT_BYTES);
    let args = vec![
        "-p".into(),
        profile.clone(),
        "/bin/sh".into(),
        "-c".into(),
        command.clone(),
    ];
    let environment =
        crate::background_process::ValidatedBackgroundEnvironment::new(vec![]).unwrap();
    let encoded = LaunchFrame::encode(
        NATIVE_SANDBOX_EXECUTABLE,
        &args,
        &environment,
        TerminalPtyDimensions {
            rows: 1,
            columns: 1,
        },
    )
    .unwrap();
    let frame = read_frame(
        &mut encoded.as_slice(),
        Instant::now() + Duration::from_secs(1),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(frame.arguments, args);
    let mut bad = args.clone();
    bad[1].push('x');
    assert!(validate_program_arguments(NATIVE_SANDBOX_EXECUTABLE, &bad).is_err());
    let mut bad = args;
    bad[4].push('x');
    assert!(validate_program_arguments(NATIVE_SANDBOX_EXECUTABLE, &bad).is_err());
    assert!(validate_program_arguments("/bin/sh", &[profile]).is_err());
    assert!(
        validate_program_arguments(
            NATIVE_SANDBOX_EXECUTABLE,
            &[
                "-p".into(),
                "profile".into(),
                NATIVE_SANDBOX_EXECUTABLE.into()
            ]
        )
        .is_err()
    );
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::background_process::{
        BackgroundProcessHelper, BackgroundProcessRequest, SystemBackgroundProcessAdapter,
    };
    use crate::terminal_pty::{
        PreparedTerminalPty, TerminalPtyDimensions, TerminalPtyHelper, TerminalPtyRequest,
    };
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    use super::super::NATIVE_TESTS as NATIVE;

    struct Fixture {
        base: PathBuf,
        allowed: PathBuf,
        extra: PathBuf,
        outside: PathBuf,
    }
    impl Fixture {
        fn new(leaf: &str) -> Self {
            let parent = std::env::temp_dir().canonicalize().unwrap();
            assert!(
                !parent.starts_with("/private/tmp") && !parent.starts_with("/tmp"),
                "macOS sandbox denial fixtures require the normal Darwin per-user temporary directory"
            );
            let base = parent.join(format!(
                "mg-os-sandbox-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&base).unwrap();
            let allowed = base.join(leaf);
            let extra = base.join("extra");
            let outside = base.join("outside");
            for path in [&allowed, &extra, &outside] {
                std::fs::create_dir(path).unwrap();
            }
            Self {
                base,
                allowed,
                extra,
                outside,
            }
        }
        fn snapshot(&self, permission: PermissionMode) -> Arc<NativeSandboxLaunch> {
            Arc::new(
                NativeSandboxLaunch::capture(
                    NativeSandboxMode::Os,
                    permission,
                    [&self.allowed, &self.extra]
                        .into_iter()
                        .map(|path| {
                            NativeSandboxRoot::new(File::open(path).unwrap(), path.clone()).unwrap()
                        })
                        .collect(),
                    Some(File::open(NATIVE_SANDBOX_EXECUTABLE).unwrap()),
                    false,
                    Instant::now() + Duration::from_secs(2),
                    &CancellationToken::new(),
                )
                .unwrap(),
            )
        }
        fn environment(&self) -> Vec<(OsString, OsString)> {
            vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("ALLOWED".into(), self.allowed.as_os_str().to_owned()),
                ("EXTRA".into(), self.extra.as_os_str().to_owned()),
                ("OUTSIDE".into(), self.outside.as_os_str().to_owned()),
            ]
        }
        fn cwd(&self) -> rustix::fd::OwnedFd {
            File::open(&self.allowed).unwrap().into()
        }
        fn assert_effects(&self) {
            assert_eq!(std::fs::read(self.allowed.join("ok")).unwrap(), b"allowed");
            assert_eq!(std::fs::read(self.extra.join("ok")).unwrap(), b"extra");
            assert!(!self.outside.join("denied").exists());
            assert!(!self.outside.join("descendant").exists());
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }
    const COMMAND: &str = "printf allowed > \"$ALLOWED/ok\"; printf extra > \"$EXTRA/ok\"; printf forbidden > \"$OUTSIDE/denied\"; /bin/sh -c 'printf forbidden > \"$OUTSIDE/descendant\"'; printf SANDBOX_DONE";

    fn helper(flag: &str, test: &str) -> TerminalPtyHelper {
        let (program, arguments) = match std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
            Some(binary) => (PathBuf::from(binary), vec![OsString::from(flag)]),
            None => (
                std::env::current_exe().unwrap(),
                vec![
                    "--exact".into(),
                    test.into(),
                    "--ignored".into(),
                    "--nocapture".into(),
                    "--quiet".into(),
                ],
            ),
        };
        TerminalPtyHelper::new(program, arguments)
            .unwrap()
            .with_test_inventory_helper()
    }

    #[test]
    fn sandbox_real_foreground_allows_exact_roots_and_denies_outside_and_descendants() {
        let _serial = NATIVE.lock().unwrap();
        let fixture = Fixture::new("root\" ) (allow default) ; \\ 雪\n");
        let helper = helper(
            crate::TERMINAL_CAPTURED_HELPER_ARGUMENT,
            "terminal_captured_exec::tests::captured_helper_child",
        );
        let executor = crate::TerminalCapturedExec::new(
            helper.program().to_owned(),
            helper.arguments().to_vec(),
            Duration::from_secs(10),
            1,
        )
        .unwrap()
        .with_test_inventory_helper();
        let shell = crate::TerminalShell::from_executable(Path::new("/bin/bash"), true)
            .unwrap()
            .with_sandbox(fixture.snapshot(PermissionMode::Ask));
        let result = futures_executor::block_on(executor.execute(
            machine_god_core::TerminalExecRequest {
                command: COMMAND.into(),
                cwd: fixture.allowed.to_str().unwrap().into(),
                profile: Some(machine_god_core::TerminalProfile::Clean),
            },
            shell,
            fixture.environment(),
            fixture.cwd(),
            CancellationToken::new(),
        ))
        .unwrap();
        assert!(
            result
                .stdout
                .bytes
                .windows(12)
                .any(|bytes| bytes == b"SANDBOX_DONE")
        );
        fixture.assert_effects();
    }

    #[test]
    fn sandbox_real_direct_argv_keeps_symlinks_outside_scope_and_yolo_preserves_configured() {
        let _serial = NATIVE.lock().unwrap();
        let fixture = Fixture::new("allowed");
        std::os::unix::fs::symlink(&fixture.outside, fixture.allowed.join("link")).unwrap();
        let sandbox = fixture.snapshot(PermissionMode::Ask);
        let mut command = sandbox
            .command(
                std::ffi::OsStr::new("/bin/sh"),
                &[
                    "-c".into(),
                    "printf forbidden > \"$ALLOWED/link/denied\"".into(),
                ],
            )
            .unwrap();
        let output = command
            .env_clear()
            .envs(fixture.environment())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!fixture.outside.join("denied").exists());
        let yolo = fixture.snapshot(PermissionMode::Yolo);
        assert_eq!(yolo.configured(), NativeSandboxMode::Os);
        assert_eq!(yolo.effective(), NativeSandboxMode::None);
        assert_eq!(sandbox.effective(), NativeSandboxMode::Os);
        let status = yolo
            .command(
                std::ffi::OsStr::new("/bin/sh"),
                &["-c".into(), "printf yolo > \"$OUTSIDE/denied\"".into()],
            )
            .unwrap()
            .env_clear()
            .envs(fixture.environment())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read(fixture.outside.join("denied")).unwrap(),
            b"yolo"
        );
    }

    #[test]
    fn sandbox_real_background_gates_release_and_inherits_policy() {
        let _serial = NATIVE.lock().unwrap();
        let fixture = Fixture::new("allowed");
        let helper = helper(
            crate::BACKGROUND_PROCESS_HELPER_ARGUMENT,
            "os_sandbox::tests::macos::background_helper_entry",
        );
        let adapter = SystemBackgroundProcessAdapter::with_helper(
            BackgroundProcessHelper::new(helper.program().to_owned(), helper.arguments().to_vec())
                .unwrap(),
        );
        let request = BackgroundProcessRequest::from_directory(
            COMMAND.into(),
            fixture.allowed.to_str().unwrap().into(),
            fixture.environment(),
            fixture.cwd(),
        )
        .unwrap()
        .with_sandbox(fixture.snapshot(PermissionMode::Auto));
        let prepared = adapter.prepare(request).unwrap();
        assert!(!fixture.allowed.join("ok").exists());
        prepared.release().unwrap().wait().unwrap();
        fixture.assert_effects();
    }

    #[test]
    #[ignore = "private helper subprocess entry"]
    fn background_helper_entry() {
        assert!(std::env::var_os("MACHINE_GOD_BACKGROUND_HELPER_MODE").is_some());
        crate::run_background_process_helper().unwrap();
        unreachable!();
    }

    fn pty_request(
        fixture: &Fixture,
        command: &str,
        sandbox: Arc<NativeSandboxLaunch>,
    ) -> TerminalPtyRequest {
        TerminalPtyRequest::new(
            "/bin/sh".into(),
            vec!["-c".into(), command.into()],
            fixture.environment(),
            fixture.cwd(),
            TerminalPtyDimensions {
                rows: 24,
                columns: 80,
            },
        )
        .unwrap()
        .with_sandbox(sandbox)
    }
    #[test]
    fn sandbox_real_pty_enforces_writes_and_drop_retains_cleanup() {
        let _serial = NATIVE.lock().unwrap();
        let fixture = Fixture::new("allowed");
        let helper = helper(
            crate::TERMINAL_PTY_HELPER_ARGUMENT,
            "os_sandbox::tests::macos::pty_helper_entry",
        );
        let command = format!("{COMMAND}; exec /bin/sleep 30");
        let prepared = PreparedTerminalPty::prepare_until(
            &helper,
            pty_request(&fixture, &command, fixture.snapshot(PermissionMode::Ask)),
            Instant::now() + Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(!fixture.allowed.join("ok").exists());
        let mut pty = prepared.commit(&CancellationToken::new()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        while !output.windows(12).any(|bytes| bytes == b"SANDBOX_DONE") {
            assert!(
                Instant::now() < deadline,
                "sandbox PTY marker missing: {output:?}"
            );
            let mut bytes = [0; 4096];
            let read = pty.read(&mut bytes).unwrap();
            output.extend_from_slice(&bytes[..read.bytes_read]);
            std::thread::sleep(Duration::from_millis(2));
        }
        fixture.assert_effects();
        pty.close(true).unwrap();
    }
    #[test]
    #[ignore = "private helper subprocess entry"]
    fn pty_helper_entry() {
        crate::terminal_helper::run_terminal_pty_helper().unwrap();
        unreachable!();
    }

    #[test]
    fn sandbox_replaced_root_rejects_pending_pty_commit_and_preparation() {
        let _serial = NATIVE.lock().unwrap();
        let fixture = Fixture::new("allowed");
        let sandbox = fixture.snapshot(PermissionMode::Ask);
        let helper = helper(
            crate::TERMINAL_PTY_HELPER_ARGUMENT,
            "os_sandbox::tests::macos::pty_helper_entry",
        );
        let prepared = PreparedTerminalPty::prepare_until(
            &helper,
            pty_request(&fixture, COMMAND, Arc::clone(&sandbox)),
            Instant::now() + Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .unwrap();
        let moved = fixture.base.join("moved");
        std::fs::rename(&fixture.allowed, &moved).unwrap();
        std::fs::create_dir(&fixture.allowed).unwrap();
        assert_eq!(
            sandbox.revalidate(
                Instant::now() + Duration::from_secs(2),
                &CancellationToken::new()
            ),
            Err(NativeSandboxError::Changed)
        );
        assert!(prepared.commit(&CancellationToken::new()).is_err());
        assert!(!moved.join("ok").exists());
        assert!(!fixture.allowed.join("ok").exists());
        std::fs::remove_dir(&fixture.allowed).unwrap();
        std::os::unix::fs::symlink(&moved, &fixture.allowed).unwrap();
        assert_eq!(
            sandbox.revalidate(
                Instant::now() + Duration::from_secs(2),
                &CancellationToken::new()
            ),
            Err(NativeSandboxError::Changed)
        );
    }

    #[test]
    fn sandbox_background_cancel_drop_and_replaced_root_never_release_command() {
        let _serial = NATIVE.lock().unwrap();
        let fixture = Fixture::new("allowed");
        let helper = helper(
            crate::BACKGROUND_PROCESS_HELPER_ARGUMENT,
            "os_sandbox::tests::macos::background_helper_entry",
        );
        let adapter = SystemBackgroundProcessAdapter::with_helper(
            BackgroundProcessHelper::new(helper.program().to_owned(), helper.arguments().to_vec())
                .unwrap(),
        );
        let snapshot = fixture.snapshot(PermissionMode::Ask);
        let request = || {
            BackgroundProcessRequest::from_directory(
                COMMAND.into(),
                fixture.allowed.to_str().unwrap().into(),
                fixture.environment(),
                fixture.cwd(),
            )
            .unwrap()
            .with_sandbox(Arc::clone(&snapshot))
        };
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(
            adapter
                .prepare_cancellable(request(), &cancellation)
                .is_err()
        );
        let prepared = adapter.prepare(request()).unwrap();
        assert!(prepared.release_cancellable(&cancellation).is_err());
        drop(adapter.prepare(request()).unwrap());
        let prepared = adapter.prepare(request()).unwrap();
        let moved = fixture.base.join("moved");
        std::fs::rename(&fixture.allowed, &moved).unwrap();
        std::fs::create_dir(&fixture.allowed).unwrap();
        assert!(prepared.release().is_err());
        assert!(!moved.join("ok").exists());
        assert!(!fixture.allowed.join("ok").exists());
        assert!(!fixture.outside.join("denied").exists());
    }

    #[test]
    fn sandbox_capture_rejects_wrong_executable_and_non_directory_roots() {
        let _serial = NATIVE.lock().unwrap();
        let fixture = Fixture::new("allowed");
        let root = || {
            NativeSandboxRoot::new(
                File::open(&fixture.allowed).unwrap(),
                fixture.allowed.clone(),
            )
            .unwrap()
        };
        assert_eq!(
            NativeSandboxLaunch::capture(
                NativeSandboxMode::Os,
                PermissionMode::Ask,
                vec![root()],
                Some(File::open("/bin/sh").unwrap()),
                false,
                Instant::now() + Duration::from_secs(2),
                &CancellationToken::new()
            )
            .unwrap_err(),
            NativeSandboxError::Changed
        );
        let path = fixture.allowed.join("file");
        std::fs::write(&path, b"ordinary").unwrap();
        assert_eq!(
            NativeSandboxLaunch::capture(
                NativeSandboxMode::Os,
                PermissionMode::Ask,
                vec![NativeSandboxRoot::new(File::open(&path).unwrap(), path).unwrap()],
                Some(File::open(NATIVE_SANDBOX_EXECUTABLE).unwrap()),
                false,
                Instant::now() + Duration::from_secs(2),
                &CancellationToken::new()
            )
            .unwrap_err(),
            NativeSandboxError::Changed
        );
    }
}
