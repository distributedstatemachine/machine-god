#![cfg(unix)]

use std::ffi::OsString;
use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use machine_god_core::{
    BackgroundOutputOwner, BackgroundStartError, BackgroundStartErrorKind, BackgroundStartRequest,
    BoxFuture, CancellationToken, Capability, MAX_BACKGROUND_CWD_BYTES, ProcessEnvironment, Tool,
    ToolError, ToolErrorKind, ToolOutput,
};
use machine_god_native::{
    MAX_TERMINAL_BACKGROUND_READ_BYTES, MAX_TERMINAL_COMMAND_BYTES,
    MAX_TERMINAL_CWD_COMPONENT_BYTES, MAX_TERMINAL_CWD_COMPONENTS,
    MAX_TERMINAL_PRODUCED_OUTPUT_BYTES,
};
use machine_god_native::{
    NativeBackgroundDetail, NativeBackgroundInspectionError, NativeBackgroundInspectionErrorKind,
    NativeBackgroundList, NativeBackgroundRecordSummary, NativeBackgroundState,
    TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE, TERMINAL_MAX_ACTIVE_LISTS, TerminalBackgroundCatalog,
    TerminalBackgroundInspector, TerminalBackgroundOutcome, TerminalBackgroundOutputReader,
    TerminalBackgroundReadError, TerminalBackgroundReadErrorKind, TerminalBackgroundReadSnapshot,
    TerminalBackgroundStarter, TerminalBackgroundWaitDelay, TerminalBackgroundWaitDelayError,
    TerminalCapturedOutput, TerminalConfigErrorKind, TerminalExecution, TerminalExecutionOutcome,
    TerminalExecutionRequest, TerminalExecutionStatus, TerminalExecutor, TerminalExecutorError,
    TerminalExecutorErrorKind, TerminalLimits, TerminalTool,
};
use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
use serde_json::{Value, json};

mod terminal_test_support;

use terminal_test_support::{TemporaryDirectory, call, context, poll_once, poll_ready};

const PRIVATE_ENVIRONMENT_KEY: &str = "MACHINE_GOD_TERMINAL_PRIVATE_KEY";
const PRIVATE_ENVIRONMENT_VALUE: &str = "PRIVATE_ENVIRONMENT_VALUE_DO_NOT_REFLECT";

#[derive(Clone, Copy)]
enum Mode {
    Exited(i32),
    Signaled(i32),
    TimedOut,
    OutputLimit,
    DelayedOutputLimit(Duration),
    OutputLimitAfterPending,
    Error(TerminalExecutorErrorKind),
    Pending,
    CancelThenExit,
    DropCancelThenExit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RequestRecord {
    program: String,
    arguments: Vec<String>,
    command: String,
    cwd: String,
    environment_profile: String,
    environment_sha256: String,
    environment: Vec<(OsString, OsString)>,
    deadline: Instant,
    directory_identity: String,
    debug: String,
}

#[derive(Default)]
struct State {
    calls: AtomicUsize,
    polls: AtomicUsize,
    drops: AtomicUsize,
    requests: Mutex<Vec<RequestRecord>>,
}

#[derive(Clone)]
struct FakeExecutor {
    mode: Mode,
    state: Arc<State>,
    stdout: Vec<u8>,
    stdout_total: usize,
    stderr: Vec<u8>,
    stderr_total: usize,
}

impl FakeExecutor {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            state: Arc::new(State::default()),
            stdout: Vec::new(),
            stdout_total: 0,
            stderr: Vec::new(),
            stderr_total: 0,
        }
    }

    fn with_output(mut self, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        self.stdout_total = stdout.len();
        self.stderr_total = stderr.len();
        self.stdout = stdout;
        self.stderr = stderr;
        self
    }

    fn with_totals(mut self, stdout_total: usize, stderr_total: usize) -> Self {
        self.stdout_total = stdout_total;
        self.stderr_total = stderr_total;
        self
    }

    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    fn polls(&self) -> usize {
        self.state.polls.load(Ordering::SeqCst)
    }

    fn drops(&self) -> usize {
        self.state.drops.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<RequestRecord> {
        self.state.requests.lock().unwrap().clone()
    }
}

impl TerminalExecutor for FakeExecutor {
    fn execute(
        &self,
        request: TerminalExecutionRequest,
        cancellation: CancellationToken,
    ) -> TerminalExecution {
        let directory = rustix::fs::fstat(request.directory_fd()).unwrap();
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        self.state.requests.lock().unwrap().push(RequestRecord {
            program: request.program().to_owned(),
            arguments: request.arguments().into_iter().map(str::to_owned).collect(),
            command: request.command().to_owned(),
            cwd: request.cwd().to_owned(),
            environment_profile: request.environment_profile().to_owned(),
            environment_sha256: request.environment_sha256().to_owned(),
            environment: request.environment().to_vec(),
            deadline: request.deadline(),
            directory_identity: format!("{}:{}", directory.st_dev, directory.st_ino),
            debug: format!("{request:?}"),
        });
        Box::pin(FakeExecution {
            mode: self.mode,
            cancellation,
            state: Arc::clone(&self.state),
            stdout: self.stdout.clone(),
            stdout_total: self.stdout_total,
            stderr: self.stderr.clone(),
            stderr_total: self.stderr_total,
        })
    }
}

#[derive(Clone, Copy)]
enum BackgroundMode {
    Success { cancel_before_return: bool },
    Error(BackgroundStartErrorKind),
}

#[derive(Default)]
struct BackgroundState {
    calls: AtomicUsize,
    requests: Mutex<Vec<(String, String, String)>>,
    owners: Mutex<Vec<Option<(String, String)>>>,
}

#[derive(Clone)]
struct FakeBackgroundStarter {
    mode: BackgroundMode,
    state: Arc<BackgroundState>,
}

impl FakeBackgroundStarter {
    fn new(mode: BackgroundMode) -> Self {
        Self {
            mode,
            state: Arc::new(BackgroundState::default()),
        }
    }

    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<(String, String, String)> {
        self.state.requests.lock().unwrap().clone()
    }

    fn owners(&self) -> Vec<Option<(String, String)>> {
        self.state.owners.lock().unwrap().clone()
    }
}

impl TerminalBackgroundStarter for FakeBackgroundStarter {
    fn start(
        &self,
        request: BackgroundStartRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalBackgroundOutcome, BackgroundStartError>> {
        let mode = self.mode;
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            state.calls.fetch_add(1, Ordering::SeqCst);
            state.requests.lock().unwrap().push((
                request.command().to_owned(),
                request.cwd().to_owned(),
                format!("{request:?}"),
            ));
            state
                .owners
                .lock()
                .unwrap()
                .push(request.output_owner().map(|owner| {
                    (
                        owner.session_id().to_string(),
                        owner.session_incarnation_id().to_string(),
                    )
                }));
            match mode {
                BackgroundMode::Success {
                    cancel_before_return,
                } => {
                    if cancel_before_return {
                        let _ = cancellation.cancel();
                    }
                    TerminalBackgroundOutcome::new(7, NonZeroU32::new(1234))
                }
                BackgroundMode::Error(kind) => Err(BackgroundStartError::new(kind)),
            }
        })
    }
}

#[derive(Clone)]
struct FakeBackgroundOutputReader {
    result: Result<TerminalBackgroundReadSnapshot, TerminalBackgroundReadError>,
    requests: Arc<Mutex<Vec<BackgroundReadRequest>>>,
    pending: bool,
}

type BackgroundReadRequest = (String, String, u64, u64, u64);

impl FakeBackgroundOutputReader {
    fn snapshot(snapshot: TerminalBackgroundReadSnapshot) -> Self {
        Self {
            result: Ok(snapshot),
            requests: Arc::new(Mutex::new(Vec::new())),
            pending: false,
        }
    }

    fn success(bytes: Vec<u8>, closed: bool) -> Self {
        let length = u64::try_from(bytes.len()).unwrap();
        Self {
            result: TerminalBackgroundReadSnapshot::new(
                bytes, length, length, length, 0, false, closed,
            ),
            requests: Arc::new(Mutex::new(Vec::new())),
            pending: false,
        }
    }

    fn error(kind: TerminalBackgroundReadErrorKind) -> Self {
        Self {
            result: Err(TerminalBackgroundReadError::new(kind)),
            requests: Arc::new(Mutex::new(Vec::new())),
            pending: false,
        }
    }

    fn pending() -> Self {
        let mut reader = Self::success(Vec::new(), false);
        reader.pending = true;
        reader
    }
}

