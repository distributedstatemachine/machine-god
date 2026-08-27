use std::collections::VecDeque;
use std::fmt::Write as _;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex, Weak, mpsc};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};

use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolAuthorization, SessionId, SessionIncarnationId, Tool,
    ToolCall, ToolCallId, ToolContext, ToolError, ToolErrorKind, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    ASK_USER_QUESTION_DEFAULT_MAX_ACTIVE_PROMPTS, ASK_USER_QUESTION_MAX_ACTIVE_PROMPTS,
    ASK_USER_QUESTION_TOOL_NAME, AskUserQuestionConfigError, AskUserQuestionTool,
    MAX_ASK_USER_QUESTION_OPTIONS_PER_QUESTION, MAX_ASK_USER_QUESTION_QUESTIONS,
    MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES, MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES,
    MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES, MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_ANSWER_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_OPTION_DESCRIPTION_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_OPTION_LABEL_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_PRESENTATION_BYTES,
    MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES, MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES,
    MAX_ASK_USER_QUESTION_SERIALIZED_PREPARED_ARGUMENT_BYTES,
    MAX_ASK_USER_QUESTION_SERIALIZED_RESULT_BYTES, MAX_ASK_USER_QUESTION_TOTAL_OPTIONS,
    MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES, QuestionPromptAnswers, QuestionPromptError,
    QuestionPromptOutcome, QuestionPromptRequest, QuestionPrompter,
};
use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
use serde_json::{Value, json};

type PromptResult = Result<QuestionPromptOutcome, QuestionPromptError>;

fn answers(values: impl IntoIterator<Item = String>) -> QuestionPromptAnswers {
    let mut answers = QuestionPromptAnswers::new();
    for value in values {
        answers
            .try_push(value)
            .expect("test fixtures stay within the public four-answer bound");
    }
    answers
}

fn answered(values: impl IntoIterator<Item = String>) -> QuestionPromptOutcome {
    QuestionPromptOutcome::Answered(answers(values))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordedOption {
    label: String,
    description: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordedQuestion {
    question: String,
    options: Vec<RecordedOption>,
}

#[derive(Default)]
struct ScriptedState {
    calls: usize,
    requests: Vec<Vec<RecordedQuestion>>,
    outcomes: VecDeque<PromptResult>,
}

#[derive(Clone, Default)]
struct ScriptedPrompter {
    state: Arc<Mutex<ScriptedState>>,
}

impl ScriptedPrompter {
    fn new(outcomes: impl IntoIterator<Item = PromptResult>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ScriptedState {
                outcomes: outcomes.into_iter().collect(),
                ..ScriptedState::default()
            })),
        }
    }

    fn calls(&self) -> usize {
        self.state.lock().unwrap().calls
    }

    fn requests(&self) -> Vec<Vec<RecordedQuestion>> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl QuestionPrompter for ScriptedPrompter {
    fn prompt(&self, request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        let recorded = request
            .questions()
            .iter()
            .map(|question| RecordedQuestion {
                question: question.question().to_owned(),
                options: question
                    .options()
                    .iter()
                    .map(|option| RecordedOption {
                        label: option.label().to_owned(),
                        description: option.description().map(str::to_owned),
                    })
                    .collect(),
            })
            .collect();
        let outcome = {
            let mut state = self.state.lock().unwrap();
            state.calls += 1;
            state.requests.push(recorded);
            state
                .outcomes
                .pop_front()
                .expect("scripted question prompter has another outcome")
        };
        Box::pin(async move { outcome })
    }
}

#[derive(Default)]
struct PendingProbe {
    calls: AtomicUsize,
    polls: AtomicUsize,
    drops: AtomicUsize,
    live: AtomicUsize,
}

#[derive(Clone)]
struct PendingPrompter {
    probe: Arc<PendingProbe>,
}

impl QuestionPrompter for PendingPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        self.probe.calls.fetch_add(1, Ordering::SeqCst);
        self.probe.live.fetch_add(1, Ordering::SeqCst);
        Box::pin(PendingPrompt {
            probe: Arc::clone(&self.probe),
        })
    }
}

struct PendingPrompt {
    probe: Arc<PendingProbe>,
}

impl Future for PendingPrompt {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.probe.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for PendingPrompt {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::SeqCst);
        self.probe.live.fetch_sub(1, Ordering::SeqCst);
    }
}

struct CancelThenReadyPrompter {
    cancellation: CancellationToken,
    outcome: Mutex<Option<PromptResult>>,
}

struct DebugCapturePrompter {
    debug: Arc<Mutex<Option<String>>>,
}

impl QuestionPrompter for DebugCapturePrompter {
    fn prompt(&self, request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        *self.debug.lock().unwrap() = Some(format!("{request:?}"));
        Box::pin(async { Ok(QuestionPromptOutcome::Unavailable) })
    }
}

struct DropCancelsReadyPrompter {
    cancellation: CancellationToken,
}

impl QuestionPrompter for DropCancelsReadyPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        Box::pin(DropCancelsReady {
            cancellation: self.cancellation.clone(),
        })
    }
}

struct DropCancelsReady {
    cancellation: CancellationToken,
}

impl Future for DropCancelsReady {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(answered(["must not publish".to_owned()])))
    }
}

impl Drop for DropCancelsReady {
    fn drop(&mut self) {
        assert!(self.cancellation.cancel());
    }
}

#[derive(Clone)]
struct PanicOncePrompter {
    calls: Arc<AtomicUsize>,
    panic_in_poll: bool,
    pending_probe: Arc<PendingProbe>,
}

impl QuestionPrompter for PanicOncePrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            call != 0 || self.panic_in_poll,
            "intentional prompt-call panic"
        );
        if call == 0 {
            return Box::pin(PanicOnPoll);
        }
        self.pending_probe.live.fetch_add(1, Ordering::SeqCst);
        Box::pin(PendingPrompt {
            probe: Arc::clone(&self.pending_probe),
        })
    }
}

struct PanicOnPoll;

impl Future for PanicOnPoll {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        panic!("intentional prompt-poll panic")
    }
}

#[derive(Default)]
struct ReentrantState {
    calls: AtomicUsize,
    tool: Mutex<Option<Weak<AskUserQuestionTool>>>,
    arguments: Mutex<Option<Value>>,
    observed: Mutex<Option<Result<ToolOutput, ToolError>>>,
}

#[derive(Clone)]
struct ReentrantPrompter {
    state: Arc<ReentrantState>,
}

impl QuestionPrompter for ReentrantPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        let call = self.state.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            Box::pin(ReentrantPending {
                state: Arc::clone(&self.state),
            })
        } else {
            Box::pin(std::future::pending())
        }
    }
}

struct ReentrantPending {
    state: Arc<ReentrantState>,
}

impl Future for ReentrantPending {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for ReentrantPending {
    fn drop(&mut self) {
        let tool = self
            .state
            .tool
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        let arguments = self.state.arguments.lock().unwrap().clone().unwrap();
        let result = poll_ready(tool.execute(context(), arguments, CancellationToken::new()));
        *self.state.observed.lock().unwrap() = Some(result);
    }
}

#[derive(Default)]
struct WakerTeardownState {
    tool: Mutex<Option<Weak<AskUserQuestionTool>>>,
    arguments: Mutex<Option<Value>>,
    observed: Mutex<Option<Result<ToolOutput, ToolError>>>,
}

struct AdmissionOnWakerDrop {
    state: Arc<WakerTeardownState>,
}

impl Wake for AdmissionOnWakerDrop {
    fn wake(self: Arc<Self>) {}
}

impl Drop for AdmissionOnWakerDrop {
    fn drop(&mut self) {
        let tool = self
            .state
            .tool
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        let arguments = self.state.arguments.lock().unwrap().clone().unwrap();
        let result = poll_ready(tool.execute(context(), arguments, CancellationToken::new()));
        *self.state.observed.lock().unwrap() = Some(result);
    }
}

impl QuestionPrompter for CancelThenReadyPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        let cancellation = self.cancellation.clone();
        let outcome = self.outcome.lock().unwrap().take().unwrap();
        Box::pin(async move {
            assert!(cancellation.cancel());
            outcome
        })
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

struct BlockingWake {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    calls: AtomicUsize,
}

const PROMPT_WAKER_FANOUT: usize = 16;

#[derive(Default)]
struct PromptWakerFanoutInner {
    published: bool,
    registered: bool,
    wakers: Vec<Waker>,
}

#[derive(Default)]
struct PromptWakerFanoutState {
    calls: AtomicUsize,
    inner: Mutex<PromptWakerFanoutInner>,
    changed: Condvar,
}

#[derive(Clone, Default)]
struct PromptWakerFanoutPrompter {
    state: Arc<PromptWakerFanoutState>,
}

impl PromptWakerFanoutPrompter {
    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    fn take_wakers(&self) -> Vec<Waker> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut inner = self.state.inner.lock().unwrap();
        while inner.wakers.len() != PROMPT_WAKER_FANOUT {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "question prompter did not retain every cloned task Waker"
            );
            let waited = self.state.changed.wait_timeout(inner, remaining).unwrap();
            inner = waited.0;
        }
        std::mem::take(&mut inner.wakers)
    }

    fn publish(&self) {
        self.state.inner.lock().unwrap().published = true;
    }
}

impl QuestionPrompter for PromptWakerFanoutPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        let mut inner = self.state.inner.lock().unwrap();
        assert!(
            inner.wakers.is_empty(),
            "a new prompt started while retained Wakers remained"
        );
        inner.published = false;
        inner.registered = false;
        drop(inner);
        Box::pin(PromptWakerFanoutFuture {
            state: Arc::clone(&self.state),
        })
    }
}

struct PromptWakerFanoutFuture {
    state: Arc<PromptWakerFanoutState>,
}

impl Future for PromptWakerFanoutFuture {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.state.inner.lock().unwrap();
        if !inner.published {
            if !inner.registered {
                inner.wakers = std::iter::repeat_with(|| context.waker().clone())
                    .take(PROMPT_WAKER_FANOUT)
                    .collect();
                inner.registered = true;
                self.state.changed.notify_all();
            }
            return Poll::Pending;
        }
        Poll::Ready(Ok(QuestionPromptOutcome::Unavailable))
    }
}

impl Drop for PromptWakerFanoutFuture {
    fn drop(&mut self) {
        let retained = {
            let mut inner = self.state.inner.lock().unwrap();
            std::mem::take(&mut inner.wakers)
        };
        drop(retained);
    }
}

#[derive(Default)]
struct CountingCallbackState {
    entered: usize,
    in_flight: usize,
    max_in_flight: usize,
    released: bool,
    returned: usize,
}

struct CountingBlockingCallback {
    state: Mutex<CountingCallbackState>,
    changed: Condvar,
    block_only_first: bool,
}

impl CountingBlockingCallback {
    fn new(block_only_first: bool) -> Self {
        Self {
            state: Mutex::new(CountingCallbackState::default()),
            changed: Condvar::new(),
            block_only_first,
        }
    }

    fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.state.lock().unwrap();
        while state.entered == 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "question Waker callback did not run");
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
        let call = state.entered;
        state.in_flight += 1;
        state.max_in_flight = state.max_in_flight.max(state.in_flight);
        self.changed.notify_all();
        while !state.released && (!self.block_only_first || call == 1) {
            state = self.changed.wait(state).unwrap();
        }
        state.in_flight -= 1;
        state.returned += 1;
        self.changed.notify_all();
    }
}

struct CountingCallbackRelease(Arc<CountingBlockingCallback>);

impl Drop for CountingCallbackRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}

const REENTRANT_PROMPT_WAKE_BUDGET: usize = 64;

