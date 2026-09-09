use super::*;
use crate::test_support::*;
use crate::{INVALID_ARGUMENTS, OUTPUT_FAILURE, run_with_hosts};
use machine_god_core::AvailableModel;
#[cfg(unix)]
use machine_god_core::BoxFuture;
use std::cell::Cell;
#[cfg(not(target_family = "wasm"))]
use std::cell::RefCell;
use std::ffi::OsString;
#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::{
    future::Future,
    io::Write as _,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
#[derive(Clone, Debug)]
struct FakeModelsHost {
    result: Result<ModelCatalog, ModelsOperationalFailure>,
    calls: Cell<usize>,
}

impl FakeModelsHost {
    fn new(result: Result<ModelCatalog, ModelsOperationalFailure>) -> Self {
        Self {
            result,
            calls: Cell::new(0),
        }
    }
}

impl ModelsCommandHost for FakeModelsHost {
    fn list_models(&self) -> ModelsCommandExecution {
        self.calls.set(self.calls.get() + 1);
        ModelsCommandExecution::without_signal_guard(self.result.clone())
    }
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompositionEffect {
    LoadConfig,
    DiscoverCredential,
    CreateTransportAndList,
}

#[cfg(not(target_family = "wasm"))]
#[derive(Debug)]
struct FakeModelsCompositionEffects {
    trace: RefCell<Vec<CompositionEffect>>,
    config_result: Result<(), ModelsOperationalFailure>,
    credential_result: Result<(), ModelsOperationalFailure>,
    catalog_result: Result<ModelCatalog, ModelsOperationalFailure>,
}

#[cfg(not(target_family = "wasm"))]
impl ModelsCompositionEffects for FakeModelsCompositionEffects {
    type Credential = ();

    fn load_and_validate_config(&self) -> Result<(), ModelsOperationalFailure> {
        self.trace.borrow_mut().push(CompositionEffect::LoadConfig);
        self.config_result
    }

    fn discover_credential(&self) -> Result<Self::Credential, ModelsOperationalFailure> {
        self.trace
            .borrow_mut()
            .push(CompositionEffect::DiscoverCredential);
        self.credential_result
    }

