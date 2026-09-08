use super::{InputBinding, Modal};
use machine_god_core::{
    BackgroundOutputOwner, CancellationToken, Capability, PermissionRequest, PermissionRequestId,
    PermissionRisk, SessionId, SessionIncarnationId, Tool, ToolCall, ToolCallId, ToolContext,
    ToolName, TurnId,
};
use machine_god_native::{
    AskUserQuestionTool, NativeInteractivePromptBridge, NativeInteractivePromptInbox,
    NativeInteractivePromptLimits, NativeInteractivePromptResponse, PermissionPromptDecision,
    PermissionPrompter, QuestionPromptOutcome,
};
use serde_json::json;
use std::{
    sync::Arc,
    task::{Context, Poll, Waker},
};

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("interactive-test").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
        turn_id: TurnId::new("turn").unwrap(),
        call_id: ToolCallId::new("question").unwrap(),
    }
}

fn bridge() -> (
    Arc<NativeInteractivePromptBridge>,
    NativeInteractivePromptInbox,
) {
    let (bridge, mut inbox) =
        NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
    let context = context();
    inbox
        .activate(BackgroundOutputOwner::new(
            context.session_id,
            context.session_incarnation_id,
        ))
        .unwrap();
    (bridge, inbox)
}

fn modal(inbox: &mut NativeInteractivePromptInbox) -> Modal {
    let Poll::Ready(Some(view)) = inbox.poll_prompt(&mut Context::from_waker(Waker::noop())) else {
        panic!("registered prompt");
    };
    Modal::new(view)
}

fn request(reason: &str) -> PermissionRequest {
    let context = context();
    PermissionRequest {
        id: PermissionRequestId::new("permission").unwrap(),
        session_id: context.session_id,
        session_incarnation_id: context.session_incarnation_id,
        turn_id: context.turn_id,
        capability: Capability::Custom {
            name: "test".into(),
            details: json!({"value":"\u{1b}[31m"}),
        },
        risk: PermissionRisk::High,
        reason: reason.into(),
    }
}

#[test]
fn permission_answers_require_the_exact_acknowledged_page() {
    let (bridge, mut inbox) = bridge();
    let mut pending = PermissionPrompter::prompt(bridge.as_ref(), request("confirm"));
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let mut modal = modal(&mut inbox);
    let binding = modal.presentation_binding();
    assert!(modal.answer("yes", &binding).is_err());
    modal.displayed = true;
    assert!(modal.answer("yes", &InputBinding::Command).is_err());
    assert!(matches!(
        modal.answer("s", &binding),
        Ok(Some(NativeInteractivePromptResponse::Permission(
            PermissionPromptDecision::AllowSession
        )))
    ));
    inbox.deactivate();
    let response = modal.answer("yes", &binding).unwrap().unwrap();
    assert!(inbox.reply(modal.view.token(), response).is_err());
}

#[test]
fn permission_render_escapes_controls_and_fails_closed_at_output_bound() {
    let (bridge, mut inbox) = bridge();
    let mut pending = PermissionPrompter::prompt(bridge.as_ref(), request("reason\u{1b}\u{202e}"));
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let output = String::from_utf8(modal(&mut inbox).render().unwrap()).unwrap();
    assert!(!output.contains('\u{1b}'));
    assert!(!output.contains('\u{202e}'));
    assert!(output.contains("\\u001b"));
    drop(pending);
    let mut oversized = PermissionPrompter::prompt(bridge.as_ref(), request(&"x".repeat(65_536)));
    assert!(
        oversized
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert!(modal(&mut inbox).render().is_err());
}

#[test]
fn question_pages_cannot_consume_buffered_previous_page_answers() {
    let (bridge, mut inbox) = bridge();
    let tool = AskUserQuestionTool::shared_prompter(bridge);
    let prepared = tool
        .prepare(ToolCall {
            id: context().call_id,
            name: ToolName::new("ask_user_question").unwrap(),
            arguments: json!({"questions":[
                {"question":"First?", "options":[{"label":"A"},{"label":"B"}]},
                {"question":"Second?", "options":[{"label":"C"},{"label":"D"}]}
            ]}),
        })
        .unwrap();
    let mut pending = tool.execute(
        context(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    );
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let mut modal = modal(&mut inbox);
    modal.displayed = true;
    let first = modal.binding();
    assert!(modal.answer("1", &first).unwrap().is_none());
    assert!(modal.answer("2", &first).is_err());
    modal.displayed = true;
    assert!(modal.answer("2", &first).is_err());
    let second = modal.binding();
    let Some(NativeInteractivePromptResponse::Question(QuestionPromptOutcome::Answered(answers))) =
        modal.answer("other my choice", &second).unwrap()
    else {
        panic!("complete ordered answers");
    };
    assert_eq!(answers.iter().collect::<Vec<_>>(), ["A", "my choice"]);
}