struct ReentrantWakeProbe {
    calls: AtomicUsize,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    remaining_rewakes: AtomicUsize,
    retained: Mutex<Option<Waker>>,
}

impl ReentrantWakeProbe {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            remaining_rewakes: AtomicUsize::new(REENTRANT_PROMPT_WAKE_BUDGET),
            retained: Mutex::new(None),
        }
    }

    fn set_retained(&self, retained: Waker) {
        let previous = self.retained.lock().unwrap().replace(retained);
        drop(previous);
    }

    fn clear_retained(&self) {
        let retained = self.retained.lock().unwrap().take();
        drop(retained);
    }

    fn callback(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(in_flight, Ordering::SeqCst);

        let should_rewake = self
            .remaining_rewakes
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok();
        if should_rewake {
            let retained = self.retained.lock().unwrap().as_ref().cloned();
            if let Some(retained) = retained {
                retained.wake_by_ref();
            }
        }

        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }

    fn snapshot(&self) -> (usize, usize, usize) {
        (
            self.calls.load(Ordering::SeqCst),
            self.in_flight.load(Ordering::SeqCst),
            self.max_in_flight.load(Ordering::SeqCst),
        )
    }
}

#[derive(Default)]
struct ReplayTargetState {
    a_calls: usize,
    b_calls: usize,
    in_flight: usize,
    max_in_flight: usize,
    a_released: bool,
}

#[derive(Default)]
struct ReplayTargetProbe {
    state: Mutex<ReplayTargetState>,
    changed: Condvar,
}

impl ReplayTargetProbe {
    fn callback_a(&self) {
        let mut state = self.state.lock().unwrap();
        state.a_calls += 1;
        state.in_flight += 1;
        state.max_in_flight = state.max_in_flight.max(state.in_flight);
        self.changed.notify_all();
        while !state.a_released {
            state = self.changed.wait(state).unwrap();
        }
        state.in_flight -= 1;
        self.changed.notify_all();
    }

    fn callback_b(&self) {
        let mut state = self.state.lock().unwrap();
        state.b_calls += 1;
        state.in_flight += 1;
        state.max_in_flight = state.max_in_flight.max(state.in_flight);
        state.in_flight -= 1;
        self.changed.notify_all();
    }

    fn wait_until_a_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.state.lock().unwrap();
        while state.a_calls == 0 || state.in_flight == 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "prompt replay target A callback did not run"
            );
            let waited = self.changed.wait_timeout(state, remaining).unwrap();
            state = waited.0;
        }
    }

    fn release_a(&self) {
        let mut state = self.state.lock().unwrap();
        state.a_released = true;
        self.changed.notify_all();
    }

    fn snapshot(&self) -> (usize, usize, usize, usize) {
        let state = self.state.lock().unwrap();
        (
            state.a_calls,
            state.b_calls,
            state.in_flight,
            state.max_in_flight,
        )
    }
}

struct ReplayTargetA {
    probe: Arc<ReplayTargetProbe>,
    teardown: Option<Waker>,
}

impl Wake for ReplayTargetA {
    fn wake(self: Arc<Self>) {
        self.probe.callback_a();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.probe.callback_a();
    }
}

impl Drop for ReplayTargetA {
    fn drop(&mut self) {
        let teardown = self.teardown.take();
        drop(teardown);
    }
}

struct ReplayTargetB {
    probe: Arc<ReplayTargetProbe>,
}

impl Wake for ReplayTargetB {
    fn wake(self: Arc<Self>) {
        self.probe.callback_b();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.probe.callback_b();
    }
}

struct ReplayTargetARelease(Arc<ReplayTargetProbe>);

impl Drop for ReplayTargetARelease {
    fn drop(&mut self) {
        self.0.release_a();
    }
}

#[derive(Debug)]
struct PrimaryCallbackPanic;

#[derive(Debug)]
struct SecondaryTargetPanic;

#[derive(Debug)]
struct SecondaryPayloadDropPanic;

impl Drop for SecondaryTargetPanic {
    fn drop(&mut self) {
        panic_any(SecondaryPayloadDropPanic);
    }
}

struct DualPanicReplayTarget {
    probe: Arc<ReplayTargetProbe>,
    teardown: Option<Waker>,
}

impl Wake for DualPanicReplayTarget {
    fn wake(self: Arc<Self>) {
        self.probe.callback_a();
        panic_any(PrimaryCallbackPanic);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.probe.callback_a();
        panic_any(PrimaryCallbackPanic);
    }
}

impl Drop for DualPanicReplayTarget {
    fn drop(&mut self) {
        let teardown = self.teardown.take();
        drop(teardown);
    }
}

const REENTRANT_DELIVERY_LIMIT: usize = 256;

struct ReentrantActivationDriver {
    commands: mpsc::Sender<ReentrantPollCommand>,
    completed: Mutex<mpsc::Receiver<ReentrantPollCompletion>>,
    retained: Mutex<Option<Waker>>,
    cancel_after_rewake: Option<(usize, CancellationToken)>,
    calls: AtomicUsize,
    terminal_completion: Mutex<Option<ReentrantPollCompletion>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    remaining: AtomicUsize,
}

enum ReentrantPollCommand {
    Poll(Waker),
    DropExecution,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReentrantPollCompletion {
    Pending,
    Cancelled,
    PromptFailed,
    Dropped,
}

impl ReentrantActivationDriver {
    fn with_rewake_budget(
        commands: mpsc::Sender<ReentrantPollCommand>,
        completed: mpsc::Receiver<ReentrantPollCompletion>,
        wake_budget: usize,
    ) -> Self {
        Self::with_options(commands, completed, wake_budget, None)
    }

    fn continuously_rewaking(
        commands: mpsc::Sender<ReentrantPollCommand>,
        completed: mpsc::Receiver<ReentrantPollCompletion>,
    ) -> Self {
        Self::with_options(commands, completed, usize::MAX, None)
    }

    fn cancelling_after_second_rewake(
        commands: mpsc::Sender<ReentrantPollCommand>,
        completed: mpsc::Receiver<ReentrantPollCompletion>,
        cancellation: CancellationToken,
    ) -> Self {
        Self::with_options(commands, completed, 2, Some((2, cancellation)))
    }

    fn with_options(
        commands: mpsc::Sender<ReentrantPollCommand>,
        completed: mpsc::Receiver<ReentrantPollCompletion>,
        wake_budget: usize,
        cancel_after_rewake: Option<(usize, CancellationToken)>,
    ) -> Self {
        Self {
            commands,
            completed: Mutex::new(completed),
            retained: Mutex::new(None),
            cancel_after_rewake,
            calls: AtomicUsize::new(0),
            terminal_completion: Mutex::new(None),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            remaining: AtomicUsize::new(wake_budget),
        }
    }

    fn set_retained(&self, retained: Waker) {
        let previous = self.retained.lock().unwrap().replace(retained);
        drop(previous);
    }

    fn clear_retained(&self) {
        let retained = self.retained.lock().unwrap().take();
        drop(retained);
    }

    fn drop_execution(&self) {
        self.commands
            .send(ReentrantPollCommand::DropExecution)
            .unwrap();
        assert_eq!(
            self.completed.lock().unwrap().recv().unwrap(),
            ReentrantPollCompletion::Dropped
        );
    }

    fn poll_outer(&self, waker: &Waker) -> ReentrantPollCompletion {
        self.commands
            .send(ReentrantPollCommand::Poll(waker.clone()))
            .unwrap();
        self.completed.lock().unwrap().recv().unwrap()
    }

    fn callback(self: &Arc<Self>, wake: &Arc<ReentrantActivationWake>) {
        let callback = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(in_flight, Ordering::SeqCst);

        let outer_waker = Waker::from(Arc::clone(wake));
        let completion = self.poll_outer(&outer_waker);
        if completion != ReentrantPollCompletion::Pending {
            assert!(matches!(
                completion,
                ReentrantPollCompletion::Cancelled | ReentrantPollCompletion::PromptFailed
            ));
            let previous = self.terminal_completion.lock().unwrap().replace(completion);
            assert!(previous.is_none(), "execution completed more than once");
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        let should_rewake = self
            .remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok();
        if should_rewake {
            let retained = self.retained.lock().unwrap().as_ref().cloned();
            if let Some(retained) = retained {
                retained.wake_by_ref();
            }
        }
        if self
            .cancel_after_rewake
            .as_ref()
            .is_some_and(|(cancel_after, _)| *cancel_after == callback)
        {
            assert!(
                self.cancel_after_rewake
                    .as_ref()
                    .expect("matched cancellation configuration")
                    .1
                    .cancel()
            );
        }

        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }

    fn snapshot(&self) -> (usize, usize, usize, usize) {
        (
            self.calls.load(Ordering::SeqCst),
            self.in_flight.load(Ordering::SeqCst),
            self.max_in_flight.load(Ordering::SeqCst),
            self.remaining.load(Ordering::SeqCst),
        )
    }

    fn terminal_completion(&self) -> Option<ReentrantPollCompletion> {
        *self.terminal_completion.lock().unwrap()
    }
}

fn classify_reentrant_poll(polled: Poll<Result<ToolOutput, ToolError>>) -> ReentrantPollCompletion {
    match polled {
        Poll::Pending => ReentrantPollCompletion::Pending,
        Poll::Ready(Err(error)) if error.kind == ToolErrorKind::Cancelled => {
            assert_error(
                &error,
                ToolErrorKind::Cancelled,
                "ask_user_question_cancelled",
                "ask_user_question was cancelled",
                false,
            );
            ReentrantPollCompletion::Cancelled
        }
        Poll::Ready(Err(error)) => {
            assert_error(
                &error,
                ToolErrorKind::Execution,
                "ask_user_question_prompt_failed",
                "ask_user_question prompt failed",
                false,
            );
            ReentrantPollCompletion::PromptFailed
        }
        Poll::Ready(Ok(output)) => {
            panic!("reentrant prompt unexpectedly returned output: {output:?}")
        }
    }
}

fn run_reentrant_poller(
    mut execution: BoxFuture<'_, Result<ToolOutput, ToolError>>,
    command_receiver: &mpsc::Receiver<ReentrantPollCommand>,
    completed_sender: &mpsc::Sender<ReentrantPollCompletion>,
) {
    while let Ok(command) = command_receiver.recv() {
        match command {
            ReentrantPollCommand::Poll(waker) => {
                completed_sender
                    .send(classify_reentrant_poll(
                        execution.as_mut().poll(&mut Context::from_waker(&waker)),
                    ))
                    .unwrap();
            }
            ReentrantPollCommand::DropExecution => {
                drop(execution);
                completed_sender
                    .send(ReentrantPollCompletion::Dropped)
                    .unwrap();
                break;
            }
        }
    }
}

struct ReentrantActivationWake {
    driver: Weak<ReentrantActivationDriver>,
}

impl ReentrantActivationWake {
    fn callback(self: &Arc<Self>) {
        if let Some(driver) = self.driver.upgrade() {
            driver.callback(self);
        }
    }
}

impl Wake for ReentrantActivationWake {
    fn wake(self: Arc<Self>) {
        self.callback();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.callback();
    }
}

impl Wake for BlockingWake {
    fn wake(self: Arc<Self>) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
    }
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

fn poll_ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    match poll_once(future.as_mut()) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("ask_user_question future unexpectedly remained pending"),
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("question-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("question-incarnation").unwrap(),
        turn_id: TurnId::new("question-turn").unwrap(),
        call_id: ToolCallId::new("question-call").unwrap(),
    }
}

fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("question-call").unwrap(),
        name: ToolName::new(ASK_USER_QUESTION_TOOL_NAME).unwrap(),
        arguments,
    }
}

