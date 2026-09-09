// A real owned child and socket isolate retirement behavior from the unrelated
// tmux bootstrap protocol.

struct ServerReapHold(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl ServerReapHold {
    fn release(&self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for ServerReapHold {
    fn drop(&mut self) {
        self.release();
    }
}

fn deferred_reap_server(
    directory: OwnedFd,
    path: PathBuf,
) -> (NativeTerminalTmuxServer, ServerReapHold, PathBuf) {
    let socket = path.join("retained.sock");
    let mut artifacts = Artifacts::new(directory, path).unwrap();
    let listener = UnixListener::bind(&socket).unwrap();
    artifacts.remember_socket(&socket).unwrap();
    drop(listener);
    let mut command = std::process::Command::new("/bin/cat");
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = crate::background_process::TmuxChild::spawn(&mut command).unwrap();
    let hold = ServerReapHold(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        true,
    )));
    child.defer_reap_for_test(std::sync::Arc::clone(&hold.0));
    let server = NativeTerminalTmuxServer {
        helper: TerminalPtyHelper::new("/bin/false".into(), Vec::new()).unwrap(),
        child: Some(child),
        executable: "/bin/cat".into(),
        environment: ValidatedBackgroundEnvironment::new(Vec::new()).unwrap(),
        artifacts: Some(artifacts),
        socket: socket.clone(),
    };
    (server, hold, socket)
}

fn settle_server_reap_scope(scope: &crate::NativeOwnedWorkerScope, pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !scope.completion().is_complete() {
        assert!(Instant::now() < deadline, "server cleanup did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
    scope.completion().wait_on_worker().unwrap();
    let pid = rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap();
    assert!(matches!(
        rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
        Err(rustix::io::Errno::CHILD)
    ));
}

#[test]
fn deferred_server_retire_preserves_child_and_namespace_until_retry_reaps() {
    let directory = Directory::new();
    let path = directory.0.clone();
    let fd = directory.fd();
    let scope = crate::NativeOwnedWorkerScope::new();
    let (mut server, hold, socket) =
        futures_executor::block_on(scope.run(move || deferred_reap_server(fd, path))).unwrap();
    let pid = server.child.as_ref().unwrap().id();
    assert!(server.retire().is_err());
    assert!(server.child.as_ref().unwrap().is_owned_for_test());
    assert_eq!(server.child.as_ref().unwrap().id(), pid);
    assert_eq!(server.artifacts.as_ref().unwrap().sockets.len(), 1);
    assert!(socket.exists());
    scope.close();
    assert!(!scope.completion().is_complete());
    hold.release();
    server.retire().unwrap();
    assert!(server.child.is_none());
    assert!(server.artifacts.as_ref().unwrap().sockets.is_empty());
    assert!(!socket.exists());
    server.retire().unwrap();
    settle_server_reap_scope(&scope, pid);
}

#[test]
fn deferred_server_drop_retains_artifacts_through_quarantine_and_scope_settlement() {
    let directory = Directory::new();
    let path = directory.0.clone();
    let fd = directory.fd();
    let scope = crate::NativeOwnedWorkerScope::new();
    let (mut server, hold, socket) =
        futures_executor::block_on(scope.run(move || deferred_reap_server(fd, path))).unwrap();
    let pid = server.child.as_ref().unwrap().id();
    assert!(server.retire().is_err());
    scope.close();
    drop(server);
    assert!(!scope.completion().is_complete());
    assert!(
        socket.exists(),
        "unreaped server lost its namespace artifact"
    );
    hold.release();
    settle_server_reap_scope(&scope, pid);
    assert!(
        !socket.exists(),
        "scope completed before artifact retirement"
    );
}
#[cfg(target_os = "macos")]
#[test]
fn sandbox_real_tmux_shell_inherits_os_write_policy() {
    let _serial = crate::os_sandbox::NATIVE_TESTS.lock().unwrap();
    let directory = Directory::new();
    let extra = Directory::new();
    let outside = Directory::new();
    let outside_path = outside.0.canonicalize().unwrap();
    assert!(!outside_path.starts_with("/private/tmp") && !outside_path.starts_with("/tmp"));
    let Some(mut request) = request(
        &directory,
        "printf allowed > ok; printf extra > \"$EXTRA/ok\"; printf denied > \"$OUTSIDE/denied\"; /bin/sh -c 'printf denied > \"$OUTSIDE/descendant\"'; printf SANDBOX_DONE; exec /bin/sleep 30",
    ) else {
        assert!(
            std::env::var_os("MACHINE_GOD_TERMINAL_TMUX_BINARY").is_none(),
            "configured tmux must exist"
        );
        return;
    };
    request
        .environment
        .push(("OUTSIDE".into(), outside_path.as_os_str().to_owned()));
    request
        .environment
        .push(("EXTRA".into(), extra.0.as_os_str().to_owned()));
    request.sandbox = Some(std::sync::Arc::new(
        crate::NativeSandboxLaunch::capture(
            crate::NativeSandboxMode::Os,
            crate::PermissionMode::Ask,
            [&directory.0, &extra.0]
                .into_iter()
                .map(|path| {
                    crate::NativeSandboxRoot::new(
                        std::fs::File::open(path).unwrap(),
                        path.canonicalize().unwrap(),
                    )
                    .unwrap()
                })
                .collect(),
            Some(std::fs::File::open(crate::NATIVE_SANDBOX_EXECUTABLE).unwrap()),
            false,
            Instant::now() + Duration::from_secs(2),
            &CancellationToken::new(),
        )
        .unwrap(),
    ));
    let prepared = PreparedTerminalTmuxLaunch::prepare(request, &CancellationToken::new()).unwrap();
    assert!(!directory.0.join("ok").exists());
    let mut backend = prepared.commit_owned(&CancellationToken::new()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    while !output.windows(12).any(|bytes| bytes == b"SANDBOX_DONE") {
        assert!(
            Instant::now() < deadline,
            "sandbox tmux marker missing: {output:?}"
        );
        let mut bytes = [0; 4096];
        let read = backend.read(&mut bytes).unwrap();
        output.extend_from_slice(&bytes[..read.bytes_read]);
        std::thread::sleep(PAUSE);
    }
    assert_eq!(std::fs::read(directory.0.join("ok")).unwrap(), b"allowed");
    assert_eq!(std::fs::read(extra.0.join("ok")).unwrap(), b"extra");
    assert!(!outside.0.join("denied").exists());
    assert!(!outside.0.join("descendant").exists());
    backend.close(true, &mut |_| {}).unwrap();
}
use super::*;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let nonce = nonce().unwrap();
        let path = std::env::temp_dir().join(format!(
            "mg-tl-{}",
            std::str::from_utf8(&nonce[..12]).unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(std::fs::canonicalize(path).unwrap())
    }
    fn fd(&self) -> OwnedFd {
        rustix::fs::open(
            &self.0,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn helper() -> TerminalPtyHelper {
    if let Some(program) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
        return TerminalPtyHelper::new(
            PathBuf::from(program),
            vec![TERMINAL_TMUX_HELPER_ARGUMENT.into()],
        )
        .unwrap()
        .with_test_inventory_helper();
    }
    let program = std::env::current_exe().unwrap();
    let script = format!(
        "export MG_TMUX_KIND=\"$1\" MG_TMUX_SOCKET=\"$2\" MG_TMUX_NONCE=\"$3\" MG_TMUX_CWD=\"$4\"; if [ \"$1\" = exec ]; then exec 2>&1; exec 1>/dev/null; fi; exec '{}' --exact terminal_tmux_startup::tests::helper_entry --nocapture",
        program.to_str().unwrap().replace('\'', "'\\''")
    );
    TerminalPtyHelper::new(
        "/bin/sh".into(),
        vec!["-c".into(), script.into(), "helper".into()],
    )
    .unwrap()
    .with_test_inventory_helper()
}

fn marker_helper() -> TerminalPtyHelper {
    if let Some(program) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
        return TerminalPtyHelper::new(
            PathBuf::from(program),
            vec![crate::terminal_helper::TERMINAL_STARTUP_MARKER_ARGUMENT.into()],
        )
        .unwrap();
    }
    TerminalPtyHelper::new(
        std::env::current_exe().unwrap(),
        vec![
            "--exact".into(),
            "terminal_startup::tests::marker_helper_entry".into(),
            "--test-threads=1".into(),
            "--quiet".into(),
        ],
    )
    .unwrap()
}
#[test]
fn helper_entry() {
    let Some(kind) = std::env::var_os("MG_TMUX_KIND") else {
        return;
    };
    let result = run_terminal_tmux_helper(&[
        kind,
        std::env::var_os("MG_TMUX_SOCKET").unwrap(),
        std::env::var_os("MG_TMUX_NONCE").unwrap(),
        std::env::var_os("MG_TMUX_CWD").unwrap(),
    ]);
    std::process::exit(if result.is_ok() { 0 } else { 125 });
}

#[test]
fn capture_completion_requires_exact_success_and_preserves_failure_on_retry() {
    for (frame, expected) in [
        (&b""[..], false),
        (&b"C"[..], false),
        (&b"C\x01"[..], false),
        (&b"X\x00"[..], false),
        (&b"C\x00"[..], true),
    ] {
        let (channel, mut peer) = UnixStream::pair().unwrap();
        channel.set_nonblocking(true).unwrap();
        peer.write_all(frame).unwrap();
        drop(peer);
        let mut completion = CaptureCompletion::new(channel);
        assert_eq!(completion.finish(), expected);
        assert_eq!(completion.finish(), expected);
    }
}

#[test]
fn tty_receipt_requires_complete_frame_and_original_deadline_and_cancellation() {
    let tty =
        rustix::fs::open("/dev/null", OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty()).unwrap();
    let pid = NonZeroU32::new(41).unwrap();
    for length in [0, 4, TTY_PROOF_BYTES - 1] {
        let (mut channel, mut sender) = UnixStream::pair().unwrap();
        channel.set_nonblocking(true).unwrap();
        sender.write_all(&[0; TTY_PROOF_BYTES][..length]).unwrap();
        drop(sender);
        assert_eq!(
            read_tty_proof(
                &mut channel,
                &tty,
                pid,
                Instant::now() + Duration::from_secs(1),
                &CancellationToken::new()
            ),
            Err(TerminalTmuxLaunchError::Protocol)
        );
    }
    let (mut channel, _sender) = UnixStream::pair().unwrap();
    channel.set_nonblocking(true).unwrap();
    assert_eq!(
        read_tty_proof(
            &mut channel,
            &tty,
            pid,
            Instant::now(),
            &CancellationToken::new()
        ),
        Err(TerminalTmuxLaunchError::Timeout)
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        read_tty_proof(
            &mut channel,
            &tty,
            pid,
            Instant::now() + Duration::from_secs(1),
            &cancellation
        ),
        Err(TerminalTmuxLaunchError::Cancelled)
    );
}
fn request(directory: &Directory, command: &str) -> Option<TerminalTmuxLaunchRequest> {
    let explicit = std::env::var_os("MACHINE_GOD_TERMINAL_TMUX_BINARY");
    let executable = explicit.clone().map(PathBuf::from).or_else(|| {
        [
            "/opt/homebrew/bin/tmux",
            "/usr/bin/tmux",
            "/usr/local/bin/tmux",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
    })?;
    if explicit.is_some() {
        assert!(executable.is_absolute() && executable.is_file());
    }
    Some(TerminalTmuxLaunchRequest {
        sandbox: None,
        executable,
        helper: helper(),
        capture_helper: helper(),
        program: "/bin/bash".into(),
        arguments: vec![
            "--noprofile".into(),
            "--norc".into(),
            "-c".into(),
            command.into(),
        ],
        initial_source: None,
        environment: vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("TERM".into(), "xterm-256color".into()),
        ],
        cwd: directory.fd(),
        cwd_path: directory.0.clone(),
        artifacts: directory.fd(),
        artifact_path: directory.0.clone(),
        dimensions: TerminalPtyDimensions {
            rows: 37,
            columns: 99,
        },
        timeout: Duration::from_secs(10),
    })
}

#[cfg(target_os = "linux")]
struct CleanupObserver(OwnedFd);

#[cfg(target_os = "linux")]
impl CleanupObserver {
    fn from_marker(path: &Path, deadline: Instant) -> Self {
        loop {
            if let Ok(text) = std::fs::read_to_string(path)
                && let Ok(pid) = text.parse::<i32>()
            {
                return Self(
                    rustix::process::pidfd_open(
                        rustix::process::Pid::from_raw(pid).unwrap(),
                        rustix::process::PidfdFlags::empty(),
                    )
                    .unwrap(),
                );
            }
            assert!(Instant::now() < deadline, "owned job marker unavailable");
            std::thread::sleep(PAUSE);
        }
    }

    fn exited(&self) -> bool {
        let mut descriptors = [rustix::event::PollFd::new(
            &self.0,
            rustix::event::PollFlags::IN,
        )];
        rustix::event::poll(&mut descriptors, Some(&rustix::event::Timespec::default())).unwrap()
            != 0
    }
}

#[cfg(target_os = "linux")]
impl Drop for CleanupObserver {
    fn drop(&mut self) {
        // Independently retain exact fixture cleanup if an assertion fails.
        let _ = rustix::process::pidfd_send_signal(&self.0, rustix::process::Signal::KILL);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn real_force_close_progresses_at_capture_capacity_before_and_after_shell_exit() {
    for shell_exits in [false, true] {
        let directory = Directory::new();
        let ending = if shell_exits { "exit 0" } else { "wait" };
        let script = format!(
            "sleep 100 & printf '%s' \"$!\" > first; while [[ ! -e release ]]; do read -r -t 0.01 unused; done; sleep 100 & printf '%s' \"$!\" > second; sleep 100 & printf '%s' \"$!\" > third; {ending}"
        );
        let Some(request) = request(&directory, &script) else {
            return;
        };
        let cancellation = CancellationToken::new();
        let mut backend = PreparedTerminalTmuxLaunch::prepare(request, &cancellation)
            .unwrap()
            .commit_owned(&cancellation)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let first = CleanupObserver::from_marker(&directory.0.join("first"), deadline);
        {
            let process = backend.backend.process_for_test();
            let identity = process.identity.clone();
            process.validate(&identity).unwrap();
            // Full-identity mismatches never gain retained cleanup authority,
            // including a correct PID paired with a different namespace/pane.
            for wrong in [
                TerminalTmuxIdentity::new(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    identity.pane().into(),
                    identity.pid(),
                )
                .unwrap(),
                TerminalTmuxIdentity::new(
                    identity.namespace().into(),
                    "%987".into(),
                    identity.pid(),
                )
                .unwrap(),
                TerminalTmuxIdentity::new(
                    identity.namespace().into(),
                    identity.pane().into(),
                    NonZeroU32::new(std::process::id()).unwrap(),
                )
                .unwrap(),
            ] {
                assert_eq!(
                    process.signal_retained_cleanup(&wrong, TerminalSignal::Kill),
                    Err(TerminalTmuxError::Identity)
                );
                assert!(!first.exited());
            }
            process.authority.exhaust_capture_budget_for_test();
        }
        std::fs::write(directory.0.join("release"), b"ready").unwrap();
        let second = CleanupObserver::from_marker(&directory.0.join("second"), deadline);
        let third = CleanupObserver::from_marker(&directory.0.join("third"), deadline);
        if shell_exits {
            while backend.backend.process_for_test().poll_job().unwrap()
                == TerminalPtyStatus::Running
            {
                assert!(Instant::now() < deadline);
                std::thread::sleep(PAUSE);
            }
        } else {
            assert_eq!(
                backend.backend.process_for_test().poll_job().unwrap(),
                TerminalPtyStatus::Running
            );
        }
        assert!(
            backend.close(true, &mut |_| {}).is_err(),
            "incomplete inventory is not quiescence"
        );
        assert!(
            backend.server.child.is_some(),
            "failed close retains namespace ownership"
        );
        while !first.exited() {
            assert!(
                Instant::now() < deadline,
                "retained exact pin was not killed"
            );
            std::thread::sleep(PAUSE);
        }
        assert!(
            !second.exited() || !third.exited(),
            "partial inventory cannot signal unproved jobs"
        );
        // No quota is increased: each retry must free settled pins and make
        // bounded progress until a complete inventory authorizes retirement.
        let receipt = loop {
            if let Ok(receipt) = backend.close(true, &mut |_| {}) {
                break receipt;
            }
            assert!(
                Instant::now() < deadline,
                "retained cleanup did not converge"
            );
            std::thread::sleep(PAUSE);
        };
        assert_ne!(receipt.status, TerminalPtyStatus::Running);
        assert!(second.exited() && third.exited());
        assert!(backend.server.child.is_none());
    }
}

#[test]
fn real_gated_pane_cannot_execute_before_challenge_commit_and_captures_exact_bytes() {
    let directory = Directory::new();
    let Some(request) = request(
        &directory,
        "stty -opost; printf '\\000\\377\\033[31mRAW\\n'; stty size; printf yes > executed; exit 7",
    ) else {
        return;
    };
    let cancellation = CancellationToken::new();
    let mut prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
    assert!(!directory.0.join("executed").exists());
    assert!(matches!(
        prepared.commit(&cancellation),
        Err(TerminalTmuxLaunchError::Protocol)
    ));
    prepared
        .pane
        .challenge(prepared.deadline, &cancellation)
        .unwrap();
    assert!(!directory.0.join("executed").exists());
    let (_control, mut capture) = prepared.commit(&cancellation).unwrap();
    let mut output = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut buffer = [0; 1024];
    loop {
        match capture.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                output.extend_from_slice(&buffer[..count]);
                if output.ends_with(b"37 99\n") {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "capture did not close");
                std::thread::sleep(PAUSE);
            }
            Err(error) => panic!("capture failed: {error}"),
        }
    }
    assert_eq!(output, b"\0\xff\x1b[31mRAW\n37 99\n");
    let mut outcome = [0; 3];
    read_gate(
        &mut prepared.pane.channel,
        &mut outcome,
        deadline,
        &cancellation,
    )
    .unwrap();
    assert_eq!(outcome, [b'O', 0, 7]);
    assert_eq!(std::fs::read(directory.0.join("executed")).unwrap(), b"yes");
    let reply = prepared
        .server
        .command(
            &[
                "display-message".into(),
                "-p".into(),
                "-t".into(),
                prepared.pane.identity.pane().into(),
                "#{pane_dead}".into(),
            ],
            deadline,
            &cancellation,
        )
        .unwrap();
    assert!(reply.success);
    assert_eq!(
        reply.output, b"0\n",
        "the retained helper is not the completed job"
    );
    drop(capture);
    prepared.server.retire().unwrap();
    assert!(prepared.server.child.is_none());
    assert!(
        prepared
            .server
            .artifacts
            .as_ref()
            .unwrap()
            .sockets
            .is_empty()
    );
}
#[test]
fn dropping_prepared_pane_and_cancelling_commit_have_no_shell_effects() {
    for cancel in [false, true] {
        let directory = Directory::new();
        let Some(request) = request(&directory, "printf forbidden > executed") else {
            return;
        };
        let cancellation = CancellationToken::new();
        let mut prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
        if cancel {
            prepared
                .pane
                .challenge(prepared.deadline, &cancellation)
                .unwrap();
            cancellation.cancel();
            assert!(matches!(
                prepared.commit(&cancellation),
                Err(TerminalTmuxLaunchError::Cancelled)
            ));
        }
        drop(prepared);
        assert!(!directory.0.join("executed").exists());
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }
}
#[test]
fn helper_rejects_unbounded_and_malformed_private_arguments_before_connect() {
    assert_eq!(
        run_terminal_tmux_helper(&[]),
        Err(TerminalTmuxLaunchError::Invalid)
    );
    for kind in ["other", "pane", "capture"] {
        assert_eq!(
            run_terminal_tmux_helper(&[
                kind.into(),
                "relative".into(),
                "x".repeat(32).into(),
                "-".into()
            ]),
            Err(TerminalTmuxLaunchError::Invalid)
        );
    }
}

#[test]
fn real_supervisor_distinguishes_fast_signal_from_same_numeric_exit_code() {
    for (script, expected) in [
        ("kill -KILL $$", [b'O', 1, 9]),
        ("exit 137", [b'O', 0, 137]),
    ] {
        let directory = Directory::new();
        let Some(request) = request(&directory, script) else {
            return;
        };
        let cancellation = CancellationToken::new();
        let mut prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
        prepared
            .pane
            .challenge(prepared.deadline, &cancellation)
            .unwrap();
        let (_control, capture) = prepared.commit(&cancellation).unwrap();
        let mut outcome = [0; 3];
        read_gate(
            &mut prepared.pane.channel,
            &mut outcome,
            Instant::now() + Duration::from_secs(10),
            &cancellation,
        )
        .unwrap();
        assert_eq!(outcome, expected);
        drop(capture);
        prepared.server.retire().unwrap();
    }
}

#[test]
fn real_commandless_source_is_queued_before_commit_without_echo_or_a_nested_tty() {
    let directory = Directory::new();
    let Some(mut request) = request(&directory, "unused") else {
        return;
    };
    request.arguments = vec!["--noprofile".into(), "--norc".into(), "-i".into()];
    request.initial_source = Some("printf 'SOURCE\\n'; exit 0\n".into());
    request.environment.push(("PS1".into(), "".into()));
    let cancellation = CancellationToken::new();
    let mut prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
    assert!(
        !rustix::termios::tcgetattr(prepared.echo.as_ref().unwrap())
            .unwrap()
            .local_modes
            .contains(rustix::termios::LocalModes::ECHO)
    );
    prepared
        .pane
        .challenge(prepared.deadline, &cancellation)
        .unwrap();
    let (_control, mut capture) = prepared.commit(&cancellation).unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    let deadline = Instant::now() + Duration::from_secs(10);
    while !bytes.windows(6).any(|bytes| bytes == b"SOURCE") {
        match capture.read(&mut buffer) {
            Ok(count) if count != 0 => bytes.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(PAUSE);
            }
            _ => panic!("source capture closed early"),
        }
        assert!(Instant::now() < deadline);
    }
    assert!(!String::from_utf8_lossy(&bytes).contains("printf"));
    let mut outcome = [0; 3];
    read_gate(
        &mut prepared.pane.channel,
        &mut outcome,
        deadline,
        &cancellation,
    )
    .unwrap();
    assert_eq!(outcome, [b'O', 0, 0]);
    set_echo(prepared.echo.as_ref().unwrap(), true).unwrap();
    prepared.echo.take();
    drop(capture);
    prepared.server.retire().unwrap();
}

#[test]
fn real_capture_overload_closes_with_explicit_loss_instead_of_blocking_tmux() {
    let directory = Directory::new();
    let Some(request) = request(
        &directory,
        "/usr/bin/head -c 8388608 /dev/zero; printf done > done",
    ) else {
        return;
    };
    let cancellation = CancellationToken::new();
    let mut prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
    prepared
        .pane
        .challenge(prepared.deadline, &cancellation)
        .unwrap();
    let (_control, mut capture) = prepared.commit(&cancellation).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !directory.0.join("done").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(PAUSE);
    }
    let mut retained = 0;
    let mut buffer = [0; CAPTURE_CHUNK];
    loop {
        match capture.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => retained += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(PAUSE);
            }
            Err(error) => panic!("capture failure: {error}"),
        }
        assert!(Instant::now() < deadline);
    }
    assert!(retained > 0 && retained < 8_388_608);
    drop(capture);
    prepared.server.retire().unwrap();
}

fn read_until(backend: &mut NativeTerminalTmuxBackend, marker: &[u8]) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = Vec::new();
    let mut bytes = [0; 4096];
    while !output.windows(marker.len()).any(|bytes| bytes == marker) {
        let read = backend.read(&mut bytes).unwrap();
        output.extend_from_slice(&bytes[..read.bytes_read]);
        assert!(output.len() <= 64 * 1024);
        assert!(Instant::now() < deadline, "missing marker in {output:?}");
        std::thread::sleep(PAUSE);
    }
    output
}
#[test]
fn real_owned_backend_resizes_writes_reports_actual_job_exit_and_reaps_server() {
    let directory = Directory::new();
    let Some(request) = request(
        &directory,
        "stty -echo; printf READY; IFS= read -r line; stty size; printf 'GOT:%s\\n' \"$line\"; exit 7",
    ) else {
        return;
    };
    let cancellation = CancellationToken::new();
    let prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
    let mut backend = prepared.commit_owned(&cancellation).unwrap();
    assert_eq!(backend.input_write_limit(), 64 * 1024);
    read_until(&mut backend, b"READY");
    backend
        .resize(&TerminalDimensions::new(40, 120).unwrap())
        .unwrap();
    let input = b"hello\n";
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let receipt = backend.write_with_paste(input, false).unwrap();
        if receipt.bytes_written() == input.len() {
            break;
        }
        assert_eq!(receipt.bytes_written(), 0);
        assert!(Instant::now() < deadline);
        std::thread::sleep(PAUSE);
    }
    let output = read_until(&mut backend, b"GOT:hello");
    assert!(String::from_utf8_lossy(&output).contains("40 120"));
    while backend.status().unwrap() == TerminalPtyStatus::Running {
        assert!(Instant::now() < deadline);
        std::thread::sleep(PAUSE);
    }
    assert_eq!(backend.status().unwrap(), TerminalPtyStatus::Exited(7));
    let receipt = backend.close(false, &mut |_| {}).unwrap();
    assert_eq!(receipt.status, TerminalPtyStatus::Exited(7));
    assert!(!receipt.output_incomplete);
    assert!(backend.server.child.is_none());
    assert!(
        backend
            .server
            .artifacts
            .as_ref()
            .unwrap()
            .sockets
            .is_empty()
    );
}

