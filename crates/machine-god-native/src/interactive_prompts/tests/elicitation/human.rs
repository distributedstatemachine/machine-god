use super::*;
use crate::McpFeatureAction;

fn human_owner() -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new("session").unwrap(),
        SessionIncarnationId::new("incarnation").unwrap(),
    )
}
fn human(
    action: McpFeatureAction,
    request: Arc<McpElicitationRequest>,
) -> McpElicitationPromptRequest {
    McpElicitationPromptRequest::new_human_feature(
        human_owner(),
        Arc::from("selected-server"),
        action,
        request,
    )
    .unwrap()
}

#[test]
fn human_source_preserves_only_actual_owner_action_server_and_shared_request() {
    for action in [McpFeatureAction::ResourceRead, McpFeatureAction::PromptGet] {
        let request = form();
        let prompt = human(action, request.clone());
        assert!(
            matches!(prompt.source(), McpElicitationPromptSource::HumanFeature { owner, action: actual } if owner == &human_owner() && *actual == action)
        );
        assert!(Arc::ptr_eq(prompt.request(), &request));
        assert_eq!(prompt.server(), "selected-server");
        assert_eq!(
            prompt.retained_byte_charge(),
            request.retained_byte_charge()
                + 256
                + "session".len()
                + "incarnation".len()
                + "selected-server".len()
        );
        for debug in [format!("{prompt:?}"), format!("{:?}", prompt.source())] {
            assert!(!debug.contains("session") && !debug.contains("selected-server"));
        }
    }
    for action in [
        McpFeatureAction::ResourceList,
        McpFeatureAction::ResourceTemplates,
        McpFeatureAction::ResourceComplete,
        McpFeatureAction::PromptList,
        McpFeatureAction::PromptComplete,
    ] {
        assert!(matches!(
            McpElicitationPromptRequest::new_human_feature(
                human_owner(),
                Arc::from("server"),
                action,
                url()
            ),
            Err(McpElicitationPromptError::InvalidSource)
        ));
    }
    for server in [Arc::from(""), Arc::from("x".repeat(257))] {
        assert!(matches!(
            McpElicitationPromptRequest::new_human_feature(
                human_owner(),
                server,
                McpFeatureAction::ResourceRead,
                url()
            ),
            Err(McpElicitationPromptError::InvalidSource)
        ));
    }
    let model = sourced(context(), url());
    assert_eq!(
        model.retained_byte_charge(),
        model.request().retained_byte_charge()
            + 256
            + "sessionincarnationturnreused-callselected-servermcp_selected_tool".len()
    );
}

