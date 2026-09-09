// Exercise owned command cleanup alongside the tmux lifecycle fixtures.

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
use super::*;
use std::collections::VecDeque;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};

fn identity() -> TerminalTmuxIdentity {
    TerminalTmuxIdentity::new(
        "0123456789abcdef0123456789abcdef".into(),
        "%7".into(),
        NonZeroU32::new(42).unwrap(),
    )
    .unwrap()
}
fn observation(
    id: &TerminalTmuxIdentity,
    status: TerminalPtyStatus,
    dimensions: &TerminalDimensions,
) -> Vec<u8> {
    let (dead, code) = match status {
        TerminalPtyStatus::Running => (0, String::new()),
        TerminalPtyStatus::Exited(code) => (1, code.to_string()),
        TerminalPtyStatus::Signalled(_) => (1, String::new()),
    };
    format!(
        "{}|{}|{}|{}|{dead}|{code}|{}|{}\n",
        id.namespace,
        id.session,
        id.pane,
        id.pid,
        dimensions.columns(),
        dimensions.rows()
    )
    .into_bytes()
}
#[allow(
    clippy::struct_excessive_bools,
    reason = "Independent fault injections for the deterministic backend."
)]
struct State {
    allowed: bool,
    absent: bool,
    status: TerminalPtyStatus,
    dimensions: TerminalDimensions,
    commands: Vec<TerminalTmuxCommand>,
    signals: Vec<TerminalSignal>,
    cleanup_signals: Vec<TerminalSignal>,
    close_failure: Option<&'static str>,
    validations: usize,
    outcome_available: bool,
    signal_exits: bool,
    namespace_present: bool,
    fails: VecDeque<TerminalTmuxCommand>,
    polls_pending: usize,
    suppress_resize: bool,
    job_status: Option<TerminalPtyStatus>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            allowed: true,
            absent: false,
            status: TerminalPtyStatus::Running,
            dimensions: TerminalDimensions::new(24, 80).unwrap(),
            commands: Vec::new(),
            signals: Vec::new(),
            cleanup_signals: Vec::new(),
            close_failure: None,
            validations: 0,
            outcome_available: true,
            signal_exits: true,
            namespace_present: true,
            fails: VecDeque::new(),
            polls_pending: 0,
            suppress_resize: false,
            job_status: None,
        }
    }
}
struct Control {
    id: TerminalTmuxIdentity,
    state: Arc<Mutex<State>>,
    pending: Option<TerminalTmuxCommand>,
}
impl TerminalTmuxControl for Control {
    fn identity(&self) -> &TerminalTmuxIdentity {
        &self.id
    }
    fn begin(&mut self, command: TerminalTmuxCommand, deadline: Instant) -> Result<()> {
        if self.pending.is_some() {
            return Err(TerminalTmuxError::Busy);
        }
        if Instant::now() >= deadline {
            return Err(TerminalTmuxError::Timeout);
        }
        self.state.lock().unwrap().commands.push(command.clone());
        self.pending = Some(command);
        Ok(())
    }
    fn poll(&mut self) -> Poll<Result<TerminalTmuxReply>> {
        let mut state = self.state.lock().unwrap();
        if state.polls_pending != 0 {
            state.polls_pending -= 1;
            return Poll::Pending;
        }
        let command = self.pending.take().expect("owned command");
        if state.fails.front() == Some(&command) {
            state.fails.pop_front();
            return Poll::Ready(Err(TerminalTmuxError::Command));
        }
        let mut success = true;
        let output = match command {
            TerminalTmuxCommand::Probe => b"tmux 3.2\n".to_vec(),
            TerminalTmuxCommand::Inspect => observation(&self.id, state.status, &state.dimensions),
            TerminalTmuxCommand::Resize(dimensions) => {
                if !state.suppress_resize {
                    state.dimensions = dimensions;
                }
                Vec::new()
            }
            TerminalTmuxCommand::KillSession => {
                state.namespace_present = false;
                Vec::new()
            }
            TerminalTmuxCommand::HasSession => {
                success = state.namespace_present;
                Vec::new()
            }
            _ => Vec::new(),
        };
        Poll::Ready(Ok(TerminalTmuxReply { success, output }))
    }
    fn abort(&mut self) -> Result<()> {
        if self.state.lock().unwrap().close_failure == Some("abort") {
            return Err(TerminalTmuxError::Cleanup);
        }
        self.pending.take();
        Ok(())
    }
}
struct Process {
    id: TerminalTmuxIdentity,
    state: Arc<Mutex<State>>,
}
impl TerminalTmuxProcess for Process {
    fn validate(&mut self, identity: &TerminalTmuxIdentity) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.validations += 1;
        if !state.allowed || identity != &self.id || state.close_failure == Some("validate") {
            return Err(TerminalTmuxError::Identity);
        }
        Ok(())
    }
    fn signal(&mut self, signal: TerminalSignal) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        if !state.allowed {
            return Err(TerminalTmuxError::Identity);
        }
        state.signals.push(signal);
        if state.close_failure == Some("signal") {
            return Err(TerminalTmuxError::Command);
        }
        if state.signal_exits {
            state.absent = true;
            state.status = TerminalPtyStatus::Signalled(if signal == TerminalSignal::Kill {
                9
            } else {
                15
            });
        }
        Ok(())
    }
    fn is_absent(&mut self) -> Result<bool> {
        let state = self.state.lock().unwrap();
        if state.close_failure == Some("absence") {
            return Err(TerminalTmuxError::Cleanup);
        }
        Ok(state.absent)
    }
    fn signal_retained_cleanup(
        &mut self,
        identity: &TerminalTmuxIdentity,
        signal: TerminalSignal,
    ) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        if !state.allowed || identity != &self.id {
            return Err(TerminalTmuxError::Identity);
        }
        state.cleanup_signals.push(signal);
        // Retained delivery may itself fail; it never certifies absence.
        Err(TerminalTmuxError::Command)
    }
    fn outcome(&mut self) -> Result<Option<TerminalPtyStatus>> {
        let state = self.state.lock().unwrap();
        Ok(state.outcome_available.then_some(state.status))
    }
    fn job_status(&mut self) -> Result<Option<TerminalPtyStatus>> {
        Ok(self.state.lock().unwrap().job_status)
    }
}
type Backend = TerminalTmuxBackend<Control, Process>;
fn fixture() -> (Backend, UnixStream, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let (capture, peer) = UnixStream::pair().unwrap();
    let control = Control {
        id: identity(),
        state: state.clone(),
        pending: None,
    };
    let process = Process {
        id: identity(),
        state: state.clone(),
    };
    let backend = Backend::attach(control, process, capture).unwrap();
    (backend, peer, state)
}
fn finish(mut backend: Backend, peer: UnixStream, state: &Arc<Mutex<State>>) {
    {
        let mut state = state.lock().unwrap();
        state.absent = true;
        state.status = TerminalPtyStatus::Exited(0);
    }
    drop(peer);
    backend.close_inner(true, &mut |_| {}).unwrap();
}

