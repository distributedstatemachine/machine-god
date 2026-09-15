use super::*;
use crate::mcp::{
    interaction::*,
    mrtr::{
        McpElicitationAction, McpElicitationRequest, McpInputRequestPayload, McpInputRequired,
        McpMrtrLimits,
    },
    protocol::ProtocolVersion,
};
use serde_json::value::RawValue;
mod human;

fn raw(text: &str) -> Box<RawValue> {
    RawValue::from_string(text.into()).unwrap()
}
fn parsed(params: &str) -> Arc<McpElicitationRequest> {
    Arc::new(
        McpElicitationRequest::parse(
            &raw(params),
            ProtocolVersion::Modern,
            McpMrtrLimits::default(),
        )
        .unwrap(),
    )
}
fn form() -> Arc<McpElicitationRequest> {
    parsed(
        r#"{"message":"Private form prompt","requestedSchema":{"type":"object","properties":{"name":{"type":"string","minLength":2},"large":{"type":"integer","minimum":9007199254740993},"ratio":{"type":"number","multipleOf":0.1},"enabled":{"type":"boolean"},"color":{"type":"string","enum":["red","blue"]},"tags":{"type":"array","items":{"type":"string","enum":["a","b"]}}},"required":["name","large","ratio","enabled","color","tags"]}}"#,
    )
}
fn url() -> Arc<McpElicitationRequest> {
    parsed(r#"{"mode":"url","message":"Authorize","url":"https://example.test/private"}"#)
}
fn input(text: &str) -> NativeInteractivePromptResponse {
    NativeInteractivePromptResponse::Elicitation(McpElicitationAnswerInput::new(raw(text)).unwrap())
}
fn sourced(
    context: ToolContext,
    request: Arc<McpElicitationRequest>,
) -> McpElicitationPromptRequest {
    McpElicitationPromptRequest::new(
        context,
        Arc::from("selected-server"),
        ToolName::new("mcp_selected_tool").unwrap(),
        request,
    )
    .unwrap()
}
fn prompt(
    bridge: &NativeInteractivePromptBridge,
    request: Arc<McpElicitationRequest>,
    cancel: CancellationToken,
) -> BoxFuture<'_, Result<McpElicitationAnswer, McpElicitationPromptError>> {
    bridge.present(sourced(context(), request), cancel)
}