impl TerminalBackgroundOutputReader for FakeBackgroundOutputReader {
    fn read(
        &self,
        owner: BackgroundOutputOwner,
        background_id: u64,
        cursor_segment: u64,
        cursor_offset: u64,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalBackgroundReadSnapshot, TerminalBackgroundReadError>>
    {
        self.requests.lock().unwrap().push((
            owner.session_id().to_string(),
            owner.session_incarnation_id().to_string(),
            background_id,
            cursor_segment,
            cursor_offset,
        ));
        let result = self.result.clone();
        if self.pending {
            Box::pin(std::future::pending())
        } else {
            Box::pin(async move { result })
        }
    }
}

#[derive(Clone, Default)]
struct PendingBackgroundOutputReader {
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

struct PendingBackgroundOutputRead {
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl Future for PendingBackgroundOutputRead {
    type Output = Result<TerminalBackgroundReadSnapshot, TerminalBackgroundReadError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for PendingBackgroundOutputRead {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl TerminalBackgroundOutputReader for PendingBackgroundOutputReader {
    fn read(
        &self,
        _owner: BackgroundOutputOwner,
        _background_id: u64,
        _cursor_segment: u64,
        _cursor_offset: u64,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalBackgroundReadSnapshot, TerminalBackgroundReadError>>
    {
        Box::pin(PendingBackgroundOutputRead {
            polls: Arc::clone(&self.polls),
            drops: Arc::clone(&self.drops),
        })
    }
}

#[derive(Clone)]
struct ReadyCancellingBackgroundOutputReader {
    snapshot: TerminalBackgroundReadSnapshot,
}

impl TerminalBackgroundOutputReader for ReadyCancellingBackgroundOutputReader {
    fn read(
        &self,
        _owner: BackgroundOutputOwner,
        _background_id: u64,
        _cursor_segment: u64,
        _cursor_offset: u64,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalBackgroundReadSnapshot, TerminalBackgroundReadError>>
    {
        let snapshot = self.snapshot.clone();
        Box::pin(async move {
            cancellation.cancel();
            Ok(snapshot)
        })
    }
}

#[derive(Clone)]
struct FakeBackgroundInspector {
    state: Arc<BackgroundInspectorState>,
    cancel_before_return: bool,
    error: Option<NativeBackgroundInspectionErrorKind>,
}

#[derive(Default)]
struct BackgroundInspectorState {
    calls: AtomicUsize,
    ids: Mutex<Vec<u64>>,
}

impl FakeBackgroundInspector {
    fn new(cancel_before_return: bool) -> Self {
        Self {
            state: Arc::new(BackgroundInspectorState::default()),
            cancel_before_return,
            error: None,
        }
    }

    fn with_error(error: NativeBackgroundInspectionErrorKind) -> Self {
        Self {
            state: Arc::new(BackgroundInspectorState::default()),
            cancel_before_return: false,
            error: Some(error),
        }
    }

    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }
}

impl TerminalBackgroundInspector for FakeBackgroundInspector {
    fn inspect(
        &self,
        background_id: u64,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeBackgroundDetail, NativeBackgroundInspectionError>> {
        let state = Arc::clone(&self.state);
        let cancel_before_return = self.cancel_before_return;
        let error = self.error;
        Box::pin(async move {
            state.calls.fetch_add(1, Ordering::SeqCst);
            state.ids.lock().unwrap().push(background_id);
            if cancel_before_return {
                let _ = cancellation.cancel();
            }
            if let Some(error) = error {
                return Err(NativeBackgroundInspectionError::new(error));
            }
            NativeBackgroundDetail::new(
                background_id,
                NativeBackgroundState::Running,
                10,
                20,
                Some(1234),
                "private command".to_owned(),
                "/private/workspace".to_owned(),
                None,
                Some("https://private.invalid".to_owned()),
                Some("private diagnostic".to_owned()),
            )
        })
    }
}

#[derive(Clone)]
enum BackgroundCatalogMode {
    Ready {
        listing: NativeBackgroundList,
        cancel_before_return: bool,
    },
    Error(NativeBackgroundInspectionErrorKind),
    Pending,
}

#[derive(Default)]
struct BackgroundCatalogState {
    calls: AtomicUsize,
    polls: AtomicUsize,
    drops: AtomicUsize,
}

#[derive(Clone)]
struct FakeBackgroundCatalog {
    mode: BackgroundCatalogMode,
    state: Arc<BackgroundCatalogState>,
}

impl FakeBackgroundCatalog {
    fn ready(listing: NativeBackgroundList) -> Self {
        Self {
            mode: BackgroundCatalogMode::Ready {
                listing,
                cancel_before_return: false,
            },
            state: Arc::new(BackgroundCatalogState::default()),
        }
    }

    fn cancelling(listing: NativeBackgroundList) -> Self {
        Self {
            mode: BackgroundCatalogMode::Ready {
                listing,
                cancel_before_return: true,
            },
            state: Arc::new(BackgroundCatalogState::default()),
        }
    }

    fn with_error(kind: NativeBackgroundInspectionErrorKind) -> Self {
        Self {
            mode: BackgroundCatalogMode::Error(kind),
            state: Arc::new(BackgroundCatalogState::default()),
        }
    }

    fn pending() -> Self {
        Self {
            mode: BackgroundCatalogMode::Pending,
            state: Arc::new(BackgroundCatalogState::default()),
        }
    }

    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    fn polls(&self) -> usize {
        self.state.polls.load(Ordering::SeqCst)
    }

    fn drops(&self) -> usize {
        self.state.drops.load(Ordering::SeqCst)
    }
}

struct PendingBackgroundCatalogFuture {
    state: Arc<BackgroundCatalogState>,
}

impl Future for PendingBackgroundCatalogFuture {
    type Output = Result<NativeBackgroundList, NativeBackgroundInspectionError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.state.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for PendingBackgroundCatalogFuture {
    fn drop(&mut self) {
        self.state.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl TerminalBackgroundCatalog for FakeBackgroundCatalog {
    fn list(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeBackgroundList, NativeBackgroundInspectionError>> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        match &self.mode {
            BackgroundCatalogMode::Ready {
                listing,
                cancel_before_return,
            } => {
                let listing = listing.clone();
                let cancel_before_return = *cancel_before_return;
                Box::pin(async move {
                    if cancel_before_return {
                        let _ = cancellation.cancel();
                    }
                    Ok(listing)
                })
            }
            BackgroundCatalogMode::Error(kind) => {
                let kind = *kind;
                Box::pin(async move { Err(NativeBackgroundInspectionError::new(kind)) })
            }
            BackgroundCatalogMode::Pending => Box::pin(PendingBackgroundCatalogFuture {
                state: Arc::clone(&self.state),
            }),
        }
    }
}

#[derive(Clone)]
struct SequenceBackgroundInspector {
    states: Arc<Vec<(NativeBackgroundState, Option<i32>)>>,
    calls: Arc<AtomicUsize>,
}

impl SequenceBackgroundInspector {
    fn new(states: Vec<(NativeBackgroundState, Option<i32>)>) -> Self {
        assert!(!states.is_empty());
        Self {
            states: Arc::new(states),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl TerminalBackgroundInspector for SequenceBackgroundInspector {
    fn inspect(
        &self,
        background_id: u64,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeBackgroundDetail, NativeBackgroundInspectionError>> {
        let states = Arc::clone(&self.states);
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            let index = calls.fetch_add(1, Ordering::SeqCst);
            let (state, exit_code) = states[index.min(states.len() - 1)];
            NativeBackgroundDetail::new(
                background_id,
                state,
                10,
                20 + u64::try_from(index).unwrap(),
                Some(1234),
                "private command".to_owned(),
                "/private/workspace".to_owned(),
                exit_code,
                None,
                None,
            )
        })
    }
}

#[derive(Clone)]
struct DelayedSequenceBackgroundInspector {
    observations: Arc<Vec<(NativeBackgroundState, Option<i32>, Duration)>>,
    calls: Arc<AtomicUsize>,
}

impl DelayedSequenceBackgroundInspector {
    fn new(observations: Vec<(NativeBackgroundState, Option<i32>, Duration)>) -> Self {
        assert!(!observations.is_empty());
        Self {
            observations: Arc::new(observations),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl TerminalBackgroundInspector for DelayedSequenceBackgroundInspector {
    fn inspect(
        &self,
        background_id: u64,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeBackgroundDetail, NativeBackgroundInspectionError>> {
        let observations = Arc::clone(&self.observations);
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            let index = calls.fetch_add(1, Ordering::SeqCst);
            let (state, exit_code, delay) = observations[index.min(observations.len() - 1)];
            std::thread::sleep(delay);
            NativeBackgroundDetail::new(
                background_id,
                state,
                10,
                20 + u64::try_from(index).unwrap(),
                Some(1234),
                "private command".to_owned(),
                "/private/workspace".to_owned(),
                exit_code,
                None,
                None,
            )
        })
    }
}

#[derive(Clone, Default)]
struct SleepingWaitDelay {
    polls: Arc<AtomicUsize>,
    deadlines: Arc<Mutex<Vec<Instant>>>,
}

impl TerminalBackgroundWaitDelay for SleepingWaitDelay {
    fn wait_until(
        &self,
        deadline: Instant,
    ) -> BoxFuture<'_, Result<(), TerminalBackgroundWaitDelayError>> {
        let polls = Arc::clone(&self.polls);
        let deadlines = Arc::clone(&self.deadlines);
        Box::pin(async move {
            polls.fetch_add(1, Ordering::SeqCst);
            deadlines.lock().unwrap().push(deadline);
            let remaining = deadline.saturating_duration_since(Instant::now());
            if !remaining.is_zero() {
                std::thread::sleep(remaining);
            }
            Ok(())
        })
    }
}

#[derive(Clone, Default)]
struct EarlyWaitDelay {
    polls: Arc<AtomicUsize>,
}

impl TerminalBackgroundWaitDelay for EarlyWaitDelay {
    fn wait_until(
        &self,
        _deadline: Instant,
    ) -> BoxFuture<'_, Result<(), TerminalBackgroundWaitDelayError>> {
        let polls = Arc::clone(&self.polls);
        Box::pin(async move {
            polls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

#[derive(Clone, Default)]
struct ErrorWaitDelay;

impl TerminalBackgroundWaitDelay for ErrorWaitDelay {
    fn wait_until(
        &self,
        _deadline: Instant,
    ) -> BoxFuture<'_, Result<(), TerminalBackgroundWaitDelayError>> {
        Box::pin(async { Err(TerminalBackgroundWaitDelayError::new()) })
    }
}

#[derive(Clone)]
struct DropCancellingWaitDelay {
    cancellation: CancellationToken,
    dropped: Arc<AtomicBool>,
}

struct DropCancellingWaitDelayFuture {
    cancellation: CancellationToken,
    dropped: Arc<AtomicBool>,
}

impl Future for DropCancellingWaitDelayFuture {
    type Output = Result<(), TerminalBackgroundWaitDelayError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        let remaining = Instant::now()
            .checked_add(Duration::from_millis(1))
            .expect("short test deadline")
            .saturating_duration_since(Instant::now());
        std::thread::sleep(remaining);
        Poll::Ready(Ok(()))
    }
}

impl Drop for DropCancellingWaitDelayFuture {
    fn drop(&mut self) {
        let _ = self.cancellation.cancel();
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl TerminalBackgroundWaitDelay for DropCancellingWaitDelay {
    fn wait_until(
        &self,
        _deadline: Instant,
    ) -> BoxFuture<'_, Result<(), TerminalBackgroundWaitDelayError>> {
        Box::pin(DropCancellingWaitDelayFuture {
            cancellation: self.cancellation.clone(),
            dropped: Arc::clone(&self.dropped),
        })
    }
}

#[derive(Clone, Default)]
struct PendingWaitDelay {
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

struct PendingWaitDelayFuture {
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl Future for PendingWaitDelayFuture {
    type Output = Result<(), TerminalBackgroundWaitDelayError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for PendingWaitDelayFuture {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl TerminalBackgroundWaitDelay for PendingWaitDelay {
    fn wait_until(
        &self,
        _deadline: Instant,
    ) -> BoxFuture<'_, Result<(), TerminalBackgroundWaitDelayError>> {
        Box::pin(PendingWaitDelayFuture {
            polls: Arc::clone(&self.polls),
            drops: Arc::clone(&self.drops),
        })
    }
}

#[derive(Clone)]
struct PendingBackgroundInspector {
    polls: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

struct PendingInspection {
    polls: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

#[derive(Clone)]
struct DropCancellingBackgroundInspector {
    dropped: Arc<AtomicBool>,
}

struct DropCancellingInspection {
    cancellation: CancellationToken,
    dropped: Arc<AtomicBool>,
    detail: Option<NativeBackgroundDetail>,
}

impl Future for DropCancellingInspection {
    type Output = Result<NativeBackgroundDetail, NativeBackgroundInspectionError>;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(self.detail.take().expect("inspection is polled once")))
    }
}

impl Drop for DropCancellingInspection {
    fn drop(&mut self) {
        let _ = self.cancellation.cancel();
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl TerminalBackgroundInspector for DropCancellingBackgroundInspector {
    fn inspect(
        &self,
        background_id: u64,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeBackgroundDetail, NativeBackgroundInspectionError>> {
        Box::pin(DropCancellingInspection {
            cancellation,
            dropped: Arc::clone(&self.dropped),
            detail: Some(
                NativeBackgroundDetail::new(
                    background_id,
                    NativeBackgroundState::Running,
                    10,
                    20,
                    Some(1234),
                    "private command".to_owned(),
                    "/private/workspace".to_owned(),
                    None,
                    None,
                    None,
                )
                .expect("valid detail"),
            ),
        })
    }
}

impl Future for PendingInspection {
    type Output = Result<NativeBackgroundDetail, NativeBackgroundInspectionError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for PendingInspection {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl TerminalBackgroundInspector for PendingBackgroundInspector {
    fn inspect(
        &self,
        _background_id: u64,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeBackgroundDetail, NativeBackgroundInspectionError>> {
        Box::pin(PendingInspection {
            polls: Arc::clone(&self.polls),
            dropped: Arc::clone(&self.dropped),
        })
    }
}

struct FakeExecution {
    mode: Mode,
    cancellation: CancellationToken,
    state: Arc<State>,
    stdout: Vec<u8>,
    stdout_total: usize,
    stderr: Vec<u8>,
    stderr_total: usize,
}

impl Future for FakeExecution {
    type Output = Result<TerminalExecutionOutcome, TerminalExecutorError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        let previous_polls = self.state.polls.fetch_add(1, Ordering::SeqCst);
        if matches!(self.mode, Mode::Pending) {
            return Poll::Pending;
        }
        if matches!(self.mode, Mode::OutputLimitAfterPending) && previous_polls == 0 {
            return Poll::Pending;
        }
        if matches!(self.mode, Mode::CancelThenExit) {
            assert!(self.cancellation.cancel());
        }
        if let Mode::DelayedOutputLimit(delay) = self.mode {
            std::thread::sleep(delay);
        }
        if let Mode::Error(kind) = self.mode {
            return Poll::Ready(Err(TerminalExecutorError::new(kind)));
        }
        let status = match self.mode {
            Mode::Exited(code) => TerminalExecutionStatus::Exited(code),
            Mode::CancelThenExit | Mode::DropCancelThenExit => TerminalExecutionStatus::Exited(0),
            Mode::Signaled(signal) => TerminalExecutionStatus::Signaled(signal),
            Mode::TimedOut => TerminalExecutionStatus::TimedOut,
            Mode::OutputLimit | Mode::DelayedOutputLimit(_) | Mode::OutputLimitAfterPending => {
                TerminalExecutionStatus::OutputLimit
            }
            Mode::Error(_) | Mode::Pending => unreachable!(),
        };
        Poll::Ready(TerminalExecutionOutcome::new(
            status,
            TerminalCapturedOutput::new(self.stdout.clone(), self.stdout_total as u64).unwrap(),
            TerminalCapturedOutput::new(self.stderr.clone(), self.stderr_total as u64).unwrap(),
            Duration::from_millis(7),
        ))
    }
}

#[derive(Default)]
struct BlockingWakeState {
    entered: bool,
    released: bool,
    returned: bool,
}

#[derive(Default)]
struct BlockingWake {
    state: Mutex<BlockingWakeState>,
    changed: Condvar,
}

impl BlockingWake {
    fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.state.lock().unwrap();
        while !state.entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "terminal Waker callback did not run");
            let waited = self.changed.wait_timeout(state, remaining).unwrap();
            state = waited.0;
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.released = true;
        self.changed.notify_all();
    }

    fn wait_until_returned(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.state.lock().unwrap();
        while !state.returned {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "terminal Waker callback did not return"
            );
            let waited = self.changed.wait_timeout(state, remaining).unwrap();
            state = waited.0;
        }
    }

    fn block(&self) {
        let mut state = self.state.lock().unwrap();
        state.entered = true;
        self.changed.notify_all();
        while !state.released {
            state = self.changed.wait(state).unwrap();
        }
        state.returned = true;
        self.changed.notify_all();
    }
}

impl Wake for BlockingWake {
    fn wake(self: Arc<Self>) {
        self.block();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.block();
    }
}

struct BlockingWakeRelease(Arc<BlockingWake>);

impl Drop for BlockingWakeRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Default)]
struct CountingBlockingWakeState {
    entered: usize,
    in_flight: usize,
    max_in_flight: usize,
    released: bool,
    returned: usize,
}

#[derive(Default)]
struct CountingBlockingWake {
    state: Mutex<CountingBlockingWakeState>,
    changed: Condvar,
}

impl CountingBlockingWake {
    fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.state.lock().unwrap();
        while state.entered == 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "terminal Waker callback did not run");
            let waited = self.changed.wait_timeout(state, remaining).unwrap();
            state = waited.0;
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.released = true;
        self.changed.notify_all();
    }

    fn snapshot(&self) -> (usize, usize, usize, usize) {
        let state = self.state.lock().unwrap();
        (
            state.entered,
            state.in_flight,
            state.max_in_flight,
            state.returned,
        )
    }

    fn block(&self) {
        let mut state = self.state.lock().unwrap();
        state.entered += 1;
        state.in_flight += 1;
        state.max_in_flight = state.max_in_flight.max(state.in_flight);
        self.changed.notify_all();
        while !state.released {
            state = self.changed.wait(state).unwrap();
        }
        state.in_flight -= 1;
        state.returned += 1;
        self.changed.notify_all();
    }
}

impl Wake for CountingBlockingWake {
    fn wake(self: Arc<Self>) {
        self.block();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.block();
    }
}

struct CountingBlockingWakeRelease(Arc<CountingBlockingWake>);

impl Drop for CountingBlockingWakeRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn poll_with_waker<F: Future + ?Sized>(future: Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(waker))
}

#[derive(Default)]
struct RetainedPublisherState {
    calls: AtomicUsize,
    polls: AtomicUsize,
    inner: Mutex<RetainedPublisherInner>,
    changed: Condvar,
}

#[derive(Default)]
struct RetainedPublisherInner {
    published: bool,
    waker: Option<Waker>,
}

#[derive(Clone, Default)]
struct RetainedPublisherExecutor {
    state: Arc<RetainedPublisherState>,
}

impl RetainedPublisherExecutor {
    fn publish_and_wake(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut inner = self.state.inner.lock().unwrap();
        while inner.waker.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "injected executor did not retain a task Waker"
            );
            let waited = self.state.changed.wait_timeout(inner, remaining).unwrap();
            inner = waited.0;
        }
        inner.published = true;
        let waker = inner.waker.take().expect("retained task Waker exists");
        drop(inner);
        waker.wake();
    }

    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }
}

impl TerminalExecutor for RetainedPublisherExecutor {
    fn execute(
        &self,
        _request: TerminalExecutionRequest,
        _cancellation: CancellationToken,
    ) -> TerminalExecution {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(RetainedPublisherExecution {
            state: Arc::clone(&self.state),
        })
    }
}

struct RetainedPublisherExecution {
    state: Arc<RetainedPublisherState>,
}

impl Drop for RetainedPublisherExecution {
    fn drop(&mut self) {
        let retained = {
            let mut inner = self.state.inner.lock().unwrap();
            inner.waker.take()
        };
        drop(retained);
    }
}

const CLONED_WAKER_FANOUT: usize = 16;

#[derive(Default)]
struct WakerFanoutState {
    calls: AtomicUsize,
    inner: Mutex<WakerFanoutInner>,
    changed: Condvar,
}

#[derive(Default)]
struct WakerFanoutInner {
    published: bool,
    wakers: Vec<Waker>,
}

#[derive(Clone, Default)]
struct WakerFanoutExecutor {
    state: Arc<WakerFanoutState>,
}

impl WakerFanoutExecutor {
    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    fn publish_and_take_wakers(&self) -> Vec<Waker> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut inner = self.state.inner.lock().unwrap();
        while inner.wakers.len() != CLONED_WAKER_FANOUT {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "injected executor did not retain every cloned task Waker"
            );
            let waited = self.state.changed.wait_timeout(inner, remaining).unwrap();
            inner = waited.0;
        }
        inner.published = true;
        std::mem::take(&mut inner.wakers)
    }
}

impl TerminalExecutor for WakerFanoutExecutor {
    fn execute(
        &self,
        _request: TerminalExecutionRequest,
        _cancellation: CancellationToken,
    ) -> TerminalExecution {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        let mut inner = self.state.inner.lock().unwrap();
        assert!(
            inner.wakers.is_empty(),
            "a new execution started while retained Wakers remained"
        );
        inner.published = false;
        drop(inner);
        Box::pin(WakerFanoutExecution {
            state: Arc::clone(&self.state),
        })
    }
}

struct WakerFanoutExecution {
    state: Arc<WakerFanoutState>,
}

impl Future for WakerFanoutExecution {
    type Output = Result<TerminalExecutionOutcome, TerminalExecutorError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.state.inner.lock().unwrap();
        if !inner.published {
            if inner.wakers.is_empty() {
                inner.wakers = std::iter::repeat_with(|| context.waker().clone())
                    .take(CLONED_WAKER_FANOUT)
                    .collect();
                self.state.changed.notify_all();
            }
            return Poll::Pending;
        }
        drop(inner);
        Poll::Ready(TerminalExecutionOutcome::new(
            TerminalExecutionStatus::Exited(0),
            TerminalCapturedOutput::new(Vec::new(), 0).unwrap(),
            TerminalCapturedOutput::new(Vec::new(), 0).unwrap(),
            Duration::from_millis(1),
        ))
    }
}

impl Future for RetainedPublisherExecution {
    type Output = Result<TerminalExecutionOutcome, TerminalExecutorError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.state.polls.fetch_add(1, Ordering::SeqCst);
        let mut inner = self.state.inner.lock().unwrap();
        if !inner.published {
            if inner.waker.is_none() {
                inner.waker = Some(context.waker().clone());
                self.state.changed.notify_all();
            }
            return Poll::Pending;
        }
        drop(inner);
        Poll::Ready(TerminalExecutionOutcome::new(
            TerminalExecutionStatus::Exited(0),
            TerminalCapturedOutput::new(Vec::new(), 0).unwrap(),
            TerminalCapturedOutput::new(Vec::new(), 0).unwrap(),
            Duration::from_millis(1),
        ))
    }
}

#[derive(Default)]
struct ObservedWake {
    calls: Mutex<usize>,
    changed: Condvar,
}

impl ObservedWake {
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }

    fn wait_for_calls(&self, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut calls = self.calls.lock().unwrap();
        while *calls < expected {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "the original host Waker was not notified"
            );
            let waited = self.changed.wait_timeout(calls, remaining).unwrap();
            calls = waited.0;
        }
    }

    fn record(&self) {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        self.changed.notify_all();
    }
}

impl Wake for ObservedWake {
    fn wake(self: Arc<Self>) {
        self.record();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.record();
    }
}

#[derive(Default)]
struct SelfRepollState {
    calls: AtomicUsize,
    inner: Mutex<SelfRepollInner>,
    changed: Condvar,
}

#[derive(Default)]
struct SelfRepollInner {
    published: bool,
    waker: Option<Waker>,
}

#[derive(Clone, Default)]
struct SelfRepollExecutor {
    state: Arc<SelfRepollState>,
}

impl SelfRepollExecutor {
    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    fn retained_waker(&self) -> Waker {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut inner = self.state.inner.lock().unwrap();
        while inner.waker.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "injected executor did not retain the terminal-supplied Waker"
            );
            let waited = self.state.changed.wait_timeout(inner, remaining).unwrap();
            inner = waited.0;
        }
        inner.waker.as_ref().expect("retained Waker exists").clone()
    }

    fn publish_and_wake(&self) {
        let retained = {
            let mut inner = self.state.inner.lock().unwrap();
            inner.published = true;
            inner.waker.take().expect("retained task Waker exists")
        };
        retained.wake();
    }
}

impl TerminalExecutor for SelfRepollExecutor {
    fn execute(
        &self,
        _request: TerminalExecutionRequest,
        _cancellation: CancellationToken,
    ) -> TerminalExecution {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        {
            let mut inner = self.state.inner.lock().unwrap();
            assert!(
                inner.waker.is_none(),
                "a new execution started with a stale Waker registration"
            );
            inner.published = false;
        }
        Box::pin(SelfRepollExecution {
            state: Arc::clone(&self.state),
        })
    }
}

struct SelfRepollExecution {
    state: Arc<SelfRepollState>,
}

impl Future for SelfRepollExecution {
    type Output = Result<TerminalExecutionOutcome, TerminalExecutorError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.state.inner.lock().unwrap();
        if !inner.published {
            if inner.waker.is_none() {
                inner.waker = Some(context.waker().clone());
                self.state.changed.notify_all();
            }
            return Poll::Pending;
        }
        drop(inner);
        Poll::Ready(TerminalExecutionOutcome::new(
            TerminalExecutionStatus::Exited(0),
            TerminalCapturedOutput::new(Vec::new(), 0).unwrap(),
            TerminalCapturedOutput::new(Vec::new(), 0).unwrap(),
            Duration::from_millis(1),
        ))
    }
}

impl Drop for SelfRepollExecution {
    fn drop(&mut self) {
        let retained = {
            let mut inner = self.state.inner.lock().unwrap();
            inner.waker.take()
        };
        drop(retained);
    }
}

fn assert_self_repoll_tail_is_busy(tool: &TerminalTool, executor: &SelfRepollExecutor) {
    let mut blocked = Box::pin(tool.execute(
        context(),
        exact_arguments("blocked-by-retained-waker", "."),
        CancellationToken::new(),
    ));
    match poll_once(blocked.as_mut()) {
        Poll::Ready(Err(error)) => {
            assert_eq!(error.kind, ToolErrorKind::Unavailable);
            assert_eq!(error.code, "terminal_busy");
        }
        Poll::Ready(Ok(_)) => panic!("execution bypassed a retained terminal Waker"),
        Poll::Pending => panic!("retained terminal Waker released its activity slot early"),
    }
    drop(blocked);
    assert_eq!(executor.calls(), 1);
}

fn assert_self_repoll_capacity_recovers(tool: &TerminalTool, executor: &SelfRepollExecutor) {
    let recovery_deadline = Instant::now() + Duration::from_secs(2);
    let mut recovered = loop {
        let mut candidate = Box::pin(tool.execute(
            context(),
            exact_arguments("recovered-after-retained-waker", "."),
            CancellationToken::new(),
        ));
        match poll_once(candidate.as_mut()) {
            Poll::Pending => break candidate,
            Poll::Ready(Err(error)) if error.code == "terminal_busy" => {
                assert!(
                    Instant::now() < recovery_deadline,
                    "capacity did not recover after the retained Waker was dropped"
                );
                drop(candidate);
                std::thread::yield_now();
            }
            Poll::Ready(Err(error)) => panic!("capacity recovery failed: {error}"),
            Poll::Ready(Ok(_)) => panic!("unpublished recovered execution completed"),
        }
    };
    assert_eq!(executor.calls(), 2);
    executor.publish_and_wake();
    let output = match poll_once(recovered.as_mut()) {
        Poll::Ready(Ok(output)) => output,
        Poll::Ready(Err(error)) => panic!("recovered execution failed: {error}"),
        Poll::Pending => panic!("published recovered execution remained pending"),
    };
    assert_eq!(output.content["status"], "exited");
}

fn assert_retained_publisher_capacity_recovers(
    tool: &TerminalTool,
    executor: &RetainedPublisherExecutor,
) {
    let recovery_deadline = Instant::now() + Duration::from_secs(2);
    let mut recovered = loop {
        let mut candidate = Box::pin(tool.execute(
            context(),
            exact_arguments("recovered", "."),
            CancellationToken::new(),
        ));
        match poll_once(candidate.as_mut()) {
            Poll::Pending => break candidate,
            Poll::Ready(Err(error)) if error.code == "terminal_busy" => {
                assert!(
                    Instant::now() < recovery_deadline,
                    "shared notifier capacity did not recover"
                );
                drop(candidate);
                std::thread::yield_now();
            }
            Poll::Ready(Err(error)) => panic!("capacity recovery failed: {error}"),
            Poll::Ready(Ok(_)) => panic!("unpublished injected execution completed"),
        }
    };
    assert_eq!(executor.calls(), 2);
    executor.publish_and_wake();
    let recovered_output = match poll_once(recovered.as_mut()) {
        Poll::Ready(Ok(output)) => output,
        Poll::Ready(Err(error)) => panic!("recovered execution failed: {error}"),
        Poll::Pending => panic!("published recovered execution remained pending"),
    };
    assert_eq!(recovered_output.content["status"], "exited");
}

impl Drop for FakeExecution {
    fn drop(&mut self) {
        if matches!(self.mode, Mode::DropCancelThenExit) {
            let _ = self.cancellation.cancel();
        }
        self.state.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn environment() -> Vec<(OsString, OsString)> {
    vec![
        (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
        (
            OsString::from(PRIVATE_ENVIRONMENT_KEY),
            OsString::from(PRIVATE_ENVIRONMENT_VALUE),
        ),
    ]
}

fn limits(max_active: usize) -> TerminalLimits {
    TerminalLimits::new(Duration::from_secs(5), max_active).unwrap()
}

fn tool(root: &std::path::Path, executor: &FakeExecutor) -> TerminalTool {
    TerminalTool::with_executor(
        root,
        environment(),
        Arc::new(executor.clone()),
        TerminalLimits::default(),
    )
    .unwrap()
}

fn background_tool(
    root: &std::path::Path,
    executor: &FakeExecutor,
    starter: &FakeBackgroundStarter,
) -> TerminalTool {
    let canonical_workspace = std::fs::canonicalize(root)
        .unwrap()
        .to_str()
        .expect("test workspace is Unicode")
        .to_owned();
    TerminalTool::with_executor_and_background(
        root,
        environment(),
        Arc::new(executor.clone()),
        limits(1),
        canonical_workspace,
        ProcessEnvironment {
            profile: TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE.to_owned(),
            sha256: "a".repeat(64),
        },
        Arc::new(starter.clone()),
    )
    .unwrap()
}

fn reading_tool<R>(
    root: &std::path::Path,
    executor: &FakeExecutor,
    starter: &FakeBackgroundStarter,
    reader: &R,
) -> TerminalTool
where
    R: TerminalBackgroundOutputReader + Clone,
{
    background_tool(root, executor, starter)
        .with_output_reader(Arc::new(reader.clone()))
        .unwrap()
}

fn assert_background_start_spec(tool: &TerminalTool) {
    let spec = tool.spec();
    assert_eq!(
        spec.description,
        "Run one foreground command or start one noninteractive background command"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["exec", "start"] },
                "command": { "type": "string" },
                "cwd": { "type": "string", "default": "." },
                "profile": { "type": "string", "enum": ["clean"], "default": "clean" }
            },
            "required": ["action", "command"],
            "additionalProperties": false
        })
    );
}

fn inspecting_tool<I>(
    root: &std::path::Path,
    executor: &FakeExecutor,
    starter: &FakeBackgroundStarter,
    inspector: &I,
) -> TerminalTool
where
    I: TerminalBackgroundInspector + Clone,
{
    let canonical_workspace = std::fs::canonicalize(root)
        .unwrap()
        .to_str()
        .expect("test workspace is Unicode")
        .to_owned();
    TerminalTool::with_executor_background_and_inspector(
        root,
        environment(),
        Arc::new(executor.clone()),
        limits(1),
        canonical_workspace,
        ProcessEnvironment {
            profile: TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE.to_owned(),
            sha256: "a".repeat(64),
        },
        Arc::new(starter.clone()),
        Arc::new(inspector.clone()),
    )
    .unwrap()
}

fn waiting_tool<I, D>(
    root: &std::path::Path,
    executor: &FakeExecutor,
    starter: &FakeBackgroundStarter,
    inspector: &I,
    delay: &D,
) -> TerminalTool
where
    I: TerminalBackgroundInspector + Clone,
    D: TerminalBackgroundWaitDelay + Clone,
{
    let canonical_workspace = std::fs::canonicalize(root)
        .unwrap()
        .to_str()
        .expect("test workspace is Unicode")
        .to_owned();
    TerminalTool::with_executor_background_inspector_and_wait_delay(
        root,
        environment(),
        Arc::new(executor.clone()),
        limits(1),
        canonical_workspace,
        ProcessEnvironment {
            profile: TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE.to_owned(),
            sha256: "a".repeat(64),
        },
        Arc::new(starter.clone()),
        Arc::new(inspector.clone()),
        Arc::new(delay.clone()),
    )
    .unwrap()
}

fn cataloging_tool<I, D, C>(
    root: &std::path::Path,
    executor: &FakeExecutor,
    starter: &FakeBackgroundStarter,
    inspector: &I,
    delay: &D,
    catalog: &C,
) -> TerminalTool
where
    I: TerminalBackgroundInspector + Clone,
    D: TerminalBackgroundWaitDelay + Clone,
    C: TerminalBackgroundCatalog + Clone,
{
    waiting_tool(root, executor, starter, inspector, delay)
        .with_catalog(Arc::new(catalog.clone()))
        .unwrap()
}

fn background_summary(
    id: u64,
    state: NativeBackgroundState,
    updated_at_ms: u64,
    command_preview: &str,
    preview_truncated: bool,
) -> NativeBackgroundRecordSummary {
    NativeBackgroundRecordSummary::new(
        id,
        state,
        updated_at_ms,
        command_preview.to_owned(),
        preview_truncated,
    )
    .unwrap()
}

fn background_listing(
    records: Vec<NativeBackgroundRecordSummary>,
    truncated: bool,
) -> NativeBackgroundList {
    NativeBackgroundList::new(records, truncated).unwrap()
}

fn exact_wait_arguments(background_id: u64, wait_ceiling_ms: u64) -> Value {
    json!({
        "action": "wait",
        "background_id": background_id,
        "return_when": { "kind": "exit" },
        "wait_ceiling_ms": wait_ceiling_ms
    })
}

fn execute(
    tool: &TerminalTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_ready(tool.execute(context(), arguments, cancellation))
}

fn assert_foreground_capacity_is_busy(tool: &TerminalTool) {
    let blocked = execute(
        tool,
        exact_arguments("blocked-foreground", "."),
        CancellationToken::new(),
    )
    .expect_err("the occupied max-active-one foreground slot must reject another exec");
    assert_eq!(blocked.kind, ToolErrorKind::Unavailable);
    assert_eq!(blocked.code, "terminal_busy");
    assert!(blocked.retryable);
}

fn assert_foreground_capacity_recovers(tool: &TerminalTool, executor: &FakeExecutor) {
    let mut recovered = Box::pin(tool.execute(
        context(),
        exact_arguments("recovered-foreground", "."),
        CancellationToken::new(),
    ));
    assert!(
        poll_with_waker(recovered.as_mut(), &futures_util::task::noop_waker()).is_pending(),
        "foreground capacity must recover after the original execution is dropped"
    );
    assert_eq!(executor.calls(), 2);
    assert_eq!(executor.polls(), 2);
}

fn exact_arguments(command: &str, cwd: &str) -> Value {
    json!({
        "action": "exec",
        "command": command,
        "cwd": cwd,
        "profile": "clean",
    })
}

fn exact_start_arguments(command: &str, cwd: &str) -> Value {
    json!({
        "action": "start",
        "command": command,
        "cwd": cwd,
        "profile": "clean",
    })
}

fn canonical_relative_cwd_with_length(length: usize) -> String {
    assert!(length > 0);
    let component_count =
        (length + MAX_TERMINAL_CWD_COMPONENT_BYTES + 1) / (MAX_TERMINAL_CWD_COMPONENT_BYTES + 1);
    assert!(component_count <= MAX_TERMINAL_CWD_COMPONENTS);
    let component_bytes = length - (component_count - 1);
    let minimum_component_bytes = component_bytes / component_count;
    let longer_components = component_bytes % component_count;
    (0..component_count)
        .map(|index| "x".repeat(minimum_component_bytes + usize::from(index < longer_components)))
        .collect::<Vec<_>>()
        .join("/")
}

fn assert_invalid_input(error: &ToolError) {
    assert_eq!(error.kind, ToolErrorKind::InvalidInput);
    assert!(!error.retryable);
}

#[test]
fn spec_and_defaults_are_strict_and_prepare_exact_process_identity() {
    let temporary = TemporaryDirectory::new("contract");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);