#[test]
fn identity_and_version_are_bounded_redacted_and_inert() {
    for namespace in [
        "",
        "0123456789ABCDEF0123456789ABCDEF",
        "../../../../../../../../../../../x",
    ] {
        assert_eq!(
            TerminalTmuxIdentity::new(namespace.into(), "%7".into(), NonZeroU32::new(1).unwrap()),
            Err(TerminalTmuxError::Invalid)
        );
    }
    for pane in ["7", "%", "%00", "%-1", "%1;kill-server", "%4294967296"] {
        assert!(
            TerminalTmuxIdentity::new(
                identity().namespace,
                pane.into(),
                NonZeroU32::new(1).unwrap()
            )
            .is_err()
        );
    }
    assert!(!format!("{:?}", identity()).contains("012345"));
    for version in ["tmux 3.2\n", "tmux 3.6a\n", "tmux 4.0"] {
        assert!(compatible_version(version.as_bytes()));
    }
    for version in [
        "tmux 3.1",
        "tmux next",
        "tmux 3.2;exec",
        "tmux 3.",
        "tmux 65536.1",
        "tmux 03.2",
    ] {
        assert!(!compatible_version(version.as_bytes()));
    }
}

#[test]
fn pane_observation_requires_exact_identity_and_authoritative_old_tmux_signal_outcome() {
    let id = identity();
    let dimensions = TerminalDimensions::new(24, 80).unwrap();
    let live = observation(&id, TerminalPtyStatus::Running, &dimensions);
    assert_eq!(
        inspect(&id, &live, || panic!(
            "running pane does not ask for outcome"
        ))
        .unwrap()
        .status,
        TerminalPtyStatus::Running
    );
    let signalled = observation(&id, TerminalPtyStatus::Signalled(9), &dimensions);
    assert_eq!(
        inspect(&id, &signalled, || Ok(Some(TerminalPtyStatus::Signalled(
            9
        ))))
        .unwrap()
        .status,
        TerminalPtyStatus::Signalled(9)
    );
    assert_eq!(
        inspect(&id, &signalled, || Ok(None)),
        Err(TerminalTmuxError::Protocol)
    );
    let mut wrong = id.clone();
    wrong.pid = NonZeroU32::new(43).unwrap();
    assert_eq!(
        inspect(&wrong, &live, || Ok(None)),
        Err(TerminalTmuxError::Identity)
    );
    assert!(inspect(&id, &live[..live.len() - 1], || Ok(None)).is_err());
    assert!(inspect(&id, &vec![b'x'; 513], || Ok(None)).is_err());
    let invalid = String::from_utf8(live)
        .unwrap()
        .replace("|0||80|24", "|2||80|24");
    assert!(inspect(&id, invalid.as_bytes(), || Ok(None)).is_err());
}

