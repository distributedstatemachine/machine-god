// Included inside terminal_tmux::tests; these exercise owned command cleanup.

struct CommandReapHold(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl CommandReapHold {
    fn release(&self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for CommandReapHold {
    fn drop(&mut self) {
        self.release();
    }
}

fn deferred_reap_command() -> (CommandProcess, CommandReapHold) {
    let mut command = Command::new("/bin/cat");
    command.env_clear();
    let mut process = CommandProcess::spawn(
        command,
        Vec::new(),
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    let hold = CommandReapHold(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        true,
    )));
    process
        .child
        .defer_reap_for_test(std::sync::Arc::clone(&hold.0));
    (process, hold)
}

fn settle_command_reap_scope(scope: &crate::NativeOwnedWorkerScope, pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !scope.completion().is_complete() {
        assert!(Instant::now() < deadline, "command cleanup did not settle");
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
fn deferred_command_reap_returns_failure_retains_owner_and_allows_retry() {
    let scope = crate::NativeOwnedWorkerScope::new();
    let (mut process, hold) = futures_executor::block_on(scope.run(deferred_reap_command)).unwrap();
    let pid = process.child.id();
    assert!(process.abort().is_err());
    assert!(!process.reaped);
    assert!(process.child.is_owned_for_test());
    assert_eq!(process.child.id(), pid);
    assert!(process.input.is_none() && process.output.is_none() && process.error.is_none());
    // The same caller can continue cleanup instead of blocking in wait().
    let mut command = Command::new("/bin/cat");
    command.env_clear();
    let mut other = CommandProcess::spawn(
        command,
        Vec::new(),
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    other.abort().unwrap();
    assert!(other.reaped);
    scope.close();
    assert!(!scope.completion().is_complete());
    hold.release();
    process.abort().unwrap();
    assert!(process.reaped);
    assert!(!process.child.is_owned_for_test());
    process.abort().unwrap();
    settle_command_reap_scope(&scope, pid);
}

#[test]
fn deferred_command_drop_keeps_quarantine_in_its_original_scope() {
    let scope = crate::NativeOwnedWorkerScope::new();
    let (mut process, hold) = futures_executor::block_on(scope.run(deferred_reap_command)).unwrap();
    let pid = process.child.id();
    assert!(process.abort().is_err());
    scope.close();
    drop(process);
    assert!(!scope.completion().is_complete());
    let unrelated = crate::NativeOwnedWorkerScope::new();
    unrelated.close();
    assert!(unrelated.completion().is_complete());
    hold.release();
    settle_command_reap_scope(&scope, pid);
}
