use super::super::super::{input_lines::InputBinding, presentation::Modal};
use super::*;
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, SessionId, SessionIncarnationId,
    ToolCallId, ToolContext, ToolName, TurnId,
};
use machine_god_native::{
    NativeInteractivePromptBridge, NativeInteractivePromptError, NativeInteractivePromptInbox,
    NativeInteractivePromptLimits,
    mcp::{
        interaction::{McpElicitationPresenter, McpElicitationPromptRequest},
        mrtr::{McpElicitationRequest, McpMrtrLimits},
        protocol::ProtocolVersion,
    },
};
use serde_json::value::RawValue;
use std::{
    sync::Arc,
    task::{Context, Poll, Waker},
};

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("url-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("url-incarnation").unwrap(),
        turn_id: TurnId::new("url-turn").unwrap(),
        call_id: ToolCallId::new("url-call").unwrap(),
    }
}
fn owner() -> BackgroundOutputOwner {
    let context = context();
    BackgroundOutputOwner::new(context.session_id, context.session_incarnation_id)
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
fn recovery() -> McpUrlRecoveryPromptRequest {
    let raw = RawValue::from_string(r#"{"mode":"url","message":"Authorize","url":"https://example.test/secret-not-for-recovery"}"#.into()).unwrap();
    McpUrlRecoveryPromptRequest::new(
        McpElicitationPromptRequest::new(
            context(),
            Arc::from("server\u{1b}\u{202e}"),
            ToolName::new("tool").unwrap(),
            Arc::new(
                McpElicitationRequest::parse(
                    &raw,
                    ProtocolVersion::Modern,
                    McpMrtrLimits::default(),
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    )
    .unwrap()
}
fn poll<T>(future: &mut BoxFuture<'_, T>) -> Poll<T> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}
fn modal(inbox: &mut NativeInteractivePromptInbox) -> Modal {
    let Poll::Ready(Some(view)) = inbox.poll_prompt(&mut Context::from_waker(Waker::noop())) else {
        panic!("queued prompt")
    };
    Modal::new(view)
}

#[test]
fn recovery_requires_acknowledged_exact_token_and_returns_only_typed_choices() {
    let (bridge, mut inbox) = bridge();
    for (line, expected) in [
        ("m", McpUrlRecoveryAnswer::ContinueManually),
        ("r", McpUrlRecoveryAnswer::RetryBrowser),
        ("c", McpUrlRecoveryAnswer::Cancel),
        ("/cancel-input", McpUrlRecoveryAnswer::Cancel),
    ] {
        let mut future = bridge.recover_url(recovery(), CancellationToken::new());
        assert!(poll(&mut future).is_pending());
        let mut modal = modal(&mut inbox);
        assert!(matches!(modal.binding(), InputBinding::AwaitingPrompt));
        assert!(modal.answer(line, &modal.presentation_binding()).is_err());
        let rendered = String::from_utf8(modal.render().unwrap()).unwrap();
        assert!(rendered.contains("Continue manually") && rendered.contains("Retry browser"));
        assert!(!rendered.contains("secret-not-for-recovery"));
        assert!(!rendered.contains('\u{1b}') && !rendered.contains('\u{202e}'));
        modal.displayed = true; // Existing driver sets this only after exact flush ack.
        assert!(modal.answer(line, &InputBinding::AwaitingPrompt).is_err());
        let binding = modal.binding();
        for invalid in [
            "yes",
            "y",
            "accept",
            "",
            "/cancel",
            "{\"action\":\"accept\"}",
        ] {
            assert!(modal.answer(invalid, &binding).is_err());
        }
        let response = modal.answer(line, &binding).unwrap().unwrap();
        assert!(
            matches!(&response, NativeInteractivePromptResponse::UrlRecovery(actual) if *actual == expected)
        );
        assert!(modal.answer(line, &binding).is_err());
        inbox.reply(modal.view.token(), response).unwrap();
        assert_eq!(poll(&mut future), Poll::Ready(Ok(expected)));
    }
}

#[test]
fn stale_recovery_binding_and_dropped_producer_cannot_reply() {
    let (bridge, mut inbox) = bridge();
    let mut first = bridge.recover_url(recovery(), CancellationToken::new());
    assert!(poll(&mut first).is_pending());
    let old = modal(&mut inbox).presentation_binding();
    drop(first);
    for (line, expected) in [
        ("m", McpUrlRecoveryAnswer::ContinueManually),
        ("r", McpUrlRecoveryAnswer::RetryBrowser),
        ("c", McpUrlRecoveryAnswer::Cancel),
        ("/cancel-input", McpUrlRecoveryAnswer::Cancel),
    ] {
        let mut future = bridge.recover_url(recovery(), CancellationToken::new());
        assert!(poll(&mut future).is_pending());
        let mut modal = modal(&mut inbox);
        assert!(
            String::from_utf8(modal.render().unwrap())
                .unwrap()
                .contains("Retry browser")
        );
        assert!(modal.answer(line, &modal.presentation_binding()).is_err());
        modal.displayed = true;
        assert!(modal.answer(line, &old).is_err());
        let binding = modal.binding();
        assert!(modal.answer("I completed it", &binding).is_err());
        let response = modal.answer(line, &binding).unwrap().unwrap();
        assert!(
            matches!(&response, NativeInteractivePromptResponse::UrlRecovery(actual) if *actual == expected)
        );
        inbox.reply(modal.view.token(), response).unwrap();
        assert_eq!(poll(&mut future), Poll::Ready(Ok(expected)));
    }
    let mut future = bridge.recover_url(recovery(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let mut modal = modal(&mut inbox);
    modal.displayed = true;
    drop(future); // A dropped recovery producer leaves only stale UI data.
    let response = modal.answer("r", &modal.binding()).unwrap().unwrap();
    assert_eq!(
        inbox.reply(modal.view.token(), response),
        Err(NativeInteractivePromptError::Stale)
    );
}