#[test]
fn denied_attach_never_executes_a_tmux_command() {
    let state = Arc::new(Mutex::new(State {
        allowed: false,
        ..State::default()
    }));
    let (capture, _peer) = UnixStream::pair().unwrap();
    let result = Backend::attach(
        Control {
            id: identity(),
            state: state.clone(),
            pending: None,
        },
        Process {
            id: identity(),
            state: state.clone(),
        },
        capture,
    );
    assert!(matches!(result, Err(TerminalTmuxError::Identity)));
    assert!(state.lock().unwrap().commands.is_empty());
}

#[test]
fn supervised_job_exit_does_not_remain_running_because_its_helper_is_alive() {
    for status in [
        TerminalPtyStatus::Exited(137),
        TerminalPtyStatus::Signalled(9),
    ] {
        let (mut backend, peer, state) = fixture();
        state.lock().unwrap().job_status = Some(status);
        backend.next_observation = Instant::now();
        backend.status().unwrap();
        assert_eq!(backend.status().unwrap(), status);
        assert!(!backend.observation.transport_closed);
        assert_eq!(state.lock().unwrap().status, TerminalPtyStatus::Running);
        finish(backend, peer, &state);
    }
}

#[test]
fn raw_capture_is_exact_nonblocking_and_has_no_startup_authority() {
    let (mut backend, mut peer, state) = fixture();
    let mut buffer = [0; 128];
    assert_eq!(backend.read(&mut buffer).unwrap().bytes_read, 0);
    let bytes = b"\0\xff\x1b[2Jfake shell-ready\r\n";
    peer.write_all(bytes).unwrap();
    let read = backend.read(&mut buffer).unwrap();
    assert_eq!(&buffer[..read.bytes_read], bytes);
    assert_eq!(backend.status().unwrap(), TerminalPtyStatus::Running);
    assert!(backend.read(&mut vec![0; READ_BYTES + 1]).is_err());
    assert!(state.lock().unwrap().signals.is_empty());
    finish(backend, peer, &state);
}

#[test]
fn writes_are_exact_single_pastes_and_keep_receipts_until_the_matching_retry() {
    let (mut backend, peer, state) = fixture();
    let bytes = b"\0\xff\r\n'$(touch forbidden)'\x1b[200~";
    assert_eq!(
        backend.write(bytes).unwrap().status(),
        BackgroundInputStatus::Backpressure
    );
    assert_eq!(
        backend.write_inner(b"different"),
        Err(TerminalTmuxError::Busy)
    );
    for _ in 0..3 {
        backend.drive().unwrap();
    }
    assert_eq!(
        backend.write_inner(b"different"),
        Err(TerminalTmuxError::Busy)
    );
    let receipt = backend.write(bytes).unwrap();
    assert_eq!(receipt.bytes_written(), bytes.len());
    assert_eq!(receipt.status(), BackgroundInputStatus::Written);
    let commands = state.lock().unwrap().commands.clone();
    assert_eq!(
        commands
            .iter()
            .filter(|command| **command == TerminalTmuxCommand::Paste)
            .count(),
        1
    );
    assert!(commands.contains(&TerminalTmuxCommand::Load(bytes.to_vec())));
    assert_eq!(backend.write(&[]).unwrap().bytes_written(), 0);
    finish(backend, peer, &state);
}

#[test]
fn maximum_paste_remains_one_operation_and_cannot_change_kind_during_retry() {
    let (mut backend, peer, state) = fixture();
    let bytes = vec![b'p'; MAX_INPUT_BYTES];
    assert_eq!(
        TerminalSessionBackend::input_write_limit(&backend),
        bytes.len()
    );
    assert_eq!(
        TerminalSessionBackend::write_with_paste(&mut backend, &bytes, true)
            .unwrap()
            .bytes_written(),
        0
    );
    assert!(backend.write_with_paste(&bytes, false).is_err());
    for _ in 0..3 {
        backend.drive().unwrap();
    }
    assert!(backend.write_with_paste(&bytes, false).is_err());
    assert_eq!(
        backend
            .write_with_paste(&bytes, true)
            .unwrap()
            .bytes_written(),
        MAX_INPUT_BYTES
    );
    let commands = state.lock().unwrap().commands.clone();
    assert_eq!(
        commands
            .iter()
            .filter(|command| **command == TerminalTmuxCommand::PasteBracketed)
            .count(),
        1
    );
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, TerminalTmuxCommand::Load(_)))
            .count(),
        1
    );
    assert!(commands.contains(&TerminalTmuxCommand::Load(bytes)));
    finish(backend, peer, &state);
}