    let spec = tool.spec();
    assert_eq!(spec.name.as_str(), "terminal");
    assert_eq!(
        spec.description,
        "Run one foreground shell command from a workspace-relative directory"
    );
    assert_eq!(
        spec.input_schema,
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["exec"] },
                "command": { "type": "string" },
                "cwd": { "type": "string", "default": "." },
                "profile": { "type": "string", "enum": ["clean"], "default": "clean" }
            },
            "required": ["action", "command"],
            "additionalProperties": false
        })
    );

    let prepared = tool
        .prepare(call(
            "terminal",
            json!({ "action": "exec", "command": "printf '%s' hello" }),
        ))
        .unwrap();
    assert_eq!(
        prepared.arguments(),
        &exact_arguments("printf '%s' hello", ".")
    );
    let Capability::Process {
        program,
        arguments,
        working_directory,
        environment,
    } = prepared
        .capability()
        .expect("terminal requires permission authority")
    else {
        panic!("terminal must prepare a process capability")
    };
    assert_eq!(program, "/bin/sh");
    assert_eq!(arguments, &["-c", "printf '%s' hello"]);
    assert_eq!(working_directory, ".");
    assert_eq!(environment.profile, "construction_snapshot");
    assert_eq!(environment.sha256.len(), 64);
    assert!(
        environment
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    assert_eq!(
        serde_json::to_value(
            prepared
                .capability()
                .expect("terminal requires permission authority"),
        )
        .unwrap(),
        json!({
            "type": "process",
            "program": "/bin/sh",
            "arguments": ["-c", "printf '%s' hello"],
            "working_directory": ".",
            "environment": {
                "profile": "construction_snapshot",
                "sha256": environment.sha256,
            }
        })
    );
}

#[test]
fn background_start_has_exact_permission_identity_and_bypasses_foreground_execution() {
    let temporary = TemporaryDirectory::new("background-start");
    std::fs::create_dir(temporary.path().join("nested")).unwrap();
    let executor = FakeExecutor::new(Mode::Pending);
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: true,
    });
    let tool = background_tool(temporary.path(), &executor, &starter);
    let mut occupied_foreground = Box::pin(tool.execute(
        context(),
        exact_arguments("occupy-foreground", "."),
        CancellationToken::new(),
    ));
    assert!(
        poll_with_waker(
            occupied_foreground.as_mut(),
            &futures_util::task::noop_waker()
        )
        .is_pending()
    );
    assert_foreground_capacity_is_busy(&tool);

    assert_background_start_spec(&tool);
    let prepared = tool
        .prepare(call(
            "terminal",
            json!({ "action": "start", "command": "sleep 1", "cwd": "nested" }),
        ))
        .unwrap();
    assert_eq!(
        prepared.arguments(),
        &exact_start_arguments("sleep 1", "nested")
    );
    let Capability::Process {
        program,
        arguments,
        working_directory,
        environment,
    } = prepared
        .capability()
        .expect("start requires process permission")
    else {
        panic!("start must prepare process permission")
    };
    assert_eq!(program, "/bin/sh");
    assert_eq!(arguments, &["-c", "sleep 1"]);
    assert_eq!(working_directory, "nested");
    assert_eq!(environment.profile, TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE);
    assert_eq!(environment.sha256, "a".repeat(64));
    assert_eq!(starter.calls(), 0);

    let mut future = Box::pin(tool.execute(
        context(),
        exact_start_arguments("sleep 1", "nested"),
        CancellationToken::new(),
    ));
    assert_eq!(starter.calls(), 0);
    let output = poll_ready(future.as_mut());
    let output = output.expect("background start succeeds despite post-commit cancellation");
    assert_eq!(
        output.content,
        json!({
            "action": "start",
            "background_id": 7,
            "pid": 1234,
            "status": "started"
        })
    );
    assert!(!output.is_error);
    assert_eq!(starter.calls(), 1);
    assert_eq!(executor.calls(), 1);
    assert_eq!(executor.polls(), 1);
    let requests = starter.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "sleep 1");
    assert_eq!(
        requests[0].1,
        format!(
            "{}/nested",
            std::fs::canonicalize(temporary.path()).unwrap().display()
        )
    );
    assert_eq!(requests[0].2, "BackgroundStartRequest { .. }");
    assert_eq!(
        starter.owners(),
        vec![Some((
            "terminal-session".to_owned(),
            "terminal-incarnation".to_owned()
        ))]
    );
    drop(occupied_foreground);
    assert_foreground_capacity_recovers(&tool, &executor);
}