    fn create_transport_and_list(&self, (): Self::Credential) -> ModelsCommandExecution {
        self.trace
            .borrow_mut()
            .push(CompositionEffect::CreateTransportAndList);
        ModelsCommandExecution::without_signal_guard(self.catalog_result.clone())
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SignalTrace {
    RegisterInterrupt,
    RegisterTerminate,
    PollInterrupt,
    PollTerminate,
    CreateProviderFuture,
    PollProviderFuture,
    DropInterrupt,
    DropTerminate,
    DropProviderFuture,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScriptedSignal {
    Pending,
    ReadyOnPoll {
        poll: usize,
        event: ModelsSignalEvent,
    },
}

#[cfg(unix)]
struct ScriptedSignalFuture {
    trace: Arc<Mutex<Vec<SignalTrace>>>,
    poll_event: SignalTrace,
    drop_event: SignalTrace,
    script: ScriptedSignal,
    polls: usize,
}

#[cfg(unix)]
impl Future for ScriptedSignalFuture {
    type Output = ModelsSignalEvent;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls += 1;
        self.trace.lock().unwrap().push(self.poll_event);
        match self.script {
            ScriptedSignal::ReadyOnPoll { poll, event } if self.polls >= poll => Poll::Ready(event),
            ScriptedSignal::Pending | ScriptedSignal::ReadyOnPoll { .. } => Poll::Pending,
        }
    }
}

#[cfg(unix)]
impl Drop for ScriptedSignalFuture {
    fn drop(&mut self) {
        self.trace.lock().unwrap().push(self.drop_event);
    }
}

#[cfg(unix)]
struct FakeModelsSignalSource {
    trace: Arc<Mutex<Vec<SignalTrace>>>,
    interrupt: ScriptedSignalFuture,
    terminate: ScriptedSignalFuture,
    fail_interrupt_registration: bool,
    fail_terminate_registration: bool,
}

#[cfg(unix)]
impl FakeModelsSignalSource {
    fn new(trace: Arc<Mutex<Vec<SignalTrace>>>, interrupt: ScriptedSignal) -> Self {
        Self {
            interrupt: ScriptedSignalFuture {
                trace: Arc::clone(&trace),
                poll_event: SignalTrace::PollInterrupt,
                drop_event: SignalTrace::DropInterrupt,
                script: interrupt,
                polls: 0,
            },
            terminate: ScriptedSignalFuture {
                trace: Arc::clone(&trace),
                poll_event: SignalTrace::PollTerminate,
                drop_event: SignalTrace::DropTerminate,
                script: ScriptedSignal::Pending,
                polls: 0,
            },
            trace,
            fail_interrupt_registration: false,
            fail_terminate_registration: false,
        }
    }
}

#[cfg(unix)]
impl ModelsSignalSource for FakeModelsSignalSource {
    fn registration_failed(&self) -> bool {
        self.trace
            .lock()
            .unwrap()
            .push(SignalTrace::RegisterInterrupt);
        self.trace
            .lock()
            .unwrap()
            .push(SignalTrace::RegisterTerminate);
        self.fail_interrupt_registration || self.fail_terminate_registration
    }

    fn poll_interrupt(&mut self, context: &mut Context<'_>) -> Poll<ModelsSignalEvent> {
        Pin::new(&mut self.interrupt).poll(context)
    }

    fn poll_terminate(&mut self, context: &mut Context<'_>) -> Poll<ModelsSignalEvent> {
        Pin::new(&mut self.terminate).poll(context)
    }
}

#[cfg(unix)]
#[derive(Clone, Debug)]
enum FakeProviderResult {
    Ready(Result<ModelCatalog, ProviderError>),
}

#[cfg(unix)]
#[derive(Clone, Debug)]
struct FakeSignalProvider {
    trace: Arc<Mutex<Vec<SignalTrace>>>,
    result: FakeProviderResult,
    cancelled_when_dropped: Arc<Mutex<Option<bool>>>,
}

#[cfg(unix)]
struct FakeProviderFuture {
    trace: Arc<Mutex<Vec<SignalTrace>>>,
    result: FakeProviderResult,
    cancellation: CancellationToken,
    cancelled_when_dropped: Arc<Mutex<Option<bool>>>,
}

#[cfg(unix)]
impl Future for FakeProviderFuture {
    type Output = Result<ModelCatalog, ProviderError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.trace
            .lock()
            .unwrap()
            .push(SignalTrace::PollProviderFuture);
        match &self.result {
            FakeProviderResult::Ready(result) => Poll::Ready(result.clone()),
        }
    }
}

#[cfg(unix)]
impl Drop for FakeProviderFuture {
    fn drop(&mut self) {
        *self.cancelled_when_dropped.lock().unwrap() = Some(self.cancellation.is_cancelled());
        self.trace
            .lock()
            .unwrap()
            .push(SignalTrace::DropProviderFuture);
    }
}

#[cfg(unix)]
impl ModelCatalogProvider for FakeSignalProvider {
    fn name(&self) -> &'static str {
        "fake-signal-provider"
    }