fn basic_arguments() -> Value {
    json!({
        "questions": [{
            "question": "Which path?",
            "options": [
                {"label": "First", "description": "The first path"},
                {"label": "Second"}
            ]
        }]
    })
}

#[derive(Clone)]
struct BoundaryOption {
    label: String,
    description: String,
}

#[derive(Clone)]
struct BoundaryQuestion {
    question: String,
    options: Vec<BoundaryOption>,
}

fn boundary_value(questions: &[BoundaryQuestion]) -> Value {
    json!({
        "questions": questions.iter().map(|question| json!({
            "question": question.question,
            "options": question.options.iter().map(|option| json!({
                "label": option.label,
                "description": option.description,
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })
}

fn distributed_boundary_value(mut c1_scalars: usize, mut literal_backslashes: usize) -> Value {
    fn fill(
        base: String,
        raw_limit: usize,
        c1_scalars: &mut usize,
        literal_backslashes: &mut usize,
    ) -> String {
        let mut value = base;
        let mut remaining = raw_limit - value.len();
        let expanded = (*c1_scalars).min(remaining / '\u{80}'.len_utf8());
        value.extend(std::iter::repeat_n('\u{80}', expanded));
        *c1_scalars -= expanded;
        remaining -= expanded * '\u{80}'.len_utf8();

        let escaped = (*literal_backslashes).min(remaining);
        value.extend(std::iter::repeat_n('\\', escaped));
        *literal_backslashes -= escaped;
        value
    }

    let questions = (0..MAX_ASK_USER_QUESTION_QUESTIONS)
        .map(|question| BoundaryQuestion {
            question: fill(
                format!("q{question}"),
                MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES,
                &mut c1_scalars,
                &mut literal_backslashes,
            ),
            options: (0..MAX_ASK_USER_QUESTION_OPTIONS_PER_QUESTION)
                .map(|option| BoundaryOption {
                    label: fill(
                        format!("l{question}-{option}"),
                        MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES,
                        &mut c1_scalars,
                        &mut literal_backslashes,
                    ),
                    description: fill(
                        "d".to_owned(),
                        MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES,
                        &mut c1_scalars,
                        &mut literal_backslashes,
                    ),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    assert_eq!(c1_scalars, 0, "fixture lacks C1 scalar capacity");
    assert_eq!(
        literal_backslashes, 0,
        "fixture lacks literal-backslash capacity"
    );
    boundary_value(&questions)
}

fn rendered_presentation_bytes(arguments: &Value) -> usize {
    arguments["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|question| {
            question["question"].as_str().unwrap().len()
                + question["options"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|option| {
                        option["label"].as_str().unwrap().len()
                            + option
                                .get("description")
                                .and_then(Value::as_str)
                                .map_or(0, str::len)
                    })
                    .sum::<usize>()
        })
        .sum()
}

fn deeply_nested_array(depth: usize) -> Value {
    let mut nested = Value::Null;
    for _ in 0..depth {
        nested = Value::Array(vec![nested]);
    }
    nested
}

fn serialized_argument_boundary(target: usize) -> Value {
    let mut questions = (0..4)
        .map(|question| BoundaryQuestion {
            question: format!("question-{question}"),
            options: (0..6)
                .map(|option| BoundaryOption {
                    label: format!("option-{question}-{option}"),
                    description: String::new(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let base = serde_json::to_vec(&boundary_value(&questions))
        .unwrap()
        .len();
    assert!(base <= target);
    let mut escaped = (target - base) / 2;
    for question in &mut questions {
        for option in &mut question.options {
            let count = escaped.min(MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES);
            option.description.extend(std::iter::repeat_n('\\', count));
            escaped -= count;
        }
    }
    for question in &mut questions {
        let capacity = MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES - question.question.len();
        let count = escaped.min(capacity);
        question.question.extend(std::iter::repeat_n('\\', count));
        escaped -= count;
    }
    assert_eq!(escaped, 0, "fixture lacks enough escaped string capacity");
    if (target - base) % 2 == 1 {
        let question = questions.last_mut().unwrap();
        assert!(question.question.len() < MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES);
        question.question.push('x');
    }
    let value = boundary_value(&questions);
    assert_eq!(serde_json::to_vec(&value).unwrap().len(), target);
    value
}

fn prepare(tool: &AskUserQuestionTool, arguments: Value) -> Value {
    let prepared = tool.prepare(call(arguments)).unwrap();
    assert_eq!(
        prepared.authorization(),
        &PreparedToolAuthorization::NoAuthorityRequired
    );
    prepared.arguments().clone()
}

fn execute(
    tool: &AskUserQuestionTool,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    poll_ready(tool.execute(context(), arguments, cancellation))
}

fn assert_error(
    error: &ToolError,
    kind: ToolErrorKind,
    code: &str,
    message: &str,
    retryable: bool,
) {
    assert_eq!(error.kind, kind);
    assert_eq!(error.code, code);
    assert_eq!(error.message, message);
    assert_eq!(error.retryable, retryable);
}

fn assert_retained_prompt_capacity_recovers_after_last_clone(
    tool: &AskUserQuestionTool,
    prompter: &PromptWakerFanoutPrompter,
    prepared: &Value,
    retained_wakers: &mut Vec<Waker>,
) {
    while retained_wakers.len() > 1 {
        drop(retained_wakers.pop());
        let busy = poll_ready(tool.execute(context(), prepared.clone(), CancellationToken::new()))
            .unwrap_err();
        assert_error(
            &busy,
            ToolErrorKind::Unavailable,
            "ask_user_question_busy",
            "ask_user_question prompt capacity is exhausted",
            true,
        );
        assert_eq!(prompter.calls(), 1);
    }
    drop(retained_wakers.pop());

    let mut recovered =
        Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(prompter.calls(), 2);
    drop(recovered);
}

fn assert_invalid(tool: &AskUserQuestionTool, arguments: Value) {
    let error = tool.prepare(call(arguments)).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_invalid_arguments",
        "ask_user_question arguments are invalid",
        false,
    );
}

#[test]
fn public_limits_and_schema_are_exact_and_strict() {
    assert_eq!(ASK_USER_QUESTION_DEFAULT_MAX_ACTIVE_PROMPTS, 1);
    assert_eq!(ASK_USER_QUESTION_MAX_ACTIVE_PROMPTS, 8);
    assert_eq!(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES, 32_768);
    assert_eq!(MAX_ASK_USER_QUESTION_QUESTIONS, 4);
    assert_eq!(MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES, 1_024);
    assert_eq!(MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES, 4_096);
    assert_eq!(MAX_ASK_USER_QUESTION_OPTIONS_PER_QUESTION, 6);
    assert_eq!(MAX_ASK_USER_QUESTION_TOTAL_OPTIONS, 24);
    assert_eq!(MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES, 128);
    assert_eq!(MAX_ASK_USER_QUESTION_RENDERED_OPTION_LABEL_BYTES, 512);
    assert_eq!(MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES, 512);
    assert_eq!(
        MAX_ASK_USER_QUESTION_RENDERED_OPTION_DESCRIPTION_BYTES,
        2_048
    );
    assert_eq!(MAX_ASK_USER_QUESTION_RENDERED_PRESENTATION_BYTES, 32_768);
    assert_eq!(
        MAX_ASK_USER_QUESTION_SERIALIZED_PREPARED_ARGUMENT_BYTES,
        49_152
    );
    assert_eq!(MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES, 4_096);
    assert_eq!(MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES, 4_096);
    assert_eq!(MAX_ASK_USER_QUESTION_RENDERED_ANSWER_BYTES, 16_384);
    assert_eq!(MAX_ASK_USER_QUESTION_SERIALIZED_RESULT_BYTES, 49_152);

    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let spec = tool.spec();
    assert_eq!(spec.name.as_str(), ASK_USER_QUESTION_TOOL_NAME);
    assert_eq!(spec.input_schema["type"], "object");
    assert_eq!(spec.input_schema["additionalProperties"], false);
    assert_eq!(spec.input_schema["required"], json!(["questions"]));
    assert_eq!(
        spec.input_schema["properties"].as_object().unwrap().len(),
        1
    );
    let questions = &spec.input_schema["properties"]["questions"];
    assert_eq!(questions["minItems"], 1);
    assert_eq!(questions["maxItems"], 4);
    assert!(
        !spec
            .input_schema
            .to_string()
            .contains("permission_request_id")
    );
}

#[test]
fn preparation_canonicalizes_in_order_and_success_allows_free_form_other() {
    let prompter = ScriptedPrompter::new([Ok(answered([
        "  an unlisted Other answer  ".to_owned(),
        "custom\u{1b}answer".to_owned(),
    ]))]);
    let tool = AskUserQuestionTool::new(prompter.clone());
    let arguments = json!({
        "questions": [
            {
                "question": " \n\u{1b}[31mChoose\u{200b}\u{061c} now\r ",
                "options": [
                    {"label": "  Alpha  ", "description": "  visible\nline  "},
                    {"label": "Beta", "description": " \t\r\n "}
                ]
            },
            {
                "question": "Next?",
                "options": [{"label": "One"}, {"label": "Two"}]
            }
        ]
    });

    let prepared = prepare(&tool, arguments);
    assert_eq!(
        prepared,
        json!({
            "questions": [
                {
                    "question": "\\x1b[31mChoose\\u{200b}\\u{061c} now",
                    "options": [
                        {"label": "Alpha", "description": "visible\\x0aline"},
                        {"label": "Beta"}
                    ]
                },
                {
                    "question": "Next?",
                    "options": [{"label": "One"}, {"label": "Two"}]
                }
            ]
        })
    );

    let output = execute(&tool, prepared, CancellationToken::new()).unwrap();
    assert_eq!(
        output,
        ToolOutput::success(json!([
            {
                "answer": "an unlisted Other answer",
                "question": "\\x1b[31mChoose\\u{200b}\\u{061c} now"
            },
            {
                "answer": "custom\\x1banswer",
                "question": "Next?"
            }
        ]))
    );
    assert_eq!(
        serde_json::to_string(&output.content).unwrap(),
        r#"[{"answer":"an unlisted Other answer","question":"\\x1b[31mChoose\\u{200b}\\u{061c} now"},{"answer":"custom\\x1banswer","question":"Next?"}]"#
    );
    assert_eq!(prompter.calls(), 1);
    assert_eq!(
        prompter.requests(),
        vec![vec![
            RecordedQuestion {
                question: "\\x1b[31mChoose\\u{200b}\\u{061c} now".to_owned(),
                options: vec![
                    RecordedOption {
                        label: "Alpha".to_owned(),
                        description: Some("visible\\x0aline".to_owned()),
                    },
                    RecordedOption {
                        label: "Beta".to_owned(),
                        description: None,
                    },
                ],
            },
            RecordedQuestion {
                question: "Next?".to_owned(),
                options: vec![
                    RecordedOption {
                        label: "One".to_owned(),
                        description: None,
                    },
                    RecordedOption {
                        label: "Two".to_owned(),
                        description: None,
                    },
                ],
            },
        ]]
    );
}

#[test]
fn strict_shape_types_and_ascii_case_duplicate_labels_fail_before_prompting() {
    let prompter = ScriptedPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let invalid = [
        Value::Null,
        json!({}),
        json!({"questions": [], "unknown": true}),
        json!({"questions": "not-an-array"}),
        json!({"questions": []}),
        json!({"questions": [{}]}),
        json!({"questions": [{"question": 7, "options": [{"label":"a"},{"label":"b"}]}]}),
        json!({"questions": [{"question": " ", "options": [{"label":"a"},{"label":"b"}]}]}),
        json!({"questions": [{"question":"q","options":[]}]}),
        json!({"questions": [{"question":"q","options":[{"label":"a"}]}]}),
        json!({"questions": [{"question":"q","options":[{"label":"a"},{"label":"b","extra":0}]}]}),
        json!({"questions": [{"question":"q","options":[{"label":"a"},{"label":"b","description":7}]}]}),
        json!({"questions": [{"question":"q","options":[{"label":" Yes "},{"label":"yes"}]}]}),
    ];
    for arguments in invalid {
        assert_invalid(&tool, arguments);
    }
    assert_eq!(prompter.calls(), 0);
}

#[test]
fn duplicate_labels_are_compared_after_terminal_rendering_and_trim_is_ascii_exact() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    assert_invalid(
        &tool,
        json!({
            "questions": [{
                "question": "q",
                "options": [{"label":"\u{1b}"}, {"label":"\\x1b"}]
            }]
        }),
    );

    prepare(
        &tool,
        json!({
            "questions": [{
                "question": "q",
                "options": [{"label":"Ä"}, {"label":"ä"}]
            }]
        }),
    );

    let prepared = prepare(
        &tool,
        json!({
            "questions": [{
                "question": "\u{000b}question\u{000c}",
                "options": [{"label":"\u{000b}yes\u{000c}"}, {"label":"no"}]
            }]
        }),
    );
    assert_eq!(prepared["questions"][0]["question"], "\\x0bquestion\\x0c");
    assert_eq!(
        prepared["questions"][0]["options"][0]["label"],
        "\\x0byes\\x0c"
    );
}

#[test]
fn every_documented_terminal_unsafe_class_has_exact_encoding_evidence() {
    let mut unsafe_text = String::new();
    let mut expected = String::new();
    for code in 0_u32..=0x1f {
        unsafe_text.push(char::from_u32(code).unwrap());
        write!(&mut expected, "\\x{code:02x}").unwrap();
    }
    unsafe_text.push('\u{7f}');
    expected.push_str("\\x7f");
    for code in 0x80_u32..=0x9f {
        unsafe_text.push(char::from_u32(code).unwrap());
        write!(&mut expected, "\\u{{{code:04x}}}").unwrap();
    }
    for code in [0x061c]
        .into_iter()
        .chain(0x200b..=0x200f)
        .chain(0x2028..=0x202e)
        .chain(0x2060..=0x206f)
        .chain([0xfeff])
    {
        unsafe_text.push(char::from_u32(code).unwrap());
        write!(&mut expected, "\\u{{{code:04x}}}").unwrap();
    }

    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let prepared = prepare(
        &tool,
        json!({
            "questions": [{
                "question": unsafe_text,
                "options": [{"label": "first"}, {"label": "second"}]
            }]
        }),
    );
    assert_eq!(prepared["questions"][0]["question"], expected);
}

#[test]
fn direct_execute_enforces_raw_limits_without_rejecting_prepared_expansion() {
    let over_raw = [
        json!({
            "questions": [{
                "question": "q".repeat(MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES + 1),
                "options": [{"label": "first"}, {"label": "second"}]
            }]
        }),
        json!({
            "questions": [{
                "question": "question",
                "options": [
                    {"label": "l".repeat(MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES + 1)},
                    {"label": "second"}
                ]
            }]
        }),
        json!({
            "questions": [{
                "question": "question",
                "options": [
                    {
                        "label": "first",
                        "description": "d".repeat(
                            MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES + 1
                        )
                    },
                    {"label": "second"}
                ]
            }]
        }),
    ];
    for arguments in over_raw {
        let prompter = ScriptedPrompter::new([Ok(QuestionPromptOutcome::Unavailable)]);
        let tool = AskUserQuestionTool::new(prompter.clone());
        let error = execute(&tool, arguments, CancellationToken::new()).unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::InvalidInput,
            "ask_user_question_resource_limit",
            "ask_user_question resource limit exceeded",
            false,
        );
        assert_eq!(prompter.calls(), 0);
    }

    let prompter = ScriptedPrompter::new([Ok(QuestionPromptOutcome::Unavailable)]);
    let tool = AskUserQuestionTool::new(prompter.clone());
    let prepared = prepare(
        &tool,
        json!({
            "questions": [{
                "question": "\0".repeat(MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES),
                "options": [
                    {
                        "label": "\0".repeat(MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES),
                        "description": "\0".repeat(
                            MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES
                        )
                    },
                    {"label": "visible"}
                ]
            }]
        }),
    );
    assert_eq!(
        prepared["questions"][0]["question"].as_str().unwrap().len(),
        MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES
    );
    let output = execute(&tool, prepared, CancellationToken::new()).unwrap();
    assert_eq!(
        output,
        ToolOutput::success(
            "(ask_user_question is only available in the interactive shell; ask the user freeform instead)"
        )
    );
    assert_eq!(prompter.calls(), 1);
}

#[test]
fn permission_request_id_has_its_fixed_precedence_and_redacted_failure() {
    let marker = "PERMISSION_REQUEST_PRIVATE_MARKER";
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    for value in [Value::Null, json!(7), json!(marker)] {
        let error = tool
            .prepare(call(json!({
                "questions": [],
                "permission_request_id": value,
                "also_unknown": marker,
            })))
            .unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::InvalidInput,
            "ask_user_question_permission_request_unsupported",
            "ask_user_question permission escalation is not supported",
            false,
        );
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains(marker));
    }
}

#[test]
fn question_option_description_and_count_boundaries_are_inclusive() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let exact = json!({
        "questions": (0..4).map(|question| json!({
            "question": format!("{question}{}", "q".repeat(1_023)),
            "options": (0..6).map(|option| json!({
                "label": format!("{option}{}", "l".repeat(127)),
                "description": "d".repeat(512),
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    });
    prepare(&tool, exact);

    for arguments in [
        json!({"questions":[{"question":"q".repeat(1_025),"options":[{"label":"a"},{"label":"b"}]}]}),
        json!({"questions":[{"question":"q","options":[{"label":"a"},{"label":"l".repeat(129)}]}]}),
        json!({"questions":[{"question":"q","options":[{"label":"a"},{"label":"b","description":"d".repeat(513)}]}]}),
    ] {
        let error = tool.prepare(call(arguments)).unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::InvalidInput,
            "ask_user_question_resource_limit",
            "ask_user_question resource limit exceeded",
            false,
        );
    }
    for arguments in [
        json!({"questions": (0..5).map(|_| json!({"question":"q","options":[{"label":"a"},{"label":"b"}]})).collect::<Vec<_>>() }),
        json!({"questions":[{"question":"q","options":(0..7).map(|index| json!({"label":format!("o{index}")})).collect::<Vec<_>>()}]}),
    ] {
        assert_invalid(&tool, arguments);
    }
}

#[test]
fn terminal_encoding_reaches_each_exact_rendered_field_maximum() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let prepared = prepare(
        &tool,
        json!({
            "questions": [{
                "question": "\0".repeat(MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES),
                "options": [
                    {
                        "label": "\0".repeat(MAX_ASK_USER_QUESTION_RAW_OPTION_LABEL_BYTES),
                        "description": "\0".repeat(MAX_ASK_USER_QUESTION_RAW_OPTION_DESCRIPTION_BYTES)
                    },
                    {"label":"visible"}
                ]
            }]
        }),
    );
    let question = prepared["questions"][0]["question"].as_str().unwrap();
    let option = &prepared["questions"][0]["options"][0];
    assert_eq!(
        question.len(),
        MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES
    );
    assert_eq!(
        option["label"].as_str().unwrap().len(),
        MAX_ASK_USER_QUESTION_RENDERED_OPTION_LABEL_BYTES
    );
    assert_eq!(
        option["description"].as_str().unwrap().len(),
        MAX_ASK_USER_QUESTION_RENDERED_OPTION_DESCRIPTION_BYTES
    );
}

#[test]
fn aggregate_rendered_presentation_limit_is_checked_without_truncation() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let exact = distributed_boundary_value(4_080, 0);
    assert_eq!(serde_json::to_vec(&exact).unwrap().len(), 9_135);
    let exact = prepare(&tool, exact);
    assert_eq!(
        rendered_presentation_bytes(&exact),
        MAX_ASK_USER_QUESTION_RENDERED_PRESENTATION_BYTES
    );

    let over = distributed_boundary_value(4_080, 1);
    assert_eq!(serde_json::to_vec(&over).unwrap().len(), 9_137);
    let error = tool.prepare(call(over)).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );
}

#[test]
fn incoming_serialized_argument_boundary_is_exact_and_checked_first() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let exact = serialized_argument_boundary(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES);
    assert_eq!(
        serde_json::to_vec(&exact).unwrap().len(),
        MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES
    );
    prepare(&tool, exact);

    let over = serialized_argument_boundary(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES + 1);
    assert_eq!(
        serde_json::to_vec(&over).unwrap().len(),
        MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES + 1
    );
    let error = tool.prepare(call(over.clone())).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );

    let marker = "PRIVATE_OVERSIZED_INPUT_MARKER";
    let mut over = over.as_object().unwrap().clone();
    over.insert(marker.to_owned(), Value::Bool(true));
    let error = tool.prepare(call(Value::Object(over))).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );
    assert!(!format!("{error:?} {error}").contains(marker));
}

#[test]
fn normalized_prepared_serialization_boundary_is_exact_and_inclusive() {
    let tool = AskUserQuestionTool::new(ScriptedPrompter::default());
    let exact = distributed_boundary_value(2_443, 13_095);
    assert_eq!(serde_json::to_vec(&exact).unwrap().len(), 32_051);
    let exact = prepare(&tool, exact);
    assert_eq!(rendered_presentation_bytes(&exact), 32_767);
    assert_eq!(
        serde_json::to_vec(&exact).unwrap().len(),
        MAX_ASK_USER_QUESTION_SERIALIZED_PREPARED_ARGUMENT_BYTES
    );

    let over = distributed_boundary_value(2_442, 13_100);
    assert_eq!(serde_json::to_vec(&over).unwrap().len(), 32_059);
    let error = tool.prepare(call(over)).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );
}

#[test]
fn direct_execute_accepts_only_canonical_prepared_arguments() {
    let prompter = ScriptedPrompter::new([Ok(QuestionPromptOutcome::Unavailable)]);
    let tool = AskUserQuestionTool::new(prompter.clone());
    let error = execute(
        &tool,
        json!({
            "questions": [{
                "question": "  noncanonical question  ",
                "options": [{"label":"yes"}, {"label":"no"}]
            }]
        }),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_invalid_arguments",
        "ask_user_question arguments are invalid",
        false,
    );
    assert_eq!(prompter.calls(), 0);
}

#[test]
fn deeply_nested_direct_input_is_rejected_without_invoking_the_prompt() {
    let prompter = ScriptedPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let mut nested = Value::Null;
    for _ in 0..2_048 {
        nested = Value::Array(vec![nested]);
    }
    let mut root = serde_json::Map::new();
    root.insert("questions".to_owned(), Value::Array(Vec::new()));
    root.insert("unknown".to_owned(), nested);
    let error = execute(&tool, Value::Object(root), CancellationToken::new()).unwrap_err();
    assert_eq!(error.kind, ToolErrorKind::InvalidInput);
    assert_eq!(prompter.calls(), 0);
}

#[test]
fn serialized_size_rejects_exact_plus_one_strings_and_keys_before_prompting() {
    let prompter = ScriptedPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());

    let exact_string =
        Value::String("s".repeat(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES - 2));
    assert_eq!(
        serde_json::to_vec(&exact_string).unwrap().len(),
        MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES
    );
    assert_invalid(&tool, exact_string);
    let over_string =
        Value::String("s".repeat(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES - 1));
    let error = tool.prepare(call(over_string)).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );

    let mut exact_key = serde_json::Map::new();
    exact_key.insert(
        "k".repeat(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES - 9),
        Value::Null,
    );
    let exact_key = Value::Object(exact_key);
    assert_eq!(
        serde_json::to_vec(&exact_key).unwrap().len(),
        MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES
    );
    assert_invalid(&tool, exact_key);
    let mut over_key = serde_json::Map::new();
    over_key.insert(
        "k".repeat(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES - 8),
        Value::Null,
    );
    let error = tool.prepare(call(Value::Object(over_key))).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );

    let oversized = Value::String("s".repeat(MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES * 32));
    let error = tool.prepare(call(oversized)).unwrap_err();
    assert_eq!(error.code, "ask_user_question_resource_limit");
    assert_eq!(prompter.calls(), 0);
}

#[test]
fn maximum_depth_drop_paths_are_iterative_on_a_small_stack() {
    const SMALL_STACK_BYTES: usize = 64 * 1024;
    const EXACT_INCOMING_ROOT_ARRAY_DEPTH: usize =
        (MAX_ASK_USER_QUESTION_SERIALIZED_ARGUMENT_BYTES - 4) / 2;
    const EXACT_PREPARED_ROOT_ARRAY_DEPTH: usize =
        (MAX_ASK_USER_QUESTION_SERIALIZED_PREPARED_ARGUMENT_BYTES - 4) / 2;

    std::thread::Builder::new()
        .name("ask-question-iterative-drop".to_owned())
        .stack_size(SMALL_STACK_BYTES)
        .spawn(|| {
            let prompter = ScriptedPrompter::default();
            let tool = AskUserQuestionTool::new(prompter.clone());

            let error = tool
                .prepare(call(deeply_nested_array(EXACT_INCOMING_ROOT_ARRAY_DEPTH)))
                .unwrap_err();
            assert_eq!(error.code, "ask_user_question_invalid_arguments");

            let error = tool
                .prepare(ToolCall {
                    id: ToolCallId::new("wrong-name-deep-call").unwrap(),
                    name: ToolName::new("wrong_name").unwrap(),
                    arguments: deeply_nested_array(EXACT_INCOMING_ROOT_ARRAY_DEPTH),
                })
                .unwrap_err();
            assert_eq!(error.code, "ask_user_question_invalid_arguments");

            let unpolled = tool.execute(
                context(),
                deeply_nested_array(EXACT_PREPARED_ROOT_ARRAY_DEPTH),
                CancellationToken::new(),
            );
            drop(unpolled);

            let error = execute(
                &tool,
                deeply_nested_array(EXACT_PREPARED_ROOT_ARRAY_DEPTH + 1),
                CancellationToken::new(),
            )
            .unwrap_err();
            assert_eq!(error.code, "ask_user_question_resource_limit");
            assert_eq!(prompter.calls(), 0);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn cancellation_wins_before_prompt_and_over_every_same_poll_outcome() {
    let pre_cancelled = CancellationToken::new();
    assert!(pre_cancelled.cancel());
    let untouched = ScriptedPrompter::new([Ok(QuestionPromptOutcome::Unavailable)]);
    let tool = AskUserQuestionTool::new(untouched.clone());
    let prepared = prepare(&tool, basic_arguments());
    let error = execute(&tool, prepared, pre_cancelled).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "ask_user_question_cancelled",
        "ask_user_question was cancelled",
        false,
    );
    assert_eq!(untouched.calls(), 0);

    for outcome in [
        Ok(answered(["answer".to_owned()])),
        Ok(QuestionPromptOutcome::Cancelled),
        Ok(QuestionPromptOutcome::Unavailable),
        Err(QuestionPromptError::new()),
    ] {
        let cancellation = CancellationToken::new();
        let tool = AskUserQuestionTool::new(CancelThenReadyPrompter {
            cancellation: cancellation.clone(),
            outcome: Mutex::new(Some(outcome)),
        });
        let prepared = prepare(&tool, basic_arguments());
        let error = execute(&tool, prepared, cancellation).unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::Cancelled,
            "ask_user_question_cancelled",
            "ask_user_question was cancelled",
            false,
        );
    }

    let cancellation = CancellationToken::new();
    let tool = AskUserQuestionTool::new(DropCancelsReadyPrompter {
        cancellation: cancellation.clone(),
    });
    let prepared = prepare(&tool, basic_arguments());
    let error = execute(&tool, prepared, cancellation).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "ask_user_question_cancelled",
        "ask_user_question was cancelled",
        false,
    );
}

#[test]
fn cancellation_during_ready_activity_teardown_wins_over_every_prompt_result() {
    for outcome in [
        Ok(answered(["must not publish".to_owned()])),
        Err(QuestionPromptError::new()),
        Ok(answered([])),
    ] {
        let cancellation = CancellationToken::new();
        let callback_cancellation = cancellation.clone();
        let (waker, callback) = reentrant_waker(Callback::Drop, move || {
            let _ = callback_cancellation.cancel();
        });
        let tool = AskUserQuestionTool::new(ScriptedPrompter::new([outcome]));
        let prepared = prepare(&tool, basic_arguments());
        let mut execution = Box::pin(tool.execute(context(), prepared, cancellation.clone()));
        let result = match execution.as_mut().poll(&mut Context::from_waker(&waker)) {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("ready prompt unexpectedly left execution pending"),
        };

        assert!(cancellation.is_cancelled());
        assert_eq!(
            callback.calls(),
            1,
            "activity teardown must destroy one registered downstream Waker"
        );
        let error = result.expect_err("teardown cancellation must replace the prompt result");
        assert_error(
            &error,
            ToolErrorKind::Cancelled,
            "ask_user_question_cancelled",
            "ask_user_question was cancelled",
            false,
        );
    }
}

#[test]
fn unpolled_pending_drop_and_fail_fast_capacity_are_owned_and_recoverable() {
    let probe = Arc::new(PendingProbe::default());
    let tool = AskUserQuestionTool::with_max_active_prompts(
        PendingPrompter {
            probe: Arc::clone(&probe),
        },
        1,
    )
    .unwrap();
    let prepared = prepare(&tool, basic_arguments());

    let unpolled = tool.execute(context(), prepared.clone(), CancellationToken::new());
    drop(unpolled);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);

    let mut first = Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));
    assert!(poll_once(first.as_mut()).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.polls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.live.load(Ordering::SeqCst), 1);

    let busy = poll_ready(tool.execute(context(), prepared.clone(), CancellationToken::new()))
        .unwrap_err();
    assert_error(
        &busy,
        ToolErrorKind::Unavailable,
        "ask_user_question_busy",
        "ask_user_question prompt capacity is exhausted",
        true,
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);

    let cancelled = CancellationToken::new();
    assert!(cancelled.cancel());
    let error = poll_ready(tool.execute(context(), prepared.clone(), cancelled)).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "ask_user_question_cancelled",
        "ask_user_question was cancelled",
        false,
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);

    drop(first);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);

    let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
    assert_eq!(probe.live.load(Ordering::SeqCst), 1);
    drop(recovered);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 2);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);
}