#[test]
fn completed_paste_receipt_survives_exit_observation_and_close_before_retry() {
    let (mut backend, peer, state) = fixture();
    backend.write_with_paste(b"committed", true).unwrap();
    for _ in 0..3 {
        backend.drive().unwrap();
    }
    {
        let mut state = state.lock().unwrap();
        state.status = TerminalPtyStatus::Exited(0);
        state.absent = true;
    }
    backend.next_observation = Instant::now();
    backend.status().unwrap();
    assert_eq!(backend.status().unwrap(), TerminalPtyStatus::Exited(0));
    drop(peer);
    backend.close_inner(true, &mut |_| {}).unwrap();
    assert!(backend.write_with_paste(b"different", true).is_err());
    let receipt = backend.write_with_paste(b"committed", true).unwrap();
    assert_eq!(receipt.bytes_written(), 9);
    assert_eq!(receipt.status(), BackgroundInputStatus::Written);
    assert!(!backend.write_ambiguous());
    assert_eq!(
        backend
            .write_with_paste(b"committed", true)
            .unwrap()
            .status(),
        BackgroundInputStatus::Closed
    );
}

#[test]
fn close_of_pending_paste_never_reports_a_reliable_zero_byte_receipt() {
    let (mut backend, peer, state) = fixture();
    backend.write(b"maybe committed").unwrap();
    backend.drive().unwrap();
    backend.drive().unwrap();
    state.lock().unwrap().polls_pending = 1;
    drop(peer);
    backend.close_inner(true, &mut |_| {}).unwrap();
    assert!(backend.write_ambiguous());
    assert_eq!(
        backend.write_inner(b"maybe committed"),
        Err(TerminalTmuxError::WriteAmbiguous)
    );
}

#[test]
fn observation_only_settlement_cancels_preparation_without_submitting_paste() {
    for advances in [0, 1] {
        let (mut backend, peer, state) = fixture();
        backend.write_with_paste(b"never sent", true).unwrap();
        for _ in 0..advances {
            backend.drive().unwrap();
        }
        let Poll::Ready(Ok(receipt)) = backend.settle_write_inner(b"never sent", true) else {
            panic!()
        };
        assert_eq!(receipt.bytes_written(), 0);
        assert_eq!(receipt.status(), BackgroundInputStatus::Closed);
        backend.status().unwrap();
        assert!(
            !state
                .lock()
                .unwrap()
                .commands
                .iter()
                .any(|command| matches!(
                    command,
                    TerminalTmuxCommand::Paste | TerminalTmuxCommand::PasteBracketed
                ))
        );
        finish(backend, peer, &state);
    }
}

#[test]
fn observation_only_settlement_preserves_close_time_completion_or_ambiguity() {
    for complete in [true, false] {
        let (mut backend, peer, state) = fixture();
        backend.write_with_paste(b"pending paste", true).unwrap();
        backend.drive().unwrap();
        backend.drive().unwrap();
        state.lock().unwrap().polls_pending = if complete { 1 } else { 2 };
        assert!(
            backend
                .settle_write_inner(b"pending paste", true)
                .is_pending()
        );
        drop(peer);
        backend.close_inner(true, &mut |_| {}).unwrap();
        let completion = backend.settle_write_inner(b"pending paste", true);
        if complete {
            let Poll::Ready(Ok(receipt)) = completion else {
                panic!()
            };
            assert_eq!(receipt.bytes_written(), 13);
            assert_eq!(receipt.status(), BackgroundInputStatus::Written);
        } else {
            assert!(matches!(
                completion,
                Poll::Ready(Err(TerminalTmuxError::WriteAmbiguous))
            ));
        }
        backend.close_inner(true, &mut |_| {}).unwrap();
        assert_eq!(
            state
                .lock()
                .unwrap()
                .commands
                .iter()
                .filter(|command| **command == TerminalTmuxCommand::PasteBracketed)
                .count(),
            1
        );
    }
}

#[test]
fn backpressure_does_not_spawn_unbounded_commands_or_extend_deadlines() {
    let (mut backend, peer, state) = fixture();
    backend.write(b"x").unwrap();
    let deadline = match backend.pending.as_ref().unwrap() {
        Pending::WriteInspect(_, deadline) => *deadline,
        _ => panic!(),
    };
    state.lock().unwrap().polls_pending = 100;
    for _ in 0..100 {
        assert_eq!(backend.write(b"x").unwrap().bytes_written(), 0);
    }
    assert_eq!(state.lock().unwrap().commands.len(), 3);
    match backend.pending.as_ref().unwrap() {
        Pending::WriteInspect(_, retained) => assert_eq!(*retained, deadline),
        _ => panic!(),
    }
    assert_eq!(
        backend.write_inner(&vec![0; MAX_INPUT_BYTES + 1]),
        Err(TerminalTmuxError::Capacity)
    );
    finish(backend, peer, &state);
}