    fn list_models(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>> {
        self.trace
            .lock()
            .unwrap()
            .push(SignalTrace::CreateProviderFuture);
        Box::pin(FakeProviderFuture {
            trace: Arc::clone(&self.trace),
            result: self.result.clone(),
            cancellation,
            cancelled_when_dropped: Arc::clone(&self.cancelled_when_dropped),
        })
    }
}

fn catalog(ids: &[&str], access: ModelCatalogAccess) -> ModelCatalog {
    ModelCatalog::new(
        ids.iter()
            .map(|id| AvailableModel::new(*id).expect("valid model ID"))
            .collect(),
        access,
    )
}

#[cfg(not(target_family = "wasm"))]
fn composition_effects(
    config_result: Result<(), ModelsOperationalFailure>,
    credential_result: Result<(), ModelsOperationalFailure>,
) -> FakeModelsCompositionEffects {
    FakeModelsCompositionEffects {
        trace: RefCell::new(Vec::new()),
        config_result,
        credential_result,
        catalog_result: Ok(catalog(
            &["provider/model"],
            ModelCatalogAccess::Authenticated,
        )),
    }
}

#[cfg(unix)]
fn signal_provider(
    trace: &Arc<Mutex<Vec<SignalTrace>>>,
    result: FakeProviderResult,
) -> FakeSignalProvider {
    FakeSignalProvider {
        trace: Arc::clone(trace),
        result,
        cancelled_when_dropped: Arc::new(Mutex::new(None)),
    }
}

#[cfg(unix)]
fn run_signal_coordination(
    provider: &FakeSignalProvider,
    signals: FakeModelsSignalSource,
) -> Result<ModelCatalog, ProviderError> {
    let phase = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime")
        .block_on(list_models_with_signal_source(provider, signals));
    phase.result
}

#[cfg(unix)]
fn coordination_trace(trace: &Arc<Mutex<Vec<SignalTrace>>>) -> Vec<SignalTrace> {
    trace
        .lock()
        .unwrap()
        .iter()
        .copied()
        .filter(|event| {
            !matches!(
                event,
                SignalTrace::DropInterrupt
                    | SignalTrace::DropTerminate
                    | SignalTrace::DropProviderFuture
            )
        })
        .collect()
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn models_composition_orders_each_effect_once_and_short_circuits_failures() {
    let success = composition_effects(Ok(()), Ok(()));
    assert!(list_models_with_effects(&success).result().is_ok());
    assert_eq!(
        *success.trace.borrow(),
        [
            CompositionEffect::LoadConfig,
            CompositionEffect::DiscoverCredential,
            CompositionEffect::CreateTransportAndList,
        ]
    );

    let config_failure = composition_effects(
        Err(ModelsOperationalFailure::Unavailable),
        Err(ModelsOperationalFailure::Unavailable),
    );
    assert_eq!(
        list_models_with_effects(&config_failure).result(),
        &Err(ModelsOperationalFailure::Unavailable)
    );
    assert_eq!(
        *config_failure.trace.borrow(),
        [CompositionEffect::LoadConfig]
    );

    let credential_failure =
        composition_effects(Ok(()), Err(ModelsOperationalFailure::Unavailable));
    assert_eq!(
        list_models_with_effects(&credential_failure).result(),
        &Err(ModelsOperationalFailure::Unavailable)
    );
    assert_eq!(
        *credential_failure.trace.borrow(),
        [
            CompositionEffect::LoadConfig,
            CompositionEffect::DiscoverCredential,
        ]
    );
}

#[cfg(unix)]
#[test]
fn model_signal_listeners_are_registered_and_polled_before_provider_dispatch() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let provider = signal_provider(
        &trace,
        FakeProviderResult::Ready(Ok(catalog(
            &["provider/model"],
            ModelCatalogAccess::Authenticated,
        ))),
    );
    let signals = FakeModelsSignalSource::new(Arc::clone(&trace), ScriptedSignal::Pending);

    assert!(run_signal_coordination(&provider, signals).is_ok());
    assert_eq!(
        coordination_trace(&trace),
        [
            SignalTrace::RegisterInterrupt,
            SignalTrace::RegisterTerminate,
            SignalTrace::PollInterrupt,
            SignalTrace::PollTerminate,
            SignalTrace::CreateProviderFuture,
            SignalTrace::PollInterrupt,
            SignalTrace::PollTerminate,
            SignalTrace::PollProviderFuture,
            SignalTrace::PollInterrupt,
            SignalTrace::PollTerminate,
        ]
    );
    let trace = trace.lock().unwrap();
    assert_eq!(
        trace
            .iter()
            .filter(|event| **event == SignalTrace::DropInterrupt)
            .count(),
        1
    );
    assert_eq!(
        trace
            .iter()
            .filter(|event| **event == SignalTrace::DropTerminate)
            .count(),
        1
    );
    assert_eq!(
        trace
            .iter()
            .filter(|event| **event == SignalTrace::DropProviderFuture)
            .count(),
        1
    );
    assert_eq!(
        *provider.cancelled_when_dropped.lock().unwrap(),
        Some(false)
    );
}

#[cfg(unix)]
#[test]
fn ready_signal_wins_same_poll_provider_success_and_drops_cancelled_provider() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let provider = signal_provider(
        &trace,
        FakeProviderResult::Ready(Ok(catalog(
            &["provider/model"],
            ModelCatalogAccess::Authenticated,
        ))),
    );
    let signals = FakeModelsSignalSource::new(
        Arc::clone(&trace),
        ScriptedSignal::ReadyOnPoll {
            poll: 3,
            event: ModelsSignalEvent::Received(ModelsSignalKind::Interrupt),
        },
    );

