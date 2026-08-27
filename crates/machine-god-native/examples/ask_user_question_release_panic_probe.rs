use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    BoxFuture, CancellationToken, SessionId, SessionIncarnationId, Tool, ToolCall, ToolCallId,
    ToolContext, ToolError, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    ASK_USER_QUESTION_TOOL_NAME, AskUserQuestionTool, QuestionPromptError, QuestionPromptOutcome,
    QuestionPromptRequest, QuestionPrompter,
};
use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
use serde_json::{Value, json};

type PromptResult = Result<QuestionPromptOutcome, QuestionPromptError>;

#[derive(Debug)]
struct PrimaryPromptDropPanic;

#[derive(Debug)]
struct AmbientExecutionDropPanic;

#[derive(Debug)]
struct SecondaryPayloadDropPanic;

struct SecondaryTargetWithPromptWakerPanic {
    _supplied_waker: Waker,
    payload_drops: Arc<AtomicUsize>,
}

impl Drop for SecondaryTargetWithPromptWakerPanic {
    fn drop(&mut self) {
        self.payload_drops.fetch_add(1, Ordering::SeqCst);
        panic_any(SecondaryPayloadDropPanic);
    }
}

#[derive(Default)]
struct ProbeState {
    prompt_calls: AtomicUsize,
    fresh_prompts_live: AtomicUsize,
    supplied_waker: Mutex<Option<Waker>>,
    target_wakes: AtomicUsize,
    target_drops: AtomicUsize,
    secondary_callbacks: AtomicUsize,
    secondary_payload_drops: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct PromptDropThenPendingPrompter {
    state: Arc<ProbeState>,
}

impl QuestionPrompter for PromptDropThenPendingPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        if self.state.prompt_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Box::pin(PromptDropPanicFuture {
                state: Arc::clone(&self.state),
            })
        } else {
            self.state.fresh_prompts_live.fetch_add(1, Ordering::SeqCst);
            Box::pin(FreshPendingPrompt {
                state: Arc::clone(&self.state),
            })
        }
    }
}

struct PromptDropPanicFuture {
    state: Arc<ProbeState>,
}

impl Future for PromptDropPanicFuture {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let previous = self
            .state
            .supplied_waker
            .lock()
            .unwrap()
            .replace(context.waker().clone());
        drop(previous);
        Poll::Pending
    }
}

impl Drop for PromptDropPanicFuture {
    fn drop(&mut self) {
        panic_any(PrimaryPromptDropPanic);
    }
}

struct FreshPendingPrompt {
    state: Arc<ProbeState>,
}

impl Future for FreshPendingPrompt {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for FreshPendingPrompt {
    fn drop(&mut self) {
        self.state.fresh_prompts_live.fetch_sub(1, Ordering::SeqCst);
    }
}

struct PanickingCleanupTarget {
    state: Arc<ProbeState>,
    teardown: Option<Waker>,
}

impl Wake for PanickingCleanupTarget {
    fn wake(self: Arc<Self>) {
        self.state.target_wakes.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.state.target_wakes.fetch_add(1, Ordering::SeqCst);
    }
}

impl Drop for PanickingCleanupTarget {
    fn drop(&mut self) {
        self.state.target_drops.fetch_add(1, Ordering::SeqCst);
        let teardown = self.teardown.take();
        drop(teardown);
    }
}

struct ExecutionDropGuard<'a>(Option<BoxFuture<'a, Result<ToolOutput, ToolError>>>);

impl Drop for ExecutionDropGuard<'_> {
    fn drop(&mut self) {
        let execution = self.0.take();
        drop(execution);
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

#[derive(Clone, Copy)]
enum PrimaryCase {
    PromptDrop,
    AmbientDrop,
}

struct ScenarioEvidence {
    target_wakes: usize,
    target_drops: usize,
    secondary_callbacks: usize,
    fresh_prompts: usize,
}

fn cleanup_target_waker(state: &Arc<ProbeState>) -> Waker {
    let callback_state = Arc::clone(state);
    let (teardown, _handle) = reentrant_waker(Callback::Drop, move || {
        callback_state
            .secondary_callbacks
            .fetch_add(1, Ordering::SeqCst);
        let retained = callback_state
            .supplied_waker
            .lock()
            .unwrap()
            .as_ref()
            .expect("prompt retained its supplied Waker")
            .clone();
        panic_any(SecondaryTargetWithPromptWakerPanic {
            _supplied_waker: retained,
            payload_drops: Arc::clone(&callback_state.secondary_payload_drops),
        });
    });
    Waker::from(Arc::new(PanickingCleanupTarget {
        state: Arc::clone(state),
        teardown: Some(teardown),
    }))
}

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

fn assert_panic_marker<T: 'static, R>(
    result: Result<R, Box<dyn std::any::Any + Send>>,
    message: &str,
) {
    let Err(payload) = result else {
        panic!("{message}: operation returned");
    };
    if !payload.is::<T>() {
        std::mem::forget(payload);
        panic!("{message}: wrong panic payload");
    }
    drop(payload);
}

