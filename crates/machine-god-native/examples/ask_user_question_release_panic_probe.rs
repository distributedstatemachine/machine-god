use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use machine_god_core::{
    BoxFuture, CancellationToken, SessionId, SessionIncarnationId, Tool, ToolCall, ToolCallId,
    ToolContext, ToolName, TurnId,
};
use machine_god_native::{
    ASK_USER_QUESTION_TOOL_NAME, AskUserQuestionTool, QuestionPromptError, QuestionPromptOutcome,
    QuestionPromptRequest, QuestionPrompter,
};
use serde_json::json;

type PromptResult = Result<QuestionPromptOutcome, QuestionPromptError>;

struct PrimaryPromptPanic;

struct PanicThenPendingPrompter {
    calls: Arc<AtomicUsize>,
}

impl QuestionPrompter for PanicThenPendingPrompter {
    fn prompt(&self, _request: QuestionPromptRequest) -> BoxFuture<'_, PromptResult> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Box::pin(PanickingPrompt)
        } else {
            Box::pin(PendingPrompt)
        }
    }
}

struct PanickingPrompt;

impl Future for PanickingPrompt {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        panic_any(PrimaryPromptPanic);
    }
}

struct PendingPrompt;

impl Future for PendingPrompt {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

#[cfg(panic = "unwind")]
fn assert_unwind_profile() {}

#[cfg(not(panic = "unwind"))]
fn assert_unwind_profile() {
    panic!("release panic recovery requires panic=unwind");
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("release-panic-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("release-panic-incarnation").unwrap(),
        turn_id: TurnId::new("release-panic-turn").unwrap(),
        call_id: ToolCallId::new("release-panic-call").unwrap(),
    }
}

fn main() {
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = AskUserQuestionTool::new(PanicThenPendingPrompter {
        calls: Arc::clone(&calls),
    });
    let prepared = tool
        .prepare(ToolCall {
            id: ToolCallId::new("release-panic-call").unwrap(),
            name: ToolName::new(ASK_USER_QUESTION_TOOL_NAME).unwrap(),
            arguments: json!({
                "questions": [{
                    "question": "Continue?",
                    "options": [{"label": "Yes"}, {"label": "No"}]
                }]
            }),
        })
        .unwrap();
    let arguments = prepared.arguments().clone();
    let mut first = tool.execute(context(), arguments.clone(), CancellationToken::new());

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let caught = catch_unwind(AssertUnwindSafe(|| poll_once(first.as_mut())));
    std::panic::set_hook(original_hook);
    let Err(payload) = caught else {
        panic!("panicking prompt unexpectedly returned");
    };
    assert!(payload.is::<PrimaryPromptPanic>());
    drop(payload);
    drop(first);
    assert_unwind_profile();
    println!("primary-caught");

    let mut recovered = tool.execute(context(), arguments, CancellationToken::new());
    assert!(poll_once(recovered.as_mut()).is_pending());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    drop(recovered);
    println!("capacity-recovered");
}