#[test]
fn permission_loss_after_load_never_pastes_and_close_retires_the_buffer() {
    let (mut backend, peer, state) = fixture();
    backend.write(b"x").unwrap();
    backend.drive().unwrap();
    state.lock().unwrap().allowed = false;
    assert_eq!(backend.drive(), Err(TerminalTmuxError::Identity));
    assert!(
        !state
            .lock()
            .unwrap()
            .commands
            .contains(&TerminalTmuxCommand::Paste)
    );
    state.lock().unwrap().allowed = true;
    finish(backend, peer, &state);
    assert!(
        state
            .lock()
            .unwrap()
            .commands
            .contains(&TerminalTmuxCommand::DeleteBuffer)
    );
}

#[test]
fn ambiguous_paste_failure_is_terminal_and_never_retried() {
    let (mut backend, peer, state) = fixture();
    state
        .lock()
        .unwrap()
        .fails
        .push_back(TerminalTmuxCommand::Paste);
    backend.write(b"x").unwrap();
    backend.drive().unwrap();
    backend.drive().unwrap();
    assert_eq!(backend.drive(), Err(TerminalTmuxError::Command));
    assert_eq!(
        backend.write_inner(b"x"),
        Err(TerminalTmuxError::WriteAmbiguous)
    );
    assert!(backend.write_ambiguous());
    assert_eq!(
        state
            .lock()
            .unwrap()
            .commands
            .iter()
            .filter(|command| **command == TerminalTmuxCommand::Paste)
            .count(),
        1
    );
    finish(backend, peer, &state);
}

#[test]
fn resize_and_signal_cannot_overtake_pending_writes_and_resize_is_verified() {
    let (mut backend, peer, state) = fixture();
    let dimensions = TerminalDimensions::new(40, 120).unwrap();
    backend.write(b"x").unwrap();
    assert!(backend.resize(&dimensions).is_err());
    assert!(backend.signal(TerminalSignal::Interrupt).is_err());
    for _ in 0..3 {
        backend.drive().unwrap();
    }
    backend.write(b"x").unwrap();
    backend.resize(&dimensions).unwrap();
    state.lock().unwrap().suppress_resize = true;
    assert!(
        backend
            .resize(&TerminalDimensions::new(41, 120).unwrap())
            .is_err()
    );
    state.lock().unwrap().signal_exits = false;
    backend.signal(TerminalSignal::Interrupt).unwrap();
    assert_eq!(state.lock().unwrap().signals, [TerminalSignal::Interrupt]);
    finish(backend, peer, &state);
}

#[test]
fn default_cleanup_fallback_grants_no_adapter_authority() {
    struct Adapter;
    impl TerminalTmuxProcess for Adapter {
        fn validate(&mut self, _: &TerminalTmuxIdentity) -> Result<()> {
            panic!("no validation fallback")
        }
        fn signal(&mut self, _: TerminalSignal) -> Result<()> {
            panic!("no signal fallback")
        }
        fn is_absent(&mut self) -> Result<bool> {
            panic!("no observation fallback")
        }
        fn outcome(&mut self) -> Result<Option<TerminalPtyStatus>> {
            panic!("no outcome fallback")
        }
    }
    assert_eq!(
        Adapter.signal_retained_cleanup(&identity(), TerminalSignal::Kill),
        Err(TerminalTmuxError::Identity)
    );
}

#[test]
fn failed_close_attempts_retained_cleanup_without_repeating_prior_delivery() {
    for force in [false, true] {
        for phase in ["abort", "validate", "absence", "signal"] {
            let (mut backend, peer, state) = fixture();
            state.lock().unwrap().close_failure = Some(phase);
            let original_error = match phase {
                "validate" => TerminalTmuxError::Identity,
                "signal" => TerminalTmuxError::Command,
                _ => TerminalTmuxError::Cleanup,
            };
            assert_eq!(backend.close_inner(force, &mut |_| {}), Err(original_error));
            let expected = if force {
                TerminalSignal::Kill
            } else {
                TerminalSignal::Terminate
            };
            {
                let state = state.lock().unwrap();
                assert!(state.namespace_present && !state.absent);
                if phase == "signal" {
                    assert_eq!(state.signals, [expected]);
                    assert!(state.cleanup_signals.is_empty());
                } else {
                    assert_eq!(state.cleanup_signals, [expected]);
                    assert!(state.signals.is_empty());
                }
            }
            state.lock().unwrap().close_failure = None;
            finish(backend, peer, &state);
        }
    }
}

#[test]
fn failed_close_never_turns_identity_rejection_into_retained_authority() {
    let (mut backend, peer, state) = fixture();
    state.lock().unwrap().allowed = false;
    assert!(backend.close_inner(true, &mut |_| {}).is_err());
    assert!(state.lock().unwrap().signals.is_empty());
    assert!(state.lock().unwrap().cleanup_signals.is_empty());
    state.lock().unwrap().allowed = true;
    finish(backend, peer, &state);
}

