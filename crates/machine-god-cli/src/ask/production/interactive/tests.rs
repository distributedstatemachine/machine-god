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
    machine_god_native::NativeInteractivePromptPrincipal,
) {
    let mut inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let bridge = inbox.router();
    let context = context();
    let principal = inbox
        .register(BackgroundOutputOwner::new(
            context.session_id,
            context.session_incarnation_id,
        ))
        .unwrap();
    (bridge, inbox, principal)
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
    let (bridge, mut inbox, principal) = bridge();
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
    drop(principal);
    let response = modal.answer("yes", &binding).unwrap().unwrap();
    assert!(inbox.reply(modal.view.token(), response).is_err());
}

#[test]
fn permission_render_escapes_controls_and_fails_closed_at_output_bound() {
    let (bridge, mut inbox, _principal) = bridge();
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
    let (bridge, mut inbox, _principal) = bridge();
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

#[test]
fn mcp_form_answers_require_each_acknowledged_page_and_exact_native_reply() {
    use machine_god_native::mcp::{
        interaction::{McpElicitationPresenter, McpElicitationPromptRequest},
        mrtr::{McpElicitationRequest, McpMrtrLimits},
        protocol::ProtocolVersion,
    };
    let (bridge, mut inbox, _principal) = bridge();
    let raw = serde_json::value::RawValue::from_string(
        r#"{"message":"Confirm value","requestedSchema":{"type":"object","properties":{"count":{"type":"number"}},"required":["count"]}}"#.into(),
    ).unwrap();
    let request = McpElicitationPromptRequest::new(
        context(),
        Arc::from("actual-server"),
        ToolName::new("mcp_actual_tool").unwrap(),
        Arc::new(
            McpElicitationRequest::parse(&raw, ProtocolVersion::Modern, McpMrtrLimits::default())
                .unwrap(),
        ),
    )
    .unwrap();
    let mut pending =
        McpElicitationPresenter::present(bridge.as_ref(), request, CancellationToken::new());
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let mut modal = modal(&mut inbox);
    let source = String::from_utf8(modal.render().unwrap()).unwrap();
    assert!(source.contains("actual-server"));
    assert!(source.contains("mcp_actual_tool"));
    let intro = modal.presentation_binding();
    assert!(modal.answer("/next", &intro).is_err());
    modal.displayed = true;
    assert!(modal.answer("/next", &intro).unwrap().is_none());
    assert!(!modal.displayed);
    modal.displayed = true;
    assert!(modal.answer("9007199254740993.00000001", &intro).is_err());
    let field = modal.binding();
    assert!(
        modal
            .answer("9007199254740993.00000001", &field)
            .unwrap()
            .is_none()
    );
    assert!(modal.answer("y", &field).is_err());
    modal.displayed = true;
    assert!(modal.answer("y", &field).is_err());
    let confirmation = modal.binding();
    let response = modal.answer("y", &confirmation).unwrap().unwrap();
    inbox.reply(modal.view.token(), response).unwrap();
    let Poll::Ready(Ok(answer)) = pending
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!()
    };
    assert!(
        answer
            .canonical_json()
            .get()
            .contains("9007199254740993.00000001")
    );
}