#[test]
fn hard_eight_prompt_capacity_fails_ninth_and_is_independent_per_tool() {
    let probe = Arc::new(PendingProbe::default());
    let prompter = PendingPrompter {
        probe: Arc::clone(&probe),
    };
    let tool = AskUserQuestionTool::with_max_active_prompts(
        prompter.clone(),
        ASK_USER_QUESTION_MAX_ACTIVE_PROMPTS,
    )
    .unwrap();
    let independent = AskUserQuestionTool::with_max_active_prompts(prompter, 1).unwrap();
    let prepared = prepare(&tool, basic_arguments());

    let mut active = (0..ASK_USER_QUESTION_MAX_ACTIVE_PROMPTS)
        .map(|_| Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new())))
        .collect::<Vec<_>>();
    for future in &mut active {
        assert!(poll_once(future.as_mut()).is_pending());
    }
    assert_eq!(probe.calls.load(Ordering::SeqCst), 8);
    assert_eq!(probe.live.load(Ordering::SeqCst), 8);

    let ninth = poll_ready(tool.execute(context(), prepared.clone(), CancellationToken::new()))
        .unwrap_err();
    assert_error(
        &ninth,
        ToolErrorKind::Unavailable,
        "ask_user_question_busy",
        "ask_user_question prompt capacity is exhausted",
        true,
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 8);

    let mut independent_prompt =
        Box::pin(independent.execute(context(), prepared.clone(), CancellationToken::new()));
    assert!(poll_once(independent_prompt.as_mut()).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 9);
    assert_eq!(probe.live.load(Ordering::SeqCst), 9);

    drop(active.pop());
    assert_eq!(probe.live.load(Ordering::SeqCst), 8);
    let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 10);
    assert_eq!(probe.live.load(Ordering::SeqCst), 9);

    drop(active);
    drop(independent_prompt);
    drop(recovered);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 10);
}