#[test]
fn close_drains_before_namespace_retirement_and_is_idempotent() {
    let (mut backend, mut peer, state) = fixture();
    peer.write_all(b"last tail").unwrap();
    drop(peer);
    let mut output = Vec::new();
    let receipt = backend
        .close_inner(false, &mut |bytes| output.extend_from_slice(bytes))
        .unwrap();
    assert_eq!(output, b"last tail");
    assert_eq!(receipt.status, TerminalPtyStatus::Signalled(15));
    assert!(!receipt.output_incomplete);
    assert_eq!(state.lock().unwrap().signals, [TerminalSignal::Terminate]);
    let count = state.lock().unwrap().commands.len();
    assert_eq!(
        backend.close_inner(true, &mut |_| panic!()).unwrap(),
        receipt
    );
    assert_eq!(state.lock().unwrap().commands.len(), count);
}

#[test]
fn unknown_exit_retains_namespace_for_same_owner_cleanup_retry() {
    let (mut backend, peer, state) = fixture();
    drop(peer);
    state.lock().unwrap().outcome_available = false;
    assert_eq!(
        backend.close_inner(true, &mut |_| {}),
        Err(TerminalTmuxError::Protocol)
    );
    assert!(state.lock().unwrap().namespace_present);
    assert!(!backend.retired);
    state.lock().unwrap().outcome_available = true;
    backend.close_inner(true, &mut |_| {}).unwrap();
    assert!(!state.lock().unwrap().namespace_present);
}

#[test]
fn namespace_retirement_failure_keeps_cleanup_retryable() {
    let (mut backend, peer, state) = fixture();
    drop(peer);
    state
        .lock()
        .unwrap()
        .fails
        .push_back(TerminalTmuxCommand::KillSession);
    assert_eq!(
        backend.close_inner(true, &mut |_| {}),
        Err(TerminalTmuxError::Cleanup)
    );
    assert!(!backend.retired);
    backend.close_inner(true, &mut |_| {}).unwrap();
    assert!(backend.retired);
    assert_eq!(state.lock().unwrap().signals, [TerminalSignal::Kill]);
}

struct SocketFixture {
    root: PathBuf,
    listener: UnixListener,
}
impl SocketFixture {
    fn new() -> Self {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!("mg-tmux-{:x}", u64::from_ne_bytes(random)));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let listener = UnixListener::bind(root.join("t.sock")).unwrap();
        std::fs::set_permissions(root.join("t.sock"), std::fs::Permissions::from_mode(0o600))
            .unwrap();
        Self { root, listener }
    }
    fn control(&self, executable: &str) -> NativeTerminalTmuxControl {
        let directory = rustix::fs::open(
            &self.root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        NativeTerminalTmuxControl::new(
            executable.into(),
            ValidatedBackgroundEnvironment::new(Vec::new()).unwrap(),
            directory,
            &self.root.join("t.sock"),
            identity(),
        )
        .unwrap()
    }
}
impl Drop for SocketFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn native_argv_preserves_exact_bytes_and_fixed_socket_target_without_shell_interpolation() {
    let fixture = SocketFixture::new();
    let control = fixture.control("/bin/echo");
    let (args, input) = control
        .arguments(&TerminalTmuxCommand::Load(b"\0\xff\n'$(bad)'".to_vec()))
        .unwrap();
    assert_eq!(input, b"\0\xff\n'$(bad)'");
    assert_eq!(args[0], "-N");
    assert_eq!(args[1], "-S");
    assert!(args.contains(&OsString::from("/dev/null")));
    assert_eq!(
        &args[5..],
        &[
            OsString::from("load-buffer"),
            "-b".into(),
            format!("machine-god-{}", identity().namespace).into(),
            "-".into()
        ]
    );
    let (args, input) = control.arguments(&TerminalTmuxCommand::Paste).unwrap();
    assert!(input.is_empty());
    assert!(!args.contains(&"-r".into()));
    assert_eq!(args.last().unwrap(), "%7");
    let (args, _) = control
        .arguments(&TerminalTmuxCommand::KillSession)
        .unwrap();
    assert_eq!(
        args.last().unwrap(),
        &OsString::from(format!("={}", identity().session))
    );
    assert_eq!(NAMESPACE_OPTION, "@machine_god_terminal_namespace");
}