#[test]
fn background_read_has_one_closed_same_incarnation_no_authority_form() {
    let temporary = TemporaryDirectory::new("background-read");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let reader = FakeBackgroundOutputReader::success(vec![b'a', 0xff, b'b'], true);
    let tool = reading_tool(temporary.path(), &executor, &starter, &reader);

    let spec = tool.spec();
    assert_eq!(
        spec.description,
        "Run a foreground command, start a background command, or read bounded same-session background output"
    );
    let forms = spec.input_schema["oneOf"].as_array().unwrap();
    assert_eq!(forms.len(), 3);
    assert_eq!(forms[2]["properties"]["action"]["const"], "read");
    assert_eq!(
        forms[2]["required"],
        json!(["action", "background_id", "cursor_segment"])
    );
    assert_eq!(forms[2]["properties"]["cursor_segment"]["const"], 1);
    assert_eq!(forms[2]["properties"]["cursor_offset"]["default"], 0);

    let prepared = tool
        .prepare(call(
            "terminal",
            json!({
                "action": "read",
                "background_id": 7,
                "cursor_segment": 1
            }),
        ))
        .unwrap();
    assert!(prepared.capability().is_none());
    assert_eq!(
        prepared.arguments(),
        &json!({
            "action": "read",
            "background_id": 7,
            "cursor_segment": 1,
            "cursor_offset": 0
        })
    );
    let output = poll_ready(tool.execute(
        context(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(
        output.content,
        json!({
            "action": "read",
            "background_id": 7,
            "cursor_segment": 1,
            "cursor_offset": 3,
            "output": "a�b",
            "output_bytes": 3,
            "retained_bytes": 3,
            "truncated": false,
            "lossy": true,
            "stream_closed": true
        })
    );
    assert_eq!(
        *reader.requests.lock().unwrap(),
        vec![(
            "terminal-session".to_owned(),
            "terminal-incarnation".to_owned(),
            7,
            1,
            0
        )]
    );
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn background_read_preserves_utf8_pages_and_live_partial_scalars() {
    let temporary = TemporaryDirectory::new("background-read-utf8");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let page_prefix = vec![b'a'; MAX_TERMINAL_BACKGROUND_READ_BYTES - 1];
    let first_reader = FakeBackgroundOutputReader::snapshot(
        TerminalBackgroundReadSnapshot::new(
            page_prefix.clone(),
            u64::try_from(page_prefix.len()).unwrap(),
            u64::try_from(page_prefix.len() + 2).unwrap(),
            u64::try_from(page_prefix.len() + 2).unwrap(),
            0,
            false,
            true,
        )
        .unwrap(),
    );
    let first_tool = reading_tool(temporary.path(), &executor, &starter, &first_reader);
    let first = poll_ready(first_tool.execute(
        context(),
        json!({
            "action": "read",
            "background_id": 7,
            "cursor_segment": 1,
            "cursor_offset": 0
        }),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(first.content["output"], "a".repeat(page_prefix.len()));
    assert_eq!(first.content["lossy"], false);

    let second_reader = FakeBackgroundOutputReader::snapshot(
        TerminalBackgroundReadSnapshot::new(
            "é".as_bytes().to_vec(),
            u64::try_from(page_prefix.len() + 2).unwrap(),
            u64::try_from(page_prefix.len() + 2).unwrap(),
            u64::try_from(page_prefix.len() + 2).unwrap(),
            0,
            false,
            true,
        )
        .unwrap(),
    );
    let second_tool = reading_tool(temporary.path(), &executor, &starter, &second_reader);
    let second = poll_ready(second_tool.execute(
        context(),
        json!({
            "action": "read",
            "background_id": 7,
            "cursor_segment": 1,
            "cursor_offset": page_prefix.len()
        }),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(second.content["output"], "é");
    assert_eq!(second.content["lossy"], false);

    let partial_reader = FakeBackgroundOutputReader::snapshot(
        TerminalBackgroundReadSnapshot::new(Vec::new(), 0, 1, 1, 1, false, false).unwrap(),
    );
    let partial_tool = reading_tool(temporary.path(), &executor, &starter, &partial_reader);
    let partial = poll_ready(partial_tool.execute(
        context(),
        json!({
            "action": "read",
            "background_id": 7,
            "cursor_segment": 1,
            "cursor_offset": 0
        }),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(partial.content["output"], "");
    assert_eq!(partial.content["cursor_offset"], 0);
    assert_eq!(partial.content["lossy"], false);
    assert_eq!(partial.content["stream_closed"], false);

    let completed_reader = FakeBackgroundOutputReader::snapshot(
        TerminalBackgroundReadSnapshot::new("é".as_bytes().to_vec(), 2, 2, 2, 0, false, false)
            .unwrap(),
    );
    let completed_tool = reading_tool(temporary.path(), &executor, &starter, &completed_reader);
    let completed = poll_ready(completed_tool.execute(
        context(),
        json!({
            "action": "read",
            "background_id": 7,
            "cursor_segment": 1,
            "cursor_offset": 0
        }),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(completed.content["output"], "é");
    assert_eq!(completed.content["cursor_offset"], 2);
    assert_eq!(completed.content["lossy"], false);
}

#[test]
fn background_read_rejects_malformed_reader_pages() {
    for malformed in [
        TerminalBackgroundReadSnapshot::new(Vec::new(), 0, 1, 1, 0, false, false),
        TerminalBackgroundReadSnapshot::new(vec![b'a', b'b'], 2, 2, 1, 0, true, true),
        TerminalBackgroundReadSnapshot::new(Vec::new(), 0, 1, 1, 1, false, true),
        TerminalBackgroundReadSnapshot::new(vec![0xc3], 1, 2, 2, 0, false, false),
        TerminalBackgroundReadSnapshot::new(vec![0xc3], 1, 1, 1, 0, false, false),
        TerminalBackgroundReadSnapshot::new(vec![0xc3], 1, 1, 1, 0, true, false),
    ] {
        assert_eq!(
            malformed.unwrap_err().kind(),
            TerminalBackgroundReadErrorKind::Unavailable
        );
    }

    let temporary = TemporaryDirectory::new("background-read-malformed");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let reader = FakeBackgroundOutputReader::snapshot(
        TerminalBackgroundReadSnapshot::new(vec![b'x'], 2, 2, 2, 0, false, true).unwrap(),
    );
    let tool = reading_tool(temporary.path(), &executor, &starter, &reader);
    let error = poll_ready(tool.execute(
        context(),
        json!({
            "action": "read",
            "background_id": 7,
            "cursor_segment": 1,
            "cursor_offset": 0
        }),
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_eq!(error.code, "terminal_read_resource_limit");
    assert!(!error.retryable);
}

#[test]
fn background_read_preserves_conservative_truncation_metadata() {
    let temporary = TemporaryDirectory::new("background-read-incomplete");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let reader = FakeBackgroundOutputReader::snapshot(
        TerminalBackgroundReadSnapshot::new(b"seen".to_vec(), 4, 4, 4, 0, true, true).unwrap(),
    );
    let tool = reading_tool(temporary.path(), &executor, &starter, &reader);

    let output = poll_ready(tool.execute(
        context(),
        json!({
            "action": "read",
            "background_id": 7,
            "cursor_segment": 1,
            "cursor_offset": 0
        }),
        CancellationToken::new(),
    ))
    .unwrap();

    assert_eq!(output.content["output"], "seen");
    assert_eq!(output.content["output_bytes"], 4);
    assert_eq!(output.content["retained_bytes"], 4);
    assert_eq!(output.content["truncated"], true);
    assert_eq!(output.content["stream_closed"], true);
}

#[test]
fn background_read_rejects_bad_shape_and_maps_fixed_reader_failures() {
    let temporary = TemporaryDirectory::new("background-read-errors");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    for arguments in [
        json!({ "action": "read", "background_id": 7 }),
        json!({ "action": "read", "background_id": 0, "cursor_segment": 1 }),
        json!({ "action": "read", "background_id": 7, "cursor_segment": 2 }),
        json!({ "action": "read", "background_id": 7, "cursor_segment": 1, "cursor_offset": -1 }),
        json!({ "action": "read", "background_id": 7, "cursor_segment": 1, "extra": true }),
    ] {
        let reader = FakeBackgroundOutputReader::success(Vec::new(), false);
        let tool = reading_tool(temporary.path(), &executor, &starter, &reader);
        assert_eq!(
            tool.prepare(call("terminal", arguments)).unwrap_err().code,
            "terminal_invalid_arguments"
        );
    }

    let cases = [
        (
            TerminalBackgroundReadErrorKind::NotFound,
            "terminal_read_not_found",
            false,
        ),
        (
            TerminalBackgroundReadErrorKind::InvalidCursor,
            "terminal_read_invalid_cursor",
            false,
        ),
        (
            TerminalBackgroundReadErrorKind::Unavailable,
            "terminal_read_unavailable",
            true,
        ),
    ];
    for (kind, code, retryable) in cases {
        let reader = FakeBackgroundOutputReader::error(kind);
        let tool = reading_tool(temporary.path(), &executor, &starter, &reader);
        let error = poll_ready(tool.execute(
            context(),
            json!({
                "action": "read",
                "background_id": 7,
                "cursor_segment": 1,
                "cursor_offset": 0
            }),
            CancellationToken::new(),
        ))
        .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.retryable, retryable);
    }
}

#[test]
fn background_read_worst_case_json_page_stays_within_result_limit() {
    let temporary = TemporaryDirectory::new("background-read-json-bound");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let bytes = vec![0; MAX_TERMINAL_BACKGROUND_READ_BYTES];
    let reader = FakeBackgroundOutputReader::success(bytes, true);
    let tool = reading_tool(temporary.path(), &executor, &starter, &reader);

    let output = poll_ready(tool.execute(
        context(),
        json!({
            "action": "read",
            "background_id": 7,
            "cursor_segment": 1,
            "cursor_offset": 0
        }),
        CancellationToken::new(),
    ))
    .expect("the maximum all-control-byte page fits the serialized result bound");
    assert_eq!(
        output.content["output"].as_str().unwrap().len(),
        MAX_TERMINAL_BACKGROUND_READ_BYTES
    );
    assert_eq!(
        output.content["cursor_offset"],
        u64::try_from(MAX_TERMINAL_BACKGROUND_READ_BYTES).unwrap()
    );
}

#[test]
fn background_read_capacity_is_fail_fast_and_recovers_on_future_drop() {
    let temporary = TemporaryDirectory::new("background-read-capacity");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let reader = FakeBackgroundOutputReader::pending();
    let tool = reading_tool(temporary.path(), &executor, &starter, &reader);
    let arguments = json!({
        "action": "read",
        "background_id": 7,
        "cursor_segment": 1,
        "cursor_offset": 0
    });
    let mut active = (0..4)
        .map(|_| tool.execute(context(), arguments.clone(), CancellationToken::new()))
        .collect::<Vec<_>>();
    for future in &mut active {
        assert!(poll_once(future.as_mut()).is_pending());
    }
    let busy = poll_ready(tool.execute(context(), arguments.clone(), CancellationToken::new()))
        .unwrap_err();
    assert_eq!(busy.code, "terminal_read_busy");
    assert!(busy.retryable);

    drop(active.pop());
    let mut replacement = tool.execute(context(), arguments, CancellationToken::new());
    assert!(poll_once(replacement.as_mut()).is_pending());
    drop(replacement);
    drop(active);
}

#[test]
fn background_read_cancellation_wakes_drops_and_recovers_exact_capacity() {
    let temporary = TemporaryDirectory::new("background-read-cancel");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let reader = PendingBackgroundOutputReader::default();
    let tool = reading_tool(temporary.path(), &executor, &starter, &reader);
    let arguments = json!({
        "action": "read",
        "background_id": 7,
        "cursor_segment": 1,
        "cursor_offset": 0
    });
    let cancellation = CancellationToken::new();
    let observed = Arc::new(ObservedWake::default());
    let waker = Waker::from(Arc::clone(&observed));
    let mut cancelled = tool.execute(context(), arguments.clone(), cancellation.clone());
    assert!(poll_with_waker(cancelled.as_mut(), &waker).is_pending());
    let mut active = (0..3)
        .map(|_| tool.execute(context(), arguments.clone(), CancellationToken::new()))
        .collect::<Vec<_>>();
    for future in &mut active {
        assert!(poll_once(future.as_mut()).is_pending());
    }
    assert_eq!(reader.polls.load(Ordering::SeqCst), 4);
    let busy = poll_ready(tool.execute(context(), arguments.clone(), CancellationToken::new()))
        .unwrap_err();
    assert_eq!(busy.code, "terminal_read_busy");

    assert!(cancellation.cancel());
    observed.wait_for_calls(1);
    let error = match poll_with_waker(cancelled.as_mut(), &waker) {
        Poll::Ready(Err(error)) => error,
        Poll::Ready(Ok(_)) => panic!("cancelled read returned output"),
        Poll::Pending => panic!("cancelled read remained pending"),
    };
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");
    assert_eq!(reader.drops.load(Ordering::SeqCst), 1);

    let mut replacement = tool.execute(context(), arguments, CancellationToken::new());
    assert!(poll_once(replacement.as_mut()).is_pending());
    drop(replacement);
    drop(active);
    assert_eq!(reader.drops.load(Ordering::SeqCst), 5);
}

#[test]
fn background_read_same_poll_cancellation_wins_ready_snapshot() {
    let temporary = TemporaryDirectory::new("background-read-ready-cancel");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let reader = ReadyCancellingBackgroundOutputReader {
        snapshot: TerminalBackgroundReadSnapshot::new(
            b"must-not-publish".to_vec(),
            16,
            16,
            16,
            0,
            false,
            true,
        )
        .unwrap(),
    };
    let tool = reading_tool(temporary.path(), &executor, &starter, &reader);

    let error = poll_ready(tool.execute(
        context(),
        json!({
            "action": "read",
            "background_id": 7,
            "cursor_segment": 1,
            "cursor_offset": 0
        }),
        CancellationToken::new(),
    ))
    .unwrap_err();

    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");
}

#[test]
fn background_inspect_is_exact_inert_and_requires_no_authority() {
    let temporary = TemporaryDirectory::new("background-inspect");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = FakeBackgroundInspector::new(false);
    let tool = inspecting_tool(temporary.path(), &executor, &starter, &inspector);

    let forms = tool.spec().input_schema["oneOf"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(forms.len(), 3);
    assert_eq!(forms[2]["properties"]["action"]["const"], "inspect");
    assert_eq!(forms[2]["required"], json!(["action", "background_id"]));
    let prepared = tool
        .prepare(call(
            "terminal",
            json!({ "action": "inspect", "background_id": 41 }),
        ))
        .unwrap();
    assert!(prepared.capability().is_none());
    assert_eq!(
        prepared.arguments(),
        &json!({ "action": "inspect", "background_id": 41 })
    );
    assert_eq!(inspector.calls(), 0);

    let mut future = Box::pin(tool.execute(
        context(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    ));
    assert_eq!(inspector.calls(), 0);
    let output = poll_ready(future.as_mut()).unwrap();
    assert_eq!(inspector.calls(), 1);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
    assert_eq!(
        output.content,
        json!({
            "action": "inspect",
            "background_id": 41,
            "recorded_state": "running",
            "started_at_ms": 10,
            "updated_at_ms": 20,
            "pid": 1234,
            "exit_code": null
        })
    );
    let encoded = serde_json::to_vec(&output.content).unwrap();
    assert!(encoded.len() <= machine_god_native::MAX_TERMINAL_SERIALIZED_RESULT_BYTES);
    let rendered = output.content.to_string();
    assert!(!rendered.contains("private command"));
    assert!(!rendered.contains("private/workspace"));
    assert!(!rendered.contains("private.invalid"));
    assert!(!rendered.contains("private diagnostic"));
}

#[test]
fn background_inspect_cancellation_and_strict_forms_bound_effects() {
    let temporary = TemporaryDirectory::new("background-inspect-cancel");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = FakeBackgroundInspector::new(false);
    let tool = inspecting_tool(temporary.path(), &executor, &starter, &inspector);
    for invalid in [
        json!({ "action": "inspect", "background_id": 0 }),
        json!({ "action": "inspect", "background_id": 1, "command": ":" }),
        json!({ "action": "inspect" }),
        json!({ "action": "inspect", "background_id": "1" }),
    ] {
        assert_invalid_input(&tool.prepare(call("terminal", invalid)).unwrap_err());
    }
    assert_eq!(inspector.calls(), 0);

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    let error = execute(
        &tool,
        json!({ "action": "inspect", "background_id": 7 }),
        cancellation,
    )
    .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(inspector.calls(), 0);

    let cancelling = FakeBackgroundInspector::new(true);
    let tool = inspecting_tool(temporary.path(), &executor, &starter, &cancelling);
    let error = execute(
        &tool,
        json!({ "action": "inspect", "background_id": 8 }),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(cancelling.calls(), 1);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn dropping_pending_background_inspect_drops_the_injected_future() {
    let temporary = TemporaryDirectory::new("background-inspect-drop");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = PendingBackgroundInspector {
        polls: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    };
    let tool = inspecting_tool(temporary.path(), &executor, &starter, &inspector);
    let mut future = Box::pin(tool.execute(
        context(),
        json!({ "action": "inspect", "background_id": 9 }),
        CancellationToken::new(),
    ));
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(inspector.polls.load(Ordering::SeqCst), 1);
    assert!(!inspector.dropped.load(Ordering::SeqCst));
    drop(future);
    assert!(inspector.dropped.load(Ordering::SeqCst));
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn cancellation_wakes_and_drops_a_pending_background_inspect() {
    let temporary = TemporaryDirectory::new("background-inspect-pending-cancel");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = PendingBackgroundInspector {
        polls: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    };
    let tool = inspecting_tool(temporary.path(), &executor, &starter, &inspector);
    let cancellation = CancellationToken::new();
    let observed = Arc::new(ObservedWake::default());
    let waker = Waker::from(Arc::clone(&observed));
    let mut future = Box::pin(tool.execute(
        context(),
        json!({ "action": "inspect", "background_id": 9 }),
        cancellation.clone(),
    ));

    assert!(poll_with_waker(future.as_mut(), &waker).is_pending());
    assert_eq!(inspector.polls.load(Ordering::SeqCst), 1);
    assert!(cancellation.cancel());
    observed.wait_for_calls(1);
    let error = match poll_with_waker(future.as_mut(), &waker) {
        Poll::Ready(Err(error)) => error,
        Poll::Ready(Ok(_)) => panic!("cancelled inspection returned output"),
        Poll::Pending => panic!("cancelled inspection remained pending"),
    };
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");
    assert!(inspector.dropped.load(Ordering::SeqCst));
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn cancellation_from_ready_inspector_drop_wins_before_output_publication() {
    let temporary = TemporaryDirectory::new("background-inspect-drop-cancel");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let dropped = Arc::new(AtomicBool::new(false));
    let inspector = DropCancellingBackgroundInspector {
        dropped: Arc::clone(&dropped),
    };
    let tool = inspecting_tool(temporary.path(), &executor, &starter, &inspector);

    let error = execute(
        &tool,
        json!({ "action": "inspect", "background_id": 9 }),
        CancellationToken::new(),
    )
    .unwrap_err();

    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");
    assert_eq!(error.message, "terminal execution was cancelled");
    assert!(!error.retryable);
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn cancellation_from_waiter_waker_drop_wins_before_output_publication() {
    let temporary = TemporaryDirectory::new("background-inspect-waker-drop-cancel");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = FakeBackgroundInspector::new(false);
    let tool = inspecting_tool(temporary.path(), &executor, &starter, &inspector);
    let cancellation = CancellationToken::new();
    let reentrant_cancellation = cancellation.clone();
    let (waker, handle) = reentrant_waker(Callback::Drop, move || {
        let _ = reentrant_cancellation.cancel();
    });
    let mut future = Box::pin(tool.execute(
        context(),
        json!({ "action": "inspect", "background_id": 9 }),
        cancellation,
    ));

    let error = match poll_with_waker(future.as_mut(), &waker) {
        Poll::Ready(Err(error)) => error,
        Poll::Ready(Ok(_)) => panic!("waiter teardown cancellation published output"),
        Poll::Pending => panic!("ready inspection remained pending"),
    };

    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");
    assert_eq!(error.message, "terminal execution was cancelled");
    assert!(!error.retryable);
    assert!(handle.calls() >= 1);
    assert_eq!(inspector.calls(), 1);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn background_inspect_native_failures_have_fixed_redacted_mappings() {
    let temporary = TemporaryDirectory::new("background-inspect-errors");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let cases = [
        (
            NativeBackgroundInspectionErrorKind::NotFound,
            "terminal_background_not_found",
            false,
        ),
        (
            NativeBackgroundInspectionErrorKind::Corrupt,
            "terminal_background_corrupt",
            false,
        ),
        (
            NativeBackgroundInspectionErrorKind::ResourceLimit,
            "terminal_inspect_resource_limit",
            false,
        ),
        (
            NativeBackgroundInspectionErrorKind::Unavailable,
            "terminal_inspect_unavailable",
            true,
        ),
        (
            NativeBackgroundInspectionErrorKind::UnsupportedPlatform,
            "terminal_unsupported",
            false,
        ),
    ];
    for (kind, code, retryable) in cases {
        let inspector = FakeBackgroundInspector::with_error(kind);
        let tool = inspecting_tool(temporary.path(), &executor, &starter, &inspector);
        let error = execute(
            &tool,
            json!({ "action": "inspect", "background_id": 9 }),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.retryable, retryable);
        assert!(!format!("{error:?}").contains("private"));
        assert_eq!(inspector.calls(), 1);
    }
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

fn assert_list_schema_and_prepare_are_exact(
    tool: &TerminalTool,
    catalog: &FakeBackgroundCatalog,
) -> Value {
    let spec = tool.spec();
    assert_eq!(
        spec.description,
        "Run a foreground command, start a background command, list persisted background records, inspect one persisted background record, or wait for its recorded exit"
    );
    let forms = spec.input_schema["oneOf"]
        .as_array()
        .expect("all five terminal action forms are advertised");
    assert_eq!(forms.len(), 5);
    assert_eq!(forms[4], terminal_list_schema());

    let prepared = tool
        .prepare(call("terminal", json!({ "action": "list" })))
        .unwrap();
    assert!(prepared.capability().is_none());
    assert_eq!(prepared.arguments(), &json!({ "action": "list" }));
    for invalid in [
        json!({ "action": "list", "background_id": 7 }),
        json!({ "action": "list", "command": ":" }),
        json!({ "action": "list", "extra": true }),
    ] {
        assert_invalid_input(&tool.prepare(call("terminal", invalid)).unwrap_err());
    }
    assert_eq!(catalog.calls(), 0);
    prepared.arguments().clone()
}

fn terminal_list_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "action": { "const": "list" } },
        "required": ["action"],
        "additionalProperties": false
    })
}

#[test]
fn background_list_has_one_strict_closed_no_authority_form_and_redacted_bounded_output() {
    let temporary = TemporaryDirectory::new("background-list-contract");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector =
        SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Exited, Some(0))]);
    let delay = SleepingWaitDelay::default();
    let first_private_preview = "PRIVATE_LIST_COMMAND_ONE_DO_NOT_REFLECT";
    let second_private_preview = "PRIVATE_LIST_COMMAND_TWO_DO_NOT_REFLECT";
    let catalog = FakeBackgroundCatalog::ready(background_listing(
        vec![
            background_summary(
                9,
                NativeBackgroundState::Exited,
                30,
                first_private_preview,
                true,
            ),
            background_summary(
                7,
                NativeBackgroundState::Running,
                20,
                second_private_preview,
                false,
            ),
        ],
        true,
    ));
    let tool = cataloging_tool(
        temporary.path(),
        &executor,
        &starter,
        &inspector,
        &delay,
        &catalog,
    );

    let arguments = assert_list_schema_and_prepare_are_exact(&tool, &catalog);

    let output = execute(&tool, arguments, CancellationToken::new()).unwrap();
    assert!(!output.is_error);
    assert_eq!(
        output.content,
        json!({
            "action": "list",
            "count": 2,
            "truncated": true,
            "records": [
                {
                    "background_id": 9,
                    "recorded_state": "exited",
                    "updated_at_ms": 30
                },
                {
                    "background_id": 7,
                    "recorded_state": "running",
                    "updated_at_ms": 20
                }
            ]
        })
    );
    let encoded = serde_json::to_vec(&output.content).unwrap();
    assert!(encoded.len() <= machine_god_native::MAX_TERMINAL_SERIALIZED_RESULT_BYTES);
    let rendered = output.content.to_string();
    assert!(!rendered.contains(first_private_preview));
    assert!(!rendered.contains(second_private_preview));
    assert_eq!(catalog.calls(), 1);
    assert_eq!(inspector.calls(), 0);
    assert_eq!(delay.polls.load(Ordering::SeqCst), 0);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);

    let tool_without_catalog =
        waiting_tool(temporary.path(), &executor, &starter, &inspector, &delay);
    assert_invalid_input(
        &tool_without_catalog
            .prepare(call("terminal", json!({ "action": "list" })))
            .unwrap_err(),
    );
    assert_eq!(catalog.calls(), 1);
}

#[test]
fn background_list_and_inspect_description_matches_advertised_actions_without_wait() {
    let temporary = TemporaryDirectory::new("background-list-inspect-schema");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = FakeBackgroundInspector::new(false);
    let catalog = FakeBackgroundCatalog::ready(background_listing(Vec::new(), false));
    let tool = inspecting_tool(temporary.path(), &executor, &starter, &inspector)
        .with_catalog(Arc::new(catalog))
        .unwrap();

    let spec = tool.spec();
    assert_eq!(
        spec.description,
        "Run a foreground command, start a background command, list persisted background records, or inspect one persisted background record"
    );
    let forms = spec.input_schema["oneOf"].as_array().unwrap();
    assert_eq!(forms.len(), 4);
    assert_eq!(forms[2]["properties"]["action"]["const"], "inspect");
    assert_eq!(forms[3], terminal_list_schema());
}

#[test]
fn combined_description_names_every_advertised_action() {
    let temporary = TemporaryDirectory::new("background-all-actions-schema");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = FakeBackgroundInspector::new(false);
    let delay = SleepingWaitDelay::default();
    let catalog = FakeBackgroundCatalog::ready(background_listing(Vec::new(), false));
    let reader = FakeBackgroundOutputReader::success(Vec::new(), false);
    let tool = cataloging_tool(
        temporary.path(),
        &executor,
        &starter,
        &inspector,
        &delay,
        &catalog,
    )
    .with_output_reader(Arc::new(reader))
    .unwrap();

    let spec = tool.spec();
    assert_eq!(
        spec.description,
        "Run a foreground command, start a background command, read bounded same-session background output, list persisted background records, inspect one persisted background record, or wait for its recorded exit"
    );
    let actions = spec.input_schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|form| form["properties"]["action"]["const"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        actions,
        ["exec", "start", "read", "inspect", "wait", "list"]
    );
}

#[test]
fn background_list_native_failures_have_fixed_redacted_mappings() {
    let temporary = TemporaryDirectory::new("background-list-errors");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector =
        SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Exited, Some(0))]);
    let delay = SleepingWaitDelay::default();
    let cases = [
        (
            NativeBackgroundInspectionErrorKind::NotFound,
            ToolErrorKind::Execution,
            "terminal_lister_failed",
            false,
        ),
        (
            NativeBackgroundInspectionErrorKind::Corrupt,
            ToolErrorKind::Execution,
            "terminal_background_corrupt",
            false,
        ),
        (
            NativeBackgroundInspectionErrorKind::ResourceLimit,
            ToolErrorKind::Unavailable,
            "terminal_list_resource_limit",
            false,
        ),
        (
            NativeBackgroundInspectionErrorKind::Unavailable,
            ToolErrorKind::Unavailable,
            "terminal_list_unavailable",
            true,
        ),
        (
            NativeBackgroundInspectionErrorKind::UnsupportedPlatform,
            ToolErrorKind::Unavailable,
            "terminal_unsupported",
            false,
        ),
    ];
    for (kind, expected_kind, code, retryable) in cases {
        let catalog = FakeBackgroundCatalog::with_error(kind);
        let tool = cataloging_tool(
            temporary.path(),
            &executor,
            &starter,
            &inspector,
            &delay,
            &catalog,
        );
        let error =
            execute(&tool, json!({ "action": "list" }), CancellationToken::new()).unwrap_err();
        assert_eq!(error.kind, expected_kind);
        assert_eq!(error.code, code);
        assert_eq!(error.retryable, retryable);
        match kind {
            NativeBackgroundInspectionErrorKind::NotFound => {
                assert_eq!(error.message, "terminal background lister failed");
            }
            NativeBackgroundInspectionErrorKind::ResourceLimit => {
                assert_eq!(
                    error.message,
                    "terminal background listing reached a resource limit"
                );
            }
            NativeBackgroundInspectionErrorKind::Unavailable => {
                assert_eq!(error.message, "terminal background listing is unavailable");
            }
            _ => {}
        }
        assert!(!error.message.contains("private"));
        assert!(!format!("{error:?}").contains("private"));
        assert_eq!(catalog.calls(), 1);
    }
    assert_eq!(inspector.calls(), 0);
    assert_eq!(delay.polls.load(Ordering::SeqCst), 0);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn background_list_constructors_reject_invalid_identity_order_and_duplicates() {
    assert!(
        NativeBackgroundRecordSummary::new(
            0,
            NativeBackgroundState::Running,
            10,
            "private command".to_owned(),
            false,
        )
        .is_err()
    );

    let older = background_summary(
        7,
        NativeBackgroundState::Running,
        20,
        "older private command",
        false,
    );
    let newer = background_summary(
        9,
        NativeBackgroundState::Exited,
        30,
        "newer private command",
        false,
    );
    assert!(NativeBackgroundList::new(vec![older.clone(), newer], false).is_err());
    assert!(NativeBackgroundList::new(vec![older.clone(), older], false).is_err());
}

#[test]
fn background_list_cancellation_bounds_calls_and_drops_pending_catalog_future() {
    let temporary = TemporaryDirectory::new("background-list-cancellation");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector =
        SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Exited, Some(0))]);
    let delay = SleepingWaitDelay::default();
    let catalog = FakeBackgroundCatalog::pending();
    let tool = cataloging_tool(
        temporary.path(),
        &executor,
        &starter,
        &inspector,
        &delay,
        &catalog,
    );
    let cancellation = CancellationToken::new();
    let observed = Arc::new(ObservedWake::default());
    let waker = Waker::from(Arc::clone(&observed));
    let mut future = tool.execute(context(), json!({ "action": "list" }), cancellation.clone());

    assert_eq!(catalog.calls(), 0);
    assert!(poll_with_waker(future.as_mut(), &waker).is_pending());
    assert_eq!(catalog.calls(), 1);
    assert_eq!(catalog.polls(), 1);
    assert_eq!(catalog.drops(), 0);
    assert!(cancellation.cancel());
    observed.wait_for_calls(1);
    let error = match poll_with_waker(future.as_mut(), &waker) {
        Poll::Ready(Err(error)) => error,
        Poll::Ready(Ok(_)) => panic!("cancelled list returned output"),
        Poll::Pending => panic!("cancelled list remained pending"),
    };
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");
    assert_eq!(catalog.drops(), 1);

    let mut dropped = tool.execute(
        context(),
        json!({ "action": "list" }),
        CancellationToken::new(),
    );
    assert!(poll_once(dropped.as_mut()).is_pending());
    assert_eq!(catalog.calls(), 2);
    assert_eq!(catalog.polls(), 2);
    drop(dropped);
    assert_eq!(catalog.drops(), 2);

    let pre_cancelled = CancellationToken::new();
    assert!(pre_cancelled.cancel());
    let error = execute(&tool, json!({ "action": "list" }), pre_cancelled).unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(catalog.calls(), 2);
    assert_eq!(inspector.calls(), 0);
    assert_eq!(delay.polls.load(Ordering::SeqCst), 0);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn cancellation_from_ready_catalog_wins_before_background_list_output_publication() {
    let temporary = TemporaryDirectory::new("background-list-ready-cancel");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector =
        SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Exited, Some(0))]);
    let delay = SleepingWaitDelay::default();
    let catalog = FakeBackgroundCatalog::cancelling(background_listing(Vec::new(), false));
    let tool = cataloging_tool(
        temporary.path(),
        &executor,
        &starter,
        &inspector,
        &delay,
        &catalog,
    );

    let error = execute(&tool, json!({ "action": "list" }), CancellationToken::new()).unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");
    assert_eq!(catalog.calls(), 1);
    assert_eq!(inspector.calls(), 0);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn background_list_has_four_independent_slots_and_recovers_after_drop() {
    let temporary = TemporaryDirectory::new("background-list-capacity");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector =
        SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Exited, Some(0))]);
    let delay = SleepingWaitDelay::default();
    let catalog = FakeBackgroundCatalog::pending();
    let tool = cataloging_tool(
        temporary.path(),
        &executor,
        &starter,
        &inspector,
        &delay,
        &catalog,
    );

    let mut active = (0..TERMINAL_MAX_ACTIVE_LISTS)
        .map(|_| {
            tool.execute(
                context(),
                json!({ "action": "list" }),
                CancellationToken::new(),
            )
        })
        .collect::<Vec<_>>();
    for future in &mut active {
        assert!(poll_once(future.as_mut()).is_pending());
    }
    assert_eq!(catalog.calls(), TERMINAL_MAX_ACTIVE_LISTS);
    assert_eq!(catalog.polls(), TERMINAL_MAX_ACTIVE_LISTS);

    let busy = execute(&tool, json!({ "action": "list" }), CancellationToken::new()).unwrap_err();
    assert_eq!(busy.kind, ToolErrorKind::Unavailable);
    assert_eq!(busy.code, "terminal_list_busy");
    assert!(busy.retryable);
    assert_eq!(catalog.calls(), TERMINAL_MAX_ACTIVE_LISTS);

    let released = active.pop().unwrap();
    drop(released);
    assert_eq!(catalog.drops(), 1);
    let mut replacement = tool.execute(
        context(),
        json!({ "action": "list" }),
        CancellationToken::new(),
    );
    assert!(poll_once(replacement.as_mut()).is_pending());
    assert_eq!(catalog.calls(), TERMINAL_MAX_ACTIVE_LISTS + 1);
    assert_eq!(catalog.polls(), TERMINAL_MAX_ACTIVE_LISTS + 1);

    drop(replacement);
    drop(active);
    assert_eq!(catalog.drops(), TERMINAL_MAX_ACTIVE_LISTS + 1);
    assert_eq!(inspector.calls(), 0);
    assert_eq!(delay.polls.load(Ordering::SeqCst), 0);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn background_wait_has_one_strict_closed_no_authority_form() {
    let temporary = TemporaryDirectory::new("background-wait-schema");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector =
        SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Exited, Some(0))]);
    let delay = SleepingWaitDelay::default();
    let tool = waiting_tool(temporary.path(), &executor, &starter, &inspector, &delay);

    let spec = tool.spec();
    assert_eq!(
        spec.description,
        "Run a foreground command, start a background command, inspect one persisted background record, or wait for its recorded exit"
    );
    let forms = spec.input_schema["oneOf"].as_array().unwrap();
    assert_eq!(forms.len(), 4);
    assert_eq!(forms[3]["properties"]["action"]["const"], "wait");
    assert_eq!(forms[3]["properties"]["wait_ceiling_ms"]["maximum"], 30_000);
    assert_eq!(
        forms[3]["required"],
        json!(["action", "background_id", "return_when", "wait_ceiling_ms"])
    );
    assert_eq!(forms[3]["additionalProperties"], false);

    let prepared = tool
        .prepare(call("terminal", exact_wait_arguments(7, 30_000)))
        .unwrap();
    assert!(prepared.capability().is_none());
    assert_eq!(prepared.arguments(), &exact_wait_arguments(7, 30_000));
    assert_eq!(inspector.calls(), 0);
    assert_eq!(executor.calls(), 0);
    assert_eq!(starter.calls(), 0);

    for invalid in [
        json!({"action":"wait","background_id":7,"return_when":{"kind":"exit"}}),
        json!({"action":"wait","background_id":0,"return_when":{"kind":"exit"},"wait_ceiling_ms":1}),
        json!({"action":"wait","background_id":7,"return_when":{"kind":"match"},"wait_ceiling_ms":1}),
        json!({"action":"wait","background_id":7,"return_when":{"kind":"exit","pattern":"x"},"wait_ceiling_ms":1}),
        json!({"action":"wait","background_id":7,"return_when":{"kind":"exit"},"wait_ceiling_ms":0}),
        json!({"action":"wait","background_id":7,"return_when":{"kind":"exit"},"wait_ceiling_ms":30_001}),
        json!({"action":"wait","background_id":7,"return_when":{"kind":"exit"},"wait_ceiling_ms":1,"extra":true}),
    ] {
        assert_invalid_input(&tool.prepare(call("terminal", invalid)).unwrap_err());
    }
    assert_eq!(inspector.calls(), 0);
}

#[test]
fn background_wait_immediate_exit_transition_and_ceiling_are_exact() {
    let temporary = TemporaryDirectory::new("background-wait-outcomes");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let delay = SleepingWaitDelay::default();

    let immediate =
        SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Failed, Some(23))]);
    let tool = waiting_tool(temporary.path(), &executor, &starter, &immediate, &delay);
    let output = execute(
        &tool,
        exact_wait_arguments(7, 30_000),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        output.content,
        json!({
            "action": "wait",
            "background_id": 7,
            "outcome": { "exited": 23 },
            "recorded_state": "failed",
            "started_at_ms": 10,
            "updated_at_ms": 20,
            "pid": 1234,
            "exit_code": 23
        })
    );
    assert_eq!(immediate.calls(), 1);
    assert_eq!(delay.polls.load(Ordering::SeqCst), 0);

    let transition = SequenceBackgroundInspector::new(vec![
        (NativeBackgroundState::Running, None),
        (NativeBackgroundState::Exited, Some(0)),
    ]);
    let tool = waiting_tool(temporary.path(), &executor, &starter, &transition, &delay);
    let output = execute(
        &tool,
        exact_wait_arguments(8, 30_000),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(output.content["outcome"], json!({"exited": 0}));
    assert_eq!(output.content["recorded_state"], "exited");
    assert_eq!(transition.calls(), 2);

    let running = SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Running, None)]);
    let tool = waiting_tool(temporary.path(), &executor, &starter, &running, &delay);
    let output = execute(&tool, exact_wait_arguments(9, 1), CancellationToken::new()).unwrap();
    assert_eq!(output.content["outcome"], json!({"safety_ceiling": {}}));
    assert_eq!(output.content["recorded_state"], "running");
    assert_eq!(running.calls(), 1);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn background_wait_ceiling_wins_after_overlong_inspection_without_a_post_ceiling_observation() {
    let temporary = TemporaryDirectory::new("background-wait-inspection-ceiling");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let delay = SleepingWaitDelay::default();

    let first_overruns = DelayedSequenceBackgroundInspector::new(vec![(
        NativeBackgroundState::Exited,
        Some(0),
        Duration::from_millis(5),
    )]);
    let tool = waiting_tool(
        temporary.path(),
        &executor,
        &starter,
        &first_overruns,
        &delay,
    );
    let output = execute(&tool, exact_wait_arguments(10, 1), CancellationToken::new()).unwrap();
    assert_eq!(output.content["outcome"], json!({"safety_ceiling": {}}));
    assert_eq!(output.content["recorded_state"], "exited");
    assert_eq!(output.content["exit_code"], 0);
    assert_eq!(first_overruns.calls(), 1);

    let transition_overruns = DelayedSequenceBackgroundInspector::new(vec![
        (NativeBackgroundState::Running, None, Duration::ZERO),
        (
            NativeBackgroundState::Exited,
            Some(0),
            Duration::from_millis(500),
        ),
    ]);
    let tool = waiting_tool(
        temporary.path(),
        &executor,
        &starter,
        &transition_overruns,
        &delay,
    );
    let output = execute(
        &tool,
        exact_wait_arguments(11, 500),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(output.content["outcome"], json!({"safety_ceiling": {}}));
    assert_eq!(output.content["recorded_state"], "exited");
    assert_eq!(output.content["updated_at_ms"], 21);
    assert_eq!(output.content["exit_code"], 0);
    assert_eq!(transition_overruns.calls(), 2);
}

#[test]
fn background_wait_uses_the_frozen_monotonic_backoff_schedule() {
    let temporary = TemporaryDirectory::new("background-wait-backoff");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = SequenceBackgroundInspector::new(vec![
        (NativeBackgroundState::Running, None),
        (NativeBackgroundState::Running, None),
        (NativeBackgroundState::Running, None),
        (NativeBackgroundState::Running, None),
        (NativeBackgroundState::Running, None),
        (NativeBackgroundState::Exited, Some(0)),
    ]);
    let delay = SleepingWaitDelay::default();
    let before_wait = Instant::now();
    let tool = waiting_tool(temporary.path(), &executor, &starter, &inspector, &delay);

    let output = execute(
        &tool,
        exact_wait_arguments(7, 2_000),
        CancellationToken::new(),
    )
    .unwrap();

    assert_eq!(output.content["outcome"], json!({"exited": 0}));
    assert_eq!(inspector.calls(), 6);
    let deadlines = delay.deadlines.lock().unwrap();
    assert_eq!(deadlines.len(), 5);
    assert!(deadlines[0].duration_since(before_wait) >= Duration::from_millis(16));
    for (observed, minimum) in deadlines.windows(2).zip([32_u64, 64, 128, 250].into_iter()) {
        assert!(observed[1].duration_since(observed[0]) >= Duration::from_millis(minimum));
    }
}

#[test]
fn background_wait_lost_delay_and_cancellation_fail_redacted() {
    let temporary = TemporaryDirectory::new("background-wait-errors");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    for (state, code) in [
        (NativeBackgroundState::Stopped, None),
        (NativeBackgroundState::Dead, None),
        (NativeBackgroundState::Stale, None),
        (NativeBackgroundState::Failed, Some(256)),
        (NativeBackgroundState::Failed, Some(-1)),
    ] {
        let inspector = SequenceBackgroundInspector::new(vec![(state, code)]);
        let delay = SleepingWaitDelay::default();
        let tool = waiting_tool(temporary.path(), &executor, &starter, &inspector, &delay);
        let error = execute(
            &tool,
            exact_wait_arguments(7, 30_000),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::Execution);
        assert_eq!(error.code, "terminal_background_lost");
        assert_eq!(
            error.message,
            "terminal background process outcome is unavailable"
        );
        assert!(!error.retryable);
    }

    let running = SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Running, None)]);
    for delay in [
        Arc::new(EarlyWaitDelay::default()) as Arc<dyn TerminalBackgroundWaitDelay>,
        Arc::new(ErrorWaitDelay) as Arc<dyn TerminalBackgroundWaitDelay>,
    ] {
        let tool = inspecting_tool(temporary.path(), &executor, &starter, &running)
            .with_wait_delay(delay)
            .unwrap();
        let error = execute(
            &tool,
            exact_wait_arguments(7, 30_000),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::Unavailable);
        assert_eq!(error.code, "terminal_wait_unavailable");
        assert_eq!(error.message, "terminal background wait is unavailable");
        assert!(error.retryable);
    }

    let delay = SleepingWaitDelay::default();
    let tool = waiting_tool(temporary.path(), &executor, &starter, &running, &delay);
    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    let before = running.calls();
    let error = execute(&tool, exact_wait_arguments(7, 1), cancellation).unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(running.calls(), before);
}