#[test]
fn pending_drop_destroys_the_prompt_before_releasing_its_active_permit() {
    let state = Arc::new(ReentrantState::default());
    let tool = Arc::new(AskUserQuestionTool::new(ReentrantPrompter {
        state: Arc::clone(&state),
    }));
    *state.tool.lock().unwrap() = Some(Arc::downgrade(&tool));
    let prepared = prepare(&tool, basic_arguments());
    *state.arguments.lock().unwrap() = Some(prepared.clone());
    let mut pending = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(pending.as_mut()).is_pending());
    drop(pending);

    let result = state.observed.lock().unwrap().take().unwrap();
    let error = result.expect_err("reentrant prompt admission must remain busy during child drop");
    assert_error(
        &error,
        ToolErrorKind::Unavailable,
        "ask_user_question_busy",
        "ask_user_question prompt capacity is exhausted",
        true,
    );
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn outer_drop_destroys_the_cancellation_waker_before_releasing_capacity() {
    let probe = Arc::new(PendingProbe::default());
    let tool = Arc::new(AskUserQuestionTool::new(PendingPrompter {
        probe: Arc::clone(&probe),
    }));
    let prepared = prepare(&tool, basic_arguments());
    let state = Arc::new(WakerTeardownState::default());
    *state.tool.lock().unwrap() = Some(Arc::downgrade(&tool));
    *state.arguments.lock().unwrap() = Some(prepared.clone());

    let waker = Waker::from(Arc::new(AdmissionOnWakerDrop {
        state: Arc::clone(&state),
    }));
    let mut pending = Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    drop(waker);
    assert!(state.observed.lock().unwrap().is_none());

    drop(pending);
    let result = state.observed.lock().unwrap().take().unwrap();
    let error =
        result.expect_err("waker teardown must retain the originating prompt capacity permit");
    assert_error(
        &error,
        ToolErrorKind::Unavailable,
        "ask_user_question_busy",
        "ask_user_question prompt capacity is exhausted",
        true,
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);

    let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn in_flight_cancellation_wake_retains_capacity_until_callback_returns() {
    let probe = Arc::new(PendingProbe::default());
    let tool = AskUserQuestionTool::new(PendingPrompter {
        probe: Arc::clone(&probe),
    });
    let prepared = prepare(&tool, basic_arguments());
    let cancellation = CancellationToken::new();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let blocking = Arc::new(BlockingWake {
        entered: entered_tx,
        release: Mutex::new(release_rx),
        calls: AtomicUsize::new(0),
    });
    let waker = Waker::from(Arc::clone(&blocking));
    let mut active = Box::pin(tool.execute(context(), prepared.clone(), cancellation.clone()));
    assert!(
        active
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.live.load(Ordering::SeqCst), 1);
    drop(waker);

    let canceller = thread::spawn(move || cancellation.cancel());
    if entered_rx.recv_timeout(Duration::from_secs(2)).is_err() {
        let _ = release_tx.send(());
        let _ = canceller.join();
        panic!("cancellation did not enter the registered Waker callback");
    }

    drop(active);
    let prompt_drops_during_callback = probe.drops.load(Ordering::SeqCst);
    let prompt_live_during_callback = probe.live.load(Ordering::SeqCst);
    let calls_before_competing_admission = probe.calls.load(Ordering::SeqCst);
    let mut competing =
        Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));
    let competing_poll = poll_once(competing.as_mut());

    release_tx.send(()).unwrap();
    assert!(canceller.join().unwrap());
    let competing_error = match competing_poll {
        Poll::Ready(Err(error)) => Some(error),
        Poll::Ready(Ok(output)) => {
            panic!("competing prompt unexpectedly succeeded: {output:?}")
        }
        Poll::Pending => None,
    };
    drop(competing);

    assert_eq!(blocking.calls.load(Ordering::SeqCst), 1);
    assert_eq!(prompt_drops_during_callback, 1);
    assert_eq!(prompt_live_during_callback, 0);
    assert_eq!(calls_before_competing_admission, 1);
    let error = competing_error
        .expect("in-flight cancellation callback must retain originating prompt capacity");
    assert_error(
        &error,
        ToolErrorKind::Unavailable,
        "ask_user_question_busy",
        "ask_user_question prompt capacity is exhausted",
        true,
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);

    let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
    assert_eq!(probe.live.load(Ordering::SeqCst), 1);
    drop(recovered);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 2);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);
}