    let error = run_signal_coordination(&provider, signals).unwrap_err();

    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    assert_eq!(error.code, "Cancelled");
    assert_eq!(
        coordination_trace(&trace),
        [
            SignalTrace::RegisterInterrupt,
            SignalTrace::RegisterTerminate,
            SignalTrace::PollInterrupt,
            SignalTrace::PollTerminate,
            SignalTrace::CreateProviderFuture,
            SignalTrace::PollInterrupt,
            SignalTrace::PollTerminate,
            SignalTrace::PollProviderFuture,
            SignalTrace::PollInterrupt,
        ]
    );
    assert_eq!(*provider.cancelled_when_dropped.lock().unwrap(), Some(true));
}

#[cfg(unix)]
#[test]
fn signal_registration_and_wait_failures_are_authoritative() {
    let registration_trace = Arc::new(Mutex::new(Vec::new()));
    let registration_provider = signal_provider(
        &registration_trace,
        FakeProviderResult::Ready(Ok(catalog(
            &["provider/model"],
            ModelCatalogAccess::Authenticated,
        ))),
    );
    let mut registration_signals =
        FakeModelsSignalSource::new(Arc::clone(&registration_trace), ScriptedSignal::Pending);
    registration_signals.fail_terminate_registration = true;

    let registration_error =
        run_signal_coordination(&registration_provider, registration_signals).unwrap_err();
    assert_eq!(registration_error.kind, ProviderErrorKind::Unavailable);
    assert_eq!(registration_error.code, "SignalUnavailable");
    assert_eq!(
        coordination_trace(&registration_trace),
        [
            SignalTrace::RegisterInterrupt,
            SignalTrace::RegisterTerminate,
        ]
    );
    assert_eq!(
        *registration_provider.cancelled_when_dropped.lock().unwrap(),
        None
    );

    let wait_trace = Arc::new(Mutex::new(Vec::new()));
    let wait_provider = signal_provider(
        &wait_trace,
        FakeProviderResult::Ready(Ok(catalog(
            &["provider/model"],
            ModelCatalogAccess::Authenticated,
        ))),
    );
    let mut wait_signals =
        FakeModelsSignalSource::new(Arc::clone(&wait_trace), ScriptedSignal::Pending);
    wait_signals.terminate.script = ScriptedSignal::ReadyOnPoll {
        poll: 3,
        event: ModelsSignalEvent::WaitFailed,
    };

    let wait_error = run_signal_coordination(&wait_provider, wait_signals).unwrap_err();
    assert_eq!(wait_error.kind, ProviderErrorKind::Unavailable);
    assert_eq!(wait_error.code, "SignalUnavailable");
    assert_eq!(
        coordination_trace(&wait_trace),
        [
            SignalTrace::RegisterInterrupt,
            SignalTrace::RegisterTerminate,
            SignalTrace::PollInterrupt,
            SignalTrace::PollTerminate,
            SignalTrace::CreateProviderFuture,
            SignalTrace::PollInterrupt,
            SignalTrace::PollTerminate,
            SignalTrace::PollProviderFuture,
            SignalTrace::PollInterrupt,
            SignalTrace::PollTerminate,
        ]
    );
    assert_eq!(
        *wait_provider.cancelled_when_dropped.lock().unwrap(),
        Some(true)
    );
    assert_eq!(
        terminate_signal_event(Some(())),
        ModelsSignalEvent::Received(ModelsSignalKind::Terminate)
    );
    assert_eq!(terminate_signal_event(None), ModelsSignalEvent::WaitFailed);
}

#[cfg(unix)]
struct SignalOutputChildProvider;

#[cfg(unix)]
struct SignalOutputChildFuture {
    cancellation: CancellationToken,
    announced: bool,
}

#[cfg(unix)]
impl Future for SignalOutputChildFuture {
    type Output = Result<ModelCatalog, ProviderError>;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.announced {
            self.announced = true;
            let mut stderr = io::stderr().lock();
            stderr.write_all(b"PROVIDER_READY\n").unwrap();
            stderr.flush().unwrap();
        }
        Poll::Pending
    }
}

#[cfg(unix)]
impl Drop for SignalOutputChildFuture {
    fn drop(&mut self) {
        let marker = if self.cancellation.is_cancelled() {
            b"PROVIDER_DROPPED_CANCELLED\n".as_slice()
        } else {
            b"PROVIDER_DROPPED_WITHOUT_CANCELLATION\n".as_slice()
        };
        let mut stderr = io::stderr().lock();
        let _ = stderr.write_all(marker);
        let _ = stderr.flush();
    }
}

#[cfg(unix)]
impl ModelCatalogProvider for SignalOutputChildProvider {
    fn name(&self) -> &'static str {
        "signal-output-child"
    }

    fn list_models(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>> {
        Box::pin(SignalOutputChildFuture {
            cancellation,
            announced: false,
        })
    }
}

#[cfg(unix)]
struct SignalOutputReadyChildProvider;

#[cfg(unix)]
impl ModelCatalogProvider for SignalOutputReadyChildProvider {
    fn name(&self) -> &'static str {
        "signal-output-ready-child"
    }

    fn list_models(
        &self,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>> {
        Box::pin(async { Ok(catalog(&[], ModelCatalogAccess::Authenticated)) })
    }
}