#[test]
fn real_owned_backend_preserves_fast_signal_and_cleans_reparented_jobs() {
    for (command, expected) in [
        ("kill -KILL $$", TerminalPtyStatus::Signalled(9)),
        (
            "/bin/sh -c 'trap \"\" HUP TERM; while :; do /bin/sleep 1; done' & printf 'BACKGROUND:%s\\n' \"$!\"; exit 0",
            TerminalPtyStatus::Exited(0),
        ),
    ] {
        let directory = Directory::new();
        let Some(request) = request(&directory, command) else {
            return;
        };
        let cancellation = CancellationToken::new();
        let prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
        let mut backend = prepared.commit_owned(&cancellation).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while backend.status().unwrap() == TerminalPtyStatus::Running {
            assert!(Instant::now() < deadline);
            std::thread::sleep(PAUSE);
        }
        assert_eq!(backend.status().unwrap(), expected);
        let mut output = Vec::new();
        let receipt = backend
            .close(false, &mut |bytes| output.extend_from_slice(bytes))
            .unwrap();
        assert_eq!(receipt.status, expected);
        assert!(backend.server.child.is_none());
        if expected == TerminalPtyStatus::Exited(0) {
            assert!(String::from_utf8_lossy(&output).contains("BACKGROUND:"));
        }
    }
}