fn prepared_arguments(tool: &AskUserQuestionTool) -> Value {
    tool.prepare(ToolCall {
        id: ToolCallId::new("release-panic-call").unwrap(),
        name: ToolName::new(ASK_USER_QUESTION_TOOL_NAME).unwrap(),
        arguments: json!({
            "questions": [{
                "question": "Continue?",
                "options": [{"label": "Yes"}, {"label": "No"}]
            }]
        }),
    })
    .unwrap()
    .arguments()
    .clone()
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("release-panic-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("release-panic-incarnation").unwrap(),
        turn_id: TurnId::new("release-panic-turn").unwrap(),
        call_id: ToolCallId::new("release-panic-call").unwrap(),
    }
}

fn run_scenario(primary: PrimaryCase) -> ScenarioEvidence {
    let state = Arc::new(ProbeState::default());
    let tool = AskUserQuestionTool::new(PromptDropThenPendingPrompter {
        state: Arc::clone(&state),
    });
    let arguments = prepared_arguments(&tool);
    let mut execution = tool.execute(context(), arguments.clone(), CancellationToken::new());
    let target = cleanup_target_waker(&state);
    assert!(
        execution
            .as_mut()
            .poll(&mut Context::from_waker(&target))
            .is_pending()
    );
    // The notifier must own the final downstream target so its destruction
    // occurs inside close and becomes a captured cleanup panic.
    drop(target);

    match primary {
        PrimaryCase::PromptDrop => assert_panic_marker::<PrimaryPromptDropPanic, _>(
            catch_unwind(AssertUnwindSafe(|| drop(execution))),
            "prompt-drop cleanup precedence failed",
        ),
        PrimaryCase::AmbientDrop => assert_panic_marker::<AmbientExecutionDropPanic, _>(
            catch_unwind(AssertUnwindSafe(|| {
                let _drop_during_unwind = ExecutionDropGuard(Some(execution));
                panic_any(AmbientExecutionDropPanic);
            })),
            "ambient cleanup precedence failed",
        ),
    }

    assert_eq!(state.target_drops.load(Ordering::SeqCst), 1);
    assert_eq!(state.secondary_callbacks.load(Ordering::SeqCst), 1);
    assert_eq!(state.secondary_payload_drops.load(Ordering::SeqCst), 0);

    let closed_waker = state
        .supplied_waker
        .lock()
        .unwrap()
        .take()
        .expect("prompt retained its supplied Waker");
    closed_waker.wake_by_ref();
    assert_eq!(state.target_wakes.load(Ordering::SeqCst), 0);

    // The leaked secondary panic payload still owns a closed supplied-Waker
    // clone, and `closed_waker` keeps another clone live across fresh admission.
    let mut recovered = tool.execute(context(), arguments, CancellationToken::new());
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(state.prompt_calls.load(Ordering::SeqCst), 2);
    assert_eq!(state.fresh_prompts_live.load(Ordering::SeqCst), 1);
    closed_waker.wake_by_ref();
    assert_eq!(state.target_wakes.load(Ordering::SeqCst), 0);
    drop(recovered);
    assert_eq!(state.fresh_prompts_live.load(Ordering::SeqCst), 0);
    drop(closed_waker);

    ScenarioEvidence {
        target_wakes: state.target_wakes.load(Ordering::SeqCst),
        target_drops: state.target_drops.load(Ordering::SeqCst),
        secondary_callbacks: state.secondary_callbacks.load(Ordering::SeqCst),
        fresh_prompts: state.prompt_calls.load(Ordering::SeqCst) - 1,
    }
}

fn assert_secondary_payload_destructor_panics() {
    let drops = Arc::new(AtomicUsize::new(0));
    let result = catch_unwind(AssertUnwindSafe(|| {
        drop(SecondaryTargetWithPromptWakerPanic {
            _supplied_waker: Waker::from(Arc::new(NoopWake)),
            payload_drops: Arc::clone(&drops),
        });
    }));
    assert_panic_marker::<SecondaryPayloadDropPanic, _>(
        result,
        "secondary payload destructor control failed",
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[cfg(panic = "unwind")]
fn assert_unwind_profile() {}

#[cfg(not(panic = "unwind"))]
fn assert_unwind_profile() {
    panic!("release panic recovery requires panic=unwind");
}

fn main() {
    assert_unwind_profile();
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let ordinary = run_scenario(PrimaryCase::PromptDrop);
    let ambient = run_scenario(PrimaryCase::AmbientDrop);
    assert_secondary_payload_destructor_panics();

    assert_eq!(ordinary.target_wakes + ambient.target_wakes, 0);
    assert_eq!(ordinary.target_drops + ambient.target_drops, 2);
    assert_eq!(
        ordinary.secondary_callbacks + ambient.secondary_callbacks,
        2
    );
    assert_eq!(ordinary.fresh_prompts + ambient.fresh_prompts, 2);

    std::panic::set_hook(original_hook);
    print!(
        "ordinary-primary=prompt-drop\n\
         ambient-primary=ambient-drop\n\
         secondary-payload-drop=panics\n\
         secondary-payloads=suppressed\n\
         stale-target-wakes=0\n\
         target-drops=2 secondary-callbacks=2\n\
         fresh-capacity=2\n"
    );
}