#[cfg(unix)]
struct SingleExecutionHost {
    execution: RefCell<Option<ModelsCommandExecution>>,
}

#[cfg(unix)]
impl ModelsCommandHost for SingleExecutionHost {
    fn list_models(&self) -> ModelsCommandExecution {
        self.execution
            .borrow_mut()
            .take()
            .expect("child execution is consumed once")
    }
}

#[cfg(unix)]
struct OutputStartWriter {
    stdout: io::Stdout,
    announced: bool,
}

#[cfg(unix)]
impl io::Write for OutputStartWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if !self.announced {
            self.announced = true;
            let mut stderr = io::stderr().lock();
            stderr.write_all(b"OUTPUT_START\n")?;
            stderr.flush()?;
        }
        self.stdout.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stdout.flush()
    }
}

#[cfg(unix)]
pub(crate) fn models_signal_output_subprocess_child() {
    let Some(mode) = std::env::var_os("MACHINE_GOD_MODELS_SIGNAL_OUTPUT_CHILD") else {
        return;
    };
    if mode == "wait-failed" {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();
        let pending_guardian = PendingModelsSignalGuardian::spawn().unwrap();
        let signals = {
            let _entered = runtime.enter();
            TokioModelsSignalSource::register()
        };
        let _guard = pending_guardian.activate(runtime, signals, true);
        panic!("wait-failed guardian activation must terminate the process");
    }

    let execution = if mode == "stop-drain" {
        list_models_with_signals(&SignalOutputReadyChildProvider)
    } else {
        list_models_with_signals(&SignalOutputChildProvider)
    };
    let host = SingleExecutionHost {
        execution: RefCell::new(Some(execution)),
    };
    let mut stdout = OutputStartWriter {
        stdout: io::stdout(),
        announced: false,
    };
    let mut stderr = io::stderr();
    let code = run_models(&host, true, &mut stdout, &mut stderr);
    std::process::exit(i32::from(code));
}

#[cfg(unix)]
fn send_process_signal(process_id: u32, signal: &str) {
    let status = std::process::Command::new("/bin/kill")
        .args([signal, &process_id.to_string()])
        .status()
        .expect("invoke /bin/kill");
    assert!(status.success(), "failed to send {signal} to {process_id}");
}

#[cfg(unix)]
fn wait_for_child_marker(
    receiver: &std::sync::mpsc::Receiver<String>,
    expected: &str,
    process_id: u32,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let line = match receiver.recv_timeout(remaining) {
            Ok(line) => line,
            Err(error) => {
                send_process_signal(process_id, "-KILL");
                panic!("child did not emit {expected:?}: {error}");
            }
        };
        if line == expected {
            return;
        }
    }
}