#[test]
fn background_wait_capacity_drop_and_cancellation_are_bounded() {
    let temporary = TemporaryDirectory::new("background-wait-capacity");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Running, None)]);
    let delay = PendingWaitDelay::default();
    let tool = waiting_tool(temporary.path(), &executor, &starter, &inspector, &delay);
    let mut active = Vec::new();
    for id in 1..=4 {
        let mut future = Box::pin(tool.execute(
            context(),
            exact_wait_arguments(id, 30_000),
            CancellationToken::new(),
        ));
        assert!(poll_once(future.as_mut()).is_pending());
        active.push(future);
    }
    let error = execute(
        &tool,
        exact_wait_arguments(5, 30_000),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(error.code, "terminal_wait_busy");
    assert!(error.retryable);
    assert_eq!(inspector.calls(), 4);
    assert_eq!(delay.polls.load(Ordering::SeqCst), 8);

    drop(active.pop());
    assert_eq!(delay.drops.load(Ordering::SeqCst), 2);
    let cancellation = CancellationToken::new();
    let observed = Arc::new(ObservedWake::default());
    let waker = Waker::from(Arc::clone(&observed));
    let mut replacement = Box::pin(tool.execute(
        context(),
        exact_wait_arguments(6, 30_000),
        cancellation.clone(),
    ));
    assert!(poll_with_waker(replacement.as_mut(), &waker).is_pending());
    assert!(cancellation.cancel());
    observed.wait_for_calls(1);
    let error = match poll_with_waker(replacement.as_mut(), &waker) {
        Poll::Ready(Err(error)) => error,
        Poll::Ready(Ok(_)) => panic!("cancelled wait returned output"),
        Poll::Pending => panic!("cancelled wait remained pending"),
    };
    assert_eq!(error.code, "terminal_cancelled");
    assert_eq!(delay.polls.load(Ordering::SeqCst), 10);
    assert_eq!(delay.drops.load(Ordering::SeqCst), 4);
    drop(active);
    assert_eq!(delay.drops.load(Ordering::SeqCst), 10);
}

#[test]
fn background_wait_cancels_pending_inspection_and_recovers_exact_wait_capacity() {
    let temporary = TemporaryDirectory::new("background-wait-pending-inspection");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = PendingBackgroundInspector {
        polls: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    };
    let delay = PendingWaitDelay::default();
    let tool = waiting_tool(temporary.path(), &executor, &starter, &inspector, &delay);
    let cancellation = CancellationToken::new();
    let observed = Arc::new(ObservedWake::default());
    let waker = Waker::from(Arc::clone(&observed));
    let mut cancelled = Box::pin(tool.execute(
        context(),
        exact_wait_arguments(1, 30_000),
        cancellation.clone(),
    ));
    assert!(poll_with_waker(cancelled.as_mut(), &waker).is_pending());
    assert_eq!(inspector.polls.load(Ordering::SeqCst), 1);
    assert!(cancellation.cancel());
    observed.wait_for_calls(1);
    let error = match poll_with_waker(cancelled.as_mut(), &waker) {
        Poll::Ready(Err(error)) => error,
        Poll::Ready(Ok(_)) => panic!("cancelled wait returned output"),
        Poll::Pending => panic!("cancelled wait remained pending"),
    };
    assert_eq!(error.code, "terminal_cancelled");
    assert!(inspector.dropped.load(Ordering::SeqCst));

    let mut recovered = Vec::new();
    for id in 2..=5 {
        let mut future = Box::pin(tool.execute(
            context(),
            exact_wait_arguments(id, 30_000),
            CancellationToken::new(),
        ));
        assert!(poll_once(future.as_mut()).is_pending());
        recovered.push(future);
    }
    let busy = execute(
        &tool,
        exact_wait_arguments(6, 30_000),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(busy.code, "terminal_wait_busy");
    assert_eq!(inspector.polls.load(Ordering::SeqCst), 5);
    assert_eq!(delay.polls.load(Ordering::SeqCst), 5);
    drop(recovered);
    assert_eq!(delay.drops.load(Ordering::SeqCst), 5);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn background_wait_pending_inspections_expire_and_release_all_wait_capacity() {
    let temporary = TemporaryDirectory::new("background-wait-pending-expiry");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = PendingBackgroundInspector {
        polls: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    };
    let delay = SleepingWaitDelay::default();
    let tool = waiting_tool(temporary.path(), &executor, &starter, &inspector, &delay);

    std::thread::scope(|scope| {
        let mut waits = Vec::new();
        for background_id in 1..=4 {
            let tool = &tool;
            waits.push(scope.spawn(move || {
                execute(
                    tool,
                    exact_wait_arguments(background_id, 25),
                    CancellationToken::new(),
                )
                .unwrap_err()
            }));
        }
        for wait in waits {
            let error = wait.join().unwrap();
            assert_eq!(error.code, "terminal_wait_unavailable");
            assert!(error.retryable);
        }
    });

    assert_eq!(inspector.polls.load(Ordering::SeqCst), 4);
    assert_eq!(delay.polls.load(Ordering::SeqCst), 4);
    let replacement =
        execute(&tool, exact_wait_arguments(5, 1), CancellationToken::new()).unwrap_err();
    assert_eq!(replacement.code, "terminal_wait_unavailable");
    assert_ne!(replacement.code, "terminal_wait_busy");
    assert_eq!(inspector.polls.load(Ordering::SeqCst), 5);
    assert_eq!(delay.polls.load(Ordering::SeqCst), 5);
    assert!(inspector.dropped.load(Ordering::SeqCst));
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn background_wait_provider_drop_cancellation_wins_before_error_mapping() {
    let temporary = TemporaryDirectory::new("background-wait-drop-cancel");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let inspector = SequenceBackgroundInspector::new(vec![(NativeBackgroundState::Running, None)]);
    let cancellation = CancellationToken::new();
    let dropped = Arc::new(AtomicBool::new(false));
    let delay = DropCancellingWaitDelay {
        cancellation: cancellation.clone(),
        dropped: Arc::clone(&dropped),
    };
    let tool = waiting_tool(temporary.path(), &executor, &starter, &inspector, &delay);
    let error = execute(&tool, exact_wait_arguments(7, 30_000), cancellation).unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn background_configuration_binds_the_named_workspace_to_the_retained_root() {
    let retained = TemporaryDirectory::new("background-retained-root");
    let replacement = TemporaryDirectory::new("background-other-root");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let error = TerminalTool::with_executor_and_background(
        retained.path(),
        environment(),
        Arc::new(executor.clone()),
        TerminalLimits::default(),
        std::fs::canonicalize(replacement.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
        ProcessEnvironment {
            profile: TERMINAL_BACKGROUND_ENVIRONMENT_PROFILE.to_owned(),
            sha256: "a".repeat(64),
        },
        Arc::new(starter.clone()),
    )
    .unwrap_err();
    assert_eq!(error.kind(), TerminalConfigErrorKind::InvalidRoot);
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn background_permission_stays_relative_after_workspace_rename_and_replacement() {
    let temporary = TemporaryDirectory::new("background-renamed-root");
    let workspace = temporary.path().join("workspace");
    let retained = temporary.path().join("retained-workspace");
    std::fs::create_dir_all(workspace.join("nested")).unwrap();
    std::fs::write(workspace.join("nested/identity"), b"retained").unwrap();
    let canonical_workspace = std::fs::canonicalize(&workspace)
        .unwrap()
        .to_str()
        .expect("test workspace is Unicode")
        .to_owned();
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let tool = background_tool(&workspace, &executor, &starter);

    std::fs::rename(&workspace, &retained).unwrap();
    std::fs::create_dir_all(workspace.join("nested")).unwrap();
    std::fs::write(workspace.join("nested/identity"), b"replacement").unwrap();

    let arguments = json!({
        "action": "start",
        "command": ":",
        "cwd": "nested",
    });
    let prepared = tool.prepare(call("terminal", arguments)).unwrap();
    let Capability::Process {
        working_directory, ..
    } = prepared
        .capability()
        .expect("background start requires process permission")
    else {
        panic!("background start must prepare process permission")
    };
    assert_eq!(working_directory, "nested");

    let output = execute(
        &tool,
        prepared.arguments().clone(),
        CancellationToken::new(),
    )
    .unwrap();
    assert!(!output.is_error);
    assert_eq!(starter.calls(), 1);
    assert_eq!(executor.calls(), 0);
    let requests = starter.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, ":");
    assert_eq!(requests[0].1, format!("{canonical_workspace}/nested"));
    assert_eq!(
        std::fs::read(retained.join("nested/identity")).unwrap(),
        b"retained"
    );
    assert_eq!(
        std::fs::read(workspace.join("nested/identity")).unwrap(),
        b"replacement"
    );
}

#[test]
fn background_outcome_rejects_zero_and_redacts_display_identities() {
    let error = TerminalBackgroundOutcome::new(0, NonZeroU32::new(99)).unwrap_err();
    assert_eq!(error.kind(), BackgroundStartErrorKind::InvalidRequest);
    let outcome = TerminalBackgroundOutcome::new(9, None).unwrap();
    assert_eq!(outcome.id(), 9);
    assert_eq!(outcome.pid(), None);
    assert_eq!(format!("{outcome:?}"), "TerminalBackgroundOutcome { .. }");
}

#[test]
fn background_start_rejects_rich_session_fields_and_pre_cancel_has_no_effects() {
    let temporary = TemporaryDirectory::new("background-start-invalid");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let tool = background_tool(temporary.path(), &executor, &starter);

    for field in [
        "shell",
        "backend",
        "return_when",
        "wait_ceiling_ms",
        "dimensions",
        "initial_monitors",
        "session_id",
    ] {
        let mut arguments = json!({ "action": "start", "command": "true" });
        arguments
            .as_object_mut()
            .unwrap()
            .insert(field.to_owned(), json!(true));
        assert_invalid_input(&tool.prepare(call("terminal", arguments)).unwrap_err());
    }
    assert_invalid_input(
        &tool
            .prepare(call("terminal", json!({ "action": "start" })))
            .unwrap_err(),
    );

    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    let error = execute(
        &tool,
        exact_start_arguments("must-not-run", "."),
        cancellation,
    )
    .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);
}

#[test]
fn every_background_start_failure_has_one_fixed_mapping() {
    let temporary = TemporaryDirectory::new("background-start-errors");
    let cases = [
        (
            BackgroundStartErrorKind::InvalidRequest,
            ToolErrorKind::Execution,
            "terminal_executor_failed",
            false,
        ),
        (
            BackgroundStartErrorKind::Capacity,
            ToolErrorKind::Unavailable,
            "terminal_busy",
            true,
        ),
        (
            BackgroundStartErrorKind::Clock,
            ToolErrorKind::Unavailable,
            "terminal_start_unavailable",
            true,
        ),
        (
            BackgroundStartErrorKind::Persistence,
            ToolErrorKind::Execution,
            "terminal_start_persistence_failed",
            false,
        ),
        (
            BackgroundStartErrorKind::Process,
            ToolErrorKind::Execution,
            "terminal_start_failed",
            false,
        ),
        (
            BackgroundStartErrorKind::Cancelled,
            ToolErrorKind::Cancelled,
            "terminal_cancelled",
            false,
        ),
    ];
    for (source, kind, code, retryable) in cases {
        let executor = FakeExecutor::new(Mode::Exited(0));
        let starter = FakeBackgroundStarter::new(BackgroundMode::Error(source));
        let tool = background_tool(temporary.path(), &executor, &starter);
        let error = execute(
            &tool,
            exact_start_arguments("true", "."),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.code, code);
        assert_eq!(error.retryable, retryable);
        assert!(!error.message.contains("true"));
        assert_eq!(starter.calls(), 1);
        assert_eq!(executor.calls(), 0);
    }
}

#[test]
fn strict_schema_command_and_cwd_boundaries_reject_without_executor_effects() {
    let temporary = TemporaryDirectory::new("invalid");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);
    let over_component = "x".repeat(256);
    let over_cwd = "x".repeat(4_097);
    let over_command = "x".repeat(32 * 1_024 + 1);
    let too_many_components = std::iter::repeat_n("x", 257).collect::<Vec<_>>().join("/");
    assert_invalid_input(
        &tool
            .prepare(call(
                "web_search",
                json!({ "action": "exec", "command": "true" }),
            ))
            .unwrap_err(),
    );
    let invalid = vec![
        json!(null),
        json!([]),
        json!({}),
        json!({ "action": "exec" }),
        json!({ "command": "true" }),
        json!({ "action": "start", "command": "true" }),
        json!({ "action": 1, "command": "true" }),
        json!({ "action": "exec", "command": 1 }),
        json!({ "action": "exec", "command": "" }),
        json!({ "action": "exec", "command": "true", "cwd": 1 }),
        json!({ "action": "exec", "command": "true", "profile": "login" }),
        json!({ "action": "exec", "command": "true", "extra": true }),
        json!({ "action": "exec", "command": over_command }),
        json!({ "action": "exec", "command": "true", "cwd": "" }),
        json!({ "action": "exec", "command": "true", "cwd": "/absolute" }),
        json!({ "action": "exec", "command": "true", "cwd": "~" }),
        json!({ "action": "exec", "command": "true", "cwd": "a//b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a/" }),
        json!({ "action": "exec", "command": "true", "cwd": "./a" }),
        json!({ "action": "exec", "command": "true", "cwd": "a/./b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a/../b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a/~/b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a\0b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a\nb" }),
        json!({ "action": "exec", "command": "true", "cwd": "a\u{2028}b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a\u{2029}b" }),
        json!({ "action": "exec", "command": "true", "cwd": "a\u{202e}b" }),
        json!({ "action": "exec", "command": "true", "cwd": over_component }),
        json!({ "action": "exec", "command": "true", "cwd": over_cwd }),
        json!({ "action": "exec", "command": "true", "cwd": too_many_components }),
    ];
    for arguments in invalid {
        assert_invalid_input(
            &tool
                .prepare(call("terminal", arguments.clone()))
                .unwrap_err(),
        );
        assert_invalid_input(&execute(&tool, arguments, CancellationToken::new()).unwrap_err());
    }
    assert_eq!(executor.calls(), 0);
    assert_eq!(executor.polls(), 0);
}

#[test]
fn exact_command_and_cwd_boundaries_prepare_successfully() {
    let temporary = TemporaryDirectory::new("exact-bounds");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);
    let exact_command = "x".repeat(32 * 1_024);
    let exact_component = "x".repeat(255);
    let exact_components = std::iter::repeat_n("x", 256).collect::<Vec<_>>().join("/");
    for (command, cwd) in [
        (exact_command.as_str(), "."),
        ("true", exact_component.as_str()),
        ("true", exact_components.as_str()),
    ] {
        let prepared = tool
            .prepare(call(
                "terminal",
                json!({
                    "action": "exec",
                    "command": command,
                    "cwd": cwd,
                    "profile": "clean",
                }),
            ))
            .unwrap();
        assert_eq!(prepared.arguments(), &exact_arguments(command, cwd));
    }
    assert_eq!(executor.calls(), 0);
}

#[test]
fn exact_background_command_boundary_moves_intact_into_the_start_request() {
    let temporary = TemporaryDirectory::new("background-command-boundary");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let tool = background_tool(temporary.path(), &executor, &starter);
    let command = "x".repeat(MAX_TERMINAL_COMMAND_BYTES);
    let prepared = tool
        .prepare(call(
            "terminal",
            json!({ "action": "start", "command": command }),
        ))
        .unwrap();
    let Capability::Process {
        arguments,
        working_directory,
        ..
    } = prepared
        .capability()
        .expect("background start requires process permission")
    else {
        panic!("background start must prepare process permission")
    };
    assert_eq!(arguments[0], "-c");
    assert_eq!(arguments[1].len(), MAX_TERMINAL_COMMAND_BYTES);
    assert_eq!(working_directory, ".");

    let output = execute(
        &tool,
        prepared.arguments().clone(),
        CancellationToken::new(),
    )
    .unwrap();
    assert!(!output.is_error);
    assert_eq!(executor.calls(), 0);
    let requests = starter.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0.len(), MAX_TERMINAL_COMMAND_BYTES);
    assert!(requests[0].0.bytes().all(|byte| byte == b'x'));
}

#[test]
fn background_cwd_combined_bound_rejects_before_permission_and_executes_at_exact_limit() {
    let temporary = TemporaryDirectory::new("background-cwd-combined-boundary");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let starter = FakeBackgroundStarter::new(BackgroundMode::Success {
        cancel_before_return: false,
    });
    let tool = background_tool(temporary.path(), &executor, &starter);
    let workspace = std::fs::canonicalize(temporary.path()).unwrap();
    let workspace = workspace.to_str().expect("test workspace is Unicode");
    let prefix_bytes = workspace.len() + usize::from(workspace != "/");
    let exact_relative_bytes = MAX_BACKGROUND_CWD_BYTES
        .checked_sub(prefix_bytes)
        .expect("temporary workspace leaves room for a relative cwd");
    let exact_cwd = canonical_relative_cwd_with_length(exact_relative_bytes);
    let over_cwd = canonical_relative_cwd_with_length(exact_relative_bytes + 1);

    let over_error = tool
        .prepare(call(
            "terminal",
            json!({ "action": "start", "command": ":", "cwd": over_cwd }),
        ))
        .unwrap_err();
    assert_eq!(over_error.kind, ToolErrorKind::InvalidInput);
    assert_eq!(over_error.code, "terminal_invalid_cwd");
    let direct_error = execute(
        &tool,
        exact_start_arguments(":", &over_cwd),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(direct_error.kind, ToolErrorKind::InvalidInput);
    assert_eq!(direct_error.code, "terminal_invalid_cwd");
    assert_eq!(starter.calls(), 0);
    assert_eq!(executor.calls(), 0);

    let prepared = tool
        .prepare(call(
            "terminal",
            json!({ "action": "start", "command": ":", "cwd": exact_cwd }),
        ))
        .unwrap();
    let Capability::Process {
        working_directory, ..
    } = prepared
        .capability()
        .expect("exact-bound start requires process permission")
    else {
        panic!("exact-bound start must prepare process permission")
    };
    assert_eq!(working_directory, &exact_cwd);

    let output = execute(
        &tool,
        prepared.arguments().clone(),
        CancellationToken::new(),
    )
    .unwrap();
    assert!(!output.is_error);
    let requests = starter.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1.len(), MAX_BACKGROUND_CWD_BYTES);
    assert_eq!(requests[0].1, format!("{workspace}/{exact_cwd}"));
    assert_eq!(executor.calls(), 0);
}

#[test]
fn literal_tilde_prefixed_directory_names_are_not_home_expansion() {
    let temporary = TemporaryDirectory::new("literal-tilde-cwd");
    std::fs::create_dir(temporary.path().join("~cache")).unwrap();
    std::fs::create_dir_all(temporary.path().join("parent/~cache")).unwrap();
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);

    for cwd in ["~cache", "parent/~cache"] {
        let arguments = exact_arguments("true", cwd);
        let prepared = tool.prepare(call("terminal", arguments.clone())).unwrap();
        assert_eq!(prepared.arguments(), &arguments);
        let output = execute(&tool, arguments, CancellationToken::new()).unwrap();
        assert!(!output.is_error);
        assert_eq!(output.content["cwd"], cwd);
    }

    for cwd in ["~", "parent/~/child"] {
        let error = tool
            .prepare(call("terminal", exact_arguments("true", cwd)))
            .unwrap_err();
        assert_invalid_input(&error);
        assert_eq!(error.code, "terminal_invalid_cwd");
    }
    assert_eq!(executor.calls(), 2);
}