#[test]
fn cloned_prompt_wakers_replay_once_after_outer_repoll_observes_the_burst() {
    let prompter = PromptWakerFanoutPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let prepared = prepare(&tool, basic_arguments());
    let blocking = Arc::new(CountingBlockingCallback::new(false));
    let callback_blocking = Arc::clone(&blocking);
    let (waker, callback) = reentrant_waker(Callback::Wake, move || {
        callback_blocking.block();
    });
    let mut first = Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));

    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    let mut retained_wakers = prompter.take_wakers();
    assert_eq!(retained_wakers.len(), PROMPT_WAKER_FANOUT);
    let later_notification = retained_wakers.pop().unwrap();
    let initial_notifications = retained_wakers.len();

    thread::scope(|scope| {
        let _release_on_unwind = CountingCallbackRelease(Arc::clone(&blocking));
        let start = Arc::new(Barrier::new(initial_notifications + 1));
        let (returned_sender, returned_receiver) = mpsc::channel();
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

        for _ in 1..initial_notifications {
            returned_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("concurrent prompt Waker notifications did not coalesce and return");
        }
        assert_eq!(blocking.snapshot(), (1, 1, 1, 0));

        assert!(
            first
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        prompter.publish();
        later_notification.wake();
        blocking.release();
        returned_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("the forwarded prompt Waker callback did not replay and return");
    });

    assert_eq!(blocking.snapshot(), (2, 0, 1, 2));
    assert_eq!(callback.calls(), 2);
    let output = match first.as_mut().poll(&mut Context::from_waker(&waker)) {
        Poll::Ready(Ok(output)) => output,
        Poll::Ready(Err(error)) => panic!("published prompt failed: {error}"),
        Poll::Pending => panic!("published prompt remained pending"),
    };
    assert!(!output.is_error);
    drop(first);

    let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(prompter.calls(), 2);
    drop(recovered);
}

#[test]
fn completed_prompt_closes_retained_waker_delivery_until_every_clone_drops() {
    let prompter = PromptWakerFanoutPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let prepared = prepare(&tool, basic_arguments());
    let blocking = Arc::new(CountingBlockingCallback::new(true));
    let callback_blocking = Arc::clone(&blocking);
    let (waker, callback) = reentrant_waker(Callback::Wake, move || {
        callback_blocking.block();
    });
    let mut first = Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));

    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    let mut retained_wakers = prompter.take_wakers();
    assert_eq!(retained_wakers.len(), PROMPT_WAKER_FANOUT);
    prompter.publish();

    let stale_snapshot = thread::scope(|scope| {
        let _release_on_unwind = CountingCallbackRelease(Arc::clone(&blocking));
        let first_notification = retained_wakers[0].clone();
        let first_publisher = scope.spawn(move || first_notification.wake());
        blocking.wait_until_entered();

        let output = match first.as_mut().poll(&mut Context::from_waker(&waker)) {
            Poll::Ready(Ok(output)) => output,
            Poll::Ready(Err(error)) => panic!("published prompt failed: {error}"),
            Poll::Pending => panic!("published prompt remained pending"),
        };
        assert!(!output.is_error);
        drop(first);

        let stale_notification = retained_wakers[1].clone();
        scope
            .spawn(move || stale_notification.wake())
            .join()
            .unwrap();
        let stale_snapshot = blocking.snapshot();

        let busy = poll_ready(tool.execute(context(), prepared.clone(), CancellationToken::new()))
            .unwrap_err();
        assert_error(
            &busy,
            ToolErrorKind::Unavailable,
            "ask_user_question_busy",
            "ask_user_question prompt capacity is exhausted",
            true,
        );
        assert_eq!(prompter.calls(), 1);

        blocking.release();
        first_publisher.join().unwrap();
        stale_snapshot
    });

    assert_eq!(stale_snapshot, (1, 1, 1, 0));
    assert_eq!(blocking.snapshot(), (1, 0, 1, 1));
    assert_eq!(callback.calls(), 1);

    while retained_wakers.len() > 1 {
        drop(retained_wakers.pop());
        let busy = poll_ready(tool.execute(context(), prepared.clone(), CancellationToken::new()))
            .unwrap_err();
        assert_error(
            &busy,
            ToolErrorKind::Unavailable,
            "ask_user_question_busy",
            "ask_user_question prompt capacity is exhausted",
            true,
        );
        assert_eq!(prompter.calls(), 1);
    }
    drop(retained_wakers.pop());

    let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(prompter.calls(), 2);
    drop(recovered);
}

#[test]
fn reentrant_prompt_wake_before_outer_repoll_has_constant_callback_work() {
    let prompter = PromptWakerFanoutPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let prepared = prepare(&tool, basic_arguments());
    let probe = Arc::new(ReentrantWakeProbe::new());
    let callback_probe = Arc::clone(&probe);
    let (waker, callback) = reentrant_waker(Callback::Wake, move || {
        callback_probe.callback();
    });
    let mut first = Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));

    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    let mut retained_wakers = prompter.take_wakers();
    assert_eq!(retained_wakers.len(), PROMPT_WAKER_FANOUT);
    probe.set_retained(retained_wakers[0].clone());

    retained_wakers[1].wake_by_ref();
    let amplified = probe.snapshot();
    let amplified_fixture_calls = callback.calls();
    probe.clear_retained();

    drop(first);
    retained_wakers[2].wake_by_ref();
    let closed = probe.snapshot();

    while retained_wakers.len() > 1 {
        drop(retained_wakers.pop());
        let busy = poll_ready(tool.execute(context(), prepared.clone(), CancellationToken::new()))
            .unwrap_err();
        assert_error(
            &busy,
            ToolErrorKind::Unavailable,
            "ask_user_question_busy",
            "ask_user_question prompt capacity is exhausted",
            true,
        );
        assert_eq!(prompter.calls(), 1);
    }
    drop(retained_wakers.pop());

    let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(prompter.calls(), 2);
    drop(recovered);

    assert_eq!(amplified, (1, 0, 1));
    assert_eq!(amplified_fixture_calls, 1);
    assert_eq!(closed, amplified);
}

#[test]
fn replay_target_drop_panic_clears_lane_for_a_fresh_notification() {
    let prompter = PromptWakerFanoutPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let prepared = prepare(&tool, basic_arguments());
    let probe = Arc::new(ReplayTargetProbe::default());
    let teardown_entries = Arc::new(AtomicUsize::new(0));
    let callback_entries = Arc::clone(&teardown_entries);
    let (teardown, _teardown_handle) = reentrant_waker(Callback::Drop, move || {
        callback_entries.fetch_add(1, Ordering::SeqCst);
        panic!("intentional replay target teardown panic");
    });
    let callback_owner = Arc::new(ReplayTargetA {
        probe: Arc::clone(&probe),
        teardown: Some(teardown),
    });
    let original_host_waker = Waker::from(Arc::clone(&callback_owner));
    drop(callback_owner);
    let replay_host_waker = Waker::from(Arc::new(ReplayTargetB {
        probe: Arc::clone(&probe),
    }));
    let mut first = Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));

    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(&original_host_waker))
            .is_pending()
    );
    let mut retained_wakers = prompter.take_wakers();
    assert_eq!(retained_wakers.len(), PROMPT_WAKER_FANOUT);

    let wake_result = thread::scope(|scope| {
        let _release_on_unwind = ReplayTargetARelease(Arc::clone(&probe));
        let first_notification = retained_wakers[0].clone();
        let publisher =
            scope.spawn(move || catch_unwind(AssertUnwindSafe(|| first_notification.wake())));
        probe.wait_until_a_entered();

        assert!(
            first
                .as_mut()
                .poll(&mut Context::from_waker(&replay_host_waker))
                .is_pending()
        );
        retained_wakers[1].wake_by_ref();
        drop(original_host_waker);
        probe.release_a();
        publisher.join().unwrap()
    });

    retained_wakers[2].wake_by_ref();
    let fresh = probe.snapshot();
    drop(first);
    retained_wakers[3].wake_by_ref();
    let closed = probe.snapshot();

    assert_retained_prompt_capacity_recovers_after_last_clone(
        &tool,
        &prompter,
        &prepared,
        &mut retained_wakers,
    );

    assert!(wake_result.is_err());
    assert_eq!(teardown_entries.load(Ordering::SeqCst), 1);
    assert_eq!(fresh, (1, 1, 0, 1));
    assert_eq!(closed, fresh);
}

#[test]
fn replay_target_drop_close_suppresses_selected_replay_and_retains_capacity() {
    let prompter = PromptWakerFanoutPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let prepared = prepare(&tool, basic_arguments());
    let probe = Arc::new(ReplayTargetProbe::default());
    let (close_request_sender, close_request_receiver) = mpsc::channel();
    let (close_complete_sender, close_complete_receiver) = mpsc::channel();
    let close_complete_receiver = Mutex::new(close_complete_receiver);
    let (teardown, teardown_handle) = reentrant_waker(Callback::Drop, move || {
        close_request_sender.send(()).unwrap();
        close_complete_receiver.lock().unwrap().recv().unwrap();
    });
    let callback_owner = Arc::new(ReplayTargetA {
        probe: Arc::clone(&probe),
        teardown: Some(teardown),
    });
    let original_host_waker = Waker::from(Arc::clone(&callback_owner));
    drop(callback_owner);
    let replay_host_waker = Waker::from(Arc::new(ReplayTargetB {
        probe: Arc::clone(&probe),
    }));
    let mut first = Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));

    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(&original_host_waker))
            .is_pending()
    );
    let mut retained_wakers = prompter.take_wakers();
    assert_eq!(retained_wakers.len(), PROMPT_WAKER_FANOUT);

    thread::scope(|scope| {
        let _release_on_unwind = ReplayTargetARelease(Arc::clone(&probe));
        let first_notification = retained_wakers[0].clone();
        let publisher = scope.spawn(move || first_notification.wake());
        probe.wait_until_a_entered();

        assert!(
            first
                .as_mut()
                .poll(&mut Context::from_waker(&replay_host_waker))
                .is_pending()
        );
        retained_wakers[1].wake_by_ref();
        drop(original_host_waker);
        probe.release_a();

        close_request_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("replay target A destructor did not request notifier close");
        drop(first);
        close_complete_sender.send(()).unwrap();
        publisher.join().unwrap();
    });

    retained_wakers[2].wake_by_ref();
    let closed = probe.snapshot();

    assert_retained_prompt_capacity_recovers_after_last_clone(
        &tool,
        &prompter,
        &prepared,
        &mut retained_wakers,
    );

    assert_eq!(teardown_handle.calls(), 1);
    assert_eq!(closed, (1, 0, 0, 1));
}