#[test]
fn real_tmux_cwd_formats_are_literal_and_owned_drop_collects_live_jobs() {
    let directory = Directory::new();
    let cwd = directory.0.join("w#{pid} 'quoted'");
    std::fs::create_dir(&cwd).unwrap();
    let Some(mut request) = request(
        &directory,
        "printf 'CWD:%s\\n' \"$PWD\"; while :; do /bin/sleep 1; done",
    ) else {
        return;
    };
    request.cwd = rustix::fs::open(
        &cwd,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap();
    request.cwd_path = cwd.clone();
    let cancellation = CancellationToken::new();
    let prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
    let pid = rustix::process::Pid::from_raw(
        i32::try_from(prepared.server.child.as_ref().unwrap().id()).unwrap(),
    )
    .unwrap();
    let mut backend = prepared.commit_owned(&cancellation).unwrap();
    let output = read_until(&mut backend, b"quoted'");
    assert!(String::from_utf8_lossy(&output).contains(cwd.to_str().unwrap()));
    drop(backend);
    assert!(matches!(
        rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
        Err(rustix::io::Errno::CHILD)
    ));
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn real_failed_close_keeps_exact_artifact_cleanup_for_retry() {
    let directory = Directory::new();
    let Some(request) = request(&directory, "printf READY; while :; do /bin/sleep 1; done") else {
        return;
    };
    let cancellation = CancellationToken::new();
    let prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
    let original = prepared.server.artifacts.as_ref().unwrap().sockets[0]
        .0
        .clone();
    let parked = directory.0.join("parked");
    let mut backend = prepared.commit_owned(&cancellation).unwrap();
    read_until(&mut backend, b"READY");
    std::fs::rename(&original, &parked).unwrap();
    let replacement = crate::terminal_helper::bind_startup_listener(
        &helper(),
        false,
        &directory.fd(),
        &directory.0,
        original.file_name().unwrap().to_str().unwrap(),
        Instant::now() + Duration::from_secs(10),
        &cancellation,
    )
    .unwrap();
    assert!(backend.close(true, &mut |_| {}).is_err());
    assert!(backend.server.child.is_none());
    assert_eq!(backend.server.artifacts.as_ref().unwrap().sockets.len(), 1);
    assert!(original.exists() && parked.exists());
    drop(replacement);
    std::fs::remove_file(&original).unwrap();
    std::fs::rename(&parked, &original).unwrap();
    backend.close(true, &mut |_| {}).unwrap();
    assert!(
        backend
            .server
            .artifacts
            .as_ref()
            .unwrap()
            .sockets
            .is_empty()
    );
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
}

#[test]
fn real_capture_overload_remains_an_explicit_gap_after_fast_job_exit() {
    let directory = Directory::new();
    let Some(request) = request(&directory, "/usr/bin/head -c 8388608 /dev/zero; exit 0") else {
        return;
    };
    let cancellation = CancellationToken::new();
    let prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
    let mut backend = prepared.commit_owned(&cancellation).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while backend.status().unwrap() == TerminalPtyStatus::Running {
        assert!(Instant::now() < deadline);
        std::thread::sleep(PAUSE);
    }
    let mut received = 0;
    let receipt = backend
        .close(false, &mut |bytes| received += bytes.len())
        .unwrap();
    assert_eq!(receipt.status, TerminalPtyStatus::Exited(0));
    assert!(receipt.output_incomplete);
    assert!(received > 0 && received < 8 * 1024 * 1024);
    assert!(backend.server.child.is_none());
}

fn startup_event<B: TerminalSessionBackend>(
    backend: &mut B,
    control: &mut crate::terminal_startup::TerminalStartupControl,
    expected: crate::terminal_startup::TerminalStartupEvent,
    output: &mut Vec<u8>,
    deadline: Instant,
    scenario: &str,
) {
    loop {
        let mut bytes = [0; 4096];
        let read = backend.read(&mut bytes).unwrap_or_else(|()| {
            panic!(
                "{scenario}: read failed awaiting {expected:?}; deadline_remaining={:?}; output_bytes={}; tail={:?}",
                deadline.checked_duration_since(Instant::now()),
                output.len(),
                String::from_utf8_lossy(&output[output.len().saturating_sub(2048)..])
            )
        });
        output.extend_from_slice(&bytes[..read.bytes_read]);
        assert!(output.len() < 64 * 1024);
        if let Some(event) = control
            .poll(Instant::now(), &CancellationToken::new())
            .unwrap()
        {
            assert_eq!(event, expected);
            return;
        }
        assert!(Instant::now() < deadline, "startup output: {output:?}");
        std::thread::sleep(PAUSE);
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "One real matrix exercises the shared startup contract across both shells and profiles."
)]
fn real_tmux_shared_bootstrap_preserves_profiles_and_durable_command_gate() {
    use crate::terminal_shell::TerminalShell;
    use crate::terminal_startup::{PreparedTerminalBootstrap, TerminalStartupEvent};
    for shell in ["/bin/bash", "/bin/zsh"] {
        if !Path::new(shell).exists() {
            continue;
        }
        for clean in [false, true] {
            for commandless in [false, true] {
                let scenario = format!("shell={shell} clean={clean} commandless={commandless}");
                let directory = Directory::new();
                let artifact_root = Directory::new();
                let mut artifact_path = artifact_root.0.clone();
                while artifact_path.as_os_str().as_bytes().len() < 850 {
                    artifact_path.push(format!("é{}", "'".repeat(60)));
                    std::fs::create_dir(&artifact_path).unwrap();
                    std::fs::set_permissions(
                        &artifact_path,
                        std::fs::Permissions::from_mode(0o700),
                    )
                    .unwrap();
                }
                let artifacts = Directory(artifact_path);
                let Some(mut request) = request(&directory, "unused") else {
                    return;
                };
                let profile = if shell.ends_with("bash") {
                    ".bash_profile"
                } else {
                    ".zprofile"
                };
                std::fs::write(directory.0.join(profile), "set -a; export FROM_PROFILE=user; printf PROFILE_OUTPUT; for fd in 3 4 5 6 7 8 9; do eval \"exec $fd>&-\"; done\n").unwrap();
                let source = format!(
                    "test \"${{FROM_PROFILE-unset}}\" = {} || exit 7; if /usr/bin/env | /usr/bin/grep '^_mg_bootstrap_' >/dev/null; then exit 8; fi; printf command > executed; exec /bin/sh -c 'exit 23'",
                    if clean { "unset" } else { "user" }
                );
                let marker = marker_helper();
                let cancellation = CancellationToken::new();
                let deadline = Instant::now() + Duration::from_secs(10);
                let bootstrap = PreparedTerminalBootstrap::new(
                    &TerminalShell::from_executable(Path::new(shell), clean).unwrap(),
                    (!commandless).then_some(source.as_str()),
                    artifacts.fd(),
                    artifacts.0.clone(),
                    &marker,
                    deadline,
                    &cancellation,
                )
                .unwrap();
                request.program = bootstrap.program().into();
                request.arguments = bootstrap.arguments().to_vec();
                request.initial_source = bootstrap.startup_source().map(str::to_owned);
                request.environment.extend([
                    ("HOME".into(), directory.0.clone().into_os_string()),
                    ("ZDOTDIR".into(), directory.0.clone().into_os_string()),
                ]);
                request.artifacts = artifacts.fd();
                request.artifact_path = artifacts.0.clone();
                let prepared =
                    PreparedTerminalTmuxLaunch::prepare_until(request, deadline, &cancellation)
                        .unwrap();
                let published = bootstrap.publish(&cancellation).unwrap();
                let (mut backend, mut control) =
                    published.attach(prepared.commit_owned(&cancellation).unwrap());
                let mut output = Vec::new();
                assert_eq!(backend.write(b"bad input\n").unwrap().bytes_written(), 0);
                startup_event(
                    &mut backend,
                    &mut control,
                    TerminalStartupEvent::ShellReady,
                    &mut output,
                    deadline,
                    &scenario,
                );
                assert!(!directory.0.join("executed").exists());
                assert!(!String::from_utf8_lossy(&output).contains("_machine_god_ack"));
                backend.restore_startup_echo().unwrap();
                assert!(
                    control
                        .acknowledge_shell_ready(Instant::now(), &cancellation)
                        .unwrap()
                );
                if commandless {
                    assert!(control.is_complete());
                    let input = format!("{source}\n");
                    loop {
                        let receipt = backend.write(input.as_bytes()).unwrap();
                        if receipt.bytes_written() == input.len() {
                            break;
                        }
                        assert_eq!(receipt.bytes_written(), 0);
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(PAUSE);
                    }
                } else {
                    startup_event(
                        &mut backend,
                        &mut control,
                        TerminalStartupEvent::CommandStarted,
                        &mut output,
                        deadline,
                        &scenario,
                    );
                    assert!(!directory.0.join("executed").exists());
                    assert!(
                        control
                            .release_command(Instant::now(), &cancellation)
                            .unwrap()
                    );
                }
                while backend.status().unwrap() == TerminalPtyStatus::Running {
                    let mut bytes = [0; 4096];
                    let read = backend.read(&mut bytes).unwrap();
                    output.extend_from_slice(&bytes[..read.bytes_read]);
                    assert!(output.len() < 64 * 1024);
                    assert!(Instant::now() < deadline, "output: {output:?}");
                    std::thread::sleep(PAUSE);
                }
                assert_eq!(backend.status().unwrap(), TerminalPtyStatus::Exited(23));
                backend
                    .close(false, &mut |bytes| output.extend_from_slice(bytes))
                    .unwrap();
                control.retry_cleanup().unwrap();
                assert_eq!(
                    std::fs::read_to_string(directory.0.join("executed")).unwrap(),
                    "command"
                );
                assert_eq!(
                    String::from_utf8_lossy(&output).contains("PROFILE_OUTPUT"),
                    !clean
                );
                assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
            }
        }
    }
}