#[test]
fn environment_digest_is_order_stable_and_raw_values_never_reflect() {
    let temporary = TemporaryDirectory::new("environment");
    let first_executor = FakeExecutor::new(Mode::Exited(0));
    let first = tool(temporary.path(), &first_executor);
    let mut reversed_environment = environment();
    reversed_environment.reverse();
    let second_executor = FakeExecutor::new(Mode::Exited(0));
    let second = TerminalTool::with_executor(
        temporary.path(),
        reversed_environment,
        Arc::new(second_executor),
        TerminalLimits::default(),
    )
    .unwrap();
    let changed_executor = FakeExecutor::new(Mode::Exited(0));
    let changed = TerminalTool::with_executor(
        temporary.path(),
        vec![(
            OsString::from(PRIVATE_ENVIRONMENT_KEY),
            OsString::from("different"),
        )],
        Arc::new(changed_executor),
        TerminalLimits::default(),
    )
    .unwrap();
    let arguments = json!({ "action": "exec", "command": "true" });
    let first_capability = first
        .prepare(call("terminal", arguments.clone()))
        .unwrap()
        .capability()
        .expect("terminal requires permission authority")
        .clone();
    let second_capability = second
        .prepare(call("terminal", arguments.clone()))
        .unwrap()
        .capability()
        .expect("terminal requires permission authority")
        .clone();
    let changed_capability = changed
        .prepare(call("terminal", arguments))
        .unwrap()
        .capability()
        .expect("terminal requires permission authority")
        .clone();
    assert_eq!(first_capability, second_capability);
    assert_ne!(first_capability, changed_capability);

    let rendered = format!("{first_capability:?}");
    assert!(!rendered.contains(PRIVATE_ENVIRONMENT_KEY));
    assert!(!rendered.contains(PRIVATE_ENVIRONMENT_VALUE));

    let output = execute(
        &first,
        exact_arguments("true", "."),
        CancellationToken::new(),
    )
    .unwrap();
    assert!(!output.is_error);
    let requests = first_executor.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].program, "/bin/sh");
    assert_eq!(requests[0].arguments, ["-c", "true"]);
    assert_eq!(requests[0].command, "true");
    assert_eq!(requests[0].cwd, ".");
    assert_eq!(requests[0].environment_profile, "construction_snapshot");
    let mut expected_environment = environment();
    expected_environment.sort();
    assert_eq!(requests[0].environment, expected_environment);
    let now = Instant::now();
    assert!(requests[0].deadline > now);
    assert!(requests[0].deadline <= now + Duration::from_secs(120));
    assert!(!requests[0].debug.contains(PRIVATE_ENVIRONMENT_KEY));
    assert!(!requests[0].debug.contains(PRIVATE_ENVIRONMENT_VALUE));
}

#[test]
fn injected_environment_is_an_owned_construction_snapshot() {
    let temporary = TemporaryDirectory::new("owned-environment");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let mut supplied_environment = environment();
    let expected_environment = supplied_environment.clone();
    let tool = TerminalTool::with_executor(
        temporary.path(),
        supplied_environment.clone(),
        Arc::new(executor.clone()),
        TerminalLimits::default(),
    )
    .unwrap();

    supplied_environment.clear();
    supplied_environment.push((
        OsString::from(PRIVATE_ENVIRONMENT_KEY),
        OsString::from("changed-after-construction"),
    ));

    let output = execute(
        &tool,
        exact_arguments("true", "."),
        CancellationToken::new(),
    )
    .unwrap();

    assert!(!output.is_error);
    let mut expected_environment = expected_environment;
    expected_environment.sort();
    assert_eq!(executor.requests()[0].environment, expected_environment);
    assert_ne!(executor.requests()[0].environment, supplied_environment);
}

#[test]
fn direct_execute_revalidates_complete_canonical_arguments_without_effects() {
    let temporary = TemporaryDirectory::new("revalidate");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);
    for arguments in [
        json!({ "action": "exec", "command": "true" }),
        json!({ "action": "exec", "command": "true", "cwd": "." }),
        json!({ "action": "exec", "command": "true", "profile": "clean" }),
        json!({ "action": "exec", "command": "true", "cwd": ".", "profile": "clean", "extra": 1 }),
        json!({ "action": "exec", "command": "true", "cwd": "a//b", "profile": "clean" }),
    ] {
        assert_invalid_input(&execute(&tool, arguments, CancellationToken::new()).unwrap_err());
    }
    assert_eq!(executor.calls(), 0);
}

#[test]
fn descriptor_relative_cwd_failures_never_reach_the_injected_executor() {
    let temporary = TemporaryDirectory::new("cwd-failures");
    std::fs::create_dir(temporary.path().join("directory")).unwrap();
    std::fs::write(temporary.path().join("regular-file"), b"not a directory").unwrap();
    std::os::unix::fs::symlink("directory", temporary.path().join("symlink")).unwrap();
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(temporary.path(), &executor);

    for cwd in ["missing", "regular-file", "symlink"] {
        let error = execute(
            &tool,
            exact_arguments("true", cwd),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ToolErrorKind::Unavailable);
        assert_eq!(error.code, "terminal_cwd_unavailable");
        assert_eq!(error.message, "terminal working directory is unavailable");
        assert!(!error.retryable);
    }
    assert_eq!(executor.calls(), 0);
    assert_eq!(executor.polls(), 0);
}

#[test]
fn retained_root_rename_and_path_replacement_cannot_redirect_request_identity() {
    let temporary = TemporaryDirectory::new("retained-root");
    let root = temporary.path().join("workspace");
    let moved = temporary.path().join("workspace-moved");
    std::fs::create_dir(&root).unwrap();
    let original = rustix::fs::stat(&root).unwrap();
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = tool(&root, &executor);

    std::fs::rename(&root, &moved).unwrap();
    std::fs::create_dir(&root).unwrap();
    let replacement = rustix::fs::stat(&root).unwrap();
    assert_ne!(
        (original.st_dev, original.st_ino),
        (replacement.st_dev, replacement.st_ino)
    );

    let output = execute(
        &tool,
        exact_arguments("true", "."),
        CancellationToken::new(),
    )
    .unwrap();
    assert!(!output.is_error);
    let requests = executor.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].directory_identity,
        format!("{}:{}", original.st_dev, original.st_ino)
    );
    assert_ne!(
        requests[0].directory_identity,
        format!("{}:{}", replacement.st_dev, replacement.st_ino)
    );
}

#[test]
fn executor_is_inert_until_poll_and_drop_owns_the_pending_execution() {
    let temporary = TemporaryDirectory::new("future");
    let executor = FakeExecutor::new(Mode::Pending);
    let tool = tool(temporary.path(), &executor);
    let mut execution = Box::pin(tool.execute(
        context(),
        exact_arguments("sleep forever", "."),
        CancellationToken::new(),
    ));
    assert_eq!(executor.calls(), 0);
    assert_eq!(executor.polls(), 0);
    assert!(poll_once(execution.as_mut()).is_pending());
    assert_eq!(executor.calls(), 1);
    assert_eq!(executor.polls(), 1);
    assert_eq!(executor.drops(), 0);
    drop(execution);
    assert_eq!(executor.drops(), 1);
}

#[test]
fn pre_cancel_and_same_poll_cancel_win_without_publishing_output() {
    let temporary = TemporaryDirectory::new("cancellation");
    let pre_executor = FakeExecutor::new(Mode::Exited(0));
    let pre_tool = tool(temporary.path(), &pre_executor);
    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());
    let error = execute(&pre_tool, exact_arguments("true", "."), cancellation).unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(pre_executor.calls(), 0);

    let race_executor = FakeExecutor::new(Mode::CancelThenExit);
    let race_tool = tool(temporary.path(), &race_executor);
    let error = execute(
        &race_tool,
        exact_arguments("true", "."),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(race_executor.calls(), 1);
    assert_eq!(race_executor.drops(), 1);
}

#[test]
fn cancellation_from_ready_executor_drop_wins_before_output_publication() {
    let temporary = TemporaryDirectory::new("drop-cancellation");
    let executor = FakeExecutor::new(Mode::DropCancelThenExit);
    let tool = tool(temporary.path(), &executor);

    let error = execute(
        &tool,
        exact_arguments("true", "."),
        CancellationToken::new(),
    )
    .unwrap_err();

    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");
    assert_eq!(error.message, "terminal execution was cancelled");
    assert!(!error.retryable);
    assert_eq!(executor.calls(), 1);
    assert_eq!(executor.polls(), 1);
    assert_eq!(executor.drops(), 1);
}

#[test]
fn statuses_streams_loss_and_truncation_render_as_bounded_protocol() {
    let temporary = TemporaryDirectory::new("outcomes");
    std::fs::create_dir(temporary.path().join("nested")).unwrap();
    let cases = [
        (Mode::Exited(0), "exited", Some(0), None, false),
        (Mode::Exited(23), "exited", Some(23), None, true),
        (Mode::Signaled(15), "signaled", None, Some(15), true),
        (Mode::TimedOut, "timed_out", None, None, true),
        (Mode::OutputLimit, "output_limit", None, None, true),
    ];
    for (mode, status, exit_code, signal, is_error) in cases {
        let produced_stdout = if matches!(mode, Mode::OutputLimit) {
            1024 * 1024 + 1
        } else {
            70_000
        };
        let executor = FakeExecutor::new(mode)
            .with_output(b"hello\n".to_vec(), vec![b'e', 0xff, b'\n'])
            .with_totals(produced_stdout, 9);
        let tool = tool(temporary.path(), &executor);
        let output = execute(
            &tool,
            exact_arguments("private command", "nested"),
            CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(output.is_error, is_error);
        assert_eq!(output.content["action"], "exec");
        assert_eq!(output.content["cwd"], "nested");
        assert_eq!(output.content["status"], status);
        assert_eq!(output.content["exit_code"], json!(exit_code));
        assert_eq!(output.content["signal"], json!(signal));
        assert_eq!(output.content["stdout"], "hello\n");
        assert_eq!(output.content["stderr"], "e�\n");
        assert_eq!(output.content["stdout_bytes"], produced_stdout);
        assert_eq!(output.content["stderr_bytes"], 9);
        assert_eq!(output.content["stdout_truncated"], true);
        assert_eq!(output.content["stderr_truncated"], true);
        assert_eq!(output.content["stdout_lossy"], false);
        assert_eq!(output.content["stderr_lossy"], true);
        assert_eq!(output.content["duration_ms"], 7);
        assert!(serde_json::to_vec(&output).unwrap().len() <= 48 * 1_024);
    }
}

#[test]
fn serialized_result_is_trimmed_below_the_contract_ceiling() {
    let temporary = TemporaryDirectory::new("serialized-cap");
    let executor = FakeExecutor::new(Mode::Exited(0)).with_output(
        std::iter::repeat_n(b'\\', 32 * 1_024).collect(),
        std::iter::repeat_n(b'"', 32 * 1_024).collect(),
    );
    let tool = tool(temporary.path(), &executor);
    let output = execute(
        &tool,
        exact_arguments("true", "."),
        CancellationToken::new(),
    )
    .unwrap();
    let serialized = serde_json::to_vec(&output).unwrap();
    assert!(serialized.len() <= 48 * 1_024, "{}", serialized.len());
    assert_eq!(output.content["stdout_truncated"], true);
    assert_eq!(output.content["stderr_truncated"], true);
}

#[test]
fn fixed_executor_failures_are_redacted_and_classified() {
    let temporary = TemporaryDirectory::new("failures");
    for kind in [
        TerminalExecutorErrorKind::Unsupported,
        TerminalExecutorErrorKind::Busy,
        TerminalExecutorErrorKind::Spawn,
        TerminalExecutorErrorKind::Wait,
        TerminalExecutorErrorKind::Pipe,
        TerminalExecutorErrorKind::Invariant,
        TerminalExecutorErrorKind::InvalidResponse,
        TerminalExecutorErrorKind::Cancelled,
    ] {
        let executor = FakeExecutor::new(Mode::Error(kind));
        let tool = tool(temporary.path(), &executor);
        let error = execute(
            &tool,
            exact_arguments("PRIVATE_COMMAND_DO_NOT_REFLECT", "."),
            CancellationToken::new(),
        )
        .unwrap_err();
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("PRIVATE_COMMAND_DO_NOT_REFLECT"));
        assert!(!rendered.contains(PRIVATE_ENVIRONMENT_VALUE));
        if matches!(kind, TerminalExecutorErrorKind::Busy) {
            assert_eq!(error.code, "terminal_busy");
            assert!(error.retryable);
        }
    }
}

#[test]
fn fail_fast_concurrency_releases_permit_after_drop() {
    let temporary = TemporaryDirectory::new("capacity");
    let executor = FakeExecutor::new(Mode::Pending);
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        limits(1),
    )
    .unwrap();
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("first", "."),
        CancellationToken::new(),
    ));
    assert!(poll_once(first.as_mut()).is_pending());
    let error = execute(
        &tool,
        exact_arguments("second", "."),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::Unavailable);
    assert_eq!(error.code, "terminal_busy");
    assert!(error.retryable);
    assert_eq!(executor.calls(), 1);
    drop(first);

    let mut third = Box::pin(tool.execute(
        context(),
        exact_arguments("third", "."),
        CancellationToken::new(),
    ));
    assert!(poll_once(third.as_mut()).is_pending());
    assert_eq!(executor.calls(), 2);
    drop(third);
}

#[test]
fn completed_public_execution_releases_capacity_for_the_next_call() {
    let temporary = TemporaryDirectory::new("capacity-complete");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        limits(1),
    )
    .unwrap();
    for command in ["first", "second"] {
        let output = execute(
            &tool,
            exact_arguments(command, "."),
            CancellationToken::new(),
        )
        .unwrap();
        assert!(!output.is_error);
    }
    assert_eq!(executor.calls(), 2);
}