#[test]
fn callback_panic_precedes_panicking_replay_target_payload_drop() {
    let prompter = PromptWakerFanoutPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let prepared = prepare(&tool, basic_arguments());
    let probe = Arc::new(ReplayTargetProbe::default());
    let teardown_entries = Arc::new(AtomicUsize::new(0));
    let callback_entries = Arc::clone(&teardown_entries);
    let (teardown, _teardown_handle) = reentrant_waker(Callback::Drop, move || {
        callback_entries.fetch_add(1, Ordering::SeqCst);
        panic_any(SecondaryTargetPanic);
    });
    let callback_owner = Arc::new(DualPanicReplayTarget {
        probe: Arc::clone(&probe),
        teardown: Some(teardown),
    });
    let original_host_waker = Waker::from(Arc::clone(&callback_owner));
    drop(callback_owner);
    let replay_host_waker = Waker::from(Arc::new(ReplayTargetB {
        probe: Arc::clone(&probe),
    }));
    let mut first = Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));

    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(&original_host_waker))
            .is_pending()
    );
    let mut retained_wakers = prompter.take_wakers();
    assert_eq!(retained_wakers.len(), PROMPT_WAKER_FANOUT);

    let wake_result = thread::scope(|scope| {
        let _release_on_unwind = ReplayTargetARelease(Arc::clone(&probe));
        let first_notification = retained_wakers[0].clone();
        let publisher =
            scope.spawn(move || catch_unwind(AssertUnwindSafe(|| first_notification.wake())));
        probe.wait_until_a_entered();

        assert!(
            first
                .as_mut()
                .poll(&mut Context::from_waker(&replay_host_waker))
                .is_pending()
        );
        retained_wakers[1].wake_by_ref();
        drop(original_host_waker);
        probe.release_a();
        publisher.join().unwrap()
    });

    retained_wakers[2].wake_by_ref();
    let fresh = probe.snapshot();
    drop(first);
    retained_wakers[3].wake_by_ref();
    let closed = probe.snapshot();
    assert_retained_prompt_capacity_recovers_after_last_clone(
        &tool,
        &prompter,
        &prepared,
        &mut retained_wakers,
    );

    let payload = wake_result.expect_err("dual-panic wake unexpectedly succeeded");
    let callback_won = payload.is::<PrimaryCallbackPanic>();
    let secondary_payload_drop_won = payload.is::<SecondaryPayloadDropPanic>();
    let _payload_drop = catch_unwind(AssertUnwindSafe(|| drop(payload)));
    assert!(callback_won, "callback panic lost deterministic precedence");
    assert!(
        !secondary_payload_drop_won,
        "secondary panic-payload destruction replaced the callback panic"
    );
    assert_eq!(teardown_entries.load(Ordering::SeqCst), 1);
    assert_eq!(fresh, (1, 1, 0, 1));
    assert_eq!(closed, fresh);
}

#[test]
fn observed_residual_wakes_progress_without_an_unrelated_activation() {
    let prompter = PromptWakerFanoutPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let prepared = prepare(&tool, basic_arguments());
    let execution = tool.execute(context(), prepared.clone(), CancellationToken::new());
    let (command_sender, command_receiver) = mpsc::channel();
    let (completed_sender, completed_receiver) = mpsc::channel();
    let driver = Arc::new(ReentrantActivationDriver::with_rewake_budget(
        command_sender,
        completed_receiver,
        2,
    ));
    let host_wake = Arc::new(ReentrantActivationWake {
        driver: Arc::downgrade(&driver),
    });
    let host_waker = Waker::from(Arc::clone(&host_wake));

    let (progressed, terminal_completion, closed) = thread::scope(|scope| {
        let poller = scope.spawn(move || {
            run_reentrant_poller(execution, &command_receiver, &completed_sender);
        });

        assert_eq!(
            driver.poll_outer(&host_waker),
            ReentrantPollCompletion::Pending
        );
        let mut retained_wakers = prompter.take_wakers();
        assert_eq!(retained_wakers.len(), PROMPT_WAKER_FANOUT);
        driver.set_retained(retained_wakers[0].clone());

        retained_wakers[1].wake_by_ref();
        let progressed = driver.snapshot();
        let terminal_completion = driver.terminal_completion();

        driver.clear_retained();
        driver.drop_execution();
        retained_wakers[3].wake_by_ref();
        let closed = driver.snapshot();
        assert_retained_prompt_capacity_recovers_after_last_clone(
            &tool,
            &prompter,
            &prepared,
            &mut retained_wakers,
        );
        poller.join().unwrap();
        (progressed, terminal_completion, closed)
    });

    assert_eq!(progressed, (3, 0, 1, 0));
    assert_eq!(terminal_completion, None);
    assert_eq!(closed, progressed);
}

#[test]
fn continuously_rewaking_prompt_stops_at_the_delivery_limit_with_redacted_error() {
    let prompter = PromptWakerFanoutPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let prepared = prepare(&tool, basic_arguments());
    let execution = tool.execute(context(), prepared.clone(), CancellationToken::new());
    let (command_sender, command_receiver) = mpsc::channel();
    let (completed_sender, completed_receiver) = mpsc::channel();
    let driver = Arc::new(ReentrantActivationDriver::continuously_rewaking(
        command_sender,
        completed_receiver,
    ));
    let host_wake = Arc::new(ReentrantActivationWake {
        driver: Arc::downgrade(&driver),
    });
    let host_waker = Waker::from(Arc::clone(&host_wake));

    let (progressed, terminal_completion, closed) = thread::scope(|scope| {
        let poller = scope.spawn(move || {
            run_reentrant_poller(execution, &command_receiver, &completed_sender);
        });

        assert_eq!(
            driver.poll_outer(&host_waker),
            ReentrantPollCompletion::Pending
        );
        let mut retained_wakers = prompter.take_wakers();
        assert_eq!(retained_wakers.len(), PROMPT_WAKER_FANOUT);
        driver.set_retained(retained_wakers[0].clone());

        retained_wakers[1].wake_by_ref();
        let progressed = driver.snapshot();
        let terminal_completion = driver.terminal_completion();

        driver.clear_retained();
        driver.drop_execution();
        retained_wakers[2].wake_by_ref();
        let closed = driver.snapshot();
        assert_retained_prompt_capacity_recovers_after_last_clone(
            &tool,
            &prompter,
            &prepared,
            &mut retained_wakers,
        );
        poller.join().unwrap();
        (progressed, terminal_completion, closed)
    });

    assert_eq!(progressed.0, REENTRANT_DELIVERY_LIMIT);
    assert_eq!(progressed.1, 0);
    assert_eq!(progressed.2, 1);
    assert_eq!(
        terminal_completion,
        Some(ReentrantPollCompletion::PromptFailed)
    );
    assert_eq!(closed, progressed);
}

#[test]
fn cancellation_in_the_residual_wake_window_progresses_and_closes_delivery() {
    let prompter = PromptWakerFanoutPrompter::default();
    let tool = AskUserQuestionTool::new(prompter.clone());
    let prepared = prepare(&tool, basic_arguments());
    let cancellation = CancellationToken::new();
    let execution = tool.execute(context(), prepared.clone(), cancellation.clone());
    let (command_sender, command_receiver) = mpsc::channel();
    let (completed_sender, completed_receiver) = mpsc::channel();
    let driver = Arc::new(ReentrantActivationDriver::cancelling_after_second_rewake(
        command_sender,
        completed_receiver,
        cancellation,
    ));
    let host_wake = Arc::new(ReentrantActivationWake {
        driver: Arc::downgrade(&driver),
    });
    let host_waker = Waker::from(Arc::clone(&host_wake));

    let (progressed, terminal_completion, closed) = thread::scope(|scope| {
        let poller = scope.spawn(move || {
            run_reentrant_poller(execution, &command_receiver, &completed_sender);
        });

        assert_eq!(
            driver.poll_outer(&host_waker),
            ReentrantPollCompletion::Pending
        );
        let mut retained_wakers = prompter.take_wakers();
        assert_eq!(retained_wakers.len(), PROMPT_WAKER_FANOUT);
        driver.set_retained(retained_wakers[0].clone());

        retained_wakers[1].wake_by_ref();
        let progressed = driver.snapshot();
        let terminal_completion = driver.terminal_completion();

        driver.clear_retained();
        driver.drop_execution();
        retained_wakers[2].wake_by_ref();
        let closed = driver.snapshot();
        assert_retained_prompt_capacity_recovers_after_last_clone(
            &tool,
            &prompter,
            &prepared,
            &mut retained_wakers,
        );
        poller.join().unwrap();
        (progressed, terminal_completion, closed)
    });

    assert_eq!(progressed, (3, 0, 1, 0));
    assert_eq!(
        terminal_completion,
        Some(ReentrantPollCompletion::Cancelled)
    );
    assert_eq!(closed, progressed);
}

#[test]
fn cancelling_a_pending_prompt_drops_it_and_releases_capacity() {
    let probe = Arc::new(PendingProbe::default());
    let tool = AskUserQuestionTool::new(PendingPrompter {
        probe: Arc::clone(&probe),
    });
    let prepared = prepare(&tool, basic_arguments());
    let cancellation = CancellationToken::new();
    let mut pending = Box::pin(tool.execute(context(), prepared.clone(), cancellation.clone()));
    assert!(poll_once(pending.as_mut()).is_pending());
    assert!(cancellation.cancel());
    let error = match poll_once(pending.as_mut()) {
        Poll::Ready(Err(error)) => error,
        other => panic!("cancelled prompt must finish immediately, got {other:?}"),
    };
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "ask_user_question_cancelled",
        "ask_user_question was cancelled",
        false,
    );
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    assert_eq!(probe.live.load(Ordering::SeqCst), 0);

    let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn prompt_call_and_poll_panics_release_capacity_during_unwind() {
    for panic_in_poll in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = Arc::new(PendingProbe::default());
        let tool = AskUserQuestionTool::new(PanicOncePrompter {
            calls: Arc::clone(&calls),
            panic_in_poll,
            pending_probe: Arc::clone(&probe),
        });
        let prepared = prepare(&tool, basic_arguments());
        let mut panicking =
            Box::pin(tool.execute(context(), prepared.clone(), CancellationToken::new()));
        assert!(catch_unwind(AssertUnwindSafe(|| poll_once(panicking.as_mut()))).is_err());
        drop(panicking);

        let mut recovered = Box::pin(tool.execute(context(), prepared, CancellationToken::new()));
        assert!(poll_once(recovered.as_mut()).is_pending());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(probe.live.load(Ordering::SeqCst), 1);
        drop(recovered);
        assert_eq!(probe.live.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn sentinels_and_invalid_prompt_responses_are_exact_and_redacted() {
    for (outcome, expected) in [
        (
            QuestionPromptOutcome::Cancelled,
            "(user cancelled the question)",
        ),
        (
            QuestionPromptOutcome::Unavailable,
            "(ask_user_question is only available in the interactive shell; ask the user freeform instead)",
        ),
    ] {
        let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(outcome)]));
        let prepared = prepare(&tool, basic_arguments());
        assert_eq!(
            execute(&tool, prepared, CancellationToken::new()).unwrap(),
            ToolOutput::success(expected)
        );
    }

    let private = "PRIVATE_INVALID_ANSWER_MARKER";
    for outcome in [Ok(answered([])), Ok(answered([" ".to_owned()]))] {
        let tool = AskUserQuestionTool::new(ScriptedPrompter::new([outcome]));
        let prepared = prepare(&tool, basic_arguments());
        let error = execute(&tool, prepared, CancellationToken::new()).unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::Execution,
            "ask_user_question_invalid_response",
            "ask_user_question prompt returned an invalid response",
            false,
        );
        assert!(!format!("{error:?} {error}").contains(private));
    }

    let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(answered([format!(
        "{private}{}",
        "x".repeat(4_096)
    )]))]));
    let prepared = prepare(&tool, basic_arguments());
    let error = execute(&tool, prepared, CancellationToken::new()).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );
    assert!(!format!("{error:?} {error}").contains(private));

    let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Err(QuestionPromptError::new())]));
    let prepared = prepare(&tool, basic_arguments());
    let error = execute(&tool, prepared, CancellationToken::new()).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Execution,
        "ask_user_question_prompt_failed",
        "ask_user_question prompt failed",
        false,
    );
}