#[cfg(unix)]
fn assert_saturated_output_terminates_on(second_signal: &str, expected_exit: i32) {
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixStream;
    use std::process::Stdio;

    let (unread_stdout, child_stdout) = UnixStream::pair().expect("stdout socket pair");
    let mut filler = child_stdout.try_clone().expect("clone stdout sender");
    let child_stdout = std::os::fd::OwnedFd::from(child_stdout);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::models_signal_output_subprocess_child",
            "--nocapture",
        ])
        .env("MACHINE_GOD_MODELS_SIGNAL_OUTPUT_CHILD", "1")
        .stdout(Stdio::from(child_stdout))
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn signal-output child");
    let process_id = child.id();
    let child_stderr = child.stderr.take().expect("child stderr");
    let (marker_sender, marker_receiver) = std::sync::mpsc::channel();
    let marker_worker = std::thread::spawn(move || {
        for line in io::BufReader::new(child_stderr).lines() {
            match line {
                Ok(line) => {
                    if marker_sender.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    wait_for_child_marker(&marker_receiver, "PROVIDER_READY", process_id);
    filler.set_nonblocking(true).unwrap();
    let block = [b'x'; 8 * 1024];
    let mut filled_bytes = 0_usize;
    loop {
        match filler.write(&block) {
            Ok(0) => panic!("stdout socket stopped accepting bytes without saturation"),
            Ok(count) => filled_bytes += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) => panic!("failed to saturate stdout socket: {error}"),
        }
    }
    assert!(filled_bytes > 0);
    filler.set_nonblocking(false).unwrap();

    send_process_signal(process_id, "-INT");
    wait_for_child_marker(&marker_receiver, "PROVIDER_DROPPED_CANCELLED", process_id);
    wait_for_child_marker(&marker_receiver, "OUTPUT_START", process_id);
    send_process_signal(process_id, second_signal);

    let (status_sender, status_receiver) = std::sync::mpsc::sync_channel(0);
    let wait_worker = std::thread::spawn(move || {
        let _ = status_sender.send(child.wait());
    });
    let status = match status_receiver.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => panic!("failed to wait for signal-output child: {error}"),
        Err(error) => {
            send_process_signal(process_id, "-KILL");
            let _ = status_receiver.recv_timeout(Duration::from_secs(2));
            panic!("signal-output child required SIGKILL cleanup: {error}");
        }
    };
    wait_worker.join().unwrap();
    marker_worker.join().unwrap();
    drop(filler);
    drop(unread_stdout);
    assert_eq!(status.code(), Some(expected_exit));
}

#[cfg(unix)]
#[test]
fn repeated_signal_terminates_while_models_json_output_is_backpressured() {
    assert_saturated_output_terminates_on("-INT", 130);
    assert_saturated_output_terminates_on("-TERM", 143);
}

#[cfg(unix)]
#[test]
fn fast_output_stop_drain_observes_signal_before_guardian_shutdown() {
    use std::io::{BufRead, Write};
    use std::process::Stdio;

    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::models_signal_output_subprocess_child",
            "--nocapture",
        ])
        .env("MACHINE_GOD_MODELS_SIGNAL_OUTPUT_CHILD", "stop-drain")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn signal stop-drain child");
    let process_id = child.id();
    let child_stderr = child.stderr.take().expect("child stderr");
    let (marker_sender, marker_receiver) = std::sync::mpsc::channel();
    let marker_worker = std::thread::spawn(move || {
        for line in io::BufReader::new(child_stderr).lines() {
            match line {
                Ok(line) => {
                    if marker_sender.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    wait_for_child_marker(&marker_receiver, "GUARDIAN_STOP_READY", process_id);
    send_process_signal(process_id, "-INT");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"x")
        .expect("release guardian stop drain");

    let (status_sender, status_receiver) = std::sync::mpsc::sync_channel(0);
    let wait_worker = std::thread::spawn(move || {
        let _ = status_sender.send(child.wait());
    });
    let status = match status_receiver.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => panic!("failed to wait for signal stop-drain child: {error}"),
        Err(error) => {
            send_process_signal(process_id, "-KILL");
            let _ = status_receiver.recv_timeout(Duration::from_secs(2));
            panic!("signal stop-drain child required SIGKILL cleanup: {error}");
        }
    };
    wait_worker.join().unwrap();
    marker_worker.join().unwrap();
    assert_eq!(status.code(), Some(130));
}

#[cfg(unix)]
#[test]
fn signal_wait_failure_fail_stops_before_output() {
    use std::process::Stdio;

    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::models_signal_output_subprocess_child",
            "--nocapture",
        ])
        .env("MACHINE_GOD_MODELS_SIGNAL_OUTPUT_CHILD", "wait-failed")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn signal wait-failure child");
    let process_id = child.id();
    let (status_sender, status_receiver) = std::sync::mpsc::sync_channel(0);
    let wait_worker = std::thread::spawn(move || {
        let _ = status_sender.send(child.wait());
    });
    let status = match status_receiver.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => panic!("failed to wait for wait-failure child: {error}"),
        Err(error) => {
            send_process_signal(process_id, "-KILL");
            let _ = status_receiver.recv_timeout(Duration::from_secs(2));
            panic!("wait-failure child required SIGKILL cleanup: {error}");
        }
    };
    wait_worker.join().unwrap();
    assert_eq!(status.code(), Some(1));
}

#[test]
fn models_human_output_is_exact_for_all_access_modes_and_empty_catalogs() {
    let cases = [
        (
            catalog(
                &["anthropic/claude-opus", "openai/gpt-5"],
                ModelCatalogAccess::Authenticated,
            ),
            concat!(
                "[models] 2 available\n",
                " - anthropic/claude-opus\n",
                " - openai/gpt-5\n",
            ),
        ),
        (
            catalog(
                &["public/model"],
                ModelCatalogAccess::PublicOnly {
                    reason: PublicCatalogReason::NoCredential,
                },
            ),
            concat!(
                "[models] 1 available\n",
                " - public/model\n",
                "[models] Using the public model catalog; set VERCEL_OIDC_TOKEN or ",
                "AI_GATEWAY_API_KEY to include private models.\n",
            ),
        ),
        (
            catalog(
                &[],
                ModelCatalogAccess::PublicOnly {
                    reason: PublicCatalogReason::AuthenticatedCredentialRejected,
                },
            ),
            concat!(
                "[models] no models returned by gateway\n",
                "[models] Gateway authentication was rejected; showing the public model catalog.\n",
            ),
        ),
    ];

    for (catalog, expected) in cases {
        let host = FakeModelsHost::new(Ok(catalog));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            [OsString::from("models")],
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                models: &host,
                ..Default::default()
            },
        );
        assert_eq!(exit, 0);
        assert_eq!(stdout, expected.as_bytes());
        assert!(stderr.is_empty());
        assert_eq!(host.calls.get(), 1);
    }
}