#[test]
fn absolute_deadline_drops_a_permanently_pending_executor_and_releases_capacity() {
    const MAX_CAPACITY_RECOVERY_ATTEMPTS: usize = 256;
    const CAPACITY_RECOVERY_BACKOFF: Duration = Duration::from_millis(1);

    let temporary = TemporaryDirectory::new("pending-timeout");
    let executor = FakeExecutor::new(Mode::Pending);
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        TerminalLimits::new(Duration::from_millis(5), 1).unwrap(),
    )
    .unwrap();
    let started = Instant::now();

    for command in ["first", "second"] {
        let mut busy_attempts = 0;
        let output = loop {
            match futures_executor::block_on(tool.execute(
                context(),
                exact_arguments(command, "."),
                CancellationToken::new(),
            )) {
                Ok(output) => break output,
                Err(error) if error.code == "terminal_busy" => {
                    assert_eq!(error.kind, ToolErrorKind::Unavailable);
                    assert!(error.retryable);
                    assert!(
                        busy_attempts < MAX_CAPACITY_RECOVERY_ATTEMPTS,
                        "deadline callback exceeded the bounded capacity recovery budget"
                    );
                    busy_attempts += 1;
                    // The timer callback intentionally retains admission until
                    // its wake tail returns. Back off passively with a fixed
                    // attempt budget instead of racing it with a busy loop.
                    std::thread::sleep(CAPACITY_RECOVERY_BACKOFF);
                }
                Err(error) => panic!("deadline execution failed: {error}"),
            }
        };
        assert!(busy_attempts <= MAX_CAPACITY_RECOVERY_ATTEMPTS);
        assert!(output.is_error);
        assert_eq!(output.content["status"], "timed_out");
        assert_eq!(output.content["exit_code"], Value::Null);
        assert_eq!(output.content["signal"], Value::Null);
    }
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(executor.calls(), 2);
    assert!(executor.polls() >= 2);
    assert_eq!(executor.drops(), 2);
}

#[test]
fn output_limit_ready_after_deadline_remains_authoritative() {
    let temporary = TemporaryDirectory::new("output-limit-deadline");
    let executor = FakeExecutor::new(Mode::DelayedOutputLimit(Duration::from_millis(30)))
        .with_totals(
            usize::try_from(MAX_TERMINAL_PRODUCED_OUTPUT_BYTES).unwrap() + 1,
            0,
        );
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        TerminalLimits::new(Duration::from_millis(10), 1).unwrap(),
    )
    .unwrap();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments("produce-overflow", "."),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(output.is_error);
    assert_eq!(output.content["status"], "output_limit");
    assert_eq!(
        output.content["stdout_bytes"],
        MAX_TERMINAL_PRODUCED_OUTPUT_BYTES + 1
    );
    assert_eq!(output.content["stderr_bytes"], 0);
    assert_eq!(executor.polls(), 1);
    assert_eq!(executor.drops(), 1);
}

#[test]
fn output_limit_ready_on_the_deadline_repoll_remains_authoritative() {
    let temporary = TemporaryDirectory::new("output-limit-deadline-repoll");
    let executor = FakeExecutor::new(Mode::OutputLimitAfterPending).with_totals(
        usize::try_from(MAX_TERMINAL_PRODUCED_OUTPUT_BYTES).unwrap() + 1,
        0,
    );
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        TerminalLimits::new(Duration::from_millis(20), 1).unwrap(),
    )
    .unwrap();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments("produce-overflow-on-repoll", "."),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(output.is_error);
    assert_eq!(output.content["status"], "output_limit");
    assert_eq!(
        output.content["stdout_bytes"],
        MAX_TERMINAL_PRODUCED_OUTPUT_BYTES + 1
    );
    assert_eq!(output.content["stderr_bytes"], 0);
    assert_eq!(executor.polls(), 2);
    assert_eq!(executor.drops(), 1);
}

#[test]
fn outer_future_drop_closes_delivery_but_retained_waker_keeps_capacity() {
    let temporary = TemporaryDirectory::new("outer-drop-retained-waker");
    let executor = SelfRepollExecutor::default();
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        limits(1),
    )
    .unwrap();
    let observed = Arc::new(ObservedWake::default());
    let external_waker = Waker::from(Arc::clone(&observed));
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("outer-drop-retained-waker", "."),
        CancellationToken::new(),
    ));

    assert!(poll_with_waker(first.as_mut(), &external_waker).is_pending());
    let retained = executor.retained_waker();
    assert_eq!(observed.calls(), 0);

    // Dropping the pending outer future destroys the injected execution,
    // which releases its stored Waker. The independently retained clone still
    // owns capacity, but its notifier must no longer target the dropped task.
    drop(first);
    retained.wake_by_ref();
    assert_eq!(observed.calls(), 0);
    assert_self_repoll_tail_is_busy(&tool, &executor);

    drop(retained);
    assert_self_repoll_capacity_recovers(&tool, &executor);
}

#[test]
fn executor_result_after_supplied_waker_repoll_reaches_the_original_host() {
    let temporary = TemporaryDirectory::new("executor-self-repoll");
    let executor = SelfRepollExecutor::default();
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        limits(1),
    )
    .unwrap();
    let observed = Arc::new(ObservedWake::default());
    let external_waker = Waker::from(Arc::clone(&observed));
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("executor-self-repoll", "."),
        CancellationToken::new(),
    ));

    assert!(poll_with_waker(first.as_mut(), &external_waker).is_pending());
    let retained = executor.retained_waker();
    assert_eq!(observed.calls(), 0);

    // A host may legally poll with the Waker supplied to its injected
    // executor. That must not replace the original task notification target.
    assert!(poll_with_waker(first.as_mut(), &retained).is_pending());
    executor.publish_and_wake();
    observed.wait_for_calls(1);
    assert_eq!(observed.calls(), 1);

    let output = match poll_with_waker(first.as_mut(), &external_waker) {
        Poll::Ready(Ok(output)) => output,
        Poll::Ready(Err(error)) => panic!("self-repolled execution failed: {error}"),
        Poll::Pending => panic!("published self-repolled execution remained pending"),
    };
    drop(first);
    assert_eq!(output.content["status"], "exited");

    // Completion closes notification delivery, but an independently retained
    // supplied Waker continues to own this execution's capacity slot.
    retained.wake_by_ref();
    assert_eq!(observed.calls(), 1);
    assert_self_repoll_tail_is_busy(&tool, &executor);
    drop(retained);
    assert_self_repoll_capacity_recovers(&tool, &executor);
}

#[test]
fn deadline_after_supplied_waker_repoll_reaches_the_original_host() {
    let temporary = TemporaryDirectory::new("deadline-self-repoll");
    let executor = SelfRepollExecutor::default();
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        TerminalLimits::new(Duration::from_millis(30), 1).unwrap(),
    )
    .unwrap();
    let observed = Arc::new(ObservedWake::default());
    let external_waker = Waker::from(Arc::clone(&observed));
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("deadline-self-repoll", "."),
        CancellationToken::new(),
    ));

    assert!(poll_with_waker(first.as_mut(), &external_waker).is_pending());
    let retained = executor.retained_waker();
    assert_eq!(observed.calls(), 0);
    assert!(poll_with_waker(first.as_mut(), &retained).is_pending());

    observed.wait_for_calls(1);
    assert_eq!(observed.calls(), 1);
    let output = match poll_with_waker(first.as_mut(), &external_waker) {
        Poll::Ready(Ok(output)) => output,
        Poll::Ready(Err(error)) => panic!("deadline execution failed: {error}"),
        Poll::Pending => panic!("deadline execution remained pending after notification"),
    };
    drop(first);
    assert_eq!(output.content["status"], "timed_out");

    retained.wake_by_ref();
    assert_eq!(observed.calls(), 1);
    assert_self_repoll_tail_is_busy(&tool, &executor);
    drop(retained);
    assert_self_repoll_capacity_recovers(&tool, &executor);
}

#[test]
fn cancellation_after_supplied_waker_repoll_reaches_the_original_host() {
    let temporary = TemporaryDirectory::new("cancellation-self-repoll");
    let executor = SelfRepollExecutor::default();
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        limits(1),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let observed = Arc::new(ObservedWake::default());
    let external_waker = Waker::from(Arc::clone(&observed));
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("cancellation-self-repoll", "."),
        cancellation.clone(),
    ));

    assert!(poll_with_waker(first.as_mut(), &external_waker).is_pending());
    let retained = executor.retained_waker();
    assert_eq!(observed.calls(), 0);
    assert!(poll_with_waker(first.as_mut(), &retained).is_pending());

    assert!(cancellation.cancel());
    observed.wait_for_calls(1);
    assert_eq!(observed.calls(), 1);
    let error = match poll_with_waker(first.as_mut(), &external_waker) {
        Poll::Ready(Err(error)) => error,
        Poll::Ready(Ok(_)) => panic!("cancelled self-repolled execution returned output"),
        Poll::Pending => panic!("cancelled self-repolled execution remained pending"),
    };
    drop(first);
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "terminal_cancelled");

    retained.wake_by_ref();
    assert_eq!(observed.calls(), 1);
    assert_self_repoll_tail_is_busy(&tool, &executor);
    drop(retained);
    assert_self_repoll_capacity_recovers(&tool, &executor);
}

#[test]
fn injected_publisher_waker_retains_the_originating_capacity_slot() {
    let temporary = TemporaryDirectory::new("injected-publisher-capacity");
    let executor = RetainedPublisherExecutor::default();
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        limits(1),
    )
    .unwrap();
    let blocking = Arc::new(BlockingWake::default());
    let waker = Waker::from(Arc::clone(&blocking));
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("first", "."),
        CancellationToken::new(),
    ));

    assert!(poll_with_waker(first.as_mut(), &waker).is_pending());
    std::thread::scope(|scope| {
        let publisher_executor = executor.clone();
        let publisher = scope.spawn(move || publisher_executor.publish_and_wake());
        let _release_on_unwind = BlockingWakeRelease(Arc::clone(&blocking));
        blocking.wait_until_entered();

        let first_output = match poll_once(first.as_mut()) {
            Poll::Ready(Ok(output)) => output,
            Poll::Ready(Err(error)) => panic!("injected execution failed: {error}"),
            Poll::Pending => panic!("published injected execution remained pending"),
        };
        drop(first);
        assert_eq!(first_output.content["status"], "exited");

        for attempt in 0..16 {
            let mut blocked = Box::pin(tool.execute(
                context(),
                exact_arguments(&format!("blocked-{attempt}"), "."),
                CancellationToken::new(),
            ));
            match poll_once(blocked.as_mut()) {
                Poll::Ready(Err(error)) => {
                    assert_eq!(error.kind, ToolErrorKind::Unavailable);
                    assert_eq!(error.code, "terminal_busy");
                }
                Poll::Ready(Ok(_)) => {
                    panic!("execution bypassed an injected publisher Waker callback")
                }
                Poll::Pending => {
                    panic!("injected publisher Waker callback released capacity early")
                }
            }
        }
        assert_eq!(executor.calls(), 1);

        blocking.release();
        publisher.join().unwrap();
    });

    let recovered = execute(
        &tool,
        exact_arguments("recovered", "."),
        CancellationToken::new(),
    )
    .unwrap();
    assert!(!recovered.is_error);
    assert_eq!(executor.calls(), 2);
}

#[test]
fn blocking_cancellation_callback_retains_capacity_until_return() {
    let temporary = TemporaryDirectory::new("cancellation-waker-capacity");
    let executor = FakeExecutor::new(Mode::Pending);
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        limits(1),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let blocking = Arc::new(BlockingWake::default());
    let waker = Waker::from(Arc::clone(&blocking));
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("first", "."),
        cancellation.clone(),
    ));

    assert!(poll_with_waker(first.as_mut(), &waker).is_pending());
    std::thread::scope(|scope| {
        let _release_on_unwind = BlockingWakeRelease(Arc::clone(&blocking));
        let cancellation_publisher = scope.spawn(move || assert!(cancellation.cancel()));
        blocking.wait_until_entered();

        let first_error = match poll_once(first.as_mut()) {
            Poll::Ready(Err(error)) => error,
            Poll::Ready(Ok(_)) => panic!("cancelled execution published output"),
            Poll::Pending => panic!("cancelled execution remained pending"),
        };
        drop(first);
        assert_eq!(first_error.kind, ToolErrorKind::Cancelled);
        assert_eq!(first_error.code, "terminal_cancelled");

        let mut blocked = Box::pin(tool.execute(
            context(),
            exact_arguments("blocked", "."),
            CancellationToken::new(),
        ));
        match poll_once(blocked.as_mut()) {
            Poll::Ready(Err(error)) => {
                assert_eq!(error.kind, ToolErrorKind::Unavailable);
                assert_eq!(error.code, "terminal_busy");
            }
            Poll::Ready(Ok(_)) => panic!("pending executor unexpectedly completed"),
            Poll::Pending => {
                panic!("blocking cancellation callback released terminal capacity early")
            }
        }
        drop(blocked);
        assert_eq!(executor.calls(), 1);

        blocking.release();
        cancellation_publisher.join().unwrap();
        blocking.wait_until_returned();
    });

    let mut recovered = Box::pin(tool.execute(
        context(),
        exact_arguments("recovered", "."),
        CancellationToken::new(),
    ));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(executor.calls(), 2);
}

#[test]
fn cloned_injected_wakers_coalesce_blocking_callbacks_and_retain_one_slot() {
    let temporary = TemporaryDirectory::new("cloned-waker-coalescing");
    let executor = WakerFanoutExecutor::default();
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        limits(1),
    )
    .unwrap();
    let blocking = Arc::new(CountingBlockingWake::default());
    let waker = Waker::from(Arc::clone(&blocking));
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("first", "."),
        CancellationToken::new(),
    ));

    assert!(poll_with_waker(first.as_mut(), &waker).is_pending());
    let retained_wakers = executor.publish_and_take_wakers();
    assert_eq!(retained_wakers.len(), CLONED_WAKER_FANOUT);

    std::thread::scope(|scope| {
        let _release_on_unwind = CountingBlockingWakeRelease(Arc::clone(&blocking));
        let start = Arc::new(std::sync::Barrier::new(CLONED_WAKER_FANOUT + 1));
        let (returned_sender, returned_receiver) = std::sync::mpsc::channel();
        for retained_waker in retained_wakers {
            let start = Arc::clone(&start);
            let returned_sender = returned_sender.clone();
            scope.spawn(move || {
                start.wait();
                retained_waker.wake();
                let _ = returned_sender.send(());
            });
        }
        drop(returned_sender);
        start.wait();
        blocking.wait_until_entered();

        let first_output = match poll_once(first.as_mut()) {
            Poll::Ready(Ok(output)) => output,
            Poll::Ready(Err(error)) => panic!("injected execution failed: {error}"),
            Poll::Pending => panic!("published injected execution remained pending"),
        };
        drop(first);
        assert_eq!(first_output.content["status"], "exited");

        let mut blocked = Box::pin(tool.execute(
            context(),
            exact_arguments("blocked", "."),
            CancellationToken::new(),
        ));
        match poll_once(blocked.as_mut()) {
            Poll::Ready(Err(error)) => {
                assert_eq!(error.kind, ToolErrorKind::Unavailable);
                assert_eq!(error.code, "terminal_busy");
            }
            Poll::Ready(Ok(_)) => panic!("execution bypassed a blocking Waker callback"),
            Poll::Pending => panic!("blocking Waker callback released terminal capacity early"),
        }
        drop(blocked);
        assert_eq!(executor.calls(), 1);

        for _ in 1..CLONED_WAKER_FANOUT {
            returned_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("concurrent Waker notifications did not coalesce and return");
        }
        assert_eq!(blocking.snapshot(), (1, 1, 1, 0));

        blocking.release();
        returned_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("the forwarded Waker callback did not return after release");
    });

    let (entered, in_flight, max_in_flight, returned) = blocking.snapshot();
    assert!((1..=2).contains(&entered));
    assert_eq!(in_flight, 0);
    assert_eq!(max_in_flight, 1);
    assert_eq!(returned, entered);
    let mut recovered = Box::pin(tool.execute(
        context(),
        exact_arguments("recovered", "."),
        CancellationToken::new(),
    ));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(executor.calls(), 2);
}

#[test]
fn deadline_and_injected_waker_families_share_one_callback_and_capacity_slot() {
    let temporary = TemporaryDirectory::new("dual-waker-capacity");
    let executor = RetainedPublisherExecutor::default();
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        TerminalLimits::new(Duration::from_millis(30), 1).unwrap(),
    )
    .unwrap();
    let publisher_blocking = Arc::new(CountingBlockingWake::default());
    let deadline_blocking = Arc::new(CountingBlockingWake::default());
    let publisher_waker = Waker::from(Arc::clone(&publisher_blocking));
    let deadline_waker = Waker::from(Arc::clone(&deadline_blocking));
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("first", "."),
        CancellationToken::new(),
    ));

    assert!(poll_with_waker(first.as_mut(), &publisher_waker).is_pending());
    assert!(poll_with_waker(first.as_mut(), &deadline_waker).is_pending());
    std::thread::scope(|scope| {
        let publisher_executor = executor.clone();
        let publisher = scope.spawn(move || {
            let wait_deadline = Instant::now() + Duration::from_secs(2);
            let mut inner = publisher_executor.state.inner.lock().unwrap();
            while inner.waker.is_none() {
                let remaining = wait_deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "injected executor did not retain a task Waker"
                );
                let waited = publisher_executor
                    .state
                    .changed
                    .wait_timeout(inner, remaining)
                    .unwrap();
                inner = waited.0;
            }
            let retained = inner.waker.take().expect("retained task Waker exists");
            drop(inner);
            retained.wake();
        });
        let _publisher_release_on_unwind =
            CountingBlockingWakeRelease(Arc::clone(&publisher_blocking));
        let _deadline_release_on_unwind =
            CountingBlockingWakeRelease(Arc::clone(&deadline_blocking));
        deadline_blocking.wait_until_entered();

        // The executor notification is already blocked in the latest raw
        // Waker. Let the independent deadline expire and notify the same
        // shared terminal notifier; that second family must coalesce instead
        // of forwarding another underlying callback.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(publisher_blocking.snapshot(), (0, 0, 0, 0));
        assert_eq!(deadline_blocking.snapshot(), (1, 1, 1, 0));

        let first_output = match poll_with_waker(first.as_mut(), &deadline_waker) {
            Poll::Ready(Ok(output)) => output,
            Poll::Ready(Err(error)) => panic!("overlapping execution failed: {error}"),
            Poll::Pending => panic!("overlapping execution remained pending after its deadline"),
        };
        drop(first);
        assert_eq!(first_output.content["status"], "timed_out");

        let mut blocked = Box::pin(tool.execute(
            context(),
            exact_arguments("blocked-during-shared-callback", "."),
            CancellationToken::new(),
        ));
        match poll_once(blocked.as_mut()) {
            Poll::Ready(Err(error)) => {
                assert_eq!(error.kind, ToolErrorKind::Unavailable);
                assert_eq!(error.code, "terminal_busy");
            }
            Poll::Ready(Ok(_)) => panic!("execution bypassed a blocking shared callback"),
            Poll::Pending => panic!("blocking shared callback released terminal capacity early"),
        }
        drop(blocked);
        assert_eq!(executor.calls(), 1);

        deadline_blocking.release();
        publisher.join().unwrap();
        assert_eq!(publisher_blocking.snapshot(), (0, 0, 0, 0));
        let (entered, in_flight, max_in_flight, returned) = deadline_blocking.snapshot();
        assert!((1..=2).contains(&entered));
        assert_eq!(in_flight, 0);
        assert_eq!(max_in_flight, 1);
        assert_eq!(returned, entered);
    });

    assert_retained_publisher_capacity_recovers(&tool, &executor);
}

#[test]
fn blocked_deadline_waker_tail_retains_capacity_until_callback_returns() {
    let temporary = TemporaryDirectory::new("deadline-waker-capacity");
    let executor = FakeExecutor::new(Mode::Pending);
    let tool = TerminalTool::with_executor(
        temporary.path(),
        environment(),
        Arc::new(executor.clone()),
        TerminalLimits::new(Duration::from_millis(20), 1).unwrap(),
    )
    .unwrap();
    let blocking = Arc::new(BlockingWake::default());
    let _release_on_unwind = BlockingWakeRelease(Arc::clone(&blocking));
    let waker = Waker::from(Arc::clone(&blocking));
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments("first", "."),
        CancellationToken::new(),
    ));

    assert!(poll_with_waker(first.as_mut(), &waker).is_pending());
    blocking.wait_until_entered();
    let first_output = match poll_once(first.as_mut()) {
        Poll::Ready(Ok(output)) => output,
        Poll::Ready(Err(error)) => panic!("deadline execution failed: {error}"),
        Poll::Pending => panic!("deadline execution remained pending after its wake"),
    };
    drop(first);
    assert_eq!(first_output.content["status"], "timed_out");

    let mut blocked = Box::pin(tool.execute(
        context(),
        exact_arguments("blocked", "."),
        CancellationToken::new(),
    ));
    match poll_once(blocked.as_mut()) {
        Poll::Ready(Err(error)) => {
            assert_eq!(error.kind, ToolErrorKind::Unavailable);
            assert_eq!(error.code, "terminal_busy");
        }
        Poll::Ready(Ok(_)) => panic!("execution bypassed a blocked Waker tail"),
        Poll::Pending => panic!("blocked Waker tail released terminal capacity early"),
    }
    drop(blocked);

    blocking.release();
    let recovery_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let mut recovered = Box::pin(tool.execute(
            context(),
            exact_arguments("recovered", "."),
            CancellationToken::new(),
        ));
        match poll_once(recovered.as_mut()) {
            Poll::Pending => {
                drop(recovered);
                break;
            }
            Poll::Ready(Err(error)) if error.code == "terminal_busy" => {
                assert!(
                    Instant::now() < recovery_deadline,
                    "capacity did not recover"
                );
                drop(recovered);
                std::thread::sleep(Duration::from_millis(1));
            }
            Poll::Ready(Err(error)) => panic!("capacity recovery failed: {error}"),
            Poll::Ready(Ok(_)) => panic!("pending executor unexpectedly completed"),
        }
    }
}