#[test]
fn exact_answer_and_aggregate_answer_boundaries_are_enforced() {
    let exact = "a".repeat(4_096);
    let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(answered([exact.clone()]))]));
    let prepared = prepare(&tool, basic_arguments());
    assert_eq!(
        execute(&tool, prepared, CancellationToken::new())
            .unwrap()
            .content[0]["answer"],
        exact
    );

    let two_questions = json!({
        "questions": [
            {"question":"first?","options":[{"label":"a"},{"label":"b"}]},
            {"question":"second?","options":[{"label":"a"},{"label":"b"}]}
        ]
    });
    for (arguments, answers) in [
        (basic_arguments(), vec!["a".repeat(4_097)]),
        (two_questions, vec!["a".repeat(2_048), "b".repeat(2_049)]),
    ] {
        let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(answered(answers))]));
        let prepared = prepare(&tool, arguments);
        let error = execute(&tool, prepared, CancellationToken::new()).unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::InvalidInput,
            "ask_user_question_resource_limit",
            "ask_user_question resource limit exceeded",
            false,
        );
    }
}

#[test]
fn complete_pretrim_answer_bytes_have_an_exact_redacted_boundary() {
    let visible = "bounded";
    let leading = MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES / 2;
    let exact = format!(
        "{}{}{}",
        " ".repeat(leading),
        visible,
        "\t".repeat(MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES - leading - visible.len())
    );
    assert_eq!(exact.len(), MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES);

    let exact_tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(answered([exact]))]));
    let exact_prepared = prepare(&exact_tool, basic_arguments());
    assert_eq!(
        execute(&exact_tool, exact_prepared, CancellationToken::new())
            .unwrap()
            .content[0]["answer"],
        visible,
    );

    let private = "PRIVATE_PRETRIM_ANSWER_MARKER";
    let over = format!(
        "{private}{}",
        "\n".repeat(MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES + 1 - private.len())
    );
    assert_eq!(over.len(), MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES + 1);
    let over_tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(answered([over]))]));
    let over_prepared = prepare(&over_tool, basic_arguments());
    let error = execute(&over_tool, over_prepared, CancellationToken::new()).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );
    assert!(!format!("{error:?} {error}").contains(private));
}

#[test]
fn complete_pretrim_answer_bytes_share_the_aggregate_boundary() {
    let questions = json!({
        "questions": [
            {"question":"first?","options":[{"label":"a"},{"label":"b"}]},
            {"question":"second?","options":[{"label":"a"},{"label":"b"}]}
        ]
    });
    let per_answer = MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES / 2;
    let leading = per_answer / 2;
    let first = format!(
        "{}first{}",
        " ".repeat(leading),
        "\r".repeat(per_answer - leading - "first".len())
    );
    let second = format!(
        "{}second{}",
        "\t".repeat(leading),
        "\n".repeat(per_answer - leading - "second".len())
    );
    assert_eq!(first.len(), per_answer);
    assert_eq!(second.len(), per_answer);
    assert_eq!(
        first.len() + second.len(),
        MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES
    );

    let exact_tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(answered([
        first.clone(),
        second,
    ]))]));
    let exact_prepared = prepare(&exact_tool, questions.clone());
    assert_eq!(
        execute(&exact_tool, exact_prepared, CancellationToken::new())
            .unwrap()
            .content,
        json!([
            {"answer":"first","question":"first?"},
            {"answer":"second","question":"second?"}
        ]),
    );

    let private = "PRIVATE_AGGREGATE_PRETRIM_MARKER";
    let over_second = format!("{private}{}", " ".repeat(per_answer + 1 - private.len()));
    assert_eq!(
        first.len() + over_second.len(),
        MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES + 1
    );
    let over_tool =
        AskUserQuestionTool::new(ScriptedPrompter::new([Ok(answered([first, over_second]))]));
    let over_prepared = prepare(&over_tool, questions);
    let error = execute(&over_tool, over_prepared, CancellationToken::new()).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );
    assert!(!format!("{error:?} {error}").contains(private));
}

#[test]
fn worst_case_terminal_rendering_keeps_the_complete_tool_output_bounded() {
    let arguments = json!({
        "questions": (0..MAX_ASK_USER_QUESTION_QUESTIONS).map(|question| json!({
            "question": "\0".repeat(MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES),
            "options": [
                {"label": format!("a{question}")},
                {"label": format!("b{question}")}
            ]
        })).collect::<Vec<_>>()
    });
    let answers = (0..MAX_ASK_USER_QUESTION_QUESTIONS)
        .map(|_| "\0".repeat(MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES / 4))
        .collect::<Vec<_>>();
    let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(answered(answers))]));
    let prepared = prepare(&tool, arguments);
    let output = execute(&tool, prepared.clone(), CancellationToken::new()).unwrap();
    let serialized = serde_json::to_vec(&output).unwrap();
    // Each raw byte produces at most five compact-JSON bytes after terminal
    // rendering: a C0 byte becomes four ASCII bytes with one escaped backslash.
    // The 4,096 question plus 4,096 answer bytes therefore contribute 40,960
    // bytes; the four ordered pairs and complete ToolOutput envelope add 142.
    assert_eq!(serialized.len(), 41_102);
    assert!(serialized.len() <= MAX_ASK_USER_QUESTION_SERIALIZED_RESULT_BYTES);
    assert_eq!(
        output
            .content
            .as_array()
            .unwrap()
            .iter()
            .fold(0, |total, pair| total
                + pair["answer"].as_str().unwrap().len()),
        MAX_ASK_USER_QUESTION_RENDERED_ANSWER_BYTES,
    );

    let oversized_answers = [1_024, 1_024, 1_024, 1_025]
        .map(|bytes| "a".repeat(bytes))
        .into_iter()
        .collect::<Vec<_>>();
    let oversized =
        AskUserQuestionTool::new(ScriptedPrompter::new([Ok(answered(oversized_answers))]));
    let error = execute(&oversized, prepared, CancellationToken::new()).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::InvalidInput,
        "ask_user_question_resource_limit",
        "ask_user_question resource limit exceeded",
        false,
    );
}

#[test]
fn public_answer_batch_has_four_private_slots_and_preserves_count_errors() {
    let private = "PRIVATE_FIFTH_ANSWER_MARKER";
    for mismatched_count in [0, 2, 3] {
        let mut mismatched = QuestionPromptAnswers::new();
        for index in 0..mismatched_count {
            mismatched.try_push(format!("answer-{index}")).unwrap();
        }
        assert_eq!(mismatched.len(), mismatched_count);
        let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(
            QuestionPromptOutcome::Answered(mismatched),
        )]));
        let prepared = prepare(&tool, basic_arguments());
        let error = execute(&tool, prepared, CancellationToken::new()).unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::Execution,
            "ask_user_question_invalid_response",
            "ask_user_question prompt returned an invalid response",
            false,
        );
    }

    let mut bounded = QuestionPromptAnswers::new();
    assert!(bounded.is_empty());
    for answer in ["one", "two", "three", "four"] {
        bounded.try_push(answer.to_owned()).unwrap();
    }
    assert_eq!(bounded.len(), MAX_ASK_USER_QUESTION_QUESTIONS);
    assert_eq!(
        bounded.iter().collect::<Vec<_>>(),
        ["one", "two", "three", "four"]
    );
    assert_eq!(bounded.try_push(private.to_owned()).unwrap_err(), private);
    assert_eq!(bounded.len(), MAX_ASK_USER_QUESTION_QUESTIONS);
    assert!(!format!("{bounded:?}").contains(private));

    let tool = AskUserQuestionTool::new(ScriptedPrompter::new([Ok(
        QuestionPromptOutcome::Answered(bounded),
    )]));
    let prepared = prepare(&tool, basic_arguments());
    let error = execute(&tool, prepared, CancellationToken::new()).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Execution,
        "ask_user_question_invalid_response",
        "ask_user_question prompt returned an invalid response",
        false,
    );
    assert!(!format!("{error:?} {error}").contains(private));
}

#[test]
#[allow(clippy::default_constructed_unit_structs)]
fn construction_and_debug_contracts_are_data_free() {
    assert!(AskUserQuestionTool::with_max_active_prompts(ScriptedPrompter::default(), 1).is_ok());
    assert!(AskUserQuestionTool::with_max_active_prompts(ScriptedPrompter::default(), 8).is_ok());
    for limit in [0, 9, usize::MAX] {
        let error =
            AskUserQuestionTool::with_max_active_prompts(ScriptedPrompter::default(), limit)
                .unwrap_err();
        assert_eq!(error, AskUserQuestionConfigError::default());
        assert_eq!(error.to_string(), "invalid ask_user_question limits");
        assert_eq!(format!("{error:?}"), "AskUserQuestionConfigError");
    }
    assert_eq!(QuestionPromptError::new(), QuestionPromptError::default());
    assert_eq!(
        format!("{:?}", QuestionPromptError::new()),
        "QuestionPromptError"
    );

    let private = "PRIVATE_PROMPT_DEBUG_MARKER";
    let prompter = ScriptedPrompter::new([Ok(answered([private.to_owned()]))]);
    let tool = AskUserQuestionTool::shared_prompter(Arc::new(prompter));
    let prepared = tool
        .prepare(call(json!({
            "questions":[{"question":private,"options":[{"label":"yes","description":private},{"label":"no"}]}]
        })))
        .unwrap();
    for diagnostic in [format!("{tool:?}"), format!("{prepared:?}")] {
        assert!(!diagnostic.contains(private));
    }
    assert!(!format!("{:?}", answered([private.to_owned()])).contains(private));

    let debug = Arc::new(Mutex::new(None));
    let tool = AskUserQuestionTool::new(DebugCapturePrompter {
        debug: Arc::clone(&debug),
    });
    let prepared = prepare(
        &tool,
        json!({
            "questions":[{"question":private,"options":[{"label":"yes","description":private},{"label":"no"}]}]
        }),
    );
    execute(&tool, prepared, CancellationToken::new()).unwrap();
    assert!(!debug.lock().unwrap().as_ref().unwrap().contains(private));
}