#[test]
fn models_preserve_unicode_and_escape_terminal_controls_in_both_formats() {
    let id = "provider/模型 v2\u{85}\u{202e}\u{2028}";
    for json in [false, true] {
        let host = FakeModelsHost::new(Ok(catalog(&[id], ModelCatalogAccess::Authenticated)));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut arguments = vec![OsString::from("models")];
        if json {
            arguments.push(OsString::from("--json"));
        }
        assert_eq!(
            run_with_hosts(
                arguments,
                &mut stdout,
                &mut stderr,
                crate::CommandHosts {
                    models: &host,
                    ..Default::default()
                }
            ),
            0
        );
        assert!(stderr.is_empty());
        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.contains("provider/模型 v2\\u0085\\u202e\\u2028"));
        assert!(!rendered.contains(['\u{85}', '\u{202e}', '\u{2028}']));
        if json {
            let decoded: serde_json::Value = serde_json::from_str(&rendered).unwrap();
            assert_eq!(decoded["ids"][0], id);
        }
    }
}

#[test]
fn models_json_output_has_exact_shape_order_escaping_and_lf() {
    let host = FakeModelsHost::new(Ok(catalog(
        &["provider/model", "quoted\"model\\id"],
        ModelCatalogAccess::PublicOnly {
            reason: PublicCatalogReason::NoCredential,
        },
    )));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let exit = run_with_hosts(
        [OsString::from("models"), OsString::from("--json")],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            models: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 0);
    assert_eq!(
        stdout,
        concat!(
            "{\"kind\":\"models\",\"count\":2,\"shown_count\":2,",
            "\"more_count\":0,\"private_models_hidden\":true,",
            "\"ids\":[\"provider/model\",\"quoted\\\"model\\\\id\"]}\n",
        )
        .as_bytes()
    );
    assert!(stderr.is_empty());
    assert_eq!(host.calls.get(), 1);

    let host = FakeModelsHost::new(Ok(catalog(&[], ModelCatalogAccess::Authenticated)));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = run_with_hosts(
        [OsString::from("models"), OsString::from("--json")],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            models: &host,
            ..Default::default()
        },
    );
    assert_eq!(exit, 0);
    assert_eq!(
        stdout,
        b"{\"kind\":\"models\",\"count\":0,\"shown_count\":0,\"more_count\":0,\"private_models_hidden\":false,\"ids\":[]}\n"
    );
    assert!(stderr.is_empty());
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn models_failures_use_exact_human_and_json_channels() {
    let cases = [
        (
            ModelsOperationalFailure::AuthenticationRejected,
            "AuthenticationRejected",
            "AuthenticationRejected",
        ),
        (
            ModelsOperationalFailure::Cancelled,
            "the request was cancelled",
            "Cancelled",
        ),
        (
            ModelsOperationalFailure::MalformedResponse,
            "MalformedResponse",
            "MalformedResponse",
        ),
        (
            ModelsOperationalFailure::ResourceLimit,
            "ResourceLimit",
            "ResourceLimit",
        ),
        (
            ModelsOperationalFailure::Unavailable,
            "Unavailable",
            "Unavailable",
        ),
    ];

    for (failure, detail, code) in cases {
        let host = FakeModelsHost::new(Err(failure));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            [OsString::from("models")],
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                models: &host,
                ..Default::default()
            },
        );
        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
        assert_eq!(
            stderr,
            format!("machine-god models: could not list models: {detail}\n").as_bytes()
        );

        let host = FakeModelsHost::new(Err(failure));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_hosts(
            [OsString::from("models"), OsString::from("--json")],
            &mut stdout,
            &mut stderr,
            crate::CommandHosts {
                models: &host,
                ..Default::default()
            },
        );
        assert_eq!(exit, 1);
        assert_eq!(
            stdout,
            format!(
                "{{\"kind\":\"models\",\"error\":\"could not list models: {detail}\",\"code\":\"{code}\"}}\n"
            )
            .as_bytes()
        );
        assert!(stderr.is_empty());
    }
}

