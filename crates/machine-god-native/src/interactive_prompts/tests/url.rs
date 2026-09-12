use super::*;
use crate::mcp::{
    interaction::*,
    mrtr::{McpElicitationRequest, McpMrtrLimits},
    protocol::ProtocolVersion,
};
use serde_json::value::RawValue;

fn source(context: ToolContext) -> McpElicitationPromptRequest {
    let raw = RawValue::from_string(
        r#"{"mode":"url","message":"private-message","url":"https://example.test/private-secret"}"#
            .into(),
    )
    .unwrap();
    McpElicitationPromptRequest::new(
        context,
        Arc::from("private-server"),
        ToolName::new("private-tool").unwrap(),
        Arc::new(
            McpElicitationRequest::parse(&raw, ProtocolVersion::Modern, McpMrtrLimits::default())
                .unwrap(),
        ),
    )
    .unwrap()
}
fn recovery() -> McpUrlRecoveryPromptRequest {
    McpUrlRecoveryPromptRequest::new(source(context())).unwrap()
}

#[test]
fn typed_choices_are_not_interchangeable_with_json_or_each_other() {
    let (bridge, mut inbox) = bridge();
    for answer in [
        McpUrlRecoveryAnswer::ContinueManually,
        McpUrlRecoveryAnswer::RetryBrowser,
        McpUrlRecoveryAnswer::Cancel,
    ] {
        let mut future = bridge.recover_url(recovery(), CancellationToken::new());
        assert!(poll(&mut future).is_pending());
        let view = view(&mut inbox);
        let request = view.url_recovery().unwrap();
        assert_eq!(request.source().context(), &context());
        assert!(view.elicitation().is_none());
        for response in [
            NativeInteractivePromptResponse::Question(QuestionPromptOutcome::Cancelled),
            NativeInteractivePromptResponse::Elicitation(
                McpElicitationAnswerInput::new(
                    RawValue::from_string(r#"{"action":"accept"}"#.into()).unwrap(),
                )
                .unwrap(),
            ),
        ] {
            assert_eq!(
                inbox.reply(view.token(), response),
                Err(NativeInteractivePromptError::InvalidResponse)
            );
        }
        inbox
            .reply(
                view.token(),
                NativeInteractivePromptResponse::UrlRecovery(answer),
            )
            .unwrap();
        assert_eq!(block_on(future), Ok(answer));
        assert_eq!(
            inbox.cancel(view.token()),
            Err(NativeInteractivePromptError::Stale)
        );
    }
}

#[test]
fn cancellation_drop_and_retirement_invalidate_recovery() {
    let (bridge, mut inbox) = bridge();
    let cancellation = CancellationToken::new();
    let mut future = bridge.recover_url(recovery(), cancellation.clone());
    assert!(poll(&mut future).is_pending());
    let old = view(&mut inbox);
    inbox
        .reply(
            old.token(),
            NativeInteractivePromptResponse::UrlRecovery(McpUrlRecoveryAnswer::RetryBrowser),
        )
        .unwrap();
    cancellation.cancel();
    assert_eq!(block_on(future), Err(McpElicitationPromptError::Cancelled));
    let mut future = bridge.recover_url(recovery(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let old = view(&mut inbox);
    drop(future); // A dropped recovery observer must remove its UI.
    assert_eq!(
        inbox.cancel(old.token()),
        Err(NativeInteractivePromptError::Stale)
    );
    let mut future = bridge.recover_url(recovery(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let old = view(&mut inbox);
    inbox
        .reply(
            old.token(),
            NativeInteractivePromptResponse::UrlRecovery(McpUrlRecoveryAnswer::RetryBrowser),
        )
        .unwrap();
    inbox.activate(owner()).unwrap();
    assert!(block_on(future).is_err());
    assert_eq!(
        inbox.cancel(old.token()),
        Err(NativeInteractivePromptError::Stale)
    );
    let mut future = bridge.recover_url(recovery(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let old = view(&mut inbox);
    inbox.cancel(old.token()).unwrap();
    assert_eq!(block_on(future), Ok(McpUrlRecoveryAnswer::Cancel));
    let mut future = bridge.recover_url(recovery(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    inbox.close();
    assert!(block_on(future).is_err());
}

#[test]
fn defaults_are_inert_unavailable_and_respect_real_cancellation() {
    let presenter = UnavailableMcpElicitationPresenter;
    assert_eq!(
        block_on(presenter.recover_url(recovery(), CancellationToken::new())),
        Err(McpElicitationPromptError::Unavailable)
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        block_on(presenter.recover_url(recovery(), cancellation.clone())),
        Err(McpElicitationPromptError::Cancelled)
    );
    let (bridge, mut inbox) = bridge();
    drop(bridge.recover_url(recovery(), CancellationToken::new()));
    assert!(
        inbox
            .poll_prompt(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let future = bridge.recover_url(recovery(), CancellationToken::new());
    inbox.activate(owner()).unwrap();
    assert!(block_on(future).is_err());
    let mut foreign = context();
    foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert!(
        block_on(bridge.recover_url(
            McpUrlRecoveryPromptRequest::new(source(foreign)).unwrap(),
            CancellationToken::new()
        ))
        .is_err()
    );
}

#[test]
fn charges_include_source_and_remain_until_answer_consumption() {
    let charge = recovery().retained_byte_charge();
    let limits = NativeInteractivePromptLimits::new(2, charge * 2)
        .unwrap()
        .with_response_bytes(64)
        .unwrap();
    let (bridge, mut inbox) = NativeInteractivePromptBridge::new(limits).unwrap();
    inbox.activate(owner()).unwrap();
    let mut first = bridge.recover_url(recovery(), CancellationToken::new());
    let mut second = bridge.recover_url(recovery(), CancellationToken::new());
    assert!(poll(&mut first).is_pending() && poll(&mut second).is_pending());
    let one = view(&mut inbox);
    inbox
        .reply(
            one.token(),
            NativeInteractivePromptResponse::UrlRecovery(McpUrlRecoveryAnswer::RetryBrowser),
        )
        .unwrap();
    let two = view(&mut inbox);
    assert_eq!(
        inbox.reply(
            two.token(),
            NativeInteractivePromptResponse::UrlRecovery(McpUrlRecoveryAnswer::RetryBrowser)
        ),
        Err(NativeInteractivePromptError::Limit)
    );
    assert_eq!(block_on(first), Ok(McpUrlRecoveryAnswer::RetryBrowser));
    inbox.cancel(two.token()).unwrap();
    assert_eq!(block_on(second), Ok(McpUrlRecoveryAnswer::Cancel));
    assert!(charge > source(context()).retained_byte_charge());
    assert!(!format!("{:?}", recovery()).contains("private"));
    for server in [Arc::from(""), Arc::from("x".repeat(257))] {
        assert!(
            McpElicitationPromptRequest::new(
                context(),
                server,
                ToolName::new("tool").unwrap(),
                source(context()).request().clone()
            )
            .is_err()
        );
    }
}

#[test]
fn recovery_source_and_independent_request_budgets_are_checked() {
    let raw = RawValue::from_string(
        r#"{"message":"form","requestedSchema":{"type":"object","properties":{}}}"#.into(),
    )
    .unwrap();
    let form = Arc::new(
        McpElicitationRequest::parse(&raw, ProtocolVersion::Modern, McpMrtrLimits::default())
            .unwrap(),
    );
    assert!(
        McpUrlRecoveryPromptRequest::new(
            McpElicitationPromptRequest::new(
                context(),
                Arc::from("server"),
                ToolName::new("tool").unwrap(),
                form
            )
            .unwrap()
        )
        .is_err()
    );
    for recovery_phase in [false, true] {
        let charge = if recovery_phase {
            recovery().retained_byte_charge()
        } else {
            source(context()).retained_byte_charge()
        };
        for (limit, admitted) in [(charge - 1, false), (charge, true)] {
            let (bridge, mut inbox) = NativeInteractivePromptBridge::new(
                NativeInteractivePromptLimits::new(1, limit).unwrap(),
            )
            .unwrap();
            inbox.activate(owner()).unwrap();
            if recovery_phase {
                let mut future = bridge.recover_url(recovery(), CancellationToken::new());
                assert_eq!(poll(&mut future).is_pending(), admitted);
            } else {
                let mut future = bridge.present(source(context()), CancellationToken::new());
                assert_eq!(poll(&mut future).is_pending(), admitted);
            }
        }
    }
}

#[test]
fn cancellation_removes_queued_and_presented_recovery_without_synthetic_retry() {
    let (bridge, mut inbox) = bridge();
    for display in [false, true] {
        let cancellation = CancellationToken::new();
        let mut future = bridge.recover_url(recovery(), cancellation.clone());
        assert!(poll(&mut future).is_pending());
        let token = display.then(|| view(&mut inbox).token().clone());
        cancellation.cancel();
        assert_eq!(block_on(future), Err(McpElicitationPromptError::Cancelled));
        if let Some(token) = token {
            assert_eq!(
                inbox.cancel(&token),
                Err(NativeInteractivePromptError::Stale)
            );
        }
        assert!(
            inbox
                .poll_prompt(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    let mut future = bridge.recover_url(recovery(), CancellationToken::new());
    assert!(poll(&mut future).is_pending());
    let view = view(&mut inbox);
    let shared = bridge.shared.clone();
    let token = view.token().clone();
    inbox.cancel(view.token()).unwrap();
    let (wake, handle) = reentrant_waker(Callback::Drop, move || {
        assert!(shared.state.try_lock().is_ok());
        assert_eq!(
            shared.reply(
                &token,
                NativeInteractivePromptResponse::UrlRecovery(McpUrlRecoveryAnswer::RetryBrowser)
            ),
            Err(NativeInteractivePromptError::Stale)
        );
    });
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(&wake)),
        Poll::Ready(Ok(McpUrlRecoveryAnswer::Cancel))
    ));
    drop(wake);
    assert!(handle.calls() > 0);
}