#[test]
fn human_forms_and_url_consent_roundtrip_through_real_inbox() {
    let (bridge, mut inbox, _principal) = bridge();
    for action in [McpFeatureAction::ResourceRead, McpFeatureAction::PromptGet] {
        for is_url in [false, true] {
            let request = if is_url {
                url()
            } else {
                parsed(
                    r#"{"message":"Read","requestedSchema":{"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}}"#,
                )
            };
            let mut future =
                bridge.present(human(action, request.clone()), CancellationToken::new());
            assert!(poll(&mut future).is_pending());
            let view = view(&mut inbox);
            assert!(
                matches!(view.elicitation().unwrap().source(), McpElicitationPromptSource::HumanFeature { owner, action: actual } if owner == &human_owner() && *actual == action)
            );
            assert!(Arc::ptr_eq(view.elicitation().unwrap().request(), &request));
            let answer = if is_url {
                r#"{"action":"accept"}"#
            } else {
                r#"{"action":"accept","content":{"n":9007199254740993.00000001}}"#
            };
            inbox.reply(view.token(), input(answer)).unwrap();
            let answer = block_on(future).unwrap();
            assert_eq!(answer.action(), McpElicitationAction::Accept);
            if !is_url {
                assert!(
                    answer
                        .canonical_json()
                        .get()
                        .contains("9007199254740993.00000001")
                );
            }
            assert_eq!(
                inbox.reply(view.token(), input(r#"{"action":"cancel"}"#)),
                Err(NativeInteractivePromptError::Stale)
            );
        }
    }
}

#[test]
fn human_owner_and_captured_activation_reject_foreign_or_stale_prompts() {
    let (bridge, mut inbox, _principal) = bridge();
    for owner in [
        BackgroundOutputOwner::new(
            SessionId::new("foreign").unwrap(),
            human_owner().session_incarnation_id().clone(),
        ),
        BackgroundOutputOwner::new(
            human_owner().session_id().clone(),
            SessionIncarnationId::new("foreign").unwrap(),
        ),
    ] {
        let request = McpElicitationPromptRequest::new_human_feature(
            owner,
            Arc::from("server"),
            McpFeatureAction::PromptGet,
            url(),
        )
        .unwrap();
        assert!(block_on(bridge.present(request, CancellationToken::new())).is_err());
    }
    let unpolled = bridge.present(
        human(McpFeatureAction::PromptGet, url()),
        CancellationToken::new(),
    );
    drop(_principal);
    let _principal = inbox.register(human_owner()).unwrap();
    assert!(block_on(unpolled).is_err());
    let mut future = bridge.present(
        human(McpFeatureAction::ResourceRead, url()),
        CancellationToken::new(),
    );
    assert!(poll(&mut future).is_pending());
    let old = view(&mut inbox);
    inbox
        .reply(old.token(), input(r#"{"action":"accept"}"#))
        .unwrap();
    drop(_principal);
    let _principal = inbox.register(human_owner()).unwrap();
    assert!(block_on(future).is_err());
    assert_eq!(
        inbox.cancel(old.token()),
        Err(NativeInteractivePromptError::Stale)
    );
}

#[test]
fn human_cancellation_wins_queued_displayed_and_ready_answers() {
    let (bridge, mut inbox, _principal) = bridge();
    for stage in 0..3 {
        let cancellation = CancellationToken::new();
        let mut future = bridge.present(
            human(McpFeatureAction::ResourceRead, url()),
            cancellation.clone(),
        );
        assert!(poll(&mut future).is_pending());
        let displayed = (stage > 0).then(|| view(&mut inbox));
        if stage == 2 {
            inbox
                .reply(
                    displayed.as_ref().unwrap().token(),
                    input(r#"{"action":"accept"}"#),
                )
                .unwrap();
        }
        cancellation.cancel();
        assert!(matches!(
            block_on(future),
            Err(McpElicitationPromptError::Cancelled)
        ));
        if let Some(displayed) = displayed {
            assert_eq!(
                inbox.cancel(displayed.token()),
                Err(NativeInteractivePromptError::Stale)
            );
        }
    }
    let mut future = bridge.present(
        human(McpFeatureAction::PromptGet, url()),
        CancellationToken::new(),
    );
    assert!(poll(&mut future).is_pending());
    let displayed = view(&mut inbox);
    inbox.cancel(displayed.token()).unwrap();
    assert_eq!(
        block_on(future).unwrap().action(),
        McpElicitationAction::Cancel
    );
}

#[test]
fn human_request_and_response_limits_remain_charged_until_consumed() {
    let request = url();
    let charge = human(McpFeatureAction::PromptGet, request.clone()).retained_byte_charge();
    for (limit, admitted) in [(charge - 1, false), (charge, true)] {
        let mut inbox = NativeInteractivePromptInbox::new(
            NativeInteractivePromptLimits::new(1, limit).unwrap(),
        )
        .unwrap();
        let bridge = inbox.router();
        let _principal = inbox.register(human_owner()).unwrap();
        let mut future = bridge.present(
            human(McpFeatureAction::PromptGet, request.clone()),
            CancellationToken::new(),
        );
        assert_eq!(poll(&mut future).is_pending(), admitted);
    }
    let answer_charge = 64 + r#"{"action":"accept"}"#.len();
    let limits = NativeInteractivePromptLimits::new(2, charge * 2)
        .unwrap()
        .with_response_bytes(answer_charge)
        .unwrap();
    let mut inbox = NativeInteractivePromptInbox::new(limits).unwrap();
    let bridge = inbox.router();
    let _principal = inbox.register(human_owner()).unwrap();
    let mut first = bridge.present(
        human(McpFeatureAction::PromptGet, request.clone()),
        CancellationToken::new(),
    );
    let mut second = bridge.present(
        human(McpFeatureAction::ResourceRead, request),
        CancellationToken::new(),
    );
    assert!(poll(&mut first).is_pending() && poll(&mut second).is_pending());
    let one = view(&mut inbox);
    inbox
        .reply(one.token(), input(r#"{"action":"accept"}"#))
        .unwrap();
    let two = view(&mut inbox);
    assert_eq!(
        inbox.reply(two.token(), input(r#"{"action":"accept"}"#)),
        Err(NativeInteractivePromptError::Limit)
    );
    assert_eq!(
        block_on(first).unwrap().action(),
        McpElicitationAction::Accept
    );
    inbox
        .reply(two.token(), input(r#"{"action":"accept"}"#))
        .unwrap();
    assert_eq!(
        block_on(second).unwrap().action(),
        McpElicitationAction::Accept
    );
}

#[test]
fn human_url_recovery_preserves_origin_and_obeys_cancellation() {
    let (bridge, mut inbox, _principal) = bridge();
    for action in [McpFeatureAction::ResourceRead, McpFeatureAction::PromptGet] {
        for answer in [
            McpUrlRecoveryAnswer::ContinueManually,
            McpUrlRecoveryAnswer::RetryBrowser,
            McpUrlRecoveryAnswer::Cancel,
        ] {
            let request = McpUrlRecoveryPromptRequest::new(human(action, url())).unwrap();
            let mut future = bridge.recover_url(request, CancellationToken::new());
            assert!(poll(&mut future).is_pending());
            let view = view(&mut inbox);
            assert!(
                matches!(view.url_recovery().unwrap().source().source(), McpElicitationPromptSource::HumanFeature { owner, action: actual } if owner == &human_owner() && *actual == action)
            );
            inbox
                .reply(
                    view.token(),
                    NativeInteractivePromptResponse::UrlRecovery(answer),
                )
                .unwrap();
            assert_eq!(block_on(future), Ok(answer));
        }
    }
    let cancellation = CancellationToken::new();
    let mut future = bridge.recover_url(
        McpUrlRecoveryPromptRequest::new(human(McpFeatureAction::PromptGet, url())).unwrap(),
        cancellation.clone(),
    );
    assert!(poll(&mut future).is_pending());
    let old = view(&mut inbox);
    cancellation.cancel();
    assert_eq!(block_on(future), Err(McpElicitationPromptError::Cancelled));
    assert_eq!(
        inbox.cancel(old.token()),
        Err(NativeInteractivePromptError::Stale)
    );
}
