// Included inside terminal_tmux_startup::tests. A real owned child and socket
// isolate retirement behavior from the unrelated tmux bootstrap protocol.

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
    let mut artifacts = Artifacts::new(directory, path.clone()).unwrap();
    let socket = path.join("retained.sock");
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