#[test]
fn socket_replacement_is_rejected_before_spawn_and_missing_server_is_not_restarted() {
    let fixture = SocketFixture::new();
    let mut control = fixture.control("/no-such-executable");
    std::fs::remove_file(fixture.root.join("t.sock")).unwrap();
    control
        .begin(
            TerminalTmuxCommand::HasSession,
            Instant::now() + COMMAND_TIMEOUT,
        )
        .unwrap();
    assert!(matches!(
        control.poll(),
        Poll::Ready(Ok(TerminalTmuxReply { success: false, .. }))
    ));
    assert_eq!(
        control.begin(TerminalTmuxCommand::Paste, Instant::now() + COMMAND_TIMEOUT),
        Err(TerminalTmuxError::Identity)
    );
    let _replacement = UnixListener::bind(fixture.root.join("t.sock")).unwrap();
    std::fs::set_permissions(
        fixture.root.join("t.sock"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert_eq!(
        control.begin(
            TerminalTmuxCommand::Inspect,
            Instant::now() + COMMAND_TIMEOUT
        ),
        Err(TerminalTmuxError::Identity)
    );
    assert!(control.pending.is_none());
    let _ = fixture.listener.local_addr().unwrap();
}

fn collect(process: &mut CommandProcess) -> Result<TerminalTmuxReply> {
    loop {
        match process.poll() {
            Poll::Ready(result) => return result,
            Poll::Pending => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

/// A foreground, directly owned server on a new private socket. Tests never
/// address the ambient/default tmux server, even during panic cleanup.
struct RealServer {
    root: PathBuf,
    executable: PathBuf,
    child: Child,
}
impl RealServer {
    fn start() -> Option<Self> {
        let executable = if let Some(path) = std::env::var_os("MACHINE_GOD_TERMINAL_TMUX_BINARY") {
            let path = PathBuf::from(path);
            assert!(
                path.is_absolute() && path.is_file(),
                "explicit tmux test executable is unavailable"
            );
            Some(path)
        } else {
            [
                "/opt/homebrew/bin/tmux",
                "/usr/bin/tmux",
                "/usr/local/bin/tmux",
            ]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
        };
        let Some(executable) = executable else {
            eprintln!("real tmux test skipped: no tmux executable installed");
            return None;
        };
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let root =
            PathBuf::from("/tmp").join(format!("mg-real-tmux-{:x}", u64::from_ne_bytes(random)));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let child = Command::new(&executable)
            .args(["-D", "-S"])
            .arg(root.join("t.sock"))
            .args(["-f", "/dev/null"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "xterm-256color")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut server = Self {
            root,
            executable,
            child,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !server.root.join("t.sock").exists() {
            assert!(
                server.child.try_wait().unwrap().is_none(),
                "private server exited before readiness"
            );
            assert!(
                Instant::now() < deadline,
                "private server readiness timeout"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        Some(server)
    }
    fn command(&self, arguments: &[&str]) -> TerminalTmuxReply {
        let mut command = Command::new(&self.executable);
        command
            .args(["-N", "-S"])
            .arg(self.root.join("t.sock"))
            .args(["-f", "/dev/null"])
            .args(arguments)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "xterm-256color");
        let mut process = CommandProcess::spawn(
            command,
            Vec::new(),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
        let result = collect(&mut process);
        process.abort().unwrap();
        result.unwrap()
    }
    fn control(&self, id: TerminalTmuxIdentity) -> NativeTerminalTmuxControl {
        NativeTerminalTmuxControl::new(
            self.executable.clone(),
            ValidatedBackgroundEnvironment::new(vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("TERM".into(), "xterm-256color".into()),
            ])
            .unwrap(),
            rustix::fs::open(
                &self.root,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .unwrap(),
            &self.root.join("t.sock"),
            id,
        )
        .unwrap()
    }
}
impl Drop for RealServer {
    fn drop(&mut self) {
        // The directly retained foreground server identity is never rebuilt
        // from a recorded PID. It owns only this fixture's unique namespace.
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "One owned private server covers the complete command adapter lifecycle."
)]
fn real_private_tmux_controller_writes_pastes_resizes_observes_and_closes() {
    let Some(mut server) = RealServer::start() else {
        return;
    };
    let namespace = identity().namespace;
    let session = format!("machine-god-{namespace}");
    let reply = server.command(&[
        "new-session",
        "-d",
        "-s",
        &session,
        "-x",
        "80",
        "-y",
        "24",
        "-P",
        "-F",
        "#{pane_id}|#{pane_pid}",
        "/bin/sh",
        "-c",
        r"stty -echo -icanon; printf '\033[?2004hREADY'; exec /bin/cat -v",
    ]);
    assert!(reply.success);
    let created = String::from_utf8(reply.output).unwrap();
    let (pane, pid) = created.trim_end().split_once('|').unwrap();
    let id = TerminalTmuxIdentity::new(
        namespace,
        pane.to_owned(),
        NonZeroU32::new(pid.parse().unwrap()).unwrap(),
    )
    .unwrap();
    assert!(
        server
            .command(&[
                "set-option",
                "-t",
                &session,
                NAMESPACE_OPTION,
                id.namespace()
            ])
            .success
    );
    assert!(
        server
            .command(&["set-option", "-t", &session, "remain-on-exit", "on"])
            .success
    );
    let mut control = server.control(id.clone());
    let invoke = |control: &mut NativeTerminalTmuxControl, command| {
        success(run(control, command, Instant::now() + Duration::from_secs(10)).unwrap()).unwrap()
    };
    assert!(compatible_version(&invoke(
        &mut control,
        TerminalTmuxCommand::Probe
    )));
    let initial = invoke(&mut control, TerminalTmuxCommand::Inspect);
    assert_eq!(
        inspect(&id, &initial, || Ok(None)).unwrap().status,
        TerminalPtyStatus::Running
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let screen = server.command(&["capture-pane", "-p", "-t", id.pane()]);
        assert!(screen.success);
        if screen.output.windows(5).any(|bytes| bytes == b"READY") {
            break;
        }
        assert!(Instant::now() < deadline, "pane readiness timeout");
        std::thread::sleep(Duration::from_millis(2));
    }
    // Full binary input reaches exactly one tmux buffer, with each native
    // pipe write independently bounded. No payload becomes command argv.
    let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(MAX_INPUT_BYTES).collect();
    invoke(&mut control, TerminalTmuxCommand::Load(bytes.clone()));
    let buffer = format!("machine-god-{}", id.namespace());
    let loaded = server.command(&["save-buffer", "-b", &buffer, "-"]);
    assert!(loaded.success);
    assert_eq!(loaded.output, bytes);
    invoke(&mut control, TerminalTmuxCommand::DeleteBuffer);
    invoke(
        &mut control,
        TerminalTmuxCommand::Load(b"ordinary-write".to_vec()),
    );
    invoke(&mut control, TerminalTmuxCommand::Paste);
    invoke(
        &mut control,
        TerminalTmuxCommand::Load(b"bracketed-paste".to_vec()),
    );
    invoke(&mut control, TerminalTmuxCommand::PasteBracketed);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let screen = server.command(&["capture-pane", "-p", "-t", id.pane()]);
        let screen = String::from_utf8(screen.output).unwrap();
        if screen.contains("ordinary-write^[[200~bracketed-paste^[[201~") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "paste was not delivered exactly once: {screen:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let dimensions = TerminalDimensions::new(40, 120).unwrap();
    invoke(
        &mut control,
        TerminalTmuxCommand::Resize(dimensions.clone()),
    );
    assert_eq!(
        inspect(
            &id,
            &invoke(&mut control, TerminalTmuxCommand::Inspect),
            || Ok(None)
        )
        .unwrap()
        .dimensions,
        dimensions
    );
    invoke(&mut control, TerminalTmuxCommand::KillSession);
    assert!(
        !run(
            &mut control,
            TerminalTmuxCommand::HasSession,
            Instant::now() + Duration::from_secs(10)
        )
        .unwrap()
        .success
    );
    assert!(server.command(&["kill-server"]).success);
    let deadline = Instant::now() + Duration::from_secs(10);
    while server.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "private server did not exit");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(server.child.wait().unwrap().success());
}
#[test]
fn owned_command_io_roundtrips_the_full_binary_input_and_reaps() {
    let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(MAX_INPUT_BYTES).collect();
    let mut command = Command::new("/bin/cat");
    command.env_clear();
    let mut process =
        CommandProcess::spawn(command, bytes.clone(), Instant::now() + COMMAND_TIMEOUT).unwrap();
    assert!(matches!(process.poll(), Poll::Pending));
    assert!(process.input_offset <= MAX_BACKGROUND_INPUT_BYTES);
    let reply = collect(&mut process).unwrap();
    assert!(reply.success);
    assert_eq!(reply.output, bytes);
    assert!(process.reaped);
    assert!(process.child.try_wait().unwrap().is_some());
}

#[test]
fn owned_command_bounds_output_stderr_time_and_collects_every_child() {
    for (script, expected, duration) in [
        (
            "exec /usr/bin/yes x",
            TerminalTmuxError::Capacity,
            COMMAND_TIMEOUT,
        ),
        (
            "exec /usr/bin/yes x >&2",
            TerminalTmuxError::Capacity,
            COMMAND_TIMEOUT,
        ),
        (
            "while :; do :; done",
            TerminalTmuxError::Timeout,
            Duration::from_millis(50),
        ),
    ] {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]).env_clear();
        let mut process =
            CommandProcess::spawn(command, Vec::new(), Instant::now() + duration).unwrap();
        assert!(matches!(collect(&mut process), Err(error) if error == expected));
        process.abort().unwrap();
        assert!(process.reaped);
        assert!(process.child.try_wait().unwrap().is_some());
        assert!(process.output_bytes.len() <= MAX_COMMAND_OUTPUT);
    }
}