#[test]
fn process_environment_contract_round_trips_exactly() {
    let environment = ProcessEnvironment {
        profile: "construction_snapshot".to_owned(),
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
    };
    let value = serde_json::to_value(&environment).unwrap();
    assert_eq!(
        value,
        json!({
            "profile": "construction_snapshot",
            "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        })
    );
    assert_eq!(
        serde_json::from_value::<ProcessEnvironment>(value).unwrap(),
        environment
    );
}

#[test]
fn limits_enforce_the_exact_public_timeout_and_capacity_boundaries() {
    let defaults = TerminalLimits::default();
    assert_eq!(defaults.timeout(), Duration::from_secs(120));
    assert_eq!(defaults.max_active_executions(), 4);
    assert!(TerminalLimits::new(Duration::from_millis(1), 1).is_ok());
    assert!(TerminalLimits::new(Duration::from_secs(600), 16).is_ok());
    for (timeout, active) in [
        (Duration::ZERO, 1),
        (Duration::from_secs(600) + Duration::from_millis(1), 1),
        (Duration::from_secs(1), 0),
        (Duration::from_secs(1), 17),
    ] {
        let error = TerminalLimits::new(timeout, active).unwrap_err();
        assert_eq!(error.kind(), TerminalConfigErrorKind::InvalidLimits);
        assert_eq!(error.to_string(), "native terminal limits are invalid");
    }
}

#[test]
fn invalid_environment_snapshots_are_bounded_redacted_and_reject_duplicates() {
    let temporary = TemporaryDirectory::new("invalid-environment");
    let executor = FakeExecutor::new(Mode::Exited(0));
    let too_many = (0..513)
        .map(|index| {
            (
                OsString::from(format!("KEY_{index}")),
                OsString::from("value"),
            )
        })
        .collect();
    let aggregate_overflow = (0..17)
        .map(|index| {
            (
                OsString::from(format!("AGGREGATE_{index}")),
                OsString::from("x".repeat(16 * 1_024)),
            )
        })
        .collect();
    let invalid = vec![
        vec![(OsString::new(), OsString::from("value"))],
        vec![(OsString::from("BAD=KEY"), OsString::from("value"))],
        vec![(OsString::from("BAD\0KEY"), OsString::from("value"))],
        vec![(OsString::from("KEY"), OsString::from("BAD\0VALUE"))],
        vec![(OsString::from("k".repeat(1_025)), OsString::from("v"))],
        vec![(
            OsString::from("KEY"),
            OsString::from("v".repeat(16 * 1_024 + 1)),
        )],
        vec![
            (OsString::from("DUPLICATE"), OsString::from("first")),
            (OsString::from("DUPLICATE"), OsString::from("second")),
        ],
        too_many,
        aggregate_overflow,
    ];
    for environment in invalid {
        let error = TerminalTool::with_executor(
            temporary.path(),
            environment,
            Arc::new(executor.clone()),
            TerminalLimits::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), TerminalConfigErrorKind::InvalidEnvironment);
        assert_eq!(
            error.to_string(),
            "native terminal environment snapshot is invalid"
        );
        let rendering = format!("{error:?} {error}");
        assert!(!rendering.contains("BAD"));
        assert!(!rendering.contains("DUPLICATE"));
        assert!(!rendering.contains("second"));
    }
    assert_eq!(executor.calls(), 0);
}

#[test]
fn injected_outcome_contract_rejects_impossible_stream_and_status_reports() {
    let too_large =
        TerminalCapturedOutput::new(vec![b'x'; 64 * 1_024 + 1], 64 * 1_024 + 1).unwrap_err();
    assert_eq!(too_large.kind(), TerminalExecutorErrorKind::InvalidResponse);
    let impossible_total = TerminalCapturedOutput::new(b"two".to_vec(), 2).unwrap_err();
    assert_eq!(
        impossible_total.kind(),
        TerminalExecutorErrorKind::InvalidResponse
    );

    let small = || TerminalCapturedOutput::new(Vec::new(), 0).unwrap();
    let overflow = || TerminalCapturedOutput::new(Vec::new(), 1024 * 1024 + 1).unwrap();
    let missing_overflow = TerminalExecutionOutcome::new(
        TerminalExecutionStatus::OutputLimit,
        small(),
        small(),
        Duration::ZERO,
    )
    .unwrap_err();
    assert_eq!(
        missing_overflow.kind(),
        TerminalExecutorErrorKind::InvalidResponse
    );
    let undeclared_overflow = TerminalExecutionOutcome::new(
        TerminalExecutionStatus::Exited(0),
        overflow(),
        small(),
        Duration::ZERO,
    )
    .unwrap_err();
    assert_eq!(
        undeclared_overflow.kind(),
        TerminalExecutorErrorKind::InvalidResponse
    );
    for (status, duration) in [
        (TerminalExecutionStatus::Exited(-1), Duration::ZERO),
        (TerminalExecutionStatus::Exited(256), Duration::ZERO),
        (TerminalExecutionStatus::Signaled(0), Duration::ZERO),
        (TerminalExecutionStatus::Signaled(256), Duration::ZERO),
        (
            TerminalExecutionStatus::Exited(0),
            Duration::from_secs(600) + Duration::from_millis(1),
        ),
    ] {
        let error = TerminalExecutionOutcome::new(status, small(), small(), duration).unwrap_err();
        assert_eq!(error.kind(), TerminalExecutorErrorKind::InvalidResponse);
    }
}

#[cfg(target_os = "linux")]
fn require_linux_executable(path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;

    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 => true,
        _ => {
            eprintln!("skipping terminal system evidence because {path} is unavailable");
            false
        }
    }
}

#[cfg(target_os = "linux")]
fn read_linux_pid(path: &std::path::Path) -> rustix::process::Pid {
    let pid = std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    rustix::process::Pid::from_raw(pid).expect("terminal process pid is positive")
}

#[cfg(target_os = "linux")]
struct EscapedProcessGuard {
    pid: rustix::process::Pid,
}

#[cfg(target_os = "linux")]
impl EscapedProcessGuard {
    fn new(pid: rustix::process::Pid) -> Self {
        Self { pid }
    }

    fn terminate(&self) -> bool {
        let _ = rustix::process::kill_process_group(self.pid, rustix::process::Signal::TERM);
        let _ = rustix::process::kill_process(self.pid, rustix::process::Signal::TERM);
        if self.wait_until_gone(Duration::from_millis(500)) {
            return true;
        }
        let _ = rustix::process::kill_process_group(self.pid, rustix::process::Signal::KILL);
        let _ = rustix::process::kill_process(self.pid, rustix::process::Signal::KILL);
        self.wait_until_gone(Duration::from_secs(2))
    }

    fn wait_until_gone(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            match rustix::process::test_kill_process(self.pid) {
                Err(error) if error == rustix::io::Errno::SRCH => return true,
                _ if Instant::now() >= deadline => return false,
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for EscapedProcessGuard {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_executor_runs_fixed_shell_in_selected_cwd_with_separate_streams() {
    let temporary = TemporaryDirectory::new("system");
    std::fs::create_dir(temporary.path().join("nested")).unwrap();
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let command = "if IFS= read -r ignored; then exit 99; fi; printf '%s' 'stdout bytes'; printf '%s' 'stderr bytes' >&2; exit 7";

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments(command, "nested"),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(output.is_error);
    assert_eq!(output.content["action"], "exec");
    assert_eq!(output.content["cwd"], "nested");
    assert_eq!(output.content["status"], "exited");
    assert_eq!(output.content["exit_code"], 7);
    assert_eq!(output.content["signal"], Value::Null);
    assert_eq!(output.content["stdout"], "stdout bytes");
    assert_eq!(output.content["stderr"], "stderr bytes");
    assert_eq!(output.content["stdout_bytes"], 12);
    assert_eq!(output.content["stderr_bytes"], 12);
    assert_eq!(output.content["stdout_truncated"], false);
    assert_eq!(output.content["stderr_truncated"], false);
    assert_eq!(output.content["stdout_lossy"], false);
    assert_eq!(output.content["stderr_lossy"], false);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_executor_uses_retained_root_after_path_replacement() {
    if !require_linux_executable("/bin/cat") {
        return;
    }
    let temporary = TemporaryDirectory::new("system-retained-root");
    let root = temporary.path().join("workspace");
    let moved = temporary.path().join("workspace-moved");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("identity.txt"), b"original-workspace").unwrap();
    let tool = TerminalTool::open(&root).unwrap();

    std::fs::rename(&root, &moved).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("identity.txt"), b"replacement-workspace").unwrap();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments("/bin/cat identity.txt", "."),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(!output.is_error);
    assert_eq!(output.content["status"], "exited");
    assert_eq!(output.content["exit_code"], 0);
    assert_eq!(output.content["stdout"], "original-workspace");
    assert_ne!(output.content["stdout"], "replacement-workspace");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_environment_snapshot_reaches_the_shell() {
    const CHILD_MODE: &str = "MACHINE_GOD_TERMINAL_ENVIRONMENT_CHILD";
    const PRESENT: &str = "MACHINE_GOD_TERMINAL_SNAPSHOT_PRESENT";
    const ABSENT: &str = "MACHINE_GOD_TERMINAL_SNAPSHOT_ABSENT";

    if std::env::var_os(CHILD_MODE).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("linux_system_environment_snapshot_reaches_the_shell")
            .arg("--nocapture")
            .env(CHILD_MODE, "1")
            .env(PRESENT, "construction-snapshot-value")
            .env_remove(ABSENT)
            .status()
            .unwrap();
        assert!(status.success(), "controlled environment child failed");
        return;
    }

    let temporary = TemporaryDirectory::new("system-environment");
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments(
            "printf '%s|%s' \"$MACHINE_GOD_TERMINAL_SNAPSHOT_PRESENT\" \"${MACHINE_GOD_TERMINAL_SNAPSHOT_ABSENT-unset}\"",
            ".",
        ),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(!output.is_error);
    assert_eq!(
        output.content["stdout"],
        "construction-snapshot-value|unset"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_executor_reports_direct_shell_signal_status() {
    let temporary = TemporaryDirectory::new("system-signal");
    let tool = TerminalTool::open(temporary.path()).unwrap();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments("kill -TERM \"$$\"", "."),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(output.is_error);
    assert_eq!(output.content["status"], "signaled");
    assert_eq!(output.content["exit_code"], Value::Null);
    assert_eq!(output.content["signal"], 15);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_executor_terminates_on_aggregate_output_pressure() {
    if !require_linux_executable("/usr/bin/head") || !std::path::Path::new("/dev/zero").exists() {
        eprintln!("skipping terminal output-limit evidence because head or /dev/zero is absent");
        return;
    }
    let temporary = TemporaryDirectory::new("system-output-limit");
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let started = Instant::now();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments("/usr/bin/head -c 1048577 /dev/zero", "."),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(output.is_error);
    assert_eq!(output.content["status"], "output_limit");
    assert_eq!(output.content["exit_code"], Value::Null);
    assert_eq!(output.content["signal"], Value::Null);
    assert!(output.content["stdout_bytes"].as_u64().unwrap() > 1024 * 1024);
    assert!(
        output.content["stdout_bytes"].as_u64().unwrap()
            <= MAX_TERMINAL_PRODUCED_OUTPUT_BYTES + 2 * 16 * 1024,
        "reader work exceeded the deterministic two-in-flight-read allowance"
    );
    assert_eq!(output.content["stderr_bytes"], 0);
    assert_eq!(output.content["stdout_truncated"], true);
    assert_eq!(output.content["stdout_lossy"], false);
    assert!(serde_json::to_vec(&output).unwrap().len() <= 48 * 1024);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_output_limit_wins_when_deadline_expires_during_term_ignoring_cleanup() {
    if !require_linux_executable("/usr/bin/head")
        || !require_linux_executable("/bin/sleep")
        || !std::path::Path::new("/dev/zero").exists()
    {
        eprintln!("skipping terminal deadline/output evidence because head or /dev/zero is absent");
        return;
    }
    let temporary = TemporaryDirectory::new("system-deadline-output");
    let tool = TerminalTool::open_with_limits(
        temporary.path(),
        TerminalLimits::new(Duration::from_millis(225), 1).unwrap(),
    )
    .unwrap();
    let started = Instant::now();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments(
            "trap '' TERM; /usr/bin/head -c 1048577 /dev/zero; while :; do /bin/sleep 1; done",
            ".",
        ),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(output.is_error);
    let produced = output.content["stdout_bytes"].as_u64().unwrap()
        + output.content["stderr_bytes"].as_u64().unwrap();
    assert_eq!(output.content["status"], "output_limit");
    assert!(produced > MAX_TERMINAL_PRODUCED_OUTPUT_BYTES);
    assert!(produced <= MAX_TERMINAL_PRODUCED_OUTPUT_BYTES + 2 * 16 * 1024);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_trivial_exit_completes_inside_a_sub_term_grace_deadline() {
    let temporary = TemporaryDirectory::new("system-trivial-exit");
    let tool = TerminalTool::open_with_limits(
        temporary.path(),
        TerminalLimits::new(Duration::from_millis(225), 1).unwrap(),
    )
    .unwrap();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments(":", "."),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(!output.is_error);
    assert_eq!(output.content["status"], "exited");
    assert_eq!(output.content["exit_code"], 0);
    assert!(output.content["duration_ms"].as_u64().unwrap() < 225);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_timeout_kills_a_term_ignoring_shell_before_publication() {
    if !require_linux_executable("/bin/sleep") {
        return;
    }
    let temporary = TemporaryDirectory::new("system-timeout");
    let tool = TerminalTool::open_with_limits(
        temporary.path(),
        TerminalLimits::new(Duration::from_millis(100), 1).unwrap(),
    )
    .unwrap();
    let started = Instant::now();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments(
            "trap '' TERM; printf '%s' \"$$\" > timeout.pid; while :; do /bin/sleep 1; done",
            ".",
        ),
        CancellationToken::new(),
    ))
    .unwrap();

    let pid = read_linux_pid(&temporary.path().join("timeout.pid"));
    let cleanup = EscapedProcessGuard::new(pid);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(output.is_error);
    assert_eq!(output.content["status"], "timed_out");
    assert_eq!(output.content["exit_code"], Value::Null);
    assert_eq!(output.content["signal"], Value::Null);
    assert_eq!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH),
        "timed-out TERM-ignoring shell survived output publication"
    );
    drop(cleanup);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_reader_cleanup_is_bounded_when_setsid_process_retains_pipe() {
    if !require_linux_executable("/usr/bin/setsid")
        || !require_linux_executable("/bin/sh")
        || !require_linux_executable("/bin/sleep")
    {
        return;
    }
    let temporary = TemporaryDirectory::new("system-setsid-pipe");
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let command = "/usr/bin/setsid /bin/sh -c 'printf \"%s\" \"$$\" > escaped.pid; exec /bin/sleep 30' & i=0; while [ ! -s escaped.pid ] && [ \"$i\" -lt 200 ]; do i=$((i + 1)); /bin/sleep 0.01; done; test -s escaped.pid";
    let started = Instant::now();

    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments(command, "."),
        CancellationToken::new(),
    ))
    .unwrap();

    let escaped = read_linux_pid(&temporary.path().join("escaped.pid"));
    let cleanup = EscapedProcessGuard::new(escaped);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!output.is_error);
    assert_eq!(output.content["status"], "exited");
    assert_eq!(output.content["exit_code"], 0);
    assert!(rustix::process::test_kill_process(escaped).is_ok());
    assert!(
        cleanup.terminate(),
        "escaped setsid test process did not terminate during explicit cleanup"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_ready_publication_has_already_terminated_background_group_members() {
    let temporary = TemporaryDirectory::new("system-ready-cleanup");
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments("/bin/sleep 60 & printf '%s' \"$!\" > descendant.pid", "."),
        CancellationToken::new(),
    ))
    .unwrap();
    assert!(!output.is_error);

    let pid = std::fs::read_to_string(temporary.path().join("descendant.pid"))
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    let pid = rustix::process::Pid::from_raw(pid).expect("terminal descendant pid is positive");
    assert_eq!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH),
        "terminal output was published before the background group member was gone"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_system_ready_publication_observes_term_ignoring_group_member_cleanup() {
    if !require_linux_executable("/bin/sleep") {
        return;
    }
    let temporary = TemporaryDirectory::new("system-ready-term-ignore");
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let output = futures_executor::block_on(tool.execute(
        context(),
        exact_arguments(
            "(trap '' TERM; printf '%s' ready > term-ignore.ready; exec /bin/sleep 60) & descendant=$!; i=0; while [ ! -s term-ignore.ready ] && [ \"$i\" -lt 200 ]; do i=$((i + 1)); /bin/sleep 0.01; done; test -s term-ignore.ready || exit 42; printf '%s' \"$descendant\" > term-ignore.pid",
            ".",
        ),
        CancellationToken::new(),
    ))
    .unwrap();
    let pid = read_linux_pid(&temporary.path().join("term-ignore.pid"));
    let cleanup = EscapedProcessGuard::new(pid);

    assert!(!output.is_error);
    assert_eq!(output.content["status"], "exited");
    assert_eq!(output.content["exit_code"], 0);
    assert_eq!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH),
        "TERM-ignoring original-group member survived successful publication"
    );
    drop(cleanup);
}

#[cfg(target_os = "linux")]
#[test]
fn blocked_linux_publisher_waker_tail_retains_capacity_until_callback_returns() {
    if !require_linux_executable("/bin/sleep") {
        return;
    }
    let temporary = TemporaryDirectory::new("system-publisher-waker-capacity");
    let tool = TerminalTool::open_with_limits(
        temporary.path(),
        TerminalLimits::new(Duration::from_secs(5), 1).unwrap(),
    )
    .unwrap();
    let blocking = Arc::new(BlockingWake::default());
    let _release_on_unwind = BlockingWakeRelease(Arc::clone(&blocking));
    let waker = Waker::from(Arc::clone(&blocking));
    let mut first = Box::pin(tool.execute(
        context(),
        exact_arguments(
            "while [ ! -f publisher.release ]; do /bin/sleep 0.01; done",
            ".",
        ),
        CancellationToken::new(),
    ));

    assert!(poll_with_waker(first.as_mut(), &waker).is_pending());
    std::fs::write(temporary.path().join("publisher.release"), b"release").unwrap();
    blocking.wait_until_entered();
    let first_output = match poll_once(first.as_mut()) {
        Poll::Ready(Ok(output)) => output,
        Poll::Ready(Err(error)) => panic!("publisher execution failed: {error}"),
        Poll::Pending => panic!("publisher result remained pending after its wake"),
    };
    drop(first);
    assert!(!first_output.is_error);

    let mut blocked = Box::pin(tool.execute(
        context(),
        exact_arguments(":", "."),
        CancellationToken::new(),
    ));
    match poll_once(blocked.as_mut()) {
        Poll::Ready(Err(error)) => {
            assert_eq!(error.kind, ToolErrorKind::Unavailable);
            assert_eq!(error.code, "terminal_busy");
        }
        Poll::Ready(Ok(_)) => panic!("execution bypassed a blocked publisher Waker tail"),
        Poll::Pending => panic!("publisher Waker tail released terminal capacity early"),
    }
    drop(blocked);

    blocking.release();
    let recovery_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match futures_executor::block_on(tool.execute(
            context(),
            exact_arguments(":", "."),
            CancellationToken::new(),
        )) {
            Ok(output) => {
                assert!(!output.is_error);
                break;
            }
            Err(error) if error.code == "terminal_busy" => {
                assert!(
                    Instant::now() < recovery_deadline,
                    "capacity did not recover"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("capacity recovery failed: {error}"),
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn dropping_linux_system_execution_synchronously_terminates_the_owned_process_group() {
    let temporary = TemporaryDirectory::new("system-drop");
    let tool = TerminalTool::open(temporary.path()).unwrap();
    let mut execution = Box::pin(tool.execute(
        context(),
        exact_arguments(
            "/bin/sleep 60 & descendant=$!; printf '%s %s' \"$$\" \"$descendant\" > owned.pids; wait",
            ".",
        ),
        CancellationToken::new(),
    ));
    assert!(poll_once(execution.as_mut()).is_pending());

    let pid_path = temporary.path().join("owned.pids");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let pids = loop {
        match std::fs::read_to_string(&pid_path) {
            Ok(pids) => {
                let pids = pids
                    .split_whitespace()
                    .map(|pid| pid.parse::<i32>().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(pids.len(), 2);
                break pids;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "terminal child did not publish its pid"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("failed to read terminal child pid: {error}"),
        }
    };
    let pids = pids
        .into_iter()
        .map(|pid| rustix::process::Pid::from_raw(pid).expect("terminal child pid is positive"))
        .collect::<Vec<_>>();
    for pid in &pids {
        assert!(rustix::process::test_kill_process(*pid).is_ok());
    }

    drop(execution);

    for pid in pids {
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH),
            "terminal future drop returned before its process group was gone"
        );
    }
}