#[test]
fn invalid_models_arguments_do_not_call_the_host() {
    let host = FakeModelsHost::new(Err(ModelsOperationalFailure::Unavailable));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = run_with_hosts(
        [OsString::from("models"), OsString::from("extra")],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            models: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 2);
    assert!(stdout.is_empty());
    assert_eq!(stderr, INVALID_ARGUMENTS.as_bytes());
    assert_eq!(host.calls.get(), 0);
}

#[test]
fn models_output_cap_fails_before_any_success_bytes_are_written() {
    let models = (0..600)
        .map(|index| {
            AvailableModel::new(format!("{index:03}{}", "a".repeat(125)))
                .expect("128-byte visible ASCII model ID")
        })
        .collect();
    let host = FakeModelsHost::new(Ok(ModelCatalog::new(
        models,
        ModelCatalogAccess::Authenticated,
    )));
    for (arguments, expected_stdout, expected_stderr) in [
        (
            vec![OsString::from("models")],
            &b""[..],
            &b"machine-god models: could not list models: ResourceLimit\n"[..],
        ),
        (
            vec![OsString::from("models"), OsString::from("--json")],
            &b"{\"kind\":\"models\",\"error\":\"could not list models: ResourceLimit\",\"code\":\"ResourceLimit\"}\n"[..],
            &b""[..],
        ),
    ] {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_with_hosts(arguments, &mut stdout, &mut stderr, crate::CommandHosts { models: &host, ..Default::default() });

        assert_eq!(exit, 1);
        assert_eq!(stdout, expected_stdout);
        assert_eq!(stderr, expected_stderr);
    }
}

#[test]
fn models_output_cap_is_inclusive() {
    let mut models = Vec::with_capacity(512);
    for _ in 0..511 {
        models.push(AvailableModel::new("a".repeat(124)).expect("valid model ID"));
    }
    models.push(AvailableModel::new("b".repeat(101)).expect("valid model ID"));
    let catalog = ModelCatalog::new(models, ModelCatalogAccess::Authenticated);

    let output = super::render_models(&catalog, false).expect("inclusive limit is accepted");

    assert_eq!(output.len(), super::MAX_MODELS_OUTPUT_BYTES);
}

#[test]
fn models_broken_stdout_uses_the_fixed_output_diagnostic() {
    let host = FakeModelsHost::new(Ok(catalog(
        &["provider/model"],
        ModelCatalogAccess::Authenticated,
    )));
    let mut stdout = BrokenWriter;
    let mut stderr = Vec::new();
    let exit = run_with_hosts(
        [OsString::from("models")],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            models: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 1);
    assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());

    let host = FakeModelsHost::new(Err(ModelsOperationalFailure::Unavailable));
    let mut stdout = BrokenWriter;
    let mut stderr = Vec::new();
    let exit = run_with_hosts(
        [OsString::from("models"), OsString::from("--json")],
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            models: &host,
            ..Default::default()
        },
    );

    assert_eq!(exit, 1);
    assert_eq!(stderr, OUTPUT_FAILURE.as_bytes());
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn provider_failures_are_mapped_without_reflecting_provider_diagnostics() {
    let cases = [
        (
            ProviderError::new(
                ProviderErrorKind::Authentication,
                "AuthenticationRejected",
                "secret-message",
                false,
            ),
            ModelsOperationalFailure::AuthenticationRejected,
        ),
        (
            ProviderError::new(
                ProviderErrorKind::Cancelled,
                "Cancelled",
                "secret-message",
                false,
            ),
            ModelsOperationalFailure::Cancelled,
        ),
        (
            ProviderError::new(
                ProviderErrorKind::Protocol,
                "MalformedResponse",
                "secret-message",
                false,
            ),
            ModelsOperationalFailure::MalformedResponse,
        ),
        (
            ProviderError::new(
                ProviderErrorKind::Other,
                "ResourceLimit",
                "secret-message",
                false,
            ),
            ModelsOperationalFailure::ResourceLimit,
        ),
        (
            ProviderError::new(
                ProviderErrorKind::Authentication,
                "secret-code",
                "secret-message",
                false,
            ),
            ModelsOperationalFailure::Unavailable,
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(classify_provider_error(&error), expected);
    }
    for code in [
        "RateLimited",
        "GatewayUnavailable",
        "Unavailable",
        "TransportFailure",
        "RuntimeRequired",
        "future-code",
    ] {
        let error = ProviderError::new(
            ProviderErrorKind::Authentication,
            code,
            "secret-message",
            false,
        );
        assert_eq!(
            classify_provider_error(&error),
            ModelsOperationalFailure::Unavailable
        );
    }
}
