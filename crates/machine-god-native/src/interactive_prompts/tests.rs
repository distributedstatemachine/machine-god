use super::*;
use crate::{ASK_USER_QUESTION_TOOL_NAME, AskUserQuestionTool, QuestionPromptAnswers};
use futures_executor::block_on;
use machine_god_core::{
    CancellationToken, Capability, PermissionRequestId, PermissionRisk, SessionId,
    SessionIncarnationId, Tool, ToolCall, ToolCallId, ToolName, TurnId,
};
use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Wake, Waker};

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
        turn_id: TurnId::new("turn").unwrap(),
        call_id: ToolCallId::new("reused-call").unwrap(),
    }
}
fn owner() -> BackgroundOutputOwner {
    let context = context();
    BackgroundOutputOwner::new(context.session_id, context.session_incarnation_id)
}
fn request(id: &str) -> PermissionRequest {
    let context = context();
    PermissionRequest {
        id: PermissionRequestId::new(id).unwrap(),
        session_id: context.session_id,
        session_incarnation_id: context.session_incarnation_id,
        turn_id: context.turn_id,
        capability: Capability::Custom {
            name: "private-operation".into(),
            details: json!({"private":"not debug"}),
        },
        risk: PermissionRisk::High,
        reason: "private rationale".into(),
    }
}
fn bridge() -> (
    Arc<NativeInteractivePromptBridge>,
    NativeInteractivePromptInbox,
) {
    let (bridge, mut inbox) =
        NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
    inbox.activate(owner()).unwrap();
    (bridge, inbox)
}
fn permission<'a>(
    bridge: &'a NativeInteractivePromptBridge,
    id: &str,
) -> BoxFuture<'a, Result<PermissionPromptDecision, PermissionPromptError>> {
    PermissionPrompter::prompt(bridge, request(id))
}
fn poll<T>(future: &mut BoxFuture<'_, T>) -> Poll<T> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}
fn view(inbox: &mut NativeInteractivePromptInbox) -> NativeInteractivePromptView {
    let Poll::Ready(Some(view)) = inbox.poll_prompt(&mut Context::from_waker(Waker::noop())) else {
        panic!("displayed prompt");
    };
    view
}
fn respond(
    inbox: &mut NativeInteractivePromptInbox,
    view: &NativeInteractivePromptView,
    decision: PermissionPromptDecision,
) {
    inbox
        .reply(
            view.token(),
            NativeInteractivePromptResponse::Permission(decision),
        )
        .unwrap();
}
fn arguments() -> Value {
    json!({"questions":[{"question":"Choose one?","options":[{"label":"First","description":"First path"},{"label":"Second"}]},{"question":"Choose two?","options":[{"label":"A"},{"label":"B"}]}]})
}
fn prepared(tool: &AskUserQuestionTool) -> Value {
    tool.prepare(ToolCall {
        id: context().call_id,
        name: ToolName::new(ASK_USER_QUESTION_TOOL_NAME).unwrap(),
        arguments: arguments(),
    })
    .unwrap()
    .arguments()
    .clone()
}

#[derive(Default)]
struct LegacyCapture(Mutex<Option<QuestionPromptRequest>>);
impl QuestionPrompter for LegacyCapture {
    fn prompt(
        &self,
        request: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        Box::pin(async move {
            *self.0.lock().unwrap() = Some(request);
            Ok(QuestionPromptOutcome::Cancelled)
        })
    }
}
fn question_request() -> QuestionPromptRequest {
    let capture = Arc::new(LegacyCapture::default());
    let tool = AskUserQuestionTool::shared_prompter(capture.clone());
    let output =
        block_on(tool.execute(context(), prepared(&tool), CancellationToken::new())).unwrap();
    assert_eq!(output.content, crate::ASK_USER_QUESTION_CANCELLED_SENTINEL);
    capture.0.lock().unwrap().take().unwrap()
}

#[test]
fn construction_is_inert_and_all_four_permission_choices_roundtrip() {
    let (bridge, mut inbox) = bridge();
    drop(permission(&bridge, "never-polled"));
    assert!(
        inbox
            .poll_prompt(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    for decision in [
        PermissionPromptDecision::AllowOnce,
        PermissionPromptDecision::AllowTurn,
        PermissionPromptDecision::AllowSession,
        PermissionPromptDecision::Deny,
    ] {
        let mut future = permission(&bridge, "same-request-id");
        assert!(poll(&mut future).is_pending());
        let prompt = view(&mut inbox);
        assert_eq!(prompt.permission().unwrap(), &request("same-request-id"));
        assert_eq!(view(&mut inbox).token(), prompt.token());
        respond(&mut inbox, &prompt, decision);
        assert_eq!(block_on(future), Ok(decision));
        assert_eq!(
            inbox.reply(
                prompt.token(),
                NativeInteractivePromptResponse::Permission(decision)
            ),
            Err(NativeInteractivePromptError::Stale)
        );
    }
}

#[test]
fn fifo_backpressure_drop_and_ready_responses_keep_admission_bounded() {
    let limits = NativeInteractivePromptLimits::new(2, 4096).unwrap();
    let (bridge, mut inbox) = NativeInteractivePromptBridge::new(limits).unwrap();
    inbox.activate(owner()).unwrap();
    let mut first = permission(&bridge, "first");
    let mut second = permission(&bridge, "second");
    assert!(poll(&mut first).is_pending());
    assert!(poll(&mut second).is_pending());
    let old = view(&mut inbox);
    assert_eq!(old.permission().unwrap().id, request("first").id);
    respond(&mut inbox, &old, PermissionPromptDecision::AllowOnce);
    assert!(block_on(permission(&bridge, "full-after-reply")).is_err());
    let next = view(&mut inbox);
    assert_eq!(next.permission().unwrap().id, request("second").id);
    drop(first);
    assert_eq!(
        inbox.cancel(old.token()),
        Err(NativeInteractivePromptError::Stale)
    );
    let mut third = permission(&bridge, "third");
    assert!(poll(&mut third).is_pending());
    drop(second);
    assert_eq!(
        view(&mut inbox).permission().unwrap().id,
        request("third").id
    );
    let current = view(&mut inbox);
    inbox.cancel(current.token()).unwrap();
    assert_eq!(block_on(third), Ok(PermissionPromptDecision::Deny));
}

#[test]
fn reactivation_rejects_old_unpolled_and_ready_answers_even_for_same_principal() {
    let (bridge, mut inbox) = bridge();
    let never = permission(&bridge, "never");
    let mut ready = permission(&bridge, "ready");
    assert!(poll(&mut ready).is_pending());
    let old = view(&mut inbox);
    respond(&mut inbox, &old, PermissionPromptDecision::AllowSession);
    inbox.activate(owner()).unwrap();
    assert!(block_on(never).is_err());
    assert!(block_on(ready).is_err());
    let mut replacement = permission(&bridge, "ready");
    assert!(poll(&mut replacement).is_pending());
    let new = view(&mut inbox);
    assert_ne!(new.token(), old.token());
    assert_eq!(
        inbox.cancel(old.token()),
        Err(NativeInteractivePromptError::Stale)
    );
    respond(&mut inbox, &new, PermissionPromptDecision::AllowOnce);
    assert_eq!(
        block_on(replacement),
        Ok(PermissionPromptDecision::AllowOnce)
    );
}

#[test]
fn no_ambient_scope_cross_bridge_token_or_closed_inbox_can_authorize() {
    let (bridge, mut inbox) =
        NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
    let no_scope = permission(&bridge, "unbound");
    inbox.activate(owner()).unwrap();
    assert!(block_on(no_scope).is_err());
    let mut other = request("wrong-owner");
    other.session_incarnation_id = SessionIncarnationId::new("different").unwrap();
    assert!(block_on(PermissionPrompter::prompt(bridge.as_ref(), other)).is_err());
    let mut future = permission(&bridge, "active");
    assert!(poll(&mut future).is_pending());
    let active = view(&mut inbox);
    let (second, mut second_inbox) = self::bridge();
    let mut second_future = permission(&second, "active");
    assert!(poll(&mut second_future).is_pending());
    let second_view = view(&mut second_inbox);
    assert_eq!(
        inbox.cancel(second_view.token()),
        Err(NativeInteractivePromptError::Stale)
    );
    inbox.deactivate();
    assert!(block_on(future).is_err());
    assert_eq!(
        inbox.cancel(active.token()),
        Err(NativeInteractivePromptError::Stale)
    );
    inbox.close();
    assert_eq!(
        inbox.activate(owner()),
        Err(NativeInteractivePromptError::Closed)
    );
    assert!(matches!(
        inbox.poll_prompt(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(None)
    ));
    assert!(block_on(permission(&bridge, "closed")).is_err());
    drop(second_inbox);
    assert!(block_on(second_future).is_err());
}

#[test]
fn actual_question_tool_forwards_exact_context_options_and_freeform_answers() {
    let (bridge, mut inbox) = bridge();
    let tool = AskUserQuestionTool::shared_prompter(bridge);
    let mut future = tool.execute(context(), prepared(&tool), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let prompt = view(&mut inbox);
    let (actual, questions) = prompt.question().unwrap();
    assert_eq!(actual, &context());
    assert_eq!(questions.questions().len(), 2);
    assert_eq!(
        questions.questions()[0].options()[0].description(),
        Some("First path")
    );
    let mut answers = QuestionPromptAnswers::new();
    answers.try_push("First".into()).unwrap();
    answers
        .try_push("a freeform choice absent from options".into())
        .unwrap();
    inbox
        .reply(
            prompt.token(),
            NativeInteractivePromptResponse::Question(QuestionPromptOutcome::Answered(answers)),
        )
        .unwrap();
    let output = block_on(future).unwrap();
    assert!(!output.is_error);
    assert_eq!(output.content[0]["answer"], "First");
    assert_eq!(
        output.content[1]["answer"],
        "a freeform choice absent from options"
    );
}

#[test]
fn question_contextless_rejects_legacy_default_survives_and_cancel_is_real() {
    let request = question_request(); // Real execute used the new default method on a legacy host.
    let (bridge, mut inbox) = bridge();
    assert!(block_on(QuestionPrompter::prompt(bridge.as_ref(), request.clone())).is_err());
    let mut future = bridge.prompt_with_context(context(), request);
    assert!(poll(&mut future).is_pending());
    let prompt = view(&mut inbox);
    inbox.cancel(prompt.token()).unwrap();
    assert_eq!(block_on(future), Ok(QuestionPromptOutcome::Cancelled));
}

#[test]
fn actual_question_cancellation_drops_registration_and_reused_call_cannot_take_old_reply() {
    let (bridge, mut inbox) = bridge();
    let tool = AskUserQuestionTool::shared_prompter(bridge);
    let cancel = CancellationToken::new();
    let mut future = tool.execute(context(), prepared(&tool), cancel.clone());
    assert!(poll(&mut future).is_pending());
    let old = view(&mut inbox);
    cancel.cancel();
    assert!(block_on(future).is_err());
    let mut next = tool.execute(context(), prepared(&tool), CancellationToken::new());
    assert!(poll(&mut next).is_pending());
    let current = view(&mut inbox);
    assert_ne!(old.token(), current.token());
    assert_eq!(
        inbox.cancel(old.token()),
        Err(NativeInteractivePromptError::Stale)
    );
    inbox.cancel(current.token()).unwrap();
    assert_eq!(
        block_on(next).unwrap().content,
        crate::ASK_USER_QUESTION_CANCELLED_SENTINEL
    );
}

#[test]
fn typed_answer_bounds_reject_without_consuming_displayed_request() {
    let (bridge, mut inbox) = bridge();
    let mut future = bridge.prompt_with_context(context(), question_request());
    assert!(poll(&mut future).is_pending());
    let prompt = view(&mut inbox);
    assert_eq!(
        inbox.reply(
            prompt.token(),
            NativeInteractivePromptResponse::Permission(PermissionPromptDecision::AllowOnce)
        ),
        Err(NativeInteractivePromptError::InvalidResponse)
    );
    for values in [
        vec!["only-one".into()],
        vec![" ".into(), "valid".into()],
        vec!["x".repeat(4096), "y".into()],
        vec!["x".repeat(4097), "y".into()],
    ] {
        let mut answers = QuestionPromptAnswers::new();
        for value in values {
            answers.try_push(value).unwrap();
        }
        assert!(
            inbox
                .reply(
                    prompt.token(),
                    NativeInteractivePromptResponse::Question(QuestionPromptOutcome::Answered(
                        answers
                    ))
                )
                .is_err()
        );
        assert_eq!(view(&mut inbox).token(), prompt.token());
    }
    let mut answers = QuestionPromptAnswers::new();
    answers.try_push("x".repeat(4095)).unwrap();
    answers.try_push("y".into()).unwrap();
    inbox
        .reply(
            prompt.token(),
            NativeInteractivePromptResponse::Question(QuestionPromptOutcome::Answered(answers)),
        )
        .unwrap();
    assert!(block_on(future).is_ok());
}

#[test]
fn exact_payload_byte_limit_aggregate_limit_and_invalid_limits_are_enforced() {
    let payload = Payload::Permission(request("bound"));
    let bytes = payload.bytes(usize::MAX).unwrap();
    for (limit, accepted) in [(bytes - 1, false), (bytes, true)] {
        let (bridge, mut inbox) = NativeInteractivePromptBridge::new(
            NativeInteractivePromptLimits::new(2, limit).unwrap(),
        )
        .unwrap();
        inbox.activate(owner()).unwrap();
        let mut future = permission(&bridge, "bound");
        assert_eq!(poll(&mut future).is_pending(), accepted);
        if accepted {
            assert!(block_on(permission(&bridge, "bound")).is_err());
            let prompt = view(&mut inbox);
            inbox.cancel(prompt.token()).unwrap();
            assert!(block_on(future).is_ok());
        }
    }
    for (count, bytes) in [(0, 1), (9, 1), (1, 0), (1, usize::MAX)] {
        assert!(NativeInteractivePromptLimits::new(count, bytes).is_err());
    }
}

#[test]
fn deep_and_excess_node_payloads_reject_and_unpolled_drop_is_iterative() {
    for polled in [false, true] {
        let (bridge, _inbox) = bridge();
        let mut value = Value::Null;
        for _ in 0..20_000 {
            value = Value::Array(vec![value]);
        }
        let mut request = request("deep");
        request.capability = Capability::Custom {
            name: "deep".into(),
            details: value,
        };
        let future = PermissionPrompter::prompt(bridge.as_ref(), request);
        if polled {
            assert!(block_on(future).is_err());
        } else {
            drop(future);
        }
    }
    let (bridge, _inbox) = bridge();
    let mut request = request("wide");
    request.capability = Capability::Custom {
        name: "wide".into(),
        details: Value::Array(vec![Value::Null; 65_536]),
    };
    assert!(block_on(PermissionPrompter::prompt(bridge.as_ref(), request)).is_err());
}

#[derive(Default)]
struct Counter(AtomicUsize);

#[test]
fn permission_json_depth_and_node_bounds_are_inclusive() {
    for (depth, accepted) in [(64, true), (65, false)] {
        let mut value = Value::Null;
        for _ in 0..depth {
            value = Value::Array(vec![value]);
        }
        let mut request = request("depth-boundary");
        request.capability = Capability::Custom {
            name: "depth".into(),
            details: value,
        };
        assert_eq!(
            Payload::Permission(request).bytes(8 * 1024 * 1024).is_ok(),
            accepted
        );
    }
    for (children, accepted) in [(65_535, true), (65_536, false)] {
        let mut request = request("node-boundary");
        request.capability = Capability::Custom {
            name: "nodes".into(),
            details: Value::Array(vec![Value::Null; children]),
        };
        assert_eq!(
            Payload::Permission(request).bytes(8 * 1024 * 1024).is_ok(),
            accepted
        );
    }
}

impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn empty_observations_do_not_self_wake_but_admission_reply_and_close_do() {
    let (bridge, mut inbox) = bridge();
    let counter = Arc::new(Counter::default());
    let wake = Waker::from(counter.clone());
    for _ in 0..3 {
        assert!(
            inbox
                .poll_prompt(&mut Context::from_waker(&wake))
                .is_pending()
        );
    }
    assert_eq!(counter.0.load(Ordering::SeqCst), 0);
    let mut future = permission(&bridge, "wake");
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&wake))
            .is_pending()
    );
    assert_eq!(counter.0.load(Ordering::SeqCst), 1);
    let prompt = view(&mut inbox);
    respond(&mut inbox, &prompt, PermissionPromptDecision::AllowOnce);
    assert_eq!(counter.0.load(Ordering::SeqCst), 2);
    assert!(block_on(future).is_ok());
    assert!(
        inbox
            .poll_prompt(&mut Context::from_waker(&wake))
            .is_pending()
    );
    inbox.close();
    assert_eq!(counter.0.load(Ordering::SeqCst), 3);
}

#[test]
fn waker_clone_drop_and_wake_reenter_outside_bridge_locks() {
    for callback in [Callback::Clone, Callback::Drop, Callback::Wake] {
        let (bridge, mut inbox) = bridge();
        let shared = bridge.shared.clone();
        let (wake, handle) = reentrant_waker(callback, move || {
            assert!(shared.state.try_lock().is_ok());
        });
        assert!(
            inbox
                .poll_prompt(&mut Context::from_waker(&wake))
                .is_pending()
        );
        let mut future = permission(&bridge, "reentrant");
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&wake))
                .is_pending()
        );
        let prompt = view(&mut inbox);
        respond(&mut inbox, &prompt, PermissionPromptDecision::AllowOnce);
        assert!(block_on(future).is_ok());
        drop(wake);
        assert!(handle.calls() > 0);
    }
}

