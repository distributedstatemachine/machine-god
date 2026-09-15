use super::*;
use machine_god_native::{McpFeatureAction, mcp::mrtr::McpElicitationAction};

fn human(action: McpFeatureAction, form: bool) -> McpElicitationPromptRequest {
    let owner = BackgroundOutputOwner::new(
        SessionId::new("url-session").unwrap(),
        SessionIncarnationId::new("url-incarnation").unwrap(),
    );
    let raw = RawValue::from_string(
        if form {
            r#"{"message":"Confirm","requestedSchema":{"type":"object","properties":{}}}"#
        } else {
            r#"{"mode":"url","message":"Authorize","url":"https://example.test/human-secret"}"#
        }
        .into(),
    )
    .unwrap();
    let request = Arc::new(
        McpElicitationRequest::parse(&raw, ProtocolVersion::Modern, McpMrtrLimits::default())
            .unwrap(),
    );
    McpElicitationPromptRequest::new_human_feature(
        owner,
        Arc::from("server\u{1b}\u{202e}"),
        action,
        request,
    )
    .unwrap()
}

#[test]
fn human_form_and_url_label_real_action_and_require_each_flush_acknowledgement() {
    let (bridge, mut inbox, _principal) = bridge();
    for action in [McpFeatureAction::ResourceRead, McpFeatureAction::PromptGet] {
        for form in [false, true] {
            let mut future = bridge.present(human(action, form), CancellationToken::new());
            assert!(poll(&mut future).is_pending());
            let mut modal = modal(&mut inbox);
            let rendered = String::from_utf8(modal.render().unwrap()).unwrap();
            assert!(rendered.contains(&format!("Human feature: {}", action.as_str())));
            assert!(!rendered.contains("Tool:") && !rendered.contains("url-session"));
            assert!(rendered.contains("/cancel cancels the current operation."));
            assert!(!rendered.contains("stops the turn"));
            assert!(!rendered.contains('\u{1b}') && !rendered.contains('\u{202e}'));
            assert!(modal.answer("y", &modal.presentation_binding()).is_err());
            modal.displayed = true;
            if form {
                let binding = modal.binding();
                assert!(modal.answer("/next", &binding).unwrap().is_none());
                assert!(!modal.displayed);
                assert!(modal.answer("y", &binding).is_err());
                modal.displayed = true;
                assert!(modal.answer("y", &binding).is_err());
            }
            let response = modal.answer("y", &modal.binding()).unwrap().unwrap();
            inbox.reply(modal.view.token(), response).unwrap();
            let Poll::Ready(Ok(answer)) = poll(&mut future) else {
                panic!("admitted human answer");
            };
            assert_eq!(answer.action(), McpElicitationAction::Accept);
        }
    }
}

#[test]
fn human_recovery_retains_action_without_url_and_obsolete_ui_has_no_answer_authority() {
    let (bridge, mut inbox, _principal) = bridge();
    for action in [McpFeatureAction::ResourceRead, McpFeatureAction::PromptGet] {
        for (line, expected) in [
            ("m", McpUrlRecoveryAnswer::ContinueManually),
            ("r", McpUrlRecoveryAnswer::RetryBrowser),
            ("c", McpUrlRecoveryAnswer::Cancel),
        ] {
            let request = McpUrlRecoveryPromptRequest::new(human(action, false)).unwrap();
            let mut future = bridge.recover_url(request, CancellationToken::new());
            assert!(poll(&mut future).is_pending());
            let mut modal = modal(&mut inbox);
            let rendered = String::from_utf8(modal.render().unwrap()).unwrap();
            assert!(rendered.contains(&format!("Human feature: {}", action.as_str())));
            assert!(!rendered.contains("Tool:") && !rendered.contains("human-secret"));
            assert!(rendered.contains("/cancel cancels the current operation."));
            assert!(!rendered.contains("stops the turn"));
            assert!(modal.answer(line, &modal.presentation_binding()).is_err());
            modal.displayed = true;
            let response = modal.answer(line, &modal.binding()).unwrap().unwrap();
            inbox.reply(modal.view.token(), response).unwrap();
            assert_eq!(poll(&mut future), Poll::Ready(Ok(expected)));
        }
    }
    let request =
        McpUrlRecoveryPromptRequest::new(human(McpFeatureAction::ResourceRead, false)).unwrap();
    let mut future = bridge.recover_url(request, CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let mut modal = modal(&mut inbox);
    modal.displayed = true;
    drop(future);
    let response = modal.answer("r", &modal.binding()).unwrap().unwrap();
    assert_eq!(
        inbox.reply(modal.view.token(), response),
        Err(NativeInteractivePromptError::Stale)
    );
}