#[test]
fn real_inbox_preserves_context_shared_request_and_every_form_kind() {
    let (bridge, mut inbox, _principal) = bridge();
    let request = form();
    let mut future = prompt(&bridge, request.clone(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let view = view(&mut inbox);
    assert!(view.permission().is_none() && view.question().is_none());
    let actual = view.elicitation().unwrap();
    let McpElicitationPromptSource::ModelTool {
        context: actual_context,
        tool,
    } = actual.source()
    else {
        panic!("actual model tool source");
    };
    assert_eq!(actual_context, &context());
    assert_eq!(actual.server(), "selected-server");
    assert_eq!(tool.as_str(), "mcp_selected_tool");
    assert!(Arc::ptr_eq(actual.request(), &request));
    assert_eq!(actual.request().form_schema().unwrap().fields().len(), 6);
    inbox.reply(view.token(), input(r#"{"action":"accept","content":{"name":"Alice","large":9007199254740993.0,"ratio":0.3000000000000000000,"enabled":true,"color":"red","tags":["a","b"]},"ignored":1e400}"#)).unwrap();
    let answer = block_on(future).unwrap();
    assert_eq!(answer.action(), McpElicitationAction::Accept);
    assert!(answer.canonical_json().get().contains("9007199254740993.0"));
    assert!(
        answer
            .canonical_json()
            .get()
            .contains("0.3000000000000000000")
    );
    assert!(!answer.canonical_json().get().contains("ignored"));
    assert_eq!(
        inbox.reply(view.token(), input(r#"{"action":"cancel"}"#)),
        Err(NativeInteractivePromptError::Stale)
    );
    for debug in [
        format!("{actual:?}"),
        format!("{answer:?}"),
        format!(
            "{:?}",
            McpElicitationAnswerInput::new(raw(r#"{"private":"value"}"#)).unwrap()
        ),
    ] {
        assert!(!debug.contains("Private") && !debug.contains("Alice") && !debug.contains("value"));
    }
}

#[test]
fn invalid_and_cross_kind_answers_leave_displayed_request_usable() {
    let (bridge, mut inbox, _principal) = bridge();
    let mut future = prompt(&bridge, form(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let view = view(&mut inbox);
    for response in [
        NativeInteractivePromptResponse::Permission(PermissionPromptDecision::AllowOnce),
        NativeInteractivePromptResponse::Question(QuestionPromptOutcome::Cancelled),
        input(r#"{"action":"accept","content":{}}"#),
        input(r#"{"action":"future"}"#),
        input(r#"{"action":"cancel","action":"accept"}"#),
        input(
            r#"{"action":"accept","content":{"name":"A","large":1,"ratio":0.31,"enabled":"yes","color":"green","tags":["a","a"]}}"#,
        ),
    ] {
        assert!(inbox.reply(view.token(), response).is_err());
        assert_eq!(self::super::view(&mut inbox).token(), view.token());
        assert!(poll(&mut future).is_pending());
    }
    inbox
        .reply(
            view.token(),
            input(r#"{"action":"decline","content":{"ignored":"private"}}"#),
        )
        .unwrap();
    let answer = block_on(future).unwrap();
    assert_eq!(answer.action(), McpElicitationAction::Decline);
    assert_eq!(answer.canonical_json().get(), r#"{"action":"decline"}"#);
    let mut permission = permission(&bridge, "permission");
    assert!(poll(&mut permission).is_pending());
    let view = self::super::view(&mut inbox);
    assert!(
        inbox
            .reply(view.token(), input(r#"{"action":"accept"}"#))
            .is_err()
    );
    inbox.cancel(view.token()).unwrap();
    assert_eq!(block_on(permission), Ok(PermissionPromptDecision::Deny));
}

#[test]
fn url_actions_are_data_and_engine_cancellation_has_precedence() {
    let (bridge, mut inbox, _principal) = bridge();
    for action in [
        McpElicitationAction::Accept,
        McpElicitationAction::Decline,
        McpElicitationAction::Cancel,
    ] {
        let mut future = prompt(&bridge, url(), CancellationToken::new());
        assert!(poll(&mut future).is_pending());
        let view = view(&mut inbox);
        assert_eq!(
            view.elicitation().unwrap().request().url(),
            Some("https://example.test/private")
        );
        assert!(
            inbox
                .reply(view.token(), input(r#"{"action":"accept","content":null}"#))
                .is_err()
        );
        match action {
            McpElicitationAction::Accept => inbox
                .reply(view.token(), input(r#"{"action":"accept"}"#))
                .unwrap(),
            McpElicitationAction::Decline => inbox
                .reply(view.token(), input(r#"{"action":"decline"}"#))
                .unwrap(),
            McpElicitationAction::Cancel => inbox.cancel(view.token()).unwrap(),
        }
        assert_eq!(block_on(future).unwrap().action(), action);
    }
    let cancel = CancellationToken::new();
    let mut future = prompt(&bridge, url(), cancel.clone());
    assert!(poll(&mut future).is_pending());
    let view = view(&mut inbox);
    inbox
        .reply(view.token(), input(r#"{"action":"accept"}"#))
        .unwrap();
    cancel.cancel();
    assert!(matches!(
        block_on(future),
        Err(McpElicitationPromptError::Cancelled)
    ));
    assert_eq!(
        inbox.cancel(view.token()),
        Err(NativeInteractivePromptError::Stale)
    );
}

#[test]
fn scope_owner_tokens_unpolled_drop_and_unavailable_are_explicit() {
    let (bridge, mut inbox, principal) = bridge();
    let request = url();
    let count = Arc::strong_count(&request);
    drop(prompt(&bridge, request.clone(), CancellationToken::new()));
    assert_eq!(Arc::strong_count(&request), count);
    assert!(
        inbox
            .poll_prompt(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let mut wrong = context();
    wrong.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert!(
        block_on(bridge.present(sourced(wrong, request.clone()), CancellationToken::new()))
            .is_err()
    );
    let never = prompt(&bridge, request.clone(), CancellationToken::new());
    let mut future = prompt(&bridge, request.clone(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let old = view(&mut inbox);
    let (other, mut other_inbox, _other_principal) = self::super::bridge();
    let mut other_future = prompt(&other, request.clone(), CancellationToken::new());
    assert!(poll(&mut other_future).is_pending());
    let foreign = view(&mut other_inbox);
    assert_eq!(
        inbox.reply(foreign.token(), input(r#"{"action":"accept"}"#)),
        Err(NativeInteractivePromptError::Stale)
    );
    drop(principal);
    let _replacement_principal = inbox.register(owner()).unwrap();
    assert!(block_on(never).is_err() && block_on(future).is_err());
    assert_eq!(
        inbox.cancel(old.token()),
        Err(NativeInteractivePromptError::Stale)
    );
    drop(other_future);
    assert_eq!(
        other_inbox.cancel(foreign.token()),
        Err(NativeInteractivePromptError::Stale)
    );
    assert!(matches!(
        block_on(
            UnavailableMcpElicitationPresenter
                .present(sourced(context(), request), CancellationToken::new())
        ),
        Err(McpElicitationPromptError::Unavailable)
    ));
}

#[test]
fn request_and_response_aggregate_charges_survive_reply_until_consumption() {
    let request = url();
    let charge = sourced(context(), request.clone()).retained_byte_charge();
    for (limit, accepted) in [(charge - 1, false), (charge, true)] {
        let mut inbox = NativeInteractivePromptInbox::new(
            NativeInteractivePromptLimits::new(2, limit).unwrap(),
        )
        .unwrap();
        let bridge = inbox.router();
        let _principal = inbox.register(owner()).unwrap();
        let mut future = prompt(&bridge, request.clone(), CancellationToken::new());
        assert_eq!(poll(&mut future).is_pending(), accepted);
        if accepted {
            assert!(block_on(prompt(&bridge, request.clone(), CancellationToken::new())).is_err());
        }
    }
    let bytes = r#"{"action":"accept"}"#.len() + 64;
    for (limit, accepted) in [(bytes - 1, false), (bytes, true)] {
        let limits = NativeInteractivePromptLimits::new(2, charge * 2)
            .unwrap()
            .with_response_bytes(limit)
            .unwrap();
        let mut inbox = NativeInteractivePromptInbox::new(limits).unwrap();
        let bridge = inbox.router();
        let _principal = inbox.register(owner()).unwrap();
        let mut first = prompt(&bridge, request.clone(), CancellationToken::new());
        let mut second = prompt(&bridge, request.clone(), CancellationToken::new());
        assert!(poll(&mut first).is_pending() && poll(&mut second).is_pending());
        let one = view(&mut inbox);
        assert_eq!(
            inbox
                .reply(one.token(), input(r#"{"action":"accept"}"#))
                .is_ok(),
            accepted
        );
        if accepted {
            let two = view(&mut inbox);
            assert_eq!(
                inbox.reply(two.token(), input(r#"{"action":"accept"}"#)),
                Err(NativeInteractivePromptError::Limit)
            );
            assert!(block_on(first).is_ok());
            inbox
                .reply(two.token(), input(r#"{"action":"accept"}"#))
                .unwrap();
            assert!(block_on(second).is_ok());
        }
    }
    assert!(
        NativeInteractivePromptLimits::default()
            .with_response_bytes(0)
            .is_err()
    );
    assert!(
        NativeInteractivePromptLimits::default()
            .with_response_bytes(usize::MAX)
            .is_err()
    );
    let raw = raw(&format!(
        "\"{}\"",
        "x".repeat(MAX_MCP_ELICITATION_ANSWER_BYTES)
    ));
    assert!(matches!(
        McpElicitationAnswerInput::new(raw),
        Err(McpElicitationPromptError::Limit)
    ));
}

#[test]
fn cancellation_and_reentrant_ready_cleanup_never_reopen_an_answered_token() {
    let (bridge, mut inbox, _principal) = bridge();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        block_on(prompt(&bridge, url(), cancel)),
        Err(McpElicitationPromptError::Cancelled)
    ));
    let cancel = CancellationToken::new();
    let mut future = prompt(&bridge, url(), cancel.clone());
    assert!(poll(&mut future).is_pending());
    let old = view(&mut inbox);
    cancel.cancel();
    assert!(block_on(future).is_err());
    assert_eq!(
        inbox.cancel(old.token()),
        Err(NativeInteractivePromptError::Stale)
    );
    let mut future = prompt(&bridge, url(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let view = view(&mut inbox);
    inbox
        .reply(view.token(), input(r#"{"action":"accept"}"#))
        .unwrap();
    let shared = bridge.shared.clone();
    let token = view.token().clone();
    let (wake, handle) = reentrant_waker(Callback::Drop, move || {
        assert!(shared.state.try_lock().is_ok());
        assert_eq!(
            shared.reply(&token, input(r#"{"action":"accept"}"#)),
            Err(NativeInteractivePromptError::Stale)
        );
    });
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(&wake)),
        Poll::Ready(Ok(_))
    ));
    drop(wake);
    assert!(handle.calls() > 0);
}

#[test]
fn nested_256_field_form_reaches_inbox_without_standalone_reparse() {
    let properties = (0..256)
        .map(|index| format!("\"field{index}\":{{\"type\":\"boolean\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    let params = format!(
        "{{\"message\":\"All fields\",\"requestedSchema\":{{\"type\":\"object\",\"properties\":{{{properties}}}}}}}"
    );
    assert!(
        McpElicitationRequest::parse(
            &raw(&params),
            ProtocolVersion::Modern,
            McpMrtrLimits::default()
        )
        .is_err()
    );
    let required = McpInputRequired::parse(
        &raw(&format!("{{\"inputRequests\":{{\"form\":{{\"method\":\"elicitation/create\",\"params\":{params}}}}}}}")),
        McpMrtrLimits::default(),
    ).unwrap();
    let McpInputRequestPayload::Elicitation(request) = required.requests()[0].payload() else {
        panic!("elicitation request");
    };
    let request = request.clone();
    drop(required);
    let (bridge, mut inbox, _principal) = bridge();
    let mut future = prompt(&bridge, request.clone(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let view = view(&mut inbox);
    let displayed = view.elicitation().unwrap();
    assert!(Arc::ptr_eq(displayed.request(), &request));
    assert_eq!(
        displayed.request().form_schema().unwrap().fields().len(),
        256
    );
    let answers = (0..256)
        .map(|index| format!("\"field{index}\":true"))
        .collect::<Vec<_>>()
        .join(",");
    inbox
        .reply(
            view.token(),
            input(&format!(
                "{{\"action\":\"accept\",\"content\":{{{answers}}}}}"
            )),
        )
        .unwrap();
    let answer = block_on(future).unwrap();
    assert_eq!(answer.action(), McpElicitationAction::Accept);
    assert!(answer.canonical_json().get().contains("\"field255\":true"));
}

#[test]
fn source_identity_is_required_bounded_charged_and_redacted() {
    let request = url();
    let make = |server: &str, tool: &str| {
        McpElicitationPromptRequest::new(
            context(),
            Arc::from(server),
            ToolName::new(tool).unwrap(),
            request.clone(),
        )
    };
    assert!(matches!(
        make("", "tool"),
        Err(McpElicitationPromptError::InvalidSource)
    ));
    assert!(make(&"s".repeat(257), "tool").is_err());
    assert!(ToolName::new("t".repeat(129)).is_err());
    let short = make("s", "t").unwrap();
    let long = make(&"s".repeat(256), &"t".repeat(128)).unwrap();
    assert_eq!(
        long.retained_byte_charge() - short.retained_byte_charge(),
        382
    );
    assert!(!format!("{long:?}").contains("ssss"));
}

#[test]
fn unrelated_owner_retirement_preserves_original_form_and_url_response_custody() {
    for request in [form(), url()] {
        let (router, mut inbox, mut parent) = bridge();
        let mut child_context = context();
        child_context.session_id = SessionId::new("child").unwrap();
        let child = inbox
            .register(BackgroundOutputOwner::new(
                child_context.session_id.clone(),
                child_context.session_incarnation_id.clone(),
            ))
            .unwrap();
        let mut parent_future = router.present(
            sourced(context(), request.clone()),
            CancellationToken::new(),
        );
        assert!(poll(&mut parent_future).is_pending());
        let mut child_future = router.present(
            sourced(child_context.clone(), request.clone()),
            CancellationToken::new(),
        );
        assert!(poll(&mut child_future).is_pending());
        let page = inbox
            .page(&mut Context::from_waker(Waker::noop()), None, 64)
            .unwrap();
        let child_view = inbox.select_prompt(page.entries()[1].token()).unwrap();
        assert!(Arc::ptr_eq(
            child_view.elicitation().unwrap().request(),
            &request
        ));
        assert!(
            matches!(child_view.elicitation().unwrap().source(), McpElicitationPromptSource::ModelTool { context, .. } if context == &child_context)
        );
        inbox
            .reply(child_view.token(), input(r#"{"action":"decline"}"#))
            .unwrap();
        parent.retire();
        assert!(block_on(parent_future).is_err());
        assert_eq!(
            block_on(child_future).unwrap().action(),
            McpElicitationAction::Decline
        );
        let mut next = router.present(sourced(child_context, request), CancellationToken::new());
        assert!(poll(&mut next).is_pending());
        drop(child);
        assert!(block_on(next).is_err());
    }
}