#[test]
fn retirement_from_ready_poll_waker_drop_suppresses_old_positive_response() {
    let (bridge, mut inbox) = bridge();
    let mut future = permission(&bridge, "ready-drop");
    assert!(poll(&mut future).is_pending());
    let prompt = view(&mut inbox);
    respond(&mut inbox, &prompt, PermissionPromptDecision::AllowSession);
    let shared = bridge.shared.clone();
    let (wake, _) = reentrant_waker(Callback::Drop, move || {
        shared.deactivate(false);
    });
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(&wake)),
        Poll::Ready(Err(_))
    ));
    assert_eq!(
        inbox.cancel(prompt.token()),
        Err(NativeInteractivePromptError::Stale)
    );
}

#[test]
fn debug_and_errors_never_format_human_or_permission_payloads() {
    let (bridge, mut inbox) = bridge();
    let mut future = permission(&bridge, "private-id");
    assert!(poll(&mut future).is_pending());
    let prompt = view(&mut inbox);
    for value in [
        format!("{bridge:?}"),
        format!("{inbox:?}"),
        format!("{prompt:?}"),
        format!("{:?}", prompt.token()),
        format!(
            "{:?}",
            NativeInteractivePromptResponse::Question(QuestionPromptOutcome::Answered({
                let mut a = QuestionPromptAnswers::new();
                a.try_push("private-answer".into()).unwrap();
                a
            }))
        ),
        NativeInteractivePromptError::Stale.to_string(),
    ] {
        assert!(!value.contains("private"));
    }
}
